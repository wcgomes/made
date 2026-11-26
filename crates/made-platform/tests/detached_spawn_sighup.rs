//! macOS detached spawn ignores `SIGHUP` process-wide only while
//! `posix_spawn` runs, and restores the caller's whole action (handler, flags
//! and mask) afterwards, on success and on failure.
//!
//! Keep this the only test in this binary: it installs a process-wide
//! `SIGHUP` handler and reads the disposition back.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use made_platform::spawn_detached_command;

extern "C" fn on_hangup(_: libc::c_int) {}

/// The current `SIGHUP` action as (handler, flags, mask).
fn sighup_action() -> (libc::sighandler_t, libc::c_int, libc::sigset_t) {
    // SAFETY: a null new action only reads the current one.
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        assert_eq!(
            libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut current),
            0
        );
        (current.sa_sigaction, current.sa_flags, current.sa_mask)
    }
}

#[test]
fn the_callers_sighup_action_is_restored_after_success_and_failure() {
    let handler = on_hangup as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: installs a handler that does nothing, with SA_RESTART and a
    // non-empty mask so every field of the restored action is checked.
    let installed = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaddset(&mut action.sa_mask, libc::SIGUSR1);
        libc::sigaddset(&mut action.sa_mask, libc::SIGTERM);
        assert_eq!(
            libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut()),
            0
        );
        sighup_action()
    };
    assert_eq!(installed.0, handler);
    assert_ne!(installed.1 & libc::SA_RESTART, 0);
    assert_ne!(installed.2, 0);

    let temp = tempfile::tempdir().unwrap();
    let report = temp.path().join("cwd");
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
    .expect("spawn /bin/sh");
    assert_eq!(sighup_action(), installed, "restored after a spawn");

    let missing = temp.path().join("missing-program");
    let error = spawn_detached_command(&missing, &[], b"").expect_err("missing program");
    assert!(
        error.to_string().contains("spawn detached command"),
        "{error}"
    );
    assert_eq!(sighup_action(), installed, "restored after a failed spawn");

    // spawn_detached_command leaves the runtime directory of a successful
    // spawn in place; remove the one the first spawn created.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !report.exists() {
        assert!(Instant::now() < deadline, "detached child never reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    let runtime = std::fs::read_to_string(&report).unwrap();
    let runtime = Path::new(runtime.trim_end());
    assert!(runtime
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("made-detach-")));
    std::fs::remove_dir(runtime).unwrap();
}
