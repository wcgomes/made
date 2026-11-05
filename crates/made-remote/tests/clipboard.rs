//! Unix private-file uploads. Windows remotes are unsupported; these tests are
//! not a pass on that host.
#![cfg(unix)]
use base64::Engine;
use made_remote::{clipboard::Uploads, protocol::*};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime},
};

fn png_bytes() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=")
        .unwrap()
}

fn generated_name(token: u64, extension: &str) -> String {
    format!("clipboard-{token:032x}.{extension}")
}

fn set_age(path: &Path, age: Duration) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(SystemTime::now() - age))
        .unwrap();
}

fn generated_pngs(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("clipboard-")
                        && name.ends_with(".png")
                        && name.len() == "clipboard-".len() + 32 + 4
                })
        })
        .collect()
}

fn generated_partials(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("clipboard-")
                        && name.ends_with(".partial")
                        && name.len() == "clipboard-".len() + 32 + ".partial".len()
                })
        })
        .collect()
}

#[test]
fn completed_png_survives_upload_owner_drop_and_unrelated_files_survive_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let mut uploads = Uploads::new(directory.clone()).unwrap();
    std::fs::write(directory.join("unrelated.png"), "keep").unwrap();
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap();
    uploads.begin(1, png.len() as u64).unwrap();
    let split = png.len() / 2;
    uploads
        .chunk(1, 0, &Chunk::new(png[..split].to_vec()).unwrap())
        .unwrap();
    uploads
        .chunk(1, split as u64, &Chunk::new(png[split..].to_vec()).unwrap())
        .unwrap();
    let path = uploads.commit(1).unwrap();
    drop(uploads);
    assert!(std::path::Path::new(path.to_utf8().unwrap()).exists());
    assert_eq!(
        std::fs::read_to_string(directory.join("unrelated.png")).unwrap(),
        "keep"
    );
}
#[test]
fn images_are_private_and_commit_never_overwrites_an_existing_path() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let mut uploads = Uploads::new(directory.clone()).unwrap();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700,
        "the clipboard image directory is owner-only"
    );

    let png = png_bytes();
    uploads.begin(1, png.len() as u64).unwrap();
    let partial = generated_partials(&directory).pop().unwrap();
    assert_eq!(
        fs::metadata(&partial).unwrap().permissions().mode() & 0o777,
        0o600,
        "in-flight private files are owner-only"
    );
    uploads
        .chunk(1, 0, &Chunk::new(png.clone()).unwrap())
        .unwrap();
    let target = partial.with_extension("png");
    fs::write(&target, b"sentinel").unwrap();
    assert_eq!(uploads.commit(1), Err(ErrorCode::Io));
    assert_eq!(
        fs::read(&target).unwrap(),
        b"sentinel",
        "an existing path is never overwritten"
    );
    assert!(
        !partial.exists(),
        "a failed commit removes its partial file"
    );

    uploads.begin(2, png.len() as u64).unwrap();
    uploads
        .chunk(2, 0, &Chunk::new(png.clone()).unwrap())
        .unwrap();
    let path = PathBuf::from(uploads.commit(2).unwrap().to_utf8().unwrap());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600,
        "a committed PNG is owner-only"
    );
    assert_eq!(fs::read(&path).unwrap(), png);
}

#[test]
fn drop_and_disconnect_mid_upload_clean_partials_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let mut uploads = Uploads::new(directory.clone()).unwrap();
    let png = png_bytes();
    let split = png.len() / 2;
    uploads.begin(1, png.len() as u64).unwrap();
    uploads
        .chunk(1, 0, &Chunk::new(png[..split].to_vec()).unwrap())
        .unwrap();
    let partial = generated_partials(&directory).pop().unwrap();
    let lock = partial.with_extension("partial.lock");
    assert!(partial.exists() && lock.exists());
    assert!(generated_pngs(&directory).is_empty());

    uploads.pending_abort_all();
    assert!(!partial.exists() && !lock.exists());
    assert_eq!(uploads.commit(1), Err(ErrorCode::InvalidRequest));
    assert!(
        generated_pngs(&directory).is_empty(),
        "disconnect does not publish a partial upload"
    );

    uploads.begin(2, png.len() as u64).unwrap();
    uploads
        .chunk(2, 0, &Chunk::new(png.clone()).unwrap())
        .unwrap();
    let partial = generated_partials(&directory).pop().unwrap();
    let lock = partial.with_extension("partial.lock");
    assert!(partial.exists() && lock.exists());
    drop(uploads);
    assert!(
        !partial.exists() && !lock.exists(),
        "dropping the upload owner removes in-flight partials"
    );
    assert!(
        generated_pngs(&directory).is_empty(),
        "dropping the upload owner does not publish an incomplete paste"
    );

    let mut uploads = Uploads::new(directory.clone()).unwrap();
    assert_eq!(uploads.commit(2), Err(ErrorCode::InvalidRequest));
    assert!(
        generated_pngs(&directory).is_empty(),
        "a reconnect cannot publish a dropped upload"
    );
}

#[test]
fn retention_removes_only_expired_generated_regular_images() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let uploads = Uploads::new(directory.clone()).unwrap();
    let now = SystemTime::now();
    let expired: Vec<_> = (0..3)
        .map(|token| {
            let path = directory.join(generated_name(token, "png"));
            fs::write(&path, b"expired").unwrap();
            set_age(&path, Duration::from_secs(8 * 24 * 3600));
            path
        })
        .collect();
    let recent = directory.join(generated_name(10, "png"));
    fs::write(&recent, b"recent").unwrap();
    set_age(&recent, Duration::from_secs(60));
    let unrelated = directory.join("keep.png");
    fs::write(&unrelated, b"unrelated").unwrap();
    let malformed_name = directory.join("clipboard-not-a-token.png");
    fs::write(&malformed_name, b"malformed").unwrap();
    let target = directory.join("symlink-target");
    fs::write(&target, b"outside").unwrap();
    let link = directory.join(generated_name(11, "png"));
    symlink(&target, &link).unwrap();
    let directory_named = directory.join(generated_name(12, "png"));
    fs::create_dir(&directory_named).unwrap();

    uploads.cleanup(None, now).unwrap();

    for path in &expired {
        assert!(!path.exists(), "expired generated image is removed");
    }
    assert!(recent.exists(), "recent image is retained");
    assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated");
    assert_eq!(fs::read(&malformed_name).unwrap(), b"malformed");
    assert_eq!(fs::read(&target).unwrap(), b"outside");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(directory_named.is_dir());
}

#[test]
fn retention_caps_at_sixty_four_and_protects_the_current_image() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let uploads = Uploads::new(directory.clone()).unwrap();
    let now = SystemTime::now();
    let images: Vec<_> = (0..70)
        .map(|token| {
            let path = directory.join(generated_name(token, "png"));
            fs::write(&path, b"image").unwrap();
            set_age(&path, Duration::from_secs((70 - token) * 60));
            path
        })
        .collect();

    uploads.cleanup(Some(&images[69]), now).unwrap();
    assert!(images[69].exists(), "the just-created image survives");
    assert_eq!(generated_pngs(&directory).len(), 64);
    for path in &images[..6] {
        assert!(!path.exists(), "oldest images are removed first");
    }
    assert!(images[6].exists());

    uploads.cleanup(None, now).unwrap();
    assert_eq!(
        generated_pngs(&directory).len(),
        64,
        "an at-cap directory is left alone"
    );

    let extra = directory.join(generated_name(70, "png"));
    fs::write(&extra, b"new").unwrap();
    set_age(&extra, Duration::from_secs(10));
    uploads.cleanup(None, now).unwrap();
    assert!(extra.exists());
    assert!(!images[6].exists(), "65 images shed exactly the oldest one");
    assert_eq!(generated_pngs(&directory).len(), 64);
}

#[test]
fn partial_uploads_abort_in_flight_caps_and_crash_remnants_are_swept() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let mut uploads = Uploads::new(directory.clone()).unwrap();

    uploads.begin(1, 4).unwrap();
    uploads
        .chunk(1, 0, &Chunk::new(b"junk".to_vec()).unwrap())
        .unwrap();
    uploads.abort(1);
    assert!(uploads.commit(1).is_err());
    for entry in fs::read_dir(&directory).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(!name.to_string_lossy().contains(".partial"));
    }

    for id in 1..=MAX_IN_FLIGHT as u64 {
        uploads.begin(id, 4).unwrap();
    }
    assert!(uploads.begin(MAX_IN_FLIGHT as u64 + 1, 4).is_err());
    uploads.pending_abort_all();
    assert!(
        generated_partials(&directory).is_empty(),
        "disconnect aborts every in-flight partial"
    );
    assert!(
        generated_pngs(&directory).is_empty(),
        "disconnect does not publish an interrupted upload"
    );

    uploads.begin(2, 24).unwrap();
    uploads
        .chunk(2, 0, &Chunk::new(vec![b'x'; 24]).unwrap())
        .unwrap();
    assert_eq!(uploads.commit(2), Err(ErrorCode::InvalidRequest));
    assert_eq!(uploads.commit(2), Err(ErrorCode::InvalidRequest));
    drop(uploads);

    let crash = directory.join(generated_name(20, "partial"));
    let crash_lock = crash.with_extension("partial.lock");
    fs::write(&crash, b"crash").unwrap();
    fs::write(&crash_lock, b"").unwrap();
    let live = directory.join(generated_name(21, "partial"));
    let live_lock = live.with_extension("partial.lock");
    fs::write(&live, b"live").unwrap();
    let held = made_platform::lock_private_file(&live_lock, Duration::ZERO).unwrap();

    let uploads = Uploads::new(directory.clone()).unwrap();
    assert!(!crash.exists() && !crash_lock.exists());
    assert!(live.exists() && live_lock.exists(), "live uploads are kept");

    drop(held);
    uploads.cleanup(None, SystemTime::now()).unwrap();
    assert!(!live.exists() && !live_lock.exists());
}

#[test]
fn cleanup_failure_never_invalidates_a_completed_paste() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let mut uploads = Uploads::new(directory.clone()).unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let lock_path = directory.join("retention.lock");
    let holder = std::thread::spawn(move || {
        let _lock = made_platform::lock_private_file(&lock_path, Duration::from_secs(1)).unwrap();
        ready_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
    });
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let png = png_bytes();
    uploads.begin(7, png.len() as u64).unwrap();
    uploads
        .chunk(7, 0, &Chunk::new(png.clone()).unwrap())
        .unwrap();
    let started = SystemTime::now();
    let path = uploads.commit(7).unwrap();
    assert!(
        started.elapsed().unwrap_or_default() >= Duration::from_millis(900),
        "the cleanup lock was actually contended"
    );
    holder.join().unwrap();
    let path = Path::new(path.to_utf8().unwrap());
    assert!(path.exists(), "cleanup failure does not undo the paste");
    assert_eq!(fs::read(path).unwrap(), png);
    drop(uploads);
    assert!(path.exists(), "the completed image survives owner drop");
}

#[test]
fn cleanup_lock_permission_denied_reports_failure_and_removes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("clipboard-images");
    let uploads = Uploads::new(directory.clone()).unwrap();
    let now = SystemTime::now();
    let old = directory.join(generated_name(30, "png"));
    fs::write(&old, b"old").unwrap();
    set_age(&old, Duration::from_secs(8 * 24 * 3600));
    let current = directory.join(generated_name(31, "png"));
    fs::write(&current, b"current").unwrap();
    let unrelated = directory.join("keep.png");
    fs::write(&unrelated, b"keep").unwrap();

    // Remove the lock from startup cleanup so a read-only directory denies
    // creating it; this asserts the lock path, not a later unlink failure.
    fs::remove_file(directory.join("retention.lock")).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = directory.join("lock-probe");
    if fs::write(&probe, b"").is_ok() {
        let _ = fs::remove_file(&probe);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        eprintln!("skipping lock-permission assertion when running as root");
        return;
    }
    let outcome = uploads.cleanup(Some(&current), now);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        outcome,
        Err(ErrorCode::Io),
        "cleanup must not claim success when it cannot take its lock"
    );

    assert!(old.exists(), "a failed cleanup removes nothing");
    assert!(current.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
    uploads.cleanup(Some(&current), SystemTime::now()).unwrap();
    assert!(!old.exists(), "the next reachable cleanup reconciles");
    assert!(current.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
}

#[test]
fn bad_upload_order_incomplete_and_oversize_never_publish_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut uploads = Uploads::new(temp.path().join("images")).unwrap();
    assert!(uploads.begin(1, MAX_UPLOAD_BYTES + 1).is_err());
    uploads.begin(1, 10).unwrap();
    assert!(uploads.chunk(1, 1, &Chunk::new(vec![1]).unwrap()).is_err());
    assert!(uploads.commit(1).is_err());
    assert!(!std::fs::read_dir(temp.path().join("images"))
        .unwrap()
        .any(|e| e.unwrap().path().extension().is_some_and(|e| e == "png")));
}
