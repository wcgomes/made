use made_remote::git::*;
use std::{path::Path, process::Command};
#[test]
fn arbitrary_made_git_bin_is_not_a_program_selector() {
    let runner = GitRunner::from_env_value(Some("/tmp/not-git-at-all".into()));
    let program = runner.program().to_string_lossy();
    assert!(
        program == "git" || program.ends_with("/git") || program.ends_with("\\git"),
        "honored arbitrary MADE_GIT_BIN value {program}"
    );
}

#[test]
fn closed_allowlist_and_existing_group_api_are_preserved() {
    assert_eq!(GitCommand::ALL.len(), 8);
    for command in GitCommand::ALL {
        assert!(is_read_only_git_args(command.args()));
    }
    for args in [
        &["add", "."][..],
        &["commit", "-m", "x"],
        &["reset", "--hard"],
    ] {
        assert!(!is_read_only_git_args(args));
    }
    let records = parse_porcelain_v1_z(
        "MM sub/both\0UU sub/conflict\0?? sub/new\0!! sub/ignored\0 M outside\0",
    );
    let grouped = group_status(&records, Path::new("/repo/sub"), Some(Path::new("/repo")));
    assert_eq!(
        grouped.groups.iter().map(|g| g.title).collect::<Vec<_>>(),
        vec![GROUP_MERGE, GROUP_STAGED, GROUP_CHANGES, GROUP_UNTRACKED]
    );
    assert!(grouped
        .groups
        .iter()
        .flat_map(|g| &g.rows)
        .all(|r| r.path.starts_with("/repo/sub")));
}
#[test]
fn real_repository_status_never_mutates_index() {
    let temp = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg(temp.path())
        .output()
        .unwrap()
        .status
        .success());
    std::fs::write(temp.path().join("new"), "text").unwrap();
    let runner = GitRunner::new();
    let bytes = runner.read_bytes(GitCommand::Status, temp.path()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("?? new"));
    assert!(!temp.path().join(".git/index").exists());
    assert!(matches!(
        runner.read_status(temp.path(), Some(temp.path())),
        GitStatusRead::Ready(_)
    ));
}
