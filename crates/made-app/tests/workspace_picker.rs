//! Folder picker model tests: tree flattening, expansion, selection, refresh.

use std::fs;
use std::path::{Path, PathBuf};

use made_app::workspace::{Surfaces, WorkspaceLaunch};
use made_app::workspace_picker::{AddressError, PickerError, WorkspacePicker, MAX_PICKER_ENTRIES};

fn fixture(temp: &tempfile::TempDir) -> PathBuf {
    let root = temp.path().join("root");
    for dir in ["Zeta", "alpha", "Beta"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::create_dir_all(root.join("alpha").join("nested").join("deep")).unwrap();
    fs::write(root.join("not-a-folder.txt"), "x").unwrap();
    root
}

fn names(picker: &WorkspacePicker) -> Vec<&str> {
    picker.rows().iter().map(|row| row.name()).collect()
}

fn canonical(path: impl AsRef<Path>) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

#[test]
fn picker_lists_root_child_folders_as_depth_zero_rows_without_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();

    assert_eq!(picker.current_dir(), canonical(&root));
    let root_names: Vec<&str> = picker.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(root_names, vec!["alpha", "Beta", "Zeta"]);
    assert_eq!(names(&picker), vec!["alpha", "Beta", "Zeta"]);
    assert!(picker.rows().iter().all(|row| row.depth() == 0));
    assert!(picker.rows().iter().all(|row| !row.expanded()));
    assert_eq!(picker.selection(), None);
    assert_eq!(picker.path_target(), canonical(&root));
    assert!(picker.entries().iter().all(|entry| entry.path().is_dir()));
}

#[test]
fn picker_expand_keeps_parents_and_collapse_removes_only_the_subtree() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    picker.select(0).unwrap();
    picker.toggle_selected().unwrap();
    assert_eq!(names(&picker), vec!["alpha", "nested", "Beta", "Zeta"]);
    assert_eq!(
        picker
            .rows()
            .iter()
            .map(|row| row.depth())
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 0]
    );
    assert!(picker.rows()[0].expanded());
    assert!(!picker.rows()[1].expanded());
    assert_eq!(picker.selection(), Some(0));
    assert_eq!(picker.path_target(), canonical(root.join("alpha")));

    picker.toggle(1).unwrap();
    assert_eq!(
        names(&picker),
        vec!["alpha", "nested", "deep", "Beta", "Zeta"]
    );
    assert_eq!(
        picker
            .rows()
            .iter()
            .map(|row| row.depth())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 0, 0]
    );
    assert_eq!(picker.selection(), Some(1));
    assert_eq!(picker.path_target(), canonical(root.join("alpha/nested")));

    // Collapsing an ancestor keeps the ancestor row and removes only its subtree.
    picker.toggle(0).unwrap();
    assert_eq!(names(&picker), vec!["alpha", "Beta", "Zeta"]);
    assert_eq!(picker.selection(), Some(0));
    assert!(!picker.rows()[0].expanded());
    assert_eq!(picker.path_target(), canonical(root.join("alpha")));
    assert!(!names(&picker).contains(&"nested"));
    assert!(!names(&picker).contains(&"deep"));
}

#[test]
fn picker_selection_and_path_target_share_one_tree_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    assert_eq!(picker.selection(), None);
    assert_eq!(
        picker.confirm_current().unwrap().explorer_root,
        canonical(&root)
    );

    picker.select(0).unwrap();
    assert_eq!(picker.path_target(), canonical(root.join("alpha")));
    assert_eq!(
        picker.confirm_current().unwrap().explorer_root,
        canonical(root.join("alpha"))
    );

    picker.toggle_selected().unwrap();
    picker.select(1).unwrap();
    assert_eq!(picker.path_target(), canonical(root.join("alpha/nested")));
    assert_eq!(
        picker.confirm_row(1).unwrap().explorer_root,
        canonical(root.join("alpha/nested"))
    );
    assert_eq!(
        picker.confirm_current().unwrap().explorer_root,
        canonical(root.join("alpha/nested"))
    );
}

#[test]
fn picker_keyboard_tree_steps_expand_collapse_and_parent_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    // No selection: Down picks first, Up picks last.
    picker.select_next();
    assert_eq!(picker.selection(), Some(0));
    picker.select_previous();
    assert_eq!(picker.selection(), Some(2));
    picker.select_previous();
    assert_eq!(picker.selection(), Some(1));
    picker.select_previous();
    assert_eq!(picker.selection(), Some(0));

    picker.expand_selected().unwrap();
    assert!(picker.rows()[0].expanded());
    assert_eq!(picker.selection(), Some(0));
    picker.expand_selected().unwrap();
    assert_eq!(
        picker.selection(),
        Some(1),
        "Right on an expanded folder selects its first child"
    );
    assert_eq!(picker.path_target(), canonical(root.join("alpha/nested")));

    // Collapsed child: Left selects the parent row.
    picker.collapse_selected().unwrap();
    assert_eq!(picker.selection(), Some(0));
    assert_eq!(picker.path_target(), canonical(root.join("alpha")));

    // Expanded parent: Left collapses it and keeps it selected.
    picker.collapse_selected().unwrap();
    assert!(!picker.rows()[0].expanded());
    assert_eq!(picker.selection(), Some(0));

    // Enter toggles both directions.
    picker.toggle_selected().unwrap();
    assert!(picker.rows()[0].expanded());
    picker.toggle_selected().unwrap();
    assert!(!picker.rows()[0].expanded());
}

#[test]
fn picker_up_reroots_clears_expansion_and_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(root.join("alpha")).unwrap();
    picker.toggle(0).unwrap();
    picker.select(0).unwrap();
    assert_eq!(picker.path_target(), canonical(root.join("alpha/nested")));

    picker.go_up().unwrap();
    assert_eq!(picker.current_dir(), canonical(&root));
    assert_eq!(picker.selection(), None);
    assert!(picker.rows().iter().all(|row| row.depth() == 0));
    assert!(picker.rows().iter().all(|row| !row.expanded()));
    assert_eq!(picker.path_target(), canonical(&root));
    assert_eq!(
        picker.confirm_current().unwrap().explorer_root,
        canonical(&root)
    );
}

#[test]
fn picker_refresh_preserves_expansion_and_valid_selection_by_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();
    picker.toggle(0).unwrap();
    picker.toggle(1).unwrap();
    picker.select(2).unwrap();
    let deep = canonical(root.join("alpha/nested/deep"));
    assert_eq!(picker.path_target(), deep);

    picker.refresh().unwrap();
    assert_eq!(picker.path_target(), deep);
    assert_eq!(picker.selection(), Some(2));
    assert!(picker.rows()[0].expanded());
    assert!(picker.rows()[1].expanded());
    assert_eq!(
        names(&picker),
        vec!["alpha", "nested", "deep", "Beta", "Zeta"]
    );

    fs::remove_dir_all(root.join("alpha/nested/deep")).unwrap();
    picker.refresh().unwrap();
    assert_eq!(picker.selection(), None, "removed path is no longer valid");
    assert_eq!(picker.path_target(), canonical(&root));
    assert_eq!(names(&picker), vec!["alpha", "nested", "Beta", "Zeta"]);
    assert!(picker.rows()[0].expanded());
    assert!(
        picker.rows()[1].expanded(),
        "the still-valid expanded folder keeps its expansion state"
    );
}

#[test]
fn picker_expand_unreadable_child_leaves_the_row_collapsed() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();
    let alpha = canonical(root.join("alpha"));
    fs::remove_dir_all(&alpha).unwrap();

    let error = picker.toggle(0).unwrap_err();
    assert!(matches!(error, PickerError::Workspace(_)));
    assert!(picker.error().is_some());
    assert!(!picker.rows()[0].expanded());
    assert_eq!(picker.selection(), Some(0));
    assert_eq!(
        names(&picker),
        vec!["alpha", "Beta", "Zeta"],
        "the failed expansion must not insert or remove rows"
    );
}

#[test]
fn picker_selection_and_invalid_tree_operations_are_safe() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    let error = picker.select(99).unwrap_err();
    assert!(matches!(
        error,
        PickerError::InvalidSelection { index: 99, len: 3 }
    ));
    assert_eq!(picker.selection(), None);
    assert_eq!(picker.path_target(), canonical(&root));

    let error = picker.toggle(99).unwrap_err();
    assert!(matches!(error, PickerError::InvalidSelection { .. }));
    let error = picker.confirm_row(99).unwrap_err();
    assert!(matches!(error, PickerError::InvalidSelection { .. }));
    assert_eq!(picker.current_dir(), canonical(&root));
}

#[test]
fn picker_caps_root_and_nested_directory_reads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large-root");
    fs::create_dir(&root).unwrap();
    let big = root.join("big");
    fs::create_dir(&big).unwrap();
    fs::create_dir(root.join("sibling")).unwrap();
    for index in 0..(MAX_PICKER_ENTRIES + 5) {
        fs::create_dir(big.join(format!("nested-{index:05}"))).unwrap();
    }
    let mut picker = WorkspacePicker::new(&root).unwrap();
    assert_eq!(picker.entries().len(), 2);
    assert_eq!(picker.truncated_entries(), 0);

    picker.toggle(0).unwrap();
    assert_eq!(picker.rows().len(), 2 + MAX_PICKER_ENTRIES);
    assert_eq!(picker.rows()[1].depth(), 1);
    assert_eq!(picker.rows()[MAX_PICKER_ENTRIES + 1].name(), "sibling");
}

#[test]
fn picker_root_cap_matches_the_existing_limit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large-root");
    fs::create_dir(&root).unwrap();
    for index in 0..(MAX_PICKER_ENTRIES + 25) {
        fs::create_dir(root.join(format!("folder-{index:05}"))).unwrap();
    }
    let mut picker = WorkspacePicker::new(&root).unwrap();
    assert_eq!(picker.entries().len(), MAX_PICKER_ENTRIES);
    assert_eq!(picker.rows().len(), MAX_PICKER_ENTRIES);
    assert_eq!(picker.truncated_entries(), 25);
    picker.select(MAX_PICKER_ENTRIES - 1).unwrap();
    assert_eq!(picker.selection(), Some(MAX_PICKER_ENTRIES - 1));
    picker.select_previous();
    assert_eq!(picker.selection(), Some(MAX_PICKER_ENTRIES - 2));
    picker.move_selection(1);
    assert_eq!(picker.selection(), Some(MAX_PICKER_ENTRIES - 1));
    picker.refresh().unwrap();
    assert_eq!(picker.entries().len(), MAX_PICKER_ENTRIES);
    assert_eq!(picker.truncated_entries(), 25);
    assert!(picker.selection().is_some());
}

#[test]
fn no_workspace_stays_in_the_picker_until_a_folder_is_confirmed() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();
    let mut launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };

    assert_eq!(launch.surfaces(), Surfaces::Picker);
    assert!(launch.terminal_cwd().is_none());
    assert!(launch.explorer_root().is_none());

    launch.confirm_picker().unwrap();
    assert_eq!(launch.surfaces(), Surfaces::full());
    assert!(launch.surfaces().has_terminal());
    assert!(launch.surfaces().has_explorer());
    assert_eq!(launch.explorer_root(), Some(canonical(&root).as_path()));
}

#[test]
fn launch_delegates_tree_toggle_without_opening_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();
    let mut launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };

    launch.picker_toggle(0).unwrap();
    assert_eq!(
        launch.picker().unwrap().current_dir(),
        canonical(&root),
        "expanding must not re-root the picker"
    );
    assert_eq!(
        launch.picker().unwrap().path_target(),
        canonical(root.join("alpha"))
    );
    assert_eq!(launch.surfaces(), Surfaces::Picker);
    assert!(launch.terminal_cwd().is_none());

    launch.picker_expand_selected().unwrap();
    assert_eq!(
        launch.picker().unwrap().path_target(),
        canonical(root.join("alpha/nested"))
    );
    launch.picker_collapse_selected().unwrap();
    assert_eq!(
        launch.picker().unwrap().path_target(),
        canonical(root.join("alpha"))
    );
    launch.picker_toggle_selected().unwrap();
    assert!(!launch.picker().unwrap().rows()[0].expanded());
    assert_eq!(launch.surfaces(), Surfaces::Picker);

    launch.picker_up().unwrap();
    assert_eq!(
        launch.picker().unwrap().current_dir(),
        canonical(temp.path())
    );
    assert_eq!(launch.surfaces(), Surfaces::Picker);
}

#[test]
fn confirming_a_vanished_folder_keeps_the_picker_visible() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vanishing");
    fs::create_dir(&root).unwrap();
    let picker = WorkspacePicker::new(&root).unwrap();
    let mut launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };

    fs::remove_dir_all(&root).unwrap();
    let error = launch.confirm_picker().unwrap_err();
    assert_eq!(
        error.kind(),
        made_app::workspace::WorkspaceErrorKind::Missing
    );
    assert_eq!(launch.surfaces(), Surfaces::Picker);
    assert!(launch.picker().is_some());
    assert!(launch.error().is_some());
    assert!(launch.terminal_cwd().is_none());
}

#[test]
fn invalid_picker_selection_is_visible_and_non_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();
    let mut launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };

    assert!(launch.picker_select(42).is_err());
    assert!(launch.picker().unwrap().error().is_some());
    assert_eq!(launch.surfaces(), Surfaces::Picker);
    assert!(Path::new(launch.picker().unwrap().current_dir()).is_dir());
}

#[test]
fn picker_refresh_and_recreation_invalidate_ui_cache_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();
    let instance = picker.instance_id();
    let revision = picker.revision();
    picker.refresh().unwrap();
    assert!(picker.revision() > revision);
    let other = WorkspacePicker::new(&root).unwrap();
    assert_ne!(instance, other.instance_id());
}

// open-workspace-redesign: the address bar's `navigate_to`.

#[test]
fn navigate_to_an_absolute_folder_reroots_with_no_selection_or_expansion() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();
    picker.toggle(0).unwrap();
    assert!(picker.rows()[0].expanded());
    let alpha = canonical(root.join("alpha"));

    picker.navigate_to(&alpha.to_string_lossy()).unwrap();
    assert_eq!(picker.current_dir(), alpha.as_path());
    assert_eq!(picker.selection(), None);
    assert_eq!(names(&picker), vec!["nested"]);
    assert!(picker.rows().iter().all(|row| !row.expanded()));
    assert_eq!(
        picker.path_target(),
        alpha.as_path(),
        "the root is the target"
    );
    assert!(picker.error().is_none());
}

#[test]
fn navigate_to_ignores_surrounding_whitespace_and_trailing_separators() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let nested = canonical(root.join("alpha").join("nested"));
    for typed in [
        format!("  {}/ ", nested.display()),
        format!("{}//", nested.display()),
        format!("\t{}\n", nested.display()),
    ] {
        let mut picker = WorkspacePicker::new(&root).unwrap();
        picker.navigate_to(&typed).unwrap();
        assert_eq!(picker.current_dir(), nested.as_path(), "{typed:?}");
        assert_eq!(picker.path_target(), nested.as_path());
    }
    #[cfg(unix)]
    {
        let mut picker = WorkspacePicker::new(&root).unwrap();
        picker.navigate_to(" / ").unwrap();
        assert_eq!(picker.current_dir(), Path::new("/"));
    }
}

/// Root, selection and target after an attempt, for unchanged-state checks.
fn state(picker: &WorkspacePicker) -> (PathBuf, Option<usize>, PathBuf, Vec<String>) {
    (
        picker.current_dir().to_path_buf(),
        picker.selection(),
        picker.path_target().to_path_buf(),
        picker
            .rows()
            .iter()
            .map(|row| row.name().to_owned())
            .collect(),
    )
}

#[test]
fn invalid_addresses_name_the_reason_and_keep_root_selection_and_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();
    picker.toggle(0).unwrap();
    picker.select(1).unwrap();
    let before = state(&picker);
    let file = canonical(root.join("not-a-folder.txt"));
    let missing = canonical(&root).join("does-not-exist");
    let cases: Vec<(String, AddressError, &str)> = vec![
        (
            "Repos/made".to_owned(),
            AddressError::NotAbsolute,
            "Path must be absolute",
        ),
        (
            "   ".to_owned(),
            AddressError::NotAbsolute,
            "Path must be absolute",
        ),
        (
            missing.display().to_string(),
            AddressError::NotFound,
            "Folder not found",
        ),
        (
            file.display().to_string(),
            AddressError::NotAFolder,
            "Not a folder",
        ),
    ];
    for (typed, expected, message) in cases {
        let error = picker.navigate_to(&typed).unwrap_err();
        assert_eq!(error, expected, "{typed:?}");
        assert_eq!(error.message(), message);
        assert_eq!(state(&picker), before, "{typed:?} must not change the tree");
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_address_keeps_the_tree() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let locked = canonical(root.join("Beta"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("permissions are not enforced for this user; unreadable case skipped");
        return;
    }
    let mut picker = WorkspacePicker::new(&root).unwrap();
    picker.select(0).unwrap();
    let before = state(&picker);
    let error = picker.navigate_to(&locked.to_string_lossy()).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(error, AddressError::NotReadable);
    assert_eq!(error.message(), "Folder is not readable");
    assert_eq!(state(&picker), before);
}
