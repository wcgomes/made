//! Display/log sanitization and copy-path redaction tests.

use std::time::Duration;

use made_app::diagnostics::{DiagnosticLog, Diagnostics, DiagnosticsSnapshot, LogEntry, LogLevel};
use made_app::integration_status::IntegrationStatus;
use made_app::sanitize::{sanitize_copy_text, sanitize_display, sanitize_log_text};
use made_bridge::security::Token;
use made_platform::{Clipboard, InMemoryClipboard};

const HOSTILE: &str = "safe\u{1b}]0;owned\u{7}\nline\u{202e}reversed\u{200b}zero\u{feff}end";

#[test]
fn display_and_log_sanitizers_strip_escape_newline_and_bidi_controls() {
    for text in [sanitize_display(HOSTILE, 400), sanitize_log_text(HOSTILE)] {
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
        assert!(!text.contains('\n'));
        assert!(!text.contains('\u{202e}'));
        assert!(!text.contains('\u{200b}'));
        assert!(!text.contains('\u{feff}'));
        assert!(text.contains("safe"));
        assert!(text.contains("line"));
    }
}

#[test]
fn every_log_entry_is_sanitized_centrally() {
    let mut log = DiagnosticLog::new();
    log.record(LogLevel::Warn, format!("plugin closed: {HOSTILE}"));
    let entry = log.entries().next().unwrap();
    assert!(!entry.message.contains('\u{1b}'));
    assert!(!entry.message.contains('\n'));
    assert!(!entry.message.contains('\u{202e}'));
    assert!(entry.message.contains("plugin closed"));
}

#[test]
fn diagnostics_snapshot_and_copy_path_are_sanitized() {
    let hostile_path = format!("/tmp/log\u{202e}\n{HOSTILE}.log");
    let diagnostics = Diagnostics::new(&hostile_path);
    let snapshot = DiagnosticsSnapshot {
        status: IntegrationStatus::Degraded,
        status_label: "degraded".to_owned(),
        reason_code: "bridge_closed",
        reason: sanitize_log_text(&format!("close: {HOSTILE}")),
        workspace_identity: None,
        workspace_label: None,
        workspace_path: None,
        shell_cwd: None,
        cwd_supported: None,
        cwd_reason: None,
        root_source: None,
        watcher: None,
        active_session_id: Some("session\n\u{202e}id".to_owned()),
        active_session_title: Some(sanitize_display(&format!("title {HOSTILE}"), 200)),
        active_subpath: None,
        active_root: None,
        worktree_status: None,
        worktree_branch: None,
        worktree_label: None,
        worktree_is_linked: false,
        worktree_reason: None,
        plugin_version: Some("0.1.0\n\u{1b}[31m".to_owned()),
        opencode_version: Some("2.1.4\u{202e}".to_owned()),
        protocol_version: Some(1),
        app_version: "0.1.0".to_owned(),
        last_revision: Some(3),
        last_heartbeat: Some(Duration::from_millis(10)),
        tabs_enabled: true,
        log_location: hostile_path,
        token_fingerprint: Some("sha256:0123456789abcdef".to_owned()),
        entries: vec![LogEntry::new(
            LogLevel::Warn,
            sanitize_log_text(&format!("closed: {HOSTILE}")),
        )],
    };

    let text = snapshot.to_text();
    for forbidden in ['\u{1b}', '\u{7}', '\u{202e}', '\u{200b}', '\u{feff}'] {
        assert!(!text.contains(forbidden), "to_text leaked {forbidden:?}");
    }
    assert!(!text.contains("plugin closed\u{1b}"));

    let mut clipboard = InMemoryClipboard::new();
    diagnostics.copy_log_path(&mut clipboard).unwrap();
    let copied = clipboard.get_text().unwrap();
    assert!(!copied.contains('\u{1b}'));
    assert!(!copied.contains('\u{202e}'));
    assert!(!copied.contains('\n'));
}

#[test]
fn workspace_and_session_titles_with_hostile_text_are_safe_for_labels() {
    let workspace = sanitize_display(&format!("workspace {HOSTILE}"), 200);
    let session = sanitize_display(&format!("session {HOSTILE}"), 200);
    for text in [workspace, session] {
        assert!(!text.chars().any(char::is_control));
        assert!(!text.contains('\u{202e}'));
        assert!(!text.contains('\u{200b}'));
    }
}

#[test]
fn copyable_log_path_redacts_token_shaped_hex_runs() {
    let secret = "a".repeat(64);
    let diagnostics = Diagnostics::new(format!("/tmp/{secret}/integration.log"));
    let copied = diagnostics.copyable_log_path();
    assert!(!copied.contains(&secret));
    assert!(copied.contains("[redacted]"));
    let mut clipboard = InMemoryClipboard::new();
    diagnostics.copy_log_path(&mut clipboard).unwrap();
    assert!(!clipboard.get_text().unwrap().contains(&secret));
}

#[test]
fn copy_path_redacts_the_bridge_token_after_sanitizing() {
    let token = Token::from_bytes([3; 32]);
    let secret = token.expose_secret();
    let path = format!("/logs/{secret}\u{202e}\nintegration.log");
    let copied = sanitize_copy_text(&path, Some(&token), 4096);
    assert!(!copied.contains(secret));
    assert!(copied.contains("[redacted]"));
    assert!(!copied.contains('\u{202e}'));
    assert!(!copied.contains('\n'));
}
