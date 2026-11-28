//! Headless model of the Open workspace screen (open-workspace-redesign,
//! phase 1): host selection, folder navigation handle, git and worktree
//! snapshot with generation, effective launch path and launch buttons.

use std::fs;
use std::path::{Path, PathBuf};

use made_app::integrations::{CLAUDE_CODE, GROK, OPENCODE, PRODUCT};
use made_app::open_workspace::{
    default_host, host_buttons, ConnectionState, CountsState, GitState, HostButton, HostId,
    LaunchKind, OpenWorkspace, SshHost, WorktreeItem,
};
use made_app::TerminalTarget;
use made_remote::git::ChangeCounts;
use made_remote::protocol::AuthorityId;

const A: AuthorityId = AuthorityId([1; 16]);
const B: AuthorityId = AuthorityId([2; 16]);

fn canonical(path: impl AsRef<Path>) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

/// `/work` with `made/crates`, `made-wt/feature-a`, `made-wt/fix` and `api`.
fn work_tree(temp: &tempfile::TempDir) -> PathBuf {
    let work = temp.path().join("work");
    for folder in [
        "made/crates",
        "made-wt/feature-a",
        "made-wt/fix",
        "api",
        "plain",
    ] {
        fs::create_dir_all(work.join(folder)).unwrap();
    }
    canonical(work)
}

fn hosts() -> Vec<SshHost> {
    vec![
        SshHost {
            id: A,
            initial_directory: Some(PathBuf::from("/srv/app")),
        },
        SshHost {
            id: B,
            initial_directory: None,
        },
    ]
}

fn screen(start: &Path) -> OpenWorkspace {
    OpenWorkspace::new(PRODUCT, hosts(), None, start)
}

fn row_index(model: &OpenWorkspace, path: &Path) -> usize {
    model
        .local_picker()
        .unwrap()
        .rows()
        .iter()
        .position(|row| row.path() == path)
        .unwrap_or_else(|| panic!("no row for {}", path.display()))
}

/// The repository of the worktree scenarios as the git reader reports it.
fn repository(work: &Path) -> GitState {
    let item = |path: &str, label: &str, missing: bool| WorktreeItem {
        path: work.join(path),
        label: Some(label.to_owned()),
        missing,
        counts: CountsState::Pending,
    };
    GitState::Repository {
        toplevel: work.join("made"),
        worktrees: vec![
            item("made", "main", false),
            item("made-wt/feature-a", "feature-a", false),
            item("made-wt/fix", "abcdef01", false),
        ],
        counts: CountsState::Pending,
    }
}

/// The same repository as read from inside a linked worktree's root.
fn repository_from(work: &Path, toplevel: &str) -> GitState {
    match repository(work) {
        GitState::Repository {
            worktrees, counts, ..
        } => GitState::Repository {
            toplevel: work.join(toplevel),
            worktrees,
            counts,
        },
        other => other,
    }
}

/// Apply `state` to the current git request, as a finished reader would.
fn complete_git(model: &mut OpenWorkspace, state: GitState) {
    let request = model.sync().expect("a target change issues a git request");
    assert_eq!(
        model.launch_target().as_deref(),
        Some(request.target.as_path())
    );
    assert!(model.apply_git(request.generation, state));
}

fn kinds(model: &OpenWorkspace) -> Vec<&'static str> {
    model
        .launch_buttons()
        .iter()
        .map(|button| button.kind.label())
        .collect()
}

#[test]
fn default_host_follows_the_active_tab() {
    let saved = [A, B];
    assert_eq!(default_host(Some(HostId::Ssh(A)), &saved), HostId::Ssh(A));
    assert_eq!(default_host(Some(HostId::Local), &saved), HostId::Local);
    assert_eq!(default_host(None, &saved), HostId::Local);
    let removed = AuthorityId([9; 16]);
    assert_eq!(
        default_host(Some(HostId::Ssh(removed)), &saved),
        HostId::Local
    );

    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let model = OpenWorkspace::new(PRODUCT, hosts(), Some(HostId::Ssh(A)), &work);
    assert_eq!(model.selected_host(), HostId::Ssh(A));
    let model = OpenWorkspace::new(PRODUCT, hosts(), Some(HostId::Ssh(removed)), &work);
    assert_eq!(model.selected_host(), HostId::Local);
}

#[test]
fn host_order_is_local_then_saved_order_then_add_host() {
    assert_eq!(
        host_buttons(&[B, A]),
        vec![
            HostButton::Host(HostId::Local),
            HostButton::Host(HostId::Ssh(B)),
            HostButton::Host(HostId::Ssh(A)),
            HostButton::AddHost,
        ]
    );
    assert_eq!(
        host_buttons(&[]),
        vec![HostButton::Host(HostId::Local), HostButton::AddHost]
    );
    let temp = tempfile::tempdir().unwrap();
    let model = screen(&work_tree(&temp));
    assert_eq!(model.host_buttons(), host_buttons(&[A, B]));
}

#[test]
fn launch_target_follows_selection_else_root() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    assert_eq!(model.selected_host(), HostId::Local);
    assert_eq!(model.launch_target(), Some(work.clone()));
    let api = row_index(&model, &work.join("api"));
    model.local_picker_mut().unwrap().select(api).unwrap();
    assert_eq!(model.launch_target(), Some(work.join("api")));
    // Re-rooting with Up clears the selection: the new root is the target.
    model.local_picker_mut().unwrap().go_up().unwrap();
    assert_eq!(model.launch_target(), Some(canonical(temp.path())));
}

#[test]
fn effective_launch_path_is_the_selected_worktree_else_the_target() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    // Pending git reads never delay or change the target.
    assert_eq!(model.effective_launch_path(), Some(work.join("made")));
    complete_git(&mut model, repository(&work));
    assert!(model.worktrees_shown());
    assert_eq!(
        model.worktree_selection(),
        Some(0),
        "the root is preselected"
    );
    assert_eq!(model.effective_launch_path(), Some(work.join("made")));
    assert!(model.select_worktree(1));
    assert_eq!(
        model.effective_launch_path(),
        Some(work.join("made-wt/feature-a"))
    );
    assert_eq!(model.launch_target(), Some(work.join("made")));
    // No list: the effective path stays the launch target.
    let plain = row_index(&model, &work.join("plain"));
    model.local_picker_mut().unwrap().select(plain).unwrap();
    complete_git(&mut model, GitState::NoRepository);
    assert!(!model.worktrees_shown());
    assert_eq!(model.effective_launch_path(), Some(work.join("plain")));
}

#[test]
fn worktree_selection_resets_on_target_root_and_host_change() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    complete_git(&mut model, repository(&work));
    assert!(model.select_worktree(2));

    // Another target: the old snapshot no longer applies.
    let api = row_index(&model, &work.join("api"));
    model.local_picker_mut().unwrap().select(api).unwrap();
    assert!(!model.worktrees_shown());
    assert_eq!(model.worktree_selection(), None);
    assert_eq!(model.effective_launch_path(), Some(work.join("api")));
    // The UI syncs every frame; the read for `api` drops the old snapshot.
    assert!(model.sync().is_some());

    // Back on the root: a fresh read preselects the root again.
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    complete_git(&mut model, repository(&work));
    assert_eq!(model.worktree_selection(), Some(0));
    assert!(model.select_worktree(1));

    // Root change with the same target path still resets.
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&work.join("made").to_string_lossy())
        .unwrap();
    assert_eq!(model.launch_target(), Some(work.join("made")));
    assert_eq!(model.worktree_selection(), None);
    complete_git(&mut model, repository(&work));
    assert_eq!(model.worktree_selection(), Some(0));
    assert!(model.select_worktree(2));

    // Host change resets too.
    model.select_host(HostId::Ssh(A));
    assert_eq!(model.worktree_selection(), None);
    assert!(!model.worktrees_shown());
    model.select_host(HostId::Local);
    assert_eq!(
        model.launch_target(),
        Some(work.clone()),
        "Local restarts at its start folder"
    );
    assert_eq!(model.worktree_selection(), None);
}

#[test]
fn a_linked_worktree_root_preselects_that_worktree_however_it_is_reached() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let feature = work.join("made-wt/feature-a");
    let expected_order = vec![
        work.join("made"),
        work.join("made-wt/feature-a"),
        work.join("made-wt/fix"),
    ];
    let check = |model: &mut OpenWorkspace| {
        assert_eq!(model.launch_target(), Some(feature.clone()));
        complete_git(model, repository_from(&work, "made-wt/feature-a"));
        assert!(model.worktrees_shown());
        let paths: Vec<_> = model
            .worktrees()
            .iter()
            .map(|item| item.path.clone())
            .collect();
        assert_eq!(paths, expected_order, "main first, then repository order");
        assert_eq!(model.worktree_selection(), Some(1), "not the main root");
        assert_eq!(model.effective_launch_path(), Some(feature.clone()));
    };

    // Pasted address.
    let mut model = screen(&work);
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&format!(" {}/ ", feature.display()))
        .unwrap();
    check(&mut model);

    // Tree selection after re-rooting at `made-wt`.
    let mut model = screen(&work);
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&work.join("made-wt").to_string_lossy())
        .unwrap();
    let index = row_index(&model, &feature);
    model.local_picker_mut().unwrap().select(index).unwrap();
    check(&mut model);

    // Up from a folder inside the linked worktree.
    fs::create_dir_all(feature.join("src")).unwrap();
    let mut model = screen(&work);
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&feature.join("src").to_string_lossy())
        .unwrap();
    model.local_picker_mut().unwrap().go_up().unwrap();
    check(&mut model);
}

#[test]
fn a_subdirectory_shows_no_list_and_keeps_its_own_target() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&work.join("made/crates").to_string_lossy())
        .unwrap();
    complete_git(&mut model, repository(&work));
    assert!(model.git_indicator());
    assert!(!model.worktrees_shown());
    assert_eq!(
        model.effective_launch_path(),
        Some(work.join("made/crates"))
    );
}

#[test]
fn a_missing_worktree_is_not_selectable_or_launched() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    let mut state = repository(&work);
    if let GitState::Repository { worktrees, .. } = &mut state {
        worktrees[2].missing = true;
    }
    complete_git(&mut model, state);
    assert!(model.worktrees()[2].missing);
    assert!(!model.select_worktree(2));
    assert_eq!(model.worktree_selection(), Some(0));
    assert_eq!(model.effective_launch_path(), Some(work.join("made")));
}

#[test]
fn stale_git_results_are_discarded() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    let old = model.sync().unwrap();
    let api = row_index(&model, &work.join("api"));
    model.local_picker_mut().unwrap().select(api).unwrap();
    let current = model.sync().unwrap();
    assert_ne!(old.generation, current.generation);
    assert!(!model.apply_git(old.generation, repository(&work)));
    assert!(!model.git_indicator());
    assert!(!model.apply_counts(
        old.generation,
        &work.join("made"),
        CountsState::Ready(ChangeCounts {
            staged: 1,
            ..ChangeCounts::default()
        })
    ));
    assert!(model.sync().is_none(), "no new request without a change");
}

#[test]
fn counts_apply_to_their_working_tree() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    let request = model.sync().unwrap();
    assert!(model.apply_git(request.generation, repository(&work)));
    let counts = ChangeCounts {
        untracked: 4,
        ..ChangeCounts::default()
    };
    assert!(model.apply_counts(
        request.generation,
        &work.join("made-wt/feature-a"),
        CountsState::Ready(counts)
    ));
    assert_eq!(model.worktrees()[1].counts, CountsState::Ready(counts));
    assert_eq!(model.worktrees()[0].counts, CountsState::Pending);
}

#[test]
fn terminal_first_then_detected_tuis_in_catalog_order() {
    let temp = tempfile::tempdir().unwrap();
    let mut model = screen(&work_tree(&temp));
    // Detection pending: Terminal alone.
    assert_eq!(kinds(&model), vec!["Terminal"]);
    // Detection order does not matter; catalog order does.
    model.set_detected(HostId::Local, vec![CLAUDE_CODE, GROK, OPENCODE]);
    assert_eq!(
        kinds(&model),
        vec!["Terminal", "OpenCode", "Grok Build", "Claude Code"]
    );
    model.set_detected(HostId::Local, vec![CLAUDE_CODE]);
    assert_eq!(kinds(&model), vec!["Terminal", "Claude Code"]);
    model.set_detected(HostId::Local, vec![GROK]);
    assert_eq!(kinds(&model), vec!["Terminal", "Grok Build"]);
    model.set_detected(HostId::Local, Vec::new());
    assert_eq!(
        kinds(&model),
        vec!["Terminal"],
        "no detected TUI shows Terminal alone"
    );
    assert!(model.launch_buttons().iter().all(|button| button.enabled));
}

#[test]
fn detection_of_one_host_is_never_shown_on_another() {
    let temp = tempfile::tempdir().unwrap();
    let mut model = screen(&work_tree(&temp));
    model.set_detected(HostId::Local, vec![OPENCODE, CLAUDE_CODE]);
    model.set_detected(HostId::Ssh(A), vec![GROK]);
    model.select_host(HostId::Ssh(A));
    assert_eq!(kinds(&model), vec!["Terminal", "Grok Build"]);
    model.select_host(HostId::Ssh(B));
    assert_eq!(kinds(&model), vec!["Terminal"]);
    model.select_host(HostId::Local);
    assert_eq!(kinds(&model), vec!["Terminal", "OpenCode", "Claude Code"]);
}

#[test]
fn other_agents_stay_out_of_the_launch_bar() {
    // The catalog is the only source of buttons: the product list has no
    // Codex or other agent, so nothing else can be offered.
    let temp = tempfile::tempdir().unwrap();
    let mut model = screen(&work_tree(&temp));
    model.set_detected(HostId::Local, PRODUCT.to_vec());
    let labels = kinds(&model);
    assert_eq!(labels.len(), 1 + PRODUCT.len());
    assert!(!labels
        .iter()
        .any(|label| label.to_lowercase().contains("codex")));
    // A catalog without Grok omits it even when "detected".
    let mut model = OpenWorkspace::new(
        &[OPENCODE, CLAUDE_CODE],
        hosts(),
        None,
        work_tree(&tempfile::tempdir().unwrap()),
    );
    model.set_detected(HostId::Local, vec![GROK, CLAUDE_CODE]);
    assert_eq!(kinds(&model), vec!["Terminal", "Claude Code"]);
}

#[test]
fn remote_launch_is_disabled_until_connected() {
    let temp = tempfile::tempdir().unwrap();
    let mut model = screen(&work_tree(&temp));
    model.set_detected(HostId::Ssh(A), vec![GROK]);
    model.select_host(HostId::Ssh(A));
    assert_eq!(model.launch_target(), Some(PathBuf::from("/srv/app")));
    for state in [
        ConnectionState::NotConnected,
        ConnectionState::Connecting,
        ConnectionState::HostKeyApprovalNeeded,
        ConnectionState::AuthenticationCancelled,
        ConnectionState::Failed("refused".to_owned()),
    ] {
        model.set_connection(A, state.clone());
        let buttons = model.launch_buttons();
        assert_eq!(buttons.len(), 2, "{state:?}");
        for button in &buttons {
            assert!(!button.enabled, "{state:?}");
            assert_eq!(button.disabled_reason, Some("Server is not connected"));
        }
        assert!(model.launch_request(LaunchKind::Terminal).is_none());
    }
    model.set_connection(A, ConnectionState::Connected);
    assert!(model.launch_buttons().iter().all(|button| button.enabled));
    let request = model.launch_request(LaunchKind::Tui(GROK)).unwrap();
    assert_eq!(request.host, HostId::Ssh(A));
    assert_eq!(request.kind, LaunchKind::Tui(GROK));
    assert_eq!(request.local_target(), None, "never a host process");
    assert_eq!(request.path, PathBuf::from("/srv/app"));
    // Local stays selectable while a server is connecting.
    model.set_connection(A, ConnectionState::Connecting);
    model.select_host(HostId::Local);
    assert_eq!(model.selected_host(), HostId::Local);
    assert!(model.launch_buttons().iter().all(|button| button.enabled));
}

#[test]
fn selecting_a_host_resets_navigation_to_its_start_folder() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    let api = row_index(&model, &work.join("api"));
    model.local_picker_mut().unwrap().select(api).unwrap();
    model.select_host(HostId::Ssh(A));
    assert!(model.local_picker().is_none());
    assert_eq!(model.remote_root(), Some(Path::new("/srv/app")));
    assert_eq!(model.launch_target(), Some(PathBuf::from("/srv/app")));
    // No initial directory: the root waits for the server's home.
    model.select_host(HostId::Ssh(B));
    assert_eq!(model.remote_root(), None);
    assert_eq!(model.launch_target(), None);
    assert!(model.launch_buttons().iter().all(|button| !button.enabled));
    model.select_host(HostId::Local);
    let picker = model.local_picker().unwrap();
    assert_eq!(picker.current_dir(), work.as_path());
    assert_eq!(picker.selection(), None);
}

#[test]
fn the_server_home_is_the_start_folder_only_without_an_initial_directory() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    // The record's initial directory wins over the reported home.
    model.select_host(HostId::Ssh(A));
    assert_eq!(
        model.apply_remote_home(Some(PathBuf::from("/home/me"))),
        Ok(())
    );
    assert_eq!(model.remote_root(), Some(Path::new("/srv/app")));
    // No initial directory: the reported home becomes the root.
    model.select_host(HostId::Ssh(B));
    assert_eq!(
        model.apply_remote_home(Some(PathBuf::from("/home/me"))),
        Ok(())
    );
    assert_eq!(model.remote_root(), Some(Path::new("/home/me")));
    assert_eq!(model.launch_target(), Some(PathBuf::from("/home/me")));
    // Neither: a contextual error, the root stays unknown, nothing launches.
    model.select_host(HostId::Ssh(B));
    let error = model.apply_remote_home(None).unwrap_err();
    assert!(error.contains("address bar"), "{error}");
    assert_eq!(model.remote_root(), None);
    assert!(model.launch_buttons().iter().all(|button| !button.enabled));
    // Local ignores a home report.
    model.select_host(HostId::Local);
    assert_eq!(
        model.apply_remote_home(Some(PathBuf::from("/home/me"))),
        Ok(())
    );
    assert_eq!(model.local_picker().unwrap().current_dir(), work.as_path());
}

#[test]
fn local_launch_requests_map_each_button_to_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    model.set_detected(HostId::Local, PRODUCT.to_vec());
    let api = row_index(&model, &work.join("api"));
    model.local_picker_mut().unwrap().select(api).unwrap();
    for (kind, target) in [
        (LaunchKind::Terminal, TerminalTarget::Shell),
        (LaunchKind::Tui(OPENCODE), TerminalTarget::Tui(OPENCODE)),
        (LaunchKind::Tui(GROK), TerminalTarget::Tui(GROK)),
        (
            LaunchKind::Tui(CLAUDE_CODE),
            TerminalTarget::Tui(CLAUDE_CODE),
        ),
    ] {
        let request = model.launch_request(kind).expect("enabled");
        assert_eq!(request.host, HostId::Local);
        assert_eq!(request.local_target(), Some(target));
        assert_eq!(request.path, work.join("api"));
    }
    // A program not detected on the selected host has no request.
    model.set_detected(HostId::Local, vec![GROK]);
    assert!(model.launch_request(LaunchKind::Tui(OPENCODE)).is_none());
}

#[test]
fn selecting_a_worktree_retargets_every_launch_button() {
    let temp = tempfile::tempdir().unwrap();
    let work = work_tree(&temp);
    let mut model = screen(&work);
    model.set_detected(HostId::Local, vec![OPENCODE, CLAUDE_CODE]);
    let made = row_index(&model, &work.join("made"));
    model.local_picker_mut().unwrap().select(made).unwrap();
    complete_git(&mut model, repository(&work));
    assert!(model.select_worktree(1));
    for button in model.launch_buttons() {
        let request = model.launch_request(button.kind).expect("enabled");
        assert_eq!(
            request.path,
            work.join("made-wt/feature-a"),
            "{}",
            button.kind.label()
        );
    }
}

#[test]
fn host_label_says_detection_pending_only_while_detection_can_arrive() {
    use made_app::app::ScreenHost;
    use made_app::open_workspace::{ConnectionState, HostId};
    use made_app::ui::host_accessible_label;
    let host = |connection| ScreenHost {
        id: HostId::Ssh(made_remote::protocol::AuthorityId([7; 16])),
        name: "lab".into(),
        address: "lab.example".into(),
        detected: None,
        connection: Some(connection),
    };
    for state in [
        ConnectionState::Failed("Permission denied".into()),
        ConnectionState::AuthenticationCancelled,
        ConnectionState::NotConnected,
    ] {
        let label = host_accessible_label(&host(state.clone()));
        assert!(!label.contains("detection pending"), "{state:?}: {label}");
    }
    for state in [
        ConnectionState::Connecting,
        ConnectionState::HostKeyApprovalNeeded,
    ] {
        let label = host_accessible_label(&host(state.clone()));
        assert!(label.contains("detection pending"), "{state:?}: {label}");
    }
}
