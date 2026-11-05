//! Companion Grok ingestion and the consented installer (SSH-10, tasks
//! 2.38/2.40). Every fixture is a temp directory or a fake process chain; no
//! live grok install, no server execution, no Grok-home writes.
use made_remote::grok::{
    self, GrokFiles, GrokSources, GrokTracker, InstallInputs, InstallOutcome, INVENTORY_CADENCE,
};
use made_remote::protocol::{
    GrokActivity, GrokEventKind, GrokSessionEvent, GrokSessionSnapshot, Operation,
};
use made_remote::supervisor::generation_exempt;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A synthetic clock so cadence behavior is observed without sleeping.
#[derive(Clone, Copy)]
struct Clock(Instant);
impl Clock {
    fn start() -> Self {
        Self(Instant::now())
    }
    fn now(&self) -> Instant {
        self.0
    }
    fn step(&mut self, duration: Duration) -> Instant {
        self.0 += duration;
        self.0
    }
}

fn snap(operation: &Operation) -> GrokSessionSnapshot {
    match operation {
        Operation::GrokSnapshot { snapshot } => snapshot.clone(),
        other => panic!("expected a Grok snapshot, got {other:?}"),
    }
}

fn snapshot_of(operations: &[Operation]) -> Option<GrokSessionSnapshot> {
    operations
        .iter()
        .rev()
        .find_map(|operation| match operation {
            Operation::GrokSnapshot { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
}

fn events(operations: &[Operation]) -> Vec<GrokSessionEvent> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::GrokEvent { event } => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn session_ids(snapshot: &GrokSessionSnapshot) -> Vec<String> {
    snapshot
        .sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect()
}

fn write_manifest(directory: &Path, version: &str) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        format!("{{\"name\":\"grok-made\",\"version\":\"{version}\"}}\n"),
    )
    .unwrap();
}

fn parent_chain(pairs: Vec<(u32, u32)>) -> impl FnMut(u32) -> Option<u32> + Send {
    move |pid| {
        pairs
            .iter()
            .find(|(child, _)| *child == pid)
            .map(|(_, p)| *p)
    }
}

fn tracker(
    home: Option<PathBuf>,
    staged: Option<PathBuf>,
    child: Option<u32>,
    grok_target: bool,
    parent_of: impl FnMut(u32) -> Option<u32> + Send + 'static,
) -> GrokTracker {
    GrokTracker::with_parent(
        child,
        grok_target,
        GrokSources {
            files: home.map(|home| GrokFiles { home }),
            staged,
        },
        Box::new(parent_of),
    )
}

fn grok_home(root: &Path) -> PathBuf {
    let home = root.join("grok-home");
    std::fs::create_dir_all(&home).unwrap();
    home
}

fn owned_row(pid: u32, session: &str, cwd: &str, opened_at: &str) -> String {
    format!(r#"{{"pid":{pid},"session_id":"{session}","cwd":"{cwd}","opened_at":"{opened_at}"}}"#)
}

fn inventory(home: &Path, body: &str) {
    std::fs::write(home.join("active_sessions.json"), body).unwrap();
}

fn hook_line(session: &str, cwd: &str, event: &str, status: &str) -> String {
    format!("{{\"sessionId\":\"{session}\",\"cwd\":\"{cwd}\",\"event\":\"{event}\",\"status\":\"{status}\"}}\n")
}

/// A line the shipped hook writes since it records the parent pid of the hook
/// process: the Grok session process that ran the hook.
fn pid_hook_line(session: &str, pid: u32, cwd: &str, event: &str, status: &str) -> String {
    format!("{{\"sessionId\":\"{session}\",\"pid\":{pid},\"cwd\":\"{cwd}\",\"event\":\"{event}\",\"status\":\"{status}\"}}\n")
}

fn append(path: &Path, line: &str) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    file.write_all(line.as_bytes()).unwrap();
    file.flush().unwrap();
}

fn tier_two_fixtures(root: &Path, home: &Path) -> PathBuf {
    let staged = root.join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    write_manifest(&home.join("installed-plugins/grok-made"), "0.1.0");
    staged
}

// ---------------------------------------------------------------- inventory

#[test]
fn inventory_refreshes_once_a_second_and_only_owned_live_rows_count() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(
        &home,
        &format!(
            "[{},\n{},\n{},\n{},\n{}]",
            owned_row(99, "foreign", "/foreign", "2024-01-01T00:00:01Z"),
            owned_row(30, "descendant", "/descendant", "2024-01-01T00:00:03Z"),
            owned_row(10, "direct", "/direct", "2024-01-01T00:00:02Z"),
            owned_row(10, "", "/empty", "2024-01-01T00:00:04Z"),
            // A repeated field has more than one interpretation.
            r#"{"pid":10,"pid":99,"session_id":"ambiguous","cwd":"/a","opened_at":"2024-01-01T00:00:05Z"}"#
        ),
    );
    let mut tracker = tracker(
        Some(home.clone()),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![(30, 20), (20, 10)])),
    );
    let mut clock = Clock::start();
    let first_ops = tracker.tick(clock.now(), true);
    let first = snapshot_of(&first_ops).expect("the first pass pushes the initial snapshot");
    assert_eq!(session_ids(&first), vec!["direct", "descendant"]);
    assert!(first.running, "a living grok child is running");
    assert_eq!(
        first.sessions[0].opened_at, 0,
        "attachment order is companion-assigned and relative"
    );
    assert_eq!(first.sessions[1].opened_at, 1);
    assert_eq!(
        first.sessions[0].cwd.as_ref().unwrap().as_bytes(),
        b"/direct",
        "cwd is the lossless remote bytes, never canonicalized"
    );

    // Within the cadence a changed file is not re-read.
    let later = owned_row(10, "later", "/later", "2024-01-01T00:00:06Z");
    inventory(
        &home,
        &format!(
            "[{},\n{},\n{},\n{},\n{},\n{}]",
            owned_row(99, "foreign", "/foreign", "2024-01-01T00:00:01Z"),
            owned_row(30, "descendant", "/descendant", "2024-01-01T00:00:03Z"),
            owned_row(10, "direct", "/direct", "2024-01-01T00:00:02Z"),
            owned_row(10, "", "/empty", "2024-01-01T00:00:04Z"),
            r#"{"pid":10,"pid":99,"session_id":"ambiguous","cwd":"/a","opened_at":"2024-01-01T00:00:05Z"}"#,
            later
        ),
    );
    let within = tracker.tick(clock.step(Duration::from_millis(500)), true);
    assert!(
        snapshot_of(&within).is_none() && events(&within).is_empty(),
        "the one-second cadence must not re-read early: {within:?}"
    );

    let after = tracker.tick(clock.step(Duration::from_millis(500)), true);
    let snapshot = snapshot_of(&after).expect("the cadence refresh observes the new row");
    assert_eq!(
        session_ids(&snapshot),
        vec!["direct", "descendant", "later"]
    );

    // A foreign-only inventory drops every owned session at the next refresh.
    inventory(
        &home,
        &format!("[{}]", owned_row(99, "foreign", "/foreign", "t")),
    );
    let dropped = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    let snapshot = snapshot_of(&dropped).expect("losing all sessions changes the snapshot");
    assert!(session_ids(&snapshot).is_empty());
    assert!(snapshot.running, "running outlives the sessions");
}

#[test]
fn a_dead_child_and_stale_rows_never_count_even_when_the_pid_row_matches() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s", "/s", "t")));
    let mut tracker = tracker(
        Some(home),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let stopped = tracker.tick(Clock::start().now(), false);
    let snapshot = snapshot_of(&stopped).expect("a liveness flip changes the snapshot");
    assert!(
        session_ids(&snapshot).is_empty(),
        "a stale or exited pid must not count"
    );
    assert!(!snapshot.running);
}

#[test]
fn a_live_descendant_counts_and_a_dead_one_does_not_with_real_processes() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep child");
    let child_pid = child.id();
    inventory(
        &home,
        &format!("[{}]", owned_row(child_pid, "live", "/live", "t")),
    );
    // A shell tab: the managed child is not grok, so running stays false even
    // while a shell-typed grok session is owned.
    let mut tracker = GrokTracker::new(
        Some(std::process::id()),
        false,
        GrokSources {
            files: Some(GrokFiles { home: home.clone() }),
            staged: None,
        },
    );
    let mut clock = Clock::start();
    let _ = tracker.tick(clock.now(), true);
    let snapshot = snapshot_of(&tracker.tick(clock.step(INVENTORY_CADENCE), true))
        .unwrap_or_else(|| snap(&tracker.state()));
    assert_eq!(session_ids(&snapshot), vec!["live"]);
    assert!(!snapshot.running);
    let _ = child.kill();
    let _ = child.wait();
    let dropped = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        snapshot_of(&dropped).is_some_and(|s| session_ids(&s).is_empty()),
        "the ancestry walk must fail for the exited process"
    );
    let _ = tracker.tick(clock.step(INVENTORY_CADENCE), true);
}

#[test]
fn corrupt_duplicate_oversized_and_non_utf8_inventory_rows_fail_bounded() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let mut clock = Clock::start();

    inventory(&home, "[not json at all");
    let mut tracker = tracker(
        Some(home.clone()),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let empty = tracker.tick(clock.now(), true);
    assert!(
        snapshot_of(&empty).is_some_and(|s| session_ids(&s).is_empty()),
        "a malformed inventory invents nothing"
    );

    inventory(
        &home,
        &format!(
            "[{},{}]",
            owned_row(10, "dup", "/first", "a"),
            owned_row(10, "dup", "/second", "b")
        ),
    );
    let _ = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    let snapshot = snap(&tracker.state());
    assert_eq!(
        session_ids(&snapshot),
        vec!["dup"],
        "duplicate ids collapse"
    );
    assert_eq!(
        snapshot.sessions[0].cwd.as_ref().unwrap().as_bytes(),
        b"/first",
        "the first occurrence wins deterministically"
    );

    let mut non_utf8 =
        b"[{\"pid\":10,\"session_id\":\"\xff\xfe\",\"cwd\":\"/x\",\"opened_at\":\"t\"},\n".to_vec();
    non_utf8.extend_from_slice(owned_row(10, "good", "/good", "t2").as_bytes());
    non_utf8.extend_from_slice(b"]");
    std::fs::write(home.join("active_sessions.json"), &non_utf8).unwrap();
    let _ = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    assert_eq!(
        session_ids(&snap(&tracker.state())),
        vec!["good"],
        "a non-UTF8 row is skipped, its neighbours still count"
    );

    let oversized = format!(
        "{}{}",
        "x".repeat(1024 * 1200),
        owned_row(10, "beyond", "/b", "t")
    );
    std::fs::write(home.join("active_sessions.json"), oversized).unwrap();
    let _ = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        session_ids(&snap(&tracker.state())).is_empty(),
        "ingestion past the byte bound must not follow the file"
    );
}

#[test]
fn truncated_malformed_and_pretty_printed_inventory_parses_only_complete_owned_rows() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let pretty_cwd = serde_json::to_string("/pretty").unwrap();
    let escaped_cwd = serde_json::to_string(r#"/weird,{}[]\"quoted"#).unwrap();
    let body = format!(
        r#"[
  {{
    "pid": 10,
    "session_id": "pretty",
    "cwd": {pretty_cwd},
    "opened_at": "t2"
  }},
  {{"pid":10,"session_id":"broken" "cwd":"/broken","opened_at":"t3"}},
  {{"pid":10,"session_id":"dup","session_id":"dup2","cwd":"/dup","opened_at":"t4"}},
  {{"pid":10,"session_id":"escaped","cwd":{escaped_cwd},"opened_at":"t5"}},
  {{"pid":10,"session_id":"after","cwd":"/after","opened_at":"t6"}},
  {{"pid":10,"session_id":"truncated","cwd":"/truncated","opened_at":"t9""#
    );
    inventory(&home, &body);
    let mut tracker = tracker(
        Some(home),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );

    let operations = tracker.tick(Clock::start().now(), true);
    assert!(
        events(&operations).is_empty(),
        "inventory never synthesizes hook or explorer movement"
    );
    let snapshot = snapshot_of(&operations).expect("the initial inventory snapshot is pushed");
    assert_eq!(
        session_ids(&snapshot),
        vec!["pretty", "escaped", "after"],
        "pretty-printed, escaped-cwd and valid neighbour rows survive; malformed, duplicate-key and truncated rows do not"
    );
    assert_eq!(
        snapshot.sessions[1].cwd.as_ref().unwrap().as_bytes(),
        br#"/weird,{}[]\"quoted"#,
        "braces, commas and escapes inside a cwd stay inside the string"
    );
    for absent in ["broken", "dup", "dup2", "truncated"] {
        assert!(
            !session_ids(&snapshot).contains(&absent.to_owned()),
            "{absent} must not be invented from an incomplete or malformed row"
        );
    }
}

#[test]
fn more_than_max_sessions_stays_bounded_and_keeps_the_newest_attachments() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let rows: Vec<String> = (0..80)
        .map(|index| {
            owned_row(
                10,
                &format!("s{index:02}"),
                &format!("/d{index}"),
                &format!("2024-01-01T00:{:02}:{:02}Z", index / 60, index % 60),
            )
        })
        .collect();
    inventory(&home, &format!("[{}]", rows.join(",")));
    let mut tracker = tracker(
        Some(home),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let snapshot = snap(&tracker.state());
    assert!(
        snapshot.sessions.len() <= 64,
        "the wire snapshot bound holds"
    );
    assert_eq!(
        session_ids(&snapshot).last(),
        Some(&"s79".to_owned()),
        "the newest attachment is kept when the bound bites"
    );
    snapshot.validate().expect("the snapshot stays wire-valid");
}

// --------------------------------------------------------------- hook tier

#[test]
fn armed_hook_lines_apply_local_semantics_with_ownership_and_monotonic_offsets() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(
        &home,
        &format!(
            "[{},{}]",
            owned_row(10, "s1", "/one", "2024-01-01T00:00:01Z"),
            owned_row(99, "foreign", "/foreign", "2024-01-01T00:00:02Z")
        ),
    );
    let staged = tier_two_fixtures(root.path(), &home);
    // The status log does not exist yet; arming must not skip the first append.
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let _ = tracker.tick(clock.now(), true);
    let log = home.join("made-grok-status.jsonl");
    append(
        &log,
        &pid_hook_line("s1", 10, "/one", "SessionStart", "idle"),
    );
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "UserPromptSubmit", "working"),
    );
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "PreToolUse", "attention"),
    );
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "Notification", "attention"),
    );
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "Notification", "idle"),
    );
    append(&log, &pid_hook_line("s1", 10, "/two", "Stop", "idle"));
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "StopFailure", "error"),
    );
    append(
        &log,
        &pid_hook_line("s1", 10, "/two", "StopCancelled", "idle"),
    );
    append(
        &log,
        &pid_hook_line("foreign", 99, "/x", "UserPromptSubmit", "working"),
    );
    // A legacy pid-less line for a session no inventory row lists yet.
    append(&log, &hook_line("unknown", "/u", "SessionStart", "idle"));
    append(&log, &pid_hook_line("s1", 10, "/two", "SessionEnd", "idle"));

    let operations = tracker.tick(clock.step(Duration::from_millis(50)), true);
    let pushed = events(&operations);
    let kinds: Vec<(GrokEventKind, GrokActivity)> = pushed
        .iter()
        .map(|event| (event.kind, event.activity))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (GrokEventKind::SessionStart, GrokActivity::Idle),
            (GrokEventKind::Prompt, GrokActivity::Working),
            (GrokEventKind::Stop, GrokActivity::Attention),
            (GrokEventKind::Stop, GrokActivity::Attention),
            (GrokEventKind::Stop, GrokActivity::Idle),
            (GrokEventKind::Stop, GrokActivity::Idle),
            (GrokEventKind::Error, GrokActivity::Error),
            (GrokEventKind::Cancel, GrokActivity::Idle),
            (GrokEventKind::SessionEnd, GrokActivity::Idle),
        ],
        "the local lifecycle semantics apply in order; the foreign pid is filtered: {kinds:?}"
    );
    assert!(
        !pushed.iter().any(|event| event.session_id == "foreign"),
        "server-global hook data must not leak another tab's session"
    );
    assert!(
        !pushed.iter().any(|event| event.session_id == "unknown"),
        "a legacy event without inventory proof must not apply, even provisionally"
    );
    assert_eq!(
        pushed
            .iter()
            .find(|event| event.kind == GrokEventKind::Prompt)
            .unwrap()
            .cwd
            .as_ref()
            .unwrap()
            .as_bytes(),
        b"/two",
        "a prompt moves the explorer to the event cwd"
    );
    let snapshot = snapshot_of(&operations).expect("the session end changes the snapshot");
    assert!(
        !session_ids(&snapshot).contains(&"s1".to_owned()),
        "a session end removes the row"
    );
    assert!(
        !session_ids(&snapshot).contains(&"unknown".to_owned()),
        "an unconfirmed session stays out of the inventory snapshot"
    );

    // Monotonic offsets: a second drain of the same file repeats nothing.
    let again = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert!(events(&again).is_empty(), "events must never double-apply");

    // The legacy session confirms through the inventory; the dropped event
    // does not resurrect and the row shows the inventory default.
    inventory(
        &home,
        &format!(
            "[{}]",
            owned_row(10, "unknown", "/u", "2024-01-01T00:00:09Z")
        ),
    );
    let confirmed = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        events(&confirmed).is_empty(),
        "the unproven legacy event was dropped, never held for later: {confirmed:?}"
    );
    let snapshot = snapshot_of(&confirmed).expect("a confirming session changes the snapshot");
    assert_eq!(session_ids(&snapshot), vec!["unknown"]);
    assert_eq!(
        snapshot.sessions[0].activity,
        GrokActivity::Idle,
        "the session shows the inventory default, never a leaked dot"
    );
}

#[test]
fn two_tabs_sharing_the_server_global_log_never_apply_an_unproven_event() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let staged = tier_two_fixtures(root.path(), &home);
    let log = home.join("made-grok-status.jsonl");
    // Tab A manages grok pid 10 directly; tab B manages pid 20 whose live
    // descendant is pid 30. Neither inventory has rows yet: the hook log is
    // server-global and both companions read every line.
    let mut tab_a = tracker(
        Some(home.clone()),
        Some(staged.clone()),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut tab_b = tracker(
        Some(home.clone()),
        Some(staged),
        Some(20),
        true,
        Box::new(parent_chain(vec![(30, 20)])),
    );
    let mut clock = Clock::start();
    let _ = tab_a.tick(clock.now(), true);
    let _ = tab_b.tick(clock.now(), true);

    // A pid-bearing event from grok 10: the owning tab applies immediately,
    // the other tab never does, even with no inventory row for the session.
    append(
        &log,
        &pid_hook_line("shared", 10, "/one", "UserPromptSubmit", "working"),
    );
    let a = events(&tab_a.tick(clock.step(Duration::from_millis(50)), true));
    assert_eq!(
        a.len(),
        1,
        "the tab whose ancestry owns the event applies it immediately: {a:?}"
    );
    assert_eq!(a[0].session_id, "shared");
    let b = events(&tab_b.tick(clock.step(Duration::from_millis(50)), true));
    assert!(
        b.is_empty(),
        "a companion without proven ancestry must not apply the event: {b:?}"
    );

    // A pid owned by neither managed child is dropped on both tabs.
    append(
        &log,
        &pid_hook_line("stray", 99, "/x", "UserPromptSubmit", "working"),
    );
    let a = events(&tab_a.tick(clock.step(Duration::from_millis(50)), true));
    let b = events(&tab_b.tick(clock.step(Duration::from_millis(50)), true));
    assert!(
        a.is_empty() && b.is_empty(),
        "a pid belonging to neither tab never applies: {a:?} {b:?}"
    );

    // A legacy pid-less line for the still-unregistered session: neither tab
    // may apply it provisionally, so nothing crosses tabs while unproven.
    append(
        &log,
        &hook_line("legacy", "/two", "UserPromptSubmit", "working"),
    );
    let a = events(&tab_a.tick(clock.step(Duration::from_millis(50)), true));
    let b = events(&tab_b.tick(clock.step(Duration::from_millis(50)), true));
    assert!(
        a.is_empty() && b.is_empty(),
        "an unproven legacy event is dropped, never provisionally applied: {a:?} {b:?}"
    );

    // The inventory proves both sessions belong to tab A's grok. The dropped
    // legacy event does not resurrect: the snapshot carries the row with
    // inventory defaults, and tab B still shows nothing.
    inventory(
        &home,
        &format!(
            "[{},{}]",
            owned_row(10, "shared", "/one", "t"),
            owned_row(10, "legacy", "/two", "t")
        ),
    );
    let confirmed = tab_a.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        events(&confirmed).is_empty(),
        "a dropped legacy event never applies later: {confirmed:?}"
    );
    let snapshot = snapshot_of(&confirmed).expect("the confirmation changes the snapshot");
    assert_eq!(session_ids(&snapshot), vec!["shared", "legacy"]);
    let legacy = snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == "legacy")
        .unwrap();
    assert_eq!(
        legacy.activity,
        GrokActivity::Idle,
        "no dot from the dropped legacy event leaks into the snapshot"
    );
    let confirmed_b = tab_b.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        events(&confirmed_b).is_empty(),
        "the other tab never applies the shared event"
    );
    let snapshot_b = snap(&tab_b.state());
    assert!(
        session_ids(&snapshot_b).is_empty(),
        "the other tab's snapshot stays empty"
    );
}

#[test]
fn pid_proven_events_apply_immediately_for_the_child_and_live_descendants_only() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let staged = tier_two_fixtures(root.path(), &home);
    let log = home.join("made-grok-status.jsonl");
    // The managed child is pid 20 and pid 30 is its live descendant; no
    // inventory rows exist at all.
    let mut tracker = tracker(
        Some(home),
        Some(staged),
        Some(20),
        true,
        Box::new(parent_chain(vec![(30, 20)])),
    );
    let mut clock = Clock::start();
    let _ = tracker.tick(clock.now(), true);
    append(
        &log,
        &pid_hook_line("desc", 30, "/d", "UserPromptSubmit", "working"),
    );
    append(&log, &pid_hook_line("child", 20, "/c", "Stop", "idle"));
    append(&log, &pid_hook_line("stray", 31, "/s", "Stop", "idle"));
    let pushed = events(&tracker.tick(clock.step(Duration::from_millis(50)), true));
    assert_eq!(
        pushed.len(),
        2,
        "the managed child's and its descendant's events apply immediately without any inventory row: {pushed:?}"
    );
    assert_eq!(pushed[0].session_id, "desc");
    assert_eq!(
        pushed[0].cwd.as_ref().unwrap().as_bytes(),
        b"/d",
        "an owned event's cwd follows immediately"
    );
    assert_eq!(pushed[1].session_id, "child");
    assert!(
        !pushed.iter().any(|event| event.session_id == "stray"),
        "a pid outside the managed tree never applies, even unregistered"
    );
}

#[test]
fn hook_history_present_at_arming_is_skipped_and_truncation_is_tolerated() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let staged = tier_two_fixtures(root.path(), &home);
    let log = home.join("made-grok-status.jsonl");
    append(
        &log,
        &hook_line("history", "/old", "UserPromptSubmit", "working"),
    );
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let first = tracker.tick(clock.now(), true);
    assert!(
        events(&first).is_empty(),
        "lines already on disk when the attachment starts must be ignored"
    );
    append(
        &log,
        &hook_line("s1", "/one", "UserPromptSubmit", "working"),
    );
    let second = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert_eq!(events(&second).len(), 1, "later appends apply");

    // Truncation: the offset never rewinds into already-applied history.
    std::fs::write(&log, "").unwrap();
    let truncated = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert!(events(&truncated).is_empty());
    append(&log, &hook_line("s1", "/one", "Stop", "idle"));
    let after = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert_eq!(
        events(&after).len(),
        1,
        "appends after truncation still apply"
    );
}

#[test]
fn oversized_and_non_utf8_hook_lines_are_skipped_without_losing_their_neighbours() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let staged = tier_two_fixtures(root.path(), &home);
    let log = home.join("made-grok-status.jsonl");
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let _ = tracker.tick(clock.now(), true);
    append(
        &log,
        &format!(
            "{{\"sessionId\":\"s1\",\"junk\":\"{}\"}}\n",
            "x".repeat(20_000)
        ),
    );
    let mut non_utf8 = Vec::new();
    non_utf8.extend_from_slice(b"{\"sessionId\":\"s1\",\"cwd\":\"/one\",\"event\":\"UserPromptSub");
    non_utf8.extend_from_slice(&[0xff, 0xfe]);
    non_utf8.extend_from_slice(b"mit\",\"status\":\"working\"}\n");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(&non_utf8)
        .unwrap();
    append(&log, &hook_line("s1", "/one", "Stop", "idle"));
    let operations = tracker.tick(clock.step(Duration::from_millis(50)), true);
    let pushed = events(&operations);
    assert_eq!(
        pushed.len(),
        1,
        "only the well-formed neighbour applies: {pushed:?}"
    );
    assert_eq!(pushed[0].kind, GrokEventKind::Stop);
}

#[test]
fn hook_drain_is_capped_per_tick_without_losing_or_repeating_lines() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let staged = tier_two_fixtures(root.path(), &home);
    let log = home.join("made-grok-status.jsonl");
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let _ = tracker.tick(clock.now(), true);
    for _ in 0..300 {
        append(
            &log,
            &hook_line("s1", "/one", "UserPromptSubmit", "working"),
        );
    }
    let first = events(&tracker.tick(clock.step(Duration::from_millis(50)), true));
    assert!(
        first.len() <= 256,
        "the per-tick event cap holds without dropping work"
    );
    let mut total = first.len();
    while total < 300 {
        let next = events(&tracker.tick(clock.step(Duration::from_millis(50)), true));
        assert!(!next.is_empty(), "no appended line may be lost");
        total += next.len();
    }
    let done = events(&tracker.tick(clock.step(Duration::from_millis(50)), true));
    assert!(done.is_empty(), "the drain stops exactly at the file end");
}

#[test]
fn self_installed_newer_plugin_reports_ready_before_any_session_or_hook() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    // The user installed this copy; no Made install or consent path runs here.
    write_manifest(&home.join("installed-plugins/grok-made"), "0.2.0");
    let mut tracker = tracker(
        Some(home),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );

    let operations = tracker.tick(Clock::start().now(), true);
    assert!(events(&operations).is_empty());
    let snapshot = snapshot_of(&operations).expect("the initial tier is pushed");
    assert!(snapshot.running);
    assert!(snapshot.hooks_ready);
    assert!(snapshot.sessions.is_empty());
    assert!(snap(&tracker.state()).hooks_ready);
}

#[test]
fn equal_or_newer_install_makes_hooks_ready_regardless_of_python3() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    // The native hook needs no interpreter: an equal or newer installed copy
    // is ready whatever the server's PATH holds, and nothing consults python3.
    let log = home.join("made-grok-status.jsonl");

    for installed in ["0.1.0", "0.2.0"] {
        write_manifest(&home.join("installed-plugins/grok-made"), installed);
        let mut ready = tracker(
            Some(home.clone()),
            Some(staged.clone()),
            Some(10),
            true,
            parent_chain(vec![]),
        );
        let mut clock = Clock::start();
        let _ = ready.tick(clock.now(), true);
        assert!(
            snap(&ready.state()).hooks_ready,
            "a {installed} install must be ready without any interpreter"
        );
        append(
            &log,
            &pid_hook_line("s1", 10, "/one", "UserPromptSubmit", "working"),
        );
        let operations = ready.tick(clock.step(INVENTORY_CADENCE), true);
        assert!(
            !events(&operations).is_empty(),
            "owned hook lines are ingested once hooks are ready"
        );
        let snapshot = snap(&ready.state());
        assert_eq!(snapshot.sessions[0].activity, GrokActivity::Working);
        let _ = std::fs::remove_file(&log);
    }

    write_manifest(&home.join("installed-plugins/grok-made"), "0.0.9");
    let mut older = tracker(
        Some(home),
        Some(staged),
        Some(10),
        true,
        parent_chain(vec![]),
    );
    assert!(
        !snap(&older.state()).hooks_ready,
        "an older install is still not ready"
    );
}

#[test]
fn inventory_only_mode_never_ingests_hooks_or_synthesizes_activity() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(
        &home,
        &format!(
            "[{},{}]",
            owned_row(10, "s1", "/one", "a"),
            owned_row(10, "s2", "/two", "b")
        ),
    );
    // Shipped plugin exists but nothing is installed: hooks are unavailable.
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let log = home.join("made-grok-status.jsonl");
    append(
        &log,
        &hook_line("s1", "/one", "UserPromptSubmit", "working"),
    );
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let operations = tracker.tick(Clock::start().now(), true);
    assert!(
        events(&operations).is_empty(),
        "inventory-only mode must not read the hook log"
    );
    let snapshot = snap(&tracker.state());
    assert_eq!(session_ids(&snapshot), vec!["s1", "s2"]);
    assert!(
        snapshot
            .sessions
            .iter()
            .all(|session| session.activity == GrokActivity::Idle),
        "inventory-only sessions show idle; nothing is synthesized"
    );
    assert!(
        snapshot.running,
        "the living grok child is running sessionless or not"
    );
    assert!(!snapshot.hooks_ready);
}

#[test]
fn hooks_arm_after_the_install_completes_without_replaying_history() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let log = home.join("made-grok-status.jsonl");
    append(
        &log,
        &hook_line("s1", "/one", "UserPromptSubmit", "working"),
    );
    let mut tracker = tracker(
        Some(home.clone()),
        Some(staged),
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let before = tracker.tick(clock.now(), true);
    assert!(events(&before).is_empty(), "tier one applies no hook data");
    assert!(!snapshot_of(&before).unwrap().hooks_ready);
    // The consented install lands while the tab is running.
    write_manifest(&home.join("installed-plugins/grok-made"), "0.1.0");
    let armed = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    assert!(
        events(&armed).is_empty(),
        "arming skips the history that predates the install"
    );
    let snapshot = snapshot_of(&armed).expect("a tier change resnapshots");
    assert!(snapshot.hooks_ready);
    assert_eq!(snapshot.revision, 2);
    append(&log, &hook_line("s1", "/one", "Stop", "idle"));
    let after = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert_eq!(events(&after).len(), 1, "fresh appends apply after arming");
}

// --------------------------------------------------------------- revisions

#[test]
fn revisions_increase_with_change_and_restart_per_attachment() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    inventory(&home, &format!("[{}]", owned_row(10, "s1", "/one", "t")));
    let mut tracker = tracker(
        Some(home.clone()),
        None,
        Some(10),
        true,
        Box::new(parent_chain(vec![])),
    );
    let mut clock = Clock::start();
    let first_ops = tracker.tick(clock.now(), true);
    let first = snapshot_of(&first_ops).expect("the first pass pushes the initial snapshot");
    assert_eq!(first.revision, 1);
    let unchanged = tracker.tick(clock.step(Duration::from_millis(50)), true);
    assert!(
        !unchanged
            .iter()
            .any(|op| matches!(op, Operation::GrokSnapshot { .. })),
        "an unchanged second must not push a duplicate snapshot"
    );
    inventory(
        &home,
        &format!(
            "[{},{}]",
            owned_row(10, "s1", "/one", "t"),
            owned_row(10, "s2", "/two", "u")
        ),
    );
    let changed = tracker.tick(clock.step(INVENTORY_CADENCE), true);
    let snapshot = snapshot_of(&changed).expect("a content change pushes a snapshot");
    assert_eq!(snapshot.revision, 2, "revisions are monotonic per epoch");
    let reply = snap(&tracker.state());
    assert_eq!(
        reply.revision, 2,
        "GrokState replies carry the current revision"
    );

    tracker.reset_attachment();
    let fresh = snap(&tracker.state());
    assert_eq!(
        fresh.revision, 1,
        "a new attachment starts a fresh revision"
    );
    assert_eq!(
        session_ids(&fresh),
        vec!["s1", "s2"],
        "state is rebuilt, not replayed"
    );
}

// ----------------------------------------------------------------- staging

#[test]
fn staged_artifacts_list_the_complete_shipped_grok_made_package() {
    assert_eq!(
        grok::GROK_ARTIFACTS,
        &[
            "plugins/grok-made/plugin.json",
            "plugins/grok-made/README.md",
            "plugins/grok-made/hooks/hooks.json",
            "plugins/grok-made/hooks/bin/made-grok-status",
        ][..],
        "the companion manifest must list the complete shipped grok-made \
         package, exactly the grok-made slice of the fixed platform/app list"
    );
}

#[test]
fn grok_detection_is_metadata_only_over_absolute_path_entries() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("grok");
    std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let found = grok::detect_on_path(
        &made_platform::Environment::from_pairs([("PATH", bin.as_os_str().to_os_string())]),
        "grok",
    )
    .expect("an absolute PATH entry is searched");
    assert_eq!(found, executable);

    assert!(
        grok::detect_on_path(
            &made_platform::Environment::from_pairs([("PATH", "relative")]),
            "grok"
        )
        .is_none(),
        "relative PATH entries are ignored"
    );
    assert!(
        grok::detect_on_path(
            &made_platform::Environment::from_pairs([("PATH", "")]),
            "grok"
        )
        .is_none(),
        "an empty PATH detects nothing"
    );
    std::fs::remove_file(&executable).unwrap();
    std::fs::create_dir_all(&executable).unwrap();
    assert!(
        grok::detect_on_path(
            &made_platform::Environment::from_pairs([("PATH", bin.as_os_str().to_os_string())]),
            "grok"
        )
        .is_none(),
        "a directory is not an executable detection"
    );
}

#[test]
fn grok_state_requests_are_generation_exempt_like_session_state() {
    assert!(generation_exempt(&Operation::GrokState));
}

// --------------------------------------------------------------- installer

struct FakeGrok {
    grok: PathBuf,
    argv_log: PathBuf,
    env_log: PathBuf,
    data_dir: PathBuf,
}
impl FakeGrok {
    /// A fake `grok` executable that records its argv and inherited
    /// `GROK_HOME`, then runs `body`. The fixture home path is baked into the
    /// script: the installer must not redirect `GROK_HOME`, so the child sees
    /// exactly what the test process sees.
    fn new(root: &Path, body: &str) -> Self {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let grok = bin.join("grok");
        let argv_log = root.join("argv.log");
        let env_log = root.join("env.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {argv}\necho \"GROK_HOME=$GROK_HOME\" >> {env}\n{body}\n",
            argv = argv_log.display(),
            env = env_log.display(),
        );
        std::fs::write(&grok, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&grok, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let data_dir = root.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        Self {
            grok,
            argv_log,
            env_log,
            data_dir,
        }
    }
    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(&self.argv_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// The fake installer body: creates an installed grok-made copy at the
/// fixture home, like the real plugin install would.
fn installer_body(home: &Path) -> String {
    format!(
        "mkdir -p \"{home}/installed-plugins/grok-made\"\nprintf '%s' '{{\"name\":\"grok-made\",\"version\":\"0.1.0\"}}' > \"{home}/installed-plugins/grok-made/plugin.json\"\n",
        home = home.display()
    )
}

fn install_inputs(fake: &FakeGrok, home: &Path, staged: &Path, budget: Duration) -> InstallInputs {
    InstallInputs {
        grok: Some(fake.grok.clone()),
        files: Some(GrokFiles {
            home: home.to_path_buf(),
        }),
        staged: Some(staged.to_path_buf()),
        data_dir: Some(fake.data_dir.clone()),
        budget,
    }
}

const INSTALL_BUDGET: Duration = Duration::from_secs(120);

fn installer_fake(root: &Path, home: &Path) -> FakeGrok {
    FakeGrok::new(root, &installer_body(home))
}

fn read_manifest_version(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let marker = "\"version\":\"";
    let start = text.find(marker).unwrap() + marker.len();
    text[start..].split('"').next().unwrap().to_owned()
}

#[test]
fn install_runs_without_any_interpreter_prerequisite() {
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let home = grok_home(root.path());
    let fake = installer_fake(root.path(), &home);
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Installed);
    assert_eq!(report.reason, None);
    assert_eq!(
        fake.argv().len(),
        1,
        "the official installer ran exactly once with no interpreter input"
    );
}

#[test]
fn install_skips_equal_and_newer_and_installs_missing_or_older_exactly_once() {
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let home = grok_home(root.path());
    let fake = installer_fake(root.path(), &home);

    // Missing: runs once and the installed copy appears.
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Installed);
    assert!(report.new_sessions_only, "hooks apply only to new sessions");
    assert_eq!(
        fake.argv(),
        vec!["plugin", "install", staged.to_str().unwrap(), "--trust"],
        "the official installer argv is exact"
    );

    // Equal: skipped, no second run.
    let ran_once = fake.argv().len();
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Skipped);
    assert_eq!(
        fake.argv().len(),
        ran_once,
        "equal installs never run the command"
    );

    // Newer installed copy: skipped and left in place.
    write_manifest(&home.join("installed-plugins/grok-made"), "9.9.9");
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Skipped);
    assert_eq!(
        fake.argv().len(),
        ran_once,
        "newer installs never run the command"
    );
    assert_eq!(
        read_manifest_version(&home.join("installed-plugins/grok-made/plugin.json")),
        "9.9.9",
        "a newer install stays in place"
    );

    // Older installed copy: exactly one more run.
    write_manifest(&home.join("installed-plugins/grok-made"), "0.0.1");
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Installed);
    assert_eq!(
        fake.argv().len(),
        ran_once * 2,
        "missing or older installs run exactly once"
    );
}

#[test]
fn install_never_edits_config_toml_or_redirects_grok_home() {
    let root = tempfile::tempdir().unwrap();
    let home = grok_home(root.path());
    let fake = installer_fake(root.path(), &home);
    let config = home.join("config.toml");
    std::fs::write(&config, b"# user configuration\n[model]\nname = \"x\"\n").unwrap();
    let unrelated = home.join("sessions.db");
    std::fs::write(&unrelated, b"unrelated-bytes").unwrap();
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let before_grok_home = std::env::var_os("GROK_HOME");

    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    assert_eq!(report.outcome, InstallOutcome::Installed);
    assert_eq!(
        std::fs::read(&config).unwrap(),
        b"# user configuration\n[model]\nname = \"x\"\n",
        "config.toml stays byte-identical"
    );
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated-bytes");
    let child_grok_home = std::fs::read_to_string(&fake.env_log).unwrap();
    let first_line = child_grok_home.lines().next().unwrap_or_default();
    match &before_grok_home {
        Some(value) => assert_eq!(
            first_line,
            format!("GROK_HOME={}", value.to_string_lossy()),
            "the installer must not redirect GROK_HOME"
        ),
        None => assert_eq!(
            first_line, "GROK_HOME=",
            "the installer must not set GROK_HOME"
        ),
    }
}

#[test]
fn install_is_bounded_when_the_command_hangs_and_when_the_lock_is_held() {
    let root = tempfile::tempdir().unwrap();
    let fake = FakeGrok::new(root.path(), "sleep 30");
    let home = grok_home(root.path());
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let started = Instant::now();
    let report = grok::install_grok_plugin(install_inputs(
        &fake,
        &home,
        &staged,
        Duration::from_millis(300),
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the install bound is observed"
    );
    assert_eq!(report.outcome, InstallOutcome::Failed);
    assert_eq!(report.reason, Some("timeout"));

    // A held private lock bounds the wait instead of blocking forever.
    let lock_dir = fake.data_dir.join("remote");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let _held = made_platform::lock_private_file(
        &lock_dir.join("grok-install.lock"),
        Duration::from_millis(50),
    )
    .expect("the test holds the lock");
    let report = grok::install_grok_plugin(install_inputs(
        &fake,
        &home,
        &staged,
        Duration::from_millis(300),
    ));
    drop(_held);
    assert_eq!(report.outcome, InstallOutcome::Failed);
    assert_eq!(report.reason, Some("lock-timeout"));
}

#[test]
fn install_reports_a_bounded_honest_line() {
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged/grok-made");
    write_manifest(&staged, "0.1.0");
    let home = grok_home(root.path());
    let fake = installer_fake(root.path(), &home);
    let report = grok::install_grok_plugin(install_inputs(&fake, &home, &staged, INSTALL_BUDGET));
    let line = report.line();
    assert!(line.len() < 512, "the report line is bounded: {line}");
    let value: serde_json::Value = serde_json::from_str(&line).expect("one JSON line");
    assert_eq!(value["outcome"], "installed");
    assert_eq!(value["new_sessions_only"], true);
    let mut inputs = install_inputs(&fake, &home, &staged, INSTALL_BUDGET);
    inputs.grok = None;
    let failed = grok::install_grok_plugin(inputs);
    let value: serde_json::Value = serde_json::from_str(&failed.line()).expect("one JSON line");
    assert_eq!(value["outcome"], "failed");
    assert_eq!(value["reason"], "grok-missing");
}

#[test]
fn grok_files_resolve_the_fixed_home_layout() {
    let root = tempfile::tempdir().unwrap();
    let explicit = GrokFiles::from_environment(&made_platform::Environment::from_pairs([
        ("GROK_HOME", root.path().join("custom").into_os_string()),
        ("HOME", root.path().join("home").into_os_string()),
    ]))
    .expect("a non-empty GROK_HOME wins");
    assert_eq!(
        explicit.sessions_file(),
        root.path().join("custom/active_sessions.json")
    );
    assert_eq!(
        explicit.status_log(),
        root.path().join("custom/made-grok-status.jsonl")
    );

    let default = GrokFiles::from_environment(&made_platform::Environment::from_pairs([
        ("GROK_HOME", "".to_owned()),
        ("HOME", root.path().join("home").display().to_string()),
    ]))
    .expect("an empty GROK_HOME falls back to HOME/.grok");
    assert_eq!(
        default.sessions_file(),
        root.path().join("home/.grok/active_sessions.json")
    );

    assert!(
        GrokFiles::from_environment(&made_platform::Environment::from_pairs([
            ("GROK_HOME", ""),
            ("HOME", ""),
        ]))
        .is_none()
    );
}
