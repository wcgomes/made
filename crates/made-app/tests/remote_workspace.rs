//! Pure controller/fake-transport evidence; not real SSH/TUI acceptance.
use made_app::remote_workspace::*;
use made_remote::protocol::{
    AuthorityId, Chunk, DirectoryEntry, EntryKind, ErrorCode, GitRead, GrokActivity, GrokEventKind,
    GrokSessionEvent, GrokSessionInfo, GrokSessionSnapshot, Identity, Operation, PreviewKind,
    RemotePath, TabCapability, WireEnvelope, MAX_IN_FLIGHT,
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

mod remote_claude_row {
    use made_app::diagnostics::DiagnosticsPill;
    use made_app::diagnostics::DiagnosticsPill::{Degraded, Inactive};
    use made_app::integrations::claude_code::diagnostics::{
        remote_claude_diagnostics_row, ClaudeDiagnosticsInputs, ClaudeDiagnosticsPhase,
        CLAUDE_GRACE,
    };
    use std::time::Duration;

    fn inputs(
        phase: ClaudeDiagnosticsPhase,
        present: bool,
        present_for: Duration,
        protocol_observed: bool,
    ) -> ClaudeDiagnosticsInputs {
        ClaudeDiagnosticsInputs {
            phase,
            // A remote tab has no plugin and no hook sessions; these never
            // change the remote row.
            plugin_missing: true,
            present,
            present_for,
            live_session: false,
            protocol_observed,
        }
    }

    fn row(inputs: ClaudeDiagnosticsInputs) -> (DiagnosticsPill, String) {
        let report = remote_claude_diagnostics_row(&inputs);
        (report.pill, report.sentence)
    }

    #[test]
    fn remote_states_use_the_tabs_own_detection_and_presence() {
        use ClaudeDiagnosticsPhase::{Available, Checking, NotOnPath};
        let young = Duration::from_secs(1);
        let old = CLAUDE_GRACE + Duration::from_secs(1);
        assert_eq!(
            row(inputs(Checking, false, Duration::ZERO, false)),
            (Inactive, "Checking for Claude Code.".to_owned())
        );
        assert_eq!(
            row(inputs(NotOnPath, false, Duration::ZERO, false)),
            (Inactive, "Claude Code is not on PATH.".to_owned())
        );
        assert_eq!(
            row(inputs(Available, false, Duration::ZERO, false)),
            (Inactive, "Claude Code is not running.".to_owned())
        );
        // A directly launched remote claude with no session-level emission.
        assert_eq!(
            row(inputs(Available, true, young, false)),
            (Inactive, "Claude Code is starting.".to_owned())
        );
        assert_eq!(
            row(inputs(Available, true, old, false)),
            (
                Degraded,
                "Claude Code status protocol is unavailable.".to_owned()
            )
        );
        // Emissions prove the protocol; remote hooks are permanently absent,
        // so the remote row never reaches a ready state.
        for present_for in [young, old] {
            assert_eq!(
                row(inputs(Available, true, present_for, true)),
                (
                    Degraded,
                    "Claude Code session-status hooks are unavailable over SSH.".to_owned()
                )
            );
        }
    }
}
