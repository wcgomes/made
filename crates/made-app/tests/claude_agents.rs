//! `claude agents --json` runs only on input or title change: a running tab
//! with a fake `claude` that counts its `agents` runs and lists whatever the
//! test writes. No real `claude` is ever run.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_app::integrations::claude_code;
use made_app::integrations::CLAUDE_CODE;
use made_app::workspace::{LaunchPlan, WorkspaceSource};
use made_app::{DiagnosticLog, TerminalTarget, Workbench};
use made_platform::{Platform, PlatformFamily};

const CHILD: &str = "MADE_CLAUDE_AGENTS_CHILD";

/// Re-run `name` in a child with an isolated home and a fake `claude` on PATH.
/// For `claude agents` it appends a line to `$HOME/agents-runs` and prints
/// `$HOME/listing`; otherwise it reports a live protocol status, then prints
/// the contents of `$HOME/title` as the window title whenever that file
/// appears. Every inherited `CLAUDE*` variable is removed.
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
        "#!/bin/sh\n\
         if [ \"$1\" = agents ]; then\n\
         echo run >> \"$HOME/agents-runs\"\n\
         cat \"$HOME/listing\" 2>/dev/null || printf '[]'\n\
         exit 0\n\
         fi\n\
         printf '\\033]7501;state=working:app=claude-code\\007'\n\
         while :; do\n\
         if [ -f \"$HOME/title\" ]; then\n\
         printf '\\033]0;%s\\007' \"$(cat \"$HOME/title\")\"\n\
         rm -f \"$HOME/title\"\n\
         fi\n\
         sleep 0.05\n\
         done\n",
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    for directory in ["tmp", "runtime"] {
        fs::create_dir_all(home.path().join(directory)).unwrap();
    }
    fs::set_permissions(
        home.path().join("runtime"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let shipped = made_platform::install_root_from_current_executable(PlatformFamily::current())
        .expect("test executable install root")
        .join("plugins")
        .join("claude-code-made");
    fs::create_dir_all(shipped.join("bin")).unwrap();
    let hook = shipped.join("bin").join("made-claude-status");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CLAUDE") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, "1")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", home.path().join("tmp"))
        .env("XDG_RUNTIME_DIR", home.path().join("runtime"))
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

/// Atomically replace `path` with `text`, so the fake never reads half of it.
fn write_atomic(path: &Path, text: &str) {
    let staged = path.with_extension("tmp");
    fs::write(&staged, text).unwrap();
    fs::rename(&staged, path).unwrap();
}

fn runs(home: &Path) -> usize {
    fs::read_to_string(home.join("agents-runs"))
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

struct Harness {
    workbench: Workbench,
    ctx: eframe::egui::Context,
}

impl Harness {
    fn tick_for(&mut self, duration: Duration) {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            self.workbench.tick(&self.ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Tick until `done` holds or `limit` passes; returns the time taken.
    fn tick_until(&mut self, limit: Duration, done: impl Fn(&Workbench) -> bool) -> Duration {
        let started = Instant::now();
        while !done(&self.workbench) && started.elapsed() < limit {
            self.workbench.tick(&self.ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(done(&self.workbench), "not within {limit:?}");
        started.elapsed()
    }

    fn directory(&self) -> Option<String> {
        claude_code::tab(&self.workbench).and_then(|tab| tab.directory())
    }
}

fn agents_running(bin: &Path) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", &format!("{}/claude agents", bin.display())])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn agents_are_listed_only_after_input_or_a_title_change() {
    if isolated("agents_are_listed_only_after_input_or_a_title_change") {
        return;
    }
    let home = made_platform::host_platform()
        .home_dir()
        .expect("isolated home");
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let after_cd = root.join("after-cd");
    let spec = root.join("spec");
    fs::create_dir(&after_cd).unwrap();
    fs::create_dir(&spec).unwrap();
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let workbench = Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Tui(CLAUDE_CODE),
        Instant::now(),
    );
    let mut harness = Harness {
        workbench,
        ctx: eframe::egui::Context::default(),
    };
    let pane_pid = harness
        .workbench
        .terminal()
        .and_then(made_app::TerminalPane::pid)
        .expect("claude child");
    let spool = PathBuf::from(
        harness
            .workbench
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
    let listing = home.join("listing");
    let interactive = |cwd: &Path| {
        format!(
            r#"{{"pid":{pane_pid},"cwd":{:?},"kind":"interactive","name":"host"}}"#,
            cwd.display().to_string()
        )
    };
    write_atomic(&listing, &format!("[{}]", interactive(&root)));

    // A live owned session, known only from its hook records.
    let hook = |event: &str, cwd: &Path| {
        writeln!(
            fs::OpenOptions::new().append(true).open(&spool).unwrap(),
            r#"{{"sessionId":"s1","cwd":{:?},"event":{event:?},"status":"idle","title":"","pids":[4000000,{pane_pid}]}}"#,
            cwd.display().to_string()
        )
        .unwrap();
    };
    assert_eq!(runs(&home), 0, "nothing lists agents before a session");
    hook("SessionStart", &root);
    harness.tick_until(Duration::from_secs(5), |workbench| {
        claude_code::tab(workbench)
            .is_some_and(|tab| tab.session_count() > 0 && tab.protocol_live())
    });
    let root_text = root.display().to_string();
    assert_eq!(harness.directory(), Some(root_text));

    // The session start lists once in a burst; then, with no trigger, nothing.
    harness.tick_for(Duration::from_millis(2600));
    let after_start = runs(&home);
    assert!(
        (1..=3).contains(&after_start),
        "the session start runs one burst: {after_start}"
    );
    harness.tick_for(Duration::from_secs(5));
    assert_eq!(runs(&home), after_start, "no listing without a trigger");

    // `/cd`: the listing moves and no hook fires. Enter lists it in time.
    write_atomic(&listing, &format!("[{}]", interactive(&after_cd)));
    let enter = |harness: &mut Harness| {
        harness.workbench.terminal_mut().unwrap().send_key(
            made_terminal::Key::Enter,
            made_terminal::Modifiers::default(),
            &mut DiagnosticLog::new(),
        );
    };
    enter(&mut harness);
    let after_cd_text = after_cd.display().to_string();
    let took = harness.tick_until(Duration::from_secs(3), |workbench| {
        claude_code::tab(workbench).and_then(|tab| tab.directory()) == Some(after_cd_text.clone())
    });
    assert!(took < Duration::from_secs(1), "followed `/cd` in {took:?}");
    harness.tick_until(Duration::from_secs(3), |workbench| {
        workbench.explorer_root() == Some(after_cd.as_path())
    });

    // The burst ends, and an idle tab lists nothing more.
    harness.tick_for(Duration::from_millis(2600));
    let after_burst = runs(&home);
    assert!(
        (after_start + 1..=after_start + 3).contains(&after_burst),
        "one burst of at most three runs: {after_start} -> {after_burst}"
    );
    harness.tick_for(Duration::from_secs(6));
    assert_eq!(runs(&home), after_burst, "no listing while idle");
    assert_eq!(harness.directory(), Some(after_cd_text.clone()));

    // Long after that burst a hook record moves the session back to the
    // root, and nothing lists it. A `/cd` to the directory the last listing
    // named still follows on Enter.
    hook("UserPromptSubmit", &root);
    harness.tick_until(Duration::from_secs(3), |workbench| {
        workbench.explorer_root() == Some(root.as_path())
    });
    enter(&mut harness);
    let took = harness.tick_until(Duration::from_secs(3), |workbench| {
        workbench.explorer_root() == Some(after_cd.as_path())
    });
    assert!(
        took < Duration::from_secs(1),
        "followed `/cd` back in {took:?}"
    );
    harness.tick_for(Duration::from_millis(2600));
    let after_back = runs(&home);
    assert!(after_back > after_burst, "{after_burst} -> {after_back}");

    // An agent-view switch names a background session in the title.
    write_atomic(
        &listing,
        &format!(
            r#"[{},{{"pid":777777,"cwd":{:?},"kind":"background","name":"Spec task"}}]"#,
            interactive(&after_cd),
            spec.display().to_string()
        ),
    );
    write_atomic(&home.join("title"), "\u{2733} Spec task");
    let spec_text = spec.display().to_string();
    let took = harness.tick_until(Duration::from_secs(3), |workbench| {
        claude_code::tab(workbench).and_then(|tab| tab.directory()) == Some(spec_text.clone())
    });
    assert!(
        took < Duration::from_secs(1),
        "followed the title in {took:?}"
    );

    // The selection outlives its listing while the title stays, with no
    // further runs.
    harness.tick_for(Duration::from_millis(2600));
    let after_title = runs(&home);
    assert!(
        (after_back + 1..=after_back + 3).contains(&after_title),
        "{after_back} -> {after_title}"
    );
    harness.tick_for(Duration::from_secs(6));
    assert_eq!(runs(&home), after_title, "no listing while idle");
    assert_eq!(harness.directory(), Some(spec_text));

    // The title stops naming it: the host session's directory is back at once.
    write_atomic(&home.join("title"), "\u{2733} Claude Code");
    harness.tick_until(Duration::from_secs(3), |workbench| {
        claude_code::tab(workbench).and_then(|tab| tab.directory()) == Some(after_cd_text.clone())
    });
    harness.tick_for(Duration::from_millis(2600));
    assert!(runs(&home) > after_title, "a title change lists again");

    harness.workbench.shutdown();
    assert!(!agents_running(&home.join("bin")), "no run is left behind");
}
