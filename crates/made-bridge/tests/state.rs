use std::path::PathBuf;

use made_bridge::{
    protocol::StateMessage,
    security::{MAX_MESSAGE_BYTES, MAX_PATH_CHARS},
    state::{SessionInfo, SessionSnapshot, SnapshotError, SnapshotTracker, MAX_PROJECT_ID_CHARS},
};
use serde_json::json;

fn absolute_test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn absolute_path_with_len(length: usize) -> PathBuf {
    #[cfg(windows)]
    let prefix = "C:\\";
    #[cfg(not(windows))]
    let prefix = "/";
    let mut path = String::from(prefix);
    path.push_str(&"a".repeat(length - prefix.len()));
    PathBuf::from(path)
}

fn absolute_traversal_paths() -> [PathBuf; 3] {
    let base = std::env::temp_dir();
    [
        base.join("..").join("etc"),
        base.join(".").join("worktree"),
        base.join("a").join("..").join("..").join("b"),
    ]
}

fn info(id: &str, title: &str) -> SessionInfo {
    SessionInfo {
        session_id: id.to_owned(),
        title: title.to_owned(),
        active: false,
        busy: false,
        attention: false,
        unread: false,
        prompt_pulse: 0,
        renaming: false,
        directory: None,
        subpath: None,
        project_id: None,
    }
}

fn snapshot(revision: u64, sessions: Vec<SessionInfo>) -> SessionSnapshot {
    SessionSnapshot {
        revision,
        active_session_id: None,
        tabs_enabled: true,
        view_directory: None,
        sessions,
    }
}

fn state_with_view(view: serde_json::Value) -> SessionSnapshot {
    let wire = json!({
        "type": "state",
        "revision": 1,
        "activeSessionID": null,
        "tabsEnabled": true,
        "sessions": [],
        "viewDirectory": view
    });
    serde_json::from_value::<StateMessage>(wire)
        .expect("state parses")
        .into_snapshot()
}

#[test]
fn valid_snapshot_is_accepted() {
    let mut session = info("session-a", "Alpha");
    session.active = true;
    session.busy = true;
    session.attention = true;
    session.prompt_pulse = 3;
    session.directory = Some(absolute_test_path("worktree"));
    session.subpath = Some("src".into());
    session.project_id = Some("project-1".into());

    let mut value = snapshot(1, vec![session, info("session-b", "Beta")]);
    value.active_session_id = Some("session-a".into());

    assert!(value.validate().is_ok());

    let mut tracker = SnapshotTracker::new();
    let accepted = tracker.accept(value.clone()).expect("accepted");
    assert_eq!(accepted, value);
    assert_eq!(tracker.last_accepted_revision(), Some(1));
    assert_eq!(tracker.epoch(), 0);
}

#[test]
fn no_active_session_is_valid() {
    let value = snapshot(1, vec![info("session-a", "Alpha")]);
    assert!(value.validate().is_ok());
}

#[test]
fn zero_revision_is_rejected() {
    let value = snapshot(0, Vec::new());
    assert_eq!(value.validate(), Err(SnapshotError::ZeroRevision));
}

#[test]
fn more_than_64_sessions_is_rejected() {
    let sessions = (0..65)
        .map(|index| info(&format!("session-{index}"), "Title"))
        .collect();
    let value = snapshot(1, sessions);
    assert!(matches!(
        value.validate(),
        Err(SnapshotError::TooManySessions { count: 65, max: 64 })
    ));
}

#[test]
fn exactly_64_sessions_is_allowed() {
    let sessions = (0..64)
        .map(|index| info(&format!("session-{index}"), "Title"))
        .collect();
    assert!(snapshot(1, sessions).validate().is_ok());
}

#[test]
fn title_length_is_measured_in_characters() {
    let mut value = snapshot(1, vec![info("session-a", &"😀".repeat(512))]);
    assert!(value.validate().is_ok());

    value.sessions[0].title = "😀".repeat(513);
    assert!(matches!(
        value.validate(),
        Err(SnapshotError::TitleTooLong {
            index: 0,
            length: 513,
            max: 512
        })
    ));
}

#[test]
fn path_length_is_bounded() {
    let mut value = snapshot(1, vec![info("session-a", "Alpha")]);
    value.sessions[0].directory = Some(absolute_path_with_len(MAX_PATH_CHARS + 1));
    assert!(matches!(
        value.validate(),
        Err(SnapshotError::PathTooLong {
            index: 0,
            field: "directory",
            ..
        })
    ));

    value.sessions[0].directory = Some(absolute_path_with_len(MAX_PATH_CHARS));
    assert!(value.validate().is_ok());

    value.sessions[0].directory = None;
    value.sessions[0].subpath = Some("a".repeat(4097));
    assert!(matches!(
        value.validate(),
        Err(SnapshotError::PathTooLong {
            field: "subpath",
            ..
        })
    ));
}

#[test]
fn session_ids_must_be_non_empty_unique_and_bounded() {
    let mut value = snapshot(1, vec![info("", "Alpha")]);
    assert_eq!(
        value.validate(),
        Err(SnapshotError::EmptySessionId { index: 0 })
    );

    value = snapshot(1, vec![info(&"a".repeat(129), "Alpha")]);
    assert!(matches!(
        value.validate(),
        Err(SnapshotError::SessionIdTooLong { index: 0, .. })
    ));

    value = snapshot(1, vec![info("same", "One"), info("same", "Two")]);
    assert_eq!(
        value.validate(),
        Err(SnapshotError::DuplicateSessionId { index: 1 })
    );
}

#[test]
fn at_most_one_session_may_be_active() {
    let mut first = info("a", "A");
    first.active = true;
    let mut second = info("b", "B");
    second.active = true;
    let value = snapshot(1, vec![first, second]);
    assert_eq!(
        value.validate(),
        Err(SnapshotError::MultipleActiveSessions { count: 2 })
    );
}

#[test]
fn active_session_must_exist() {
    let mut value = snapshot(1, vec![info("a", "A")]);
    value.active_session_id = Some("missing".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::ActiveSessionMissing("missing".into()))
    );
}

#[test]
fn tracker_rejects_equal_and_older_revisions() {
    let mut tracker = SnapshotTracker::new();
    tracker
        .accept(snapshot(5, vec![info("a", "A")]))
        .expect("revision 5 accepted");

    let stale_equal = tracker
        .accept(snapshot(5, vec![info("a", "A")]))
        .expect_err("equal revision rejected");
    assert_eq!(
        stale_equal,
        SnapshotError::StaleRevision {
            revision: 5,
            last_accepted: 5
        }
    );

    let stale_old = tracker
        .accept(snapshot(4, vec![info("a", "A")]))
        .expect_err("older revision rejected");
    assert_eq!(
        stale_old,
        SnapshotError::StaleRevision {
            revision: 4,
            last_accepted: 5
        }
    );

    tracker
        .accept(snapshot(6, vec![info("a", "A")]))
        .expect("newer revision accepted");
    assert_eq!(tracker.last_accepted_revision(), Some(6));
}

#[test]
fn new_epoch_resets_revision_tracking() {
    let mut tracker = SnapshotTracker::new();
    tracker
        .accept(snapshot(9, vec![info("a", "A")]))
        .expect("initial revision");

    assert_eq!(tracker.begin_epoch(), 1);
    assert_eq!(tracker.epoch(), 1);
    assert_eq!(tracker.last_accepted_revision(), None);

    tracker
        .accept(snapshot(1, vec![info("a", "A")]))
        .expect("revision restarts in new epoch");
    assert_eq!(tracker.last_accepted_revision(), Some(1));

    assert_eq!(tracker.begin_epoch(), 2);
    assert_eq!(tracker.epoch(), 2);
}

#[test]
fn tracker_validates_before_accepting() {
    let mut tracker = SnapshotTracker::new();
    assert_eq!(
        tracker.accept(snapshot(0, Vec::new())),
        Err(SnapshotError::ZeroRevision)
    );
    assert_eq!(tracker.last_accepted_revision(), None);
}

#[test]
fn wire_field_names_match_the_production_plugin() {
    let directory = absolute_test_path("worktree")
        .to_string_lossy()
        .into_owned();
    let wire = json!({
        "type": "state",
        "revision": 7,
        "activeSessionID": "session-a",
        "tabsEnabled": true,
        "sessions": [{
            "sessionID": "session-a",
            "title": "Alpha",
            "active": true,
            "busy": false,
            "attention": true,
            "unread": true,
            "promptPulse": 4,
            "renaming": false,
            "directory": directory,
            "subpath": "src/lib.rs",
            "projectID": "project-1",
            "futureField": "ignored"
        }],
        "futureTopLevel": {"ignored": true}
    });

    let message: StateMessage = serde_json::from_value(wire).expect("state parses");
    let snapshot = message.into_snapshot();
    assert_eq!(snapshot.revision, 7);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("session-a"));
    assert!(snapshot.tabs_enabled);
    assert_eq!(snapshot.sessions[0].session_id, "session-a");
    assert!(snapshot.sessions[0].attention);
    assert!(snapshot.sessions[0].unread);
    assert_eq!(snapshot.sessions[0].prompt_pulse, 4);
    assert_eq!(
        snapshot.sessions[0].directory.as_deref(),
        Some(std::path::Path::new(&directory))
    );
    assert_eq!(snapshot.sessions[0].subpath.as_deref(), Some("src/lib.rs"));
    assert_eq!(
        snapshot.sessions[0].project_id.as_deref(),
        Some("project-1")
    );
    assert!(snapshot.validate().is_ok());
}

#[test]
fn view_directory_is_optional_nullable_and_wire_named() {
    let directory = absolute_test_path("view-directory")
        .to_string_lossy()
        .into_owned();

    let snapshot = state_with_view(json!(directory));
    assert!(snapshot.validate().is_ok());
    let wire = serde_json::to_value(StateMessage::new(snapshot)).expect("serializes");
    assert_eq!(wire["viewDirectory"], directory);

    let absent = serde_json::from_value::<StateMessage>(json!({
        "type": "state",
        "revision": 1,
        "activeSessionID": null,
        "tabsEnabled": true,
        "sessions": []
    }))
    .expect("state without a view directory parses")
    .into_snapshot();
    assert!(absent.validate().is_ok());
    let wire = serde_json::to_value(StateMessage::new(absent)).expect("serializes");
    assert!(
        wire.get("viewDirectory").is_some(),
        "viewDirectory must stay additive and explicit on the wire"
    );
    assert_eq!(wire["viewDirectory"], serde_json::Value::Null);

    let null = state_with_view(serde_json::Value::Null);
    assert!(null.validate().is_ok());
    let wire = serde_json::to_value(StateMessage::new(null)).expect("serializes");
    assert_eq!(wire["viewDirectory"], serde_json::Value::Null);
}

#[test]
fn view_directory_must_be_absolute_nonempty_nul_free_and_without_traversal() {
    for (value, expected) in [
        (json!(""), "empty"),
        (json!("relative/view"), "absolute"),
        (
            json!(format!("{}\0view", std::env::temp_dir().display())),
            "nul",
        ),
        (
            json!(absolute_traversal_paths()[0].to_string_lossy()),
            "traversal",
        ),
    ] {
        let error = state_with_view(value)
            .validate()
            .expect_err("invalid view directory must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("viewDirectory"),
            "view-directory-specific reason expected: {message}"
        );
        assert!(
            message.to_lowercase().contains(expected),
            "missing {expected:?} in {message}"
        );
    }

    let error = state_with_view(json!(
        absolute_path_with_len(MAX_PATH_CHARS + 1).to_string_lossy()
    ))
    .validate()
    .expect_err("oversized view directory must be rejected");
    assert!(
        error.to_string().contains("viewDirectory"),
        "view-directory-specific reason expected: {error}"
    );
    assert!(error
        .to_string()
        .contains(&(MAX_PATH_CHARS + 1).to_string()));

    let max = state_with_view(json!(
        absolute_path_with_len(MAX_PATH_CHARS).to_string_lossy()
    ));
    assert!(max.validate().is_ok());
    let wire = serde_json::to_vec(&StateMessage::new(max)).expect("serializes");
    assert!(
        wire.len() < MAX_MESSAGE_BYTES,
        "a maximum-length view directory must fit the message limit"
    );
}

#[test]
fn unread_is_optional_for_forward_compatibility() {
    // OpenCode v2.1.4 exposes attention/promptPulse but not unread.
    let wire = json!({
        "type": "state",
        "revision": 1,
        "activeSessionID": null,
        "tabsEnabled": true,
        "sessions": [{
            "sessionID": "session-a",
            "title": "Alpha",
            "active": false,
            "busy": false,
            "attention": false,
            "promptPulse": 0,
            "renaming": false,
            "directory": null,
            "subpath": null,
            "projectID": null
        }]
    });
    let message: StateMessage = serde_json::from_value(wire).expect("state parses");
    assert!(!message.snapshot.sessions[0].unread);
    assert!(message.snapshot.sessions[0].directory.is_none());
}

#[test]
fn snapshot_serializes_with_protocol_names() {
    let mut session = info("session-a", "Alpha");
    session.directory = Some(absolute_test_path("alpha"));
    let mut value = snapshot(3, vec![session]);
    value.active_session_id = Some("session-a".into());
    let wire = serde_json::to_value(StateMessage::new(value)).expect("serializes");
    assert_eq!(wire["type"], "state");
    assert_eq!(wire["revision"], 3);
    assert_eq!(wire["activeSessionID"], "session-a");
    assert_eq!(wire["tabsEnabled"], true);
    assert_eq!(wire["sessions"][0]["sessionID"], "session-a");
    assert_eq!(wire["sessions"][0]["promptPulse"], 0);
    assert!(wire["sessions"][0].get("projectID").is_some());
}

#[test]
fn directory_must_be_absolute_nonempty_and_without_traversal() {
    let mut value = snapshot(1, vec![info("session-a", "Alpha")]);
    value.sessions[0].directory = Some(PathBuf::new());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::EmptyDirectory { index: 0 })
    );

    value.sessions[0].directory = Some(PathBuf::from("relative/path"));
    assert_eq!(
        value.validate(),
        Err(SnapshotError::DirectoryNotAbsolute { index: 0 })
    );

    for traversal in absolute_traversal_paths() {
        value.sessions[0].directory = Some(traversal.clone());
        assert_eq!(
            value.validate(),
            Err(SnapshotError::PathTraversal {
                index: 0,
                field: "directory"
            }),
            "directory {}",
            traversal.display()
        );
    }

    value.sessions[0].directory = Some(PathBuf::from(format!(
        "{}\0worktree",
        std::env::temp_dir().display()
    )));
    assert_eq!(
        value.validate(),
        Err(SnapshotError::NulInField {
            index: 0,
            field: "directory"
        })
    );

    value.sessions[0].directory = Some(absolute_test_path("worktree"));
    assert!(value.validate().is_ok());
}

#[test]
fn subpath_must_be_relative_and_without_parent_segments() {
    let mut value = snapshot(1, vec![info("session-a", "Alpha")]);

    for invalid in ["/etc/passwd", "a/../../b", "../escape"] {
        value.sessions[0].subpath = Some(invalid.into());
        let result = value.validate();
        assert!(
            matches!(
                result,
                Err(SnapshotError::SubpathNotRelative { .. } | SnapshotError::PathTraversal { .. })
            ),
            "subpath {invalid}: {result:?}"
        );
    }

    value.sessions[0].subpath = Some("/etc/passwd".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::SubpathNotRelative { index: 0 })
    );

    value.sessions[0].subpath = Some("a/../b".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::PathTraversal {
            index: 0,
            field: "subpath"
        })
    );

    value.sessions[0].subpath = Some("a/\0b".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::NulInField {
            index: 0,
            field: "subpath"
        })
    );

    value.sessions[0].subpath = Some("crates/made-bridge".into());
    assert!(value.validate().is_ok());
}

#[test]
fn project_ids_and_session_ids_are_bounded_and_nul_free() {
    let mut value = snapshot(1, vec![info("session-a", "Alpha")]);
    value.sessions[0].project_id = Some("p".repeat(MAX_PROJECT_ID_CHARS));
    assert!(value.validate().is_ok());

    value.sessions[0].project_id = Some("p".repeat(MAX_PROJECT_ID_CHARS + 1));
    assert_eq!(
        value.validate(),
        Err(SnapshotError::ProjectIdTooLong {
            index: 0,
            length: MAX_PROJECT_ID_CHARS + 1,
            max: MAX_PROJECT_ID_CHARS,
        })
    );

    value.sessions[0].project_id = Some("project\0id".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::NulInField {
            index: 0,
            field: "projectID"
        })
    );

    value.sessions[0].project_id = None;
    value.sessions[0].session_id = "session\0a".into();
    assert_eq!(
        value.validate(),
        Err(SnapshotError::NulInField {
            index: 0,
            field: "sessionID"
        })
    );
}

#[test]
fn active_session_id_must_agree_with_the_active_flag() {
    let mut active = info("a", "A");
    active.active = true;

    let mut value = snapshot(1, vec![active.clone(), info("b", "B")]);
    value.active_session_id = Some("a".into());
    assert!(value.validate().is_ok());

    value.active_session_id = Some("b".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::ActiveSessionIdMismatch {
            active_session_id: Some("b".into()),
            flagged: "a".into(),
        })
    );

    value.active_session_id = None;
    assert_eq!(
        value.validate(),
        Err(SnapshotError::ActiveSessionIdMismatch {
            active_session_id: None,
            flagged: "a".into(),
        })
    );

    let mut not_flagged = info("a", "A");
    not_flagged.active = false;
    value = snapshot(1, vec![not_flagged]);
    value.active_session_id = Some("a".into());
    assert_eq!(
        value.validate(),
        Err(SnapshotError::ActiveSessionNotActive("a".into()))
    );
}
