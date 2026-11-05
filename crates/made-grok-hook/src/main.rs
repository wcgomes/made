//! `made-grok-status`: the grok-made plugin's session-status hook.
//!
//! Grok leaves standard input open after the JSON object, so a reader thread
//! feeds the decoder and the hook exits after one value or about two seconds.
//! It never writes to standard output and always exits with status 0, so it
//! never denies a tool or a prompt.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use made_grok_hook::{build_line, log_path, read_event_with, status_of, Chunk, READ_DEADLINE};

fn main() {
    // A panic must stay silent: standard error is part of the empty-output contract.
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(run);
    std::process::exit(0);
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

/// Feed standard input to the decoder from a thread, because a blocking read
/// cannot be given a deadline on every platform.
fn spawn_stdin_reader() -> mpsc::Receiver<Chunk> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = vec![0_u8; 65_536];
        loop {
            let chunk = match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => Chunk::Eof,
                Ok(count) => Chunk::Data(buffer[..count].to_vec()),
            };
            let done = chunk == Chunk::Eof;
            if sender.send(chunk).is_err() || done {
                break;
            }
        }
    });
    receiver
}

#[cfg(unix)]
fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

/// The parent process id from a process snapshot, the same source Python's
/// `os.getppid()` uses on Windows.
#[cfg(windows)]
fn parent_pid() -> u32 {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    // SAFETY: plain Win32 calls; `entry` is a zeroed POD struct whose `dwSize`
    // is set before use, and the snapshot handle is closed on every path.
    unsafe {
        let own = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut entry: PROCESSENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
        let mut parent = 0;
        let mut more = Process32First(snapshot, &mut entry);
        while more != 0 {
            if entry.th32ProcessID == own {
                parent = entry.th32ParentProcessID;
                break;
            }
            more = Process32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        parent
    }
}
