//! The hook's record contract: event-to-status mapping, the fixed record
//! shape, metadata-only output, title sanitizing and the silent exit.
#![cfg(unix)]

mod common;

use std::io::Write;
use std::time::Duration;

use common::{assert_silent, event_json, run_closed, run_event, run_open, spawn, wait_open, Spool};
use serde_json::{json, Value};

const MARKERS: [&str; 6] = [
    "MARKER-PROMPT",
    "MARKER-TOOL-INPUT",
    "MARKER-MESSAGE",
    "MARKER-TRANSCRIPT",
    "MARKER-LAST-ASSISTANT",
    "MARKER-MSG",
];

fn one_record(spool: &Spool, payload: &[u8]) -> Value {
    let mut records = run_event(spool, payload);
    assert_eq!(records.len(), 1, "{}", spool.raw());
    records.remove(0)
}

#[test]
fn events_map_to_the_documented_status() {
    let cases = [
        ("SessionStart", vec![], "", "idle"),
        ("SessionEnd", vec![], "", "ended"),
        ("UserPromptSubmit", vec![], "", "working"),
        ("Stop", vec![], "", "idle"),
        ("StopFailure", vec![], "", "error"),
        ("PermissionRequest", vec![], "", "attention"),
        (
            "Notification",
            vec![("notification_type", json!("permission_prompt"))],
            "permission_prompt",
            "attention",
        ),
        (
            "Notification",
            vec![("notification_type", json!("idle_prompt"))],
            "idle_prompt",
            "idle",
        ),
        ("CwdChanged", vec![], "", ""),
    ];
    for (event, extra, notification, status) in cases {
        let spool = Spool::new();
        let record = one_record(&spool, &event_json(event, &extra));
        assert_eq!(record["event"], event);
        assert_eq!(record["notification"], notification, "{event}");
        assert_eq!(record["status"], status, "{event}");
        assert_eq!(record["sessionId"], "s");
        assert_eq!(record["cwd"], "/work");
    }
}

#[test]
fn record_holds_exactly_the_documented_fields() {
    let spool = Spool::new();
    let record = one_record(&spool, &event_json("UserPromptSubmit", &[]));
    let mut keys: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "at",
            "cwd",
            "event",
            "notification",
            "pids",
            "sessionId",
            "status",
            "title"
        ]
    );
    let at = record["at"].as_str().unwrap();
    assert!(
        at.len() == 20 && at.ends_with('Z') && at.contains('T'),
        "{at}"
    );
    assert_eq!(record["title"], "");
    let pids = record["pids"].as_array().unwrap();
    assert_eq!(pids[0], u64::from(std::process::id()));
}

#[test]
fn cwd_is_taken_from_every_event() {
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "Stop",
        "StopFailure",
        "PermissionRequest",
        "CwdChanged",
    ] {
        let spool = Spool::new();
        let record = one_record(&spool, &event_json(event, &[("cwd", json!("/wt/feature"))]));
        assert_eq!(record["cwd"], "/wt/feature", "{event}");
    }
}

#[test]
fn session_id_falls_back_to_the_environment() {
    let spool = Spool::new();
    let payload = serde_json::to_vec(&json!({"hook_event_name": "Stop", "cwd": "/w"})).unwrap();
    let mut command = std::process::Command::new(common::HOOK);
    command
        .env_clear()
        .env("MADE_CLAUDE_SPOOL", &spool.file)
        .env("CLAUDE_CODE_SESSION_ID", "from-env")
        .stdin(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&payload).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(spool.records()[0]["sessionId"], "from-env");
}

#[test]
fn title_is_sanitized_and_capped() {
    let spool = Spool::new();
    let raw = "a\nb\tc\u{202e}d\u{200b}e\u{7}f\u{2066}g";
    let record = one_record(
        &spool,
        &event_json("UserPromptSubmit", &[("session_title", json!(raw))]),
    );
    assert_eq!(record["title"], "a b cde fg");

    let spool = Spool::new();
    let long = "x".repeat(500);
    let record = one_record(
        &spool,
        &event_json("UserPromptSubmit", &[("session_title", json!(long))]),
    );
    assert_eq!(record["title"].as_str().unwrap().chars().count(), 120);
}

#[test]
fn payload_content_never_reaches_the_record() {
    let spool = Spool::new();
    let payload = event_json(
        "Notification",
        &[
            ("notification_type", json!("permission_prompt")),
            ("prompt", json!(MARKERS[0])),
            ("tool_input", json!({"command": MARKERS[1]})),
            ("tool_name", json!("Bash")),
            ("message", json!(MARKERS[2])),
            ("transcript_path", json!(format!("/t/{}.jsonl", MARKERS[3]))),
            ("last_assistant_message", json!(MARKERS[4])),
            ("msg", json!(MARKERS[5])),
        ],
    );
    one_record(&spool, &payload);
    let raw = spool.raw();
    for marker in MARKERS {
        assert!(!raw.contains(marker), "{marker} leaked into {raw}");
    }
    assert!(!raw.contains("Bash"));
}

#[test]
fn input_left_open_is_read_without_waiting_for_eof() {
    let spool = Spool::new();
    let outcome = run_open(
        Some(&spool.file),
        &event_json("Stop", &[]),
        Duration::from_secs(3),
    );
    assert_silent(&outcome);
    assert!(
        outcome.elapsed < Duration::from_secs(1),
        "{:?}",
        outcome.elapsed
    );
    assert_eq!(spool.records().len(), 1);
}

#[test]
fn silent_input_is_abandoned_after_about_two_seconds() {
    let spool = Spool::new();
    let child = spawn(Some(&spool.file));
    let (outcome, _stdin) = wait_open(child, Duration::from_secs(4)).expect("hook gave up");
    assert_silent(&outcome);
    assert!(
        outcome.elapsed >= Duration::from_millis(1500),
        "{:?}",
        outcome.elapsed
    );
    assert!(spool.records().is_empty());
}

#[test]
fn bad_input_writes_nothing_and_exits_zero_silently() {
    let payloads: [&[u8]; 8] = [
        b"",
        b"   \n",
        b"not json",
        b"[1,2,3]",
        b"{\"hook_event_name\":",
        b"{}",
        br#"{"hook_event_name":"NotARealHook","session_id":"s","cwd":"/w"}"#,
        br#"{"hook_event_name":"Notification","notification_type":"auth_success","session_id":"s","cwd":"/w"}"#,
    ];
    for payload in payloads {
        let spool = Spool::new();
        let outcome = run_closed(Some(&spool.file), payload);
        assert_silent(&outcome);
        assert!(
            spool.records().is_empty(),
            "{:?}",
            String::from_utf8_lossy(payload)
        );
        assert!(spool.raw().is_empty());
    }
}

#[test]
fn unset_spool_is_silent_success() {
    let outcome = run_closed(None, &event_json("Stop", &[]));
    assert_silent(&outcome);
}
