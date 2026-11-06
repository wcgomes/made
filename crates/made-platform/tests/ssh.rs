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

/// Write a fixture executable through a child process. A write descriptor
/// held by this multi-threaded test process can be inherited by another test
/// thread's fork, which makes exec of the fixture fail with ETXTBSY.
#[cfg(unix)]
fn write_executable(path: &std::path::Path, content: impl AsRef<[u8]>) {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("cat > \"$1\" && chmod 700 \"$1\"")
        .arg("sh")
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_ref())
        .unwrap();
    assert!(child.wait().unwrap().success());
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
    for command in [
        SshCommand::Probe,
        SshCommand::Companion,
        SshCommand::Stage,
        SshCommand::GrokPluginInstall,
    ] {
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

/// Fake `uname` plus detection targets, so probe assertions stay hermetic even
/// where the host /usr/bin carries python3. The values mirror the running host
/// so fixed scripts' OS dispatch still computes the real layout.
fn probe_fixture_uname(dir: &std::path::Path) -> std::path::PathBuf {
    let (os, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("Darwin", "arm64"),
        ("macos", "x86_64") => ("Darwin", "x86_64"),
        ("linux", "x86_64") => ("Linux", "x86_64"),
        ("linux", "aarch64") => ("Linux", "aarch64"),
        _ => ("FixtureOS", "fixturearch"),
    };
    std::fs::write(
        dir.join("uname"),
        format!(
            "#!/bin/sh\ncase \"$1\" in -s) printf '{os}\\n';; -m) printf '{arch}\\n';; *) exit 1;; esac\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("uname"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
    }
    dir.to_path_buf()
}

#[test]
#[cfg(unix)]
fn probe_reports_grok_with_metadata_only_path_lookup_and_never_looks_up_python3() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let bin = home.join("abs-bin");
    std::fs::create_dir_all(&bin).unwrap();
    // Detection targets that would leave a marker if they were ever executed
    // or version-checked; the probe must only perform metadata lookups.
    let executed = temp.path().join("probe-executed");
    for name in ["grok", "python3"] {
        let path = bin.join(name);
        write_executable(
            &path,
            format!("#!/bin/sh\nprintf x >> '{}'\nexit 4\n", executed.display()),
        );
    }
    // A relative PATH entry must stay invisible to detection.
    let relative = temp.path().join("rel-bin");
    std::fs::create_dir_all(&relative).unwrap();
    let impostor = relative.join("grok");
    write_executable(&impostor, "#!/bin/sh\nexit 0\n");
    let tools = temp.path().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let tools = probe_fixture_uname(&tools);
    let run = |path: String| {
        Command::new("/bin/sh")
            .args(["-c", SshCommand::Probe.script()])
            .current_dir(temp.path())
            .env_clear()
            .env("HOME", &home)
            .env("PATH", path)
            .output()
            .unwrap()
    };
    let output = run(format!(
        "rel-bin::{}:/bin:{}",
        bin.display(),
        tools.display()
    ));
    assert!(output.status.success(), "fixed probe failed");
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert_eq!(lines.first().map(String::as_str), Some("MADE-PROBE-1"));
    assert!(
        !lines[1].is_empty() && !lines[2].is_empty(),
        "probe must report os and arch"
    );
    assert_eq!(lines[3], "", "the fixture has no opencode");
    assert_eq!(lines[4], format!("{}/grok", bin.display()));
    // Claude Code is the last probe line: python3 sits on the same PATH and is
    // never looked up, reported or executed.
    assert_eq!(lines.len(), 7, "{lines:?}");
    assert_eq!(lines[5], "");
    assert_eq!(lines[6], "");
    assert!(
        !SshCommand::Probe.script().contains("python3"),
        "the fixed probe must not mention python3"
    );
    assert!(
        !executed.exists(),
        "detection must never spawn grok or check a version"
    );
    // Only relative and empty PATH entries: nothing is detected on them.
    let output = run(format!("rel-bin::/bin:{}", tools.display()));
    assert!(output.status.success());
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert_eq!(lines[4], "", "relative PATH entries must not be detected");
}

#[test]
#[cfg(unix)]
fn probe_reports_claude_as_one_more_metadata_only_path_line() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let bin = home.join("abs-bin");
    std::fs::create_dir_all(&bin).unwrap();
    // A launched or version-checked claude would leave this marker.
    let executed = temp.path().join("claude-executed");
    for name in ["claude", "grok"] {
        write_executable(
            &bin.join(name),
            format!("#!/bin/sh\nprintf x >> '{}'\nexit 4\n", executed.display()),
        );
    }
    let relative = temp.path().join("rel-bin");
    std::fs::create_dir_all(&relative).unwrap();
    write_executable(&relative.join("claude"), "#!/bin/sh\nexit 0\n");
    let tools = temp.path().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let tools = probe_fixture_uname(&tools);
    let run = |path: String| {
        Command::new("/bin/sh")
            .args(["-c", SshCommand::Probe.script()])
            .current_dir(temp.path())
            .env_clear()
            .env("HOME", &home)
            .env("PATH", path)
            .output()
            .unwrap()
    };
    let lines_of = |output: std::process::Output| -> Vec<String> {
        assert!(output.status.success(), "fixed probe failed");
        String::from_utf8_lossy(&output.stdout)
            .split('\n')
            .map(str::to_owned)
            .collect()
    };
    let lines = lines_of(run(format!(
        "rel-bin::{}:/bin:{}",
        bin.display(),
        tools.display()
    )));
    assert_eq!(lines.len(), 7, "{lines:?}");
    assert_eq!(lines[4], format!("{}/grok", bin.display()));
    assert_eq!(lines[5], format!("{}/claude", bin.display()));
    assert!(
        !executed.exists(),
        "detection must never spawn claude or check a version"
    );
    // Only relative and empty PATH entries: claude is not detected on them.
    let lines = lines_of(run(format!("rel-bin::/bin:{}", tools.display())));
    assert_eq!(lines[5], "", "relative PATH entries must not be detected");
}

#[test]
#[cfg(unix)]
fn grok_plugin_install_command_runs_only_the_staged_companion_in_install_mode() {
    use std::{io::Write, os::unix::fs::PermissionsExt, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return,
    };
    let root = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Made")
    } else {
        home.join("data/made")
    };
    let staged = root.join("remote/1.0").join(target);
    std::fs::create_dir_all(&staged).unwrap();
    let marker = temp.path().join("install-argv");
    let stdin_record = temp.path().join("install-stdin");
    let companion = staged.join("made-remote");
    write_executable(
        &companion,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\ncat > '{}'\n",
            marker.display(),
            stdin_record.display()
        ),
    );
    // The fixed SSH command itself must never invoke grok; the consented
    // companion install mode owns the official installer call.
    let tools = temp.path().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let grok_marker = temp.path().join("grok-executed");
    std::fs::write(
        tools.join("grok"),
        format!(
            "#!/bin/sh\nprintf executed >> '{}'\nexit 7\n",
            grok_marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(tools.join("grok"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let tools = probe_fixture_uname(&tools);
    let run = |stdin: &str| {
        let mut child = Command::new("/bin/sh")
            .args(["-c", SshCommand::GrokPluginInstall.script()])
            .env_clear()
            .env("HOME", &home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", format!("/bin:{}", tools.display()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let output = run(&format!("1.0\n{target}\nconsent-payload"));
    assert!(output.status.success(), "install command failed");
    let argv = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(
        argv, "--install-grok-plugin\n",
        "the fixed command must exec the staged companion in install mode only"
    );
    assert_eq!(
        std::fs::read(&stdin_record).unwrap_or_default(),
        b"consent-payload",
        "stdin after the version/target header must reach the companion"
    );
    assert!(
        !grok_marker.exists(),
        "the fixed command must not run grok itself"
    );
    // Hostile or unsupported stdin is refused without executing anything.
    let records = std::fs::read_to_string(&marker).unwrap();
    for hostile in [
        format!("bad;rm\n{target}\n"),
        format!("\n{target}\n"),
        "1.0\nx86_64-w64-mingw32\n".to_string(),
    ] {
        let output = run(&hostile);
        assert!(
            !output.status.success(),
            "hostile install stdin must fail: {hostile:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        records,
        "nothing may be executed for refused install input"
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
    write_executable(
        &tmux,
        format!(
            "#!/bin/sh\necho executed > '{}'\nexit 99\n",
            tmux_marker.display()
        ),
    );
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return,
    };
    // An existing personal Grok home that staging must leave byte-identical:
    // the plugin install is a separate consent, not part of staging.
    let grok_home = home.join(".grok");
    std::fs::create_dir_all(&grok_home).unwrap();
    let grok_config = grok_home.join("config.toml");
    std::fs::write(&grok_config, b"existing = true\n").unwrap();
    let hash = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";
    let mut input = staging_header(true, "1.0", target).unwrap();
    for _ in 0..10 {
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
    // The complete shipped grok-made package is staged beside opencode-made
    // with private permissions and the executable hook bit.
    let grok_plugin = destination.join("plugins/grok-made");
    for name in ["plugin.json", "README.md", "hooks/hooks.json"] {
        assert_eq!(std::fs::read(grok_plugin.join(name)).unwrap(), b"x");
        assert_eq!(
            std::fs::metadata(grok_plugin.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(
        std::fs::read(grok_plugin.join("hooks/bin/made-grok-status")).unwrap(),
        b"x"
    );
    assert_eq!(
        std::fs::metadata(grok_plugin.join("hooks/bin/made-grok-status"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "the shipped hook must keep an executable bit"
    );
    // Staging never writes the remote user's Grok home; that is the separate
    // consented install's only permitted mutation.
    assert_eq!(std::fs::read(&grok_config).unwrap(), b"existing = true\n");
    assert_eq!(
        std::fs::read_dir(&grok_home).unwrap().count(),
        1,
        "staging must not add files to the Grok home"
    );
    assert!(!root.join("remote/1.0/.stage-lock").exists());
    assert_eq!(result.stdout, b"MADE-STAGED-1\n");
    assert!(!tmux_marker.exists(), "staging executed tmux");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod fixed_staging {
    use made_platform::ssh::{staging_header, staging_update_header, SshCommand};
    use std::{
        collections::BTreeMap,
        fs,
        io::Write,
        os::unix::{
            fs::{MetadataExt, PermissionsExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::{Child, Command, Output, Stdio},
    };

    pub const ARTIFACTS: [&str; 10] = [
        "made-remote",
        "plugins/opencode-made/package.json",
        "plugins/opencode-made/tui.ts",
        "plugins/opencode-made/index.ts",
        "plugins/opencode-made/src/protocol.ts",
        "plugins/opencode-made/src/integration.ts",
        "plugins/grok-made/plugin.json",
        "plugins/grok-made/README.md",
        "plugins/grok-made/hooks/hooks.json",
        "plugins/grok-made/hooks/bin/made-grok-status",
    ];
    pub const UPDATE_REQUIRED: &str = "MADE-STAGE-UPDATE-REQUIRED";
    pub const RECOVERY_REQUIRED: &str = "MADE-STAGE-RECOVERY-REQUIRED";

    pub fn target() -> Option<&'static str> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
            ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
            ("macos", "x86_64") => Some("x86_64-apple-darwin"),
            ("macos", "aarch64") => Some("aarch64-apple-darwin"),
            _ => None,
        }
    }

    pub fn sha256_hex(bytes: &[u8]) -> String {
        for name in ["sha256sum", "shasum"] {
            let mut command = Command::new(name);
            if name == "shasum" {
                command.args(["-a", "256"]);
            }
            let Ok(mut child) = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            child.stdin.take().unwrap().write_all(bytes).unwrap();
            let output = child.wait_with_output().unwrap();
            if output.status.success() {
                return String::from_utf8(output.stdout)
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_owned();
            }
        }
        panic!("no sha256 tool on PATH");
    }

    pub fn home() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        fs::create_dir(home.join("bin")).unwrap();
        (temp, home)
    }

    pub fn data_root(home: &Path) -> PathBuf {
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Made")
        } else {
            home.join("data/made")
        }
    }

    pub fn version_root(home: &Path, version: &str) -> PathBuf {
        data_root(home).join("remote").join(version)
    }

    pub fn destination(home: &Path, version: &str) -> PathBuf {
        version_root(home, version).join(target().unwrap())
    }

    pub fn body(update: bool, version: &str, content: &[u8]) -> Vec<u8> {
        body_with_hash(update, version, content, &sha256_hex(content))
    }

    pub fn body_with_hash(update: bool, version: &str, content: &[u8], hash: &str) -> Vec<u8> {
        let target = target().unwrap();
        let mut bytes = if update {
            staging_update_header(true, version, target).unwrap()
        } else {
            staging_header(true, version, target).unwrap()
        };
        for _ in ARTIFACTS {
            bytes.extend_from_slice(format!("{}\n{hash}\n", content.len()).as_bytes());
            bytes.extend_from_slice(content);
        }
        bytes
    }

    pub fn run(home: &Path, payload: &[u8]) -> Output {
        run_with_path(home, payload, Path::new("/usr/bin:/bin:/usr/sbin:/sbin"))
    }

    pub fn run_with_path(home: &Path, payload: &[u8], path: &Path) -> Output {
        let mut child = Command::new("/bin/sh")
            .args(["-c", SshCommand::Stage.script()])
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        child.wait_with_output().unwrap()
    }

    /// Spawn one fixed-stage attempt in its own process group so the test can
    /// simulate an uncatchable SIGKILL of the whole attempt (EXIT traps do not
    /// run) without leaving fixture children behind.
    pub fn spawn_with_path(home: &Path, payload: &[u8], path: &Path) -> Child {
        // The fixed script is itself `sh -c '...'`. bash execs a last command
        // in place but dash forks it, which would make the killed pid differ
        // from the lock owner's `$$`; `exec` pins the same pid on every host.
        let mut child = Command::new("/bin/sh")
            .args(["-c", &format!("exec {}", SshCommand::Stage.script())])
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        child
    }

    pub fn kill_group(child: &mut Child) {
        // The child was started in a fresh process group whose id is its PID,
        // so the negative target names only this fixture's group.
        let _ = Command::new("/bin/kill")
            // `--` keeps procps kill from reading the negative group id as a
            // signal name; macOS kill accepts it too.
            .args(["-9", "--", &format!("-{}", child.id())])
            .status();
        let _ = child.wait();
    }

    pub fn wait_for(path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture did not reach {}",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn lock_owner(home: &Path, version: &str) -> u32 {
        let lock = version_root(home, version).join(".stage-lock");
        let text = fs::read_to_string(&lock).unwrap();
        text.trim()
            .parse()
            .unwrap_or_else(|error| panic!("lock owner {text:?}: {error}"))
    }

    pub fn child_pids(pid: u32) -> Vec<u32> {
        let Ok(output) = Command::new("/bin/ps")
            .args(["-o", "pid=", "-P", &pid.to_string()])
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect()
    }

    pub fn pid_alive(pid: u32) -> bool {
        Command::new("/bin/ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Block promotion after the original has been renamed to `.replace`, but
    /// before the staged tree is promoted, so a SIGKILL exercises the exact
    /// window the recovery path exists for.
    pub fn slow_move_after_replace(home: &Path, marker: &Path) -> PathBuf {
        let shim = home.join("bin");
        let replace = version_root(home, "1.0").join(".replace");
        let script = format!(
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=$arg; done\n/bin/mv \"$@\" || exit $?\nif test \"$last\" = \"{}\"; then : > \"{}\"; sleep 30; fi\nexit 0\n",
            replace.display(),
            marker.display()
        );
        let path = shim.join("mv");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        shim
    }

    /// Block the first checksum call while the private staging directory and
    /// owner lock already exist.
    pub fn slow_checksum(home: &Path, marker: &Path) -> PathBuf {
        let shim = home.join("bin");
        let script = format!("#!/bin/sh\n: > \"{}\"\nsleep 30\n", marker.display());
        let path = shim.join("sha256sum");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        shim
    }

    /// Block removal of the exact dead staging directory. This lands a SIGKILL
    /// after the stale lock was moved to `.stage-reap.<pid>` but before the
    /// dead owner's private tree was reaped.
    pub fn slow_remove(home: &Path, target: &Path, marker: &Path) -> PathBuf {
        let shim = home.join("bin");
        let script = format!(
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=$arg; done\nif test \"$last\" = \"{}\"; then : > \"{}\"; sleep 30; fi\nexec /bin/rm \"$@\"\n",
            target.display(),
            marker.display()
        );
        let path = shim.join("rm");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        shim
    }

    /// Block the hard-link publication of the prewritten owner token, so a
    /// SIGKILL leaves only the private acquisition temp and never a fixed
    /// `.stage-lock` without its token.
    pub fn slow_link_before_lock(home: &Path, marker: &Path) -> PathBuf {
        let shim = home.join("bin");
        let lock = version_root(home, "1.0").join(".stage-lock");
        let script = format!(
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=$arg; done\nif test \"$last\" = \"{}\"; then : > \"{}\"; sleep 30; fi\nif test -x /usr/bin/link; then exec /usr/bin/link \"$@\"; fi\nexec /bin/link \"$@\"\n",
            lock.display(),
            marker.display()
        );
        let path = shim.join("link");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        shim
    }

    /// A PATH shim that fails only renames to the current destination. The
    /// caller can therefore fail promotion and the restoration attempt while
    /// leaving the original -> `.replace` rename intact.
    pub fn failing_move_to(home: &Path, destination: &Path) -> PathBuf {
        let shim = home.join("bin");
        let script = format!(
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=$arg; done\nif test \"$last\" = \"{}\"; then printf 'injected move failure\\n' >&2; exit 1; fi\nexec /bin/mv \"$@\"\n",
            destination.display()
        );
        let path = shim.join("mv");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        shim
    }

    pub fn run_script(home: &Path, command: SshCommand, stdin: &[u8]) -> Output {
        let mut child = Command::new("/bin/sh")
            .args(["-c", command.script()])
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    }

    pub type Fingerprint = BTreeMap<PathBuf, (u64, i64, i64, u32, u64)>;

    pub fn fingerprint(root: &Path) -> Fingerprint {
        let mut map = Fingerprint::new();
        collect(root, root, &mut map);
        map
    }

    fn collect(root: &Path, path: &Path, map: &mut Fingerprint) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            map.insert(
                relative,
                (
                    meta.len(),
                    meta.mtime(),
                    meta.mtime_nsec(),
                    meta.permissions().mode() & 0o777,
                    meta.ino(),
                ),
            );
            if meta.is_dir() {
                collect(root, &path, map);
            }
        }
    }

    pub fn entries(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    pub fn assert_no_temporaries(root: &Path) {
        for entry in fs::read_dir(root).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.starts_with(".stage-") && !name.starts_with(".replace-"),
                "temporary directory accumulated: {name}"
            );
        }
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_update_header_is_consent_gated_and_keeps_initial_staging_unchanged() {
    use made_platform::ssh::{staging_header, staging_update_header};
    let Some(target) = fixed_staging::target() else {
        return;
    };
    assert!(staging_update_header(false, "1.0", target).is_err());
    let update = staging_update_header(true, "1.0", target).unwrap();
    assert!(update.starts_with(b"consent-v2-update\n1.0\n"));
    let initial = staging_header(true, "1.0", target).unwrap();
    assert!(initial.starts_with(b"consent-v1\n1.0\n"));
    assert!(staging_update_header(true, "..", target).is_err());
    assert!(staging_update_header(true, "1.0", "sparc-unknown-linux-gnu").is_err());
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_exact_reuse_after_initial_install_writes_nothing() {
    let Some(target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let first = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert!(first.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);
    let second = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert!(second.status.success(), "exact reuse failed");
    assert_eq!(second.stdout, b"MADE-STAGED-1\n");
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "exact reuse must not rewrite any artifact"
    );
    assert_eq!(
        fixed_staging::entries(&fixed_staging::version_root(&home, "1.0")),
        vec![target.to_owned()]
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_same_version_mismatch_missing_or_corruption_reports_update_required_without_changes() {
    let Some(target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");

    let before = fixed_staging::fingerprint(&destination);
    let mismatch = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"y"));
    assert_eq!(mismatch.status.code(), Some(74));
    assert!(!String::from_utf8_lossy(&mismatch.stdout).contains("MADE-STAGED-1"));
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains(fixed_staging::UPDATE_REQUIRED));
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "a mismatch must not write or delete anything"
    );

    let corrupt = destination.join("plugins/opencode-made/tui.ts");
    let mut bytes = std::fs::read(&corrupt).unwrap();
    bytes.push(b'!');
    std::fs::write(&corrupt, &bytes).unwrap();
    let corrupt_before = fixed_staging::fingerprint(&destination);
    let corrupt_result = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert_ne!(corrupt_result.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&corrupt_result.stderr).contains(fixed_staging::UPDATE_REQUIRED)
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        corrupt_before,
        "corruption detection must not repair or delete anything"
    );

    let missing = destination.join("plugins/grok-made/README.md");
    std::fs::remove_file(&missing).unwrap();
    let missing_before = fixed_staging::fingerprint(&destination);
    let missing_result = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert_ne!(missing_result.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&missing_result.stderr).contains(fixed_staging::UPDATE_REQUIRED)
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        missing_before,
        "missing artifact detection must not recreate or delete anything"
    );
    assert!(!missing.exists());
    assert_eq!(
        fixed_staging::entries(&version_root),
        vec![target.to_owned()],
        "no staging/replacement directory may accumulate"
    );
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_approved_update_swaps_cleans_keeps_the_old_process_and_reuses_exactly() {
    use std::{os::unix::fs::PermissionsExt, process::Stdio};
    let Some(target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        initial.status.success(),
        "initial staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let mut old_process = std::process::Command::new(destination.join("made-remote"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let old_pid = old_process.id();
    assert!(
        fixed_staging::child_pids(old_pid).is_empty(),
        "the old-process fixture must exec its sleeper so no orphan remains"
    );

    let before_approval = fixed_staging::fingerprint(&destination);
    let refused = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", new));
    assert_eq!(refused.status.code(), Some(74));
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before_approval,
        "no write may happen before the separate update approval"
    );
    assert!(
        old_process.try_wait().unwrap().is_none(),
        "an unapproved attempt must not signal a running companion"
    );

    let update = fixed_staging::run(&home, &fixed_staging::body(true, "1.0", new));
    assert!(
        update.status.success(),
        "approved update failed: {:?}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert_eq!(update.stdout, b"MADE-STAGED-1\n");
    assert_eq!(std::fs::read(destination.join("made-remote")).unwrap(), new);
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
    assert!(
        old_process.try_wait().unwrap().is_none(),
        "the update must not signal a running companion"
    );
    assert_eq!(
        fixed_staging::entries(&version_root),
        vec![target.to_owned()],
        "temporary and replaced directories must be removed after success"
    );
    fixed_staging::assert_no_temporaries(&version_root);

    // Companion and Grok install keep resolving the same version/target path.
    for command in [SshCommand::Companion, SshCommand::GrokPluginInstall] {
        let resolved =
            fixed_staging::run_script(&home, command, format!("1.0\n{target}\n").as_bytes());
        assert_eq!(
            resolved.stdout, b"updated\n",
            "{command:?} must run the updated companion at the current path"
        );
    }

    // A fourth attempt over the updated bytes is exact reuse, not a write.
    let updated = fixed_staging::fingerprint(&destination);
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", new));
    assert!(reuse.status.success(), "post-update exact reuse failed");
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        updated,
        "post-update exact reuse must not rewrite artifacts"
    );

    let _ = old_process.kill();
    let _ = old_process.wait();
    assert!(
        !fixed_staging::pid_alive(old_pid),
        "the recorded old-process PID must be the sleeper itself"
    );
    assert!(
        fixed_staging::child_pids(old_pid).is_empty(),
        "killing the recorded PID must not leave an orphan sleep child"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_approved_update_failure_before_swap_leaves_the_original_byte_identical() {
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        initial.status.success(),
        "initial staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);

    let bad_hash = fixed_staging::body_with_hash(true, "1.0", new, &"0".repeat(64));
    let corrupt = fixed_staging::run(&home, &bad_hash);
    assert!(!corrupt.status.success(), "a corrupt upload must fail");
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "a pre-swap hash failure must leave the original untouched"
    );
    fixed_staging::assert_no_temporaries(&version_root);

    let mut truncated = fixed_staging::body(true, "1.0", new);
    truncated.truncate(truncated.len() - 1);
    let short = fixed_staging::run(&home, &truncated);
    assert!(!short.status.success(), "a truncated upload must fail");
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "a pre-swap size failure must leave the original untouched"
    );
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_planted_symlinks_keep_leaf_and_ancestor_classification_distinct() {
    use std::os::unix::fs::symlink;
    let Some(target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");

    // A symlinked target directory is a security failure, distinct from the
    // explicit update-required classification for an artifact leaf.
    let target_backup = home.join("target-backup");
    std::fs::rename(&destination, &target_backup).unwrap();
    symlink(&target_backup, &destination).unwrap();
    let target_symlink = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert_eq!(target_symlink.status.code(), Some(73));
    assert!(
        !String::from_utf8_lossy(&target_symlink.stderr).contains(fixed_staging::UPDATE_REQUIRED)
    );
    std::fs::remove_file(&destination).unwrap();
    std::fs::rename(&target_backup, &destination).unwrap();

    let leaf = destination.join("plugins/opencode-made/tui.ts");
    let sibling = destination.join("plugins/opencode-made/index.ts");
    std::fs::remove_file(&leaf).unwrap();
    std::os::unix::fs::symlink(&sibling, &leaf).unwrap();
    let artifact_attempt = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert_eq!(
        artifact_attempt.status.code(),
        Some(74),
        "an artifact symlink must reach the explicit update-required classification"
    );
    let stderr = String::from_utf8_lossy(&artifact_attempt.stderr);
    assert!(stderr.contains(fixed_staging::UPDATE_REQUIRED));
    assert!(!stderr.contains("operation failed"));
    assert!(
        std::fs::symlink_metadata(&leaf)
            .unwrap()
            .file_type()
            .is_symlink(),
        "classification must not remove or rewrite the planted artifact symlink"
    );
    assert!(
        !fixed_staging::fingerprint(&version_root).is_empty(),
        "the failed classification must leave the tree in place"
    );

    // An ancestor symlink remains a security failure, not update-required.
    std::fs::remove_file(&leaf).unwrap();
    std::fs::copy(&sibling, &leaf).unwrap();
    let plugins = destination.join("plugins/opencode-made");
    let moved = home.join("moved-opencode-made");
    std::fs::rename(&plugins, &moved).unwrap();
    symlink(&moved, &plugins).unwrap();
    let ancestor_attempt = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", b"x"));
    assert_eq!(ancestor_attempt.status.code(), Some(73));
    assert!(
        !String::from_utf8_lossy(&ancestor_attempt.stderr).contains(fixed_staging::UPDATE_REQUIRED)
    );
    assert!(std::fs::symlink_metadata(&plugins)
        .unwrap()
        .file_type()
        .is_symlink());
    fixed_staging::assert_no_temporaries(&version_root);
    assert_eq!(
        fixed_staging::entries(&version_root),
        vec![target.to_owned()],
        "classification attempts must not leave staging directories"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_failed_promotion_and_rollback_preserves_then_recovers_the_original() {
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        initial.status.success(),
        "initial staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let replaced = version_root.join(".replace");
    let before = fixed_staging::fingerprint(&destination);

    let shim = fixed_staging::failing_move_to(&home, &destination);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let failed = fixed_staging::run_with_path(
        &home,
        &fixed_staging::body(true, "1.0", new),
        std::path::Path::new(&path),
    );
    assert_eq!(
        failed.status.code(),
        Some(75),
        "a double substitution failure must be distinct from generic I/O: {:?}",
        String::from_utf8_lossy(&failed.stderr)
    );
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains(fixed_staging::RECOVERY_REQUIRED));
    assert!(!stderr.contains(fixed_staging::UPDATE_REQUIRED));
    assert!(
        !destination.exists(),
        "the failed promotion must not claim the current path"
    );
    assert_eq!(
        fixed_staging::fingerprint(&replaced),
        before,
        "the only original must survive byte-identically at the fixed recovery path"
    );
    assert_eq!(
        std::fs::read(replaced.join("made-remote")).unwrap(),
        old,
        "the original must never be deleted"
    );
    fixed_staging::assert_no_temporaries(&version_root);

    // The next fixed-stage attempt restores the preserved original before it
    // compares artifacts, so an unapproved attempt reports update-required
    // instead of installing new bytes over a missing current path.
    let restored = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", new));
    assert_eq!(restored.status.code(), Some(74));
    assert!(String::from_utf8_lossy(&restored.stderr).contains(fixed_staging::UPDATE_REQUIRED));
    assert_eq!(
        std::fs::read(destination.join("made-remote")).unwrap(),
        old,
        "the next attempt must restore the original"
    );
    assert!(
        !replaced.exists(),
        "restoration must consume the recovery path"
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "restoration must be byte-identical"
    );
    fixed_staging::assert_no_temporaries(&version_root);

    // An approved retry now completes normally and leaves no temporaries.
    let update = fixed_staging::run(&home, &fixed_staging::body(true, "1.0", new));
    assert!(
        update.status.success(),
        "approved retry failed: {:?}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert_eq!(update.stdout, b"MADE-STAGED-1\n");
    assert_eq!(std::fs::read(destination.join("made-remote")).unwrap(), new);
    assert!(!replaced.exists());
    fixed_staging::assert_no_temporaries(&version_root);
}

// Real aliases/ProxyJump/trust/auth methods are exercised only by the isolated
// SSH harness. These argv unit tests do not claim network/authentication proof.

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_sigkill_between_renames_recovers_replace_and_the_next_attempt_exactly_reuses() {
    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        initial.status.success(),
        "initial staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let replaced = version_root.join(".replace");
    let before = fixed_staging::fingerprint(&destination);

    // SIGKILL the attempt after the original was moved to .replace but before
    // the staged tree was promoted. The EXIT trap never runs.
    let marker = home.join("between-renames");
    let shim = fixed_staging::slow_move_after_replace(&home, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut killed = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(true, "1.0", new),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);
    let owner = fixed_staging::lock_owner(&home, "1.0");
    let dead_stage = version_root.join(format!(".stage-{owner}"));
    assert!(
        replaced.exists(),
        "the original must be preserved at .replace"
    );
    assert!(!destination.exists(), "promotion had not completed");
    assert!(version_root.join(".stage-lock").exists());
    assert!(
        dead_stage.exists(),
        "the killed attempt left its private temp"
    );
    fixed_staging::kill_group(&mut killed);
    assert!(replaced.exists());
    assert!(version_root.join(".stage-lock").exists());
    assert!(dead_stage.exists());

    // The next attempt reclaims only the dead owner's lock, restores the
    // preserved original, removes the dead temp, and completes exact reuse.
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        reuse.status.success(),
        "recovery exact reuse failed: {:?}",
        String::from_utf8_lossy(&reuse.stderr)
    );
    assert_eq!(reuse.stdout, b"MADE-STAGED-1\n");
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "recovery must restore the original byte-identically"
    );
    assert!(!replaced.exists(), "restoration must consume .replace");
    assert!(
        !version_root.join(".stage-lock").exists(),
        "the reclaimed lock must be released"
    );
    assert!(
        !dead_stage.exists(),
        "the dead attempt's temp must be removed"
    );
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_sigkill_during_reap_cleans_the_artifact_and_dead_owner_stage() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        initial.status.success(),
        "initial staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);

    // The first owner dies while writing, leaving its published lock and exact
    // private staging directory behind.
    let marker = home.join("writing");
    let shim = fixed_staging::slow_checksum(&home, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut owner = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(true, "1.0", old),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);
    let dead_owner = fixed_staging::lock_owner(&home, "1.0");
    let dead_stage = version_root.join(format!(".stage-{dead_owner}"));
    fixed_staging::kill_group(&mut owner);
    assert!(version_root.join(".stage-lock").exists());
    assert!(dead_stage.exists());

    // The second owner reclaims that lock, moves it aside to
    // `.stage-reap.<pid>`, and dies before the dead staging tree is removed.
    let marker = home.join("reaping");
    let shim = fixed_staging::slow_remove(&home, &dead_stage, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut reaper = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(false, "1.0", old),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);
    let reaper_id = reaper.id();
    let reap = version_root.join(format!(".stage-reap.{reaper_id}"));
    assert!(reap.exists(), "the reclaimed lock must be moved aside");
    assert!(dead_stage.exists(), "the dead tree was not reaped yet");
    assert!(
        !version_root.join(".stage-lock").exists(),
        "the dead lock was moved aside"
    );
    fixed_staging::kill_group(&mut reaper);
    assert!(reap.exists());
    assert!(dead_stage.exists());
    assert_eq!(fixed_staging::fingerprint(&destination), before);

    // The next attempt validates both dead PIDs and tokens, removes the exact
    // dead-owned artifacts, and completes exact reuse without rewriting.
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        reuse.status.success(),
        "recovery after reap crash failed: {:?}",
        String::from_utf8_lossy(&reuse.stderr)
    );
    assert_eq!(reuse.stdout, b"MADE-STAGED-1\n");
    assert!(!reap.exists(), "the dead reap artifact must be cleaned");
    assert!(
        !dead_stage.exists(),
        "the associated dead staging directory must be cleaned"
    );
    assert!(!version_root.join(".stage-lock").exists());
    assert_eq!(fixed_staging::fingerprint(&destination), before);
    fixed_staging::assert_no_temporaries(&version_root);

    // A live reaper's artifact and its referenced stage stay untouched, even
    // though the token names a dead PID.
    let live_reap = version_root.join(format!(".stage-reap.{}", std::process::id()));
    std::fs::write(&live_reap, format!("{reaper_id}\n")).unwrap();
    let live_orphan = version_root.join(format!(".stage-{reaper_id}"));
    std::fs::create_dir(&live_orphan).unwrap();
    std::fs::set_permissions(&live_orphan, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Unknown tokens and symlinked artifacts are never interpreted or followed.
    let unknown_reap = version_root.join(format!(".stage-reap.{dead_owner}"));
    std::fs::write(&unknown_reap, b"unknown\n").unwrap();
    let link_target = home.join("reap-link-target");
    std::fs::create_dir(&link_target).unwrap();
    let symlink_reap = version_root.join(format!(".stage-reap.{reaper_id}"));
    symlink(&link_target, &symlink_reap).unwrap();

    let guarded = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        guarded.status.success(),
        "guarded exact reuse failed: {:?}",
        String::from_utf8_lossy(&guarded.stderr)
    );
    assert_eq!(guarded.stdout, b"MADE-STAGED-1\n");
    assert_eq!(fixed_staging::fingerprint(&destination), before);
    assert!(live_reap.exists(), "a live reaper artifact must survive");
    assert!(live_orphan.exists(), "a live owner's stage must survive");
    assert!(unknown_reap.exists(), "an unknown token must survive");
    assert!(
        std::fs::symlink_metadata(&symlink_reap)
            .unwrap()
            .file_type()
            .is_symlink(),
        "a symlinked reap artifact must survive"
    );
    assert!(
        link_target.is_dir(),
        "a symlink target must not be traversed"
    );

    std::fs::remove_file(&live_reap).unwrap();
    std::fs::remove_dir(&live_orphan).unwrap();
    std::fs::remove_file(&unknown_reap).unwrap();
    std::fs::remove_file(&symlink_reap).unwrap();
    std::fs::remove_dir(&link_target).unwrap();
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_incomplete_owner_token_is_never_stolen() {
    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nprintf 'old\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);
    // A directory created without a completed owner token is a creation race,
    // not proof of a dead owner. It must fail closed and remain untouched.
    std::fs::create_dir(version_root.join(".stage-lock")).unwrap();

    let attempt = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert_eq!(
        attempt.status.code(),
        Some(73),
        "an incomplete owner token must not be reclaimed: {:?}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "the blocked attempt must not write or delete anything"
    );
    assert!(version_root.join(".stage-lock").is_dir());
    assert!(!version_root.join(".stage-lock/owner").exists());
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_sigkill_before_owner_publication_leaves_no_fixed_lock_and_cleans_the_temp() {
    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nprintf 'old\\n'\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let lock = version_root.join(".stage-lock");
    let before = fixed_staging::fingerprint(&destination);

    let marker = home.join("before-lock-publication");
    let shim = fixed_staging::slow_link_before_lock(&home, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut killed = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(false, "1.0", old),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);

    // The owner token is fully written but the fixed lock path has not been
    // published: a SIGKILL here cannot leave a tokenless `.stage-lock`.
    assert!(
        !lock.exists(),
        "the fixed lock must not exist before its owner token is published"
    );
    let temps: Vec<_> = std::fs::read_dir(&version_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".stage-lock."))
        })
        .collect();
    assert_eq!(temps.len(), 1, "one private acquisition temp: {temps:?}");
    let temp = temps[0].clone();
    assert_eq!(
        std::fs::read_to_string(&temp).unwrap().trim(),
        killed.id().to_string()
    );

    fixed_staging::kill_group(&mut killed);
    assert!(!lock.exists());
    assert!(temp.exists(), "the killed attempt leaves only its temp");
    assert_eq!(fixed_staging::fingerprint(&destination), before);

    // The next attempt removes the exact dead-owned temp, never creates a
    // tokenless fixed lock, and completes exact reuse.
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        reuse.status.success(),
        "recovery after pre-publication kill failed: {:?}",
        String::from_utf8_lossy(&reuse.stderr)
    );
    assert_eq!(reuse.stdout, b"MADE-STAGED-1\n");
    assert!(!temp.exists(), "dead acquisition temp must be cleaned");
    assert!(!lock.exists());
    assert_eq!(
        std::fs::read(destination.join("made-remote")).unwrap(),
        old,
        "exact reuse must not rewrite artifacts"
    );
    assert_eq!(fixed_staging::fingerprint(&destination), before);
    fixed_staging::assert_no_temporaries(&version_root);

    // A kill one step earlier leaves the same exact temp without owner bytes.
    // It must not block, and dead ownership must still clean it.
    let empty = version_root.join(format!(".stage-lock.{}", killed.id()));
    std::fs::write(&empty, b"").unwrap();
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        reuse.status.success(),
        "a tokenless acquisition temp must not block: {:?}",
        String::from_utf8_lossy(&reuse.stderr)
    );
    assert!(
        !empty.exists(),
        "dead tokenless acquisition temp must be cleaned"
    );
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_live_owner_lock_is_never_stolen_and_serialization_is_preserved() {
    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);

    let marker = home.join("holding");
    let shim = fixed_staging::slow_checksum(&home, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut holding = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(true, "1.0", b"different"),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);
    let owner = fixed_staging::lock_owner(&home, "1.0");
    let dead_stage = version_root.join(format!(".stage-{owner}"));

    // A concurrent attempt must fail closed: no mutation, no stolen lock, no
    // removed temp while the owner is demonstrably alive.
    let concurrent = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert_eq!(
        concurrent.status.code(),
        Some(73),
        "a live owner lock must not be reclaimed: {:?}",
        String::from_utf8_lossy(&concurrent.stderr)
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "the blocked attempt must not write or delete anything"
    );
    assert_eq!(fixed_staging::lock_owner(&home, "1.0"), owner);
    assert!(dead_stage.exists());

    fixed_staging::kill_group(&mut holding);
    // Once the owner is demonstrably dead, the lock is reclaimable and exact
    // reuse completes without the dead private temp.
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(
        reuse.status.success(),
        "post-crash exact reuse failed: {:?}",
        String::from_utf8_lossy(&reuse.stderr)
    );
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "exact reuse must not rewrite artifacts"
    );
    assert!(!dead_stage.exists());
    assert!(!version_root.join(".stage-lock").exists());
    fixed_staging::assert_no_temporaries(&version_root);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn staging_sigkill_while_writing_recovers_and_allows_the_approved_update() {
    let Some(_target) = fixed_staging::target() else {
        return;
    };
    let (_temp, home) = fixed_staging::home();
    let old = b"#!/bin/sh\necho old\n";
    let new = b"#!/bin/sh\necho new\n";
    let initial = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", old));
    assert!(initial.status.success(), "initial staging failed");
    let destination = fixed_staging::destination(&home, "1.0");
    let version_root = fixed_staging::version_root(&home, "1.0");
    let before = fixed_staging::fingerprint(&destination);

    let marker = home.join("writing");
    let shim = fixed_staging::slow_checksum(&home, &marker);
    let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", shim.display());
    let mut killed = fixed_staging::spawn_with_path(
        &home,
        &fixed_staging::body(true, "1.0", new),
        std::path::Path::new(&path),
    );
    fixed_staging::wait_for(&marker);
    let owner = fixed_staging::lock_owner(&home, "1.0");
    let dead_stage = version_root.join(format!(".stage-{owner}"));
    assert!(dead_stage.exists());
    fixed_staging::kill_group(&mut killed);
    assert_eq!(
        fixed_staging::fingerprint(&destination),
        before,
        "the pre-swap kill must leave the original untouched"
    );

    let update = fixed_staging::run(&home, &fixed_staging::body(true, "1.0", new));
    assert!(
        update.status.success(),
        "recovery update failed: {:?}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert_eq!(update.stdout, b"MADE-STAGED-1\n");
    assert_eq!(std::fs::read(destination.join("made-remote")).unwrap(), new);
    assert!(!dead_stage.exists());
    assert!(!version_root.join(".stage-lock").exists());
    fixed_staging::assert_no_temporaries(&version_root);

    let updated = fixed_staging::fingerprint(&destination);
    let reuse = fixed_staging::run(&home, &fixed_staging::body(false, "1.0", new));
    assert!(reuse.status.success(), "post-update exact reuse failed");
    assert_eq!(fixed_staging::fingerprint(&destination), updated);
}

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
    write_executable(&fake, format!(
            "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_8.4p1\\n' >&2; exit 0; fi\nsleep 30 &\necho $! > '{}'\nwait\n",
            child_pid.display()
        ),);
    let askpass = temp.path().join("askpass");
    write_executable(&askpass, "#!/bin/sh\nexit 0\n");
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

/// The local ssh client must not inherit the Claude Code spool or plugin
/// directories. The environment is set on a re-executed copy of this test so
/// the multi-threaded parent never mutates its own environment.
#[test]
#[cfg(unix)]
fn local_ssh_client_does_not_inherit_the_claude_spool_or_plugin_dirs() {
    use made_platform::ssh::SshProcess;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::time::{Duration, Instant};

    const NAME: &str = "local_ssh_client_does_not_inherit_the_claude_spool_or_plugin_dirs";
    if std::env::var_os("MADE_SSH_ENV_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--test-threads=1"])
            .env("MADE_SSH_ENV_CHILD", "1")
            .env("MADE_CLAUDE_SPOOL", "/spool/ws-1.jsonl")
            .env("CLAUDE_CODE_PLUGIN_DIRS", "/plugins/made")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "the re-executed test must actually run"
        );
        return;
    }

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
    let seen = root.join("seen.env");
    let fake = root.join("ssh");
    write_executable(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_8.4p1\\n' >&2; exit 0; fi\n\
             printf '%s|%s' \"${{MADE_CLAUDE_SPOOL-unset}}\" \"${{CLAUDE_CODE_PLUGIN_DIRS-unset}}\" > '{}'\n\
             sleep 30\n",
            seen.display()
        ),
    );
    let askpass = root.join("askpass");
    write_executable(&askpass, "#!/bin/sh\nexit 0\n");
    let mut options = options();
    options.executable = fake;
    options.askpass_executable = askpass;
    options.private_directory = root.to_path_buf();
    options.known_hosts = known_hosts;
    assert_eq!(
        std::env::var("MADE_CLAUDE_SPOOL").as_deref(),
        Ok("/spool/ws-1.jsonl"),
        "the fixture parent environment carries the spool"
    );
    let mut process = SshProcess::spawn(options).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let text = loop {
        if let Ok(text) = std::fs::read_to_string(&seen) {
            if !text.is_empty() {
                break text;
            }
        }
        assert!(Instant::now() < deadline, "fake ssh did not report its env");
        std::thread::sleep(Duration::from_millis(10));
    };
    process.cancel();
    assert_eq!(text, "unset|unset");
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
fn stage_update_required_is_a_distinct_sanitized_failure() {
    use made_platform::ssh::{classify_ssh_stderr, SshFailureKind};
    let failure = classify_ssh_stderr(b"noise\nMADE-STAGE-UPDATE-REQUIRED\n");
    assert_eq!(failure.kind().code(), "update_required");
    assert_ne!(failure.kind(), SshFailureKind::Io);
    let debug = format!("{failure:?} {failure}");
    assert!(!debug.contains("MADE-STAGE-UPDATE-REQUIRED"));
    assert!(!debug.contains("noise"));
    let recovery = classify_ssh_stderr(b"noise\nMADE-STAGE-RECOVERY-REQUIRED\nhash=deadbeef\n");
    assert_eq!(recovery.kind().code(), "update_required");
    assert_ne!(recovery.kind(), SshFailureKind::Io);
    let debug = format!("{recovery:?} {recovery}");
    assert!(!debug.contains("MADE-STAGE-RECOVERY-REQUIRED"));
    assert!(!debug.contains("deadbeef"));
    assert!(!debug.contains("noise"));
    // The other classifications are unchanged.
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
}

#[test]
#[cfg(unix)]
fn stage_update_required_exit_is_not_generic_io_and_keeps_no_stderr() {
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
    write_executable(&fake, "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_9.6p1\\n' >&2; exit 0; fi\nprintf 'MADE-STAGE-UPDATE-REQUIRED\\nhash=deadbeef\\n' >&2\nexit 74\n",);
    let askpass = root.join("askpass");
    write_executable(&askpass, "#!/bin/sh\nexit 0\n");
    let mut options = options();
    options.executable = fake;
    options.askpass_executable = askpass;
    options.private_directory = root.to_path_buf();
    options.known_hosts = known_hosts;
    options.command = SshCommand::Stage;
    let mut process = SshProcess::spawn(options).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let failure = loop {
        match process.poll() {
            Some(SshEvent::Failed(failure)) => break failure,
            Some(SshEvent::Exited(_)) => {
                panic!("update-required was reported as a generic exit")
            }
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => panic!("update-required was not classified"),
        }
    };
    assert_eq!(failure.kind(), SshFailureKind::UpdateRequired);
    assert_eq!(failure.kind().code(), "update_required");
    let debug = format!("{failure:?} {failure}");
    assert!(!debug.contains("MADE-STAGE-UPDATE-REQUIRED"));
    assert!(!debug.contains("deadbeef"));
}

#[test]
#[cfg(unix)]
fn terminal_exit_sets_finished_and_later_polls_report_none() {
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
    write_executable(&fake, "#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then printf 'OpenSSH_9.6p1\\n' >&2; exit 0; fi\nexit 73\n",);
    let askpass = root.join("askpass");
    write_executable(&askpass, "#!/bin/sh\nexit 0\n");
    let mut options = options();
    options.executable = fake;
    options.askpass_executable = askpass;
    options.private_directory = root.to_path_buf();
    options.known_hosts = known_hosts;
    options.command = SshCommand::Stage;
    let mut process = SshProcess::spawn(options).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let failure = loop {
        match process.poll() {
            Some(SshEvent::Failed(failure)) => break failure,
            Some(event) => panic!("unexpected event: {event:?}"),
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => panic!("terminal exit was not reported"),
        }
    };
    assert_eq!(failure.kind(), SshFailureKind::Io);
    assert!(
        process.is_finished(),
        "the terminal exit must leave the process finished"
    );
    assert!(
        process.poll().is_none(),
        "a finished process reports no further events"
    );
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
    write_executable(&fake, "#!/bin/sh\nif [ \"$1\" = -V ]; then printf 'OpenSSH_9.6p1\\n' >&2; exit 0; fi\nprintf 'WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\\npassword=hunter2\\n' >&2\nexit 255\n",);
    let askpass = root.join("askpass");
    write_executable(&askpass, "#!/bin/sh\nexit 0\n");
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

#[test]
#[cfg(unix)]
fn forget_known_host_removes_only_the_matching_app_entry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let key = root.join("key");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(status.success(), "ssh-keygen fixture failed");
    let public = std::fs::read_to_string(root.join("key.pub")).unwrap();
    let public = public
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    let known_hosts = root.join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "server.example {public}\n[server.example]:2222 {public}\nother.example {public}\n"
        ),
    )
    .unwrap();
    let ssh_keygen = made_platform::find_executable_on_path(
        std::ffi::OsStr::new("ssh-keygen"),
        &std::env::var_os("PATH").unwrap_or_default(),
    )
    .expect("ssh-keygen on PATH");
    made_platform::ssh::forget_known_host(&ssh_keygen, &known_hosts, "server.example", 22).unwrap();
    let text = std::fs::read_to_string(&known_hosts).unwrap();
    assert!(!text.lines().any(|line| line.starts_with("server.example ")));
    assert!(text.contains("[server.example]:2222"));
    assert!(text.contains("other.example"));
    assert!(!root.join("known_hosts.old").exists());
    assert!(made_platform::ssh::forget_known_host(&ssh_keygen, &known_hosts, "*", 22).is_err());
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
