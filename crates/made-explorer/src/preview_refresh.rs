//! Preview auto-refresh for external file changes (Phase 2, tasks 2.9-2.10).
//!
//! Watches the open file's parent directory with `notify`, debounces bursts, and
//! re-runs the read-only [`PreviewLoader`] for the current selection. The loader
//! reclassifies text, binary, oversized, deleted, and unreadable files.
//!
//! Selection changes are stale-safe:
//!
//! - replacing the selection drops the old watcher and its queued events;
//! - every selection has a [`PreviewSelection`] token containing the root
//!   generation and a monotonically increasing sequence;
//! - [`PreviewRefresher::refresh`] and [`PreviewRefresher::accept`] reject a
//!   result produced for a previous selection or root.
//!
//! Watcher setup is best-effort for a missing/unreadable parent: the selection
//! still works and can be refreshed explicitly, which yields the loader's safe
//! missing/unreadable message.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::paths::{PathError, PathGuard};
use crate::preview::{Preview, PreviewLoader};
use crate::root_switch::{RootGeneration, StaleGeneration};
use crate::watch::{normalize_event_path, raw_event_channel, start_debounce, try_send_raw_event};

/// Default debounce window for preview changes.
pub const DEFAULT_PREVIEW_DEBOUNCE: Duration = Duration::from_millis(150);

/// Token identifying one preview selection and its root generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewSelection {
    generation: RootGeneration,
    sequence: u64,
}

impl PreviewSelection {
    /// Returns the root generation of the selection.
    #[must_use]
    pub const fn generation(self) -> RootGeneration {
        self.generation
    }

    /// Returns the selection sequence.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns `true` when `self` no longer matches `current`.
    #[must_use]
    pub const fn is_stale(self, current: Self) -> bool {
        !(self.generation.is_current(current.generation) && self.sequence == current.sequence)
    }
}

/// Best-effort external-change notification for the active preview selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRefreshEvent {
    /// Canonical path at selection time.
    pub path: PathBuf,
    /// Root generation at selection time.
    pub generation: RootGeneration,
    /// Selection token at selection time.
    pub selection: PreviewSelection,
}

impl PreviewRefreshEvent {
    /// Returns `true` when this event still belongs to `selection` and its root
    /// generation.
    #[must_use]
    pub fn is_current(&self, selection: PreviewSelection) -> bool {
        self.generation.is_current(selection.generation) && self.selection == selection
    }
}

/// Preview selection/refresh failure.
#[derive(Debug)]
pub enum PreviewError {
    /// The path guard rejected the selected path.
    Path(PathError),
    /// The selection or root changed before a refresh result was applied.
    StaleSelection {
        /// Selection that is current now, if any.
        current: Option<PreviewSelection>,
        /// Selection carried by the rejected result.
        attempted: PreviewSelection,
    },
    /// A preview read carried an older root generation.
    StaleGeneration(StaleGeneration),
    /// A root switch reused or moved back a generation.
    NonMonotonicGeneration {
        /// Current generation.
        current: RootGeneration,
        /// Requested generation.
        requested: RootGeneration,
    },
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => write!(f, "{error}"),
            Self::StaleSelection { current, attempted } => match current {
                Some(current) => write!(
                    f,
                    "stale preview selection {} (current sequence is {})",
                    attempted.sequence, current.sequence
                ),
                None => write!(
                    f,
                    "stale preview selection {} (no selection is current)",
                    attempted.sequence
                ),
            },
            Self::StaleGeneration(error) => write!(f, "{error}"),
            Self::NonMonotonicGeneration { current, requested } => write!(
                f,
                "root generation {requested} is not newer than current generation {current}"
            ),
        }
    }
}

impl Error for PreviewError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::StaleGeneration(error) => Some(error),
            Self::StaleSelection { .. } | Self::NonMonotonicGeneration { .. } => None,
        }
    }
}

impl From<PathError> for PreviewError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

/// Whether an event can change previewable file content. Access-time and
/// metadata-only notifications (notably macOS FSEvents INODE_META_MOD) must
/// not make reading the preview feed its own refresh.
///
/// This intentionally drops permission and attribute changes (chmod, chown,
/// xattr, touch): they can affect whether a file is readable, but never its
/// bytes, so a preview re-read is not required. The tradeoff is that a preview
/// whose file becomes unreadable keeps its current contents until the next
/// content event or explicit refresh.
fn is_content_event(kind: &notify::EventKind) -> bool {
    !matches!(
        kind,
        notify::EventKind::Access(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Metadata(_))
    )
}

/// Internal debounced signal; the current selection supplies the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RefreshSignal;

struct ActiveSelection {
    selection: PreviewSelection,
    path: PathBuf,
    events: Option<Receiver<Vec<RefreshSignal>>>,
    _watcher: Option<RecommendedWatcher>,
    _thread: Option<JoinHandle<()>>,
}

/// Owns the open preview selection and its debounced file watcher.
///
/// The refresher can be moved to the app thread; `select`, `poll_event`, and
/// `refresh` must be called from that owning thread. `PreviewLoader` clones can
/// be sent to a worker and the result applied with [`PreviewRefresher::accept`].
pub struct PreviewRefresher {
    guard: PathGuard,
    loader: PreviewLoader,
    generation: RootGeneration,
    selection: Option<ActiveSelection>,
    next_sequence: u64,
    debounce: Duration,
}

impl PreviewRefresher {
    /// Creates a refresher for `root` with the default debounce window.
    pub fn new(root: impl AsRef<Path>, generation: RootGeneration) -> Result<Self, PathError> {
        Self::with_debounce(root, generation, DEFAULT_PREVIEW_DEBOUNCE)
    }

    /// Creates a refresher with an explicit debounce window.
    pub fn with_debounce(
        root: impl AsRef<Path>,
        generation: RootGeneration,
        debounce: Duration,
    ) -> Result<Self, PathError> {
        let guard = PathGuard::new(root)?;
        Ok(Self::from_guard(guard, generation, debounce))
    }

    /// Creates a refresher from an existing guard.
    #[must_use]
    pub fn from_guard(guard: PathGuard, generation: RootGeneration, debounce: Duration) -> Self {
        let loader = PreviewLoader::from_guard(guard.clone(), generation);
        Self {
            guard,
            loader,
            generation,
            selection: None,
            next_sequence: 0,
            debounce,
        }
    }

    /// Returns the canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.guard.root()
    }

    /// Returns the active root generation.
    #[must_use]
    pub const fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Returns the loader used for refresh reads.
    #[must_use]
    pub fn loader(&self) -> &PreviewLoader {
        &self.loader
    }

    /// Returns the configured debounce window.
    #[must_use]
    pub const fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Returns the current selection token.
    #[must_use]
    pub const fn current_selection(&self) -> Option<PreviewSelection> {
        match &self.selection {
            Some(selection) => Some(selection.selection),
            None => None,
        }
    }

    /// Returns the current selected path.
    #[must_use]
    pub fn current_path(&self) -> Option<&Path> {
        self.selection
            .as_ref()
            .map(|selection| selection.path.as_path())
    }

    /// Returns `true` when `selection` is still current.
    #[must_use]
    pub fn is_current(&self, selection: PreviewSelection) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|active| active.selection == selection)
    }

    /// Selects a path and starts watching its parent directory.
    ///
    /// Missing files are selectable (the loader reports `Missing`), and a
    /// missing/unreadable parent only disables automatic refresh. A path outside
    /// the root is rejected by the guard.
    pub fn select(&mut self, path: impl AsRef<Path>) -> Result<PreviewSelection, PreviewError> {
        let path = self.guard.resolve_for_create(path)?;
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("preview selection counter exhausted");
        let selection = PreviewSelection {
            generation: self.generation,
            sequence,
        };
        let active = self.start_watching(path, selection);
        self.selection = Some(active);
        Ok(selection)
    }

    /// Clears the selection and stops watching the old path.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Switches to a new root and generation, invalidating the old selection.
    ///
    /// The generation must be strictly greater than the current one. A stale
    /// load from the old selection is rejected even if the sequence counter
    /// later reaches the same value.
    pub fn switch_root(
        &mut self,
        root: impl AsRef<Path>,
        generation: RootGeneration,
    ) -> Result<(), PreviewError> {
        if !self.generation.is_stale(generation) {
            return Err(PreviewError::NonMonotonicGeneration {
                current: self.generation,
                requested: generation,
            });
        }
        let guard = PathGuard::new(root)?;
        self.loader = PreviewLoader::from_guard(guard.clone(), generation);
        self.guard = guard;
        self.generation = generation;
        self.selection = None;
        Ok(())
    }

    /// Returns the next debounced change for the current selection.
    ///
    /// Events for an old selection never reach this method because replacing
    /// the selection drops the old receiver; a defensive generation/token check
    /// also discards stale queued events.
    pub fn poll_event(&mut self) -> Option<PreviewRefreshEvent> {
        let active = self.selection.as_ref()?;
        let mut changed = false;
        if let Some(events) = &active.events {
            while let Ok(batch) = events.try_recv() {
                if !batch.is_empty() {
                    changed = true;
                }
            }
        }
        if !changed {
            return None;
        }

        let event = PreviewRefreshEvent {
            path: active.path.clone(),
            generation: active.selection.generation,
            selection: active.selection,
        };
        if self.is_current(event.selection) {
            Some(event)
        } else {
            None
        }
    }

    /// Waits for a refresh event, intended for tests and background consumers.
    pub fn wait_for_event(&mut self, timeout: Duration) -> Option<PreviewRefreshEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.poll_event() {
                return Some(event);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Reloads the current selection and returns the new safe preview.
    ///
    /// Returns [`PreviewError::StaleSelection`] if the selection changed since
    /// the token was issued, including after the read completes.
    pub fn refresh(&self, selection: PreviewSelection) -> Result<Preview, PreviewError> {
        if !self.is_current(selection) {
            return Err(PreviewError::StaleSelection {
                current: self.current_selection(),
                attempted: selection,
            });
        }
        let Some(active) = self.selection.as_ref() else {
            return Err(PreviewError::StaleSelection {
                current: None,
                attempted: selection,
            });
        };
        let path = active.path.clone();

        let preview = self
            .loader
            .load_for(&path, selection.generation)
            .map_err(PreviewError::StaleGeneration)?;

        if !self.is_current(selection) {
            return Err(PreviewError::StaleSelection {
                current: self.current_selection(),
                attempted: selection,
            });
        }
        Ok(preview)
    }

    /// Validates an off-thread refresh result before it is displayed.
    pub fn accept(
        &self,
        selection: PreviewSelection,
        preview: Preview,
    ) -> Result<Preview, PreviewError> {
        if self.is_current(selection) {
            Ok(preview)
        } else {
            Err(PreviewError::StaleSelection {
                current: self.current_selection(),
                attempted: selection,
            })
        }
    }

    /// Polls for one change and reloads the current preview.
    ///
    /// Returns `Ok(None)` when nothing changed. UI code can instead poll the
    /// event and perform the read on a worker.
    pub fn poll_refresh(&mut self) -> Result<Option<Preview>, PreviewError> {
        let Some(event) = self.poll_event() else {
            return Ok(None);
        };
        self.refresh(event.selection).map(Some)
    }

    fn start_watching(&self, path: PathBuf, selection: PreviewSelection) -> ActiveSelection {
        let Some(parent) = path.parent() else {
            return ActiveSelection {
                selection,
                path,
                events: None,
                _watcher: None,
                _thread: None,
            };
        };

        if !parent.is_dir() {
            return ActiveSelection {
                selection,
                path,
                events: None,
                _watcher: None,
                _thread: None,
            };
        }

        let (raw_tx, raw_rx, raw_overflow) = raw_event_channel();
        let callback_overflow = Arc::clone(&raw_overflow);
        let mut watcher = match notify::recommended_watcher(move |result| {
            try_send_raw_event(&raw_tx, &callback_overflow, result);
        }) {
            Ok(watcher) => watcher,
            Err(_) => {
                return ActiveSelection {
                    selection,
                    path,
                    events: None,
                    _watcher: None,
                    _thread: None,
                };
            }
        };

        if watcher.watch(parent, RecursiveMode::NonRecursive).is_err() {
            return ActiveSelection {
                selection,
                path,
                events: None,
                _watcher: None,
                _thread: None,
            };
        }

        let target = path.clone();
        let watched_parent = parent.to_path_buf();
        let (events, thread) = start_debounce(
            raw_rx,
            raw_overflow,
            self.debounce,
            move |event| {
                if !is_content_event(&event.kind) {
                    // Reading the preview must not feed its own watcher.
                    return Vec::new();
                }
                let relevant = event
                    .paths
                    .iter()
                    .filter_map(|event_path| normalize_event_path(event_path, &watched_parent))
                    .any(|event_path| event_path == target || event_path == watched_parent);
                if relevant {
                    vec![RefreshSignal]
                } else {
                    Vec::new()
                }
            },
            |_| {},
            || RefreshSignal,
        );

        ActiveSelection {
            selection,
            path,
            events: Some(events),
            _watcher: Some(watcher),
            _thread: Some(thread),
        }
    }
}

/// Convenience alias matching the module name.
pub type PreviewRefresh = PreviewRefresher;

#[cfg(test)]
mod tests {
    use super::is_content_event;
    use notify::event::{AccessKind, AccessMode, MetadataKind, ModifyKind};
    use notify::EventKind;

    #[test]
    fn access_and_metadata_events_are_not_content_changes() {
        assert!(!is_content_event(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!is_content_event(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Any
        ))));
        assert!(!is_content_event(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::AccessTime
        ))));
    }

    #[test]
    fn create_remove_and_data_changes_are_content_changes() {
        assert!(is_content_event(&EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(is_content_event(&EventKind::Remove(
            notify::event::RemoveKind::File
        )));
        assert!(is_content_event(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content
        ))));
    }
}
