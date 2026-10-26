//! Lazy tree model with error rows and expansion/selection state (Phase 2,
//! tasks 2.1-2.2).
//!
//! Directories load on demand, entries sort directories-first with a stable
//! case-insensitive order, and unreadable directories render as error rows
//! instead of failing the tree.
//!
//! The model is only a value: it never spawns threads and never touches UI
//! types. `load_dir` can be called from a background worker; `apply_load` and
//! the watcher-event methods must run on the thread that owns the model. Every
//! listing carries a [`RootGeneration`]; a load from an older root is rejected
//! by both [`TreeModel::apply_load`] and [`TreeModel::apply_watch_events`].

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::{PathError, PathGuard};
use crate::root_switch::{RootGeneration, StaleGeneration};
use crate::watch::{WatchEvent, WatchEventKind};

/// Maximum number of real directory entries read for one listing.
///
/// When a directory contains more entries, the listing keeps this many entries
/// plus one explicit truncation row. The cap prevents a single huge directory
/// from allocating or sorting an unbounded number of rows.
pub const MAX_DIR_ENTRIES: usize = 10_000;

/// Maximum number of rows returned by [`TreeModel::rows`].
///
/// Expanded state may be deeper, but flattening stops at this bound and emits a
/// truncation row so rendering cannot grow without limit.
pub const MAX_TREE_ROWS: usize = 50_000;

/// Maximum recursion depth used when flattening expanded directories.
///
/// A directory row at this depth is shown with a truncation row instead of
/// expanding into deeper rows.
pub const MAX_TREE_DEPTH: usize = 64;

const TRUNCATED_ROW_NAME: &str = "(listing truncated)";
const TREE_LIMIT_ROW_NAME: &str = "(tree truncated)";

/// Kind of an entry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A readable directory. Expanding it triggers a lazy load.
    Directory,
    /// A regular file.
    File,
    /// A symbolic link, shown as reported and never followed by the tree.
    Symlink,
    /// A special filesystem entry (socket, FIFO, device).
    Other,
    /// A row representing an unreadable entry or directory.
    Error,
}

impl EntryKind {
    /// Returns `true` for a directory row that can be expanded.
    #[must_use]
    pub const fn is_directory(self) -> bool {
        matches!(self, Self::Directory)
    }
}

/// One row in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// File name for display. Non-UTF-8 names use a lossy rendering; `path` is
    /// always the exact filesystem path.
    pub name: String,
    /// Exact path as reported by the directory entry.
    pub path: PathBuf,
    /// Entry type.
    pub kind: EntryKind,
    /// Safe error text for [`EntryKind::Error`] rows.
    pub error: Option<String>,
}

impl TreeEntry {
    /// Returns `true` for an error row.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.kind, EntryKind::Error)
    }

    /// Returns `true` for an expandable directory row.
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        self.kind.is_directory()
    }
}

/// Result of reading one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirListing {
    /// Canonical directory path.
    pub dir: PathBuf,
    /// Root generation active when the listing was read.
    pub generation: RootGeneration,
    /// Sorted entries. A directory-level read failure yields exactly one error
    /// row here and also sets [`DirListing::error`]. When the directory exceeds
    /// [`MAX_DIR_ENTRIES`], the final row is a truncation error row.
    pub entries: Vec<TreeEntry>,
    /// Directory-level error when the whole directory could not be read.
    pub error: Option<String>,
    /// `true` when the listing hit [`MAX_DIR_ENTRIES`] and omitted entries.
    pub truncated: bool,
}

impl DirListing {
    /// Reads exactly one directory.
    ///
    /// Only the requested directory is scanned; child directories are not
    /// visited. Guard failures (outside root, missing root) are returned as
    /// errors. A directory that exists but cannot be read becomes a successful
    /// listing with one error row, because the tree must keep rendering. At most
    /// [`MAX_DIR_ENTRIES`] real entries are read; a larger directory gets a
    /// final truncation row.
    pub fn load(
        guard: &PathGuard,
        dir: impl AsRef<Path>,
        generation: RootGeneration,
    ) -> Result<Self, PathError> {
        Self::load_with_limit(guard, dir, generation, MAX_DIR_ENTRIES)
    }

    /// Reads one directory with an explicit entry cap.
    ///
    /// This is separated from [`DirListing::load`] so tests can exercise the
    /// truncation path with a small fixture instead of creating thousands of
    /// files. Production callers use [`MAX_DIR_ENTRIES`].
    fn load_with_limit(
        guard: &PathGuard,
        dir: impl AsRef<Path>,
        generation: RootGeneration,
        max_entries: usize,
    ) -> Result<Self, PathError> {
        let dir = guard.resolve_existing(dir)?;

        let read_dir = match fs::read_dir(&dir) {
            Ok(read_dir) => read_dir,
            Err(error) => {
                return Ok(Self::error_listing(
                    dir,
                    generation,
                    format!("cannot read directory: {error}"),
                ));
            }
        };

        let mut entries = Vec::new();
        let mut truncated = false;
        for item in read_dir {
            // We only need one item past the cap to know the listing is
            // truncated; stop rather than reading the rest of the directory.
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            match item {
                Ok(entry) => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let path = entry.path();
                    match entry.file_type() {
                        Ok(file_type) => {
                            let kind = if file_type.is_dir() {
                                EntryKind::Directory
                            } else if file_type.is_file() {
                                EntryKind::File
                            } else if file_type.is_symlink() {
                                EntryKind::Symlink
                            } else {
                                EntryKind::Other
                            };
                            entries.push(TreeEntry {
                                name,
                                path,
                                kind,
                                error: None,
                            });
                        }
                        Err(error) => entries.push(TreeEntry {
                            name,
                            path,
                            kind: EntryKind::Error,
                            error: Some(format!("cannot read entry metadata: {error}")),
                        }),
                    }
                }
                Err(error) => entries.push(TreeEntry {
                    name: "(unreadable entry)".to_string(),
                    path: dir.clone(),
                    kind: EntryKind::Error,
                    error: Some(format!("cannot read directory entry: {error}")),
                }),
            }
        }

        sort_entries(&mut entries);
        if truncated {
            entries.push(TreeEntry {
                name: TRUNCATED_ROW_NAME.to_string(),
                path: dir.clone(),
                kind: EntryKind::Error,
                error: Some(format!(
                    "Directory has more than {max_entries} entries; only the first {max_entries} are shown."
                )),
            });
        }
        Ok(Self {
            dir,
            generation,
            entries,
            error: None,
            truncated,
        })
    }

    /// Builds a listing containing one safe error row.
    #[must_use]
    pub fn error_listing(
        dir: impl Into<PathBuf>,
        generation: RootGeneration,
        message: impl Into<String>,
    ) -> Self {
        let dir = dir.into();
        let message = message.into();
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());
        Self {
            entries: vec![TreeEntry {
                name,
                path: dir.clone(),
                kind: EntryKind::Error,
                error: Some(message.clone()),
            }],
            dir,
            generation,
            error: Some(message),
            truncated: false,
        }
    }

    /// Returns `true` when the directory itself could not be read.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// Returns `true` when the listing hit [`MAX_DIR_ENTRIES`] and omitted
    /// entries.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Tree operation failure.
#[derive(Debug)]
pub enum TreeError {
    /// Path guard rejection.
    Path(PathError),
    /// A file was requested where a directory load/expansion was required.
    NotDirectory {
        /// File path.
        path: PathBuf,
    },
    /// A load or event belongs to a previous root generation.
    StaleGeneration(StaleGeneration),
    /// A root switch attempted to reuse or move back a generation.
    NonMonotonicGeneration {
        /// Current generation.
        current: RootGeneration,
        /// Requested generation.
        requested: RootGeneration,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => write!(f, "{error}"),
            Self::NotDirectory { path } => {
                write!(f, "path is not a directory: {}", path.display())
            }
            Self::StaleGeneration(error) => write!(f, "{error}"),
            Self::NonMonotonicGeneration { current, requested } => write!(
                f,
                "root generation {requested} is not newer than current generation {current}"
            ),
        }
    }
}

impl Error for TreeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::StaleGeneration(error) => Some(error),
            Self::NotDirectory { .. } | Self::NonMonotonicGeneration { .. } => None,
        }
    }
}

impl From<PathError> for TreeError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

/// A directory load prepared for a background worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirRequest {
    dir: PathBuf,
    generation: RootGeneration,
}

impl DirRequest {
    /// Returns the canonical directory to read.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the generation the result must carry.
    #[must_use]
    pub const fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Reads the requested directory. Safe to call on a background thread.
    pub fn load(&self, guard: &PathGuard) -> Result<DirListing, PathError> {
        DirListing::load(guard, &self.dir, self.generation)
    }
}

/// Changes produced by applying watcher events without performing I/O.
///
/// `reload_dirs` are loaded directories whose entries changed, plus expanded
/// directories remapped by a rename. The app can load them on a background
/// worker and feed the results back through [`TreeModel::apply_load`]. `removed`
/// lists paths whose cached subtrees were discarded. Selection is cleared if the
/// selected path disappeared, and rename events remap selection/expansion when
/// possible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeUpdate {
    /// Loaded directories that should be re-read.
    pub reload_dirs: Vec<PathBuf>,
    /// Paths removed from the cached tree.
    pub removed: Vec<PathBuf>,
    /// `true` when a deleted/renamed path cleared the selection.
    pub selection_changed: bool,
}

impl TreeUpdate {
    /// Returns `true` when no cached state changed and no reload is needed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reload_dirs.is_empty() && self.removed.is_empty() && !self.selection_changed
    }
}

/// One flattened, visible row for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Exact path of the row.
    pub path: PathBuf,
    /// Display name.
    pub name: String,
    /// Nesting depth below the root; root children have depth 0.
    pub depth: usize,
    /// Entry kind.
    pub kind: EntryKind,
    /// Whether this directory row is expanded.
    pub expanded: bool,
    /// Whether this row is selected.
    pub selected: bool,
    /// Safe error text for error rows.
    pub error: Option<String>,
}

/// Lazy tree model for one active root and generation.
#[derive(Debug, Clone)]
pub struct TreeModel {
    root: PathBuf,
    generation: RootGeneration,
    guard: PathGuard,
    listings: BTreeMap<PathBuf, DirListing>,
    expanded: BTreeSet<PathBuf>,
    selection: Option<PathBuf>,
    loading: BTreeSet<PathBuf>,
}

impl TreeModel {
    /// Creates an unloaded tree rooted at `root`.
    ///
    /// No directory is read here. The first read happens through
    /// [`TreeModel::expand`] or [`TreeModel::load_dir`], which keeps startup
    /// independent of workspace size.
    pub fn new(root: impl AsRef<Path>, generation: RootGeneration) -> Result<Self, PathError> {
        let guard = PathGuard::new(root)?;
        Ok(Self::from_guard(guard, generation))
    }

    /// Creates a tree from an existing guard. The guard's root is used.
    #[must_use]
    pub fn from_guard(guard: PathGuard, generation: RootGeneration) -> Self {
        Self {
            root: guard.root().to_path_buf(),
            generation,
            guard,
            listings: BTreeMap::new(),
            expanded: BTreeSet::new(),
            selection: None,
            loading: BTreeSet::new(),
        }
    }

    /// Returns the canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the active root generation.
    #[must_use]
    pub const fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Returns the guard used for loads and selections.
    #[must_use]
    pub fn path_guard(&self) -> &PathGuard {
        &self.guard
    }

    /// Switches to a new root and generation, discarding all cached state.
    ///
    /// The new generation must be strictly greater than the current one so a
    /// stale load/event can never become current again.
    pub fn switch_root(
        &mut self,
        root: impl AsRef<Path>,
        generation: RootGeneration,
    ) -> Result<(), TreeError> {
        if !self.generation.is_stale(generation) {
            return Err(TreeError::NonMonotonicGeneration {
                current: self.generation,
                requested: generation,
            });
        }
        let guard = PathGuard::new(root)?;
        self.root = guard.root().to_path_buf();
        self.generation = generation;
        self.guard = guard;
        self.listings.clear();
        self.expanded.clear();
        self.selection = None;
        self.loading.clear();
        Ok(())
    }

    /// Returns `true` when `dir` has a cached listing, including error listings.
    #[must_use]
    pub fn is_loaded(&self, dir: impl AsRef<Path>) -> bool {
        self.listings.contains_key(dir.as_ref())
    }

    /// Returns the cached listing for `dir`, if any.
    #[must_use]
    pub fn listing(&self, dir: impl AsRef<Path>) -> Option<&DirListing> {
        self.listings.get(dir.as_ref())
    }

    /// Returns the cached entries for `dir`, if the directory was loaded.
    #[must_use]
    pub fn children(&self, dir: impl AsRef<Path>) -> Option<&[TreeEntry]> {
        self.listings
            .get(dir.as_ref())
            .map(|listing| listing.entries.as_slice())
    }

    /// Returns the directories currently being loaded in the background.
    pub fn loading_dirs(&self) -> impl Iterator<Item = &Path> {
        self.loading.iter().map(PathBuf::as_path)
    }

    /// Returns `true` when `dir` is marked as loading.
    #[must_use]
    pub fn is_loading(&self, dir: impl AsRef<Path>) -> bool {
        self.loading.contains(dir.as_ref())
    }

    /// Prepares a background load for `dir`.
    ///
    /// This resolves and guards the path, including a missing directory's
    /// canonical ancestor, but performs no read. Use [`DirRequest::load`] on a
    /// worker and [`TreeModel::finish_load`] with the result. A worker that gets
    /// a missing/unreadable error can build [`DirListing::error_listing`] and
    /// finish the request so the tree still renders an error row.
    pub fn begin_load(&mut self, dir: impl AsRef<Path>) -> Result<DirRequest, TreeError> {
        let dir = self.guard.resolve_for_create(dir.as_ref())?;
        self.loading.insert(dir.clone());
        Ok(DirRequest {
            dir,
            generation: self.generation,
        })
    }

    /// Applies the result of a [`TreeModel::begin_load`] request.
    pub fn finish_load(
        &mut self,
        request: &DirRequest,
        listing: DirListing,
    ) -> Result<(), TreeError> {
        self.ensure_generation(request.generation)?;
        if listing.dir != request.dir {
            return Err(TreeError::Path(PathError::OutsideRoot {
                path: listing.dir,
            }));
        }
        self.loading.remove(&request.dir);
        self.apply_load(listing)
    }

    /// Applies an already loaded directory listing.
    ///
    /// Cached expansion and selection are untouched. A listing from another
    /// generation is rejected wholesale.
    pub fn apply_load(&mut self, listing: DirListing) -> Result<(), TreeError> {
        self.ensure_generation(listing.generation)?;
        if !self.guard.contains(&listing.dir) {
            return Err(TreeError::Path(PathError::OutsideRoot {
                path: listing.dir,
            }));
        }
        self.loading.remove(&listing.dir);
        let dir = listing.dir.clone();
        let selection_present = self
            .selection
            .as_deref()
            .is_none_or(|selected| selected.parent() != Some(dir.as_path()))
            || listing
                .entries
                .iter()
                .any(|entry| self.selection.as_deref() == Some(entry.path.as_path()));
        self.listings.insert(dir, listing);
        if !selection_present {
            self.selection = None;
        }
        Ok(())
    }

    /// Loads one directory synchronously through the guard.
    ///
    /// Read and resolution failures for missing or unreadable directories become
    /// error listings rather than failing the whole tree; an outside-root path is
    /// still an error.
    pub fn load_dir(&mut self, dir: impl AsRef<Path>) -> Result<(), TreeError> {
        let requested = dir.as_ref();
        let dir = match self.guard.resolve_for_create(requested) {
            Ok(dir) => dir,
            Err(PathError::NotFound { path }) => {
                self.insert_error_listing(
                    path,
                    "directory is missing or no longer available".to_string(),
                );
                return Ok(());
            }
            Err(PathError::Unreadable { path, source }) => {
                self.insert_error_listing(path, format!("directory is unreadable: {source}"));
                return Ok(());
            }
            Err(error) => return Err(TreeError::Path(error)),
        };
        self.loading.remove(&dir);

        match DirListing::load(&self.guard, &dir, self.generation) {
            Ok(listing) => self.apply_load(listing),
            Err(PathError::NotFound { .. }) => {
                self.insert_error_listing(
                    dir,
                    "directory is missing or no longer available".to_string(),
                );
                Ok(())
            }
            Err(PathError::Unreadable { source, .. }) => {
                self.insert_error_listing(dir, format!("directory is unreadable: {source}"));
                Ok(())
            }
            Err(error) => Err(TreeError::Path(error)),
        }
    }

    /// Expands a directory and lazily loads it if needed.
    pub fn expand(&mut self, dir: impl AsRef<Path>) -> Result<(), TreeError> {
        let requested = dir.as_ref();
        let dir = match self.resolve_directory(requested) {
            Ok(dir) => dir,
            Err(TreeError::Path(PathError::NotFound { .. }))
            | Err(TreeError::Path(PathError::Unreadable { .. })) => {
                self.load_dir(requested)?;
                if let Ok(key) = self.guard.resolve_for_create(requested) {
                    self.expanded.insert(key);
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.expanded.insert(dir.clone());
        if !self.listings.contains_key(&dir) {
            self.load_dir(&dir)?;
        }
        Ok(())
    }

    /// Expands and lazily loads the active root.
    pub fn expand_root(&mut self) -> Result<(), TreeError> {
        let root = self.root.clone();
        self.expand(root)
    }

    /// Collapses a directory without discarding its cached listing.
    pub fn collapse(&mut self, dir: impl AsRef<Path>) {
        self.expanded.remove(dir.as_ref());
    }

    /// Toggles expansion, loading the directory when expanding.
    ///
    /// Returns `true` when the directory is now expanded.
    pub fn toggle(&mut self, dir: impl AsRef<Path>) -> Result<bool, TreeError> {
        let dir = self.resolve_directory(dir.as_ref())?;
        if self.expanded.contains(&dir) {
            self.expanded.remove(&dir);
            Ok(false)
        } else {
            self.expanded.insert(dir.clone());
            if !self.listings.contains_key(&dir) {
                self.load_dir(&dir)?;
            }
            Ok(true)
        }
    }

    /// Returns `true` when `dir` is expanded.
    #[must_use]
    pub fn is_expanded(&self, dir: impl AsRef<Path>) -> bool {
        self.expanded.contains(dir.as_ref())
    }

    /// Iterates expanded directories.
    pub fn expanded_dirs(&self) -> impl Iterator<Item = &Path> {
        self.expanded.iter().map(PathBuf::as_path)
    }

    /// Selects a path after validating it against the guard.
    ///
    /// Missing paths are allowed through [`PathGuard::resolve_for_create`] so a
    /// stale row can still open the preview's missing-file message. Paths outside
    /// the root are rejected.
    pub fn select(&mut self, path: impl AsRef<Path>) -> Result<(), TreeError> {
        let path = self.guard.resolve_for_create(path)?;
        self.selection = Some(path);
        Ok(())
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Returns the selected path, if any.
    #[must_use]
    pub fn selection(&self) -> Option<&Path> {
        self.selection.as_deref()
    }

    /// Returns the selected entry when its parent listing is cached.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&TreeEntry> {
        let selected = self.selection.as_ref()?;
        self.listings
            .values()
            .flat_map(|listing| listing.entries.iter())
            .find(|entry| &entry.path == selected)
    }

    /// Flattens the expanded portion of the tree into visible rows.
    ///
    /// The root itself is not a row; its children have depth 0. Returns an empty
    /// vector while the root is collapsed or unloaded. Flattening stops at
    /// [`MAX_TREE_ROWS`] rows and [`MAX_TREE_DEPTH`] levels; a final
    /// [`EntryKind::Error`] row marks truncation.
    #[must_use]
    pub fn rows(&self) -> Vec<TreeRow> {
        self.rows_with_limits(MAX_TREE_ROWS, MAX_TREE_DEPTH)
    }

    fn rows_with_limits(&self, max_rows: usize, max_depth: usize) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        if self.expanded.contains(&self.root) {
            self.collect_rows(&self.root, 0, max_rows, max_depth, &mut rows);
        }
        rows
    }

    /// Applies debounced watcher events without reading directories.
    ///
    /// All events must belong to the current generation; a single stale event
    /// rejects the batch before any mutation. Paths outside the root are
    /// rejected. Cached expansion and selection survive unless a removed or
    /// renamed path is involved.
    pub fn apply_watch_events(&mut self, events: &[WatchEvent]) -> Result<TreeUpdate, TreeError> {
        if let Some(event) = events
            .iter()
            .find(|event| !event.is_current(self.generation))
        {
            return Err(TreeError::StaleGeneration(StaleGeneration::new(
                self.generation,
                event.generation,
            )));
        }

        let mut update = TreeUpdate::default();
        for event in events {
            self.ensure_event_path(&event.path)?;
            if let Some(old_path) = &event.old_path {
                self.ensure_event_path(old_path)?;
            }

            match event.kind {
                WatchEventKind::Created => {
                    self.remove_cached_subtree(&event.path);
                    self.note_parent(&event.path, &mut update);
                }
                WatchEventKind::Changed => {
                    self.note_parent(&event.path, &mut update);
                }
                WatchEventKind::Deleted => {
                    self.remove_subtree(&event.path, &mut update);
                    self.note_parent(&event.path, &mut update);
                }
                WatchEventKind::Renamed => {
                    if let Some(old_path) = event.old_path.as_deref() {
                        let old_was_expanded = self.expanded.contains(old_path);
                        self.remap_expanded(old_path, &event.path);
                        self.remap_selection(old_path, &event.path, &mut update);
                        self.remove_subtree(old_path, &mut update);
                        self.note_parent(old_path, &mut update);
                        if old_was_expanded {
                            update.reload_dirs.push(event.path.clone());
                        }
                    }
                    self.note_parent(&event.path, &mut update);
                }
                WatchEventKind::Rescan => {
                    // Events were dropped, so reload every directory whose
                    // contents are cached or expanded. The work is bounded by
                    // the user's expansion state and the tree row/depth caps.
                    update.reload_dirs.extend(self.listings.keys().cloned());
                    update.reload_dirs.extend(self.expanded.iter().cloned());
                }
            }
        }

        update.reload_dirs.sort();
        update.reload_dirs.dedup();
        update
            .reload_dirs
            .retain(|dir| self.listings.contains_key(dir) || self.expanded.contains(dir));
        update.removed.sort();
        update.removed.dedup();
        Ok(update)
    }

    /// Convenience for tests and small roots: applies events and immediately
    /// reloads affected cached directories on the calling thread.
    ///
    /// UI code should use [`TreeModel::apply_watch_events`] and reload the
    /// returned directories on a background worker to keep the event loop free.
    pub fn apply_watch_events_and_reload(
        &mut self,
        events: &[WatchEvent],
    ) -> Result<TreeUpdate, TreeError> {
        let update = self.apply_watch_events(events)?;
        for dir in update.reload_dirs.clone() {
            self.load_dir(dir)?;
        }
        Ok(update)
    }

    fn ensure_generation(&self, generation: RootGeneration) -> Result<(), TreeError> {
        if generation.is_current(self.generation) {
            Ok(())
        } else {
            Err(TreeError::StaleGeneration(StaleGeneration::new(
                self.generation,
                generation,
            )))
        }
    }

    fn insert_error_listing(&mut self, path: PathBuf, message: String) {
        if self
            .selection
            .as_deref()
            .is_some_and(|selected| selected != path && selected.starts_with(&path))
        {
            self.selection = None;
        }
        self.loading.remove(&path);
        self.listings.insert(
            path.clone(),
            DirListing::error_listing(path, self.generation, message),
        );
    }

    fn ensure_event_path(&self, path: &Path) -> Result<(), TreeError> {
        if self.guard.contains(path) {
            Ok(())
        } else {
            Err(TreeError::Path(PathError::OutsideRoot {
                path: path.to_path_buf(),
            }))
        }
    }

    fn resolve_directory(&self, dir: &Path) -> Result<PathBuf, TreeError> {
        let canonical = self.guard.resolve_existing(dir)?;
        if !canonical.is_dir() {
            return Err(TreeError::NotDirectory { path: canonical });
        }
        Ok(canonical)
    }

    fn collect_rows(
        &self,
        dir: &Path,
        depth: usize,
        max_rows: usize,
        max_depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        let Some(listing) = self.listings.get(dir) else {
            return;
        };
        for entry in &listing.entries {
            if rows.len() >= max_rows {
                return;
            }
            // Leave room for a truncation marker when the row cap is reached.
            if rows.len() + 1 >= max_rows {
                push_limit_row(
                    rows,
                    max_rows,
                    dir.to_path_buf(),
                    depth,
                    format!("Tree is limited to {max_rows} visible rows."),
                );
                return;
            }

            let expanded = entry.kind.is_directory() && self.expanded.contains(&entry.path);
            rows.push(TreeRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                kind: entry.kind,
                expanded,
                selected: self.selection.as_deref() == Some(entry.path.as_path()),
                error: entry.error.clone(),
            });
            if expanded {
                if depth + 1 > max_depth {
                    push_limit_row(
                        rows,
                        max_rows,
                        entry.path.clone(),
                        depth,
                        format!("Tree expansion is limited to {max_depth} levels."),
                    );
                } else {
                    self.collect_rows(&entry.path, depth + 1, max_rows, max_depth, rows);
                }
            }
        }
    }

    fn note_parent(&self, path: &Path, update: &mut TreeUpdate) {
        let Some(parent) = path.parent() else {
            return;
        };
        if parent == path {
            return;
        }
        if self.listings.contains_key(parent) {
            update.reload_dirs.push(parent.to_path_buf());
        }
    }

    fn remove_subtree(&mut self, path: &Path, update: &mut TreeUpdate) {
        self.remove_cached_subtree(path);
        self.remove_expanded_subtree(path);
        if self
            .selection
            .as_deref()
            .is_some_and(|selected| selected == path || selected.starts_with(path))
        {
            self.selection = None;
            update.selection_changed = true;
        }
        update.removed.push(path.to_path_buf());
    }

    fn remove_cached_subtree(&mut self, path: &Path) {
        let keys: Vec<PathBuf> = self
            .listings
            .keys()
            .filter(|key| key.as_path() == path || key.starts_with(path))
            .cloned()
            .collect();
        for key in keys {
            self.listings.remove(&key);
        }
    }

    fn remove_expanded_subtree(&mut self, path: &Path) {
        let keys: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter(|key| key.as_path() == path || key.starts_with(path))
            .cloned()
            .collect();
        for key in keys {
            self.expanded.remove(&key);
        }
    }

    fn remap_expanded(&mut self, old_path: &Path, new_path: &Path) {
        let remapped: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter_map(|path| remap_path(path, old_path, new_path))
            .collect();
        for path in remapped {
            self.expanded.insert(path);
        }
    }

    fn remap_selection(&mut self, old_path: &Path, new_path: &Path, update: &mut TreeUpdate) {
        let Some(selected) = self.selection.as_deref() else {
            return;
        };
        if let Some(remapped) = remap_path(selected, old_path, new_path) {
            self.selection = Some(remapped);
            update.selection_changed = true;
        }
    }
}

fn push_limit_row(
    rows: &mut Vec<TreeRow>,
    max_rows: usize,
    path: PathBuf,
    depth: usize,
    message: String,
) {
    if rows.len() >= max_rows {
        return;
    }
    rows.push(TreeRow {
        path,
        name: TREE_LIMIT_ROW_NAME.to_string(),
        depth,
        kind: EntryKind::Error,
        expanded: false,
        selected: false,
        error: Some(message),
    });
}

fn remap_path(path: &Path, old_path: &Path, new_path: &Path) -> Option<PathBuf> {
    if path == old_path {
        Some(new_path.to_path_buf())
    } else {
        path.strip_prefix(old_path)
            .ok()
            .map(|suffix| new_path.join(suffix))
    }
}

fn sort_entries(entries: &mut [TreeEntry]) {
    entries.sort_by(|left, right| {
        entry_rank(left.kind)
            .cmp(&entry_rank(right.kind))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn entry_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::File | EntryKind::Symlink | EntryKind::Other => 1,
        EntryKind::Error => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_entry_cap_appends_one_truncation_row() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        for index in 0..6 {
            fs::write(root.join(format!("file-{index}.txt")), b"").unwrap();
        }

        let guard = PathGuard::new(&root).unwrap();
        let listing =
            DirListing::load_with_limit(&guard, &root, RootGeneration::INITIAL, 5).unwrap();

        assert!(listing.is_truncated());
        assert!(!listing.is_error());
        assert_eq!(
            listing.entries.len(),
            6,
            "five entries plus a truncation row"
        );
        let truncation = listing.entries.last().unwrap();
        assert_eq!(truncation.kind, EntryKind::Error);
        assert_eq!(truncation.name, TRUNCATED_ROW_NAME);
        assert!(truncation.error.as_deref().unwrap().contains("more than 5"));
    }

    #[test]
    fn exact_entry_cap_is_not_marked_truncated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        for index in 0..5 {
            fs::write(root.join(format!("file-{index}.txt")), b"").unwrap();
        }

        let guard = PathGuard::new(&root).unwrap();
        let listing =
            DirListing::load_with_limit(&guard, &root, RootGeneration::INITIAL, 5).unwrap();

        assert!(!listing.is_truncated());
        assert_eq!(listing.entries.len(), 5);
        assert!(listing
            .entries
            .iter()
            .all(|entry| entry.kind != EntryKind::Error));
    }

    #[test]
    fn row_cap_emits_a_truncation_row_instead_of_unbounded_rows() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(root.join(name), b"").unwrap();
        }

        let mut tree = TreeModel::new(&root, RootGeneration::INITIAL).unwrap();
        tree.expand_root().unwrap();
        let rows = tree.rows_with_limits(2, MAX_TREE_DEPTH);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, TREE_LIMIT_ROW_NAME);
        assert_eq!(rows[1].kind, EntryKind::Error);
        assert!(tree.rows().len() <= MAX_TREE_ROWS);
    }

    #[test]
    fn depth_cap_stops_recursive_flattening() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let first = root.join("d1");
        let second = first.join("d2");
        let third = second.join("d3");
        fs::create_dir_all(&third).unwrap();
        fs::write(third.join("file.txt"), b"").unwrap();

        let mut tree = TreeModel::new(&root, RootGeneration::INITIAL).unwrap();
        for dir in [&root, &first, &second, &third] {
            tree.expand(dir).unwrap();
        }
        let rows = tree.rows_with_limits(MAX_TREE_ROWS, 1);

        assert!(rows.iter().any(|row| row.name == TREE_LIMIT_ROW_NAME));
        assert!(rows.iter().all(|row| row.depth <= 1));
        assert!(!rows.iter().any(|row| row.depth > MAX_TREE_DEPTH));
    }
}
