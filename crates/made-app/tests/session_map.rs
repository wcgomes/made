//! Session map tests: open/close, active marker, activity, ordering, labels.

use std::path::PathBuf;

use made_app::session_map::{truncate_end, truncate_path, SessionMap};
use made_bridge::state::{SessionInfo, SessionSnapshot};

fn session(id: &str, title: &str, path: &str) -> SessionInfo {
    SessionInfo {
        session_id: id.to_owned(),
        title: title.to_owned(),
        active: false,
        busy: false,
        attention: false,
        unread: false,
        prompt_pulse: 0,
        renaming: false,
        directory: Some(PathBuf::from(path)),
        subpath: None,
        project_id: None,
    }
}

fn snapshot(active: Option<&str>, sessions: Vec<SessionInfo>) -> SessionSnapshot {
    let mut sessions = sessions;
    for session in &mut sessions {
        session.active = active == Some(session.session_id.as_str());
    }
    SessionSnapshot {
        revision: 1,
        active_session_id: active.map(str::to_owned),
        tabs_enabled: true,
        sessions,
    }
}

#[test]
fn open_sessions_are_listed_with_one_active_marker() {
    let mut map = SessionMap::new();
    let update = map.apply_snapshot(&snapshot(
        Some("b"),
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
            session("c", "Gamma", "/work/gamma"),
        ],
    ));

    assert_eq!(update.added, vec!["a", "b", "c"]);
    assert_eq!(map.len(), 3);
    assert_eq!(map.active_session_id(), Some("b"));
    assert_eq!(
        map.rows().iter().filter(|row| row.active).count(),
        1,
        "exactly one row may be active"
    );
    assert!(map.row("b").unwrap().active);
    assert!(!map.row("a").unwrap().active);
    assert_eq!(
        map.row("b").unwrap().directory,
        Some(PathBuf::from("/work/beta"))
    );
}

#[test]
fn session_close_removes_the_row_and_clears_selection() {
    let mut map = SessionMap::new();
    map.apply_snapshot(&snapshot(
        Some("b"),
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
        ],
    ));
    assert!(map.select("b"));
    assert_eq!(map.selected_session_id(), Some("b"));

    let update = map.apply_snapshot(&snapshot(
        Some("a"),
        vec![session("a", "Alpha", "/work/alpha")],
    ));

    assert_eq!(update.removed, vec!["b"]);
    assert_eq!(map.len(), 1);
    assert!(map.row("b").is_none());
    assert_eq!(map.active_session_id(), Some("a"));
    assert!(map.selected().is_none());
}

#[test]
fn activity_indicators_update_without_reordering() {
    let mut map = SessionMap::new();
    map.apply_snapshot(&snapshot(
        Some("a"),
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
        ],
    ));

    let mut beta = session("b", "Beta", "/work/beta");
    beta.busy = true;
    beta.attention = true;
    beta.unread = true;
    beta.prompt_pulse = 3;
    beta.renaming = true;
    let update = map.apply_snapshot(&snapshot(
        Some("a"),
        vec![session("a", "Alpha", "/work/alpha"), beta],
    ));

    assert!(update.updated.contains(&"b".to_owned()));
    assert!(!update.reordered);
    let row = map.row("b").unwrap();
    assert!(row.busy);
    assert!(row.attention);
    assert!(row.unread);
    assert_eq!(row.prompt_pulse, 3);
    assert!(row.renaming);
    assert_eq!(
        map.rows()
            .iter()
            .map(|row| row.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn unread_uses_the_plugin_derived_boolean_even_without_a_prompt_pulse() {
    let mut map = SessionMap::new();
    let mut info = session("a", "Alpha", "/work/alpha");
    info.unread = true;
    info.prompt_pulse = 0;
    map.apply_snapshot(&snapshot(Some("a"), vec![info]));

    assert!(map.row("a").unwrap().unread);
    assert_eq!(map.row("a").unwrap().prompt_pulse, 0);
}

#[test]
fn long_titles_and_paths_are_bounded_for_display() {
    let mut map = SessionMap::new();
    let long_title = "A very long session title ".repeat(20);
    let long_path = "/home/user/projects/a/very/deep/workspace/worktree-name";
    map.apply_snapshot(&snapshot(
        Some("long"),
        vec![session("long", &long_title, long_path)],
    ));
    let row = map.row("long").unwrap();

    let title = row.display_title(32);
    assert!(title.chars().count() <= 32);
    assert!(title.ends_with('…'));
    assert!(row.title.chars().count() > 32);

    let path = row.display_path(28);
    assert!(path.chars().count() <= 28);
    assert!(path.starts_with('…'));
    assert!(path.ends_with("worktree-name"));
}

#[test]
fn truncation_helpers_preserve_short_values_and_count_characters() {
    assert_eq!(truncate_end("short", 10), "short");
    assert_eq!(truncate_end("abcdef", 4), "abc…");
    assert_eq!(truncate_end("abcdef", 1), "…");
    assert_eq!(truncate_end("abcdef", 0), "");
    assert_eq!(truncate_path("/a/b/c", 20), "/a/b/c");
    let truncated = truncate_path("/one/two/three/four", 12);
    assert!(truncated.chars().count() <= 12);
    assert!(truncated.ends_with("four") || truncated.ends_with("hree/four"));
}

#[test]
fn no_active_session_leaves_every_row_inactive() {
    let mut map = SessionMap::new();
    map.apply_snapshot(&snapshot(
        None,
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
        ],
    ));

    assert!(map.active_session_id().is_none());
    assert!(map.rows().iter().all(|row| !row.active));
}

#[test]
fn duplicate_active_flags_never_produce_two_markers() {
    let mut map = SessionMap::new();
    let mut first = session("a", "Alpha", "/work/alpha");
    let mut second = session("b", "Beta", "/work/beta");
    first.active = true;
    second.active = true;
    map.apply_snapshot(&SessionSnapshot {
        revision: 1,
        active_session_id: None,
        tabs_enabled: true,
        sessions: vec![first, second],
    });

    assert_eq!(map.active_session_id(), Some("a"));
    assert_eq!(map.rows().iter().filter(|row| row.active).count(), 1);
}

#[test]
fn selection_moves_and_survives_only_existing_rows() {
    let mut map = SessionMap::new();
    map.apply_snapshot(&snapshot(
        Some("a"),
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
            session("c", "Gamma", "/work/gamma"),
        ],
    ));

    assert!(map.selected().is_none());
    map.select_next();
    assert_eq!(map.selected_session_id(), Some("a"));
    assert!(map.select_index(1));
    assert_eq!(map.selected_session_id(), Some("b"));
    map.select_next();
    assert_eq!(map.selected_session_id(), Some("c"));
    map.select_next();
    assert_eq!(map.selected_session_id(), Some("a"));
    map.select_previous();
    assert_eq!(map.selected_session_id(), Some("c"));

    map.apply_snapshot(&snapshot(
        Some("a"),
        vec![
            session("a", "Alpha", "/work/alpha"),
            session("b", "Beta", "/work/beta"),
        ],
    ));
    assert!(map.selected().is_none());
}

#[test]
fn an_empty_map_reserves_no_default_strip_data() {
    let map = SessionMap::new();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    assert!(map.rows().is_empty());
    assert!(map.active_session_id().is_none());
}

#[test]
fn maps_are_owned_independently_by_each_workspace() {
    let mut first = SessionMap::new();
    let mut second = SessionMap::new();
    first.apply_snapshot(&snapshot(
        Some("a"),
        vec![session("a", "Alpha", "/work/alpha")],
    ));
    second.apply_snapshot(&snapshot(
        Some("z"),
        vec![session("z", "Zeta", "/work/zeta")],
    ));
    assert!(first.select("a"));
    assert!(second.select("z"));

    assert_eq!(first.active_session_id(), Some("a"));
    assert_eq!(second.active_session_id(), Some("z"));
    assert!(first.row("z").is_none());
    assert!(second.row("a").is_none());
    assert_eq!(first.selected_session_id(), Some("a"));
    assert_eq!(second.selected_session_id(), Some("z"));

    second.clear();
    assert_eq!(first.len(), 1);
    assert_eq!(first.active_session_id(), Some("a"));
}
