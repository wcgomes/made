//! Unavailable-root and asynchronous watcher behavior.

use std::fs;
use std::time::{Duration, Instant};

use made_app::diagnostics::DiagnosticLog;
use made_app::explorer_view::ExplorerPane;
use made_app::workbench::worktree_indicator;
use made_app::worktree::{MetadataStatus, WorktreeMetadata};
use made_explorer::root_switch::RootGeneration;

fn wait_for_root(pane: &mut ExplorerPane, log: &mut DiagnosticLog, deadline: Instant) {
    while Instant::now() < deadline {
        pane.tick(log);
        if pane.is_root_ready() && (!pane.watcher_loading() || pane.has_watcher()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "root did not become ready; error={:?} watcher_loading={} watcher_error={:?}",
        pane.root_error(),
        pane.watcher_loading(),
        pane.watcher_error()
    );
}

#[test]
fn initial_watcher_install_is_backgrounded_and_root_is_immediately_usable() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello").unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    let mut log = DiagnosticLog::new();
    let started = Instant::now();

    let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
    assert!(
        pane.watcher_loading() || pane.has_watcher(),
        "watcher installation should be pending or already installed"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "creating the explorer must not block on recursive watcher setup"
    );

    wait_for_root(&mut pane, &mut log, Instant::now() + Duration::from_secs(3));
    assert!(pane.has_watcher(), "watcher did not install");
    assert!(pane.watcher_error().is_none());
    assert!(pane.visible_row_count() >= 2);
}

#[test]
fn unavailable_root_banner_retains_the_last_tree_and_clears_on_recovery() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("one.txt"), "one").unwrap();
    fs::write(temp.path().join("two.txt"), "two").unwrap();
    let mut log = DiagnosticLog::new();
    let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
    wait_for_root(&mut pane, &mut log, Instant::now() + Duration::from_secs(3));
    let rows_before = pane.visible_row_count();
    assert!(rows_before >= 2);

    pane.mark_root_unavailable("the active session directory was removed");
    assert!(pane.root_unavailable().is_some());
    assert_eq!(
        pane.visible_row_count(),
        rows_before,
        "the last tree must remain visible under the banner"
    );

    pane.clear_root_unavailable();
    assert!(pane.root_unavailable().is_none());
    assert_eq!(pane.visible_row_count(), rows_before);
}

#[test]
fn worktree_metadata_unavailable_is_visibly_distinct() {
    let unavailable = WorktreeMetadata::unavailable("git missing");
    let unknown = WorktreeMetadata::unknown("not a repository");
    let known = WorktreeMetadata {
        status: MetadataStatus::Known,
        is_worktree: true,
        toplevel: Some("/tmp/worktree".into()),
        branch: Some("feature".to_owned()),
        detached: false,
        commit_abbrev: None,
        commit_subject: None,
        is_linked_worktree: true,
        directory_name: Some("worktree".to_owned()),
        worktree_label: Some("feature".to_owned()),
        git_dir: None,
        common_dir: None,
        worktree_count: 2,
        reason: None,
    };
    assert_eq!(
        worktree_indicator(Some(&unavailable), false),
        "git unavailable"
    );
    assert_eq!(
        worktree_indicator(Some(&unknown), false),
        "unknown git metadata"
    );
    assert_eq!(
        worktree_indicator(Some(&known), false),
        "linked worktree · feature"
    );
}
