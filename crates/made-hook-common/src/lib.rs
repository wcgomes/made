//! Code shared by the native session-status hooks.
//!
//! `made-grok-status` and `made-claude-status` read one JSON object from a
//! standard input that the agent leaves open, and write one metadata line.
//! This crate holds what both do identically: the chunk type and reader
//! thread, the read deadline, the timestamp and JSON text helpers, the field
//! lookups, the panic-silent `main` wrapper and, on Windows, the process
//! snapshot used to find parent pids. The decoder, the status rules and the
//! record formats stay in each hook.

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Map, Value};

/// The most time a hook spends waiting for the first complete JSON value.
pub const READ_DEADLINE: Duration = Duration::from_secs(2);

/// Longest single wait for a chunk, so the deadline is checked regularly.
pub const POLL_SLICE: Duration = Duration::from_millis(200);

/// What one read attempt on standard input produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    /// Bytes that arrived.
    Data(Vec<u8>),
    /// Standard input ended.
    Eof,
    /// Neither bytes nor end of input arrived within the wait.
    Idle,
}

/// Run `run` and exit with status 0, whatever happens.
///
/// A panic stays silent, because standard error is part of the hooks'
/// empty-output contract.
pub fn run_silently(run: fn()) -> ! {
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(run);
    std::process::exit(0);
}

/// Feed standard input to the decoder from a thread, because a blocking read
/// cannot be given a deadline on every platform.
pub fn spawn_stdin_reader() -> mpsc::Receiver<Chunk> {
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

/// A field's text: its value when that is a string, else empty.
pub fn field_text<'a>(event: &'a Map<String, Value>, key: &str) -> &'a str {
    match event.get(key) {
        Some(Value::String(value)) => value,
        _ => "",
    }
}

/// The text of the first named field with non-empty text, else empty.
pub fn text_of(event: &Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .map(|key| field_text(event, key))
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

/// UTC `YYYY-MM-DDTHH:MM:SSZ` for a Unix time in seconds.
pub fn format_timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds = unix_seconds % 86_400;
    // Civil date from days since 1970-01-01 (proleptic Gregorian calendar).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// `text` as a JSON string literal.
pub fn json_string(text: &str) -> String {
    // Serializing a string cannot fail.
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// This process's pid and a pid-to-parent-pid table from one process
/// snapshot, the same source Python's `os.getppid()` uses on Windows. `None`
/// when the snapshot cannot be taken.
#[cfg(windows)]
pub fn process_parents() -> Option<(u32, std::collections::HashMap<u32, u32>)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let mut parents = std::collections::HashMap::new();
    // SAFETY: plain Win32 calls; `entry` is a zeroed POD struct whose `dwSize`
    // is set before use, and the snapshot handle is closed on every path.
    let own = unsafe {
        let own = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
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
    Some((own, parents))
}
