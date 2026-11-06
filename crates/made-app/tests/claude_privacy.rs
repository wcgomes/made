//! Claude Code privacy tests (spec `No prompt or message content anywhere`,
//! `Window titles and screen are not used`, `Permission message never shown`
//! and `No network or Claude Code configuration access`).
//!
//! The hook side (payload content never reaches a written record) is proven by
//! `made-claude-hook/tests/status.rs::payload_content_never_reaches_the_record`
//! against the real hook binary. This file proves the Made side: unique markers
//! that a spool record carries in fields Made must not read never reach any
//! state, log, tooltip input or report; window-title text never drives status,
//! identity or directory; and Made opens no listener or connection and touches
//! no Claude Code transcript, session, IDE lockfile or configuration path.

use made_app::claude_follow::{parse_record, ClaudeActivity, ClaudeTab};

const MARKERS: [&str; 5] = [
    "MARKER-PROMPT-7f3a",
    "MARKER-TOOL-INPUT-7f3a",
    "MARKER-MESSAGE-7f3a",
    "MARKER-MSG-7f3a",
    "MARKER-TRANSCRIPT-7f3a",
];

/// A record line as an untrusted writer could produce it: the metadata fields
/// plus content fields Made must never read.
fn record_with_markers(cwd: &str, event: &str, status: &str, pids: &[u32]) -> String {
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    format!(
        "{{\"at\":\"t\",\"sessionId\":\"s1\",\"cwd\":{cwd:?},\"event\":{event:?},\"notification\":\"\",\"status\":{status:?},\"title\":\"\",\"pids\":[{}],\"prompt\":\"{}\",\"tool_input\":{{\"command\":\"{}\"}},\"message\":\"{}\",\"msg\":\"{}\",\"transcript_path\":\"/t/{}.jsonl\"}}",
        pids.join(","),
        MARKERS[0],
        MARKERS[1],
        MARKERS[2],
        MARKERS[3],
        MARKERS[4],
    )
}

/// Claude Code 20 runs under the tab's shell 10; the hook shell 99 has exited.
fn tree(pid: u32) -> Option<u32> {
    match pid {
        20 => Some(10),
        10 => Some(1),
        _ => None,
    }
}

fn assert_no_markers(text: &str, what: &str) {
    for marker in MARKERS {
        assert!(
            !text.contains(marker),
            "{marker} leaked into {what}: {text}"
        );
    }
}

#[test]
fn content_fields_never_reach_a_parsed_record_or_the_tab_state() {
    let mut tab = ClaudeTab::new(10, false);
    let records: Vec<_> = [
        ("UserPromptSubmit", "working"),
        ("Notification", "attention"),
        ("Stop", "idle"),
    ]
    .into_iter()
    .map(|(event, status)| {
        parse_record(&record_with_markers("/work", event, status, &[99, 20, 10]))
            .expect("a well-formed record")
    })
    .collect();
    for record in &records {
        assert_no_markers(&format!("{record:?}"), "a parsed record");
    }
    tab.ingest(records, tree);
    assert_eq!(tab.session_count(), 1, "the records were owned and applied");
    assert_no_markers(&format!("{tab:?}"), "the tab state");
    assert_no_markers(&tab.tooltip_title(None), "the tooltip title");
    assert_no_markers(&format!("{:?}", tab.dots()), "the dots");
    assert_no_markers(&format!("{:?}", tab.directory()), "the directory");
}

#[test]
fn window_title_text_never_drives_status_identity_or_directory() {
    let mut tab = ClaudeTab::new(10, false);
    let title = "TITLE-MARKER-7f3a /tmp/elsewhere \u{2733} Working \u{280b} idle";

    // Without any status the title is not read: not the tooltip, no dot, no
    // directory, no presence.
    assert_eq!(tab.tooltip_title(Some(title)), "Claude Code");
    assert!(tab.dots().is_empty());
    assert_eq!(tab.directory(), None);
    assert!(!tab.present());
    assert_eq!(tab.session_count(), 0);

    // A hook session supplies status, identity and directory; a different
    // window title changes none of them and is not shown without a live
    // protocol status.
    let record = parse_record(&record_with_markers(
        "/work",
        "SessionStart",
        "idle",
        &[99, 20, 10],
    ))
    .expect("record");
    tab.ingest(vec![record], tree);
    let before = (tab.dots(), tab.directory(), tab.session_count());
    assert_eq!(before.0, vec![ClaudeActivity::Idle]);
    assert_eq!(before.1.as_deref(), Some("/work"));
    for window_title in [title, "Working", "\u{2733} cd /etc"] {
        let shown = tab.tooltip_title(Some(window_title));
        assert!(
            !shown.contains("TITLE-MARKER") && shown == "Claude Code",
            "no title text without a live protocol status: {shown:?}"
        );
        assert_eq!(
            (tab.dots(), tab.directory(), tab.session_count()),
            before,
            "{window_title:?} must not change status, identity or directory"
        );
    }
}

#[test]
fn made_claude_code_sources_open_no_listener_connection_or_claude_path() {
    let sources = [
        ("claude_follow.rs", include_str!("../src/claude_follow.rs")),
        (
            "made-claude-hook/src/lib.rs",
            include_str!("../../made-claude-hook/src/lib.rs"),
        ),
        (
            "made-claude-hook/src/main.rs",
            include_str!("../../made-claude-hook/src/main.rs"),
        ),
        (
            "made-remote/src/claude.rs",
            include_str!("../../made-remote/src/claude.rs"),
        ),
    ];
    for (name, source) in sources {
        let lower = source.to_ascii_lowercase();
        for forbidden in [
            "tcplistener",
            "tcpstream",
            "udpsocket",
            "unixlistener",
            "unixstream",
            "std::net",
            "reqwest",
            "ureq",
            "websocket",
            "\".claude\"",
            ".claude/",
            ".claude.json",
            "settings.json",
            "lockfile",
            "ide_lock",
            "std::fs::read_dir(&home",
        ] {
            assert!(
                !lower.contains(forbidden),
                "{name} references forbidden surface {forbidden:?}"
            );
        }
    }
    // Transcripts are named only to say they are ignored.
    for (name, source) in sources {
        for line in source.lines() {
            let lower = line.to_ascii_lowercase();
            if lower.contains("transcript") {
                let trimmed = line.trim_start();
                assert!(
                    trimmed.starts_with("//") || trimmed.starts_with("\"transcript"),
                    "{name} reads a transcript: {line}"
                );
            }
        }
    }

    // The shipped plugin registers command hooks only: no HTTP hook, no URL.
    let hooks = include_str!("../../../plugins/claude-code-made/hooks/hooks.json");
    let value: serde_json::Value = serde_json::from_str(hooks).expect("hooks.json parses");
    let mut count = 0;
    for (_, groups) in value["hooks"].as_object().expect("hooks object") {
        for group in groups.as_array().expect("groups") {
            for hook in group["hooks"].as_array().expect("hooks") {
                assert_eq!(hook["type"], "command", "{hook}");
                assert!(hook.get("url").is_none(), "{hook}");
                count += 1;
            }
        }
    }
    assert!(count > 0);
    assert!(!hooks.contains("http://") && !hooks.contains("https://"));
}

#[cfg(unix)]
mod running_tab {
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use made_app::workspace::{LaunchPlan, WorkspaceSource};
    use made_app::{CompatibleTui, TerminalTarget, Workbench};
    use made_platform::{Platform, PlatformFamily};

    use super::{assert_no_markers, record_with_markers};

    const CHILD: &str = "MADE_CLAUDE_PRIVACY_CHILD";
    const TRIGGER: &str = "MADE_CLAUDE_PRIVACY_TRIGGER";
    const TITLE: &str = "TITLE-MARKER-9c1d";

    /// Re-run `name` in a child with an isolated home, a fake `claude` that
    /// prints a window title and then a protocol status once the trigger file
    /// exists, and the shipped plugin staged beside the test executable.
    fn isolated(name: &str) -> bool {
        if std::env::var_os(CHILD).is_some() {
            return false;
        }
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let claude = bin.join("claude");
        fs::write(
            &claude,
            format!(
                "#!/bin/sh\nprintf '\\033]0;{TITLE} /tmp\\007'\n\
                 while [ ! -f \"${TRIGGER}\" ]; do sleep 0.05; done\n\
                 printf '\\033]7501;state=working:app=claude-code\\007'\nsleep 30\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        for directory in ["tmp", "runtime", "AppData/Roaming", "AppData/Local"] {
            fs::create_dir_all(home.path().join(directory)).unwrap();
        }
        fs::set_permissions(
            home.path().join("runtime"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let shipped =
            made_platform::install_root_from_current_executable(PlatformFamily::current())
                .expect("test executable install root")
                .join("plugins")
                .join("claude-code-made");
        fs::create_dir_all(shipped.join("bin")).unwrap();
        let hook = shipped.join("bin").join("made-claude-status");
        fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, "1")
            .env(TRIGGER, home.path().join("trigger"))
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("TMPDIR", home.path().join("tmp"))
            .env("XDG_RUNTIME_DIR", home.path().join("runtime"))
            .env("APPDATA", home.path().join("AppData/Roaming"))
            .env("LOCALAPPDATA", home.path().join("AppData/Local"))
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("SHELL", "/bin/bash")
            .env_remove(made_bridge::config::CONFIG_CONTENT_ENV)
            .env_remove(made_platform::SHELL_ENV)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

    /// `O_NONBLOCK` for the supported Unix hosts.
    const O_NONBLOCK: i32 = if cfg!(target_os = "linux") {
        0o4000
    } else {
        0x4
    };

    /// Plant a FIFO at `path`. Opening it for reading blocks until a writer
    /// appears, which a writer with `O_NONBLOCK` can detect: that open fails
    /// with `ENXIO` unless some process is already trying to read the path.
    fn plant_fifo(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("mkfifo is available");
        assert!(status.success());
    }

    fn someone_is_reading(path: &Path) -> bool {
        fs::OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
            .is_ok()
    }

    fn regular_files(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                regular_files(&path, out);
            } else if kind.is_file() {
                out.push(path);
            }
        }
    }

    #[test]
    fn a_running_tab_never_surfaces_content_titles_or_reads_claude_paths() {
        if isolated(
            "running_tab::a_running_tab_never_surfaces_content_titles_or_reads_claude_paths",
        ) {
            return;
        }
        use std::io::Write;

        let home = made_platform::host_platform()
            .home_dir()
            .expect("isolated home");
        let fifos: Vec<PathBuf> = [
            ".claude.json",
            ".claude/settings.json",
            ".claude/projects/p/session.jsonl",
            ".claude/ide/1234.lock",
            ".claude/sessions/s1.json",
        ]
        .iter()
        .map(|relative| home.join(relative))
        .collect();
        for fifo in &fifos {
            plant_fifo(fifo);
        }
        // Positive control: the probe does detect a process reading a FIFO.
        let control = home.join(".claude/control.fifo");
        plant_fifo(&control);
        let reader = {
            let control = control.clone();
            std::thread::spawn(move || fs::File::open(control).map(drop))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !someone_is_reading(&control) {
            assert!(
                Instant::now() < deadline,
                "the read probe never saw a reader"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        reader.join().unwrap().unwrap();

        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
        let mut workbench = Workbench::start(
            Some(plan),
            None,
            root.clone(),
            TerminalTarget::Tui(CompatibleTui::ClaudeCode),
            Instant::now(),
        );
        let pane_pid = workbench
            .terminal()
            .and_then(made_app::TerminalPane::pid)
            .expect("claude child");
        let spool = PathBuf::from(
            workbench
                .terminal()
                .unwrap()
                .options()
                .env
                .iter()
                .find(|(key, _)| key == "MADE_CLAUDE_SPOOL")
                .expect("spool binding")
                .1
                .clone(),
        );
        let ctx = eframe::egui::Context::default();
        let tick = |workbench: &mut Workbench, frames: u32| {
            for _ in 0..frames {
                workbench.tick(&ctx);
                std::thread::sleep(Duration::from_millis(20));
            }
        };

        // Phase 1: the fake claude printed a window title and no status. The
        // title drives nothing and is not the tooltip.
        let deadline = Instant::now() + Duration::from_secs(5);
        while workbench.terminal().and_then(|terminal| terminal.title())
            != Some(&format!("{TITLE} /tmp"))
            && Instant::now() < deadline
        {
            tick(&mut workbench, 1);
        }
        let window_title = workbench
            .terminal()
            .and_then(|terminal| terminal.title())
            .map(str::to_owned);
        assert!(
            window_title.as_deref().is_some_and(|t| t.contains(TITLE)),
            "the fixture title reached the terminal: {window_title:?}"
        );
        let tab = workbench.claude_tab().expect("direct tab");
        assert!(tab.dots().is_empty(), "a title is not a status");
        assert_eq!(tab.directory(), None, "a title is not a directory");
        assert_eq!(
            tab.tooltip_title(window_title.as_deref()),
            "Claude Code",
            "no title text without a live protocol status"
        );
        assert_eq!(workbench.explorer_root(), Some(root.as_path()));

        // Phase 2: an owned hook record carrying content fields, then the
        // protocol status goes live.
        {
            let mut file = fs::OpenOptions::new().append(true).open(&spool).unwrap();
            for (event, status) in [
                ("SessionStart", "idle"),
                ("UserPromptSubmit", "working"),
                ("Notification", "attention"),
            ] {
                writeln!(
                    file,
                    "{}",
                    record_with_markers(
                        &elsewhere.display().to_string(),
                        event,
                        status,
                        &[4_000_000, pane_pid]
                    )
                )
                .unwrap();
            }
        }
        fs::write(std::env::var_os(TRIGGER).unwrap(), b"go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !workbench
            .claude_tab()
            .is_some_and(|tab| tab.protocol_live())
            && Instant::now() < deadline
        {
            tick(&mut workbench, 1);
        }
        tick(&mut workbench, 20);
        let tab = workbench.claude_tab().expect("direct tab");
        assert!(tab.protocol_live(), "the protocol status went live");
        let window_title = workbench
            .terminal()
            .and_then(|terminal| terminal.title())
            .map(str::to_owned);
        let shown = tab.tooltip_title(window_title.as_deref());
        assert!(
            shown.contains(TITLE),
            "the sanitized window title is the tooltip while the protocol is live: {shown:?}"
        );

        // Content markers appear on no surface Made renders or records.
        assert_no_markers(&format!("{:?}", workbench.claude_tab()), "the tab state");
        assert_no_markers(&shown, "the tooltip");
        assert_no_markers(&workbench.sanitized_diagnostics_text(), "the copied report");
        let log: String = workbench
            .diagnostics()
            .log()
            .entries()
            .map(|entry| entry.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert_no_markers(&log, "the workspace log");
        let mut files = Vec::new();
        regular_files(&home, &mut files);
        for file in files {
            if file.starts_with(spool.parent().unwrap()) || file.starts_with(home.join("bin")) {
                continue; // the spool is this test's own input
            }
            if let Ok(text) = fs::read_to_string(&file) {
                assert_no_markers(&text, &file.display().to_string());
            }
        }

        // Made never tried to read any Claude Code configuration, session,
        // transcript or IDE lockfile path.
        for fifo in &fifos {
            assert!(
                !someone_is_reading(fifo),
                "Made tried to read {}",
                fifo.display()
            );
        }
        workbench.shutdown();
    }
}
