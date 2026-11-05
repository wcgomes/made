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
    assert_eq!(
        row.pill,
        made_app::diagnostics::GrokDiagnosticsPill::Inactive
    );
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
    assert_eq!(
        row.pill,
        made_app::diagnostics::GrokDiagnosticsPill::Degraded
    );
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
    assert_eq!(
        row.pill,
        made_app::diagnostics::GrokDiagnosticsPill::Inactive
    );
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
    assert_eq!(
        row.pill,
        made_app::diagnostics::GrokDiagnosticsPill::Degraded
    );
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

    // Missing remote python3 is a declared honest failure reason.
    let row = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Failed {
            reason: Some("python3-missing"),
        }),
        true,
        true,
    );
    assert!(
        row.sentence.contains("remote python3 is missing"),
        "{}",
        row.sentence
    );

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
    assert_eq!(row.pill, made_app::diagnostics::GrokDiagnosticsPill::Ready);
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
        view.grok_sentence, "Grok Build is not on PATH.",
        "the pre-existing host sentence is what a remote tab must replace"
    );

    let report = remote_grok_row(
        false,
        true,
        None,
        true,
        Some(RemoteGrokInstallOutcome::Failed {
            reason: Some("python3-missing"),
        }),
        true,
        true,
    );
    view.set_remote_grok(report.clone());
    assert_eq!(view.grok_pill, report.pill);
    assert_eq!(view.grok_sentence, report.sentence);
    let text = view.to_text();
    assert!(
        text.contains("grok status: degraded"),
        "the copied report must include the Grok pill: {text}"
    );
    assert!(
        text.contains("remote python3 is missing"),
        "the copied report must include the honest reason: {text}"
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
    assert_eq!(
        row.pill,
        made_app::diagnostics::GrokDiagnosticsPill::Degraded
    );
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
