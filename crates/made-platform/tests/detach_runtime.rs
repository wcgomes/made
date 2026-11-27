//! Detached spawns share one private per-user runtime directory under the
//! temp directory instead of creating a new one per spawn that nothing ever
//! removes.
//!
//! Keep this the only test in this binary: it points `TMPDIR` at its own
//! temporary directory for the whole process.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_platform::spawn_detached_command;

#[test]
fn detached_spawns_reuse_one_private_runtime_directory() {
    let temp_root = tempfile::tempdir().unwrap();
    let temp_root = temp_root.path().canonicalize().unwrap();
    let reports = tempfile::tempdir().unwrap();
    // No other test runs in this binary, so nothing reads the environment
    // concurrently.
    std::env::set_var("TMPDIR", &temp_root);

    let report_paths: Vec<PathBuf> = (0..3)
        .map(|index| reports.path().join(format!("cwd-{index}")))
        .collect();
    for report in &report_paths {
        spawn_detached_command(
            Path::new("/bin/sh"),
            &[
                OsString::from("-c"),
                OsString::from(r#"pwd -P > "$1.tmp" && mv "$1.tmp" "$1""#),
                OsString::from("sh"),
                report.clone().into_os_string(),
            ],
            b"",
        )
        .expect("spawn detached child");
    }
    spawn_detached_command(&temp_root.join("missing-program"), &[], b"")
        .expect_err("a missing program fails");

    let deadline = Instant::now() + Duration::from_secs(10);
    let cwds: Vec<String> = report_paths
        .iter()
        .map(|report| {
            while !report.exists() {
                assert!(Instant::now() < deadline, "detached child never reported");
                std::thread::sleep(Duration::from_millis(10));
            }
            std::fs::read_to_string(report)
                .unwrap()
                .trim_end()
                .to_owned()
        })
        .collect();

    // SAFETY: geteuid has no arguments and cannot fail.
    let expected = temp_root.join(format!("made-detach-{}", unsafe { libc::geteuid() }));
    let entries: Vec<PathBuf> = std::fs::read_dir(&temp_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(
        entries,
        vec![expected.clone()],
        "one runtime directory for every spawn"
    );
    for cwd in &cwds {
        assert_eq!(Path::new(cwd), expected, "every child runs in it");
    }
    let mode = std::fs::symlink_metadata(&expected)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700, "the runtime directory is owner-only");
}
