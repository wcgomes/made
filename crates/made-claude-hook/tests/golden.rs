//! Golden corpus of the Claude Code status hook, written against the code
//! before the shared hook library is extracted (agent-integration-modules,
//! phase 0).
//!
//! Library level: every event, malformed and truncated JSON, the 8 MiB input
//! cap and the `}` completion gate, split multibyte characters, slow and
//! closed standard input, title sanitization and the pid chain with an
//! injected ancestry produce exactly these records, with a fixed clock.
//! Binary level: the executable appends the same records to the spool,
//! compared on every field but the time and the pids, with the spool's bytes,
//! mode and the exit status. The expectations below do not change during the
//! refactor; only the adapter block does.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

// BEGIN characterization adapter
use made_claude_hook::{
    build_record, read_event_with, status_of, Chunk, Input, MAX_INPUT, MAX_PIDS, MAX_RECORD,
    MAX_SPOOL, MAX_TITLE_CHARS,
};

/// The hook executable.
const HOOK: &str = env!("CARGO_BIN_EXE_made-claude-status");

/// The environment variable that names the spool.
const SPOOL_VARIABLE: &str = "MADE_CLAUDE_SPOOL";

/// The environment variable that carries the session id of the agent.
const SESSION_VARIABLE: &str = "CLAUDE_CODE_SESSION_ID";

/// Directory name the spool must live under.
const SPOOL_ROOT: &str = "claude";

/// The record for `payload` read as one chunk with a fixed clock and an
/// injected ancestry.
fn library_record(payload: &[u8], pids: &[u32]) -> Option<String> {
    let mut sent = false;
    let event = read_event_with(
        |_| {
            if sent {
                Chunk::Idle
            } else {
                sent = true;
                Chunk::Data(payload.to_vec())
            }
        },
        Duration::from_secs(10),
    )?;
    record_of(&event, None, pids)
}

/// The record for an already decoded object.
fn record_of(event: &Map<String, Value>, fallback: Option<&str>, pids: &[u32]) -> Option<String> {
    let input = Input::from_event(event, fallback);
    let status = status_of(&input)?;
    build_record(&input, status, pids, CLOCK)
}

/// Read one object from scripted chunks with `deadline`.
fn read_scripted(chunks: Vec<Chunk>, deadline: Duration) -> Option<Map<String, Value>> {
    let mut chunks = chunks.into_iter();
    read_event_with(|_| chunks.next().unwrap_or(Chunk::Idle), deadline)
}

/// The sanitized title the hook would record for a raw title.
fn title_of(raw: &str) -> String {
    let event: Map<String, Value> = serde_json::from_value(serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_title": raw,
    }))
    .unwrap();
    Input::from_event(&event, None).title
}
// END characterization adapter

/// 2023-11-14T22:13:20Z.
const CLOCK: u64 = 1_700_000_000;
const ANCESTRY: [u32; 3] = [4242, 4241, 1];

/// Payload and the exact record the hook writes for it, or `None` when the
/// event is not recorded.
fn table() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    vec![
        (
            "session start",
            r#"{"hook_event_name":"SessionStart","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "stop",
            r#"{"hook_event_name":"Stop","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "user prompt submit",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "permission request",
            r#"{"hook_event_name":"PermissionRequest","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"PermissionRequest\",\"notification\":\"\",\"status\":\"attention\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "stop failure",
            r#"{"hook_event_name":"StopFailure","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"StopFailure\",\"notification\":\"\",\"status\":\"error\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "session end",
            r#"{"hook_event_name":"SessionEnd","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"SessionEnd\",\"notification\":\"\",\"status\":\"ended\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "cwd changed keeps the current status",
            r#"{"hook_event_name":"CwdChanged","session_id":"s","cwd":"/elsewhere"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/elsewhere\",\"event\":\"CwdChanged\",\"notification\":\"\",\"status\":\"\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "permission prompt",
            r#"{"hook_event_name":"Notification","notification_type":"permission_prompt","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"Notification\",\"notification\":\"permission_prompt\",\"status\":\"attention\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "idle prompt",
            r#"{"hook_event_name":"Notification","notification_type":"idle_prompt","session_id":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"Notification\",\"notification\":\"idle_prompt\",\"status\":\"idle\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "other notification",
            r#"{"hook_event_name":"Notification","notification_type":"auth_success","session_id":"s","cwd":"/work"}"#,
            None,
        ),
        (
            "notification without a type",
            r#"{"hook_event_name":"Notification","session_id":"s","cwd":"/work"}"#,
            None,
        ),
        (
            "unknown event",
            r#"{"hook_event_name":"NotARealHook","session_id":"s","cwd":"/work"}"#,
            None,
        ),
        (
            "camel case spellings",
            r#"{"hookEventName":"UserPromptSubmit","sessionId":"cc","cwd":"/camel","notificationType":"idle_prompt"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"cc\",\"cwd\":\"/camel\",\"event\":\"UserPromptSubmit\",\"notification\":\"idle_prompt\",\"status\":\"working\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "title is recorded",
            r#"{"hook_event_name":"SessionStart","session_id":"s","cwd":"/work","session_title":"Refactor parser"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"Refactor parser\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "escapes",
            r#"{"hook_event_name":"SessionStart","session_id":"a\"b\\c","cwd":"/café/\u0001\n"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"a\\\"b\\\\c\",\"cwd\":\"/café/\\u0001\\n\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
        (
            "prompt text and tool data never reach the record",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"s","cwd":"/work","prompt":"SECRET PROMPT","tool_input":{"command":"SECRET"},"transcript_path":"/tmp/t"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\",\"title\":\"\",\"pids\":[4242,4241,1]}\n",
            ),
        ),
    ]
}

#[test]
fn every_event_produces_its_exact_record_at_library_level() {
    for (name, payload, expected) in table() {
        assert_eq!(
            library_record(payload.as_bytes(), &ANCESTRY).as_deref(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn the_session_id_falls_back_to_the_agent_session() {
    let event: Map<String, Value> =
        serde_json::from_str(r#"{"hook_event_name":"Stop","cwd":"/w"}"#).unwrap();
    assert_eq!(
        record_of(&event, Some("from-env"), &[7]).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"from-env\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[7]}\n")
    );
    assert_eq!(
        record_of(&event, None, &[7]).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[7]}\n")
    );
    // The payload's own id wins over the fallback.
    let event: Map<String, Value> =
        serde_json::from_str(r#"{"hook_event_name":"Stop","session_id":"own"}"#).unwrap();
    assert!(record_of(&event, Some("from-env"), &[7])
        .unwrap()
        .contains("\"sessionId\":\"own\""));
}

#[test]
fn titles_are_sanitized_and_capped() {
    assert_eq!(title_of("plain title"), "plain title");
    // Control characters and line separators become spaces.
    assert_eq!(
        title_of("a\u{1b}[31mb\nc\td\u{2028}e\u{2029}f"),
        "a [31mb c d e f"
    );
    assert_eq!(title_of("tab\tand\rreturn"), "tab and return");
    // Bidirectional and other format controls are removed.
    assert_eq!(
        title_of("a\u{202e}b\u{200b}c\u{feff}d\u{2066}e\u{ad}f\u{e0041}g"),
        "abcdefg"
    );
    // The cap counts characters, not bytes.
    let long = "é".repeat(MAX_TITLE_CHARS + 30);
    assert_eq!(title_of(&long).chars().count(), MAX_TITLE_CHARS);
    assert_eq!(MAX_TITLE_CHARS, 120);
    // Camel case spelling and a non-string title.
    let event: Map<String, Value> =
        serde_json::from_str(r#"{"hook_event_name":"SessionStart","sessionTitle":"Camel"}"#)
            .unwrap();
    assert_eq!(Input::from_event(&event, None).title, "Camel");
    let event: Map<String, Value> =
        serde_json::from_str(r#"{"hook_event_name":"SessionStart","session_title":5}"#).unwrap();
    assert_eq!(Input::from_event(&event, None).title, "");
}

#[test]
fn the_pid_chain_is_recorded_as_given_and_capped() {
    let payload = br#"{"hook_event_name":"Stop","session_id":"s","cwd":"/w"}"#;
    let event = read_scripted(vec![Chunk::Data(payload.to_vec())], Duration::from_secs(5)).unwrap();
    // Empty ancestry.
    assert!(record_of(&event, None, &[])
        .unwrap()
        .ends_with("\"title\":\"\",\"pids\":[]}\n"));
    // Order is kept, nearest first.
    assert!(record_of(&event, None, &[30, 20, 10])
        .unwrap()
        .ends_with("\"pids\":[30,20,10]}\n"));
    // At most MAX_PIDS are recorded.
    assert_eq!(MAX_PIDS, 32);
    let long: Vec<u32> = (1..=40).collect();
    let line = record_of(&event, None, &long).unwrap();
    let listed = line
        .split("\"pids\":[")
        .nth(1)
        .unwrap()
        .trim_end_matches("]}\n");
    let listed: Vec<u32> = listed.split(',').map(|pid| pid.parse().unwrap()).collect();
    assert_eq!(listed, (1..=32).collect::<Vec<u32>>());
}

#[test]
fn a_record_larger_than_the_limit_is_dropped() {
    assert_eq!(MAX_RECORD, 4096);
    let event_with = |cwd_length: usize| -> Map<String, Value> {
        serde_json::from_value(serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": "s",
            "cwd": "c".repeat(cwd_length),
        }))
        .unwrap()
    };
    let base = record_of(&event_with(0), None, &[1]).unwrap().len();
    let fits = record_of(&event_with(MAX_RECORD - base), None, &[1]).unwrap();
    assert_eq!(
        fits.len(),
        MAX_RECORD,
        "a record of exactly the limit is kept"
    );
    assert_eq!(
        record_of(&event_with(MAX_RECORD - base + 1), None, &[1]),
        None,
        "one byte more is dropped"
    );
}

#[test]
fn malformed_truncated_and_non_object_input_records_nothing() {
    // A syntax error in a buffer that ends in a closing brace ends the read
    // at once.
    let started = Instant::now();
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"{\"a\": nope}".to_vec())],
            Duration::from_secs(5)
        ),
        None
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // The completion gate parses only a buffer that ends in a closing brace,
    // so text without one is waited on until the deadline.
    let started = Instant::now();
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"not json".to_vec())],
            Duration::from_millis(400)
        ),
        None
    );
    assert!(started.elapsed() >= Duration::from_millis(400));
    for payload in ["[1,2]", "\"text\"", "12", "true", "null"] {
        assert_eq!(
            read_scripted(
                vec![Chunk::Data(payload.as_bytes().to_vec())],
                Duration::from_millis(400)
            ),
            None,
            "{payload}"
        );
    }
    // A truncated value waits for the deadline.
    let started = Instant::now();
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"{\"hook_event_name\":\"Sess".to_vec())],
            Duration::from_millis(300)
        ),
        None
    );
    assert!(started.elapsed() >= Duration::from_millis(300));
    // Input that ends first gives up at once.
    assert_eq!(
        read_scripted(vec![Chunk::Eof], Duration::from_secs(5)),
        None
    );
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"{\"a\":".to_vec()), Chunk::Eof],
            Duration::from_secs(5)
        ),
        None
    );
}

#[test]
fn the_completion_gate_waits_for_a_closing_brace() {
    let object = "{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"cwd\":\"/w\"}";
    let expected = library_record(object.as_bytes(), &[1]);
    assert!(expected.is_some());
    // Trailing whitespace is ignored by the gate.
    for payload in [
        format!("  \n\t{object}"),
        format!("{object}\n"),
        format!("{object} \r\n\t "),
        // Two values: the gate sees the second closing brace, the first value
        // is used.
        format!("{object}{{\"hook_event_name\":\"SessionEnd\"}}"),
    ] {
        assert_eq!(
            library_record(payload.as_bytes(), &[1]),
            expected,
            "{payload:?}"
        );
    }
    // An object followed by text that does not end in a closing brace is not
    // parsed until the deadline.
    let started = Instant::now();
    let waiting = read_scripted(
        vec![Chunk::Data(format!("{object} trailing").into_bytes())],
        Duration::from_millis(400),
    );
    assert_eq!(waiting, None);
    assert!(started.elapsed() >= Duration::from_millis(400));
    // The gate opens when a later chunk ends the buffer with a brace.
    let event = read_scripted(
        vec![
            Chunk::Data(format!("{object} trailing").into_bytes()),
            Chunk::Data(b" }".to_vec()),
        ],
        Duration::from_secs(5),
    );
    assert_eq!(
        event.as_ref().map(|event| event.len()),
        Some(3),
        "the first value is the object"
    );
}

#[test]
fn the_input_cap_accepts_exactly_the_limit_and_refuses_one_byte_more() {
    assert_eq!(MAX_INPUT, 8 * 1024 * 1024);
    let object_of = |total: usize| -> Vec<u8> {
        let head =
            "{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"cwd\":\"/w\",\"filler\":\"";
        let tail = "\"}";
        let fill = total - head.len() - tail.len();
        let mut bytes = head.as_bytes().to_vec();
        bytes.extend(std::iter::repeat_n(b'x', fill));
        bytes.extend_from_slice(tail.as_bytes());
        assert_eq!(bytes.len(), total);
        bytes
    };
    let chunks_of = |bytes: &[u8]| -> Vec<Chunk> {
        bytes
            .chunks(1024 * 1024)
            .map(|part| Chunk::Data(part.to_vec()))
            .collect()
    };
    let at_limit = object_of(MAX_INPUT);
    let event = read_scripted(chunks_of(&at_limit), Duration::from_secs(60))
        .expect("an object of exactly the limit is accepted");
    assert_eq!(
        record_of(&event, None, &[1]).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[1]}\n")
    );
    let started = Instant::now();
    let over = object_of(MAX_INPUT + 1);
    assert_eq!(
        read_scripted(chunks_of(&over), Duration::from_secs(60)),
        None,
        "one byte over the limit is refused"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "refused without waiting for the deadline"
    );
}

#[test]
fn a_large_payload_in_small_chunks_is_read_quickly() {
    let prompt = "p".repeat(2 * 1024 * 1024);
    let whole = format!(
        "{{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"cwd\":\"/w\",\"prompt\":\"{prompt}\"}}"
    )
    .into_bytes();
    let chunks: Vec<Chunk> = whole
        .chunks(4096)
        .map(|part| Chunk::Data(part.to_vec()))
        .collect();
    let started = Instant::now();
    let event = read_scripted(chunks, Duration::from_secs(30)).expect("read");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(record_of(&event, None, &[1]).is_some());
}

#[test]
fn a_multibyte_character_split_across_reads_stays_intact() {
    let whole = "{\"hook_event_name\":\"Stop\",\"cwd\":\"/caf\u{e9}\"}"
        .as_bytes()
        .to_vec();
    for cut in 1..whole.len() {
        let event = read_scripted(
            vec![
                Chunk::Data(whole[..cut].to_vec()),
                Chunk::Idle,
                Chunk::Data(whole[cut..].to_vec()),
            ],
            Duration::from_secs(5),
        )
        .unwrap_or_else(|| panic!("split at {cut}"));
        assert_eq!(
            record_of(&event, None, &[1]).as_deref(),
            Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"cwd\":\"/café\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[1]}\n"),
            "split at {cut}"
        );
    }
}

#[test]
fn invalid_utf8_becomes_replacement_characters() {
    let mut payload = b"{\"hook_event_name\":\"Stop\",\"cwd\":\"/a".to_vec();
    payload.extend_from_slice(&[0xff, 0xfe]);
    payload.extend_from_slice(b"b\"}");
    assert_eq!(
        library_record(&payload, &[1]).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"cwd\":\"/a\u{fffd}\u{fffd}b\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[1]}\n")
    );
}

#[test]
fn slow_input_is_waited_for_within_the_deadline() {
    let mut chunks = vec![
        Chunk::Idle,
        Chunk::Idle,
        Chunk::Data(b"{\"hook_event_name\":".to_vec()),
        Chunk::Idle,
        Chunk::Data(b"\"Stop\"}".to_vec()),
    ];
    chunks.reverse();
    let event = read_event_with(
        |_| chunks.pop().unwrap_or(Chunk::Idle),
        Duration::from_secs(5),
    )
    .expect("slow input completes");
    assert_eq!(
        record_of(&event, None, &[1]).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"cwd\":\"\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[1]}\n")
    );
}

// ---- binary level ------------------------------------------------------------

/// A private spool directory and the spool file in it.
struct Spool {
    _root: tempfile::TempDir,
    dir: PathBuf,
    file: PathBuf,
}

impl Spool {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(SPOOL_ROOT).join("1-abc");
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(dir.join("owner.lock"), "").unwrap();
        let file = dir.join("ws-1.jsonl");
        std::fs::write(&file, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self {
            _root: root,
            dir,
            file,
        }
    }

    fn raw(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }
}

/// What one run of the executable left behind.
struct Run {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    elapsed: Duration,
}

/// Run the hook with a cleared environment. `feed` writes the payload to
/// standard input, which stays open until it returns.
fn run_hook(
    spool: Option<&Path>,
    session: Option<&str>,
    feed: impl FnOnce(&mut std::process::ChildStdin),
) -> Run {
    let mut command = Command::new(HOOK);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(spool) = spool {
        command.env(SPOOL_VARIABLE, spool);
    }
    if let Some(session) = session {
        command.env(SESSION_VARIABLE, session);
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let started = Instant::now();
    let mut child = command.spawn().expect("spawn the hook");
    let mut stdin = child.stdin.take().expect("piped stdin");
    feed(&mut stdin);
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the hook did not exit");
        std::thread::sleep(Duration::from_millis(10));
    };
    let elapsed = started.elapsed();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    Run {
        success: status.success(),
        code: status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        elapsed,
    }
}

/// Write to a hook that may already have exited: a broken pipe is its valid
/// early exit.
fn write_tolerant(stdin: &mut std::process::ChildStdin, bytes: &[u8]) {
    if let Err(error) = stdin.write_all(bytes).and_then(|()| stdin.flush()) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
}

/// The record with its time replaced by a placeholder and its pids returned.
fn mask(line: &str) -> (String, Vec<u32>) {
    let at_start = line.find("\"at\":\"").expect("at") + "\"at\":\"".len();
    let at_end = at_start + line[at_start..].find('"').unwrap();
    let masked = format!("{}<time>{}", &line[..at_start], &line[at_end..]);
    let pids_start = masked.find("\"pids\":[").expect("pids") + "\"pids\":[".len();
    let pids_end = pids_start + masked[pids_start..].find(']').unwrap();
    let pids: Vec<u32> = masked[pids_start..pids_end]
        .split(',')
        .filter(|pid| !pid.is_empty())
        .map(|pid| pid.parse().unwrap())
        .collect();
    let mut masked = masked;
    masked.replace_range(pids_start..pids_end, "<pids>");
    (masked, pids)
}

#[test]
fn the_executable_writes_the_same_records_as_the_library() {
    for (name, payload, expected) in table() {
        let spool = Spool::new();
        let run = run_hook(Some(&spool.file), None, |stdin| {
            write_tolerant(stdin, payload.as_bytes());
        });
        assert!(run.success && run.code == Some(0), "{name}");
        assert!(run.stdout.is_empty(), "{name}: stdout");
        assert!(run.stderr.is_empty(), "{name}: stderr");
        assert!(
            run.elapsed < Duration::from_secs(10),
            "{name}: exits without waiting for stdin to close"
        );
        let written = spool.raw();
        match expected {
            Some(expected) => {
                let (masked, pids) = mask(&written);
                assert_eq!(masked, mask(expected).0, "{name}");
                // The chain starts at the hook's parent, this test process.
                assert_eq!(pids.first().copied(), Some(std::process::id()), "{name}");
                assert!(pids.len() <= MAX_PIDS && !pids.contains(&0), "{name}");
                let mut unique = pids.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(unique.len(), pids.len(), "{name}: no repeated pid");
            }
            None => assert_eq!(written, "", "{name}: nothing is recorded"),
        }
    }
}

#[test]
fn the_executable_appends_records_in_order_and_keeps_the_spool_private() {
    let spool = Spool::new();
    for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
        let payload =
            format!("{{\"hook_event_name\":\"{event}\",\"session_id\":\"s\",\"cwd\":\"/w\"}}");
        let run = run_hook(Some(&spool.file), None, |stdin| {
            write_tolerant(stdin, payload.as_bytes());
        });
        assert!(run.success);
    }
    let masked: Vec<String> = spool.raw().lines().map(|line| mask(line).0).collect();
    assert_eq!(
        masked,
        [
            "{\"at\":\"<time>\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[<pids>]}",
            "{\"at\":\"<time>\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\",\"title\":\"\",\"pids\":[<pids>]}",
            "{\"at\":\"<time>\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[<pids>]}",
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&spool.file).unwrap().permissions().mode() & 0o777,
            0o600,
            "the hook never loosens the spool's mode"
        );
        assert_eq!(
            std::fs::metadata(&spool.dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn the_session_fallback_comes_from_the_agent_environment() {
    let spool = Spool::new();
    let run = run_hook(Some(&spool.file), Some("env-session"), |stdin| {
        write_tolerant(stdin, br#"{"hook_event_name":"Stop","cwd":"/w"}"#);
    });
    assert!(run.success);
    assert_eq!(
        mask(&spool.raw()).0,
        "{\"at\":\"<time>\",\"sessionId\":\"env-session\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[<pids>]}\n"
    );
}

#[test]
fn a_missing_empty_or_relative_spool_exits_zero_silently_and_writes_nothing() {
    for (name, spool) in [
        ("missing", None),
        ("empty", Some(PathBuf::new())),
        ("relative", Some(PathBuf::from("relative/spool.jsonl"))),
    ] {
        let run = run_hook(spool.as_deref(), None, |stdin| {
            write_tolerant(stdin, br#"{"hook_event_name":"Stop","session_id":"s"}"#);
        });
        assert!(run.success && run.code == Some(0), "{name}");
        assert!(run.stdout.is_empty() && run.stderr.is_empty(), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn an_unsafe_spool_is_never_written() {
    use std::os::unix::fs::PermissionsExt;
    let payload = br#"{"hook_event_name":"Stop","session_id":"s","cwd":"/w"}"#;
    let send = |spool: &Path| {
        let run = run_hook(Some(spool), None, |stdin| write_tolerant(stdin, payload));
        assert!(run.success && run.code == Some(0));
        assert!(run.stdout.is_empty() && run.stderr.is_empty());
    };

    // A directory that is not private.
    let spool = Spool::new();
    std::fs::set_permissions(&spool.dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    send(&spool.file);
    assert_eq!(spool.raw(), "");

    // A file that is not private.
    let spool = Spool::new();
    std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o644)).unwrap();
    send(&spool.file);
    assert_eq!(spool.raw(), "");

    // No owner marker beside the spool.
    let spool = Spool::new();
    std::fs::remove_file(spool.dir.join("owner.lock")).unwrap();
    send(&spool.file);
    assert_eq!(spool.raw(), "");

    // A spool that does not exist is not created.
    let spool = Spool::new();
    let missing = spool.dir.join("other.jsonl");
    send(&missing);
    assert!(!missing.exists());

    // A symlink to the spool is not followed.
    let spool = Spool::new();
    let link = spool.dir.join("link.jsonl");
    std::os::unix::fs::symlink(&spool.file, &link).unwrap();
    send(&link);
    assert_eq!(spool.raw(), "");
}

#[test]
fn a_spool_at_its_size_limit_drops_the_record() {
    assert_eq!(MAX_SPOOL, 1024 * 1024);
    let spool = Spool::new();
    let filler = vec![b'x'; MAX_SPOOL as usize];
    std::fs::write(&spool.file, &filler).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let run = run_hook(Some(&spool.file), None, |stdin| {
        write_tolerant(
            stdin,
            br#"{"hook_event_name":"Stop","session_id":"s","cwd":"/w"}"#,
        );
    });
    assert!(run.success && run.code == Some(0));
    assert_eq!(std::fs::read(&spool.file).unwrap(), filler);
}

#[test]
fn malformed_input_exits_zero_silently_and_writes_nothing() {
    for (name, payload) in [
        ("not json", &b"not json"[..]),
        ("array", b"[1]"),
        ("empty", b""),
    ] {
        let spool = Spool::new();
        let run = run_hook(Some(&spool.file), None, |stdin| {
            write_tolerant(stdin, payload);
        });
        assert!(run.success && run.code == Some(0), "{name}");
        assert!(run.stdout.is_empty() && run.stderr.is_empty(), "{name}");
        assert_eq!(spool.raw(), "", "{name}");
    }
}

#[test]
fn truncated_input_with_stdin_closed_exits_zero_silently_and_writes_nothing() {
    let spool = Spool::new();
    let mut command = Command::new(HOOK);
    command
        .env_clear()
        .env(SPOOL_VARIABLE, &spool.file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    write_tolerant(&mut stdin, b"{\"hook_event_name\":\"Sess");
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert_eq!(spool.raw(), "");
}

#[test]
fn slow_input_split_across_writes_is_recorded() {
    let spool = Spool::new();
    let run = run_hook(Some(&spool.file), None, |stdin| {
        write_tolerant(stdin, b"{\"hook_event_name\":\"UserPro");
        std::thread::sleep(Duration::from_millis(600));
        write_tolerant(
            stdin,
            b"mptSubmit\",\"session_id\":\"s\",\"cwd\":\"/work\"}",
        );
    });
    assert!(run.success);
    assert_eq!(
        mask(&spool.raw()).0,
        "{\"at\":\"<time>\",\"sessionId\":\"s\",\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\",\"title\":\"\",\"pids\":[<pids>]}\n"
    );
}

#[test]
fn an_object_followed_by_text_is_not_read_until_the_deadline() {
    let spool = Spool::new();
    let run = run_hook(Some(&spool.file), None, |stdin| {
        write_tolerant(
            stdin,
            b"{\"hook_event_name\":\"Stop\",\"session_id\":\"s\"} not a closing brace",
        );
    });
    assert!(run.success && run.stdout.is_empty() && run.stderr.is_empty());
    assert!(
        run.elapsed >= Duration::from_millis(1500),
        "the completion gate keeps waiting: {:?}",
        run.elapsed
    );
    assert_eq!(spool.raw(), "");
}

#[test]
fn input_over_the_cap_is_refused_without_a_record() {
    let spool = Spool::new();
    let head = "{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"filler\":\"";
    let mut payload = head.as_bytes().to_vec();
    payload.extend(std::iter::repeat_n(b'x', MAX_INPUT));
    payload.extend_from_slice(b"\"}");
    let run = run_hook(Some(&spool.file), None, |stdin| {
        for part in payload.chunks(1024 * 1024) {
            write_tolerant(stdin, part);
        }
    });
    assert!(run.success && run.code == Some(0));
    assert!(run.stdout.is_empty() && run.stderr.is_empty());
    assert_eq!(spool.raw(), "");
}

#[test]
fn input_that_never_completes_gives_up_after_about_two_seconds() {
    let spool = Spool::new();
    let run = run_hook(Some(&spool.file), None, |stdin| {
        write_tolerant(stdin, b"{\"hook_event_name\":");
    });
    assert!(run.success && run.stdout.is_empty() && run.stderr.is_empty());
    assert!(
        run.elapsed >= Duration::from_millis(1500) && run.elapsed < Duration::from_secs(10),
        "{:?}",
        run.elapsed
    );
    assert_eq!(spool.raw(), "");
}
