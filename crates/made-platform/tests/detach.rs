//! Isolated detach and private-runtime tests. Not a remote SSH proof.
use made_platform::{
    create_private_directory_new, create_private_file_new, ensure_private_directory,
    exec_remote_child, lock_private_file, read_private_file, remote_path_bytes,
    remote_path_from_bytes, remote_program_version_supported, remote_target,
    spawn_detached_command, PtyRequest,
};
use std::path::Path;
use std::time::Duration;

#[test]
fn remote_target_matches_this_build_or_fails_preflight() {
    match remote_target() {
        Ok(target) => {
            let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
                ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
                ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
                ("macos", "x86_64") => "x86_64-apple-darwin",
                ("macos", "aarch64") => "aarch64-apple-darwin",
                _ => panic!("remote_target succeeded on an unsupported host: {target}"),
            };
            assert_eq!(target.as_str(), expected);
        }
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("unsupported") && text.contains("no multiplexer fallback"),
                "{text}"
            );
        }
    }
}

#[test]
fn version_probe_never_executes_tmux_and_bounds_opencode() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("ran");
    let tmux = temp.path().join("tmux");
    std::fs::write(
        &tmux,
        format!("#!/bin/sh\necho executed > '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(!remote_program_version_supported(&tmux, "tmux").unwrap());
    assert!(!marker.exists(), "tmux was executed");
    assert!(remote_program_version_supported(Path::new("opencode"), "opencode").is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let opencode = temp.path().join("opencode");
        std::fs::write(&opencode, "#!/bin/sh\nprintf '2.1.4\\n'\n").unwrap();
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(remote_program_version_supported(&opencode, "opencode").unwrap());
        std::fs::write(&opencode, "#!/bin/sh\nprintf '1.9.0\\n'\n").unwrap();
        assert!(!remote_program_version_supported(&opencode, "opencode").unwrap());
        std::fs::write(&opencode, "#!/bin/sh\nsleep 30\n").unwrap();
        let started = std::time::Instant::now();
        assert!(!remote_program_version_supported(&opencode, "opencode").unwrap());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[test]
#[cfg(unix)]
fn remote_paths_round_trip_posix_bytes_without_host_resolution() {
    let bytes = b"/tmp/spaces \xff/name";
    let path = remote_path_from_bytes(bytes).unwrap();
    assert_eq!(remote_path_bytes(&path).unwrap(), bytes);
    for bad in [
        b"relative".as_slice(),
        b"/../x",
        b"/a/../b",
        b"/a//b",
        b"/bad\0path",
    ] {
        assert!(remote_path_from_bytes(bad).is_err(), "{bad:?}");
    }
}

#[test]
#[cfg(unix)]
fn private_directory_and_file_operations_reject_symlinks_and_loose_modes() {
    use made_platform::PlatformError;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let temp = tempfile::tempdir().unwrap();
    let parent_mode = std::fs::symlink_metadata(temp.path())
        .unwrap()
        .permissions()
        .mode();
    let nested = temp.path().join("app").join("runtime");
    ensure_private_directory(&nested).unwrap();
    assert_eq!(
        std::fs::symlink_metadata(temp.path())
            .unwrap()
            .permissions()
            .mode(),
        parent_mode,
        "ensure must not chmod an existing ancestor"
    );
    assert_eq!(
        std::fs::symlink_metadata(temp.path().join("app"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert!(create_private_directory_new(&nested).is_err());
    let created = nested.join("exclusive");
    create_private_directory_new(&created).unwrap();
    match create_private_directory_new(&created) {
        Err(PlatformError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
        }
        other => panic!("expected already exists, got {other:?}"),
    }

    let file = created.join("note");
    {
        let mut output = create_private_file_new(&file, false).unwrap();
        output.write_all(b"secret").unwrap();
    }
    assert_eq!(read_private_file(&file, 64).unwrap(), b"secret");
    assert_eq!(
        std::fs::symlink_metadata(&file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(create_private_file_new(&file, false).is_err());

    let executable = created.join("helper");
    let _ = create_private_file_new(&executable, true).unwrap();
    assert_eq!(
        std::fs::symlink_metadata(&executable)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let loose = created.join("loose");
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&loose)
        .unwrap();
    assert!(read_private_file(&loose, 16).is_err());

    let link = created.join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(read_private_file(&link, 16).is_err());
    assert!(create_private_file_new(&link, false).is_err());

    std::fs::hard_link(&file, created.join("hard")).unwrap();
    assert!(read_private_file(&file, 16).is_err());

    // A fixed name can collide if two tests share a directory and then wait on
    // the same flock. The counter makes each call a distinct file.
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_LOCK: AtomicU64 = AtomicU64::new(1);
    let lock_path = created.join(format!(
        "stage-{}-{}.lock",
        std::process::id(),
        NEXT_LOCK.fetch_add(1, Ordering::Relaxed)
    ));
    let first = lock_private_file(&lock_path, Duration::from_secs(1)).unwrap();
    assert!(lock_private_file(&lock_path, Duration::ZERO).is_err());
    drop(first);
    let _second = lock_private_file(&lock_path, Duration::ZERO).unwrap();
    assert_eq!(
        std::fs::symlink_metadata(&lock_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
#[cfg(unix)]
fn detach_survives_parent_hangup_without_invoking_tmux() {
    if std::env::var_os("MADE_DETACH_ROLE").as_deref() == Some(std::ffi::OsStr::new("parent")) {
        detach_parent_role();
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("parent-ready");
    let pid_file = temp.path().join("leader.pid");
    let child_file = temp.path().join("child.pid");
    let error_file = temp.path().join("error");
    let tmux_marker = temp.path().join("tmux-ran");
    let cwd_file = temp.path().join("cwd");
    use std::os::unix::process::CommandExt;
    let mut parent = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("detach_survives_parent_hangup_without_invoking_tmux")
        .arg("--test-threads=1")
        .env("MADE_DETACH_ROLE", "parent")
        .env("MADE_DETACH_READY", &ready)
        .env("MADE_DETACH_PID", &pid_file)
        .env("MADE_DETACH_CHILD", &child_file)
        .env("MADE_DETACH_ERROR", &error_file)
        .env("MADE_DETACH_CWD", &cwd_file)
        .env("MADE_TMUX_MARKER", &tmux_marker)
        .env("MADE_DETACH_HOME", temp.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !ready.exists() && std::time::Instant::now() < deadline {
        if let Some(status) = parent.try_wait().unwrap() {
            panic!("detach parent exited before ready: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.exists(), "detach parent did not become ready");
    let leader = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let child = std::fs::read_to_string(&child_file)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let status = std::process::Command::new("/bin/kill")
        .args(["-HUP", &format!("-{}", parent.id())])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "could not hang up the parent process group"
    );
    // Some hosted Linux shells start the test binary with SIGHUP ignored, so
    // the hangup may not reap the parent. Wait a bounded time, then force the
    // parent down; the detached child must still survive.
    let reap_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut reaped = false;
    while std::time::Instant::now() < reap_deadline {
        if parent.try_wait().unwrap().is_some() {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !reaped {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", &format!("-{}", parent.id())])
            .status();
        let _ = parent.wait();
    }
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        pid_alive(leader),
        "detached session died on parent hangup: {}",
        std::fs::read_to_string(&error_file).unwrap_or_default()
    );
    assert!(
        pid_alive(child),
        "detached session child died on parent hangup"
    );
    assert!(!tmux_marker.exists(), "detach invoked tmux");
    let cwd = std::fs::read_to_string(&cwd_file).unwrap();
    assert!(
        cwd.contains("made-detach-"),
        "detached cwd is not the private runtime: {cwd}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::symlink_metadata(cwd.trim())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", &leader.to_string()])
        .status();
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", &child.to_string()])
        .status();
}

#[cfg(unix)]
fn detach_parent_role() {
    use std::os::unix::fs::PermissionsExt;
    let home = std::env::var_os("MADE_DETACH_HOME").unwrap();
    let bin = Path::new(&home).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tmux = bin.join("tmux");
    std::fs::write(
        &tmux,
        "#!/bin/sh\necho executed > \"$MADE_TMUX_MARKER\"\nexit 99\n",
    )
    .unwrap();
    std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    let mut prefixed = bin.into_os_string();
    prefixed.push(":");
    prefixed.push(path);
    path = prefixed;
    std::env::set_var("PATH", &path);
    let script = r#"
set -eu
read -r payload
test "$payload" = made-detach-payload
if [ -r /proc/self/stat ]; then
  sid=$(awk '{print $6}' /proc/self/stat)
elif command -v python3 >/dev/null 2>&1; then
  sid=$(python3 -c 'import os; print(os.getsid(0))')
else
  echo "no session probe" > "$MADE_DETACH_ERROR"
  exit 6
fi
test "$sid" = "$$" || { echo "session $sid pid $$" > "$MADE_DETACH_ERROR"; exit 6; }
if [ -t 1 ] || [ -t 2 ] || [ -p /dev/fd/1 ] || [ -p /dev/fd/2 ]; then
  echo stdio > "$MADE_DETACH_ERROR"
  exit 3
fi
kill -HUP $$ || { echo sighup > "$MADE_DETACH_ERROR"; exit 5; }
pwd > "$MADE_DETACH_CWD"
echo $$ > "$MADE_DETACH_PID"
sleep 60 &
echo $! > "$MADE_DETACH_CHILD"
touch "$MADE_DETACH_READY"
sleep 60
"#;
    spawn_detached_command(
        Path::new("/bin/sh"),
        &[
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from(script),
        ],
        b"made-detach-payload\n",
    )
    .unwrap();
    let ready = std::env::var_os("MADE_DETACH_READY").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !Path::new(&ready).exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !Path::new(&ready).exists() {
        let error = std::env::var_os("MADE_DETACH_ERROR").unwrap();
        panic!(
            "detached command did not become ready: {}",
            std::fs::read_to_string(error).unwrap_or_default()
        );
    }
    loop {
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[test]
#[cfg(unix)]
fn exec_remote_child_replaces_the_process_without_a_shell_string() {
    if std::env::var_os("MADE_EXEC_ROLE").is_some() {
        let marker = std::env::var_os("MADE_EXEC_MARKER").unwrap();
        let program = std::env::var_os("MADE_EXEC_PROGRAM").unwrap();
        let mut request = PtyRequest::new(program, std::env::temp_dir());
        request.args = vec![marker];
        request.env = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let error = exec_remote_child(&request).unwrap_err();
        panic!("exec returned: {error}");
    }
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("ran");
    let program = temp.path().join("program name.sh");
    std::fs::write(&program, "#!/bin/sh\necho ok > \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("exec_remote_child_replaces_the_process_without_a_shell_string")
        .env("MADE_EXEC_ROLE", "1")
        .env("MADE_EXEC_MARKER", &marker)
        .env("MADE_EXEC_PROGRAM", &program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "exec child failed: {status}");
    assert_eq!(std::fs::read(&marker).unwrap(), b"ok\n");
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
#[cfg(not(unix))]
fn windows_detach_preflight_fails_without_a_multiplexer_fallback() {
    let error =
        spawn_detached_command(Path::new("C:\\Windows\\System32\\cmd.exe"), &[], b"").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("unsupported"), "{text}");
    assert!(text.contains("no multiplexer fallback"), "{text}");
    assert!(remote_target().is_err());
}
