//! macOS PTY child spawn keeps `execvp`'s `ENOEXEC` fallback: an executable
//! file without a `#!` line runs as a `/bin/sh` script with its arguments.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use made_platform::{host_platform, Platform, PtyRequest, PtySize};

#[test]
fn executable_without_a_shebang_runs_through_bin_sh() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let script = root.join("no-shebang");
    std::fs::write(
        &script,
        "printf 'ran:%s:%s\\n' \"$1\" \"$2\"\nread _\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let request = PtyRequest {
        program: script.into_os_string(),
        args: vec![OsString::from("first"), OsString::from("second arg")],
        cwd: root,
        env: vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
        env_remove: Vec::new(),
        size: PtySize::default(),
    };
    let session = host_platform()
        .spawn_pty(&request)
        .expect("a script without a shebang spawns like execvp");

    let mut reader = session.take_reader().unwrap();
    let mut writer = session.take_writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || tx.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    while !output.contains("ran:first:second arg") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let chunk = rx
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("script output never arrived: {output:?}"));
        output.push_str(&String::from_utf8_lossy(&chunk));
    }

    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
    let status = loop {
        if let Some(status) = session.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "script did not exit");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((status.code, status.signal), (7, None));
}
