//! Real OpenCode TUI smoke test (tasks 1.16-1.17).
//!
//! Runs `opencode --standalone` in an isolated HOME/XDG/temp workspace through
//! the real PTY adapter, parses its output with `TerminalCore`, and asserts that
//! the alternate-screen TUI starts within a bounded timeout. It does not touch
//! the user's OpenCode service or data. If `opencode` is not installed the test
//! reports the skip honestly and returns.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use made_platform::{host_platform, PtySize};
use made_terminal::{PtyEvent, PtyOptions, PtyProcess, TerminalCore, TerminalSize};

fn opencode_available() -> bool {
    Command::new("opencode")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn real_opencode_tui_starts_in_the_alternate_screen() {
    if !opencode_available() {
        eprintln!("opencode_tui: skipped (opencode is not on PATH)");
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    let tmp = temp.path().join("tmp");
    for dir in [&home, &workspace, &tmp] {
        fs::create_dir_all(dir).expect("create isolated directory");
    }

    let platform = host_platform();
    let options = PtyOptions::new(&workspace)
        .with_shell("opencode")
        .with_args(["--standalone"])
        .with_size(PtySize {
            rows: 36,
            cols: 120,
        })
        .with_channel_capacity(256)
        .with_env("HOME", home.as_os_str())
        .with_env("XDG_CONFIG_HOME", home.join(".config").into_os_string())
        .with_env("XDG_DATA_HOME", home.join(".local/share").into_os_string())
        .with_env("XDG_STATE_HOME", home.join(".local/state").into_os_string())
        .with_env("XDG_CACHE_HOME", home.join(".cache").into_os_string())
        .with_env("TMPDIR", tmp.as_os_str())
        .with_env("OPENCODE_CONFIG_PROJECT_DISABLE", "1")
        .without_env("OPENCODE_CLI_CONFIG_CONTENT");

    let mut pty = PtyProcess::spawn(&platform, options).expect("spawn real opencode TUI");
    let mut core = TerminalCore::with_size(
        TerminalSize {
            rows: 36,
            cols: 120,
        },
        2_000,
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut bytes = 0_usize;
    let mut saw_escape = false;
    let mut alternate = false;
    let mut screen_text = String::new();
    let mut exit = None;

    while Instant::now() < deadline {
        match pty.recv_timeout(Duration::from_millis(50)) {
            Ok(PtyEvent::Data(data)) => {
                bytes += data.len();
                saw_escape |= data.contains(&0x1b);
                core.feed(&data);
            }
            Ok(PtyEvent::Eof) => break,
            Ok(PtyEvent::Exited(status)) => {
                exit = Some(status);
                break;
            }
            Ok(PtyEvent::ReadError(error)) => panic!("opencode PTY read failed: {error}"),
            Ok(PtyEvent::MonitorError(error)) => {
                panic!("opencode PTY monitor failed: {error}")
            }
            Err(made_terminal::PtyError::Timeout) => {}
            Err(made_terminal::PtyError::Closed) => break,
            Err(error) => panic!("opencode PTY failed: {error}"),
        }

        let snapshot = core.snapshot();
        alternate = snapshot.alternate_screen;
        screen_text = snapshot.text();
        if alternate && !screen_text.trim().is_empty() {
            break;
        }
    }

    eprintln!(
        "opencode_tui: bytes={bytes} alternate={alternate} escape={saw_escape} screen_chars={} exit={exit:?}",
        screen_text.chars().count()
    );

    let terminate = pty.terminate();
    eprintln!("opencode_tui: terminate={terminate:?}");
    assert!(
        !pty.is_running(),
        "opencode child was not reaped after terminate"
    );

    assert!(saw_escape, "opencode produced no terminal escape sequences");
    assert!(
        alternate,
        "opencode TUI did not enter the alternate screen within the bounded timeout; screen text: {:?}",
        screen_text.chars().take(200).collect::<String>()
    );
    assert!(
        !screen_text.trim().is_empty(),
        "opencode alternate screen was empty"
    );
    assert!(
        exit.is_none(),
        "opencode exited before the alternate-screen check completed: {exit:?}"
    );
    assert!(Path::new(&workspace).is_dir());
}
