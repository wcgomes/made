//! Degraded/unsupported integration scenarios from the change checkpoint.

use std::path::PathBuf;
use std::time::Instant;

use made_app::integration_status::{
    is_supported_opencode_version, IntegrationState, IntegrationStatus, ReasonCode, V1_SCOPE,
};
use made_bridge::server::{
    BridgeEvent, ConnectionInfo, DisconnectedInfo, Goodbye, HeartbeatTimeout, ProtocolError,
};
use made_bridge::state::{SessionInfo, SessionSnapshot};

fn root() -> PathBuf {
    PathBuf::from("/workspace/project")
}

fn state() -> IntegrationState {
    IntegrationState::new(Some(root()), PathBuf::from("/workspace/project"))
}

fn connection() -> ConnectionInfo {
    ConnectionInfo {
        connection_id: 9,
        pane_id: "pane-1".to_owned(),
        plugin_version: "0.1.0".to_owned(),
        opencode_version: "2.1.4".to_owned(),
        pid: 4242,
        cwd: root(),
        protocol_version: 1,
    }
}

fn snapshot(active: Option<&str>, tabs_enabled: bool) -> SessionSnapshot {
    let sessions = active
        .map(|id| SessionInfo {
            session_id: id.to_owned(),
            title: "Session".to_owned(),
            active: true,
            busy: false,
            attention: false,
            unread: false,
            prompt_pulse: 0,
            renaming: false,
            directory: Some(root()),
            subpath: None,
            project_id: None,
        })
        .into_iter()
        .collect();
    SessionSnapshot {
        revision: 1,
        active_session_id: active.map(str::to_owned),
        tabs_enabled,
        view_directory: None,
        sessions,
    }
}

#[test]
fn app_start_is_healthy_with_optional_integration_inactive() {
    let state = state();
    assert_eq!(state.status(), IntegrationStatus::Inactive);
    assert_eq!(state.reason().code, ReasonCode::Inactive);
    assert!(state.reason().human().contains("inactive"));
    assert_eq!(state.fallback_root(), Some(root().as_path()));
}

#[test]
fn state_without_an_actual_hello_cannot_activate_optional_integration() {
    let mut state = state();
    let change = state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("unowned"), true)));
    assert_eq!(change.current, IntegrationStatus::Inactive);
    assert!(state.active_session_id().is_none());
    assert_eq!(state.last_revision(), None);
}

#[test]
fn plugin_loads_on_first_opencode_launch() {
    let mut state = state();
    let hello = state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    assert_eq!(hello.current, IntegrationStatus::Connected);
    assert_eq!(hello.reason.code, ReasonCode::Connected);

    let changed = state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), true)));
    assert_eq!(changed.current, IntegrationStatus::Connected);
    assert_eq!(state.reason().code, ReasonCode::Connected);
    assert_eq!(state.active_session_id(), Some("ses_1"));
    assert_eq!(state.active_session_title(), Some("Session"));
    assert_eq!(state.last_revision(), Some(1));
    assert!(state.last_heartbeat().is_some());

    let report = state.report();
    assert_eq!(report.plugin_version.as_deref(), Some("0.1.0"));
    assert_eq!(report.opencode_version.as_deref(), Some("2.1.4"));
    assert_eq!(report.protocol_version, Some(1));
    assert_eq!(report.connection_id, Some(9));
    assert!(report.tabs_enabled);
}

#[test]
fn connected_state_without_an_active_session_keeps_the_root() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/project");
    let changed = state.apply_bridge_event(&BridgeEvent::State(snapshot(None, true)));

    assert_eq!(changed.current, IntegrationStatus::Connected);
    assert!(state.active_session_id().is_none());
    assert_eq!(state.last_revision(), Some(1));
    assert_eq!(
        state.last_known_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );
}

#[test]
fn session_tabs_unavailable_is_degraded_and_retains_the_root() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/last");
    let changed = state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), false)));

    assert_eq!(changed.current, IntegrationStatus::Degraded);
    assert_eq!(changed.reason.code, ReasonCode::TabsUnavailable);
    assert_eq!(state.fallback_root(), Some(root().as_path()));
}

#[test]
fn incompatible_protocol_version_is_unsupported_and_retains_root() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/last");

    let changed = state.apply_bridge_event(&BridgeEvent::ProtocolError(ProtocolError {
        connection_id: Some(9),
        code: "protocol_mismatch".to_owned(),
        message: "protocol version 2 is unsupported".to_owned(),
        fatal: true,
    }));

    assert_eq!(changed.current, IntegrationStatus::Unsupported);
    assert_eq!(changed.reason.code, ReasonCode::ProtocolMismatch);
    assert_eq!(state.fallback_root(), Some(root().as_path()));
    assert!(!state.status().is_connected());
}

#[test]
fn authentication_rejection_is_degraded_and_keeps_the_root() {
    let mut state = state();
    state.note_root("/workspace/last");
    let changed = state.apply_bridge_event(&BridgeEvent::ProtocolError(ProtocolError {
        connection_id: Some(1),
        code: "invalid_token".to_owned(),
        message: "token was rejected".to_owned(),
        fatal: true,
    }));

    assert_eq!(changed.current, IntegrationStatus::Degraded);
    assert_eq!(changed.reason.code, ReasonCode::AuthenticationRejected);
    assert_eq!(state.fallback_root(), Some(root().as_path()));
}

#[test]
fn bridge_disconnects_mid_session_and_heartbeat_timeout_retain_root() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/project");

    let closed = state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 9,
        authenticated: true,
        code: Some(1006),
        reason: "socket closed".to_owned(),
        replaced: false,
    }));
    assert_eq!(closed.current, IntegrationStatus::Degraded);
    assert_eq!(closed.reason.code, ReasonCode::BridgeClosed);
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );

    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    let heartbeat = state.apply_bridge_event(&BridgeEvent::HeartbeatTimeout(HeartbeatTimeout {
        connection_id: 9,
        missed_pongs: 3,
    }));
    assert_eq!(heartbeat.current, IntegrationStatus::Degraded);
    assert_eq!(heartbeat.reason.code, ReasonCode::HeartbeatTimeout);
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );
}

#[test]
fn protocol_rejection_survives_the_following_disconnect() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/project");
    state.apply_bridge_event(&BridgeEvent::ProtocolError(ProtocolError {
        connection_id: Some(9),
        code: "protocol_mismatch".to_owned(),
        message: "protocol version 2 is unsupported".to_owned(),
        fatal: true,
    }));
    let after_close = state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 9,
        authenticated: false,
        code: Some(4402),
        reason: "unsupported protocol version".to_owned(),
        replaced: false,
    }));

    assert_eq!(after_close.current, IntegrationStatus::Unsupported);
    assert_eq!(after_close.reason.code, ReasonCode::ProtocolMismatch);
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );
}

#[test]
fn opencode_exit_reason_survives_the_following_disconnect() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/project");
    state.apply_bridge_event(&BridgeEvent::Goodbye(Goodbye {
        connection_id: 9,
        reason: "plugin unloaded".to_owned(),
    }));
    let after_close = state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 9,
        authenticated: true,
        code: Some(1000),
        reason: "goodbye".to_owned(),
        replaced: false,
    }));

    assert_eq!(after_close.current, IntegrationStatus::Inactive);
    assert_eq!(after_close.reason.code, ReasonCode::OpenCodeExited);
}

#[test]
fn a_replaced_connection_does_not_degrade_the_status() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    let changed = state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 9,
        authenticated: true,
        code: Some(4409),
        reason: "replaced".to_owned(),
        replaced: true,
    }));
    assert_eq!(changed.current, IntegrationStatus::Connected);
    assert!(!changed.status_changed);
}

#[test]
fn rejected_clients_cannot_reset_a_healthy_authenticated_workspace() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), true)));
    state.note_root("/workspace/project");

    for connection_id in [1, 2, 3] {
        let rejected = state.apply_bridge_event(&BridgeEvent::ProtocolError(ProtocolError {
            connection_id: Some(connection_id),
            code: "invalid_token".to_owned(),
            message: "token was rejected".to_owned(),
            fatal: true,
        }));
        assert_eq!(rejected.current, IntegrationStatus::Connected);
        assert!(!rejected.status_changed);
        state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
            connection_id,
            authenticated: false,
            code: Some(4401),
            reason: "invalid token".to_owned(),
            replaced: false,
        }));
    }
    // A pre-authentication error carries no connection id and is equally noise.
    state.apply_bridge_event(&BridgeEvent::ProtocolError(ProtocolError {
        connection_id: None,
        code: "invalid_hello".to_owned(),
        message: "malformed hello".to_owned(),
        fatal: true,
    }));

    assert_eq!(state.status(), IntegrationStatus::Connected);
    assert_eq!(state.reason().code, ReasonCode::Connected);
    assert_eq!(state.connection().map(|info| info.connection_id), Some(9));
    assert_eq!(state.active_session_id(), Some("ses_1"));
    assert_eq!(state.last_revision(), Some(1));
    assert_eq!(
        state.last_known_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );
}

#[test]
fn foreign_disconnect_and_goodbye_do_not_clear_the_current_connection() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), true)));

    state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 3,
        authenticated: true,
        code: Some(1006),
        reason: "foreign".to_owned(),
        replaced: false,
    }));
    state.apply_bridge_event(&BridgeEvent::Goodbye(Goodbye {
        connection_id: 4,
        reason: "foreign unload".to_owned(),
    }));
    state.apply_bridge_event(&BridgeEvent::HeartbeatTimeout(HeartbeatTimeout {
        connection_id: 5,
        missed_pongs: 3,
    }));

    assert_eq!(state.status(), IntegrationStatus::Connected);
    assert_eq!(state.connection().map(|info| info.connection_id), Some(9));
    assert_eq!(state.active_session_id(), Some("ses_1"));
}

#[test]
fn opencode_exits_normally_and_keeps_the_shell_usable() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.note_root("/workspace/project");

    let goodbye = state.apply_bridge_event(&BridgeEvent::Goodbye(Goodbye {
        connection_id: 9,
        reason: "plugin unloaded".to_owned(),
    }));
    assert_eq!(goodbye.current, IntegrationStatus::Inactive);
    assert_eq!(goodbye.reason.code, ReasonCode::OpenCodeExited);
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );
    assert!(state.connection().is_none());

    let direct = state.note_opencode_exited();
    assert_eq!(direct.current, IntegrationStatus::Inactive);
    assert_eq!(direct.reason.code, ReasonCode::OpenCodeExited);
}

#[test]
fn opencode_missing_or_older_than_v2_is_unsupported() {
    let mut state = state();
    let missing = state.note_opencode_missing();
    assert_eq!(missing.current, IntegrationStatus::Unsupported);
    assert_eq!(missing.reason.code, ReasonCode::OpenCodeMissing);

    let old = state.note_opencode_version("1.9.0");
    assert_eq!(old.current, IntegrationStatus::Unsupported);
    assert_eq!(old.reason.code, ReasonCode::OpenCodeTooOld);

    let compatible = state.note_opencode_version("2.1.4");
    assert_eq!(compatible.current, IntegrationStatus::Inactive);
    assert_eq!(compatible.reason.code, ReasonCode::Inactive);

    assert!(is_supported_opencode_version("2.0.0"));
    assert!(is_supported_opencode_version("2.1.4"));
    assert!(!is_supported_opencode_version("1.99.0"));
    assert!(!is_supported_opencode_version("unknown"));
}

#[test]
fn opencode_started_with_extension_loading_disabled_and_config_failure_are_degraded() {
    let mut state = state();
    let extension = state.note_extension_loading_disabled();
    assert_eq!(extension.current, IntegrationStatus::Degraded);
    assert_eq!(extension.reason.code, ReasonCode::ExtensionLoadingDisabled);

    let config = state.note_config_composition_failed("merge failed");
    assert_eq!(config.current, IntegrationStatus::Degraded);
    assert_eq!(config.reason.code, ReasonCode::ConfigCompositionFailed);
    assert!(config.reason.human().contains("merge failed"));
}

#[test]
fn plugin_unavailable_is_degraded_with_a_reason() {
    let mut state = state();
    let changed = state.note_plugin_unavailable("no plugin connection");
    assert_eq!(changed.current, IntegrationStatus::Degraded);
    assert_eq!(changed.reason.code, ReasonCode::PluginNotLoaded);
    assert!(changed.reason.human().contains("did not load"));
}

#[test]
fn focus_command_failure_keeps_the_connection_and_workspace() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), true)));
    state.note_root("/workspace/project");

    let failed = state.note_focus_failed("session is gone");
    assert_eq!(failed.current, IntegrationStatus::Connected);
    assert_eq!(failed.reason.code, ReasonCode::FocusFailed);
    assert_eq!(state.focus_error(), Some("session is gone"));
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );

    let success = state.note_focus_succeeded();
    assert_eq!(success.current, IntegrationStatus::Connected);
    assert_eq!(success.reason.code, ReasonCode::Connected);
    assert!(state.focus_error().is_none());

    let timed_out = state.note_focus_timed_out();
    assert_eq!(timed_out.current, IntegrationStatus::Connected);
    assert_eq!(timed_out.reason.code, ReasonCode::FocusTimedOut);
}

#[test]
fn unavailable_directory_and_worktree_metadata_stay_connected() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_1"), true)));
    state.note_root("/workspace/project");

    let directory = state.note_active_directory_unavailable("ses_1", "removed by OpenCode");
    assert_eq!(directory.current, IntegrationStatus::Connected);
    assert_eq!(
        directory.reason.code,
        ReasonCode::ActiveDirectoryUnavailable
    );
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/project").as_path())
    );

    let worktree = state.note_worktree_metadata_unavailable("git missing");
    assert_eq!(worktree.current, IntegrationStatus::Connected);
    assert_eq!(
        worktree.reason.code,
        ReasonCode::WorktreeMetadataUnavailable
    );
}

#[test]
fn reconnect_restores_connected_status() {
    let mut state = state();
    state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    state.apply_bridge_event(&BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id: 9,
        authenticated: true,
        code: Some(1006),
        reason: "dropped".to_owned(),
        replaced: false,
    }));
    assert_eq!(state.status(), IntegrationStatus::Degraded);

    let reconnected = state.apply_bridge_event(&BridgeEvent::Connected(connection()));
    assert_eq!(reconnected.current, IntegrationStatus::Connected);
    assert_eq!(reconnected.reason.code, ReasonCode::Reconnected);

    let resumed = state.apply_bridge_event(&BridgeEvent::State(snapshot(Some("ses_2"), true)));
    assert_eq!(resumed.current, IntegrationStatus::Connected);
    assert_eq!(state.active_session_id(), Some("ses_2"));
}

#[test]
fn fallback_uses_latest_shell_not_last_agent_or_launch() {
    let mut state = IntegrationState::new(
        Some(PathBuf::from("/workspace/launch")),
        PathBuf::from("/workspace/terminal"),
    );
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/terminal").as_path())
    );
    state.note_root("/workspace/last");
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/terminal").as_path())
    );
    state.set_launch_root("/workspace/launch-2");
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/terminal").as_path())
    );
    state.set_terminal_cwd("/workspace/terminal-2");
    assert_eq!(
        state.fallback_root(),
        Some(PathBuf::from("/workspace/terminal-2").as_path())
    );
}

#[test]
fn reason_codes_and_scope_are_enumerated() {
    for code in ReasonCode::ALL {
        assert!(!code.code().is_empty());
        assert!(!code.message().is_empty());
    }
    assert_eq!(V1_SCOPE.agent_integrations.len(), 1);
    const { assert!(!V1_SCOPE.chat_ui) };
    const { assert!(!V1_SCOPE.worktree_management) };
    const { assert!(!V1_SCOPE.separate_opencode_server) };
}

#[test]
fn heartbeat_age_is_measured_from_the_supplied_instant() {
    let mut state = state();
    let now = Instant::now();
    state.note_heartbeat(now);
    let report = state.report();
    assert_eq!(report.heartbeat_age(now), Some(std::time::Duration::ZERO));
    assert!(!report.heartbeat_stale(now, std::time::Duration::from_secs(1)));
}
