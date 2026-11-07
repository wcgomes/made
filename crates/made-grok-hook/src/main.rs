//! `made-grok-status`: the grok-made plugin's session-status hook.
//!
//! Grok leaves standard input open after the JSON object, so a reader thread
//! feeds the decoder and the hook exits after one value or about two seconds.
//! It never writes to standard output and always exits with status 0, so it
//! never denies a tool or a prompt.

use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use made_grok_hook::{build_line, log_path, read_event_with, status_of, Chunk, READ_DEADLINE};
use made_hook_common::{run_silently, spawn_stdin_reader};

fn main() {
    run_silently(run);
}

fn run() {
    let receiver = spawn_stdin_reader();
    let event = read_event_with(
        |wait| match receiver.recv_timeout(wait) {
            Ok(chunk) => chunk,
            Err(mpsc::RecvTimeoutError::Timeout) => Chunk::Idle,
            Err(mpsc::RecvTimeoutError::Disconnected) => Chunk::Eof,
        },
        READ_DEADLINE,
    );
    let Some(event) = event else { return };
    let Some(status) = status_of(&event) else {
        return;
    };
    let Some(path) = log_path(std::env::var_os("GROK_HOME"), std::env::home_dir()) else {
        return;
    };
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    // The parent of this process is the Grok process that ran the hook.
    let line = build_line(&event, status, parent_pid(), seconds);
    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    // One write call: with O_APPEND, concurrent runs on a local filesystem
    // never interleave partial lines.
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = log.write_all(line.as_bytes());
    }
}

#[cfg(unix)]
fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

/// The parent process id from a process snapshot, the same source Python's
/// `os.getppid()` uses on Windows.
#[cfg(windows)]
fn parent_pid() -> u32 {
    made_hook_common::process_parents()
        .and_then(|(own, parents)| parents.get(&own).copied())
        .unwrap_or(0)
}
