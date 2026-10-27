//! Headless folder picker used when Made starts without a workspace.
//!
//! The picker is a model, not a widget: it exposes the start directory as a
//! tree root, flattened visible rows, expansion state, selection, and refresh.
//! The UI renders it and calls [`WorkspacePicker::confirm_current`] to produce a
//! validated [`LaunchPlan`].
//!
//! No workspace surfaces are created while the picker is visible. Reading a
//! directory failure keeps the picker usable, leaves the failed row collapsed,
//! and stores a non-blocking error instead of panicking.

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::workspace::{LaunchPlan, WorkspaceError, WorkspaceRoot, WorkspaceSource};

/// One root child directory.
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

/// One visible flattened tree row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    path: PathBuf,
    name: String,
    depth: usize,
    expanded: bool,
}

impl PickerRow {
    /// Canonical directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Display name (the final path component).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tree depth below the picker root; root children are depth 0.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Whether this folder's children are visible below the row.
    pub fn expanded(&self) -> bool {
        self.expanded
    }
}

/// Maximum directories retained for each directory read.
pub const MAX_PICKER_ENTRIES: usize = 1_000;

static NEXT_PICKER_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// A picker operation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerError {
    /// The directory could not be opened.
    Workspace(WorkspaceError),
    /// The requested index is outside the visible row list.
    InvalidSelection {
        /// Requested index.
        index: usize,
        /// Number of visible rows.
        len: usize,
    },
    /// The selected row is not a directory.
    NotADirectory {
        /// Selected path.
        path: PathBuf,
    },
    /// The current tree root has no parent to navigate to.
    AtRoot {
        /// Current root.
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
                    "picker row {index} is outside {len} visible rows"
                )
            }
            Self::NotADirectory { path } => {
                write!(formatter, "picker row is not a folder: {}", path.display())
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

/// Headless folder picker tree state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePicker {
    current_dir: PathBuf,
    entries: Vec<PickerEntry>,
    rows: Vec<PickerRow>,
    selection: Option<usize>,
    expanded: BTreeSet<PathBuf>,
    children: HashMap<PathBuf, Vec<PickerEntry>>,
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
    /// best-effort launch-time start directory. The start directory is the tree
    /// root and is not itself a selectable row.
    pub fn new(start: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = WorkspaceRoot::resolve(start, WorkspaceSource::Picker)?;
        let (entries, truncated) = read_directories(root.path())?;
        let mut picker = Self {
            current_dir: root.path().to_path_buf(),
            entries,
            rows: Vec::new(),
            selection: None,
            expanded: BTreeSet::new(),
            children: HashMap::new(),
            error: None,
            instance: NEXT_PICKER_INSTANCE.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            truncated,
        };
        picker.rebuild_rows(None);
        Ok(picker)
    }

    /// Open the picker at the process working directory.
    pub fn at_current_dir() -> Result<Self, WorkspaceError> {
        Self::new(
            std::env::current_dir()
                .map_err(|error| WorkspaceError::from_io(PathBuf::from("."), &error))?,
        )
    }

    /// Current canonical tree root.
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// Process-unique picker instance identity, used by UI caches.
    pub fn instance_id(&self) -> u64 {
        self.instance
    }

    /// Monotonic revision bumped by refresh and tree expansion/collapse.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Number of root directories omitted by the list cap.
    pub fn truncated_entries(&self) -> usize {
        self.truncated
    }

    /// Root child directories, sorted case-insensitively.
    ///
    /// Root children are depth-0 rows in [`Self::rows`]; this accessor remains
    /// for callers that need the root directory itself.
    pub fn entries(&self) -> &[PickerEntry] {
        &self.entries
    }

    /// Visible flattened tree rows.
    pub fn rows(&self) -> &[PickerRow] {
        &self.rows
    }

    /// Selected visible row index.
    pub fn selection(&self) -> Option<usize> {
        self.selection
    }

    /// Selected visible row.
    pub fn selected_row(&self) -> Option<&PickerRow> {
        self.selection.and_then(|index| self.rows.get(index))
    }

    /// Selected row path, or the tree root when no row is selected.
    pub fn path_target(&self) -> &Path {
        self.selected_row()
            .map(PickerRow::path)
            .unwrap_or(&self.current_dir)
    }

    /// Selected row path, when one exists.
    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_row().map(PickerRow::path)
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

    /// Re-read the root and every expanded directory.
    ///
    /// Expansion and selection are preserved by canonical path when still
    /// valid; removed/unreadable expanded directories are pruned. On a root
    /// failure the current tree is kept and the error is stored.
    pub fn refresh(&mut self) -> Result<(), WorkspaceError> {
        self.revision = self.revision.saturating_add(1);
        let selected = self.selected_path().map(Path::to_path_buf);
        let desired = self.expanded.clone();
        let (entries, truncated) = match read_directories(&self.current_dir) {
            Ok(read) => read,
            Err(error) => {
                self.error = Some(PickerError::Workspace(error.clone()));
                return Err(error);
            }
        };
        self.entries = entries;
        self.truncated = truncated;
        self.children.clear();
        self.expanded.clear();
        self.error = None;
        let root_entries = self.entries.clone();
        let mut valid = BTreeSet::new();
        let mut visited = BTreeSet::new();
        self.reload_expanded(&root_entries, &desired, &mut valid, &mut visited);
        self.expanded = valid;
        self.rebuild_rows(selected.as_deref());
        Ok(())
    }

    /// Select a visible row.
    pub fn select(&mut self, index: usize) -> Result<(), PickerError> {
        if index >= self.rows.len() {
            let error = PickerError::InvalidSelection {
                index,
                len: self.rows.len(),
            };
            self.error = Some(error.clone());
            return Err(error);
        }
        self.selection = Some(index);
        self.error = None;
        Ok(())
    }

    /// Move the selection by `delta` visible rows, wrapping at the ends.
    ///
    /// From no selection, Down chooses the first row and Up chooses the last.
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.selection = None;
            return;
        }
        let len = self.rows.len() as isize;
        let next = match self.selection {
            Some(current) => (current as isize).saturating_add(delta).rem_euclid(len),
            None if delta > 0 => 0,
            None => len - 1,
        };
        self.selection = Some(next as usize);
        self.error = None;
    }

    /// Move the selection to the next visible row.
    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    /// Move the selection to the previous visible row.
    pub fn select_previous(&mut self) {
        self.move_selection(-1);
    }

    /// Toggle expansion of the selected folder.
    pub fn toggle_selected(&mut self) -> Result<(), PickerError> {
        match self.selection {
            Some(index) => self.toggle(index),
            None => Ok(()),
        }
    }

    /// Toggle expansion of the folder at `index`.
    ///
    /// The row is selected first. Collapsing removes only that subtree; expanding
    /// reads the children synchronously (capped per directory). An unreadable
    /// directory keeps the row collapsed and stores the error.
    pub fn toggle(&mut self, index: usize) -> Result<(), PickerError> {
        let row = self
            .rows
            .get(index)
            .cloned()
            .ok_or(PickerError::InvalidSelection {
                index,
                len: self.rows.len(),
            })?;
        self.selection = Some(index);
        if row.expanded {
            self.expanded.remove(&row.path);
            self.rebuild_rows(Some(&row.path));
            self.error = None;
            return Ok(());
        }
        let entries = match read_directories(&row.path) {
            Ok((entries, _)) => entries,
            Err(error) => {
                let error = PickerError::Workspace(error);
                self.error = Some(error.clone());
                return Err(error);
            }
        };
        self.children.insert(row.path.clone(), entries);
        self.expanded.insert(row.path.clone());
        self.rebuild_rows(Some(&row.path));
        self.error = None;
        Ok(())
    }

    /// Expand the selected folder, or select its first child when already open.
    pub fn expand_selected(&mut self) -> Result<(), PickerError> {
        let Some(index) = self.selection else {
            return Ok(());
        };
        let row = self.rows[index].clone();
        if !row.expanded {
            return self.toggle(index);
        }
        if self
            .rows
            .get(index + 1)
            .is_some_and(|child| child.depth == row.depth + 1)
        {
            self.selection = Some(index + 1);
            self.error = None;
        }
        Ok(())
    }

    /// Collapse the selected folder, or select its parent row when collapsed.
    ///
    /// Depth-0 rows have no parent row, so Left on a collapsed root child is a
    /// no-op apart from keeping the row selected.
    pub fn collapse_selected(&mut self) -> Result<(), PickerError> {
        let Some(index) = self.selection else {
            return Ok(());
        };
        let row = self.rows[index].clone();
        if row.expanded {
            self.expanded.remove(&row.path);
            self.rebuild_rows(Some(&row.path));
            self.error = None;
            return Ok(());
        }
        if row.depth > 0 {
            if let Some(parent) = self.rows[..index]
                .iter()
                .rposition(|candidate| candidate.depth + 1 == row.depth)
            {
                self.selection = Some(parent);
                self.error = None;
            }
        }
        Ok(())
    }

    /// Re-root the picker to the current root's parent directory.
    ///
    /// The new tree starts at the top with no expansion or selection.
    pub fn go_up(&mut self) -> Result<(), PickerError> {
        let parent = self.current_dir.parent().map(Path::to_path_buf);
        let Some(parent) = parent else {
            let error = PickerError::AtRoot {
                path: self.current_dir.clone(),
            };
            self.error = Some(error.clone());
            return Err(error);
        };
        match Self::new(parent) {
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

    /// Confirm the selected row path, or the tree root when nothing is selected.
    pub fn confirm_current(&self) -> Result<LaunchPlan, WorkspaceError> {
        LaunchPlan::open(self.path_target(), WorkspaceSource::Picker)
    }

    /// Confirm the visible row at `index` as the workspace.
    pub fn confirm_row(&self, index: usize) -> Result<LaunchPlan, PickerError> {
        let row = self.rows.get(index).ok_or(PickerError::InvalidSelection {
            index,
            len: self.rows.len(),
        })?;
        if !row.path.is_dir() {
            return Err(PickerError::NotADirectory {
                path: row.path.clone(),
            });
        }
        LaunchPlan::open(&row.path, WorkspaceSource::Picker).map_err(PickerError::Workspace)
    }

    fn rebuild_rows(&mut self, preserve: Option<&Path>) {
        self.revision = self.revision.saturating_add(1);
        let mut rows = Vec::new();
        let mut visited = BTreeSet::new();
        collect_rows(
            &self.entries,
            0,
            &self.expanded,
            &self.children,
            &mut visited,
            &mut rows,
        );
        self.rows = rows;
        self.selection =
            preserve.and_then(|path| self.rows.iter().position(|row| row.path == path));
    }

    fn reload_expanded(
        &mut self,
        entries: &[PickerEntry],
        desired: &BTreeSet<PathBuf>,
        valid: &mut BTreeSet<PathBuf>,
        visited: &mut BTreeSet<PathBuf>,
    ) {
        for entry in entries {
            if !desired.contains(&entry.path) || !visited.insert(entry.path.clone()) {
                continue;
            }
            match read_directories(&entry.path) {
                Ok((children, _)) => {
                    self.children.insert(entry.path.clone(), children.clone());
                    valid.insert(entry.path.clone());
                    self.reload_expanded(&children, desired, valid, visited);
                }
                Err(error) => {
                    self.error = Some(PickerError::Workspace(error));
                }
            }
        }
    }
}

fn collect_rows(
    entries: &[PickerEntry],
    depth: usize,
    expanded: &BTreeSet<PathBuf>,
    children: &HashMap<PathBuf, Vec<PickerEntry>>,
    visited: &mut BTreeSet<PathBuf>,
    rows: &mut Vec<PickerRow>,
) {
    for entry in entries {
        let cycle = !visited.insert(entry.path.clone());
        let is_expanded = !cycle && expanded.contains(&entry.path);
        rows.push(PickerRow {
            path: entry.path.clone(),
            name: entry.name.clone(),
            depth,
            expanded: is_expanded,
        });
        if is_expanded {
            if let Some(entries) = children.get(&entry.path) {
                collect_rows(entries, depth + 1, expanded, children, visited, rows);
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
