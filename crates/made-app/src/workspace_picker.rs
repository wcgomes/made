//! Headless folder picker used when Made starts without a workspace.
//!
//! The picker is a model, not a widget: it exposes the current directory, the
//! sorted child directories, selection, and navigation. The UI renders it and
//! calls [`WorkspacePicker::confirm_current`] to produce a validated
//! [`LaunchPlan`].
//!
//! No workspace surfaces are created while the picker is visible. Reading a
//! directory failure keeps the picker usable, and an invalid selection returns
//! an error instead of panicking.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::workspace::{LaunchPlan, WorkspaceError, WorkspaceRoot, WorkspaceSource};

/// One directory displayed in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    path: PathBuf,
    name: String,
}

impl PickerEntry {
    /// Canonical directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Display name (the final path component).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Maximum directories retained in the picker list.
pub const MAX_PICKER_ENTRIES: usize = 1_000;

static NEXT_PICKER_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// A picker operation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerError {
    /// The directory could not be opened.
    Workspace(WorkspaceError),
    /// The requested index is outside the visible entry list.
    InvalidSelection {
        /// Requested index.
        index: usize,
        /// Number of visible entries.
        len: usize,
    },
    /// The selected entry is not a directory.
    NotADirectory {
        /// Selected path.
        path: PathBuf,
    },
    /// The current directory has no parent to navigate to.
    AtRoot {
        /// Current directory.
        path: PathBuf,
    },
}

impl fmt::Display for PickerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(formatter, "{error}"),
            Self::InvalidSelection { index, len } => {
                write!(
                    formatter,
                    "picker selection {index} is outside {len} entries"
                )
            }
            Self::NotADirectory { path } => {
                write!(
                    formatter,
                    "picker entry is not a folder: {}",
                    path.display()
                )
            }
            Self::AtRoot { path } => {
                write!(formatter, "no parent directory above {}", path.display())
            }
        }
    }
}

impl Error for PickerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            _ => None,
        }
    }
}

impl From<WorkspaceError> for PickerError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

/// Headless folder picker state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePicker {
    current_dir: PathBuf,
    entries: Vec<PickerEntry>,
    selection: Option<usize>,
    error: Option<PickerError>,
    instance: u64,
    revision: u64,
    truncated: usize,
}

impl WorkspacePicker {
    /// Open the picker at `start`.
    ///
    /// The starting directory must exist and be readable; use
    /// [`crate::workspace::WorkspaceLaunch::waiting_for_picker`] for a
    /// best-effort launch-time start directory.
    pub fn new(start: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = WorkspaceRoot::resolve(start, WorkspaceSource::Picker)?;
        let mut picker = Self {
            current_dir: root.path().to_path_buf(),
            entries: Vec::new(),
            selection: None,
            error: None,
            instance: NEXT_PICKER_INSTANCE.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            truncated: 0,
        };
        picker.refresh()?;
        Ok(picker)
    }

    /// Open the picker at the process working directory.
    pub fn at_current_dir() -> Result<Self, WorkspaceError> {
        Self::new(
            std::env::current_dir()
                .map_err(|error| WorkspaceError::from_io(PathBuf::from("."), &error))?,
        )
    }

    /// Current canonical directory.
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// Process-unique picker instance identity, used by UI caches.
    pub fn instance_id(&self) -> u64 {
        self.instance
    }

    /// Monotonic revision bumped by refresh and directory navigation.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Number of directories omitted by the list cap.
    pub fn truncated_entries(&self) -> usize {
        self.truncated
    }

    /// Visible child directories, sorted case-insensitively.
    pub fn entries(&self) -> &[PickerEntry] {
        &self.entries
    }

    /// Selected entry index.
    pub fn selection(&self) -> Option<usize> {
        self.selection
    }

    /// Selected entry.
    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.selection.and_then(|index| self.entries.get(index))
    }

    /// Last picker error.
    pub fn error(&self) -> Option<&PickerError> {
        self.error.as_ref()
    }

    /// Store an externally observed error for display.
    pub fn set_error(&mut self, error: PickerError) {
        self.error = Some(error);
    }

    /// Clear the visible error.
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Re-read the current directory.
    ///
    /// The previous selection is preserved by name when possible. On failure
    /// the current directory and entries are kept and the error is stored.
    pub fn refresh(&mut self) -> Result<(), WorkspaceError> {
        self.revision = self.revision.saturating_add(1);
        let previous = self.selected_entry().map(|entry| entry.name.clone());
        match read_directories(&self.current_dir) {
            Ok((entries, truncated)) => {
                self.entries = entries;
                self.truncated = truncated;
                self.selection = previous
                    .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
                    .or(if self.entries.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some(PickerError::Workspace(error.clone()));
                Err(error)
            }
        }
    }

    /// Select an index.
    pub fn select(&mut self, index: usize) -> Result<(), PickerError> {
        if index >= self.entries.len() {
            let error = PickerError::InvalidSelection {
                index,
                len: self.entries.len(),
            };
            self.error = Some(error.clone());
            return Err(error);
        }
        self.selection = Some(index);
        self.error = None;
        Ok(())
    }

    /// Move the selection by `delta`, wrapping at the ends.
    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.selection = None;
            return;
        }
        let len = self.entries.len() as isize;
        let current = self.selection.unwrap_or(0) as isize;
        let next = current.saturating_add(delta).rem_euclid(len);
        self.selection = Some(next as usize);
        self.error = None;
    }

    /// Move the selection to the next entry.
    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    /// Move the selection to the previous entry.
    pub fn select_previous(&mut self) {
        self.move_selection(-1);
    }

    /// Enter the selected directory.
    pub fn enter_selected(&mut self) -> Result<(), PickerError> {
        let index = self.selection.ok_or(PickerError::InvalidSelection {
            index: 0,
            len: self.entries.len(),
        })?;
        self.enter(index)
    }

    /// Enter the directory at `index`.
    pub fn enter(&mut self, index: usize) -> Result<(), PickerError> {
        let entry = self
            .entries
            .get(index)
            .cloned()
            .ok_or(PickerError::InvalidSelection {
                index,
                len: self.entries.len(),
            })?;
        if !entry.path.is_dir() {
            let error = PickerError::NotADirectory { path: entry.path };
            self.error = Some(error.clone());
            return Err(error);
        }
        self.navigate_to(&entry.path)
    }

    /// Navigate to the parent directory.
    pub fn go_up(&mut self) -> Result<(), PickerError> {
        let parent = self.current_dir.parent().map(Path::to_path_buf);
        let Some(parent) = parent else {
            let error = PickerError::AtRoot {
                path: self.current_dir.clone(),
            };
            self.error = Some(error.clone());
            return Err(error);
        };
        self.navigate_to(&parent)
    }

    /// Confirm the current directory as the workspace.
    pub fn confirm_current(&self) -> Result<LaunchPlan, WorkspaceError> {
        LaunchPlan::open(&self.current_dir, WorkspaceSource::Picker)
    }

    /// Confirm the directory at `index` as the workspace.
    pub fn confirm_entry(&self, index: usize) -> Result<LaunchPlan, PickerError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(PickerError::InvalidSelection {
                index,
                len: self.entries.len(),
            })?;
        if !entry.path.is_dir() {
            return Err(PickerError::NotADirectory {
                path: entry.path.clone(),
            });
        }
        LaunchPlan::open(&entry.path, WorkspaceSource::Picker).map_err(PickerError::Workspace)
    }

    fn navigate_to(&mut self, path: &Path) -> Result<(), PickerError> {
        match Self::new(path) {
            Ok(next) => {
                *self = next;
                Ok(())
            }
            Err(error) => {
                let error = PickerError::Workspace(error);
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
}

fn read_directories(dir: &Path) -> Result<(Vec<PickerEntry>, usize), WorkspaceError> {
    let read_dir = fs::read_dir(dir).map_err(|error| WorkspaceError::from_io(dir, &error))?;
    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|error| WorkspaceError::from_io(dir, &error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| WorkspaceError::from_io(entry.path(), &error))?;
        let is_directory = if file_type.is_dir() {
            true
        } else if file_type.is_symlink() {
            fs::metadata(entry.path())
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        } else {
            false
        };
        if !is_directory {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let path = fs::canonicalize(entry.path())
            .map_err(|error| WorkspaceError::from_io(entry.path(), &error))?;
        entries.push(PickerEntry { path, name });
    }

    entries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    let total = entries.len();
    if total > MAX_PICKER_ENTRIES {
        entries.truncate(MAX_PICKER_ENTRIES);
    }
    Ok((entries, total.saturating_sub(MAX_PICKER_ENTRIES)))
}
