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
        GitCommand::FilterDrivers.args(),
        &[
            "config",
            "--show-scope",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|process|smudge|required)$",
        ]
    );
    assert_eq!(
        GitCommand::FilterDriversUnscoped.args(),
        &[
            "config",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|process|smudge|required)$",
        ]
    );
    assert_eq!(
        GitCommand::Status.args(),
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
            "--",
            ".",
        ]
    );
}

#[test]
fn closed_allowlist_and_existing_group_api_are_preserved() {
    assert_eq!(GitCommand::ALL.len(), 10);
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
// POSIX-only: it supports the /bin/sh script runner test.
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

// open-workspace-redesign: change pills and the worktree list.

fn counts(porcelain: &str) -> ChangeCounts {
    ChangeCounts::from_records(&parse_porcelain_v1_z(porcelain))
}

#[test]
fn change_counts_classify_like_the_grouped_view() {
    // A conflicted path counts only as conflicted; a partially staged path
    // counts once as staged and once as modified; ignored paths are dropped.
    let counts = counts("UU src/clash.rs\0AA both-added\0MM src/both.rs\0M  staged.rs\0 M dirty.rs\0?? new.txt\0!! build.log\0");
    assert_eq!(
        counts,
        ChangeCounts {
            conflicted: 2,
            staged: 2,
            modified: 2,
            untracked: 1,
        }
    );
}

#[test]
fn change_counts_cover_the_whole_working_tree_without_a_root_filter() {
    // Paths in every directory count, whatever folder the user selected.
    let counts =
        counts(" M README.md\0 M deep/nested/dir/file.rs\0?? other/new.txt\0R  next.rs\0prev.rs\0");
    assert_eq!(counts.modified, 2);
    assert_eq!(counts.untracked, 1);
    assert_eq!(counts.staged, 1, "a rename counts once as staged");
    assert_eq!(counts.conflicted, 0);
}

#[test]
fn change_counts_match_a_snapshot_grouped_at_the_working_tree_root() {
    let records = parse_porcelain_v1_z("UU a\0MM b\0?? c/d\0 D e\0");
    let snapshot = group_status(&records, Path::new("/repo"), Some(Path::new("/repo")));
    assert_eq!(
        ChangeCounts::from_snapshot(&snapshot),
        ChangeCounts::from_records(&records)
    );
}

#[test]
fn zero_kinds_are_absent_from_the_pills_and_summary() {
    let root = counts("UU a\0M  b\0A  c\0 M d\0M  e\0 M f\0 M g\0");
    assert_eq!(
        root.nonzero(),
        vec![
            (PillKind::Conflicted, 1),
            (PillKind::Staged, 3),
            (PillKind::Modified, 3),
        ]
    );
    assert_eq!(
        root.pill_labels(),
        vec!["1 conflicted", "3 staged", "3 modified"]
    );
    assert_eq!(
        root.summary().as_deref(),
        Some("Changes: 1 conflicted · 3 staged · 3 modified")
    );
    let untracked = counts("?? a\0?? b\0?? c\0?? d\0");
    assert_eq!(untracked.pill_labels(), vec!["4 untracked"]);
    assert_eq!(untracked.summary().as_deref(), Some("Changes: 4 untracked"));
    let clean = counts("!! ignored.log\0");
    assert!(clean.is_clean());
    assert!(clean.nonzero().is_empty());
    assert!(clean.pill_labels().is_empty());
    assert_eq!(clean.summary(), None, "a clean tree shows no `0`");
}

#[test]
fn worktree_list_parser_keeps_main_first_and_repository_order() {
    let text = "worktree /work/made\nHEAD 0123456789abcdef0123456789abcdef01234567\nbranch refs/heads/main\n\n\
worktree /work/made-wt/feature-a\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/feature-a\n\n\
worktree /work/made-wt/fix\nHEAD abcdef0123456789abcdef0123456789abcdef01\ndetached\n\n\
worktree /work/made-wt/gone\nHEAD 2222222222222222222222222222222222222222\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n\n";
    let entries = parse_worktree_entries(text);
    let paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            Path::new("/work/made").to_path_buf(),
            Path::new("/work/made-wt/feature-a").to_path_buf(),
            Path::new("/work/made-wt/fix").to_path_buf(),
            Path::new("/work/made-wt/gone").to_path_buf(),
        ]
    );
    assert_eq!(entries[0].head_label().as_deref(), Some("main"));
    assert_eq!(entries[1].head_label().as_deref(), Some("feature-a"));
    assert!(entries[2].detached);
    assert_eq!(entries[2].branch, None);
    assert_eq!(entries[2].head_label().as_deref(), Some("abcdef01"));
    assert!(!entries[0].missing && !entries[1].missing && !entries[2].missing);
    assert!(
        entries[3].missing,
        "a prunable entry's directory is missing"
    );
}

#[test]
fn worktree_list_parser_skips_bare_entries_and_tolerates_garbage() {
    let text = "worktree /srv/repo.git\nbare\n\nworktree /srv/checkout\nHEAD 3333333333333333333333333333333333333333\nbranch refs/heads/dev\n\nnot a record\n";
    let entries = parse_worktree_entries(text);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, Path::new("/srv/checkout"));
    assert_eq!(entries[0].head_label().as_deref(), Some("dev"));
    assert!(parse_worktree_entries("").is_empty());
}

#[test]
fn real_worktree_list_reads_identically_from_main_and_linked_roots() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repo");
    init_repository(&repository);
    let linked = temp.path().join("linked");
    git_run(
        &repository,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
    );
    let runner = GitRunner::new();
    let from_main = parse_worktree_entries(&String::from_utf8_lossy(
        &runner
            .read_bytes(GitCommand::WorktreeList, &repository)
            .unwrap(),
    ));
    let from_linked = parse_worktree_entries(&String::from_utf8_lossy(
        &runner
            .read_bytes(GitCommand::WorktreeList, &linked)
            .unwrap(),
    ));
    assert_eq!(from_main, from_linked);
    assert_eq!(from_main.len(), 2);
    assert_eq!(
        fs::canonicalize(&from_main[0].path).unwrap(),
        fs::canonicalize(&repository).unwrap()
    );
    assert_eq!(from_main[1].head_label().as_deref(), Some("linked"));
}

/// A repository whose `core.fsmonitor` and `log.showSignature` would run a
/// script that creates `marker` (#security: repo config must not execute).
#[cfg(unix)]
fn hostile_repository(base: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let repository = base.join("repo");
    init_repository(&repository);
    let marker = base.join("marker");
    let script = base.join("hook.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    git_run(
        &repository,
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    git_run(
        &repository,
        &["config", "gpg.program", script.to_str().unwrap()],
    );
    git_run(&repository, &["config", "log.showSignature", "true"]);
    fs::write(repository.join("root.txt"), "changed").unwrap();
    (repository, marker)
}

#[cfg(unix)]
#[test]
fn repository_configured_commands_never_run_during_reads() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (repository, marker) = hostile_repository(&base);
    // Control: plain git honors the hook, so the marker proves execution.
    Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["status", "--porcelain"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        marker.exists(),
        "control: the fsmonitor hook runs in plain git"
    );
    fs::remove_file(&marker).unwrap();

    for runner in [
        GitRunner::new(),
        GitRunner::with_limits("git", GitLimits::UNBOUNDED),
    ] {
        for command in GitCommand::ALL {
            let _ = runner.read_bytes(command, &repository);
        }
        let metadata = runner.read_metadata(&repository);
        assert_eq!(metadata.branch.as_deref(), Some("main"));
        let GitStatusRead::Ready(snapshot) = runner.read_status(&repository, Some(&repository))
        else {
            panic!("status read failed");
        };
        assert_eq!(ChangeCounts::from_snapshot(&snapshot).nonzero().len(), 1);
    }
    assert!(!marker.exists(), "a repository-configured command ran");
}

#[cfg(unix)]
#[test]
fn submodule_worktrees_are_not_entered_but_their_commit_changes_show() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (inner, marker) = hostile_repository(&base);
    let outer = base.join("outer");
    init_repository(&outer);
    git_run(
        &outer,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            inner.to_str().unwrap(),
            "inner",
        ],
    );
    // The repository asks git to look inside the submodule.
    git_run(
        &outer,
        &[
            "config",
            "-f",
            ".gitmodules",
            "submodule.inner.ignore",
            "none",
        ],
    );
    git_run(&outer, &["add", ".gitmodules"]);
    git_run(&outer, &["commit", "-q", "-m", "submodule"]);
    git_run(&outer, &["config", "submodule.inner.ignore", "none"]);
    let submodule = outer.join("inner");
    let hook = base.join("hook.sh");
    git_run(
        &submodule,
        &["config", "core.fsmonitor", hook.to_str().unwrap()],
    );
    git_run(
        &submodule,
        &["config", "filter.evil.clean", hook.to_str().unwrap()],
    );
    fs::write(
        base.join("outer/.git/modules/inner/info/attributes"),
        "* filter=evil\n",
    )
    .unwrap_or_else(|_| {
        fs::create_dir_all(base.join("outer/.git/modules/inner/info")).unwrap();
        fs::write(
            base.join("outer/.git/modules/inner/info/attributes"),
            "* filter=evil\n",
        )
        .unwrap()
    });
    fs::write(submodule.join("root.txt"), "dirty").unwrap();
    let _ = fs::remove_file(&marker);
    let bytes = GitRunner::new()
        .read_bytes(GitCommand::Status, &outer)
        .unwrap();
    assert!(!marker.exists(), "status entered the submodule worktree");
    assert!(bytes.is_empty(), "{:?}", String::from_utf8_lossy(&bytes));

    // A new commit checked out in the submodule is still a change.
    Command::new("git")
        .arg("-C")
        .arg(&submodule)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "user.name=Made Test",
            "-c",
            "user.email=made-test@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "moved",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_file(&marker);
    let bytes = GitRunner::new()
        .read_bytes(GitCommand::Status, &outer)
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), " M inner\0");
    assert!(!marker.exists(), "status entered the submodule worktree");
}

#[cfg(unix)]
fn marker_script(base: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let marker = base.join("marker");
    let script = base.join("hook.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, marker)
}

#[cfg(unix)]
#[test]
fn a_partial_clone_never_fetches_through_a_repository_transport_command() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (script, marker) = marker_script(&base);
    let repository = base.join("repo");
    init_repository(&repository);
    for (key, value) in [
        ("core.repositoryformatversion", "1"),
        ("extensions.partialClone", "origin"),
        ("remote.origin.url", "ssh://example.invalid/repo"),
        ("remote.origin.promisor", "true"),
        ("remote.origin.uploadpack", script.to_str().unwrap()),
        ("core.sshCommand", script.to_str().unwrap()),
        ("protocol.ssh.allow", "always"),
    ] {
        git_run(&repository, &["config", key, value]);
    }
    // A staged change under `sub` makes status read the missing tree.
    fs::write(repository.join("sub/b.txt"), "b").unwrap();
    git_run(&repository, &["add", "sub/b.txt"]);
    let tree = git_run(&repository, &["rev-parse", "HEAD:sub"]);
    fs::remove_file(
        repository
            .join(".git/objects")
            .join(&tree[..2])
            .join(&tree[2..]),
    )
    .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["status", "--porcelain"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(marker.exists(), "control: plain git fetches lazily");
    fs::remove_file(&marker).unwrap();

    let runner = GitRunner::new();
    for command in GitCommand::ALL {
        let _ = runner.read_bytes(command, &repository);
    }
    let _ = runner.read_status(&repository, Some(&repository));
    let _ = runner.read_metadata(&repository);
    assert!(!marker.exists(), "a lazy fetch ran a transport command");
}

/// Make status re-hash `root.txt` without changing its content.
#[cfg(unix)]
fn touch_unchanged(repository: &Path, stamp: &str) {
    assert!(Command::new("touch")
        .args(["-t", stamp])
        .arg(repository.join("root.txt"))
        .status()
        .unwrap()
        .success());
}

#[cfg(unix)]
#[test]
fn filter_drivers_never_run_and_status_still_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (script, marker) = marker_script(&base);
    let repository = base.join("repo");
    init_repository(&repository);
    let script = script.to_str().unwrap();
    git_run(&repository, &["config", "filter.evil.clean", script]);
    git_run(&repository, &["config", "filter.evil.required", "true"]);
    git_run(&repository, &["config", "filter.Dot.ted.process", script]);
    git_run(&repository, &["config", "filter.Dot.ted.required", "true"]);
    fs::write(
        repository.join(".git/info/attributes"),
        "*.txt filter=evil\nroot.txt filter=Dot.ted\nsub/* filter=evil\n",
    )
    .unwrap();
    touch_unchanged(&repository, "202001010000");
    let control = Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["status", "--porcelain"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(marker.exists(), "control: plain git runs the filter");
    assert!(!control.status.success());
    fs::remove_file(&marker).unwrap();

    for (index, runner) in [
        GitRunner::new(),
        GitRunner::with_limits("git", GitLimits::UNBOUNDED),
    ]
    .into_iter()
    .enumerate()
    {
        touch_unchanged(&repository, &format!("20200101000{}", index + 1));
        let bytes = runner.read_bytes(GitCommand::Status, &repository).unwrap();
        assert!(bytes.is_empty(), "{:?}", String::from_utf8_lossy(&bytes));
        touch_unchanged(&repository, &format!("20200101001{}", index + 1));
        assert!(matches!(
            runner.read_status(&repository, Some(&repository)),
            GitStatusRead::Ready(_)
        ));
    }
    assert!(!marker.exists(), "a filter driver ran");
}

#[cfg(unix)]
#[test]
fn a_filter_driver_that_cannot_be_neutralized_fails_status_closed() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (script, marker) = marker_script(&base);
    let repository = base.join("repo");
    init_repository(&repository);
    // `-c filter.a=b.clean=` would set `filter.a` to `b.clean=`.
    git_run(
        &repository,
        &["config", "filter.a=b.clean", script.to_str().unwrap()],
    );
    fs::write(repository.join(".git/info/attributes"), "* filter=a=b\n").unwrap();
    touch_unchanged(&repository, "202001010000");
    let runner = GitRunner::new();
    assert!(runner.read_bytes(GitCommand::Status, &repository).is_err());
    assert!(matches!(
        runner.read_status(&repository, Some(&repository)),
        GitStatusRead::Failed { .. }
    ));
    assert!(!marker.exists(), "a filter driver ran");
    // Other reads are unaffected.
    assert_eq!(
        String::from_utf8(runner.read_bytes(GitCommand::Branch, &repository).unwrap())
            .unwrap()
            .trim(),
        "main"
    );
}

#[test]
fn worktree_parsers_agree_on_paths_branches_and_bare_entries() {
    let text = "worktree /srv/bare.git\nbare\n\nworktree /srv/main\nHEAD 0123456789abcdef\nbranch refs/heads/main\n\nworktree /srv/wt\nHEAD abcdef0123456789\ndetached\nprunable gitdir file points to non-existent location\n\nworktree \nbranch refs/heads/ghost\n";
    let entries = parse_worktree_entries(text);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (
                entry.path.clone(),
                entry.branch.clone(),
                entry.detached,
                entry.missing
            ))
            .collect::<Vec<_>>(),
        vec![
            ("/srv/main".into(), Some("main".to_owned()), false, false),
            ("/srv/wt".into(), None, true, true),
        ]
    );
    assert_eq!(entries[1].head_label().as_deref(), Some("abcdef01"));
}

/// An isolated user home whose global config defines `[filter "up"]` (a
/// clean filter that upper-cases and logs to `global-marker`) and
/// `[filter "plain"]` (`tr a-z A-Z`, like the reviewer's demo), plus a git
/// wrapper that runs with that home as its only global config.
#[cfg(unix)]
struct TrustedHome {
    home: std::path::PathBuf,
    git: std::path::PathBuf,
    marker: std::path::PathBuf,
}

#[cfg(unix)]
fn trusted_home(base: &Path) -> TrustedHome {
    use std::os::unix::fs::PermissionsExt;
    let home = base.join("home");
    fs::create_dir_all(home.join("xdg")).unwrap();
    let marker = base.join("global-marker");
    let upper = base.join("upper.sh");
    fs::write(
        &upper,
        format!(
            "#!/bin/sh\necho ran >> '{}'\ntr a-z A-Z\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&upper, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        home.join(".gitconfig"),
        format!(
            "[filter \"up\"]\n\tclean = {}\n\tsmudge = cat\n\trequired = true\n[filter \"plain\"]\n\tclean = tr a-z A-Z\n\tsmudge = cat\n",
            upper.display()
        ),
    )
    .unwrap();
    let git = base.join("home-git.sh");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\nHOME='{home}' XDG_CONFIG_HOME='{home}/xdg' exec git \"$@\"\n",
            home = home.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    TrustedHome { home, git, marker }
}

/// A repository with `a.up` (filter `up`) and `b.plain` (filter `plain`)
/// committed through the trusted home's filters.
#[cfg(unix)]
fn filtered_repository(base: &Path, trusted: &TrustedHome) -> std::path::PathBuf {
    let repository = base.join("repo");
    init_repository(&repository);
    fs::write(
        repository.join(".gitattributes"),
        "*.up filter=up\n*.plain filter=plain\n",
    )
    .unwrap();
    fs::write(repository.join("a.up"), "abc\n").unwrap();
    fs::write(repository.join("b.plain"), "def\n").unwrap();
    for args in [
        &["add", ".gitattributes", "a.up", "b.plain"][..],
        &["commit", "-q", "-m", "filtered"],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args([
                "-c",
                "user.name=Made Test",
                "-c",
                "user.email=made-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_CONFIG_GLOBAL")
            .env("HOME", &trusted.home)
            .env("XDG_CONFIG_HOME", trusted.home.join("xdg"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Stored upper-cased by the trusted clean filters.
    assert_eq!(git_run(&repository, &["show", "HEAD:a.up"]), "ABC");
    assert_eq!(git_run(&repository, &["show", "HEAD:b.plain"]), "DEF");
    let _ = fs::remove_file(&trusted.marker);
    repository
}

#[cfg(unix)]
fn touch_filtered(repository: &Path, stamp: &str) {
    for name in ["a.up", "b.plain"] {
        assert!(Command::new("touch")
            .args(["-t", stamp])
            .arg(repository.join(name))
            .status()
            .unwrap()
            .success());
    }
}

#[cfg(unix)]
#[test]
fn user_global_filter_drivers_keep_working_for_status() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let trusted = trusted_home(&base);
    let repository = filtered_repository(&base, &trusted);
    let runner = GitRunner::with_limits(&trusted.git, GitLimits::BOUNDED);
    touch_filtered(&repository, "202001010000");
    let bytes = runner.read_bytes(GitCommand::Status, &repository).unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), "", "no spurious change");
    assert!(trusted.marker.exists(), "the user's global filter ran");
    touch_filtered(&repository, "202001010001");
    let GitStatusRead::Ready(snapshot) = runner.read_status(&repository, Some(&repository)) else {
        panic!("status failed");
    };
    assert!(ChangeCounts::from_snapshot(&snapshot).is_clean());
}

#[cfg(unix)]
#[test]
fn a_repository_redefinition_of_a_global_driver_is_neutralized() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let trusted = trusted_home(&base);
    let repository = filtered_repository(&base, &trusted);
    let (script, marker) = marker_script(&base);
    // Only one key comes from the repository; the whole driver is disabled.
    git_run(
        &repository,
        &["config", "filter.up.process", script.to_str().unwrap()],
    );
    let runner = GitRunner::with_limits(&trusted.git, GitLimits::BOUNDED);
    touch_filtered(&repository, "202001010000");
    let bytes = runner.read_bytes(GitCommand::Status, &repository).unwrap();
    assert!(!marker.exists(), "the repository's process filter ran");
    assert!(!trusted.marker.exists(), "the redefined driver ran at all");
    // `up` is disabled, so `a.up` reads as changed; `plain`, defined only
    // globally, still normalizes `b.plain`, which stays clean.
    assert_eq!(String::from_utf8_lossy(&bytes), " M a.up\0");
}

#[cfg(unix)]
#[test]
fn unparseable_scopes_or_other_config_errors_fail_status_closed() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let repository = base.join("repo");
    init_repository(&repository);
    for (name, config_reply) in [
        // Output without a scope column.
        ("no-scope.sh", "printf 'filter.evil.clean\\n'; exit 0"),
        (
            "bad-scope.sh",
            "printf 'Local\\tfilter.evil.clean\\n'; exit 0",
        ),
        // Usage-style exits that are not the missing option.
        (
            "bad-config.sh",
            "echo 'fatal: bad config line 3 in file .git/config' >&2; exit 128",
        ),
        (
            "other-usage.sh",
            "echo 'error: unknown option `name-only'\"'\"'' >&2; exit 129",
        ),
    ] {
        let script = base.join(name);
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$*\" in *--get-regexp*) {config_reply} ;; esac\nexec git \"$@\"\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let runner = GitRunner::with_limits(&script, GitLimits::BOUNDED);
        assert!(
            runner.read_bytes(GitCommand::Status, &repository).is_err(),
            "{name}"
        );
        assert!(
            matches!(
                runner.read_status(&repository, Some(&repository)),
                GitStatusRead::Failed { .. }
            ),
            "{name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_git_without_show_scope_disables_every_filter_driver_and_still_reads_status() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (script, marker) = marker_script(&base);
    let repository = base.join("repo");
    init_repository(&repository);
    git_run(
        &repository,
        &["config", "filter.evil.clean", script.to_str().unwrap()],
    );
    git_run(&repository, &["config", "filter.evil.required", "true"]);
    fs::write(repository.join(".git/info/attributes"), "* filter=evil\n").unwrap();
    let log = base.join("argv.log");
    // Git before 2.26: `--show-scope` is an unknown option (exit 129 with
    // usage); everything else is the real git.
    let old_git = base.join("old-git.sh");
    fs::write(
        &old_git,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$*\" in *--show-scope*) echo 'error: unknown option `show-scope'\"'\"'' >&2; echo 'usage: git config [<options>]' >&2; exit 129 ;; esac\nexec git \"$@\"\n",
            log = log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&old_git, fs::Permissions::from_mode(0o755)).unwrap();
    let runner = GitRunner::with_limits(&old_git, GitLimits::BOUNDED);
    touch_unchanged(&repository, "202001010000");
    let bytes = runner.read_bytes(GitCommand::Status, &repository).unwrap();
    assert!(bytes.is_empty(), "{:?}", String::from_utf8_lossy(&bytes));
    touch_unchanged(&repository, "202001010001");
    assert!(matches!(
        runner.read_status(&repository, Some(&repository)),
        GitStatusRead::Ready(_)
    ));
    assert!(!marker.exists(), "a filter driver ran");
    let logged = fs::read_to_string(&log).unwrap();
    let status: Vec<&str> = logged
        .lines()
        .filter(|line| line.contains(" status --porcelain=v1"))
        .collect();
    assert_eq!(status.len(), 2, "{logged}");
    for line in status {
        for setting in [
            "-c filter.evil.clean= ",
            "-c filter.evil.process= ",
            "-c filter.evil.required=false ",
        ] {
            assert!(line.contains(setting), "{line}");
        }
    }
    assert!(
        logged.contains(" config --name-only --get-regexp "),
        "{logged}"
    );
}

#[cfg(unix)]
#[test]
fn a_global_filter_keeps_correct_counts_for_a_real_change_beside_a_touched_file() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let trusted = trusted_home(&base);
    let repository = filtered_repository(&base, &trusted);
    let runner = GitRunner::with_limits(&trusted.git, GitLimits::BOUNDED);
    // `a.up` really changes (same size, so only re-hashing tells); `b.plain`
    // is only touched.
    fs::write(repository.join("a.up"), "abd\n").unwrap();
    touch_filtered(&repository, "202001010000");
    let bytes = runner.read_bytes(GitCommand::Status, &repository).unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), " M a.up\0");
    assert!(trusted.marker.exists(), "the user's global filter ran");
    touch_filtered(&repository, "202001010001");
    let GitStatusRead::Ready(snapshot) = runner.read_status(&repository, Some(&repository)) else {
        panic!("status failed");
    };
    assert_eq!(
        ChangeCounts::from_snapshot(&snapshot),
        ChangeCounts {
            modified: 1,
            ..ChangeCounts::default()
        }
    );
}
