use made_platform::{PtyRequest, ShellLaunchPlan};

#[test]
#[cfg(unix)]
fn private_cwd_sink_reports_without_any_terminal_output_and_keeps_profiles() {
    use made_platform::shell_integration::CwdReportSink;
    use std::os::unix::fs::PermissionsExt;
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("spaces é")).unwrap();
    let startup = b"PROMPT_COMMAND='printf USER_HOOK'\nPS1='USER_PROMPT>'\n";
    std::fs::write(root.join(".bashrc"), startup).unwrap();
    let sink = CwdReportSink::new(&root).unwrap();
    let mut request = PtyRequest::new("/bin/bash", &root);
    request.env = std::env::vars_os().filter(|(k, _)| k != "HOME").collect();
    request
        .env
        .push(("HOME".into(), root.clone().into_os_string()));
    let plan = ShellLaunchPlan::with_cwd_sink(
        request,
        env!("CARGO_BIN_EXE_made-askpass").into(),
        sink.path().to_owned(),
    );
    assert!(plan.capability().supported, "{}", plan.capability().reason);
    let request = plan.request();
    // No terminal is attached. The report path does not depend on OSC
    // passthrough or on any user multiplexer.
    let bootstrap = plan.bootstrap_directory().expect("bootstrap directory");
    assert_eq!(
        std::fs::symlink_metadata(bootstrap)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for entry in std::fs::read_dir(bootstrap).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            assert_eq!(
                entry.metadata().unwrap().permissions().mode() & 0o777,
                0o600,
                "bootstrap file was not published owner-only"
            );
        }
    }
    assert_eq!(
        std::fs::symlink_metadata(sink.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut child = Command::new(&request.program)
        .args(&request.args)
        .env_clear()
        .envs(request.env.iter().cloned())
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let wait = |expected: &std::path::Path| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if sink.poll().unwrap().as_deref() == Some(expected.as_os_str().as_encoded_bytes()) {
                break;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    };
    let initial = wait(&root);
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("cd 'spaces é'\n".as_bytes())
        .unwrap();
    let changed = wait(&root.join("spaces é"));
    let _ = child.kill();
    let _ = child.wait();
    assert!(initial && changed, "out-of-band shell cwd was not received");
    assert_eq!(std::fs::read(root.join(".bashrc")).unwrap(), startup);
}

#[test]
#[cfg(unix)]
fn cwd_datagrams_are_bounded_and_lossless_not_host_path_resolution() {
    use made_platform::shell_integration::{emit_cwd_report, CwdReportSink};
    let temp = tempfile::tempdir().unwrap();
    let sink = CwdReportSink::new(temp.path()).unwrap();
    let path = b"/not-a-host-path/\xff name";
    emit_cwd_report(sink.path(), path).unwrap();
    assert_eq!(sink.poll().unwrap().as_deref(), Some(path.as_slice()));
    for bad in [b"relative".as_slice(), b"/escape/../path", b"/bad\0path"] {
        assert!(emit_cwd_report(sink.path(), bad).is_err());
    }
    assert!(emit_cwd_report(sink.path(), &vec![b'/'; 4097]).is_err());
}

#[cfg(unix)]
const DSR_QUERY: &[u8] = b"\x1b[6n";
#[cfg(unix)]
const DSR_RESPONSE: &[u8] = b"\x1b[1;1R";

/// PowerShell's readline asks the terminal for a cursor-position report (DSR)
/// and waits for the reply before painting its prompt. Answer every query so a
/// headless PTY is a valid terminal.
#[cfg(unix)]
fn answer_cursor_requests(
    output: &[u8],
    writer: &mut dyn std::io::Write,
    answered: &mut usize,
) -> bool {
    let requests = output
        .windows(DSR_QUERY.len())
        .filter(|window| *window == DSR_QUERY)
        .count();
    while *answered < requests {
        if writer.write_all(DSR_RESPONSE).is_err() {
            return false;
        }
        let _ = writer.flush();
        *answered += 1;
    }
    true
}

/// Read until every report requested so far has arrived.
#[cfg(unix)]
fn receive_shell_report(
    rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    output: &mut Vec<u8>,
    writer: &mut dyn std::io::Write,
    reports: usize,
    answer_cursor_position: bool,
    answered: &mut usize,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let text = String::from_utf8_lossy(output);
        if text.matches("\x1b]7;").count() >= reports
            && text.matches("USER_PROMPT>").count() >= reports
        {
            return true;
        }
        if let Ok(bytes) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
            output.extend(bytes);
            if answer_cursor_position && !answer_cursor_requests(output, writer, answered) {
                return false;
            }
        }
    }
    false
}

/// Send input one character at a time when the shell queries the terminal.
/// Writing the whole line first would queue the DSR replies behind the
/// command text, where readline consumes them as literal keystrokes.
#[cfg(unix)]
fn send_shell_command(
    writer: &mut dyn std::io::Write,
    rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    output: &mut Vec<u8>,
    command: &str,
    answer_cursor_position: bool,
    answered: &mut usize,
) {
    if !answer_cursor_position {
        writer.write_all(command.as_bytes()).unwrap();
        writer.flush().unwrap();
        return;
    }
    for character in command.chars() {
        let mut encoded = [0; 4];
        writer
            .write_all(character.encode_utf8(&mut encoded).as_bytes())
            .unwrap();
        writer.flush().unwrap();
        let mut quiet_since = std::time::Instant::now();
        while quiet_since.elapsed() < std::time::Duration::from_millis(50) {
            match rx.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(bytes) => {
                    output.extend(bytes);
                    if !answer_cursor_requests(output, writer, answered) {
                        return;
                    }
                    quiet_since = std::time::Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[test]
fn decoded_cwd_paths_reject_nonabsolute_control_and_network_paths() {
    for path in [
        "",
        "relative",
        "C:relative",
        "//server/share",
        "/C:relative",
        "/tmp/\0bad",
        "/tmp/\ncontrol",
        "/tmp/\\server",
    ] {
        assert!(made_platform::local_cwd_path(path).is_none(), "{path:?}");
    }
}

#[test]
#[cfg(windows)]
fn native_windows_cwd_requires_drive_absolute_non_device_paths() {
    assert_eq!(
        made_platform::local_cwd_path("/C:/spaces é"),
        Some(std::path::PathBuf::from("C:/spaces é"))
    );
    for path in [
        "/root-relative",
        "/C:/file:stream",
        "/C:/bad?name",
        "/C:/bad*name",
        "//?/C:/device",
    ] {
        assert!(made_platform::local_cwd_path(path).is_none());
    }
}

#[test]
#[cfg(unix)]
fn native_posix_cwd_preserves_absolute_unicode_paths() {
    assert_eq!(
        made_platform::local_cwd_path("/tmp/spaces é"),
        Some(std::path::PathBuf::from("/tmp/spaces é"))
    );
}

#[test]
fn absent_shell_never_claims_reporting() {
    let mut request = PtyRequest::new("/made-nonexistent-shell/bash", std::env::temp_dir());
    request.env = std::env::vars_os().collect();
    let plan = ShellLaunchPlan::new(request.clone());
    assert!(!plan.capability().is_supported());
    assert_eq!(plan.request(), &request);
    assert!(plan.capability().reason.contains("unavailable"));
}

#[test]
#[cfg(unix)]
fn real_installed_shells_preserve_startup_files_and_emit_local_reports() {
    use made_platform::{host_platform, Platform};
    use std::{io::Read, sync::mpsc};
    for shell in ["bash", "zsh", "fish", "pwsh", "powershell"] {
        if std::process::Command::new(shell)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("PENDING (not passed): {shell} executable unavailable");
            continue;
        }
        let root =
            std::env::temp_dir().join(format!("made-shell-fixture-{}-{shell}", std::process::id()));
        std::fs::create_dir_all(root.join("spaces é 日本")).unwrap();
        let (startup, contents) = match shell {
            "bash" => (root.join(".bashrc"), "printf STARTUP_OK\nPROMPT_COMMAND='printf HOOK_OK'\nPS1='USER_PROMPT> '\n"),
            "zsh" => (root.join(".zshrc"), "printf STARTUP_OK\nfunction user_precmd { printf HOOK_OK; }\nprecmd_functions+=(user_precmd)\nPROMPT='USER_PROMPT> '\n"),
            "fish" => (root.join(".config/fish/config.fish"), "printf STARTUP_OK\nfunction fish_prompt\nprintf 'HOOK_OK USER_PROMPT> '\nend\n"),
            _ => (root.join(".config/powershell/Microsoft.PowerShell_profile.ps1"), "[Console]::Write('STARTUP_OK')\nfunction prompt { 'HOOK_OK USER_PROMPT> ' }\n"),
        };
        std::fs::create_dir_all(startup.parent().unwrap()).unwrap();
        std::fs::write(&startup, contents).unwrap();
        let mut request = PtyRequest::new(shell, &root);
        request.env = std::env::vars_os()
            .filter(|(k, _)| k != "ZDOTDIR")
            .collect();
        for (key, value) in [
            ("HOME", root.clone()),
            ("XDG_CONFIG_HOME", root.join(".config")),
        ] {
            request.env.retain(|(k, _)| k != key);
            request.env.push((key.into(), value.into_os_string()));
        }
        let plan = ShellLaunchPlan::new(request);
        assert!(
            plan.capability().is_supported(),
            "{shell}: {}",
            plan.capability().reason
        );
        let platform = host_platform();
        let session = platform.spawn_pty(plan.request()).unwrap();
        let mut reader = session.take_reader().unwrap();
        let mut writer = session.take_writer().unwrap();
        let (tx, rx) = mpsc::sync_channel(32);
        let handle = std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut output = Vec::new();
        let answer_cursor_position = matches!(shell, "pwsh" | "powershell");
        let mut answered = 0;
        let initial = receive_shell_report(
            &rx,
            &mut output,
            writer.as_mut(),
            1,
            answer_cursor_position,
            &mut answered,
        );
        assert!(
            initial,
            "{shell} initial cwd missing: {}",
            String::from_utf8_lossy(&output)
        );
        send_shell_command(
            writer.as_mut(),
            &rx,
            &mut output,
            "cd 'spaces é 日本'\r",
            answer_cursor_position,
            &mut answered,
        );
        let changed = receive_shell_report(
            &rx,
            &mut output,
            writer.as_mut(),
            2,
            answer_cursor_position,
            &mut answered,
        );
        assert!(
            changed,
            "{shell} cd cwd missing: {}",
            String::from_utf8_lossy(&output)
        );
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("STARTUP_OK") && text.contains("HOOK_OK"),
            "{shell}: {text}"
        );
        assert!(
            text.contains("%20") && text.to_ascii_uppercase().contains("%C3%A9"),
            "{shell}: {text}"
        );
        assert_eq!(std::fs::read(&startup).unwrap(), contents.as_bytes());
        session.terminate().unwrap();
        drop(writer);
        drop(session);
        drop(rx);
        handle.join().unwrap();
        drop(plan);
        assert_eq!(std::fs::read(&startup).unwrap(), contents.as_bytes());
        std::fs::remove_dir_all(root).unwrap();
        eprintln!("PASSED real PTY: {shell}");
    }
}

#[test]
#[cfg(unix)]
fn zsh_wrappers_preserve_unset_and_reassigned_zdotdir_sources() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("made-zsh-wrappers-{}", std::process::id()));
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let fake_zsh = root.join("zsh");
    std::fs::write(&fake_zsh, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&fake_zsh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut request = PtyRequest::new(fake_zsh, home.clone());
    request.env = vec![("HOME".into(), home.into_os_string())];
    let plan = ShellLaunchPlan::new(request);
    assert!(
        plan.capability().is_supported(),
        "{}",
        plan.capability().reason
    );
    let bootstrap = plan.bootstrap_directory().expect("bootstrap directory");
    let zshenv = std::fs::read_to_string(bootstrap.join(".zshenv")).unwrap();
    assert!(zshenv.contains("${MADE_ORIGINAL_ZDOTDIR+x}"));
    assert!(zshenv.contains("unset ZDOTDIR"));
    assert!(zshenv.contains("__made_zdotdir=${ZDOTDIR:-$HOME}"));
    assert!(zshenv.contains("$__made_zdotdir/.zshenv"));

    for name in [".zprofile", ".zshrc", ".zlogin", ".zlogout"] {
        let script = std::fs::read_to_string(bootstrap.join(name)).unwrap();
        assert!(
            script.contains("__made_zdotdir=${ZDOTDIR:-$HOME}"),
            "{name} must fall back to $HOME while preserving the user's state:\n{script}"
        );
        assert!(
            script.contains(&format!("$__made_zdotdir/{name}")),
            "{name} must chain the user's file:\n{script}"
        );
        assert!(script.contains("MADE_USER_ZDOTDIR=$ZDOTDIR"));
        assert!(script.contains("MADE_ZDOTDIR_UNSET=1"));
    }
    let zshrc = std::fs::read_to_string(bootstrap.join(".zshrc")).unwrap();
    assert!(zshrc.contains("precmd_functions+=(__made_cwd)"));
    assert!(zshrc.contains("[[ ! -o login ]]"));

    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn real_zsh_zdotdir_unset_and_reassignment_follow_user_startup_files() {
    use made_platform::{host_platform, Platform};
    use std::{
        io::{Read, Write},
        sync::mpsc,
        time::{Duration, Instant},
    };

    let available = std::process::Command::new("zsh")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        eprintln!("PENDING (not passed): zsh executable unavailable");
        return;
    }

    let root = std::env::temp_dir().join(format!("made-zsh-zdotdir-{}", std::process::id()));
    let home = root.join("home é");
    let alt = home.join("alt zsh");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&alt).unwrap();

    // The first scenario unsets ZDOTDIR in .zshenv; zsh's normal fallback is
    // $HOME. The second reassigns it, so every later startup file must come
    // from the new directory.
    std::fs::write(
        home.join(".zshenv"),
        "unset ZDOTDIR\nprint 'profile' > \"$HOME/user.zprofile\"\n",
    )
    .unwrap();
    std::fs::write(
        home.join(".zprofile"),
        "print 'home-profile' > \"$HOME/startup.txt\"\n",
    )
    .unwrap();
    std::fs::write(
        home.join(".zshrc"),
        "print 'home-rc' >> \"$HOME/startup.txt\"\nPROMPT='USER_PROMPT> '\n",
    )
    .unwrap();
    std::fs::write(
        home.join(".zlogin"),
        "print 'home-login' >> \"$HOME/startup.txt\"\n",
    )
    .unwrap();

    let run = |args: &[&str]| {
        let platform = host_platform();
        let mut request = PtyRequest::new("zsh", home.clone());
        request.args = args.iter().map(Into::into).collect();
        request.env = std::env::vars_os()
            .filter(|(key, _)| key != "ZDOTDIR")
            .filter(|(key, _)| key != "HOME")
            .collect();
        request
            .env
            .push(("HOME".into(), home.clone().into_os_string()));
        let plan = ShellLaunchPlan::new(request);
        assert!(
            plan.capability().is_supported(),
            "{}",
            plan.capability().reason
        );
        let session = platform.spawn_pty(plan.request()).unwrap();
        let mut reader = session.take_reader().unwrap();
        let mut writer = session.take_writer().unwrap();
        let (tx, rx) = mpsc::sync_channel(32);
        let handle = std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 || tx.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !home.join("startup.txt").is_file() {
            std::thread::sleep(Duration::from_millis(20));
        }
        // Startup files can exist before zsh installs its line editor and
        // prints the first prompt. Wait for the prompt (and the cwd report)
        // before injecting the final command, otherwise the input can be lost
        // on slower runners such as macOS arm64.
        let mut output = Vec::new();
        let mut answered = 0;
        assert!(
            receive_shell_report(&rx, &mut output, writer.as_mut(), 1, false, &mut answered),
            "zsh prompt did not become ready: {}",
            String::from_utf8_lossy(&output)
        );
        writer
            .write_all(b"print -r -- \"${ZDOTDIR-unset}\" > \"$HOME/final.txt\"\n")
            .unwrap();
        writer.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !home.join("final.txt").is_file() {
            // Keep draining the bounded reader channel: zsh's per-keystroke
            // echo can otherwise fill it and block the child before the
            // command runs.
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(bytes) => output.extend(bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let final_zdotdir = std::fs::read_to_string(home.join("final.txt"))
            .expect("zsh must report the final ZDOTDIR state")
            .trim()
            .to_owned();
        let _ = session.terminate();
        drop(writer);
        drop(session);
        drop(rx);
        handle.join().unwrap();
        final_zdotdir
    };

    let final_zdotdir = run(&["-i", "-l"]);
    let startup = std::fs::read_to_string(home.join("startup.txt")).unwrap();
    assert_eq!(
        startup, "home-profile\nhome-rc\nhome-login\n",
        "unset ZDOTDIR must chain login startup files from $HOME"
    );
    assert_eq!(final_zdotdir, "unset");
    assert!(home.join("user.zprofile").is_file());

    std::fs::write(home.join(".zshenv"), "ZDOTDIR=\"$HOME/alt zsh\"\n").unwrap();
    std::fs::write(
        alt.join(".zprofile"),
        "print 'alt-profile' > \"$HOME/startup.txt\"\n",
    )
    .unwrap();
    std::fs::write(
        alt.join(".zshrc"),
        "print 'alt-rc' >> \"$HOME/startup.txt\"\nPROMPT='USER_PROMPT> '\n",
    )
    .unwrap();
    std::fs::write(
        alt.join(".zlogin"),
        "print 'alt-login' >> \"$HOME/startup.txt\"\n",
    )
    .unwrap();
    std::fs::remove_file(home.join("startup.txt")).unwrap();
    std::fs::remove_file(home.join("final.txt")).unwrap();
    let final_zdotdir = run(&["-i", "-l"]);
    let startup = std::fs::read_to_string(home.join("startup.txt")).unwrap();
    assert_eq!(
        startup, "alt-profile\nalt-rc\nalt-login\n",
        "a reassigned ZDOTDIR must chain the new directory's startup files"
    );
    assert_eq!(
        std::path::Path::new(&final_zdotdir),
        alt.as_path(),
        "the reassigned ZDOTDIR must survive startup"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capability_is_explicit_and_resources_are_launch_owned() {
    let mut request = PtyRequest::new("bash", std::env::temp_dir());
    request.env = std::env::vars_os().collect();
    let plan = ShellLaunchPlan::new(request.clone());
    if !plan.capability().is_supported() {
        eprintln!("PENDING: bash unavailable: {}", plan.capability().reason);
        return;
    }
    assert!(plan.capability().is_supported());
    let path = plan.bootstrap_directory().unwrap().to_owned();
    assert!(path.is_dir());
    let other = ShellLaunchPlan::new(request);
    assert_ne!(other.bootstrap_directory(), Some(path.as_path()));
    drop(plan);
    assert!(!path.exists());
    assert!(other.bootstrap_directory().unwrap().exists());
}

#[test]
fn unsafe_modes_and_unknown_shells_are_unchanged() {
    for (shell, args) in [
        ("cmd.exe", vec![]),
        ("unknown", vec![]),
        ("bash", vec!["-c", "echo ok"]),
        ("bash", vec!["--restricted"]),
        ("pwsh", vec!["-ExecutionPolicy", "Restricted"]),
    ] {
        let mut request = PtyRequest::new(shell, std::env::temp_dir());
        request.args = args.into_iter().map(Into::into).collect();
        let plan = ShellLaunchPlan::new(request.clone());
        assert!(!plan.capability().is_supported());
        assert!(!plan.capability().reason.is_empty());
        assert_eq!(plan.request(), &request);
        assert!(plan.bootstrap_directory().is_none());
    }
}
