//! macOS detached spawn ignores `SIGHUP` process-wide only while
//! `posix_spawn` runs, and restores the caller's disposition afterwards, on
//! success and on failure.
//!
//! Keep this the only test in this binary: it installs a process-wide
//! `SIGHUP` handler and reads the disposition back.
#![cfg(target_os = "macos")]

use std::path::Path;

use made_platform::spawn_detached_command;

extern "C" fn on_hangup(_: libc::c_int) {}

fn sighup_handler() -> libc::sighandler_t {
    // SAFETY: a null new action only reads the current one.
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        assert_eq!(
            libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut current),
            0
        );
        current.sa_sigaction
    }
}

#[test]
fn the_callers_sighup_disposition_is_restored_after_success_and_failure() {
    let handler = on_hangup as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: installs a handler that does nothing.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(
            libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut()),
            0
        );
    }

    spawn_detached_command(Path::new("/usr/bin/true"), &[], b"").expect("spawn /usr/bin/true");
    assert_eq!(sighup_handler(), handler, "restored after a spawn");

    let missing = tempfile::tempdir().unwrap().path().join("missing-program");
    let error = spawn_detached_command(&missing, &[], b"").expect_err("missing program");
    assert!(
        error.to_string().contains("spawn detached command"),
        "{error}"
    );
    assert_eq!(sighup_handler(), handler, "restored after a failed spawn");
}
