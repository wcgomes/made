//! Record one Grok hook event as a session status line.
//!
//! Pure logic for the `made-grok-status` executable: the chunk-fed decoder
//! that reads the first JSON value without waiting for standard input to
//! close, the field text and status rules, and the log line builder. The
//! process glue lives in `main.rs`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

/// The most time the hook spends waiting for the first complete JSON value.
pub const READ_DEADLINE: Duration = Duration::from_secs(2);

/// Longest single wait for a chunk, so the deadline is checked regularly.
const POLL_SLICE: Duration = Duration::from_millis(200);

/// File name of the status log inside the Grok home.
pub const LOG_FILE: &str = "made-grok-status.jsonl";

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

/// Read one JSON object. `read_chunk` gets the longest time it may wait.
///
/// Leading whitespace is ignored and invalid UTF-8 becomes replacement
/// characters. The accumulated buffer is decoded as a whole, so a valid
/// multibyte character split across two reads stays intact. Returns `None`
/// when no complete value arrives before `deadline`, input ends first, or the
/// first value is not an object.
pub fn read_event_with<F>(mut read_chunk: F, deadline: Duration) -> Option<Map<String, Value>>
where
    F: FnMut(Duration) -> Chunk,
{
    let end = Instant::now() + deadline;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match read_chunk(remaining.min(POLL_SLICE)) {
            Chunk::Idle => continue,
            Chunk::Eof => return None,
            Chunk::Data(bytes) => buffer.extend_from_slice(&bytes),
        }
        let decoded = String::from_utf8_lossy(&buffer);
        let text = decoded.trim_start();
        if text.is_empty() {
            continue;
        }
        let mut values = serde_json::Deserializer::from_str(text).into_iter::<Value>();
        match values.next() {
            Some(Ok(Value::Object(event))) => return Some(event),
            Some(Ok(_)) => return None,
            // Truncated so far: wait for more bytes until the deadline. A syntax
            // error can never be repaired by later bytes, so it ends the read.
            Some(Err(error)) if error.is_eof() => continue,
            Some(Err(_)) => return None,
            None => continue,
        }
    }
}

/// A field's text: its value when that is a non-empty string, else empty.
fn field_text<'a>(event: &'a Map<String, Value>, key: &str) -> &'a str {
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

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().map(|n| n != 0.0).unwrap_or(true),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

/// The status for an event, or `None` when no rule matches.
pub fn status_of(event: &Map<String, Value>) -> Option<&'static str> {
    let name = text_of(event, &["hookEventName", "hook_event_name"])
        .to_lowercase()
        .replace('-', "_");
    let note = text_of(event, &["notificationType", "notification_type"]).to_lowercase();
    let tool = text_of(event, &["toolName", "tool_name"]).to_lowercase();
    let asking = matches!(
        tool.as_str(),
        "ask_user_question" | "askuserquestion" | "askquestion"
    );
    let stop_active = ["stopHookActive", "stop_hook_active"]
        .iter()
        .any(|key| event.get(*key).is_some_and(is_truthy));
    let name = name.as_str();
    if matches!(name, "session_start" | "sessionstart") {
        return Some("idle");
    }
    if matches!(name, "user_prompt_submit" | "userpromptsubmit") {
        return Some("working");
    }
    if matches!(name, "pre_tool_use" | "pretooluse") && asking {
        return Some("attention");
    }
    if matches!(name, "post_tool_use" | "posttooluse") && asking {
        return Some("working");
    }
    if note == "permission_prompt" {
        return Some("attention");
    }
    if note == "idle_prompt" {
        return Some("idle");
    }
    if matches!(name, "stop_failure" | "stopfailure") {
        return Some("error");
    }
    if matches!(
        name,
        "stop_cancelled" | "stopcancelled" | "session_end" | "sessionend"
    ) {
        return Some("idle");
    }
    if name == "stop" {
        return Some(if stop_active { "working" } else { "idle" });
    }
    None
}

/// The log file for the given `GROK_HOME` and user home directory.
///
/// An unset or empty `GROK_HOME` selects `~/.grok`.
pub fn log_path(grok_home: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = match grok_home {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home?.join(".grok"),
    };
    Some(base.join(LOG_FILE))
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

fn json_string(text: &str) -> String {
    // Serializing a string cannot fail.
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// The log line, newline included, for a matched event.
///
/// The object carries exactly `at`, `sessionId`, `pid`, `cwd`, `event`,
/// `notification` and `status`, in that order.
pub fn build_line(
    event: &Map<String, Value>,
    status: &str,
    parent_pid: u32,
    unix_seconds: u64,
) -> String {
    format!(
        "{{\"at\":{},\"sessionId\":{},\"pid\":{},\"cwd\":{},\"event\":{},\"notification\":{},\"status\":{}}}\n",
        json_string(&format_timestamp(unix_seconds)),
        json_string(&text_of(event, &["sessionId", "session_id"])),
        parent_pid,
        json_string(&text_of(event, &["cwd"])),
        json_string(&text_of(event, &["hookEventName", "hook_event_name"])),
        json_string(&text_of(event, &["notificationType", "notification_type"])),
        json_string(status),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_reader_returns_one_object_without_closing_stdin() {
        let payload = br#"{"hookEventName":"SessionStart","sessionId":"s","cwd":"/work"}"#;
        let mut pending = Some(payload.to_vec());
        // One value, then neither bytes nor end of input.
        let event = read_event_with(
            |_| match pending.take() {
                Some(bytes) => Chunk::Data(bytes),
                None => Chunk::Idle,
            },
            READ_DEADLINE,
        )
        .expect("one object");
        assert_eq!(text_of(&event, &["sessionId"]), "s");
        assert_eq!(text_of(&event, &["cwd"]), "/work");
        let unknown: Map<String, Value> =
            serde_json::from_str(r#"{"hookEventName":"NotARealHook"}"#).unwrap();
        assert_eq!(status_of(&unknown), None);
    }

    #[test]
    fn chunk_reader_gives_up_at_the_deadline() {
        let started = Instant::now();
        let event = read_event_with(|_| Chunk::Idle, Duration::from_millis(300));
        assert!(event.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn chunk_reader_joins_split_values_and_characters() {
        let whole = "  \n{\"cwd\":\"/caf\u{e9}\"}".as_bytes().to_vec();
        // Split inside the two-byte `e acute`.
        let cut = whole.len() - 4;
        let mut parts = vec![whole[cut..].to_vec(), whole[..cut].to_vec()];
        let event = read_event_with(
            |_| match parts.pop() {
                Some(bytes) => Chunk::Data(bytes),
                None => Chunk::Idle,
            },
            READ_DEADLINE,
        )
        .expect("object");
        assert_eq!(text_of(&event, &["cwd"]), "/caf\u{e9}");
    }

    #[test]
    fn chunk_reader_stops_at_a_syntax_error_but_waits_on_truncation() {
        let started = Instant::now();
        let bad = read_event_with(
            |_| Chunk::Data(b"not json".to_vec()),
            Duration::from_secs(5),
        );
        assert!(bad.is_none() && started.elapsed() < Duration::from_secs(1));
        let mut sent = false;
        let truncated = read_event_with(
            |_| {
                if sent {
                    Chunk::Idle
                } else {
                    sent = true;
                    Chunk::Data(b"{\"a\":".to_vec())
                }
            },
            Duration::from_millis(300),
        );
        assert!(truncated.is_none() && started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn timestamp_formats_utc_seconds() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_timestamp(1_767_225_599), "2025-12-31T23:59:59Z");
        assert_eq!(format_timestamp(4_102_444_800), "2100-01-01T00:00:00Z");
    }
}
