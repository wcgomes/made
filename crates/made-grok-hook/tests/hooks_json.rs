//! The hooks.json matcher contract, against the repository source and, when
//! the Windows variant is generated at staging, against that variant too.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn source_hooks_json() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/grok-made/hooks/hooks.json")
}

fn load(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read hooks.json"))
        .expect("parse hooks.json")
}

fn matcher<'a>(hooks: &'a Value, event: &str) -> Vec<&'a str> {
    hooks["hooks"][event][0]["matcher"]
        .as_str()
        .unwrap_or_else(|| panic!("{event} has a matcher"))
        .split('|')
        .collect()
}

fn assert_matchers(hooks: &Value) {
    for event in ["PreToolUse", "PostToolUse"] {
        assert!(matcher(hooks, event).contains(&"ask_user_question"));
    }
    let notification = matcher(hooks, "Notification");
    assert!(notification.contains(&"permission_prompt"));
    assert!(notification.contains(&"idle_prompt"));
}

#[test]
fn source_hooks_json_maps_ask_user_question() {
    assert_matchers(&load(&source_hooks_json()));
}

#[test]
fn source_hooks_json_registers_exactly_the_nine_events() {
    let hooks = load(&source_hooks_json());
    let events: Vec<&str> = hooks["hooks"]
        .as_object()
        .expect("hooks object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = vec![
        "Notification",
        "PostToolUse",
        "PreToolUse",
        "SessionEnd",
        "SessionStart",
        "Stop",
        "StopCancelled",
        "StopFailure",
        "UserPromptSubmit",
    ];
    let mut found = events.clone();
    found.sort_unstable();
    expected.sort_unstable();
    assert_eq!(found, expected);
    for event in events {
        for group in hooks["hooks"][event].as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                assert_eq!(handler["type"], "command");
                assert!(handler["command"].is_string());
                assert_eq!(handler["timeout"], 5);
            }
        }
    }
}
