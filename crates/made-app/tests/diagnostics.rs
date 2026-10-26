//! Diagnostics tests: required fields, token redaction, bounded logs, copy.

use std::path::PathBuf;
use std::time::Instant;

use made_app::diagnostics::{
    DiagnosticLog, Diagnostics, LogEntry, LogLevel, WorkspaceDiagnostics, MAX_LOG_ENTRIES,
};
use made_app::integration_status::IntegrationState;
use made_app::session_follow::FollowedSession;
use made_app::worktree::{MetadataStatus, WorktreeMetadata};
use made_bridge::security::Token;
use made_bridge::server::{BridgeEvent, ConnectionInfo, DisconnectedInfo};
use made_bridge::state::{SessionInfo, SessionSnapshot};
use made_platform::{Clipboard, InMemoryClipboard};

fn token() -> Token {
    Token::from_hex("0123456789abcdef".repeat(4)).unwrap()
}

fn connection() -> ConnectionInfo {
    ConnectionInfo {
        connection_id: 12,
        pane_id: "pane".to_owned(),
        plugin_version: "0.1.0".to_owned(),
        opencode_version: "2.1.4".to_owned(),
        pid: 1,
        cwd: PathBuf::from("/workspace"),
        protocol_version: 1,
    }
}

fn snapshot() -> SessionSnapshot {
    SessionSnapshot {
        revision: 7,
        active_session_id: Some("ses_active".to_owned()),
        tabs_enabled: true,
        sessions: vec![SessionInfo {
            session_id: "ses_active".to_owned(),
            title: "Investigate the parser".to_owned(),
            active: true,
            busy: true,
            attention: true,
            unread: true,
            prompt_pulse: 2,
            renaming: false,
            directory: Some(PathBuf::from("/workspace/linked")),
            subpath: Some("src".to_owned()),
            project_id: None,
        }],
    }
}

fn active() -> FollowedSession {
    FollowedSession {
        session_id: "ses_active".to_owned(),
        title: "Investigate the parser".to_owned(),
        requested_directory: Some(PathBuf::from("/workspace/linked")),
        canonical_directory: Some(PathBuf::from("/workspace/linked")),
        subpath: Some("src".to_owned()),
        available: true,
    }
}

fn worktree() -> WorktreeMetadata {
    WorktreeMetadata {
        status: MetadataStatus::Known,
        is_worktree: true,
        toplevel: Some(PathBuf::from("/workspace/linked")),
        branch: Some("feature".to_owned()),
        detached: false,
        is_linked_worktree: true,
        worktree_label: Some("feature".to_owned()),
        git_dir: Some(PathBuf::from("/workspace/main/.git/worktrees/linked")),
        common_dir: Some(PathBuf::from("/workspace/main/.git")),
        worktree_count: 2,
        reason: None,
    }
}

#[test]
fn connected_diagnostics_expose_every_required_field() {
    let mut integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    integration.apply_bridge_event(&BridgeEvent::Connected(connection()));
    integration.apply_bridge_event(&BridgeEvent::State(snapshot()));
    integration.note_root("/workspace/linked");

    let mut diagnostics = Diagnostics::new("/home/user/.local/state/made/log/integration.log")
        .with_app_version("9.9.9")
        .with_token_fingerprint("sha256:0011223344556677");
    diagnostics.log_mut().push(
        LogEntry::new(LogLevel::Info, "connected to plugin")
            .with_reason("connected")
            .with_connection(12),
    );

    let view = diagnostics.snapshot(
        &integration,
        Some(&active()),
        Some(&PathBuf::from("/workspace/linked")),
        Some(&worktree()),
    );
    assert_eq!(view.status_label, "connected");
    assert_eq!(view.reason_code, "connected");
    assert!(view.reason.contains("Connected"));
    assert_eq!(view.active_session_id.as_deref(), Some("ses_active"));
    assert_eq!(
        view.active_session_title.as_deref(),
        Some("Investigate the parser")
    );
    assert_eq!(view.active_subpath.as_deref(), Some("src"));
    assert_eq!(view.active_root, Some(PathBuf::from("/workspace/linked")));
    assert_eq!(view.worktree_status, Some(MetadataStatus::Known));
    assert_eq!(view.worktree_branch.as_deref(), Some("feature"));
    assert!(view.worktree_is_linked);
    assert_eq!(view.plugin_version.as_deref(), Some("0.1.0"));
    assert_eq!(view.protocol_version, Some(1));
    assert_eq!(view.opencode_version.as_deref(), Some("2.1.4"));
    assert_eq!(view.app_version, "9.9.9");
    assert_eq!(view.last_revision, Some(7));
    assert!(view.last_heartbeat.is_some());
    assert_eq!(
        view.log_location,
        "/home/user/.local/state/made/log/integration.log"
    );
    assert_eq!(
        view.token_fingerprint.as_deref(),
        Some("sha256:0011223344556677")
    );
    assert_eq!(view.entries.len(), 1);

    let text = view.to_text();
    for expected in [
        "connected",
        "ses_active",
        "Investigate the parser",
        "/workspace/linked",
        "feature",
        "0.1.0",
        "2.1.4",
        "9.9.9",
        "7",
        "/home/user/.local/state/made/log/integration.log",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
}

#[test]
fn workspace_shell_context_is_inspectable_and_redacted() {
    let token = token();
    let integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    let diagnostics = Diagnostics::new("/tmp/made-integration.log");
    let mut view =
        diagnostics.snapshot(&integration, None, Some(&PathBuf::from("/workspace")), None);
    assert_eq!(view.status_label, "inactive");
    view.attach_workspace(WorkspaceDiagnostics {
        identity: "made-123-7".to_owned(),
        label: "made".to_owned(),
        path: Some(PathBuf::from("/workspace")),
        shell_cwd: Some(PathBuf::from("/workspace/sub")),
        cwd_supported: Some(false),
        cwd_reason: Some(format!(
            "cmd has no demonstrated safe cwd hook (token {})",
            token.expose_secret()
        )),
        root_source: Some("shell".to_owned()),
        watcher: Some("live".to_owned()),
    });

    assert_eq!(view.workspace_identity.as_deref(), Some("made-123-7"));
    assert_eq!(view.workspace_label.as_deref(), Some("made"));
    assert_eq!(view.workspace_path, Some(PathBuf::from("/workspace")));
    assert_eq!(view.shell_cwd, Some(PathBuf::from("/workspace/sub")));
    assert_eq!(view.cwd_supported, Some(false));
    assert_eq!(view.root_source.as_deref(), Some("shell"));

    let text = view.to_text();
    for expected in [
        "workspace identity: made-123-7",
        "workspace label: made",
        "shell cwd: /workspace/sub",
        "cwd reporting: unsupported",
        "cwd reason: cmd has no demonstrated safe cwd hook",
        "root source: shell",
        "watcher: live",
        "effective root: /workspace",
        "status: inactive",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert!(!text.contains(token.expose_secret()));
    assert!(text.contains("[redacted]"));
    assert!(!view.contains_secret(&token));
    assert!(!format!("{:?}", view).contains(token.expose_secret()));
}

#[test]
fn logs_redact_the_bridge_token_and_never_render_it() {
    let token = token();
    let mut integration = IntegrationState::new(None, "/workspace");
    integration.apply_bridge_event(&BridgeEvent::Connected(connection()));
    integration.apply_bridge_event(&BridgeEvent::State(snapshot()));

    let mut diagnostics = Diagnostics::new("/tmp/made-integration.log");
    diagnostics.log_mut().record_redacted(
        LogLevel::Warn,
        format!("handshake failed with token {}", token.expose_secret()),
        &token,
    );
    diagnostics.log_mut().record_redacted(
        LogLevel::Error,
        format!("bad hello token={}", token.expose_secret()),
        &token,
    );

    let view = diagnostics.snapshot(&integration, Some(&active()), None, None);
    let text = view.to_text();
    assert!(!text.contains(token.expose_secret()));
    assert!(text.contains("[redacted]"));
    assert!(!view.contains_secret(&token));
    assert!(!format!("{:?}", view).contains(token.expose_secret()));
    assert!(!format!("{:?}", diagnostics.log()).contains(token.expose_secret()));
}

#[test]
fn redact_all_scrubs_raw_entries_recorded_before_the_token_was_known() {
    let token = token();
    let mut log = DiagnosticLog::new();
    log.push(LogEntry::new(
        LogLevel::Warn,
        format!("secret={} before rotation", token.expose_secret()),
    ));
    assert!(log
        .entries()
        .any(|entry| entry.message.contains(token.expose_secret())));
    log.redact_all(&token);
    assert!(log
        .entries()
        .all(|entry| !entry.message.contains(token.expose_secret())));
}

#[test]
fn record_automatically_redacts_token_shaped_hex_runs() {
    let token = token();
    let mut log = DiagnosticLog::new();
    log.record(
        LogLevel::Warn,
        format!("configured token={} unexpectedly", token.expose_secret()),
    );
    let message = &log.entries().next().unwrap().message;
    assert!(!message.contains(token.expose_secret()));
    assert!(message.contains("[redacted]"));
    assert!(message.contains("configured token="));
}

#[test]
fn short_hex_identifiers_are_not_redacted() {
    let mut log = DiagnosticLog::new();
    log.record(LogLevel::Info, "connection 6d0dd448 revision 12");
    assert_eq!(
        log.entries().next().unwrap().message,
        "connection 6d0dd448 revision 12"
    );
}

#[test]
fn the_snapshot_redacts_raw_structured_entries_too() {
    let token = token();
    let mut diagnostics = Diagnostics::new("/tmp/made-integration.log");
    diagnostics.log_mut().push(LogEntry::new(
        LogLevel::Error,
        format!("raw structured secret {}", token.expose_secret()),
    ));
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);

    assert!(!view.to_text().contains(token.expose_secret()));
    assert!(!format!("{:?}", view).contains(token.expose_secret()));
}

#[test]
fn log_revision_tracks_appends_and_clear_for_ui_caching() {
    let mut log = DiagnosticLog::new();
    let start = log.revision();
    log.record(LogLevel::Info, "first");
    assert!(log.revision() > start);
    let after_record = log.revision();
    log.clear();
    assert!(log.revision() > after_record);
}

#[test]
fn the_log_is_bounded_and_keeps_the_newest_entries() {
    let mut log = DiagnosticLog::with_capacity(3);
    for index in 0..5 {
        log.push(LogEntry::new(LogLevel::Info, format!("entry {index}")));
    }
    assert_eq!(log.capacity(), 3);
    assert_eq!(log.len(), 3);
    let entries: Vec<&LogEntry> = log.entries().collect();
    assert_eq!(entries[0].sequence, 3);
    assert_eq!(entries[2].sequence, 5);
    assert_eq!(entries[2].message, "entry 4");
    assert_eq!(MAX_LOG_ENTRIES, 500);
}

#[test]
fn degraded_diagnostics_show_reason_and_last_known_root() {
    let token = token();
    let mut integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    integration.apply_bridge_event(&BridgeEvent::Connected(connection()));
    integration.apply_bridge_event(&BridgeEvent::State(snapshot()));
    integration.note_root("/workspace/linked");
    integration.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 12,
        authenticated: true,
        code: Some(1006),
        reason: "socket closed".to_owned(),
        replaced: false,
    }));

    let diagnostics = Diagnostics::new("/tmp/made-integration.log");
    let view = diagnostics.snapshot(&integration, None, None, None);
    assert_eq!(view.status_label, "degraded");
    assert_eq!(view.reason_code, "bridge_closed");
    assert!(view.reason.contains("connection closed"));
    assert_eq!(view.active_root, Some(PathBuf::from("/workspace/linked")));
    assert!(!view.contains_secret(&token));
}

#[test]
fn unsupported_diagnostics_are_actionable() {
    let mut integration = IntegrationState::new(None, "/workspace");
    integration.note_opencode_version("1.2.3");
    let view = Diagnostics::new("/tmp/log").snapshot(&integration, None, None, None);
    assert_eq!(view.status_label, "unsupported");
    assert_eq!(view.reason_code, "opencode_too_old");
    assert!(view.reason.contains("upgrade OpenCode"));
}

#[test]
fn the_log_path_is_copyable() {
    let diagnostics = Diagnostics::new("/tmp/made/integration.log");
    let mut clipboard = InMemoryClipboard::new();
    diagnostics.copy_log_path(&mut clipboard).unwrap();
    assert_eq!(clipboard.get_text().unwrap(), "/tmp/made/integration.log");
    assert_eq!(diagnostics.copyable_log_path(), "/tmp/made/integration.log");
}

#[test]
fn snapshot_heartbeat_age_uses_the_supplied_instant() {
    let mut integration = IntegrationState::new(None, "/workspace");
    let earlier = Instant::now();
    integration.note_heartbeat(earlier);
    let diagnostics = Diagnostics::new("/tmp/log");
    let view = diagnostics.snapshot_at(&integration, None, None, None, earlier);
    assert_eq!(view.last_heartbeat, Some(std::time::Duration::ZERO));
}
