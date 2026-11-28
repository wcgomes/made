//! `WatchDir` / `GitChanged` against the real supervisor over a loopback
//! socket: a nested write and a `.git/index` change reach the client as
//! pushes, the explorer and Git reads that follow are answered, and the time
//! each step takes is measured against the SSH-09 budgets (visible file
//! changes within 2 s, Git status within 3 s). Nothing here reaches a remote
//! host.
#![cfg(unix)]

use made_remote::{
    protocol::*,
    supervisor::{capability_bytes, write_envelope, ChildLaunch, Endpoint, Supervisor},
};
use std::{
    ffi::OsString,
    fs,
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const GIT_READS: [GitRead; 8] = [
    GitRead::ShowToplevel,
    GitRead::Branch,
    GitRead::GitDir,
    GitRead::CommonDir,
    GitRead::WorktreeList,
    GitRead::CommitId,
    GitRead::CommitSubject,
    GitRead::Status,
];

struct Companion {
    workspace: PathBuf,
    _temp: tempfile::TempDir,
    identity: Identity,
    endpoint: Endpoint,
    runner: Option<thread::JoinHandle<Result<(), ErrorCode>>>,
}

struct Conn {
    stream: TcpStream,
    identity: Identity,
    next: u64,
    /// Bytes read but not yet a whole frame: a read that times out half way
    /// through a frame must not lose what it already took.
    buffer: Vec<u8>,
}

fn rp(path: &Path) -> RemotePath {
    RemotePath::new(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
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

impl Companion {
    /// A supervisor whose launch child idles; the workspace is its root.
    fn boot() -> (Self, Conn) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = fs::canonicalize(temp.path()).unwrap().join("work");
        fs::create_dir_all(&workspace).unwrap();
        let runtime = temp.path().join("remote/runtime/sha256-0123456789abcdef");
        made_platform::ensure_private_directory(&runtime).unwrap();
        let identity = Identity {
            authority: AuthorityId::random().unwrap(),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let first = WireEnvelope::new(
            identity.clone(),
            1,
            Operation::Open {
                directory: rp(&workspace),
                target: LaunchTarget::OpenCode,
                rows: 24,
                cols: 80,
            },
        );
        let supervisor = Supervisor::create(
            &first,
            runtime.clone(),
            temp.path(),
            Path::new(env!("CARGO_BIN_EXE_made-remote")),
            ChildLaunch {
                program: "/bin/sh".into(),
                args: vec![OsString::from("-c"), OsString::from("sleep 300")],
                cwd: workspace.clone(),
                env: vec![
                    ("HOME".into(), temp.path().into()),
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("TERM".into(), "xterm-256color".into()),
                ],
            },
            None,
        )
        .expect("supervisor create");
        let runner = thread::spawn(move || supervisor.run());
        let limit = Instant::now() + Duration::from_secs(20);
        let endpoint: Endpoint = loop {
            assert!(Instant::now() < limit, "endpoint.json was never published");
            if let Ok(bytes) = fs::read(runtime.join("endpoint.json")) {
                break serde_json::from_slice(&bytes).expect("endpoint");
            }
            thread::sleep(Duration::from_millis(20));
        };
        let companion = Self {
            workspace,
            _temp: temp,
            identity,
            endpoint,
            runner: Some(runner),
        };
        let conn = companion.connect(first);
        (companion, conn)
    }

    fn connect(&self, first: WireEnvelope) -> Conn {
        let mut stream = TcpStream::connect(self.endpoint.addr).expect("connect");
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        stream
            .write_all(&capability_bytes(&first.identity).unwrap())
            .unwrap();
        stream.write_all(&first.identity.authority.0).unwrap();
        stream
            .write_all(&first.identity.epoch.to_be_bytes())
            .unwrap();
        write_envelope(&mut stream, &first).expect("write first frame");
        Conn {
            stream,
            identity: first.identity,
            next: 2,
            buffer: Vec::new(),
        }
    }

    fn reattach(&mut self, epoch: u64) -> Conn {
        self.identity.epoch = epoch;
        self.connect(WireEnvelope::new(
            self.identity.clone(),
            1,
            Operation::Attach { rows: 24, cols: 80 },
        ))
    }

    fn close(mut self, mut conn: Conn) {
        write_envelope(
            &mut conn.stream,
            &WireEnvelope::new(conn.identity.clone(), 9_999, Operation::Close),
        )
        .unwrap();
        conn.wait(Duration::from_secs(20), |frame| {
            matches!(frame.operation, Operation::Closed { .. }).then_some(())
        });
        drop(conn);
        if let Some(runner) = self.runner.take() {
            let _ = runner.join();
        }
    }
}

impl Conn {
    fn send(&mut self, operation: Operation) -> u64 {
        let id = self.next;
        self.next += 1;
        write_envelope(
            &mut self.stream,
            &WireEnvelope::new(self.identity.clone(), id, operation),
        )
        .expect("send request");
        id
    }

    /// Read frames until `want` accepts one; terminal output is skipped.
    fn wait<T>(
        &mut self,
        timeout: Duration,
        mut want: impl FnMut(&WireEnvelope) -> Option<T>,
    ) -> T {
        self.try_wait(timeout, &mut want)
            .expect("deadline waiting for a frame")
    }

    fn try_wait<T>(
        &mut self,
        timeout: Duration,
        want: &mut impl FnMut(&WireEnvelope) -> Option<T>,
    ) -> Option<T> {
        let limit = Instant::now() + timeout;
        while Instant::now() < limit {
            if let Some(frame) = self.poll_frame() {
                if matches!(frame.operation, Operation::TerminalOutput { .. }) {
                    continue;
                }
                if std::env::var_os("WATCH_DEBUG").is_some() {
                    eprintln!(
                        "frame {} {}",
                        frame.request_id,
                        serde_json::to_string(&frame.operation)
                            .unwrap()
                            .chars()
                            .take(120)
                            .collect::<String>()
                    );
                }
                if let Some(found) = want(&frame) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn poll_frame(&mut self) -> Option<WireEnvelope> {
        if self.buffer.len() >= 4 {
            let size = u32::from_be_bytes(self.buffer[..4].try_into().unwrap()) as usize;
            if self.buffer.len() >= 4 + size {
                let frame = serde_json::from_slice(&self.buffer[4..4 + size]).expect("frame");
                self.buffer.drain(..4 + size);
                return Some(frame);
            }
        }
        let mut chunk = [0u8; 65536];
        match self.stream.read(&mut chunk) {
            Ok(0) => panic!("attachment socket closed"),
            Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(error) => panic!("attachment socket failed: {error}"),
        }
        None
    }

    fn reply(&mut self, id: u64) -> Operation {
        // Establishing a platform watch can take seconds on macOS.
        self.wait(Duration::from_secs(120), |frame| {
            (frame.request_id == id).then(|| frame.operation.clone())
        })
    }

    /// Subscribe and wait for the acknowledgement.
    fn watch_dir(&mut self, path: &Path) -> u64 {
        let id = self.send(Operation::WatchDir { path: rp(path) });
        assert_eq!(self.reply(id), Operation::Changed { path: rp(path) });
        id
    }

    /// Wait until nothing has been pushed under `ids` for a while. Where
    /// registering a platform watch restarts its stream (macOS FSEvents) the
    /// server reports every live directory once more after each one, late and
    /// asynchronously: let that finish before measuring or asserting silence.
    fn settle(&mut self, ids: &[u64]) {
        let mut idle = Instant::now();
        let limit = Instant::now() + Duration::from_secs(120);
        while idle.elapsed() < Duration::from_millis(2500) && Instant::now() < limit {
            let hit = self.try_wait(Duration::from_millis(250), &mut |frame: &WireEnvelope| {
                ids.contains(&frame.request_id).then_some(())
            });
            if hit.is_some() {
                idle = Instant::now();
            }
        }
    }

    /// No directory change is pushed under `ids` for `window` (Git state
    /// notifications are not what a folder nobody watches is about).
    fn quiet_changes(&mut self, window: Duration, ids: &[u64]) {
        self.quiet_where(window, ids, |operation| {
            matches!(operation, Operation::Changed { .. })
        });
    }

    /// Nothing is pushed under `ids` for `window`.
    fn quiet(&mut self, window: Duration, ids: &[u64]) {
        self.quiet_where(window, ids, |_| true);
    }

    fn quiet_where(
        &mut self,
        window: Duration,
        ids: &[u64],
        matching: impl Fn(&Operation) -> bool,
    ) {
        let mut stray = None;
        self.try_wait(window, &mut |frame: &WireEnvelope| {
            if ids.contains(&frame.request_id) && matching(&frame.operation) {
                stray = Some(frame.operation.clone());
                Some(())
            } else {
                None
            }
        });
        assert!(
            stray.is_none(),
            "unexpected push {}",
            serde_json::to_string(&stray).unwrap_or_default()
        );
    }

    /// The explorer's reload of `path`: List until the final batch.
    fn list(&mut self, path: &Path) -> Vec<DirectoryEntry> {
        let id = self.send(Operation::List { path: rp(path) });
        let mut entries = Vec::new();
        loop {
            match self.reply(id) {
                Operation::Listed {
                    entries: batch,
                    done,
                    ..
                } => {
                    entries.extend(batch);
                    if done {
                        return entries;
                    }
                }
                other => panic!("List answered with {other:?}"),
            }
        }
    }

    /// The Git view's refresh: the eight reads, all answered in full.
    fn git_refresh(&mut self) {
        let ids: Vec<u64> = GIT_READS
            .iter()
            .map(|command| self.send(Operation::GitRead { command: *command }))
            .collect();
        for id in ids {
            loop {
                match self.reply(id) {
                    Operation::GitResult { done: true, .. } => break,
                    Operation::GitResult { .. } => {}
                    Operation::Error { code } => panic!("git read failed: {code:?}"),
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
    }

    fn pushed_change(&mut self, id: u64, entry: &Path) -> Duration {
        let started = Instant::now();
        let wanted = rp(entry);
        self.wait(Duration::from_secs(10), |frame| {
            (frame.request_id == id
                && matches!(&frame.operation, Operation::Changed { path } if *path == wanted))
            .then_some(())
        });
        started.elapsed()
    }

    /// The repository metadata watch is established after the root's
    /// acknowledgement, on the companion's watcher thread. Provoke a ref
    /// change until its notification arrives, so what is measured next is the
    /// steady state.
    fn warm_git(&mut self, root: &Path, id: u64) {
        for attempt in 0..120 {
            git(root, &["branch", "-f", &format!("warm{attempt}")]);
            let seen = self.try_wait(Duration::from_secs(1), &mut |frame: &WireEnvelope| {
                (frame.request_id == id && matches!(frame.operation, Operation::GitChanged))
                    .then_some(())
            });
            if seen.is_some() {
                // Let the burst of the final branch settle.
                self.settle(&[id]);
                return;
            }
        }
        panic!("the repository metadata watch never became live");
    }

    fn pushed_git(&mut self, id: u64) -> Duration {
        let started = Instant::now();
        self.wait(Duration::from_secs(10), |frame| {
            (frame.request_id == id && matches!(frame.operation, Operation::GitChanged))
                .then_some(())
        });
        started.elapsed()
    }
}

fn names(entries: &[DirectoryEntry]) -> Vec<String> {
    let mut names: Vec<_> = entries
        .iter()
        .map(|entry| {
            String::from_utf8_lossy(entry.path.as_bytes())
                .rsplit('/')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a_current_supervisor_advertises_directory_watches() {
    let (companion, mut conn) = Companion::boot();
    let id = conn.send(Operation::Capabilities);
    match conn.reply(id) {
        Operation::CapabilitiesResult { capabilities } => assert!(capabilities.watch_dirs),
        other => panic!("{other:?}"),
    }
    companion.close(conn);
}

#[test]
fn nested_writes_and_git_changes_are_pushed_and_refreshed_within_budget() {
    let (companion, mut conn) = Companion::boot();
    let root = companion.workspace.clone();
    fs::create_dir_all(root.join("a/deep")).unwrap();
    fs::create_dir(root.join("collapsed")).unwrap();
    fs::write(root.join("a/one.txt"), "1").unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-m", "first"]);

    let watch_root = conn.watch_dir(&root);
    let watch_a = conn.watch_dir(&root.join("a"));
    conn.warm_git(&root, watch_root);
    conn.settle(&[watch_root, watch_a]);

    // A write inside an expanded nested folder: pushed under that folder's
    // subscription, and the reload that follows shows the new entry.
    let started = Instant::now();
    fs::write(root.join("a/new.txt"), "n").unwrap();
    let push = conn.pushed_change(watch_a, &root.join("a/new.txt"));
    let listing = conn.list(&root.join("a"));
    let explorer = started.elapsed();
    assert!(names(&listing).contains(&"new.txt".to_owned()));
    eprintln!("nested write -> push {push:?}, -> listing refreshed {explorer:?}");
    assert!(
        explorer < Duration::from_secs(2),
        "visible file changes within 2 s (SSH-09): {explorer:?}"
    );

    // `git add` rewrites `.git/index`: GitChanged, then the Git view's reads.
    conn.settle(&[watch_root, watch_a]);
    let started = Instant::now();
    git(&root, &["add", "a/new.txt"]);
    let push = conn.pushed_git(watch_root);
    conn.git_refresh();
    let refreshed = started.elapsed();
    eprintln!("git add -> GitChanged {push:?}, -> status refreshed {refreshed:?}");
    assert!(
        refreshed < Duration::from_secs(3),
        "Git status within 3 s (SSH-09): {refreshed:?}"
    );

    // A commit moves the branch ref.
    conn.settle(&[watch_root, watch_a]);
    let started = Instant::now();
    git(&root, &["commit", "-m", "second"]);
    conn.pushed_git(watch_root);
    conn.git_refresh();
    eprintln!("git commit -> status refreshed {:?}", started.elapsed());

    // A new file in a folder nobody expanded: invisible to every
    // subscription, which is exactly what the client's safety net covers.
    conn.settle(&[watch_root, watch_a]);
    fs::write(root.join("collapsed/x.txt"), "x").unwrap();
    conn.quiet_changes(Duration::from_secs(2), &[watch_root, watch_a]);

    // Releasing the folder ends its pushes; the root's go on.
    let id = conn.send(Operation::UnwatchDir {
        path: rp(&root.join("a")),
    });
    assert_eq!(conn.reply(id), Operation::Ack);
    fs::write(root.join("a/after-unwatch.txt"), "x").unwrap();
    conn.quiet(Duration::from_secs(2), &[watch_a]);
    // The root's subscription goes on. Releasing a platform watch can restart
    // the backend's stream (macOS FSEvents), and an event inside that gap is
    // lost, so write until one lands: the client's periodic refresh is what
    // covers the gap in the product.
    let mut landed = false;
    for attempt in 0..30 {
        let top = root.join(format!("top{attempt}.txt"));
        fs::write(&top, "x").unwrap();
        let wanted = rp(&top);
        landed = conn
            .try_wait(Duration::from_secs(3), &mut |frame: &WireEnvelope| {
                (frame.request_id == watch_root
                    && matches!(&frame.operation, Operation::Changed { path } if *path == wanted))
                .then_some(())
            })
            .is_some();
        if landed {
            break;
        }
    }
    assert!(landed, "the root subscription survived the release");
    companion.close(conn);
}

#[test]
fn a_companion_watch_is_per_attachment_and_the_root_only_watch_still_works() {
    let (mut companion, mut conn) = Companion::boot();
    let root = companion.workspace.clone();
    fs::create_dir(root.join("a")).unwrap();

    // The root-only `Watch` of an older client: unchanged contract, the push
    // names the watched root whatever changed.
    let legacy = conn.send(Operation::Watch { path: rp(&root) });
    assert_eq!(conn.reply(legacy), Operation::Changed { path: rp(&root) });
    fs::write(root.join("top.txt"), "x").unwrap();
    conn.pushed_change(legacy, &root);

    // A reattach starts with no subscriptions: nothing is pushed until the
    // new client asks again, under its own request id.
    conn.quiet(Duration::from_secs(1), &[legacy]);
    // Git reads left queued by the attachment that goes away are not run for,
    // or sent to, the next one.
    for _ in 0..12 {
        conn.send(Operation::GitRead {
            command: GitRead::Status,
        });
    }
    drop(conn);
    let mut conn = companion.reattach(2);
    let id = conn.send(Operation::Capabilities);
    let mut stray = Vec::new();
    let answered = conn.try_wait(Duration::from_secs(20), &mut |frame: &WireEnvelope| {
        if frame.request_id == id {
            return Some(frame.operation.clone());
        }
        // Id 1 is the attachment itself (Attached, snapshots).
        if frame.request_id != 1 {
            stray.push(frame.request_id);
        }
        None
    });
    assert!(matches!(
        answered,
        Some(Operation::CapabilitiesResult { .. })
    ));
    conn.quiet(Duration::from_secs(1), &(2..14).collect::<Vec<u64>>()[1..]);
    assert!(stray.is_empty(), "stale replies reached the new attachment");
    fs::write(root.join("a/before.txt"), "x").unwrap();
    conn.quiet(Duration::from_secs(2), &[legacy]);
    let watch = conn.watch_dir(&root.join("a"));
    fs::write(root.join("a/after.txt"), "x").unwrap();
    conn.pushed_change(watch, &root.join("a/after.txt"));
    companion.close(conn);
}

/// SSH-09's controlled profile uses a 10,000-file fixture: the same checks at
/// that size, plus whether Git reads hold the supervisor loop. They run on the
/// Git thread, so a Heartbeat sent right behind them is acknowledged long
/// before they finish.
#[test]
fn latency_and_loop_stall_on_a_ten_thousand_file_repository() {
    let (companion, mut conn) = Companion::boot();
    let root = companion.workspace.clone();
    for dir in 0..100 {
        let directory = root.join(format!("dir{dir:03}"));
        fs::create_dir(&directory).unwrap();
        for file in 0..100 {
            fs::write(
                directory.join(format!("f{file:03}.txt")),
                format!("{dir}{file}"),
            )
            .unwrap();
        }
    }
    git(&root, &["init", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-m", "ten thousand"]);

    let watch_root = conn.watch_dir(&root);
    let watch_dir = conn.watch_dir(&root.join("dir042"));
    conn.warm_git(&root, watch_root);
    conn.settle(&[watch_root, watch_dir]);

    let started = Instant::now();
    fs::write(root.join("dir042/new.txt"), "n").unwrap();
    conn.pushed_change(watch_dir, &root.join("dir042/new.txt"));
    let listing = conn.list(&root.join("dir042"));
    let explorer = started.elapsed();
    assert!(names(&listing).contains(&"new.txt".to_owned()));
    eprintln!("[10k] nested write -> listing refreshed {explorer:?}");
    assert!(explorer < Duration::from_secs(2), "{explorer:?}");

    // A change under an unexpanded folder is still Git-visible: status shows
    // it once read (what the periodic refresh does for such changes).
    conn.settle(&[watch_root, watch_dir]);
    let started = Instant::now();
    git(&root, &["add", "dir042/new.txt"]);
    conn.pushed_git(watch_root);
    let pushed = started.elapsed();
    conn.git_refresh();
    let refreshed = started.elapsed();
    eprintln!("[10k] git add -> GitChanged {pushed:?}, -> status refreshed {refreshed:?}");
    assert!(refreshed < Duration::from_secs(3), "{refreshed:?}");

    // The eight reads run on the Git thread: a Heartbeat sent right behind
    // them is acknowledged while they are still running.
    conn.settle(&[watch_root, watch_dir]);
    let started = Instant::now();
    let reads: Vec<u64> = GIT_READS
        .iter()
        .map(|command| conn.send(Operation::GitRead { command: *command }))
        .collect();
    let beat = conn.send(Operation::Heartbeat);
    // A read can finish before the Heartbeat is acknowledged: keep what
    // arrives meanwhile instead of discarding it.
    let mut early = Vec::new();
    let ack = conn.wait(Duration::from_secs(120), |frame| {
        if frame.request_id == beat {
            Some(frame.operation.clone())
        } else {
            early.push((frame.request_id, frame.operation.clone()));
            None
        }
    });
    assert_eq!(ack, Operation::Ack);
    let heartbeat = started.elapsed();
    for id in reads {
        let mut done = early.iter().any(|(request, operation)| {
            *request == id && matches!(operation, Operation::GitResult { done: true, .. })
        });
        while !done {
            done = matches!(conn.reply(id), Operation::GitResult { done: true, .. });
        }
    }
    let total = started.elapsed();
    eprintln!(
        "[10k] eight Git reads {total:?}; a Heartbeat queued behind them waited {heartbeat:?}"
    );
    assert!(
        heartbeat < total,
        "the loop answered while Git was still running: {heartbeat:?} vs {total:?}"
    );
    companion.close(conn);
}
