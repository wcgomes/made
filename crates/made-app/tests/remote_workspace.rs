//! Pure controller/fake-transport evidence; not real SSH/TUI acceptance.
use made_app::remote_workspace::*;
use made_remote::protocol::{
    AuthorityId, Chunk, DirectoryEntry, EntryKind, ErrorCode, GitRead, Identity, Operation,
    PreviewKind, RemotePath, TabCapability, WireEnvelope, MAX_IN_FLIGHT,
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
            Operation::SessionState => Operation::SessionSnapshot {
                snapshot: snapshot.clone(),
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
        ["Merge", "Staged", "Changes", "Untracked"]
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
    let command = workspace.next_command().unwrap();
    assert_eq!(command.request_id, id);
    let resolution = made_remote::protocol::RootResolution {
        identity: command.identity.clone(),
        request_id: id,
        result: Ok(path("/project/src")),
        generation: 2,
    };
    let result = workspace.accept_root(resolution).unwrap();
    assert!(matches!(result, Some(RemoteResult::Root(root)) if root == path("/project/src")));
    assert_eq!(workspace.identity().generation, 2);
    assert_eq!(workspace.root(), &path("/project/src"));
}
