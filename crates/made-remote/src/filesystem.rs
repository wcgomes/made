//! Server-side filesystem authority. All content reads reuse the local guards.
use crate::git::{refs_directory, resolve_git_dirs, GitDirs};
use crate::protocol::*;
use made_explorer::{
    paths::PathGuard,
    preview::{Preview, PreviewLoader},
    root_switch::RootGeneration,
    tree::{DirListing, EntryKind as TreeKind},
    watch::{FsWatcher, WatchEvent, WatchEventKind},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// These lossless native conversions are platform capabilities, not display conversions.
pub fn native(path: &RemotePath) -> Result<PathBuf, ErrorCode> {
    made_platform::remote_path_from_bytes(path.as_bytes()).map_err(|_| ErrorCode::Unsupported)
}
pub fn remote(path: &Path) -> Result<RemotePath, ErrorCode> {
    RemotePath::new(made_platform::remote_path_bytes(path).map_err(|_| ErrorCode::Unsupported)?)
        .map_err(|_| ErrorCode::InvalidRequest)
}

/// One directory subscription made with `WatchDir`.
struct DirWatch {
    /// Identifies this subscription across a release and re-request of the
    /// same path, so a late registration result never lands on a newer entry.
    serial: u64,
    /// The spelling the client asked for, which events are reported under.
    requested: RemotePath,
    canonical: PathBuf,
    /// The request a pending registration is answered under, then the id
    /// pushes ride.
    request: u64,
    /// The platform watch is established. Until then the subscription only
    /// holds its place in the table.
    live: bool,
    /// The watched directory itself was removed or renamed away (replaced by
    /// `rm -rf` and `mkdir`, a checkout, `cargo clean`): the platform dropped
    /// the watch with the inode, and it is armed again once a directory is
    /// back at the path.
    dead: bool,
    /// A re-arm registration is in flight.
    rearming: bool,
    /// A failed re-arm is not tried again before this.
    retry_at: Option<Instant>,
}

/// The repository metadata watched on behalf of the root's subscription.
struct GitWatch {
    request: u64,
    dirs: GitDirs,
    /// Every path registered for it, released together.
    paths: Vec<PathBuf>,
    /// The registration completed.
    ready: bool,
}

/// What a platform registration was for, to apply its outcome.
enum Ticket {
    Dir(u64),
    Rearm(u64),
    Git,
    Unwatch,
}

enum Command {
    /// Subscribe to all of `paths` in one platform operation.
    Watch {
        ticket: u64,
        paths: Vec<(PathBuf, bool)>,
    },
    Unwatch {
        ticket: u64,
        path: PathBuf,
    },
}

/// Every `WatchDir` subscription of the tab. One platform watcher (one
/// `inotify` instance) serves all of them, and it lives on its own thread:
/// establishing or releasing a platform watch can take seconds (macOS
/// FSEvents restarts its stream for each one), and the supervisor loop that
/// relays the terminal must never wait for that. Registrations complete
/// asynchronously (`Filesystem::poll_watch_results`).
struct DirWatches {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Vec<WatchEvent>>,
    done: mpsc::Receiver<(u64, Result<(), ErrorCode>)>,
    tickets: Vec<(u64, Ticket)>,
    next_ticket: u64,
    next_serial: u64,
    dirs: Vec<DirWatch>,
    git: Option<GitWatch>,
    /// How many subscriptions (directories, Git metadata) hold each platform
    /// watch: the platform watch is released with its last holder, so
    /// unwatching an expanded `.git` cannot end the Git watch.
    holders: HashMap<PathBuf, usize>,
    /// Events inside a platform stream restart may have been lost: report
    /// every live directory (and Git) once.
    rescan: bool,
    rearm_checked: Instant,
}

/// How a `WatchDir` request stands when `watch_dir` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchOutcome {
    /// Already subscribed; the acknowledgement can be sent now.
    Live,
    /// Being established; `poll_watch_results` reports it.
    Pending,
}

/// Entries of the Git directories that change what status reports.
const GIT_STATE_FILES: [&str; 7] = [
    "HEAD",
    "index",
    "packed-refs",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "REBASE_HEAD",
];
/// Backends that restart their platform stream for every watch they add or
/// drop (macOS FSEvents) lose events in the gap; the server reports every live
/// directory again after each such operation.
const RESTARTS_STREAM: bool = made_platform::WATCH_BACKEND_RESTARTS_STREAM;
/// How often directories that were replaced are looked for again.
const REARM_INTERVAL: Duration = Duration::from_millis(500);
/// A re-arm that failed waits this long before it is tried again.
const REARM_RETRY: Duration = Duration::from_secs(3);
/// Most distinct paths one directory reports per poll; a burst past it is
/// reported as the directory itself. With `MAX_DIR_WATCHES` directories one
/// poll stays far below the client's queued-reply bound.
const MAX_PUSHED_PATHS: usize = 4;

pub struct Filesystem {
    guard: PathGuard,
    generation: u64,
    watcher: Option<(RemotePath, FsWatcher)>,
    watches: Option<DirWatches>,
    dir_watch_limit: usize,
}
impl Filesystem {
    pub fn new(root: &RemotePath, generation: u64) -> Result<Self, ErrorCode> {
        if generation == 0 {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(Self {
            guard: PathGuard::new(native(root)?).map_err(|_| ErrorCode::Unavailable)?,
            generation,
            watcher: None,
            watches: None,
            dir_watch_limit: MAX_DIR_WATCHES,
        })
    }
    /// Lower the number of `WatchDir` subscriptions held (never above
    /// `MAX_DIR_WATCHES`).
    pub fn with_dir_watch_limit(mut self, limit: usize) -> Self {
        self.dir_watch_limit = limit.min(MAX_DIR_WATCHES);
        self
    }
    pub fn root(&self) -> &Path {
        self.guard.root()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn check(&self, generation: u64) -> Result<(), ErrorCode> {
        if generation == self.generation {
            Ok(())
        } else {
            Err(ErrorCode::Stale)
        }
    }
    /// Root transitions require an authenticated owning-tab selection/report.
    /// No file read API grants a root implicitly.
    pub fn select(&mut self, path: &RemotePath) -> Result<Operation, ErrorCode> {
        let guard = PathGuard::new(native(path)?).map_err(|_| ErrorCode::Unavailable)?;
        let next = self.generation.checked_add(1).ok_or(ErrorCode::Limit)?;
        let canonical = remote(guard.root())?;
        self.guard = guard;
        self.generation = next;
        self.watcher = None;
        self.watches = None;
        Ok(Operation::RootResult {
            requested: path.clone(),
            path: canonical,
            generation: next,
        })
    }
    pub fn validate_root(&self, path: &RemotePath) -> Result<Operation, ErrorCode> {
        let guard = PathGuard::new(native(path)?).map_err(|_| ErrorCode::Unavailable)?;
        Ok(Operation::RootResult {
            requested: path.clone(),
            path: remote(guard.root())?,
            generation: self.generation,
        })
    }
    pub fn list(&self, path: &RemotePath) -> Result<Vec<Operation>, ErrorCode> {
        let listing = DirListing::load(
            &self.guard,
            native(path)?,
            RootGeneration::new(self.generation),
        )
        .map_err(|_| ErrorCode::Unauthorized)?;
        if listing.error.is_some() {
            return Err(ErrorCode::Unavailable);
        }
        let mut entries = Vec::with_capacity(listing.entries.len().min(MAX_DIRECTORY_ENTRIES));
        for entry in listing.entries {
            // Synthetic truncation/error rows do not have a usable wire path.
            if entry.kind == TreeKind::Error {
                continue;
            }
            let name = entry.path.file_name().ok_or(ErrorCode::InvalidRequest)?;
            // Preserve the requested spelling (a guarded in-root symlink parent
            // may canonicalize differently), never reconstruct from display name.
            let exact = native(path)?.join(name);
            entries.push(DirectoryEntry {
                path: remote(&exact)?,
                kind: match entry.kind {
                    TreeKind::Directory => EntryKind::Directory,
                    TreeKind::File => EntryKind::File,
                    TreeKind::Symlink => EntryKind::Symlink,
                    TreeKind::Other | TreeKind::Error => EntryKind::Other,
                },
            });
        }
        // Worst-case byte paths expand to JSON arrays; 128 entries alone is not
        // a safe frame bound. Eight max-sized paths fit the 256 KiB frame cap.
        let batches: Vec<_> = entries.chunks(8).collect();
        if batches.is_empty() {
            return Ok(vec![Operation::Listed {
                path: path.clone(),
                entries: vec![],
                truncated: listing.truncated,
                done: true,
            }]);
        }
        Ok(batches
            .iter()
            .enumerate()
            .map(|(i, batch)| {
                let done = i + 1 == batches.len();
                // The client rejects `truncated` on a non-final frame.
                Operation::Listed {
                    path: path.clone(),
                    entries: batch.to_vec(),
                    truncated: listing.truncated && done,
                    done,
                }
            })
            .collect())
    }
    pub fn preview(&self, path: &RemotePath) -> Result<Vec<Operation>, ErrorCode> {
        let native = native(path)?;
        // Keep missing classification in PreviewLoader, but never open a special entry.
        if let Ok(canonical) = self.guard.resolve_existing(&native) {
            if std::fs::metadata(canonical).is_ok_and(|m| !m.is_file()) {
                return Ok(vec![preview_result(
                    path,
                    PreviewKind::NotRegular,
                    0,
                    &[],
                    true,
                )?]);
            }
        }
        let preview =
            PreviewLoader::from_guard(self.guard.clone(), RootGeneration::new(self.generation))
                .load(native);
        let kind = match &preview {
            Preview::Text { .. } => PreviewKind::Text,
            Preview::Binary { .. } => PreviewKind::Binary,
            Preview::TooLarge { .. } => PreviewKind::TooLarge,
            Preview::InvalidUtf8 { .. } => PreviewKind::InvalidUtf8,
            Preview::Missing { .. } => PreviewKind::Missing,
            Preview::Unreadable { .. } => PreviewKind::Unreadable,
            Preview::Forbidden { .. } => return Err(ErrorCode::Unauthorized),
        };
        let text = preview.text().unwrap_or("");
        let mut result = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = (start + MAX_CHUNK_BYTES).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            result.push(preview_result(
                path,
                kind,
                start as u64,
                &text.as_bytes()[start..end],
                end == text.len(),
            )?);
            start = end;
        }
        if result.is_empty() {
            result.push(preview_result(path, kind, 0, &[], true)?);
        }
        Ok(result)
    }
    pub fn watch(&mut self, path: &RemotePath) -> Result<(), ErrorCode> {
        let canonical = self
            .guard
            .resolve_existing(native(path)?)
            .map_err(|_| ErrorCode::Unauthorized)?;
        let watcher = FsWatcher::new_non_recursive(canonical, RootGeneration::new(self.generation))
            .map_err(|_| ErrorCode::Unavailable)?;
        self.watcher = Some((path.clone(), watcher));
        Ok(())
    }
    pub fn cancel_watch(&mut self) {
        self.watcher = None;
        self.watches = None;
    }
    /// Subscribe to one directory (`WatchDir`), additive and non-recursive.
    /// The request is validated now (inside the root, a directory, room in the
    /// table) and the platform watch is established on the watcher thread:
    /// `Pending` is answered through `poll_watch_results`. Resubscribing to a
    /// directory only re-binds its request id. At most `MAX_DIR_WATCHES`
    /// directories are held; the platform refusing another watch (inotify
    /// exhausted) is the same `Limit` and leaves every earlier subscription
    /// working. Once the root's watch is live, the Git metadata of the
    /// repository the root is in is watched too, best effort.
    pub fn watch_dir(
        &mut self,
        path: &RemotePath,
        request: u64,
    ) -> Result<WatchOutcome, ErrorCode> {
        let canonical = self
            .guard
            .resolve_existing(native(path)?)
            .map_err(|_| ErrorCode::Unauthorized)?;
        if !std::fs::metadata(&canonical).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(ErrorCode::Unavailable);
        }
        let root = RootGeneration::new(self.generation);
        let guard_root = self.guard.root().to_path_buf();
        let limit = self.dir_watch_limit;
        let watches = self
            .watches
            .get_or_insert_with(|| DirWatches::start(guard_root, root));
        if let Some(existing) = watches.dirs.iter_mut().find(|d| &d.requested == path) {
            existing.request = request;
            return Ok(if existing.live {
                WatchOutcome::Live
            } else {
                WatchOutcome::Pending
            });
        }
        if watches.dirs.len() >= limit {
            return Err(ErrorCode::Limit);
        }
        let serial = watches.next_serial;
        watches.next_serial += 1;
        watches.register(Ticket::Dir(serial), vec![(canonical.clone(), false)])?;
        watches.dirs.push(DirWatch {
            serial,
            requested: path.clone(),
            canonical,
            request,
            live: false,
            dead: false,
            rearming: false,
            retry_at: None,
        });
        Ok(WatchOutcome::Pending)
    }
    /// Outcomes of the registrations `watch_dir` left pending, as
    /// `(request id, Changed acknowledgement | error)`. A subscription that was
    /// released meanwhile reports nothing.
    pub fn poll_watch_results(&mut self) -> Vec<(u64, Operation)> {
        let Some(watches) = self.watches.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // A registration that really changed the platform watches.
        let mut changed_watches = false;
        while let Ok((ticket, outcome)) = watches.done.try_recv() {
            let Some(index) = watches.tickets.iter().position(|(id, _)| *id == ticket) else {
                continue;
            };
            changed_watches |= outcome.is_ok();
            match watches.tickets.swap_remove(index).1 {
                Ticket::Dir(serial) => {
                    let Some(at) = watches.dirs.iter().position(|d| d.serial == serial) else {
                        continue;
                    };
                    match outcome {
                        Ok(()) => {
                            let dir = &mut watches.dirs[at];
                            dir.live = true;
                            let request = dir.request;
                            out.push((
                                request,
                                Operation::Changed {
                                    path: dir.requested.clone(),
                                },
                            ));
                            if dir.canonical == self.guard.root() {
                                watches.watch_git(self.guard.root(), request);
                            }
                        }
                        Err(code) => {
                            let dir = watches.dirs.remove(at);
                            watches.release_path(&dir.canonical);
                            out.push((dir.request, Operation::Error { code }));
                        }
                    }
                }
                Ticket::Rearm(serial) => {
                    if let Some(dir) = watches.dirs.iter_mut().find(|d| d.serial == serial) {
                        dir.rearming = false;
                        if outcome.is_err() {
                            dir.retry_at = Some(Instant::now() + REARM_RETRY);
                        } else {
                            dir.dead = false;
                            dir.retry_at = None;
                            // Whatever happened while it was unwatched.
                            out.push((
                                dir.request,
                                Operation::Changed {
                                    path: dir.requested.clone(),
                                },
                            ));
                        }
                    }
                }
                Ticket::Git => {
                    if let Some(git) = &mut watches.git {
                        if outcome.is_ok() {
                            git.ready = true;
                        } else if let Some(git) = watches.git.take() {
                            for path in &git.paths {
                                watches.release_path(path);
                            }
                        }
                    }
                }
                Ticket::Unwatch => {}
            }
        }
        // One rescan for a burst of registrations, once the last one is done
        // (each of them restarts the stream on the backends that lose events).
        if RESTARTS_STREAM && changed_watches && watches.tickets.is_empty() {
            watches.rescan = true;
        }
        out
    }
    /// Release the `WatchDir` subscription of `path`; unknown paths are fine.
    pub fn unwatch_dir(&mut self, path: &RemotePath) {
        if let Some(watches) = &mut self.watches {
            watches.release(|dir| &dir.requested == path);
        }
    }
    /// A `Cancel` of a subscription's request id releases it too.
    pub fn cancel_watch_request(&mut self, request: u64) {
        if let Some(watches) = &mut self.watches {
            watches.release(|dir| dir.request == request);
        }
    }
    /// Directories currently subscribed with `WatchDir` (pending included).
    pub fn watched_dirs(&self) -> usize {
        self.watches
            .as_ref()
            .map_or(0, |watches| watches.dirs.len())
    }
    /// Whether the root's Git metadata is being watched: found, and every
    /// registration it needed has completed (`poll_watch_results` drives it).
    pub fn watching_git(&self) -> bool {
        self.watches
            .as_ref()
            .is_some_and(|watches| watches.git.as_ref().is_some_and(|git| git.ready))
    }
    /// Coalesced notifications for the `WatchDir` subscriptions, as
    /// `(request id, Changed | GitChanged)`: one `Changed` per distinct changed
    /// entry inside a watched directory (the directory itself when a burst
    /// exceeds a bound), so a client reloads only what changed. The paths
    /// carry the spelling the subscription was requested with. A watched
    /// directory that was replaced is watched again here.
    pub fn poll_changes(&mut self) -> Vec<(u64, Operation)> {
        self.rearm_replaced_directories();
        let Some(watches) = self.watches.as_mut() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Ok(batch) = watches.events.try_recv() {
            events.extend(batch);
            if events.len() >= 4096 {
                break;
            }
        }
        let rescan = std::mem::take(&mut watches.rescan);
        if events.is_empty() && !rescan {
            return Vec::new();
        }
        let mut per_dir: Vec<Vec<RemotePath>> = vec![Vec::new(); watches.dirs.len()];
        // A rescan reports every live directory as itself.
        let mut overflow = vec![rescan; watches.dirs.len()];
        let mut git = rescan;
        let mut repository_appeared = false;
        let root = self.guard.root().to_path_buf();
        for event in events {
            if event.is_rescan() {
                // Events may have been lost: everything watched may be stale.
                git = true;
                overflow.iter_mut().for_each(|flag| *flag = true);
                continue;
            }
            let gone = matches!(
                event.kind,
                WatchEventKind::Deleted | WatchEventKind::Renamed
            );
            let paths = std::iter::once(&event.path).chain(event.old_path.as_ref());
            for path in paths {
                git |= watches.is_git_change(path);
                if path.file_name().is_some_and(|name| name == ".git")
                    && path.parent() == Some(root.as_path())
                {
                    repository_appeared = true;
                }
                for (index, dir) in watches.dirs.iter_mut().enumerate() {
                    let reported = if *path == dir.canonical {
                        // The watched directory itself went away: the
                        // platform dropped the watch with its inode.
                        dir.dead |= gone && !dir.rearming;
                        Some(dir.requested.clone())
                    } else if path.parent() == Some(dir.canonical.as_path()) {
                        path.file_name()
                            .and_then(|name| native(&dir.requested).ok().map(|d| d.join(name)))
                            .and_then(|joined| remote(&joined).ok())
                    } else {
                        None
                    };
                    let Some(reported) = reported else { continue };
                    if per_dir[index].len() >= MAX_PUSHED_PATHS {
                        overflow[index] = true;
                    } else if !per_dir[index].contains(&reported) {
                        per_dir[index].push(reported);
                    }
                }
            }
        }
        let mut out = Vec::new();
        let mut root_request = None;
        for (index, dir) in watches.dirs.iter().enumerate() {
            if dir.canonical == root {
                root_request = Some(dir.request);
            }
            if !dir.live {
                continue;
            }
            if overflow[index] {
                out.push((
                    dir.request,
                    Operation::Changed {
                        path: dir.requested.clone(),
                    },
                ));
            } else {
                out.extend(
                    per_dir[index]
                        .drain(..)
                        .map(|path| (dir.request, Operation::Changed { path })),
                );
            }
        }
        // A repository created under a root that had none: watch it now.
        if repository_appeared && watches.git.is_none() {
            if let Some(request) = root_request {
                watches.watch_git(&root, request);
            }
        }
        if git {
            if let Some(request) = watches.git.as_ref().map(|git| git.request) {
                out.push((request, Operation::GitChanged));
            }
        }
        out
    }
    /// Watch again the directories that were replaced: once a directory is
    /// back at the path (through the guard, so a replacement that leaves the
    /// root is refused) it is registered again and reported once.
    fn rearm_replaced_directories(&mut self) {
        let Some(watches) = self.watches.as_mut() else {
            return;
        };
        if !watches.dirs.iter().any(|dir| dir.dead && !dir.rearming)
            || watches.rearm_checked.elapsed() < REARM_INTERVAL
        {
            return;
        }
        watches.rearm_checked = Instant::now();
        for index in 0..watches.dirs.len() {
            let (serial, requested) = {
                let dir = &watches.dirs[index];
                if !dir.dead || dir.rearming || dir.retry_at.is_some_and(|at| Instant::now() < at) {
                    continue;
                }
                (dir.serial, dir.requested.clone())
            };
            let Some(canonical) = native(&requested)
                .ok()
                .and_then(|native| self.guard.resolve_existing(native).ok())
                .filter(|canonical| std::fs::metadata(canonical).is_ok_and(|m| m.is_dir()))
            else {
                continue;
            };
            let old = watches.dirs[index].canonical.clone();
            let armed = if old == canonical {
                // The same path, a different directory behind it (replaced,
                // or moved away and recreated): drop the platform watch of
                // the old inode explicitly before watching again, or it
                // lingers and reports the moved directory under this name.
                watches.reset_path(Ticket::Rearm(serial), &canonical)
            } else {
                let armed =
                    watches.register(Ticket::Rearm(serial), vec![(canonical.clone(), false)]);
                if armed.is_ok() {
                    watches.release_path(&old);
                }
                armed
            };
            if armed.is_err() {
                continue;
            }
            watches.dirs[index].canonical = canonical;
            watches.dirs[index].rearming = true;
        }
    }
    /// Coalesce all changes to the subscription path: clients resnapshot visible
    /// listings/preview/Git instead of trusting deleted or renamed paths as grants.
    pub fn poll_change(&mut self) -> Option<Operation> {
        let (path, watcher) = self.watcher.as_mut()?;
        let mut changed = false;
        for _ in 0..4096 {
            if watcher.poll().is_some() {
                changed = true;
            } else {
                break;
            }
        }
        changed.then(|| Operation::Changed { path: path.clone() })
    }
}
impl DirWatches {
    /// Start the watcher thread. It owns the platform watcher, runs the
    /// registrations it is sent in order, and forwards the debounced events.
    fn start(root: PathBuf, generation: RootGeneration) -> Self {
        let (commands, command_rx) = mpsc::channel::<Command>();
        let (event_tx, events) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            let mut watcher = FsWatcher::new_detached(&root, generation).ok();
            loop {
                match command_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(Command::Watch { ticket, paths }) => {
                        let outcome = match watcher.as_mut() {
                            Some(watcher) => watcher.watch_paths(&paths).map_err(|error| {
                                if error.is_resource_limit() {
                                    ErrorCode::Limit
                                } else {
                                    ErrorCode::Unavailable
                                }
                            }),
                            None => Err(ErrorCode::Unavailable),
                        };
                        if done_tx.send((ticket, outcome)).is_err() {
                            break;
                        }
                    }
                    Ok(Command::Unwatch { ticket, path }) => {
                        if let Some(watcher) = watcher.as_mut() {
                            watcher.unwatch_path(&path);
                        }
                        if done_tx.send((ticket, Ok(()))).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if let Some(watcher) = watcher.as_mut() {
                    let batch = watcher.poll_all();
                    if !batch.is_empty() && event_tx.send(batch).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            commands,
            events,
            done,
            tickets: Vec::new(),
            next_ticket: 1,
            next_serial: 1,
            dirs: Vec::new(),
            git: None,
            holders: HashMap::new(),
            rescan: false,
            rearm_checked: Instant::now(),
        }
    }

    /// Send a registration for `paths` and remember what it was for. Every
    /// path gains a holder; the platform watch ends with its last.
    fn register(&mut self, kind: Ticket, paths: Vec<(PathBuf, bool)>) -> Result<(), ErrorCode> {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        self.commands
            .send(Command::Watch {
                ticket,
                paths: paths.clone(),
            })
            .map_err(|_| ErrorCode::Unavailable)?;
        for (path, _) in paths {
            *self.holders.entry(path).or_insert(0) += 1;
        }
        self.tickets.push((ticket, kind));
        Ok(())
    }

    /// Drop the platform watch of `path` and watch it again, for the holders
    /// it already has (a new directory now sits at the path).
    fn reset_path(&mut self, kind: Ticket, path: &Path) -> Result<(), ErrorCode> {
        let unwatch = self.next_ticket;
        let watch = unwatch + 1;
        self.next_ticket += 2;
        self.commands
            .send(Command::Unwatch {
                ticket: unwatch,
                path: path.to_path_buf(),
            })
            .map_err(|_| ErrorCode::Unavailable)?;
        self.tickets.push((unwatch, Ticket::Unwatch));
        self.commands
            .send(Command::Watch {
                ticket: watch,
                paths: vec![(path.to_path_buf(), false)],
            })
            .map_err(|_| ErrorCode::Unavailable)?;
        self.tickets.push((watch, kind));
        Ok(())
    }

    /// One holder of `path` is gone; the platform watch goes with the last.
    fn release_path(&mut self, path: &Path) {
        let Some(count) = self.holders.get_mut(path) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        self.holders.remove(path);
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        if self
            .commands
            .send(Command::Unwatch {
                ticket,
                path: path.to_path_buf(),
            })
            .is_ok()
        {
            self.tickets.push((ticket, Ticket::Unwatch));
        }
    }

    /// Arm the repository metadata of the repository `root` is in. Failures
    /// leave Git unwatched; the client's periodic refresh covers it.
    fn watch_git(&mut self, root: &Path, request: u64) {
        if let Some(git) = &mut self.git {
            git.request = request;
            return;
        }
        let Some(dirs) = resolve_git_dirs(root) else {
            return;
        };
        let mut targets = vec![(dirs.git_dir.clone(), false)];
        if dirs.common_dir != dirs.git_dir {
            targets.push((dirs.common_dir.clone(), false));
        }
        if let Some(refs) = refs_directory(&dirs.common_dir) {
            targets.push((refs, true));
        }
        // One registration for the whole set.
        let paths: Vec<PathBuf> = targets.iter().map(|(path, _)| path.clone()).collect();
        if self.register(Ticket::Git, targets).is_err() {
            return;
        }
        self.git = Some(GitWatch {
            request,
            dirs,
            paths,
            ready: false,
        });
    }

    /// Whether `path` is a change to the Git state: `HEAD`, the index, merge
    /// markers or `packed-refs` directly in a Git directory, or any ref.
    fn is_git_change(&self, path: &Path) -> bool {
        let Some(git) = &self.git else {
            return false;
        };
        let name = path.file_name().and_then(|name| name.to_str());
        if path.parent() == Some(git.dirs.git_dir.as_path())
            || path.parent() == Some(git.dirs.common_dir.as_path())
        {
            return name.is_some_and(|name| GIT_STATE_FILES.contains(&name));
        }
        path.starts_with(git.dirs.common_dir.join("refs"))
            && !name.is_some_and(|name| name.ends_with(".lock"))
    }

    /// Drop the subscriptions `select` matches. Removing the root drops the
    /// Git metadata with it.
    fn release(&mut self, select: impl Fn(&DirWatch) -> bool) {
        let mut released = Vec::new();
        self.dirs.retain(|dir| {
            let drop = select(dir);
            if drop {
                released.push(dir.canonical.clone());
            }
            !drop
        });
        for canonical in released {
            self.release_path(&canonical);
        }
        if let Some(git) = &self.git {
            if !self.dirs.iter().any(|dir| dir.request == git.request) {
                if let Some(git) = self.git.take() {
                    // A registration still in flight is released after it
                    // lands: the worker runs its commands in order.
                    self.tickets
                        .retain(|(_, kind)| !matches!(kind, Ticket::Git));
                    for path in &git.paths {
                        self.release_path(path);
                    }
                }
            }
        }
    }
}

fn preview_result(
    path: &RemotePath,
    kind: PreviewKind,
    offset: u64,
    bytes: &[u8],
    done: bool,
) -> Result<Operation, ErrorCode> {
    Ok(Operation::PreviewResult {
        path: path.clone(),
        kind,
        offset,
        data: Chunk::new(bytes.to_vec()).map_err(|_| ErrorCode::Limit)?,
        done,
    })
}
