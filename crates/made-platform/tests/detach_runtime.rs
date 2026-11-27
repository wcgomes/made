//! Detached spawns share the one private runtime directory their caller
//! names, instead of creating a new temp directory per spawn that nothing
//! ever removes.
//!
//! Keep this the only test in this binary: it points `TMPDIR` at its own
//! temporary directory for the whole process, to prove nothing goes there.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_platform::spawn_detached_command;

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn detached_spawns_reuse_the_one_private_runtime_directory_they_are_given() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path().canonicalize().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let reports = tempfile::tempdir().unwrap();
    // No other test runs in this binary, so nothing reads the environment
    // concurrently.
    std::env::set_var("TMPDIR", temp.path());
    // Two missing components: both are created owner-only.
    let runtime = base.join("remote").join("detach");

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
            &runtime,
        )
        .expect("spawn detached child");
    }
    spawn_detached_command(&base.join("missing-program"), &[], b"", &runtime)
        .expect_err("a missing program fails");

    let deadline = Instant::now() + Duration::from_secs(10);
    for report in &report_paths {
        while !report.exists() {
            assert!(Instant::now() < deadline, "detached child never reported");
            std::thread::sleep(Duration::from_millis(10));
        }
        let cwd = std::fs::read_to_string(report).unwrap();
        assert_eq!(Path::new(cwd.trim_end()), runtime, "every child runs in it");
    }

    let entries = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    };
    assert_eq!(entries(&base), vec![base.join("remote")]);
    assert_eq!(entries(&base.join("remote")), vec![runtime.clone()]);
    assert!(entries(&runtime).is_empty(), "nothing is written there");
    assert_eq!(mode(&base.join("remote")), 0o700);
    assert_eq!(mode(&runtime), 0o700, "the runtime directory is owner-only");
    assert!(
        entries(temp.path()).is_empty(),
        "nothing is created in TMPDIR"
    );
}
