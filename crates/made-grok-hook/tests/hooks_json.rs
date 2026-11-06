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
                assert_eq!(handler["command"], "bin/made-grok-status");
                assert_eq!(handler["timeout"], 5);
            }
        }
    }
}

/// The Windows variant is generated at staging; stage it from the repository
/// source and hold it to the same matcher contract. Staging needs bash, so the
/// check runs on Unix hosts only; Windows staging is covered by the packaged
/// layout CI step.
#[cfg(unix)]
#[test]
fn generated_windows_hooks_json_keeps_the_matchers_and_names_the_exe() {
    let root = tempfile::tempdir().unwrap();
    let hook = root.path().join("hook");
    std::fs::write(&hook, b"fixture").unwrap();
    let out = root.path().join("plugins/grok-made");
    let status = std::process::Command::new("bash")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.github/scripts/stage-grok-plugin.sh"),
        )
        .args(["x86_64-pc-windows-msvc"])
        .arg(&hook)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let generated = load(&out.join("hooks/hooks.json"));
    assert_matchers(&generated);
    let source = load(&source_hooks_json());
    assert_eq!(
        generated["hooks"].as_object().unwrap().len(),
        source["hooks"].as_object().unwrap().len()
    );
    for (event, groups) in generated["hooks"].as_object().unwrap() {
        for group in groups.as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                assert_eq!(handler["command"], "bin/made-grok-status.exe", "{event}");
                assert_eq!(handler["timeout"], 5);
            }
        }
    }
}
