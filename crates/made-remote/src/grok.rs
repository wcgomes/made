//! Bounded companion-side Grok Build ingestion and the consented installer.
//!
//! Ingestion reads only the two Grok-native files named by SSH-10 under the
//! remote user's existing Grok home (`$GROK_HOME`, else `~/.grok`):
//! `active_sessions.json` about once a second, and `made-grok-status.jsonl`
//! by append offset once — and only once — the consented install completed.
//! A row or event belongs to this tab only when its process id is the tab's
//! managed PTY child or a live descendant of it. Hook lines carry the Grok
//! parent pid the hook ran under, so owned events apply immediately, while
//! pid-less legacy lines apply only once the inventory proves ownership.
//! There is no Grok-home write here, no host-style path canonicalization, and
//! no other file access.
//!
//! The consented installer is a separate explicit mode: it needs no
//! interpreter, compares dotted numeric `plugin.json` versions, skips equal or
//! newer installs, and runs the official `grok plugin install <staged>
//! --trust` exactly once per missing/older version under a private lock and a
//! hard time bound. It never edits `config.toml` and never redirects
//! `GROK_HOME`.
use crate::protocol::{
    GrokActivity, GrokEventKind, GrokSessionEvent, GrokSessionInfo, GrokSessionSnapshot, Operation,
    RemotePath, MAX_PATH_BYTES,
};
use made_platform::{find_executable_on_path, Environment};
use serde::{
    de::{Deserializer, MapAccess, Visitor},
    Deserialize, Serialize,
};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt, fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Inventory cadence: `active_sessions.json` is re-read at most once a second.
pub const INVENTORY_CADENCE: Duration = Duration::from_secs(1);

/// Inventory read bound. A larger file is ingested only up to this prefix.
const INVENTORY_MAX_BYTES: u64 = 1024 * 1024;
/// Maximum inventory rows parsed per refresh.
const INVENTORY_MAX_ROWS: usize = 4096;
/// Hook-log bytes read per tick. The remainder waits for the next tick.
const HOOK_MAX_READ_BYTES: u64 = 256 * 1024;
/// One hook line larger than this is skipped as oversized.
const HOOK_MAX_LINE_BYTES: usize = 16 * 1024;
/// Hook events pushed per tick; the append offset pauses until the next tick.
const HOOK_MAX_EVENTS_PER_TICK: usize = 256;
/// Plugin manifest read bound.
const MANIFEST_MAX_BYTES: u64 = 64 * 1024;
/// Installed-plugin entries inspected per version comparison.
const INSTALLED_MAX_ENTRIES: usize = 256;
/// Live parent-walk depth shared with the local Grok follow discipline.
const ANCESTRY_MAX_HOPS: usize = 32;
/// Hard cap on remembered per-session activity/cwd hints between refreshes.
const SESSION_HINTS_MAX: usize = 512;
/// Bounded installer poll interval.
const INSTALL_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Total consented-install bound, including the private-lock wait.
pub const INSTALL_BUDGET: Duration = Duration::from_secs(120);

/// The complete shipped grok-made package the companion stages beside
/// `plugins/opencode-made` — exactly the grok-made slice of the fixed
/// platform/app staging list. The native hook executable is staged with the
/// executable mode so the installed hook can run without any interpreter.
pub const GROK_ARTIFACTS: &[&str] = &[
    "plugins/grok-made/plugin.json",
    "plugins/grok-made/README.md",
    "plugins/grok-made/hooks/hooks.json",
    "plugins/grok-made/hooks/bin/made-grok-status",
];

/// The fixed Grok-home locations ingestion may touch. Nothing else under the
/// home is ever opened by ingestion.
pub struct GrokFiles {
    pub home: PathBuf,
}
impl GrokFiles {
    /// `$GROK_HOME` when non-empty, else `$HOME/.grok`. The lookup uses the
    /// provided environment, never a host desktop path.
    pub fn from_environment(environment: &Environment) -> Option<Self> {
        if let Some(value) = environment.get("GROK_HOME").filter(|v| !v.is_empty()) {
            return Some(Self {
                home: PathBuf::from(value),
            });
        }
        let home = environment.get("HOME").filter(|v| !v.is_empty())?;
        Some(Self {
            home: Path::new(home).join(".grok"),
        })
    }
    pub fn sessions_file(&self) -> PathBuf {
        self.home.join("active_sessions.json")
    }
    pub fn status_log(&self) -> PathBuf {
        self.home.join("made-grok-status.jsonl")
    }
}

/// Everything the tracker may read, resolved by the caller. The native hook
/// needs no interpreter, so readiness depends only on the installed version.
pub struct GrokSources {
    pub files: Option<GrokFiles>,
    pub staged: Option<PathBuf>,
}

pub type ParentOf = dyn FnMut(u32) -> Option<u32> + Send;

/// Metadata-only PATH detection. Empty and relative entries are ignored and
/// nothing is spawned or version-checked.
pub fn detect_on_path(environment: &Environment, name: &str) -> Option<PathBuf> {
    let path = environment.get("PATH")?;
    find_executable_on_path(OsStr::new(name), path)
}

#[derive(Clone)]
struct InventoryRow {
    pid: u32,
    session_id: String,
    cwd: String,
    opened_at: String,
}

/// Companion-side Grok state for one tab: the owned inventory, the hook tier,
/// and the revisioned snapshot push discipline.
pub struct GrokTracker {
    child: Option<u32>,
    grok_target: bool,
    files: Option<GrokFiles>,
    staged: Option<PathBuf>,
    parent_of: Box<ParentOf>,
    revision: u64,
    /// Last pushed (revision, running, hooks ready, sessions); `None` after re-attach.
    pushed: Option<(u64, bool, bool, Vec<GrokSessionInfo>)>,
    rows: Vec<InventoryRow>,
    activities: HashMap<String, GrokActivity>,
    cwds: HashMap<String, RemotePath>,
    /// Sessions whose end was reported and that the inventory still lists.
    ended: HashSet<String>,
    hooks_ready: bool,
    log_armed: bool,
    log_offset: u64,
    last_inventory: Option<Instant>,
    last_alive: bool,
    /// Liveness of the last snapshot build, so the supervisor loop's fast
    /// passes skip the row rebuild when nothing can have changed.
    last_built_alive: Option<bool>,
}

impl GrokTracker {
    pub fn new(child: Option<u32>, grok_target: bool, sources: GrokSources) -> Self {
        Self::with_parent(
            child,
            grok_target,
            sources,
            Box::new(made_platform::process_parent),
        )
    }
    pub fn with_parent(
        child: Option<u32>,
        grok_target: bool,
        sources: GrokSources,
        parent_of: Box<ParentOf>,
    ) -> Self {
        let GrokSources { files, staged } = sources;
        Self {
            child,
            grok_target,
            files,
            staged,
            parent_of,
            revision: 0,
            pushed: None,
            rows: Vec::new(),
            activities: HashMap::new(),
            cwds: HashMap::new(),
            ended: HashSet::new(),
            hooks_ready: false,
            log_armed: false,
            log_offset: 0,
            last_inventory: None,
            // The child was just spawned and alive at creation; the first tick
            // corrects this within one loop pass.
            last_alive: true,
            last_built_alive: None,
        }
    }

    /// A new attachment discards revision history so the client applies a
    /// fresh snapshot. Events never replay: the append offset survives.
    pub fn reset_attachment(&mut self) {
        self.revision = 0;
        self.pushed = None;
        self.last_inventory = None;
    }

    /// One supervisor-loop pass. `child_alive` is the managed PTY child's
    /// liveness. Returns the operations to push under the current attach id.
    pub fn tick(&mut self, now: Instant, child_alive: bool) -> Vec<Operation> {
        self.last_alive = child_alive;
        let mut out = Vec::new();
        let mut dirty = self.last_built_alive != Some(child_alive) || self.pushed.is_none();
        let due = self
            .last_inventory
            .is_none_or(|last| now.saturating_duration_since(last) >= INVENTORY_CADENCE);
        if due {
            self.last_inventory = Some(now);
            self.refresh_tier();
            self.refresh_inventory();
            dirty = true;
        }
        if self.hooks_ready && child_alive {
            let drained = self.drain_hook_log();
            dirty |= !drained.is_empty();
            out.extend(drained);
        }
        if dirty {
            if let Some(push) = self.build_snapshot(child_alive) {
                out.push(push);
            }
            self.last_built_alive = Some(child_alive);
        }
        out
    }

    /// The current snapshot as a `GrokState` reply. A fresh attachment builds
    /// synchronously so the first reply after `Attached` is current.
    pub fn state(&mut self) -> Operation {
        if self.pushed.is_none() {
            self.refresh_tier();
            self.refresh_inventory();
            self.last_inventory = Some(Instant::now());
        }
        if let Some(push) = self.build_snapshot(self.last_alive) {
            self.last_built_alive = Some(self.last_alive);
            return push;
        }
        self.last_built_alive = Some(self.last_alive);
        let (revision, running, hooks_ready, sessions) =
            self.pushed.clone().expect("snapshot built above");
        Operation::GrokSnapshot {
            snapshot: GrokSessionSnapshot {
                revision,
                running,
                hooks_ready,
                sessions,
            },
        }
    }

    fn refresh_tier(&mut self) {
        let ready = self.compute_hooks_ready();
        if ready && !self.log_armed {
            // Skip whatever history is already on disk at arm time; a missing
            // log does not arm the skip, so the first append is read.
            self.log_offset = self
                .files
                .as_ref()
                .and_then(|files| fs::metadata(files.status_log()).ok())
                .map_or(0, |meta| meta.len());
            self.log_armed = true;
        } else if !ready && self.log_armed {
            self.log_armed = false;
            self.log_offset = 0;
        }
        self.hooks_ready = ready;
    }

    /// Hook tier two requires an installed grok-made copy at least the shipped
    /// staged version; no interpreter is involved. Nothing is spawned and
    /// nothing is installed.
    fn compute_hooks_ready(&self) -> bool {
        let Some(staged) = &self.staged else {
            return false;
        };
        let Some(shipped) = manifest_version(&staged.join("plugin.json")) else {
            return false;
        };
        let Some(files) = &self.files else {
            return false;
        };
        let Some(installed) = installed_grok_made_version(&files.home) else {
            return false;
        };
        cmp_dotted(&installed, &shipped) != Ordering::Less
    }

    fn refresh_inventory(&mut self) {
        self.rows = match &self.files {
            Some(files) => read_inventory(&files.sessions_file()),
            None => Vec::new(),
        };
        let ids: HashSet<String> = self.rows.iter().map(|row| row.session_id.clone()).collect();
        // An ended id vanishes once its row is gone, so a later re-registration
        // can be observed again.
        self.ended.retain(|id| ids.contains(id));
        self.activities.retain(|id, _| ids.contains(id));
        self.cwds.retain(|id, _| ids.contains(id));
        for row in &self.rows {
            if let Some(path) = validate_remote_dir(&row.cwd) {
                self.cwds.insert(row.session_id.clone(), path);
            }
        }
    }

    fn owned(&mut self, pid: u32, child_alive: bool) -> bool {
        let Some(child) = self.child else {
            return false;
        };
        // A stale or exited pid never counts, even when the row still lists it.
        if !child_alive {
            return false;
        }
        pid == child || self.belongs_to(pid, child)
    }

    /// Live remote PID ancestry: the walk dies at the first exited or
    /// unfindable parent, so a reused pid does not inherit the child.
    fn belongs_to(&mut self, mut pid: u32, root: u32) -> bool {
        for _ in 0..ANCESTRY_MAX_HOPS {
            let Some(parent) = (self.parent_of)(pid) else {
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

    fn ownership(&mut self, session_id: &str) -> Ownership {
        let pid = self
            .rows
            .iter()
            .find(|row| row.session_id == session_id)
            .map(|row| row.pid);
        match pid {
            Some(pid) if self.owned(pid, self.last_alive) => Ownership::Owned,
            Some(_) => Ownership::Foreign,
            None => Ownership::Unknown,
        }
    }

    fn drain_hook_log(&mut self) -> Vec<Operation> {
        let Some(files) = &self.files else {
            return Vec::new();
        };
        let path = files.status_log();
        let Ok(meta) = fs::metadata(&path) else {
            // The plugin has not created the log yet; the offset stays armed
            // at zero so the first append is read.
            return Vec::new();
        };
        let len = meta.len();
        if len < self.log_offset {
            // Truncated or rotated: never rewind into applied history.
            self.log_offset = len;
            return Vec::new();
        }
        if len == self.log_offset {
            return Vec::new();
        }
        let Ok(mut file) = fs::File::open(&path) else {
            return Vec::new();
        };
        if file.seek(SeekFrom::Start(self.log_offset)).is_err() {
            return Vec::new();
        }
        let budget = HOOK_MAX_READ_BYTES.min(len - self.log_offset);
        let mut bytes = Vec::new();
        if (&mut file).take(budget).read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        // Only complete lines are consumed; a torn trailing line waits.
        let last_newline = bytes.iter().rposition(|byte| *byte == b'\n');
        let Some(last_newline) = last_newline else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut consumed = 0usize;
        for line in bytes[..=last_newline].split_inclusive(|byte| *byte == b'\n') {
            if out.len() >= HOOK_MAX_EVENTS_PER_TICK {
                // The cap pauses the offset; the rest waits for the next tick.
                break;
            }
            consumed += line.len();
            let Ok(text) = std::str::from_utf8(line) else {
                continue;
            };
            let Some(parsed) = parse_hook_line(text) else {
                continue;
            };
            if let Some(operation) = self.apply_hook_event(parsed) {
                out.push(operation);
            }
        }
        self.log_offset += u64::try_from(consumed).unwrap_or(u64::MAX);
        out
    }

    fn apply_hook_event(&mut self, parsed: ParsedHookLine) -> Option<Operation> {
        let (_, session_id, pid, _, _) = &parsed;
        // The log is server-global. A pid-bearing line applies only when its
        // recorded Grok process is this tab's managed PTY child or a live
        // descendant of it; a legacy pid-less line applies only when the
        // inventory proves the session's process belongs to this tab. A
        // session no inventory row lists is not proven owned — it may belong
        // to another tab sharing this server — so its events never apply
        // provisionally (SSH-10).
        let owned = match pid {
            Some(pid) => self.owned(*pid, self.last_alive),
            None => self.ownership(session_id) == Ownership::Owned,
        };
        if !owned {
            return None;
        }
        let (kind, session_id, _, cwd, activity) = parsed;
        if kind == GrokEventKind::SessionEnd {
            self.activities.remove(&session_id);
            self.ended.insert(session_id.clone());
        } else {
            self.ended.remove(&session_id);
            if self.activities.len() < SESSION_HINTS_MAX
                || self.activities.contains_key(&session_id)
            {
                self.activities.insert(session_id.clone(), activity);
            }
        }
        if let Some(cwd) = cwd.clone() {
            if self.cwds.len() < SESSION_HINTS_MAX || self.cwds.contains_key(&session_id) {
                self.cwds.insert(session_id.clone(), cwd);
            }
        }
        Some(Operation::GrokEvent {
            event: GrokSessionEvent {
                kind,
                session_id,
                cwd,
                activity,
            },
        })
    }

    fn build_snapshot(&mut self, child_alive: bool) -> Option<Operation> {
        let running = self.grok_target && child_alive;
        // Rows that are not reported ended, cloned so the mutable ancestry
        // walk can run against them.
        let candidates: Vec<InventoryRow> = self
            .rows
            .iter()
            .filter(|row| !self.ended.contains(&row.session_id))
            .cloned()
            .collect();
        let mut owned_rows: Vec<InventoryRow> = candidates
            .into_iter()
            .filter(|row| self.owned(row.pid, child_alive))
            .collect();
        owned_rows.sort_by(|left, right| left.opened_at.cmp(&right.opened_at));
        // The wire bound holds; the newest attachments are the ones kept.
        if owned_rows.len() > made_bridge::state::MAX_SESSIONS {
            owned_rows.drain(..owned_rows.len() - made_bridge::state::MAX_SESSIONS);
        }
        let sessions: Vec<GrokSessionInfo> = owned_rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| GrokSessionInfo {
                cwd: self.cwds.get(&row.session_id).cloned(),
                opened_at: index as u64,
                activity: self
                    .activities
                    .get(&row.session_id)
                    .copied()
                    .unwrap_or(GrokActivity::Idle),
                session_id: row.session_id,
            })
            .collect();
        let unchanged = self.pushed.as_ref().is_some_and(
            |(_, pushed_running, pushed_hooks_ready, pushed_sessions)| {
                *pushed_running == running
                    && *pushed_hooks_ready == self.hooks_ready
                    && *pushed_sessions == sessions
            },
        );
        if unchanged {
            return None;
        }
        self.revision += 1;
        self.pushed = Some((self.revision, running, self.hooks_ready, sessions.clone()));
        Some(Operation::GrokSnapshot {
            snapshot: GrokSessionSnapshot {
                revision: self.revision,
                running,
                hooks_ready: self.hooks_ready,
                sessions,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Owned,
    Foreign,
    Unknown,
}

fn bounded_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.chars().count() <= made_bridge::state::MAX_SESSION_ID_CHARS
        && !session_id.chars().any(char::is_control)
}

fn validate_remote_dir(cwd: &str) -> Option<RemotePath> {
    if cwd.len() > MAX_PATH_BYTES {
        return None;
    }
    RemotePath::new(cwd.as_bytes().to_vec()).ok()
}

/// One complete `active_sessions.json` object. Shared by the companion's
/// inventory and the local Grok follow so both sides agree on what a complete,
/// unambiguous row is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRecord {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub opened_at: String,
}

/// Tolerant bounded inventory parser. Only complete, balanced top-level
/// objects are considered, and each one is parsed as real JSON: truncated
/// objects, malformed rows, duplicate keys, non-string `pid`/`cwd`, and
/// unreadable bytes are skipped, while valid neighbouring rows survive.
/// Braces, commas, and escapes inside string values stay inside the string.
pub fn parse_inventory(text: &str) -> Vec<InventoryRecord> {
    let mut records = Vec::new();
    for object in complete_objects(text) {
        if object.contains('\u{FFFD}') {
            continue;
        }
        let Ok(InventoryObject(fields)) = serde_json::from_str(object) else {
            continue;
        };
        let Some(pid) = fields
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            continue;
        };
        let Some(serde_json::Value::String(cwd)) = fields.get("cwd") else {
            continue;
        };
        let session_id = fields
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let opened_at = fields
            .get("opened_at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        records.push(InventoryRecord {
            pid,
            session_id,
            cwd: cwd.clone(),
            opened_at,
        });
    }
    records
}

/// Every complete top-level object in `text`. The scan is string-aware, so
/// braces inside values never affect the depth, and it stops at an object that
/// never closes instead of accepting a truncated final row.
fn complete_objects(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut objects = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        let start = index;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        let mut end = None;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else {
                match byte {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(index + 1);
                            index += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            index += 1;
        }
        match end {
            Some(end) => objects.push(&text[start..end]),
            None => break,
        }
    }
    objects
}

/// A JSON object deserialized with duplicate keys rejected. `serde_json::Value`
/// alone would keep the last duplicate silently, so the ambiguous row would be
/// guessed instead of skipped.
struct InventoryObject(serde_json::Map<String, serde_json::Value>);

impl<'de> Deserialize<'de> for InventoryObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = serde_json::Map<String, serde_json::Value>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object with unique keys")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if fields.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                }
                Ok(fields)
            }
        }
        deserializer
            .deserialize_map(ObjectVisitor)
            .map(InventoryObject)
    }
}

/// Tolerant bounded inventory reader. Rows with repeated fields, ambiguous
/// interpretations, unreadable bytes, or a session id outside the wire shape
/// are skipped; duplicate ids collapse to the first occurrence.
fn read_inventory(path: &Path) -> Vec<InventoryRow> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    let mut file = file;
    if (&mut file)
        .take(INVENTORY_MAX_BYTES)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for record in parse_inventory(&text) {
        if rows.len() >= INVENTORY_MAX_ROWS {
            break;
        }
        if !bounded_session_id(&record.session_id) || !seen.insert(record.session_id.clone()) {
            continue;
        }
        rows.push(InventoryRow {
            pid: record.pid,
            session_id: record.session_id,
            cwd: record.cwd,
            opened_at: record.opened_at,
        });
    }
    rows
}

fn key_count(object: &str, key: &str) -> usize {
    let pattern = format!("\"{key}\"");
    object.matches(pattern.as_str()).count()
}

fn json_u32(object: &str, key: &str) -> Option<u32> {
    let rest = after_key(object, key)?;
    let number: String = rest
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    number.parse().ok()
}

fn json_string(object: &str, key: &str) -> Option<String> {
    let rest = after_key(object, key)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => out.push(chars.next().unwrap_or('\\')),
            _ => out.push(ch),
        }
    }
    None
}

fn after_key<'a>(object: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{key}\"");
    let start = object.find(&pattern)? + pattern.len();
    let rest = object[start..].trim_start();
    rest.strip_prefix(':')
}

/// One hook status line: the classified kind, the session, the Grok parent
/// pid the hook recorded (`None` for legacy pid-less lines), the validated
/// cwd, and the resulting activity. Only lines whose event and status the
/// shipped hook can produce are interpreted; everything else is skipped,
/// never guessed.
type ParsedHookLine = (
    GrokEventKind,
    String,
    Option<u32>,
    Option<RemotePath>,
    GrokActivity,
);

fn parse_hook_line(line: &str) -> Option<ParsedHookLine> {
    if line.len() > HOOK_MAX_LINE_BYTES {
        return None;
    }
    if ["sessionId", "cwd", "event", "status", "pid"]
        .into_iter()
        .any(|key| key_count(line, key) > 1)
    {
        return None;
    }
    let session_id = json_string(line, "sessionId")?;
    if !bounded_session_id(&session_id) {
        return None;
    }
    let event = json_string(line, "event")?;
    let activity = match json_string(line, "status")?.as_str() {
        "idle" => GrokActivity::Idle,
        "working" => GrokActivity::Working,
        "attention" => GrokActivity::Attention,
        "error" => GrokActivity::Error,
        _ => return None,
    };
    let kind = classify_hook_event(&event)?;
    let cwd = json_string(line, "cwd").and_then(|cwd| validate_remote_dir(&cwd));
    // The hook records the parent pid of the hook process — the Grok session
    // process that ran it. A zero pid is no process id, so such a line reads
    // as legacy pid-less data.
    let pid = json_u32(line, "pid").filter(|pid| *pid != 0);
    Some((kind, session_id, pid, cwd, activity))
}

/// `Stop` is the in-session pause family: the stop hook itself, the idle and
/// permission notices, and the question tool boundaries. The activity field
/// carries the precise status; only SessionStart, Prompt, Error, Cancel and
/// SessionEnd are distinct lifecycle transitions.
fn classify_hook_event(event: &str) -> Option<GrokEventKind> {
    let name = event.to_ascii_lowercase().replace('-', "_");
    match name.as_str() {
        "session_start" | "sessionstart" => Some(GrokEventKind::SessionStart),
        "user_prompt_submit" | "userpromptsubmit" => Some(GrokEventKind::Prompt),
        "stop_failure" | "stopfailure" => Some(GrokEventKind::Error),
        "stop_cancelled" | "stopcancelled" => Some(GrokEventKind::Cancel),
        "session_end" | "sessionend" => Some(GrokEventKind::SessionEnd),
        "stop" | "pre_tool_use" | "pretooluse" | "post_tool_use" | "posttooluse"
        | "notification" => Some(GrokEventKind::Stop),
        _ => None,
    }
}

// ------------------------------------------------------ plugin versioning

fn read_limited(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut text = String::new();
    (&mut file)
        .take(MANIFEST_MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// The shipped manifest must name grok-made and carry a dotted numeric
/// version, exactly like the local installer comparison.
fn manifest_version(path: &Path) -> Option<Vec<u64>> {
    let text = read_limited(path)?;
    if json_string(&text, "name").as_deref() != Some("grok-made") {
        return None;
    }
    parse_dotted_version(&json_string(&text, "version")?)
}

/// The best (highest) installed grok-made version under `installed-plugins`.
fn installed_grok_made_version(home: &Path) -> Option<Vec<u64>> {
    let entries = fs::read_dir(home.join("installed-plugins")).ok()?;
    let mut best: Option<Vec<u64>> = None;
    for (index, entry) in entries.flatten().enumerate() {
        if index >= INSTALLED_MAX_ENTRIES {
            break;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(version) = manifest_version(&entry.path().join("plugin.json")) else {
            continue;
        };
        best = Some(match best {
            Some(current) if cmp_dotted(&current, &version) != Ordering::Less => current,
            _ => version,
        });
    }
    best
}

fn parse_dotted_version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for part in text.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    Some(parts)
}

fn cmp_dotted(left: &[u64], right: &[u64]) -> Ordering {
    let len = left.len().max(right.len());
    for index in 0..len {
        let ordering = left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

// -------------------------------------------------------------- installer

/// One bounded consented install attempt. Every input is resolved by the
/// caller; the function never reads the process environment.
pub struct InstallInputs {
    pub grok: Option<PathBuf>,
    pub files: Option<GrokFiles>,
    pub staged: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub budget: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallOutcome {
    Installed,
    Skipped,
    Failed,
}

/// Honest structured result. `new_sessions_only` discloses that plugin hooks
/// attach when Grok starts, so they apply only to sessions started after the
/// install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallReport {
    pub outcome: InstallOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub new_sessions_only: bool,
}
impl InstallReport {
    pub fn failed(&self) -> bool {
        self.outcome == InstallOutcome::Failed
    }
    /// One bounded JSON line on stdout for the consent command's consumer.
    pub fn line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"outcome\":\"failed\"}".to_owned())
    }
}

fn failed(reason: &'static str) -> InstallReport {
    InstallReport {
        outcome: InstallOutcome::Failed,
        reason: Some(reason),
        new_sessions_only: false,
    }
}

/// The explicit `--install-grok-plugin` behavior. The mode runs only when
/// invoked; watchers and launches never call this.
pub fn install_grok_plugin(inputs: InstallInputs) -> InstallReport {
    let InstallInputs {
        grok,
        files,
        staged,
        data_dir,
        budget,
    } = inputs;
    let (Some(grok), Some(files), Some(data_dir)) = (grok, files, data_dir) else {
        return failed("grok-missing");
    };
    let Some(staged) = staged else {
        return failed("staged-plugin-missing");
    };
    let Some(shipped) = manifest_version(&staged.join("plugin.json")) else {
        return failed("staged-plugin-missing");
    };
    let deadline = Instant::now() + budget;
    if made_platform::ensure_private_directory(&data_dir.join("remote")).is_err() {
        return failed("lock-unavailable");
    }
    let lock_path = data_dir.join("remote").join("grok-install.lock");
    let remaining = deadline.saturating_duration_since(Instant::now());
    // Concurrent installs serialize on the app-owned private lock.
    let Ok(_lock) = made_platform::lock_private_file(&lock_path, remaining) else {
        return failed("lock-timeout");
    };
    // Idempotent per server and version, re-checked under the lock: equal or
    // newer installed copies skip, missing or older installs run exactly once.
    if let Some(installed) = installed_grok_made_version(&files.home) {
        if cmp_dotted(&installed, &shipped) != Ordering::Less {
            return InstallReport {
                outcome: InstallOutcome::Skipped,
                reason: None,
                new_sessions_only: true,
            };
        }
    }
    // Official installer only: the detected absolute grok argv, null stdin, the
    // inherited environment (never a `GROK_HOME` redirection), no config edits.
    let mut child = match Command::new(&grok)
        .arg("plugin")
        .arg("install")
        .arg(&staged)
        .arg("--trust")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return failed("install-failed"),
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    InstallReport {
                        outcome: InstallOutcome::Installed,
                        reason: None,
                        new_sessions_only: true,
                    }
                } else {
                    failed("install-failed")
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return failed("timeout");
                }
                thread::sleep(
                    INSTALL_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return failed("install-failed");
            }
        }
    }
}
