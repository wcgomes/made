//! Process-level helpers shared by the hook tests.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const HOOK: &str = env!("CARGO_BIN_EXE_made-claude-status");

/// What one hook run left behind.
pub struct Outcome {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
}

/// A private spool directory (mode 0700 on POSIX) and the spool file in it.
pub struct Spool {
    pub root: tempfile::TempDir,
    pub dir: PathBuf,
    pub file: PathBuf,
}

impl Spool {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("claude").join("1-abc");
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        // Made pre-creates the spool and the owner marker; the hook only appends.
        std::fs::write(dir.join("owner.lock"), "").unwrap();
        let file = dir.join("ws-1.jsonl");
        std::fs::write(&file, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self { root, dir, file }
    }

    /// The records written so far; empty when the spool does not exist.
    pub fn records(&self) -> Vec<Value> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).expect("record parses"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn raw(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }
}

/// Start the hook with a cleared environment, `MADE_CLAUDE_SPOOL` as given and
/// a piped standard input.
pub fn spawn(spool: Option<&Path>) -> Child {
    spawn_hook(HOOK, spool)
}

pub fn spawn_hook(program: impl AsRef<std::ffi::OsStr>, spool: Option<&Path>) -> Child {
    let mut command = Command::new(program);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(spool) = spool {
        command.env("MADE_CLAUDE_SPOOL", spool);
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
pub fn run_open(spool: Option<&Path>, payload: &[u8], limit: Duration) -> Outcome {
    let mut child = spawn(spool);
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
pub fn run_closed(spool: Option<&Path>, payload: &[u8]) -> Outcome {
    let mut child = spawn(spool);
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

/// A Claude Code hook payload for `name` with extra or overriding fields.
pub fn event_json(name: &str, extra: &[(&str, Value)]) -> Vec<u8> {
    let mut object = serde_json::json!({
        "hook_event_name": name,
        "session_id": "s",
        "cwd": "/work",
    });
    for (key, value) in extra {
        object[*key] = value.clone();
    }
    serde_json::to_vec(&object).unwrap()
}

/// Assert the silent-success contract of one outcome.
pub fn assert_silent(outcome: &Outcome) {
    assert!(
        outcome.status.success(),
        "{:?}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
}

/// Run one event against the spool and return its records.
pub fn run_event(spool: &Spool, payload: &[u8]) -> Vec<Value> {
    let outcome = run_open(Some(&spool.file), payload, Duration::from_secs(3));
    assert_silent(&outcome);
    spool.records()
}
