//! Companion launch of a remote Claude Code (SSH-11). Fixtures are temp
//! directories; nothing is spawned, version-checked or written.
use made_platform::Environment;
use made_remote::grok;

#[cfg(unix)]
fn executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
#[cfg(unix)]
fn the_launch_program_is_the_absolute_claude_on_path() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    let environment = Environment::from_pairs([("PATH", bin.as_os_str().to_os_string())]);
    let program = grok::detect_on_path(&environment, "claude").expect("claude on PATH");
    assert_eq!(program, bin.join("claude"));
    assert!(program.is_absolute());
}

#[test]
#[cfg(unix)]
fn detection_ignores_relative_and_empty_path_entries_and_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    for path in ["relative", "", "bin::."] {
        let environment = Environment::from_pairs([("PATH", path)]);
        assert!(
            grok::detect_on_path(&environment, "claude").is_none(),
            "{path:?}"
        );
    }
    // A vanished executable fails closed instead of substituting a program.
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let environment = Environment::from_pairs([("PATH", empty.as_os_str().to_os_string())]);
    assert!(grok::detect_on_path(&environment, "claude").is_none());
    assert!(grok::detect_on_path(&Environment::from_pairs([("HOME", "/x")]), "claude").is_none());
}

// ---- shared pure helpers (moved from the local Claude Code module) ----------

mod shared_helpers {
    use made_remote::claude::{
        normalize_title, parse_agents, parse_record, parse_record_line, AgentCwd, HookStatus,
        MAX_RECORD_BYTES,
    };

    fn line(fields: &str) -> String {
        format!("{{\"sessionId\":\"s1\",{fields}}}")
    }

    #[test]
    fn a_record_keeps_only_its_metadata_fields() {
        let record = parse_record(&line(
            "\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"status\":\"working\",\
             \"title\":\"  Fix\\u0007 it \\u202e \",\"pids\":[3,2,1],\
             \"prompt\":\"MARKER\",\"tool_input\":{\"command\":\"MARKER\"}",
        ))
        .expect("a well-formed record");
        assert_eq!(record.session_id, "s1");
        assert_eq!(record.cwd, "/work");
        assert_eq!(record.event, "UserPromptSubmit");
        assert_eq!(record.status, Some(HookStatus::Working));
        assert_eq!(record.title, "Fix  it");
        assert_eq!(record.pids, vec![3, 2, 1]);
        assert!(!format!("{record:?}").contains("MARKER"));
        for (status, expected) in [
            ("", None),
            ("idle", Some(HookStatus::Idle)),
            ("attention", Some(HookStatus::Attention)),
            ("error", Some(HookStatus::Error)),
            ("ended", Some(HookStatus::Ended)),
        ] {
            let record = parse_record(&line(&format!("\"status\":\"{status}\""))).unwrap();
            assert_eq!(record.status, expected, "{status}");
        }
    }

    #[test]
    fn malformed_and_truncated_records_are_rejected() {
        for bad in [
            "",
            "not json",
            "{\"sessionId\":\"s1\",\"cwd\":\"/work\"",
            "{\"cwd\":\"/work\"}",
            "{\"sessionId\":\"\"}",
            "{\"sessionId\":\"s1\",\"status\":\"busy\"}",
            "{\"sessionId\":\"s1\",\"pids\":\"1\"}",
        ] {
            assert_eq!(parse_record(bad), None, "{bad}");
        }
        let too_deep: Vec<String> = (0..33).map(|pid| pid.to_string()).collect();
        assert_eq!(
            parse_record(&line(&format!("\"pids\":[{}]", too_deep.join(",")))),
            None
        );
    }

    #[test]
    fn a_record_line_is_bounded_to_four_kib() {
        let base = line("\"cwd\":\"/w\",\"title\":\"");
        let fits = MAX_RECORD_BYTES - base.len() - "\"}\n".len();
        let at_bound = format!("{base}{}\"}}\n", "x".repeat(fits));
        assert_eq!(at_bound.len(), MAX_RECORD_BYTES);
        assert!(parse_record_line(at_bound.as_bytes()).is_some());
        let over = format!("{base}{}\"}}\n", "x".repeat(fits + 1));
        assert_eq!(parse_record_line(over.as_bytes()), None);
        assert_eq!(parse_record_line(b"{\"sessionId\":\"s1\",\xff}\n"), None);
    }

    // The `:` list and its absolute paths are Unix ones; the companion that
    // composes it runs only on Unix servers.
    #[test]
    #[cfg(unix)]
    fn plugin_dirs_keep_order_add_one_entry_and_skip_an_unsafe_path() {
        use made_remote::claude::compose_plugin_dirs;
        use std::ffi::{OsStr, OsString};
        use std::path::Path;

        let shipped = Path::new("/srv/made/plugins/claude-code-made");
        let made = "/srv/made/plugins/claude-code-made";
        let compose =
            |existing: Option<&str>| compose_plugin_dirs(existing.map(OsStr::new), shipped, ':');
        assert_eq!(compose(None), OsString::from(made), "single entry");
        assert_eq!(
            compose(Some("/u/b:/u/a")),
            OsString::from(format!("/u/b:/u/a:{made}")),
            "order kept"
        );
        assert_eq!(compose(Some(made)), OsString::from(made), "not duplicated");
        assert_eq!(
            compose(Some(&format!("/u/a:{made}/"))),
            OsString::from(format!("/u/a:{made}/"))
        );
        for unsafe_path in ["relative/dir", "/a:b", "/a$b", "/a\"b", "/a\\b", "/a\nb"] {
            assert_eq!(
                compose_plugin_dirs(Some(OsStr::new("/u/a")), Path::new(unsafe_path), ':'),
                OsString::from("/u/a"),
                "{unsafe_path:?}"
            );
            assert_eq!(
                compose_plugin_dirs(None, Path::new(unsafe_path), ':'),
                OsString::new()
            );
        }
    }

    #[test]
    fn a_listing_keeps_only_pid_cwd_kind_and_name_and_drops_a_malformed_entry_alone() {
        let entries = parse_agents(
            br#"[{"pid":7,"cwd":"/work/a","kind":"interactive","name":"x","sessionId":"MARKER"},
                 {"pid":"bad","cwd":"/work/bad"},
                 {"pid":8,"cwd":"/work/b","kind":"background","name":"  \u2733 Spec task ","state":"MARKER"},
                 {"pid":9,"kind":"interactive"},
                 5]"#,
        )
        .expect("a JSON array");
        assert_eq!(
            entries,
            vec![
                AgentCwd {
                    pid: 7,
                    cwd: "/work/a".to_owned(),
                    background: false,
                    name: String::new(),
                },
                AgentCwd {
                    pid: 8,
                    cwd: "/work/b".to_owned(),
                    background: true,
                    name: "Spec task".to_owned(),
                },
            ]
        );
        assert!(!format!("{entries:?}").contains("work"));
        assert_eq!(parse_agents(b"{}"), None);
    }

    #[test]
    fn titles_normalize_like_the_local_tooltip() {
        assert_eq!(normalize_title("\u{2733} Spec task  "), "Spec task");
        assert_eq!(
            normalize_title("\u{2802} \u{00b7} Spec\u{0007}task"),
            "Spec task"
        );
        assert_eq!(normalize_title("\u{202e}Spec"), "Spec");
        assert_eq!(normalize_title(&"x".repeat(200)).chars().count(), 120);
    }
}

// ---- server-side launch environment (SSH-11) --------------------------------

#[cfg(unix)]
mod environment {
    use made_platform::Environment;
    use made_remote::claude::{
        launch_environment, spool_path, staged_plugin_dir, CLAUDE_CODE_PLUGIN_DIRS_ENV,
        CLAUDE_SPOOL_ENV,
    };
    use made_remote::LaunchTarget;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// A companion beside a staged claude-code-made package.
    fn staged_companion(root: &Path, hook_mode: Option<u32>) -> PathBuf {
        let companion = root.join("made-remote");
        std::fs::write(&companion, "companion").unwrap();
        let package = root.join("plugins/claude-code-made");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::create_dir_all(package.join(".claude-plugin")).unwrap();
        std::fs::write(package.join(".claude-plugin/plugin.json"), "{}").unwrap();
        if let Some(mode) = hook_mode {
            let hook = package.join("bin/made-claude-status");
            std::fs::write(&hook, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        companion
    }

    fn value(env: &[(OsString, OsString)], key: &str) -> Option<String> {
        env.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    }

    #[test]
    fn shell_and_claude_targets_get_the_staged_entry_and_a_spool_binding() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let companion = staged_companion(&root, Some(0o700));
        let staged = staged_plugin_dir(&companion).expect("a ready staged package");
        assert_eq!(staged, root.join("plugins/claude-code-made"));
        let spool = spool_path(&root.join("runtime/tab"));
        assert_eq!(
            spool,
            root.join("runtime/tab/claude/0-supervisor/spool.jsonl")
        );
        let made = staged.to_string_lossy().into_owned();
        // The remote user's own entries are kept in order; there is no
        // claude on this PATH, which changes nothing.
        let user = Environment::from_pairs([
            (CLAUDE_CODE_PLUGIN_DIRS_ENV, "/u/one:/u/two"),
            ("PATH", "/nowhere"),
        ]);
        for target in [LaunchTarget::Shell, LaunchTarget::Claude] {
            let composed = launch_environment(target, Some(&staged), &user, &spool);
            assert!(composed.hooks_available, "{target:?}");
            assert_eq!(
                value(&composed.env, CLAUDE_CODE_PLUGIN_DIRS_ENV).as_deref(),
                Some(format!("/u/one:/u/two:{made}").as_str())
            );
            assert_eq!(
                value(&composed.env, CLAUDE_SPOOL_ENV).as_deref(),
                Some(spool.to_string_lossy().as_ref())
            );
            assert_eq!(composed.env.len(), 2);
        }
        // Without a user value, Made's entry is the whole list; an existing
        // Made entry is not added twice.
        let bare = Environment::from_pairs([("HOME", "/home/u")]);
        let composed = launch_environment(LaunchTarget::Shell, Some(&staged), &bare, &spool);
        assert_eq!(
            value(&composed.env, CLAUDE_CODE_PLUGIN_DIRS_ENV),
            Some(made.clone())
        );
        let present = Environment::from_pairs([(CLAUDE_CODE_PLUGIN_DIRS_ENV, made.as_str())]);
        let composed = launch_environment(LaunchTarget::Shell, Some(&staged), &present, &spool);
        assert_eq!(
            value(&composed.env, CLAUDE_CODE_PLUGIN_DIRS_ENV),
            Some(made)
        );
    }

    #[test]
    fn opencode_and_grok_targets_get_neither() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let staged = staged_plugin_dir(&staged_companion(&root, Some(0o700))).unwrap();
        let user = Environment::from_pairs([(CLAUDE_CODE_PLUGIN_DIRS_ENV, "/u/one")]);
        for target in [LaunchTarget::OpenCode, LaunchTarget::Grok] {
            let composed = launch_environment(target, Some(&staged), &user, &spool_path(&root));
            assert!(composed.env.is_empty(), "{target:?}");
            assert!(!composed.hooks_available, "{target:?}");
        }
    }

    #[test]
    fn a_missing_or_unready_staged_package_adds_no_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        // No package at all: an older staged set.
        let bare = root.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("made-remote"), "companion").unwrap();
        assert_eq!(staged_plugin_dir(&bare.join("made-remote")), None);
        // A package whose hook is missing or not executable.
        for (name, mode) in [("nohook", None), ("noexec", Some(0o600))] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            assert_eq!(
                staged_plugin_dir(&staged_companion(&dir, mode)),
                None,
                "{name}"
            );
        }
        // An unreadable package directory.
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        let companion = staged_companion(&locked, Some(0o700));
        let package = locked.join("plugins/claude-code-made");
        std::fs::set_permissions(&package, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads a mode-000 directory, so the check means nothing there.
        let enforced = std::fs::read_dir(&package).is_err();
        let unreadable = staged_plugin_dir(&companion);
        std::fs::set_permissions(&package, std::fs::Permissions::from_mode(0o700)).unwrap();
        if enforced {
            assert_eq!(unreadable, None);
        } else {
            eprintln!("skipping unreadable-package assertion when running as root");
        }
        let user = Environment::from_pairs([(CLAUDE_CODE_PLUGIN_DIRS_ENV, "/u/one")]);
        for target in [LaunchTarget::Shell, LaunchTarget::Claude] {
            let composed = launch_environment(target, None, &user, &spool_path(&root));
            assert!(composed.env.is_empty(), "{target:?}");
            assert!(!composed.hooks_available);
        }
    }

    #[test]
    fn only_the_given_remote_environment_is_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let staged = staged_plugin_dir(&staged_companion(&root, Some(0o700))).unwrap();
        // Whatever this process inherited (the host side of a test run) is
        // not consulted: only the remote environment passed in composes.
        let remote = Environment::from_pairs([(CLAUDE_CODE_PLUGIN_DIRS_ENV, "/remote/only")]);
        let composed = launch_environment(
            LaunchTarget::Shell,
            Some(&staged),
            &remote,
            &spool_path(&root),
        );
        let list = value(&composed.env, CLAUDE_CODE_PLUGIN_DIRS_ENV).unwrap();
        assert_eq!(list, format!("/remote/only:{}", staged.display()));
    }
}

// ---- companion tracker (SSH-11 ownership, liveness, privacy) -----------------

#[cfg(unix)]
mod tracker {
    use made_remote::claude::{audit_file_access, ClaudeTracker};
    use made_remote::{
        encode, AuthorityId, ClaudeActivity, ClaudeSessionSnapshot, Identity, Operation,
        TabCapability, WireEnvelope,
    };
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const MARKERS: [&str; 4] = [
        "MARKER-PROMPT-c1",
        "MARKER-TOOL-c1",
        "MARKER-MSG-c1",
        "MARKER-TRANSCRIPT-c1",
    ];

    /// A mutable process table: pid -> parent.
    #[derive(Clone, Default)]
    struct Table(Arc<Mutex<HashMap<u32, u32>>>);
    impl Table {
        fn with(pairs: &[(u32, u32)]) -> Self {
            Self(Arc::new(Mutex::new(pairs.iter().copied().collect())))
        }
        fn kill(&self, pid: u32) {
            self.0.lock().unwrap().remove(&pid);
        }
        fn parent_of(&self) -> Box<dyn FnMut(u32) -> Option<u32> + Send> {
            let table = self.0.clone();
            Box::new(move |pid| table.lock().unwrap().get(&pid).copied())
        }
    }

    fn record(session: &str, cwd: &str, event: &str, status: &str, pids: &[u32]) -> String {
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        format!(
            "{{\"sessionId\":\"{session}\",\"cwd\":\"{cwd}\",\"event\":\"{event}\",\"status\":\"{status}\",\
             \"title\":\"\",\"pids\":[{}],\"prompt\":\"{}\",\"tool_input\":{{\"command\":\"{}\"}},\
             \"msg\":\"{}\",\"transcript_path\":\"/t/{}.jsonl\"}}\n",
            pids.join(","),
            MARKERS[0],
            MARKERS[1],
            MARKERS[2],
            MARKERS[3]
        )
    }

    fn append(spool: &Path, text: &str) {
        std::fs::OpenOptions::new()
            .append(true)
            .open(spool)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn spool(dir: &Path) -> PathBuf {
        let path = dir.join("spool.jsonl");
        std::fs::write(&path, "").unwrap();
        path
    }

    /// The latest snapshot `operations` carry.
    fn snapshot(operations: &[Operation]) -> Option<ClaudeSessionSnapshot> {
        operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                Operation::ClaudeEvent { snapshot } | Operation::ClaudeSnapshot { snapshot } => {
                    Some(snapshot.clone())
                }
                _ => None,
            })
    }

    fn state(tracker: &mut ClaudeTracker) -> ClaudeSessionSnapshot {
        snapshot(&[tracker.state()]).unwrap()
    }

    fn cwd(snapshot: &ClaudeSessionSnapshot, index: usize) -> String {
        String::from_utf8(
            snapshot.sessions[index]
                .cwd
                .clone()
                .unwrap()
                .as_bytes()
                .to_vec(),
        )
        .unwrap()
    }

    // Shell 10 is the tab root; Claude Code 20 runs under it; the hook shell
    // 99 has already exited when the record is read.
    fn shell_tab(dir: &Path, table: &Table) -> (ClaudeTracker, PathBuf) {
        let path = spool(dir);
        let tracker = ClaudeTracker::with_parent(
            Some(10),
            false,
            Some(path.clone()),
            None,
            table.parent_of(),
        );
        (tracker, path)
    }

    #[test]
    fn only_records_appended_after_the_session_started_apply() {
        let temp = tempfile::tempdir().unwrap();
        let table = Table::with(&[(10, 1), (20, 10)]);
        let path = spool(temp.path());
        append(
            &path,
            &record("old", "/work/old", "SessionStart", "idle", &[99, 20]),
        );
        let mut tracker = ClaudeTracker::with_parent(
            Some(10),
            false,
            Some(path.clone()),
            None,
            table.parent_of(),
        );
        let now = Instant::now();
        tracker.tick(now, true);
        assert!(
            state(&mut tracker).sessions.is_empty(),
            "history is not applied"
        );
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20, 10]),
        );
        let pushed = tracker.tick(now + Duration::from_millis(200), true);
        let snapshot = snapshot(&pushed).expect("a pushed change");
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot.hooks_available);
        assert_eq!(snapshot.sessions[0].pid, 20);
        assert_eq!(cwd(&snapshot, 0), "/work/a");
    }

    #[test]
    fn foreign_tmux_exited_and_other_tab_processes_are_not_claimed() {
        let temp = tempfile::tempdir().unwrap();
        // Tab A's shell 10; tab B's shell 40 with Claude Code 41; a tmux server
        // 50 reparented to init with Claude Code 51 that inherited A's spool.
        let table = Table::with(&[(10, 1), (40, 1), (41, 40), (50, 1), (51, 50)]);
        let (mut a, path) = shell_tab(temp.path(), &table);
        let now = Instant::now();
        a.tick(now, true);
        for pids in [&[99, 41, 40][..], &[99, 51, 50], &[99, 77, 10], &[]] {
            append(&path, &record("x", "/work/x", "SessionStart", "idle", pids));
        }
        a.tick(now + Duration::from_millis(200), true);
        assert!(state(&mut a).sessions.is_empty());
        // Tab B owns its own process; A's spool never reaches it.
        let b_dir = tempfile::tempdir().unwrap();
        let b_path = spool(b_dir.path());
        let mut b = ClaudeTracker::with_parent(
            Some(40),
            false,
            Some(b_path.clone()),
            None,
            table.parent_of(),
        );
        b.tick(now, true);
        append(
            &b_path,
            &record("b", "/work/b", "SessionStart", "idle", &[99, 41, 40]),
        );
        b.tick(now + Duration::from_millis(200), true);
        assert_eq!(state(&mut b).sessions[0].session_id, "b");
        assert!(state(&mut a).sessions.is_empty());
    }

    #[test]
    fn one_session_per_process_across_clear_and_exit_within_a_second() {
        let temp = tempfile::tempdir().unwrap();
        let table = Table::with(&[(10, 1), (20, 10), (30, 10)]);
        let (mut tab, path) = shell_tab(temp.path(), &table);
        let start = Instant::now();
        tab.tick(start, true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        append(
            &path,
            &record("p", "/work/p", "SessionStart", "idle", &[99, 30]),
        );
        tab.tick(start + Duration::from_millis(100), true);
        assert_eq!(state(&mut tab).sessions.len(), 2, "separate processes");
        // `/clear` starts a new session id in the same process.
        append(
            &path,
            &record("s2", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        append(
            &path,
            &record("s2", "/work/a", "UserPromptSubmit", "working", &[99, 20]),
        );
        tab.tick(start + Duration::from_millis(200), true);
        let current = state(&mut tab);
        let ids: Vec<&str> = current
            .sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert_eq!(ids, ["p", "s2"]);
        assert_eq!(current.sessions[1].activity, ClaudeActivity::Busy);
        // Process 20 exits: the session is gone within about a second.
        table.kill(20);
        let pushed = tab.tick(start + Duration::from_millis(1300), true);
        let after = snapshot(&pushed).expect("the exit is pushed");
        assert_eq!(after.sessions.len(), 1);
        assert_eq!(after.sessions[0].session_id, "p");
    }

    #[test]
    fn malformed_truncated_and_oversized_records_change_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let table = Table::with(&[(10, 1), (20, 10)]);
        let (mut tab, path) = shell_tab(temp.path(), &table);
        let start = Instant::now();
        tab.tick(start, true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        tab.tick(start + Duration::from_millis(100), true);
        let before = state(&mut tab);
        let oversized = record(
            "s1",
            &format!("/work/{}", "x".repeat(5000)),
            "CwdChanged",
            "",
            &[99, 20],
        );
        append(&path, "{not json}\n");
        append(&path, &oversized);
        append(&path, "{\"sessionId\":\"s1\",\"cwd\":\"/work/torn\"");
        append(
            &path,
            &record("s1", "relative/dir", "CwdChanged", "", &[99, 20]),
        );
        let pushed = tab.tick(start + Duration::from_millis(200), true);
        assert!(snapshot(&pushed).is_none(), "nothing changed");
        let after = state(&mut tab);
        assert_eq!(after.sessions, before.sessions);
    }

    #[test]
    fn state_and_relay_frames_carry_only_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let table = Table::with(&[(10, 1), (20, 10)]);
        let (mut tab, path) = shell_tab(temp.path(), &table);
        let start = Instant::now();
        tab.tick(start, true);
        for (event, status) in [
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "working"),
            ("Notification", "attention"),
            ("Stop", "idle"),
        ] {
            append(&path, &record("s1", "/work/a", event, status, &[99, 20]));
        }
        let mut frames = tab.tick(start + Duration::from_millis(100), true);
        frames.push(tab.state());
        let identity = Identity {
            authority: AuthorityId([1; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        for operation in frames {
            let bytes = encode(&WireEnvelope::new(identity.clone(), 1, operation)).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for marker in MARKERS {
                assert!(!text.contains(marker), "{marker} relayed: {text}");
            }
        }
    }

    #[test]
    fn the_snapshot_stays_inside_the_frame_bound() {
        let temp = tempfile::tempdir().unwrap();
        let mut pairs = vec![(10, 1)];
        pairs.extend((0..64).map(|i| (100 + i, 10)));
        let table = Table::with(&pairs);
        let (mut tab, path) = shell_tab(temp.path(), &table);
        let start = Instant::now();
        tab.tick(start, true);
        let long = format!("/{}", "d".repeat(3000));
        for i in 0..64u32 {
            let cwd = format!("{long}/{i}");
            append(
                &path,
                &record(
                    &format!("s{i}"),
                    &cwd,
                    "SessionStart",
                    "idle",
                    &[99, 100 + i],
                ),
            );
        }
        let pushed = tab.tick(start + Duration::from_millis(100), true);
        let identity = Identity {
            authority: AuthorityId([1; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let latest = snapshot(&pushed).expect("a pushed snapshot");
        assert!(!latest.sessions.is_empty());
        assert!(
            latest.sessions.len() < 64,
            "the least recent sessions are left out"
        );
        for operation in pushed {
            encode(&WireEnvelope::new(identity.clone(), 1, operation)).expect("within the bound");
        }
        encode(&WireEnvelope::new(identity, 1, tab.state())).expect("within the bound");
    }

    #[test]
    fn a_tracker_without_a_spool_reports_hooks_unavailable() {
        let table = Table::with(&[(10, 1)]);
        let mut tab = ClaudeTracker::with_parent(Some(10), false, None, None, table.parent_of());
        tab.tick(Instant::now(), true);
        let snapshot = state(&mut tab);
        assert!(!snapshot.hooks_available);
        assert!(snapshot.sessions.is_empty());
        assert!(snapshot.revision > 0);
    }

    #[test]
    fn reattach_restarts_revisions_with_the_live_state_and_no_replay() {
        let temp = tempfile::tempdir().unwrap();
        let table = Table::with(&[(10, 1), (20, 10)]);
        let (mut tab, path) = shell_tab(temp.path(), &table);
        let start = Instant::now();
        tab.tick(start, true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        tab.tick(start + Duration::from_millis(100), true);
        // Detached: records keep arriving and are applied to the state.
        append(
            &path,
            &record("s1", "/work/b", "UserPromptSubmit", "working", &[99, 20]),
        );
        append(&path, &record("s1", "/work/c", "CwdChanged", "", &[99, 20]));
        assert!(tab
            .tick(start + Duration::from_millis(200), false)
            .is_empty());
        tab.reset_attachment();
        let fresh = state(&mut tab);
        assert_eq!(fresh.revision, 1, "a fresh epoch starts over");
        assert_eq!(cwd(&fresh, 0), "/work/c");
        assert_eq!(fresh.sessions[0].activity, ClaudeActivity::Busy);
        // Nothing of the outage is pushed again as individual records.
        assert!(tab
            .tick(start + Duration::from_millis(300), true)
            .is_empty());
    }

    fn listing_claude(dir: &Path, output: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("claude");
        std::fs::write(&program, format!("#!/bin/sh\nprintf '%s' '{output}'\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    /// Tick until `done` holds for the pushed operations, for up to 4 s.
    fn tick_until(
        tab: &mut ClaudeTracker,
        mut done: impl FnMut(&[Operation]) -> bool,
    ) -> Vec<Operation> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            seen.extend(tab.tick(Instant::now(), true));
            if done(&seen) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        seen
    }

    #[test]
    fn a_triggered_listing_follows_cd_and_relays_background_entries() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        made_platform::ensure_private_directory(&dir.join("data/remote")).unwrap();
        let program = listing_claude(
            &dir,
            r#"[{"pid":20,"cwd":"/work/after-cd","kind":"interactive","sessionId":"MARKER-TRANSCRIPT-c1"},{"pid":8,"cwd":"/work/spec","kind":"background","name":"\u2733 Spec task"}]"#,
        );
        let table = Table::with(&[(10, 1), (20, 10)]);
        let path = spool(&dir);
        let worker =
            made_remote::claude::ListingWorker::new(Some(program), dir.join("data/remote/claude"));
        let mut tab = ClaudeTracker::with_parent(
            Some(10),
            false,
            Some(path.clone()),
            Some(worker),
            table.parent_of(),
        );
        tab.tick(Instant::now(), true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        std::thread::sleep(Duration::from_millis(60));
        tab.tick(Instant::now(), true);
        std::thread::sleep(Duration::from_millis(100));
        tab.trigger();
        let seen = tick_until(&mut tab, |seen| {
            seen.iter()
                .any(|op| matches!(op, Operation::ClaudeListing { .. }))
                && snapshot(seen).is_some_and(|s| cwd(&s, 0) == "/work/after-cd")
        });
        let latest = snapshot(&seen).expect("a pushed snapshot");
        assert_eq!(cwd(&latest, 0), "/work/after-cd");
        let background = seen
            .iter()
            .find_map(|op| match op {
                Operation::ClaudeListing { listing } => Some(listing.background.clone()),
                _ => None,
            })
            .expect("a relayed listing");
        assert_eq!(background.len(), 1);
        assert_eq!(background[0].name, "Spec task");
        assert_eq!(background[0].cwd.as_bytes(), b"/work/spec");
    }

    /// A state reply is always a new snapshot revision, and listings count
    /// their own sequence, so the client never compares the two.
    #[test]
    fn state_replies_take_new_revisions_and_listings_count_their_own() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        made_platform::ensure_private_directory(&dir.join("data/remote")).unwrap();
        let program = listing_claude(
            &dir,
            r#"[{"pid":8,"cwd":"/work/spec","kind":"background","name":"Spec task"}]"#,
        );
        let table = Table::with(&[(10, 1), (20, 10)]);
        let path = spool(&dir);
        let worker =
            made_remote::claude::ListingWorker::new(Some(program), dir.join("data/remote/claude"));
        let mut tab = ClaudeTracker::with_parent(
            Some(10),
            false,
            Some(path.clone()),
            Some(worker),
            table.parent_of(),
        );
        tab.tick(Instant::now(), true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        std::thread::sleep(Duration::from_millis(60));
        tab.tick(Instant::now(), true);
        tab.trigger();
        let seen = tick_until(&mut tab, |seen| {
            seen.iter()
                .any(|op| matches!(op, Operation::ClaudeListing { .. }))
        });
        let listed = seen
            .iter()
            .find_map(|op| match op {
                Operation::ClaudeListing { listing } => Some(listing.revision),
                _ => None,
            })
            .expect("a relayed listing");
        assert_eq!(listed, 1, "the first listing starts its own sequence");
        let reply = state(&mut tab);
        // Each reply is its own item in the snapshot sequence.
        assert!(state(&mut tab).revision > reply.revision);
    }

    #[test]
    fn a_listing_without_the_sessions_pid_changes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        made_platform::ensure_private_directory(&dir.join("data/remote")).unwrap();
        let program = listing_claude(
            &dir,
            r#"[{"pid":999,"cwd":"/work/other","kind":"interactive"}]"#,
        );
        let table = Table::with(&[(10, 1), (20, 10)]);
        let path = spool(&dir);
        let worker =
            made_remote::claude::ListingWorker::new(Some(program), dir.join("data/remote/claude"));
        let mut tab = ClaudeTracker::with_parent(
            Some(10),
            false,
            Some(path.clone()),
            Some(worker),
            table.parent_of(),
        );
        tab.tick(Instant::now(), true);
        append(
            &path,
            &record("s1", "/work/a", "SessionStart", "idle", &[99, 20]),
        );
        std::thread::sleep(Duration::from_millis(60));
        tab.tick(Instant::now(), true);
        let before = state(&mut tab);
        tab.trigger();
        let seen = tick_until(&mut tab, |seen| {
            seen.iter()
                .any(|op| matches!(op, Operation::ClaudeListing { .. }))
        });
        assert!(seen
            .iter()
            .any(|op| matches!(op, Operation::ClaudeListing { .. })));
        assert!(snapshot(&seen).is_none(), "the session did not change");
        assert_eq!(state(&mut tab).sessions, before.sessions);
    }

    #[test]
    fn the_tracker_opens_nothing_under_the_remote_claude_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let claude_home = home.join(".claude");
        std::fs::create_dir_all(claude_home.join("sessions")).unwrap();
        std::fs::write(claude_home.join("settings.json"), "{}").unwrap();
        let runtime = home.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let table = Table::with(&[(10, 1), (20, 10)]);
        audit_file_access(true);
        let (mut tab, path) = shell_tab(&runtime, &table);
        let start = Instant::now();
        tab.tick(start, true);
        append(
            &path,
            &record(
                "s1",
                &claude_home.display().to_string(),
                "SessionStart",
                "idle",
                &[99, 20],
            ),
        );
        for step in 1..5 {
            tab.tick(start + Duration::from_millis(300 * step), true);
        }
        tab.state();
        let opened = audit_file_access(false);
        assert!(
            opened.contains(&path),
            "the audit records the spool: {opened:?}"
        );
        assert!(
            !opened.iter().any(|path| path.starts_with(&claude_home)),
            "{opened:?}"
        );
    }
}

// ---- per-server-user listing worker -------------------------------------------

#[cfg(unix)]
mod listing {
    use made_remote::claude::{audit_file_access, ListingWorker};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// A fixture `claude` that logs each run as `<ms> <argv> <stdin> <pgid==pid>`,
    /// flags a run that overlaps another, then runs `tail`.
    fn fixture(dir: &Path, tail: &str) -> PathBuf {
        let program = dir.join("claude");
        let log = dir.join("runs.log");
        let busy = dir.join("busy");
        let overlap = dir.join("overlap");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\n\
                 if ! mkdir '{busy}' 2>/dev/null; then : > '{overlap}'; fi\n\
                 if read -r line; then stdin=open; else stdin=closed; fi\n\
                 pgid=$(ps -o pgid= -p $$ | tr -d ' ')\n\
                 group=other; [ \"$pgid\" = \"$$\" ] && group=leader\n\
                 printf '%s %s %s\\n' \"$*\" \"$stdin\" \"$group\" >> '{log}'\n\
                 sleep 0.2\n\
                 rmdir '{busy}'\n\
                 {tail}\n",
                busy = busy.display(),
                overlap = overlap.display(),
                log = log.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    fn runs(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("runs.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn coordination(dir: &Path) -> PathBuf {
        let path = dir.join("data/remote/claude");
        std::fs::create_dir_all(dir.join("data/remote")).unwrap();
        path
    }

    fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        done()
    }

    const OUTPUT: &str = r#"printf '[{"pid":7,"cwd":"/work/a","kind":"interactive","prompt":"MARKER"},{"pid":8,"cwd":"/work/spec","kind":"background","name":"Spec task"}]'"#;

    #[test]
    fn a_trigger_runs_the_detected_claude_directly_in_a_burst() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let worker = ListingWorker::new(Some(fixture(&dir, OUTPUT)), coordination(&dir));
        std::thread::sleep(Duration::from_millis(400));
        assert!(runs(&dir).is_empty(), "nothing runs without a trigger");
        let triggered = Instant::now();
        assert!(worker.trigger().is_some());
        assert!(wait_until(Duration::from_secs(5), || runs(&dir).len() >= 3));
        let elapsed = triggered.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1900),
            "the burst reaches about 2 s: {elapsed:?}"
        );
        for run in runs(&dir) {
            assert_eq!(run, "agents --json closed leader", "argv, stdin and group");
        }
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(runs(&dir).len(), 3, "one burst is three runs, then nothing");
        let listing = worker.latest().expect("a fresh listing");
        assert_eq!(listing.entries.len(), 2);
        assert!(listing.entries[1].background);
        assert_eq!(listing.entries[1].name, "Spec task");
    }

    #[test]
    fn triggers_never_run_in_parallel_across_tabs_and_keep_the_spacing() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let program = fixture(&dir, OUTPUT);
        let first = ListingWorker::new(Some(program.clone()), coordination(&dir));
        let second = ListingWorker::new(Some(program), coordination(&dir));
        let triggered = Instant::now();
        for _ in 0..3 {
            first.trigger();
            second.trigger();
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(3500));
        assert!(!dir.join("overlap").exists(), "two runs overlapped");
        let count = runs(&dir).len();
        assert!((1..=6).contains(&count), "{count} runs");
        // Both tabs read the one published listing.
        assert_eq!(
            first.latest().unwrap().started_ms,
            second.latest().unwrap().started_ms
        );
        assert!(triggered.elapsed() >= Duration::from_secs(3));
    }

    /// A fixture `claude` that logs `start <pid>`, runs for about a second
    /// and then logs `end <pid>` and prints an empty listing.
    fn slow_fixture(dir: &Path) -> PathBuf {
        let program = dir.join("claude");
        let log = dir.join("lifecycle.log");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh
                 printf 'start %s\\n' \"$$\" >> '{log}'
                 sleep 1
                 printf 'end %s\\n' \"$$\" >> '{log}'
                 printf '[]'
",
                log = log.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    fn lifecycle(dir: &Path) -> Vec<(String, String)> {
        std::fs::read_to_string(dir.join("lifecycle.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(what, pid)| (what.to_owned(), pid.to_owned()))
            .collect()
    }

    fn first_started(dir: &Path) -> Option<String> {
        lifecycle(dir)
            .into_iter()
            .find(|(what, _)| what == "start")
            .map(|(_, pid)| pid)
    }

    fn ended(dir: &Path, pid: &str) -> bool {
        lifecycle(dir)
            .iter()
            .any(|(what, ended)| what == "end" && ended == pid)
    }

    #[test]
    fn a_run_outlives_its_tab_only_while_another_tab_waits_for_it() {
        // Alone: the tab's teardown kills its run.
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let alone = ListingWorker::new(Some(slow_fixture(&dir)), coordination(&dir));
        alone.trigger();
        assert!(wait_until(Duration::from_secs(3), || first_started(&dir).is_some()));
        let pid = first_started(&dir).unwrap();
        let dropped = Instant::now();
        drop(alone);
        assert!(dropped.elapsed() < Duration::from_millis(900));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!ended(&dir, &pid), "a run no tab waits for is killed");

        // Another tab of the same server user waits for the run: it is not
        // killed with the tab that started it, and it publishes for both.
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let program = slow_fixture(&dir);
        let first = ListingWorker::new(Some(program.clone()), coordination(&dir));
        let second = ListingWorker::new(Some(program), coordination(&dir));
        first.trigger();
        assert!(wait_until(Duration::from_secs(3), || first_started(&dir).is_some()));
        let pid = first_started(&dir).unwrap();
        second.trigger();
        std::thread::sleep(Duration::from_millis(150));
        drop(first);
        assert!(
            wait_until(Duration::from_secs(3), || ended(&dir, &pid)),
            "the run another tab waits for completes"
        );
        assert!(wait_until(Duration::from_secs(2), || second
            .latest()
            .is_some()));
    }

    #[test]
    fn the_published_listing_goes_with_the_last_supervisor_of_the_server_user() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let program = fixture(&dir, OUTPUT);
        let listing = coordination(&dir).join("listing.json");
        let first = ListingWorker::new(Some(program.clone()), coordination(&dir));
        let second = ListingWorker::new(Some(program.clone()), coordination(&dir));
        first.trigger();
        assert!(wait_until(Duration::from_secs(3), || listing.exists()));
        // Another supervisor still runs: the listing stays.
        drop(first);
        assert!(listing.exists());
        assert!(second.latest().is_some());
        // The last one removes it.
        drop(second);
        assert!(!listing.exists(), "the last supervisor removes the listing");

        // A listing left by supervisors that are gone is swept at helper
        // start; one a live supervisor may still use is kept.
        std::fs::write(&listing, b"{}").unwrap();
        let started = ListingWorker::new(Some(program.clone()), coordination(&dir));
        assert!(
            !listing.exists(),
            "a stale listing is swept at helper start"
        );
        std::fs::write(&listing, b"{}").unwrap();
        let another = ListingWorker::new(Some(program), coordination(&dir));
        assert!(listing.exists(), "a live supervisor's listing stays");
        drop(another);
        assert!(listing.exists());
        drop(started);
        assert!(!listing.exists());
    }

    #[test]
    fn a_helper_start_without_claude_still_sweeps_a_stale_listing() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        made_platform::ensure_private_directory(&coordination(&dir)).unwrap();
        let listing = coordination(&dir).join("listing.json");
        std::fs::write(&listing, b"{}").unwrap();
        let undetected = ListingWorker::new(None, coordination(&dir));
        assert!(!listing.exists(), "swept although claude was not detected");
        // A listing a live supervisor may still use is kept.
        let live = ListingWorker::new(Some(fixture(&dir, OUTPUT)), coordination(&dir));
        std::fs::write(&listing, b"{}").unwrap();
        let another = ListingWorker::new(None, coordination(&dir));
        assert!(listing.exists());
        drop((undetected, another, live));
    }

    #[test]
    fn nothing_runs_when_claude_was_not_detected() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let worker = ListingWorker::new(None, coordination(&dir));
        assert_eq!(worker.trigger(), None);
        std::thread::sleep(Duration::from_millis(400));
        assert!(worker.latest().is_none());
        assert!(!coordination(&dir).join("listing.json").exists());
    }

    #[test]
    fn every_failure_leaves_the_published_listing_unchanged() {
        for (label, tail) in [
            ("exit", "exit 3"),
            ("malformed", "printf 'not json'"),
            ("timeout", "sleep 30"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().canonicalize().unwrap();
            let worker = ListingWorker::new(Some(fixture(&dir, tail)), coordination(&dir));
            let triggered = Instant::now();
            worker.trigger();
            assert!(
                wait_until(Duration::from_secs(4), || !runs(&dir).is_empty()),
                "{label}"
            );
            std::thread::sleep(Duration::from_millis(2600));
            assert!(worker.latest().is_none(), "{label}");
            if label == "timeout" {
                // The first run was killed after about two seconds, so the
                // next one of the burst has started by now.
                assert!(runs(&dir).len() >= 2, "{:?}", triggered.elapsed());
                // Dropping the worker kills the in-flight run's whole group.
                drop(worker);
                let gone = wait_until(Duration::from_secs(2), || {
                    std::process::Command::new("pgrep")
                        .args(["-f", &dir.join("claude").display().to_string()])
                        .output()
                        .unwrap()
                        .stdout
                        .is_empty()
                });
                assert!(gone, "the timed-out runs were killed");
            }
        }
    }

    #[test]
    fn the_worker_opens_nothing_under_the_remote_claude_home() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let claude_home = dir.join(".claude");
        std::fs::create_dir_all(claude_home.join("jobs")).unwrap();
        audit_file_access(true);
        let worker = ListingWorker::new(Some(fixture(&dir, OUTPUT)), coordination(&dir));
        worker.trigger();
        assert!(wait_until(Duration::from_secs(3), || worker
            .latest()
            .is_some()));
        let opened = audit_file_access(false);
        assert!(
            opened
                .iter()
                .any(|path| path.starts_with(coordination(&dir))),
            "{opened:?}"
        );
        assert!(
            !opened.iter().any(|path| path.starts_with(&claude_home)),
            "{opened:?}"
        );
    }
}
