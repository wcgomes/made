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
        assert!(*truncated, "truncation survives every batch");
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
