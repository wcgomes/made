//! The packaged hook, run from the shipped `plugins/grok-made` layout.
//!
//! Gated by `MADE_PACKAGED_HOOK`, the absolute path of the staged hook. CI sets
//! it after staging the desktop package; an ordinary `cargo test` skips these
//! tests. The hook runs with a PATH that holds neither `python` nor `python3`
//! and with its standard input left open, as Grok runs it.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn packaged_hook() -> Option<PathBuf> {
    std::env::var_os("MADE_PACKAGED_HOOK")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[test]
fn packaged_hook_logs_one_line_without_any_interpreter() {
    let Some(hook) = packaged_hook() else {
        eprintln!("skipped: MADE_PACKAGED_HOOK is not set");
        return;
    };
    assert!(hook.is_absolute(), "{}", hook.display());
    assert!(hook.is_file(), "{}", hook.display());
    let scratch = tempfile::tempdir().unwrap();
    let grok_home = scratch.path().join("grok-home");
    // An empty directory is the whole PATH: no python, no python3, no shell.
    let path = scratch.path().join("empty-path");
    std::fs::create_dir(&path).unwrap();
    for name in ["python", "python3", "python.exe", "python3.exe"] {
        assert!(!path.join(name).exists());
    }
    let mut command = Command::new(&hook);
    command
        .env_clear()
        .env("PATH", &path)
        .env("GROK_HOME", &grok_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let mut child = command.spawn().expect("spawn the packaged hook");
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(br#"{"hookEventName":"UserPromptSubmit","sessionId":"packaged","cwd":"/work"}"#)
        .unwrap();
    stdin.flush().unwrap();
    // Standard input stays open until the hook has exited.
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the packaged hook waited for standard input to close"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let text = std::fs::read_to_string(grok_home.join("made-grok-status.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    let line: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(line["sessionId"], "packaged");
    assert_eq!(line["status"], "working");
    assert_eq!(line["pid"], u64::from(std::process::id()));
}
