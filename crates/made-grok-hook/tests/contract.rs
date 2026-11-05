//! Hook contract beyond the ported Python cases: field spellings, status rule
//! order, line shape, log location, bounded input and failure behavior.

mod common;

use std::io::Write;
use std::time::Duration;

use common::*;
use serde_json::{json, Value};

const SHORT: Duration = Duration::from_secs(3);

fn single(grok_home: &std::path::Path, payload: &Value) -> Value {
    let lines = run_event(grok_home, &serde_json::to_vec(payload).unwrap());
    assert_eq!(lines.len(), 1, "{payload}");
    lines.into_iter().next().unwrap()
}

fn status_for(payload: Value) -> Option<String> {
    let grok_home = tempfile::tempdir().unwrap();
    let lines = run_event(grok_home.path(), &serde_json::to_vec(&payload).unwrap());
    lines
        .first()
        .map(|line| line["status"].as_str().unwrap().to_string())
}

#[test]
fn snake_case_field_spellings_are_accepted() {
    let grok_home = tempfile::tempdir().unwrap();
    let line = single(
        grok_home.path(),
        &json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "session_id": "sn",
            "cwd": "/snake",
            "tool_name": "ask_user_question",
        }),
    );
    assert_eq!(line["sessionId"], "sn");
    assert_eq!(line["event"], "Notification");
    assert_eq!(line["notification"], "permission_prompt");
    assert_eq!(line["status"], "attention");

    assert_eq!(
        status_for(json!({"hook_event_name": "PreToolUse", "tool_name": "AskQuestion"})),
        Some("attention".into())
    );
    assert_eq!(
        status_for(json!({"hook_event_name": "Stop", "stop_hook_active": true})),
        Some("working".into())
    );
}

#[test]
fn stop_hook_active_truthiness_follows_the_rules() {
    for falsy in [
        json!(false),
        json!(null),
        json!(0),
        json!(0.0),
        json!(""),
        json!([]),
        json!({}),
    ] {
        assert_eq!(
            status_for(json!({"hookEventName": "Stop", "stopHookActive": falsy})),
            Some("idle".into()),
            "{falsy}"
        );
    }
    for truthy in [
        json!(true),
        json!(1),
        json!(-2),
        json!(0.5),
        json!("no"),
        json!([0]),
        json!({"a": 0}),
    ] {
        assert_eq!(
            status_for(json!({"hookEventName": "Stop", "stopHookActive": truthy})),
            Some("working".into()),
            "{truthy}"
        );
    }
    assert_eq!(
        status_for(json!({"hookEventName": "Stop"})),
        Some("idle".into())
    );
    // The second spelling counts when the first is falsy.
    assert_eq!(
        status_for(
            json!({"hookEventName": "Stop", "stopHookActive": false, "stop_hook_active": true})
        ),
        Some("working".into())
    );
}

#[test]
fn event_names_normalize_dashes_and_case() {
    for (name, status) in [
        ("session-start", "idle"),
        ("SESSION_START", "idle"),
        ("sessionstart", "idle"),
        ("User-Prompt-Submit", "working"),
        ("USERPROMPTSUBMIT", "working"),
        ("stop-failure", "error"),
        ("Stop-Cancelled", "idle"),
        ("session-end", "idle"),
        ("STOP", "idle"),
    ] {
        assert_eq!(
            status_for(json!({"hookEventName": name})),
            Some(status.into()),
            "{name}"
        );
    }
    assert_eq!(
        status_for(json!({"hookEventName": "pre-tool-use", "toolName": "ASK_USER_QUESTION"})),
        Some("attention".into())
    );
    assert_eq!(
        status_for(json!({"hookEventName": "Post-Tool-Use", "toolName": "AskUserQuestion"})),
        Some("working".into())
    );
    assert_eq!(
        status_for(
            json!({"hookEventName": "Notification", "notificationType": "PERMISSION_PROMPT"})
        ),
        Some("attention".into())
    );
}

#[test]
fn first_matching_rule_wins_in_order() {
    // A permission prompt outranks a failure and a stop.
    assert_eq!(
        status_for(
            json!({"hookEventName": "StopFailure", "notificationType": "permission_prompt"})
        ),
        Some("attention".into())
    );
    // An idle prompt outranks a failure.
    assert_eq!(
        status_for(json!({"hookEventName": "StopFailure", "notificationType": "idle_prompt"})),
        Some("idle".into())
    );
    // Session start and prompt submission outrank the notification rules.
    assert_eq!(
        status_for(json!({"hookEventName": "UserPromptSubmit", "notificationType": "idle_prompt"})),
        Some("working".into())
    );
    assert_eq!(
        status_for(
            json!({"hookEventName": "SessionStart", "notificationType": "permission_prompt"})
        ),
        Some("idle".into())
    );
    // An asking tool outranks the notification rules for tool events.
    assert_eq!(
        status_for(
            json!({"hookEventName": "PreToolUse", "toolName": "askquestion", "notificationType": "idle_prompt"})
        ),
        Some("attention".into())
    );
    // A tool event with a non-asking tool matches no tool rule.
    assert_eq!(
        status_for(json!({"hookEventName": "PreToolUse", "toolName": "bash"})),
        None
    );
    // The first named field with non-empty text supplies the value.
    assert_eq!(
        status_for(json!({"hookEventName": "", "hook_event_name": "SessionStart"})),
        Some("idle".into())
    );
    // A notification type that matches no rule on an unmatched event.
    assert_eq!(
        status_for(json!({"hookEventName": "Notification", "notificationType": "other"})),
        None
    );
}

#[test]
fn line_carries_exactly_the_seven_fields() {
    let grok_home = tempfile::tempdir().unwrap();
    let outcome = run_open(
        Some(grok_home.path()),
        None,
        &serde_json::to_vec(&json!({
            "hookEventName": "UserPromptSubmit",
            "sessionId": "s1",
            "cwd": "/work/a",
        }))
        .unwrap(),
        SHORT,
    );
    assert!(outcome.status.success());
    let text = std::fs::read_to_string(log_file(grok_home.path())).unwrap();
    assert!(text.ends_with('\n') && text.matches('\n').count() == 1);
    let line: Value = serde_json::from_str(text.trim_end()).unwrap();
    let object = line.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "at",
            "cwd",
            "event",
            "notification",
            "pid",
            "sessionId",
            "status"
        ]
    );
    assert_eq!(line["sessionId"], "s1");
    assert_eq!(line["cwd"], "/work/a");
    assert_eq!(line["event"], "UserPromptSubmit");
    assert_eq!(line["notification"], "");
    assert_eq!(line["status"], "working");
    assert!(line["pid"].is_u64());
    let at = line["at"].as_str().unwrap();
    let shape: Vec<char> = at.chars().collect();
    assert_eq!(shape.len(), 20, "{at}");
    for (index, symbol) in shape.iter().enumerate() {
        match index {
            4 | 7 => assert_eq!(*symbol, '-', "{at}"),
            10 => assert_eq!(*symbol, 'T', "{at}"),
            13 | 16 => assert_eq!(*symbol, ':', "{at}"),
            19 => assert_eq!(*symbol, 'Z', "{at}"),
            _ => assert!(symbol.is_ascii_digit(), "{at}"),
        }
    }
    // The event and notification are written exactly as received.
    let grok_home = tempfile::tempdir().unwrap();
    let line = single(
        grok_home.path(),
        &json!({"hookEventName": "Session-Start", "notificationType": "Idle_Prompt"}),
    );
    assert_eq!(line["event"], "Session-Start");
    assert_eq!(line["notification"], "Idle_Prompt");
}

#[test]
fn absent_text_fields_are_empty_strings() {
    let grok_home = tempfile::tempdir().unwrap();
    let line = single(
        grok_home.path(),
        &json!({"hookEventName": "SessionStart", "sessionId": 7, "cwd": ["x"]}),
    );
    assert_eq!(line["sessionId"], "");
    assert_eq!(line["cwd"], "");
    assert_eq!(line["notification"], "");
    assert_eq!(line["event"], "SessionStart");
}

#[test]
fn grok_home_unset_and_empty_default_to_the_home_directory() {
    for grok_home in [None, Some(std::path::Path::new(""))] {
        let home = tempfile::tempdir().unwrap();
        let outcome = run_open(
            grok_home,
            Some(home.path()),
            &event_json("SessionStart", &[]),
            SHORT,
        );
        assert!(outcome.status.success());
        assert!(outcome.stdout.is_empty());
        let lines = log_lines(&home.path().join(".grok"));
        assert_eq!(lines.len(), 1, "GROK_HOME {grok_home:?}");
        assert_eq!(lines[0]["status"], "idle");
    }
}

#[test]
fn missing_log_directory_is_created() {
    let root = tempfile::tempdir().unwrap();
    let grok_home = root.path().join("a").join("b");
    let lines = run_event(&grok_home, &event_json("SessionStart", &[]));
    assert_eq!(lines.len(), 1);
}

#[test]
fn existing_log_gains_a_line_without_losing_earlier_ones() {
    let grok_home = tempfile::tempdir().unwrap();
    run_event(grok_home.path(), &event_json("SessionStart", &[]));
    let lines = run_event(grok_home.path(), &event_json("UserPromptSubmit", &[]));
    let statuses: Vec<&str> = lines
        .iter()
        .map(|line| line["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["idle", "working"]);
}

fn assert_silent_no_line(payload: &[u8], close: bool) -> Outcome {
    let grok_home = tempfile::tempdir().unwrap();
    let outcome = if close {
        run_closed(Some(grok_home.path()), None, payload)
    } else {
        run_open(
            Some(grok_home.path()),
            None,
            payload,
            Duration::from_secs(5),
        )
    };
    assert!(outcome.status.success(), "{payload:?}");
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
    assert!(!log_file(grok_home.path()).exists(), "{payload:?}");
    outcome
}

#[test]
fn malformed_empty_or_late_input_writes_nothing() {
    // Closed empty, closed whitespace, truncated and invalid input end at once.
    for payload in [&b""[..], b"  \n\t", b"{\"hookEventName\":\"SessionSta"] {
        let outcome = assert_silent_no_line(payload, true);
        assert!(outcome.elapsed < Duration::from_secs(2), "{payload:?}");
    }
    // A JSON array, string, number or null is not an object.
    for payload in [&b"[1,2,3]"[..], b"\"SessionStart\"", b"42", b"null", b"[]"] {
        assert_silent_no_line(payload, true);
        assert_silent_no_line(payload, false);
    }
    // Invalid JSON can never be repaired by later bytes, so it ends the run at
    // once even with standard input still open.
    for payload in [&b"not json at all"[..], b"{]"] {
        let outcome = assert_silent_no_line(payload, false);
        assert!(
            outcome.elapsed < Duration::from_millis(1_500),
            "{payload:?} took {:?}",
            outcome.elapsed
        );
    }
    // Truncated JSON and silence never complete: about two seconds.
    for payload in [&b"{\"hookEventName\":\"SessionSta"[..], b""] {
        let outcome = assert_silent_no_line(payload, false);
        assert!(
            outcome.elapsed >= Duration::from_millis(1_500)
                && outcome.elapsed < Duration::from_millis(4_500),
            "{payload:?} took {:?}",
            outcome.elapsed
        );
    }
}

#[test]
fn leading_whitespace_is_ignored() {
    let grok_home = tempfile::tempdir().unwrap();
    let mut payload = b" \r\n\t ".to_vec();
    payload.extend(event_json("SessionStart", &[]));
    assert_eq!(run_event(grok_home.path(), &payload).len(), 1);
}

#[test]
fn unwritable_log_still_exits_zero_with_empty_stdout() {
    let root = tempfile::tempdir().unwrap();
    // A regular file where the Grok home directory should be.
    let blocker = root.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let outcome = run_open(
        Some(&blocker.join("home")),
        None,
        &event_json("SessionStart", &[]),
        SHORT,
    );
    assert!(outcome.status.success());
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
    // The log path itself is a directory.
    let grok_home = tempfile::tempdir().unwrap();
    std::fs::create_dir(log_file(grok_home.path())).unwrap();
    let outcome = run_open(
        Some(grok_home.path()),
        None,
        &event_json("SessionStart", &[]),
        SHORT,
    );
    assert!(outcome.status.success());
    assert!(outcome.stdout.is_empty());
}

#[test]
fn invalid_utf8_becomes_replacement_characters() {
    let grok_home = tempfile::tempdir().unwrap();
    let mut payload = br#"{"hookEventName":"SessionStart","sessionId":"a"#.to_vec();
    payload.extend([0xff, 0xfe]);
    payload.extend(br#"b","cwd":"/w"}"#);
    let lines = run_event(grok_home.path(), &payload);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["sessionId"], "a\u{fffd}\u{fffd}b");
}

#[test]
fn valid_character_split_across_reads_stays_intact() {
    let grok_home = tempfile::tempdir().unwrap();
    let whole = serde_json::to_vec(&json!({
        "hookEventName": "SessionStart",
        "sessionId": "s",
        "cwd": "/caf\u{e9}/\u{4e2d}\u{6587}",
    }))
    .unwrap();
    // Cut between the two bytes of the e acute.
    let cut = whole
        .windows(2)
        .position(|pair| pair == [0xc3, 0xa9])
        .unwrap()
        + 1;
    let mut child = spawn(Some(grok_home.path()), None);
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(&whole[..cut]).unwrap();
        stdin.flush().unwrap();
    }
    std::thread::sleep(Duration::from_millis(400));
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&whole[cut..])
        .unwrap();
    let (outcome, _stdin) = wait_open(child, SHORT).expect("exit with stdin open");
    assert!(outcome.status.success());
    let lines = log_lines(grok_home.path());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["cwd"], "/caf\u{e9}/\u{4e2d}\u{6587}");
}

#[test]
fn concurrent_runs_append_whole_lines() {
    let grok_home = tempfile::tempdir().unwrap();
    let count = 24;
    let padding = "p".repeat(2_000);
    let mut children = Vec::new();
    for index in 0..count {
        let mut child = spawn(Some(grok_home.path()), None);
        let payload = serde_json::to_vec(&json!({
            "hookEventName": "UserPromptSubmit",
            "sessionId": format!("s{index}"),
            "cwd": format!("/work/{index}/{padding}"),
        }))
        .unwrap();
        child.stdin.as_mut().unwrap().write_all(&payload).unwrap();
        children.push(child);
    }
    let mut open = Vec::new();
    for child in children {
        let (outcome, stdin) = wait_open(child, Duration::from_secs(10)).expect("exit");
        assert!(outcome.status.success());
        open.push(stdin);
    }
    let text = std::fs::read_to_string(log_file(grok_home.path())).unwrap();
    assert!(text.ends_with('\n'));
    let mut seen: Vec<String> = text
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).expect("whole parseable line");
            value["sessionId"].as_str().unwrap().to_string()
        })
        .collect();
    seen.sort();
    let mut expected: Vec<String> = (0..count).map(|index| format!("s{index}")).collect();
    expected.sort();
    assert_eq!(seen, expected);
}

#[test]
fn nothing_is_written_to_standard_output_for_any_input() {
    let grok_home = tempfile::tempdir().unwrap();
    for payload in [
        event_json("SessionStart", &[]),
        b"{".to_vec(),
        b"[]".to_vec(),
    ] {
        let outcome = run_closed(Some(grok_home.path()), None, &payload);
        assert!(outcome.status.success());
        assert!(outcome.stdout.is_empty());
        assert!(outcome.stderr.is_empty());
    }
}
