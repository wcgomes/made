//! Golden corpus of the Grok Build status hook, written against the code
//! before the shared hook library is extracted (agent-integration-modules,
//! phase 0).
//!
//! Library level: every event, malformed and truncated JSON, oversized input,
//! split multibyte characters, and slow and closed standard input produce
//! exactly these records, with a fixed clock and a fixed parent pid. Binary
//! level: the executable writes the same records, compared on every field but
//! the time and the pid, with its file mode and exit status. The expectations
//! below do not change during the refactor; only the adapter block does.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// BEGIN characterization adapter
use made_grok_hook::{build_line, log_path, read_event_with, status_of, Chunk, LOG_FILE};

/// The hook executable.
const HOOK: &str = env!("CARGO_BIN_EXE_made-grok-status");

/// The environment variable that names the hook's home.
const HOME_VARIABLE: &str = "GROK_HOME";

/// A hook's home directory below a user home.
const DEFAULT_HOME_DIRECTORY: &str = ".grok";

/// The record for `payload` read as one chunk with a fixed clock and parent.
fn library_record(payload: &[u8]) -> Option<String> {
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
    let status = status_of(&event)?;
    Some(build_line(&event, status, PARENT, CLOCK))
}

/// Read one object from scripted chunks with `deadline`.
fn read_scripted(
    chunks: Vec<Chunk>,
    deadline: Duration,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut chunks = chunks.into_iter();
    read_event_with(|_| chunks.next().unwrap_or(Chunk::Idle), deadline)
}

/// The record for an already decoded object.
fn record_of(event: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let status = status_of(event)?;
    Some(build_line(event, status, PARENT, CLOCK))
}

/// Where the hook with `home` as its home writes its log.
fn log_file(home: &Path) -> std::path::PathBuf {
    log_path(Some(home.as_os_str().to_owned()), None).expect("log path")
}
// END characterization adapter

/// 2023-11-14T22:13:20Z.
const CLOCK: u64 = 1_700_000_000;
const PARENT: u32 = 4242;

/// Payload and the exact record the hook writes for it, or `None` when the
/// event is not recorded.
fn table() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    vec![
        (
            "session start",
            r#"{"hookEventName":"SessionStart","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "user prompt submit",
            r#"{"hookEventName":"UserPromptSubmit","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\"}\n",
            ),
        ),
        (
            "pre tool use asking",
            r#"{"hookEventName":"PreToolUse","sessionId":"s","cwd":"/work","toolName":"ask_user_question"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"PreToolUse\",\"notification\":\"\",\"status\":\"attention\"}\n",
            ),
        ),
        (
            "pre tool use asking in another spelling",
            r#"{"hookEventName":"PreToolUse","sessionId":"s","cwd":"/work","toolName":"AskUserQuestion"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"PreToolUse\",\"notification\":\"\",\"status\":\"attention\"}\n",
            ),
        ),
        (
            "pre tool use of another tool",
            r#"{"hookEventName":"PreToolUse","sessionId":"s","cwd":"/work","toolName":"Bash"}"#,
            None,
        ),
        (
            "post tool use asking",
            r#"{"hookEventName":"PostToolUse","sessionId":"s","cwd":"/work","toolName":"AskQuestion"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"PostToolUse\",\"notification\":\"\",\"status\":\"working\"}\n",
            ),
        ),
        (
            "permission prompt",
            r#"{"hookEventName":"Notification","notificationType":"permission_prompt","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"Notification\",\"notification\":\"permission_prompt\",\"status\":\"attention\"}\n",
            ),
        ),
        (
            "idle prompt",
            r#"{"hookEventName":"Notification","notificationType":"idle_prompt","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"Notification\",\"notification\":\"idle_prompt\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "other notification",
            r#"{"hookEventName":"Notification","notificationType":"auth_success","sessionId":"s","cwd":"/work"}"#,
            None,
        ),
        (
            "stop failure",
            r#"{"hookEventName":"StopFailure","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"StopFailure\",\"notification\":\"\",\"status\":\"error\"}\n",
            ),
        ),
        (
            "stop cancelled",
            r#"{"hookEventName":"StopCancelled","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"StopCancelled\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "session end",
            r#"{"hookEventName":"SessionEnd","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"SessionEnd\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "stop",
            r#"{"hookEventName":"Stop","sessionId":"s","cwd":"/work"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "stop with the stop hook active",
            r#"{"hookEventName":"Stop","sessionId":"s","cwd":"/work","stopHookActive":true}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"working\"}\n",
            ),
        ),
        (
            "snake case spellings with a dashed name",
            r#"{"hook_event_name":"Session-Start","session_id":"sn","cwd":"/snake"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"sn\",\"pid\":4242,\"cwd\":\"/snake\",\"event\":\"Session-Start\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "unknown event",
            r#"{"hookEventName":"NotARealHook","sessionId":"s","cwd":"/work"}"#,
            None,
        ),
        (
            "escapes and non-text fields",
            r#"{"hookEventName":"SessionStart","sessionId":"a\"b\\c","cwd":"/café/\u0001\n"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"a\\\"b\\\\c\",\"pid\":4242,\"cwd\":\"/café/\\u0001\\n\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "numeric session id is empty text",
            r#"{"hookEventName":"SessionStart","sessionId":5,"cwd":null}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"pid\":4242,\"cwd\":\"\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\"}\n",
            ),
        ),
        (
            "prompt text never reaches the record",
            r#"{"hookEventName":"UserPromptSubmit","sessionId":"s","cwd":"/work","prompt":"SECRET PROMPT","transcript_path":"/tmp/t"}"#,
            Some(
                "{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\"}\n",
            ),
        ),
    ]
}

#[test]
fn every_event_produces_its_exact_record_at_library_level() {
    for (name, payload, expected) in table() {
        assert_eq!(
            library_record(payload.as_bytes()).as_deref(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn the_timestamp_is_fixed_by_the_clock() {
    let record = library_record(br#"{"hookEventName":"Stop"}"#).unwrap();
    assert!(record.starts_with("{\"at\":\"2023-11-14T22:13:20Z\","));
    assert_eq!(
        library_record(br#"{"hookEventName":"Stop"}"#),
        library_record(br#"{"hookEventName":"Stop"}"#)
    );
}

#[test]
fn malformed_truncated_and_non_object_input_records_nothing() {
    let started = Instant::now();
    // A syntax error ends the read at once.
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"not json".to_vec())],
            Duration::from_secs(5)
        ),
        None
    );
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"{\"a\": nope}".to_vec())],
            Duration::from_secs(5)
        ),
        None
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    // The first value is not an object.
    for payload in ["[1,2]", "\"text\"", "12", "true", "null"] {
        assert_eq!(
            read_scripted(
                vec![Chunk::Data(payload.as_bytes().to_vec())],
                Duration::from_secs(5)
            ),
            None,
            "{payload}"
        );
    }
    // A truncated value waits for the deadline.
    let started = Instant::now();
    assert_eq!(
        read_scripted(
            vec![Chunk::Data(b"{\"hookEventName\":\"Sess".to_vec())],
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
fn leading_whitespace_trailing_text_and_a_second_value_are_tolerated() {
    let expected = library_record(br#"{"hookEventName":"Stop","sessionId":"s","cwd":"/w"}"#);
    assert!(expected.is_some());
    for payload in [
        "  \n\t{\"hookEventName\":\"Stop\",\"sessionId\":\"s\",\"cwd\":\"/w\"}",
        "{\"hookEventName\":\"Stop\",\"sessionId\":\"s\",\"cwd\":\"/w\"} trailing text",
        "{\"hookEventName\":\"Stop\",\"sessionId\":\"s\",\"cwd\":\"/w\"}{\"hookEventName\":\"SessionEnd\"}",
    ] {
        assert_eq!(library_record(payload.as_bytes()), expected, "{payload}");
    }
}

#[test]
fn a_multibyte_character_split_across_reads_stays_intact() {
    let whole = "{\"hookEventName\":\"Stop\",\"cwd\":\"/caf\u{e9}\"}"
        .as_bytes()
        .to_vec();
    // Every split point, including the middle of the two-byte character.
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
            record_of(&event).as_deref(),
            Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"pid\":4242,\"cwd\":\"/café\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\"}\n"),
            "split at {cut}"
        );
    }
}

#[test]
fn invalid_utf8_becomes_replacement_characters() {
    let mut payload = b"{\"hookEventName\":\"Stop\",\"cwd\":\"/a".to_vec();
    payload.extend_from_slice(&[0xff, 0xfe]);
    payload.extend_from_slice(b"b\"}");
    assert_eq!(
        library_record(&payload).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"pid\":4242,\"cwd\":\"/a\u{fffd}\u{fffd}b\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\"}\n")
    );
}

#[test]
fn slow_input_is_waited_for_within_the_deadline() {
    let mut chunks = vec![
        Chunk::Idle,
        Chunk::Idle,
        Chunk::Data(b"{\"hookEventName\":".to_vec()),
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
        record_of(&event).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"\",\"pid\":4242,\"cwd\":\"\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\"}\n")
    );
}

#[test]
fn oversized_input_is_not_capped_and_trailing_text_does_not_gate_the_read() {
    // No input cap: a 9 MiB object (more than the 8 MiB the Claude Code hook
    // accepts) still decodes.
    let filler = "x".repeat(9 * 1024 * 1024);
    let whole = format!(
        "{{\"hookEventName\":\"Stop\",\"sessionId\":\"s\",\"cwd\":\"/w\",\"filler\":\"{filler}\"}}"
    )
    .into_bytes();
    let chunks: Vec<Chunk> = whole
        .chunks(1024 * 1024)
        .map(|part| Chunk::Data(part.to_vec()))
        .collect();
    let event = read_scripted(chunks, Duration::from_secs(60)).expect("no input cap");
    assert_eq!(
        record_of(&event).as_deref(),
        Some("{\"at\":\"2023-11-14T22:13:20Z\",\"sessionId\":\"s\",\"pid\":4242,\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\"}\n")
    );
    // No completion gate: an object followed by text that does not end in a
    // closing brace is read at once.
    let started = Instant::now();
    let event = read_scripted(
        vec![Chunk::Data(
            b"{\"hookEventName\":\"Stop\"} not a closing brace".to_vec(),
        )],
        Duration::from_secs(5),
    );
    assert!(event.is_some());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn the_log_location_follows_the_hook_home() {
    let home = Path::new("/home/user");
    assert_eq!(
        log_path(Some("/custom".into()), Some(home.to_path_buf())),
        Some(Path::new("/custom").join(LOG_FILE))
    );
    assert_eq!(
        log_path(None, Some(home.to_path_buf())),
        Some(home.join(DEFAULT_HOME_DIRECTORY).join(LOG_FILE))
    );
    assert_eq!(
        log_path(Some("".into()), Some(home.to_path_buf())),
        Some(home.join(DEFAULT_HOME_DIRECTORY).join(LOG_FILE))
    );
    assert_eq!(log_path(None, None), None);
    assert_eq!(LOG_FILE, "made-grok-status.jsonl");
}

// ---- binary level ------------------------------------------------------------

/// What one run of the executable left behind.
struct Run {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    elapsed: Duration,
}

/// Run the hook with a cleared environment and `home` as its home. `feed`
/// writes the payload to standard input, which stays open until it returns.
fn run_hook(
    home: Option<&Path>,
    user_home: Option<&Path>,
    feed: impl FnOnce(&mut std::process::ChildStdin),
) -> Run {
    run_hook_with(false, home, user_home, feed)
}

/// Like [`run_hook`]; with `fixed_umask` (POSIX only) the hook runs under
/// `umask 022`, so the mode of a file it creates is a property of the hook.
fn run_hook_with(
    fixed_umask: bool,
    home: Option<&Path>,
    user_home: Option<&Path>,
    feed: impl FnOnce(&mut std::process::ChildStdin),
) -> Run {
    let mut command = if fixed_umask {
        let mut command = Command::new("sh");
        command.arg("-c").arg("umask 022; exec \"$0\"").arg(HOOK);
        command
    } else {
        Command::new(HOOK)
    };
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = home {
        command.env(HOME_VARIABLE, home);
    }
    if let Some(user_home) = user_home {
        command.env("HOME", user_home).env("USERPROFILE", user_home);
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let started = Instant::now();
    let mut child = command.spawn().expect("spawn the hook");
    let mut stdin = child.stdin.take().expect("piped stdin");
    feed(&mut stdin);
    // The hook exits without waiting for standard input to close.
    let deadline = Instant::now() + Duration::from_secs(20);
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

/// The record with its time and pid replaced by placeholders.
fn mask(line: &str) -> String {
    let at_start = line.find("\"at\":\"").expect("at") + "\"at\":\"".len();
    let at_end = at_start + line[at_start..].find('"').unwrap();
    let mut masked = format!("{}<time>{}", &line[..at_start], &line[at_end..]);
    let pid_start = masked.find("\"pid\":").expect("pid") + "\"pid\":".len();
    let pid_end = pid_start
        + masked[pid_start..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap();
    assert!(pid_end > pid_start, "the pid is a number");
    masked.replace_range(pid_start..pid_end, "<pid>");
    masked
}

fn masked_expected(expected: &str) -> String {
    mask(expected)
}

#[test]
fn the_executable_writes_the_same_records_as_the_library() {
    for (name, payload, expected) in table() {
        let home = tempfile::tempdir().unwrap();
        let run = run_hook(Some(home.path()), None, |stdin| {
            stdin.write_all(payload.as_bytes()).unwrap();
            stdin.flush().unwrap();
        });
        assert!(run.success && run.code == Some(0), "{name}");
        assert!(run.stdout.is_empty(), "{name}: stdout");
        assert!(run.stderr.is_empty(), "{name}: stderr");
        assert!(
            run.elapsed < Duration::from_secs(10),
            "{name}: exits without waiting for stdin to close"
        );
        let log = std::fs::read_to_string(log_file(home.path())).unwrap_or_default();
        match expected {
            Some(expected) => assert_eq!(mask(&log), masked_expected(expected), "{name}"),
            None => assert_eq!(log, "", "{name}: nothing is recorded"),
        }
    }
}

#[test]
fn the_executable_appends_one_line_per_run_with_the_parent_pid() {
    let home = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let run = run_hook(Some(home.path()), None, |stdin| {
            stdin
                .write_all(br#"{"hookEventName":"SessionStart","sessionId":"s","cwd":"/work"}"#)
                .unwrap();
        });
        assert!(run.success);
    }
    let log = std::fs::read_to_string(log_file(home.path())).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        // The hook's parent is this test process.
        assert_eq!(value["pid"].as_u64(), Some(u64::from(std::process::id())));
        assert_eq!(
            mask(line),
            "{\"at\":\"<time>\",\"sessionId\":\"s\",\"pid\":<pid>,\"cwd\":\"/work\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\"}"
        );
    }
}

#[cfg(unix)]
#[test]
fn the_executable_creates_the_log_with_mode_0644_under_umask_022() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let nested = home.path().join("not").join("yet");
    let run = run_hook_with(true, Some(&nested), None, |stdin| {
        stdin
            .write_all(br#"{"hookEventName":"Stop","sessionId":"s","cwd":"/w"}"#)
            .unwrap();
    });
    assert!(run.success);
    let log = log_file(&nested);
    assert_eq!(
        std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert!(std::fs::read_to_string(&log).unwrap().ends_with('\n'));
}

#[test]
fn malformed_input_exits_zero_silently_and_writes_nothing() {
    for (name, payload) in [
        ("not json", &b"not json"[..]),
        ("array", b"[1]"),
        ("empty", b""),
    ] {
        let home = tempfile::tempdir().unwrap();
        let run = run_hook(Some(home.path()), None, |stdin| {
            stdin.write_all(payload).unwrap();
        });
        assert!(run.success && run.code == Some(0), "{name}");
        assert!(run.stdout.is_empty() && run.stderr.is_empty(), "{name}");
        assert!(!log_file(home.path()).exists(), "{name}");
    }
}

#[test]
fn truncated_input_with_stdin_closed_exits_zero_silently_and_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let mut command = Command::new(HOOK);
    command
        .env_clear()
        .env(HOME_VARIABLE, home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"{\"hookEventName\":\"Sess").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert!(!log_file(home.path()).exists());
}

#[test]
fn slow_input_split_across_writes_is_recorded() {
    let home = tempfile::tempdir().unwrap();
    let run = run_hook(Some(home.path()), None, |stdin| {
        stdin.write_all(b"{\"hookEventName\":\"UserPro").unwrap();
        stdin.flush().unwrap();
        std::thread::sleep(Duration::from_millis(600));
        stdin
            .write_all(b"mptSubmit\",\"sessionId\":\"s\",\"cwd\":\"/work\"}")
            .unwrap();
        stdin.flush().unwrap();
    });
    assert!(run.success);
    let log = std::fs::read_to_string(log_file(home.path())).unwrap();
    assert_eq!(
        mask(&log),
        "{\"at\":\"<time>\",\"sessionId\":\"s\",\"pid\":<pid>,\"cwd\":\"/work\",\"event\":\"UserPromptSubmit\",\"notification\":\"\",\"status\":\"working\"}\n"
    );
}

#[test]
fn input_that_never_completes_gives_up_after_about_two_seconds() {
    let home = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let run = run_hook(Some(home.path()), None, |stdin| {
        stdin.write_all(b"{\"hookEventName\":").unwrap();
        stdin.flush().unwrap();
    });
    assert!(run.success && run.stdout.is_empty() && run.stderr.is_empty());
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1500) && elapsed < Duration::from_secs(10),
        "{elapsed:?}"
    );
    assert!(!log_file(home.path()).exists());
}

#[test]
fn the_home_variable_empty_or_missing_selects_the_user_home() {
    let user_home = tempfile::tempdir().unwrap();
    let payload = br#"{"hookEventName":"Stop","sessionId":"s","cwd":"/w"}"#;
    // Missing: the default directory below the user home.
    let run = run_hook(None, Some(user_home.path()), |stdin| {
        stdin.write_all(payload).unwrap();
    });
    assert!(run.success);
    let default_log = user_home.path().join(DEFAULT_HOME_DIRECTORY).join(LOG_FILE);
    assert!(default_log.is_file());
    std::fs::remove_file(&default_log).unwrap();
    // Empty: the same.
    let run = run_hook(Some(Path::new("")), Some(user_home.path()), |stdin| {
        stdin.write_all(payload).unwrap();
    });
    assert!(run.success);
    assert!(default_log.is_file());
}

#[test]
fn without_any_home_the_hook_exits_zero_and_writes_nothing() {
    let run = run_hook(None, None, |stdin| {
        stdin
            .write_all(br#"{"hookEventName":"Stop","sessionId":"s","cwd":"/w"}"#)
            .unwrap();
    });
    assert!(run.success && run.code == Some(0));
    assert!(run.stdout.is_empty() && run.stderr.is_empty());
}
