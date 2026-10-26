//! Folder picker model tests: sorted entries, navigation, selection, confirm.

use std::fs;
use std::path::Path;

use made_app::workspace::{Surfaces, WorkspaceLaunch};
use made_app::workspace_picker::{PickerError, WorkspacePicker, MAX_PICKER_ENTRIES};

fn fixture(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    for dir in ["Zeta", "alpha", "Beta"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::create_dir_all(root.join("alpha").join("nested")).unwrap();
    fs::write(root.join("not-a-folder.txt"), "x").unwrap();
    root
}

#[test]
fn picker_lists_only_folders_in_stable_case_insensitive_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();

    assert_eq!(picker.current_dir(), fs::canonicalize(&root).unwrap());
    let names: Vec<&str> = picker.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(names, vec!["alpha", "Beta", "Zeta"]);
    assert_eq!(picker.selection(), Some(0));
    assert!(picker.entries().iter().all(|entry| entry.path().is_dir()));
}

#[test]
fn picker_navigates_into_a_folder_and_back_up() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    picker.select(0).unwrap();
    picker.enter_selected().unwrap();
    assert_eq!(
        picker.current_dir(),
        fs::canonicalize(root.join("alpha")).unwrap()
    );
    assert_eq!(picker.entries().len(), 1);
    assert_eq!(picker.entries()[0].name(), "nested");

    picker.go_up().unwrap();
    assert_eq!(picker.current_dir(), fs::canonicalize(root).unwrap());
}

#[test]
fn picker_selection_wraps_and_rejects_invalid_indices() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let mut picker = WorkspacePicker::new(&root).unwrap();

    picker.select(2).unwrap();
    picker.select_next();
    assert_eq!(picker.selection(), Some(0));
    picker.select_previous();
    assert_eq!(picker.selection(), Some(2));

    let selected = picker.selection();
    let error = picker.select(99).unwrap_err();
    assert!(matches!(
        error,
        PickerError::InvalidSelection { index: 99, len: 3 }
    ));
    assert_eq!(picker.selection(), selected);

    let error = picker.enter(99).unwrap_err();
    assert!(matches!(error, PickerError::InvalidSelection { .. }));
    assert_eq!(picker.current_dir(), fs::canonicalize(&root).unwrap());
}

#[test]
fn confirming_the_current_folder_opens_the_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(root.join("alpha")).unwrap();
    let plan = picker.confirm_current().unwrap();

    assert_eq!(
        plan.explorer_root,
        fs::canonicalize(root.join("alpha")).unwrap()
    );
    assert_eq!(plan.terminal_cwd(), plan.explorer_root.as_path());
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
    assert_eq!(
        launch.explorer_root(),
        Some(fs::canonicalize(&root).unwrap().as_path())
    );
}

#[test]
fn launch_delegates_picker_navigation_without_opening_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let picker = WorkspacePicker::new(&root).unwrap();
    let mut launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };

    launch.picker_select(1).unwrap();
    launch.picker_enter(1).unwrap();
    assert_eq!(
        launch.picker().unwrap().current_dir(),
        fs::canonicalize(root.join("Beta")).unwrap()
    );
    assert_eq!(launch.surfaces(), Surfaces::Picker);

    launch.picker_up().unwrap();
    launch.picker_move(2);
    assert_eq!(launch.picker().unwrap().selection(), Some(2));
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
fn picker_caps_large_directories_and_keeps_selection_usable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large");
    fs::create_dir(&root).unwrap();
    for index in 0..(MAX_PICKER_ENTRIES + 25) {
        fs::create_dir(root.join(format!("folder-{index:05}"))).unwrap();
    }
    let mut picker = WorkspacePicker::new(&root).unwrap();
    assert_eq!(picker.entries().len(), MAX_PICKER_ENTRIES);
    assert_eq!(picker.truncated_entries(), 25);
    assert_eq!(picker.selection(), Some(0));
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
