use made_platform::ssh::{SshCommand, SshDestination, SshOptions};
use std::path::PathBuf;

fn host_private(name: &str) -> PathBuf {
    // Forward slashes stay absolute on Windows and are accepted by argv.
    // A Unix path is not absolute there and must not be passed to argv.
    if cfg!(windows) {
        PathBuf::from(format!("C:/private/{name}"))
    } else {
        PathBuf::from(format!("/private/{name}"))
    }
}

fn options() -> SshOptions {
    SshOptions {
        executable: if cfg!(windows) {
            PathBuf::from("C:/Windows/System32/OpenSSH/ssh.exe")
        } else {
            PathBuf::from("/usr/bin/ssh")
        },
        askpass_executable: host_private("made-askpass"),
        private_directory: host_private("made"),
        known_hosts: host_private("Made data/known_hosts"),
        destination: SshDestination {
            host: "configured-alias".into(),
            user: None,
            port: None,
            identity_file: None,
        },
        command: SshCommand::Companion,
        timeout: None,
    }
}
#[test]
fn retains_alias_without_overriding_jump_or_config_and_disables_forwarding() {
    let args = options().argv().unwrap();
    let args: Vec<_> = args.iter().map(|a| a.to_str().unwrap()).collect();
    for required in [
        "-T",
        "-a",
        "ForwardAgent=no",
        "ClearAllForwardings=yes",
        "PermitLocalCommand=no",
        "StrictHostKeyChecking=ask",
        "ControlPath=none",
        "UpdateHostKeys=no",
    ] {
        assert!(args.contains(&required), "missing {required}");
    }
    assert!(!args.contains(&"-F"));
    assert!(!args.contains(&"-J"));
    assert_eq!(
        &args[args.len() - 3..args.len() - 1],
        &["--", "configured-alias"]
    );
    let known = options().known_hosts;
    let expected = format!(
        "UserKnownHostsFile=\"{}\"",
        known.to_str().expect("known_hosts is valid Unicode")
    );
    assert!(args.contains(&expected.as_str()), "missing {expected}");
}
#[test]
fn rejects_option_and_command_injection_without_shell_interpolation() {
    for host in [
        "-F/tmp/config",
        "host;touch /tmp/pwned",
        "x\ny",
        "user@host",
        "$(id)",
    ] {
        let mut o = options();
        o.destination.host = host.into();
        assert!(o.argv().is_err());
    }
    let mut o = options();
    o.destination.identity_file = Some(PathBuf::from("relative-key"));
    assert!(o.argv().is_err());
    for path in ["/tmp/%h", "/tmp/a\nknown", "/tmp/\"known"] {
        o.destination.identity_file = None;
        o.known_hosts = path.into();
        assert!(o.argv().is_err());
    }
}
#[test]
fn debug_redacts_paths_destination_and_arguments() {
    let options = options();
    let debug = format!("{options:?} {:?}", options.destination);
    assert!(!debug.contains("configured-alias"));
    assert!(!debug.contains("Made data"));
}

#[test]
#[cfg(unix)]
fn fixed_bootstrap_scripts_parse_and_headers_cannot_inject_script_source() {
    use made_platform::ssh::{companion_header, staging_header};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for command in [SshCommand::Probe, SshCommand::Companion, SshCommand::Stage] {
        assert!(
            !command.script().contains("tmux"),
            "fixed script must not name or probe tmux: {command:?}"
        );
        let mut child = Command::new("sh")
            .arg("-n")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(command.script().as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }
    for version in ["..", ".", "v1; touch /tmp/pwned", "v1\nline", "$(id)"] {
        assert!(companion_header(version, "x86_64-unknown-linux-gnu").is_err());
    }
    assert!(staging_header(false, "1.0", "x86_64-unknown-linux-gnu").is_err());
    assert_eq!(
        companion_header("1.0", "aarch64-apple-darwin").unwrap(),
        b"1.0\naarch64-apple-darwin\n"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_fixed_script_checks_hashes_and_only_writes_owned_private_files() {
    use made_platform::ssh::staging_header;
    use std::{
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let bin = home.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let tmux_marker = home.join("tmux-ran");
    let tmux = bin.join("tmux");
    std::fs::write(
        &tmux,
        format!(
            "#!/bin/sh\necho executed > '{}'\nexit 99\n",
            tmux_marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return,
    };
    let hash = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";
    let mut input = staging_header(true, "1.0", target).unwrap();
    for _ in 0..6 {
        input.extend_from_slice(format!("1\n{hash}\nx").as_bytes());
    }
    let mut child = Command::new("/bin/sh")
        .args(["-c", SshCommand::Stage.script()])
        .env_clear()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "fixed staging bootstrap failed");
    let root = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Made")
    } else {
        home.join("data/made")
    };
    let destination = root.join("remote/1.0").join(target);
    assert_eq!(
        std::fs::read(destination.join("made-remote")).unwrap(),
        b"x"
    );
    assert_eq!(
        std::fs::metadata(destination.join("made-remote"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(destination.join("plugins/opencode-made/tui.ts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!root.join("remote/1.0/.stage-lock").exists());
    assert_eq!(result.stdout, b"MADE-STAGED-1\n");
    assert!(!tmux_marker.exists(), "staging executed tmux");
}

// Real aliases/ProxyJump/trust/auth methods are exercised only by the isolated
// SSH harness. These argv unit tests do not claim network/authentication proof.

#[test]
#[cfg(unix)]
fn cancel_kills_the_process_group_not_only_the_direct_child() {
    use made_platform::ssh::SshProcess;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    // Keep the socket path short: macOS rejects unix sockets beyond SUN_LEN.
    let root = temp.path();
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let known_hosts = root.join("known_hosts");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&known_hosts)
        .unwrap();
    let child_pid = temp.path().join("child.pid");
    let fake = temp.path().join("ssh");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_8.4p1\\n' >&2; exit 0; fi\nsleep 30 &\necho $! > '{}'\nwait\n",
            child_pid.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let askpass = temp.path().join("askpass");
    std::fs::write(&askpass, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut options = options();
    options.executable = fake;
    options.askpass_executable = askpass;
    options.private_directory = root.to_path_buf();
    options.known_hosts = known_hosts;
    let mut process = SshProcess::spawn(options).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(&child_pid) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "fake ssh did not record its child"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    process.cancel();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && pid_alive(pid) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !pid_alive(pid),
        "process-group descendant survived cancellation"
    );
}

#[test]
fn ssh_failure_classification_is_sanitized_and_stable() {
    use made_platform::ssh::{
        changed_host_key_answer_refused, classify_ssh_stderr, openssh_version_supported,
        parse_openssh_version, prompt_is_changed_host_key, SshFailureKind,
    };
    let changed = b"@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
password=hunter2 token=sekrit\n\
SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
Are you sure you want to continue connecting (yes/no)?\n";
    let failure = classify_ssh_stderr(changed);
    assert_eq!(failure.kind(), SshFailureKind::ChangedHostKey);
    assert_eq!(failure.kind().code(), "changed_host_key");
    assert!(failure.unknown_host_fingerprint().is_none());
    let debug = format!("{failure:?} {failure}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("sekrit"));
    assert!(!debug.contains("SHA256:"));
    assert!(!debug.contains("IDENTIFICATION"));
    assert!(prompt_is_changed_host_key(
        "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\nAre you sure you want to continue connecting (yes/no)?"
    ));
    assert!(changed_host_key_answer_refused(
        "REMOTE HOST IDENTIFICATION HAS CHANGED",
        b"yes"
    ));
    assert!(changed_host_key_answer_refused(
        "host key for example has changed",
        b"SHA256:abc"
    ));
    assert!(!changed_host_key_answer_refused(
        "The authenticity of host 'example' can't be established.",
        b"yes"
    ));

    let unknown = b"The authenticity of host 'secret.example (10.0.0.1)' can't be established.\n\
ED25519 key fingerprint is SHA256:abcDEF1234567890+/=abcd\n\
password=hunter2\n\
Host key verification failed.\n";
    let failure = classify_ssh_stderr(unknown);
    assert_eq!(failure.kind(), SshFailureKind::UnknownHost);
    assert_eq!(
        failure.unknown_host_fingerprint(),
        Some("SHA256:abcDEF1234567890+/=abcd")
    );
    let debug = format!("{failure:?}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("secret.example"));
    assert!(!debug.contains("SHA256:"));
    assert!(!debug.contains("authenticity"));

    assert_eq!(
        classify_ssh_stderr(b"Permission denied (publickey).").kind(),
        SshFailureKind::AuthFailed
    );
    assert_eq!(
        classify_ssh_stderr(b"Connection timed out").kind(),
        SshFailureKind::Timeout
    );
    assert_eq!(
        classify_ssh_stderr(b"kex_exchange_identification: Connection closed").kind(),
        SshFailureKind::Io
    );
    assert_eq!(parse_openssh_version("OpenSSH_8.3p1"), Some((8, 3)));
    assert!(!openssh_version_supported("OpenSSH_8.3p1"));
    assert!(openssh_version_supported("OpenSSH_8.4p1 Ubuntu-1"));
    assert!(openssh_version_supported(
        "OpenSSH_for_Windows_9.5p1, LibreSSL 3.8.2"
    ));
    assert!(!openssh_version_supported("not ssh"));
}

#[test]
#[cfg(unix)]
fn changed_host_key_exit_is_not_generic_io_and_does_not_keep_stderr() {
    use made_platform::ssh::{SshEvent, SshFailureKind, SshProcess};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let known_hosts = root.join("known_hosts");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&known_hosts)
        .unwrap();
    let fake = root.join("ssh");
    std::fs::write(
        &fake,
        "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_9.6p1\\n' >&2; exit 0; fi\nprintf 'WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\\npassword=hunter2\\n' >&2\nexit 255\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let askpass = root.join("askpass");
    std::fs::write(&askpass, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut options = options();
    options.executable = fake;
    options.askpass_executable = askpass;
    options.private_directory = root.to_path_buf();
    options.known_hosts = known_hosts;
    let mut process = SshProcess::spawn(options).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let failure = loop {
        match process.poll() {
            Some(SshEvent::Failed(failure)) => break failure,
            Some(SshEvent::Exited(_)) => panic!("changed host key was reported as a normal exit"),
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => panic!("changed host key was not classified"),
        }
    };
    assert_eq!(failure.kind(), SshFailureKind::ChangedHostKey);
    let debug = format!("{failure:?}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("IDENTIFICATION"));
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
