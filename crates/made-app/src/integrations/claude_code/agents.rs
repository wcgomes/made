//! Spike: follow a Claude Code `/cd` through `claude agents --json`.
//!
//! Claude Code fires no hook for `/cd`, so the hook spool only learns the new
//! directory at the next prompt. `claude agents --json` reports every running
//! session's process id and current working directory immediately. While a
//! tab has a live owned session, one app-wide worker thread runs it about once
//! a second (never through a shell, never two runs at once, killed after a
//! short timeout) and publishes the latest snapshot; tabs match it to their
//! owned Claude Code pids. Every failure leaves the previous snapshot in place,
//! and the worker stops once no tab has asked for a snapshot for a while.
//!
//! A background session opened from Claude Code's agent view runs under the
//! Claude Code daemon, not under the tab, so it is never matched by pid. While
//! the tab's host session shows a live status, the tab's window title names the
//! attached session: a title that equals the `name` of exactly one background
//! entry selects that entry's directory ([`background_cwd_for_title`]).
//!
//! Only `pid`, `cwd`, `kind` and `name` are deserialized; every other field of
//! the output is skipped without being copied, and nothing is logged.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::follow::MAX_TITLE_CHARS;
use crate::integrations::CLAUDE_CODE;
use crate::sanitize::sanitize_display;

/// Interval between the starts of two runs.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Longest a run may take before it is killed and ignored.
const RUN_TIMEOUT: Duration = Duration::from_secs(2);

/// The worker stops when no tab asked for a snapshot for this long.
const IDLE_AFTER: Duration = Duration::from_secs(3);

/// Most output bytes read from one run.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// One running Claude Code session as `claude agents --json` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentCwd {
    pub pid: u32,
    /// Untrusted and unvalidated, exactly like a hook record's `cwd`.
    pub cwd: String,
    /// A background (agent-view) session: matched only by name, never by pid.
    pub background: bool,
    /// The session name normalized by [`normalize_title`]; kept only for
    /// background sessions. Untrusted display text, never logged.
    pub name: String,
}

/// The output of one successful run.
#[derive(Debug)]
pub(crate) struct AgentSnapshot {
    /// When the run started: the reported directories are at least this new.
    pub started_at: Instant,
    pub entries: Vec<AgentCwd>,
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

/// Parse `claude agents --json` output, or `None` when it is not a JSON array
/// of agents. Entries without a pid or a directory are dropped.
pub(crate) fn parse_agents(json: &[u8]) -> Option<Vec<AgentCwd>> {
    let raw: Vec<RawAgent> = serde_json::from_slice(json).ok()?;
    Some(
        raw.into_iter()
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
pub(crate) fn cwd_for_pid(entries: &[AgentCwd], pid: u32) -> Option<&str> {
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
pub(crate) fn normalize_title(text: &str) -> String {
    sanitize_display(text, MAX_TITLE_CHARS)
        .trim_start_matches(|c: char| c.is_whitespace() || is_title_glyph(c))
        .trim_end()
        .to_owned()
}

/// The directory of the one background session whose name equals the
/// normalized `title`. An empty title, no match or more than one background
/// session with that name gives `None`; interactive sessions never match.
pub(crate) fn background_cwd_for_title<'a>(
    entries: &'a [AgentCwd],
    title: &str,
) -> Option<&'a str> {
    let title = normalize_title(title);
    if title.is_empty() {
        return None;
    }
    let mut matches = entries
        .iter()
        .filter(|entry| entry.background && entry.name == title);
    let first = matches.next()?;
    matches.next().is_none().then_some(first.cwd.as_str())
}

struct Poller {
    demanded_at: Option<Instant>,
    running: bool,
    snapshot: Option<Arc<AgentSnapshot>>,
}

static POLLER: Mutex<Poller> = Mutex::new(Poller {
    demanded_at: None,
    running: false,
    snapshot: None,
});

/// Keep the worker running for a tab with a live owned session, and return
/// the latest snapshot. Never blocks on the command.
pub(crate) fn demand() -> Option<Arc<AgentSnapshot>> {
    let mut poller = POLLER.lock().unwrap_or_else(PoisonError::into_inner);
    poller.demanded_at = Some(Instant::now());
    if !poller.running {
        poller.running = thread::Builder::new()
            .name("made-claude-agents".to_owned())
            .spawn(work)
            .is_ok();
    }
    poller.snapshot.clone()
}

fn work() {
    loop {
        {
            let mut poller = POLLER.lock().unwrap_or_else(PoisonError::into_inner);
            if poller
                .demanded_at
                .is_none_or(|at| at.elapsed() > IDLE_AFTER)
            {
                poller.running = false;
                poller.snapshot = None;
                return;
            }
        }
        let started_at = Instant::now();
        if let Some(entries) = run_agents() {
            POLLER
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .snapshot = Some(Arc::new(AgentSnapshot {
                started_at,
                entries,
            }));
        }
        thread::sleep(POLL_INTERVAL.saturating_sub(started_at.elapsed()));
    }
}

/// Run `<absolute claude> agents --json` once, without a shell.
fn run_agents() -> Option<Vec<AgentCwd>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let program = made_platform::find_executable_on_path(
        OsStr::new(CLAUDE_CODE.descriptor().program),
        &path,
    )?;
    let bytes = run_with_timeout(&program, &["agents", "--json"], RUN_TIMEOUT)?;
    parse_agents(&bytes)
}

/// Standard output of a successful run of `program`, or `None` when it fails
/// to start, exits unsuccessfully or outlives `timeout`.
fn run_with_timeout(program: &Path, args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    made_platform::hide_console_window(&mut command);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take();
    let (sender, receiver) = mpsc::channel();
    let reader = stdout.and_then(|stdout| {
        thread::Builder::new()
            .name("made-claude-agents-read".to_owned())
            .spawn(move || {
                let mut bytes = Vec::new();
                let read = stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes);
                let _ = sender.send(read.ok().map(|_| bytes));
            })
            .ok()
    });
    let output = reader.and_then(|_| {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
            .flatten()
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => break None,
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    status.success().then_some(output).flatten()
}

#[cfg(test)]
mod tests {
    use super::super::follow::{ClaudeTab, HookStatus, Record};
    use super::*;

    const OUTPUT: &str = r#"[
        {"id":"b1","pid":4101,"cwd":"/work/background","kind":"background",
         "name":"n","sessionId":"s1","startedAt":1,"state":"x","status":"idle"},
        {"pid":4202,"cwd":"/work/after-cd","kind":"interactive","name":"n",
         "sessionId":"s2","startedAt":2,"status":"idle"},
        {"pid":4303,"kind":"interactive","sessionId":"s3"},
        {"cwd":"/work/no-pid","kind":"interactive","sessionId":"s4"}
    ]"#;

    #[test]
    fn parses_agents_and_matches_an_interactive_pid() {
        let entries = parse_agents(OUTPUT.as_bytes()).expect("a JSON array");
        assert_eq!(
            entries,
            vec![
                background(4101, "/work/background", "n"),
                interactive(4202, "/work/after-cd"),
            ]
        );
        assert_eq!(cwd_for_pid(&entries, 4202), Some("/work/after-cd"));
        // A background session never matches, nor does a pid with no cwd.
        assert_eq!(cwd_for_pid(&entries, 4101), None);
        assert_eq!(cwd_for_pid(&entries, 4303), None);
        assert_eq!(cwd_for_pid(&entries, 9999), None);
    }

    /// Runs the real `claude agents --json`: `cargo test -- --ignored
    /// real_claude_agents`. Prints only pids.
    #[test]
    #[ignore = "needs claude on PATH"]
    fn real_claude_agents_output_parses() {
        let entries = run_agents().expect("claude agents --json ran and parsed");
        let pids: Vec<u32> = entries.iter().map(|entry| entry.pid).collect();
        eprintln!("agent pids: {pids:?}");
        assert!(entries.iter().all(|entry| !entry.cwd.is_empty()));
    }

    fn interactive(pid: u32, cwd: &str) -> AgentCwd {
        AgentCwd {
            pid,
            cwd: cwd.to_owned(),
            background: false,
            name: String::new(),
        }
    }

    fn background(pid: u32, cwd: &str, name: &str) -> AgentCwd {
        AgentCwd {
            pid,
            cwd: cwd.to_owned(),
            background: true,
            name: normalize_title(name),
        }
    }

    #[test]
    fn a_title_selects_exactly_one_background_session() {
        let entries = vec![
            background(90172, "/work/spec", "Spec grok-native-hook pendências"),
            background(90173, "/work/other", "Other task"),
        ];
        assert_eq!(
            background_cwd_for_title(&entries, "Spec grok-native-hook pendências"),
            Some("/work/spec")
        );
    }

    #[test]
    fn a_glyph_prefixed_title_matches_its_session() {
        let entries = vec![background(90172, "/work/spec", "Spec task")];
        for title in [
            "✳ Spec task",
            "⠂ Spec task",
            "✻  Spec task  ",
            " · Spec task",
        ] {
            assert_eq!(
                background_cwd_for_title(&entries, title),
                Some("/work/spec"),
                "{title}"
            );
        }
    }

    #[test]
    fn a_title_that_names_no_background_session_matches_nothing() {
        let entries = vec![background(90172, "/work/spec", "Spec task")];
        assert_eq!(background_cwd_for_title(&entries, "Claude Code"), None);
        assert_eq!(background_cwd_for_title(&entries, "Spec"), None);
        assert_eq!(background_cwd_for_title(&entries, "✳ "), None);
        assert_eq!(background_cwd_for_title(&[], "Spec task"), None);
    }

    #[test]
    fn duplicate_background_names_match_nothing() {
        let entries = vec![
            background(1, "/work/a", "Same name"),
            background(2, "/work/b", "Same name"),
        ];
        assert_eq!(background_cwd_for_title(&entries, "Same name"), None);
    }

    #[test]
    fn an_interactive_session_with_the_same_name_is_ignored() {
        // The parser keeps no name for an interactive session.
        let parsed =
            parse_agents(br#"[{"pid":1,"cwd":"/work/a","kind":"interactive","name":"Shared"}]"#)
                .unwrap();
        assert_eq!(background_cwd_for_title(&parsed, "Shared"), None);
        let both = vec![parsed[0].clone(), background(2, "/work/b", "Shared")];
        assert_eq!(background_cwd_for_title(&both, "Shared"), Some("/work/b"));
        // A background entry never matches by pid.
        assert_eq!(cwd_for_pid(&both, 2), None);
    }

    #[test]
    fn the_attached_background_directory_falls_back_when_the_title_changes() {
        use made_terminal::{ProtocolEvent, ProtocolState};
        let pid = 4202;
        let alive = |candidate: u32| (candidate == pid).then_some(1);
        let mut tab = ClaudeTab::new(pid, true);
        tab.ingest(
            vec![Record {
                session_id: "host".to_owned(),
                cwd: "/work/host".to_owned(),
                event: "SessionStart".to_owned(),
                status: Some(HookStatus::Idle),
                title: String::new(),
                pids: vec![pid],
            }],
            alive,
        );
        let entries = vec![
            interactive(pid, "/work/host"),
            background(90172, "/work/spec", "Spec task"),
        ];

        // Without a live status the title is not trusted.
        tab.apply_attached_title(&entries, Some("✳ Spec task"));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));

        tab.note_protocol(ProtocolEvent::Status {
            state: ProtocolState::Working,
        });
        tab.apply_attached_title(&entries, Some("✳ Spec task"));
        assert_eq!(tab.directory().as_deref(), Some("/work/spec"));

        // The title changes: the host session's directory applies again.
        tab.apply_attached_title(&entries, Some("✳ Claude Code"));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
    }

    #[test]
    fn rejects_output_that_is_not_an_agent_array() {
        assert_eq!(parse_agents(b""), None);
        assert_eq!(parse_agents(b"{\"pid\":1}"), None);
        assert_eq!(parse_agents(b"[{\"pid\":\"x\"}]"), None);
        assert_eq!(parse_agents(b"[]"), Some(Vec::new()));
    }

    #[test]
    fn the_newer_of_hook_and_agents_directory_wins() {
        let pid = 4202;
        let alive = |candidate: u32| (candidate == pid).then_some(1);
        let mut tab = ClaudeTab::new(pid, true);
        let record = |cwd: &str| Record {
            session_id: "s2".to_owned(),
            cwd: cwd.to_owned(),
            event: "UserPromptSubmit".to_owned(),
            status: Some(HookStatus::Working),
            title: String::new(),
            pids: vec![pid],
        };
        let before_hook = Instant::now();
        tab.ingest(vec![record("/work/start")], alive);
        let entries = vec![interactive(pid, "/work/after-cd")];

        // A snapshot started before the hook record was read is older.
        tab.apply_agent_cwds(&entries, before_hook);
        assert_eq!(tab.directory().as_deref(), Some("/work/start"));

        // A later snapshot wins over the hook.
        tab.apply_agent_cwds(&entries, Instant::now());
        assert_eq!(tab.directory().as_deref(), Some("/work/after-cd"));

        // A later hook record wins over that snapshot.
        tab.ingest(vec![record("/work/hook-later")], alive);
        tab.apply_agent_cwds(&entries, before_hook);
        assert_eq!(tab.directory().as_deref(), Some("/work/hook-later"));
    }
}
