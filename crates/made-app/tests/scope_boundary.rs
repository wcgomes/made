//! V1 scope-boundary tests: OpenCode v2, Grok Build and Claude Code only, no chat UI, no
//! worktree management, and no separate server requirement.

use std::path::{Path, PathBuf};

use made_app::integration_status::V1_SCOPE;
use made_app::integrations::{CLAUDE_CODE, GROK, OPENCODE, PRODUCT};
use made_app::worktree::{
    is_read_only_git_args, GitCommand, READ_ONLY_GIT_ARGUMENTS, WORKTREE_MANAGEMENT_AVAILABLE,
};

const WORKTREE_SOURCE: &str = include_str!("../src/worktree.rs");
const CORE_SOURCES: &[&str] = &[
    include_str!("../src/workspace.rs"),
    include_str!("../src/workspace_picker.rs"),
    include_str!("../src/session_follow.rs"),
    include_str!("../src/session_map.rs"),
    include_str!("../src/session_focus.rs"),
    include_str!("../src/worktree.rs"),
    include_str!("../src/integration_status.rs"),
    include_str!("../src/diagnostics.rs"),
    include_str!("../src/metrics.rs"),
    include_str!("../src/runtime_guard.rs"),
];

/// Agents and surfaces that must stay out of the core and the integration
/// status code. Claude Code is an offered row, so it is not listed.
const FORBIDDEN_SURFACES: &[&str] = &["codex", "gemini", "copilot", "aider", "acp", "chat ui"];

#[test]
fn only_opencode_grok_and_claude_code_are_offered() {
    let rows: Vec<_> = V1_SCOPE
        .agent_integrations
        .iter()
        .map(|row| (row.descriptor().scope_id, row.descriptor().scope_name))
        .collect();
    let product: Vec<_> = PRODUCT
        .iter()
        .map(|row| (row.descriptor().scope_id, row.descriptor().scope_name))
        .collect();
    assert_eq!(rows, product);
    assert_eq!(V1_SCOPE.agent_integrations, PRODUCT);
    assert_eq!(
        rows,
        [
            ("opencode-v2", "OpenCode v2"),
            ("grok-build", "Grok Build"),
            ("claude-code", "Claude Code"),
        ]
    );
    assert_eq!(V1_SCOPE.agent_integrations[0], OPENCODE);
    assert_eq!(V1_SCOPE.agent_integrations[1], GROK);
    assert_eq!(V1_SCOPE.agent_integrations[2], CLAUDE_CODE);
}

/// Every `.rs` file under `dir`, read recursively, as (path, text).
fn read_sources(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            read_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
            out.push((path, text));
        }
    }
}

/// The sources of the integration modules, with a guard that the walk found
/// them: a scan over moved code must not pass because it read nothing.
fn integration_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/integrations");
    let mut sources = Vec::new();
    read_sources(&root, &mut sources);
    let modules = sources
        .iter()
        .filter(|(path, _)| {
            path.file_name().is_some_and(|name| name == "mod.rs")
                && path.parent().is_some_and(|parent| parent != root)
        })
        .count();
    assert!(
        modules >= 3,
        "expected at least three integration modules, read {modules}"
    );
    sources
}

#[test]
fn there_is_no_chat_ui_or_other_agent_integration() {
    const { assert!(!V1_SCOPE.chat_ui) };
    for source in CORE_SOURCES {
        let lower = source.to_ascii_lowercase();
        for forbidden in FORBIDDEN_SURFACES {
            assert!(
                !lower.contains(forbidden),
                "core source mentions forbidden agent/chat surface {forbidden:?}"
            );
        }
    }
    let sources = integration_sources();
    for (path, source) in &sources {
        let lower = source.to_ascii_lowercase();
        for forbidden in FORBIDDEN_SURFACES {
            assert!(
                !lower.contains(forbidden),
                "{} mentions forbidden agent/chat surface {forbidden:?}",
                path.display()
            );
        }
    }
    let integration = sources
        .iter()
        .map(|(_, source)| source.to_ascii_lowercase())
        .collect::<String>();
    for offered in ["grok build", "claude code"] {
        assert!(
            integration.contains(offered),
            "{offered} is an offered integration, not a forbidden mention"
        );
    }
}

#[test]
fn there_is_no_worktree_management_surface() {
    const { assert!(!V1_SCOPE.worktree_management) };
    const { assert!(!WORKTREE_MANAGEMENT_AVAILABLE) };

    for command in GitCommand::ALL {
        assert!(
            is_read_only_git_args(command.args()),
            "{} is not read-only",
            command.label()
        );
    }
    for command in READ_ONLY_GIT_ARGUMENTS {
        let joined = command.join(" ");
        assert!(!joined.contains(" add "));
        assert!(!joined.contains(" remove "));
        assert!(!joined.contains(" merge "));
        assert!(!joined.contains(" prune "));
    }

    let lower = WORKTREE_SOURCE.to_ascii_lowercase();
    for forbidden in [
        "worktree add",
        "worktree remove",
        "worktree merge",
        "worktree prune",
        "create_worktree",
        "remove_worktree",
        "merge_worktree",
        "prune_worktree",
    ] {
        assert!(
            !lower.contains(forbidden),
            "worktree module exposes mutating surface {forbidden:?}"
        );
    }
}

#[test]
fn no_core_module_exposes_a_worktree_lifecycle_function() {
    for source in CORE_SOURCES {
        for line in source.lines() {
            let line = line.trim();
            if !line.starts_with("pub fn ") && !line.starts_with("pub async fn ") {
                continue;
            }
            let index = if line.starts_with("pub async fn ") {
                3
            } else {
                2
            };
            let name = line
                .split_whitespace()
                .nth(index)
                .unwrap_or_default()
                .trim_end_matches('(');
            for forbidden in [
                "create_worktree",
                "remove_worktree",
                "merge_worktree",
                "prune_worktree",
                "add_worktree",
            ] {
                assert!(
                    !name.contains(forbidden),
                    "core module exposes mutating function {name}"
                );
            }
        }
    }
}

#[test]
fn no_separate_opencode_server_is_required() {
    const { assert!(!V1_SCOPE.separate_opencode_server) };
    for source in CORE_SOURCES {
        let lower = source.to_ascii_lowercase();
        assert!(!lower.contains("serve --stdio"));
        assert!(!lower.contains("command::new(\"opencode\")"));
        assert!(!lower.contains("command::new(\"opencode.exe\")"));
        assert!(!lower.contains("\"serve\","));
    }
}

#[test]
fn the_core_is_headless_and_ui_agnostic() {
    for source in CORE_SOURCES {
        for forbidden in ["egui::", "eframe::", "wgpu::", "winit::"] {
            assert!(
                !source.contains(forbidden),
                "a core module imports the UI/windowing dependency {forbidden}"
            );
        }
    }
}

#[test]
fn the_app_core_has_no_acp_or_multi_agent_protocol_types() {
    for source in CORE_SOURCES {
        let lower = source.to_ascii_lowercase();
        assert!(!lower.contains("acp::"));
        assert!(!lower.contains("agentprotocol"));
        assert!(!lower.contains("agent_protocol"));
    }
}
