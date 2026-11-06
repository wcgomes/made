//! Process-level helpers shared by the hook tests.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const HOOK: &str = env!("CARGO_BIN_EXE_made-grok-status");

/// What one hook run left behind.
pub struct Outcome {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
}

/// Start the hook with a cleared environment, `GROK_HOME` as given and a
/// piped standard input.
pub fn spawn(grok_home: Option<&Path>, home: Option<&Path>) -> Child {
    spawn_hook(HOOK, grok_home, home)
}

pub fn spawn_hook(program: &str, grok_home: Option<&Path>, home: Option<&Path>) -> Child {
    let mut command = Command::new(program);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(grok_home) = grok_home {
        command.env("GROK_HOME", grok_home);
    }
    if let Some(home) = home {
        command.env("HOME", home).env("USERPROFILE", home);
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command.spawn().expect("spawn the hook")
}

/// Wait for exit while keeping the child's standard input open, killing it
/// after `limit`. Returns `None` for a child that outlived the limit.
pub fn wait_open(mut child: Child, limit: Duration) -> Option<(Outcome, std::process::ChildStdin)> {
    let started = Instant::now();
    let stdin = child.stdin.take().expect("piped stdin");
    loop {
        if let Some(status) = child.try_wait().expect("poll the hook") {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            child
                .stdout
                .take()
                .unwrap()
                .read_to_end(&mut stdout)
                .unwrap();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_end(&mut stderr)
                .unwrap();
            return Some((
                Outcome {
                    status,
                    stdout,
                    stderr,
                    elapsed: started.elapsed(),
                },
                stdin,
            ));
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Write `payload`, leave standard input open and wait for exit.
pub fn run_open(
    grok_home: Option<&Path>,
    home: Option<&Path>,
    payload: &[u8],
    limit: Duration,
) -> Outcome {
    let mut child = spawn(grok_home, home);
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(payload).unwrap();
        stdin.flush().unwrap();
    }
    let (outcome, _stdin) =
        wait_open(child, limit).expect("hook exited without waiting for stdin to close");
    outcome
}

/// Write `payload`, close standard input and wait for exit.
pub fn run_closed(grok_home: Option<&Path>, home: Option<&Path>, payload: &[u8]) -> Outcome {
    let mut child = spawn(grok_home, home);
    drop({
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(payload).unwrap();
        stdin
    });
    let started = Instant::now();
    let output = child.wait_with_output().expect("wait for the hook");
    Outcome {
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        elapsed: started.elapsed(),
    }
}

pub fn log_file(grok_home: &Path) -> PathBuf {
    grok_home.join("made-grok-status.jsonl")
}

/// The log's lines as parsed JSON; empty when the log does not exist.
pub fn log_lines(grok_home: &Path) -> Vec<Value> {
    match std::fs::read_to_string(log_file(grok_home)) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str(line).expect("log line parses"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn event_json(name: &str, extra: &[(&str, Value)]) -> Vec<u8> {
    let mut object = serde_json::json!({
        "hookEventName": name,
        "sessionId": "s",
        "cwd": "/work",
    });
    for (key, value) in extra {
        object[*key] = value.clone();
    }
    serde_json::to_vec(&object).unwrap()
}

/// Run one event and require a clean silent exit before stdin closes.
pub fn run_event(grok_home: &Path, payload: &[u8]) -> Vec<Value> {
    let outcome = run_open(Some(grok_home), None, payload, Duration::from_secs(3));
    assert!(
        outcome.status.success(),
        "{:?}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
    log_lines(grok_home)
}
