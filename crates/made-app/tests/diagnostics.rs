//! Diagnostics tests: required fields, token redaction, bounded logs, copy.

use std::path::PathBuf;
use std::time::Instant;

use made_app::diagnostics::{
    DiagnosticLog, Diagnostics, LogEntry, LogLevel, WorkspaceDiagnostics, LOG_TRUNCATION_MARKER,
    MAX_LOG_ENTRIES, MAX_LOG_FILE_BYTES, MAX_LOG_MESSAGE_BYTES, MAX_LOG_TEXT_BYTES,
};
use made_app::integration_status::{IntegrationState, IntegrationStatus, ReasonCode};
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
        view_directory: None,
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
        commit_abbrev: None,
        commit_subject: None,
        is_linked_worktree: true,
        directory_name: Some("linked".to_owned()),
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
    assert_eq!(view.reason, "OpenCode session follow is ready.");
    assert_eq!(view.status.label(), "connected");
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
fn workspace_view_directory_is_sanitized_redacted_and_reported() {
    let token = token();
    let integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    let diagnostics = Diagnostics::new("/tmp/made-integration.log");
    let mut view = diagnostics.snapshot(
        &integration,
        None,
        Some(&PathBuf::from("/workspace/view")),
        None,
    );
    view.attach_workspace(WorkspaceDiagnostics {
        identity: "made-123-7".to_owned(),
        label: "made".to_owned(),
        path: Some(PathBuf::from("/workspace")),
        shell_cwd: Some(PathBuf::from("/workspace/sub")),
        cwd_supported: Some(true),
        cwd_reason: None,
        root_source: Some("opencode".to_owned()),
        watcher: Some("live".to_owned()),
        view_directory: Some(PathBuf::from(format!(
            "/workspace/view\n\u{202e}hidden-{}",
            token.expose_secret()
        ))),
        view_directory_available: Some(false),
        view_directory_reason: Some(format!("directory unavailable: {}", token.expose_secret())),
        ssh_authority: None,
        remote_root: None,
        ssh_transport: None,
        ssh_integration: None,
        remote_version: None,
        cleanup_unconfirmed: None,
    });

    let directory = view.view_directory.as_ref().expect("view directory kept");
    assert!(!directory.to_string_lossy().contains('\n'));
    assert!(!directory.to_string_lossy().contains('\u{202e}'));
    assert_eq!(view.view_directory_available, Some(false));
    let reason = view.view_directory_reason.as_deref().expect("reason kept");
    assert!(!reason.contains(token.expose_secret()));
    assert!(reason.contains("[redacted]"));

    let text = view.to_text();
    assert!(text.contains("root source: opencode"));
    assert!(text.contains("view directory: /workspace/view"));
    assert!(text.contains("view directory available: no"));
    assert!(text.contains("view directory reason: directory unavailable: [redacted]"));
    assert!(!text.contains(token.expose_secret()));
    assert!(!view.contains_secret(&token));
}

#[test]
fn tabs_unavailable_diagnostics_report_no_view_directory() {
    let integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    let diagnostics = Diagnostics::new("/tmp/made-integration.log");
    let mut view =
        diagnostics.snapshot(&integration, None, Some(&PathBuf::from("/workspace")), None);
    view.attach_workspace(WorkspaceDiagnostics {
        identity: "made-123-7".to_owned(),
        label: "made".to_owned(),
        path: Some(PathBuf::from("/workspace")),
        shell_cwd: Some(PathBuf::from("/workspace/sub")),
        cwd_supported: Some(true),
        cwd_reason: None,
        root_source: Some("shell".to_owned()),
        watcher: Some("live".to_owned()),
        view_directory: None,
        view_directory_available: None,
        view_directory_reason: None,
        ssh_authority: None,
        remote_root: None,
        ssh_transport: None,
        ssh_integration: None,
        remote_version: None,
        cleanup_unconfirmed: None,
    });

    assert_eq!(view.view_directory, None);
    let text = view.to_text();
    assert!(text.contains("tabs enabled: no"));
    assert!(text.contains("view directory: none"));
    assert!(!text.contains("view directory available:"));
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
        view_directory: None,
        view_directory_available: None,
        view_directory_reason: None,
        ssh_authority: None,
        remote_root: None,
        ssh_transport: None,
        ssh_integration: None,
        remote_version: None,
        cleanup_unconfirmed: None,
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

fn stored_and_rendered(message: &str) -> (String, String) {
    let mut diagnostics = Diagnostics::new("/tmp/made/truncation.log");
    diagnostics
        .log_mut()
        .push(LogEntry::new(LogLevel::Info, message));
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);
    let stored = view.entries.last().expect("stored entry").message.clone();
    let text = view.to_text();
    let rendered = text
        .lines()
        .find(|line| line.starts_with("  #1 [info] "))
        .expect("rendered entry")
        .to_owned();
    (stored, rendered)
}

#[test]
fn message_truncation_boundaries_are_utf8_safe_for_stored_and_rendered_text() {
    let exact = "x".repeat(MAX_LOG_MESSAGE_BYTES);
    let (stored, rendered) = stored_and_rendered(&exact);
    assert_eq!(stored.len(), MAX_LOG_MESSAGE_BYTES);
    assert!(!stored.contains(LOG_TRUNCATION_MARKER));
    assert_eq!(rendered.len(), 12 + stored.len());

    let over = "x".repeat(MAX_LOG_MESSAGE_BYTES + 1);
    let (stored, rendered) = stored_and_rendered(&over);
    assert_eq!(stored.len(), MAX_LOG_MESSAGE_BYTES);
    assert!(stored.ends_with(LOG_TRUNCATION_MARKER), "{stored:?}");
    assert_eq!(rendered.len(), 12 + stored.len());

    let three_byte = "€".repeat(4098 / 3 + 1);
    let (stored, rendered) = stored_and_rendered(&three_byte);
    assert_eq!(
        stored.len(),
        4095,
        "3-byte code point must back up to a boundary"
    );
    let keep = stored
        .strip_suffix(LOG_TRUNCATION_MARKER)
        .expect("truncation marker");
    assert!(keep.chars().all(|character| character == '€'));
    assert!(std::str::from_utf8(stored.as_bytes()).is_ok());
    assert_eq!(rendered.len(), 12 + stored.len());

    let four_byte = format!("a{}", "😀".repeat(1025));
    let (stored, rendered) = stored_and_rendered(&four_byte);
    assert_eq!(
        stored.len(),
        4093,
        "4-byte code point must back up to a boundary"
    );
    let keep = stored
        .strip_suffix(LOG_TRUNCATION_MARKER)
        .expect("truncation marker");
    assert!(keep.starts_with('a'));
    assert!(keep[1..].chars().all(|character| character == '😀'));
    assert!(!stored.contains('\u{FFFD}'));
    assert_eq!(rendered.len(), 12 + stored.len());
}

#[test]
fn a_worst_case_log_stays_below_the_file_threshold_and_keeps_newest() {
    let mut diagnostics = Diagnostics::new("/tmp/made/worst-case.log");
    for index in 0..MAX_LOG_ENTRIES {
        diagnostics.log_mut().push(LogEntry::new(
            LogLevel::Info,
            format!("{index:04}{}", "x".repeat(MAX_LOG_MESSAGE_BYTES)),
        ));
    }
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);
    let text = view.to_text();

    assert!(
        text.len() < MAX_LOG_FILE_BYTES as usize,
        "worst-case snapshot is {} bytes",
        text.len()
    );
    assert!(
        text.len() <= MAX_LOG_TEXT_BYTES + 64 * 1024,
        "log text exceeded its budget: {} bytes",
        text.len()
    );
    assert!(
        view.entries.len() < MAX_LOG_ENTRIES,
        "entries were not evicted"
    );
    let newest = view.entries.last().expect("newest entry");
    assert!(newest.message.starts_with("0499"), "{}", newest.message);
    assert!(
        view.entries.iter().all(|entry| entry.sequence > 1),
        "oldest entries must be evicted first"
    );
}

#[test]
fn snapshot_redacts_hex_session_ids_for_ui() {
    let secret = "0123456789abcdef".repeat(4);
    let mut diagnostics = Diagnostics::new("/tmp/made/session-redaction.log");
    diagnostics
        .log_mut()
        .push(LogEntry::new(LogLevel::Info, "session event").with_session(&secret));
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);
    let session = view.entries[0].session_id.as_deref().expect("session id");
    assert!(session.contains("[redacted]"), "{session}");
    assert!(!session.contains(&secret), "{session}");
    assert!(!view.to_text().contains(&secret));
}

#[test]
fn previous_log_path_exists_and_is_copyable() {
    let temp = tempfile::tempdir().unwrap();
    let active = temp.path().join("integration.log");
    let previous = temp.path().join("integration.log.1");
    let diagnostics = Diagnostics::new(&active);

    assert_eq!(diagnostics.previous_log_location(), previous.as_path());
    assert!(!diagnostics.previous_log_exists());
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);
    assert_eq!(
        view.previous_log_location.as_deref(),
        Some(previous.to_string_lossy().as_ref())
    );
    assert!(!view.previous_log_exists);

    std::fs::write(&previous, b"previous record\n").unwrap();
    assert!(diagnostics.previous_log_exists());
    let view = diagnostics.snapshot(&integration, None, None, None);
    assert!(view.previous_log_exists);
    assert!(view.to_text().contains("previous log location"));

    let mut clipboard = InMemoryClipboard::new();
    diagnostics.copy_previous_log_path(&mut clipboard).unwrap();
    assert_eq!(
        clipboard.get_text().unwrap(),
        previous.to_string_lossy().as_ref()
    );
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
    assert_eq!(
        view.previous_log_location.as_deref(),
        Some("/tmp/made-integration.log.1")
    );
    assert!(!view.previous_log_exists);
    assert!(!view.contains_secret(&token));
}

#[test]
fn unavailable_diagnostics_have_no_previous_log_state() {
    let diagnostics = Diagnostics::unavailable("app paths unavailable");
    let integration = IntegrationState::new(None, "/workspace");
    let view = diagnostics.snapshot(&integration, None, None, None);
    assert_eq!(view.previous_log_location, None);
    assert!(!view.previous_log_exists);
    assert!(view
        .to_text()
        .contains("unavailable: app paths unavailable"));
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

fn opencode_reason_sentence(code: ReasonCode) -> &'static str {
    match code {
        ReasonCode::Inactive => "OpenCode is not running.",
        ReasonCode::Connected | ReasonCode::Reconnected => "OpenCode session follow is ready.",
        ReasonCode::OpenCodeMissing => "OpenCode is not on PATH.",
        ReasonCode::PluginNotLoaded => {
            "The OpenCode v2 integration plugin did not load; restart OpenCode in the Made terminal."
        }
        ReasonCode::OpenCodeTooOld => {
            "OpenCode v2 or newer is required; upgrade OpenCode and restart it in the terminal."
        }
        ReasonCode::ExtensionLoadingDisabled => {
            "OpenCode started with extension loading disabled; enable plugins for this launch."
        }
        ReasonCode::ConfigCompositionFailed => {
            "Made could not compose its per-launch OpenCode configuration; your files were not changed."
        }
        ReasonCode::ProtocolMismatch => {
            "The Made plugin protocol is incompatible with this app; update Made or the plugin."
        }
        ReasonCode::AuthenticationRejected => {
            "The integration connection was rejected; restart OpenCode in the Made terminal."
        }
        ReasonCode::BridgeClosed => {
            "The integration connection closed; waiting to reconnect while the terminal stays usable."
        }
        ReasonCode::HeartbeatTimeout => {
            "The integration stopped responding; waiting to reconnect while the terminal stays usable."
        }
        ReasonCode::OpenCodeExited => "OpenCode exited; the shell remains usable.",
        ReasonCode::TabsUnavailable => {
            "OpenCode session tabs are disabled; Made requires tabs to follow sessions."
        }
        ReasonCode::ActiveDirectoryUnavailable => {
            "The active session directory is unavailable; showing the last known folder."
        }
        ReasonCode::WorktreeMetadataUnavailable => {
            "Git metadata could not be read; showing the folder without branch information."
        }
        ReasonCode::FocusFailed => {
            "The session could not be focused in OpenCode; the current workspace was kept."
        }
        ReasonCode::FocusTimedOut => "Focusing the session timed out; the current workspace was kept.",
    }
}

fn assert_copied_opencode_reason(
    integration: &IntegrationState,
    sentence: &str,
    status: &str,
    reason_code: &str,
) {
    let view =
        Diagnostics::new("/tmp/made-integration.log").snapshot(integration, None, None, None);
    assert_eq!(view.status_label, status);
    assert_eq!(view.status.label(), status, "machine status code changed");
    assert_eq!(view.reason_code, reason_code);
    assert_eq!(view.reason, sentence);
    let text = view.to_text();
    assert!(
        text.contains(&format!("status: {status}")),
        "copied status changed: {text}"
    );
    assert!(
        text.contains(&format!("reason code: {reason_code}")),
        "copied reason code changed: {text}"
    );
    assert!(
        text.contains(&format!("reason: {sentence}")),
        "copied reason missing {sentence:?}: {text}"
    );
    assert!(
        text.contains("grok: Grok Build is not on PATH."),
        "Grok sentence changed: {text}"
    );
    for stale in [
        "Optional OpenCode integration is inactive.",
        "Connected to the OpenCode v2 integration.",
        "Reconnected to the OpenCode v2 integration.",
        "OpenCode v2 is not installed or not on PATH; install OpenCode v2 and run it in the terminal.",
    ] {
        assert!(!text.contains(stale), "copied stale sentence {stale:?}: {text}");
    }
}

#[test]
fn opencode_reason_sentences_follow_the_grok_wording() {
    for code in ReasonCode::ALL {
        assert_eq!(
            code.message(),
            opencode_reason_sentence(code),
            "reason sentence changed for {}",
            code.code()
        );
    }
    assert_eq!(IntegrationStatus::Connected.label(), "connected");
    assert_eq!(IntegrationStatus::Inactive.label(), "inactive");
    assert_eq!(IntegrationStatus::Degraded.label(), "degraded");
    assert_eq!(IntegrationStatus::Unsupported.label(), "unsupported");

    let workspace = PathBuf::from("/workspace");
    let mut integration = IntegrationState::new(Some(workspace.clone()), workspace);
    assert_copied_opencode_reason(
        &integration,
        "OpenCode is not running.",
        "inactive",
        "inactive",
    );

    integration.apply_bridge_event(&BridgeEvent::Connected(connection()));
    integration.apply_bridge_event(&BridgeEvent::State(snapshot()));
    assert_eq!(integration.status().label(), "connected");
    assert_copied_opencode_reason(
        &integration,
        "OpenCode session follow is ready.",
        "connected",
        "connected",
    );

    integration.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 12,
        authenticated: true,
        code: Some(1006),
        reason: "dropped".to_owned(),
        replaced: false,
    }));
    integration.apply_bridge_event(&BridgeEvent::Connected(connection()));
    assert_eq!(integration.reason().code, ReasonCode::Reconnected);
    assert_copied_opencode_reason(
        &integration,
        "OpenCode session follow is ready.",
        "connected",
        "reconnected",
    );

    integration.note_opencode_missing();
    assert_eq!(integration.status(), IntegrationStatus::Unsupported);
    assert_copied_opencode_reason(
        &integration,
        "OpenCode is not on PATH.",
        "unsupported",
        "opencode_missing",
    );
}

/// The owning remote tab's OpenCode integration state, reported instead of
/// the host-local integration so SSH transport loss stays distinguishable
/// from remote integration loss.
#[test]
fn remote_opencode_diagnostics_use_the_owning_remote_state_honestly() {
    use made_app::diagnostics::remote_opencode_report;
    use made_app::remote_workspace::{ConnectionState, IntegrationState as RemoteIntegration};
    use made_remote::protocol::ErrorCode;

    // Before the SSH session attaches, the remote integration has not been
    // observed; no host sentence may stand in for it.
    let report = remote_opencode_report(
        false,
        None,
        ConnectionState::Disconnected,
        RemoteIntegration::Unknown,
        None,
    );
    assert_eq!(report.status, IntegrationStatus::Inactive);
    assert_eq!(report.code, "inactive");
    assert!(
        report.reason.contains("remote"),
        "the reason names the remote integration: {}",
        report.reason
    );

    // A healthy transport with a connected remote integration.
    let report = remote_opencode_report(
        true,
        None,
        ConnectionState::Connected,
        RemoteIntegration::Connected,
        None,
    );
    assert_eq!(report.status, IntegrationStatus::Connected);
    assert_eq!(report.code, "connected");
    assert_eq!(report.reason, "OpenCode session follow is ready.");

    // The remote integration failed while the transport stayed healthy: the
    // reason blames the integration, not the transport.
    let report = remote_opencode_report(
        true,
        None,
        ConnectionState::Connected,
        RemoteIntegration::Unavailable,
        Some(ErrorCode::Unavailable),
    );
    assert_eq!(report.status, IntegrationStatus::Degraded);
    assert_eq!(report.code, "unavailable");
    assert!(
        report
            .reason
            .contains("remote OpenCode integration is unavailable"),
        "the reason blames the integration: {}",
        report.reason
    );
    assert!(
        !report.reason.contains("transport"),
        "an integration failure must not claim a transport failure: {}",
        report.reason
    );

    // The transport dropped while attached: the reason blames the transport
    // and never claims the integration closed.
    let report = remote_opencode_report(
        true,
        None,
        ConnectionState::Disconnected,
        RemoteIntegration::Unknown,
        None,
    );
    assert_eq!(report.status, IntegrationStatus::Degraded);
    assert!(
        report.reason.contains("SSH transport"),
        "the transport reason stays honest: {}",
        report.reason
    );

    // A failed tab reports its own failure sentence.
    let report = remote_opencode_report(
        false,
        Some("SSH disconnected. Queued input was discarded and will not be replayed."),
        ConnectionState::Disconnected,
        RemoteIntegration::Unknown,
        None,
    );
    assert_eq!(report.status, IntegrationStatus::Degraded);
    assert_eq!(
        report.reason,
        "SSH disconnected. Queued input was discarded and will not be replayed."
    );

    // Applying the report replaces the host-local status in the snapshot and
    // the copied report, with the owning tab's SSH rows still present.
    let diagnostics = Diagnostics::new("/home/user/.local/state/made/log/integration.log")
        .with_app_version("9.9.9");
    let mut view = diagnostics.snapshot(
        &IntegrationState::new(None, PathBuf::from("/workspace")),
        None,
        None,
        None,
    );
    assert_eq!(view.reason, "OpenCode is not running.");
    view.attach_workspace(WorkspaceDiagnostics {
        identity: "pane".to_owned(),
        label: "Remote".to_owned(),
        ssh_authority: Some("server-a".to_owned()),
        remote_root: Some("/project".to_owned()),
        ssh_transport: Some("disconnected".to_owned()),
        ssh_integration: Some("unknown".to_owned()),
        ..WorkspaceDiagnostics::default()
    });
    view.set_remote_opencode(report);
    assert_eq!(view.status, IntegrationStatus::Degraded);
    assert_eq!(view.status_label, "degraded");
    assert!(
        view.reason.contains("SSH disconnected"),
        "the snapshot carries the owning tab's reason: {}",
        view.reason
    );
    let text = view.to_text();
    assert!(text.contains("status: degraded"), "{text}");
    assert!(text.contains("ssh transport: disconnected"), "{text}");
    assert!(text.contains("ssh integration: unknown"), "{text}");
}

#[test]
fn remote_grok_rows_follow_the_owning_tab_tier_honestly() {
    use made_app::diagnostics::{remote_grok_row, RemoteGrokInstallOutcome};

    // Before the SSH session attaches, nothing remote may be claimed.
    let row = remote_grok_row(false, false, None, true, None, false, true);
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Inactive);
    assert!(
        row.sentence.contains("not known yet"),
        "an unattached tab claims no Grok state: {}",
        row.sentence
    );

    // A failed tab (transport loss) outranks every Grok claim, even with a
    // pre-disconnect running projection: no stale claim may survive.
    let row = remote_grok_row(
        true,
        false,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Succeeded),
        true,
        true,
    );
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Degraded);
    assert!(
        row.sentence.contains("SSH transport is disconnected"),
        "the transport failure must name the transport: {}",
        row.sentence
    );
    assert!(
        !row.sentence.contains("ready"),
        "a transport failure must not claim readiness: {}",
        row.sentence
    );
    // The same transport failure with nothing running stays transport-shaped.
    let row = remote_grok_row(true, false, None, true, None, false, false);
    assert!(
        row.sentence.contains("SSH transport is disconnected"),
        "{}",
        row.sentence
    );

    // Grok was not detected by this tab's own probe.
    let row = remote_grok_row(false, true, None, false, None, false, true);
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Inactive);
    assert!(
        row.sentence.contains("not detected on this server"),
        "{}",
        row.sentence
    );

    // The bounded install attempt is busy while it runs.
    let row = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Running),
        true,
        false,
    );
    assert!(
        row.sentence.contains("Installing the Grok Build plugin"),
        "{}",
        row.sentence
    );

    // Nothing running: the honest row is not running, whatever the tier.
    for install in [
        None,
        Some(RemoteGrokInstallOutcome::Succeeded),
        Some(RemoteGrokInstallOutcome::Failed { reason: None }),
    ] {
        let row = remote_grok_row(false, true, None, true, install, true, false);
        assert!(
            row.sentence.contains("Grok Build is not running."),
            "install {install:?} with nothing open: {}",
            row.sentence
        );
    }

    // Inventory tier without consent: the reason says the install was not
    // approved, and never claims hook readiness.
    let row = remote_grok_row(false, true, None, true, None, false, true);
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Degraded);
    assert!(
        row.sentence.contains("hooks are unavailable"),
        "{}",
        row.sentence
    );
    assert!(
        row.sentence.contains("not approved"),
        "the inventory reason names the missing approval: {}",
        row.sentence
    );

    // Consent granted but the install has not run in this session.
    let row = remote_grok_row(false, true, None, true, None, true, true);
    assert!(
        row.sentence.contains("has not run on this server yet"),
        "{}",
        row.sentence
    );

    // A failed install keeps the inventory tier and names the failure.
    let row = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Failed {
            reason: Some("install-failed"),
        }),
        true,
        true,
    );
    assert!(
        row.sentence.contains("the plugin install failed"),
        "{}",
        row.sentence
    );

    // No remote Grok state mentions python3: a reason the companion no longer
    // reports degrades to the generic failed install row, and every state
    // names no interpreter.
    for install in [
        None,
        Some(RemoteGrokInstallOutcome::Running),
        Some(RemoteGrokInstallOutcome::Succeeded),
        Some(RemoteGrokInstallOutcome::Failed { reason: None }),
        Some(RemoteGrokInstallOutcome::Failed {
            reason: Some("install-failed"),
        }),
    ] {
        for consented in [false, true] {
            for (detected, running) in [(false, false), (true, false), (true, true)] {
                let row = remote_grok_row(false, detected, None, consented, install, running, true);
                assert!(
                    !row.sentence.to_lowercase().contains("python"),
                    "{install:?}: {}",
                    row.sentence
                );
            }
        }
    }

    // A successful install is ready while a session is open; it stays ready
    // even after a withdrawal, because Made never uninstalls.
    let row = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Succeeded),
        false,
        true,
    );
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Ready);
    assert_eq!(row.sentence, "Grok Build session follow is ready.");
}

#[test]
fn remote_grok_row_reaches_the_snapshot_and_copied_report() {
    use made_app::diagnostics::{remote_grok_row, RemoteGrokInstallOutcome};

    let diagnostics = Diagnostics::new("/home/user/.local/state/made/log/integration.log")
        .with_app_version("9.9.9");
    let mut view = diagnostics.snapshot(
        &IntegrationState::new(None, PathBuf::from("/workspace")),
        None,
        None,
        None,
    );
    // The host row starts as the local one; a remote tab replaces it.
    view.attach_workspace(WorkspaceDiagnostics {
        identity: "pane".to_owned(),
        label: "Remote".to_owned(),
        ssh_authority: Some("server-a".to_owned()),
        remote_root: Some("/project".to_owned()),
        ..WorkspaceDiagnostics::default()
    });
    assert_eq!(
        view.row("grok status").unwrap().sentence,
        "Grok Build is not on PATH.",
        "the pre-existing host sentence is what a remote tab must replace"
    );

    let report = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Failed {
            reason: Some("install-failed"),
        }),
        true,
        true,
    );
    view.set_remote_grok(report.clone());
    assert_eq!(view.row("grok status").unwrap().pill, report.pill);
    assert_eq!(view.row("grok status").unwrap().sentence, report.sentence);
    let text = view.to_text();
    assert!(
        text.contains("grok status: degraded"),
        "the copied report must include the Grok pill: {text}"
    );
    assert!(
        text.contains("the plugin install failed") && !text.contains("python3"),
        "the copied report must include the honest reason and no interpreter: {text}"
    );
    assert!(
        !text.contains("Grok Build is not on PATH"),
        "the host sentence must not survive the remote row: {text}"
    );
}

#[test]
fn remote_grok_failure_names_the_actual_failure_not_generic_transport() {
    use made_app::diagnostics::remote_grok_row;

    // A vanished grok is an opening failure with its own sentence. It must not
    // be flattened into the generic transport row.
    let detail = "Grok was not detected on this server. Nothing ran locally.";
    let row = remote_grok_row(true, false, Some(detail), false, None, false, false);
    assert_eq!(row.pill, made_app::diagnostics::DiagnosticsPill::Degraded);
    assert!(
        row.sentence
            .contains("Grok was not detected on this server"),
        "the row must keep the tab's failure detail: {}",
        row.sentence
    );
    assert!(
        !row.sentence.contains("SSH transport is disconnected"),
        "an opening failure must not claim transport loss: {}",
        row.sentence
    );

    // Stage and companion failures use the same distinction.
    let detail = "A packaged companion file is missing. Made did not start a local shell.";
    let row = remote_grok_row(true, false, Some(detail), true, None, false, false);
    assert!(
        row.sentence
            .contains("A packaged companion file is missing"),
        "{}",
        row.sentence
    );
    assert!(!row.sentence.contains("SSH transport is disconnected"));

    // A later transport loss with no opening failure is still transport.
    let row = remote_grok_row(true, false, None, true, None, false, false);
    assert!(
        row.sentence.contains("SSH transport is disconnected"),
        "{}",
        row.sentence
    );

    // The copyable report carries the same detail rather than the generic one.
    let diagnostics = Diagnostics::new("/home/user/.local/state/made/log/integration.log")
        .with_app_version("9.9.9");
    let mut view = diagnostics.snapshot(
        &IntegrationState::new(None, PathBuf::from("/workspace")),
        None,
        None,
        None,
    );
    view.set_remote_grok(row);
    let text = view.to_text();
    assert!(
        text.contains("SSH transport is disconnected"),
        "a real transport loss still reaches the copy: {text}"
    );

    let row = remote_grok_row(true, false, Some(detail), true, None, false, false);
    view.set_remote_grok(row);
    let text = view.to_text();
    assert!(
        text.contains("A packaged companion file is missing"),
        "the copy must carry the actual stage failure: {text}"
    );
    assert!(
        !text.contains("SSH transport is disconnected"),
        "the copy must not replace the stage failure with transport: {text}"
    );
}

#[test]
fn update_outcomes_reach_the_bounded_redacted_log() {
    use made_app::updates::UpdateEvent;
    let mut diagnostics = Diagnostics::new("/home/alice/.local/state/made/log/integration.log");
    let events = [
        UpdateEvent::KeptFound("0.3.0".into()),
        UpdateEvent::CheckStarted { manual: false },
        UpdateEvent::CheckStarted { manual: true },
        UpdateEvent::UpToDate,
        UpdateEvent::Available("0.3.0".into()),
        UpdateEvent::CheckFailed,
        UpdateEvent::Ready("0.3.0".into()),
        UpdateEvent::DownloadFailed("0.3.0".into()),
        UpdateEvent::VerificationFailed("0.3.0".into()),
        UpdateEvent::Busy,
        UpdateEvent::RestartRequested("0.3.0".into()),
        UpdateEvent::RestartFailed("0.3.0".into()),
        // A version string carrying a path, user name or token never reaches the log.
        UpdateEvent::Available(format!("/home/alice/{}", "0123456789abcdef".repeat(4))),
    ];
    for event in &events {
        diagnostics.record_update(event);
    }
    let text: Vec<String> = diagnostics
        .log()
        .entries()
        .map(|entry| entry.message.clone())
        .collect();
    assert_eq!(text.len(), events.len());
    for expected in [
        "update: kept verified update 0.3.0 found",
        "update: automatic check started",
        "update: manual check started",
        "update: up to date",
        "update: check failed: the update service could not be reached",
        "update: version 0.3.0 downloaded and verified",
        "update: download of version 0.3.0 failed",
        "update: version 0.3.0 failed verification and was discarded",
        "update: restart into version 0.3.0 confirmed",
        "update: version unknown available, downloading",
    ] {
        assert!(
            text.iter().any(|line| line == expected),
            "missing {expected:?} in {text:?}"
        );
    }
    for line in &text {
        assert!(
            !line.contains("alice") && !line.contains('/') && !line.contains("0123456789abcdef"),
            "{line}"
        );
    }
    for _ in 0..(MAX_LOG_ENTRIES + 50) {
        diagnostics.record_update(&UpdateEvent::CheckFailed);
    }
    assert!(
        diagnostics.log().len() <= MAX_LOG_ENTRIES,
        "the log stays bounded"
    );
}

#[test]
fn the_last_command_line_apply_is_reported_redacted() {
    let mut diagnostics = Diagnostics::new("/home/alice/.local/state/made/log/integration.log");
    let integration = IntegrationState::new(
        Some(PathBuf::from("/workspace")),
        PathBuf::from("/workspace"),
    );
    let text = diagnostics
        .snapshot(&integration, None, None, None)
        .to_text();
    assert!(text.contains("last command-line apply: none"), "{text}");
    diagnostics.set_update_apply_outcome(Some("installed Made 0.3.0".to_owned()));
    let view = diagnostics.snapshot(&integration, None, None, None);
    assert_eq!(view.update_apply.as_deref(), Some("installed Made 0.3.0"));
    assert!(view
        .to_text()
        .contains("last command-line apply: installed Made 0.3.0"));
    for hostile in [
        "installed Made /home/alice/0.3.0".to_owned(),
        format!("installed {}", "0123456789abcdef".repeat(8)),
        "installed Made 0.3.0\nforged: line".to_owned(),
    ] {
        diagnostics.set_update_apply_outcome(Some(hostile.clone()));
        let text = diagnostics
            .snapshot(&integration, None, None, None)
            .to_text();
        assert!(
            text.contains("last command-line apply: none"),
            "{hostile:?} must be dropped: {text}"
        );
    }
}

// ---- Claude Code diagnostics row ------------------------------------------------

mod claude_row {
    use std::path::PathBuf;
    use std::time::Duration;

    use made_app::diagnostics::{Diagnostics, DiagnosticsPill};
    use made_app::integration_status::{
        claude_diagnostics_row, ClaudeDiagnosticsInputs, ClaudeDiagnosticsPhase, IntegrationState,
        CLAUDE_GRACE,
    };

    /// A present Claude Code with a plugin, a live session, an observed
    /// protocol, and the grace elapsed: the ready state.
    fn ready() -> ClaudeDiagnosticsInputs {
        ClaudeDiagnosticsInputs {
            phase: ClaudeDiagnosticsPhase::Available,
            plugin_missing: false,
            present: true,
            present_for: CLAUDE_GRACE + Duration::from_secs(1),
            live_session: true,
            protocol_observed: true,
        }
    }

    fn row(inputs: ClaudeDiagnosticsInputs) -> (DiagnosticsPill, String) {
        let report = claude_diagnostics_row(&inputs);
        (report.pill, report.sentence)
    }

    #[test]
    fn grace_is_about_ten_seconds() {
        assert_eq!(CLAUDE_GRACE, Duration::from_secs(10));
    }

    #[test]
    fn checking_is_not_shown_as_not_on_path() {
        let inputs = ClaudeDiagnosticsInputs {
            phase: ClaudeDiagnosticsPhase::Checking,
            ..ready()
        };
        assert_eq!(
            row(inputs),
            (
                DiagnosticsPill::Inactive,
                "Checking for Claude Code.".to_owned()
            )
        );
        let inputs = ClaudeDiagnosticsInputs {
            phase: ClaudeDiagnosticsPhase::NotOnPath,
            ..ready()
        };
        assert_eq!(
            row(inputs),
            (
                DiagnosticsPill::Inactive,
                "Claude Code is not on PATH.".to_owned()
            )
        );
    }

    #[test]
    fn a_missing_plugin_is_degraded_before_presence_is_considered() {
        for present in [false, true] {
            let inputs = ClaudeDiagnosticsInputs {
                plugin_missing: true,
                present,
                ..ready()
            };
            assert_eq!(
                row(inputs),
                (
                    DiagnosticsPill::Degraded,
                    "Claude Code session-status hooks are unavailable.".to_owned()
                )
            );
        }
        // Detection states outrank a missing plugin.
        let inputs = ClaudeDiagnosticsInputs {
            phase: ClaudeDiagnosticsPhase::NotOnPath,
            plugin_missing: true,
            ..ready()
        };
        assert_eq!(row(inputs).1, "Claude Code is not on PATH.");
    }

    #[test]
    fn not_running_covers_another_workspace_and_opencode_owning_the_label() {
        // Presence is evaluated for this workspace only: another workspace's
        // Claude Code arrives here as `present: false`.
        let inputs = ClaudeDiagnosticsInputs {
            present: false,
            live_session: false,
            protocol_observed: false,
            ..ready()
        };
        assert_eq!(
            row(inputs),
            (
                DiagnosticsPill::Inactive,
                "Claude Code is not running.".to_owned()
            )
        );
        // Presence alone decides; nothing about the tab label is an input, so
        // OpenCode owning it cannot hide a present Claude Code.
        assert_eq!(row(ready()).1, "Claude Code session follow is ready.");
    }

    #[test]
    fn starting_within_the_grace_without_a_session_or_an_observed_protocol() {
        for (live_session, protocol_observed) in [(false, false), (false, true), (true, false)] {
            let inputs = ClaudeDiagnosticsInputs {
                present_for: Duration::from_secs(3),
                live_session,
                protocol_observed,
                ..ready()
            };
            assert_eq!(
                row(inputs),
                (
                    DiagnosticsPill::Inactive,
                    "Claude Code is starting.".to_owned()
                ),
                "session {live_session}, observed {protocol_observed}"
            );
        }
        // Both channels already working inside the grace is ready, not starting.
        let inputs = ClaudeDiagnosticsInputs {
            present_for: Duration::from_secs(3),
            ..ready()
        };
        assert_eq!(row(inputs).1, "Claude Code session follow is ready.");
    }

    #[test]
    fn hooks_unavailable_after_the_grace_without_a_live_session() {
        for protocol_observed in [false, true] {
            let inputs = ClaudeDiagnosticsInputs {
                live_session: false,
                protocol_observed,
                ..ready()
            };
            assert_eq!(
                row(inputs),
                (
                    DiagnosticsPill::Degraded,
                    "Claude Code session-status hooks are unavailable.".to_owned()
                )
            );
        }
    }

    #[test]
    fn a_probe_alone_is_not_observed_so_the_protocol_is_unavailable_after_the_grace() {
        // `protocol_observed` is true only for a session-level app=claude-code
        // emission; the ClaudeTab tests cover that the probe does not set it.
        let inputs = ClaudeDiagnosticsInputs {
            protocol_observed: false,
            ..ready()
        };
        assert_eq!(
            row(inputs),
            (
                DiagnosticsPill::Degraded,
                "Claude Code status protocol is unavailable.".to_owned()
            )
        );
    }

    #[test]
    fn ready_needs_a_live_session_and_an_observed_protocol() {
        assert_eq!(
            row(ready()),
            (
                DiagnosticsPill::Ready,
                "Claude Code session follow is ready.".to_owned()
            )
        );
    }

    #[test]
    fn an_explicit_catalog_without_detection_uses_the_plugin_and_running_states() {
        // `Available` is what an explicit catalog that lists Claude Code uses
        // without running detection.
        let inputs = ClaudeDiagnosticsInputs {
            present: false,
            ..ready()
        };
        assert_eq!(row(inputs).1, "Claude Code is not running.");
        let inputs = ClaudeDiagnosticsInputs {
            plugin_missing: true,
            ..ready()
        };
        assert_eq!(
            row(inputs).1,
            "Claude Code session-status hooks are unavailable."
        );
        let inputs = ClaudeDiagnosticsInputs {
            present_for: Duration::from_secs(1),
            live_session: false,
            ..ready()
        };
        assert_eq!(row(inputs).1, "Claude Code is starting.");
        assert_eq!(
            row(ClaudeDiagnosticsInputs {
                live_session: true,
                protocol_observed: false,
                ..ready()
            })
            .1,
            "Claude Code status protocol is unavailable."
        );
    }

    #[test]
    fn the_snapshot_and_copied_report_include_the_claude_sentence_in_every_state() {
        let diagnostics = Diagnostics::new("/home/user/.local/state/made/log/integration.log")
            .with_app_version("9.9.9");
        let mut view = diagnostics.snapshot(
            &IntegrationState::new(None, PathBuf::from("/workspace")),
            None,
            None,
            None,
        );
        assert_eq!(
            view.row("claude status").unwrap().sentence,
            "Claude Code is not on PATH.",
            "the default is the not-on-PATH row"
        );
        for inputs in [
            ready(),
            ClaudeDiagnosticsInputs {
                phase: ClaudeDiagnosticsPhase::Checking,
                ..ready()
            },
            ClaudeDiagnosticsInputs {
                plugin_missing: true,
                ..ready()
            },
            ClaudeDiagnosticsInputs {
                present: false,
                ..ready()
            },
        ] {
            let report = claude_diagnostics_row(&inputs);
            view.set_row("claude status", report.clone());
            assert_eq!(view.row("claude status").unwrap().pill, report.pill);
            assert_eq!(view.row("claude status").unwrap().sentence, report.sentence);
            let text = view.to_text();
            assert!(
                text.contains(&format!("claude status: {}", report.pill.label())),
                "{text}"
            );
            assert!(
                text.contains(&format!("claude: {}", report.sentence)),
                "{text}"
            );
            assert!(text.contains("grok: Grok Build is not on PATH."), "{text}");
            assert!(
                text.contains("reason: "),
                "the OpenCode sentence stays: {text}"
            );
        }
    }
}

// ---- Characterization: copied report and rows (agent-integration-modules phase 0) ----

/// Byte-for-byte golden text of the copied report for fixed snapshots, and the
/// Grok Build and Claude Code rows before and after detection, written against
/// the code before the integration-module refactor. The expected text below
/// does not change during the refactor; only the adapter block does.
mod copied_report_characterization {
    use std::path::PathBuf;
    use std::time::Duration;

    use made_app::diagnostics::{Diagnostics, DiagnosticsSnapshot};
    use made_app::integration_status::{IntegrationState, CLAUDE_GRACE};

    // BEGIN characterization adapter
    use made_app::app::{AppModel, AppOptions, GrokDiagnosticsHold, GrokRunningHold};
    use made_app::diagnostics::{remote_grok_row, RemoteGrokInstallOutcome};
    use made_app::integration_status::{
        claude_diagnostics_row, remote_claude_diagnostics_row, ClaudeDiagnosticsInputs,
        ClaudeDiagnosticsPhase,
    };
    use made_app::integrations::{CLAUDE_CODE, GROK, OPENCODE};

    /// Apply the local Claude Code row for `inputs`.
    fn apply_claude(view: &mut DiagnosticsSnapshot, inputs: &ClaudeDiagnosticsInputs) {
        view.set_row("claude status", claude_diagnostics_row(inputs));
    }

    /// Apply the remote Claude Code row for `inputs`.
    fn apply_remote_claude(view: &mut DiagnosticsSnapshot, inputs: &ClaudeDiagnosticsInputs) {
        view.set_row("claude status", remote_claude_diagnostics_row(inputs));
    }

    /// Apply a remote Grok Build row.
    fn apply_remote_grok(view: &mut DiagnosticsSnapshot, case: RemoteGrok) {
        let report = match case {
            RemoteGrok::InstallFailed => remote_grok_row(
                false,
                true,
                None,
                true,
                Some(RemoteGrokInstallOutcome::Failed {
                    reason: Some("install-failed"),
                }),
                true,
                true,
            ),
            RemoteGrok::NotDetected => {
                remote_grok_row(false, true, None, false, None, false, false)
            }
            RemoteGrok::TransportDown => {
                remote_grok_row(true, false, None, true, None, true, false)
            }
        };
        view.set_remote_grok(report);
    }

    /// Copy both secondary rows the open workspace derived into `view`.
    fn copy_rows(view: &mut DiagnosticsSnapshot, model: &AppModel) {
        let derived = model
            .workbench()
            .expect("a workspace is open")
            .diagnostics_snapshot();
        view.rows = derived.rows;
    }

    /// Model options for a state of detection: pending, or finished with an
    /// explicit list of detected programs.
    fn options(state: Detection) -> AppOptions {
        match state {
            Detection::Pending => AppOptions::new().with_compatible_tui_detection(),
            Detection::Nothing => AppOptions::new().with_compatible_tuis([]),
            Detection::All => AppOptions::new().with_compatible_tuis([OPENCODE, GROK, CLAUDE_CODE]),
        }
    }

    fn hold_grok(model: &mut AppModel, hold: Hold) {
        model.hold_grok_diagnostics(match hold {
            Hold::Detecting => GrokDiagnosticsHold::Detecting,
            Hold::NotOnPath => GrokDiagnosticsHold::NotOnPath,
            Hold::Installing => GrokDiagnosticsHold::Installing,
            Hold::Installed => GrokDiagnosticsHold::Installed,
            Hold::AlreadyCurrent => GrokDiagnosticsHold::AlreadyCurrent,
            Hold::InstallFailed => GrokDiagnosticsHold::InstallFailed,
        });
    }

    fn hold_grok_running(model: &mut AppModel, running: Mark) {
        let id = model.active_workspace_id().expect("a workspace is open");
        model.hold_workspace_grok_running(
            id,
            match running {
                Mark::LivingTarget => GrokRunningHold::LivingTarget,
                Mark::OpenSession => GrokRunningHold::OpenSession,
                Mark::NotRunning => GrokRunningHold::NotRunning,
            },
        );
    }

    #[derive(Clone, Copy)]
    enum RemoteGrok {
        InstallFailed,
        NotDetected,
        TransportDown,
    }

    #[derive(Clone, Copy)]
    enum Detection {
        Pending,
        Nothing,
        All,
    }

    #[derive(Clone, Copy)]
    enum Hold {
        Detecting,
        NotOnPath,
        Installing,
        Installed,
        AlreadyCurrent,
        InstallFailed,
    }

    #[derive(Clone, Copy)]
    enum Mark {
        LivingTarget,
        OpenSession,
        NotRunning,
    }

    type Phase = ClaudeDiagnosticsPhase;
    // END characterization adapter

    fn fixed() -> DiagnosticsSnapshot {
        Diagnostics::new("/home/user/.local/state/made/log/integration.log")
            .with_app_version("9.9.9")
            .snapshot(
                &IntegrationState::new(None, PathBuf::from("/workspace")),
                None,
                None,
                None,
            )
    }

    fn inputs(phase: Phase) -> ClaudeDiagnosticsInputs {
        ClaudeDiagnosticsInputs {
            phase,
            plugin_missing: false,
            present: true,
            present_for: CLAUDE_GRACE + Duration::from_secs(1),
            live_session: true,
            protocol_observed: true,
        }
    }

    /// The copied report around the two secondary rows.
    fn report(grok: (&str, &str), claude: (&str, &str)) -> String {
        format!(
            "{PREFIX}grok status: {}\ngrok: {}\nclaude status: {}\nclaude: {}\n{SUFFIX}",
            grok.0, grok.1, claude.0, claude.1
        )
    }

    const PREFIX: &str = "Made diagnostics\n\
status: inactive\n\
reason code: inactive\n\
reason: OpenCode is not running.\n";
    const SUFFIX: &str = "view directory: none\n\
active session: none\n\
active title: none\n\
effective root: none\n\
worktree: none\n\
plugin version: none\n\
protocol version: none\n\
opencode version: none\n\
app version: 9.9.9\n\
last command-line apply: none\n\
last revision: none\n\
last heartbeat: none\n\
tabs enabled: no\n\
log location: /home/user/.local/state/made/log/integration.log\n\
previous log location: /home/user/.local/state/made/log/integration.log.1\n\
recent log:\n";

    const GROK_NOT_ON_PATH: (&str, &str) = ("inactive", "Grok Build is not on PATH.");
    const CLAUDE_NOT_ON_PATH: (&str, &str) = ("inactive", "Claude Code is not on PATH.");
    /// No shipped Claude Code plugin is present beside the child executable.
    const CLAUDE_HOOKS_UNAVAILABLE: (&str, &str) = (
        "degraded",
        "Claude Code session-status hooks are unavailable.",
    );

    #[test]
    fn rows_before_detection_are_the_not_on_path_rows_in_the_copied_report() {
        let view = fixed();
        assert_eq!(
            view.row("grok status").unwrap().pill.label(),
            GROK_NOT_ON_PATH.0
        );
        assert_eq!(
            view.row("grok status").unwrap().sentence,
            GROK_NOT_ON_PATH.1
        );
        assert_eq!(
            view.row("claude status").unwrap().pill.label(),
            CLAUDE_NOT_ON_PATH.0
        );
        assert_eq!(
            view.row("claude status").unwrap().sentence,
            CLAUDE_NOT_ON_PATH.1
        );
        assert_eq!(view.to_text(), report(GROK_NOT_ON_PATH, CLAUDE_NOT_ON_PATH));
    }

    #[test]
    fn local_claude_rows_in_the_copied_report() {
        let cases: [(&str, ClaudeDiagnosticsInputs, (&str, &str)); 8] = [
            (
                "checking",
                inputs(Phase::Checking),
                ("inactive", "Checking for Claude Code."),
            ),
            ("not on path", inputs(Phase::NotOnPath), CLAUDE_NOT_ON_PATH),
            (
                "not running",
                ClaudeDiagnosticsInputs {
                    present: false,
                    ..inputs(Phase::Available)
                },
                ("inactive", "Claude Code is not running."),
            ),
            (
                "starting",
                ClaudeDiagnosticsInputs {
                    present_for: Duration::from_secs(3),
                    live_session: false,
                    ..inputs(Phase::Available)
                },
                ("inactive", "Claude Code is starting."),
            ),
            (
                "plugin missing",
                ClaudeDiagnosticsInputs {
                    plugin_missing: true,
                    ..inputs(Phase::Available)
                },
                (
                    "degraded",
                    "Claude Code session-status hooks are unavailable.",
                ),
            ),
            (
                "no live session",
                ClaudeDiagnosticsInputs {
                    live_session: false,
                    ..inputs(Phase::Available)
                },
                (
                    "degraded",
                    "Claude Code session-status hooks are unavailable.",
                ),
            ),
            (
                "protocol unavailable",
                ClaudeDiagnosticsInputs {
                    protocol_observed: false,
                    ..inputs(Phase::Available)
                },
                ("degraded", "Claude Code status protocol is unavailable."),
            ),
            (
                "ready",
                inputs(Phase::Available),
                ("ready", "Claude Code session follow is ready."),
            ),
        ];
        for (name, claude_inputs, claude) in cases {
            let mut view = fixed();
            apply_claude(&mut view, &claude_inputs);
            assert_eq!(
                view.to_text(),
                report(GROK_NOT_ON_PATH, claude),
                "local Claude Code row: {name}"
            );
        }
    }

    #[test]
    fn remote_claude_rows_in_the_copied_report() {
        let cases: [(&str, ClaudeDiagnosticsInputs, (&str, &str)); 6] = [
            (
                "checking",
                inputs(Phase::Checking),
                ("inactive", "Checking for Claude Code."),
            ),
            ("not on path", inputs(Phase::NotOnPath), CLAUDE_NOT_ON_PATH),
            (
                "not running",
                ClaudeDiagnosticsInputs {
                    present: false,
                    ..inputs(Phase::Available)
                },
                ("inactive", "Claude Code is not running."),
            ),
            (
                "starting",
                ClaudeDiagnosticsInputs {
                    present_for: Duration::from_secs(3),
                    protocol_observed: false,
                    ..inputs(Phase::Available)
                },
                ("inactive", "Claude Code is starting."),
            ),
            (
                "protocol unavailable",
                ClaudeDiagnosticsInputs {
                    protocol_observed: false,
                    ..inputs(Phase::Available)
                },
                ("degraded", "Claude Code status protocol is unavailable."),
            ),
            (
                "hooks unavailable over ssh",
                inputs(Phase::Available),
                (
                    "degraded",
                    "Claude Code session-status hooks are unavailable over SSH.",
                ),
            ),
        ];
        for (name, claude_inputs, claude) in cases {
            let mut view = fixed();
            apply_remote_claude(&mut view, &claude_inputs);
            assert_eq!(
                view.to_text(),
                report(GROK_NOT_ON_PATH, claude),
                "remote Claude Code row: {name}"
            );
        }
    }

    #[test]
    fn remote_grok_rows_in_the_copied_report() {
        let cases: [(&str, RemoteGrok, (&str, &str)); 3] = [
            (
                "install failed",
                RemoteGrok::InstallFailed,
                (
                    "degraded",
                    "Remote Grok Build session-status hooks are unavailable: the plugin install failed on this server.",
                ),
            ),
            (
                "not detected",
                RemoteGrok::NotDetected,
                ("inactive", "Grok Build is not detected on this server."),
            ),
            (
                "transport down",
                RemoteGrok::TransportDown,
                (
                    "degraded",
                    "The SSH transport is disconnected; the remote Grok Build state is unknown until reattach.",
                ),
            ),
        ];
        for (name, case, grok) in cases {
            let mut view = fixed();
            apply_remote_grok(&mut view, case);
            assert_eq!(
                view.to_text(),
                report(grok, CLAUDE_NOT_ON_PATH),
                "remote Grok Build row: {name}"
            );
        }
    }

    /// Re-run `name` in a child process with an isolated home. Returns `true`
    /// in the parent after the child passed.
    #[cfg(unix)]
    fn isolated(name: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("MADE_DIAGNOSTICS_CHILD").is_some() {
            return false;
        }
        let home = tempfile::tempdir().unwrap();
        let tmp = home.path().join("tmp");
        let runtime = home.path().join("runtime");
        let roaming = home.path().join("AppData").join("Roaming");
        let local = home.path().join("AppData").join("Local");
        for directory in [&tmp, &runtime, &roaming, &local] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        // An install root of its own, so no shipped plugin staged beside the
        // cargo `deps` directory by another test is visible to the child.
        let install = home.path().join("install");
        std::fs::create_dir_all(&install).unwrap();
        let executable = install.join("diagnostics-characterization");
        let current = std::env::current_exe().unwrap();
        if std::fs::hard_link(&current, &executable).is_err() {
            std::fs::copy(&current, &executable).unwrap();
        }
        let output = std::process::Command::new(&executable)
            .args(["--exact", name, "--nocapture"])
            .env("MADE_DIAGNOSTICS_CHILD", "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("TMPDIR", &tmp)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("APPDATA", &roaming)
            .env("LOCALAPPDATA", &local)
            .env("SHELL", "/bin/bash")
            .env_remove(made_bridge::config::CONFIG_CONTENT_ENV)
            .env_remove(made_platform::SHELL_ENV)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

    /// A model with one open workspace for the given detection state.
    #[cfg(unix)]
    fn model_with_workspace(state: Detection) -> (AppModel, tempfile::TempDir) {
        use made_app::workspace::{LaunchPlan, WorkspaceLaunch, WorkspaceSource};
        let root = tempfile::tempdir().unwrap();
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), options(state));
        model.open_plan(LaunchPlan::open(root.path(), WorkspaceSource::Picker).unwrap());
        model.ensure_workbench();
        (model, root)
    }

    /// The copied report of the open workspace's two secondary rows.
    #[cfg(unix)]
    fn model_report(model: &AppModel) -> String {
        let mut view = fixed();
        copy_rows(&mut view, model);
        view.to_text()
    }

    #[cfg(unix)]
    #[test]
    fn rows_before_and_after_detection_in_the_copied_report() {
        if isolated(
            "copied_report_characterization::rows_before_and_after_detection_in_the_copied_report",
        ) {
            return;
        }
        let mut out = String::new();
        let mut check = |name: &str, model: &AppModel, grok: (&str, &str), claude: (&str, &str)| {
            let actual = model_report(model);
            let expected = report(grok, claude);
            if actual != expected {
                out.push_str(&format!("{name}:\n{actual}\n"));
            }
        };

        // Before detection finishes: both rows are checking.
        let (model, _root) = model_with_workspace(Detection::Pending);
        check(
            "pending",
            &model,
            ("inactive", "Checking for Grok Build."),
            ("inactive", "Checking for Claude Code."),
        );
        drop(model);

        // After detection found nothing.
        let (model, _root) = model_with_workspace(Detection::Nothing);
        check("nothing", &model, GROK_NOT_ON_PATH, CLAUDE_NOT_ON_PATH);
        drop(model);

        // After detection found everything.
        let (mut model, _root) = model_with_workspace(Detection::All);
        check(
            "all",
            &model,
            (
                "inactive",
                "Grok Build plugin was not installed for this launch.",
            ),
            CLAUDE_HOOKS_UNAVAILABLE,
        );
        let not_running = ("inactive", "Grok Build is not running.");
        for (name, hold, grok) in [
            (
                "detecting",
                Hold::Detecting,
                ("inactive", "Checking for Grok Build."),
            ),
            ("not on path", Hold::NotOnPath, GROK_NOT_ON_PATH),
            (
                "installing",
                Hold::Installing,
                ("inactive", "Installing the Grok Build plugin."),
            ),
            ("installed", Hold::Installed, not_running),
            ("already current", Hold::AlreadyCurrent, not_running),
            (
                "install failed",
                Hold::InstallFailed,
                (
                    "degraded",
                    "Grok Build session-status hooks are unavailable.",
                ),
            ),
        ] {
            hold_grok(&mut model, hold);
            check(
                &format!("hold {name}"),
                &model,
                grok,
                CLAUDE_HOOKS_UNAVAILABLE,
            );
        }
        // An install is ready only while Grok Build is running in the workspace.
        let ready = ("ready", "Grok Build session follow is ready.");
        for hold in [Hold::Installed, Hold::AlreadyCurrent] {
            hold_grok(&mut model, hold);
            for (name, running, grok) in [
                ("living target", Mark::LivingTarget, ready),
                ("open session", Mark::OpenSession, ready),
                ("not running", Mark::NotRunning, not_running),
            ] {
                hold_grok_running(&mut model, running);
                check(
                    &format!("running {name}"),
                    &model,
                    grok,
                    CLAUDE_HOOKS_UNAVAILABLE,
                );
            }
        }
        assert_eq!(out, "");
    }
}
