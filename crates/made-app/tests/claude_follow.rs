//! Claude Code follow tests: spool ingestion, ownership by live process
//! ancestry, liveness, presence, dots and the tooltip title chain. Every test
//! uses fixture spools and an injected process tree, never a real `claude`.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_app::claude_follow::{
    parse_record, resolve_anchor, ClaudeActivity, ClaudeTab, HookStatus, Record, SpoolReader,
    SpoolWatcher, MAX_HOPS,
};
use made_terminal::{ProtocolEvent, ProtocolState};

const ROOT: u32 = 100;
const CLAUDE: u32 = 200;
const HOOK_SHELL: u32 = 300;

/// A mutable fake process tree: `pid -> parent`. A pid that is absent has exited.
#[derive(Clone, Default)]
struct Tree(HashMap<u32, u32>);

impl Tree {
    fn new(edges: &[(u32, u32)]) -> Self {
        Self(edges.iter().copied().collect())
    }

    fn parent(&self) -> impl FnMut(u32) -> Option<u32> + '_ {
        |pid| self.0.get(&pid).copied()
    }

    fn kill(&mut self, pid: u32) {
        self.0.remove(&pid);
    }
}

/// Shell tab: root shell 100 runs claude 200; the hook shell 300 has already exited.
fn shell_tree() -> Tree {
    Tree::new(&[(ROOT, 50), (CLAUDE, ROOT), (50, 1)])
}

fn record(session: &str, cwd: &str, event: &str, status: Option<HookStatus>) -> Record {
    Record {
        session_id: session.to_owned(),
        cwd: cwd.to_owned(),
        event: event.to_owned(),
        status,
        title: String::new(),
        pids: vec![HOOK_SHELL, CLAUDE, ROOT, 50],
    }
}

fn record_via(pid: u32, mut record: Record) -> Record {
    record.pids = vec![HOOK_SHELL, pid, ROOT];
    record
}

fn line(session: &str, cwd: &str, event: &str, status: &str) -> String {
    format!(
        "{{\"at\":\"2026-01-01T00:00:00Z\",\"sessionId\":\"{session}\",\"cwd\":\"{cwd}\",\"event\":\"{event}\",\"notification\":\"\",\"status\":\"{status}\",\"title\":\"\",\"pids\":[{HOOK_SHELL},{CLAUDE},{ROOT}]}}\n"
    )
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn spool(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("tab.jsonl")
}

// ---- spool reading -------------------------------------------------------

#[test]
fn only_records_appended_after_tab_start_apply() {
    let temp = tempfile::tempdir().unwrap();
    let path = spool(&temp);
    append(&path, &line("old", "/old", "SessionStart", "idle"));
    let mut reader = SpoolReader::new(&path);
    assert!(
        reader.poll().is_empty(),
        "a record already on disk when the tab started must not apply"
    );
    append(&path, &line("new", "/new", "SessionStart", "idle"));
    let records = reader.poll();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "new");
    assert_eq!(records[0].cwd, "/new");
    assert_eq!(records[0].status, Some(HookStatus::Idle));
    assert_eq!(records[0].pids, vec![HOOK_SHELL, CLAUDE, ROOT]);
    assert!(reader.poll().is_empty(), "a record applies once");
}

#[test]
fn a_spool_created_after_the_reader_starts_is_read_from_the_start() {
    let temp = tempfile::tempdir().unwrap();
    let path = spool(&temp);
    let mut reader = SpoolReader::new(&path);
    assert!(reader.poll().is_empty());
    append(&path, &line("s", "/w", "SessionStart", "idle"));
    assert_eq!(reader.poll().len(), 1);
}

#[test]
fn malformed_truncated_and_oversized_records_are_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let path = spool(&temp);
    fs::write(&path, "").unwrap();
    let mut reader = SpoolReader::new(&path);

    append(&path, "not json at all\n");
    append(&path, "{\"sessionId\":\"s\",\"cwd\":\n");
    append(
        &path,
        "{\"sessionId\":7,\"cwd\":\"/w\",\"event\":\"Stop\",\"status\":\"idle\",\"pids\":[]}\n",
    );
    append(
        &path,
        "{\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"Stop\",\"status\":\"bogus\",\"pids\":[]}\n",
    );
    let oversized = format!(
        "{{\"sessionId\":\"big\",\"cwd\":\"/{}\",\"event\":\"Stop\",\"status\":\"idle\",\"pids\":[]}}\n",
        "d".repeat(5000)
    );
    append(&path, &oversized);
    assert!(reader.poll().is_empty());

    // A record split across two polls is not a record until its newline arrives.
    let whole = line("late", "/late", "SessionStart", "idle");
    let (first, second) = whole.split_at(whole.len() / 2);
    append(&path, first);
    assert!(
        reader.poll().is_empty(),
        "a truncated record is not applied"
    );
    append(&path, second);
    let records = reader.poll();
    assert_eq!(records.len(), 1, "the completed record applies");
    assert_eq!(records[0].session_id, "late");

    append(&path, &line("after", "/after", "Stop", "idle"));
    assert_eq!(
        reader.poll()[0].session_id,
        "after",
        "bad lines never block later records"
    );
}

#[test]
fn record_parsing_keeps_only_metadata() {
    let parsed = parse_record(
        "{\"at\":\"t\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\",\"title\":\"a\\u202eb\\nc\",\"pids\":[3,2,1],\"prompt\":\"SECRET-MARKER\"}",
    )
    .expect("a well-formed record");
    assert_eq!(parsed.status, Some(HookStatus::Working));
    assert_eq!(parsed.title, "ab c", "titles are sanitized on ingestion");
    assert_eq!(parsed.pids, vec![3, 2, 1]);
    assert!(
        !format!("{parsed:?}").contains("SECRET-MARKER"),
        "unknown payload fields never reach Made's state"
    );
    let empty = parse_record(
        "{\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"CwdChanged\",\"status\":\"\",\"pids\":[]}",
    )
    .unwrap();
    assert_eq!(empty.status, None, "an empty status keeps the current one");
    assert!(parse_record("{\"sessionId\":\"\",\"cwd\":\"/w\",\"pids\":[]}").is_none());
    let many: Vec<String> = (1..=40).map(|pid| pid.to_string()).collect();
    assert!(
        parse_record(&format!(
            "{{\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"Stop\",\"status\":\"idle\",\"pids\":[{}]}}",
            many.join(",")
        ))
        .is_none(),
        "an ancestor chain beyond 32 hops is malformed"
    );
}

#[test]
fn polling_reads_new_records_off_the_calling_thread_within_the_interval() {
    let temp = tempfile::tempdir().unwrap();
    let path = spool(&temp);
    fs::write(&path, "").unwrap();
    let watcher = SpoolWatcher::spawn(path.clone(), Duration::from_millis(50));
    let written = Instant::now();
    append(&path, &line("s", "/w", "SessionStart", "idle"));
    let mut seen = Vec::new();
    while seen.is_empty() && written.elapsed() < Duration::from_secs(3) {
        seen = watcher.drain();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.len(), 1, "the watcher delivers the record");
    assert!(
        written.elapsed() < Duration::from_millis(500),
        "delivered within the polling interval, took {:?}",
        written.elapsed()
    );
}

// ---- ownership ------------------------------------------------------------

#[test]
fn anchor_is_the_grandparent_through_the_hook_shell_else_the_parent() {
    let tree = shell_tree();
    let mut parent = tree.parent();
    // hook shell -> claude -> root: the anchor is claude.
    assert_eq!(
        resolve_anchor(ROOT, false, &[HOOK_SHELL, CLAUDE, ROOT, 50], &mut parent),
        Some(CLAUDE)
    );
    // No hook shell: claude is the hook's parent and a direct child of the root.
    assert_eq!(
        resolve_anchor(ROOT, false, &[CLAUDE, ROOT, 50], &mut parent),
        Some(CLAUDE)
    );
    // A nested wrapper between claude and the root: still the grandparent.
    let nested = Tree::new(&[(ROOT, 50), (150, ROOT), (CLAUDE, 150)]);
    assert_eq!(
        resolve_anchor(
            ROOT,
            false,
            &[HOOK_SHELL, CLAUDE, 150, ROOT],
            &mut nested.parent()
        ),
        Some(CLAUDE)
    );
}

#[test]
fn a_directly_launched_tab_anchors_on_the_terminal_child() {
    let tree = Tree::new(&[(ROOT, 50), (50, 1)]);
    let mut parent = tree.parent();
    assert_eq!(
        resolve_anchor(ROOT, true, &[HOOK_SHELL, ROOT, 50], &mut parent),
        Some(ROOT)
    );
    assert_eq!(
        resolve_anchor(ROOT, true, &[ROOT, 50], &mut parent),
        Some(ROOT),
        "without a hook shell the terminal child is the parent"
    );
}

#[test]
fn ownership_rejects_other_tabs_reparented_processes_and_exited_anchors() {
    let tree = shell_tree();
    let mut parent = tree.parent();
    // Another tab: the chain never reaches this tab's root.
    assert_eq!(
        resolve_anchor(ROOT, false, &[HOOK_SHELL, 900, 800, 1], &mut parent),
        None
    );
    // A tmux-style process: the recorded chain names the root, but the live
    // process was reparented and no longer descends from it.
    let reparented = Tree::new(&[(ROOT, 50), (CLAUDE, 1)]);
    assert_eq!(
        resolve_anchor(
            ROOT,
            false,
            &[HOOK_SHELL, CLAUDE, ROOT, 50],
            &mut reparented.parent()
        ),
        None
    );
    // The anchor already exited.
    let mut exited = shell_tree();
    exited.kill(CLAUDE);
    assert_eq!(
        resolve_anchor(
            ROOT,
            false,
            &[HOOK_SHELL, CLAUDE, ROOT, 50],
            &mut exited.parent()
        ),
        None
    );
    // An empty or unresolved chain is ignored.
    assert_eq!(resolve_anchor(ROOT, false, &[], &mut parent), None);
}

#[test]
fn ownership_stops_at_32_hops() {
    // Claude sits 40 live hops below the root.
    let mut edges = vec![(CLAUDE, 1000)];
    for hop in 0..40_u32 {
        edges.push((1000 + hop, 1001 + hop));
    }
    let far_root = 1000 + 40;
    let tree = Tree::new(&edges);
    assert_eq!(
        resolve_anchor(
            far_root,
            false,
            &[HOOK_SHELL, CLAUDE, 1000, far_root],
            &mut tree.parent()
        ),
        None,
        "live ancestry deeper than {MAX_HOPS} hops is not owned"
    );
    let near = Tree::new(&[(CLAUDE, 1000), (1000, ROOT)]);
    assert_eq!(
        resolve_anchor(
            ROOT,
            false,
            &[HOOK_SHELL, CLAUDE, 1000, ROOT],
            &mut near.parent()
        ),
        Some(CLAUDE)
    );
}

#[test]
fn foreign_and_unresolvable_records_do_not_create_a_session() {
    let tree = shell_tree();
    let mut tab = ClaudeTab::new(ROOT, false);
    let mut foreign = record("s", "/w", "SessionStart", Some(HookStatus::Idle));
    foreign.pids = vec![HOOK_SHELL, 900, 800, 1];
    tab.ingest(vec![foreign], tree.parent());
    assert!(!tab.present());
    assert_eq!(tab.directory(), None);
    assert!(tab.dots().is_empty());
}

// ---- sessions, liveness and the directory ----------------------------------

#[test]
fn liveness_drops_a_killed_session_and_keeps_others() {
    let mut tree = Tree::new(&[(ROOT, 50), (CLAUDE, ROOT), (210, ROOT)]);
    let mut tab = ClaudeTab::new(ROOT, false);
    let first = record_via(
        CLAUDE,
        record("a", "/a", "SessionStart", Some(HookStatus::Idle)),
    );
    let second = record_via(
        210,
        record("b", "/b", "SessionStart", Some(HookStatus::Idle)),
    );
    tab.ingest(vec![first, second], tree.parent());
    assert_eq!(
        tab.dots().len(),
        2,
        "separate processes stay separate sessions"
    );

    tree.kill(210);
    assert!(tab.check_liveness(tree.parent()));
    assert_eq!(tab.dots().len(), 1);
    assert_eq!(tab.directory().as_deref(), Some("/a"));
    assert!(!tab.check_liveness(tree.parent()), "nothing else changed");

    tree.kill(CLAUDE);
    assert!(tab.check_liveness(tree.parent()));
    assert!(!tab.present());
    assert!(tab.dots().is_empty());
}

#[test]
fn a_new_session_id_from_the_same_process_replaces_the_old_one() {
    let tree = shell_tree();
    let mut tab = ClaudeTab::new(ROOT, false);
    tab.ingest(
        vec![record(
            "old",
            "/old",
            "SessionStart",
            Some(HookStatus::Working),
        )],
        tree.parent(),
    );
    tab.ingest(
        vec![
            record("old", "/old", "SessionEnd", Some(HookStatus::Ended)),
            record("new", "/new", "SessionStart", Some(HookStatus::Idle)),
        ],
        tree.parent(),
    );
    assert_eq!(
        tab.dots(),
        vec![ClaudeActivity::Idle],
        "one dot after /clear"
    );
    assert_eq!(tab.directory().as_deref(), Some("/new"));

    // A resume that never recorded the old end still replaces it.
    tab.ingest(
        vec![record(
            "resumed",
            "/resumed",
            "SessionStart",
            Some(HookStatus::Idle),
        )],
        tree.parent(),
    );
    assert_eq!(tab.dots().len(), 1);
    // A late end for a replaced session id does not end the current one.
    tab.ingest(
        vec![record("new", "/new", "SessionEnd", Some(HookStatus::Ended))],
        tree.parent(),
    );
    assert_eq!(tab.dots().len(), 1);
    assert_eq!(tab.directory().as_deref(), Some("/resumed"));
    // Its own end removes it.
    tab.ingest(
        vec![record(
            "resumed",
            "/resumed",
            "SessionEnd",
            Some(HookStatus::Ended),
        )],
        tree.parent(),
    );
    assert!(!tab.present());
}

#[test]
fn the_directory_is_the_cwd_of_the_latest_owned_record_of_any_event() {
    let tree = shell_tree();
    let mut tab = ClaudeTab::new(ROOT, false);
    // `claude -w`: the start reports the original directory, later records the worktree.
    tab.ingest(
        vec![record("w", "/repo", "SessionStart", Some(HookStatus::Idle))],
        tree.parent(),
    );
    assert_eq!(tab.directory().as_deref(), Some("/repo"));
    tab.ingest(
        vec![record(
            "w",
            "/repo/.claude/worktrees/w",
            "UserPromptSubmit",
            Some(HookStatus::Working),
        )],
        tree.parent(),
    );
    assert_eq!(
        tab.directory().as_deref(),
        Some("/repo/.claude/worktrees/w")
    );
    tab.ingest(
        vec![record(
            "w",
            "/elsewhere",
            "Notification",
            Some(HookStatus::Attention),
        )],
        tree.parent(),
    );
    assert_eq!(tab.directory().as_deref(), Some("/elsewhere"));
    // A record without a directory keeps the last one.
    tab.ingest(
        vec![record("w", "", "Stop", Some(HookStatus::Idle))],
        tree.parent(),
    );
    assert_eq!(tab.directory().as_deref(), Some("/elsewhere"));
}

#[test]
fn with_two_sessions_the_most_recently_active_one_supplies_the_directory() {
    let tree = Tree::new(&[(ROOT, 50), (CLAUDE, ROOT), (210, ROOT)]);
    let mut tab = ClaudeTab::new(ROOT, false);
    let first = record_via(
        CLAUDE,
        record("a", "/a", "SessionStart", Some(HookStatus::Idle)),
    );
    let second = record_via(
        210,
        record("b", "/b", "SessionStart", Some(HookStatus::Idle)),
    );
    tab.ingest(vec![first, second], tree.parent());
    assert_eq!(tab.directory().as_deref(), Some("/b"));
    let prompt = record_via(
        CLAUDE,
        record("a", "/a", "UserPromptSubmit", Some(HookStatus::Working)),
    );
    tab.ingest(vec![prompt], tree.parent());
    assert_eq!(tab.directory().as_deref(), Some("/a"));
}

// ---- presence -------------------------------------------------------------

fn status(state: ProtocolState) -> ProtocolEvent {
    ProtocolEvent::Status { state }
}

#[test]
fn each_source_makes_claude_code_present() {
    let tree = shell_tree();

    // Directly launched: the child is enough, before any status or hook record.
    let mut direct = ClaudeTab::new(ROOT, true);
    assert!(!direct.present(), "no child yet");
    direct.set_child_running(true);
    assert!(direct.present());
    assert!(
        direct.dots().is_empty(),
        "no dot before any status or record"
    );

    // Shell tab with a live owned session and hooks only.
    let mut hooks = ClaudeTab::new(ROOT, false);
    hooks.set_child_running(true);
    assert!(!hooks.present(), "a running shell alone is not Claude Code");
    hooks.ingest(
        vec![record("s", "/w", "SessionStart", Some(HookStatus::Idle))],
        tree.parent(),
    );
    assert!(hooks.present());

    // Shell tab whose Claude Code has hooks disabled and sends working.
    let mut protocol = ClaudeTab::new(ROOT, false);
    protocol.note_protocol(status(ProtocolState::Working));
    assert!(protocol.present());
    assert_eq!(protocol.dots(), vec![ClaudeActivity::Busy]);
    assert_eq!(
        protocol.directory(),
        None,
        "presence alone supplies no directory"
    );
}

#[test]
fn protocol_only_presence_ends_on_clear_child_exit_or_the_shell_prompt() {
    let live = || {
        let mut tab = ClaudeTab::new(ROOT, false);
        tab.set_child_running(true);
        tab.note_protocol(status(ProtocolState::Working));
        assert!(tab.present());
        tab
    };
    let mut cleared = live();
    cleared.note_protocol(ProtocolEvent::Clear { bare: false });
    assert!(!cleared.present());

    let mut bare = live();
    bare.note_protocol(ProtocolEvent::Clear { bare: true });
    assert!(!bare.present(), "a bare clear ends a live status");

    let mut exited = live();
    exited.set_child_running(false);
    assert!(!exited.present());

    let mut prompt = live();
    prompt.note_shell_prompt();
    assert!(!prompt.present());

    // A bare clear with no live status is ignored and cannot invent presence.
    let mut idle = ClaudeTab::new(ROOT, false);
    idle.note_protocol(ProtocolEvent::Clear { bare: true });
    assert!(!idle.present());

    // A prompt report does not end a live owned session.
    let tree = shell_tree();
    let mut session = ClaudeTab::new(ROOT, false);
    session.ingest(
        vec![record("s", "/w", "SessionStart", Some(HookStatus::Idle))],
        tree.parent(),
    );
    session.note_shell_prompt();
    assert!(session.present());
}

#[test]
fn a_direct_tab_stays_present_while_its_child_runs() {
    let mut tab = ClaudeTab::new(ROOT, true);
    tab.set_child_running(true);
    tab.note_protocol(status(ProtocolState::Working));
    tab.note_protocol(ProtocolEvent::Clear { bare: false });
    assert!(tab.present(), "the running child still makes it present");
    tab.set_child_running(false);
    assert!(!tab.present());
}

#[test]
fn channel_observations_are_exposed_for_diagnostics() {
    let mut tab = ClaudeTab::new(ROOT, false);
    assert!(!tab.protocol_probe_seen());
    assert!(!tab.protocol_status_seen());
    tab.note_protocol(ProtocolEvent::Probe);
    assert!(tab.protocol_probe_seen());
    assert!(
        !tab.protocol_status_seen(),
        "the probe alone is not a status emission"
    );
    assert!(!tab.present(), "a probe alone is not presence");
    tab.note_protocol(status(ProtocolState::Idle));
    assert!(tab.protocol_status_seen());
}

// ---- dots -----------------------------------------------------------------

#[test]
fn dot_status_prefers_protocol_then_hook_then_idle() {
    let tree = Tree::new(&[(ROOT, 50), (CLAUDE, ROOT), (210, ROOT)]);
    let mut tab = ClaudeTab::new(ROOT, false);
    let older = record_via(
        CLAUDE,
        record("a", "/a", "SessionStart", Some(HookStatus::Attention)),
    );
    let newer = record_via(
        210,
        record("b", "/b", "UserPromptSubmit", Some(HookStatus::Working)),
    );
    tab.ingest(vec![older, newer], tree.parent());
    assert_eq!(
        tab.dots(),
        vec![ClaudeActivity::Attention, ClaudeActivity::Busy],
        "hook-derived status without a protocol status"
    );

    tab.note_protocol(status(ProtocolState::Done));
    assert_eq!(
        tab.dots(),
        vec![ClaudeActivity::Attention, ClaudeActivity::Idle],
        "the protocol status applies to the most recent session only; a finished turn is idle"
    );
    tab.note_protocol(status(ProtocolState::Blocked));
    assert_eq!(tab.dots()[1], ClaudeActivity::Attention);
    tab.note_protocol(status(ProtocolState::Error));
    assert_eq!(tab.dots()[1], ClaudeActivity::Attention);
    tab.note_protocol(ProtocolEvent::Clear { bare: false });
    assert_eq!(
        tab.dots()[1],
        ClaudeActivity::Busy,
        "back to the hook status"
    );

    let mut idle = ClaudeTab::new(ROOT, false);
    idle.ingest(
        vec![record("s", "/w", "CwdChanged", None)],
        shell_tree().parent(),
    );
    assert_eq!(idle.dots(), vec![ClaudeActivity::Idle], "no status is idle");
}

#[test]
fn hook_statuses_follow_the_event_mapping() {
    let tree = shell_tree();
    let mut tab = ClaudeTab::new(ROOT, false);
    for (event, status, expected) in [
        (
            "UserPromptSubmit",
            Some(HookStatus::Working),
            ClaudeActivity::Busy,
        ),
        (
            "PermissionRequest",
            Some(HookStatus::Attention),
            ClaudeActivity::Attention,
        ),
        (
            "Notification",
            Some(HookStatus::Attention),
            ClaudeActivity::Attention,
        ),
        ("Notification", Some(HookStatus::Idle), ClaudeActivity::Idle),
        ("Stop", Some(HookStatus::Idle), ClaudeActivity::Idle),
        (
            "StopFailure",
            Some(HookStatus::Error),
            ClaudeActivity::Attention,
        ),
        ("CwdChanged", None, ClaudeActivity::Attention),
    ] {
        tab.ingest(vec![record("s", "/w", event, status)], tree.parent());
        assert_eq!(tab.dots(), vec![expected], "{event} {status:?}");
    }
}

// ---- tooltip title ----------------------------------------------------------

#[test]
fn title_chain_uses_the_window_title_only_while_protocol_status_is_live() {
    let tree = shell_tree();
    let mut tab = ClaudeTab::new(ROOT, false);
    let mut titled = record("s", "/w", "UserPromptSubmit", Some(HookStatus::Working));
    titled.title = "foo".to_owned();
    tab.ingest(vec![titled], tree.parent());

    assert_eq!(
        tab.tooltip_title(Some("◐ Refactor parser")),
        "foo",
        "without a protocol status the window title is not read"
    );
    tab.note_protocol(status(ProtocolState::Working));
    assert_eq!(
        tab.tooltip_title(Some("Refactor parser")),
        "Refactor parser"
    );
    assert_eq!(
        tab.tooltip_title(Some("")),
        "foo",
        "an empty window title falls through to the hook title"
    );
    assert_eq!(tab.tooltip_title(None), "foo");

    let mut plain = ClaudeTab::new(ROOT, false);
    plain.note_protocol(status(ProtocolState::Idle));
    assert_eq!(plain.tooltip_title(Some("   ")), "Claude Code");
    assert_eq!(plain.tooltip_title(None), "Claude Code");
    let unprotocolled = ClaudeTab::new(ROOT, true);
    assert_eq!(unprotocolled.tooltip_title(Some("Window")), "Claude Code");
}

#[test]
fn the_window_title_is_sanitized_and_capped() {
    let mut tab = ClaudeTab::new(ROOT, false);
    tab.note_protocol(status(ProtocolState::Working));
    let hostile = format!("a\u{1b}[31m\nb\u{202e}c{}", "x".repeat(300));
    let shown = tab.tooltip_title(Some(&hostile));
    assert!(shown.chars().count() <= 120, "{}", shown.chars().count());
    assert!(
        !shown
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{202e}' | '\n')),
        "{shown:?}"
    );
    assert!(shown.starts_with("a [31m b"), "{shown:?}");
}

#[test]
fn the_hook_title_comes_from_the_most_recent_live_session() {
    let tree = Tree::new(&[(ROOT, 50), (CLAUDE, ROOT), (210, ROOT)]);
    let mut tab = ClaudeTab::new(ROOT, false);
    let mut first = record_via(
        CLAUDE,
        record("a", "/a", "UserPromptSubmit", Some(HookStatus::Working)),
    );
    first.title = "first".to_owned();
    let mut second = record_via(
        210,
        record("b", "/b", "UserPromptSubmit", Some(HookStatus::Working)),
    );
    second.title = "second".to_owned();
    tab.ingest(vec![first, second], tree.parent());
    assert_eq!(tab.tooltip_title(None), "second");
    let stop = record_via(210, record("b", "/b", "Stop", Some(HookStatus::Idle)));
    tab.ingest(vec![stop], tree.parent());
    assert_eq!(
        tab.tooltip_title(None),
        "second",
        "a record without a title keeps the last title of its session"
    );
}

// ---- workspaces -----------------------------------------------------------

#[test]
fn presence_does_not_cross_tabs() {
    let tree = shell_tree();
    let mut a = ClaudeTab::new(ROOT, false);
    let mut b = ClaudeTab::new(777, false);
    a.ingest(
        vec![record("s", "/w", "SessionStart", Some(HookStatus::Idle))],
        tree.parent(),
    );
    b.ingest(
        vec![record("s", "/w", "SessionStart", Some(HookStatus::Idle))],
        tree.parent(),
    );
    assert!(a.present());
    assert!(
        !b.present(),
        "tab B's root is not in the record's live ancestry"
    );
}
