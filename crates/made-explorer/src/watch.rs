//! Filesystem watcher with debounce and update latency budget (Phase 2, tasks
//! 2.3-2.4).
//!
//! `notify` reports raw platform events on its own thread. A small debounce
//! thread coalesces events for the same path and kind, then publishes typed
//! [`WatchEvent`] values on a channel. The app can poll from its egui event loop
//! without blocking; [`FsWatcher::wait_for_event`] is available for tests and
//! non-UI callers.
//!
//! Every event carries the [`RootGeneration`] that was active when the watcher
//! was created. A watcher for an old root may still deliver an event after the
//! app switched roots; the tree/model rejects it through the generation check.

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::fs;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::paths::{PathError, PathGuard};
use crate::root_switch::RootGeneration;

/// Default quiet window used to coalesce bursts of filesystem events.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(150);

/// Maximum number of typed events buffered by the watcher.
///
/// The cap is applied twice: while a debounce batch is accumulating and again
/// when the owning [`FsWatcher`] drains batches into its polling queue. When the
/// cap is reached the oldest queued event is replaced by a single
/// [`WatchEventKind::Rescan`] marker so memory stays bounded without silently
/// dropping the fact that state may be stale.
pub const MAX_PENDING_EVENTS: usize = 4096;

/// Raw platform events buffered between the `notify` callback and the debounce
/// thread. `notify` callbacks must never block, so the callback uses
/// `try_send`; a full queue sets an overflow flag that becomes a rescan event.
const RAW_EVENT_QUEUE_CAPACITY: usize = 1024;

/// Debounced batches buffered between the debounce thread and the watcher.
/// The watcher drains this quickly, but the channel is bounded so a consumer
/// that stops polling cannot grow memory without bound.
const EVENT_BATCH_QUEUE_CAPACITY: usize = 8;

/// Kind of change reported for the active root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchEventKind {
    /// A path was created.
    Created,
    /// An existing path changed (content, metadata, or type).
    Changed,
    /// A path was removed.
    Deleted,
    /// A path was renamed. [`WatchEvent::old_path`] holds the previous path when
    /// the platform reports it; some backends only report the affected path.
    Renamed,
    /// The watcher reported an error or its bounded raw queue overflowed.
    ///
    /// Some typed events may have been dropped, so cached tree state may be
    /// stale. Consumers should reload visible/expanded directories; the event
    /// carries the watched root as its path.
    Rescan,
}

impl WatchEventKind {
    /// Short stable label, useful for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Changed => "changed",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Rescan => "rescan",
        }
    }
}

/// Typed filesystem update for the active root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WatchEvent {
    /// What happened.
    pub kind: WatchEventKind,
    /// New or directly affected path.
    pub path: PathBuf,
    /// Previous path when `kind` is [`WatchEventKind::Renamed`].
    pub old_path: Option<PathBuf>,
    /// Root generation the watcher belonged to.
    pub generation: RootGeneration,
}

impl WatchEvent {
    /// Creates a create/change/delete event.
    #[must_use]
    pub fn new(kind: WatchEventKind, path: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self {
            kind,
            path: path.into(),
            old_path: None,
            generation,
        }
    }

    /// Creates a create event.
    #[must_use]
    pub fn created(path: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self::new(WatchEventKind::Created, path, generation)
    }

    /// Creates a change event.
    #[must_use]
    pub fn changed(path: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self::new(WatchEventKind::Changed, path, generation)
    }

    /// Creates a delete event.
    #[must_use]
    pub fn deleted(path: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self::new(WatchEventKind::Deleted, path, generation)
    }

    /// Creates a rename event when both sides are known.
    #[must_use]
    pub fn renamed(
        old_path: impl Into<PathBuf>,
        new_path: impl Into<PathBuf>,
        generation: RootGeneration,
    ) -> Self {
        Self {
            kind: WatchEventKind::Renamed,
            path: new_path.into(),
            old_path: Some(old_path.into()),
            generation,
        }
    }

    /// Creates a rename event when the platform reports a rename but cannot
    /// associate the old path with the new one (for example macOS FSEvents).
    ///
    /// The parent listing reload removes the old entry; selection/expansion
    /// remapping is not possible without the old path.
    #[must_use]
    pub fn renamed_unknown(path: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self {
            kind: WatchEventKind::Renamed,
            path: path.into(),
            old_path: None,
            generation,
        }
    }

    /// Creates a rescan marker for the watched root.
    ///
    /// A rescan means platform events may have been lost because `notify`
    /// reported an error or the bounded raw queue overflowed. It is not a
    /// file-level change; consumers should reload cached directories.
    #[must_use]
    pub fn rescan(root: impl Into<PathBuf>, generation: RootGeneration) -> Self {
        Self::new(WatchEventKind::Rescan, root, generation)
    }

    /// Returns `true` for a rescan marker.
    #[must_use]
    pub const fn is_rescan(&self) -> bool {
        matches!(self.kind, WatchEventKind::Rescan)
    }

    /// Returns `true` when the event belongs to `current`.
    #[must_use]
    pub fn is_current(&self, current: RootGeneration) -> bool {
        self.generation.is_current(current)
    }

    /// Returns `true` when the event is older than `current`.
    #[must_use]
    pub fn is_stale(&self, current: RootGeneration) -> bool {
        self.generation.is_stale(current)
    }

    /// Returns the old path for a rename, otherwise the affected path.
    #[must_use]
    pub fn previous_path(&self) -> Option<&Path> {
        self.old_path.as_deref()
    }
}

/// Watcher setup failure.
#[derive(Debug)]
pub enum WatchError {
    /// The root path could not be canonicalized or is invalid.
    Path(PathError),
    /// The platform watcher could not be created or attached to the root.
    Notify {
        /// Root that failed to watch.
        path: PathBuf,
        /// Underlying notify error.
        source: notify::Error,
    },
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => write!(f, "cannot watch workspace root: {error}"),
            Self::Notify { path, source } => {
                write!(f, "cannot watch {}: {source}", path.display())
            }
        }
    }
}

impl Error for WatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::Notify { source, .. } => Some(source),
        }
    }
}

impl From<PathError> for WatchError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

/// Debounced recursive watcher for one workspace root.
///
/// The watcher owns its platform watcher and a debounce thread. Both internal
/// queues are bounded: the raw `notify` queue drops its newest event when full
/// and emits a rescan marker, and the polling queue coalesces duplicate events
/// and stays within [`MAX_PENDING_EVENTS`]. Dropping the watcher disconnects the
/// internal channels and lets the threads exit; pending events may be flushed
/// but are no longer observable.
pub struct FsWatcher {
    root: PathBuf,
    generation: RootGeneration,
    debounce: Duration,
    events: Receiver<Vec<PendingEvent>>,
    pending: VecDeque<WatchEvent>,
    pending_seen: HashSet<WatchEvent>,
    _watcher: RecommendedWatcher,
    _thread: Option<JoinHandle<()>>,
}

impl FsWatcher {
    /// Watches `root` recursively with [`DEFAULT_DEBOUNCE`].
    ///
    /// The root is canonicalized through [`PathGuard`] so event paths and tree
    /// paths share the same canonical spelling.
    pub fn new(root: impl AsRef<Path>, generation: RootGeneration) -> Result<Self, WatchError> {
        Self::with_debounce(root, generation, DEFAULT_DEBOUNCE)
    }

    /// Watches `root` recursively with an explicit debounce window.
    pub fn with_debounce(
        root: impl AsRef<Path>,
        generation: RootGeneration,
        debounce: Duration,
    ) -> Result<Self, WatchError> {
        let guard = PathGuard::new(root)?;
        let root = guard.root().to_path_buf();

        let (raw_tx, raw_rx, raw_overflow) = raw_event_channel();
        let callback_overflow = Arc::clone(&raw_overflow);
        let mut watcher = notify::recommended_watcher(move |result| {
            // The callback must not block the platform thread. A full queue
            // sets the overflow flag, which the debounce thread turns into a
            // rescan event.
            try_send_raw_event(&raw_tx, &callback_overflow, result);
        })
        .map_err(|source| WatchError::Notify {
            path: root.clone(),
            source,
        })?;

        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|source| WatchError::Notify {
                path: root.clone(),
                source,
            })?;

        let event_root = root.clone();
        let rescan_root = root.clone();
        let (events, thread) = start_debounce(
            raw_rx,
            raw_overflow,
            debounce,
            move |event| normalize_event(event, &event_root, generation),
            pair_split_renames,
            move || PendingEvent::Public(WatchEvent::rescan(rescan_root.clone(), generation)),
        );

        Ok(Self {
            root,
            generation,
            debounce,
            events,
            pending: VecDeque::new(),
            pending_seen: HashSet::new(),
            _watcher: watcher,
            _thread: Some(thread),
        })
    }

    /// Returns the canonical watched root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the generation stamped onto emitted events.
    #[must_use]
    pub const fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Returns the configured quiet window.
    #[must_use]
    pub const fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Returns `true` when `generation` matches the watcher generation.
    #[must_use]
    pub const fn is_current(&self, generation: RootGeneration) -> bool {
        self.generation.is_current(generation)
    }

    /// Returns the next debounced event without blocking.
    ///
    /// Duplicate events already queued are coalesced. If the queue exceeded
    /// [`MAX_PENDING_EVENTS`], a [`WatchEventKind::Rescan`] marker occupies one
    /// slot and tells the consumer to reload cached state.
    pub fn poll(&mut self) -> Option<WatchEvent> {
        self.fill_pending();
        let event = self.pending.pop_front()?;
        self.pending_seen.remove(&event);
        Some(event)
    }

    /// Drains every event currently available without blocking.
    pub fn poll_all(&mut self) -> Vec<WatchEvent> {
        self.fill_pending();
        self.pending_seen.clear();
        self.pending.drain(..).collect()
    }

    /// Waits for an event up to `timeout`.
    ///
    /// This is intended for tests and background consumers, not the UI thread;
    /// the app should use [`FsWatcher::poll`] from its event loop.
    pub fn wait_for_event(&mut self, timeout: Duration) -> Option<WatchEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.poll() {
                return Some(event);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn fill_pending(&mut self) {
        while let Ok(batch) = self.events.try_recv() {
            for event in batch {
                self.push_pending(event.into_public());
            }
        }
    }

    /// Adds one event to the polling queue, coalescing exact duplicates and
    /// bounding memory. When the queue is full, the newest event is dropped and
    /// the last slot becomes a single rescan marker.
    fn push_pending(&mut self, event: WatchEvent) {
        if self.pending_seen.contains(&event) {
            return;
        }
        if self.pending.len() >= MAX_PENDING_EVENTS {
            let rescan = WatchEvent::rescan(self.root.clone(), self.generation);
            if self.pending_seen.insert(rescan.clone()) {
                if let Some(replaced) = self.pending.pop_back() {
                    self.pending_seen.remove(&replaced);
                }
                self.pending.push_back(rescan);
            }
            return;
        }
        self.pending_seen.insert(event.clone());
        self.pending.push_back(event);
    }
}

/// Internal event wrapper so split rename halves can be paired precisely.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PendingEvent {
    Public(WatchEvent),
    RenameFrom(WatchEvent),
    RenameTo(WatchEvent),
}

impl PendingEvent {
    fn into_public(self) -> WatchEvent {
        match self {
            Self::Public(event) | Self::RenameFrom(event) | Self::RenameTo(event) => event,
        }
    }

    fn event(&self) -> &WatchEvent {
        match self {
            Self::Public(event) | Self::RenameFrom(event) | Self::RenameTo(event) => event,
        }
    }
}

/// Maps a platform event path into the watcher's canonical root spelling.
///
/// Windows reports paths without the `\\?\` verbatim prefix that
/// `canonicalize` adds, and macOS may spell a temporary root through `/var`
/// rather than its `/private/var` target. An event whose lexical spelling is
/// outside the root is checked by canonicalizing the path, or the parent plus
/// file name for a deletion. Paths that still fall outside are dropped; no file
/// content is read.
pub(crate) fn normalize_event_path(path: &Path, root: &Path) -> Option<PathBuf> {
    let inside = |candidate: &Path| candidate == root || candidate.starts_with(root);
    if inside(path) {
        return Some(path.to_path_buf());
    }
    if let Ok(canonical) = fs::canonicalize(path) {
        if inside(&canonical) {
            return Some(canonical);
        }
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = fs::canonicalize(parent) {
            let candidate = canonical_parent.join(name);
            if inside(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn normalize_event(event: Event, root: &Path, generation: RootGeneration) -> Vec<PendingEvent> {
    let paths: Vec<PathBuf> = event
        .paths
        .into_iter()
        .filter_map(|path| normalize_event_path(&path, root))
        .collect();
    if paths.is_empty() {
        return Vec::new();
    }

    let public = |events: Vec<WatchEvent>| {
        events
            .into_iter()
            .map(PendingEvent::Public)
            .collect::<Vec<_>>()
    };

    match event.kind {
        EventKind::Create(_) => public(
            paths
                .into_iter()
                .map(|path| WatchEvent::created(path, generation))
                .collect(),
        ),
        EventKind::Modify(ModifyKind::Name(mode)) => match mode {
            RenameMode::Both if paths.len() >= 2 => public(vec![WatchEvent::renamed(
                paths[0].clone(),
                paths[1].clone(),
                generation,
            )]),
            RenameMode::From => paths
                .into_iter()
                .map(|path| PendingEvent::RenameFrom(WatchEvent::deleted(path, generation)))
                .collect(),
            RenameMode::To => paths
                .into_iter()
                .map(|path| PendingEvent::RenameTo(WatchEvent::created(path, generation)))
                .collect(),
            RenameMode::Both | RenameMode::Any => public(
                paths
                    .into_iter()
                    .map(|path| WatchEvent::renamed_unknown(path, generation))
                    .collect(),
            ),
            RenameMode::Other => public(
                paths
                    .into_iter()
                    .map(|path| WatchEvent::changed(path, generation))
                    .collect(),
            ),
        },
        EventKind::Modify(_) => public(
            paths
                .into_iter()
                .map(|path| WatchEvent::changed(path, generation))
                .collect(),
        ),
        EventKind::Remove(_) => public(
            paths
                .into_iter()
                .map(|path| WatchEvent::deleted(path, generation))
                .collect(),
        ),
        EventKind::Access(_) => Vec::new(),
        EventKind::Any | EventKind::Other => public(
            paths
                .into_iter()
                .map(|path| WatchEvent::changed(path, generation))
                .collect(),
        ),
    }
}

/// Creates the bounded channel used between a `notify` callback and the
/// debounce thread. The returned `AtomicBool` is set when the callback cannot
/// enqueue because the queue is full.
pub(crate) fn raw_event_channel() -> (
    SyncSender<notify::Result<Event>>,
    Receiver<notify::Result<Event>>,
    Arc<AtomicBool>,
) {
    let (sender, receiver) = mpsc::sync_channel(RAW_EVENT_QUEUE_CAPACITY);
    (sender, receiver, Arc::new(AtomicBool::new(false)))
}

/// Enqueues one raw `notify` result without blocking the platform callback.
pub(crate) fn try_send_raw_event(
    sender: &SyncSender<notify::Result<Event>>,
    overflow: &AtomicBool,
    result: notify::Result<Event>,
) {
    if matches!(sender.try_send(result), Err(TrySendError::Full(_))) {
        overflow.store(true, Ordering::Relaxed);
    }
}

/// Bounded, coalescing pending batch used by the debounce thread.
struct PendingBatch<T> {
    items: Vec<T>,
    seen: HashSet<T>,
    capacity: usize,
    overflowed: bool,
}

impl<T> PendingBatch<T>
where
    T: Clone + Eq + Hash,
{
    fn new(capacity: usize) -> Self {
        Self {
            items: Vec::new(),
            seen: HashSet::new(),
            capacity,
            overflowed: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty() && !self.overflowed
    }

    fn push(&mut self, item: T) {
        if self.overflowed || self.seen.contains(&item) {
            return;
        }
        if self.items.len() >= self.capacity {
            self.overflowed = true;
            return;
        }
        self.seen.insert(item.clone());
        self.items.push(item);
    }

    fn mark_overflow(&mut self) {
        self.overflowed = true;
    }

    /// Finalizes, deduplicates, and drains the batch. `finalize` may remove or
    /// combine items; a rescan marker is appended when events were dropped.
    fn finish<F, O>(&mut self, finalize: &mut F, rescan: &O) -> Vec<T>
    where
        F: FnMut(&mut Vec<T>),
        O: Fn() -> T,
    {
        finalize(&mut self.items);
        if self.overflowed {
            self.items.push(rescan());
        }
        dedupe(&mut self.items);
        self.seen.clear();
        self.overflowed = false;
        std::mem::take(&mut self.items)
    }
}

/// Starts a generic debounce thread over raw notify events.
///
/// `convert` turns one platform event into zero or more typed items. Items are
/// coalesced by equality in a hash set while the burst is pending, and the
/// pending batch is capped at [`MAX_PENDING_EVENTS`]. Before a batch is
/// published, `finalize` may coalesce related items (the watcher uses it to pair
/// split rename events). Platform errors, raw-queue overflow, and pending-batch
/// overflow append one `rescan` item instead of being ignored. The thread
/// flushes after a quiet window or after four windows of continuous activity,
/// keeping the worst-case latency bounded.
pub(crate) fn start_debounce<T, F, C, R>(
    raw: Receiver<notify::Result<Event>>,
    raw_overflow: Arc<AtomicBool>,
    debounce: Duration,
    mut convert: F,
    mut finalize: C,
    rescan: R,
) -> (Receiver<Vec<T>>, JoinHandle<()>)
where
    T: Clone + Eq + Hash + Send + 'static,
    F: FnMut(Event) -> Vec<T> + Send + 'static,
    C: FnMut(&mut Vec<T>) + Send + 'static,
    R: Fn() -> T + Send + 'static,
{
    let (out_tx, out_rx) = mpsc::sync_channel(EVENT_BATCH_QUEUE_CAPACITY);
    let thread = thread::spawn(move || {
        let poll_interval = (debounce / 4)
            .max(Duration::from_millis(5))
            .min(Duration::from_millis(50));
        let max_latency = debounce.checked_mul(4).unwrap_or(Duration::from_secs(2));

        let mut batch = PendingBatch::new(MAX_PENDING_EVENTS);
        let mut first_at: Option<Instant> = None;
        let mut last_at = Instant::now();

        loop {
            let mut activity = false;
            match raw.recv_timeout(poll_interval) {
                Ok(Ok(event)) => {
                    let converted = convert(event);
                    if !converted.is_empty() {
                        let now = Instant::now();
                        if first_at.is_none() {
                            first_at = Some(now);
                        }
                        last_at = now;
                        for item in converted {
                            batch.push(item);
                        }
                    }
                }
                Ok(Err(_)) => {
                    // A platform watcher error means events may have been
                    // missed. Mark the batch stale so a rescan is published.
                    batch.mark_overflow();
                    activity = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if raw_overflow.swap(false, Ordering::Relaxed) {
                        batch.mark_overflow();
                    }
                    if !batch.is_empty() {
                        let items = batch.finish(&mut finalize, &rescan);
                        let _ = out_tx.send(items);
                    }
                    break;
                }
            }

            if raw_overflow.swap(false, Ordering::Relaxed) {
                batch.mark_overflow();
                activity = true;
            }
            if activity {
                let now = Instant::now();
                if first_at.is_none() {
                    first_at = Some(now);
                }
                last_at = now;
            }

            if batch.is_empty() {
                continue;
            }

            let now = Instant::now();
            let quiet = now.duration_since(last_at) >= debounce;
            let too_old = first_at
                .map(|first| now.duration_since(first) >= max_latency)
                .unwrap_or(false);
            if quiet || too_old {
                let items = batch.finish(&mut finalize, &rescan);
                if out_tx.send(items).is_err() {
                    break;
                }
                first_at = None;
            }
        }
    });

    (out_rx, thread)
}

fn dedupe<T>(items: &mut Vec<T>)
where
    T: Clone + Eq + Hash,
{
    let mut seen = HashSet::with_capacity(items.len());
    items.retain(|item| seen.insert(item.clone()));
}

/// Pairs the two halves of a split rename into one typed event.
///
/// Linux reports a paired rename directly. Windows and some other backends emit
/// `From`/`To` separately. Only events that the platform explicitly labeled as
/// rename halves are paired, so an ordinary create plus delete in the same burst
/// stays separate. If exactly one half of each side is present, they are paired;
/// unmatched halves degrade to a deleted/created event.
fn pair_split_renames(events: &mut Vec<PendingEvent>) {
    let mut from: Option<usize> = None;
    let mut to: Option<usize> = None;
    for (index, event) in events.iter().enumerate() {
        match event {
            PendingEvent::RenameFrom(_) if from.is_none() => from = Some(index),
            PendingEvent::RenameTo(_) if to.is_none() => to = Some(index),
            PendingEvent::RenameFrom(_) | PendingEvent::RenameTo(_) => {
                // More than one half on either side cannot be paired safely
                // within this batch. Keep them as ordinary delete/create events.
                return;
            }
            PendingEvent::Public(_) => {}
        }
    }

    let (Some(from_index), Some(to_index)) = (from, to) else {
        return;
    };
    let old_path = events[from_index].event().path.clone();
    let new_path = events[to_index].event().path.clone();
    let generation = events[from_index].event().generation;

    let (first, second) = if from_index < to_index {
        (from_index, to_index)
    } else {
        (to_index, from_index)
    };
    events.remove(second);
    events.remove(first);
    events.push(PendingEvent::Public(WatchEvent::renamed(
        old_path, new_path, generation,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_stale_when_generation_advances() {
        let event = WatchEvent::created("/workspace/file", RootGeneration::new(4));
        assert!(event.is_current(RootGeneration::new(4)));
        assert!(event.is_stale(RootGeneration::new(5)));
        assert!(!event.is_current(RootGeneration::new(5)));
    }

    #[test]
    fn split_rename_is_paired_within_a_debounce_batch() {
        let generation = RootGeneration::new(2);
        let mut events = vec![
            PendingEvent::RenameFrom(WatchEvent::deleted("/workspace/old.txt", generation)),
            PendingEvent::RenameTo(WatchEvent::created("/workspace/new.txt", generation)),
        ];

        pair_split_renames(&mut events);

        assert_eq!(events.len(), 1);
        let event = events[0].event();
        assert_eq!(event.kind, WatchEventKind::Renamed);
        assert_eq!(event.path, PathBuf::from("/workspace/new.txt"));
        assert_eq!(
            event.old_path.as_deref(),
            Some(Path::new("/workspace/old.txt"))
        );
    }

    #[test]
    fn paired_rename_deduplicates_an_existing_both_event() {
        let generation = RootGeneration::new(3);
        let renamed = WatchEvent::renamed("/workspace/old.txt", "/workspace/new.txt", generation);
        let mut events = vec![
            PendingEvent::Public(renamed.clone()),
            PendingEvent::RenameFrom(WatchEvent::deleted("/workspace/old.txt", generation)),
            PendingEvent::RenameTo(WatchEvent::created("/workspace/new.txt", generation)),
        ];

        pair_split_renames(&mut events);
        dedupe(&mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event(), &renamed);
    }

    fn synthetic_event(path: &str) -> Event {
        Event::new(EventKind::Create(notify::event::CreateKind::File)).add_path(PathBuf::from(path))
    }

    #[test]
    fn pending_batch_caps_events_and_appends_one_rescan() {
        let mut batch = PendingBatch::new(4);
        for index in 0..20 {
            batch.push(format!("event-{index}"));
        }

        let items = batch.finish(&mut |_| {}, &|| "rescan".to_string());

        assert_eq!(items.len(), 5, "four events plus one rescan marker");
        assert_eq!(items.last().map(String::as_str), Some("rescan"));
        assert_eq!(
            items
                .iter()
                .filter(|item| item.as_str() == "rescan")
                .count(),
            1
        );
        assert!(batch.is_empty());
    }

    #[test]
    fn pending_batch_coalesces_duplicates_by_hash() {
        let mut batch = PendingBatch::new(MAX_PENDING_EVENTS);
        for _ in 0..10_000 {
            batch.push("same-event".to_string());
        }

        let items = batch.finish(&mut |_| {}, &|| "rescan".to_string());

        assert_eq!(items, vec!["same-event".to_string()]);
    }

    #[test]
    fn debounce_burst_overflow_becomes_a_rescan_without_unbounded_output() {
        let (raw_tx, raw_rx, overflow) = raw_event_channel();
        // A long quiet window guarantees the burst is handled as one batch;
        // dropping the sender flushes it immediately.
        let (events, thread) = start_debounce(
            raw_rx,
            overflow,
            Duration::from_secs(60),
            |event| {
                event
                    .paths
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect()
            },
            |_| {},
            || "rescan".to_string(),
        );

        for index in 0..(MAX_PENDING_EVENTS + 500) {
            raw_tx
                .send(Ok(synthetic_event(&format!("/workspace/file-{index}"))))
                .unwrap();
        }
        drop(raw_tx);

        let batch = events
            .recv_timeout(Duration::from_secs(5))
            .expect("debounce thread should flush on disconnect");
        thread.join().unwrap();

        assert!(
            batch.len() <= MAX_PENDING_EVENTS + 1,
            "burst output must stay bounded, got {}",
            batch.len()
        );
        assert!(
            batch.iter().any(|item| item == "rescan"),
            "dropped events must be reported as a rescan"
        );
        assert_eq!(
            batch
                .iter()
                .filter(|item| item.as_str() == "rescan")
                .count(),
            1
        );
    }

    #[test]
    fn debounce_notify_error_becomes_a_rescan() {
        let (raw_tx, raw_rx, overflow) = raw_event_channel();
        let (events, thread) = start_debounce(
            raw_rx,
            overflow,
            Duration::from_millis(20),
            |_| Vec::new(),
            |_| {},
            || "rescan".to_string(),
        );

        raw_tx
            .send(Err(notify::Error::generic("platform watcher failed")))
            .unwrap();
        drop(raw_tx);

        let batch = events
            .recv_timeout(Duration::from_secs(5))
            .expect("notify errors must produce an event");
        thread.join().unwrap();

        assert_eq!(batch, vec!["rescan".to_string()]);
    }

    #[test]
    fn debounce_raw_overflow_flag_becomes_a_rescan() {
        let (raw_tx, raw_rx, overflow) = raw_event_channel();
        let (events, thread) = start_debounce(
            raw_rx,
            Arc::clone(&overflow),
            Duration::from_millis(20),
            |_| Vec::new(),
            |_| {},
            || "rescan".to_string(),
        );

        overflow.store(true, Ordering::Relaxed);
        drop(raw_tx);

        let batch = events
            .recv_timeout(Duration::from_secs(5))
            .expect("raw queue overflow must produce an event");
        thread.join().unwrap();

        assert_eq!(batch, vec!["rescan".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn alternate_root_spelling_is_normalized_for_existing_and_deleted_paths() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let alias = temp.path().join("alias");
        symlink(&root, &alias).unwrap();
        let file = root.join("file.txt");
        fs::write(&file, "content").unwrap();

        let event_path = alias.join("file.txt");
        assert_eq!(normalize_event_path(&event_path, &root), Some(file.clone()));

        fs::remove_file(&file).unwrap();
        assert_eq!(normalize_event_path(&event_path, &root), Some(file));
    }
}
