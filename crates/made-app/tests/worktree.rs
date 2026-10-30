//! Read-only worktree metadata tests with real temporary git repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    assert!(metadata.reason.is_some());
}
