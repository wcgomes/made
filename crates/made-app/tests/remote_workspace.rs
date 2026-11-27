//! Pure controller/fake-transport evidence; not real SSH/TUI acceptance.
use made_app::remote_workspace::*;
use made_remote::protocol::{
    AuthorityId, Chunk, ClaudeActivity, ClaudeBackgroundEntry, ClaudeListingResult,
    ClaudeSessionInfo, ClaudeSessionSnapshot, DirectoryEntry, EntryKind, ErrorCode, GitRead,
    GrokActivity, GrokEventKind, GrokSessionEvent, GrokSessionInfo, GrokSessionSnapshot, Identity,
    Operation, PreviewKind, RemotePath, TabCapability, WireEnvelope, MAX_IN_FLIGHT,
};

fn path(value: &str) -> RemotePath {
    RemotePath::new(value.as_bytes().to_vec()).unwrap()
}
fn model(authority: u8) -> RemoteWorkspace {
    RemoteWorkspace::new(
        Identity {
            authority: AuthorityId([authority; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        path("/project"),
    )
    .unwrap()
}

#[test]
fn replies_are_scoped_and_partial_preview_is_not_published() {
    let mut a = model(1);
    let mut b = model(2);
    let req = a
        .request(
            Operation::Preview {
                path: path("/project/file"),
            },
            None,
        )
        .unwrap();
    let response = |identity, offset, bytes: &[u8], done| {
        WireEnvelope::new(
            identity,
            req.request_id,
            Operation::PreviewResult {
                path: path("/project/file"),
                kind: PreviewKind::Text,
                offset,
                data: Chunk::new(bytes.to_vec()).unwrap(),
                done,
            },
        )
    };
    assert!(b
        .accept(response(req.identity.clone(), 0, b"x", true))
        .is_err());
    assert_eq!(
        a.accept(response(req.identity.clone(), 0, b"a", false))
            .unwrap(),
        None
    );
    assert!(
        matches!(a.accept(response(req.identity.clone(), 1, b"b", true)).unwrap(), Some(RemoteResult::Preview { data, .. }) if data == b"ab")
    );
    assert!(a
        .accept(response(req.identity.clone(), 0, b"x", true))
        .is_err());
}

fn reply(request: &WireEnvelope, operation: Operation) -> WireEnvelope {
    WireEnvelope::new(request.identity.clone(), request.request_id, operation)
}
fn empty_snapshot(revision: u64) -> made_bridge::state::SessionSnapshot {
    made_bridge::state::SessionSnapshot {
        revision,
        active_session_id: None,
        tabs_enabled: true,
        view_directory: None,
        sessions: vec![],
    }
}
fn active_snapshot(revision: u64, directory: &str) -> made_bridge::state::SessionSnapshot {
    serde_json::from_value(serde_json::json!({
        "revision": revision, "activeSessionID": "one", "tabsEnabled": true,
        "sessions": [{ "sessionID": "one", "title": "Remote", "active": true, "directory": directory }]
    })).unwrap()
}
/// Protocol responder spy: recognizes only remote protocol verbs. There is no
/// host path resolver/executable fallback in this controller's dependency seam.
fn settle(
    workspace: &mut RemoteWorkspace,
    snapshot: &made_bridge::state::SessionSnapshot,
) -> Vec<WireEnvelope> {
    settle_with_grok(workspace, Some(snapshot), None)
}

/// Protocol responder spy with an optional Grok inventory. `None` OpenCode
/// consumes SessionState requests without replying (integration stays down);
/// `None` Grok answers GrokState with an honest error.
fn settle_with_grok(
    workspace: &mut RemoteWorkspace,
    snapshot: Option<&made_bridge::state::SessionSnapshot>,
    grok: Option<&GrokSessionSnapshot>,
) -> Vec<WireEnvelope> {
    let mut observed = Vec::new();
    for _ in 0..100 {
        let Some(request) = workspace.next_command() else {
            return observed;
        };
        let operation = match &request.operation {
            Operation::RootValidate { path } => Operation::RootResult {
                requested: path.clone(),
                path: path.clone(),
                generation: request.identity.generation,
            },
            Operation::RootSelect { path } => Operation::RootResult {
                requested: path.clone(),
                path: path.clone(),
                generation: request.identity.generation + 1,
            },
            Operation::List { path } => Operation::Listed {
                path: path.clone(),
                entries: vec![],
                truncated: false,
                done: true,
            },
            Operation::GitRead { .. } => Operation::Error {
                code: ErrorCode::Unavailable,
            },
            Operation::SessionState => match snapshot {
                Some(snapshot) => Operation::SessionSnapshot {
                    snapshot: snapshot.clone(),
                },
                None => {
                    observed.push(request);
                    continue;
                }
            },
            Operation::GrokState => match grok {
                Some(snapshot) => Operation::GrokSnapshot {
                    snapshot: snapshot.clone(),
                },
                None => Operation::Error {
                    code: ErrorCode::Unavailable,
                },
            },
            Operation::Watch { path } => Operation::Changed { path: path.clone() },
            Operation::Cancel { .. } => {
                observed.push(request);
                continue;
            }
            _ => panic!("unexpected remote verb"),
        };
        workspace.accept(reply(&request, operation)).unwrap();
        observed.push(request);
    }
    panic!("unbounded command feedback")
}

fn grok_session(
    id: &str,
    cwd: Option<&str>,
    opened_at: u64,
    activity: GrokActivity,
) -> GrokSessionInfo {
    GrokSessionInfo {
        session_id: id.to_owned(),
        cwd: cwd.map(path),
        opened_at,
        activity,
    }
}
fn grok_snapshot(
    revision: u64,
    running: bool,
    sessions: Vec<GrokSessionInfo>,
) -> GrokSessionSnapshot {
    GrokSessionSnapshot {
        revision,
        running,
        hooks_ready: false,
        sessions,
    }
}
fn grok_event_envelope(
    identity: &Identity,
    request_id: u64,
    kind: GrokEventKind,
    session_id: &str,
    cwd: Option<&str>,
    activity: GrokActivity,
) -> WireEnvelope {
    WireEnvelope::new(
        identity.clone(),
        request_id,
        Operation::GrokEvent {
            event: GrokSessionEvent {
                kind,
                session_id: session_id.to_owned(),
                cwd: cwd.map(path),
                activity,
            },
        },
    )
}

#[test]
fn poisoned_stream_duplicate_cross_batch_and_forged_root_are_rejected() {
    let mut workspace = model(1);
    let req = workspace
        .request(
            Operation::List {
                path: path("/project"),
            },
            None,
        )
        .unwrap();
    let batch = |done| {
        reply(
            &req,
            Operation::Listed {
                path: path("/project"),
                entries: vec![DirectoryEntry {
                    path: path("/project/a"),
                    kind: EntryKind::File,
                }],
                truncated: false,
                done,
            },
        )
    };
    assert_eq!(workspace.accept(batch(false)).unwrap(), None);
    assert!(workspace.accept(batch(true)).is_err());
    assert!(workspace.projection().directories.is_empty());
    assert_eq!(workspace.pending_count(), 0);
    let req = workspace
        .request(
            Operation::RootSelect {
                path: path("/chosen"),
            },
            Some(&path("/chosen")),
        )
        .unwrap();
    assert!(workspace
        .accept(reply(
            &req,
            Operation::RootResult {
                requested: path("/chosen"),
                path: path("/arbitrary"),
                generation: 2
            }
        ))
        .is_err());
    assert_eq!(workspace.root(), &path("/project"));
}

#[test]
fn split_utf8_preview_is_complete_only_after_final_reply_and_poison_is_terminal() {
    let mut workspace = model(1);
    workspace.open_preview(path("/project/file")).unwrap();
    let req = workspace.next_command().unwrap();
    let preview = |offset, bytes: &[u8], done| {
        reply(
            &req,
            Operation::PreviewResult {
                path: path("/project/file"),
                kind: PreviewKind::Text,
                offset,
                data: Chunk::new(bytes.to_vec()).unwrap(),
                done,
            },
        )
    };
    workspace.accept(preview(0, &[0xe2], false)).unwrap();
    assert!(workspace
        .projection()
        .preview
        .as_ref()
        .unwrap()
        .data
        .is_empty());
    workspace.accept(preview(1, &[0x82, 0xac], true)).unwrap();
    assert_eq!(
        workspace.projection().preview.as_ref().unwrap().data,
        "€".as_bytes()
    );
    workspace.refresh_preview().unwrap();
    let refresh = workspace.next_command().unwrap();
    assert!(workspace
        .accept(reply(
            &refresh,
            Operation::PreviewResult {
                path: path("/project/file"),
                kind: PreviewKind::Text,
                offset: 1,
                data: Chunk::new(b"x".to_vec()).unwrap(),
                done: true
            }
        ))
        .is_err());
    assert!(workspace
        .accept(reply(
            &refresh,
            Operation::PreviewResult {
                path: path("/project/file"),
                kind: PreviewKind::Text,
                offset: 0,
                data: Chunk::new(b"x".to_vec()).unwrap(),
                done: true
            }
        ))
        .is_err());
}

#[test]
fn root_precedence_shell_keeps_up_while_session_wins_and_focus_is_correlated() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/shell-a"));
    workspace.refresh_sessions().unwrap();
    settle(&mut workspace, &active_snapshot(2, "/session"));
    assert_eq!(workspace.root(), &path("/session"));
    assert_eq!(workspace.projection().source, RootSource::Session);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-c"))
        .unwrap();
    settle(&mut workspace, &active_snapshot(2, "/session"));
    assert_eq!(workspace.root(), &path("/session"));
    assert!(workspace.focus_session("foreign-session").is_err());
    workspace.focus_session("one").unwrap();
    let focus = workspace.next_command().unwrap();
    assert!(workspace
        .accept(reply(
            &focus,
            Operation::FocusResult {
                session_id: "wrong".into(),
                accepted: true
            }
        ))
        .is_err());
    workspace.focus_session("one").unwrap();
    let focus = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &focus,
            Operation::FocusResult {
                session_id: "one".into(),
                accepted: true,
            },
        ))
        .unwrap();
    assert_eq!(workspace.projection().focus, Some(("one".into(), true)));
    workspace.refresh_sessions().unwrap();
    let mut view = empty_snapshot(3);
    view.view_directory = Some("/view".into());
    settle(&mut workspace, &view);
    assert_eq!(workspace.root(), &path("/view"));
    let identity = workspace.identity().clone();
    workspace
        .integration_changed(&identity, IntegrationState::Unavailable)
        .unwrap();
    // No session state is available, so the independent shell root wins.
    assert_eq!(workspace.connection(), ConnectionState::Connected);
    let select = workspace.next_command().unwrap();
    assert!(
        matches!(select.operation, Operation::RootSelect { ref path } if path == &self::path("/shell-c"))
    );
}

#[test]
fn unavailable_active_root_does_not_expose_unrelated_root_or_disable_transport() {
    let mut workspace = model(1);
    workspace.refresh_sessions().unwrap();
    let state = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &state,
            Operation::SessionSnapshot {
                snapshot: active_snapshot(1, "/missing"),
            },
        ))
        .unwrap();
    let root = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &root,
            Operation::Error {
                code: ErrorCode::Unavailable,
            },
        ))
        .unwrap();
    assert_eq!(
        workspace.projection().root_state,
        Freshness::Unavailable(ErrorCode::Unavailable)
    );
    assert!(workspace.projection().directories.is_empty());
    assert_eq!(workspace.connection(), ConnectionState::Connected);
    assert!(workspace.next_command().is_none());
}

#[test]
fn unsolicited_snapshot_events_require_current_identity_and_revision() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    let event = |identity: Identity, revision: u64| {
        WireEnvelope::new(
            identity,
            1,
            Operation::SessionSnapshot {
                snapshot: empty_snapshot(revision),
            },
        )
    };
    let mut foreign = identity.clone();
    foreign.epoch = 2;
    assert!(workspace.accept_session_event(event(foreign, 1)).is_err());
    assert_eq!(workspace.integration(), IntegrationState::Unknown);
    assert_eq!(workspace.pending_count(), 0);

    workspace
        .accept_session_event(event(identity.clone(), 2))
        .unwrap();
    assert_eq!(workspace.integration(), IntegrationState::Connected);
    assert_eq!(
        workspace.projection().sessions.as_ref().unwrap().revision,
        2
    );

    assert!(workspace
        .accept_session_event(event(identity.clone(), 1))
        .is_err());
    assert_eq!(
        workspace.projection().sessions.as_ref().unwrap().revision,
        2,
        "a stale event must not overwrite the accepted snapshot"
    );

    workspace.disconnect();
    assert!(workspace.accept_session_event(event(identity, 3)).is_err());
}

#[test]
fn new_root_clears_preview_and_rejects_pending_generation() {
    let mut workspace = model(1);
    workspace.open_preview(path("/project/a")).unwrap();
    let old_preview = workspace.next_command().unwrap();
    workspace.select_root(path("/next")).unwrap();
    assert!(workspace.projection().preview.is_none());
    assert!(workspace
        .accept(reply(
            &old_preview,
            Operation::PreviewResult {
                path: path("/project/a"),
                kind: PreviewKind::Text,
                offset: 0,
                data: Chunk::new(b"old".to_vec()).unwrap(),
                done: true
            }
        ))
        .is_err());
    settle(&mut workspace, &active_snapshot(1, "/next"));
    assert_eq!(workspace.root(), &path("/next"));
    assert!(workspace.identity().generation > old_preview.identity.generation);
}

#[test]
fn reattach_resnapshots_before_current_and_never_replays_focus_or_preview() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert!(!workspace.resnapshot_pending());
    workspace.open_preview(path("/project/a")).unwrap();
    let old_preview = workspace.next_command().unwrap();
    workspace.disconnect();
    assert!(workspace.next_command().is_none());
    assert_eq!(
        workspace.projection().preview.as_ref().unwrap().freshness,
        Freshness::Stale
    );
    workspace.reattach(2).unwrap();
    assert!(workspace.resnapshot_pending());
    assert!(workspace.projection().preview.is_none());
    let commands = settle(&mut workspace, &empty_snapshot(1));
    assert!(commands
        .iter()
        .any(|c| matches!(c.operation, Operation::RootValidate { .. })));
    assert!(commands
        .iter()
        .any(|c| matches!(c.operation, Operation::List { .. })));
    assert!(commands
        .iter()
        .any(|c| matches!(c.operation, Operation::SessionState)));
    assert!(commands.iter().all(|c| !matches!(
        c.operation,
        Operation::Focus { .. }
            | Operation::Preview { .. }
            | Operation::TerminalInput { .. }
            | Operation::UploadBegin { .. }
    )));
    assert!(!workspace.resnapshot_pending());
    assert!(workspace
        .accept(reply(
            &old_preview,
            Operation::Error {
                code: ErrorCode::Io
            }
        ))
        .is_err());
}

#[test]
fn same_path_two_authorities_and_two_same_server_tabs_do_not_alias() {
    let mut a = model(1);
    let mut b = model(2);
    let mut sibling = model(1);
    for (workspace, name) in [
        (&mut a, "/project/a"),
        (&mut b, "/project/b"),
        (&mut sibling, "/project/c"),
    ] {
        workspace.load_directory(path("/project")).unwrap();
        let request = workspace.next_command().unwrap();
        workspace
            .accept(reply(
                &request,
                Operation::Listed {
                    path: path("/project"),
                    entries: vec![DirectoryEntry {
                        path: path(name),
                        kind: EntryKind::File,
                    }],
                    truncated: false,
                    done: true,
                },
            ))
            .unwrap();
    }
    assert_eq!(
        a.projection().directories[&path("/project")].entries[0].path,
        path("/project/a")
    );
    assert_eq!(
        b.projection().directories[&path("/project")].entries[0].path,
        path("/project/b")
    );
    a.disconnect();
    assert_eq!(
        sibling.projection().directories[&path("/project")].freshness,
        Freshness::Current
    );
}

#[test]
fn acknowledged_watch_is_required_and_changes_coalesce_until_reconcile() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    assert!(workspace
        .filesystem_changed(&identity, &path("/project"))
        .is_err());
    workspace.begin_resnapshot().unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    let identity = workspace.identity().clone();
    for _ in 0..1000 {
        workspace
            .filesystem_changed(&identity, &path("/project"))
            .unwrap();
    }
    assert_eq!(workspace.pending_count(), 0);
    assert!(workspace.next_command().is_none());
    workspace.reconcile().unwrap();
    assert!(workspace.pending_count() <= MAX_IN_FLIGHT);
    let mut count = 0;
    while workspace.next_command().is_some() {
        count += 1;
    }
    assert!(count <= MAX_IN_FLIGHT);
    assert!(workspace
        .filesystem_changed(&identity, &path("/outside"))
        .is_err());
}

#[test]
fn git_grouping_uses_remote_bytes_and_filters_subroot_ignored_and_traversal() {
    let mut workspace = model(1);
    workspace.refresh_git().unwrap();
    while let Some(request) = workspace.next_command() {
        let Operation::GitRead { command } = request.operation else {
            panic!()
        };
        let data: &[u8] = match command {
            GitRead::ShowToplevel => b"/project\n",
            GitRead::Branch => b"HEAD\n",
            GitRead::GitDir => b"/project/.git/worktrees/linked\n",
            GitRead::CommonDir => b"/project/.git\n",
            GitRead::CommitId => b"12345678\n",
            GitRead::CommitSubject => b"A commit\n",
            GitRead::WorktreeList => b"",
            GitRead::Status => b"MM a\0UU conflict\0?? raw-\xff\0!! ignored\0 M ../escape\0",
        };
        workspace
            .accept(reply(
                &request,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(data.to_vec()).unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
    let git = workspace.projection().git_view.as_ref().unwrap();
    assert!(git.detached);
    assert!(git.linked_worktree);
    assert_eq!(git.commit.as_deref(), Some("12345678"));
    assert_eq!(
        git.groups.iter().map(|g| g.title).collect::<Vec<_>>(),
        ["Merge Changes", "Staged Changes", "Changes", "Untracked"]
    );
    assert_eq!(git.groups[3].rows[0].path.as_bytes(), b"/project/raw-\xff");
    assert!(git
        .groups
        .iter()
        .flat_map(|g| &g.rows)
        .all(|row| row.path.is_within(&path("/project"))));
}

#[test]
fn close_deadline_and_offline_close_are_honest_and_do_not_reattach() {
    let mut workspace = model(1);
    workspace.begin_close().unwrap();
    assert_eq!(workspace.close_state(), CloseState::Waiting);
    workspace
        .expire(std::time::Instant::now() + std::time::Duration::from_secs(3))
        .unwrap();
    assert_eq!(workspace.close_state(), CloseState::Unconfirmed);
    assert!(workspace.projection().cleanup_unconfirmed);
    assert!(workspace.reattach(2).is_err());
    let mut offline = model(2);
    offline.disconnect();
    offline.begin_close().unwrap();
    assert_eq!(offline.close_state(), CloseState::Unconfirmed);
    assert!(offline.next_command().is_none());
}

#[test]
fn remote_controller_has_no_host_io_or_native_path_resolution_dependency() {
    let source = include_str!("../src/remote_workspace.rs");
    for forbidden in [
        "std::fs",
        "std::process",
        ".canonicalize(",
        "PathBuf::from(",
        "GitRunner",
    ] {
        assert!(
            !source.contains(forbidden),
            "host capability introduced: {forbidden}"
        );
    }
    let host_path_new = source
        .match_indices("Path::new(")
        .any(|(index, _)| !source[..index].ends_with("Remote"));
    assert!(
        !host_path_new,
        "host Path::new must not interpret a remote path"
    );
}

#[test]
fn disconnect_discards_pending_and_old_epoch_cannot_apply_after_reattach() {
    let mut workspace = model(1);
    let request = workspace
        .request(
            Operation::List {
                path: path("/project"),
            },
            None,
        )
        .unwrap();
    workspace.disconnect();
    assert_eq!(workspace.connection(), ConnectionState::Disconnected);
    assert_eq!(workspace.pending_count(), 0);
    assert!(workspace.reattach(1).is_err());
    workspace.reattach(2).unwrap();
    let reply = WireEnvelope::new(
        request.identity,
        request.request_id,
        Operation::Listed {
            path: path("/project"),
            entries: vec![],
            truncated: false,
            done: true,
        },
    );
    assert!(workspace.accept(reply).is_err());
}

#[test]
fn bounded_requests_and_outside_root_require_explicit_grant() {
    let mut workspace = model(1);
    assert!(workspace
        .request(
            Operation::List {
                path: path("/elsewhere")
            },
            None
        )
        .is_err());
    assert!(workspace
        .request(
            Operation::RootSelect {
                path: path("/elsewhere")
            },
            None
        )
        .is_err());
    for _ in 0..MAX_IN_FLIGHT {
        workspace.request(Operation::SessionState, None).unwrap();
    }
    assert!(workspace.request(Operation::SessionState, None).is_err());
}

#[test]
fn root_port_resolution_updates_the_workspace_generation() {
    let mut workspace = model(3);
    let id = workspace.select_root(path("/project/src")).unwrap();
    assert_eq!(
        workspace.projection().requested_root.as_ref(),
        Some(&path("/project/src"))
    );
    let command = workspace.next_command().unwrap();
    assert_eq!(command.request_id, id);
    let mut identity = command.identity.clone();
    identity.generation = 2;
    let resolution = made_remote::protocol::RootResolution {
        identity,
        request_id: id,
        result: Ok(path("/project/src")),
        generation: 2,
    };
    let result = workspace.accept_root(resolution).unwrap();
    assert!(matches!(result, Some(RemoteResult::Root(root)) if root == path("/project/src")));
    assert_eq!(workspace.identity().generation, 2);
    assert_eq!(workspace.root(), &path("/project/src"));
    assert!(workspace.projection().requested_root.is_none());
    let mut listed = false;
    let mut git = false;
    while let Some(request) = workspace.next_command() {
        match request.operation {
            Operation::List { path: listed_path } => {
                listed |= listed_path == path("/project/src");
            }
            Operation::GitRead { .. } => git = true,
            _ => {}
        }
    }
    assert!(listed, "accepted root must list the new root");
    assert!(git, "accepted root must refresh Git");
}

fn accept_git(workspace: &mut RemoteWorkspace, status: Result<&[u8], ErrorCode>) {
    for _ in 0..8 {
        let Some(request) = workspace.next_command() else {
            break;
        };
        let Operation::GitRead { command } = request.operation else {
            panic!("expected GitRead");
        };
        let data: &[u8] = match command {
            GitRead::ShowToplevel => b"/project\n",
            GitRead::Branch => b"main\n",
            GitRead::GitDir => b"/project/.git\n",
            GitRead::CommonDir => b"/project/.git\n",
            GitRead::WorktreeList => b"",
            GitRead::CommitId => b"12345678\n",
            GitRead::CommitSubject => b"Subject\n",
            GitRead::Status => match status {
                Ok(data) => data,
                Err(code) => {
                    workspace
                        .accept(reply(&request, Operation::Error { code }))
                        .unwrap();
                    continue;
                }
            },
        };
        workspace
            .accept(reply(
                &request,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(data.to_vec()).unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
}

#[test]
fn disconnect_clears_a_pending_requested_root() {
    let mut workspace = model(1);
    workspace.select_root(path("/project/next")).unwrap();
    assert!(workspace.projection().requested_root.is_some());
    workspace.disconnect();
    assert!(workspace.projection().requested_root.is_none());
}

#[test]
fn git_failure_keeps_the_last_complete_view_and_marks_missing_status() {
    let mut workspace = model(1);
    workspace.refresh_git().unwrap();
    accept_git(&mut workspace, Ok(b" M file\0"));
    let view = workspace.projection().git_view.as_ref().unwrap();
    assert!(view.has_status);
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].rows.len(), 1);

    workspace.refresh_git().unwrap();
    accept_git(&mut workspace, Err(ErrorCode::Unavailable));
    let view = workspace.projection().git_view.as_ref().unwrap();
    assert!(view.has_status);
    assert_eq!(view.branch.as_deref(), Some("main"));
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].rows.len(), 1);

    let mut fresh = model(2);
    fresh.refresh_git().unwrap();
    accept_git(&mut fresh, Err(ErrorCode::Unavailable));
    let view = fresh.projection().git_view.as_ref().unwrap();
    assert!(!view.has_status);
    assert!(view.groups.is_empty());
    assert_eq!(
        view.freshness,
        Freshness::Unavailable(ErrorCode::Unavailable)
    );
}

#[test]
fn entering_git_refreshes_even_when_current_and_dedupes_in_flight() {
    let mut workspace = model(1);
    workspace.refresh_git().unwrap();
    assert_eq!(workspace.projection().git.len(), 8);
    // A second entry while the first refresh is in flight queues nothing.
    workspace.refresh_git_if_idle().unwrap();
    assert_eq!(workspace.projection().git.len(), 8);

    accept_git(&mut workspace, Ok(b""));
    assert_eq!(
        workspace.projection().git_view.as_ref().unwrap().freshness,
        Freshness::Current
    );
    workspace.refresh_git_if_idle().unwrap();
    let request = workspace.next_command().expect("fresh GitRead");
    assert!(matches!(request.operation, Operation::GitRead { .. }));
    assert!(workspace
        .projection()
        .git
        .iter()
        .any(|git| git.freshness == Freshness::Loading));
}

#[test]
fn git_selection_is_independent_and_preview_does_not_move_tree_selection() {
    let mut workspace = model(1);
    let tree = path("/project/tree");
    let git = path("/project/git");
    let preview = path("/project/preview");
    workspace.select(tree.clone()).unwrap();
    workspace.select_git(git.clone()).unwrap();
    assert_eq!(workspace.projection().selected.as_ref(), Some(&tree));
    assert_eq!(workspace.projection().git_selected.as_ref(), Some(&git));
    workspace.open_preview(preview).unwrap();
    assert_eq!(workspace.projection().selected.as_ref(), Some(&tree));
    assert_eq!(workspace.projection().git_selected.as_ref(), Some(&git));
}

#[test]
fn root_listing_error_closes_the_open_preview() {
    let mut workspace = model(1);
    workspace.open_preview(path("/project/file")).unwrap();
    workspace.load_directory(path("/project")).unwrap();
    let request = loop {
        let request = workspace.next_command().expect("list request");
        if matches!(request.operation, Operation::List { .. }) {
            break request;
        }
    };
    workspace
        .accept(reply(
            &request,
            Operation::Error {
                code: ErrorCode::Unavailable,
            },
        ))
        .unwrap();
    assert!(workspace.projection().preview.is_none());
    assert_eq!(
        workspace
            .projection()
            .directories
            .get(&path("/project"))
            .unwrap()
            .freshness,
        Freshness::Unavailable(ErrorCode::Unavailable)
    );
}

#[test]
fn full_pending_queue_transition_drops_old_work_without_cancel_frames() {
    let mut workspace = model(1);
    workspace.refresh_git().unwrap();
    let id = workspace.select_root(path("/project/next")).unwrap();
    let mut commands = Vec::new();
    while let Some(command) = workspace.next_command() {
        commands.push(command);
    }
    assert_eq!(commands.len(), 1, "old work must be dropped silently");
    assert_eq!(commands[0].request_id, id);
    assert!(matches!(
        commands[0].operation,
        Operation::RootSelect { path: ref selected } if selected == &path("/project/next")
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command.operation, Operation::Cancel { .. })),
        "transition must not enqueue one Cancel per old pending request"
    );
}

#[test]
fn grok_push_never_consumes_pending_work_even_by_numeric_id() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    // One pending List whose request id the push will numerically reuse.
    let list_id = workspace.load_directory(path("/project")).unwrap();
    let pending_before = workspace.pending_count();

    let identity = workspace.identity().clone();
    workspace
        .accept_grok_push(WireEnvelope::new(
            identity,
            list_id,
            Operation::GrokSnapshot {
                snapshot: grok_snapshot(1, true, vec![]),
            },
        ))
        .expect("a current-attach Grok push applies without a workspace mapping");
    assert!(workspace.projection().grok.is_some());
    assert_eq!(
        workspace.pending_count(),
        pending_before,
        "a Grok push must not consume or poison a numeric-equal pending request"
    );
    // The pending List itself is still the queued correlated request.
    let request = workspace.next_command().expect("the List stays queued");
    assert_eq!(request.request_id, list_id);
    assert!(matches!(request.operation, Operation::List { .. }));

    // The correlated reply for that pending request still applies afterwards.
    let reply = WireEnvelope::new(
        request.identity.clone(),
        list_id,
        Operation::Listed {
            path: path("/project"),
            entries: vec![],
            truncated: false,
            done: true,
        },
    );
    workspace
        .accept(reply)
        .expect("the correlated List reply applies");
    assert!(
        workspace.pending_count() < pending_before,
        "the answered request must complete normally"
    );
}

#[test]
fn grok_hook_tier_comes_only_from_a_current_snapshot_or_hook_event() {
    let mut workspace = model(1);
    assert_eq!(workspace.grok_hooks_ready(), None);

    // An authenticated idle event proves the hook tier even before the first
    // snapshot; no non-idle activity inference is needed.
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            1,
            GrokEventKind::SessionStart,
            "idle",
            None,
            GrokActivity::Idle,
        ))
        .unwrap();
    assert_eq!(workspace.grok_hooks_ready(), Some(true));

    workspace.begin_resnapshot().unwrap();
    assert_eq!(workspace.grok_hooks_ready(), None);
    let identity = workspace.identity().clone();
    workspace
        .accept_grok_push(WireEnvelope::new(
            identity.clone(),
            2,
            Operation::GrokSnapshot {
                snapshot: grok_snapshot(1, true, vec![]),
            },
        ))
        .unwrap();
    assert_eq!(workspace.grok_hooks_ready(), Some(false));

    // An equal revision cannot replace the already accepted tier.
    let mut duplicate = grok_snapshot(1, true, vec![]);
    duplicate.hooks_ready = true;
    workspace
        .accept_grok_push(WireEnvelope::new(
            identity.clone(),
            3,
            Operation::GrokSnapshot {
                snapshot: duplicate,
            },
        ))
        .unwrap();
    assert_eq!(workspace.grok_hooks_ready(), Some(false));

    let mut ready = grok_snapshot(2, true, vec![]);
    ready.hooks_ready = true;
    workspace
        .accept_grok_push(WireEnvelope::new(
            identity,
            4,
            Operation::GrokSnapshot { snapshot: ready },
        ))
        .unwrap();
    assert_eq!(workspace.grok_hooks_ready(), Some(true));
    workspace.disconnect();
    assert_eq!(workspace.grok_hooks_ready(), None);
}

#[test]
fn grok_root_sits_below_opencode_and_above_shell_with_honest_lifecycle() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/shell-a"));

    // A living sessionless Grok process never moves the root.
    let sessionless = grok_snapshot(1, true, vec![]);
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&sessionless));
    assert_eq!(workspace.root(), &path("/shell-a"));
    let state = workspace.projection().grok.as_ref().unwrap();
    assert!(state.running && state.sessions.is_empty());

    // The owned session's cwd wins over the validated shell cwd.
    let inventory = grok_snapshot(
        2,
        true,
        vec![grok_session("a", Some("/grok-a"), 7, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));
    assert_eq!(workspace.projection().source, RootSource::Grok);

    // A connected OpenCode session stays authoritative above Grok.
    workspace.refresh_sessions().unwrap();
    settle_with_grok(
        &mut workspace,
        Some(&active_snapshot(2, "/session")),
        Some(&inventory),
    );
    assert_eq!(workspace.root(), &path("/session"));
    assert_eq!(workspace.projection().source, RootSource::Session);

    // OpenCode yields; the Grok slot retakes the root above the shell.
    let identity = workspace.identity().clone();
    workspace
        .integration_changed(&identity, IntegrationState::Unavailable)
        .unwrap();
    settle_with_grok(&mut workspace, None, None);
    assert_eq!(workspace.root(), &path("/grok-a"));
    assert_eq!(workspace.projection().source, RootSource::Grok);

    // The last owned session ends; the shell cwd is restored honestly.
    workspace.refresh_grok().unwrap();
    let ended = grok_snapshot(3, false, vec![]);
    settle_with_grok(&mut workspace, None, Some(&ended));
    assert_eq!(workspace.root(), &path("/shell-a"));
    assert_eq!(workspace.projection().source, RootSource::Shell);
    assert!(workspace
        .projection()
        .grok
        .as_ref()
        .is_some_and(|state| state.sessions.is_empty()));
}

#[test]
fn grok_newest_attachment_wins_prompts_move_and_inventory_does_not_steal_back() {
    let mut workspace = model(1);
    let inventory = grok_snapshot(
        2,
        true,
        vec![
            grok_session("older", Some("/older"), 1, GrokActivity::Idle),
            grok_session("newer", Some("/newer"), 2, GrokActivity::Idle),
            grok_session("attached", Some("/attached"), 3, GrokActivity::Idle),
        ],
    );
    // The newest attachment moves the explorer (A).
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/attached"));

    // A prompt in the older owned session moves immediately (B)...
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            9,
            GrokEventKind::Prompt,
            "older",
            Some("/older"),
            GrokActivity::Working,
        ))
        .unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.root(), &path("/older"));
    assert_eq!(workspace.projection().source, RootSource::Grok);
    assert_eq!(
        workspace.projection().grok.as_ref().unwrap().sessions[0].activity,
        GrokActivity::Working
    );

    // ...and the unchanged newest attachment must not steal the root back.
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/older"));

    // A new attachment moves once more (C): repeated A→B→C keeps working.
    let attached = grok_snapshot(
        3,
        true,
        vec![
            grok_session("older", Some("/older"), 1, GrokActivity::Idle),
            grok_session("attached", Some("/attached"), 3, GrokActivity::Idle),
            grok_session("fresh", Some("/fresh"), 4, GrokActivity::Idle),
        ],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&attached));
    assert_eq!(workspace.root(), &path("/fresh"));

    // Ending the followed session keeps the root while others remain.
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            10,
            GrokEventKind::SessionEnd,
            "fresh",
            None,
            GrokActivity::Idle,
        ))
        .unwrap();
    assert_eq!(workspace.root(), &path("/fresh"));
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            11,
            GrokEventKind::SessionEnd,
            "attached",
            None,
            GrokActivity::Idle,
        ))
        .unwrap();
    assert_eq!(workspace.root(), &path("/fresh"));
    // The last owned session ending returns to the launch fallback.
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            12,
            GrokEventKind::SessionEnd,
            "older",
            None,
            GrokActivity::Idle,
        ))
        .unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.root(), &path("/project"));
    assert_eq!(workspace.projection().source, RootSource::Launch);
}

#[test]
fn grok_events_require_current_attachment_and_never_consume_pending_work() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace.begin_resnapshot().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    let inventory = grok_snapshot(
        1,
        true,
        vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));

    // A disconnected transport cannot mutate Grok state.
    workspace.disconnect();
    assert!(workspace
        .accept_grok_event(grok_event_envelope(
            &identity,
            1,
            GrokEventKind::Prompt,
            "a",
            Some("/grok-b"),
            GrokActivity::Working
        ))
        .is_err());
    assert!(workspace.projection().grok.is_none());
    workspace.reattach(2).unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    // Neither can a foreign epoch or a non-Grok envelope.
    let mut foreign = workspace.identity().clone();
    foreign.epoch = 3;
    assert!(workspace
        .accept_grok_event(grok_event_envelope(
            &foreign,
            1,
            GrokEventKind::Prompt,
            "a",
            Some("/grok-b"),
            GrokActivity::Working
        ))
        .is_err());
    assert!(workspace
        .accept_grok_event(WireEnvelope::new(
            workspace.identity().clone(),
            1,
            Operation::SessionState
        ))
        .is_err());

    // Re-establish the projection; an event never consumes pending work.
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));
    let list = workspace
        .request(
            Operation::List {
                path: path("/grok-a"),
            },
            None,
        )
        .unwrap();
    workspace
        .accept_grok_event(grok_event_envelope(
            &workspace.identity().clone(),
            list.request_id,
            GrokEventKind::Error,
            "a",
            None,
            GrokActivity::Attention,
        ))
        .unwrap();
    assert_eq!(
        workspace.pending_count(),
        1,
        "an event must not consume a numerically equal pending request"
    );
    assert_eq!(
        workspace.projection().grok.as_ref().unwrap().sessions[0].activity,
        GrokActivity::Attention
    );
    workspace
        .accept(reply(
            &list,
            Operation::Listed {
                path: path("/grok-a"),
                entries: vec![],
                truncated: false,
                done: true,
            },
        ))
        .unwrap();
    assert_eq!(workspace.pending_count(), 0);
}

#[test]
fn grok_state_replies_are_correlated_revision_gated_and_honest_on_error() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    let inventory = grok_snapshot(
        1,
        true,
        vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
    );

    // A GrokSnapshot bound to another request id is rejected without state.
    workspace.refresh_grok().unwrap();
    let state = workspace.next_command().unwrap();
    assert!(matches!(state.operation, Operation::GrokState));
    let mut forged = reply(
        &state,
        Operation::GrokSnapshot {
            snapshot: inventory.clone(),
        },
    );
    forged.request_id = state.request_id + 1;
    assert!(workspace.accept(forged).is_err());
    assert_eq!(
        workspace.pending_count(),
        1,
        "a foreign reply id must not consume the pending GrokState"
    );
    // The correctly correlated reply owns its pending request and moves the root.
    workspace
        .accept(reply(
            &state,
            Operation::GrokSnapshot {
                snapshot: inventory.clone(),
            },
        ))
        .unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.pending_count(), 0);
    assert_eq!(workspace.root(), &path("/grok-a"));

    // A lower revision cannot overwrite the projection or move the root.
    let newer = grok_snapshot(
        2,
        true,
        vec![grok_session("b", Some("/grok-b"), 5, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    let state = workspace.next_command().unwrap();
    workspace
        .accept(reply(&state, Operation::GrokSnapshot { snapshot: newer }))
        .unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.root(), &path("/grok-b"));
    workspace.refresh_grok().unwrap();
    let state = workspace.next_command().unwrap();
    assert!(workspace
        .accept(reply(
            &state,
            Operation::GrokSnapshot {
                snapshot: inventory
            }
        ))
        .is_err());
    assert_eq!(workspace.projection().grok.as_ref().unwrap().revision, 2);

    // An equal revision is a refresh completion, not a new move.
    workspace.refresh_grok().unwrap();
    let state = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &state,
            Operation::GrokSnapshot {
                snapshot: grok_snapshot(
                    2,
                    true,
                    vec![grok_session("b", Some("/grok-b"), 5, GrokActivity::Idle)],
                ),
            },
        ))
        .unwrap();
    assert!(workspace.next_command().is_none());
    assert_eq!(workspace.root(), &path("/grok-b"));

    // A GrokState error clears the projection honestly without touching the
    // OpenCode integration state, and the root falls back.
    let integration_before = workspace.integration();
    workspace.refresh_grok().unwrap();
    let state = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &state,
            Operation::Error {
                code: ErrorCode::Unavailable,
            },
        ))
        .unwrap();
    assert!(workspace.projection().grok.is_none());
    assert_eq!(workspace.integration(), integration_before);
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.root(), &path("/project"));
    assert_eq!(workspace.projection().source, RootSource::Launch);
}

#[test]
fn reconnect_discards_previous_epoch_grok_state_and_resnapshots() {
    let mut workspace = model(1);
    let old_identity = workspace.identity().clone();
    workspace.begin_resnapshot().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    let inventory = grok_snapshot(
        4,
        true,
        vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));

    // Disconnect clears the projection and the follow state.
    workspace.disconnect();
    assert!(workspace.projection().grok.is_none());
    workspace.reattach(2).unwrap();
    // A previous-attachment event cannot mutate the fresh epoch.
    assert!(workspace
        .accept_grok_event(grok_event_envelope(
            &old_identity,
            3,
            GrokEventKind::Prompt,
            "a",
            Some("/grok-a"),
            GrokActivity::Working
        ))
        .is_err());
    // The fresh-epoch snapshot re-establishes the root from a clean gate.
    let fresh = grok_snapshot(
        1,
        true,
        vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&fresh));
    assert_eq!(workspace.root(), &path("/grok-a"));
    assert_eq!(workspace.projection().source, RootSource::Grok);
    assert_eq!(workspace.projection().grok.as_ref().unwrap().revision, 1);
}

#[test]
fn grok_session_start_moves_before_the_first_inventory_snapshot() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    let identity = workspace.identity().clone();
    workspace
        .accept_grok_event(grok_event_envelope(
            &identity,
            5,
            GrokEventKind::SessionStart,
            "live",
            Some("/started"),
            GrokActivity::Idle,
        ))
        .unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    assert_eq!(workspace.root(), &path("/started"));
    assert_eq!(workspace.projection().source, RootSource::Grok);

    // The inventory confirms the session afterwards without stealing the root.
    let inventory = grok_snapshot(
        1,
        true,
        vec![grok_session(
            "live",
            Some("/started"),
            9,
            GrokActivity::Idle,
        )],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/started"));
    assert_eq!(
        workspace.projection().grok.as_ref().unwrap().sessions[0].opened_at,
        9
    );
}

#[test]
fn pre_attach_failure_projects_a_bounded_unavailable_tree() {
    let mut workspace = model(1);
    workspace.load_directory(path("/project")).unwrap();
    assert!(!workspace.projection().directories.is_empty());
    let revision = workspace.projection().revision;

    workspace.project_unavailable(ErrorCode::Unavailable);
    let projection = workspace.projection();
    assert!(matches!(
        projection.root_state,
        Freshness::Unavailable(ErrorCode::Unavailable)
    ));
    assert!(
        projection.directories.is_empty(),
        "a failure must not keep a partial tree"
    );
    assert!(projection.expanded.is_empty());
    assert!(projection.preview.is_none());
    assert_ne!(projection.revision, revision);
}

#[test]
fn grok_cwd_is_used_byte_exact_and_granted_like_any_remote_report() {
    // A cwd outside the launch root is selected exactly as reported; the
    // server validates the path, never the host filesystem.
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), None);
    let inventory = grok_snapshot(
        1,
        true,
        vec![grok_session(
            "a",
            Some("/srv/odd-name."),
            1,
            GrokActivity::Idle,
        )],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/srv/odd-name."));
    assert_eq!(workspace.projection().source, RootSource::Grok);
}

fn claude_session(id: &str, pid: u32, cwd: Option<&str>, recency: u64) -> ClaudeSessionInfo {
    ClaudeSessionInfo {
        session_id: id.to_owned(),
        pid,
        cwd: cwd.map(path),
        activity: ClaudeActivity::Idle,
        title: String::new(),
        recency,
        nested: false,
    }
}
fn claude_frame(identity: &Identity, request_id: u64, operation: Operation) -> WireEnvelope {
    WireEnvelope::new(identity.clone(), request_id, operation)
}
fn claude_event(revision: u64, sessions: Vec<ClaudeSessionInfo>) -> Operation {
    Operation::ClaudeEvent {
        snapshot: ClaudeSessionSnapshot {
            revision,
            hooks_available: true,
            sessions,
        },
    }
}
fn claude_listing(revision: u64, background: &[(&str, &str)]) -> Operation {
    Operation::ClaudeListing {
        listing: ClaudeListingResult {
            revision,
            background: background
                .iter()
                .map(|(name, cwd)| ClaudeBackgroundEntry {
                    name: (*name).to_owned(),
                    cwd: path(cwd),
                })
                .collect(),
        },
    }
}

#[test]
fn claude_relay_is_accepted_only_for_the_owning_tab_current_epoch_and_newer_revision() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    assert!(workspace.projection().claude.is_none());
    assert_eq!(workspace.claude_hooks_available(), None);

    // Another tab and a previous epoch are rejected without effect.
    let other = model(1).identity().clone();
    assert!(workspace
        .accept_claude(claude_frame(&other, 1, claude_event(1, vec![])))
        .is_err());
    let mut stale_epoch = identity.clone();
    stale_epoch.epoch = 0;
    assert!(workspace
        .accept_claude(claude_frame(&stale_epoch, 1, claude_event(1, vec![])))
        .is_err());
    assert!(workspace.projection().claude.is_none());

    // The deferred state reply and pushes apply by revision; Claude Code
    // state is not filesystem-generation scoped.
    let mut moved = identity.clone();
    moved.generation += 3;
    workspace
        .accept_claude(claude_frame(
            &moved,
            7,
            Operation::ClaudeSnapshot {
                snapshot: ClaudeSessionSnapshot {
                    revision: 2,
                    hooks_available: true,
                    sessions: vec![claude_session("a", 40, Some("/claude-a"), 1)],
                },
            },
        ))
        .unwrap();
    assert_eq!(workspace.claude_hooks_available(), Some(true));
    assert_eq!(workspace.projection().claude.as_ref().unwrap().revision, 2);

    // A late or duplicated item does not apply again.
    assert!(workspace
        .accept_claude(claude_frame(&identity, 1, claude_event(1, vec![])))
        .is_err());
    workspace
        .accept_claude(claude_frame(&identity, 1, claude_event(2, vec![])))
        .unwrap();
    assert_eq!(
        workspace
            .projection()
            .claude
            .as_ref()
            .unwrap()
            .sessions
            .len(),
        1,
        "an equal revision is a duplicate and changes nothing"
    );
    workspace
        .accept_claude(claude_frame(&identity, 1, claude_event(3, vec![])))
        .unwrap();
    assert!(workspace
        .projection()
        .claude
        .as_ref()
        .unwrap()
        .sessions
        .is_empty());

    // A listing is handed out once.
    workspace
        .accept_claude(claude_frame(
            &identity,
            1,
            claude_listing(4, &[("Spec task", "/bg")]),
        ))
        .unwrap();
    let listing = workspace.take_claude_listing().unwrap();
    assert_eq!(listing.background[0].cwd, path("/bg"));
    assert!(workspace.take_claude_listing().is_none());
    workspace
        .accept_claude(claude_frame(
            &identity,
            1,
            claude_listing(4, &[("Spec task", "/bg")]),
        ))
        .unwrap();
    assert!(
        workspace.take_claude_listing().is_none(),
        "a duplicated listing is not applied twice"
    );
    assert!(workspace
        .accept_claude(claude_frame(&identity, 1, claude_listing(3, &[])))
        .is_err());

    // Corrupt or oversized items fail bounded and change nothing.
    let mut long_title = claude_session("b", 41, None, 2);
    long_title.title = "x".repeat(made_remote::protocol::MAX_CLAUDE_TITLE_CHARS + 1);
    assert!(workspace
        .accept_claude(claude_frame(
            &identity,
            1,
            claude_event(9, vec![long_title])
        ))
        .is_err());
    let crowded: Vec<(String, String)> = (0..=made_remote::protocol::MAX_CLAUDE_BACKGROUND)
        .map(|index| (format!("n{index}"), format!("/bg{index}")))
        .collect();
    let crowded: Vec<(&str, &str)> = crowded
        .iter()
        .map(|(name, cwd)| (name.as_str(), cwd.as_str()))
        .collect();
    assert!(workspace
        .accept_claude(claude_frame(&identity, 1, claude_listing(10, &crowded)))
        .is_err());
    assert!(workspace
        .accept_claude(claude_frame(
            &identity,
            1,
            Operation::GrokSnapshot {
                snapshot: grok_snapshot(11, true, vec![]),
            },
        ))
        .is_err());
    assert_eq!(workspace.projection().claude.as_ref().unwrap().revision, 3);
    assert!(workspace.take_claude_listing().is_none());
}

#[test]
fn claude_push_never_consumes_pending_work_even_by_numeric_id() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    let list_id = workspace.load_directory(path("/project")).unwrap();
    let pending = workspace.pending_count();
    let identity = workspace.identity().clone();
    workspace
        .accept_claude(claude_frame(&identity, list_id, claude_event(1, vec![])))
        .unwrap();
    assert_eq!(workspace.pending_count(), pending);
    assert_eq!(workspace.next_command().unwrap().request_id, list_id);
}

#[test]
fn claude_root_sits_below_opencode_and_above_grok_and_each_is_retained() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    let inventory = grok_snapshot(
        1,
        true,
        vec![grok_session("g", Some("/grok-a"), 1, GrokActivity::Idle)],
    );
    workspace.refresh_grok().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));

    // Claude Code holds the root over Grok; Grok is retained under it.
    workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/claude-a"));
    assert_eq!(workspace.projection().source, RootSource::Claude);
    workspace.set_claude_root(None).unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(1)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/grok-a"));
    assert_eq!(workspace.projection().source, RootSource::Grok);

    // OpenCode outranks Claude Code, whose directory is retained under it.
    workspace.refresh_sessions().unwrap();
    settle_with_grok(
        &mut workspace,
        Some(&active_snapshot(2, "/opencode")),
        Some(&inventory),
    );
    assert_eq!(workspace.root(), &path("/opencode"));
    workspace.set_claude_root(Some(path("/claude-b"))).unwrap();
    settle_with_grok(
        &mut workspace,
        Some(&active_snapshot(2, "/opencode")),
        Some(&inventory),
    );
    assert_eq!(workspace.root(), &path("/opencode"));
    assert_eq!(workspace.projection().source, RootSource::Session);
    workspace.refresh_sessions().unwrap();
    settle_with_grok(&mut workspace, Some(&empty_snapshot(3)), Some(&inventory));
    assert_eq!(workspace.root(), &path("/claude-b"));
    assert_eq!(workspace.projection().source, RootSource::Claude);
}

#[test]
fn a_claude_directory_rejected_by_the_server_does_not_move_the_explorer() {
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/shell-a"));

    workspace.set_claude_root(Some(path("/missing"))).unwrap();
    let select = workspace.next_command().unwrap();
    assert!(matches!(
        &select.operation,
        Operation::RootSelect { path: selected } if selected == &path("/missing")
    ));
    workspace
        .accept(reply(
            &select,
            Operation::Error {
                code: ErrorCode::Unavailable,
            },
        ))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/shell-a"));
    assert_eq!(workspace.projection().source, RootSource::Shell);
    assert_eq!(workspace.projection().root_state, Freshness::Current);
    // The same rejected directory is not retried; a different one is.
    workspace.set_claude_root(Some(path("/missing"))).unwrap();
    assert!(workspace.next_command().is_none());
    workspace.set_claude_root(Some(path("/claude-b"))).unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/claude-b"));
}

#[test]
fn a_transient_failure_retries_the_claude_directory_after_a_backoff() {
    use std::time::{Duration, Instant};
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    let fail = |workspace: &mut RemoteWorkspace, code| {
        let select = workspace.next_command().unwrap();
        assert!(matches!(
            &select.operation,
            Operation::RootSelect { path: selected } if selected == &path("/claude-a")
        ));
        workspace
            .accept(reply(&select, Operation::Error { code }))
            .unwrap();
        settle(workspace, &empty_snapshot(1));
    };
    for code in [ErrorCode::Timeout, ErrorCode::Stale, ErrorCode::Io] {
        workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
        fail(&mut workspace, code);
        assert_eq!(workspace.root(), &path("/shell-a"), "{code:?}");
        // Not before the backoff.
        let started = Instant::now();
        workspace.expire(started).unwrap();
        workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
        assert!(workspace.next_command().is_none(), "{code:?}");
        // After about a second the same directory is tried again.
        workspace
            .expire(started + Duration::from_millis(1100))
            .unwrap();
        settle(&mut workspace, &empty_snapshot(1));
        assert_eq!(workspace.root(), &path("/claude-a"), "{code:?}");
        assert_eq!(workspace.projection().source, RootSource::Claude);
        workspace.set_claude_root(None).unwrap();
        settle(&mut workspace, &empty_snapshot(1));
        assert_eq!(workspace.root(), &path("/shell-a"));
    }
    // A missing, denied or invalid directory is a rejection: no retry.
    for code in [
        ErrorCode::Unavailable,
        ErrorCode::Unauthorized,
        ErrorCode::InvalidRequest,
    ] {
        workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
        fail(&mut workspace, code);
        workspace
            .expire(Instant::now() + Duration::from_secs(5))
            .unwrap();
        workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
        assert!(workspace.next_command().is_none(), "{code:?}");
        assert_eq!(workspace.root(), &path("/shell-a"));
        workspace.set_claude_root(None).unwrap();
    }
}

#[test]
fn an_ended_session_cancels_a_pending_claude_directory_retry() {
    use std::time::{Duration, Instant};
    let mut workspace = model(1);
    let identity = workspace.identity().clone();
    workspace
        .report_shell_cwd(&identity, path("/shell-a"))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    workspace.set_claude_root(Some(path("/claude-a"))).unwrap();
    let select = workspace.next_command().unwrap();
    workspace
        .accept(reply(
            &select,
            Operation::Error {
                code: ErrorCode::Timeout,
            },
        ))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    // The session ends while its directory waits for the retry: the root
    // never comes back to it.
    workspace.set_claude_root(None).unwrap();
    workspace
        .expire(Instant::now() + Duration::from_secs(5))
        .unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/shell-a"));
    assert_ne!(workspace.projection().source, RootSource::Claude);
}

#[test]
fn disconnect_and_reattach_clear_claude_state_until_a_fresh_snapshot() {
    let mut workspace = model(1);
    workspace.begin_resnapshot().unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    let old = workspace.identity().clone();
    workspace
        .accept_claude(claude_frame(
            &old,
            1,
            claude_event(5, vec![claude_session("b", 40, Some("/claude-b"), 1)]),
        ))
        .unwrap();
    workspace
        .accept_claude(claude_frame(&old, 1, claude_listing(6, &[])))
        .unwrap();
    workspace.set_claude_root(Some(path("/claude-b"))).unwrap();
    settle(&mut workspace, &empty_snapshot(1));
    assert_eq!(workspace.root(), &path("/claude-b"));

    workspace.disconnect();
    assert!(workspace.projection().claude.is_none());
    assert_eq!(workspace.claude_hooks_available(), None);
    assert!(workspace.take_claude_listing().is_none());
    workspace.reattach(2).unwrap();
    // Previous-epoch state cannot apply to the fresh attachment.
    assert!(workspace
        .accept_claude(claude_frame(&old, 1, claude_event(7, vec![])))
        .is_err());
    settle(&mut workspace, &empty_snapshot(1));
    assert_ne!(workspace.projection().source, RootSource::Claude);
    // The fresh epoch restarts its revisions at 1.
    let fresh = workspace.identity().clone();
    workspace
        .accept_claude(claude_frame(
            &fresh,
            1,
            claude_event(1, vec![claude_session("b", 40, Some("/claude-c"), 1)]),
        ))
        .unwrap();
    assert_eq!(workspace.projection().claude.as_ref().unwrap().revision, 1);
}

mod remote_claude_row {
    use made_app::diagnostics::DiagnosticsPill;
    use made_app::diagnostics::DiagnosticsPill::{Degraded, Inactive, Ready};
    use made_app::integrations::claude_code::diagnostics::{
        claude_diagnostics_row, ClaudeDiagnosticsInputs, ClaudeDiagnosticsPhase, CLAUDE_GRACE,
    };
    use std::time::Duration;

    /// A remote tab's inputs: its server's detection, whether the staged
    /// package is unavailable for the tab (as on an older supervisor), its
    /// presence, live owned sessions and observed protocol.
    fn inputs(
        phase: ClaudeDiagnosticsPhase,
        staged_unavailable: bool,
        present: bool,
        present_for: Duration,
        live_session: bool,
        protocol_observed: bool,
    ) -> ClaudeDiagnosticsInputs {
        ClaudeDiagnosticsInputs {
            phase,
            plugin_missing: staged_unavailable,
            present,
            present_for,
            live_session,
            protocol_observed,
        }
    }

    fn row(inputs: ClaudeDiagnosticsInputs) -> (DiagnosticsPill, String) {
        let report = claude_diagnostics_row(&inputs);
        (report.pill, report.sentence)
    }

    #[test]
    fn remote_states_use_the_tabs_own_detection_and_the_local_sentences() {
        use ClaudeDiagnosticsPhase::{Available, Checking, NotOnPath};
        let young = Duration::from_secs(1);
        let old = CLAUDE_GRACE + Duration::from_secs(1);
        assert_eq!(
            row(inputs(Checking, false, false, Duration::ZERO, false, false)),
            (Inactive, "Checking for Claude Code.".to_owned())
        );
        assert_eq!(
            row(inputs(
                NotOnPath,
                false,
                false,
                Duration::ZERO,
                false,
                false
            )),
            (Inactive, "Claude Code is not on PATH.".to_owned())
        );
        assert_eq!(
            row(inputs(
                Available,
                false,
                false,
                Duration::ZERO,
                false,
                false
            )),
            (Inactive, "Claude Code is not running.".to_owned())
        );
        assert_eq!(
            row(inputs(Available, false, true, young, false, false)),
            (Inactive, "Claude Code is starting.".to_owned())
        );
        // `--bare` past the grace period: protocol only, no live session.
        assert_eq!(
            row(inputs(Available, false, true, old, false, true)),
            (
                Degraded,
                "Claude Code session-status hooks are unavailable.".to_owned()
            )
        );
        // A live owned session without a session-level emission.
        assert_eq!(
            row(inputs(Available, false, true, old, true, false)),
            (
                Degraded,
                "Claude Code status protocol is unavailable.".to_owned()
            )
        );
        for (_, sentence) in [
            row(inputs(Available, false, true, old, false, true)),
            row(inputs(Available, false, true, old, true, false)),
        ] {
            assert!(!sentence.contains("over SSH"), "{sentence}");
        }
    }

    #[test]
    fn a_remote_row_reaches_ready_with_a_live_owned_session_and_the_protocol() {
        let report = row(inputs(
            ClaudeDiagnosticsPhase::Available,
            false,
            true,
            Duration::from_secs(1),
            true,
            true,
        ));
        assert_eq!(
            report,
            (Ready, "Claude Code session follow is ready.".to_owned())
        );
    }

    #[test]
    fn an_older_supervisor_reports_session_status_hooks_unavailable() {
        for present in [false, true] {
            assert_eq!(
                row(inputs(
                    ClaudeDiagnosticsPhase::Available,
                    true,
                    present,
                    CLAUDE_GRACE * 2,
                    false,
                    true,
                )),
                (
                    Degraded,
                    "Claude Code session-status hooks are unavailable.".to_owned()
                )
            );
        }
    }
}

// ---- Open workspace remote browse grant, client side (SSH-12, task 5.5) ----

mod browse {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use made_app::integrations::{CLAUDE_CODE, GROK, OPENCODE, PRODUCT};
    use made_app::open_workspace::{
        ConnectionState, CountsState, GitState, HostId, LaunchKind, OpenWorkspace, SshHost,
    };
    use made_app::remote_session::SshFailureClass;
    use made_app::remote_workspace::{
        browse_consent, parse_remote_address, remote_launch_target, BrowseConnection,
        BrowseConsent, BrowseEvent, BrowsePhase, RemoteBrowse, RemoteFolder, BROWSE_TIMEOUT,
        MAX_WORKTREES,
    };
    use made_app::workspace_picker::AddressError;
    use made_app::TerminalTarget;
    use made_platform::ssh::SshEvent;
    use made_platform::ssh_askpass::{PromptAnswer, PromptRequest};
    use made_remote::git::ChangeCounts;
    use made_remote::protocol::MAX_IN_FLIGHT;
    use made_remote::protocol::{
        encode_browse, AuthorityId, BrowseEnvelope, BrowseOperation, Chunk, DirectoryCheck,
        DirectoryEntry, EntryKind, ErrorCode, GitRead, RemotePath,
    };

    const A: AuthorityId = AuthorityId([7; 16]);

    fn path(value: &str) -> RemotePath {
        RemotePath::new(value.as_bytes().to_vec()).unwrap()
    }

    fn reply(request: &BrowseEnvelope, operation: BrowseOperation) -> BrowseEnvelope {
        BrowseEnvelope::new(request.identity.clone(), request.request_id, operation)
    }

    /// An open grant with its open frame answered.
    fn opened() -> RemoteBrowse {
        let mut browse = RemoteBrowse::open(A, 1).unwrap();
        let open = browse.next_frame().unwrap();
        assert_eq!(open.operation, BrowseOperation::BrowseOpen);
        let events = browse
            .accept(reply(
                &open,
                BrowseOperation::BrowseOpened {
                    home: Some(path("/home/me")),
                },
            ))
            .unwrap();
        assert_eq!(
            events,
            vec![BrowseEvent::Opened {
                home: Some(path("/home/me"))
            }]
        );
        assert_eq!(browse.phase(), BrowsePhase::Open);
        browse
    }

    fn frames(browse: &mut RemoteBrowse) -> Vec<BrowseEnvelope> {
        std::iter::from_fn(|| browse.next_frame()).collect()
    }

    fn git_result(request: &BrowseEnvelope, bytes: &[u8]) -> BrowseEnvelope {
        let BrowseOperation::BrowseGitRead { path, command } = &request.operation else {
            panic!("not a git read");
        };
        reply(
            request,
            BrowseOperation::BrowseGitResult {
                path: path.clone(),
                command: *command,
                offset: 0,
                data: Chunk::new(bytes.to_vec()).unwrap(),
                done: true,
            },
        )
    }

    fn checked(request: &BrowseEnvelope, result: DirectoryCheck) -> BrowseEnvelope {
        let BrowseOperation::BrowseCheck { path } = &request.operation else {
            panic!("not a check");
        };
        reply(
            request,
            BrowseOperation::BrowseChecked {
                requested: path.clone(),
                result,
            },
        )
    }

    #[test]
    fn the_first_frame_opens_a_fresh_random_grant() {
        let mut one = RemoteBrowse::open(A, 1).unwrap();
        let mut two = RemoteBrowse::open(A, 1).unwrap();
        let first = one.next_frame().unwrap();
        assert_eq!(first.operation, BrowseOperation::BrowseOpen);
        assert_eq!(
            two.next_frame().unwrap().operation,
            BrowseOperation::BrowseOpen
        );
        assert_ne!(one.identity().grant, two.identity().grant);
        // Requests queued before the open reply follow the open frame.
        one.list(path("/srv")).unwrap();
        assert!(matches!(
            one.next_frame().unwrap().operation,
            BrowseOperation::BrowseList { .. }
        ));
    }

    #[test]
    fn listings_show_sanitized_folders_with_the_truncation_notice() {
        let mut browse = opened();
        browse.set_root(path("/srv"));
        browse.list(path("/srv")).unwrap();
        let request = browse.next_frame().unwrap();
        let entry = |name: &str, kind| DirectoryEntry {
            path: path(&format!("/srv/{name}")),
            kind,
        };
        let first = reply(
            &request,
            BrowseOperation::BrowseListed {
                path: path("/srv"),
                entries: vec![
                    entry("app", EntryKind::Directory),
                    entry("notes.txt", EntryKind::File),
                ],
                truncated: false,
                done: false,
            },
        );
        assert_eq!(
            browse.accept(first).unwrap(),
            Vec::new(),
            "partial listing is not published"
        );
        let last = reply(
            &request,
            BrowseOperation::BrowseListed {
                path: path("/srv"),
                entries: vec![
                    entry("link", EntryKind::Symlink),
                    entry("-rf $(x)\u{1b}[31m", EntryKind::Directory),
                ],
                truncated: true,
                done: true,
            },
        );
        let events = browse.accept(last).unwrap();
        let [BrowseEvent::Listed {
            path: listed,
            folders,
            truncated,
        }] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(*listed, path("/srv"));
        assert!(*truncated);
        let names: Vec<&str> = folders
            .iter()
            .map(|folder: &RemoteFolder| folder.name.as_str())
            .collect();
        assert_eq!(names.len(), 2, "folders only; links are not followed");
        assert_eq!(names[0], "app");
        assert!(!names[1].contains('\u{1b}'), "{:?}", names[1]);
        assert!(
            names[1].starts_with("-rf $(x)"),
            "shown as text, never executed"
        );
    }

    #[test]
    fn stale_root_listings_and_foreign_identity_replies_are_discarded() {
        let mut browse = opened();
        browse.set_root(path("/srv/app"));
        browse.list(path("/srv/app/src")).unwrap();
        let request = browse.next_frame().unwrap();
        // A reply under another epoch is rejected and leaves the request.
        let mut foreign = reply(
            &request,
            BrowseOperation::BrowseListed {
                path: path("/srv/app/src"),
                entries: Vec::new(),
                truncated: false,
                done: true,
            },
        );
        foreign.identity.epoch = 2;
        assert!(browse.accept(foreign).is_err());
        assert_eq!(browse.pending_count(), 1);
        // The root moved: the late listing is dropped.
        browse.set_root(path("/other"));
        let late = reply(
            &request,
            BrowseOperation::BrowseListed {
                path: path("/srv/app/src"),
                entries: Vec::new(),
                truncated: false,
                done: true,
            },
        );
        assert_eq!(browse.accept(late).unwrap(), Vec::new());
        assert_eq!(browse.pending_count(), 0);
        // A reply nobody asked for is rejected.
        assert!(browse
            .accept(reply(&request, BrowseOperation::BrowseClosed))
            .is_err());
    }

    #[test]
    fn address_checks_are_server_side_and_map_to_the_four_reasons() {
        assert_eq!(
            parse_remote_address("srv/app"),
            Err(AddressError::NotAbsolute)
        );
        assert_eq!(parse_remote_address("  /srv/app// "), Ok(path("/srv/app")));
        assert_eq!(parse_remote_address(" / "), Ok(path("/")));
        let mut browse = opened();
        for (result, expected) in [
            (
                DirectoryCheck::Directory {
                    path: path("/srv/real"),
                },
                Ok(path("/srv/real")),
            ),
            (DirectoryCheck::Missing, Err(AddressError::NotFound)),
            (DirectoryCheck::Unreadable, Err(AddressError::NotReadable)),
            (DirectoryCheck::NotDirectory, Err(AddressError::NotAFolder)),
        ] {
            browse.check_address(path("/srv/link")).unwrap();
            let request = browse.next_frame().unwrap();
            assert_eq!(
                request.operation,
                BrowseOperation::BrowseCheck {
                    path: path("/srv/link")
                }
            );
            let events = browse.accept(checked(&request, result)).unwrap();
            assert_eq!(
                events,
                vec![BrowseEvent::AddressChecked {
                    requested: path("/srv/link"),
                    result: expected,
                }]
            );
        }
    }

    /// Answer every git step of a scripted remote repository.
    fn serve_repository(browse: &mut RemoteBrowse) -> Vec<BrowseEvent> {
        let porcelain = "worktree /work/made\nHEAD 0123456789abcdef0123456789abcdef01234567\nbranch refs/heads/main\n\nworktree /work/made-wt/feature-a\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/feature-a\n\nworktree /work/made-wt/fix\nHEAD abcdef0123456789abcdef0123456789abcdef01\ndetached\n\n";
        let mut events = Vec::new();
        for _ in 0..20 {
            let requests = frames(browse);
            if requests.is_empty() {
                break;
            }
            for request in requests {
                let answer = match &request.operation {
                    BrowseOperation::BrowseGitRead { path: at, command } => {
                        let at = at.to_utf8().unwrap();
                        match command {
                            GitRead::ShowToplevel => git_result(&request, format!("{at}\n").as_bytes()),
                            GitRead::WorktreeList => git_result(&request, porcelain.as_bytes()),
                            GitRead::Status if at == "/work/made" => git_result(
                                &request,
                                b"UU clash\0MM both.rs\0M  staged.rs\0 M dirty.rs\0!! ignored.log\0",
                            ),
                            GitRead::Status => git_result(&request, b"?? a\0?? b\0"),
                            other => panic!("unexpected git read {other:?}"),
                        }
                    }
                    BrowseOperation::BrowseCheck { path: at } => {
                        if at.to_utf8().unwrap() == "/work/made-wt/fix" {
                            checked(&request, DirectoryCheck::Missing)
                        } else {
                            checked(&request, DirectoryCheck::Directory { path: at.clone() })
                        }
                    }
                    other => panic!("unexpected request {other:?}"),
                };
                events.extend(browse.accept(answer).unwrap());
            }
        }
        events
    }

    fn remote_screen() -> OpenWorkspace {
        let mut screen = OpenWorkspace::new(
            PRODUCT,
            vec![SshHost {
                id: A,
                initial_directory: Some(PathBuf::from("/work/made")),
            }],
            Some(HostId::Ssh(A)),
            std::env::temp_dir(),
        );
        screen.set_connection(A, ConnectionState::Connected);
        screen
    }

    #[test]
    fn remote_git_parity_preselects_the_root_and_counts_like_the_git_view() {
        let mut screen = remote_screen();
        let request = screen.sync().unwrap();
        let mut browse = opened();
        browse
            .read_git(request.generation, path("/work/made"))
            .unwrap();
        let events = serve_repository(&mut browse);
        for event in &events {
            match event {
                BrowseEvent::Git { generation, state } => {
                    assert!(screen.apply_git(*generation, state.clone()));
                }
                BrowseEvent::Counts {
                    generation,
                    path,
                    counts,
                } => {
                    assert!(screen.apply_counts(*generation, path, *counts));
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(screen.git_indicator() && screen.worktrees_shown());
        let items = screen.worktrees();
        assert_eq!(
            items
                .iter()
                .map(|item| item.path.clone())
                .collect::<Vec<_>>(),
            vec![
                PathBuf::from("/work/made"),
                PathBuf::from("/work/made-wt/feature-a"),
                PathBuf::from("/work/made-wt/fix"),
            ]
        );
        assert_eq!(items[1].label.as_deref(), Some("feature-a"));
        assert_eq!(items[2].label.as_deref(), Some("abcdef01"));
        assert!(items[2].missing, "a deleted remote worktree is missing");
        assert_eq!(screen.worktree_selection(), Some(0));
        assert_eq!(
            items[0].counts,
            CountsState::Ready(ChangeCounts {
                conflicted: 1,
                staged: 2,
                modified: 2,
                untracked: 0,
            })
        );
        assert_eq!(
            items[1].counts,
            CountsState::Ready(ChangeCounts {
                untracked: 2,
                ..ChangeCounts::default()
            })
        );
        assert_eq!(
            items[2].counts,
            CountsState::Pending,
            "a missing worktree is not read"
        );
    }

    #[test]
    fn a_non_repository_or_missing_git_degrades_to_no_indicator() {
        let mut browse = opened();
        browse.read_git(4, path("/srv/plain")).unwrap();
        let request = browse.next_frame().unwrap();
        let events = browse
            .accept(reply(
                &request,
                BrowseOperation::Error {
                    code: ErrorCode::Unavailable,
                },
            ))
            .unwrap();
        assert_eq!(
            events,
            vec![BrowseEvent::Git {
                generation: 4,
                state: GitState::NoRepository,
            }]
        );
        assert!(browse.next_frame().is_none(), "no further git read");
    }

    #[test]
    fn stale_git_generations_are_discarded_without_further_reads() {
        let mut browse = opened();
        browse.read_git(1, path("/work/made")).unwrap();
        let old = browse.next_frame().unwrap();
        browse.read_git(2, path("/srv/plain")).unwrap();
        // One git read at a time: the new one is sent once the old answers,
        // since the server would run it only after the old one anyway.
        assert!(browse.next_frame().is_none());
        assert_eq!(
            browse.accept(git_result(&old, b"/work/made\n")).unwrap(),
            Vec::new()
        );
        let current = browse.next_frame().unwrap();
        assert!(
            matches!(&current.operation, BrowseOperation::BrowseGitRead { path: at, command: GitRead::ShowToplevel } if *at == path("/srv/plain"))
        );
        assert!(browse.next_frame().is_none(), "the stale sequence stops");
        let events = browse
            .accept(reply(
                &current,
                BrowseOperation::Error {
                    code: ErrorCode::Unavailable,
                },
            ))
            .unwrap();
        assert_eq!(
            events,
            vec![BrowseEvent::Git {
                generation: 2,
                state: GitState::NoRepository
            }]
        );
    }

    #[test]
    fn launch_revalidates_the_path_and_fails_contextually_when_it_vanished() {
        let mut browse = opened();
        browse
            .check_launch(LaunchKind::Tui(GROK), path("/srv/app"))
            .unwrap();
        let request = browse.next_frame().unwrap();
        let events = browse
            .accept(checked(&request, DirectoryCheck::Missing))
            .unwrap();
        assert_eq!(
            events,
            vec![BrowseEvent::LaunchChecked {
                kind: LaunchKind::Tui(GROK),
                requested: path("/srv/app"),
                result: Err(AddressError::NotFound),
            }]
        );
        browse
            .check_launch(LaunchKind::Terminal, path("/srv/app"))
            .unwrap();
        let request = browse.next_frame().unwrap();
        let events = browse
            .accept(checked(
                &request,
                DirectoryCheck::Directory {
                    path: path("/srv/app"),
                },
            ))
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [BrowseEvent::LaunchChecked { result: Ok(_), .. }]
        ));
    }

    #[test]
    fn remote_launch_targets_bind_to_the_selected_server_only() {
        assert_eq!(
            remote_launch_target(LaunchKind::Terminal, A),
            TerminalTarget::RemoteShell { authority: A }
        );
        assert_eq!(
            remote_launch_target(LaunchKind::Tui(OPENCODE), A),
            TerminalTarget::RemoteOpenCode { authority: A }
        );
        assert_eq!(
            remote_launch_target(LaunchKind::Tui(GROK), A),
            TerminalTarget::RemoteGrok { authority: A }
        );
        assert_eq!(
            remote_launch_target(LaunchKind::Tui(CLAUDE_CODE), A),
            TerminalTarget::RemoteClaude { authority: A }
        );
        for kind in [LaunchKind::Terminal, LaunchKind::Tui(GROK)] {
            let target = remote_launch_target(kind, A);
            assert!(target.is_remote());
            assert!(
                target
                    .pty_options("/tmp", std::ffi::OsStr::new("/usr/bin"))
                    .is_none(),
                "never a host executable"
            );
        }
    }

    #[test]
    fn a_stalled_server_times_out_without_blocking_and_ends_cleanly() {
        let mut browse = opened();
        browse.set_root(path("/srv"));
        browse.list(path("/srv")).unwrap();
        browse.read_git(3, path("/srv")).unwrap();
        browse.check_address(path("/srv/x")).unwrap();
        assert_eq!(frames(&mut browse).len(), 3);
        // Nothing answers: expiry fails each request on its own.
        assert!(browse.expire(Instant::now()).is_empty());
        let events = browse.expire(Instant::now() + Duration::from_secs(60));
        assert!(events.contains(&BrowseEvent::ListFailed { path: path("/srv") }));
        assert!(events.contains(&BrowseEvent::Git {
            generation: 3,
            state: GitState::NoRepository
        }));
        assert!(events.contains(&BrowseEvent::AddressChecked {
            requested: path("/srv/x"),
            result: Err(AddressError::NotReadable),
        }));
        assert_eq!(browse.pending_count(), 0);

        // Selecting Local never waits on the grant.
        let mut screen = remote_screen();
        screen.select_host(HostId::Local);
        assert_eq!(screen.selected_host(), HostId::Local);

        // Ending the grant drops late replies and refuses new requests.
        browse.list(path("/srv")).unwrap();
        let late = browse.next_frame().unwrap();
        browse.close();
        let close = browse.next_frame().unwrap();
        assert_eq!(close.operation, BrowseOperation::BrowseClose);
        assert_eq!(browse.phase(), BrowsePhase::Closing);
        let listed = reply(
            &late,
            BrowseOperation::BrowseListed {
                path: path("/srv"),
                entries: Vec::new(),
                truncated: false,
                done: true,
            },
        );
        assert!(browse.accept(listed).is_err());
        assert_eq!(
            browse
                .accept(reply(&close, BrowseOperation::BrowseClosed))
                .unwrap(),
            vec![BrowseEvent::Ended]
        );
        assert_eq!(browse.phase(), BrowsePhase::Ended);
        assert!(browse.list(path("/srv")).is_err());
    }

    #[test]
    fn consent_refusal_writes_nothing_and_gives_an_actionable_reason() {
        assert_eq!(browse_consent(true, None), BrowseConsent::Proceed);
        assert_eq!(browse_consent(false, Some(true)), BrowseConsent::Proceed);
        assert_eq!(browse_consent(false, None), BrowseConsent::Ask);
        let BrowseConsent::Refused(reason) = browse_consent(false, Some(false)) else {
            panic!("refused");
        };
        assert!(reason.contains("approve"), "{reason}");
        assert!(reason.contains("Nothing was written"), "{reason}");
    }

    /// A scripted stream: records written bytes and replays queued events.
    #[derive(Default)]
    struct Scripted {
        written: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        events: std::collections::VecDeque<SshEvent>,
        /// Writes refused with `WouldBlock` before writes are accepted.
        would_block: usize,
        prompts: std::collections::VecDeque<PromptRequest>,
    }
    impl made_remote::client::ClientStream for Scripted {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            if self.would_block > 0 {
                self.would_block -= 1;
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }
        fn poll(&mut self) -> Option<SshEvent> {
            self.events.pop_front()
        }
        fn detach(&mut self) {}
        fn poll_prompt(&mut self) -> Option<PromptRequest> {
            self.prompts.pop_front()
        }
        fn answer_prompt(&mut self, _: u64, _: Option<PromptAnswer>) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn written_frames(written: &std::sync::Mutex<Vec<u8>>) -> Vec<BrowseEnvelope> {
        let bytes = written.lock().unwrap().clone();
        let mut decoder = made_remote::protocol::BrowseFrameDecoder::new();
        let mut rest = bytes.as_slice();
        let mut frames = Vec::new();
        while !rest.is_empty() {
            let (used, frame) = decoder.push(rest).unwrap();
            rest = &rest[used..];
            frames.extend(frame);
        }
        frames
    }

    #[test]
    fn requests_beyond_the_in_flight_cap_wait_their_turn() {
        let mut browse = opened();
        browse.set_root(path("/srv"));
        let ids: Vec<u64> = (0..40)
            .map(|index| browse.list(path(&format!("/srv/d{index}"))).unwrap())
            .collect();
        let sent = frames(&mut browse);
        assert_eq!(sent.len(), MAX_IN_FLIGHT, "only the cap is on the wire");
        assert_eq!(browse.pending_count(), 40);
        assert_eq!(
            sent.iter()
                .map(|frame| frame.request_id)
                .collect::<Vec<_>>(),
            ids[..MAX_IN_FLIGHT].to_vec()
        );
        // One answer releases exactly one more, in order.
        let events = browse
            .accept(reply(
                &sent[0],
                BrowseOperation::BrowseListed {
                    path: path("/srv/d0"),
                    entries: Vec::new(),
                    truncated: false,
                    done: true,
                },
            ))
            .unwrap();
        assert_eq!(events.len(), 1);
        let next = frames(&mut browse);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].request_id, ids[MAX_IN_FLIGHT]);
        // Waiting requests never time out before they are sent.
        let events = browse.expire(Instant::now() + Duration::from_secs(60));
        assert_eq!(events.len(), MAX_IN_FLIGHT);
        assert_eq!(frames(&mut browse).len(), MAX_IN_FLIGHT);
    }

    #[test]
    fn a_huge_worktree_list_is_deduplicated_capped_and_checked_within_the_cap() {
        let mut browse = opened();
        browse.read_git(1, path("/r/main")).unwrap();
        let top = frames(&mut browse).remove(0);
        assert!(browse
            .accept(git_result(&top, b"/r/main\n"))
            .unwrap()
            .is_empty());
        let list = frames(&mut browse).remove(0);
        let mut porcelain = String::from("worktree /r/main\nbranch refs/heads/main\n\n");
        for index in 0..5000 {
            // Every path twice.
            for _ in 0..2 {
                porcelain.push_str(&format!("worktree /r/wt{index}\nHEAD {index:040}\n\n"));
            }
        }
        let mut events = Vec::new();
        let chunks: Vec<&[u8]> = porcelain.as_bytes().chunks(60_000).collect();
        let mut offset = 0u64;
        for (index, chunk) in chunks.iter().enumerate() {
            let BrowseOperation::BrowseGitRead { path: at, command } = &list.operation else {
                panic!("git read");
            };
            events.extend(
                browse
                    .accept(reply(
                        &list,
                        BrowseOperation::BrowseGitResult {
                            path: at.clone(),
                            command: *command,
                            offset,
                            data: Chunk::new(chunk.to_vec()).unwrap(),
                            done: index + 1 == chunks.len(),
                        },
                    ))
                    .unwrap(),
            );
            offset += chunk.len() as u64;
        }
        assert!(events.is_empty());
        let mut checks = 0;
        let mut git = None;
        for _ in 0..100 {
            let wire = frames(&mut browse);
            assert!(wire.len() <= MAX_IN_FLIGHT, "{} frames at once", wire.len());
            if wire.is_empty() {
                break;
            }
            for request in wire {
                let answer = match &request.operation {
                    BrowseOperation::BrowseCheck { path: at } => {
                        checks += 1;
                        checked(&request, DirectoryCheck::Directory { path: at.clone() })
                    }
                    BrowseOperation::BrowseGitRead { .. } => git_result(&request, b""),
                    other => panic!("unexpected {other:?}"),
                };
                for event in browse.accept(answer).unwrap() {
                    if let BrowseEvent::Git { state, .. } = event {
                        git = Some(state);
                    }
                }
            }
        }
        let Some(GitState::Repository { worktrees, .. }) = git else {
            panic!("no repository state");
        };
        assert_eq!(worktrees.len(), MAX_WORKTREES);
        assert_eq!(checks, MAX_WORKTREES);
        let mut unique: Vec<_> = worktrees.iter().map(|item| item.path.clone()).collect();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), MAX_WORKTREES, "duplicates were kept");
        assert_eq!(worktrees[0].path, PathBuf::from("/r/main"));
        assert!(browse.git_truncated());
    }

    #[test]
    fn one_git_read_is_on_the_wire_at_a_time_and_none_waits_out_its_timeout() {
        let mut browse = opened();
        browse.set_root(path("/r"));
        browse.read_git(1, path("/r/main")).unwrap();
        let top = frames(&mut browse).remove(0);
        assert!(browse
            .accept(git_result(&top, b"/r/main\n"))
            .unwrap()
            .is_empty());
        let list = frames(&mut browse).remove(0);
        let mut porcelain = String::from("worktree /r/main\nbranch refs/heads/main\n\n");
        for index in 0..20 {
            porcelain.push_str(&format!("worktree /r/wt{index}\nHEAD {index:040}\n\n"));
        }
        assert!(browse
            .accept(git_result(&list, porcelain.as_bytes()))
            .unwrap()
            .is_empty());
        // Every worktree is checked; listings are not held back by git reads.
        let mut counts = 0;
        for round in 0..100 {
            if round == 1 {
                browse.list(path("/r")).unwrap();
            }
            let wire = frames(&mut browse);
            if wire.is_empty() {
                break;
            }
            let git_reads = wire
                .iter()
                .filter(|frame| matches!(frame.operation, BrowseOperation::BrowseGitRead { .. }))
                .count();
            assert!(git_reads <= 1, "{git_reads} git reads at once");
            for request in wire {
                let answer = match &request.operation {
                    BrowseOperation::BrowseCheck { path: at } => {
                        checked(&request, DirectoryCheck::Directory { path: at.clone() })
                    }
                    BrowseOperation::BrowseGitRead { .. } => git_result(&request, b""),
                    BrowseOperation::BrowseList { path: at } => reply(
                        &request,
                        BrowseOperation::BrowseListed {
                            path: at.clone(),
                            entries: Vec::new(),
                            truncated: false,
                            done: true,
                        },
                    ),
                    other => panic!("unexpected {other:?}"),
                };
                for event in browse.accept(answer).unwrap() {
                    if matches!(
                        event,
                        BrowseEvent::Counts {
                            counts: CountsState::Ready(_),
                            ..
                        }
                    ) {
                        counts += 1;
                    }
                }
            }
            // Each status may take nine seconds on the server: a waiting
            // read's clock starts only when it is sent.
            assert!(
                browse
                    .expire(Instant::now() + Duration::from_secs(9))
                    .is_empty(),
                "a queued git read timed out"
            );
        }
        assert_eq!(counts, 21);
    }

    #[test]
    fn a_full_write_queue_is_retried_on_the_next_pump() {
        let browse = RemoteBrowse::open(A, 1).unwrap();
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stream = Scripted {
            written: written.clone(),
            would_block: 2,
            ..Scripted::default()
        };
        let mut connection = BrowseConnection::new(stream, browse);
        assert!(connection.pump(Instant::now()).is_empty());
        assert!(connection.pump(Instant::now()).is_empty());
        assert_eq!(connection.browse().phase(), BrowsePhase::Opening);
        assert!(written.lock().unwrap().is_empty());
        assert!(connection.pump(Instant::now()).is_empty());
        let sent = written_frames(&written);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].operation, BrowseOperation::BrowseOpen);
    }

    #[test]
    fn a_flood_of_unknown_reply_frames_is_processed_within_a_pump_budget() {
        let browse = RemoteBrowse::open(A, 1).unwrap();
        let identity = browse.identity().clone();
        let mut events = std::collections::VecDeque::new();
        for index in 0..20_000u64 {
            let frame = BrowseEnvelope::new(
                identity.clone(),
                1_000 + index,
                BrowseOperation::BrowseOpened { home: None },
            );
            events.push_back(SshEvent::Data(encode_browse(&frame).unwrap()));
        }
        let stream = Scripted {
            events,
            ..Scripted::default()
        };
        let mut connection = BrowseConnection::new(stream, browse);
        assert!(connection.pump(Instant::now()).is_empty());
        let left = connection.stream_mut().events.len();
        assert!(left > 0, "one pump drained the whole flood");
        assert!(left < 20_000);
        assert_eq!(connection.browse().phase(), BrowsePhase::Opening);
    }

    #[test]
    fn the_open_timeout_waits_while_a_prompt_awaits_the_user() {
        let browse = RemoteBrowse::open(A, 1).unwrap();
        let stream = Scripted {
            prompts: [PromptRequest {
                id: 9,
                text: "me@srv's password:".to_owned(),
                expires_at: Instant::now() + Duration::from_secs(300),
            }]
            .into(),
            ..Scripted::default()
        };
        let mut connection = BrowseConnection::new(stream, browse);
        assert!(connection.pump(Instant::now()).is_empty());
        let prompt = connection.poll_prompt().expect("the prompt");
        // The user takes a minute: the open request is not expired.
        assert!(connection
            .pump(Instant::now() + Duration::from_secs(60))
            .is_empty());
        assert_eq!(connection.browse().phase(), BrowsePhase::Opening);
        connection
            .answer_prompt(prompt.id, PromptAnswer::new(b"secret".to_vec()).ok())
            .unwrap();
        // Answered: the open gets a fresh window, then expires on its own.
        assert!(connection
            .pump(Instant::now() + Duration::from_secs(5))
            .is_empty());
        assert_eq!(
            connection.pump(Instant::now() + BROWSE_TIMEOUT + Duration::from_secs(1)),
            vec![BrowseEvent::Ended]
        );
        assert_eq!(connection.end_failure(), None);
    }

    #[test]
    fn a_changed_host_key_ends_the_grant_with_the_changed_key_class() {
        for (event, class) in [
            (
                SshEvent::Failed(made_platform::ssh::classify_ssh_stderr(
                    b"@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @",
                )),
                Some(SshFailureClass::ChangedHostKey),
            ),
            (
                SshEvent::Failed(made_platform::ssh::classify_ssh_stderr(
                    b"Permission denied (publickey).",
                )),
                Some(SshFailureClass::AuthFailed),
            ),
            (SshEvent::Eof, None),
        ] {
            let browse = RemoteBrowse::open(A, 1).unwrap();
            let stream = Scripted {
                events: [event].into(),
                ..Scripted::default()
            };
            let mut connection = BrowseConnection::new(stream, browse);
            assert_eq!(connection.pump(Instant::now()), vec![BrowseEvent::Ended]);
            assert_eq!(connection.end_failure(), class);
            assert_eq!(connection.browse().phase(), BrowsePhase::Ended);
        }
    }

    #[test]
    fn the_connection_pumps_frames_without_blocking_and_ends_on_eof() {
        let browse = RemoteBrowse::open(A, 1).unwrap();
        let identity = browse.identity().clone();
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let opened = BrowseEnvelope::new(identity, 1, BrowseOperation::BrowseOpened { home: None });
        let bytes = encode_browse(&opened).unwrap();
        let (head, tail) = bytes.split_at(3);
        let stream = Scripted {
            written: written.clone(),
            events: [
                SshEvent::Data(head.to_vec()),
                SshEvent::Data(tail.to_vec()),
                SshEvent::Eof,
            ]
            .into(),
            ..Scripted::default()
        };
        let mut connection = BrowseConnection::new(stream, browse);
        let events = connection.pump(Instant::now());
        assert_eq!(
            events,
            vec![BrowseEvent::Opened { home: None }, BrowseEvent::Ended]
        );
        assert!(
            !written.lock().unwrap().is_empty(),
            "the open frame was sent"
        );
        assert_eq!(connection.browse().phase(), BrowsePhase::Ended);
        // Nothing more happens and nothing blocks.
        assert!(connection.pump(Instant::now()).is_empty());
    }
}

mod remote_launch {
    use made_app::open_workspace::{HostId, OpenWorkspace, SshHost};
    use made_app::{AppModel, AppOptions, TerminalTarget, WorkspaceLaunch};
    use made_remote::protocol::{AuthorityId, RemotePath};
    use std::path::PathBuf;

    #[test]
    fn a_remote_launch_refuses_local_targets_and_unknown_servers_and_starts_nothing() {
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let directory = RemotePath::new(b"/srv/app".to_vec()).unwrap();
        assert!(model
            .begin_remote_workspace_in(TerminalTarget::Shell, directory.clone())
            .is_err());
        let unknown = AuthorityId([0xEE; 16]);
        assert!(model
            .begin_remote_workspace_in(
                TerminalTarget::RemoteShell { authority: unknown },
                directory
            )
            .is_err());
        assert_eq!(model.workspace_count(), 0);
        assert!(model.is_picker(), "the screen stays open");
    }

    #[test]
    fn the_server_reported_or_typed_root_reroots_the_remote_tree() {
        let a = AuthorityId([3; 16]);
        let mut screen = OpenWorkspace::new(
            made_app::integrations::PRODUCT,
            vec![SshHost {
                id: a,
                initial_directory: None,
            }],
            Some(HostId::Ssh(a)),
            std::env::temp_dir(),
        );
        assert_eq!(screen.launch_target(), None);
        assert!(screen.set_remote_root(PathBuf::from("/home/deploy")));
        assert_eq!(screen.launch_target(), Some(PathBuf::from("/home/deploy")));
        assert!(screen.set_remote_selection(Some(PathBuf::from("/home/deploy/app"))));
        assert_eq!(
            screen.launch_target(),
            Some(PathBuf::from("/home/deploy/app"))
        );
        // A selection outside the root is refused.
        assert!(!screen.set_remote_selection(Some(PathBuf::from("/etc"))));
        assert!(screen.set_remote_root(PathBuf::from("/srv")));
        assert_eq!(
            screen.launch_target(),
            Some(PathBuf::from("/srv")),
            "re-rooting clears the selection"
        );
        screen.select_host(HostId::Local);
        assert!(
            !screen.set_remote_root(PathBuf::from("/srv")),
            "Local has no remote root"
        );
    }
}
