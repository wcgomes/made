//! The plugin package contract, against the repository source and, on Unix
//! hosts, against the Windows variant generated at staging.

use std::path::{Path, PathBuf};

use serde_json::Value;

const EVENTS: [&str; 8] = [
    "CwdChanged",
    "Notification",
    "PermissionRequest",
    "SessionEnd",
    "SessionStart",
    "Stop",
    "StopFailure",
    "UserPromptSubmit",
];

fn plugin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/claude-code-made")
}

fn load(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read json")).expect("parse json")
}

fn assert_hooks(hooks: &Value, command: &str) {
    let registered = hooks["hooks"].as_object().expect("hooks object");
    let mut found: Vec<&str> = registered.keys().map(String::as_str).collect();
    found.sort_unstable();
    assert_eq!(found, EVENTS);
    for (event, groups) in registered {
        let groups = groups.as_array().unwrap();
        assert_eq!(groups.len(), 1, "{event}");
        let handlers = groups[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1, "{event}");
        assert_eq!(handlers[0]["type"], "command", "{event}");
        assert_eq!(handlers[0]["async"], true, "{event}");
        assert_eq!(handlers[0]["command"], command, "{event}");
        let matcher = groups[0].get("matcher").and_then(Value::as_str);
        if event == "Notification" {
            assert_eq!(matcher, Some("permission_prompt|idle_prompt"));
        } else {
            assert_eq!(matcher, None, "{event}");
        }
    }
}

#[test]
fn manifest_names_the_plugin_made_and_carries_metadata() {
    let manifest = load(&plugin_dir().join(".claude-plugin/plugin.json"));
    let name = manifest["name"].as_str().expect("name");
    assert_eq!(name, "made");
    assert!(!name.contains("claude"));
    for field in ["version", "description", "license"] {
        assert!(
            manifest[field]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{field}"
        );
    }
    let author = &manifest["author"];
    assert!(
        author["name"].as_str().is_some_and(|text| !text.is_empty()),
        "{author}"
    );
}

#[test]
fn readme_exists() {
    let readme = std::fs::read_to_string(plugin_dir().join("README.md")).expect("README.md");
    assert!(readme.contains("CLAUDE_CODE_PLUGIN_DIRS"));
}

#[test]
fn source_hooks_json_registers_the_eight_async_events() {
    assert_hooks(
        &load(&plugin_dir().join("hooks/hooks.json")),
        "\"${CLAUDE_PLUGIN_ROOT}/bin/made-claude-status\"",
    );
}

/// The Windows variant is generated at staging. Staging needs bash, so the
/// check runs on Unix hosts only; Windows staging is covered by the
/// packaged-layout CI step.
#[cfg(unix)]
#[test]
fn generated_windows_hooks_json_names_the_exe() {
    let root = tempfile::tempdir().unwrap();
    let hook = root.path().join("hook");
    std::fs::write(&hook, b"fixture").unwrap();
    let out = root.path().join("plugins/claude-code-made");
    let status = std::process::Command::new("bash")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.github/scripts/stage-claude-plugin.sh"),
        )
        .args(["x86_64-pc-windows-msvc"])
        .arg(&hook)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    assert_hooks(
        &load(&out.join("hooks/hooks.json")),
        "\"${CLAUDE_PLUGIN_ROOT}/bin/made-claude-status.exe\"",
    );
}
