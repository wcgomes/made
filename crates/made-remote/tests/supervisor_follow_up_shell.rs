//! `StartShell`: after a launch-bar TUI exited cleanly the host asks the
//! supervisor for the remote interactive shell in the same attachment. Real
//! supervisor, real PTY children and the real companion over a loopback
//! socket; nothing here reaches a remote host.
#![cfg(unix)]

use made_remote::{
    client::{ClientStream, RemoteClient},
    integration::Integration,
    protocol::*,
    supervisor::{
        capability_bytes, read_envelope, write_envelope, ChildLaunch, Endpoint, FollowUpShell,
        Supervisor, SCROLLBACK_LINES,
    },
};
use made_terminal::{MouseMode, PtyEvent, TerminalCore, TerminalSize, TerminalTransport};
use std::{
    ffi::OsString,
    io::{ErrorKind, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    workspace: PathBuf,
    _temp: tempfile::TempDir,
    identity: Identity,
    endpoint: Endpoint,
    runner: Option<thread::JoinHandle<Result<(), ErrorCode>>>,
    next: u64,
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

/// What a supervisor under test is launched with.
struct Launch {
    target: LaunchTarget,
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    integration: Option<Integration>,
    follow_up: Option<FollowUpShell>,
}

fn base_env(temp: &Path) -> Vec<(OsString, OsString)> {
    vec![
        ("HOME".into(), temp.into()),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("TERM".into(), "xterm-256color".into()),
        ("PS1".into(), "$ ".into()),
    ]
}

fn bash_follow_up() -> FollowUpShell {
    FollowUpShell {
        program: "/bin/bash".into(),
        args: vec![OsString::from("--noprofile"), OsString::from("--norc")],
        env: Vec::new(),
        integration: None,
    }
}

impl Fixture {
    /// A supervisor whose launch child is `/bin/sh -c script`, as if it were a
    /// launch-bar TUI. `follow_up` configures the remote interactive shell the
    /// way the companion does for a non-shell launch.
    fn start(script: &str, follow_up: bool) -> (Self, TcpStream) {
        Self::boot(|temp, _| Launch {
            target: LaunchTarget::OpenCode,
            program: "/bin/sh".into(),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: base_env(temp),
            integration: None,
            follow_up: follow_up.then(bash_follow_up),
        })
    }

    /// A supervisor launched as `launch` describes, given the fixture's
    /// temporary directory and the tab's runtime directory.
    fn boot(launch: impl FnOnce(&Path, &Path) -> Launch) -> (Self, TcpStream) {
        let (fixture, first) = Self::spawn(launch);
        let stream = fixture.connect(first);
        (fixture, stream)
    }

    /// The supervisor is running; nothing is attached yet. `first` is the
    /// `Open` the attachment starts with.
    fn spawn(launch: impl FnOnce(&Path, &Path) -> Launch) -> (Self, WireEnvelope) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(temp.path()).unwrap().join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        let runtime = temp.path().join("remote/runtime/sha256-0123456789abcdef");
        made_platform::ensure_private_directory(&runtime).unwrap();
        let launch = launch(temp.path(), &runtime);
        let identity = Identity {
            authority: AuthorityId::random().unwrap(),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let first = WireEnvelope::new(
            identity.clone(),
            1,
            Operation::Open {
                directory: path(&workspace),
                target: launch.target,
                rows: 24,
                cols: 80,
            },
        );
        let mut supervisor = Supervisor::create(
            &first,
            runtime.clone(),
            temp.path(),
            Path::new(env!("CARGO_BIN_EXE_made-remote")),
            ChildLaunch {
                program: launch.program,
                args: launch.args,
                cwd: workspace.clone(),
                env: launch.env,
            },
            launch.integration,
        )
        .expect("supervisor create");
        if let Some(follow_up) = launch.follow_up {
            supervisor = supervisor.with_follow_up_shell(follow_up);
        }
        let runner = thread::spawn(move || supervisor.run());
        let limit = deadline();
        let endpoint: Endpoint = loop {
            assert!(Instant::now() < limit, "endpoint.json was never published");
            if let Ok(bytes) = std::fs::read(runtime.join("endpoint.json")) {
                break serde_json::from_slice(&bytes).expect("endpoint");
            }
            thread::sleep(Duration::from_millis(20));
        };
        let fixture = Self {
            workspace,
            _temp: temp,
            identity,
            endpoint,
            runner: Some(runner),
            next: 2,
        };
        (fixture, first)
    }

    /// A connected socket past the authenticated preamble, for a client that
    /// sends its own first frame.
    fn connect_preamble(&self) -> TcpStream {
        let mut stream = TcpStream::connect(self.endpoint.addr).expect("connect");
        stream.set_nodelay(true).unwrap();
        stream
            .write_all(&capability_bytes(&self.identity).unwrap())
            .unwrap();
        stream.write_all(&self.identity.authority.0).unwrap();
        stream
            .write_all(&self.identity.epoch.to_be_bytes())
            .unwrap();
        stream
    }

    fn connect(&self, first: WireEnvelope) -> TcpStream {
        let mut stream = TcpStream::connect(self.endpoint.addr).expect("connect");
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        stream
            .write_all(&capability_bytes(&first.identity).unwrap())
            .unwrap();
        stream.write_all(&first.identity.authority.0).unwrap();
        stream
            .write_all(&first.identity.epoch.to_be_bytes())
            .unwrap();
        write_envelope(&mut stream, &first).expect("write first frame");
        stream
    }

    fn reattach(&mut self, epoch: u64, rows: u16, cols: u16) -> TcpStream {
        self.identity.epoch = epoch;
        self.connect(WireEnvelope::new(
            self.identity.clone(),
            1,
            Operation::Attach { rows, cols },
        ))
    }

    fn request(&mut self, stream: &mut TcpStream, operation: Operation) -> u64 {
        let id = self.next;
        self.next += 1;
        write_envelope(
            stream,
            &WireEnvelope::new(self.identity.clone(), id, operation),
        )
        .expect("send request");
        id
    }

    fn type_text(&mut self, stream: &mut TcpStream, text: &str) {
        self.request(
            stream,
            Operation::TerminalInput {
                data: Chunk::new(text.as_bytes().to_vec()).unwrap(),
            },
        );
    }

    fn close(mut self, mut stream: TcpStream) {
        write_envelope(
            &mut stream,
            &WireEnvelope::new(self.identity.clone(), 9_999, Operation::Close),
        )
        .expect("send close");
        frames_until(&mut stream, |operation| {
            matches!(operation, Operation::Closed { .. }).then_some(())
        });
        drop(stream);
        if let Some(runner) = self.runner.take() {
            let _ = runner.join();
        }
    }
}

fn path(path: &Path) -> RemotePath {
    RemotePath::new(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

fn frames_until<T>(stream: &mut TcpStream, mut want: impl FnMut(&Operation) -> Option<T>) -> T {
    let limit = deadline();
    loop {
        assert!(Instant::now() < limit, "deadline waiting for a frame");
        match read_envelope(stream) {
            Ok(frame) => {
                if let Some(found) = want(&frame.operation) {
                    return found;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(error) => panic!("attachment socket failed: {error}"),
        }
    }
}

/// What the host's pane would hold: every output frame fed in arrival order,
/// plus the raw bytes for substring checks.
struct Pane {
    screen: TerminalCore,
    raw: Vec<u8>,
}

impl Pane {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            screen: TerminalCore::with_size(TerminalSize { rows, cols }, SCROLLBACK_LINES),
            raw: Vec::new(),
        }
    }

    fn feed(&mut self, operation: &Operation) {
        if let Operation::TerminalOutput { data, .. } = operation {
            self.raw.extend_from_slice(data.as_bytes());
            self.screen.feed(data.as_bytes());
        }
    }

    fn contains(&self, needle: &str) -> bool {
        self.raw
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    fn until_exit(&mut self, stream: &mut TcpStream) -> u32 {
        frames_until(stream, |operation| {
            self.feed(operation);
            match operation {
                Operation::TerminalExit { code } => Some(*code),
                _ => None,
            }
        })
    }

    fn until_output(&mut self, stream: &mut TcpStream, needle: &str) {
        if self.contains(needle) {
            return;
        }
        frames_until(stream, |operation| {
            self.feed(operation);
            self.contains(needle).then_some(())
        });
    }

    fn until_reply(&mut self, stream: &mut TcpStream, request: u64) -> Operation {
        let limit = deadline();
        loop {
            assert!(Instant::now() < limit, "deadline waiting for the reply");
            match read_envelope(stream) {
                Ok(frame) => {
                    self.feed(&frame.operation);
                    if frame.request_id == request
                        && !matches!(frame.operation, Operation::TerminalOutput { .. })
                    {
                        return frame.operation;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(error) => panic!("attachment socket failed: {error}"),
            }
        }
    }
}

/// A TUI that printed a last line, entered the alternate screen, turned mouse
/// reporting and bracketed paste on, and exited 0 without leaving any of it.
const FULL_SCREEN_THEN_EXIT_0: &str =
    "printf 'last line\\r\\n\\033[?1049h\\033[?1000h\\033[?1006h\\033[?2004hinside'; exit 0";

/// Spec `Remote TUI exits cleanly`: the same attachment gets the remote
/// interactive shell in the explorer root, with the screen kept, the modes the
/// TUI set left, and the pane's size.
#[test]
fn a_clean_tui_exit_then_start_shell_runs_the_shell_in_the_directory_keeping_the_screen() {
    let (mut fixture, mut stream) = Fixture::start(FULL_SCREEN_THEN_EXIT_0, true);
    let followed = fixture.workspace.join("followed");
    std::fs::create_dir_all(&followed).unwrap();
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    assert!(
        pane.screen.alternate_screen(),
        "the TUI left the alt screen on"
    );

    // The pane is resized before the shell starts: the new shell uses it.
    fixture.request(
        &mut stream,
        Operation::TerminalResize {
            rows: 30,
            cols: 100,
        },
    );
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&followed),
            fallback: path(&fixture.workspace),
        },
    );
    match pane.until_reply(&mut stream, request) {
        Operation::ShellStarted { path: started } => assert_eq!(started, path(&followed)),
        other => panic!("expected ShellStarted, got {other:?}"),
    }
    fixture.type_text(&mut stream, "echo CWD=$(pwd) SIZE=$(stty size)\n");
    pane.until_output(
        &mut stream,
        &format!("CWD={} SIZE=30 100\r\n", followed.display()),
    );

    assert!(!pane.screen.alternate_screen(), "off the alternate screen");
    let modes = pane.screen.input_modes();
    assert_eq!(modes.mouse_mode, MouseMode::Off);
    assert!(!modes.sgr_mouse);
    assert!(!modes.bracketed_paste);
    assert!(
        pane.screen.screen_text().contains("last line"),
        "the TUI's last line stays above the new prompt: {:?}",
        pane.screen.screen_text()
    );
    fixture.close(stream);
}

/// Spec `Remote TUI exits cleanly`, second run: the explorer root is gone on
/// the server, so the shell starts in the tab's launch folder.
#[test]
fn start_shell_falls_back_to_the_launch_folder_when_the_directory_is_gone() {
    let (mut fixture, mut stream) = Fixture::start("echo bye; exit 0", true);
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    let gone = fixture.workspace.join("removed-before-exit");
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&gone),
            fallback: path(&fixture.workspace),
        },
    );
    match pane.until_reply(&mut stream, request) {
        Operation::ShellStarted { path: started } => {
            assert_eq!(started, path(&fixture.workspace))
        }
        other => panic!("expected ShellStarted, got {other:?}"),
    }
    fixture.type_text(&mut stream, "echo CWD=$(pwd)\n");
    pane.until_output(
        &mut stream,
        &format!("CWD={}\r\n", fixture.workspace.display()),
    );
    fixture.close(stream);
}

/// The follow-up shell is the tab's shell for later attachments too: a
/// reattach restores the TUI's last line and no TUI modes.
#[test]
fn a_reattach_after_start_shell_restores_the_kept_screen_and_the_shell() {
    let (mut fixture, mut stream) = Fixture::start(FULL_SCREEN_THEN_EXIT_0, true);
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&fixture.workspace),
            fallback: path(&fixture.workspace),
        },
    );
    assert!(matches!(
        pane.until_reply(&mut stream, request),
        Operation::ShellStarted { .. }
    ));
    fixture.type_text(&mut stream, "echo READY\n");
    pane.until_output(&mut stream, "READY\r\n");
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    let mut stream = fixture.reattach(2, 24, 80);
    let mut restored = Pane::new(24, 80);
    let mut done = false;
    frames_until(&mut stream, |operation| {
        if let Operation::TerminalOutput { done: last, .. } = operation {
            if !done {
                restored.feed(operation);
                done = *last;
            }
        }
        done.then_some(())
    });
    assert!(!restored.screen.alternate_screen());
    assert!(restored.screen.screen_text().contains("last line"));
    assert_eq!(restored.screen.input_modes().mouse_mode, MouseMode::Off);
    fixture.type_text(&mut stream, "echo AGAIN\n");
    restored.until_output(&mut stream, "AGAIN\r\n");
    fixture.close(stream);
}

/// A start request never replaces a child that still runs, and a second one
/// never replaces the shell it started.
#[test]
fn start_shell_is_refused_while_a_child_runs() {
    let (mut fixture, mut stream) = Fixture::start("cat", true);
    let mut pane = Pane::new(24, 80);
    let start = |fixture: &mut Fixture, stream: &mut TcpStream| {
        fixture.request(
            stream,
            Operation::StartShell {
                directory: path(&fixture.workspace.clone()),
                fallback: path(&fixture.workspace.clone()),
            },
        )
    };
    let request = start(&mut fixture, &mut stream);
    assert_eq!(
        pane.until_reply(&mut stream, request),
        Operation::Error {
            code: ErrorCode::InvalidRequest
        }
    );
    // The running child is untouched.
    fixture.type_text(&mut stream, "ping\n");
    pane.until_output(&mut stream, "ping\r\n");
    fixture.type_text(&mut stream, "\x04");
    assert_eq!(pane.until_exit(&mut stream), 0);
    let request = start(&mut fixture, &mut stream);
    assert!(matches!(
        pane.until_reply(&mut stream, request),
        Operation::ShellStarted { .. }
    ));
    let request = start(&mut fixture, &mut stream);
    assert_eq!(
        pane.until_reply(&mut stream, request),
        Operation::Error {
            code: ErrorCode::InvalidRequest
        }
    );
    fixture.close(stream);
}

/// A supervisor that was not given a follow-up shell (a plain shell launch)
/// refuses the request and starts nothing.
#[test]
fn start_shell_is_refused_without_a_configured_shell() {
    let (mut fixture, mut stream) = Fixture::start("exit 0", false);
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&fixture.workspace.clone()),
            fallback: path(&fixture.workspace.clone()),
        },
    );
    assert!(matches!(
        pane.until_reply(&mut stream, request),
        Operation::Error { .. }
    ));
    fixture.close(stream);
}

/// An unclean end is never reported as a clean one: a non-zero code and a
/// signal both reach the host as a non-zero `TerminalExit`.
#[test]
fn unclean_ends_report_a_non_zero_exit() {
    let (fixture, mut stream) = Fixture::start("exit 3", true);
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 3);
    fixture.close(stream);
    let (fixture, mut stream) = Fixture::start("kill -9 $$", true);
    let mut pane = Pane::new(24, 80);
    assert_ne!(pane.until_exit(&mut stream), 0);
    fixture.close(stream);
}

/// The exit never overtakes output the child printed before it: everything
/// the TUI said is in the pane when the host learns it exited.
#[test]
fn the_exit_follows_the_output_printed_before_it() {
    let (fixture, mut stream) = Fixture::start(
        "i=0; while [ $i -lt 400 ]; do echo \"line $i\"; i=$((i+1)); done; echo tail-marker; exit 0",
        true,
    );
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    assert!(
        pane.contains("tail-marker"),
        "output reached the host first"
    );
    fixture.close(stream);
}

/// The follow-up shell is the tab's shell: it reports the directory it moves
/// to, which is what keeps the explorer following it.
#[test]
fn the_follow_up_shell_reports_its_directory_changes() {
    let (mut fixture, mut stream) = Fixture::start("exit 0", true);
    let target = fixture.workspace.join("moved-to");
    std::fs::create_dir_all(&target).unwrap();
    let mut pane = Pane::new(24, 80);
    assert_eq!(pane.until_exit(&mut stream), 0);
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&fixture.workspace.clone()),
            fallback: path(&fixture.workspace.clone()),
        },
    );
    assert!(matches!(
        pane.until_reply(&mut stream, request),
        Operation::ShellStarted { .. }
    ));
    fixture.type_text(&mut stream, &format!("cd '{}'\n", target.display()));
    let expected = path(&target);
    frames_until(&mut stream, |operation| match operation {
        Operation::ShellCwd { path } if *path == expected => Some(()),
        _ => None,
    });
    fixture.close(stream);
}

// ---- the follow-up shell's environment -----------------------------------------

/// Plugin package and user config an OpenCode integration needs, in `root`.
fn opencode_fixture(root: &Path) -> (PathBuf, PathBuf) {
    let plugin = root.join("opencode-plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("package.json"), "{\"name\":\"opencode-made\"}").unwrap();
    std::fs::write(plugin.join("tui.ts"), "export default {}").unwrap();
    let config = root.join("cli.json");
    std::fs::write(&config, r#"{"theme":"remote","plugins":["existing"]}"#).unwrap();
    (plugin, config)
}

fn start_integration(root: &Path) -> Option<Integration> {
    let (plugin, config) = opencode_fixture(root);
    Integration::start(&plugin, root, config, None, "remote".into()).ok()
}

/// Launch `kind` the way the companion composes it: its environment and, for
/// a TUI, the shell that takes its place.
fn composed(kind: LaunchTarget, temp: &Path, runtime: &Path, script: &str) -> Launch {
    let temp = &std::fs::canonicalize(temp).unwrap();
    let runtime = &std::fs::canonicalize(runtime).unwrap();
    let staged = temp.join("plugins/claude-made");
    std::fs::create_dir_all(&staged).unwrap();
    let base = made_platform::Environment::from_pairs(base_env(temp));
    let user =
        made_platform::Environment::from_pairs([("CLAUDE_CODE_PLUGIN_DIRS", "/user/plugins")]);
    // Shell and OpenCode launches own the OpenCode bridge; Grok Build and
    // Claude Code launches own none.
    let owns = matches!(kind, LaunchTarget::Shell | LaunchTarget::OpenCode);
    let integration = owns.then(|| start_integration(temp)).flatten();
    let environment = made_remote::launch::compose(
        kind,
        &base,
        integration.as_ref().map(Integration::environment),
        Some(&staged),
        &user,
        &made_remote::claude::spool_path(runtime),
    );
    let (program, args, follow_up) = if kind == LaunchTarget::Shell {
        (
            "/bin/bash".into(),
            vec![OsString::from("--noprofile"), OsString::from("--norc")],
            None,
        )
    } else {
        let root = temp.to_path_buf();
        let start: Option<made_remote::supervisor::IntegrationStart> =
            integration.is_none().then(|| {
                Box::new(move || start_integration(&root))
                    as made_remote::supervisor::IntegrationStart
            });
        (
            "/bin/sh".into(),
            vec![OsString::from("-c"), OsString::from(script)],
            Some(FollowUpShell {
                env: environment.follow_up,
                integration: start,
                ..bash_follow_up()
            }),
        )
    };
    Launch {
        target: kind,
        program,
        args,
        env: environment.launch,
        integration,
        follow_up,
    }
}

/// The environment the interactive shell of `fixture` runs with.
fn shell_env(
    fixture: &mut Fixture,
    stream: &mut TcpStream,
    pane: &mut Pane,
) -> std::collections::BTreeMap<String, String> {
    let file = fixture.workspace.parent().unwrap().join("shell-env.txt");
    fixture.type_text(
        stream,
        &format!("env > '{}'; echo ENV$((1+1))DONE\n", file.display()),
    );
    pane.until_output(stream, "ENV2DONE");
    let text = std::fs::read_to_string(&file).unwrap();
    let tmp = fixture.workspace.parent().unwrap().display().to_string();
    let mut vars = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        // Per-run values: where the shell is, and this fixture's own paths.
        if matches!(key, "_" | "PWD" | "OLDPWD" | "SHLVL") {
            continue;
        }
        vars.insert(key.to_owned(), value.replace(&tmp, "<TMP>"));
    }
    vars
}

/// Spec `Remote TUI exits cleanly`: the shell that takes a launch-bar TUI's
/// place is the shell `Terminal SSH` starts, with the environment a fresh one
/// gets, for every TUI kind.
#[test]
fn the_follow_up_shell_environment_is_what_a_terminal_ssh_shell_gets() {
    let (mut reference, mut stream) =
        Fixture::boot(|temp, runtime| composed(LaunchTarget::Shell, temp, runtime, ""));
    let mut pane = Pane::new(24, 80);
    pane.until_output(&mut stream, "$ ");
    let expected = shell_env(&mut reference, &mut stream, &mut pane);
    reference.close(stream);
    assert!(
        expected.contains_key("OPENCODE_CLI_CONFIG_CONTENT")
            && expected.contains_key("CLAUDE_CODE_PLUGIN_DIRS")
            && expected.contains_key("MADE_CLAUDE_SPOOL"),
        "the reference carries the integration and the Claude Code plugin: {:?}",
        expected.keys().collect::<Vec<_>>()
    );

    for kind in [
        LaunchTarget::OpenCode,
        LaunchTarget::Grok,
        LaunchTarget::Claude,
    ] {
        let (mut fixture, mut stream) =
            Fixture::boot(|temp, runtime| composed(kind, temp, runtime, "exit 0"));
        let mut pane = Pane::new(24, 80);
        assert_eq!(pane.until_exit(&mut stream), 0);
        let request = fixture.request(
            &mut stream,
            Operation::StartShell {
                directory: path(&fixture.workspace.clone()),
                fallback: path(&fixture.workspace.clone()),
            },
        );
        assert!(matches!(
            pane.until_reply(&mut stream, request),
            Operation::ShellStarted { .. }
        ));
        pane.until_output(&mut stream, "$ ");
        let actual = shell_env(&mut fixture, &mut stream, &mut pane);
        fixture.close(stream);
        let differing: Vec<_> = expected
            .keys()
            .chain(actual.keys())
            .filter(|key| {
                // Per-shell values: the bridge's address and token, and the
                // sockets and token of the tab's URL and directory reports.
                // They must be there; they are the shell's own.
                if matches!(
                    key.as_str(),
                    "OPENCODE_CLI_CONFIG_CONTENT"
                        | "MADE_URL_TOKEN"
                        | "MADE_URL_SOCKET"
                        | "MADE_CWD_SOCKET"
                        | "BROWSER"
                ) {
                    expected.contains_key(*key) != actual.contains_key(*key)
                } else {
                    expected.get(*key) != actual.get(*key)
                }
            })
            .collect();
        assert!(differing.is_empty(), "{kind:?}: {differing:?}");
        let config: serde_json::Value =
            serde_json::from_str(&actual["OPENCODE_CLI_CONFIG_CONTENT"]).unwrap_or_else(|_| {
                panic!("{kind:?}: the shell has the OpenCode integration's configuration")
            });
        assert!(config["plugins"].is_array(), "{kind:?}");
    }
}

/// `Open shell` after an unclean end asks the same `StartShell`: for a launch
/// that ended with a non-zero code or a signal, and for a plain shell tab
/// whose shell ended.
#[test]
fn start_shell_serves_an_unclean_tui_end_and_an_ended_plain_shell() {
    for (target, script) in [
        (LaunchTarget::Grok, "exit 3"),
        (LaunchTarget::Claude, "kill -9 $$"),
        (LaunchTarget::Shell, "exit 3"),
    ] {
        let (mut fixture, mut stream) = Fixture::boot(|temp, _| Launch {
            target,
            program: "/bin/sh".into(),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: base_env(temp),
            integration: None,
            follow_up: Some(bash_follow_up()),
        });
        let mut pane = Pane::new(24, 80);
        assert_ne!(pane.until_exit(&mut stream), 0, "{target:?}");
        let request = fixture.request(
            &mut stream,
            Operation::StartShell {
                directory: path(&fixture.workspace.clone()),
                fallback: path(&fixture.workspace.clone()),
            },
        );
        assert!(
            matches!(
                pane.until_reply(&mut stream, request),
                Operation::ShellStarted { .. }
            ),
            "{target:?}"
        );
        fixture.type_text(&mut stream, "echo BACK$((1+1))\n");
        pane.until_output(&mut stream, "BACK2\r\n");
        fixture.close(stream);
    }
}

// ---- the real client and transport over the real supervisor ---------------------

/// The SSH stream of a client, here a loopback socket to the supervisor.
struct Bridge {
    stream: TcpStream,
    received: std::sync::mpsc::Receiver<Vec<u8>>,
    /// While set, what the supervisor sent is held back from the client.
    hold: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Bridge {
    fn new(stream: TcpStream, hold: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        let (tx, received) = std::sync::mpsc::channel();
        let mut reader = stream.try_clone().unwrap();
        thread::spawn(move || {
            use std::io::Read;
            let mut buffer = vec![0; 64 * 1024];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 || tx.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            stream,
            received,
            hold,
        }
    }
}

impl ClientStream for Bridge {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(bytes)
    }
    fn poll(&mut self) -> Option<made_platform::ssh::SshEvent> {
        if self.hold.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        self.received
            .try_recv()
            .ok()
            .map(made_platform::ssh::SshEvent::Data)
    }
    fn detach(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// What the desktop's tab holds: the real client, its terminal transport, the
/// screen it feeds and every exit and reply it has seen.
struct Host {
    client: RemoteClient,
    transport: made_remote::client::RemoteTerminalTransport,
    pane: Pane,
    exits: Vec<u32>,
    replies: Vec<WireEnvelope>,
    hold: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Host {
    fn attach(fixture: &Fixture, first: &WireEnvelope) -> Self {
        let hold = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let client = RemoteClient::new(
            Bridge::new(fixture.connect_preamble(), hold.clone()),
            first.identity.clone(),
            path(&fixture.workspace),
        )
        .unwrap();
        let transport = client.terminal_transport();
        let mut host = Self {
            client,
            transport,
            pane: Pane::new(24, 80),
            exits: Vec::new(),
            replies: Vec::new(),
            hold,
        };
        let id = host
            .client
            .request(first.operation.clone(), None)
            .expect("open");
        assert_eq!(host.reply(id), Operation::Attached);
        host
    }

    /// Take in what arrived: output into the screen, exits and replies noted.
    fn pump(&mut self) {
        while let Some(event) = self.transport.poll() {
            match event {
                PtyEvent::Data(bytes) => {
                    self.pane.raw.extend_from_slice(&bytes);
                    self.pane.screen.feed(&bytes);
                }
                PtyEvent::Exited(status) => self.exits.push(status.code),
                _ => {}
            }
        }
        while let Some(reply) = self.client.poll_reply() {
            self.replies.push(reply);
        }
    }

    fn wait(&mut self, what: &str, mut ready: impl FnMut(&Host) -> bool) {
        let limit = deadline();
        loop {
            self.pump();
            if ready(self) {
                return;
            }
            assert!(Instant::now() < limit, "deadline: {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn reply(&mut self, id: u64) -> Operation {
        self.wait("a reply", |host| {
            host.replies.iter().any(|reply| reply.request_id == id)
        });
        self.replies
            .iter()
            .find(|reply| reply.request_id == id)
            .map(|reply| reply.operation.clone())
            .unwrap()
    }

    /// Let `ms` pass, taking in everything that arrives.
    fn settle(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            self.pump();
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn start_shell(&mut self, directory: &Path, fallback: &Path) -> Operation {
        let id = self
            .client
            .request(
                Operation::StartShell {
                    directory: path(directory),
                    fallback: path(fallback),
                },
                None,
            )
            .expect("StartShell is a valid request");
        self.reply(id)
    }

    fn type_text(&mut self, text: &str) {
        self.transport
            .write(text.as_bytes())
            .expect("the terminal takes input");
    }

    fn see(&mut self, needle: &str) {
        self.wait(needle, |host| host.pane.contains(needle));
    }

    fn finish(mut self, mut fixture: Fixture) {
        let id = self.client.request(Operation::Close, None).unwrap();
        assert!(matches!(self.reply(id), Operation::Closed { .. }));
        if let Some(runner) = fixture.runner.take() {
            let _ = runner.join();
        }
    }
}

/// A program's exit is reported once, not on every pass of the supervisor:
/// repeats would flood the client and overtake the new shell's replies. The
/// shell that replaces the program takes input and resizes, and its own exit
/// is reported once as well.
#[test]
fn a_real_client_sees_one_exit_per_child_and_a_shell_that_takes_input_and_resizes() {
    let (fixture, first) = Fixture::spawn(|temp, _| Launch {
        target: LaunchTarget::Grok,
        program: "/bin/sh".into(),
        args: vec![
            OsString::from("-c"),
            OsString::from("printf 'last line\\r\\n\\033[?1049h\\033[?1000h'; exit 0"),
        ],
        env: base_env(temp),
        integration: None,
        follow_up: Some(bash_follow_up()),
    });
    let mut host = Host::attach(&fixture, &first);
    host.wait("the exit", |host| !host.exits.is_empty());
    host.settle(1_500);
    assert_eq!(
        host.exits,
        vec![0],
        "one exit for the program, not a stream"
    );

    let workspace = fixture.workspace.clone();
    assert!(matches!(
        host.start_shell(&workspace, &workspace),
        Operation::ShellStarted { .. }
    ));
    // The link is as connected as before the program ended.
    assert_eq!(
        host.transport.status(),
        made_terminal::TransportStatus::Connected
    );
    host.settle(1_500);
    assert_eq!(host.exits, vec![0], "no stale exit reaches the new shell");
    host.type_text("echo hi$((1+1))\n");
    host.see("hi2\r\n");
    host.transport
        .resize(30, 100)
        .expect("the terminal resizes");
    host.type_text("stty size\n");
    host.see("30 100\r\n");
    assert!(!host.pane.screen.alternate_screen());
    assert!(host.pane.screen.screen_text().contains("last line"));

    // The shell's own clean exit is one more exit, and the last: that closes
    // the tab.
    host.type_text("exit\n");
    host.wait("the shell's exit", |host| host.exits.len() == 2);
    host.settle(1_000);
    assert_eq!(host.exits, vec![0, 0]);
    host.finish(fixture);
}

/// `Open shell` after an unclean end, and after a plain shell ended: typing
/// works in the shell it starts.
#[test]
fn a_real_client_opens_a_shell_after_an_unclean_end_and_types_into_it() {
    for (target, script) in [
        (LaunchTarget::Claude, "exit 3"),
        (LaunchTarget::Shell, "exit 3"),
    ] {
        let (fixture, first) = Fixture::spawn(|temp, _| Launch {
            target,
            program: "/bin/sh".into(),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: base_env(temp),
            integration: None,
            follow_up: Some(bash_follow_up()),
        });
        let mut host = Host::attach(&fixture, &first);
        host.wait("the exit", |host| !host.exits.is_empty());
        host.settle(500);
        assert_eq!(host.exits, vec![3], "{target:?}");
        let workspace = fixture.workspace.clone();
        assert!(matches!(
            host.start_shell(&workspace, &workspace),
            Operation::ShellStarted { .. }
        ));
        host.type_text("echo back$((2+2))\n");
        host.see("back4\r\n");
        host.settle(500);
        assert_eq!(host.exits, vec![3], "{target:?}");
        host.finish(fixture);
    }
}

/// The client forgets the old program's exit when the shell starts, even if
/// that exit is still queued.
#[test]
fn the_client_drops_a_queued_exit_when_the_shell_starts() {
    let (fixture, first) = Fixture::spawn(|temp, _| Launch {
        target: LaunchTarget::OpenCode,
        program: "/bin/sh".into(),
        args: vec![OsString::from("-c"), OsString::from("exit 0")],
        env: base_env(temp),
        integration: None,
        follow_up: Some(bash_follow_up()),
    });
    let mut host = Host::attach(&fixture, &first);
    // Wait for the exit to be queued, but do not take it from the client.
    let taken_before = host.exits.len();
    thread::sleep(Duration::from_millis(1_500));
    let workspace = fixture.workspace.clone();
    let id = host
        .client
        .request(
            Operation::StartShell {
                directory: path(&workspace),
                fallback: path(&workspace),
            },
            None,
        )
        .unwrap();
    // The reply, without taking in the terminal's queue.
    let limit = deadline();
    loop {
        match host.client.poll_reply() {
            Some(reply) if reply.request_id == id => break,
            Some(_) => {}
            None => {
                assert!(Instant::now() < limit, "no reply to the start");
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    host.settle(300);
    assert_eq!(
        host.exits.len(),
        taken_before,
        "the queued exit of the old program never reaches the new shell: {:?}",
        host.exits
    );
    host.type_text("echo ok$((1+1))\n");
    host.see("ok2\r\n");
    host.finish(fixture);
}

/// A platform whose child can never be observed: its monitor fails.
struct UnobservableChild;

struct UnobservableSession;

impl made_platform::Platform for UnobservableChild {
    fn family(&self) -> made_platform::PlatformFamily {
        made_platform::PlatformFamily::current()
    }
    fn resolve_shell(
        &self,
        _requested: Option<&std::ffi::OsStr>,
    ) -> Result<made_platform::ShellSpec, made_platform::PlatformError> {
        Ok(made_platform::ShellSpec::new("/bin/sh"))
    }
    fn environment(
        &self,
        _overrides: &[(OsString, OsString)],
        _removed: &[OsString],
    ) -> made_platform::Environment {
        made_platform::Environment::from_pairs([("PATH", "/bin")])
    }
    fn app_paths(&self) -> Result<made_platform::AppPaths, made_platform::PlatformError> {
        Err(made_platform::PlatformError::message("not used"))
    }
    fn clipboard(&self) -> Result<Box<dyn made_platform::Clipboard>, made_platform::PlatformError> {
        Err(made_platform::PlatformError::message("not used"))
    }
    fn spawn_pty(
        &self,
        _request: &made_platform::PtyRequest,
    ) -> Result<Box<dyn made_platform::PtySession>, made_platform::PlatformError> {
        Ok(Box::new(UnobservableSession))
    }
}

impl made_platform::PtySession for UnobservableSession {
    fn take_reader(&self) -> Result<Box<dyn std::io::Read + Send>, made_platform::PlatformError> {
        Ok(Box::new(std::io::empty()))
    }
    fn take_writer(&self) -> Result<Box<dyn Write + Send>, made_platform::PlatformError> {
        Ok(Box::new(std::io::sink()))
    }
    fn resize(&self, _size: made_platform::PtySize) -> Result<(), made_platform::PlatformError> {
        Ok(())
    }
    fn size(&self) -> Result<made_platform::PtySize, made_platform::PlatformError> {
        Ok(made_platform::PtySize::default())
    }
    fn try_wait(&self) -> Result<Option<made_platform::ExitStatus>, made_platform::PlatformError> {
        Err(made_platform::PlatformError::message(
            "injected monitor failure",
        ))
    }
    fn kill(&self) -> Result<(), made_platform::PlatformError> {
        Ok(())
    }
    fn pid(&self) -> Option<u32> {
        Some(4242)
    }
}

/// A child whose state the monitor lost may still run: it is never replaced,
/// the request is refused and the child is left alone.
#[test]
fn a_child_whose_state_the_monitor_lost_is_never_replaced() {
    use made_remote::supervisor::NativeSession;
    let mut session = NativeSession::spawn(&UnobservableChild, made_terminal::PtyOptions::new("/"))
        .expect("spawn the fake child");
    thread::sleep(Duration::from_millis(300));
    assert!(!session.has_ended(), "an unobservable child has not ended");
    let temp = tempfile::tempdir().unwrap();
    let refused = session.restart(
        &made_platform::host_platform(),
        made_terminal::PtyOptions::new(temp.path()),
    );
    assert_eq!(refused.err(), Some(ErrorCode::InvalidRequest));
    assert_eq!(session.pid(), Some(4242), "the child was not replaced");
}

/// A client reset (a root transition) can discard the `ShellStarted` that
/// resets the client's terminal. The tab that then asks again, is told a child
/// runs and adopts the shell resets it itself: the shell takes input and
/// resizes.
#[test]
fn an_adopted_shell_takes_input_after_a_reset_discarded_the_answer() {
    let (fixture, first) = Fixture::spawn(|temp, _| Launch {
        target: LaunchTarget::Grok,
        program: "/bin/sh".into(),
        args: vec![OsString::from("-c"), OsString::from("exit 0")],
        env: base_env(temp),
        integration: None,
        follow_up: Some(bash_follow_up()),
    });
    let mut host = Host::attach(&fixture, &first);
    host.wait("the exit", |host| !host.exits.is_empty());
    let workspace = fixture.workspace.clone();
    // The companion starts the shell; its answer is held until the reset.
    host.hold.store(true, std::sync::atomic::Ordering::SeqCst);
    host.client
        .request(
            Operation::StartShell {
                directory: path(&workspace),
                fallback: path(&workspace),
            },
            None,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(800));
    host.client.reset_pending().unwrap();
    host.hold.store(false, std::sync::atomic::Ordering::SeqCst);
    host.settle(300);
    assert!(
        host.transport.write(b"x").is_err(),
        "the discarded answer left the terminal reading as ended"
    );
    // The tab asks again: a child runs, so it adopts the shell.
    assert_eq!(
        host.start_shell(&workspace, &workspace),
        Operation::Error {
            code: ErrorCode::InvalidRequest
        }
    );
    host.client.child_replaced();
    host.type_text("echo adopted$((1+1))\n");
    host.see("adopted2\r\n");
    host.transport.resize(30, 100).expect("a resize is taken");
    host.type_text("stty size\n");
    host.see("30 100\r\n");
    assert_eq!(host.exits, vec![0], "no exit besides the program's own");
    host.finish(fixture);
}

/// The tab adopts a shell it never saw start when a reattach's capabilities
/// say one runs there: `shell_cwd` is false for a program launch and true once
/// the follow-up shell runs.
#[test]
fn shell_cwd_capability_is_false_for_a_program_and_true_once_the_follow_up_shell_runs() {
    let (mut fixture, mut stream) = Fixture::start("exit 0", true);
    let mut pane = Pane::new(24, 80);
    let shell_cwd = |fixture: &mut Fixture, stream: &mut TcpStream, pane: &mut Pane| {
        let request = fixture.request(stream, Operation::Capabilities);
        match pane.until_reply(stream, request) {
            Operation::CapabilitiesResult { capabilities } => capabilities.shell_cwd,
            other => panic!("expected capabilities, got {other:?}"),
        }
    };
    assert_eq!(pane.until_exit(&mut stream), 0);
    assert!(
        !shell_cwd(&mut fixture, &mut stream, &mut pane),
        "no shell runs for a program launch"
    );
    let request = fixture.request(
        &mut stream,
        Operation::StartShell {
            directory: path(&fixture.workspace.clone()),
            fallback: path(&fixture.workspace.clone()),
        },
    );
    assert!(matches!(
        pane.until_reply(&mut stream, request),
        Operation::ShellStarted { .. }
    ));
    assert!(
        shell_cwd(&mut fixture, &mut stream, &mut pane),
        "the follow-up shell reports its directory"
    );
    fixture.close(stream);
}
