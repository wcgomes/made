use made_remote::git::*;
use std::{
    fs, io,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn git_run(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "-c",
            "user.name=Made Test",
            "-c",
            "user.email=made-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repository(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    git_run(directory, &["init", "-b", "main"]);
    fs::write(directory.join("root.txt"), "one").unwrap();
    fs::create_dir(directory.join("sub")).unwrap();
    fs::write(directory.join("sub/a.txt"), "a").unwrap();
    git_run(directory, &["add", "-A"]);
    git_run(directory, &["commit", "-m", "initial subject"]);
}
#[test]
fn arbitrary_made_git_bin_is_not_a_program_selector() {
    let runner = GitRunner::from_env_value(Some("/tmp/not-git-at-all".into()));
    let program = runner.program().to_string_lossy();
    let name = Path::new(program.as_ref())
        .file_name()
        .and_then(|name| name.to_str());
    assert!(
        matches!(name, Some("git") | Some("git.exe")),
        "honored arbitrary MADE_GIT_BIN value {program}"
    );
}

#[test]
fn every_command_keeps_its_exact_argv_vector() {
    assert_eq!(
        GitCommand::ShowToplevel.args(),
        &["rev-parse", "--show-toplevel"]
    );
    assert_eq!(
        GitCommand::Branch.args(),
        &["rev-parse", "--abbrev-ref", "HEAD"]
    );
    assert_eq!(GitCommand::GitDir.args(), &["rev-parse", "--git-dir"]);
    assert_eq!(
        GitCommand::CommonDir.args(),
        &["rev-parse", "--git-common-dir"]
    );
    assert_eq!(
        GitCommand::WorktreeList.args(),
        &["worktree", "list", "--porcelain"]
    );
    assert_eq!(
        GitCommand::CommitId.args(),
        &["rev-parse", "--short=8", "HEAD"]
    );
    assert_eq!(
        GitCommand::CommitSubject.args(),
        &["log", "-1", "--format=%s"]
    );
    assert_eq!(
        GitCommand::Status.args(),
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ]
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
fn metadata_reports_branch_linked_worktree_and_detached_head() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repo");
    init_repository(&repository);
    let runner = GitRunner::new();

    let metadata = runner.read_metadata(&repository);
    assert_eq!(metadata.status, MetadataStatus::Known);
    assert_eq!(metadata.branch.as_deref(), Some("main"));
    assert!(!metadata.detached);
    assert!(!metadata.is_linked_worktree);
    assert_eq!(metadata.worktree_count, 1);
    assert_eq!(metadata.header_label(), "worktree · main");
    assert_eq!(
        metadata.toplevel.as_deref(),
        Some(fs::canonicalize(&repository).unwrap().as_path())
    );

    let linked = temp.path().join("linked");
    git_run(
        &repository,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
    );
    let linked_metadata = runner.read_metadata(&linked);
    assert_eq!(linked_metadata.status, MetadataStatus::Known);
    assert!(linked_metadata.is_linked_worktree);
    assert_eq!(linked_metadata.branch.as_deref(), Some("linked"));
    assert_eq!(linked_metadata.header_label(), "linked worktree · linked");
    assert_eq!(linked_metadata.worktree_count, 2);
    assert_ne!(linked_metadata.git_dir, linked_metadata.common_dir);
    assert_eq!(
        linked_metadata.common_dir.as_deref(),
        Some(fs::canonicalize(repository.join(".git")).unwrap().as_path())
    );

    git_run(&linked, &["checkout", "--detach", "HEAD"]);
    let detached = runner.read_metadata(&linked);
    assert_eq!(detached.status, MetadataStatus::Known);
    assert!(detached.detached);
    assert!(detached.branch.is_none());
    assert_eq!(detached.header_label(), "linked worktree · detached HEAD");
    let commit = detached.commit_abbrev.as_deref().unwrap();
    assert_eq!(commit.len(), 8);
    assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(detached.commit_subject.as_deref(), Some("initial subject"));
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

#[test]
fn status_filters_ignored_and_outside_rows_and_keeps_partial_staging() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repo");
    init_repository(&repository);
    fs::write(repository.join(".gitignore"), "*.log\n").unwrap();
    git_run(&repository, &["add", ".gitignore"]);
    git_run(&repository, &["commit", "-m", "ignore logs"]);

    fs::write(repository.join("root.txt"), "changed").unwrap();
    fs::write(repository.join("sub/a.txt"), "staged").unwrap();
    git_run(&repository, &["add", "sub/a.txt"]);
    fs::write(repository.join("sub/a.txt"), "staged then changed").unwrap();
    fs::write(repository.join("sub/new.txt"), "new").unwrap();
    fs::write(repository.join("sub/ignored.log"), "ignored").unwrap();
    fs::write(repository.join("root.log"), "ignored").unwrap();

    let index_before = fs::read(repository.join(".git/index")).unwrap();
    let worktree_before: Vec<Vec<u8>> = ["root.txt", "sub/a.txt", "sub/new.txt"]
        .iter()
        .map(|name| fs::read(repository.join(name)).unwrap())
        .collect();
    let runner = GitRunner::new();
    let subroot = fs::canonicalize(repository.join("sub")).unwrap();
    let GitStatusRead::Ready(scoped) = runner.read_status(&subroot, Some(&repository)) else {
        panic!("real status read must succeed");
    };
    assert!(!scoped.groups.is_empty());
    assert!(scoped
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .all(|row| row.path.starts_with(&subroot)));
    assert!(scoped
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .all(|row| row
            .path
            .file_name()
            .is_some_and(|name| name != "ignored.log")));
    let titles: Vec<_> = scoped.groups.iter().map(|group| group.title).collect();
    assert!(titles.contains(&GROUP_STAGED));
    assert!(titles.contains(&GROUP_CHANGES));
    assert!(titles.contains(&GROUP_UNTRACKED));
    assert!(!titles.contains(&GROUP_MERGE));
    assert!(scoped
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .any(|row| row.relative == Path::new("new.txt")));
    assert_eq!(
        scoped
            .groups
            .iter()
            .flat_map(|group| &group.rows)
            .filter(|row| row.relative == Path::new("a.txt"))
            .count(),
        2,
        "partially staged content appears in Staged and Changes"
    );

    let GitStatusRead::Ready(whole) = runner.read_status(&repository, Some(&repository)) else {
        panic!("real status read must succeed");
    };
    assert!(whole
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .any(|row| row.relative == Path::new("root.txt")));
    assert!(whole
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .all(|row| row
            .path
            .file_name()
            .is_some_and(|name| name != "ignored.log")));

    assert_eq!(
        fs::read(repository.join(".git/index")).unwrap(),
        index_before
    );
    for (name, before) in ["root.txt", "sub/a.txt", "sub/new.txt"]
        .iter()
        .zip(worktree_before)
    {
        assert_eq!(fs::read(repository.join(name)).unwrap(), before);
    }
}

/// The error of a call that must fail, retried while the freshly written
/// script is still open for writing in a sibling thread's forked child
/// (ETXTBSY, a race between parallel tests, not a runner behavior).
#[cfg(unix)]
fn err_retrying_busy<T: std::fmt::Debug>(call: impl Fn() -> io::Result<T>) -> io::Error {
    for _ in 0..50 {
        let error = call().unwrap_err();
        if !error.to_string().contains("Text file busy") {
            return error;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    call().unwrap_err()
}

#[cfg(unix)]
#[test]
fn runner_bounds_output_and_time_and_reports_failures_honestly() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let script = |name: &str, body: &str| {
        let path = temp.path().join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let output_cap = script(
        "output-cap.sh",
        "#!/bin/sh\ndd if=/dev/zero bs=2097153 count=1 2>/dev/null | tr '\\0' 'a'\n",
    );
    let failing = script("failing.sh", "#!/bin/sh\nprintf boom >&2\nexit 3\n");
    let timeout = script("timeout.sh", "#!/bin/sh\nsleep 30\n");

    let error = err_retrying_busy(|| {
        GitRunner::with_program(&output_cap).read_bytes(GitCommand::Status, temp.path())
    });
    assert_eq!(error.to_string(), "git output limit");

    assert_eq!(GitRunner::new().limits(), GitLimits::BOUNDED);

    let error = err_retrying_busy(|| {
        GitRunner::with_program(&failing).read_bytes(GitCommand::Status, temp.path())
    });
    assert_eq!(error.to_string(), "git exited with 3: boom");
    assert!(error.to_string().contains("boom"));

    let started = Instant::now();
    let error = err_retrying_busy(|| {
        GitRunner::with_limits(
            &timeout,
            GitLimits {
                deadline: Some(Duration::from_millis(150)),
                ..GitLimits::BOUNDED
            },
        )
        .read_bytes(GitCommand::Status, temp.path())
    });
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.to_string(), "git read deadline");
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "the injected deadline is enforced, not a short stream read"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the test never waits for the nine-second default"
    );

    let missing = temp.path().join("missing-git");
    let error = GitRunner::with_program(&missing)
        .read_bytes(GitCommand::Status, temp.path())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!error.to_string().is_empty());
    assert!(matches!(
        GitRunner::with_program(&missing).read_status(temp.path(), Some(temp.path())),
        GitStatusRead::Unavailable { reason } if reason.contains("could not run git")
    ));
    let metadata = GitRunner::with_program(&missing).read_metadata(temp.path());
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
    assert!(metadata
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("could not run git")));

    let non_executable = script("non-executable.sh", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();
    let error = GitRunner::with_program(&non_executable)
        .read_bytes(GitCommand::Status, temp.path())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let metadata = GitRunner::with_program(&non_executable).read_metadata(temp.path());
    assert_eq!(metadata.status, MetadataStatus::Unavailable);
    assert!(metadata
        .reason
        .as_deref()
        .is_some_and(|reason| reason.to_lowercase().contains("permission denied")));
}

#[test]
fn missing_root_stays_unknown_instead_of_becoming_a_spawn_failure() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-root");

    let metadata = GitRunner::with_program("git").read_metadata(&missing);
    assert_eq!(metadata.status, MetadataStatus::Unknown);
    assert!(metadata
        .reason
        .as_deref()
        .is_some_and(|reason| reason.starts_with("not a git repository")));
}
