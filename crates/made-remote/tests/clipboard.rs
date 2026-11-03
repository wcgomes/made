//! Unix private-file uploads. Windows remotes are unsupported; these tests are
//! not a pass on that host.
#![cfg(unix)]
use base64::Engine;
use made_remote::{clipboard::Uploads, protocol::*};
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
