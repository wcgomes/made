//! Process-level port of the removed Python hook's test cases.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{json, Value};

#[test]
fn one_object_appends_fields_while_stdin_stays_open() {
    let grok_home = tempfile::tempdir().unwrap();
    let payload = serde_json::to_vec(&json!({
        "hookEventName": "UserPromptSubmit",
        "sessionId": "sess-1",
        "cwd": "/work/app",
    }))
    .unwrap();
    let outcome = run_open(
        Some(grok_home.path()),
        None,
        &payload,
        Duration::from_secs(3),
    );
    assert!(
        outcome.status.success(),
        "{:?}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
    let lines = log_lines(grok_home.path());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["sessionId"], "sess-1");
    assert_eq!(lines[0]["cwd"], "/work/app");
    assert_eq!(lines[0]["event"], "UserPromptSubmit");
    assert_eq!(lines[0]["status"], "working");
}

#[test]
fn question_permission_stop_and_session_end() {
    type Case = (&'static str, Vec<(&'static str, Value)>, &'static str);
    let cases: Vec<Case> = vec![
        ("SessionStart", vec![], "idle"),
        (
            "PreToolUse",
            vec![("toolName", json!("ask_user_question"))],
            "attention",
        ),
        (
            "PostToolUse",
            vec![("toolName", json!("ask_user_question"))],
            "working",
        ),
        (
            "Notification",
            vec![("notificationType", json!("permission_prompt"))],
            "attention",
        ),
        (
            "Notification",
            vec![("notificationType", json!("idle_prompt"))],
            "idle",
        ),
        ("Stop", vec![("stopHookActive", json!(true))], "working"),
        ("Stop", vec![("stopHookActive", json!(false))], "idle"),
        ("StopFailure", vec![], "error"),
        ("StopCancelled", vec![], "idle"),
        ("SessionEnd", vec![], "idle"),
    ];
    for (name, extra, status) in cases {
        let grok_home = tempfile::tempdir().unwrap();
        let lines = run_event(grok_home.path(), &event_json(name, &extra));
        assert_eq!(lines.len(), 1, "{name}");
        assert_eq!(lines[0]["sessionId"], "s", "{name}");
        assert_eq!(lines[0]["cwd"], "/work", "{name}");
        assert_eq!(lines[0]["event"], name);
        assert_eq!(lines[0]["status"], status, "{name} {extra:?}");
    }
}

#[test]
fn logged_line_records_the_hook_parent_pid() {
    // The hook runs as a child of the Grok session process, so the logged pid
    // is the hook's parent: here the test process.
    let grok_home = tempfile::tempdir().unwrap();
    let lines = run_event(grok_home.path(), &event_json("UserPromptSubmit", &[]));
    assert_eq!(lines[0]["pid"], u64::from(std::process::id()));
}

#[test]
fn unrecognized_event_is_not_logged() {
    let grok_home = tempfile::tempdir().unwrap();
    let outcome = run_open(
        Some(grok_home.path()),
        None,
        &event_json("NotARealHook", &[]),
        Duration::from_secs(3),
    );
    assert!(outcome.status.success());
    let text = std::fs::read_to_string(log_file(grok_home.path())).unwrap_or_default();
    assert_eq!(text, "");
}
