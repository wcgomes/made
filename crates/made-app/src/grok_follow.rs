//! Trial follow for a Grok Build session open in this terminal.
//!
//! `~/.grok/active_sessions.json` lists every session this TUI still has open.
//! Registering one removes its previous row and appends it, so the newest
//! `opened_at` among the rows for this terminal is the session just attached.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct OpenSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub opened_at: String,
}

/// One status line written by the grok-made hook.
pub(crate) struct HookEvent {
    pub session_id: String,
    pub cwd: String,
    pub event: String,
    pub status: String,
}

/// Open sessions whose process belongs to this terminal, oldest attachment first.
pub(crate) fn open_sessions(terminal_pid: u32) -> Vec<OpenSession> {
    let Some(path) = sessions_file() else {
        return Vec::new();
    };
    open_sessions_at(&path, terminal_pid)
}

fn open_sessions_at(path: &Path, terminal_pid: u32) -> Vec<OpenSession> {
    open_sessions_matching(path, terminal_pid, made_platform::process_parent)
}

fn open_sessions_matching(
    path: &Path,
    terminal_pid: u32,
    mut parent_of: impl FnMut(u32) -> Option<u32>,
) -> Vec<OpenSession> {
    let mut matched: Vec<OpenSession> = read_open_sessions_at(path)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            row.pid == terminal_pid
                || process_belongs_to_with(row.pid, terminal_pid, &mut parent_of)
        })
        .filter(|row| !row.session_id.is_empty())
        .collect();
    matched.sort_by(|left, right| left.opened_at.cmp(&right.opened_at));
    matched
}

/// Whether `session_id` in the inventory belongs to this terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionOwnership {
    /// Its pid is the terminal or a descendant, including a shell-typed grok.
    Owned,
    /// The row is present under some other pid.
    Foreign,
    /// The inventory does not list the session yet.
    Unknown,
}

pub(crate) fn session_ownership(terminal_pid: u32, session_id: &str) -> SessionOwnership {
    let Some(path) = sessions_file() else {
        return SessionOwnership::Unknown;
    };
    session_ownership_at(&path, terminal_pid, session_id)
}

fn session_ownership_at(path: &Path, terminal_pid: u32, session_id: &str) -> SessionOwnership {
    let Some(rows) = read_open_sessions_at(path) else {
        return SessionOwnership::Unknown;
    };
    let Some(row) = rows.iter().find(|row| row.session_id == session_id) else {
        return SessionOwnership::Unknown;
    };
    if row.pid == terminal_pid || process_belongs_to(row.pid, terminal_pid) {
        SessionOwnership::Owned
    } else {
        SessionOwnership::Foreign
    }
}

/// Hook lines appended after `offset`. The returned offset is the new file end.
pub(crate) fn read_hook_events(offset: u64) -> (u64, Vec<HookEvent>) {
    let Some(path) = status_log() else {
        return (0, Vec::new());
    };
    read_hook_events_at(&path, offset)
}

fn read_hook_events_at(path: &Path, offset: u64) -> (u64, Vec<HookEvent>) {
    let Ok(file) = std::fs::File::open(path) else {
        // The plugin has not created the log yet. Do not arm the history skip,
        // or the first append after Made is already ticking would be ignored.
        return (0, Vec::new());
    };
    let Ok(meta) = file.metadata() else {
        return (offset, Vec::new());
    };
    let len = meta.len();
    // `u64::MAX` means this is the first time the file has been observed, and
    // it already existed. Skip history that was on disk before Made started.
    if offset == u64::MAX || len < offset {
        return (len, Vec::new());
    }
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (offset, Vec::new());
    }
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return (offset, Vec::new());
    }
    let events = text
        .lines()
        .filter_map(|line| {
            let session_id = json_string(line, "sessionId")?;
            if session_id.is_empty() {
                return None;
            }
            Some(HookEvent {
                session_id,
                cwd: json_string(line, "cwd").unwrap_or_default(),
                event: json_string(line, "event").unwrap_or_default(),
                status: json_string(line, "status").unwrap_or_else(|| "idle".to_owned()),
            })
        })
        .collect();
    (len, events)
}

fn status_log() -> Option<PathBuf> {
    let home = std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".grok")))?;
    Some(home.join("made-grok-status.jsonl"))
}

fn read_open_sessions_at(path: &Path) -> Option<Vec<OpenSession>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut rows = Vec::new();
    for object in text.split("},") {
        // A repeated field has more than one interpretation, so the row cannot
        // choose an attachment. Skipping it keeps the explorer where it was.
        if ["pid", "session_id", "cwd", "opened_at"]
            .into_iter()
            .any(|key| key_count(object, key) > 1)
        {
            continue;
        }
        if let (Some(pid), Some(cwd)) = (json_u32(object, "pid"), json_string(object, "cwd")) {
            rows.push(OpenSession {
                pid,
                session_id: json_string(object, "session_id").unwrap_or_default(),
                cwd,
                opened_at: json_string(object, "opened_at").unwrap_or_default(),
            });
        }
    }
    Some(rows)
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

fn sessions_file() -> Option<PathBuf> {
    let home = std::env::var_os("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".grok")))?;
    Some(home.join("active_sessions.json"))
}

fn process_belongs_to(pid: u32, root: u32) -> bool {
    process_belongs_to_with(pid, root, made_platform::process_parent)
}

fn process_belongs_to_with(
    mut pid: u32,
    root: u32,
    mut parent_of: impl FnMut(u32) -> Option<u32>,
) -> bool {
    for _ in 0..32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn inventory_keeps_this_terminal_and_orders_newest_attachment_last() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("active_sessions.json");
        let terminal_pid = std::process::id();
        let older = root.path().join("older");
        let newest = root.path().join("newest");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::create_dir_all(&newest).unwrap();
        let older = older.canonicalize().unwrap();
        let newest = newest.canonicalize().unwrap();

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep child");
        let child_pid = child.id();
        let outside = made_platform::process_parent(terminal_pid).unwrap_or(1);
        // `json_string` honors JSON escapes. A raw Windows `display()` path
        // (`\\?\C:\...`) would be decoded as `\?C:...` and fail the real-cwd
        // comparison. Encode the fixture the same way a writer would.
        let older_cwd = serde_json::to_string(&older.display().to_string())
            .expect("older cwd encodes as a JSON string");
        let newest_cwd = serde_json::to_string(&newest.display().to_string())
            .expect("newest cwd encodes as a JSON string");
        let body = format!(
            r#"[
            {{"pid":{outside},"session_id":"outside","cwd":{older_cwd},"opened_at":"2024-01-01T00:00:03Z"}},
            {{"pid":{terminal_pid},"session_id":"","cwd":{older_cwd},"opened_at":"2024-01-01T00:00:04Z"}},
            {{"pid":{terminal_pid},"pid":{outside},"session_id":"ambiguous","cwd":{older_cwd},"cwd":{newest_cwd},"opened_at":"2024-01-01T00:00:05Z"}},
            {{"pid":{terminal_pid},"session_id":"self","cwd":{older_cwd},"opened_at":"2024-01-01T00:00:01Z"}},
            {{"pid":{child_pid},"session_id":"child","cwd":{newest_cwd},"opened_at":"2024-01-01T00:00:02Z"}}
            ]"#
        );
        write(&sessions, &body);

        let open = open_sessions_at(&sessions, terminal_pid);
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(
            open.len(),
            2,
            "only the terminal and its descendant count, not an outside pid, an empty session, or an ambiguous row: {open:?}"
        );
        assert_eq!(open[0].session_id, "self");
        assert_eq!(open[0].cwd, older.display().to_string());
        assert_eq!(open[1].session_id, "child");
        assert_eq!(
            open[1].cwd,
            newest.display().to_string(),
            "the newest opened_at is the newly attached directory"
        );

        let missing = root.path().join("missing.json");
        assert!(
            open_sessions_at(&missing, terminal_pid).is_empty(),
            "an unreadable inventory must not invent a session"
        );
        let unreadable = root.path();
        assert!(
            open_sessions_at(unreadable, terminal_pid).is_empty(),
            "a malformed inventory path must not invent a session"
        );
    }

    #[test]
    fn windows_fixture_cwd_keeps_real_backslashes() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("active_sessions.json");
        let cwd = r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpzeFZ5P\older";
        let encoded = serde_json::to_string(cwd).expect("windows cwd encodes");
        let body = format!(
            r#"[{{"pid":1,"session_id":"self","cwd":{encoded},"opened_at":"2024-01-01T00:00:01Z"}}]"#
        );
        write(&sessions, &body);
        let open = open_sessions_matching(&sessions, 1, |_| None);
        assert_eq!(
            open.len(),
            1,
            "the encoded fixture must stay one row: {body}"
        );
        assert_eq!(open[0].cwd, cwd);
    }

    #[test]
    fn a_descendant_pid_counts_with_an_injected_parent_chain() {
        // Windows has no proc_pidinfo. Membership still has to follow a parent
        // chain: terminal 10, its shell child 20, and grok 30 typed in that shell.
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("active_sessions.json");
        let body = r#"[
            {"pid":30,"session_id":"grok","cwd":"/grok","opened_at":"2024-01-01T00:00:02Z"},
            {"pid":10,"session_id":"self","cwd":"/self","opened_at":"2024-01-01T00:00:01Z"},
            {"pid":40,"session_id":"other","cwd":"/other","opened_at":"2024-01-01T00:00:03Z"}
        ]"#;
        write(&sessions, body);
        fn parent(pid: u32) -> Option<u32> {
            match pid {
                30 => Some(20),
                20 => Some(10),
                40 => Some(7),
                _ => None,
            }
        }
        let open = open_sessions_matching(&sessions, 10, parent);
        let ids: Vec<&str> = open.iter().map(|row| row.session_id.as_str()).collect();
        assert_eq!(ids, ["self", "grok"]);
        assert!(process_belongs_to_with(30, 10, parent));
        assert!(!process_belongs_to_with(40, 10, parent));
    }

    #[test]
    fn hook_lines_written_after_startup_apply_when_the_log_was_absent() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("made-grok-status.jsonl");
        let (offset, skipped) = read_hook_events_at(&log, u64::MAX);
        assert!(skipped.is_empty(), "a missing log has nothing to apply yet");
        assert_ne!(
            offset,
            u64::MAX,
            "absence must not arm the history skip; the next append has to be read"
        );

        write(
            &log,
            "{\"sessionId\":\"live\",\"cwd\":\"/live\",\"event\":\"SessionStart\",\"status\":\"idle\"}\n",
        );
        let (_next, events) = read_hook_events_at(&log, offset);
        assert_eq!(
            events.len(),
            1,
            "lines written after Made is ticking must apply"
        );
        assert_eq!(events[0].session_id, "live");
        assert_eq!(events[0].status, "idle");
    }

    #[test]
    fn hook_history_present_at_start_is_ignored_and_later_lines_apply() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("made-grok-status.jsonl");
        write(
            &log,
            "{\"sessionId\":\"old\",\"cwd\":\"/history\",\"event\":\"UserPromptSubmit\",\"status\":\"working\"}\n",
        );
        let (offset, skipped) = read_hook_events_at(&log, u64::MAX);
        assert!(
            skipped.is_empty(),
            "lines already on disk when Made starts must be ignored"
        );
        assert_eq!(offset, std::fs::metadata(&log).unwrap().len());

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(
            file,
            "{{\"sessionId\":\"new\",\"cwd\":\"/live\",\"event\":\"SessionStart\",\"status\":\"idle\"}}"
        )
        .unwrap();
        drop(file);

        let (next, events) = read_hook_events_at(&log, offset);
        assert!(next > offset);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id, "new");
        assert_eq!(events[0].cwd, "/live");
        assert_eq!(events[0].event, "SessionStart");
        assert_eq!(events[0].status, "idle");
    }
}
