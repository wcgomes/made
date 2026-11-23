//! Record one Claude Code hook event as a spool line.
//!
//! Pure logic for the `made-claude-status` executable: the chunk-fed decoder
//! that reads the first JSON value without waiting for standard input to
//! close, the fixed input struct, the status and title rules, and the record
//! encoder. Only the fields of [`Input`] are ever copied out of the payload, so
//! prompts, tool data, notification text and transcript paths cannot reach a
//! write. The process glue lives in `main.rs`.

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

pub use made_hook_common::{format_timestamp, Chunk, READ_DEADLINE};
use made_hook_common::{json_string, text_of, POLL_SLICE};

/// Most bytes of standard input the hook accepts before giving up.
pub const MAX_INPUT: usize = 8 * 1024 * 1024;

/// Largest record, newline included, the hook writes.
pub const MAX_RECORD: usize = 4 * 1024;

/// Largest spool file; a record that would pass it is dropped.
pub const MAX_SPOOL: u64 = 1024 * 1024;

/// Most ancestor pids recorded.
pub const MAX_PIDS: usize = 32;

/// Longest recorded session title, in characters.
pub const MAX_TITLE_CHARS: usize = 120;

/// Read one JSON object. `read_chunk` gets the longest time it may wait.
///
/// Leading whitespace is ignored and invalid UTF-8 becomes replacement
/// characters. The accumulated buffer is decoded as a whole, so a valid
/// multibyte character split across two reads stays intact. Returns `None`
/// when no complete value arrives before `deadline`, input ends first, more
/// than [`MAX_INPUT`] bytes arrive, or the first value is not an object.
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
        if buffer.len() > MAX_INPUT {
            return None;
        }
        // A complete object ends in `}`, so decode and parse only then: this
        // keeps a large payload that arrives in many chunks linear.
        if buffer.iter().rfind(|byte| !byte.is_ascii_whitespace()) != Some(&b'}') {
            continue;
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

/// The only payload fields the hook keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub session_id: String,
    pub cwd: String,
    pub event: String,
    pub notification: String,
    /// Sanitized and capped; empty when the event carries no title.
    pub title: String,
}

impl Input {
    /// Copy the recorded fields out of a payload. `fallback_session` is used
    /// when the payload has no session id.
    pub fn from_event(event: &Map<String, Value>, fallback_session: Option<&str>) -> Self {
        let mut session_id = text_of(event, &["session_id", "sessionId"]);
        if session_id.is_empty() {
            session_id = fallback_session.unwrap_or("").to_string();
        }
        Self {
            session_id,
            cwd: text_of(event, &["cwd"]),
            event: text_of(event, &["hook_event_name", "hookEventName"]),
            notification: text_of(event, &["notification_type", "notificationType"]),
            title: sanitize_title(&text_of(event, &["session_title", "sessionTitle"])),
        }
    }
}

/// The derived status for an event: `Some("")` keeps the session's current
/// status, and `None` means the event is not recorded.
pub fn status_of(input: &Input) -> Option<&'static str> {
    match input.event.as_str() {
        "SessionStart" | "Stop" => Some("idle"),
        "UserPromptSubmit" => Some("working"),
        "PermissionRequest" => Some("attention"),
        "StopFailure" => Some("error"),
        "SessionEnd" => Some("ended"),
        "CwdChanged" => Some(""),
        "Notification" => match input.notification.as_str() {
            "permission_prompt" => Some("attention"),
            "idle_prompt" => Some("idle"),
            _ => None,
        },
        _ => None,
    }
}

/// Bidirectional and other format controls that must not reach a display.
fn is_removed_format(c: char) -> bool {
    matches!(c,
        '\u{ad}' | '\u{61c}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{206f}'
        | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}'
        | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0001}'
        | '\u{e0020}'..='\u{e007f}')
}

/// Control characters and newlines become spaces, bidirectional and format
/// controls are removed, and the result is cut to [`MAX_TITLE_CHARS`].
pub fn sanitize_title(raw: &str) -> String {
    raw.chars()
        .filter(|c| !is_removed_format(*c))
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// The record line, newline included, or `None` when it would exceed
/// [`MAX_RECORD`].
///
/// The object carries exactly `at`, `sessionId`, `cwd`, `event`,
/// `notification`, `status`, `title` and `pids`, in that order.
pub fn build_record(
    input: &Input,
    status: &str,
    pids: &[u32],
    unix_seconds: u64,
) -> Option<String> {
    let pids: Vec<String> = pids
        .iter()
        .take(MAX_PIDS)
        .map(|pid| pid.to_string())
        .collect();
    let line = format!(
        "{{\"at\":{},\"sessionId\":{},\"cwd\":{},\"event\":{},\"notification\":{},\"status\":{},\"title\":{},\"pids\":[{}]}}\n",
        json_string(&format_timestamp(unix_seconds)),
        json_string(&input.session_id),
        json_string(&input.cwd),
        json_string(&input.event),
        json_string(&input.notification),
        json_string(status),
        json_string(&input.title),
        pids.join(","),
    );
    (line.len() <= MAX_RECORD).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(text: &str) -> Map<String, Value> {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn chunk_reader_returns_one_object_without_closing_stdin() {
        let payload = br#"{"hook_event_name":"SessionStart","session_id":"s","cwd":"/work"}"#;
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
        let input = Input::from_event(&event, None);
        assert_eq!(input.session_id, "s");
        assert_eq!(input.cwd, "/work");
        assert_eq!(status_of(&input), Some("idle"));
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
        assert_eq!(Input::from_event(&event, None).cwd, "/caf\u{e9}");
    }

    #[test]
    fn chunk_reader_stops_at_a_syntax_error_but_waits_on_truncation() {
        let started = Instant::now();
        let bad = read_event_with(
            |_| Chunk::Data(b"not json}".to_vec()),
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
    fn chunk_reader_handles_a_large_payload_in_small_chunks_quickly() {
        let prompt = "p".repeat(2 * 1024 * 1024);
        let whole = format!(
            "{{\"hook_event_name\":\"UserPromptSubmit\",\"prompt\":\"{prompt}\",\"cwd\":\"/w\"}}"
        )
        .into_bytes();
        let mut chunks = whole.chunks(1024);
        let started = Instant::now();
        let event = read_event_with(
            |_| match chunks.next() {
                Some(bytes) => Chunk::Data(bytes.to_vec()),
                None => Chunk::Idle,
            },
            READ_DEADLINE,
        )
        .expect("object");
        assert_eq!(Input::from_event(&event, None).cwd, "/w");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn chunk_reader_refuses_input_past_the_bound() {
        let mut first = true;
        let event = read_event_with(
            |_| {
                if first {
                    first = false;
                    Chunk::Data(vec![b' '; MAX_INPUT + 1])
                } else {
                    Chunk::Data(b"{}".to_vec())
                }
            },
            READ_DEADLINE,
        );
        assert!(event.is_none());
    }

    #[test]
    fn input_keeps_only_the_recorded_fields() {
        let event = object(
            r#"{"hook_event_name":"Notification","notification_type":"idle_prompt",
                "session_id":"s","cwd":"/w","prompt":"p","message":"m","tool_input":{"a":1}}"#,
        );
        let input = Input::from_event(&event, Some("env"));
        assert_eq!(
            input,
            Input {
                session_id: "s".into(),
                cwd: "/w".into(),
                event: "Notification".into(),
                notification: "idle_prompt".into(),
                title: String::new(),
            }
        );
        let no_session = object(r#"{"hook_event_name":"Stop"}"#);
        assert_eq!(
            Input::from_event(&no_session, Some("env")).session_id,
            "env"
        );
    }

    #[test]
    fn title_rules() {
        assert_eq!(sanitize_title("a\nb\r\tc"), "a b  c");
        assert_eq!(sanitize_title("x\u{202e}y\u{200f}z\u{2066}\u{feff}"), "xyz");
        assert_eq!(sanitize_title(&"é".repeat(300)).chars().count(), 120);
        // Line and paragraph separators become spaces; musical format
        // controls are removed, as in the app's display sanitizer.
        assert_eq!(sanitize_title("a\u{2028}b\u{2029}c"), "a b c");
        assert_eq!(sanitize_title("a\u{1d173}b\u{1d17a}c\u{1d174}"), "abc");
    }

    #[test]
    fn record_is_bounded_and_ordered() {
        let input = Input::from_event(
            &object(r#"{"hook_event_name":"Stop","session_id":"s","cwd":"/w"}"#),
            None,
        );
        let line = build_record(&input, "idle", &[3, 2, 1], 0).unwrap();
        assert_eq!(
            line,
            "{\"at\":\"1970-01-01T00:00:00Z\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[3,2,1]}\n"
        );
        let many: Vec<u32> = (1..=100).collect();
        let line = build_record(&input, "idle", &many, 0).unwrap();
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["pids"].as_array().unwrap().len(), MAX_PIDS);
        let huge = Input {
            cwd: "d".repeat(MAX_RECORD),
            ..input
        };
        assert_eq!(build_record(&huge, "idle", &[], 0), None);
    }

    #[test]
    fn timestamp_formats_utc_seconds() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_timestamp(1_767_225_599), "2025-12-31T23:59:59Z");
        assert_eq!(format_timestamp(4_102_444_800), "2100-01-01T00:00:00Z");
    }
}
