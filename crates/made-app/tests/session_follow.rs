//! Session-follow tests: same/different worktree, main checkout, stale
//! revisions, missing directories, no active session, subpath, and fallback.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use made_app::session_follow::{
    FollowedSession, RootSource, SessionFollowCoordinator, SessionFollowUpdate,
};
use made_bridge::server::{BridgeEvent, ConnectionInfo, DisconnectedInfo, ProtocolError};
use made_bridge::state::{SessionInfo, SessionSnapshot};

fn fixture_dir(temp: &tempfile::TempDir, name: &str) -> PathBuf {
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("marker.txt"), name).unwrap();
    dir.canonicalize().unwrap()
}

fn session(id: &str, title: &str, directory: &Path, subpath: Option<&str>) -> SessionInfo {
    SessionInfo {
        session_id: id.to_owned(),
        title: title.to_owned(),
        active: false,
        busy: false,
        attention: false,
        unread: false,
        prompt_pulse: 0,
        renaming: false,
        directory: Some(directory.to_path_buf()),
        subpath: subpath.map(str::to_owned),
        project_id: None,
    }
}

fn snapshot(active: Option<&str>, sessions: Vec<SessionInfo>, revision: u64) -> SessionSnapshot {
    let mut sessions = sessions;
    for session in &mut sessions {
        session.active = active == Some(session.session_id.as_str());
    }
    SessionSnapshot {
        revision,
        active_session_id: active.map(str::to_owned),
        tabs_enabled: true,
        view_directory: None,
        sessions,
    }
}

fn snapshot_with_view(
    active: Option<&str>,
    sessions: Vec<SessionInfo>,
    view: Option<&Path>,
    revision: u64,
) -> SessionSnapshot {
    let mut value = snapshot(active, sessions, revision);
    value.view_directory = view.map(Path::to_path_buf);
    value
}

fn coordinator(temp: &tempfile::TempDir) -> (SessionFollowCoordinator, PathBuf, PathBuf) {
    let launch = fixture_dir(temp, "launch");
    let terminal = fixture_dir(temp, "terminal");
    let coordinator = SessionFollowCoordinator::new(&launch, &terminal).unwrap();
    (coordinator, launch, terminal)
}

#[test]
fn a_launch_plan_starts_follow_at_the_explorer_root() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = fixture_dir(&temp, "workspace");
    let plan = made_app::workspace::LaunchPlan::open(
        &workspace,
        made_app::workspace::WorkspaceSource::LaunchArgument,
    )
    .unwrap();
    let follow = SessionFollowCoordinator::from_launch_plan(&plan).unwrap();

    assert_eq!(follow.current_root(), plan.explorer_root.as_path());
    assert_eq!(
        follow.generation(),
        made_explorer::root_switch::RootGeneration::INITIAL
    );
    assert_eq!(follow.fallback_root(), plan.explorer_root);
    assert_eq!(
        follow.launch_workspace(),
        Some(plan.explorer_root.as_path())
    );
}

#[test]
fn same_worktree_switch_updates_identity_without_switching_root() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let (mut follow, _, _) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    assert!(matches!(first.update, SessionFollowUpdate::ActiveSession));
    assert!(first.root_switched);
    let root = first.root.clone();
    let generation = first.generation;

    let second = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![
            session("a", "A", &worktree, None),
            session("b", "B", &worktree, None),
        ],
        2,
    ));
    assert!(matches!(second.update, SessionFollowUpdate::ActiveSession));
    assert!(!second.root_switched);
    assert_eq!(second.root, root);
    assert_eq!(second.generation, generation);
    assert_eq!(second.active_session_id(), Some("b"));
    assert_eq!(follow.generation(), generation);
}

#[test]
fn different_worktree_switches_root_and_advances_generation() {
    let temp = tempfile::tempdir().unwrap();
    let first_dir = fixture_dir(&temp, "worktree-a");
    let second_dir = fixture_dir(&temp, "worktree-b");
    let (mut follow, _, _) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &first_dir, None)],
        1,
    ));
    let generation = first.generation;

    let second = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![
            session("a", "A", &first_dir, None),
            session("b", "B", &second_dir, None),
        ],
        2,
    ));
    assert!(matches!(second.update, SessionFollowUpdate::ActiveSession));
    assert!(second.root_switched);
    assert_eq!(second.root, fs::canonicalize(&second_dir).unwrap());
    assert_eq!(second.generation, generation.next());
    assert_eq!(
        follow.current_root(),
        fs::canonicalize(&second_dir).unwrap()
    );
}

#[test]
fn main_checkout_is_followed_because_it_is_the_active_directory() {
    let temp = tempfile::tempdir().unwrap();
    let main = fixture_dir(&temp, "main-checkout");
    let linked = fixture_dir(&temp, "linked-worktree");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(
        Some("linked"),
        vec![session("linked", "Linked", &linked, None)],
        1,
    ));
    let outcome = follow.apply_snapshot(snapshot(
        Some("main"),
        vec![
            session("linked", "Linked", &linked, None),
            session("main", "Main", &main, None),
        ],
        2,
    ));

    assert_eq!(outcome.root, fs::canonicalize(&main).unwrap());
    assert_eq!(outcome.active_session_id(), Some("main"));
    assert!(outcome.root_switched);
}

#[test]
fn rapid_switching_discards_stale_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let a = fixture_dir(&temp, "a");
    let b = fixture_dir(&temp, "b");
    let c = fixture_dir(&temp, "c");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(Some("a"), vec![session("a", "A", &a, None)], 1));
    let latest = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![session("a", "A", &a, None), session("b", "B", &b, None)],
        3,
    ));
    assert_eq!(latest.root, fs::canonicalize(&b).unwrap());

    let stale = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &a, None), session("b", "B", &b, None)],
        2,
    ));
    assert!(stale.stale_rejected());
    assert!(matches!(
        stale.update,
        SessionFollowUpdate::StaleRevision {
            revision: 2,
            last_accepted: 3
        }
    ));
    assert_eq!(stale.root, fs::canonicalize(&b).unwrap());
    assert_eq!(stale.active_session_id(), Some("b"));
    assert!(!stale.root_switched);

    let final_update = follow.apply_snapshot(snapshot(
        Some("c"),
        vec![
            session("a", "A", &a, None),
            session("b", "B", &b, None),
            session("c", "C", &c, None),
        ],
        4,
    ));
    assert_eq!(final_update.root, fs::canonicalize(&c).unwrap());
    assert_eq!(final_update.active_session_id(), Some("c"));
}

#[test]
fn missing_directory_retains_the_previous_root_and_reports_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let missing = temp.path().join("removed-worktree");
    let (mut follow, _, _) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    let root = first.root.clone();

    let outcome = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![
            session("a", "A", &worktree, None),
            session("b", "Removed", &missing, None),
        ],
        2,
    ));

    assert!(outcome.update.is_unavailable());
    assert_eq!(outcome.root, root);
    assert!(!outcome.root_switched);
    match &outcome.update {
        SessionFollowUpdate::ActiveDirectoryUnavailable {
            session_id,
            directory,
            detail,
        } => {
            assert_eq!(session_id, "b");
            assert_eq!(directory.as_deref(), Some(missing.as_path()));
            assert!(!detail.is_empty());
        }
        other => panic!("unexpected update: {other:?}"),
    }
    let active = outcome.active_session.expect("active identity retained");
    assert!(!active.available);
    assert_eq!(
        active.requested_directory.as_deref(),
        Some(missing.as_path())
    );
    assert!(active.canonical_directory.is_none());
}

#[test]
fn unavailable_directory_advances_generation_once_and_recovers_immediately() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let unavailable = temp.path().join("removed-worktree");
    let (mut follow, _, _) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    let root = first.root.clone();
    let generation = first.generation;
    assert_eq!(follow.unavailable_root(), None);

    let missing = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![session("b", "B", &unavailable, None)],
        2,
    ));
    assert!(missing.update.is_unavailable());
    assert_eq!(missing.root, root);
    assert!(!missing.root_switched);
    assert!(
        missing.generation > generation,
        "an unavailable effective root must advance the generation"
    );
    assert_eq!(follow.generation(), missing.generation);
    assert_eq!(
        follow.unavailable_root(),
        Some(unavailable.as_path()),
        "the unavailable identity is tracked while the last tree remains"
    );

    // Revisions for the same unavailable identity must not advance again.
    let mut still_missing = session("b", "Still missing", &unavailable, None);
    still_missing.busy = true;
    let repeated = follow.apply_snapshot(snapshot(Some("b"), vec![still_missing], 3));
    assert!(repeated.update.is_unavailable());
    assert_eq!(repeated.generation, missing.generation);

    // Recovery revalidates at the next snapshot for this identity.
    fixture_dir(&temp, "removed-worktree");
    let recovered = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![session("b", "Back", &unavailable, None)],
        4,
    ));
    assert!(matches!(
        recovered.update,
        SessionFollowUpdate::ActiveSession
    ));
    assert_eq!(recovered.root, fs::canonicalize(&unavailable).unwrap());
    assert!(recovered.root_switched);
    assert!(recovered.generation > missing.generation);
    assert_eq!(follow.unavailable_root(), None);
}

#[test]
fn releasing_after_an_unavailable_root_clears_unavailable_state_at_the_same_path() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let unavailable = temp.path().join("removed-worktree");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    follow.follow_shell_cwd(&worktree).unwrap();
    let current = follow.current_root().to_path_buf();
    let unavailable_outcome = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![session("b", "B", &unavailable, None)],
        2,
    ));
    assert!(unavailable_outcome.update.is_unavailable());
    assert!(follow.unavailable_root().is_some());

    let released = follow.release_active_session();
    assert!(matches!(
        released.update,
        SessionFollowUpdate::NoActiveSession
    ));
    assert_eq!(released.root, fs::canonicalize(&worktree).unwrap());
    assert_eq!(released.root, current);
    assert!(!released.root_switched);
    assert_eq!(released.generation, unavailable_outcome.generation);
    assert_eq!(follow.unavailable_root(), None);
    assert!(!follow.is_root_unavailable());
}

#[test]
fn foreign_lifecycle_events_do_not_release_the_authenticated_session() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let (mut follow, _, _) = coordinator(&temp);

    let connected = BridgeEvent::Connected(ConnectionInfo {
        connection_id: 7,
        pane_id: "pane".to_owned(),
        plugin_version: "0.1.0".to_owned(),
        opencode_version: "2.1.4".to_owned(),
        pid: 1,
        cwd: temp.path().to_path_buf(),
        protocol_version: 1,
    });
    follow.apply_bridge_event(&connected);
    let active = follow
        .apply_bridge_event(&BridgeEvent::State(snapshot(
            Some("a"),
            vec![session("a", "A", &worktree, None)],
            1,
        )))
        .expect("state event");
    assert_eq!(active.active_session_id(), Some("a"));
    let generation = active.generation;

    for event in [
        BridgeEvent::ProtocolError(ProtocolError {
            connection_id: Some(8),
            code: "invalid_token".to_owned(),
            message: "rejected".to_owned(),
            fatal: true,
        }),
        BridgeEvent::Disconnected(DisconnectedInfo {
            connection_id: 8,
            authenticated: false,
            code: Some(4401),
            reason: "foreign".to_owned(),
            replaced: false,
        }),
        BridgeEvent::Goodbye(made_bridge::server::Goodbye {
            connection_id: 8,
            reason: "foreign".to_owned(),
        }),
        BridgeEvent::HeartbeatTimeout(made_bridge::server::HeartbeatTimeout {
            connection_id: 8,
            missed_pongs: 2,
        }),
        BridgeEvent::ProtocolError(ProtocolError {
            connection_id: None,
            code: "invalid_hello".to_owned(),
            message: "unauthenticated".to_owned(),
            fatal: true,
        }),
    ] {
        assert!(
            follow.apply_bridge_event(&event).is_none(),
            "foreign event must not release the session"
        );
    }
    assert_eq!(
        follow.active_session().map(|s| s.session_id.as_str()),
        Some("a")
    );
    assert_eq!(follow.current_root(), worktree.as_path());
    assert_eq!(follow.generation(), generation);

    // The current connection still owns its lifecycle.
    let released = follow
        .apply_bridge_event(&BridgeEvent::Goodbye(made_bridge::server::Goodbye {
            connection_id: 7,
            reason: "normal".to_owned(),
        }))
        .expect("current goodbye");
    assert!(matches!(
        released.update,
        SessionFollowUpdate::NoActiveSession
    ));
}

#[test]
fn unreadable_directory_retains_the_previous_root() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let unreadable = temp.path().join("unreadable");
    fs::create_dir(&unreadable).unwrap();
    if !chmod(&unreadable, "000") {
        eprintln!("skipping unreadable-directory assertion: chmod is unavailable");
        return;
    }
    let still_readable = fs::read_dir(&unreadable).is_ok();
    let (mut follow, _, _) = coordinator(&temp);
    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    let root = first.root.clone();

    let outcome = follow.apply_snapshot(snapshot(
        Some("b"),
        vec![
            session("a", "A", &worktree, None),
            session("b", "Unreadable", &unreadable, None),
        ],
        2,
    ));
    let restored = chmod(&unreadable, "755");
    assert!(restored, "could not restore permissions");

    if still_readable {
        eprintln!(
            "skipping unreadable-directory assertion: the current user can read a mode-000 directory"
        );
        return;
    }
    assert!(outcome.update.is_unavailable());
    assert_eq!(outcome.root, root);
    assert!(!outcome.root_switched);
    assert!(!outcome.active_session.unwrap().available);
}

fn chmod(path: &Path, mode: &str) -> bool {
    std::process::Command::new("chmod")
        .arg(mode)
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[test]
fn busy_and_title_snapshots_reuse_the_cached_directory_validation() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "First title", &worktree, None)],
        1,
    ));
    assert_eq!(follow.directory_validation_count(), 1);
    let checked_at = follow.last_directory_check();

    let mut updated = session("a", "Second title", &worktree, None);
    updated.busy = true;
    updated.attention = true;
    let outcome = follow.apply_snapshot(snapshot(Some("a"), vec![updated], 2));

    assert!(matches!(outcome.update, SessionFollowUpdate::ActiveSession));
    assert_eq!(follow.directory_validation_count(), 1);
    assert_eq!(follow.last_directory_check(), checked_at);
    assert_eq!(
        outcome
            .active_session
            .as_ref()
            .map(|session| session.title.as_str()),
        Some("Second title")
    );
}

#[test]
fn unavailable_directory_result_is_cached_until_the_ttl_expires() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let (mut follow, _, _) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "Missing", &missing, None)],
        1,
    ));
    assert!(first.update.is_unavailable());
    assert_eq!(follow.directory_validation_count(), 1);

    let mut busy = session("a", "Still missing", &missing, None);
    busy.busy = true;
    let second = follow.apply_snapshot(snapshot(Some("a"), vec![busy], 2));
    assert!(second.update.is_unavailable());
    assert_eq!(follow.directory_validation_count(), 1);

    follow.set_directory_cache_ttl(Duration::ZERO);
    let third = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "Recheck", &missing, None)],
        3,
    ));
    assert!(third.update.is_unavailable());
    assert_eq!(follow.directory_validation_count(), 2);
}

#[test]
fn changing_the_active_directory_invalidates_the_cache() {
    let temp = tempfile::tempdir().unwrap();
    let first_dir = fixture_dir(&temp, "a");
    let second_dir = fixture_dir(&temp, "b");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &first_dir, None)],
        1,
    ));
    assert_eq!(follow.directory_validation_count(), 1);

    follow.apply_snapshot(snapshot(
        Some("b"),
        vec![
            session("a", "A", &first_dir, None),
            session("b", "B", &second_dir, None),
        ],
        2,
    ));
    assert_eq!(follow.directory_validation_count(), 2);
}

#[test]
fn no_active_session_returns_to_latest_shell_root() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let (mut follow, _, terminal) = coordinator(&temp);

    let first = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    let generation = first.generation;

    let none = follow.apply_snapshot(SessionSnapshot {
        revision: 2,
        active_session_id: None,
        tabs_enabled: true,
        view_directory: None,
        sessions: vec![session("a", "A", &worktree, None)],
    });

    assert!(matches!(none.update, SessionFollowUpdate::NoActiveSession));
    assert_eq!(none.root, terminal);
    assert!(none.generation > generation);
    assert!(none.active_session.is_none());
    assert!(none.root_switched);
}

#[test]
fn usable_view_directory_is_followed_without_an_active_session() {
    let temp = tempfile::tempdir().unwrap();
    let view = fixture_dir(&temp, "view");
    let shell = fixture_dir(&temp, "shell");
    let other = fixture_dir(&temp, "other-session");
    let (mut follow, _, terminal) = coordinator(&temp);

    assert_eq!(follow.follow_shell_cwd(&shell).unwrap().root, shell);
    let outcome = follow.apply_snapshot(snapshot_with_view(
        None,
        vec![session("inactive", "Inactive", &other, None)],
        Some(&view),
        1,
    ));

    assert!(
        matches!(outcome.update, SessionFollowUpdate::ViewDirectory),
        "unexpected update: {:?}",
        outcome.update
    );
    assert_eq!(outcome.root, fs::canonicalize(&view).unwrap());
    assert!(outcome.root_switched);
    assert!(outcome.active_session.is_none());
    assert_eq!(follow.effective_root_source(), RootSource::OpenCode);
    assert!(follow.is_agent_owned());
    let recorded = follow.view_directory().expect("view directory retained");
    assert_eq!(recorded.requested, view);
    assert!(recorded.available);
    assert_eq!(recorded.reason, None);

    // Shell reports are recorded but cannot switch the root while the view wins.
    let hidden_shell = follow.follow_shell_cwd(&terminal).unwrap();
    assert_eq!(hidden_shell.root, fs::canonicalize(&view).unwrap());
    assert!(!hidden_shell.root_switched);
    assert!(matches!(
        hidden_shell.update,
        SessionFollowUpdate::ViewDirectory
    ));
    assert_eq!(follow.terminal_cwd(), Some(terminal.as_path()));
}

#[test]
fn view_directory_change_switches_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let first = fixture_dir(&temp, "view-a");
    let second = fixture_dir(&temp, "view-b");
    let (mut follow, _, _) = coordinator(&temp);

    let initial = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&first), 1));
    let generation = initial.generation;
    let changed = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&second), 2));

    assert!(matches!(changed.update, SessionFollowUpdate::ViewDirectory));
    assert!(changed.root_switched);
    assert_eq!(changed.root, fs::canonicalize(&second).unwrap());
    assert_eq!(changed.generation, generation.next());
}

#[test]
fn active_session_directory_wins_over_a_view_directory() {
    let temp = tempfile::tempdir().unwrap();
    let active_dir = fixture_dir(&temp, "active");
    let missing_active = temp.path().join("missing-active");
    let view = fixture_dir(&temp, "view");
    let other_view = fixture_dir(&temp, "view-2");
    let (mut follow, _, _) = coordinator(&temp);

    let active = follow.apply_snapshot(snapshot_with_view(
        Some("a"),
        vec![session("a", "A", &active_dir, None)],
        Some(&view),
        1,
    ));
    assert!(matches!(active.update, SessionFollowUpdate::ActiveSession));
    assert_eq!(active.root, fs::canonicalize(&active_dir).unwrap());
    assert_eq!(follow.effective_root_source(), RootSource::OpenCode);

    // A view-directory change never replaces an active session root.
    let changed = follow.apply_snapshot(snapshot_with_view(
        Some("a"),
        vec![session("a", "A", &active_dir, None)],
        Some(&other_view),
        2,
    ));
    assert!(matches!(changed.update, SessionFollowUpdate::ActiveSession));
    assert!(!changed.root_switched);
    assert_eq!(changed.root, fs::canonicalize(&active_dir).unwrap());

    // An unavailable active directory is not silently replaced by the view.
    let unavailable = follow.apply_snapshot(snapshot_with_view(
        Some("b"),
        vec![session("b", "B", &missing_active, None)],
        Some(&view),
        3,
    ));
    assert!(unavailable.update.is_unavailable());
    assert_eq!(unavailable.root, active.root);
    assert!(!unavailable.root_switched);
    assert_eq!(
        follow.unavailable_root(),
        Some(missing_active.as_path()),
        "the unavailable active identity is retained"
    );
}

#[test]
fn missing_view_directory_falls_back_to_shell_with_a_reason() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-view");
    let shell = fixture_dir(&temp, "shell");
    let (mut follow, _, _) = coordinator(&temp);
    follow.follow_shell_cwd(&shell).unwrap();

    let outcome = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&missing), 1));
    match &outcome.update {
        SessionFollowUpdate::ViewDirectoryUnavailable { directory, detail } => {
            assert_eq!(directory, &missing);
            assert!(!detail.is_empty());
        }
        other => panic!("unexpected update: {other:?}"),
    }
    assert_eq!(outcome.root, shell);
    assert!(!outcome.root_switched);
    assert!(!follow.is_root_unavailable());
    assert_eq!(follow.unavailable_root(), None);
    assert_eq!(follow.effective_root_source(), RootSource::Shell);
    let recorded = follow.view_directory().expect("view directory retained");
    assert!(!recorded.available);
    assert!(recorded
        .reason
        .as_deref()
        .is_some_and(|reason| !reason.is_empty()));

    fixture_dir(&temp, "missing-view");
    let recovered = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&missing), 2));
    assert!(matches!(
        recovered.update,
        SessionFollowUpdate::ViewDirectory
    ));
    assert_eq!(recovered.root, fs::canonicalize(&missing).unwrap());
    assert!(recovered.root_switched);
    assert_eq!(follow.effective_root_source(), RootSource::OpenCode);
}

#[test]
fn unreadable_view_directory_falls_back_to_shell_without_unavailable_root() {
    let temp = tempfile::tempdir().unwrap();
    let shell = fixture_dir(&temp, "shell");
    let unreadable = temp.path().join("unreadable-view");
    fs::create_dir(&unreadable).unwrap();
    if !chmod(&unreadable, "000") {
        eprintln!("skipping unreadable-view assertion: chmod is unavailable");
        return;
    }
    let still_readable = fs::read_dir(&unreadable).is_ok();
    let (mut follow, _, _) = coordinator(&temp);
    follow.follow_shell_cwd(&shell).unwrap();

    let outcome = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&unreadable), 1));
    let restored = chmod(&unreadable, "755");
    assert!(restored, "could not restore permissions");

    if still_readable {
        eprintln!(
            "skipping unreadable-view assertion: the current user can read a mode-000 directory"
        );
        return;
    }
    match &outcome.update {
        SessionFollowUpdate::ViewDirectoryUnavailable { directory, detail } => {
            assert_eq!(directory, &unreadable);
            assert!(!detail.is_empty());
        }
        other => panic!("unexpected update: {other:?}"),
    }
    assert_eq!(outcome.root, shell);
    assert!(!outcome.root_switched);
    assert!(!follow.is_root_unavailable());
    assert_eq!(follow.unavailable_root(), None);
    assert_eq!(follow.effective_root_source(), RootSource::Shell);
    let recorded = follow.view_directory().expect("view directory retained");
    assert!(!recorded.available);
    assert_eq!(recorded.canonical, None);
    assert!(recorded
        .reason
        .as_deref()
        .is_some_and(|reason| !reason.is_empty()));
}

#[test]
fn tabs_disabled_clears_the_view_directory_and_keeps_the_shell_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let view = fixture_dir(&temp, "view");
    let shell = fixture_dir(&temp, "shell");
    let (mut follow, _, _) = coordinator(&temp);
    follow.follow_shell_cwd(&shell).unwrap();
    follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&view), 1));
    assert_eq!(follow.current_root(), fs::canonicalize(&view).unwrap());

    let mut disabled = snapshot_with_view(None, Vec::new(), Some(&view), 2);
    disabled.tabs_enabled = false;
    let outcome = follow.apply_snapshot(disabled);

    assert!(matches!(
        outcome.update,
        SessionFollowUpdate::NoActiveSession
    ));
    assert_eq!(outcome.root, shell);
    assert!(outcome.root_switched);
    assert!(follow.view_directory().is_none());
    assert_eq!(follow.effective_root_source(), RootSource::Shell);

    // An active session and a view directory are both ignored when tabs are off.
    let mut disabled_active = snapshot_with_view(
        Some("a"),
        vec![session("a", "A", &view, None)],
        Some(&view),
        3,
    );
    disabled_active.tabs_enabled = false;
    let outcome = follow.apply_snapshot(disabled_active);
    assert_eq!(outcome.root, shell);
    assert!(outcome.active_session.is_none());
    assert!(follow.view_directory().is_none());
}

#[test]
fn clearing_the_view_directory_resumes_shell_follow() {
    let temp = tempfile::tempdir().unwrap();
    let view = fixture_dir(&temp, "view");
    let shell = fixture_dir(&temp, "shell");
    let (mut follow, _, _) = coordinator(&temp);
    follow.follow_shell_cwd(&shell).unwrap();
    follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&view), 1));

    let cleared = follow.apply_snapshot(snapshot_with_view(None, Vec::new(), None, 2));
    assert!(matches!(
        cleared.update,
        SessionFollowUpdate::NoActiveSession
    ));
    assert_eq!(cleared.root, shell);
    assert!(cleared.root_switched);
    assert!(follow.view_directory().is_none());
    assert_eq!(follow.effective_root_source(), RootSource::Shell);
}

#[test]
fn lifecycle_events_and_a_new_epoch_clear_the_view_directory() {
    let temp = tempfile::tempdir().unwrap();
    let first_view = fixture_dir(&temp, "view-a");
    let second_view = fixture_dir(&temp, "view-b");
    let (mut follow, _, terminal) = coordinator(&temp);

    let connected = |connection_id| {
        BridgeEvent::Connected(ConnectionInfo {
            connection_id,
            pane_id: "pane".to_owned(),
            plugin_version: "0.1.0".to_owned(),
            opencode_version: "2.1.4".to_owned(),
            pid: 1,
            cwd: temp.path().to_path_buf(),
            protocol_version: 1,
        })
    };

    follow.apply_bridge_event(&connected(7));
    follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&first_view), 1));
    assert!(follow.view_directory().is_some());

    let goodbye = follow
        .apply_bridge_event(&BridgeEvent::Goodbye(made_bridge::server::Goodbye {
            connection_id: 7,
            reason: "normal exit".to_owned(),
        }))
        .expect("current goodbye");
    assert!(matches!(
        goodbye.update,
        SessionFollowUpdate::NoActiveSession
    ));
    assert_eq!(goodbye.root, terminal);
    assert!(follow.view_directory().is_none());

    follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&first_view), 2));
    assert!(follow.view_directory().is_some());
    let disconnected = follow
        .apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
            connection_id: 7,
            authenticated: true,
            code: Some(1000),
            reason: "normal exit".to_owned(),
            replaced: false,
        }))
        .expect("current disconnect");
    assert_eq!(disconnected.root, terminal);
    assert!(follow.view_directory().is_none());

    follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&first_view), 3));
    let reconnected = follow
        .apply_bridge_event(&connected(8))
        .expect("new connection");
    assert!(follow.view_directory().is_none());
    assert_eq!(reconnected.root, terminal);
    assert_eq!(follow.epoch(), 2);

    let outcome =
        follow.apply_snapshot(snapshot_with_view(None, Vec::new(), Some(&second_view), 1));
    assert!(matches!(outcome.update, SessionFollowUpdate::ViewDirectory));
    assert_eq!(outcome.root, fs::canonicalize(&second_view).unwrap());
}

#[test]
fn relative_view_directory_is_rejected_at_the_snapshot_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let (mut follow, _, _) = coordinator(&temp);

    let outcome = follow.apply_snapshot(snapshot_with_view(
        None,
        Vec::new(),
        Some(Path::new("relative/view")),
        1,
    ));

    match &outcome.update {
        SessionFollowUpdate::RejectedSnapshot { detail } => {
            assert!(
                detail.contains("viewDirectory"),
                "unexpected rejection detail: {detail}"
            );
        }
        other => panic!("expected RejectedSnapshot, got {other:?}"),
    }
    assert!(!outcome.root_switched);
    assert!(follow.view_directory().is_none());
}

#[test]
fn subpath_is_context_only_and_never_changes_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let subdir = worktree.join("src");
    fs::create_dir_all(&subdir).unwrap();
    let (mut follow, _, _) = coordinator(&temp);

    let outcome = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, Some("src"))],
        1,
    ));

    assert_eq!(outcome.root, fs::canonicalize(&worktree).unwrap());
    let active = outcome.active_session.unwrap();
    assert_eq!(active.subpath.as_deref(), Some("src"));
    assert_eq!(
        active.canonical_directory,
        Some(fs::canonicalize(&worktree).unwrap())
    );
}

#[test]
fn relative_directory_is_rejected_at_the_snapshot_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let (mut follow, _, _) = coordinator(&temp);

    let outcome = follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", Path::new("relative/worktree"), None)],
        1,
    ));

    // The bridge validates snapshots before they reach the follow coordinator.
    // A relative directory is a protocol violation, not an unavailable root, so
    // the coordinator rejects the whole snapshot and keeps the current root.
    match &outcome.update {
        SessionFollowUpdate::RejectedSnapshot { detail } => {
            assert!(
                detail.contains("not absolute"),
                "unexpected rejection detail: {detail}"
            );
        }
        other => panic!("expected RejectedSnapshot, got {other:?}"),
    }
    assert!(!outcome.root_switched);
}

#[test]
fn fallback_precedence_is_latest_shell_then_launch_never_old_agent() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "active-worktree");
    let (mut follow, launch, terminal) = coordinator(&temp);

    assert_eq!(follow.fallback_root(), fs::canonicalize(&terminal).unwrap());
    follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "A", &worktree, None)],
        1,
    ));
    assert_eq!(follow.fallback_root(), fs::canonicalize(&terminal).unwrap());

    fs::remove_dir_all(&worktree).unwrap();
    assert_eq!(follow.fallback_root(), fs::canonicalize(&terminal).unwrap());

    fs::remove_dir_all(&terminal).unwrap();
    assert_eq!(follow.fallback_root(), fs::canonicalize(&launch).unwrap());
}

#[test]
fn shell_updates_under_agent_precedence_and_every_release_returns_to_latest_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let a = fixture_dir(&temp, "shell-a");
    let b = fixture_dir(&temp, "agent-b");
    let c = fixture_dir(&temp, "shell-c");
    let (mut follow, _, _) = coordinator(&temp);
    assert_eq!(follow.follow_shell_cwd(&a).unwrap().root, a);
    let active = follow.apply_snapshot(snapshot(
        Some("agent"),
        vec![session("agent", "Agent", &b, None)],
        1,
    ));
    assert_eq!(active.root, b);
    let hidden_shell = follow.follow_shell_cwd(&c).unwrap();
    assert_eq!(hidden_shell.root, b);
    assert!(!hidden_shell.root_switched);
    assert_eq!(follow.terminal_cwd(), Some(c.as_path()));
    assert!(follow
        .follow_shell_cwd(&temp.path().join("missing"))
        .is_err());
    assert_eq!(follow.terminal_cwd(), Some(c.as_path()));
    let released = follow.release_active_session();
    assert_eq!(released.root, c);
    assert!(released.generation > active.generation);
    assert!(!follow.release_active_session().root_switched);
    follow.apply_snapshot(snapshot(
        Some("agent"),
        vec![session("agent", "Agent", &b, None)],
        2,
    ));
    let mut disabled = snapshot(Some("agent"), vec![session("agent", "Agent", &b, None)], 3);
    disabled.tabs_enabled = false;
    assert_eq!(follow.apply_snapshot(disabled).root, c);
    let mut no_directory = session("agent", "Agent", &b, None);
    no_directory.directory = None;
    assert_eq!(
        follow
            .apply_snapshot(snapshot(Some("agent"), vec![no_directory], 4))
            .root,
        c
    );
}

#[test]
fn a_new_connection_epoch_accepts_revision_one_again() {
    let temp = tempfile::tempdir().unwrap();
    let a = fixture_dir(&temp, "a");
    let b = fixture_dir(&temp, "b");
    let (mut follow, _, _) = coordinator(&temp);

    follow.apply_snapshot(snapshot(Some("a"), vec![session("a", "A", &a, None)], 5));
    assert_eq!(follow.last_revision(), Some(5));

    let connected = BridgeEvent::Connected(ConnectionInfo {
        connection_id: 7,
        pane_id: "pane".to_owned(),
        plugin_version: "0.1.0".to_owned(),
        opencode_version: "2.1.4".to_owned(),
        pid: 1,
        cwd: a.clone(),
        protocol_version: 1,
    });
    assert!(matches!(
        follow.apply_bridge_event(&connected).unwrap().update,
        SessionFollowUpdate::NoActiveSession
    ));
    assert_eq!(follow.epoch(), 1);

    let outcome = follow.apply_bridge_event(&BridgeEvent::State(snapshot(
        Some("b"),
        vec![session("b", "B", &b, None)],
        1,
    )));
    let outcome = outcome.expect("state event is applied");
    assert_eq!(outcome.active_session_id(), Some("b"));
    assert_eq!(outcome.root, fs::canonicalize(&b).unwrap());
}

#[test]
fn active_session_identity_exposes_directories_for_the_header() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = fixture_dir(&temp, "worktree");
    let (mut follow, _, _) = coordinator(&temp);
    follow.apply_snapshot(snapshot(
        Some("a"),
        vec![session("a", "Title", &worktree, None)],
        1,
    ));

    let active: &FollowedSession = follow.active_session().unwrap();
    assert_eq!(active.session_id, "a");
    assert_eq!(active.title, "Title");
    assert_eq!(
        active.directory(),
        Some(fs::canonicalize(&worktree).unwrap().as_path())
    );
    assert!(active.available);
}

#[test]
fn missing_launch_workspace_starts_at_the_terminal_directory() {
    let temp = tempfile::tempdir().unwrap();
    let terminal = fixture_dir(&temp, "terminal");
    let missing_launch = temp.path().join("missing-launch");
    let follow = SessionFollowCoordinator::new(&missing_launch, &terminal).unwrap();

    assert_eq!(
        follow.current_root(),
        fs::canonicalize(&terminal).unwrap().as_path()
    );
    assert_eq!(follow.launch_workspace(), None);
    assert_eq!(follow.fallback_root(), fs::canonicalize(&terminal).unwrap());
}

#[test]
fn coordinator_rejects_when_neither_fallback_is_usable() {
    let temp = tempfile::tempdir().unwrap();
    let missing_launch = temp.path().join("missing-launch");
    let missing_terminal = temp.path().join("missing-terminal");
    assert!(SessionFollowCoordinator::new(&missing_launch, &missing_terminal).is_err());
}
