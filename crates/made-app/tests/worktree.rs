//! Read-only worktree metadata tests with real temporary git repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use made_app::worktree::GitLimits;
use made_app::worktree::{
    is_read_only_git_args, ChangeKind, GitCommand, GitRunner, GitStatusRead, MetadataStatus,
    GIT_PROGRAM_ENV, GROUP_CHANGES, GROUP_MERGE, GROUP_STAGED, GROUP_UNTRACKED,
    READ_ONLY_GIT_ARGUMENTS, WORKTREE_MANAGEMENT_AVAILABLE,
};

fn run_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Made Test")
        .env("GIT_AUTHOR_EMAIL", "made@example.invalid")
        .env("GIT_COMMITTER_NAME", "Made Test")
        .env("GIT_COMMITTER_EMAIL", "made@example.invalid")
        .output()
        .expect("git must be installed for the worktree tests");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    run_git(root, &["init", "-q", "-b", "main"]);
    run_git(root, &["config", "user.name", "Made Test"]);
    run_git(root, &["config", "user.email", "made@example.invalid"]);
    fs::write(root.join("README.md"), "# fixture\n").unwrap();
    run_git(root, &["add", "README.md"]);
    run_git(root, &["commit", "-q", "-m", "initial"]);
}

fn runner() -> GitRunner {
    GitRunner::with_program("git")
}

#[test]
fn main_checkout_metadata_has_branch_and_toplevel() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);

    let metadata = runner().read_metadata(&main);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert!(metadata.is_worktree);
    assert_eq!(metadata.toplevel, Some(fs::canonicalize(&main).unwrap()));
    assert_eq!(metadata.branch.as_deref(), Some("main"));
    assert!(!metadata.detached);
    assert!(metadata.commit_abbrev.is_none());
    assert!(metadata.commit_subject.is_none());
    assert!(!metadata.is_linked_worktree);
    assert_eq!(metadata.worktree_label.as_deref(), Some("main"));
    assert_eq!(metadata.worktree_count, 1);
    assert!(metadata.is_known());
    assert!(metadata.header_label().contains("main"));
}

#[test]
fn linked_worktree_is_classified_from_read_only_git_commands() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    let linked = temp.path().join("linked");
    init_repo(&main);
    run_git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );

    let metadata = runner().read_metadata(&linked);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert!(metadata.is_worktree);
    assert_eq!(metadata.toplevel, Some(fs::canonicalize(&linked).unwrap()));
    assert_eq!(metadata.branch.as_deref(), Some("linked"));
    assert!(metadata.is_linked_worktree);
    assert_eq!(metadata.worktree_label.as_deref(), Some("linked"));
    assert_eq!(metadata.directory_name.as_deref(), Some("linked"));
    assert_eq!(metadata.worktree_count, 2);
    assert_ne!(metadata.git_dir, metadata.common_dir);
    assert!(metadata.header_label().contains("linked worktree"));
}

#[test]
fn a_subdirectory_resolves_to_its_worktree_root() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    let nested = main.join("src").join("nested");
    fs::create_dir_all(&nested).unwrap();

    let metadata = runner().read_metadata(&nested);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert_eq!(metadata.toplevel, Some(fs::canonicalize(&main).unwrap()));
    assert_eq!(metadata.branch.as_deref(), Some("main"));
    assert!(!metadata.is_linked_worktree);
    assert_eq!(metadata.git_dir, metadata.common_dir);
}

#[test]
fn non_git_directory_is_unknown_without_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();

    let metadata = runner().read_metadata(&plain);
    assert_eq!(metadata.status, MetadataStatus::Unknown);
    assert!(!metadata.is_worktree);
    assert!(metadata.toplevel.is_none());
    assert!(metadata.branch.is_none());
    assert!(!metadata.is_linked_worktree);
    assert!(metadata.reason.is_some());
    assert!(metadata.header_label().contains("unknown"));
}

#[test]
fn missing_git_binary_is_unavailable_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let missing = temp.path().join("missing-git-binary");

    let metadata = GitRunner::with_program(&missing).read_metadata(&plain);
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
    assert!(metadata
        .reason
        .as_deref()
        .unwrap()
        .contains("could not run git"));
    assert!(metadata.header_label().contains("unavailable"));
}

#[test]
fn the_git_binary_can_be_overridden_by_environment() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let missing = temp.path().join("env-missing-git");

    let previous = env::var_os(GIT_PROGRAM_ENV);
    env::set_var(GIT_PROGRAM_ENV, &missing);
    let configured = GitRunner::from_env();
    let metadata = configured.read_metadata(&plain);
    match previous {
        Some(value) => env::set_var(GIT_PROGRAM_ENV, value),
        None => env::remove_var(GIT_PROGRAM_ENV),
    }

    assert_eq!(configured.program(), missing.as_os_str());
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
}

#[test]
fn worktree_git_types_unify_with_the_shared_made_remote_contract() {
    // These parameters only accept the `made_remote::git` types. Passing the
    // app types proves they are re-exports, not a second implementation.
    fn remote_command(_: made_remote::git::GitCommand) {}
    fn remote_metadata(_: made_remote::git::WorktreeMetadata) {}
    fn remote_status(_: made_remote::git::GitStatusRead) {}

    remote_command(GitCommand::ShowToplevel);
    remote_metadata(made_app::worktree::WorktreeMetadata::unknown(
        "compatibility",
    ));
    remote_status(GitStatusRead::Failed {
        reason: String::new(),
    });
}

#[test]
fn git_argument_allowlist_contains_only_read_commands() {
    assert_eq!(GitCommand::ALL.len(), READ_ONLY_GIT_ARGUMENTS.len());
    for command in GitCommand::ALL {
        assert!(
            is_read_only_git_args(command.args()),
            "{} is not in the allowlist",
            command.label()
        );
        assert!(READ_ONLY_GIT_ARGUMENTS.contains(&command.args()));
    }

    let forbidden: &[&[&str]] = &[
        &["worktree", "add", "/tmp/x"],
        &["worktree", "remove", "/tmp/x"],
        &["worktree", "merge", "main"],
        &["worktree", "prune"],
        &["merge", "main"],
    ];
    for args in forbidden {
        assert!(!is_read_only_git_args(args));
    }
    const { assert!(!WORKTREE_MANAGEMENT_AVAILABLE) };
}

#[test]
fn detached_head_is_reported_without_a_branch_name() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    run_git(&main, &["checkout", "-q", "--detach"]);

    let metadata = runner().read_metadata(&main);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert!(metadata.detached);
    assert!(metadata.branch.is_none());
    assert!(metadata.header_label().contains("detached HEAD"));
}

#[test]
fn detached_metadata_reads_an_eight_character_id_and_subject() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    run_git(
        &main,
        &["commit", "--allow-empty", "-q", "-m", "Fix the header"],
    );
    run_git(&main, &["checkout", "-q", "--detach"]);

    let abbrev = run_git(&main, &["rev-parse", "--short=8", "HEAD"]);
    let metadata = runner().read_metadata(&main);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert!(metadata.detached);
    assert!(metadata.branch.is_none());
    assert_eq!(metadata.commit_abbrev.as_deref(), Some(abbrev.as_str()));
    let id: String = abbrev.chars().take(8).collect();
    assert_eq!(id.chars().count(), 8);
    assert!(abbrev.starts_with(&id));
    assert_eq!(metadata.commit_subject.as_deref(), Some("Fix the header"));
}

#[test]
fn empty_commit_subject_is_read_as_empty() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    run_git(
        &main,
        &[
            "commit",
            "--allow-empty",
            "--allow-empty-message",
            "-q",
            "-m",
            "",
        ],
    );
    run_git(&main, &["checkout", "-q", "--detach"]);

    let metadata = runner().read_metadata(&main);
    assert!(metadata.detached);
    assert!(metadata
        .commit_abbrev
        .as_deref()
        .is_some_and(|id| { id.chars().count() >= 8 }));
    assert_eq!(metadata.commit_subject.as_deref(), Some(""));
}

fn row_letters(snapshot: &made_app::worktree::GitStatusSnapshot, name: &str) -> Vec<char> {
    snapshot
        .groups
        .iter()
        .flat_map(|group| group.rows.iter())
        .filter(|row| row.relative.ends_with(name))
        .map(|row| row.letter)
        .collect()
}

#[test]
fn status_of_a_temp_repo_groups_changes_and_hides_ignored_files() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    fs::write(main.join("tracked.txt"), "one\n").unwrap();
    fs::write(main.join("gone.txt"), "gone\n").unwrap();
    run_git(&main, &["add", "tracked.txt", "gone.txt"]);
    run_git(&main, &["commit", "-q", "-m", "tracked"]);

    fs::write(main.join("tracked.txt"), "one\nstaged\n").unwrap();
    run_git(&main, &["add", "tracked.txt"]);
    fs::write(main.join("tracked.txt"), "one\nstaged\nunstaged\n").unwrap();
    fs::write(main.join("fresh.txt"), "new\n").unwrap();
    fs::write(main.join("skip.log"), "ignored\n").unwrap();
    fs::write(main.join(".gitignore"), "*.log\n").unwrap();
    fs::create_dir(main.join("vendor")).unwrap();
    fs::write(main.join("vendor").join("lib.txt"), "dep\n").unwrap();
    run_git(&main, &["mv", "README.md", "renamed.md"]);
    fs::remove_file(main.join("gone.txt")).unwrap();

    let canonical = fs::canonicalize(&main).unwrap();
    let GitStatusRead::Ready(snapshot) = runner().read_status(&main, Some(&canonical)) else {
        panic!("status should be ready");
    };
    let titles: Vec<_> = snapshot.groups.iter().map(|group| group.title).collect();
    assert!(titles.contains(&GROUP_STAGED));
    assert!(titles.contains(&GROUP_CHANGES));
    assert!(titles.contains(&GROUP_UNTRACKED));
    assert!(!titles.contains(&GROUP_MERGE));
    let partial = row_letters(&snapshot, "tracked.txt");
    assert!(partial.contains(&'M'));
    assert_eq!(partial.iter().filter(|letter| **letter == 'M').count(), 2);
    assert!(row_letters(&snapshot, "renamed.md").contains(&'R'));
    assert!(row_letters(&snapshot, "gone.txt").contains(&'D'));
    assert!(row_letters(&snapshot, "fresh.txt").contains(&'U'));
    assert!(snapshot.groups.iter().any(|group| {
        group.title == GROUP_UNTRACKED
            && group.rows.iter().any(|row| {
                row.relative.ends_with("lib.txt") && !row.is_directory && row.letter == 'U'
            })
    }));
    assert!(row_letters(&snapshot, "skip.log").is_empty());
    let vendor = snapshot
        .folders
        .get(&canonical.join("vendor"))
        .expect("untracked directory is a folder dot");
    assert_eq!(vendor.single(), Some(ChangeKind::New));
}

#[test]
fn new_files_in_tracked_and_untracked_directories_are_marked_and_ignored_paths_are_not() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    fs::create_dir(main.join("tracked")).unwrap();
    fs::create_dir(main.join("edited")).unwrap();
    fs::write(main.join("tracked").join("keep.txt"), "keep\n").unwrap();
    fs::write(main.join("edited").join("keep.txt"), "keep\n").unwrap();
    fs::write(main.join(".gitignore"), "*.log\n").unwrap();
    run_git(
        &main,
        &["add", "tracked/keep.txt", "edited/keep.txt", ".gitignore"],
    );
    run_git(&main, &["commit", "-q", "-m", "tracked dirs"]);
    fs::write(main.join("tracked").join("fresh.txt"), "new\n").unwrap();
    fs::create_dir(main.join("newdir")).unwrap();
    fs::write(main.join("newdir").join("inside.txt"), "nested\n").unwrap();
    fs::create_dir(main.join("ignored")).unwrap();
    fs::write(main.join("ignored").join("skip.log"), "secret\n").unwrap();
    fs::write(main.join("edited").join("keep.txt"), "changed\n").unwrap();
    fs::write(main.join("gone.txt"), "gone\n").unwrap();
    run_git(&main, &["add", "gone.txt"]);
    run_git(&main, &["commit", "-q", "-m", "gone"]);
    fs::remove_file(main.join("gone.txt")).unwrap();

    let canonical = fs::canonicalize(&main).unwrap();
    let GitStatusRead::Ready(snapshot) = runner().read_status(&main, Some(&canonical)) else {
        panic!("status should be ready");
    };
    let fresh = snapshot
        .groups
        .iter()
        .flat_map(|group| group.rows.iter())
        .find(|row| row.relative.ends_with("fresh.txt"))
        .expect("new file in a tracked directory");
    assert_eq!(fresh.letter, 'U');
    assert!(!fresh.is_directory);
    let inside = snapshot
        .groups
        .iter()
        .flat_map(|group| group.rows.iter())
        .find(|row| row.relative.ends_with("inside.txt"))
        .expect("file inside an untracked directory");
    assert_eq!(inside.letter, 'U');
    assert!(!inside.is_directory);
    assert!(row_letters(&snapshot, "skip.log").is_empty());
    assert!(row_letters(&snapshot, "keep.txt").contains(&'M'));
    assert!(row_letters(&snapshot, "gone.txt").contains(&'D'));
    assert_eq!(
        snapshot
            .folders
            .get(&canonical.join("tracked"))
            .expect("tracked parent")
            .single(),
        Some(ChangeKind::New)
    );
    assert_eq!(
        snapshot
            .folders
            .get(&canonical.join("newdir"))
            .expect("untracked parent")
            .single(),
        Some(ChangeKind::New)
    );
    assert_eq!(
        snapshot
            .folders
            .get(&canonical.join("edited"))
            .expect("modified parent")
            .single(),
        Some(ChangeKind::Modified)
    );
    assert!(!snapshot.folders.contains_key(&canonical.join("ignored")));
}

#[test]
fn subdirectory_status_uses_cwd_and_pathspec_dot() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    let sub = main.join("sub");
    fs::create_dir(&sub).unwrap();
    fs::create_dir(sub.join("nested")).unwrap();
    fs::write(sub.join("nested").join("inside.txt"), "in\n").unwrap();
    run_git(&main, &["add", "sub/nested/inside.txt"]);
    run_git(&main, &["commit", "-q", "-m", "inside"]);
    fs::write(main.join("outside.txt"), "out\n").unwrap();
    fs::write(sub.join("nested").join("inside.txt"), "changed\n").unwrap();
    fs::write(sub.join("extra.txt"), "extra\n").unwrap();

    let toplevel = fs::canonicalize(&main).unwrap();
    let sub_root = fs::canonicalize(&sub).unwrap();
    let GitStatusRead::Ready(snapshot) = runner().read_status(&sub_root, Some(&toplevel)) else {
        panic!("subdirectory status should be ready");
    };
    let relatives: Vec<_> = snapshot
        .groups
        .iter()
        .flat_map(|group| group.rows.iter().map(|row| row.relative.clone()))
        .collect();
    assert!(relatives.iter().any(|path| path.ends_with("inside.txt")));
    assert!(relatives.iter().any(|path| path.ends_with("extra.txt")));
    assert!(!relatives.iter().any(|path| path.ends_with("outside.txt")));
    assert!(snapshot
        .groups
        .iter()
        .flat_map(|group| group.rows.iter())
        .all(|row| row.path.starts_with(&sub_root)));
    let mark = snapshot
        .folders
        .get(&sub_root.join("nested"))
        .expect("collapsed nested folder is marked from the full status list");
    assert_eq!(mark.single(), Some(ChangeKind::Modified));
}

#[test]
fn conflicted_merge_is_grouped_as_merge_changes() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main");
    init_repo(&main);
    fs::write(main.join("clash.txt"), "base\n").unwrap();
    run_git(&main, &["add", "clash.txt"]);
    run_git(&main, &["commit", "-q", "-m", "base"]);
    run_git(&main, &["checkout", "-q", "-b", "other"]);
    fs::write(main.join("clash.txt"), "left\n").unwrap();
    run_git(&main, &["commit", "-q", "-am", "left"]);
    run_git(&main, &["checkout", "-q", "main"]);
    fs::write(main.join("clash.txt"), "right\n").unwrap();
    run_git(&main, &["commit", "-q", "-am", "right"]);
    let merge = Command::new("git")
        .arg("-C")
        .arg(&main)
        .args(["merge", "--no-commit", "--no-ff", "other"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        !merge.status.success(),
        "expected a conflict: {}",
        String::from_utf8_lossy(&merge.stderr)
    );

    let canonical = fs::canonicalize(&main).unwrap();
    let GitStatusRead::Ready(snapshot) = runner().read_status(&main, Some(&canonical)) else {
        panic!("conflict status should be ready");
    };
    assert_eq!(snapshot.groups[0].title, GROUP_MERGE);
    assert!(row_letters(&snapshot, "clash.txt").contains(&'C'));
    assert!(!snapshot.groups.iter().skip(1).any(|group| group
        .rows
        .iter()
        .any(|row| row.relative.ends_with("clash.txt"))));
}

#[test]
fn missing_directory_is_unknown_and_never_panics() {
    let temp = tempfile::tempdir().unwrap();
    let missing: PathBuf = temp.path().join("missing");
    let metadata = runner().read_metadata(&missing);
    assert_eq!(metadata.status, MetadataStatus::Unknown);
    assert!(metadata
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("cannot inspect")));
}

#[cfg(unix)]
fn script_in(directory: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn spawn_and_git_failures_keep_their_os_stderr_and_exit_details() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("plain");
    fs::create_dir(&directory).unwrap();
    let failing = script_in(
        temp.path(),
        "failing.sh",
        "#!/bin/sh\nprintf boom >&2\nexit 3\n",
    );

    let metadata = GitRunner::with_program(&failing).read_metadata(&directory);
    assert_eq!(metadata.status, MetadataStatus::Unknown);
    assert_eq!(
        metadata.reason.as_deref(),
        Some("not a git repository or git failed: boom")
    );
    let status = GitRunner::with_program(&failing).read_status(&directory, None);
    assert_eq!(
        status,
        GitStatusRead::Failed {
            reason: "git exited with 3: boom".to_owned()
        }
    );

    let missing = temp.path().join("missing-git-binary");
    let metadata = GitRunner::with_program(&missing).read_metadata(&directory);
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
    let reason = metadata.reason.as_deref().unwrap();
    assert!(reason.contains("could not run git"));
    assert!(
        reason.to_lowercase().contains("no such file")
            || reason.to_lowercase().contains("cannot find"),
        "the spawn OS reason is preserved: {reason}"
    );

    let non_executable = script_in(temp.path(), "non-executable.sh", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();
    let metadata = GitRunner::with_program(&non_executable).read_metadata(&directory);
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
    assert!(metadata
        .reason
        .as_deref()
        .is_some_and(|reason| reason.to_lowercase().contains("permission denied")));
}

#[cfg(unix)]
#[test]
fn local_runner_keeps_unbounded_output_and_deadline_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let large = script_in(
        temp.path(),
        "large-status.sh",
        "#!/bin/sh\ncase \"$*\" in *--get-regexp*) exit 1 ;; esac\ndd if=/dev/zero bs=2097153 count=1 2>/dev/null | tr '\\0' 'a'\n",
    );
    let slow = script_in(
        temp.path(),
        "slow-status.sh",
        "#!/bin/sh\ncase \"$*\" in *--get-regexp*) exit 1 ;; esac\nsleep 0.3\nprintf '?? local.txt\\0'\n",
    );

    let local = GitRunner::with_program(&large);
    assert_eq!(local.limits(), GitLimits::UNBOUNDED);
    assert!(matches!(
        local.read_status(temp.path(), None),
        GitStatusRead::Ready(_)
    ));

    let remote = made_remote::git::GitRunner::with_limits(
        &slow,
        made_remote::git::GitLimits {
            deadline: Some(Duration::from_millis(50)),
            max_output_bytes: None,
        },
    );
    assert!(matches!(
        remote.read_status(temp.path(), None),
        GitStatusRead::Failed { reason } if reason.contains("deadline")
    ));
    let GitStatusRead::Ready(snapshot) =
        GitRunner::with_program(&slow).read_status(temp.path(), None)
    else {
        panic!("the local compatibility runner must not inherit the remote deadline");
    };
    assert!(snapshot
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .any(|row| row.relative.ends_with("local.txt")));
}

// open-workspace-redesign: the Open workspace screen's bounded, off-thread,
// read-only worktree list and change counts.

use made_app::open_workspace::{CountsState, GitState, WorktreeItem};
use made_app::worktree::{ChangeCounts, WorktreeReadUpdate, WorktreeReads};

/// A repository at `made` with linked worktrees `made-wt/feature-a` (branch
/// `feature-a`) and `made-wt/fix` (detached). Returns canonical paths.
fn repository_with_worktrees(temp: &tempfile::TempDir) -> (PathBuf, PathBuf, PathBuf) {
    let main = temp.path().join("made");
    init_repo(&main);
    fs::create_dir_all(main.join("crates")).unwrap();
    let feature = temp.path().join("made-wt").join("feature-a");
    let fix = temp.path().join("made-wt").join("fix");
    run_git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature-a",
            feature.to_str().unwrap(),
        ],
    );
    run_git(
        &main,
        &["worktree", "add", "-q", "--detach", fix.to_str().unwrap()],
    );
    (
        fs::canonicalize(main).unwrap(),
        fs::canonicalize(feature).unwrap(),
        fs::canonicalize(fix).unwrap(),
    )
}

fn worktree_paths(state: &GitState) -> Vec<PathBuf> {
    match state {
        GitState::Repository { worktrees, .. } => {
            worktrees.iter().map(|item| item.path.clone()).collect()
        }
        GitState::NoRepository => Vec::new(),
    }
}

fn toplevel(state: &GitState) -> Option<&Path> {
    match state {
        GitState::Repository { toplevel, .. } => Some(toplevel),
        GitState::NoRepository => None,
    }
}

fn head_commit(directory: &Path) -> String {
    run_git(directory, &["rev-parse", "HEAD"])
}

#[test]
fn repository_root_lists_main_first_then_linked_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, fix) = repository_with_worktrees(&temp);
    let state = runner().read_open_workspace_target(&main);
    assert_eq!(toplevel(&state), Some(main.as_path()));
    let GitState::Repository {
        worktrees, counts, ..
    } = &state
    else {
        panic!("a repository");
    };
    let short: String = head_commit(&fix).chars().take(8).collect();
    assert_eq!(
        worktrees,
        &vec![
            WorktreeItem {
                path: main.clone(),
                label: Some("main".to_owned()),
                missing: false,
                counts: CountsState::Pending,
            },
            WorktreeItem {
                path: feature,
                label: Some("feature-a".to_owned()),
                missing: false,
                counts: CountsState::Pending,
            },
            WorktreeItem {
                path: fix,
                label: Some(short),
                missing: false,
                counts: CountsState::Pending,
            },
        ]
    );
    assert_eq!(*counts, CountsState::Pending);
}

#[test]
fn a_linked_worktree_root_reads_the_same_list_as_the_main_root() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, _) = repository_with_worktrees(&temp);
    let from_main = runner().read_open_workspace_target(&main);
    let from_linked = runner().read_open_workspace_target(&feature);
    assert_eq!(toplevel(&from_linked), Some(feature.as_path()));
    assert_eq!(worktree_paths(&from_linked), worktree_paths(&from_main));
    assert_eq!(worktree_paths(&from_linked)[0], main, "main first");
    match (&from_main, &from_linked) {
        (GitState::Repository { worktrees: a, .. }, GitState::Repository { worktrees: b, .. }) => {
            assert_eq!(a, b)
        }
        _ => panic!("both are repositories"),
    }
}

#[test]
fn subdirectory_single_worktree_and_non_repository_targets() {
    let temp = tempfile::tempdir().unwrap();
    let (main, _, _) = repository_with_worktrees(&temp);
    // A subdirectory reports its working tree's top level, not itself.
    let sub = runner().read_open_workspace_target(&main.join("crates"));
    assert_eq!(toplevel(&sub), Some(main.as_path()));

    let single = temp.path().join("single");
    init_repo(&single);
    let single = fs::canonicalize(single).unwrap();
    let state = runner().read_open_workspace_target(&single);
    assert_eq!(toplevel(&state), Some(single.as_path()));
    assert_eq!(worktree_paths(&state), vec![single.clone()]);

    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(
        runner().read_open_workspace_target(&plain),
        GitState::NoRepository
    );
    assert_eq!(
        GitRunner::with_program(temp.path().join("missing-git")).read_open_workspace_target(&main),
        GitState::NoRepository,
        "git missing degrades to no indicator"
    );
    assert_eq!(
        GitRunner::with_program(temp.path().join("missing-git")).read_change_counts(&main),
        CountsState::Failed
    );
}

#[test]
fn a_deleted_linked_worktree_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, _) = repository_with_worktrees(&temp);
    fs::remove_dir_all(&feature).unwrap();
    let GitState::Repository { worktrees, .. } = runner().read_open_workspace_target(&main) else {
        panic!("a repository");
    };
    assert_eq!(worktrees.len(), 3);
    assert!(!worktrees[0].missing);
    assert!(worktrees[1].missing, "{worktrees:?}");
    assert!(!worktrees[2].missing);
}

#[test]
fn change_counts_cover_the_whole_working_tree_with_the_grouped_classification() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, _) = repository_with_worktrees(&temp);
    fs::write(main.join(".gitignore"), "*.log\n").unwrap();
    fs::write(main.join("a.txt"), "a\n").unwrap();
    fs::write(main.join("crates").join("b.txt"), "b\n").unwrap();
    run_git(&main, &["add", ".gitignore", "a.txt", "crates/b.txt"]);
    run_git(&main, &["commit", "-q", "-m", "tracked"]);
    // Partially staged: once staged, once modified.
    fs::write(main.join("a.txt"), "a2\n").unwrap();
    run_git(&main, &["add", "a.txt"]);
    fs::write(main.join("a.txt"), "a3\n").unwrap();
    // A change outside the `crates` subfolder still counts.
    fs::write(main.join("README.md"), "changed\n").unwrap();
    fs::write(main.join("crates").join("b.txt"), "b2\n").unwrap();
    fs::write(main.join("build.log"), "ignored\n").unwrap();
    for index in 0..4 {
        fs::write(feature.join(format!("new-{index}.txt")), "n\n").unwrap();
    }

    assert_eq!(
        runner().read_change_counts(&main),
        CountsState::Ready(ChangeCounts {
            conflicted: 0,
            staged: 1,
            modified: 3,
            untracked: 0,
        })
    );
    assert_eq!(
        runner().read_change_counts(&feature),
        CountsState::Ready(ChangeCounts {
            untracked: 4,
            ..ChangeCounts::default()
        })
    );
    let clean = temp.path().join("clean");
    init_repo(&clean);
    assert_eq!(
        runner().read_change_counts(&clean),
        CountsState::Ready(ChangeCounts::default())
    );
}

/// Updates of `generation` until its target and `counts` count updates came.
fn collect(reads: &mut WorktreeReads, counts: usize) -> Vec<WorktreeReadUpdate> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut updates = Vec::new();
    loop {
        updates.extend(reads.poll());
        let targets = updates
            .iter()
            .filter(|update| matches!(update, WorktreeReadUpdate::Target { .. }))
            .count();
        let counted = updates
            .iter()
            .filter(|update| matches!(update, WorktreeReadUpdate::Counts { .. }))
            .count();
        if targets == 1 && counted == counts {
            return updates;
        }
        assert!(std::time::Instant::now() < deadline, "{updates:?}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn off_thread_reads_count_every_listed_worktree_or_only_the_toplevel() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, fix) = repository_with_worktrees(&temp);
    let mut reads = WorktreeReads::new(runner());
    reads.request(1, main.clone());
    let updates = collect(&mut reads, 3);
    let mut counted: Vec<_> = updates
        .iter()
        .filter_map(|update| match update {
            WorktreeReadUpdate::Counts {
                generation, path, ..
            } => {
                assert_eq!(*generation, 1);
                Some(path.clone())
            }
            WorktreeReadUpdate::Target { .. } => None,
        })
        .collect();
    counted.sort();
    let mut expected = vec![main.clone(), feature, fix];
    expected.sort();
    assert_eq!(counted, expected);

    // A subdirectory shows no list: only its working tree is counted.
    reads.request(2, main.join("crates"));
    let updates = collect(&mut reads, 1);
    assert!(updates.iter().any(|update| matches!(
        update,
        WorktreeReadUpdate::Counts { generation: 2, path, .. } if *path == main
    )));
}

#[cfg(unix)]
#[test]
fn stale_generations_are_discarded() {
    let temp = tempfile::tempdir().unwrap();
    let (main, _, _) = repository_with_worktrees(&temp);
    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    // Reads of the repository are slow; the plain folder answers at once.
    let slow_git = script_in(
        temp.path(),
        "slow-git",
        &format!(
            "#!/bin/sh\nprev=\nfor arg in \"$@\"; do [ \"$prev\" = -C ] && dir=$arg; prev=$arg; done\ncase \"$dir\" in {main}|{main}/*) sleep 1 ;; esac\nexec git \"$@\"\n",
            main = main.display()
        ),
    );
    let mut reads = WorktreeReads::new(GitRunner::with_program(&slow_git));
    reads.request(1, main.clone());
    reads.request(2, plain.clone());
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut updates = Vec::new();
    while std::time::Instant::now() < deadline {
        updates.extend(reads.poll());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        updates,
        vec![WorktreeReadUpdate::Target {
            generation: 2,
            state: GitState::NoRepository,
        }],
        "the late repository read is discarded"
    );
}

#[cfg(unix)]
#[test]
fn the_screen_reads_issue_only_read_only_git_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let (main, feature, _) = repository_with_worktrees(&temp);
    let log = temp.path().join("git-args.log");
    let logging_git = script_in(
        temp.path(),
        "logging-git",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec git \"$@\"\n",
            log.display()
        ),
    );
    let runner = GitRunner::with_program(&logging_git);
    let mut reads = WorktreeReads::new(runner.clone());
    reads.request(1, main.clone());
    collect(&mut reads, 3);
    reads.request(2, feature.clone());
    collect(&mut reads, 3);
    let _ = runner.read_open_workspace_target(&main.join("crates"));
    let _ = runner.read_change_counts(&main);

    let logged = fs::read_to_string(&log).unwrap();
    assert!(!logged.is_empty());
    for line in logged.lines() {
        // The forced read-only configuration (and, for status, the disabled
        // filter drivers), `-C <directory>`, then the fixed argument vector.
        let (settings, rest) = line.split_once(" -C ").expect(line);
        assert!(
            settings.starts_with(
                "-c core.fsmonitor=false -c log.showSignature=false -c protocol.allow=never "
            ),
            "{line}"
        );
        assert!(settings
            .strip_prefix("-c ")
            .expect(line)
            .split(" -c ")
            .all(|setting| !setting.is_empty() && !setting.contains(' ')));
        let args = READ_ONLY_GIT_ARGUMENTS
            .iter()
            .find(|args| rest.ends_with(&format!(" {}", args.join(" "))));
        assert!(args.is_some(), "not a read-only git invocation: {line}");
    }
    assert!(logged.contains("worktree list --porcelain"));
    assert!(logged.contains("status --porcelain=v1"));
    assert!(logged.contains("config --show-scope --name-only --get-regexp"));
}

/// Drive the screen model with the real reader until the target and its
/// counts are applied.
fn read_into(model: &mut made_app::open_workspace::OpenWorkspace, reads: &mut WorktreeReads) {
    let request = model.sync().expect("target changed");
    reads.request(request.generation, request.target);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        for update in reads.poll() {
            match update {
                WorktreeReadUpdate::Target { generation, state } => {
                    model.apply_git(generation, state);
                }
                WorktreeReadUpdate::Counts {
                    generation,
                    path,
                    counts,
                } => {
                    model.apply_counts(generation, &path, counts);
                }
            }
        }
        let pending = match model.git_state() {
            None => true,
            Some(GitState::NoRepository) => false,
            Some(GitState::Repository {
                counts, worktrees, ..
            }) => {
                if model.worktrees_shown() {
                    worktrees
                        .iter()
                        .any(|item| !item.missing && item.counts == CountsState::Pending)
                } else {
                    *counts == CountsState::Pending
                }
            }
        };
        if !pending {
            return;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn the_screen_preselects_the_navigated_working_tree_from_real_reads() {
    use made_app::open_workspace::OpenWorkspace;

    let temp = tempfile::tempdir().unwrap();
    let (main, feature, _) = repository_with_worktrees(&temp);
    fs::write(feature.join("new.txt"), "n\n").unwrap();
    let mut reads = WorktreeReads::new(runner());

    // Repository root: the root is preselected and is the effective path.
    let mut model = OpenWorkspace::new(made_app::integrations::PRODUCT, Vec::new(), None, &main);
    read_into(&mut model, &mut reads);
    assert!(model.git_indicator() && model.worktrees_shown());
    assert_eq!(model.worktree_selection(), Some(0));
    assert_eq!(model.effective_launch_path(), Some(main.clone()));
    assert_eq!(
        model.worktrees()[1].counts,
        CountsState::Ready(ChangeCounts {
            untracked: 1,
            ..ChangeCounts::default()
        })
    );
    assert_eq!(
        model.worktrees()[0].counts,
        CountsState::Ready(ChangeCounts::default())
    );

    // Pasted linked worktree root: that worktree, not the main root.
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&feature.to_string_lossy())
        .unwrap();
    read_into(&mut model, &mut reads);
    assert!(model.worktrees_shown());
    assert_eq!(model.worktrees()[0].path, main);
    assert_eq!(model.worktree_selection(), Some(1));
    assert_eq!(model.effective_launch_path(), Some(feature.clone()));

    // Subdirectory: indicator and the working tree's counts, no list.
    model
        .local_picker_mut()
        .unwrap()
        .navigate_to(&main.join("crates").to_string_lossy())
        .unwrap();
    read_into(&mut model, &mut reads);
    assert!(model.git_indicator() && !model.worktrees_shown());
    assert_eq!(model.effective_launch_path(), Some(main.join("crates")));
    assert!(matches!(
        model.git_state(),
        Some(GitState::Repository {
            counts: CountsState::Ready(_),
            ..
        })
    ));
}

#[cfg(unix)]
#[test]
fn rapid_target_changes_run_one_git_process_at_a_time() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("runs.log");
    // Every read takes a while and logs when it starts and ends.
    let slow_git = script_in(
        temp.path(),
        "slow-git",
        &format!(
            "#!/bin/sh\necho start >> '{log}'\nsleep 0.2\necho end >> '{log}'\nexit 1\n",
            log = log.display()
        ),
    );
    let mut reads = WorktreeReads::new(GitRunner::with_program(&slow_git));
    // Holding Down: one target change per frame.
    let requesting = std::time::Instant::now();
    for generation in 1..=30 {
        reads.request(generation, temp.path().join(format!("dir-{generation}")));
        std::thread::sleep(Duration::from_millis(5));
    }
    // Each read holds the single worker for at least 200 ms, so a latest-wins
    // slot starts at most one read per 200 ms while requests arrive, plus the
    // final one. A loaded runner stretches the request loop, not this bound's
    // meaning: a backlog would still start one read per request.
    let span = requesting.elapsed().as_millis();
    let max_runs = usize::try_from(span / 200 + 2).unwrap();
    assert!(max_runs < 30, "the request loop took {span} ms");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut updates = Vec::new();
    while !updates
        .iter()
        .any(|update| matches!(update, WorktreeReadUpdate::Target { generation: 30, .. }))
    {
        assert!(std::time::Instant::now() < deadline, "{updates:?}");
        updates.extend(reads.poll());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        updates,
        vec![WorktreeReadUpdate::Target {
            generation: 30,
            state: GitState::NoRepository,
        }]
    );
    // Give stray workers, if any, time to log.
    std::thread::sleep(Duration::from_millis(500));
    let logged = fs::read_to_string(&log).unwrap();
    let (mut running, mut peak, mut runs) = (0i32, 0i32, 0);
    for line in logged.lines() {
        if line == "start" {
            running += 1;
            runs += 1;
            peak = peak.max(running);
        } else {
            running -= 1;
        }
    }
    assert_eq!(peak, 1, "git reads overlapped:\n{logged}");
    assert!(
        runs <= max_runs,
        "superseded targets kept forking git: {runs} runs in a {span} ms request loop"
    );
}

#[cfg(unix)]
#[test]
fn local_screen_reads_never_run_a_repository_fsmonitor_hook() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("repo");
    init_repo(&main);
    let marker = temp.path().join("marker");
    let hook = script_in(
        temp.path(),
        "hook.sh",
        &format!("#!/bin/sh\necho ran >> '{}'\n", marker.display()),
    );
    run_git(&main, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
    fs::write(main.join("README.md"), "changed\n").unwrap();
    let main = fs::canonicalize(main).unwrap();
    let mut reads = WorktreeReads::new(runner());
    reads.request(1, main.clone());
    collect(&mut reads, 1);
    assert_eq!(
        runner().read_change_counts(&main),
        CountsState::Ready(ChangeCounts {
            modified: 1,
            ..ChangeCounts::default()
        })
    );
    let _ = runner().read_metadata(&main);
    assert!(matches!(
        runner().read_status(&main, Some(&main)),
        GitStatusRead::Ready(_)
    ));
    assert!(!marker.exists(), "the repository's fsmonitor hook ran");
}
