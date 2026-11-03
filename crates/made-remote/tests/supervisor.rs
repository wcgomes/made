//! Native supervisor tests. tmux is absent from the owner and from these cases.
use made_platform::PtySize;
use made_remote::{
    protocol::*,
    supervisor::{
        generation_exempt, live_terminal_output, next_snapshot_frame, transport_matches, Lease,
        NativeSession, VtState, SCROLLBACK_LINES,
    },
};
use made_terminal::{MouseMode, PtyOptions, TerminalCore, TerminalSize};
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

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

fn process_alive(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

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
