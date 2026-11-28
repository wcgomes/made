//! `WatchDir` subscriptions: one non-recursive watch per expanded directory,
//! the repository metadata of the root, and the bounds around both. Real
//! filesystem and real `git`; nothing here reaches a remote host.
#![cfg(unix)]
use made_remote::{
    filesystem::{remote, Filesystem, WatchOutcome},
    git::{refs_directory, resolve_git_dirs},
    protocol::*,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

/// macOS FSEvents can lag well beyond the watcher's 150 ms debounce.
const DEADLINE: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(2);

fn canonical(path: &Path) -> RemotePath {
    remote(&fs::canonicalize(path).unwrap()).unwrap()
}

fn git(directory: &Path, args: &[&str]) {
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Poll until `found` accepts the notifications gathered so far.
fn gather(
    service: &mut Filesystem,
    mut found: impl FnMut(&[(u64, Operation)]) -> bool,
) -> Vec<(u64, Operation)> {
    let deadline = Instant::now() + DEADLINE;
    let mut seen = Vec::new();
    loop {
        seen.extend(service.poll_changes());
        if found(&seen) {
            return seen;
        }
        assert!(Instant::now() < deadline, "no notification: {seen:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Subscribe and wait until the platform watch is established (it is set up
/// on the watcher thread, never on the caller). Returns the outcome reported
/// for the request.
fn subscribe(service: &mut Filesystem, path: &RemotePath, request: u64) -> Operation {
    match service.watch_dir(path, request).unwrap() {
        WatchOutcome::Live => Operation::Changed { path: path.clone() },
        WatchOutcome::Pending => {
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                if let Some((id, operation)) = service.poll_watch_results().into_iter().next() {
                    assert_eq!(id, request);
                    return operation;
                }
                assert!(Instant::now() < deadline, "registration never completed");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Wait for the repository metadata watch, armed once the root is live.
fn await_git(service: &mut Filesystem) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !service.watching_git() {
        service.poll_watch_results();
        assert!(Instant::now() < deadline, "git watch never became ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Write entries into `dir` until a notification for one of them arrives under
/// `request`. Releasing a platform watch can restart the backend's stream
/// (macOS FSEvents) and an event inside that gap is lost, so a write right
/// after a release is retried instead of trusted.
fn land(
    service: &mut Filesystem,
    dir: &Path,
    base: &RemotePath,
    request: u64,
    prefix: &str,
) -> Vec<(u64, Operation)> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seen = Vec::new();
    let mut attempt = 0;
    while Instant::now() < deadline {
        attempt += 1;
        let name = format!("{prefix}{attempt}");
        fs::write(dir.join(&name), "x").unwrap();
        let wanted = base.join_name(&name);
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            seen.extend(service.poll_changes());
            if changed(&seen, request, &wanted) {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    panic!("no notification for {prefix}*: {seen:?}");
}

fn quiet(service: &mut Filesystem) -> Vec<(u64, Operation)> {
    let deadline = Instant::now() + SETTLE;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        seen.extend(service.poll_changes());
        std::thread::sleep(Duration::from_millis(10));
    }
    seen
}

fn changed(seen: &[(u64, Operation)], request: u64, path: &RemotePath) -> bool {
    seen.iter()
        .any(|(id, op)| *id == request && matches!(op, Operation::Changed { path: p } if p == path))
}

fn git_changed(seen: &[(u64, Operation)], request: u64) -> bool {
    seen.iter()
        .any(|(id, op)| *id == request && matches!(op, Operation::GitChanged))
}

struct Tree {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn tree() -> Tree {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap().join("root");
    fs::create_dir_all(root.join("a/deep")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    Tree { _temp: temp, root }
}

#[test]
fn expanded_directories_report_the_changed_entry_and_nothing_else() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &root, 10),
        Operation::Changed { .. }
    ));
    assert!(matches!(
        subscribe(&mut service, &a, 11),
        Operation::Changed { .. }
    ));

    // A change inside an expanded nested directory names that entry, under
    // that directory's own subscription, and not the root's.
    fs::write(tree.root.join("a/new.txt"), "x").unwrap();
    let new = canonical(&tree.root.join("a")).join_name("new.txt");
    let seen = gather(&mut service, |seen| changed(seen, 11, &new));
    // (Where registering a watch restarts the platform stream, the root is
    // reported as itself once; never as the child's entry.)
    assert!(
        !seen.iter().any(|(id, operation)| *id == 10
            && !matches!(operation, Operation::Changed { path } if *path == root)),
        "the root subscription does not see a child directory's entry: {seen:?}"
    );

    // A directory nobody expanded is not watched, and the watch is not
    // recursive: a write two levels down is invisible.
    fs::write(tree.root.join("b/ignored.txt"), "x").unwrap();
    fs::write(tree.root.join("a/deep/ignored.txt"), "x").unwrap();
    assert!(quiet(&mut service).is_empty());

    // Root-level entries still reach the root subscription, with the
    // entry's exact path rather than just the root.
    fs::write(tree.root.join("top.txt"), "x").unwrap();
    let top = root.join_name("top.txt");
    gather(&mut service, |seen| changed(seen, 10, &top));

    // Rename and delete are reported under both spellings of the entry.
    fs::rename(tree.root.join("a/new.txt"), tree.root.join("a/renamed.txt")).unwrap();
    let renamed = a.join_name("renamed.txt");
    gather(&mut service, |seen| {
        changed(seen, 11, &renamed) && changed(seen, 11, &new)
    });
    fs::remove_file(tree.root.join("a/renamed.txt")).unwrap();
    gather(&mut service, |seen| changed(seen, 11, &renamed));
}

#[test]
fn unwatch_cancel_and_root_switch_stop_the_notifications() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let b = canonical(&tree.root.join("b"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 21),
        Operation::Changed { .. }
    ));
    assert!(matches!(
        subscribe(&mut service, &b, 22),
        Operation::Changed { .. }
    ));
    assert_eq!(service.watched_dirs(), 2);

    // Positive controls first, so the negatives below are not a dead watcher.
    fs::write(tree.root.join("a/one"), "x").unwrap();
    fs::write(tree.root.join("b/one"), "x").unwrap();
    gather(&mut service, |seen| {
        changed(seen, 21, &a.join_name("one")) && changed(seen, 22, &b.join_name("one"))
    });

    service.unwatch_dir(&a);
    assert_eq!(service.watched_dirs(), 1);
    fs::write(tree.root.join("a/two"), "x").unwrap();
    let seen = land(&mut service, &tree.root.join("b"), &b, 22, "two");
    assert!(!seen.iter().any(|(id, _)| *id == 21), "{seen:?}");
    assert!(quiet(&mut service).iter().all(|(id, _)| *id != 21));

    // A Cancel of the request id releases the same way.
    service.cancel_watch_request(22);
    assert_eq!(service.watched_dirs(), 0);
    fs::write(tree.root.join("b/three"), "x").unwrap();
    assert!(quiet(&mut service).is_empty());

    // Resubscribing works, and a root switch drops everything.
    assert!(matches!(
        subscribe(&mut service, &b, 23),
        Operation::Changed { .. }
    ));
    land(&mut service, &tree.root.join("b"), &b, 23, "four");
    service.select(&a).unwrap();
    assert_eq!(service.watched_dirs(), 0);
    fs::write(tree.root.join("b/five"), "x").unwrap();
    assert!(quiet(&mut service).is_empty());
}

#[test]
fn the_watch_table_is_bounded_and_a_refusal_keeps_earlier_watches() {
    // The documented bound is `MAX_DIR_WATCHES`; the same check runs against a
    // small limit so the test does not pay for 32 platform subscriptions.
    const LIMIT: usize = 3;
    const { assert!(LIMIT < MAX_DIR_WATCHES) };
    let temp = tempfile::tempdir().unwrap();
    let root_path = fs::canonicalize(temp.path()).unwrap().join("root");
    fs::create_dir(&root_path).unwrap();
    let mut dirs = Vec::new();
    for index in 0..=LIMIT {
        let dir = root_path.join(format!("d{index:02}"));
        fs::create_dir(&dir).unwrap();
        dirs.push(canonical(&dir));
    }
    let root = canonical(&root_path);
    let mut service = Filesystem::new(&root, 1)
        .unwrap()
        .with_dir_watch_limit(LIMIT);
    for (index, dir) in dirs.iter().take(LIMIT).enumerate() {
        assert!(matches!(
            subscribe(&mut service, dir, 100 + index as u64),
            Operation::Changed { .. }
        ));
    }
    assert_eq!(service.watched_dirs(), LIMIT);
    // One more is refused with Limit; re-binding an existing one is not a new
    // watch.
    assert_eq!(service.watch_dir(&dirs[LIMIT], 999), Err(ErrorCode::Limit));
    assert!(matches!(
        subscribe(&mut service, &dirs[0], 5000),
        Operation::Changed { .. }
    ));
    assert_eq!(service.watched_dirs(), LIMIT);
    // Releasing one makes room, and the earlier watches kept working.
    service.unwatch_dir(&dirs[1]);
    assert!(matches!(
        subscribe(&mut service, &dirs[LIMIT], 999),
        Operation::Changed { .. }
    ));
    land(&mut service, &root_path.join("d02"), &dirs[2], 102, "x");
    // The root outside the table is not watched implicitly.
    fs::write(root_path.join("loose"), "x").unwrap();
    assert!(quiet(&mut service).is_empty());
}

#[test]
fn a_burst_in_one_directory_collapses_to_the_directory() {
    let tree = tree();
    let a = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&canonical(&tree.root), 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 31),
        Operation::Changed { .. }
    ));
    for index in 0..40 {
        fs::write(tree.root.join(format!("a/f{index}")), "x").unwrap();
    }
    let seen = gather(&mut service, |seen| changed(seen, 31, &a));
    assert!(
        seen.len() <= 8,
        "a burst is bounded, not one frame per file: {}",
        seen.len()
    );
    for (_, operation) in &seen {
        operation.validate().unwrap();
    }
}

#[test]
fn git_state_changes_are_reported_under_the_root_subscription() {
    let tree = tree();
    let root = canonical(&tree.root);
    git(&tree.root, &["init", "-b", "main"]);
    fs::write(tree.root.join("tracked.txt"), "one").unwrap();
    git(&tree.root, &["add", "-A"]);
    git(&tree.root, &["commit", "-m", "first"]);

    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &root, 40),
        Operation::Changed { .. }
    ));
    await_git(&mut service);
    quiet(&mut service);

    // Staging touches only the index (through index.lock, which is ignored).
    // The worktree entry is untracked content in a collapsed directory, not
    // an event of any watched directory.
    fs::create_dir(tree.root.join("collapsed")).unwrap();
    quiet(&mut service);
    fs::write(tree.root.join("collapsed/new.txt"), "n").unwrap();
    git(&tree.root, &["add", "collapsed/new.txt"]);
    let seen = gather(&mut service, |seen| git_changed(seen, 40));
    assert!(
        !seen
            .iter()
            .any(|(_, op)| matches!(op, Operation::Changed { path } if path.as_bytes().ends_with(b"index.lock"))),
        "{seen:?}"
    );

    // A commit moves the branch ref and HEAD's reflog target.
    quiet(&mut service);
    git(&tree.root, &["commit", "-m", "second"]);
    gather(&mut service, |seen| git_changed(seen, 40));

    // A new branch only adds a ref.
    quiet(&mut service);
    git(&tree.root, &["branch", "topic"]);
    gather(&mut service, |seen| git_changed(seen, 40));

    // Checkout rewrites HEAD.
    quiet(&mut service);
    git(&tree.root, &["checkout", "topic"]);
    gather(&mut service, |seen| git_changed(seen, 40));

    // Releasing the root releases the Git watch with it.
    service.unwatch_dir(&root);
    assert!(!service.watching_git());
    git(&tree.root, &["branch", "late"]);
    assert!(quiet(&mut service).is_empty());
}

#[test]
fn a_linked_worktree_watches_its_own_git_directory_and_the_shared_refs() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let main = base.join("main");
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-b", "main"]);
    fs::write(main.join("f.txt"), "one").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-m", "first"]);
    let linked = base.join("linked");
    git(
        &main,
        &["worktree", "add", "-b", "wt", linked.to_str().unwrap()],
    );

    // `.git` is a file in the worktree: `gitdir:` points into the main
    // repository's `worktrees/` directory, whose `commondir` points back.
    let dirs = resolve_git_dirs(&linked).unwrap();
    assert_eq!(
        dirs.common_dir,
        fs::canonicalize(main.join(".git")).unwrap()
    );
    assert_eq!(
        dirs.git_dir.parent().unwrap(),
        dirs.common_dir.join("worktrees")
    );

    let root = canonical(&linked);
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &root, 50),
        Operation::Changed { .. }
    ));
    await_git(&mut service);
    quiet(&mut service);

    // Staging in the worktree rewrites that worktree's own index.
    fs::write(linked.join("g.txt"), "g").unwrap();
    git(&linked, &["add", "g.txt"]);
    gather(&mut service, |seen| git_changed(seen, 50));

    // A commit on the worktree's branch moves a ref in the shared directory.
    quiet(&mut service);
    git(&linked, &["commit", "-m", "wt"]);
    gather(&mut service, |seen| git_changed(seen, 50));
}

#[test]
fn a_root_below_the_repository_top_still_watches_the_repository() {
    let tree = tree();
    git(&tree.root, &["init", "-b", "main"]);
    fs::write(tree.root.join("a/f.txt"), "one").unwrap();
    git(&tree.root, &["add", "-A"]);
    git(&tree.root, &["commit", "-m", "first"]);
    let sub = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&sub, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &sub, 60),
        Operation::Changed { .. }
    ));
    await_git(&mut service);
    quiet(&mut service);
    git(&tree.root, &["branch", "topic"]);
    gather(&mut service, |seen| git_changed(seen, 60));
}

#[test]
fn git_directory_resolution_follows_pointers_and_gives_up_cleanly() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();

    // No repository above: nothing to watch.
    let plain = base.join("plain/inner");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(resolve_git_dirs(&plain), None);

    // A plain repository found from a subdirectory.
    let repo = base.join("repo");
    fs::create_dir_all(repo.join("src/deep")).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let dirs = resolve_git_dirs(&repo.join("src/deep")).unwrap();
    assert_eq!(dirs.git_dir, repo.join(".git"));
    assert_eq!(dirs.common_dir, dirs.git_dir);

    // A `.git` file with a relative target, as submodules have.
    let sub = base.join("super/sub");
    fs::create_dir_all(sub.clone()).unwrap();
    fs::create_dir_all(base.join("super/.git/modules/sub")).unwrap();
    fs::write(
        base.join("super/.git/modules/sub/HEAD"),
        "ref: refs/heads/main\n",
    )
    .unwrap();
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    let dirs = resolve_git_dirs(&sub).unwrap();
    assert_eq!(dirs.git_dir, base.join("super/.git/modules/sub"));

    // A pointer to nothing is not a repository, and a hostile huge file is
    // read only up to its first line bound.
    let broken = base.join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), "gitdir: /does/not/exist\n").unwrap();
    assert_eq!(resolve_git_dirs(&broken), None);
    let huge = base.join("huge");
    fs::create_dir(&huge).unwrap();
    fs::write(huge.join(".git"), vec![b'x'; 1 << 20]).unwrap();
    assert_eq!(resolve_git_dirs(&huge), None);

    // A `.git` that exists but does not resolve ends the search: the
    // repository around it is not adopted in its place.
    let inner = repo.join("vendor/inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join(".git"), "gitdir: /does/not/exist\n").unwrap();
    assert_eq!(resolve_git_dirs(&inner), None);
    assert!(resolve_git_dirs(&repo.join("vendor")).is_some());

    // A directory that is not a Git directory (no `HEAD`) is no target, and a
    // `commondir` must be the directory the worktree's own one lives under.
    let fake = base.join("fake");
    fs::create_dir_all(fake.join(".git")).unwrap();
    assert_eq!(resolve_git_dirs(&fake), None);
    let stranger = base.join("stranger");
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(stranger.clone()).unwrap();
    fs::create_dir_all(elsewhere.join("gitdir")).unwrap();
    fs::write(elsewhere.join("gitdir/HEAD"), "ref: refs/heads/x\n").unwrap();
    fs::write(elsewhere.join("HEAD"), "ref: refs/heads/x\n").unwrap();
    fs::write(elsewhere.join("gitdir/commondir"), "..\n").unwrap();
    fs::write(
        stranger.join(".git"),
        format!("gitdir: {}\n", elsewhere.join("gitdir").display()),
    )
    .unwrap();
    assert_eq!(resolve_git_dirs(&stranger), None, "not under worktrees/");

    // `refs` is watched recursively only when it is a real directory.
    let common = base.join("repo/.git");
    fs::create_dir_all(common.join("refs/heads")).unwrap();
    assert_eq!(refs_directory(&common), Some(common.join("refs")));
    fs::remove_dir_all(common.join("refs")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, common.join("refs")).unwrap();
    assert_eq!(refs_directory(&common), None, "a link to another tree");
}

/// M2: a watched directory that is replaced (`rm -rf` and `mkdir`, a checkout,
/// `cargo clean`) loses its platform watch with its inode; the subscription
/// must watch the new directory without the client asking again.
#[test]
fn a_replaced_directory_is_watched_again() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 61),
        Operation::Changed { .. }
    ));
    land(&mut service, &tree.root.join("a"), &a, 61, "before");

    fs::remove_dir_all(tree.root.join("a")).unwrap();
    fs::create_dir(tree.root.join("a")).unwrap();
    // The subscription keeps reporting entries of the new directory: writes
    // are retried while it is being watched again.
    land(&mut service, &tree.root.join("a"), &a, 61, "after");
    assert_eq!(service.watched_dirs(), 1);

    // Replaced a second time, to show the re-armed watch is a working one.
    fs::remove_dir_all(tree.root.join("a")).unwrap();
    fs::create_dir(tree.root.join("a")).unwrap();
    land(&mut service, &tree.root.join("a"), &a, 61, "again");
}

/// A late registration result belongs to the subscription it was made for: a
/// folder released and asked for again while the first registration was still
/// pending is not live until its own registration completes.
#[test]
fn a_released_and_requested_again_directory_waits_for_its_own_registration() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert_eq!(service.watch_dir(&a, 71).unwrap(), WatchOutcome::Pending);
    service.unwatch_dir(&a);
    assert_eq!(service.watch_dir(&a, 72).unwrap(), WatchOutcome::Pending);
    // Whatever order results arrive in, nothing is reported for the released
    // request, and the new one is reported exactly once.
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut results = Vec::new();
    while !results.iter().any(|(id, _)| *id == 72) {
        results.extend(service.poll_watch_results());
        assert!(Instant::now() < deadline, "registration never completed");
        std::thread::sleep(Duration::from_millis(10));
    }
    results.extend(quiet_results(&mut service));
    assert!(results.iter().all(|(id, _)| *id == 72), "{results:?}");
    assert_eq!(results.len(), 1);
}

fn quiet_results(service: &mut Filesystem) -> Vec<(u64, Operation)> {
    let until = Instant::now() + Duration::from_secs(1);
    let mut out = Vec::new();
    while Instant::now() < until {
        out.extend(service.poll_watch_results());
        std::thread::sleep(Duration::from_millis(10));
    }
    out
}

/// m3: the platform watch of a path ends with its last holder, so releasing an
/// expanded `.git` cannot end the Git watch.
#[test]
fn releasing_an_expanded_git_directory_keeps_the_git_watch() {
    let tree = tree();
    let root = canonical(&tree.root);
    git(&tree.root, &["init", "-b", "main"]);
    fs::write(tree.root.join("f.txt"), "one").unwrap();
    git(&tree.root, &["add", "-A"]);
    git(&tree.root, &["commit", "-m", "first"]);
    let dot_git = canonical(&tree.root.join(".git"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &root, 81),
        Operation::Changed { .. }
    ));
    await_git(&mut service);
    assert!(matches!(
        subscribe(&mut service, &dot_git, 82),
        Operation::Changed { .. }
    ));
    service.unwatch_dir(&dot_git);
    quiet_results(&mut service);
    assert!(service.watching_git());
    // Git changes still arrive, retried across the stream restart a release
    // can cause on some platforms.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seen = false;
    let mut attempt = 0;
    while !seen && Instant::now() < deadline {
        attempt += 1;
        git(&tree.root, &["branch", "-f", &format!("kept{attempt}")]);
        let until = Instant::now() + Duration::from_secs(3);
        while !seen && Instant::now() < until {
            seen = git_changed(&service.poll_changes(), 81);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(seen, "the Git watch survived the release of `.git`");
}

/// m3: a repository created under a root that had none is watched from then on.
#[test]
fn a_repository_created_later_is_watched() {
    let tree = tree();
    let root = canonical(&tree.root);
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &root, 91),
        Operation::Changed { .. }
    ));
    assert!(!service.watching_git());
    git(&tree.root, &["init", "-b", "main"]);
    let deadline = Instant::now() + Duration::from_secs(120);
    while !service.watching_git() && Instant::now() < deadline {
        service.poll_changes();
        service.poll_watch_results();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(service.watching_git(), "the new repository is found");
}

/// M1 (server side): a backend that restarts its stream for every watch loses
/// the events in the gap, so after each registration every live directory is
/// reported once more (macOS FSEvents; inotify has no such gap).
#[cfg(target_os = "macos")]
#[test]
fn a_registration_reports_the_other_live_directories_again() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let b = canonical(&tree.root.join("b"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 101),
        Operation::Changed { .. }
    ));
    quiet(&mut service);
    assert!(matches!(
        subscribe(&mut service, &b, 102),
        Operation::Changed { .. }
    ));
    gather(&mut service, |seen| changed(seen, 101, &a));
}

/// R2-1: a burst of registrations is followed by one rescan, not one per
/// registration (each of which would reload every folder: quadratic work).
#[test]
fn a_burst_of_registrations_is_followed_by_one_rescan() {
    let tree = tree();
    let root = canonical(&tree.root);
    let dirs: Vec<_> = ["a", "b", "a/deep"]
        .iter()
        .map(|name| canonical(&tree.root.join(name)))
        .collect();
    let mut service = Filesystem::new(&root, 1).unwrap();
    for (index, dir) in dirs.iter().enumerate() {
        assert_eq!(
            service.watch_dir(dir, 200 + index as u64).unwrap(),
            WatchOutcome::Pending
        );
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut acknowledged = 0;
    while acknowledged < dirs.len() {
        acknowledged += service.poll_watch_results().len();
        assert!(Instant::now() < deadline, "registrations never completed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = quiet(&mut service);
    let first = seen
        .iter()
        .filter(|(id, op)| {
            *id == 200 && matches!(op, Operation::Changed { path } if *path == dirs[0])
        })
        .count();
    assert!(
        first <= 1,
        "one rescan for the burst, saw {first}: {seen:?}"
    );
}

/// R2-1: a watched directory that is removed and not recreated is retried
/// slowly, and the retries do not rescan every live directory.
#[test]
fn a_directory_that_stays_gone_does_not_storm() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let b = canonical(&tree.root.join("b"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 211),
        Operation::Changed { .. }
    ));
    assert!(matches!(
        subscribe(&mut service, &b, 212),
        Operation::Changed { .. }
    ));
    quiet(&mut service);
    fs::remove_dir_all(tree.root.join("a")).unwrap();
    let seen = quiet(&mut service);
    let reports = seen.iter().filter(|(id, _)| *id == 212).count();
    assert!(reports <= 2, "b is not reloaded again and again: {seen:?}");
}

/// R2-4: a watched directory that is moved away and replaced is watched anew,
/// and the moved directory is no longer reported under the old name.
#[test]
fn a_moved_away_directory_leaves_no_ghost_watch() {
    let tree = tree();
    let root = canonical(&tree.root);
    let a = canonical(&tree.root.join("a"));
    let mut service = Filesystem::new(&root, 1).unwrap();
    assert!(matches!(
        subscribe(&mut service, &a, 221),
        Operation::Changed { .. }
    ));
    fs::rename(tree.root.join("a"), tree.root.join("a.moved")).unwrap();
    fs::create_dir(tree.root.join("a")).unwrap();
    land(&mut service, &tree.root.join("a"), &a, 221, "fresh");
    // Writes into the moved directory are not reported as entries of `a`.
    let deadline = Instant::now() + Duration::from_secs(4);
    let ghost = a.join_name("ghost.txt");
    let mut leaked = false;
    while Instant::now() < deadline {
        fs::write(tree.root.join("a.moved/ghost.txt"), "x").unwrap();
        leaked |= changed(&service.poll_changes(), 221, &ghost);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!leaked, "the moved directory's watch is gone");
}

trait JoinName {
    fn join_name(&self, name: &str) -> RemotePath;
}
impl JoinName for RemotePath {
    fn join_name(&self, name: &str) -> RemotePath {
        let mut bytes = self.as_bytes().to_vec();
        if bytes != b"/" {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(name.as_bytes());
        RemotePath::new(bytes).unwrap()
    }
}
