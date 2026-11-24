//! Spike: follow a Claude Code `/cd` through `claude agents --json`.
//!
//! Claude Code fires no hook for `/cd`, so the hook spool only learns the new
//! directory at the next prompt. `claude agents --json` reports every running
//! session's process id and current working directory immediately. While a
//! tab has a live owned session, one app-wide worker thread runs it about once
//! a second (never through a shell, never two runs at once, in its own process
//! group that is killed after a short timeout, when the worker is stopped, or
//! when the output stays held open after the run exits) and publishes the latest snapshot; tabs match it to their owned Claude Code
//! pids. Every failure leaves the previous snapshot in place, a snapshot older
//! than a few seconds is not used, and the worker stops once no tab has asked
//! for a snapshot for a while or the app shuts down ([`shutdown`]).
//!
//! A background session opened from Claude Code's agent view runs under the
//! Claude Code daemon, not under the tab, so it is never matched by pid. While
//! the tab's host session shows a live status, the tab's window title names the
//! attached session: a title that equals the `name` of exactly one background
//! entry selects that entry's directory ([`background_cwd_for_title`]).
//!
//! Each array entry is decoded on its own, so a malformed entry drops only
//! itself. Only `pid`, `cwd`, `kind` and `name` are kept; every other field is
//! discarded, and nothing is logged.

use std::ffi::OsStr;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
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

/// A snapshot whose run started longer ago than this is not used.
const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(5);

/// How long the output reader may still take once the run has exited.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// Granularity of every wait, and so of reacting to [`shutdown`].
const STEP: Duration = Duration::from_millis(10);

/// Most output bytes read from one run.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

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

/// The output of one successful run.
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
    worker: Option<JoinHandle<()>>,
    /// The output reader of the latest run. No run starts while it is alive.
    reader: Option<JoinHandle<()>>,
    snapshot: Option<Arc<AgentSnapshot>>,
}

static POLLER: Mutex<Poller> = Mutex::new(Poller {
    demanded_at: None,
    running: false,
    worker: None,
    reader: None,
    snapshot: None,
});

/// Bumped by [`shutdown`]. A worker runs only while the generation it started
/// in is current, so a shutdown stops exactly the workers started before it.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Longest [`shutdown`] waits for the stopped worker to exit.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(1);

fn poller() -> std::sync::MutexGuard<'static, Poller> {
    POLLER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the worker started in `generation` has been stopped.
fn stopped(generation: u64) -> bool {
    GENERATION.load(Ordering::Acquire) != generation
}

/// Whether `snapshot` is recent enough to be used.
fn is_fresh(snapshot: &AgentSnapshot) -> bool {
    snapshot.started_at.elapsed() <= MAX_SNAPSHOT_AGE
}

/// Keep the worker running for a tab with a live owned session, and return
/// the latest snapshot unless it is too old. Never blocks on the command.
pub(crate) fn demand() -> Option<Arc<AgentSnapshot>> {
    let mut poller = poller();
    poller.demanded_at = Some(Instant::now());
    if !poller.running {
        let generation = GENERATION.load(Ordering::Acquire);
        if let Ok(worker) = thread::Builder::new()
            .name("made-claude-agents".to_owned())
            .spawn(move || work(generation))
        {
            poller.running = true;
            poller.worker = Some(worker);
        }
    }
    poller
        .snapshot
        .clone()
        .filter(|snapshot| is_fresh(snapshot))
}

/// Stop the worker, killing the process group of its in-flight run, and wait
/// a bounded time for it to exit. A later [`demand`] starts a new worker.
pub(crate) fn shutdown() {
    let worker = {
        let mut poller = poller();
        GENERATION.fetch_add(1, Ordering::AcqRel);
        poller.running = false;
        poller.demanded_at = None;
        poller.snapshot = None;
        poller.worker.take()
    };
    let Some(worker) = worker else {
        return;
    };
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(STEP);
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
}

fn work(generation: u64) {
    let stop = || stopped(generation);
    loop {
        let reader_alive = {
            let mut poller = poller();
            if stop() {
                return;
            }
            if poller
                .demanded_at
                .is_none_or(|at| at.elapsed() > IDLE_AFTER)
            {
                poller.running = false;
                poller.snapshot = None;
                return;
            }
            poller
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.is_finished())
        };
        let started_at = Instant::now();
        if !reader_alive {
            let mut reader = None;
            let entries = run_agents(&mut reader, &stop);
            let mut poller = poller();
            poller.reader = reader;
            if stop() {
                return;
            }
            if let Some(entries) = entries {
                poller.snapshot = Some(Arc::new(AgentSnapshot {
                    started_at,
                    entries,
                }));
            }
        }
        pause(POLL_INTERVAL.saturating_sub(started_at.elapsed()), &stop);
    }
}

/// Sleep for `duration`, or until `stop` answers yes.
fn pause(duration: Duration, stop: &dyn Fn() -> bool) {
    let until = Instant::now() + duration;
    while !stop() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        thread::sleep(left.min(STEP));
    }
}

/// Run `<absolute claude> agents --json` once, without a shell. `reader`
/// receives the run's output reader thread.
fn run_agents(
    reader: &mut Option<JoinHandle<()>>,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<AgentCwd>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let program = made_platform::find_executable_on_path(
        OsStr::new(CLAUDE_CODE.descriptor().program),
        &path,
    )?;
    let bytes = run_with_timeout(&program, &["agents", "--json"], RUN_TIMEOUT, reader, stop)?;
    parse_agents(&bytes)
}

/// Standard output of a successful run of `program`, or `None` when it fails
/// to start, exits unsuccessfully, outlives `timeout`, `stop` answers yes or
/// its output is not complete shortly after it exits. The run leads its own
/// process group: on timeout or stop the group is killed with the run, and when
/// the output is still held open after the run exits, what is left of the
/// group is killed. `reader` receives the output reader thread, which may
/// outlive the call.
fn run_with_timeout(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    reader: &mut Option<JoinHandle<()>>,
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
    made_platform::hide_console_window(&mut command);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take();
    let (sender, receiver) = mpsc::channel();
    *reader = stdout.and_then(|stdout| {
        thread::Builder::new()
            .name("made-claude-agents-read".to_owned())
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
        let entries =
            run_agents(&mut None, &|| false).expect("claude agents --json ran and parsed");
        let pids: Vec<u32> = entries.iter().map(|entry| entry.pid).collect();
        eprintln!("agent pids: {pids:?}");
        assert!(entries.iter().all(|entry| !entry.cwd.is_empty()));
    }

    /// A script that first records its pid (its process group id), then
    /// leaves a background process holding its output open and runs `tail`.
    #[cfg(unix)]
    fn group_script(temp: &Path, tail: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let program = temp.join("run");
        let pid_file = temp.join("pgid");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho $$ > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 30 &\n{tail}\n",
                pid = pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        (program, pid_file)
    }

    /// Wait for the run's process group to be empty and its output reader to
    /// finish. Returns whether both happened.
    #[cfg(unix)]
    fn group_and_reader_gone(pid_file: &Path, reader: Option<JoinHandle<()>>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !pid_file.exists() && Instant::now() < deadline {
            thread::sleep(STEP);
        }
        let pgid = std::fs::read_to_string(pid_file).expect("the run recorded its pid");
        let group_alive = || {
            Command::new("/bin/sh")
                .args(["-c", "kill -0 -- -\"$1\"", "sh", pgid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        let reader = reader.expect("a reader thread");
        while (group_alive() || !reader.is_finished()) && Instant::now() < deadline {
            thread::sleep(STEP);
        }
        !group_alive() && reader.is_finished()
    }

    #[cfg(unix)]
    #[test]
    fn a_timed_out_run_is_killed_with_its_whole_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let (program, pid_file) = group_script(temp.path(), "sleep 30");
        let mut reader = None;
        let output = run_with_timeout(&program, &[], Duration::from_secs(3), &mut reader, &|| {
            false
        });
        assert_eq!(output, None);
        assert!(group_and_reader_gone(&pid_file, reader));
    }

    #[cfg(unix)]
    #[test]
    fn a_stopped_run_is_killed_with_its_whole_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let (program, pid_file) = group_script(temp.path(), "sleep 30");
        let started = Instant::now();
        let mut reader = None;
        let output = run_with_timeout(&program, &[], Duration::from_secs(60), &mut reader, &|| {
            pid_file.exists()
        });
        assert_eq!(output, None);
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(group_and_reader_gone(&pid_file, reader));
    }

    #[cfg(unix)]
    #[test]
    fn output_held_open_after_the_run_exits_kills_the_rest_of_its_group() {
        let temp = tempfile::tempdir().unwrap();
        let (program, pid_file) = group_script(temp.path(), "printf '[]'\nexit 0");
        let mut reader = None;
        let output = run_with_timeout(&program, &[], Duration::from_secs(30), &mut reader, &|| {
            false
        });
        assert_eq!(output, None);
        assert!(group_and_reader_gone(&pid_file, reader));
    }

    #[cfg(unix)]
    #[test]
    fn output_is_read_shortly_after_the_run_exits() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("quick");
        std::fs::write(&program, "#!/bin/sh\nprintf '[]'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut reader = None;
        let output = run_with_timeout(&program, &[], RUN_TIMEOUT, &mut reader, &|| false);
        assert_eq!(output.as_deref(), Some(&b"[]"[..]));
    }

    #[test]
    fn a_snapshot_older_than_the_limit_is_not_used() {
        let snapshot = |age: Duration| AgentSnapshot {
            started_at: Instant::now()
                .checked_sub(age)
                .expect("the clock has run long enough"),
            entries: Vec::new(),
        };
        assert!(is_fresh(&snapshot(Duration::ZERO)));
        assert!(is_fresh(&snapshot(
            MAX_SNAPSHOT_AGE - Duration::from_millis(500)
        )));
        assert!(!is_fresh(&snapshot(
            MAX_SNAPSHOT_AGE + Duration::from_secs(1)
        )));
    }

    #[test]
    fn shutdown_stops_earlier_workers_only_and_may_run_concurrently() {
        let before = GENERATION.load(Ordering::Acquire);
        let shutdowns: Vec<_> = (0..4).map(|_| thread::spawn(shutdown)).collect();
        for handle in shutdowns {
            handle.join().unwrap();
        }
        assert!(stopped(before), "a worker started before is stopped");
        let now = GENERATION.load(Ordering::Acquire);
        assert!(!stopped(now), "a worker started afterwards runs");
        let poller = poller();
        assert!(!poller.running && poller.worker.is_none() && poller.snapshot.is_none());
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
        assert_eq!(parse_agents(b"[]"), Some(Vec::new()));
    }

    #[test]
    fn a_malformed_entry_drops_only_itself() {
        let parsed = parse_agents(
            br#"[{"pid":"x","cwd":"/work/bad"},7,{"pid":1,"cwd":["/work/bad"]},
                {"pid":4202,"cwd":"/work/good","kind":"interactive"}]"#,
        );
        assert_eq!(parsed, Some(vec![interactive(4202, "/work/good")]));
    }

    #[test]
    fn debug_output_leaves_out_the_directory_and_name() {
        let shown = format!("{:?}", background(7, "/work/secret-dir", "Secret name"));
        assert!(shown.contains('7'), "{shown}");
        assert!(
            !shown.contains("secret-dir") && !shown.contains("Secret"),
            "{shown}"
        );
    }

    /// A tab owning pid 4202 and a hook record builder for its session.
    fn hooked_tab() -> (
        ClaudeTab,
        impl Fn(&str) -> Record,
        impl Fn(u32) -> Option<u32>,
    ) {
        let pid = 4202;
        let record = move |cwd: &str| Record {
            session_id: "s2".to_owned(),
            cwd: cwd.to_owned(),
            event: "UserPromptSubmit".to_owned(),
            status: Some(HookStatus::Working),
            title: String::new(),
            pids: vec![pid],
        };
        let alive = move |candidate: u32| (candidate == pid).then_some(1);
        (ClaudeTab::new(pid, true), record, alive)
    }

    /// An instant strictly after every earlier one in the test.
    fn later() -> Instant {
        thread::sleep(Duration::from_millis(2));
        Instant::now()
    }

    #[test]
    fn a_listing_from_before_the_hook_change_does_not_apply() {
        let (mut tab, record, alive) = hooked_tab();
        let before_hook = Instant::now();
        thread::sleep(Duration::from_millis(2));
        tab.ingest(vec![record("/work/start")], alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], before_hook);
        assert_eq!(tab.directory().as_deref(), Some("/work/start"));
    }

    #[test]
    fn a_cd_listed_first_by_a_run_from_before_a_hook_move_still_follows() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/start")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/start")], later());
        // A run starts, `/cd` happens, then a hook record moves the session
        // before that run's snapshot is applied.
        let in_flight = later();
        thread::sleep(Duration::from_millis(2));
        tab.ingest(vec![record("/work/hook")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], in_flight);
        assert_eq!(tab.directory().as_deref(), Some("/work/hook"));
        // The next run lists the same directory: it is still a change.
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/after-cd"));
    }

    #[test]
    fn a_cd_still_follows() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/start")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/start")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/start"));

        // `/cd`: the listing changes and no hook fires.
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/after-cd"));

        // A later hook move wins; the unchanged listing does not undo it.
        tab.ingest(vec![record("/work/hook-later")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/hook-later"));

        // The next `/cd` follows again.
        tab.apply_agent_cwds(&[interactive(4202, "/work/second-cd")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/second-cd"));
    }

    #[test]
    fn a_persistent_hook_and_listing_disagreement_does_not_alternate() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/root/sub")], &alive);
        let listing = [interactive(4202, "/work/root")];
        let mut seen = Vec::new();
        for _ in 0..5 {
            tab.apply_agent_cwds(&listing, later());
            seen.push(tab.directory().unwrap());
            tab.ingest(vec![record("/work/root/sub")], &alive);
            seen.push(tab.directory().unwrap());
        }
        // The first sighting differs from the hook directory and applies once;
        // the next hook record takes it back and the listing never applies
        // again.
        assert_eq!(seen[0], "/work/root");
        assert!(
            seen[1..].iter().all(|cwd| cwd == "/work/root/sub"),
            "{seen:?}"
        );
    }

    #[test]
    fn a_stale_hook_after_a_newer_listing_steps_back_at_most_once() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/a")], &alive);
        let listing = [interactive(4202, "/work/b")];
        tab.apply_agent_cwds(&listing, later());
        assert_eq!(tab.directory().as_deref(), Some("/work/b"));

        // A stale record still naming the old directory is read late.
        tab.ingest(vec![record("/work/a")], &alive);
        assert_eq!(tab.directory().as_deref(), Some("/work/a"));
        for _ in 0..5 {
            tab.apply_agent_cwds(&listing, later());
            assert_eq!(tab.directory().as_deref(), Some("/work/a"));
        }
        // A real change in the listing still follows.
        tab.apply_agent_cwds(&[interactive(4202, "/work/c")], later());
        assert_eq!(tab.directory().as_deref(), Some("/work/c"));
    }
}
