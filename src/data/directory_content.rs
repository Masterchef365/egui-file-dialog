use crate::config::{FileDialogConfig, FileFilter};
use crate::FileSystem;
use egui::mutex::Mutex;
use poll_promise::Promise;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::SystemTime;
use std::io;

/// Contains the metadata of a directory item.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Metadata {
    pub(crate) size: Option<u64>,
    pub(crate) last_modified: Option<SystemTime>,
    pub(crate) created: Option<SystemTime>,
    pub(crate) file_type: Option<String>,
}

impl Metadata {
    /// Create a new custom metadata
    pub fn new(
        size: Option<u64>,
        last_modified: Option<SystemTime>,
        created: Option<SystemTime>,
        file_type: Option<String>,
    ) -> Self {
        Self {
            size,
            last_modified,
            created,
            file_type,
        }
    }
}

/// Contains the information of a directory item.
///
/// This struct is mainly there so that the information and metadata can be loaded once and not that
/// a request has to be sent to the OS every frame using, for example, `path.is_file()`.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct DirectoryEntry {
    path: PathBuf,
    metadata: Metadata,
    is_directory: bool,
    is_system_file: bool,
    is_hidden: bool,
    icon: String,
    /// If the item is marked as selected as part of a multi selection.
    pub selected: bool,
}

impl DirectoryEntry {
    /// Creates a new directory entry from a path
    pub fn from_path(config: &FileDialogConfig, path: &Path, file_system: &dyn FileSystem) -> Self {
        Self {
            path: path.to_path_buf(),
            metadata: file_system.metadata(path).unwrap_or_default(),
            is_directory: file_system.is_dir(path),
            is_system_file: !file_system.is_dir(path) && !file_system.is_file(path),
            icon: gen_path_icon(config, path, file_system),
            is_hidden: file_system.is_path_hidden(path),
            selected: false,
        }
    }

    /// Returns the metadata of the directory entry.
    pub const fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Checks if the path of the current directory entry matches the other directory entry.
    pub fn path_eq(&self, other: &Self) -> bool {
        other.as_path() == self.as_path()
    }

    /// Returns true if the item is a directory.
    /// False is returned if the item is a file or the path did not exist when the
    /// `DirectoryEntry` object was created.
    pub const fn is_dir(&self) -> bool {
        self.is_directory
    }

    /// Returns true if the item is a file.
    /// False is returned if the item is a directory or the path did not exist when the
    /// `DirectoryEntry` object was created.
    pub const fn is_file(&self) -> bool {
        !self.is_directory
    }

    /// Returns true if the item is a system file.
    pub const fn is_system_file(&self) -> bool {
        self.is_system_file
    }

    /// Returns the icon of the directory item.
    pub fn icon(&self) -> &str {
        &self.icon
    }

    /// Returns the path of the directory item.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Clones the path of the directory item.
    pub fn to_path_buf(&self) -> PathBuf {
        self.path.clone()
    }

    /// Returns the file name of the directory item.
    pub fn file_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| {
                // Make sure the root directories like ["C:", "\"] and ["\\?\C:", "\"] are
                // displayed correctly
                #[cfg(windows)]
                if self.path.components().count() == 2 {
                    let path = self
                        .path
                        .iter()
                        .nth(0)
                        .and_then(|seg| seg.to_str())
                        .unwrap_or_default();

                    // Skip path namespace prefix if present, for example: "\\?\C:"
                    if path.contains(r"\\?\") {
                        return path.get(4..).unwrap_or(path);
                    }

                    return path;
                }

                // Make sure the root directory "/" is displayed correctly
                #[cfg(not(windows))]
                if self.path.iter().count() == 1 {
                    return self.path.to_str().unwrap_or_default();
                }

                ""
            })
    }

    /// Returns whether the path this `DirectoryEntry` points to is considered hidden.
    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }
}

/*
/// Contains the state of the directory content.
#[derive(Debug, PartialEq, Eq)]
pub enum DirectoryContentState {
    /// If we are currently waiting for the loading process on another thread.
    /// The value is the timestamp when the loading process started.
    Pending(SystemTime),
    /// If loading the directory content finished since the last update call.
    /// This is only returned once.
    Finished,
    /// If loading the directory content was successful.
    Success,
    /// If there was an error loading the directory content.
    /// The value contains the error message.
    Errored(String),
}
*/

type DirectoryContentReceiver =
    Option<Arc<Mutex<mpsc::Receiver<Result<Vec<DirectoryEntry>, std::io::Error>>>>>;

/// Contains the content of a directory.
pub struct DirectoryContent {
    /// Current state of the directory content.
    pub(crate) content: Promise<Result<Vec<DirectoryEntry>, String>>,
    /// Timestamp of Promise creation
    pub(crate) creation_time: SystemTime,
}

impl Default for DirectoryContent {
    fn default() -> Self {
        Self {
            content: Promise::from_ready(Ok(vec![])),
            creation_time: SystemTime::now(),
            //state: DirectoryContentState::Success,
            //content: Vec::new(),
            //content_recv: None,
        }
    }
}

impl std::fmt::Debug for DirectoryContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO
        f.debug_struct("DirectoryContent")
            .finish()
        /*
        f.debug_struct("DirectoryContent")
            .field("state", &self.state)
            .field("content", &self.content)
            .field(
                "content_recv",
                if self.content_recv.is_some() {
                    &"<Receiver>"
                } else {
                    &"None"
                },
            )
            .finish()
        */
    }
}

impl DirectoryContent {
    /// Create a new `DirectoryContent` object and loads the contents of the given path.
    /// Use `include_files` to include or exclude files in the content list.
    pub fn from_path(
        config: &FileDialogConfig,
        path: &Path,
        include_files: bool,
        file_filter: Option<&FileFilter>,
        file_system: Arc<dyn FileSystem + Sync + Send + 'static>,
    ) -> Self {
        if config.load_via_thread {
            Self::with_thread(config, path, include_files, file_filter, file_system)
        } else {
            Self::without_thread(config, path, include_files, file_filter, &*file_system)
        }
    }

    fn with_thread(
        config: &FileDialogConfig,
        path: &Path,
        include_files: bool,
        file_filter: Option<&FileFilter>,
        file_system: Arc<dyn FileSystem + Send + Sync + 'static>,
    ) -> Self {
        let c = config.clone();
        let p = path.to_path_buf();
        let f = file_filter.cloned();

        let content = Promise::spawn_thread("File dialog load", move || {
            load_directory(&c, &p, include_files, f.as_ref(), &*file_system).map_err(|e| e.to_string())
        });

        Self {
            content,
            creation_time: SystemTime::now(),
        }
    }

    fn without_thread(
        config: &FileDialogConfig,
        path: &Path,
        include_files: bool,
        file_filter: Option<&FileFilter>,
        file_system: &dyn FileSystem,
    ) -> Self {
        Self {
            content: Promise::from_ready(load_directory(config, path, include_files, file_filter, file_system).map_err(|e| e.to_string())),
            creation_time: SystemTime::now(),
        }
    }

    /// Returns an iterator in the given range of the directory cotnents.
    /// No filters are applied using this iterator.
    pub fn iter_range_mut(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = &mut DirectoryEntry> {
        match self.content.ready_mut() {
            Some(Ok(dirs)) => &mut dirs[range],
            _ => &mut [],
        }.iter_mut()
    }

    /// Returns an iterator in the given range of the directory cotnents.
    /// No filters are applied using this iterator.
    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut DirectoryEntry> {
        match self.content.ready_mut() {
            Some(Ok(dirs)) => &mut dirs[..],
            _ => &mut [],
        }.iter_mut()
    }

    /// Returns an iterator over the directory cotnents.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = &DirectoryEntry> {
        match self.content.ready() {
            Some(Ok(dirs)) => &dirs[..],
            _ => &[],
        }.iter()
    }

    pub fn filtered_iter<'s>(
        &'s self,
        search_value: &'s str,
    ) -> impl Iterator<Item = &'s DirectoryEntry> + 's {
        self.iter()
            .filter(|p| apply_search_value(p, search_value))
    }

    pub fn filtered_iter_mut<'s>(
        &'s mut self,
        search_value: &'s str,
    ) -> impl Iterator<Item = &'s mut DirectoryEntry> + 's {
        self.iter_mut()
            .filter(|p| apply_search_value(p, search_value))
    }

    /// Marks each element in the content as unselected.
    pub fn reset_multi_selection(&mut self) {
        for item in self.iter_mut() {
            item.selected = false;
        }
    }

    /// Returns the number of elements inside the directory.
    pub fn len(&self) -> usize {
        match self.content.ready() {
            Some(Ok(content)) => content.len(),
            _ => 0,
        }
    }

    /// Pushes a new item to the content.
    pub fn push(&mut self, item: DirectoryEntry) {
        if let Some(Ok(content)) = self.content.ready_mut() {
            content.push(item);
        }
    }
}

fn apply_search_value(entry: &DirectoryEntry, value: &str) -> bool {
    value.is_empty()
        || entry
            .file_name()
            .to_lowercase()
            .contains(&value.to_lowercase())
}

/// Loads the contents of the given directory.
fn load_directory(
    config: &FileDialogConfig,
    path: &Path,
    include_files: bool,
    file_filter: Option<&FileFilter>,
    file_system: &dyn FileSystem,
) -> io::Result<Vec<DirectoryEntry>> {
    let mut result: Vec<DirectoryEntry> = Vec::new();
    for path in file_system.read_dir(path)? {
        let entry = DirectoryEntry::from_path(config, &path, file_system);

        if !config.storage.show_system_files && entry.is_system_file() {
            continue;
        }

        if !include_files && entry.is_file() {
            continue;
        }

        if !config.storage.show_hidden && entry.is_hidden() {
            continue;
        }

        if let Some(file_filter) = file_filter {
            if entry.is_file() && !(file_filter.filter)(entry.as_path()) {
                continue;
            }
        }

        result.push(entry);
    }

    result.sort_by(|a, b| {
        if a.is_dir() == b.is_dir() {
            a.file_name().cmp(b.file_name())
        } else if a.is_dir() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });

    Ok(result)
}


/// Generates the icon for the specific path.
/// The default icon configuration is taken into account, as well as any configured
/// file icon filters.
fn gen_path_icon(config: &FileDialogConfig, path: &Path, file_system: &dyn FileSystem) -> String {
    for def in &config.file_icon_filters {
        if (def.filter)(path) {
            return def.icon.clone();
        }
    }

    if file_system.is_dir(path) {
        config.default_folder_icon.clone()
    } else {
        config.default_file_icon.clone()
    }
}
