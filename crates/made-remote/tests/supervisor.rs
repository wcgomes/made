//! Native supervisor tests. tmux is absent from the owner and from these cases.
//! The PTY spawn case is Unix-only; Windows remotes are unsupported and that
//! test is not treated as a pass there.
#[cfg(unix)]
use made_platform::PtySize;
#[cfg(unix)]
use made_remote::supervisor::NativeSession;
use made_remote::{
    protocol::*,
    supervisor::{
        generation_exempt, live_terminal_output, next_snapshot_frame, transport_matches, Lease,
        VtState, SCROLLBACK_LINES,
    },
};
#[cfg(unix)]
use made_terminal::PtyOptions;
use made_terminal::{MouseMode, TerminalCore, TerminalSize};
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn resnapshot_ends_with_one_done_before_live_output() {
    let mut rest = b"abcdef".repeat(20_000);
    let mut frames = Vec::new();
    let mut completed = false;
    while let Some((operation, consumed, closed)) = next_snapshot_frame(&rest, completed) {
        match &operation {
            Operation::TerminalOutput { data, done } => {
                if !closed {
                    assert!(!*done);
                    assert!(!data.as_bytes().is_empty());
                } else {
                    assert!(*done);
                }
            }
            other => panic!("unexpected {other:?}"),
        }
        frames.push(operation);
        rest.drain(..consumed);
        completed = closed;
    }
    assert!(completed);
    assert!(frames.len() >= 2);
    assert!(frames[..frames.len() - 1]
        .iter()
        .all(|frame| matches!(frame, Operation::TerminalOutput { done: false, .. })));
    assert!(matches!(
        frames.last(),
        Some(Operation::TerminalOutput { done: true, .. })
    ));
    assert!(next_snapshot_frame(&[], true).is_none());
    let live = live_terminal_output(b"after").unwrap();
    assert!(matches!(
        live,
        Operation::TerminalOutput { done: false, .. }
    ));
    assert!(live_terminal_output(b"").is_none());
    let (only, _, closed) = next_snapshot_frame(b"", false).unwrap();
    assert!(closed);
    assert!(matches!(
        only,
        Operation::TerminalOutput {
            done: true,
            ..
        } if only_empty(&only)
    ));
}

fn only_empty(operation: &Operation) -> bool {
    matches!(operation, Operation::TerminalOutput { data, done: true } if data.as_bytes().is_empty())
}

#[test]
fn lease_uses_authenticated_heartbeat_time_and_exact_24_hours() {
    let now = Instant::now();
    let mut lease = Lease::new(now);
    assert!(!lease.expired(now + DETACHED_LEASE - Duration::from_secs(1)));
    assert!(lease.expired(now + DETACHED_LEASE));
    lease.renew(now + HEARTBEAT_INTERVAL);
    assert!(!lease.expired(now + DETACHED_LEASE));
}

#[test]
fn capabilities_field_is_not_a_tmux_gate_and_generation_does_not_block_heartbeat() {
    assert!(generation_exempt(&Operation::Heartbeat));
    assert!(generation_exempt(&Operation::TerminalInput {
        data: Chunk::new(b"x".to_vec()).unwrap(),
    }));
    assert!(!generation_exempt(&Operation::List {
        path: RemotePath::new(b"/project".to_vec()).unwrap(),
    }));
    let mut current = Identity {
        authority: AuthorityId([1; 16]),
        tab: TabCapability::random().unwrap(),
        epoch: 2,
        generation: 4,
    };
    let mut heartbeat = current.clone();
    heartbeat.generation = 1;
    assert!(transport_matches(&heartbeat, &current));
    current.epoch = 3;
    assert!(!transport_matches(&heartbeat, &current));
}

#[test]
fn resnapshot_restores_modes_mouse_alternate_screen_and_primary_without_replay() {
    let mut owner = VtState::new(8, 40);
    owner.feed(b"PRIMARY\r\n");
    owner.feed(b"\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1h");
    owner.feed(b"\x1b[?1049hALT\x1b]999;do-not-replay\x07");
    assert!(owner.alternate_screen());
    let snapshot = owner.resnapshot();
    assert!(
        !snapshot
            .windows(b"do-not-replay".len())
            .any(|window| window == b"do-not-replay"),
        "historical PTY bytes must not be replayed"
    );
    assert!(snapshot.len() < SCROLLBACK_LINES * 64);

    let mut restored =
        TerminalCore::with_size(TerminalSize { rows: 8, cols: 40 }, SCROLLBACK_LINES);
    restored.feed(&snapshot);
    assert!(restored.alternate_screen());
    assert!(restored.screen_text().contains("ALT"));
    let modes = restored.input_modes();
    assert_eq!(modes.mouse_mode, MouseMode::Click);
    assert!(modes.sgr_mouse);
    assert!(modes.bracketed_paste);
    assert!(modes.app_cursor);
    restored.feed(b"\x1b[?1049l");
    assert!(!restored.alternate_screen());
    assert!(
        restored.screen_text().contains("PRIMARY"),
        "leaving alternate screen must show the retained primary screen, got {:?}",
        restored.screen_text()
    );
}

#[test]
fn primary_scrollback_survives_alternate_screen_resnapshot() {
    let mut owner = VtState::new(4, 20);
    owner.feed(b"line-0\r\nline-1\r\nline-2\r\nline-3\r\nline-4\r\n");
    owner.feed(b"\x1b[?1049h");
    let snapshot = owner.resnapshot();
    let mut restored =
        TerminalCore::with_size(TerminalSize { rows: 4, cols: 20 }, SCROLLBACK_LINES);
    restored.feed(&snapshot);
    restored.feed(b"\x1b[?1049l");
    let history = restored
        .history(SCROLLBACK_LINES)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>();
    assert!(
        history.iter().any(|line| line.contains("line-0"))
            || restored.screen_text().contains("line-0"),
        "primary scrollback missing after resnapshot: screen {:?} history {history:?}",
        restored.screen_text()
    );
}

#[test]
#[cfg(unix)]
fn same_child_pid_and_session_survive_frontend_drop_without_tmux() {
    let platform = made_platform::host_platform();
    let temp = tempfile::tempdir().unwrap();
    let session = NativeSession::spawn(
        &platform,
        PtyOptions::new(temp.path())
            .with_shell("/bin/sh")
            .with_args(["-c", "sleep 30"])
            .with_size(PtySize { rows: 24, cols: 80 }),
    )
    .expect("spawn native pty");
    let pid = session.pid().expect("child pid");
    let before = process_session(pid);
    let attachment = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    drop(attachment);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(session.pid(), Some(pid));
    assert!(process_alive(pid), "frontend drop killed the PTY child");
    assert_eq!(process_session(pid), before);
    drop(session);
    thread::sleep(Duration::from_millis(50));
    assert!(
        !process_alive(pid),
        "scoped close must terminate the owned child"
    );
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(unix)]
fn process_session(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "sess=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[test]
#[cfg(unix)]
fn attached_reader_survives_idle_reads_and_close_confirms_over_a_real_socket() {
    use made_remote::supervisor::{
        capability_bytes, read_envelope, write_envelope, ChildLaunch, Endpoint, Supervisor,
    };
    use std::io::{ErrorKind, Write};
    use std::net::TcpStream;

    // Regression for two macOS-only stalls that the loopback harness exposed:
    // BSD accepted sockets inherit the listener's O_NONBLOCK, so an idle
    // attachment read fails read_exact with EAGAIN and silently kills the
    // reader (set_read_timeout(None) does not clear the flag); and a session
    // runtime path past the kernel AF_UNIX budget must not prevent the cwd
    // sink from binding. Both fixes are exercised through a real socket.
    let temp = tempfile::tempdir().unwrap();
    let deep = temp.path().join("h".repeat(72));
    let runtime = deep.join("remote/runtime/sha256-0123456789abcdef");
    // The frontend creates the private runtime base before the per-session
    // leaf; mirror that here.
    made_platform::ensure_private_directory(&runtime).unwrap();
    let over_limit = runtime.join("cwd-1-0/report.sock").as_os_str().len();
    assert!(over_limit > 103, "fixture must exceed the AF_UNIX budget");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
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
            directory: RemotePath::new(workspace.as_os_str().as_encoded_bytes().to_vec()).unwrap(),
            target: LaunchTarget::Shell,
            rows: 24,
            cols: 80,
        },
    );
    let supervisor = Supervisor::create(
        &first,
        runtime.clone(),
        temp.path(),
        std::path::Path::new("/bin/sh"),
        ChildLaunch {
            program: workspace.clone(),
            args: Vec::new(),
            cwd: workspace.clone(),
            env: Vec::new(),
        },
        None,
    )
    .expect("supervisor create with a long runtime path");
    let runner = thread::spawn(move || supervisor.run());

    let deadline = Instant::now() + Duration::from_secs(15);
    let endpoint: Endpoint = loop {
        assert!(
            Instant::now() < deadline,
            "endpoint.json was never published"
        );
        if let Ok(bytes) = std::fs::read(runtime.join("endpoint.json")) {
            break serde_json::from_slice(&bytes).expect("endpoint");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let mut stream = TcpStream::connect(endpoint.addr).expect("connect");
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let capability = capability_bytes(&identity).expect("capability");
    stream.write_all(&capability).unwrap();
    stream.write_all(&identity.authority.0).unwrap();
    stream.write_all(&identity.epoch.to_be_bytes()).unwrap();
    write_envelope(&mut stream, &first).expect("write open");
    fn read_until(
        stream: &mut TcpStream,
        deadline: Instant,
        want: impl Fn(&Operation) -> bool,
    ) -> WireEnvelope {
        loop {
            assert!(Instant::now() < deadline, "deadline waiting for a frame");
            match read_envelope(stream) {
                Ok(frame) if want(&frame.operation) => return frame,
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(error) => panic!("attachment socket failed: {error}"),
            }
        }
    }
    read_until(&mut stream, deadline, |operation| {
        matches!(operation, Operation::Attached)
    });
    // The idle window: a nonblocking reader dies here before any byte arrives.
    thread::sleep(Duration::from_millis(400));
    write_envelope(
        &mut stream,
        &WireEnvelope::new(identity.clone(), 2, Operation::Close),
    )
    .expect("write close");
    let closed = read_until(&mut stream, deadline, |operation| {
        matches!(operation, Operation::Closed { .. })
    });
    let Operation::Closed { confirmed } = closed.operation else {
        panic!("expected Closed")
    };
    assert!(confirmed, "close was not confirmed");
    drop(stream);
    assert!(runner.join().expect("run thread").is_ok());
    // The runtime directory is removed by the supervisor after Close.
    let removed = Instant::now() + Duration::from_secs(5);
    while Instant::now() < removed && runtime.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!runtime.exists(), "runtime directory survived Close");
}

#[cfg(unix)]
mod live {
    //! Real supervisor, real PTY child and the real `made-remote` companion
    //! over a loopback socket, which is the transport the SSH frontend relays.
    use super::*;
    use made_remote::supervisor::{
        capability_bytes, read_envelope, write_envelope, ChildLaunch, Endpoint, Supervisor,
    };
    use std::ffi::OsString;
    use std::io::{ErrorKind, Write};
    use std::net::TcpStream;
    use std::path::{Path, PathBuf};

    pub struct Session {
        pub workspace: PathBuf,
        _temp: tempfile::TempDir,
        identity: Identity,
        endpoint: Endpoint,
        runner: Option<thread::JoinHandle<Result<(), ErrorCode>>>,
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(15)
    }

    impl Session {
        /// Starts the supervisor with `args` for `/bin/bash` and an environment
        /// that, like a non-interactive SSH exec session, has no locale.
        pub fn start(args: &[&str]) -> (Self, TcpStream) {
            let temp = tempfile::tempdir().unwrap();
            let workspace = std::fs::canonicalize(temp.path()).unwrap().join("work");
            std::fs::create_dir_all(&workspace).unwrap();
            let runtime = temp.path().join("remote/runtime/sha256-0123456789abcdef");
            made_platform::ensure_private_directory(&runtime).unwrap();
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
                    directory: RemotePath::new(workspace.as_os_str().as_encoded_bytes().to_vec())
                        .unwrap(),
                    target: LaunchTarget::Shell,
                    rows: 24,
                    cols: 80,
                },
            );
            let env: Vec<(OsString, OsString)> = vec![
                ("HOME".into(), temp.path().into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
                ("PS1".into(), "$ ".into()),
            ];
            let supervisor = Supervisor::create(
                &first,
                runtime.clone(),
                temp.path(),
                Path::new(env!("CARGO_BIN_EXE_made-remote")),
                ChildLaunch {
                    program: "/bin/bash".into(),
                    args: args.iter().map(OsString::from).collect(),
                    cwd: workspace.clone(),
                    env,
                },
                None,
            )
            .expect("supervisor create");
            let runner = thread::spawn(move || supervisor.run());
            let limit = deadline();
            let endpoint: Endpoint = loop {
                assert!(Instant::now() < limit, "endpoint.json was never published");
                if let Ok(bytes) = std::fs::read(runtime.join("endpoint.json")) {
                    break serde_json::from_slice(&bytes).expect("endpoint");
                }
                thread::sleep(Duration::from_millis(20));
            };
            let session = Self {
                workspace,
                _temp: temp,
                identity,
                endpoint,
                runner: Some(runner),
            };
            let stream = session.connect(first);
            (session, stream)
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

        /// Reattach as a new transport epoch, like the client does after a drop.
        pub fn reattach(&mut self, epoch: u64, rows: u16, cols: u16) -> TcpStream {
            self.identity.epoch = epoch;
            self.connect(WireEnvelope::new(
                self.identity.clone(),
                1,
                Operation::Attach { rows, cols },
            ))
        }

        pub fn type_text(&self, stream: &mut TcpStream, request: u64, text: &str) {
            write_envelope(
                stream,
                &WireEnvelope::new(
                    self.identity.clone(),
                    request,
                    Operation::TerminalInput {
                        data: Chunk::new(text.as_bytes().to_vec()).unwrap(),
                    },
                ),
            )
            .expect("send input");
        }

        pub fn close(mut self, mut stream: TcpStream) {
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

    /// Reads frames until `want` returns a value; panics at the deadline.
    pub fn frames_until<T>(
        stream: &mut TcpStream,
        mut want: impl FnMut(&Operation) -> Option<T>,
    ) -> T {
        let limit = deadline();
        let mut trail: Vec<String> = Vec::new();
        loop {
            assert!(
                Instant::now() < limit,
                "deadline waiting for a frame; frames seen: {trail:?}"
            );
            match read_envelope(stream) {
                Ok(frame) => {
                    trail.push(match &frame.operation {
                        Operation::TerminalOutput { data, done } => {
                            format!(
                                "output(done={done}) {:?}",
                                String::from_utf8_lossy(data.as_bytes())
                            )
                        }
                        Operation::ShellCwd { path } => {
                            format!("cwd {:?}", String::from_utf8_lossy(path.as_bytes()))
                        }
                        _ => "other".into(),
                    });
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

    /// Waits for an interactive bash prompt. `PS1` from the environment is not
    /// authoritative: Debian's bash reads `/etc/bash.bashrc`, which replaces it
    /// with `user@host:dir$ ` (`# ` for root, as in the glibc 2.31 container).
    pub fn prompt_until(stream: &mut TcpStream, collected: &mut Vec<u8>) {
        frames_until(stream, |operation| {
            if let Operation::TerminalOutput { data, .. } = operation {
                collected.extend_from_slice(data.as_bytes());
            }
            collected
                .windows(2)
                .any(|window| window == b"$ " || window == b"# ")
                .then_some(())
        });
    }

    /// Collects terminal output until it contains `needle`.
    pub fn output_until(stream: &mut TcpStream, collected: &mut Vec<u8>, needle: &str) {
        let needle = needle.as_bytes();
        frames_until(stream, |operation| {
            if let Operation::TerminalOutput { data, .. } = operation {
                collected.extend_from_slice(data.as_bytes());
            }
            collected
                .windows(needle.len())
                .any(|window| window == needle)
                .then_some(())
        });
    }
}

#[test]
#[cfg(unix)]
fn shell_cwd_follows_cd_into_a_path_with_spaces_and_non_ascii() {
    let (session, mut stream) = live::Session::start(&["--noprofile", "--norc"]);
    let target = session.workspace.join("dir com espaço").join("ação 日本");
    std::fs::create_dir_all(&target).unwrap();
    let mut wire = Vec::new();
    live::prompt_until(&mut stream, &mut wire);
    session.type_text(
        &mut stream,
        2,
        &format!("cd '{}'\n", target.to_str().unwrap()),
    );
    let expected = target.as_os_str().as_encoded_bytes().to_vec();
    let reported = live::frames_until(&mut stream, |operation| match operation {
        Operation::ShellCwd { path } if path.as_bytes() == expected => Some(path.clone()),
        _ => None,
    });
    assert_eq!(reported.as_bytes(), expected);
    session.close(stream);
}

#[test]
#[cfg(unix)]
fn non_ascii_input_reaches_a_remote_shell_that_started_without_a_locale() {
    let installed = made_remote::supervisor::installed_locales();
    if !installed.iter().any(|name| {
        let name = name.to_ascii_lowercase();
        name.contains("utf-8") || name.contains("utf8")
    }) {
        eprintln!("no UTF-8 locale installed on this host; nothing to select");
        return;
    }
    let (session, mut stream) = live::Session::start(&["--noprofile", "--norc"]);
    let mut wire = Vec::new();
    live::prompt_until(&mut stream, &mut wire);
    // Bash counts characters only in a UTF-8 locale; under `C` this is 12.
    session.type_text(&mut stream, 2, "x='ação ç ã'; echo \"LEN=${#x}\"\n");
    live::output_until(&mut stream, &mut wire, "LEN=8\r\n");
    let text = String::from_utf8_lossy(&wire);
    assert!(
        text.contains("x='ação ç ã'"),
        "readline must echo the accented text, not octal escapes: {text:?}"
    );
    session.close(stream);
}

#[test]
#[cfg(unix)]
fn reattach_makes_a_full_screen_child_repaint() {
    let script = "printf '\\033[?1049h\\033[31mred\\033[0m'; \
                  trap 'echo \"WINCH $(stty size)\"' WINCH; echo ready; \
                  while :; do sleep 0.02; done";
    let (mut session, mut stream) = live::Session::start(&["-c", script]);
    let mut first = Vec::new();
    live::output_until(&mut stream, &mut first, "ready");
    // The connection drops; the same child keeps running under the supervisor.
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    // Same size as before: without a nudge the child would never be signalled.
    let mut stream = session.reattach(2, 24, 80);
    let mut snapshot = Vec::new();
    let mut done = false;
    live::frames_until(&mut stream, |operation| {
        if let Operation::TerminalOutput { data, done: last } = operation {
            if !done {
                snapshot.extend_from_slice(data.as_bytes());
                done = *last;
            }
        }
        done.then_some(())
    });
    let mut restored =
        TerminalCore::with_size(TerminalSize { rows: 24, cols: 80 }, SCROLLBACK_LINES);
    restored.feed(&snapshot);
    assert!(restored.alternate_screen());
    let red = &restored.snapshot().cells[0];
    assert_eq!(red.ch, 'r');
    assert_ne!(
        red.fg,
        made_terminal::TerminalColor::Default,
        "the snapshot must carry colors, not only text"
    );

    // The child must be signalled although the size never changed for it, and
    // end at the real size. Whether it also observes the 23-row step depends on
    // how fast it runs its trap (a slow macOS runner coalesces both SIGWINCH
    // into one handler run that already sees 24 rows), so that is not asserted.
    let mut output = Vec::new();
    live::output_until(&mut stream, &mut output, "WINCH 24 80");
    session.close(stream);
}

#[test]
fn resnapshot_keeps_cell_colors_and_attributes() {
    let bytes = b"\x1b[1;31mred\x1b[0m \x1b[48;2;1;2;3;4mtrue\x1b[0m \x1b[38;5;200;7mx\x1b[0m";
    let size = TerminalSize { rows: 4, cols: 40 };
    let mut owner = VtState::new(4, 40);
    owner.feed(bytes);
    let mut restored = TerminalCore::with_size(size, SCROLLBACK_LINES);
    restored.feed(&owner.resnapshot());
    let mut reference = TerminalCore::with_size(size, SCROLLBACK_LINES);
    reference.feed(bytes);
    let (restored, reference) = (restored.snapshot(), reference.snapshot());
    for (index, (got, want)) in restored.cells.iter().zip(&reference.cells).enumerate() {
        assert_eq!(
            (got.ch, got.fg, got.bg, got.attributes),
            (want.ch, want.fg, want.bg, want.attributes),
            "cell {index}"
        );
    }
}

#[test]
fn utf8_locale_selection_trusts_only_what_the_server_lists() {
    use made_remote::supervisor::ensure_utf8_locale;
    use std::ffi::OsString;
    let env = |pairs: &[(&str, &str)]| -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    };
    let ctype = |env: &[(OsString, OsString)]| {
        env.iter()
            .rev()
            .find(|(k, _)| k == "LC_CTYPE")
            .map(|(_, v)| v.to_string_lossy().into_owned())
    };
    // A forwarded plain `UTF-8` (macOS client) is not a glibc locale.
    let mut forwarded = env(&[("LC_CTYPE", "UTF-8")]);
    ensure_utf8_locale(&mut forwarded, || vec!["C".into(), "C.utf8".into()]);
    assert_eq!(ctype(&forwarded).as_deref(), Some("C.utf8"));
    // A forwarded locale the server lists is kept, whatever its spelling.
    let mut listed = env(&[("LANG", "en_US.UTF-8")]);
    ensure_utf8_locale(&mut listed, || vec!["en_US.utf8".into(), "C.utf8".into()]);
    assert_eq!(ctype(&listed), None);
    // LANG is not trusted by name alone either.
    let mut unlisted = env(&[("LANG", "pt_BR.UTF-8")]);
    ensure_utf8_locale(&mut unlisted, || vec!["C.utf8".into()]);
    assert_eq!(ctype(&unlisted).as_deref(), Some("C.utf8"));
    // No locale session, none installed: nothing is invented.
    let mut none = env(&[]);
    ensure_utf8_locale(&mut none, || vec!["C".into(), "POSIX".into()]);
    assert_eq!(ctype(&none), None);
    // A non-empty LC_ALL is the user's choice.
    let mut pinned = env(&[("LC_ALL", "C")]);
    ensure_utf8_locale(&mut pinned, || vec!["C.utf8".into()]);
    assert_eq!(ctype(&pinned), None);
}

#[test]
#[cfg(unix)]
fn reattach_restores_the_shell_cwd_the_client_forgot() {
    let (mut session, mut stream) = live::Session::start(&["--noprofile", "--norc"]);
    let target = session.workspace.join("depois do cd");
    std::fs::create_dir_all(&target).unwrap();
    let expected = target.as_os_str().as_encoded_bytes().to_vec();
    let mut wire = Vec::new();
    live::prompt_until(&mut stream, &mut wire);
    session.type_text(
        &mut stream,
        2,
        &format!("cd '{}'\n", target.to_str().unwrap()),
    );
    live::frames_until(&mut stream, |operation| match operation {
        Operation::ShellCwd { path } if path.as_bytes() == expected => Some(()),
        _ => None,
    });
    drop(stream);
    thread::sleep(Duration::from_millis(100));
    let mut stream = session.reattach(2, 24, 80);
    // The report is re-sent without another cd, after the attach reply.
    live::frames_until(&mut stream, |operation| match operation {
        Operation::ShellCwd { path } if path.as_bytes() == expected => Some(()),
        _ => None,
    });
    session.close(stream);
}
