//! Spike: follow a Claude Code `/cd` through `claude agents --json`.
//!
//! Claude Code fires no hook for `/cd`, so the hook spool only learns the new
//! directory at the next prompt. `claude agents --json` reports every running
//! session's process id and current working directory immediately. Nothing
//! runs it on a timer: a tab with a live owned session [`trigger`]s a short
//! burst of runs ([`BURST`]) when a session starts, its user submits input or
//! its window title changes, the moments a `/cd` or an agent-view switch can
//! happen. One app-wide worker thread does the runs (never through a shell,
//! never two at once, never two starts closer than [`MIN_RUN_GAP`], in its own
//! process group that is killed after a short timeout, when the worker is
//! stopped, or when the output stays held open after the run exits) and
//! publishes the latest snapshot, tagged with the burst it served; tabs match
//! it to their owned Claude Code pids. Every failure leaves the previous
//! snapshot in place, a snapshot older than a few seconds is not used, and the
//! worker stops once no burst run is pending or the app shuts down
//! ([`shutdown`]).
//!
//! A background session opened from Claude Code's agent view runs under the
//! Claude Code daemon, not under the tab, so it is never matched by pid. While
//! the tab's host session shows a live status, the tab's window title names the
//! attached session: a title that equals the `name` of exactly one background
//! entry selects that entry's directory ([`background_cwd_for_name`]). The
//! selection outlives the snapshot that made it while the title stays the
//! same; a title change detaches it.
//!
//! Each array entry is decoded on its own, so a malformed entry drops only
//! itself. Only `pid`, `cwd`, `kind` and `name` are kept; every other field is
//! discarded, and nothing is logged.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) use made_remote::claude::{background_cwd_for_name, cwd_for_pid, normalize_title};
pub use made_remote::claude::{parse_agents, AgentCwd};

use crate::integrations::CLAUDE_CODE;

/// When the runs of one burst are due, measured from its [`trigger`].
const BURST: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(700),
    Duration::from_secs(2),
];

/// Shortest time between the starts of two runs, so triggers that keep
/// arriving never run `claude` back to back.
const MIN_RUN_GAP: Duration = Duration::from_millis(500);

/// Longest a run may take before it is killed and ignored.
const RUN_TIMEOUT: Duration = Duration::from_secs(2);

/// A snapshot whose run started longer ago than this is not used.
const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(5);

/// How long the output reader may still take once the run has exited.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// Granularity of every wait, and so of reacting to [`shutdown`].
const STEP: Duration = Duration::from_millis(10);

/// Most output bytes read from one run.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// The output of one successful run.
pub(crate) struct AgentSnapshot {
    /// When the run started: the reported directories are at least this new.
    pub started_at: Instant,
    /// The trigger burst the run served.
    pub burst: u64,
    pub entries: Vec<AgentCwd>,
}

struct Poller {
    /// When the pending runs are due, earliest first. The worker stops once it
    /// is empty.
    due: Vec<Instant>,
    /// Bumped by every [`trigger`]: the burst a run started now serves.
    burst: u64,
    /// When the latest run started.
    last_started: Option<Instant>,
    running: bool,
    worker: Option<JoinHandle<()>>,
    /// The output reader of the latest run. No run starts while it is alive.
    reader: Option<JoinHandle<()>>,
    snapshot: Option<Arc<AgentSnapshot>>,
}

static POLLER: Mutex<Poller> = Mutex::new(Poller {
    due: Vec::new(),
    burst: 0,
    last_started: None,
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

/// The latest snapshot unless it is too old. Starts nothing.
pub(crate) fn latest() -> Option<Arc<AgentSnapshot>> {
    poller()
        .snapshot
        .clone()
        .filter(|snapshot| is_fresh(snapshot))
}

/// Schedule a burst of runs for a tab with a live owned session, starting the
/// worker when it is not running, and return the burst's id. Never blocks on
/// the command.
pub(crate) fn trigger() -> u64 {
    let mut poller = poller();
    poller.burst += 1;
    let burst = poller.burst;
    schedule_burst(&mut poller.due, Instant::now());
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
    burst
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
/// Returns whether any was.
fn take_due(due: &mut Vec<Instant>, now: Instant) -> bool {
    let count = due.partition_point(|at| *at <= now);
    due.drain(..count);
    count > 0
}

/// Stop the worker, killing the process group of its in-flight run, and wait
/// a bounded time for it to exit. A later [`trigger`] starts a new worker.
pub(crate) fn shutdown() {
    let worker = {
        let mut poller = poller();
        GENERATION.fetch_add(1, Ordering::AcqRel);
        poller.running = false;
        poller.due.clear();
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

/// Serve the pending burst runs, then stop. A run due while the previous run's
/// output reader is still alive is skipped.
fn work(generation: u64) {
    let stop = || stopped(generation);
    loop {
        let run = {
            let mut poller = poller();
            if stop() {
                return;
            }
            let Some(next) = next_start(&poller.due, poller.last_started) else {
                poller.running = false;
                return;
            };
            let now = Instant::now();
            if next > now {
                Err(next - now)
            } else {
                take_due(&mut poller.due, now);
                let idle = poller.reader.as_ref().is_none_or(JoinHandle::is_finished);
                if idle {
                    poller.last_started = Some(now);
                }
                Ok(idle.then_some((now, poller.burst)))
            }
        };
        match run {
            // A newer trigger may move the next run earlier: wait in steps.
            Err(wait) => pause(wait.min(STEP), &stop),
            Ok(None) => {}
            Ok(Some((started_at, burst))) => {
                let mut reader = None;
                let entries = run_agents(&mut reader, &stop);
                if !publish(&mut poller(), &stop, reader, entries, started_at, burst) {
                    return;
                }
            }
        }
    }
}

/// Record a finished run in `poller`, unless the worker was stopped meanwhile:
/// a stopped worker leaves the state of a newer one alone. Returns whether the
/// worker keeps running.
fn publish(
    poller: &mut Poller,
    stop: &dyn Fn() -> bool,
    reader: Option<JoinHandle<()>>,
    entries: Option<Vec<AgentCwd>>,
    started_at: Instant,
    burst: u64,
) -> bool {
    if stop() {
        return false;
    }
    poller.reader = reader;
    if let Some(entries) = entries {
        poller.snapshot = Some(Arc::new(AgentSnapshot {
            started_at,
            burst,
            entries,
        }));
    }
    true
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
    /// leaves a background process holding its output open, marks that with a
    /// `.ready` file beside the pid file, and runs `tail`.
    #[cfg(unix)]
    fn group_script(temp: &Path, tail: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let program = temp.join("run");
        let pid_file = temp.join("pgid");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho $$ > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 30 &\n: > '{pid}.ready'\n{tail}\n",
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
        // Stop once the background process exists, not while it is forked.
        let ready = pid_file.with_extension("ready");
        let started = Instant::now();
        let mut reader = None;
        let output = run_with_timeout(&program, &[], Duration::from_secs(60), &mut reader, &|| {
            ready.exists()
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
            burst: 0,
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
        // Other tests may shut down concurrently too: only the stop of the
        // earlier generation is certain.
        assert!(stopped(before), "a worker started before is stopped");
    }

    #[test]
    fn a_stopped_worker_leaves_the_poller_state_alone() {
        let mut poller = Poller {
            due: Vec::new(),
            burst: 0,
            last_started: None,
            running: true,
            worker: None,
            reader: None,
            snapshot: None,
        };
        let finished = || thread::spawn(|| {});
        let entries = || Some(vec![interactive(1, "/work")]);
        assert!(!publish(
            &mut poller,
            &|| true,
            Some(finished()),
            entries(),
            Instant::now(),
            1
        ));
        assert!(poller.reader.is_none() && poller.snapshot.is_none());
        assert!(publish(
            &mut poller,
            &|| false,
            Some(finished()),
            entries(),
            Instant::now(),
            1
        ));
        assert!(poller.reader.is_some() && poller.snapshot.is_some());
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

    /// The background directory a raw window title selects.
    fn for_title<'a>(entries: &'a [AgentCwd], title: &str) -> Option<&'a str> {
        background_cwd_for_name(entries, &normalize_title(title))
    }

    #[test]
    fn a_title_selects_exactly_one_background_session() {
        let entries = vec![
            background(90172, "/work/spec", "Spec grok-native-hook pendências"),
            background(90173, "/work/other", "Other task"),
        ];
        assert_eq!(
            for_title(&entries, "Spec grok-native-hook pendências"),
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
            assert_eq!(for_title(&entries, title), Some("/work/spec"), "{title}");
        }
    }

    #[test]
    fn a_title_that_names_no_background_session_matches_nothing() {
        let entries = vec![background(90172, "/work/spec", "Spec task")];
        assert_eq!(for_title(&entries, "Claude Code"), None);
        assert_eq!(for_title(&entries, "Spec"), None);
        assert_eq!(for_title(&entries, "✳ "), None);
        assert_eq!(for_title(&[], "Spec task"), None);
    }

    #[test]
    fn duplicate_background_names_match_nothing() {
        let entries = vec![
            background(1, "/work/a", "Same name"),
            background(2, "/work/b", "Same name"),
        ];
        assert_eq!(for_title(&entries, "Same name"), None);
    }

    #[test]
    fn an_interactive_session_with_the_same_name_is_ignored() {
        // The parser keeps no name for an interactive session.
        let parsed =
            parse_agents(br#"[{"pid":1,"cwd":"/work/a","kind":"interactive","name":"Shared"}]"#)
                .unwrap();
        assert_eq!(for_title(&parsed, "Shared"), None);
        let both = vec![parsed[0].clone(), background(2, "/work/b", "Shared")];
        assert_eq!(for_title(&both, "Shared"), Some("/work/b"));
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

        // Without a live status the title is neither compared nor kept.
        assert!(!tab.apply_attached_title(Some(&entries), Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));

        // Once trusted, the title counts as changed.
        tab.note_protocol(ProtocolEvent::Status {
            state: ProtocolState::Working,
        });
        assert!(tab.apply_attached_title(Some(&entries), Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/spec"));

        // The title changes: the host session's directory applies again.
        assert!(tab.apply_attached_title(Some(&entries), Some("✳ Claude Code")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
    }

    #[test]
    fn an_attached_selection_outlives_its_listing_until_the_title_changes() {
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
        tab.note_protocol(ProtocolEvent::Status {
            state: ProtocolState::Working,
        });
        let entries = vec![
            interactive(pid, "/work/host"),
            background(90172, "/work/spec", "Spec task"),
        ];

        // A new title with no fresh listing selects nothing yet.
        assert!(tab.apply_attached_title(None, Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
        assert!(!tab.apply_attached_title(Some(&entries), Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/spec"));

        // The listing goes stale: the same title (a spinner frame is the same
        // title) keeps the selection.
        for title in ["✳ Spec task", "⠂ Spec task", "✳ Spec task"] {
            assert!(!tab.apply_attached_title(None, Some(title)), "{title}");
            assert_eq!(tab.directory().as_deref(), Some("/work/spec"));
        }

        // A protocol clear drops the selection; once the status is live
        // again the title counts as changed, so a burst lists it again.
        tab.note_protocol(ProtocolEvent::Clear { bare: false });
        assert!(!tab.apply_attached_title(None, Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
        tab.note_protocol(ProtocolEvent::Status {
            state: ProtocolState::Idle,
        });
        assert!(tab.apply_attached_title(None, Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
        assert!(!tab.apply_attached_title(Some(&entries), Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/spec"));

        // A title change detaches at once, before any listing.
        assert!(tab.apply_attached_title(None, Some("✳ Claude Code")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
        // Coming back needs a listing again.
        assert!(tab.apply_attached_title(None, Some("✳ Spec task")));
        assert_eq!(tab.directory().as_deref(), Some("/work/host"));
    }

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn a_trigger_schedules_one_short_burst() {
        let start = Instant::now();
        let mut due = Vec::new();
        schedule_burst(&mut due, start);
        assert_eq!(due, vec![at(start, 200), at(start, 700), at(start, 2000)]);

        assert!(!take_due(&mut due, at(start, 199)));
        assert!(take_due(&mut due, at(start, 200)));
        assert!(!take_due(&mut due, at(start, 699)));
        // Runs that fell due together are served by one run.
        assert!(take_due(&mut due, at(start, 5000)));
        assert!(due.is_empty(), "the burst is over");
        assert!(!take_due(&mut due, at(start, 9000)));
    }

    #[test]
    fn a_new_trigger_restarts_the_burst_without_stacking_runs() {
        let start = Instant::now();
        let mut due = Vec::new();
        schedule_burst(&mut due, start);
        assert!(take_due(&mut due, at(start, 200)));
        assert!(take_due(&mut due, at(start, 700)));

        // Mid-burst: the burst restarts from the new trigger, dropping the run
        // still pending at 2 s.
        schedule_burst(&mut due, at(start, 1000));
        assert_eq!(due, vec![at(start, 1200), at(start, 1700), at(start, 3000)]);

        // Triggers that keep coming never postpone the run already due first.
        schedule_burst(&mut due, at(start, 1100));
        schedule_burst(&mut due, at(start, 1150));
        assert_eq!(
            due,
            vec![
                at(start, 1200),
                at(start, 1350),
                at(start, 1850),
                at(start, 3150)
            ]
        );
    }

    #[test]
    fn run_starts_keep_a_minimum_gap() {
        let start = Instant::now();
        assert_eq!(next_start(&[], Some(start)), None);
        assert_eq!(next_start(&[at(start, 200)], None), Some(at(start, 200)));
        // A run due 200 ms after the latest start waits for the gap.
        assert_eq!(
            next_start(&[at(start, 200)], Some(start)),
            Some(at(start, 500))
        );
        // A burst's own runs are already far enough apart.
        assert_eq!(
            next_start(&[at(start, 700)], Some(at(start, 200))),
            Some(at(start, 700))
        );
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
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], before_hook, 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/start"));
    }

    #[test]
    fn a_cd_listed_first_by_a_run_from_before_a_hook_move_still_follows() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/start")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/start")], later(), 1);
        // A run starts, `/cd` happens, then a hook record moves the session
        // before that run's snapshot is applied.
        let in_flight = later();
        thread::sleep(Duration::from_millis(2));
        tab.ingest(vec![record("/work/hook")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], in_flight, 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/hook"));
        // The next run lists the same directory: it is still a change.
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/after-cd"));
    }

    #[test]
    fn a_cd_still_follows() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/start")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/start")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/start"));

        // `/cd`: the listing changes and no hook fires.
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/after-cd"));

        // A later hook move wins; the unchanged listing does not undo it.
        tab.ingest(vec![record("/work/hook-later")], &alive);
        tab.apply_agent_cwds(&[interactive(4202, "/work/after-cd")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/hook-later"));

        // The next `/cd` follows again.
        tab.apply_agent_cwds(&[interactive(4202, "/work/second-cd")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/second-cd"));
    }

    #[test]
    fn a_persistent_hook_and_listing_disagreement_does_not_alternate() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/root/sub")], &alive);
        let listing = [interactive(4202, "/work/root")];
        let mut seen = Vec::new();
        for _ in 0..5 {
            tab.apply_agent_cwds(&listing, later(), 1);
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
        tab.apply_agent_cwds(&listing, later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/b"));

        // A stale record still naming the old directory is read late.
        tab.ingest(vec![record("/work/a")], &alive);
        assert_eq!(tab.directory().as_deref(), Some("/work/a"));
        for _ in 0..5 {
            tab.apply_agent_cwds(&listing, later(), 1);
            assert_eq!(tab.directory().as_deref(), Some("/work/a"));
        }
        // A real change in the listing still follows.
        tab.apply_agent_cwds(&[interactive(4202, "/work/c")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/c"));
    }

    #[test]
    fn a_cd_back_to_a_listed_directory_after_a_hook_move_follows() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/w")], &alive);
        // Burst 1 (Enter after `/cd /work/a`) lists A.
        tab.note_trigger(1);
        tab.apply_agent_cwds(&[interactive(4202, "/work/a")], later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/a"));
        // Long after that burst a hook record moves the session to W; nothing
        // lists it.
        tab.ingest(vec![record("/work/w")], &alive);
        assert_eq!(tab.directory().as_deref(), Some("/work/w"));
        // `/cd /work/a` and Enter: burst 2 lists A again, which follows.
        tab.note_trigger(2);
        tab.apply_agent_cwds(&[interactive(4202, "/work/a")], later(), 2);
        assert_eq!(tab.directory().as_deref(), Some("/work/a"));
    }

    #[test]
    fn a_persistent_disagreement_steps_at_most_once_per_burst() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/x")], &alive);
        let listing = [interactive(4202, "/work/y")];
        let mut seen = Vec::new();
        for burst in [1, 1, 1, 2, 2, 2] {
            tab.note_trigger(burst);
            tab.apply_agent_cwds(&listing, later(), burst);
            seen.push(tab.directory().unwrap());
            tab.ingest(vec![record("/work/x")], &alive);
            seen.push(tab.directory().unwrap());
        }
        // Burst 1 and burst 2 each step to the listing once; every hook record
        // takes it back, and the other runs of a burst never apply.
        let y = |index: usize| seen[index] == "/work/y";
        assert!(y(0) && y(6), "{seen:?}");
        assert!(
            (0..seen.len()).filter(|index| y(*index)).count() == 2,
            "{seen:?}"
        );
    }

    #[test]
    fn another_tabs_bursts_never_step_a_persistent_disagreement() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/x")], &alive);
        let listing = [interactive(4202, "/work/y")];
        // Tab 1's own trigger steps once.
        tab.note_trigger(1);
        tab.apply_agent_cwds(&listing, later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/y"));
        tab.ingest(vec![record("/work/x")], &alive);
        // Tab 2 triggers bursts 2 to 6; tab 1 only sees their snapshots.
        for burst in 2..=6 {
            tab.apply_agent_cwds(&listing, later(), burst);
            assert_eq!(tab.directory().as_deref(), Some("/work/x"), "{burst}");
            tab.ingest(vec![record("/work/x")], &alive);
        }
        // A real `/cd` in tab 1 still follows from another tab's burst.
        tab.apply_agent_cwds(&[interactive(4202, "/work/z")], later(), 7);
        assert_eq!(tab.directory().as_deref(), Some("/work/z"));
    }

    #[test]
    fn without_a_new_burst_hook_records_never_bring_the_listing_back() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/x")], &alive);
        let listing = [interactive(4202, "/work/y")];
        tab.apply_agent_cwds(&listing, later(), 1);
        assert_eq!(tab.directory().as_deref(), Some("/work/y"));
        for _ in 0..5 {
            tab.ingest(vec![record("/work/x")], &alive);
            // The same snapshot is applied again every frame.
            tab.apply_agent_cwds(&listing, later(), 1);
            assert_eq!(tab.directory().as_deref(), Some("/work/x"));
        }
    }

    #[test]
    fn identical_listings_within_one_burst_apply_at_most_once() {
        let (mut tab, record, alive) = hooked_tab();
        tab.ingest(vec![record("/work/x")], &alive);
        let listing = [interactive(4202, "/work/y")];
        tab.apply_agent_cwds(&listing, later(), 7);
        assert_eq!(tab.directory().as_deref(), Some("/work/y"));
        tab.ingest(vec![record("/work/x")], &alive);
        for _ in 0..3 {
            tab.apply_agent_cwds(&listing, later(), 7);
            assert_eq!(tab.directory().as_deref(), Some("/work/x"));
        }
    }
}
