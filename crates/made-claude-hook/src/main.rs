//! `made-claude-status`: the claude-code-made plugin's session-event hook.
//!
//! Claude Code may leave standard input open after the JSON object, so a
//! reader thread feeds the decoder and the hook exits after one value or about
//! two seconds. It appends at most one metadata record to the spool named by
//! `MADE_CLAUDE_SPOOL`, never writes to standard output and always exits with
//! status 0, so it never denies a tool or a prompt.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use made_claude_hook::{
    build_record, read_event_with, status_of, Chunk, Input, MAX_PIDS, MAX_SPOOL, READ_DEADLINE,
};

/// How long the hook waits for the reader's lock before dropping the record.
const LOCK_WAIT: Duration = Duration::from_millis(300);

fn main() {
    // A panic must stay silent: standard error is part of the empty-output contract.
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(run);
    std::process::exit(0);
}

fn run() {
    let Some(spool) = std::env::var_os("MADE_CLAUDE_SPOOL")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    else {
        return;
    };
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
    let fallback = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let input = Input::from_event(&event, fallback.as_deref());
    drop(event);
    let Some(status) = status_of(&input) else {
        return;
    };
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let Some(line) = build_record(&input, status, &pid_chain(), seconds) else {
        return;
    };
    append(&spool, line.as_bytes());
}

/// Whether `path` is `<...>/claude/<pid>-<id>/<name>.jsonl` with Made's
/// `owner.lock` marker, a regular file, beside it. Made creates both, so a
/// stale or foreign path is never written.
fn spool_shape_is_valid(path: &Path, parent: &Path) -> bool {
    let name_of = |p: &Path| p.file_name().and_then(|n| n.to_str()).map(str::to_owned);
    let Some(file_name) = name_of(path) else {
        return false;
    };
    let Some(directory) = name_of(parent) else {
        return false;
    };
    let owner_dir_ok = directory.split_once('-').is_some_and(|(pid, id)| {
        !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty()
    });
    file_name.len() > ".jsonl".len()
        && file_name.ends_with(".jsonl")
        && owner_dir_ok
        && parent
            .parent()
            .and_then(name_of)
            .is_some_and(|name| name == "claude")
        && std::fs::symlink_metadata(parent.join("owner.lock"))
            .is_ok_and(|metadata| metadata.is_file())
}

/// Append one record, once, to a spool that passes every safety check.
///
/// Made pre-creates the spool and its `owner.lock`; the hook never creates
/// either. The parent directory must be a real directory private to the current user,
/// the file must not be reached through a symlink, and the file must stay at
/// or below [`MAX_SPOOL`]. The write happens under the advisory lock the
/// reader takes before it truncates, and the hook gives up rather than wait.
fn append(path: &Path, line: &[u8]) {
    let Some(parent) = path.parent() else { return };
    if !spool_shape_is_valid(path, parent) || !private_directory(parent) {
        return;
    }
    let Some(mut file) = open_private_file(path) else {
        return;
    };
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return,
        }
    }
    let Ok(metadata) = file.metadata() else {
        return;
    };
    if metadata.len().saturating_add(line.len() as u64) > MAX_SPOOL {
        return;
    }
    // One write call: with O_APPEND, concurrent runs on a local filesystem
    // never interleave partial lines.
    let _ = file.write_all(line);
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

/// Whether `directory` is a real (not symlinked) directory that only the
/// current user can use.
#[cfg(unix)]
fn private_directory(directory: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(metadata) = std::fs::symlink_metadata(directory) else {
        return false;
    };
    metadata.is_dir()
        && metadata.uid() == effective_uid()
        && metadata.permissions().mode() & 0o777 == 0o700
}

/// Open the existing spool file for appending, never following a symlink and
/// only when it is a private regular file we own.
#[cfg(unix)]
fn open_private_file(path: &Path) -> Option<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    // O_NONBLOCK keeps a planted FIFO from stalling the open; it is harmless
    // on the regular file that passes the check below.
    let file = std::fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    (metadata.is_file()
        && metadata.uid() == effective_uid()
        && metadata.permissions().mode() & 0o777 == 0o600)
        .then_some(file)
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

/// Windows relies on the per-user temp location; only symlinks and reparse
/// points are refused.
#[cfg(not(unix))]
fn private_directory(directory: &Path) -> bool {
    std::fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> Option<File> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
        return None;
    }
    std::fs::OpenOptions::new().append(true).open(path).ok()
}

/// The ancestor pids, nearest first: this process's parent, then its parent,
/// and so on, at most [`MAX_PIDS`] and never repeating a pid.
#[cfg(unix)]
fn pid_chain() -> Vec<u32> {
    let mut chain = Vec::new();
    let mut pid = std::os::unix::process::parent_id();
    while pid > 0 && chain.len() < MAX_PIDS && !chain.contains(&pid) {
        chain.push(pid);
        match parent_of(pid) {
            Some(parent) => pid = parent,
            None => break,
        }
    }
    chain
}

#[cfg(target_os = "linux")]
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesised and may hold spaces and parentheses,
    // so the fields start after the last `)`: state, then the parent pid.
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "macos")]
fn parent_of(pid: u32) -> Option<u32> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a zeroed POD struct of exactly `size` bytes and
    // `proc_pidinfo` writes at most `size` bytes into it.
    let (written, info) = unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let written = libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
            size,
        );
        (written, info)
    };
    (written == size).then_some(info.pbi_ppid)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn parent_of(_pid: u32) -> Option<u32> {
    None
}

/// The ancestor pids from one process snapshot, nearest first.
#[cfg(windows)]
fn pid_chain() -> Vec<u32> {
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let mut parents: HashMap<u32, u32> = HashMap::new();
    // SAFETY: plain Win32 calls; `entry` is a zeroed POD struct whose `dwSize`
    // is set before use, and the snapshot handle is closed on every path.
    let own = unsafe {
        let own = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut entry: PROCESSENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
        let mut more = Process32First(snapshot, &mut entry);
        while more != 0 {
            parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            more = Process32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        own
    };
    let mut chain = Vec::new();
    let mut pid = parents.get(&own).copied().unwrap_or(0);
    while pid > 0 && chain.len() < MAX_PIDS && !chain.contains(&pid) {
        chain.push(pid);
        match parents.get(&pid) {
            Some(parent) => pid = *parent,
            None => break,
        }
    }
    chain
}
