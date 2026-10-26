use std::fs;
use std::io::Write;

use made_explorer::preview::{
    Preview, PreviewKind, PreviewLoader, BINARY_SNIFF_LEN, PREVIEW_SIZE_LIMIT,
};
use made_explorer::root_switch::RootGeneration;

fn fixture_root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn text_file_is_returned_as_read_only_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("greeting.txt");
    let content = "hello\nworld \u{1f642}\n";
    fs::write(&file, content).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::Text);
    assert!(preview.is_text());
    assert_eq!(preview.text(), Some(content));
    assert_eq!(preview.size(), Some(content.len() as u64));
    assert!(preview.message().is_none());
    assert_eq!(preview.path(), fs::canonicalize(&file).unwrap());
}

#[test]
fn binary_file_shows_a_safe_message_and_no_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("binary.bin");
    let bytes = b"hello\0world\0hidden";
    fs::write(&file, bytes).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::Binary);
    assert!(preview.text().is_none());
    assert_eq!(preview.size(), Some(bytes.len() as u64));
    let message = preview.message().unwrap();
    assert!(message.contains("Binary file"));
    assert!(message.contains(&bytes.len().to_string()));
    assert!(!message.contains("hidden"));
}

#[test]
fn large_file_reports_size_and_limit_without_reading_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("large.log");
    let size = PREVIEW_SIZE_LIMIT + 123;
    let handle = fs::File::create(&file).unwrap();
    handle.set_len(size).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::TooLarge);
    assert!(preview.text().is_none());
    assert_eq!(preview.size(), Some(size));

    let message = preview.message().unwrap();
    assert!(message.contains(&size.to_string()));
    assert!(message.contains(&PREVIEW_SIZE_LIMIT.to_string()));
    assert!(message.contains("2 MiB"));
}

#[test]
fn invalid_utf8_is_reported_without_replacement_noise() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("truncated.txt");
    // A truncated multi-byte sequence is invalid UTF-8.
    fs::write(&file, [0x66, 0x6f, 0x80, 0x80]).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::InvalidUtf8);
    assert!(preview.text().is_none());
    let message = preview.message().unwrap();
    assert!(message.contains("UTF-8"));
    assert!(message.contains("4 bytes"));
}

#[test]
fn missing_and_deleted_files_are_reported_as_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let loader = PreviewLoader::new(&root).unwrap();

    let missing = loader.load(root.join("never-existed.txt"));
    assert_eq!(missing.kind(), PreviewKind::Missing);
    assert!(missing.text().is_none());

    let deleted_path = root.join("deleted.txt");
    fs::write(&deleted_path, "gone soon").unwrap();
    fs::remove_file(&deleted_path).unwrap();
    let deleted = loader.load(&deleted_path);
    assert_eq!(deleted.kind(), PreviewKind::Missing);
}

#[test]
fn directory_is_reported_as_not_a_regular_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let loader = PreviewLoader::new(&root).unwrap();

    let preview = loader.load(&root);
    assert_eq!(preview.kind(), PreviewKind::Unreadable);
    assert!(preview.text().is_none());
    assert!(preview.message().unwrap().contains("regular file"));
}

#[test]
fn outside_root_reads_are_forbidden() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "secret").unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&outside);
    assert_eq!(preview.kind(), PreviewKind::Forbidden);
    assert!(preview.text().is_none());
    assert!(!preview.message().unwrap().contains("secret"));
}

#[test]
fn stale_generation_is_rejected_before_reading() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("file.txt");
    fs::write(&file, "content").unwrap();

    let generation = RootGeneration::new(3);
    let loader = PreviewLoader::with_generation(&root, generation).unwrap();
    let error = loader.load_for(&file, RootGeneration::new(2)).unwrap_err();
    assert_eq!(error.expected, generation);
    assert_eq!(error.received, RootGeneration::new(2));
    assert_eq!(
        loader.load_for(&file, generation).unwrap().kind(),
        PreviewKind::Text
    );
}

#[test]
fn nul_after_the_sniff_window_is_not_a_binary_signal() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("sniff.txt");
    let mut bytes = vec![b'a'; BINARY_SNIFF_LEN + 100];
    bytes.push(0);
    bytes.push(b'b');
    fs::write(&file, &bytes).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::Text);
    assert!(preview.text().unwrap().contains('\0'));
}

#[test]
fn file_at_exact_limit_is_previewed() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("limit.txt");
    let mut handle = fs::File::create(&file).unwrap();
    handle
        .write_all(&vec![b'a'; PREVIEW_SIZE_LIMIT as usize])
        .unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);
    assert_eq!(preview.kind(), PreviewKind::Text);
    assert_eq!(preview.size(), Some(PREVIEW_SIZE_LIMIT));
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_forbidden() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "secret").unwrap();
    let link = root.join("link.txt");
    symlink(&outside, &link).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&link);
    assert_eq!(preview.kind(), PreviewKind::Forbidden);
    assert!(preview.text().is_none());
}

#[cfg(unix)]
#[test]
fn fifo_is_rejected_without_blocking_on_open() {
    use std::os::unix::fs::FileTypeExt;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let fifo = root.join("pipe");

    let status = Command::new("mkfifo").arg(&fifo).status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("skipping FIFO assertion because mkfifo is unavailable");
        return;
    }
    let metadata = fs::metadata(&fifo).unwrap();
    if !metadata.file_type().is_fifo() {
        eprintln!("skipping FIFO assertion because the fixture is not a FIFO");
        return;
    }

    let loader = PreviewLoader::new(&root).unwrap();
    let fifo_path = fifo.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = sender.send(loader.load(&fifo_path));
    });

    // If the loader ever performs a blocking open on the FIFO, this test fails
    // instead of hanging the suite. The test itself never opens the FIFO.
    let preview = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("FIFO preview must not block");
    worker.join().unwrap();

    assert_eq!(preview.kind(), PreviewKind::Unreadable);
    assert!(preview.text().is_none());
    assert!(preview.message().unwrap().contains("regular file"));
}

#[cfg(unix)]
#[test]
fn unreadable_file_shows_a_safe_message() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("secret.txt");
    fs::write(&file, "secret").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();

    let loader = PreviewLoader::new(&root).unwrap();
    let preview = loader.load(&file);

    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

    if preview.kind() == PreviewKind::Text {
        // Running as root bypasses the permission bits.
        eprintln!("skipping unreadable-file assertion when running as root");
        return;
    }
    assert_eq!(preview.kind(), PreviewKind::Unreadable);
    assert!(preview.text().is_none());
    assert!(preview.message().unwrap().contains("unreadable"));
}

#[test]
fn preview_loader_can_be_cloned_for_background_reads() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("file.txt");
    fs::write(&file, "background").unwrap();
    let loader = PreviewLoader::new(&root).unwrap();
    let clone = loader.clone();

    let handle = std::thread::spawn(move || clone.load(&file));
    let preview = handle.join().unwrap();
    assert_eq!(preview.kind(), PreviewKind::Text);
    assert_eq!(preview.text(), Some("background"));
}

#[test]
fn preview_variants_do_not_offer_content_for_non_text_results() {
    let preview = Preview::Binary {
        path: "file.bin".into(),
        size: 4,
        message: "Binary file (4 bytes) is not rendered in the preview.".into(),
    };
    assert!(!preview.is_text());
    assert!(preview.text().is_none());
}
