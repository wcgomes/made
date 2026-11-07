//! Claude Code follow: hook spool, ownership, liveness, presence and titles.
//!
//! Made learns about Claude Code through two independent channels. The
//! session-only plugin's hook appends one metadata record per event to a
//! per-tab spool, and the terminal's OSC 7501 receiver reports session-level
//! status. This module reads the spool ([`SpoolReader`], [`SpoolWatcher`]),
//! owns the spool lifecycle ([`SpoolInstance`]) and joins both channels for one
//! tab ([`ClaudeTab`]).
//!
//! Records are metadata only: session id, working directory, event kind, a
//! derived status, an optional sanitized session title and the hook's process
//! ancestry. Nothing else of the hook payload ever reaches this module, and
//! the terminal window title is read only through [`ClaudeTab::tooltip_title`]
//! while a protocol status is live.

use std::collections::hash_map::RandomState;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::Duration;

use made_terminal::{ProtocolEvent, ProtocolState};
use serde::Deserialize;

use crate::integrations::shared::process_ancestry::process_belongs_to_with;
use crate::sanitize::sanitize_display;

/// Most parent hops from a Claude Code process to its tab's terminal process.
pub const MAX_HOPS: usize = 32;

/// Size the hook never lets a spool exceed. A fully consumed spool with less
/// than [`MAX_RECORD_BYTES`] of room below it is truncated.
pub const SPOOL_BOUND: u64 = 1024 * 1024;

/// Largest record, newline included, that is applied.
pub const MAX_RECORD_BYTES: usize = 4 * 1024;

/// Longest title shown, in characters.
pub const MAX_TITLE_CHARS: usize = 120;

/// Interval between spool polls.
pub const SPOOL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Title shown when no better one exists.
pub const DEFAULT_TITLE: &str = "Claude Code";

/// File name of the lock a running Made instance holds in its spool directory.
const OWNER_LOCK: &str = "owner.lock";

/// Name the lock file has until it is held.
const OWNER_LOCK_PENDING: &str = "owner.lock.pending";

/// Directory under the private per-user temp directory that holds spools.
const SPOOL_ROOT: &str = "claude";

/// The shipped plugin directory when its hook is really there.
///
/// `shipped` is the readable, separator-safe directory the platform layer
/// found. The plugin counts as present only when its `bin/made-claude-status`
/// is an executable regular file, because a plugin whose hook cannot run
/// reports nothing. This reads the filesystem: call it at launch, not per
/// frame.
pub fn ready_claude_plugin_dir(shipped: Option<&Path>) -> Option<PathBuf> {
    let dir = shipped?;
    let hook = dir.join("bin").join(format!(
        "made-claude-status{}",
        std::env::consts::EXE_SUFFIX
    ));
    made_platform::is_executable_file(&hook).then(|| dir.to_path_buf())
}

// ---- records ---------------------------------------------------------------

/// The status a hook record derived for its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStatus {
    Idle,
    Working,
    Attention,
    Error,
    /// The session ended.
    Ended,
}

/// One validated spool record. It carries no prompt, tool or message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Claude Code session id.
    pub session_id: String,
    /// Working directory the event reported. Untrusted and unvalidated.
    pub cwd: String,
    /// Hook event kind.
    pub event: String,
    /// Derived status; `None` keeps the session's current one.
    pub status: Option<HookStatus>,
    /// Sanitized session title, empty when the event carried none.
    pub title: String,
    /// Ancestor pids of the hook process, nearest first.
    pub pids: Vec<u32>,
}

#[derive(Deserialize)]
struct RawRecord {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pids: Vec<u32>,
}

/// Parse one spool line, or `None` when it is malformed.
///
/// Only the fields of [`Record`] are deserialized; every other field of the
/// line is skipped without being copied.
pub fn parse_record(line: &str) -> Option<Record> {
    let raw: RawRecord = serde_json::from_str(line).ok()?;
    if raw.session_id.is_empty() || raw.pids.len() > MAX_HOPS {
        return None;
    }
    let status = match raw.status.as_str() {
        "" => None,
        "idle" => Some(HookStatus::Idle),
        "working" => Some(HookStatus::Working),
        "attention" => Some(HookStatus::Attention),
        "error" => Some(HookStatus::Error),
        "ended" => Some(HookStatus::Ended),
        _ => return None,
    };
    Some(Record {
        session_id: raw.session_id,
        cwd: raw.cwd,
        event: raw.event,
        status,
        title: sanitize_display(&raw.title, MAX_TITLE_CHARS)
            .trim()
            .to_owned(),
        pids: raw.pids,
    })
}

// ---- reading ---------------------------------------------------------------

/// Incremental reader of one tab's spool.
#[derive(Debug)]
pub struct SpoolReader {
    path: PathBuf,
    offset: u64,
}

impl SpoolReader {
    /// Start reading `path`. Whatever the file already holds was written
    /// before the tab started and is never applied; a file that does not exist
    /// yet is read from its start.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let offset = regular_file_len(&path).unwrap_or(0);
        Self { path, offset }
    }

    /// Apply the records appended since the last poll.
    ///
    /// Malformed, oversized and incomplete lines are skipped, and an
    /// incomplete trailing line waits for its newline. The hook drops a record
    /// that would take the spool past [`SPOOL_BOUND`], so a fully consumed
    /// spool that has less than one maximum record of room left is truncated
    /// under the advisory lock the hook also takes.
    pub fn poll(&mut self) -> Vec<Record> {
        // One handle for the whole poll: the length, the bytes and the file type
        // all come from the file that was opened without following a symlink.
        let Ok(mut file) = made_platform::open_regular_file_nofollow(&self.path, false) else {
            return Vec::new();
        };
        let Ok(len) = file.metadata().map(|meta| meta.len()) else {
            return Vec::new();
        };
        if len < self.offset {
            // Replaced or truncated by someone else: start over.
            self.offset = 0;
        }
        let mut records = Vec::new();
        if len > self.offset {
            if let Some(bytes) = self.read_from_offset(&mut file, len) {
                self.consume(&bytes, &mut records);
            }
        }
        drop(file);
        if self.offset == len && len + MAX_RECORD_BYTES as u64 > SPOOL_BOUND {
            self.truncate_consumed();
        }
        records
    }

    fn read_from_offset(&self, file: &mut File, len: u64) -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut bytes = Vec::new();
        file.take(len - self.offset).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    }

    fn consume(&mut self, bytes: &[u8], records: &mut Vec<Record>) {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        for line in bytes[..complete].split_inclusive(|byte| *byte == b'\n') {
            if line.len() > MAX_RECORD_BYTES {
                continue;
            }
            let Ok(text) = std::str::from_utf8(line) else {
                continue;
            };
            if let Some(record) = parse_record(text.trim_end()) {
                records.push(record);
            }
        }
        let tail = bytes.len() - complete;
        // A tail that already exceeds the record bound can never become a valid
        // record, so skip it instead of waiting on it forever.
        let skipped = if tail > MAX_RECORD_BYTES { tail } else { 0 };
        self.offset += (complete + skipped) as u64;
    }

    fn truncate_consumed(&mut self) {
        let Ok(file) = made_platform::open_regular_file_nofollow(&self.path, true) else {
            return;
        };
        // The hook holds this lock while it appends, so under it the length is
        // stable. Wait for the next poll rather than for the hook.
        if file.try_lock().is_err() {
            return;
        }
        if file.metadata().is_ok_and(|meta| meta.len() == self.offset) && file.set_len(0).is_ok() {
            self.offset = 0;
        }
    }
}

fn regular_file_len(path: &Path) -> Option<u64> {
    made_platform::open_regular_file_nofollow(path, false)
        .ok()?
        .metadata()
        .ok()
        .map(|meta| meta.len())
}

/// A [`SpoolReader`] polled on its own thread.
///
/// The thread wakes every interval, reads new records and queues them; the UI
/// thread only drains the queue. Dropping the watcher stops the thread.
#[derive(Debug)]
pub struct SpoolWatcher {
    receiver: Receiver<Vec<Record>>,
    stop: Arc<AtomicBool>,
    thread: Thread,
    handle: Option<JoinHandle<()>>,
}

impl SpoolWatcher {
    /// Start polling `path` every `interval`. Records already in the file are
    /// not applied.
    pub fn spawn(path: impl Into<PathBuf>, interval: Duration) -> Self {
        let mut reader = SpoolReader::new(path);
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("made-claude-spool".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let records = reader.poll();
                    if !records.is_empty() && sender.send(records).is_err() {
                        break;
                    }
                    thread::park_timeout(interval);
                }
            })
            .ok();
        let thread = handle
            .as_ref()
            .map_or_else(thread::current, |handle| handle.thread().clone());
        Self {
            receiver,
            stop,
            thread,
            handle,
        }
    }

    /// Take every record read since the last call.
    pub fn drain(&self) -> Vec<Record> {
        let mut records = Vec::new();
        while let Ok(batch) = self.receiver.try_recv() {
            records.extend(batch);
        }
        records
    }
}

impl Drop for SpoolWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            self.thread.unpark();
            let _ = handle.join();
        }
    }
}

// ---- spool lifecycle ---------------------------------------------------------

/// This Made instance's spool directory,
/// `<private temp>/claude/<owner-pid>-<instance-id>/`.
///
/// The directory names its owner, and the instance holds an advisory lock on
/// `owner.lock` inside it for as long as it runs. A later start removes
/// another instance's directory only when that lock is free, so a concurrently
/// running instance is never touched and a reused pid cannot keep a dead
/// instance's spools alive.
#[derive(Debug)]
pub struct SpoolInstance {
    root: PathBuf,
    dir: PathBuf,
    _owner: File,
}

impl SpoolInstance {
    /// Create this process's instance directory under `temp_dir`.
    pub fn create(temp_dir: &Path) -> io::Result<Self> {
        Self::create_for(temp_dir, std::process::id(), &random_instance_id())
    }

    /// Create the instance directory for an explicit owner pid and id.
    pub fn create_for(temp_dir: &Path, owner_pid: u32, instance_id: &str) -> io::Result<Self> {
        let root = temp_dir.join(SPOOL_ROOT);
        ensure_private_directory(&root)?;
        let id: String = instance_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        if id.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "instance id has no usable characters",
            ));
        }
        let dir = root.join(format!("{owner_pid}-{id}"));
        create_private_directory(&dir)?;
        // The lock is taken before the file has its final name, so a concurrent
        // start never sees an `owner.lock` that nobody holds yet. The lock
        // belongs to the open file and survives the rename.
        let pending = dir.join(OWNER_LOCK_PENDING);
        let owner = create_private_file(&pending)?;
        owner.lock()?;
        fs::rename(&pending, dir.join(OWNER_LOCK))?;
        Ok(Self {
            root,
            dir,
            _owner: owner,
        })
    }

    /// This instance's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Create the empty, private spool for `tab_id` and return its absolute
    /// path. It fails rather than reuse an existing path.
    pub fn create_spool(&self, tab_id: &str) -> io::Result<PathBuf> {
        let path = self.spool_path(tab_id);
        create_private_file(&path)?;
        Ok(path)
    }

    /// Delete `tab_id`'s spool. A missing spool is not an error.
    pub fn remove_spool(&self, tab_id: &str) {
        let _ = fs::remove_file(self.spool_path(tab_id));
    }

    fn spool_path(&self, tab_id: &str) -> PathBuf {
        let name: String = tab_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{name}.jsonl"))
    }

    /// Remove the spool directories of Made instances that are no longer
    /// running and return how many were removed.
    ///
    /// Their records are never read. Entries that are not instance
    /// directories, this instance's own directory and the directory of any
    /// instance that still holds its lock are left alone.
    pub fn remove_stale(&self) -> usize {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.dir {
                continue;
            }
            let Some(owner_pid) = entry
                .file_name()
                .to_str()
                .and_then(parse_instance_directory_name)
            else {
                continue;
            };
            let is_directory = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
            if is_directory && owner_is_gone(&path, owner_pid) && fs::remove_dir_all(&path).is_ok()
            {
                removed += 1;
            }
        }
        removed
    }
}

/// `<pid>-<id>` to the pid, or `None` for any other name.
fn parse_instance_directory_name(name: &str) -> Option<u32> {
    let (pid, id) = name.split_once('-')?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    pid.parse().ok()
}

/// Whether the instance that owns `dir` has exited.
fn owner_is_gone(dir: &Path, owner_pid: u32) -> bool {
    match File::open(dir.join(OWNER_LOCK)) {
        // The lock is free only when its holder is gone. It is released again
        // when `lock` is dropped, before the directory is removed.
        Ok(lock) => lock.try_lock().is_ok(),
        // An instance that has not yet created its lock is still starting if its
        // pid is alive.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            made_platform::process_parent(owner_pid).is_none()
        }
        Err(_) => false,
    }
}

fn random_instance_id() -> String {
    format!("{:016x}", RandomState::new().build_hasher().finish())
}

/// Whether the host has no owner-only directory primitives in `made-platform`
/// and relies on the per-user temp location instead (Windows).
fn relies_on_per_user_temp() -> bool {
    made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows
}

fn platform_error(error: made_platform::PlatformError) -> io::Error {
    io::Error::other(error.to_string())
}

/// Create the directory if needed and require it to be private to the user.
fn ensure_private_directory(path: &Path) -> io::Result<()> {
    if relies_on_per_user_temp() {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        return if fs::symlink_metadata(path)?.is_dir() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "spool directory is not a directory",
            ))
        };
    }
    made_platform::ensure_private_directory(path).map_err(platform_error)
}

/// Create a new private directory; an existing path is an error.
fn create_private_directory(path: &Path) -> io::Result<()> {
    if relies_on_per_user_temp() {
        return fs::create_dir(path);
    }
    made_platform::create_private_directory_new(path).map_err(platform_error)
}

/// Create a new private file that never follows a symlink; an existing path is
/// an error.
fn create_private_file(path: &Path) -> io::Result<File> {
    if relies_on_per_user_temp() {
        return OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path);
    }
    made_platform::create_private_file_new(path, false).map_err(platform_error)
}

static INSTANCES: Mutex<Vec<(PathBuf, Arc<SpoolInstance>)>> = Mutex::new(Vec::new());

/// This process's [`SpoolInstance`] under `temp_dir`, created on first use.
///
/// The first call under a `temp_dir` also removes the spools of instances that
/// are gone.
pub fn shared_instance(temp_dir: &Path) -> io::Result<Arc<SpoolInstance>> {
    let mut instances = INSTANCES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, instance)) = instances
        .iter()
        .find(|(dir, instance)| dir == temp_dir && instance.dir().is_dir())
    {
        return Ok(Arc::clone(instance));
    }
    let instance = Arc::new(SpoolInstance::create(temp_dir)?);
    instance.remove_stale();
    instances.retain(|(dir, _)| dir != temp_dir);
    instances.push((temp_dir.to_path_buf(), Arc::clone(&instance)));
    Ok(instance)
}

/// One tab's spool: the file, its poller, and deletion when the tab closes.
#[derive(Debug)]
pub struct TabSpool {
    instance: Arc<SpoolInstance>,
    tab_id: String,
    path: PathBuf,
    watcher: SpoolWatcher,
}

impl TabSpool {
    /// Create `tab_id`'s spool in `instance` and start polling it.
    pub fn open(instance: Arc<SpoolInstance>, tab_id: &str) -> io::Result<Self> {
        let path = instance.create_spool(tab_id)?;
        let watcher = SpoolWatcher::spawn(path.clone(), SPOOL_POLL_INTERVAL);
        Ok(Self {
            instance,
            tab_id: tab_id.to_owned(),
            path,
            watcher,
        })
    }

    /// Absolute path handed to the tab's processes as `MADE_CLAUDE_SPOOL`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records read since the last call.
    pub fn drain(&self) -> Vec<Record> {
        self.watcher.drain()
    }
}

impl Drop for TabSpool {
    fn drop(&mut self) {
        self.instance.remove_spool(&self.tab_id);
    }
}

// ---- ownership ---------------------------------------------------------------

/// The liveness anchor of a record's Claude Code process, when this tab owns
/// it.
///
/// `chain` is the hook's ancestry, nearest first. The anchor is the nearest
/// pid in it that is still alive when the record is read and that is the
/// tab's terminal process `root` or descends from it within [`MAX_HOPS`] live
/// parent hops. The hook's own shell is transient and has exited by then, so
/// the anchor is the process that ran the hook: Claude Code itself, whether it
/// sits under nested shells, replaced the shell with `exec`, or is a
/// `claude -p` run by another Claude Code.
///
/// The nearest live pid decides. A live process that does not descend from
/// `root` (another tab, or one reparented away such as under a tmux server)
/// rejects the record rather than letting it fall through to the root. A live
/// `root` is the anchor only when it is the terminal child of a directly
/// launched tab, or when it is at most one hop (the hook shell) from the hook:
/// a shell whose Claude Code has already exited is not Claude Code.
pub fn resolve_anchor(
    root: u32,
    direct: bool,
    chain: &[u32],
    mut parent_of: impl FnMut(u32) -> Option<u32>,
) -> Option<u32> {
    let chain = &chain[..chain.len().min(MAX_HOPS)];
    for (position, pid) in chain.iter().copied().enumerate() {
        if parent_of(pid).is_none() {
            continue;
        }
        if pid == root {
            return (direct || position <= 1).then_some(pid);
        }
        return process_belongs_to_with(pid, root, &mut parent_of).then_some(pid);
    }
    None
}

// ---- per-tab state ------------------------------------------------------------

/// A dot status: the same three states as OpenCode and Grok session dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeActivity {
    Idle,
    Busy,
    Attention,
}

impl From<HookStatus> for ClaudeActivity {
    fn from(status: HookStatus) -> Self {
        match status {
            HookStatus::Working => Self::Busy,
            HookStatus::Attention | HookStatus::Error => Self::Attention,
            HookStatus::Idle | HookStatus::Ended => Self::Idle,
        }
    }
}

impl From<ProtocolState> for ClaudeActivity {
    fn from(state: ProtocolState) -> Self {
        match state {
            ProtocolState::Working => Self::Busy,
            ProtocolState::Blocked | ProtocolState::Error => Self::Attention,
            // A finished turn shows idle.
            ProtocolState::Idle | ProtocolState::Done => Self::Idle,
        }
    }
}

#[derive(Debug, Clone)]
struct Session {
    anchor: u32,
    /// The hook's ancestry as of this session's latest record, nearest first.
    chain: Vec<u32>,
    id: String,
    cwd: String,
    hook: ClaudeActivity,
    title: String,
    /// Orders sessions by their latest start, prompt or directory change.
    recency: u64,
}

/// Claude Code state for one local tab.
#[derive(Debug, Clone)]
pub struct ClaudeTab {
    root: u32,
    direct: bool,
    child_running: bool,
    sessions: Vec<Session>,
    next_recency: u64,
    protocol: Option<ClaudeActivity>,
    status_seen: bool,
    /// Ids of sessions that ended, newest last. Records still in flight after
    /// a session's end are ignored.
    closed: Vec<String>,
}

impl ClaudeTab {
    /// State for a tab whose terminal process is `root`. `direct` is true for a
    /// tab launched from the Claude Code row, whose terminal child is claude.
    pub fn new(root: u32, direct: bool) -> Self {
        Self {
            root,
            direct,
            child_running: false,
            sessions: Vec::new(),
            next_recency: 0,
            protocol: None,
            status_seen: false,
            closed: Vec::new(),
        }
    }

    /// Follow a relaunched terminal process. Sessions of the old process are
    /// dropped by the next liveness check.
    pub fn set_root(&mut self, root: u32) {
        self.root = root;
    }

    /// Whether the terminal process is known. Until it is, no record can be
    /// attributed to this tab.
    pub fn root_known(&self) -> bool {
        self.root != 0
    }

    /// Apply hook records. A record is ignored unless this tab owns the Claude
    /// Code process that ran its hook; see [`resolve_anchor`].
    pub fn ingest(&mut self, records: Vec<Record>, mut parent_of: impl FnMut(u32) -> Option<u32>) {
        if !self.root_known() {
            return;
        }
        for record in records {
            let ended = record.status == Some(HookStatus::Ended);
            if record.event == "SessionStart" {
                // A resumed session reuses its id.
                self.closed.retain(|id| *id != record.session_id);
            } else if !ended && self.closed.contains(&record.session_id) {
                continue;
            }
            let Some(anchor) = resolve_anchor(self.root, self.direct, &record.pids, &mut parent_of)
            else {
                continue;
            };
            // A shell tab's root is the shell. A record that resolves to it
            // came from a Claude Code that has already exited, so it may only
            // update a session that exists, never start one.
            if !self.direct
                && anchor == self.root
                && !self
                    .sessions
                    .iter()
                    .any(|s| s.anchor == anchor && s.id == record.session_id)
            {
                continue;
            }
            if ended {
                self.close(&record.session_id);
            }
            self.reanchor(anchor, &record.session_id, &mut parent_of);
            self.apply(anchor, record);
        }
    }

    fn close(&mut self, session_id: &str) {
        const MAX_CLOSED: usize = 64;
        if self.closed.iter().any(|id| id == session_id) {
            return;
        }
        if self.closed.len() >= MAX_CLOSED {
            self.closed.remove(0);
        }
        self.closed.push(session_id.to_owned());
    }

    /// A record read while its transient hook shell was still running anchors
    /// on that shell. The session's next record resolves to the real process:
    /// move the session there instead of starting a second one.
    fn reanchor(
        &mut self,
        anchor: u32,
        session_id: &str,
        parent_of: &mut impl FnMut(u32) -> Option<u32>,
    ) {
        if self.sessions.iter().any(|s| s.anchor == anchor) {
            return;
        }
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|s| s.id == session_id && parent_of(s.anchor).is_none())
        {
            session.anchor = anchor;
        }
    }

    fn apply(&mut self, anchor: u32, record: Record) {
        let existing = self.sessions.iter().position(|s| s.anchor == anchor);
        if record.status == Some(HookStatus::Ended) {
            // A late end for a session that was already replaced must not end
            // the current one.
            if let Some(index) =
                existing.filter(|index| self.sessions[*index].id == record.session_id)
            {
                self.sessions.remove(index);
            }
            return;
        }
        let index = match existing {
            Some(index) if self.sessions[index].id == record.session_id => index,
            replaced => {
                // One current session per process: a new id replaces the old.
                if let Some(index) = replaced {
                    self.sessions.remove(index);
                }
                self.sessions.push(Session {
                    anchor,
                    chain: Vec::new(),
                    id: record.session_id.clone(),
                    cwd: String::new(),
                    hook: ClaudeActivity::Idle,
                    title: String::new(),
                    recency: 0,
                });
                self.touch_new();
                self.sessions.len() - 1
            }
        };
        let moved = !record.cwd.is_empty() && self.sessions[index].cwd != record.cwd;
        let active = matches!(
            record.event.as_str(),
            "SessionStart" | "UserPromptSubmit" | "CwdChanged"
        );
        let session = &mut self.sessions[index];
        session.chain.clone_from(&record.pids);
        if moved {
            session.cwd = record.cwd;
        }
        if let Some(status) = record.status {
            session.hook = status.into();
        }
        if !record.title.is_empty() {
            session.title = record.title;
        }
        if moved || active {
            self.touch(index);
        }
    }

    fn touch_new(&mut self) {
        let last = self.sessions.len() - 1;
        self.touch(last);
    }

    fn touch(&mut self, index: usize) {
        self.next_recency += 1;
        self.sessions[index].recency = self.next_recency;
    }

    /// Drop sessions whose Claude Code process has exited. Returns whether any
    /// session was dropped.
    ///
    /// A session anchored on a pid that has gone (the transient hook shell
    /// caught by an early record) is first re-resolved from its stored hook
    /// ancestry (when the dead pid was the hook's direct parent), so only a
    /// session with no live process left is dropped.
    pub fn check_liveness(&mut self, mut parent_of: impl FnMut(u32) -> Option<u32>) -> bool {
        let before = self.sessions.len();
        for index in 0..self.sessions.len() {
            if parent_of(self.sessions[index].anchor).is_some() {
                continue;
            }
            let chain = self.sessions[index].chain.clone();
            // Only the hook's direct parent can be the transient hook shell. A
            // session anchored deeper in the chain lost its real process.
            if chain.first() != Some(&self.sessions[index].anchor) {
                continue;
            }
            let Some(anchor) = resolve_anchor(self.root, self.direct, &chain, &mut parent_of)
            else {
                continue;
            };
            let free = !self.sessions.iter().any(|s| s.anchor == anchor);
            if free && (self.direct || anchor != self.root) {
                self.sessions[index].anchor = anchor;
            }
        }
        self.sessions
            .retain(|session| parent_of(session.anchor).is_some());
        self.sessions.len() != before
    }

    /// Whether the tab's directly launched Claude Code child is running. A
    /// child that exited also ends protocol-only presence.
    pub fn set_child_running(&mut self, running: bool) {
        self.child_running = running;
        if !running {
            self.protocol = None;
        }
    }

    /// Apply a Program Status Protocol event from this tab's terminal.
    pub fn note_protocol(&mut self, event: ProtocolEvent) {
        match event {
            // The capability probe is not presence and is not tracked.
            ProtocolEvent::Probe => {}
            ProtocolEvent::Status { state } => {
                self.status_seen = true;
                self.protocol = Some(state.into());
            }
            // A bare clear is only meaningful while a status is live, and ends
            // it exactly as a clear carrying the app does.
            ProtocolEvent::Clear { .. } => self.protocol = None,
        }
    }

    /// The tab's launch-local shell integration reported the shell prompt:
    /// Claude Code is no longer running through the status protocol alone.
    pub fn note_shell_prompt(&mut self) {
        self.protocol = None;
    }

    /// Apply one frame's terminal reports: the protocol events first, then the
    /// shell-prompt report. The prompt is drawn after Claude Code exits, so
    /// when both arrive in one frame it is the later fact; applying it first
    /// would let a status emitted before the exit outlive it.
    pub fn apply_terminal_reports(
        &mut self,
        events: impl IntoIterator<Item = ProtocolEvent>,
        prompt_reported: bool,
    ) {
        for event in events {
            self.note_protocol(event);
        }
        if prompt_reported {
            self.note_shell_prompt();
        }
    }

    /// Whether Claude Code is present in this tab.
    pub fn present(&self) -> bool {
        (self.direct && self.child_running) || !self.sessions.is_empty() || self.protocol.is_some()
    }

    /// Forget which channels were observed. Called when Claude Code is no
    /// longer present, so the next Claude Code in this tab starts clean.
    pub fn reset_observations(&mut self) {
        self.status_seen = false;
    }

    /// Whether a session-level protocol status is live.
    pub fn protocol_live(&self) -> bool {
        self.protocol.is_some()
    }

    /// Whether at least one session-level status emission arrived.
    pub fn protocol_status_seen(&self) -> bool {
        self.status_seen
    }

    /// Number of live owned sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Whether `session` runs under another live session of this tab, as a
    /// `claude -p` started by Claude Code does.
    fn is_nested(&self, session: &Session) -> bool {
        let Some(position) = session.chain.iter().position(|pid| *pid == session.anchor) else {
            return false;
        };
        session.chain[position + 1..].iter().any(|pid| {
            self.sessions
                .iter()
                .any(|other| other.anchor == *pid && other.anchor != session.anchor)
        })
    }

    /// The tab's current session: the most recent one that does not run under
    /// another session, so a nested `claude -p` never takes the tab over.
    fn latest(&self) -> Option<usize> {
        let pick = |nested: bool| {
            self.sessions
                .iter()
                .enumerate()
                .filter(|(_, session)| self.is_nested(session) == nested)
                .max_by_key(|(_, session)| session.recency)
                .map(|(index, _)| index)
        };
        pick(false).or_else(|| pick(true))
    }

    /// The working directory of the most recent live owned session, exactly as
    /// the hook reported it. Callers validate it before use.
    pub fn directory(&self) -> Option<String> {
        let session = &self.sessions[self.latest()?];
        (!session.cwd.is_empty()).then(|| session.cwd.clone())
    }

    /// One status per live owned session, in attachment order, or exactly one
    /// when Claude Code is present only through a protocol status. The protocol
    /// status applies to the most recent session; every other session keeps its
    /// hook-derived status.
    pub fn dots(&self) -> Vec<ClaudeActivity> {
        let Some(latest) = self.latest() else {
            return self.protocol.into_iter().collect();
        };
        self.sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                if index == latest {
                    self.protocol.unwrap_or(session.hook)
                } else {
                    session.hook
                }
            })
            .collect()
    }

    /// The tooltip title: the terminal's window title, read only while a
    /// protocol status is live, then the most recent session's hook title,
    /// then `Claude Code`. Titles are untrusted: sanitized and capped.
    pub fn tooltip_title(&self, window_title: Option<&str>) -> String {
        if self.protocol.is_some() {
            if let Some(title) = window_title {
                let title = sanitize_display(title, MAX_TITLE_CHARS);
                let title = title.trim();
                if !title.is_empty() {
                    return title.to_owned();
                }
            }
        }
        self.latest()
            .map(|index| self.sessions[index].title.as_str())
            .filter(|title| !title.is_empty())
            .map_or_else(|| DEFAULT_TITLE.to_owned(), str::to_owned)
    }
}

impl From<ClaudeActivity> for crate::session_map::SessionActivity {
    fn from(activity: ClaudeActivity) -> Self {
        match activity {
            ClaudeActivity::Idle => Self::Idle,
            ClaudeActivity::Busy => Self::Busy,
            ClaudeActivity::Attention => Self::Attention,
        }
    }
}
