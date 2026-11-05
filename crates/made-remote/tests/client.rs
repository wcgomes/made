//! Deterministic fake stream boundary tests, NOT SSH/tmux parity acceptance.
use made_platform::ssh::SshEvent;
use made_remote::{
    client::{ClientStream, RemoteClient, TerminalFrame},
    protocol::*,
};
use made_terminal::{PtyError, TerminalTransport};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct FakeState {
    sent: Vec<u8>,
    received: VecDeque<SshEvent>,
    detached: bool,
    blocked: bool,
}
struct Fake(Arc<Mutex<FakeState>>);
impl ClientStream for Fake {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut s = self.0.lock().unwrap();
        if s.blocked {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        s.sent.extend_from_slice(bytes);
        Ok(())
    }
    fn poll(&mut self) -> Option<SshEvent> {
        self.0.lock().unwrap().received.pop_front()
    }
    fn detach(&mut self) {
        self.0.lock().unwrap().detached = true;
    }
}
fn client() -> (RemoteClient, Arc<Mutex<FakeState>>) {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let identity = Identity {
        authority: AuthorityId::random().unwrap(),
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    (
        RemoteClient::new(
            Fake(state.clone()),
            identity,
            RemotePath::new(b"/project".to_vec()).unwrap(),
        )
        .unwrap(),
        state,
    )
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn snapshot(revision: u64) -> made_bridge::state::SessionSnapshot {
    made_bridge::state::SessionSnapshot {
        revision,
        active_session_id: None,
        tabs_enabled: true,
        view_directory: None,
        sessions: Vec::new(),
    }
}
fn grok_event(kind: GrokEventKind, session: &str) -> Operation {
    Operation::GrokEvent {
        event: GrokSessionEvent {
            kind,
            session_id: session.to_owned(),
            cwd: None,
            activity: GrokActivity::Idle,
        },
    }
}
fn grok_snapshot(revision: u64) -> Operation {
    Operation::GrokSnapshot {
        snapshot: GrokSessionSnapshot {
            revision,
            running: true,
            hooks_ready: false,
            sessions: Vec::new(),
        },
    }
}
fn attach(client: &RemoteClient) -> u64 {
    client
        .request(
            Operation::Open {
                directory: RemotePath::new(b"/project".to_vec()).unwrap(),
                target: LaunchTarget::Shell,
                rows: 24,
                cols: 80,
            },
            None,
        )
        .unwrap()
}
fn push(fake: &Arc<Mutex<FakeState>>, identity: Identity, request_id: u64, operation: Operation) {
    fake.lock().unwrap().received.push_back(SshEvent::Data(
        encode(&WireEnvelope::new(identity, request_id, operation)).unwrap(),
    ));
}
#[test]
fn malformed_and_foreign_identity_disconnect_without_application() {
    let (client, fake) = client();
    client.request(Operation::Capabilities, None).unwrap();
    let mut foreign = client.identity();
    foreign.epoch += 1;
    let frame = WireEnvelope::new(
        foreign,
        1,
        Operation::Error {
            code: ErrorCode::Io,
        },
    );
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(encode(&frame).unwrap()));
    wait(|| !client.is_connected());
    assert!(client.poll_reply().is_none());
}
#[test]
fn bounded_control_and_terminal_queues_and_no_replay_on_detach() {
    let (client, fake) = client();
    fake.lock().unwrap().blocked = true;
    for _ in 0..MAX_IN_FLIGHT {
        client.request(Operation::Capabilities, None).unwrap();
    }
    assert_eq!(
        client.request(Operation::Capabilities, None),
        Err(ProtocolError::Limit)
    );
    let mut terminal = client.terminal_transport();
    for _ in 0..MAX_QUEUED_CHUNKS {
        terminal.write(b"private-input").unwrap();
    }
    assert!(matches!(
        terminal.write(b"overflow"),
        Err(PtyError::WouldBlock)
    ));
    terminal.detach().unwrap();
    assert!(matches!(terminal.write(b"late"), Err(PtyError::Closed)));
    fake.lock().unwrap().blocked = false;
    wait(|| fake.lock().unwrap().detached);
    assert!(fake.lock().unwrap().sent.is_empty());
    assert!(client.poll_reply().is_none());
}
#[test]
fn cumulative_duplicate_directory_batches_are_rejected() {
    let (client, fake) = client();
    let path = RemotePath::new(b"/project".to_vec()).unwrap();
    let id = client
        .request(Operation::List { path: path.clone() }, None)
        .unwrap();
    let op = Operation::Listed {
        path,
        entries: vec![DirectoryEntry {
            path: RemotePath::new(b"/project/file".to_vec()).unwrap(),
            kind: EntryKind::File,
        }],
        truncated: false,
        done: false,
    };
    let bytes = encode(&WireEnvelope::new(client.identity(), id, op)).unwrap();
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data([bytes.clone(), bytes].concat()));
    wait(|| !client.is_connected());
    assert!(client.poll_reply().is_none());
}
#[test]
fn newer_generation_terminal_output_does_not_drop_the_transport() {
    let (client, fake) = client();
    let mut identity = client.identity();
    identity.generation += 3;
    let frame = WireEnvelope::new(
        identity,
        9,
        Operation::TerminalOutput {
            data: Chunk::new(b"still-live".to_vec()).unwrap(),
            done: false,
        },
    );
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(encode(&frame).unwrap()));
    let mut transport = client.terminal_transport();
    let mut seen = None;
    wait(|| {
        seen = transport.poll();
        seen.is_some()
    });
    assert!(client.is_connected());
    assert!(matches!(seen, Some(made_terminal::PtyEvent::Data(bytes)) if bytes == b"still-live"));
}

#[test]
fn path_outside_root_does_not_invalidate_the_transport() {
    let (client, fake) = client();
    let frame = WireEnvelope::new(
        client.identity(),
        4,
        Operation::Changed {
            path: RemotePath::new(b"/other".to_vec()).unwrap(),
        },
    );
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(encode(&frame).unwrap()));
    thread::sleep(Duration::from_millis(30));
    assert!(client.is_connected());
    assert!(client.poll_reply().is_none());
}

#[test]
fn reply_progress_accumulates_directory_batches_until_done() {
    let (client, fake) = client();
    let path = RemotePath::new(b"/project".to_vec()).unwrap();
    let id = client
        .request(Operation::List { path: path.clone() }, None)
        .unwrap();
    let first = Operation::Listed {
        path: path.clone(),
        entries: vec![DirectoryEntry {
            path: RemotePath::new(b"/project/a".to_vec()).unwrap(),
            kind: EntryKind::File,
        }],
        truncated: false,
        done: false,
    };
    let second = Operation::Listed {
        path,
        entries: vec![DirectoryEntry {
            path: RemotePath::new(b"/project/b".to_vec()).unwrap(),
            kind: EntryKind::File,
        }],
        truncated: false,
        done: true,
    };
    let bytes = [
        encode(&WireEnvelope::new(client.identity(), id, first)).unwrap(),
        encode(&WireEnvelope::new(client.identity(), id, second)).unwrap(),
    ]
    .concat();
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(bytes));
    let mut replies = Vec::new();
    wait(|| {
        while let Some(frame) = client.poll_reply() {
            replies.push(frame);
        }
        replies.len() == 2
    });
    assert!(client.is_connected());
    assert!(matches!(
        replies[1].operation,
        Operation::Listed { done: true, .. }
    ));
}

fn push_output(fake: &Arc<Mutex<FakeState>>, identity: Identity, bytes: &[u8], done: bool) {
    let frame = WireEnvelope::new(
        identity,
        9,
        Operation::TerminalOutput {
            data: Chunk::new(bytes.to_vec()).unwrap(),
            done,
        },
    );
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(encode(&frame).unwrap()));
}

#[test]
fn client_preserves_done_and_empty_poll_is_not_completion() {
    let (client, fake) = client();
    let mut transport = client.terminal_transport();
    assert!(transport.poll_output().is_none());
    assert!(!transport.resnapshot_complete());
    push_output(&fake, client.identity(), b"snap", false);
    let mut first = None;
    wait(|| {
        first = transport.poll_output();
        first.is_some()
    });
    assert_eq!(
        first,
        Some(TerminalFrame {
            bytes: b"snap".to_vec(),
            done: false,
        })
    );
    assert!(!transport.resnapshot_complete());
    assert!(transport.poll_output().is_none());
    assert!(!transport.resnapshot_complete());
    push_output(&fake, client.identity(), b"", true);
    let mut end = None;
    wait(|| {
        end = transport.poll_output();
        end.is_some()
    });
    assert_eq!(
        end,
        Some(TerminalFrame {
            bytes: Vec::new(),
            done: true,
        })
    );
    assert!(transport.resnapshot_complete());
    push_output(&fake, client.identity(), b"live", false);
    let mut live = None;
    wait(|| {
        live = transport.poll_output();
        live.is_some()
    });
    assert_eq!(
        live,
        Some(TerminalFrame {
            bytes: b"live".to_vec(),
            done: false,
        })
    );
    assert!(client.is_connected());
}

#[test]
fn second_done_true_is_not_applied() {
    let (client, fake) = client();
    let mut transport = client.terminal_transport();
    push_output(&fake, client.identity(), b"", true);
    wait(|| transport.resnapshot_complete());
    let _ = transport.poll_output();
    push_output(&fake, client.identity(), b"again", true);
    thread::sleep(Duration::from_millis(40));
    assert!(transport.poll_output().is_none());
    assert!(client.is_connected());
    assert!(transport.resnapshot_complete());
}

#[test]
fn heartbeat_is_not_stuck_behind_a_full_control_queue() {
    assert!(made_remote::client::heartbeat_ready(
        true,
        false,
        MAX_IN_FLIGHT,
        MAX_IN_FLIGHT
    ));
    assert!(!made_remote::client::heartbeat_ready(true, true, 0, 0));
    assert!(!made_remote::client::heartbeat_ready(false, false, 0, 0));
}

#[test]
fn reset_discards_late_correlated_replies_but_keeps_attach_snapshot() {
    let (client, fake) = client();
    let directory = RemotePath::new(b"/project".to_vec()).unwrap();
    let attach_id = client
        .request(
            Operation::Open {
                directory,
                target: LaunchTarget::Shell,
                rows: 24,
                cols: 80,
            },
            None,
        )
        .unwrap();
    fake.lock().unwrap().received.push_back(SshEvent::Data(
        encode(&WireEnvelope::new(
            client.identity(),
            attach_id,
            Operation::Attached,
        ))
        .unwrap(),
    ));
    let mut attached = false;
    wait(|| {
        while let Some(reply) = client.poll_reply() {
            attached |= matches!(reply.operation, Operation::Attached);
        }
        attached
    });

    let session_id = client.request(Operation::SessionState, None).unwrap();
    let git_id = client
        .request(
            Operation::GitRead {
                command: GitRead::Status,
            },
            None,
        )
        .unwrap();
    client.reset_pending().unwrap();

    let late_snapshot = WireEnvelope::new(
        client.identity(),
        session_id,
        Operation::SessionSnapshot {
            snapshot: snapshot(99),
        },
    );
    let late_git = WireEnvelope::new(
        client.identity(),
        git_id,
        Operation::GitResult {
            command: GitRead::Status,
            offset: 0,
            data: Chunk::new(b"late".to_vec()).unwrap(),
            done: true,
        },
    );
    let attach_snapshot = WireEnvelope::new(
        client.identity(),
        attach_id,
        Operation::SessionSnapshot {
            snapshot: snapshot(1),
        },
    );
    {
        let mut fake = fake.lock().unwrap();
        fake.received
            .push_back(SshEvent::Data(encode(&late_snapshot).unwrap()));
        fake.received
            .push_back(SshEvent::Data(encode(&late_git).unwrap()));
        fake.received
            .push_back(SshEvent::Data(encode(&attach_snapshot).unwrap()));
    }

    let mut sink = client.session_sink();
    let mut seen = Vec::new();
    wait(|| {
        while let Some(frame) = sink.poll() {
            seen.push(frame);
        }
        !seen.is_empty()
    });
    assert_eq!(seen.len(), 1);
    assert!(matches!(
        &seen[0].operation,
        Operation::SessionSnapshot { snapshot } if snapshot.revision == 1
    ));
    thread::sleep(Duration::from_millis(30));
    assert!(sink.poll().is_none());
    assert!(
        client.poll_reply().is_none(),
        "late correlated replies must still be discarded"
    );
    assert!(client.is_connected());
}

#[test]
fn current_attach_grok_pushes_reach_the_session_sink_without_consuming_the_attach() {
    let (client, fake) = client();
    let attach_id = attach(&client);
    let identity = client.identity();
    // The push arrives while the attach is still pending: it must reach the
    // owning session sink without being consumed as the attach reply.
    push(
        &fake,
        identity.clone(),
        attach_id,
        grok_event(GrokEventKind::Prompt, "s0"),
    );
    let mut sink = client.session_sink();
    let mut event = None;
    wait(|| {
        event = sink.poll();
        event.is_some()
    });
    assert!(matches!(
        event.map(|frame| frame.operation),
        Some(Operation::GrokEvent { .. })
    ));
    assert!(client.is_connected(), "a push must not consume the attach");
    // The pending attach still completes normally afterwards.
    push(&fake, identity, attach_id, Operation::Attached);
    let mut attached = false;
    wait(|| {
        while let Some(reply) = client.poll_reply() {
            attached |= matches!(reply.operation, Operation::Attached);
        }
        attached
    });
}

#[test]
fn unsolicited_grok_pushes_go_to_the_sink_and_grok_state_replies_stay_correlated() {
    let (client, fake) = client();
    let attach_id = attach(&client);
    push(&fake, client.identity(), attach_id, Operation::Attached);
    wait(|| client.poll_reply().is_some());
    // An unsolicited push rides the current attach id and reaches the sink.
    push(&fake, client.identity(), attach_id, grok_snapshot(7));
    let mut sink = client.session_sink();
    let mut pushed = None;
    wait(|| {
        pushed = sink.poll();
        pushed.is_some()
    });
    assert!(matches!(
        pushed.map(|frame| frame.operation),
        Some(Operation::GrokSnapshot { snapshot }) if snapshot.revision == 7
    ));
    // A reply to a pending GrokState request stays in the generic reply queue
    // where the owning workspace correlation (id map) consumes it; it never
    // appears as an unsolicited session event.
    let state = client.request(Operation::GrokState, None).unwrap();
    push(&fake, client.identity(), state, grok_snapshot(8));
    let mut correlated = None;
    wait(|| {
        while let Some(reply) = client.poll_reply() {
            if reply.request_id == state {
                correlated = Some(reply.operation);
            }
        }
        correlated.is_some()
    });
    assert!(matches!(
        correlated,
        Some(Operation::GrokSnapshot { snapshot }) if snapshot.revision == 8
    ));
    assert!(sink.poll().is_none(), "correlated replies own the id map");
}

#[test]
fn after_reset_only_current_attach_grok_pushes_bypass_the_discard_window() {
    let (client, fake) = client();
    let attach_id = attach(&client);
    push(&fake, client.identity(), attach_id, Operation::Attached);
    wait(|| client.poll_reply().is_some());
    let session = client.request(Operation::SessionState, None).unwrap();
    let grok_state = client.request(Operation::GrokState, None).unwrap();
    client.reset_pending().unwrap();
    // Late correlated GrokState replies and old session snapshots stay
    // discarded.
    push(&fake, client.identity(), grok_state, grok_snapshot(9));
    push(
        &fake,
        client.identity(),
        session,
        Operation::SessionSnapshot {
            snapshot: snapshot(99),
        },
    );
    // The current attach remains the authorized source for Grok pushes.
    push(
        &fake,
        client.identity(),
        attach_id,
        grok_event(GrokEventKind::SessionStart, "s1"),
    );
    let mut sink = client.session_sink();
    let mut seen = Vec::new();
    wait(|| {
        while let Some(frame) = sink.poll() {
            seen.push(frame);
        }
        !seen.is_empty()
    });
    assert_eq!(seen.len(), 1, "only the current-attach push survives");
    assert!(matches!(
        &seen[0].operation,
        Operation::GrokEvent { event } if event.session_id == "s1"
    ));
    thread::sleep(Duration::from_millis(30));
    assert!(sink.poll().is_none());
    assert!(
        client.poll_reply().is_none(),
        "late correlated replies must still be discarded"
    );
    assert!(client.is_connected());
}

#[test]
fn new_attach_replaces_the_grok_source_and_failure_clears_it() {
    let (client, fake) = client();
    let first = attach(&client);
    push(&fake, client.identity(), first, Operation::Attached);
    wait(|| client.poll_reply().is_some());
    // A failed attach clears the authorized source: a later push riding that
    // id is discarded, not delivered.
    let failed = client
        .request(Operation::Attach { rows: 24, cols: 80 }, None)
        .unwrap();
    push(
        &fake,
        client.identity(),
        failed,
        Operation::Error {
            code: ErrorCode::Unavailable,
        },
    );
    let mut failure_seen = false;
    wait(|| {
        while let Some(reply) = client.poll_reply() {
            failure_seen |= reply.request_id == failed;
        }
        failure_seen
    });
    client.reset_pending().unwrap();
    push(
        &fake,
        client.identity(),
        failed,
        grok_event(GrokEventKind::Prompt, "late"),
    );
    let mut sink = client.session_sink();
    thread::sleep(Duration::from_millis(40));
    assert!(
        sink.poll().is_none(),
        "a failed attach must not remain an authorized Grok source"
    );
    // A new attach replaces the source: only its id delivers after the reset.
    let second = client
        .request(Operation::Attach { rows: 24, cols: 80 }, None)
        .unwrap();
    push(&fake, client.identity(), second, Operation::Attached);
    wait(|| client.poll_reply().is_some());
    client.reset_pending().unwrap();
    push(
        &fake,
        client.identity(),
        first,
        grok_event(GrokEventKind::Prompt, "old"),
    );
    push(
        &fake,
        client.identity(),
        second,
        grok_event(GrokEventKind::Prompt, "new"),
    );
    let mut seen = Vec::new();
    wait(|| {
        while let Some(frame) = sink.poll() {
            seen.push(frame);
        }
        !seen.is_empty()
    });
    assert_eq!(seen.len(), 1);
    assert!(matches!(
        &seen[0].operation,
        Operation::GrokEvent { event } if event.session_id == "new"
    ));
}

#[test]
fn root_port_never_consults_host_filesystem() {
    let (client, fake) = client();
    let mut root = client.root_resolver();
    let path = RemotePath::new(b"/certainly-not-a-host-directory".to_vec()).unwrap();
    root.resolve(RootRequest {
        identity: client.identity(),
        request_id: 1,
        path: path.clone(),
        select: true,
    })
    .unwrap();
    let result = WireEnvelope::new(
        client.identity(),
        1,
        Operation::RootResult {
            requested: path.clone(),
            path: path.clone(),
            generation: 2,
        },
    );
    fake.lock()
        .unwrap()
        .received
        .push_back(SshEvent::Data(encode(&result).unwrap()));
    let mut received = None;
    wait(|| {
        received = root.poll();
        received.is_some()
    });
    assert_eq!(received.unwrap().result, Ok(path));
}
