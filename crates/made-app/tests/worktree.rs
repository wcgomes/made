//! Read-only worktree metadata tests with real temporary git repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use made_app::worktree::{
    is_read_only_git_args, GitCommand, GitRunner, MetadataStatus, GIT_PROGRAM_ENV,
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
fn missing_directory_is_unknown_and_never_panics() {
    let temp = tempfile::tempdir().unwrap();
    let missing: PathBuf = temp.path().join("missing");
    let metadata = runner().read_metadata(&missing);
    assert_eq!(metadata.status, MetadataStatus::Unknown);
    assert!(metadata.reason.is_some());
}
