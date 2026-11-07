#![cfg(unix)]
//! Live Claude Code acceptance suite (task 8.13).
//!
//! Every test here is `#[ignore]` and additionally gated by
//! `MADE_CLAUDE_LIVE=1`: it drives the real `claude` (2.1.295 or later) through
//! Made's real `Workbench` (environment composition, per-tab spool, status
//! protocol receiver, hook records), so it needs a logged-in Claude Code and a
//! directory Claude Code already trusts, given by `MADE_CLAUDE_LIVE_CWD`.
//!
//! ```text
//! MADE_CLAUDE_LIVE=1 MADE_CLAUDE_LIVE_CWD=/path/to/trusted/dir \
//!   cargo test -p made-app --test claude_live -- --ignored --test-threads=1
//! ```
//!
//! Sessions use no prompt except where a case needs one (`/clear`, a minimal
//! `cd` prompt, a deliberately failing model), never
//! `--dangerously-skip-permissions`, and run under the trusted directory. A
//! shim `claude` earlier on `PATH` logs every argv Made or the shell starts
//! before it execs the real binary, so "no subcommand ran" is observed.
//!
//! Each case prints one `LIVE-RESULT <case>: PASS` or
//! `LIVE-RESULT <case>: UNVERIFIED (<reason>)` line. A case whose evidence
//! cannot be produced (Claude Code did not fire the event, no login, a trust
//! dialog) is `UNVERIFIED`, never claimed as passing; a case whose evidence
//! contradicts the spec fails. Fixture-record tests for the same behavior live
//! in `integrations/claude_code/follow.rs` and in `made-claude-hook/tests/status.rs`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::integrations::claude_code;
use made_app::integrations::claude_code::follow::{shared_instance, TabSpool};
use made_app::integrations::CLAUDE_CODE;
use made_app::workspace::{LaunchPlan, WorkspaceSource};
use made_app::{TerminalPane, TerminalTarget, Workbench};
use made_platform::{host_platform, PlatformFamily};
use made_terminal::{PtyEvent, PtyOptions, PtyProcess};
use serde_json::Value;

const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";
const SESSION_WAIT: Duration = Duration::from_secs(45);

static SERIAL: Mutex<()> = Mutex::new(());

enum Verdict {
    Pass,
    Unverified(String),
}

fn finish(case: &str, verdict: Verdict) {
    match verdict {
        Verdict::Pass => eprintln!("LIVE-RESULT {case}: PASS"),
        Verdict::Unverified(reason) => eprintln!("LIVE-RESULT {case}: UNVERIFIED ({reason})"),
    }
}

fn unverified(reason: impl Into<String>) -> Verdict {
    Verdict::Unverified(reason.into())
}

/// Process-environment edits restored on drop. The suite is serial.
struct Env {
    saved: Vec<(OsString, Option<OsString>)>,
}

impl Env {
    fn set(&mut self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
        self.saved.push((key.into(), std::env::var_os(key)));
        std::env::set_var(key, value);
    }

    fn remove(&mut self, key: &str) {
        self.saved.push((key.into(), std::env::var_os(key)));
        std::env::remove_var(key);
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn gate_open() -> bool {
    if std::env::var_os("MADE_CLAUDE_LIVE").is_none_or(|value| value != "1") {
        eprintln!("claude_live: skipped (set MADE_CLAUDE_LIVE=1 to run against the real claude)");
        return false;
    }
    true
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn hook_binary() -> PathBuf {
    let deps = std::env::current_exe().unwrap();
    let profile = deps.parent().and_then(Path::parent).unwrap().to_path_buf();
    let hook = profile.join("made-claude-status");
    if !hook.is_file() {
        let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["build", "-p", "made-claude-hook", "--locked"])
            .status()
            .expect("cargo builds the hook");
        assert!(status.success() && hook.is_file(), "hook binary build");
    }
    hook
}

fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        other => panic!("unsupported live-suite host {other:?}"),
    }
}

/// Stage the shipped plugin where the app resolves it, with the real hook, or
/// remove it so the tab reports hooks unavailable.
fn stage_shipped_plugin(with_hook: bool) -> PathBuf {
    let shipped = made_platform::install_root_from_current_executable(PlatformFamily::current())
        .expect("install root")
        .join("plugins")
        .join("claude-code-made");
    let _ = fs::remove_dir_all(&shipped);
    if with_hook {
        let root = made_platform::repository_root();
        let status = Command::new("bash")
            .arg(root.join(".github/scripts/stage-claude-plugin.sh"))
            .arg(host_target())
            .arg(hook_binary())
            .arg(&shipped)
            .output()
            .expect("stage script runs");
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    shipped
}

/// Contents of the user Claude Code configuration Made must never touch:
/// settings, memory file, plugin registries and the plugin directory listing.
/// `~/.claude.json` is Claude Code's own runtime state and is not compared.
fn config_snapshot() -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let home = host_platform_home();
    let claude = home.join(".claude");
    let mut files = vec![
        claude.join("settings.json"),
        claude.join("settings.local.json"),
        claude.join("CLAUDE.md"),
        claude.join("plugins/known_marketplaces.json"),
        claude.join("plugins/installed_plugins.json"),
    ];
    let mut listing: Vec<PathBuf> = fs::read_dir(claude.join("plugins"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    listing.sort();
    let mut snapshot: Vec<(PathBuf, Option<Vec<u8>>)> = files
        .drain(..)
        .map(|path| {
            let bytes = fs::read(&path).ok();
            (path, bytes)
        })
        .collect();
    snapshot.extend(listing.into_iter().map(|path| (path, None)));
    snapshot
}

fn host_platform_home() -> PathBuf {
    use made_platform::Platform;
    host_platform().home_dir().expect("home directory")
}

struct Live {
    _serial: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    env: Env,
    cwd: PathBuf,
    shim: PathBuf,
    shim_log: PathBuf,
    shipped: PathBuf,
    ctx: egui::Context,
}

impl Live {
    /// `None` when the gate is closed.
    fn new(with_hook: bool) -> Option<Self> {
        if !gate_open() {
            return None;
        }
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let cwd = PathBuf::from(
            std::env::var_os("MADE_CLAUDE_LIVE_CWD")
                .expect("MADE_CLAUDE_LIVE_CWD must name a directory Claude Code already trusts"),
        )
        .canonicalize()
        .expect("MADE_CLAUDE_LIVE_CWD exists");
        let real = find_on_path("claude").expect("claude is on PATH");
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let shim = bin.join("claude");
        let shim_log = temp.path().join("shim.log");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\nprintf 'argc=%s' \"$#\" >> '{log}'\nfor a in \"$@\"; do printf ' [%s]' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\nexec '{real}' \"$@\"\n",
                log = shim_log.display(),
                real = real.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = temp.path().join("runtime");
        let tmp = temp.path().join("tmp");
        let state = temp.path().join("state");
        for dir in [&runtime, &tmp, &state] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();

        let mut env = Env { saved: Vec::new() };
        let inherited: Vec<String> = std::env::vars()
            .map(|(key, _)| key)
            .filter(|key| key.starts_with("CLAUDE") || key.starts_with("TMUX"))
            .collect();
        for key in inherited {
            env.remove(&key);
        }
        env.remove("MADE_CLAUDE_SPOOL");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
        env.set("PATH", std::env::join_paths(paths).unwrap());
        env.set("TMPDIR", &tmp);
        env.set("XDG_RUNTIME_DIR", &runtime);
        env.set("XDG_STATE_HOME", &state);
        env.set("SHELL", "/bin/bash");
        env.set("TERM", "xterm-256color");
        let shipped = stage_shipped_plugin(with_hook);
        Some(Self {
            _serial: serial,
            _temp: temp,
            env,
            cwd,
            shim,
            shim_log,
            shipped,
            ctx: egui::Context::default(),
        })
    }

    fn start(&self, target: TerminalTarget) -> Workbench {
        let plan = LaunchPlan::open(&self.cwd, WorkspaceSource::Picker).unwrap();
        Workbench::start(Some(plan), None, self.cwd.clone(), target, Instant::now())
    }

    fn direct() -> TerminalTarget {
        TerminalTarget::Tui(CLAUDE_CODE)
    }

    fn type_line(workbench: &mut Workbench, text: &str) {
        let mut log = made_app::DiagnosticLog::new();
        let terminal = workbench.terminal_mut().expect("terminal");
        for ch in text.chars() {
            terminal.send_key(
                made_terminal::Key::Char(ch),
                made_terminal::Modifiers::default(),
                &mut log,
            );
        }
        terminal.send_key(
            made_terminal::Key::Enter,
            made_terminal::Modifiers::default(),
            &mut log,
        );
    }

    /// Type a prompt, pause so Claude Code does not read the Enter as part of a
    /// paste, then submit it.
    fn submit(&self, workbench: &mut Workbench, text: &str) {
        let mut log = made_app::DiagnosticLog::new();
        let terminal = workbench.terminal_mut().expect("terminal");
        for ch in text.chars() {
            terminal.send_key(
                made_terminal::Key::Char(ch),
                made_terminal::Modifiers::default(),
                &mut log,
            );
        }
        self.pump_for(workbench, Duration::from_millis(1200));
        workbench.terminal_mut().expect("terminal").send_key(
            made_terminal::Key::Enter,
            made_terminal::Modifiers::default(),
            &mut log,
        );
    }

    /// Type the shim's absolute path (with arguments) into a shell tab.
    fn run_in_shell(&self, workbench: &mut Workbench, arguments: &str) {
        Self::type_line(workbench, &format!("'{}' {arguments}", self.shim.display()));
    }

    fn pump_until(
        &self,
        workbench: &mut Workbench,
        limit: Duration,
        mut done: impl FnMut(&Workbench) -> bool,
    ) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            workbench.tick(&self.ctx);
            if done(workbench) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn pump_for(&self, workbench: &mut Workbench, duration: Duration) {
        self.pump_until(workbench, duration, |_| false);
    }

    fn shim_invocations(&self) -> Vec<String> {
        fs::read_to_string(&self.shim_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn spool_path(workbench: &Workbench) -> PathBuf {
    PathBuf::from(
        workbench
            .terminal()
            .expect("terminal")
            .options()
            .env
            .iter()
            .find(|(key, _)| key == "MADE_CLAUDE_SPOOL")
            .expect("the tab has a spool binding")
            .1
            .clone(),
    )
}

fn records(workbench: &Workbench) -> Vec<Value> {
    read_records(&spool_path(workbench))
}

fn read_records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn events(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| record["event"].as_str().map(str::to_owned))
        .collect()
}

fn session_ready(workbench: &Workbench) -> bool {
    claude_code::tab(&workbench).is_some_and(|tab| tab.session_count() == 1)
}

fn snapshot_unchanged(before: &[(PathBuf, Option<Vec<u8>>)]) {
    let after = config_snapshot();
    assert_eq!(
        before.len(),
        after.len(),
        "the Claude Code plugin directory gained or lost an entry"
    );
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(
            before,
            after,
            "{} changed while Claude Code ran in Made",
            before.0.display()
        );
    }
}

// ---- cases ---------------------------------------------------------------------

/// Shell-typed and menu-launched sessions load the plugin once, and Made runs
/// no `claude` subcommand.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_shell_typed_and_menu_launched_sessions_load_the_plugin_once() {
    let case = "shell-and-menu-load-plugin-once";
    let Some(live) = Live::new(true) else { return };
    let before = config_snapshot();
    let mut verdict = Verdict::Pass;
    for (name, target) in [("shell", TerminalTarget::Shell), ("menu", Live::direct())] {
        let mut workbench = live.start(target);
        if name == "shell" {
            live.run_in_shell(&mut workbench, "");
        }
        let ready = live.pump_until(&mut workbench, SESSION_WAIT, |wb| {
            session_ready(wb) && claude_code::tab(&wb).is_some_and(|tab| tab.protocol_status_seen())
        });
        if !ready {
            verdict = unverified(format!(
                "{name}: no session or no protocol status within {SESSION_WAIT:?} (login or trust dialog?)"
            ));
            workbench.shutdown();
            continue;
        }
        live.pump_for(&mut workbench, Duration::from_secs(3));
        let starts = events(&records(&workbench))
            .iter()
            .filter(|event| *event == "SessionStart")
            .count();
        assert_eq!(
            starts, 1,
            "{name}: the plugin loaded once, one SessionStart"
        );
        assert_eq!(
            claude_code::tab(&workbench).unwrap().session_count(),
            1,
            "{name}: one session, one dot"
        );
        workbench.shutdown();
    }
    let invocations = live.shim_invocations();
    assert!(
        invocations.iter().all(|line| line == "argc=0"),
        "Made ran claude with arguments: {invocations:?}"
    );
    assert_eq!(invocations.len(), 2, "{invocations:?}");
    snapshot_unchanged(&before);
    finish(case, verdict);
}

/// A user plugin and its hooks still run beside Made's, and the user's
/// `CLAUDE_CODE_PLUGIN_DIRS` entry keeps its position.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_user_plugins_and_plugin_dirs_entries_are_preserved() {
    let case = "user-plugins-and-plugin-dirs-preserved";
    let Some(mut live) = Live::new(true) else {
        return;
    };
    let plugin = live._temp.path().join("user-plugin");
    let marker = live._temp.path().join("user-hook-ran");
    fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
    fs::create_dir_all(plugin.join("hooks")).unwrap();
    fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"userplug","version":"0.0.1","description":"live suite user plugin","author":{"name":"made-live"},"license":"MIT"}"#,
    )
    .unwrap();
    fs::write(
        plugin.join("hooks/hooks.json"),
        format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"touch '{}'"}}]}}]}}}}"#,
            marker.display()
        ),
    )
    .unwrap();
    live.env.set(PLUGIN_DIRS, &plugin);
    let mut verdict = Verdict::Pass;
    for (name, target) in [("shell", TerminalTarget::Shell), ("menu", Live::direct())] {
        let _ = fs::remove_file(&marker);
        let mut workbench = live.start(target);
        let composed = workbench
            .terminal()
            .unwrap()
            .options()
            .env
            .iter()
            .find(|(key, _)| key == PLUGIN_DIRS)
            .map(|(_, value)| value.clone())
            .expect("composed plugin dirs");
        assert_eq!(
            composed,
            OsString::from(format!("{}:{}", plugin.display(), live.shipped.display())),
            "{name}: the user entry first, Made's entry once, last"
        );
        if name == "shell" {
            live.run_in_shell(&mut workbench, "");
        }
        if !live.pump_until(&mut workbench, SESSION_WAIT, session_ready) {
            verdict = unverified(format!("{name}: no session within {SESSION_WAIT:?}"));
            workbench.shutdown();
            continue;
        }
        live.pump_for(&mut workbench, Duration::from_secs(2));
        assert!(
            marker.exists(),
            "{name}: the user plugin's SessionStart hook ran"
        );
        assert_eq!(
            claude_code::tab(&workbench).unwrap().session_count(),
            1,
            "{name}: Made's plugin ran with the user's"
        );
        workbench.shutdown();
    }
    finish(case, verdict);
}

/// Claude Code outside Made does not load Made's plugin, even with a copy of a
/// Made tab's spool binding; the same harness sees records when the plugin is
/// loaded (positive control).
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_claude_outside_made_does_not_load_the_plugin() {
    let case = "outside-made-does-not-load-plugin";
    let Some(live) = Live::new(true) else { return };
    let before = config_snapshot();
    let temp_dir = PathBuf::from(std::env::var_os("TMPDIR").unwrap());
    let instance = shared_instance(&temp_dir).expect("spool instance");
    let run = |tab: &str, load_plugin: bool, wait: Duration, stop_on_record: bool| {
        let spool = TabSpool::open(instance.clone(), tab).expect("spool");
        let mut options = PtyOptions::new(&live.cwd)
            .with_shell(find_on_path("claude").unwrap().into_os_string())
            .with_size(made_platform::PtySize {
                rows: 40,
                cols: 120,
            })
            .with_channel_capacity(256)
            .with_env("MADE_CLAUDE_SPOOL", spool.path().as_os_str());
        options = if load_plugin {
            options.with_env(PLUGIN_DIRS, live.shipped.as_os_str())
        } else {
            options.without_env(PLUGIN_DIRS)
        };
        let mut pty = PtyProcess::spawn(&host_platform(), options).expect("claude starts");
        let deadline = Instant::now() + wait;
        let started = Instant::now();
        let mut first_record = None;
        let mut output = 0usize;
        while Instant::now() < deadline {
            match pty.recv_timeout(Duration::from_millis(50)) {
                Ok(PtyEvent::Data(data)) => {
                    output += data.len();
                    if data.windows(3).any(|w| w == b"\x1b[c") {
                        let _ = pty.write(b"\x1b[?62;22c");
                    }
                }
                Ok(PtyEvent::Eof | PtyEvent::Exited(_)) => break,
                _ => {}
            }
            if first_record.is_none() && !read_records(spool.path()).is_empty() {
                first_record = Some(started.elapsed());
                if stop_on_record {
                    break;
                }
            }
        }
        let _ = pty.terminate();
        (first_record, read_records(spool.path()).len(), output)
    };
    let (control_at, control_records, control_output) = run("control", true, SESSION_WAIT, true);
    let Some(control_at) = control_at else {
        finish(
            case,
            unverified(format!(
                "positive control wrote no record within {SESSION_WAIT:?} (output bytes {control_output}); cannot judge absence"
            )),
        );
        return;
    };
    assert!(control_records > 0);
    let wait = control_at + Duration::from_secs(12);
    let (outside_at, outside_records, outside_output) = run("outside", false, wait, false);
    assert!(
        outside_output > 0,
        "claude started outside Made (output seen)"
    );
    assert_eq!(
        (outside_at, outside_records),
        (None, 0),
        "Claude Code outside Made loaded no Made plugin for {wait:?}"
    );
    snapshot_unchanged(&before);
    finish(case, Verdict::Pass);
}

/// `--bare` keeps protocol status only.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_bare_mode_keeps_protocol_status_only() {
    let case = "bare-keeps-protocol-status-only";
    let Some(live) = Live::new(true) else { return };
    let mut workbench = live.start(TerminalTarget::Shell);
    live.run_in_shell(&mut workbench, "--bare");
    let seen = live.pump_until(&mut workbench, SESSION_WAIT, |wb| {
        claude_code::tab(&wb).is_some_and(|tab| tab.protocol_status_seen())
    });
    if !seen {
        workbench.shutdown();
        finish(
            case,
            unverified(format!(
                "no protocol status within {SESSION_WAIT:?} under --bare (bare mode skips OAuth login; needs ANTHROPIC_API_KEY)"
            )),
        );
        return;
    }
    live.pump_for(&mut workbench, Duration::from_secs(4));
    let tab = claude_code::tab(&workbench).unwrap();
    assert_eq!(tab.session_count(), 0, "--bare runs no hook");
    assert!(!tab.dots().is_empty(), "protocol status drives a dot");
    assert!(events(&records(&workbench)).is_empty(), "no hook record");
    workbench.shutdown();
    finish(case, Verdict::Pass);
}

/// `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` keeps hooks only.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_disabled_terminal_title_keeps_hooks_only() {
    let case = "disabled-title-keeps-hooks-only";
    let Some(mut live) = Live::new(true) else {
        return;
    };
    live.env.set("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
    let mut workbench = live.start(TerminalTarget::Shell);
    live.run_in_shell(&mut workbench, "");
    if !live.pump_until(&mut workbench, SESSION_WAIT, session_ready) {
        workbench.shutdown();
        finish(case, unverified("no hook session within the wait"));
        return;
    }
    live.pump_for(&mut workbench, Duration::from_secs(12));
    let tab = claude_code::tab(&workbench).unwrap();
    let protocol = tab.protocol_status_seen();
    workbench.shutdown();
    if protocol {
        finish(
            case,
            unverified("Claude Code still emitted protocol status with the title disabled"),
        );
        return;
    }
    // The diagnostics sentence needs the app-wide detection phase that only an
    // `AppModel` sets; its wording is covered by the fixture tests in
    // `diagnostics.rs` and `ui_model.rs`.
    finish(case, Verdict::Pass);
}

/// tmux started from a Made tab leaves a plain terminal.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_tmux_from_a_made_tab_yields_a_plain_terminal() {
    let case = "tmux-yields-plain-terminal";
    let Some(live) = Live::new(true) else { return };
    if find_on_path("tmux").is_none() {
        finish(case, unverified("tmux is not installed"));
        return;
    }
    let socket = format!("made-live-{}", std::process::id());
    let mut workbench = live.start(TerminalTarget::Shell);
    Live::type_line(
        &mut workbench,
        &format!("tmux -L {socket} new-session '{}'", live.shim.display()),
    );
    let started = live.pump_until(&mut workbench, SESSION_WAIT, |wb| !records(wb).is_empty());
    live.pump_for(&mut workbench, Duration::from_secs(3));
    let tab = claude_code::tab(&workbench).unwrap();
    let plain = tab.session_count() == 0 && tab.dots().is_empty() && !tab.present();
    let root_unmoved = workbench.explorer_root() == Some(live.cwd.as_path());
    let _ = Command::new("tmux")
        .args(["-L", &socket, "kill-server"])
        .status();
    workbench.shutdown();
    assert!(
        plain,
        "no misattributed session or dot for tmux Claude Code"
    );
    assert!(root_unmoved, "the explorer did not move");
    if !started {
        finish(
            case,
            unverified("Claude Code under tmux wrote no (unowned) hook record to prove it ran"),
        );
        return;
    }
    finish(case, Verdict::Pass);
}

/// Neither channel: the shell-typed tab stays plain, the directly launched tab
/// keeps its branding with no dot.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_neither_channel_keeps_direct_launch_branding_only() {
    let case = "neither-channel-direct-branding-only";
    // No hook in the shipped plugin: hooks unavailable; title disabled: no
    // protocol status.
    let Some(mut live) = Live::new(false) else {
        return;
    };
    live.env.set("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
    let mut shell = live.start(TerminalTarget::Shell);
    live.run_in_shell(&mut shell, "");
    let mut direct = live.start(Live::direct());
    live.pump_for(&mut shell, Duration::from_secs(15));
    live.pump_for(&mut direct, Duration::from_secs(1));
    let shell_tab = claude_code::tab(&shell);
    assert!(
        shell_tab.is_none_or(|tab| !tab.present() && tab.dots().is_empty()),
        "the shell-typed tab stays a plain terminal"
    );
    let direct_tab = claude_code::tab(&direct).expect("direct tab");
    assert!(direct_tab.present(), "the direct tab keeps its branding");
    assert!(direct_tab.dots().is_empty(), "no dot without a channel");
    assert!(direct.terminal().is_some_and(TerminalPane::is_running));
    assert!(shell.terminal().is_some_and(TerminalPane::is_running));
    let invoked = live.shim_invocations().len();
    shell.shutdown();
    direct.shutdown();
    if invoked < 2 {
        finish(
            case,
            unverified(format!("only {invoked} claude invocation(s) observed")),
        );
        return;
    }
    finish(case, Verdict::Pass);
}

/// `/clear` replaces the session.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_clear_replaces_the_session() {
    let case = "clear-replaces-the-session";
    let Some(live) = Live::new(true) else { return };
    let mut workbench = live.start(Live::direct());
    if !live.pump_until(&mut workbench, SESSION_WAIT, session_ready) {
        workbench.shutdown();
        finish(case, unverified("no session within the wait"));
        return;
    }
    live.pump_for(&mut workbench, Duration::from_secs(3));
    let first: Vec<String> = distinct_sessions(&records(&workbench));
    Live::type_line(&mut workbench, "/clear");
    let replaced = live.pump_until(&mut workbench, Duration::from_secs(30), |wb| {
        distinct_sessions(&records(wb)).len() > first.len()
    });
    live.pump_for(&mut workbench, Duration::from_secs(3));
    let count = claude_code::tab(&workbench).unwrap().session_count();
    let root = workbench.explorer_root().map(Path::to_path_buf);
    workbench.shutdown();
    if !replaced {
        finish(
            case,
            unverified("/clear produced no new session id in the spool"),
        );
        return;
    }
    assert_eq!(count, 1, "one dot after /clear, not two");
    assert_eq!(root.as_deref(), Some(live.cwd.as_path()));
    finish(case, Verdict::Pass);
}

fn distinct_sessions(records: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = records
        .iter()
        .filter_map(|record| record["sessionId"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// A directory change moves the explorer. Attempted with one minimal prompt
/// that asks for a `cd`; permission is granted interactively.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_directory_change_moves_the_explorer() {
    let case = "working-directory-change";
    let Some(live) = Live::new(true) else { return };
    let sub = live.cwd.join("made-live-sub");
    fs::create_dir_all(&sub).unwrap();
    let sub = sub.canonicalize().unwrap();
    let mut workbench = live.start(Live::direct());
    let verdict = (|| {
        if !live.pump_until(&mut workbench, SESSION_WAIT, session_ready) {
            return unverified("no session within the wait");
        }
        live.submit(
            &mut workbench,
            &format!(
                "Run the Bash command `cd {}` and then reply with the single word done.",
                sub.display()
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut approvals = 0;
        let moved = loop {
            live.pump_for(&mut workbench, Duration::from_millis(500));
            let seen = records(&workbench);
            if seen.iter().any(|record| record["event"] == "CwdChanged") {
                break true;
            }
            if claude_code::tab(&workbench).is_some_and(|tab| {
                tab.dots().contains(
                    &made_app::integrations::claude_code::follow::ClaudeActivity::Attention,
                )
            }) && approvals < 3
            {
                approvals += 1;
                let mut log = made_app::DiagnosticLog::new();
                workbench.terminal_mut().unwrap().send_key(
                    made_terminal::Key::Enter,
                    made_terminal::Modifiers::default(),
                    &mut log,
                );
            }
            if Instant::now() > deadline {
                break false;
            }
        };
        if !moved {
            return unverified(format!(
                "Claude Code fired no CwdChanged hook (events seen: {:?})",
                events(&records(&workbench))
            ));
        }
        live.pump_for(&mut workbench, Duration::from_secs(3));
        assert_eq!(
            workbench.explorer_root(),
            Some(sub.as_path()),
            "the explorer follows the CwdChanged directory"
        );
        Verdict::Pass
    })();
    workbench.shutdown();
    let _ = fs::remove_dir(&sub);
    finish(case, verdict);
}

/// A failed turn marks attention. Attempted with a deliberately unknown model.
#[test]
#[ignore = "needs the real claude; run with MADE_CLAUDE_LIVE=1"]
fn live_stop_failure_marks_attention() {
    let case = "turn-stop-and-stop-failure";
    let Some(mut live) = Live::new(true) else {
        return;
    };
    live.env
        .set("ANTHROPIC_MODEL", "made-live-nonexistent-model");
    live.env.set("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
    let mut workbench = live.start(Live::direct());
    let verdict = (|| {
        if !live.pump_until(&mut workbench, SESSION_WAIT, session_ready) {
            return unverified("no session within the wait");
        }
        Live::type_line(&mut workbench, "hi");
        let failed = live.pump_until(&mut workbench, Duration::from_secs(60), |wb| {
            events(&records(wb)).iter().any(|e| e == "StopFailure")
        });
        if !failed {
            return unverified(format!(
                "Claude Code fired no StopFailure hook (events seen: {:?})",
                events(&records(&workbench))
            ));
        }
        live.pump_for(&mut workbench, Duration::from_secs(2));
        let dots = claude_code::tab(&workbench).unwrap().dots();
        assert_eq!(
            dots,
            vec![made_app::integrations::claude_code::follow::ClaudeActivity::Attention],
            "the dot is attention after the failure"
        );
        Verdict::Pass
    })();
    workbench.shutdown();
    finish(case, verdict);
}

impl Drop for Live {
    fn drop(&mut self) {
        // Leave no real hook staged for the fixture-based tests.
        let _ = fs::remove_dir_all(&self.shipped);
    }
}
