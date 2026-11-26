//! Claude Code helpers shared by the local integration and the companion, and
//! the companion's server side of remote Claude Code (SSH-11).
//!
//! The pure pieces both sides need come first: hook record parsing, the
//! `CLAUDE_CODE_PLUGIN_DIRS` composition, `claude agents --json` listing
//! parsing and window-title normalization. Records and listings are metadata
//! only: every other field of a hook line or a listing entry is skipped
//! without being copied, and nothing logs.
//!
//! The companion side composes the launch environment from server-staged
//! paths, owns the per-session spool inside the supervisor runtime state,
//! ingests it with live process-ancestry ownership, and runs the coordinated
//! per-server-user listing. It opens only the staged package, the tab's spool
//! and the app-owned listing coordination files, never anything under the
//! remote user's `~/.claude`.

use crate::sanitize::sanitize_display;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use made_platform::Environment;
use serde::{Deserialize, Serialize};

use crate::protocol::{
    ClaudeActivity, ClaudeBackgroundEntry, ClaudeListingResult, ClaudeSessionInfo,
    ClaudeSessionSnapshot, LaunchTarget, Operation, RemotePath, MAX_CLAUDE_BACKGROUND,
    MAX_FRAME_BYTES,
};

/// Most parent hops from a Claude Code process to its tab's terminal process.
pub const MAX_HOPS: usize = 32;

/// Size the hook never lets a spool exceed. A fully consumed spool with less
/// than [`MAX_RECORD_BYTES`] of room below it is truncated.
pub const SPOOL_BOUND: u64 = 1024 * 1024;

/// Largest record, newline included, that is applied.
pub const MAX_RECORD_BYTES: usize = 4 * 1024;

/// Longest title shown, in characters.
pub const MAX_TITLE_CHARS: usize = 120;

/// Environment variable naming a tab's Claude Code hook spool.
pub const CLAUDE_SPOOL_ENV: &str = "MADE_CLAUDE_SPOOL";

/// Claude Code's session-only plugin directory list variable.
pub const CLAUDE_CODE_PLUGIN_DIRS_ENV: &str = "CLAUDE_CODE_PLUGIN_DIRS";

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

/// Parse one complete spool line as read, newline included. A line over
/// [`MAX_RECORD_BYTES`], or one that is not UTF-8, is `None`.
pub fn parse_record_line(line: &[u8]) -> Option<Record> {
    if line.len() > MAX_RECORD_BYTES {
        return None;
    }
    parse_record(std::str::from_utf8(line).ok()?.trim_end())
}

// ---- plugin list -------------------------------------------------------------

/// Whether `path` may be written into the plugin list: absolute, and free of
/// the list separator, quotes, backtick, `$`, newlines and, on POSIX (a `:`
/// separator), backslash, so no consumer can split or expand it.
pub fn plugin_entry_is_safe(path: &Path, separator: char) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && !text
            .chars()
            .any(|c| c == separator || matches!(c, '"' | '`' | '$' | '\n' | '\r'))
        && !(separator == ':' && text.contains('\\'))
}

/// Compose the [`CLAUDE_CODE_PLUGIN_DIRS_ENV`] value for a Made-started process.
///
/// Every existing entry is kept byte-for-byte in its original order and
/// Made's `shipped` entry is appended, unless an entry already equals it
/// (ignoring trailing slashes). An absent or empty `existing` value yields
/// just `shipped`, with no leading separator. A `shipped` path that is not
/// safe (see [`plugin_entry_is_safe`]) adds nothing: the result is `existing`
/// unchanged, or empty. `separator` must be ASCII.
pub fn compose_plugin_dirs(existing: Option<&OsStr>, shipped: &Path, separator: char) -> OsString {
    debug_assert!(separator.is_ascii());
    let safe = plugin_entry_is_safe(shipped, separator);
    let shipped = shipped.as_os_str();
    let Some(existing) = existing.filter(|value| !value.is_empty()) else {
        return if safe {
            shipped.to_owned()
        } else {
            OsString::new()
        };
    };
    if !safe {
        return existing.to_owned();
    }
    let separator = separator as u8;
    let bytes = existing.as_encoded_bytes();
    // A `;` list is a Windows one: both slashes end an entry and paths compare
    // case-insensitively.
    let windows = separator == b';';
    let trimmed = |entry: &[u8]| {
        let end = entry
            .iter()
            .rposition(|byte| *byte != b'/' && !(windows && *byte == b'\\'))
            .map_or(0, |i| i + 1);
        let mut entry = entry[..end].to_vec();
        if windows {
            entry.make_ascii_lowercase();
        }
        entry
    };
    let wanted = trimmed(shipped.as_encoded_bytes());
    if bytes
        .split(|byte| *byte == separator)
        .any(|entry| trimmed(entry) == wanted)
    {
        return existing.to_owned();
    }
    let mut composed = existing.to_owned();
    if bytes.last() != Some(&separator) {
        composed.push(char::from(separator).to_string());
    }
    composed.push(shipped);
    composed
}

// ---- agents listing ------------------------------------------------------------

/// One running Claude Code session as `claude agents --json` reports it.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentCwd {
    pub pid: u32,
    /// Untrusted and unvalidated, exactly like a hook record's `cwd`.
    pub cwd: String,
    /// A background (agent-view) session: matched only by name, never by pid.
    pub background: bool,
    /// The session name normalized by [`normalize_title`]; kept only for
    /// background sessions. Untrusted display text, never logged.
    pub name: String,
}

/// Leaves out the untrusted `cwd` and `name`.
impl fmt::Debug for AgentCwd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentCwd")
            .field("pid", &self.pid)
            .field("background", &self.background)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct RawAgent {
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// Parse `claude agents --json` output, or `None` when it is not a JSON array.
/// Entries that are malformed or lack a pid or a directory are dropped.
pub fn parse_agents(json: &[u8]) -> Option<Vec<AgentCwd>> {
    let raw: Vec<serde_json::Value> = serde_json::from_slice(json).ok()?;
    Some(
        raw.into_iter()
            .filter_map(|value| serde_json::from_value::<RawAgent>(value).ok())
            .filter_map(|agent| {
                let pid = agent.pid.filter(|pid| *pid != 0)?;
                let cwd = agent.cwd.filter(|cwd| !cwd.is_empty())?;
                let background = agent.kind.as_deref() == Some("background");
                let name = match agent.name.as_deref() {
                    Some(name) if background => normalize_title(name),
                    _ => String::new(),
                };
                Some(AgentCwd {
                    pid,
                    cwd,
                    background,
                    name,
                })
            })
            .collect(),
    )
}

/// The directory `entries` report for the Claude Code process `pid`. A
/// background session never matches by pid.
pub fn cwd_for_pid(entries: &[AgentCwd], pid: u32) -> Option<&str> {
    entries
        .iter()
        .find(|entry| !entry.background && entry.pid == pid)
        .map(|entry| entry.cwd.as_str())
}

/// Whether `c` is a status glyph Claude Code may put before its window title:
/// a Braille spinner frame (U+2800 to U+28FF), a Dingbats star or asterisk
/// (U+2700 to U+27BF, such as `✳` and `✻`), a middle dot or a bullet.
fn is_title_glyph(c: char) -> bool {
    matches!(c, '\u{2800}'..='\u{28FF}' | '\u{2700}'..='\u{27BF}' | '·' | '•' | '●')
}

/// A window title or session name reduced to the text that is compared:
/// sanitized and capped exactly as the tooltip title is, then every leading
/// status glyph and whitespace character removed, then trailing whitespace
/// removed.
pub fn normalize_title(text: &str) -> String {
    sanitize_display(text, MAX_TITLE_CHARS)
        .trim_start_matches(|c: char| c.is_whitespace() || is_title_glyph(c))
        .trim_end()
        .to_owned()
}

/// The directory of the one background session whose name equals `name`, a
/// window title already normalized by [`normalize_title`]. An empty name, no
/// match or more than one background session with that name gives `None`;
/// interactive sessions never match.
pub fn background_cwd_for_name<'a>(entries: &'a [AgentCwd], name: &str) -> Option<&'a str> {
    if name.is_empty() {
        return None;
    }
    let mut matches = entries
        .iter()
        .filter(|entry| entry.background && entry.name == name);
    let first = matches.next()?;
    matches.next().is_none().then_some(first.cwd.as_str())
}

// ---- companion: launch environment ---------------------------------------------

/// The staged claude-code-made package beside the companion executable.
pub const STAGED_PACKAGE: &str = "plugins/claude-code-made";

/// The staged package's native session-event hook, relative to the package.
const STAGED_HOOK: &str = "bin/made-claude-status";

/// Directory of the tab's spool inside its supervisor runtime state.
const SPOOL_DIRECTORY: &str = "claude";

/// The supervisor's own directory inside [`SPOOL_DIRECTORY`]. Its
/// `<pid>-<id>` name is the spool shape the shipped hook writes to.
const SPOOL_OWNER_DIRECTORY: &str = "0-supervisor";

/// The tab's spool file inside [`SPOOL_OWNER_DIRECTORY`].
const SPOOL_FILE: &str = "spool.jsonl";

/// The lock a live supervisor holds in its spool directory.
const OWNER_LOCK: &str = "owner.lock";

/// Name the lock file has until it is held.
const OWNER_LOCK_PENDING: &str = "owner.lock.pending";

/// The staged claude-code-made package beside `companion`, when it is a
/// readable directory whose hook is an executable file and whose path can be
/// written into the plugin list safely. `None` adds no plugin entry.
pub fn staged_plugin_dir(companion: &Path) -> Option<PathBuf> {
    let dir = companion.parent()?.join(STAGED_PACKAGE);
    if !plugin_entry_is_safe(&dir, ':') {
        return None;
    }
    note_access(&dir);
    fs::read_dir(&dir).ok()?;
    made_platform::is_executable_file(&dir.join(STAGED_HOOK)).then_some(dir)
}

/// The spool path bound for the tab whose supervisor runtime state is
/// `runtime`.
pub fn spool_path(runtime: &Path) -> PathBuf {
    runtime
        .join(SPOOL_DIRECTORY)
        .join(SPOOL_OWNER_DIRECTORY)
        .join(SPOOL_FILE)
}

/// The Claude Code environment a remote tab's terminal child receives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeLaunchEnv {
    /// Variables to set on the child. Empty when the target gets none.
    pub env: Vec<(OsString, OsString)>,
    /// Whether the staged plugin entry and the spool binding were added.
    pub hooks_available: bool,
}

/// Compose a remote launch's Claude Code environment on the server.
///
/// Shell and Claude Code targets get the staged package's entry appended to
/// the remote user's own `CLAUDE_CODE_PLUGIN_DIRS` (read from `environment`
/// only, never from the host) and the tab's spool binding. OpenCode and Grok
/// targets get neither, and a missing or unready `staged` package adds
/// nothing. Whether claude was detected on the server does not matter: a
/// shell-typed claude loads the plugin too.
pub fn launch_environment(
    target: LaunchTarget,
    staged: Option<&Path>,
    environment: &Environment,
    spool: &Path,
) -> ClaudeLaunchEnv {
    if !matches!(target, LaunchTarget::Shell | LaunchTarget::Claude) {
        return ClaudeLaunchEnv::default();
    }
    let Some(staged) = staged else {
        return ClaudeLaunchEnv::default();
    };
    let composed = compose_plugin_dirs(environment.get(CLAUDE_CODE_PLUGIN_DIRS_ENV), staged, ':');
    if composed.is_empty() {
        return ClaudeLaunchEnv::default();
    }
    ClaudeLaunchEnv {
        env: vec![
            (OsString::from(CLAUDE_CODE_PLUGIN_DIRS_ENV), composed),
            (
                OsString::from(CLAUDE_SPOOL_ENV),
                spool.as_os_str().to_owned(),
            ),
        ],
        hooks_available: true,
    }
}

// ---- companion: spool lifecycle -------------------------------------------------

/// One tab's spool, owned by its supervisor:
/// `<runtime>/claude/0-supervisor/spool.jsonl`.
///
/// The directories are created with mode 0700 and the file with mode 0600,
/// none following a symlink, and the supervisor holds an advisory lock on
/// `0-supervisor/owner.lock` for as long as it runs. Dropping it removes the
/// directory, so owned-session teardown and lease expiry end it; a Claude
/// Code session ending does not.
#[derive(Debug)]
pub struct ClaudeSpool {
    dir: PathBuf,
    path: PathBuf,
    _owner: fs::File,
}

impl ClaudeSpool {
    /// Create the spool inside `runtime`. It fails rather than reuse or
    /// follow an existing path.
    pub fn create(runtime: &Path) -> io::Result<Self> {
        let dir = runtime.join(SPOOL_DIRECTORY);
        note_access(&dir);
        made_platform::create_private_directory_new(&dir).map_err(platform_error)?;
        let result = (|| {
            let owner_dir = dir.join(SPOOL_OWNER_DIRECTORY);
            made_platform::create_private_directory_new(&owner_dir).map_err(platform_error)?;
            // The lock is taken before the file has its final name, so a sweep
            // never sees an `owner.lock` that nobody holds yet.
            let pending = owner_dir.join(OWNER_LOCK_PENDING);
            let owner =
                made_platform::create_private_file_new(&pending, false).map_err(platform_error)?;
            owner.lock()?;
            fs::rename(&pending, owner_dir.join(OWNER_LOCK))?;
            let path = owner_dir.join(SPOOL_FILE);
            made_platform::create_private_file_new(&path, false).map_err(platform_error)?;
            Ok((owner, path))
        })();
        match result {
            Ok((owner, path)) => Ok(Self {
                dir,
                path,
                _owner: owner,
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(&dir);
                Err(error)
            }
        }
    }

    /// The spool path bound as `MADE_CLAUDE_SPOOL`.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ClaudeSpool {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Remove the spools of supervisors that are no longer running under
/// `runtime_root` (`<data>/remote/runtime`) and return how many were removed.
///
/// Only a real `<tab>/claude` directory whose `0-supervisor/owner.lock` exists
/// and is free
/// is removed; a live supervisor holds its lock, and a spool directory with no
/// lock yet is still being created. Nothing is followed through a symlink and
/// no record is read.
pub fn sweep_stale_spools(runtime_root: &Path) -> usize {
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let dir = entry.path().join(SPOOL_DIRECTORY);
        let real_directory = |path: &Path| {
            fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.is_symlink())
        };
        if !real_directory(&entry.path()) || !real_directory(&dir) {
            continue;
        }
        let owner_dir = dir.join(SPOOL_OWNER_DIRECTORY);
        if !real_directory(&owner_dir) {
            continue;
        }
        let Ok(lock) =
            made_platform::open_regular_file_nofollow(&owner_dir.join(OWNER_LOCK), false)
        else {
            continue;
        };
        // The lock is free only when its holder is gone. It is released again
        // when `lock` is dropped, before the directory is removed.
        if lock.try_lock().is_ok() {
            drop(lock);
            if fs::remove_dir_all(&dir).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

fn platform_error(error: made_platform::PlatformError) -> io::Error {
    io::Error::other(error.to_string())
}

// ---- companion: file-access audit ---------------------------------------------

/// Paths the companion's Claude Code code opened while an audit is enabled.
static AUDIT: Mutex<(usize, Vec<PathBuf>)> = Mutex::new((0, Vec::new()));

/// Most distinct paths one audit keeps.
const AUDIT_MAX_PATHS: usize = 1024;

fn note_access(path: &Path) {
    let mut audit = AUDIT.lock().unwrap_or_else(PoisonError::into_inner);
    let (enabled, paths) = &mut *audit;
    if *enabled > 0 && paths.len() < AUDIT_MAX_PATHS && !paths.iter().any(|seen| seen == path) {
        paths.push(path.to_path_buf());
    }
}

/// Test seam for the privacy audit: `true` starts recording every path the
/// spool, tracker and listing worker open (or execute); `false` stops one
/// recording and returns what was recorded since the first one started.
#[doc(hidden)]
pub fn audit_file_access(enable: bool) -> Vec<PathBuf> {
    let mut audit = AUDIT.lock().unwrap_or_else(PoisonError::into_inner);
    let (enabled, paths) = &mut *audit;
    if enable {
        if *enabled == 0 {
            paths.clear();
        }
        *enabled += 1;
        Vec::new()
    } else {
        *enabled = enabled.saturating_sub(1);
        paths.clone()
    }
}

// ---- companion: spool reading ----------------------------------------------------

/// Spool bytes read per tick; the rest waits for the next tick.
const SPOOL_MAX_READ_BYTES: u64 = 256 * 1024;

/// Incremental, bounded reader of one tab's spool.
struct SpoolTail {
    path: PathBuf,
    offset: u64,
}

impl SpoolTail {
    /// Whatever the file already holds was written before the managed session
    /// started and is never applied.
    fn new(path: PathBuf) -> Self {
        note_access(&path);
        let offset = made_platform::open_regular_file_nofollow(&path, false)
            .and_then(|file| file.metadata())
            .map_or(0, |meta| meta.len());
        Self { path, offset }
    }

    /// The complete records appended since the last call. Malformed, oversized
    /// and incomplete lines are skipped; a torn trailing line waits.
    fn poll(&mut self) -> Vec<Record> {
        note_access(&self.path);
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
        if len > self.offset && file.seek(SeekFrom::Start(self.offset)).is_ok() {
            let mut bytes = Vec::new();
            let budget = SPOOL_MAX_READ_BYTES.min(len - self.offset);
            if (&mut file).take(budget).read_to_end(&mut bytes).is_ok() {
                let complete = bytes
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |index| index + 1);
                records.extend(
                    bytes[..complete]
                        .split_inclusive(|byte| *byte == b'\n')
                        .filter_map(parse_record_line),
                );
                let tail = bytes.len() - complete;
                // A tail that already exceeds the record bound can never become
                // a valid record.
                let skipped = if tail > MAX_RECORD_BYTES { tail } else { 0 };
                self.offset += (complete + skipped) as u64;
            }
        }
        drop(file);
        if self.offset == len && len + MAX_RECORD_BYTES as u64 > SPOOL_BOUND {
            self.truncate_consumed();
        }
        records
    }

    fn truncate_consumed(&mut self) {
        let Ok(file) = made_platform::open_regular_file_nofollow(&self.path, true) else {
            return;
        };
        // The hook holds this lock while it appends.
        if file.try_lock().is_err() {
            return;
        }
        if file.metadata().is_ok_and(|meta| meta.len() == self.offset) && file.set_len(0).is_ok() {
            self.offset = 0;
        }
    }
}

// ---- companion: owned sessions ---------------------------------------------------

/// How often the tab's spool is read for new records.
const SPOOL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How often owned Claude Code processes are checked for exit.
const LIVENESS_INTERVAL: Duration = Duration::from_millis(250);

/// How often a published listing is looked for while sessions are owned.
const LISTING_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// The liveness anchor of a record's Claude Code process, when the tab whose
/// managed terminal process is `root` owns it: the nearest live pid of the
/// hook's ancestry `chain`, accepted only when it is `root` (for a direct
/// launch, or at most one hop from the hook) or descends from it within
/// [`MAX_HOPS`] live parent hops. The local tab's rule, on the server.
fn resolve_anchor(
    root: u32,
    direct: bool,
    chain: &[u32],
    parent_of: &mut dyn FnMut(u32) -> Option<u32>,
) -> Option<u32> {
    let chain = &chain[..chain.len().min(MAX_HOPS)];
    for (position, pid) in chain.iter().copied().enumerate() {
        if parent_of(pid).is_none() {
            continue;
        }
        if pid == root {
            return (direct || position <= 1).then_some(pid);
        }
        return belongs_to(pid, root, parent_of).then_some(pid);
    }
    None
}

/// Whether `pid` descends from `root` through the live parent chain.
fn belongs_to(mut pid: u32, root: u32, parent_of: &mut dyn FnMut(u32) -> Option<u32>) -> bool {
    for _ in 0..MAX_HOPS {
        let Some(parent) = parent_of(pid) else {
            return false;
        };
        if parent == root {
            return true;
        }
        if parent <= 1 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

fn activity(status: HookStatus) -> ClaudeActivity {
    match status {
        HookStatus::Working => ClaudeActivity::Busy,
        HookStatus::Attention | HookStatus::Error => ClaudeActivity::Attention,
        HookStatus::Idle | HookStatus::Ended => ClaudeActivity::Idle,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// A record or listing directory as a remote path, or `None` when it is not
/// an absolute, bounded POSIX path.
fn remote_dir(cwd: &str) -> Option<RemotePath> {
    RemotePath::new(cwd.as_bytes().to_vec()).ok()
}

#[derive(Clone)]
struct Session {
    anchor: u32,
    chain: Vec<u32>,
    id: String,
    cwd: Option<RemotePath>,
    /// Wall-clock milliseconds of the last hook record that changed `cwd`; a
    /// listing counts only when its run started after this.
    hook_moved_at: Option<u64>,
    hook: ClaudeActivity,
    title: String,
    recency: u64,
}

/// The tab's live owned sessions: the local tab's session rules (one current
/// session per process, ended sessions ignored, transient hook shells
/// re-anchored, counted listings), evaluated on the server.
#[derive(Default)]
struct OwnedSessions {
    sessions: Vec<Session>,
    next_recency: u64,
    closed: Vec<String>,
    /// The directory the latest counted listing gave for each owned pid and the
    /// tab's own trigger it answered.
    listed: Vec<(u32, RemotePath, Option<u64>)>,
}

impl OwnedSessions {
    fn ingest(
        &mut self,
        root: u32,
        direct: bool,
        records: Vec<Record>,
        parent_of: &mut dyn FnMut(u32) -> Option<u32>,
    ) {
        for record in records {
            let ended = record.status == Some(HookStatus::Ended);
            if record.event == "SessionStart" {
                // A resumed session reuses its id.
                self.closed.retain(|id| *id != record.session_id);
            } else if !ended && self.closed.contains(&record.session_id) {
                continue;
            }
            let Some(anchor) = resolve_anchor(root, direct, &record.pids, parent_of) else {
                continue;
            };
            // A shell tab's root is the shell: a record resolving to it came
            // from a Claude Code that already exited and may only update a
            // session that exists.
            if !direct
                && anchor == root
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
            self.reanchor(anchor, &record.session_id, parent_of);
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

    fn reanchor(
        &mut self,
        anchor: u32,
        session_id: &str,
        parent_of: &mut dyn FnMut(u32) -> Option<u32>,
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
                if self.sessions.len() >= made_bridge::state::MAX_SESSIONS {
                    return;
                }
                self.sessions.push(Session {
                    anchor,
                    chain: Vec::new(),
                    id: record.session_id.clone(),
                    cwd: None,
                    hook_moved_at: None,
                    hook: ClaudeActivity::Idle,
                    title: String::new(),
                    recency: 0,
                });
                let last = self.sessions.len() - 1;
                self.touch(last);
                last
            }
        };
        let cwd = remote_dir(&record.cwd);
        let moved = cwd.is_some() && self.sessions[index].cwd != cwd;
        let active = matches!(
            record.event.as_str(),
            "SessionStart" | "UserPromptSubmit" | "CwdChanged"
        );
        let session = &mut self.sessions[index];
        session.chain.clone_from(&record.pids);
        if moved {
            session.cwd = cwd;
            session.hook_moved_at = Some(now_ms());
        }
        if let Some(status) = record.status {
            session.hook = activity(status);
        }
        if !record.title.is_empty() {
            session.title = record.title;
        }
        if moved || active {
            self.touch(index);
        }
    }

    fn touch(&mut self, index: usize) {
        self.next_recency += 1;
        self.sessions[index].recency = self.next_recency;
    }

    /// The local counted-listing rule: a listing applies to an owned pid only
    /// when its run started (`observed_ms`) after the last hook move, and then
    /// at most once per trigger of this tab (`own_trigger`).
    fn apply_listing(&mut self, entries: &[AgentCwd], observed_ms: u64, own_trigger: Option<u64>) {
        let sessions = &self.sessions;
        self.listed
            .retain(|(pid, _, _)| sessions.iter().any(|session| session.anchor == *pid));
        for index in 0..self.sessions.len() {
            let anchor = self.sessions[index].anchor;
            let Some(cwd) = cwd_for_pid(entries, anchor).and_then(remote_dir) else {
                continue;
            };
            if self.sessions[index]
                .hook_moved_at
                .is_some_and(|at| observed_ms <= at)
            {
                continue;
            }
            let answers = own_trigger.filter(|own| observed_ms >= *own);
            let changed = match self.listed.iter_mut().find(|(pid, _, _)| *pid == anchor) {
                Some((_, seen, answered))
                    if *seen == cwd && (answers.is_none() || *answered == answers) =>
                {
                    false
                }
                Some((_, seen, answered)) => {
                    seen.clone_from(&cwd);
                    if answers.is_some() {
                        *answered = answers;
                    }
                    true
                }
                None => {
                    self.listed.push((anchor, cwd.clone(), answers));
                    true
                }
            };
            if !changed || self.sessions[index].cwd.as_ref() == Some(&cwd) {
                continue;
            }
            self.sessions[index].cwd = Some(cwd);
            self.touch(index);
        }
    }

    /// Drop sessions whose Claude Code process exited, re-resolving one that
    /// was anchored on its transient hook shell first. Returns whether any
    /// session was dropped or moved.
    fn check_liveness(
        &mut self,
        root: u32,
        direct: bool,
        parent_of: &mut dyn FnMut(u32) -> Option<u32>,
    ) -> bool {
        let before: Vec<u32> = self.sessions.iter().map(|s| s.anchor).collect();
        for index in 0..self.sessions.len() {
            if parent_of(self.sessions[index].anchor).is_some() {
                continue;
            }
            let chain = self.sessions[index].chain.clone();
            if chain.first() != Some(&self.sessions[index].anchor) {
                continue;
            }
            let Some(anchor) = resolve_anchor(root, direct, &chain, parent_of) else {
                continue;
            };
            let free = !self.sessions.iter().any(|s| s.anchor == anchor);
            if free && (direct || anchor != root) {
                self.sessions[index].anchor = anchor;
            }
        }
        self.sessions
            .retain(|session| parent_of(session.anchor).is_some());
        let after: Vec<u32> = self.sessions.iter().map(|s| s.anchor).collect();
        before != after
    }

    /// Whether `session` runs under another live session of this tab.
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

    fn infos(&self) -> Vec<ClaudeSessionInfo> {
        self.sessions
            .iter()
            .filter(|session| {
                session.id.chars().count() <= made_bridge::state::MAX_SESSION_ID_CHARS
                    && !session.id.chars().any(char::is_control)
            })
            .map(|session| ClaudeSessionInfo {
                session_id: session.id.clone(),
                pid: session.anchor,
                cwd: session.cwd.clone(),
                activity: session.hook,
                title: session.title.clone(),
                recency: session.recency,
                nested: self.is_nested(session),
            })
            .collect()
    }
}

// ---- companion: tracker ----------------------------------------------------------

/// Parent lookup injected for tests; the companion uses live process state.
pub type ParentOf = dyn FnMut(u32) -> Option<u32> + Send;

/// Companion-side Claude Code state for one tab: spool ingestion, ownership
/// and liveness against the tab's managed process tree, counted listings, and
/// the revisioned owned-session snapshot. Only metadata leaves it.
pub struct ClaudeTracker {
    root: Option<u32>,
    direct: bool,
    spool: Option<SpoolTail>,
    worker: Option<ListingWorker>,
    parent_of: Box<ParentOf>,
    sessions: OwnedSessions,
    own_trigger: Option<u64>,
    applied_listing: Option<u64>,
    pending_listing: Option<Vec<ClaudeBackgroundEntry>>,
    last_spool_poll: Option<Instant>,
    last_liveness: Option<Instant>,
    last_listing_check: Option<Instant>,
    revision: u64,
    /// The listings' own revision sequence, so a listing is never compared
    /// with a snapshot the client may apply first.
    listing_revision: u64,
    /// Last pushed (hooks available, sessions); `None` after re-attach.
    pushed: Option<Vec<ClaudeSessionInfo>>,
}

impl ClaudeTracker {
    /// State for the tab whose managed terminal process is `root`. `direct`
    /// is true for a `Claude Code SSH` launch. `spool` is the bound spool, or
    /// `None` when hooks are unavailable for the tab.
    pub fn new(
        root: Option<u32>,
        direct: bool,
        spool: Option<PathBuf>,
        worker: Option<ListingWorker>,
    ) -> Self {
        Self::with_parent(
            root,
            direct,
            spool,
            worker,
            Box::new(made_platform::process_parent),
        )
    }

    pub fn with_parent(
        root: Option<u32>,
        direct: bool,
        spool: Option<PathBuf>,
        worker: Option<ListingWorker>,
        parent_of: Box<ParentOf>,
    ) -> Self {
        Self {
            root: root.filter(|pid| *pid != 0),
            direct,
            spool: spool.map(SpoolTail::new),
            worker,
            parent_of,
            sessions: OwnedSessions::default(),
            own_trigger: None,
            applied_listing: None,
            pending_listing: None,
            last_spool_poll: None,
            last_liveness: None,
            last_listing_check: None,
            revision: 0,
            listing_revision: 0,
            pushed: None,
        }
    }

    /// Whether the staged package and the spool are bound for this tab.
    pub fn hooks_available(&self) -> bool {
        self.spool.is_some()
    }

    /// A new attachment discards revision history so the client applies a
    /// fresh snapshot. Records are never replayed: the spool offset survives.
    pub fn reset_attachment(&mut self) {
        self.revision = 0;
        self.listing_revision = 0;
        self.pushed = None;
        self.pending_listing = None;
    }

    /// A client-detected trigger: schedule a listing burst while the tab owns
    /// a live session. Nothing runs when claude was not detected on the server.
    pub fn trigger(&mut self) {
        // Only a tab that owns a live session lists anything.
        if self.sessions.sessions.is_empty() {
            return;
        }
        if let Some(burst) = self.worker.as_ref().and_then(ListingWorker::trigger) {
            self.own_trigger = Some(burst);
        }
    }

    /// One supervisor-loop pass. Ingestion, liveness and listings apply
    /// whether or not a client is attached; changes are returned as pushes
    /// only when `deliver` is true, so a detached tab keeps its state and a
    /// reattached one gets a fresh snapshot instead of a replay.
    pub fn tick(&mut self, now: Instant, deliver: bool) -> Vec<Operation> {
        if let Some(root) = self.root {
            let poll = self
                .last_spool_poll
                .is_none_or(|last| now.saturating_duration_since(last) >= SPOOL_POLL_INTERVAL);
            let records = if poll {
                self.last_spool_poll = Some(now);
                self.spool.as_mut().map(SpoolTail::poll).unwrap_or_default()
            } else {
                Vec::new()
            };
            if !records.is_empty() {
                self.sessions
                    .ingest(root, self.direct, records, &mut *self.parent_of);
            }
            let due = self
                .last_liveness
                .is_none_or(|last| now.saturating_duration_since(last) >= LIVENESS_INTERVAL);
            if due {
                self.last_liveness = Some(now);
                self.sessions
                    .check_liveness(root, self.direct, &mut *self.parent_of);
            }
            self.check_listing(now);
        }
        if !deliver {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(snapshot) = self.changed_snapshot() {
            out.push(Operation::ClaudeEvent { snapshot });
        }
        if let Some(background) = self.pending_listing.take() {
            self.listing_revision += 1;
            out.push(Operation::ClaudeListing {
                listing: ClaudeListingResult {
                    revision: self.listing_revision,
                    background,
                },
            });
        }
        out
    }

    /// The current snapshot as a `ClaudeSnapshot` reply. A reply is always a
    /// new item of the snapshot revision sequence it shares with events; the
    /// supervisor sends it after that pass's pushes. Listings count their own.
    pub fn state(&mut self) -> Operation {
        let snapshot = self.changed_snapshot().unwrap_or_else(|| {
            self.revision += 1;
            ClaudeSessionSnapshot {
                revision: self.revision,
                hooks_available: self.hooks_available(),
                sessions: self.pushed.clone().unwrap_or_default(),
            }
        });
        Operation::ClaudeSnapshot { snapshot }
    }

    fn changed_snapshot(&mut self) -> Option<ClaudeSessionSnapshot> {
        let mut sessions = self.sessions.infos();
        // The frame bound holds: the least recent sessions are left out.
        while encoded_len(&sessions) > MAX_STATE_BYTES {
            let Some(oldest) = sessions
                .iter()
                .enumerate()
                .min_by_key(|(_, session)| session.recency)
                .map(|(index, _)| index)
            else {
                break;
            };
            sessions.remove(oldest);
        }
        if self.pushed.as_ref() == Some(&sessions) {
            return None;
        }
        self.revision += 1;
        self.pushed = Some(sessions.clone());
        Some(ClaudeSessionSnapshot {
            revision: self.revision,
            hooks_available: self.hooks_available(),
            sessions,
        })
    }

    /// Apply a newer fresh published listing while sessions are owned.
    fn check_listing(&mut self, now: Instant) {
        let Some(worker) = &self.worker else {
            return;
        };
        if self.sessions.sessions.is_empty() {
            return;
        }
        let due = self
            .last_listing_check
            .is_none_or(|last| now.saturating_duration_since(last) >= LISTING_CHECK_INTERVAL);
        if !due {
            return;
        }
        self.last_listing_check = Some(now);
        let Some(listing) = worker.latest() else {
            return;
        };
        if self
            .applied_listing
            .is_some_and(|applied| listing.started_ms <= applied)
        {
            return;
        }
        self.applied_listing = Some(listing.started_ms);
        self.sessions
            .apply_listing(&listing.entries, listing.started_ms, self.own_trigger);
        self.pending_listing = Some(
            listing
                .entries
                .iter()
                .filter(|entry| entry.background && !entry.name.is_empty())
                .filter_map(|entry| {
                    Some(ClaudeBackgroundEntry {
                        name: entry.name.clone(),
                        cwd: remote_dir(&entry.cwd)?,
                    })
                })
                .take(MAX_CLAUDE_BACKGROUND)
                .collect(),
        );
        if let Some(background) = &mut self.pending_listing {
            while encoded_len(background) > MAX_STATE_BYTES && background.pop().is_some() {}
        }
    }
}

/// Most serialized bytes of one relayed session list or background list, well
/// inside the frame bound.
const MAX_STATE_BYTES: usize = MAX_FRAME_BYTES / 2;

fn encoded_len(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

// ---- companion: listing worker ---------------------------------------------------

/// When the runs of one burst are due, measured from its trigger.
const BURST: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(700),
    Duration::from_secs(2),
];

/// Shortest time between the starts of two runs of any tab.
const MIN_RUN_GAP: Duration = Duration::from_millis(500);

/// Longest a run may take before it is killed with its process group.
const RUN_TIMEOUT: Duration = Duration::from_secs(2);

/// A published listing whose run started longer ago than this is not used.
const MAX_LISTING_AGE_MS: u64 = 5_000;

/// How long the output reader may still take once the run has exited.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// Granularity of every wait, and so of reacting to a stop.
const STEP: Duration = Duration::from_millis(10);

/// Most output bytes read from one run.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// Most entries one published listing keeps.
const MAX_LISTING_ENTRIES: usize = 256;

/// Bound of the published listing file.
const MAX_LISTING_FILE_BYTES: usize = 1024 * 1024;

/// The app-owned lock that serializes every run of the server user's tabs.
const LISTING_LOCK: &str = "listing.lock";

/// The app-owned published result every tab of the server user reads.
const LISTING_FILE: &str = "listing.json";

/// The app-owned file every worker with a pending burst holds a shared lock
/// on, so a run is not killed with its tab while another tab waits for it.
const LISTING_INTEREST: &str = "listing.interest";

/// One published `claude agents --json` result: when its run started (the
/// burst it serves) and only the `pid`, `cwd`, `kind` and `name` of each entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub started_ms: u64,
    pub entries: Vec<AgentCwd>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedEntry {
    pid: u32,
    cwd: String,
    background: bool,
    name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Published {
    started_ms: u64,
    entries: Vec<PublishedEntry>,
}

struct WorkerState {
    due: Vec<Instant>,
    last_started: Option<Instant>,
    running: bool,
    thread: Option<JoinHandle<()>>,
}

struct WorkerShared {
    claude: Option<PathBuf>,
    dir: PathBuf,
    stop: AtomicBool,
    state: Mutex<WorkerState>,
}

/// The server user's listing worker, as one tab's supervisor sees it.
///
/// Runs happen only on a trigger, in bursts, with the detected absolute claude
/// and exactly `agents --json`, never through a shell, with standard input
/// closed, as the leader of their own process group, killed with that group
/// after about two seconds or when the worker is dropped. Every tab of the
/// server user coordinates through one app-owned directory: a run holds its
/// lock, so no two runs overlap, and its result is published there for every
/// tab, so one run serves them all. A failed run publishes nothing.
pub struct ListingWorker {
    shared: Arc<WorkerShared>,
    /// This supervisor's shared hold on the server user's listing users; the
    /// last one to go removes the published listing.
    user: Option<fs::File>,
}

impl ListingWorker {
    /// `claude` is the absolute path detection found on the server, `None`
    /// when claude was not detected. `coordination` is the app-owned
    /// directory shared by the server user's tabs.
    pub fn new(claude: Option<PathBuf>, coordination: PathBuf) -> Self {
        // A listing left by supervisors that are all gone is swept at every
        // helper start, like a dead supervisor's spool; only a supervisor that
        // can run the listing holds it in use.
        let user = match claude {
            Some(_) => listing_user(&coordination),
            None => {
                drop(remove_unused_listing(&coordination, Duration::ZERO));
                None
            }
        };
        Self {
            user,
            shared: Arc::new(WorkerShared {
                claude,
                dir: coordination,
                stop: AtomicBool::new(false),
                state: Mutex::new(WorkerState {
                    due: Vec::new(),
                    last_started: None,
                    running: false,
                    thread: None,
                }),
            }),
        }
    }

    /// Schedule a burst and return its id (wall-clock milliseconds), or
    /// `None` when claude was not detected and nothing will run.
    pub fn trigger(&self) -> Option<u64> {
        self.shared.claude.as_ref()?;
        let burst = now_ms();
        let mut state = self.shared.lock();
        schedule_burst(&mut state.due, Instant::now());
        if !state.running {
            let shared = Arc::clone(&self.shared);
            if let Ok(thread) = thread::Builder::new()
                .name("made-remote-claude-agents".to_owned())
                .spawn(move || shared.work())
            {
                state.running = true;
                state.thread = Some(thread);
            }
        }
        Some(burst)
    }

    /// The latest published listing unless it is too old.
    pub fn latest(&self) -> Option<Listing> {
        let published = self.shared.read_published()?;
        let now = now_ms();
        if now.saturating_sub(published.started_ms) > MAX_LISTING_AGE_MS
            || published.started_ms > now + 1_000
        {
            return None;
        }
        Some(Listing {
            started_ms: published.started_ms,
            entries: published
                .entries
                .into_iter()
                .map(|entry| AgentCwd {
                    pid: entry.pid,
                    cwd: entry.cwd,
                    background: entry.background,
                    name: entry.name,
                })
                .collect(),
        })
    }
}

impl Drop for ListingWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, AtomicOrdering::Release);
        let thread = self.shared.lock().thread.take();
        if let Some(thread) = thread {
            // A run another tab waits for still ends within its own bound.
            let deadline = Instant::now() + RUN_TIMEOUT + EXIT_GRACE + Duration::from_secs(1);
            while !thread.is_finished() && Instant::now() < deadline {
                thread::sleep(STEP);
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
        // The last supervisor of the server user removes the listing.
        if self.user.take().is_some() {
            remove_unused_listing(&self.shared.dir, LISTING_USERS_WAIT);
        }
    }
}

/// The app-owned file every supervisor that can run the listing holds a
/// shared lock on for its whole life.
const LISTING_USERS: &str = "listing.users";

/// How long the last supervisor's probe retries the users lock. A child forked
/// by any thread of a supervisor holds every descriptor, close-on-exec ones
/// included, until it execs, so a users hold can outlive its supervisor for
/// that moment; a live supervisor holds it for good. The retry is best effort:
/// a miss leaves a listing that [`ListingWorker::latest`] ignores once it is
/// older than [`MAX_LISTING_AGE_MS`] and that the next helper start sweeps.
/// The start sweep probes once, so a start never waits on a live supervisor.
const LISTING_USERS_WAIT: Duration = Duration::from_millis(100);

/// Remove the published listing when no supervisor holds the users lock,
/// retrying the lock for up to `wait`. Returns the probe's exclusive hold,
/// released by the caller.
fn remove_unused_listing(dir: &Path, wait: Duration) -> Option<fs::File> {
    let path = dir.join(LISTING_USERS);
    note_access(&path);
    let file = made_platform::open_regular_file_nofollow(&path, false)
        .ok()
        .or_else(|| made_platform::create_private_file_new(&path, false).ok())?;
    let deadline = Instant::now() + wait;
    while file.try_lock().is_err() {
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(STEP);
    }
    note_access(&dir.join(LISTING_FILE));
    let listing = dir.join(LISTING_FILE);
    if fs::symlink_metadata(&listing).is_ok_and(|meta| meta.is_file()) {
        let _ = fs::remove_file(&listing);
    }
    Some(file)
}

/// Sweep a listing no live supervisor uses, then hold a shared users lock.
fn listing_user(dir: &Path) -> Option<fs::File> {
    made_platform::ensure_private_directory(dir).ok()?;
    drop(remove_unused_listing(dir, Duration::ZERO));
    let path = dir.join(LISTING_USERS);
    let file = made_platform::open_regular_file_nofollow(&path, false).ok()?;
    file.lock_shared().ok()?;
    Some(file)
}

impl WorkerShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stopped(&self) -> bool {
        self.stop.load(AtomicOrdering::Acquire)
    }

    /// Serve the pending burst runs, then stop.
    fn work(&self) {
        let mut interest = self.interest();
        loop {
            let run = {
                let mut state = self.lock();
                if self.stopped() {
                    state.running = false;
                    return;
                }
                let Some(next) = next_start(&state.due, state.last_started) else {
                    state.running = false;
                    return;
                };
                let now = Instant::now();
                if next > now {
                    Err(next - now)
                } else {
                    take_due(&mut state.due, now);
                    state.last_started = Some(now);
                    Ok(now_ms())
                }
            };
            match run {
                Err(wait) => thread::sleep(wait.min(STEP)),
                Ok(wanted_ms) => {
                    self.run_once(wanted_ms, &mut interest);
                    if interest.is_none() {
                        interest = self.interest();
                    }
                }
            }
        }
    }

    /// This worker's shared hold on the server user's listing interest.
    fn interest(&self) -> Option<fs::File> {
        made_platform::ensure_private_directory(&self.dir).ok()?;
        let path = self.dir.join(LISTING_INTEREST);
        note_access(&path);
        let file = made_platform::open_regular_file_nofollow(&path, false)
            .ok()
            .or_else(|| made_platform::create_private_file_new(&path, false).ok())?;
        file.lock_shared().ok()?;
        Some(file)
    }

    /// Whether another tab's worker of the server user still waits for a
    /// listing. Only then does a run outlive its own tab's teardown.
    fn others_waiting(&self) -> bool {
        let path = self.dir.join(LISTING_INTEREST);
        made_platform::open_regular_file_nofollow(&path, false)
            .is_ok_and(|file| file.try_lock().is_err())
    }

    /// One coordinated run under the shared lock, unless a run of any tab
    /// that started at or after `wanted_ms` already serves it. The run is
    /// killed when this tab is torn down, unless another tab waits for it.
    fn run_once(&self, wanted_ms: u64, interest: &mut Option<fs::File>) {
        let Some(claude) = &self.claude else {
            return;
        };
        if made_platform::ensure_private_directory(&self.dir).is_err() {
            return;
        }
        let lock_path = self.dir.join(LISTING_LOCK);
        note_access(&lock_path);
        // Another tab's run holds the lock for at most its timeout and grace.
        let deadline = Instant::now() + RUN_TIMEOUT + EXIT_GRACE + Duration::from_secs(1);
        let _lock = loop {
            if self.stopped() {
                return;
            }
            match made_platform::lock_private_file(&lock_path, Duration::ZERO) {
                Ok(lock) => break lock,
                Err(_) if Instant::now() < deadline => thread::sleep(STEP),
                Err(_) => return,
            }
        };
        // Holding the run lock is this worker's claim; its interest would
        // only hide whether another tab still waits.
        drop(interest.take());
        if let Some(published) = self.read_published() {
            if published.started_ms >= wanted_ms {
                return;
            }
            let gap = u64::try_from(MIN_RUN_GAP.as_millis()).unwrap_or(u64::MAX);
            while now_ms() < published.started_ms.saturating_add(gap) {
                if self.stopped() {
                    return;
                }
                thread::sleep(STEP);
            }
        }
        let started_ms = now_ms();
        note_access(claude);
        let Some(bytes) = run_with_timeout(claude, &["agents", "--json"], RUN_TIMEOUT, &|| {
            self.stopped() && !self.others_waiting()
        }) else {
            return;
        };
        let Some(entries) = parse_agents(&bytes) else {
            return;
        };
        let published = Published {
            started_ms,
            entries: entries
                .into_iter()
                .take(MAX_LISTING_ENTRIES)
                .map(|entry| PublishedEntry {
                    pid: entry.pid,
                    cwd: entry.cwd,
                    background: entry.background,
                    name: entry.name,
                })
                .collect(),
        };
        if let Ok(bytes) = serde_json::to_vec(&published) {
            note_access(&self.dir.join(LISTING_FILE));
            let _ = made_platform::write_private_metadata_atomic(
                &self.dir,
                LISTING_FILE,
                &bytes,
                MAX_LISTING_FILE_BYTES,
            );
        }
    }

    fn read_published(&self) -> Option<Published> {
        note_access(&self.dir.join(LISTING_FILE));
        let bytes =
            made_platform::read_private_metadata(&self.dir, LISTING_FILE, MAX_LISTING_FILE_BYTES)
                .ok()??;
        serde_json::from_slice(&bytes).ok()
    }
}

/// Replace the pending runs in `due` with a burst triggered at `now`. A run
/// already due before the burst's first one is kept, so triggers that keep
/// arriving never postpone every run; runs are never stacked.
fn schedule_burst(due: &mut Vec<Instant>, now: Instant) {
    let earliest = due.first().copied();
    due.clear();
    due.extend(BURST.iter().map(|offset| now + *offset));
    if let Some(earliest) = earliest.filter(|earliest| *earliest < due[0]) {
        due.insert(0, earliest);
    }
}

/// When the next run may start: its due time, but no sooner than
/// [`MIN_RUN_GAP`] after the latest start. `None` when no run is pending.
fn next_start(due: &[Instant], last_started: Option<Instant>) -> Option<Instant> {
    let next = *due.first()?;
    Some(last_started.map_or(next, |last| next.max(last + MIN_RUN_GAP)))
}

/// Remove every run in `due` that is due at `now`; they are served by one run.
fn take_due(due: &mut Vec<Instant>, now: Instant) {
    let count = due.partition_point(|at| *at <= now);
    due.drain(..count);
}

/// Standard output of a successful run of `program`, or `None` when it fails
/// to start, exits unsuccessfully, outlives `timeout`, `stop` answers yes or
/// its output is not complete shortly after it exits. The run leads its own
/// process group, which is killed with it on timeout or stop, and what is left
/// of it when the output stays held open after the run exits.
fn run_with_timeout(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    made_platform::own_process_group(&mut command);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take();
    let (sender, receiver) = mpsc::channel();
    let reader = stdout.and_then(|stdout| {
        thread::Builder::new()
            .name("made-remote-claude-agents-read".to_owned())
            .spawn(move || {
                let mut bytes = Vec::new();
                let read = stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes);
                let _ = sender.send(read.ok().map(|_| bytes));
            })
            .ok()
    });
    if reader.is_none() {
        made_platform::kill_child_and_group(&mut child);
        return None;
    }
    let status = loop {
        if stop() {
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(STEP),
            _ => break None,
        }
    };
    let Some(status) = status else {
        made_platform::kill_child_and_group(&mut child);
        return None;
    };
    let output = match receiver.recv_timeout(EXIT_GRACE) {
        Ok(output) => output,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            made_platform::kill_reaped_child_group(child.id());
            return None;
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    };
    status.success().then_some(output).flatten()
}
