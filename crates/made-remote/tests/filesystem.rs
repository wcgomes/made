//! Unix path and private-root operations. Windows remotes are unsupported.
#![cfg(unix)]
use made_remote::{
    filesystem::{remote, Filesystem},
    protocol::*,
};
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{symlink, PermissionsExt},
        net::UnixListener,
    },
    path::Path,
    time::{Duration, Instant},
};

/// macOS FSEvents can lag well beyond the watcher's 150 ms debounce. Negative
/// assertions poll for this long so they cannot pass merely because the event
/// had not arrived yet. Positive controls on the same subscription prove the
/// watcher was live around each negative window.
const SETTLE: Duration = Duration::from_secs(2);
/// Generous upper bound for a condition-based positive wait under parallel CI.
const CHANGE_DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn guarded_list_preview_and_generation_are_server_scoped() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("unicode.txt"), "é".repeat(MAX_CHUNK_BYTES)).unwrap();
    fs::write(temp.path().join("binary"), b"hello\0world").unwrap();
    let root = remote(&fs::canonicalize(temp.path()).unwrap()).unwrap();
    let mut service = Filesystem::new(&root, 1).unwrap();
    let listings = service.list(&root).unwrap();
    assert!(matches!(
        listings.last(),
        Some(Operation::Listed { done: true, .. })
    ));
    for op in &listings {
        op.validate().unwrap();
    }
    let path = remote(&service.root().join("unicode.txt")).unwrap();
    let previews = service.preview(&path).unwrap();
    assert!(previews.len() > 1);
    for op in &previews {
        op.validate().unwrap();
    }
    let binary = remote(&service.root().join("binary")).unwrap();
    assert!(matches!(
        service.preview(&binary).unwrap()[0],
        Operation::PreviewResult {
            kind: PreviewKind::Binary,
            ..
        }
    ));
    service.select(&root).unwrap();
    assert_eq!(service.check(1), Err(ErrorCode::Stale));
}
#[test]
fn outside_root_never_reaches_a_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(temp.path().join("secret"), "not allowed").unwrap();
    let root = remote(&fs::canonicalize(root).unwrap()).unwrap();
    let service = Filesystem::new(&root, 1).unwrap();
    let outside = remote(&fs::canonicalize(temp.path()).unwrap().join("secret")).unwrap();
    assert_eq!(service.preview(&outside), Err(ErrorCode::Unauthorized));
}
fn canonical_remote(path: &Path) -> RemotePath {
    remote(&fs::canonicalize(path).unwrap()).unwrap()
}

fn listings(operations: &[Operation]) -> Vec<&DirectoryEntry> {
    operations
        .iter()
        .flat_map(|operation| match operation {
            Operation::Listed { entries, .. } => entries.iter().collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn wait_for_change(service: &mut Filesystem, expected: &RemotePath) {
    let deadline = Instant::now() + CHANGE_DEADLINE;
    loop {
        if let Some(operation) = service.poll_change() {
            assert_eq!(
                operation,
                Operation::Changed {
                    path: expected.clone()
                }
            );
            // A burst may queue more than one coalesced change; clear them so
            // the following negative window observes only new activity.
            drain_changes(service);
            return;
        }
        assert!(Instant::now() < deadline, "watch deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn drain_changes(service: &mut Filesystem) {
    while service.poll_change().is_some() {}
}

/// Asserts no change is delivered during `settle`, while polling continuously.
/// This is deliberately a bounded condition wait, not an immediate `None`
/// check: macOS FSEvents delivery can lag by more than a second.
fn assert_no_change_for(service: &mut Filesystem, settle: Duration) {
    let deadline = Instant::now() + settle;
    loop {
        if let Some(operation) = service.poll_change() {
            panic!("unexpected watch change during a negative window: {operation:?}");
        }
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Polls a real filesystem condition after draining pending changes. This
/// tolerates macOS splitting one logical operation into several FSEvents
/// notifications without weakening the assertion to a single early sample.
fn wait_until(
    service: &mut Filesystem,
    timeout: Duration,
    mut ready: impl FnMut(&Filesystem) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = service.poll_change();
        if ready(service) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn has_entry(service: &Filesystem, root: &RemotePath, name: &str) -> bool {
    listings(&service.list(root).unwrap())
        .iter()
        .any(|entry| entry.path.as_bytes().ends_with(name.as_bytes()))
}

fn preview_text(service: &Filesystem, path: &RemotePath) -> Vec<u8> {
    service
        .preview(path)
        .unwrap()
        .iter()
        .flat_map(|operation| match operation {
            Operation::PreviewResult { data, .. } => data.as_bytes().to_vec(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn listing_streams_eight_entry_batches_and_preserves_non_utf8_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    for index in 0..17 {
        fs::write(root_path.join(format!("file-{index:02}.txt")), b"").unwrap();
    }
    let raw_name = OsString::from_vec(b"raw-\xff.txt".to_vec());
    // macOS APFS rejects invalid UTF-8 file names with EILSEQ; Linux accepts
    // arbitrary POSIX path bytes. The wire conversion is exercised on both.
    let raw_created = fs::write(root_path.join(&raw_name), "raw text").is_ok();
    let root = canonical_remote(&root_path);
    let service = Filesystem::new(&root, 1).unwrap();

    let operations = service.list(&root).unwrap();
    assert_eq!(operations.len(), 3, "8 + 8 + the remaining 1 or 2 entries");
    let mut entries = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        operation.validate().unwrap();
        let Operation::Listed {
            path,
            entries: batch,
            truncated,
            done,
        } = operation
        else {
            panic!("list returned a non-listing operation");
        };
        assert_eq!(path, &root);
        assert!(!truncated);
        assert!(batch.len() <= 8);
        assert_eq!(*done, index + 1 == operations.len());
        entries.extend(batch.iter().cloned());
    }
    assert_eq!(entries.len(), if raw_created { 18 } else { 17 });

    let mut raw_bytes = root.as_bytes().to_vec();
    raw_bytes.push(b'/');
    raw_bytes.extend_from_slice(b"raw-\xff.txt");
    let raw_path = remote(&service.root().join(&raw_name)).unwrap();
    assert_eq!(raw_path.as_bytes(), raw_bytes);
    assert!(raw_path.to_utf8().is_err());
    if let Some(raw_entry) = entries
        .iter()
        .find(|entry| entry.path.as_bytes() == raw_bytes)
    {
        assert_eq!(raw_entry.kind, EntryKind::File);
        let previews = service.preview(&raw_entry.path).unwrap();
        let content: Vec<u8> = previews
            .iter()
            .flat_map(|operation| match operation {
                Operation::PreviewResult { data, .. } => data.as_bytes().to_vec(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(content, b"raw text");
    } else {
        assert!(!raw_created, "a host that accepts the name must list it");
        let previews = service.preview(&raw_path).unwrap();
        assert!(matches!(
            &previews[0],
            Operation::PreviewResult {
                kind: PreviewKind::Missing,
                ..
            }
        ));
    }
}

#[test]
fn directory_entry_cap_is_visible_without_a_synthetic_error_path() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    for index in 0..=MAX_DIRECTORY_ENTRIES {
        fs::write(root_path.join(format!("f-{index:05}")), b"").unwrap();
    }
    let root = canonical_remote(&root_path);
    let service = Filesystem::new(&root, 1).unwrap();

    let operations = service.list(&root).unwrap();
    assert_eq!(MAX_DIRECTORY_ENTRIES, 10_000);
    assert_eq!(
        operations.len(),
        MAX_DIRECTORY_ENTRIES.div_ceil(8),
        "only the 10,000 real entries are streamed"
    );
    let mut count = 0;
    for (index, operation) in operations.iter().enumerate() {
        operation.validate().unwrap();
        let Operation::Listed {
            entries,
            truncated,
            done,
            ..
        } = operation
        else {
            panic!("list returned a non-listing operation");
        };
        // The client rejects `truncated` on a non-final frame.
        assert_eq!(
            *truncated,
            index + 1 == operations.len(),
            "truncation rides on the final batch"
        );
        assert!(entries.len() <= 8);
        assert!(entries.iter().all(|entry| entry.kind == EntryKind::File));
        assert_eq!(*done, index + 1 == operations.len());
        count += entries.len();
    }
    assert_eq!(count, MAX_DIRECTORY_ENTRIES);
}

#[test]
fn preview_caps_classify_exact_chunks_oversize_invalid_utf8_and_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    fs::write(root_path.join("empty"), b"").unwrap();
    fs::write(root_path.join("chunked"), vec![b'a'; MAX_CHUNK_BYTES * 2]).unwrap();
    fs::write(
        root_path.join("too-large"),
        vec![b'a'; MAX_PREVIEW_BYTES as usize + 1],
    )
    .unwrap();
    fs::write(root_path.join("invalid"), [0xff, 0xfe]).unwrap();
    fs::create_dir(root_path.join("directory")).unwrap();
    let root = canonical_remote(&root_path);
    let service = Filesystem::new(&root, 1).unwrap();
    let path = |name: &str| remote(&fs::canonicalize(root_path.join(name)).unwrap()).unwrap();

    let empty = service.preview(&path("empty")).unwrap();
    assert_eq!(empty.len(), 1);
    assert!(matches!(
        &empty[0],
        Operation::PreviewResult {
            kind: PreviewKind::Text,
            offset: 0,
            data,
            done: true,
            ..
        } if data.as_bytes().is_empty()
    ));

    let chunked = service.preview(&path("chunked")).unwrap();
    assert_eq!(chunked.len(), 2);
    for (index, operation) in chunked.iter().enumerate() {
        operation.validate().unwrap();
        let Operation::PreviewResult {
            kind,
            offset,
            data,
            done,
            ..
        } = operation
        else {
            panic!("preview returned a non-preview operation");
        };
        assert_eq!(*kind, PreviewKind::Text);
        assert_eq!(*offset, (index * MAX_CHUNK_BYTES) as u64);
        assert_eq!(data.as_bytes().len(), MAX_CHUNK_BYTES);
        assert_eq!(*done, index == 1);
    }

    let too_large = service.preview(&path("too-large")).unwrap();
    assert!(matches!(
        &too_large[0],
        Operation::PreviewResult {
            kind: PreviewKind::TooLarge,
            done: true,
            data,
            ..
        } if data.as_bytes().is_empty()
    ));
    let invalid = service.preview(&path("invalid")).unwrap();
    assert!(matches!(
        &invalid[0],
        Operation::PreviewResult {
            kind: PreviewKind::InvalidUtf8,
            done: true,
            data,
            ..
        } if data.as_bytes().is_empty()
    ));
    let directory = service.preview(&path("directory")).unwrap();
    assert!(matches!(
        &directory[0],
        Operation::PreviewResult {
            kind: PreviewKind::NotRegular,
            ..
        }
    ));
    let missing = service
        .preview(&remote(&root_path.join("missing")).unwrap())
        .unwrap();
    assert!(matches!(
        &missing[0],
        Operation::PreviewResult {
            kind: PreviewKind::Missing,
            ..
        }
    ));
    assert_eq!(
        service.list(&path("empty")),
        Err(ErrorCode::Unavailable),
        "a file is never listed as a directory"
    );
}

#[test]
fn unreadable_child_directory_is_typed_unavailable_without_hiding_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    fs::write(root_path.join("visible.txt"), "visible").unwrap();
    let locked_path = root_path.join("locked");
    fs::create_dir(&locked_path).unwrap();
    fs::write(locked_path.join("hidden.txt"), "hidden").unwrap();
    fs::set_permissions(&locked_path, fs::Permissions::from_mode(0o000)).unwrap();

    if fs::read_dir(&locked_path).is_ok() {
        fs::set_permissions(&locked_path, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipping unreadable-child assertion when running as root");
        return;
    }

    let root = canonical_remote(&root_path);
    let locked = canonical_remote(&locked_path);
    let service = Filesystem::new(&root, 1).unwrap();

    let operations = service.list(&root).unwrap();
    let entries = listings(&operations);
    assert!(
        entries.iter().any(|entry| {
            entry.path.as_bytes().ends_with(b"/locked") && entry.kind == EntryKind::Directory
        }),
        "the unreadable directory remains visible as a directory entry"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.path.as_bytes().ends_with(b"/visible.txt")),
        "an unreadable child never hides its readable siblings"
    );

    let unreadable = service.list(&locked);
    let preview = service.preview(&locked).unwrap();
    fs::set_permissions(&locked_path, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        unreadable,
        Err(ErrorCode::Unavailable),
        "the server reports an unreadable child instead of an empty listing"
    );
    assert!(
        matches!(
            &preview[0],
            Operation::PreviewResult {
                kind: PreviewKind::NotRegular,
                ..
            }
        ),
        "a directory preview never opens the unreadable child"
    );
    assert!(
        service.list(&locked).is_ok(),
        "restoring permissions proves the failure came from the child's mode"
    );
}

#[test]
fn symlink_escape_is_listed_but_outside_root_following_is_forbidden() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "outside secret").unwrap();
    fs::write(root_path.join("inside.txt"), "inside").unwrap();
    fs::create_dir(root_path.join("directory")).unwrap();
    symlink(&outside, root_path.join("escape")).unwrap();
    symlink(root_path.join("inside.txt"), root_path.join("inside-link")).unwrap();
    let _socket = UnixListener::bind(root_path.join("socket")).unwrap();
    let root = canonical_remote(&root_path);
    let service = Filesystem::new(&root, 1).unwrap();

    let operations = service.list(&root).unwrap();
    let entries = listings(&operations);
    let kind = |name: &str| {
        entries
            .iter()
            .find(|entry| {
                entry
                    .path
                    .as_bytes()
                    .ends_with(format!("/{name}").as_bytes())
            })
            .map(|entry| entry.kind)
            .unwrap()
    };
    assert_eq!(kind("escape"), EntryKind::Symlink);
    assert_eq!(kind("inside-link"), EntryKind::Symlink);
    assert_eq!(kind("socket"), EntryKind::Other);
    assert_eq!(kind("directory"), EntryKind::Directory);

    let escape = remote(&root_path.join("escape")).unwrap();
    assert_eq!(service.preview(&escape), Err(ErrorCode::Unauthorized));
    let socket = remote(&root_path.join("socket")).unwrap();
    let preview = service.preview(&socket).unwrap();
    assert!(matches!(
        &preview[0],
        Operation::PreviewResult {
            kind: PreviewKind::NotRegular,
            ..
        }
    ));
    let inside_link = remote(&root_path.join("inside-link")).unwrap();
    let preview = service.preview(&inside_link).unwrap();
    assert!(matches!(
        &preview[0],
        Operation::PreviewResult {
            kind: PreviewKind::Text,
            ..
        }
    ));

    let outside_remote = remote(&fs::canonicalize(&outside).unwrap()).unwrap();
    assert_eq!(service.list(&outside_remote), Err(ErrorCode::Unauthorized));
    let mut service = service;
    assert_eq!(service.watch(&outside_remote), Err(ErrorCode::Unauthorized));
    assert_eq!(
        RemotePath::new([root.as_bytes(), b"/../outside.txt"].concat()),
        Err(ProtocolError::Path),
        "lexical traversal cannot even be represented on the wire"
    );
    assert!(
        matches!(
            Filesystem::new(&remote(&outside).unwrap(), 1),
            Err(ErrorCode::Unavailable)
        ),
        "a file cannot become a trusted root"
    );
    assert!(matches!(
        Filesystem::new(&root, 0),
        Err(ErrorCode::InvalidRequest)
    ));
}

#[test]
fn watch_is_non_recursive_and_cancel_or_root_switch_drops_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    let sub_path = root_path.join("sub");
    let nested_path = sub_path.join("nested");
    fs::create_dir_all(&nested_path).unwrap();
    let root = canonical_remote(&root_path);
    let sub = canonical_remote(&sub_path);
    let mut service = Filesystem::new(&root, 1).unwrap();

    // Positive control: this exact subscription is live before the negative.
    service.watch(&sub).unwrap();
    fs::write(sub_path.join("direct.txt"), "direct").unwrap();
    wait_for_change(&mut service, &sub);

    // A non-recursive subscription never reports a nested write. The settle
    // window covers macOS FSEvents coalescing latency; the positive control
    // above and below proves the watcher is otherwise live.
    fs::write(nested_path.join("nested.txt"), "nested").unwrap();
    assert_no_change_for(&mut service, SETTLE);

    service.cancel_watch();
    fs::write(sub_path.join("after-cancel.txt"), "cancelled").unwrap();
    assert_no_change_for(&mut service, SETTLE);

    // Root switch: prove the root subscription was live, then select a new
    // root and verify writes under the old root produce nothing.
    service.watch(&root).unwrap();
    fs::write(root_path.join("root-direct.txt"), "root").unwrap();
    wait_for_change(&mut service, &root);
    let selected = service.select(&sub).unwrap();
    assert!(matches!(
        selected,
        Operation::RootResult {
            ref requested,
            ref path,
            generation: 2,
        } if requested == &sub && path == &sub
    ));
    assert_eq!(service.check(1), Err(ErrorCode::Stale));
    assert_eq!(service.check(2), Ok(()));
    fs::write(sub_path.join("while-unwatched.txt"), "ignored").unwrap();
    assert_no_change_for(&mut service, SETTLE);

    // A fresh subscription after the switch is live again.
    service.watch(&sub).unwrap();
    fs::write(sub_path.join("watched.txt"), "watched").unwrap();
    wait_for_change(&mut service, &sub);

    let missing = remote(&root_path.join("missing")).unwrap();
    assert_eq!(service.select(&missing), Err(ErrorCode::Unavailable));
    assert_eq!(service.check(2), Ok(()), "failed roots never advance");
    let validated = service.validate_root(&root).unwrap();
    assert!(matches!(
        validated,
        Operation::RootResult { generation: 2, .. }
    ));
    assert_eq!(service.check(2), Ok(()), "validation is read-only");

    let mut max = Filesystem::new(&root, u64::MAX).unwrap();
    assert_eq!(max.select(&root), Err(ErrorCode::Limit));
    assert_eq!(max.generation(), u64::MAX);
    assert_eq!(max.check(u64::MAX), Ok(()));
}

#[test]
fn watch_change_is_a_rescan_signal_for_listing_and_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("root");
    fs::create_dir(&root_path).unwrap();
    let existing = root_path.join("existing.txt");
    fs::write(&existing, "before").unwrap();
    let root = canonical_remote(&root_path);
    let mut service = Filesystem::new(&root, 1).unwrap();
    service.watch(&root).unwrap();

    // Create: Changed is the rescan signal, and a reload sees the new entry.
    let created = root_path.join("created.txt");
    fs::write(&created, "created").unwrap();
    wait_for_change(&mut service, &root);
    assert!(
        wait_until(&mut service, CHANGE_DEADLINE, |service| has_entry(
            service,
            &root,
            "/created.txt"
        )),
        "a rescan reloads the listing with the created entry"
    );

    // Modify: reloading the affected preview returns the new content.
    fs::write(&existing, "after").unwrap();
    wait_for_change(&mut service, &root);
    assert!(
        wait_until(&mut service, CHANGE_DEADLINE, |service| {
            preview_text(service, &canonical_remote(&existing)) == b"after"
        }),
        "a rescan reloads the affected preview"
    );

    // Rename: a rescan reports the new name and no stale old name.
    let renamed = root_path.join("renamed.txt");
    fs::rename(&created, &renamed).unwrap();
    wait_for_change(&mut service, &root);
    assert!(
        wait_until(&mut service, CHANGE_DEADLINE, |service| {
            has_entry(service, &root, "/renamed.txt") && !has_entry(service, &root, "/created.txt")
        }),
        "a rescan replaces the renamed entry without a stale old name"
    );

    // Delete: a rescan removes the entry instead of serving stale metadata.
    fs::remove_file(&renamed).unwrap();
    wait_for_change(&mut service, &root);
    assert!(
        wait_until(&mut service, CHANGE_DEADLINE, |service| !has_entry(
            service,
            &root,
            "/renamed.txt"
        )),
        "a rescan removes a deleted entry from the listing"
    );
}

// Screen-scoped remote browse grant (SSH-12), exercised end to end through the
// companion's `--stdio` entry point exactly as the SSH channel reaches it.

use made_remote::{
    git::{is_read_only_git_args, GitCommand},
    sanitize::sanitize_display,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
};

struct BrowseRun {
    status: ExitStatus,
    replies: Vec<BrowseEnvelope>,
    raw: Vec<u8>,
    home: tempfile::TempDir,
    cwd: tempfile::TempDir,
}

impl BrowseRun {
    fn replies_to(&self, request_id: u64) -> Vec<&BrowseOperation> {
        self.replies
            .iter()
            .filter(|reply| reply.request_id == request_id)
            .map(|reply| &reply.operation)
            .collect()
    }
    /// The grant writes nothing: no runtime, data or state directory under
    /// the remote user's home, and nothing in the helper's working directory.
    fn assert_wrote_nothing(&self) {
        assert_eq!(fs::read_dir(self.home.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(self.cwd.path()).unwrap().count(), 0);
    }
}

fn browse_grant() -> BrowseIdentity {
    BrowseIdentity {
        authority: AuthorityId([5; 16]),
        grant: BrowseGrant::random().unwrap(),
        epoch: 1,
    }
}

fn browse_frames(identity: &BrowseIdentity, operations: Vec<BrowseOperation>) -> Vec<u8> {
    operations
        .into_iter()
        .enumerate()
        .flat_map(|(index, operation)| {
            encode_browse(&BrowseEnvelope::new(
                identity.clone(),
                index as u64 + 1,
                operation,
            ))
            .unwrap()
        })
        .collect()
}

fn raw_frame(value: &serde_json::Value) -> Vec<u8> {
    let payload = serde_json::to_vec(value).unwrap();
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend(payload);
    bytes
}

/// Run one companion `--stdio` session with an isolated, empty home and
/// working directory. The session must end within bounded time.
fn run_browse(input: &[u8]) -> BrowseRun {
    run_browse_with_home(input, HomeEnv::Isolated)
}

/// What the helper process's own `HOME` is for one session.
enum HomeEnv<'a> {
    Isolated,
    Set(&'a Path),
    Unset,
}

fn run_browse_with_home(input: &[u8], home_env: HomeEnv<'_>) -> BrowseRun {
    run_browse_with_env(input, home_env, &[])
}

/// One session with extra variables in the helper's environment.
fn run_browse_with_env(
    input: &[u8],
    home_env: HomeEnv<'_>,
    extra: &[(&str, &std::ffi::OsStr)],
) -> BrowseRun {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_made-remote"));
    command.arg("--stdio").env_clear();
    command.envs(extra.iter().copied());
    match home_env {
        HomeEnv::Isolated => {
            command.env("HOME", home.path());
        }
        HomeEnv::Set(path) => {
            command.env("HOME", path);
        }
        HomeEnv::Unset => {}
    }
    let mut child = command
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .current_dir(cwd.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    {
        // The helper may end the grant before reading everything.
        let mut stdin = child.stdin.take().unwrap();
        let _ = stdin.write_all(input);
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the browse helper did not finish within bounded time");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let raw = reader.join().unwrap();
    let mut decoder = BrowseFrameDecoder::new();
    let mut rest = raw.as_slice();
    let mut replies = Vec::new();
    while !rest.is_empty() {
        let (consumed, frame) = decoder.push(rest).unwrap();
        rest = &rest[consumed..];
        replies.extend(frame);
    }
    decoder.finish().unwrap();
    BrowseRun {
        status,
        replies,
        raw,
        home,
        cwd,
    }
}

fn checked(run: &BrowseRun, request_id: u64) -> DirectoryCheck {
    match run.replies_to(request_id).as_slice() {
        [BrowseOperation::BrowseChecked { result, .. }] => result.clone(),
        other => panic!("request {request_id}: unexpected replies {}", other.len()),
    }
}

/// Every frame of one listing passes the client's bounded progress check.
fn browse_listing(
    run: &BrowseRun,
    identity: &BrowseIdentity,
    request_id: u64,
    path: &RemotePath,
) -> (Vec<DirectoryEntry>, bool) {
    let mut progress = BrowseReplyProgress::new(BrowseEnvelope::new(
        identity.clone(),
        request_id,
        BrowseOperation::BrowseList { path: path.clone() },
    ))
    .unwrap();
    let mut entries = Vec::new();
    for reply in run.replies.iter().filter(|r| r.request_id == request_id) {
        progress.accept(reply).unwrap();
        match &reply.operation {
            BrowseOperation::BrowseListed { entries: batch, .. } => {
                entries.extend(batch.iter().cloned())
            }
            _ => panic!("request {request_id}: listing failed"),
        }
    }
    progress.finish().unwrap();
    (entries, progress.is_truncated())
}

fn entry_kind(entries: &[DirectoryEntry], name: &[u8]) -> Option<EntryKind> {
    entries
        .iter()
        .find(|entry| entry.path.as_bytes().rsplit(|b| *b == b'/').next() == Some(name))
        .map(|entry| entry.kind)
}

fn native_remote(path: &Path) -> RemotePath {
    remote(path).unwrap()
}

#[test]
fn browse_grant_checks_absolute_directories_on_the_server() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    fs::create_dir(base.join("project")).unwrap();
    fs::write(base.join("notes.txt"), "notes").unwrap();
    symlink(base.join("project"), base.join("linked")).unwrap();
    symlink(base.join("gone"), base.join("dangling")).unwrap();
    fs::create_dir(base.join("locked")).unwrap();
    fs::set_permissions(base.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let root_bypasses_modes = fs::read_dir(base.join("locked")).is_ok();

    let identity = browse_grant();
    let check = |name: &str| BrowseOperation::BrowseCheck {
        path: native_remote(&base.join(name)),
    };
    let run = run_browse(&browse_frames(
        &identity,
        vec![
            BrowseOperation::BrowseOpen,
            check("project"),
            check("notes.txt"),
            check("missing"),
            check("linked"),
            check("dangling"),
            check("locked"),
            check("notes.txt/child"),
            BrowseOperation::BrowseClose,
        ],
    ));
    fs::set_permissions(base.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(run.status.success());
    assert_eq!(
        run.replies_to(1),
        [&BrowseOperation::BrowseOpened {
            home: Some(native_remote(&fs::canonicalize(run.home.path()).unwrap())),
        }],
        "the open reply carries the helper user's canonical home"
    );
    let project = native_remote(&base.join("project"));
    assert_eq!(
        checked(&run, 2),
        DirectoryCheck::Directory {
            path: project.clone()
        }
    );
    assert_eq!(checked(&run, 3), DirectoryCheck::NotDirectory);
    assert_eq!(checked(&run, 4), DirectoryCheck::Missing);
    assert_eq!(
        checked(&run, 5),
        DirectoryCheck::Directory { path: project },
        "a symlinked directory resolves to the server's canonical path"
    );
    assert_eq!(checked(&run, 6), DirectoryCheck::Missing);
    if root_bypasses_modes {
        eprintln!("skipping unreadable-directory assertion when running as root");
    } else {
        assert_eq!(checked(&run, 7), DirectoryCheck::Unreadable);
    }
    assert_eq!(checked(&run, 8), DirectoryCheck::Missing);
    assert_eq!(run.replies_to(9), [&BrowseOperation::BrowseClosed]);
    for reply in &run.replies {
        assert!(reply.identity.matches(&identity));
    }
    run.assert_wrote_nothing();

    // Absolute-path validation happens on the server too: a relative or
    // traversing path is a malformed frame that ends the grant unanswered.
    let request = serde_json::to_value(BrowseEnvelope::new(
        identity.clone(),
        2,
        BrowseOperation::BrowseCheck {
            path: native_remote(&base),
        },
    ))
    .unwrap();
    for hostile in [
        b"project".to_vec(),
        [base.to_str().unwrap().as_bytes(), b"/../etc"].concat(),
        [base.to_str().unwrap().as_bytes(), b"/./project"].concat(),
        b"".to_vec(),
    ] {
        let mut frame = request.clone();
        frame["operation"]["data"]["path"] = serde_json::json!(hostile);
        let mut input = browse_frames(&identity, vec![BrowseOperation::BrowseOpen]);
        input.extend(raw_frame(&frame));
        let run = run_browse(&input);
        assert!(!run.status.success());
        assert_eq!(run.replies.len(), 1, "only the open is answered");
        assert!(run.replies_to(2).is_empty());
        run.assert_wrote_nothing();
    }
}

#[test]
fn browse_listing_is_capped_at_1000_entries_with_folders_kept() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let big = base.join("big");
    fs::create_dir(&big).unwrap();
    for i in 0..1100 {
        fs::write(big.join(format!("file-{i:04}")), "").unwrap();
    }
    for i in 0..5 {
        fs::create_dir(big.join(format!("zz-folder-{i}"))).unwrap();
    }
    let small = base.join("small");
    fs::create_dir(&small).unwrap();
    fs::create_dir(small.join("one")).unwrap();

    let identity = browse_grant();
    let big_path = native_remote(&big);
    let small_path = native_remote(&small);
    let run = run_browse(&browse_frames(
        &identity,
        vec![
            BrowseOperation::BrowseOpen,
            BrowseOperation::BrowseList {
                path: big_path.clone(),
            },
            BrowseOperation::BrowseList {
                path: small_path.clone(),
            },
            BrowseOperation::BrowseClose,
        ],
    ));
    assert!(run.status.success());
    let (entries, truncated) = browse_listing(&run, &identity, 2, &big_path);
    assert_eq!(entries.len(), MAX_BROWSE_ENTRIES);
    assert!(truncated, "the cap is reported, never silent");
    for i in 0..5 {
        assert_eq!(
            entry_kind(&entries, format!("zz-folder-{i}").as_bytes()),
            Some(EntryKind::Directory),
            "folders survive the cap"
        );
    }
    assert!(entries.iter().all(|e| e.path.is_direct_child_of(&big_path)));
    let (entries, truncated) = browse_listing(&run, &identity, 3, &small_path);
    assert!(!truncated);
    assert_eq!(entries.len(), 1);
    assert_eq!(entry_kind(&entries, b"one"), Some(EntryKind::Directory));
    run.assert_wrote_nothing();
}

#[test]
fn browse_listing_reports_symlinks_without_following_them() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::create_dir(outside.join("inner")).unwrap();
    fs::write(base.join("secret.txt"), "outside secret bytes").unwrap();
    fs::create_dir(root.join("real")).unwrap();
    symlink(base.join("secret.txt"), root.join("escape")).unwrap();
    symlink(&outside, root.join("outside-dir")).unwrap();

    let identity = browse_grant();
    let root_path = native_remote(&root);
    let through_link = native_remote(&root.join("outside-dir"));
    let run = run_browse(&browse_frames(
        &identity,
        vec![
            BrowseOperation::BrowseOpen,
            BrowseOperation::BrowseList {
                path: root_path.clone(),
            },
            BrowseOperation::BrowseList {
                path: through_link.clone(),
            },
            BrowseOperation::BrowseCheck {
                path: native_remote(&root.join("escape")),
            },
            BrowseOperation::BrowseList {
                path: native_remote(&root.join("escape")),
            },
            BrowseOperation::BrowseGitRead {
                path: native_remote(&root.join("escape")),
                command: GitRead::Status,
            },
            BrowseOperation::BrowseClose,
        ],
    ));
    assert!(run.status.success());
    let (entries, _) = browse_listing(&run, &identity, 2, &root_path);
    assert_eq!(entry_kind(&entries, b"escape"), Some(EntryKind::Symlink));
    assert_eq!(
        entry_kind(&entries, b"outside-dir"),
        Some(EntryKind::Symlink)
    );
    assert_eq!(entry_kind(&entries, b"real"), Some(EntryKind::Directory));
    // Navigating through a link lists its target's children under the
    // requested spelling, so every entry stays a direct child of the request.
    let (entries, _) = browse_listing(&run, &identity, 3, &through_link);
    assert_eq!(entry_kind(&entries, b"inner"), Some(EntryKind::Directory));
    assert!(entries
        .iter()
        .all(|e| e.path.is_direct_child_of(&through_link)));
    assert_eq!(checked(&run, 4), DirectoryCheck::NotDirectory);
    assert!(matches!(
        run.replies_to(5).as_slice(),
        [BrowseOperation::Error { .. }]
    ));
    assert!(matches!(
        run.replies_to(6).as_slice(),
        [BrowseOperation::Error { .. }]
    ));
    assert!(!run
        .raw
        .windows(b"outside secret".len())
        .any(|w| w == b"outside secret"));
    run.assert_wrote_nothing();
}

#[test]
fn browse_never_reads_contents_executes_names_or_accepts_other_verbs() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let dir = base.join("hostile");
    fs::create_dir(&dir).unwrap();
    let names = [
        "$(touch PWNED)",
        "-rf",
        "; touch PWNED2",
        "--upload-pack=touch PWNED3",
        "line\nbreak",
        "bidi\u{202E}txt",
        "`touch PWNED4`",
    ];
    for name in names {
        fs::create_dir(dir.join(name)).unwrap();
    }
    // Opening a FIFO for reading would block: the helper must only stat it.
    let fifo = dir.join("pipe");
    let has_fifo = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|s| s.success());
    fs::write(dir.join("secret"), "never read").unwrap();
    fs::set_permissions(dir.join("secret"), fs::Permissions::from_mode(0o000)).unwrap();

    let identity = browse_grant();
    let dir_path = native_remote(&dir);
    let mut operations = vec![
        BrowseOperation::BrowseOpen,
        BrowseOperation::BrowseList {
            path: dir_path.clone(),
        },
        BrowseOperation::BrowseCheck {
            path: native_remote(&dir.join("secret")),
        },
        BrowseOperation::BrowseCheck {
            path: native_remote(&fifo),
        },
    ];
    for name in names {
        operations.push(BrowseOperation::BrowseGitRead {
            path: native_remote(&dir.join(name)),
            command: GitRead::ShowToplevel,
        });
    }
    operations.push(BrowseOperation::BrowseClose);
    let run = run_browse(&browse_frames(&identity, operations));
    fs::set_permissions(dir.join("secret"), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(run.status.success());

    let (entries, _) = browse_listing(&run, &identity, 2, &dir_path);
    for name in names {
        assert_eq!(
            entry_kind(&entries, name.as_bytes()),
            Some(EntryKind::Directory),
            "names travel as exact bytes"
        );
        let shown = sanitize_display(name, 255);
        assert!(!shown
            .chars()
            .any(|c| c.is_control() || made_remote::sanitize::is_format_control(c)));
    }
    if has_fifo {
        assert_eq!(entry_kind(&entries, b"pipe"), Some(EntryKind::Other));
        assert_eq!(checked(&run, 4), DirectoryCheck::NotDirectory);
    }
    assert_eq!(checked(&run, 3), DirectoryCheck::NotDirectory);
    assert!(!run.raw.windows(10).any(|w| w == b"never read"));
    for (base_dir, label) in [
        (dir.as_path(), "workspace"),
        (run.cwd.path(), "cwd"),
        (run.home.path(), "home"),
    ] {
        for pwned in ["PWNED", "PWNED2", "PWNED3", "PWNED4"] {
            assert!(
                !base_dir.join(pwned).exists(),
                "{label}: a name was executed"
            );
        }
    }
    run.assert_wrote_nothing();

    // No content-reading, watching, terminal or upload verb exists for the
    // browse grant: such a frame ends the grant unanswered.
    let template = serde_json::to_value(BrowseEnvelope::new(
        identity.clone(),
        2,
        BrowseOperation::BrowseList {
            path: native_remote(&dir.join("secret")),
        },
    ))
    .unwrap();
    for op in [
        "Preview",
        "Refresh",
        "Watch",
        "Open",
        "TerminalInput",
        "UploadBegin",
        "RootSelect",
        "BrowseRead",
    ] {
        let mut frame = template.clone();
        frame["operation"]["op"] = serde_json::json!(op);
        let mut input = browse_frames(&identity, vec![BrowseOperation::BrowseOpen]);
        input.extend(raw_frame(&frame));
        let run = run_browse(&input);
        assert!(!run.status.success(), "{op}");
        assert!(run.replies_to(2).is_empty(), "{op}");
        run.assert_wrote_nothing();
    }
}

fn git(directory: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "-c",
            "user.name=Made Test",
            "-c",
            "user.email=made@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn tree_snapshot(path: &Path, into: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            tree_snapshot(&entry.path(), into);
        } else {
            let bytes = if kind.is_file() {
                fs::read(entry.path()).unwrap()
            } else {
                Vec::new()
            };
            into.insert(entry.path(), bytes);
        }
    }
}

#[test]
fn browse_git_reads_are_the_existing_read_only_set() {
    for command in [
        GitRead::ShowToplevel,
        GitRead::Branch,
        GitRead::GitDir,
        GitRead::CommonDir,
        GitRead::WorktreeList,
        GitRead::CommitId,
        GitRead::CommitSubject,
        GitRead::Status,
    ] {
        assert!(is_read_only_git_args(GitCommand::from(command).args()));
    }

    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let repo = base.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("tracked.txt"), "one").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "-q", "-m", "first subject"]);
    git(&repo, &["worktree", "add", "-q", "../linked", "-b", "side"]);
    fs::write(repo.join("tracked.txt"), "two").unwrap();
    fs::write(repo.join("untracked.txt"), "new").unwrap();
    let mut before = BTreeMap::new();
    tree_snapshot(&base, &mut before);

    let identity = browse_grant();
    let repo_path = native_remote(&repo);
    let commands = [
        GitRead::ShowToplevel,
        GitRead::Branch,
        GitRead::GitDir,
        GitRead::CommonDir,
        GitRead::WorktreeList,
        GitRead::CommitId,
        GitRead::CommitSubject,
        GitRead::Status,
    ];
    let mut operations = vec![BrowseOperation::BrowseOpen];
    for command in commands {
        operations.push(BrowseOperation::BrowseGitRead {
            path: repo_path.clone(),
            command,
        });
    }
    operations.push(BrowseOperation::BrowseGitRead {
        path: native_remote(&base.join("linked")),
        command: GitRead::Branch,
    });
    operations.push(BrowseOperation::BrowseClose);
    let run = run_browse(&browse_frames(&identity, operations));
    assert!(run.status.success());

    let output = |request_id: u64, path: &RemotePath, command: GitRead| {
        let mut progress = BrowseReplyProgress::new(BrowseEnvelope::new(
            identity.clone(),
            request_id,
            BrowseOperation::BrowseGitRead {
                path: path.clone(),
                command,
            },
        ))
        .unwrap();
        let mut bytes = Vec::new();
        for reply in run.replies.iter().filter(|r| r.request_id == request_id) {
            progress.accept(reply).unwrap();
            match &reply.operation {
                BrowseOperation::BrowseGitResult { data, .. } => {
                    bytes.extend_from_slice(data.as_bytes())
                }
                _ => panic!("{command:?} failed"),
            }
        }
        progress.finish().unwrap();
        String::from_utf8(bytes).unwrap()
    };
    assert_eq!(
        output(2, &repo_path, GitRead::ShowToplevel).trim(),
        repo.to_str().unwrap()
    );
    assert_eq!(output(3, &repo_path, GitRead::Branch).trim(), "main");
    assert!(output(6, &repo_path, GitRead::WorktreeList).contains("linked"));
    let commit = output(7, &repo_path, GitRead::CommitId);
    assert!(!commit.trim().is_empty() && commit.trim().bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        output(8, &repo_path, GitRead::CommitSubject).trim(),
        "first subject"
    );
    let status = output(9, &repo_path, GitRead::Status);
    assert!(status.contains(" M tracked.txt"));
    assert!(status.contains("?? untracked.txt"));
    assert_eq!(
        output(10, &native_remote(&base.join("linked")), GitRead::Branch).trim(),
        "side"
    );

    // No byte of the repository, its metadata or the linked worktree changed.
    let mut after = BTreeMap::new();
    tree_snapshot(&base, &mut after);
    assert_eq!(before, after, "a browse Git read wrote to the repository");
    run.assert_wrote_nothing();

    // Anything outside the closed read set is not representable.
    let template = serde_json::to_value(BrowseEnvelope::new(
        identity.clone(),
        2,
        BrowseOperation::BrowseGitRead {
            path: repo_path,
            command: GitRead::Status,
        },
    ))
    .unwrap();
    for command in [
        serde_json::json!("Checkout"),
        serde_json::json!("Fetch"),
        serde_json::json!(["status", "--porcelain"]),
        serde_json::json!({"Status": {"args": ["gc"]}}),
    ] {
        let mut frame = template.clone();
        frame["operation"]["data"]["command"] = command.clone();
        let mut input = browse_frames(&identity, vec![BrowseOperation::BrowseOpen]);
        input.extend(raw_frame(&frame));
        let run = run_browse(&input);
        assert!(!run.status.success(), "{command}");
        assert!(run.replies_to(2).is_empty(), "{command}");
    }
}

#[test]
fn browse_grant_ends_on_close_and_never_crosses_grants() {
    let temp = tempfile::tempdir().unwrap();
    let base = native_remote(&fs::canonicalize(temp.path()).unwrap());
    let identity = browse_grant();
    let list = || BrowseOperation::BrowseList { path: base.clone() };

    // Close ends the grant: later requests get no answer.
    let run = run_browse(&browse_frames(
        &identity,
        vec![
            BrowseOperation::BrowseOpen,
            list(),
            BrowseOperation::BrowseClose,
            list(),
        ],
    ));
    assert!(run.status.success());
    assert!(!run.replies_to(2).is_empty());
    assert_eq!(run.replies_to(3), [&BrowseOperation::BrowseClosed]);
    assert!(run.replies_to(4).is_empty());
    run.assert_wrote_nothing();

    // The transport closing ends the grant too.
    let run = run_browse(&browse_frames(&identity, vec![BrowseOperation::BrowseOpen]));
    assert!(run.status.success());
    assert_eq!(run.replies.len(), 1);
    run.assert_wrote_nothing();

    // A second open under the live grant is refused without ending it.
    let run = run_browse(&browse_frames(
        &identity,
        vec![
            BrowseOperation::BrowseOpen,
            BrowseOperation::BrowseOpen,
            list(),
            BrowseOperation::BrowseClose,
        ],
    ));
    assert!(run.status.success());
    assert_eq!(
        run.replies_to(2),
        [&BrowseOperation::Error {
            code: ErrorCode::InvalidRequest
        }]
    );
    assert!(!run.replies_to(3).is_empty());

    // A frame under another grant, authority or epoch ends the grant
    // unanswered, and nothing after it is served.
    let mut foreign_grant = identity.clone();
    foreign_grant.grant = BrowseGrant::random().unwrap();
    let mut foreign_authority = identity.clone();
    foreign_authority.authority = AuthorityId([6; 16]);
    let mut stale_epoch = identity.clone();
    stale_epoch.epoch = 2;
    for foreign in [foreign_grant, foreign_authority, stale_epoch] {
        let mut input = browse_frames(&identity, vec![BrowseOperation::BrowseOpen]);
        input.extend(encode_browse(&BrowseEnvelope::new(foreign, 2, list())).unwrap());
        input.extend(encode_browse(&BrowseEnvelope::new(identity.clone(), 3, list())).unwrap());
        let run = run_browse(&input);
        assert!(!run.status.success());
        assert_eq!(run.replies.len(), 1, "only the open is answered");
        run.assert_wrote_nothing();
    }

    // A tab frame cannot address the browse grant.
    let tab = Identity {
        authority: identity.authority,
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let mut input = browse_frames(&identity, vec![BrowseOperation::BrowseOpen]);
    input.extend(
        encode(&WireEnvelope::new(
            tab,
            2,
            Operation::List { path: base.clone() },
        ))
        .unwrap(),
    );
    let run = run_browse(&input);
    assert!(!run.status.success());
    assert_eq!(run.replies.len(), 1);
    run.assert_wrote_nothing();

    // A browse grant is opened explicitly first; no other verb opens one.
    for first in [list(), BrowseOperation::BrowseClose] {
        let run = run_browse(&browse_frames(&identity, vec![first]));
        assert!(!run.status.success());
        assert!(run.replies.is_empty());
        run.assert_wrote_nothing();
    }
}

#[test]
fn browse_open_reports_the_helper_users_home_only_when_it_is_a_readable_directory() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let real_home = base.join("real-home");
    fs::create_dir(&real_home).unwrap();
    symlink(&real_home, base.join("linked-home")).unwrap();
    fs::write(base.join("file-home"), "not a directory").unwrap();
    let locked_home = base.join("locked-home");
    fs::create_dir(&locked_home).unwrap();
    fs::set_permissions(&locked_home, fs::Permissions::from_mode(0o000)).unwrap();
    let root_bypasses_modes = fs::read_dir(&locked_home).is_ok();

    let identity = browse_grant();
    let open = |home_env| {
        let run = run_browse_with_home(
            &browse_frames(
                &identity,
                vec![BrowseOperation::BrowseOpen, BrowseOperation::BrowseClose],
            ),
            home_env,
        );
        assert!(run.status.success());
        assert_eq!(run.replies_to(2), [&BrowseOperation::BrowseClosed]);
        match run.replies_to(1).as_slice() {
            [BrowseOperation::BrowseOpened { home }] => home.clone(),
            _ => panic!("the open was not answered"),
        }
    };
    let canonical = native_remote(&real_home);
    let (linked_home, file_home, missing_home) = (
        base.join("linked-home"),
        base.join("file-home"),
        base.join("missing-home"),
    );
    assert_eq!(open(HomeEnv::Set(&real_home)), Some(canonical.clone()));
    assert_eq!(
        open(HomeEnv::Set(&linked_home)),
        Some(canonical),
        "a symlinked home is reported by its canonical server path"
    );
    assert_eq!(open(HomeEnv::Set(&file_home)), None);
    assert_eq!(open(HomeEnv::Set(&missing_home)), None);
    assert_eq!(open(HomeEnv::Set(Path::new("relative/home"))), None);
    assert_eq!(open(HomeEnv::Unset), None);
    let locked = open(HomeEnv::Set(&locked_home));
    fs::set_permissions(&locked_home, fs::Permissions::from_mode(0o755)).unwrap();
    if root_bypasses_modes {
        eprintln!("skipping unreadable-home assertion when running as root");
    } else {
        assert_eq!(locked, None);
    }
    // Resolving the home reads nothing and writes nothing into it.
    assert_eq!(fs::read_dir(&real_home).unwrap().count(), 0);
}

#[test]
fn browse_git_reads_never_run_repository_configured_commands_or_inherit_git_env() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let repo = base.join("repo");
    let other = base.join("other");
    for directory in [&repo, &other] {
        fs::create_dir(directory).unwrap();
        git(directory, &["init", "-q"]);
        fs::write(directory.join("tracked.txt"), "one").unwrap();
        git(directory, &["add", "tracked.txt"]);
        git(directory, &["commit", "-q", "-m", "first"]);
    }
    let marker = base.join("marker");
    let script = base.join("hook.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &repo,
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    git(
        &repo,
        &["config", "filter.evil.clean", script.to_str().unwrap()],
    );
    git(&repo, &["config", "filter.evil.required", "true"]);
    fs::write(repo.join(".git/info/attributes"), "* filter=evil\n").unwrap();
    fs::write(repo.join("tracked.txt"), "two").unwrap();
    fs::write(other.join("other-only.txt"), "x").unwrap();

    let identity = browse_grant();
    let operations = vec![
        BrowseOperation::BrowseOpen,
        BrowseOperation::BrowseGitRead {
            path: native_remote(&repo),
            command: GitRead::Status,
        },
        BrowseOperation::BrowseGitRead {
            path: native_remote(&repo),
            command: GitRead::ShowToplevel,
        },
        BrowseOperation::BrowseClose,
    ];
    let other_git = other.join(".git");
    let parameters = std::ffi::OsString::from(format!("'core.fsmonitor'='{}'", script.display()));
    let run = run_browse_with_env(
        &browse_frames(&identity, operations),
        HomeEnv::Isolated,
        &[
            ("GIT_DIR", other_git.as_os_str()),
            ("GIT_WORK_TREE", other.as_os_str()),
            ("GIT_CONFIG_PARAMETERS", parameters.as_os_str()),
        ],
    );
    assert!(run.status.success());
    let text = |request_id: u64| -> String {
        let mut bytes = Vec::new();
        for reply in run.replies_to(request_id) {
            match reply {
                BrowseOperation::BrowseGitResult { data, .. } => {
                    bytes.extend_from_slice(data.as_bytes())
                }
                other => panic!("request {request_id}: {other:?}"),
            }
        }
        String::from_utf8(bytes).unwrap()
    };
    let status = text(2);
    assert!(status.contains(" M tracked.txt"), "{status:?}");
    assert!(!status.contains("other-only"), "inherited GIT_DIR was used");
    assert_eq!(text(3).trim(), repo.to_str().unwrap());
    assert!(!marker.exists(), "a repository-configured command ran");
}

#[test]
fn a_tab_listing_at_the_entry_cap_marks_only_its_final_frame_truncated() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_DIRECTORY_ENTRIES + 5 {
        fs::write(temp.path().join(format!("f{index:05}")), b"").unwrap();
    }
    let root = remote(&temp.path().canonicalize().unwrap()).unwrap();
    let service = Filesystem::new(&root, 1).unwrap();
    let frames = service.list(&root).unwrap();
    assert!(frames.len() > 1);
    // Helper side: `truncated` rides only on the final frame.
    for (index, frame) in frames.iter().enumerate() {
        let Operation::Listed {
            truncated, done, ..
        } = frame
        else {
            panic!("frame {index} is not a listing");
        };
        let last = index + 1 == frames.len();
        assert_eq!(*done, last, "frame {index}");
        assert_eq!(*truncated, last, "frame {index}");
    }
    // Client side: every frame passes the bounded progress check.
    let identity = Identity {
        authority: AuthorityId([7; 16]),
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let request = WireEnvelope::new(identity.clone(), 9, Operation::List { path: root.clone() });
    let mut progress = ReplyProgress::new(request).unwrap();
    let mut listed = 0;
    for frame in frames {
        if let Operation::Listed { entries, .. } = &frame {
            listed += entries.len();
        }
        progress
            .accept(&WireEnvelope::new(identity.clone(), 9, frame))
            .unwrap();
    }
    assert_eq!(listed, MAX_DIRECTORY_ENTRIES);
    assert!(progress.is_truncated());
}
