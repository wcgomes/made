use std::fs;
use std::path::Path;

use made_explorer::paths::PathGuard;
use made_explorer::root_switch::RootGeneration;
use made_explorer::tree::{
    DirListing, EntryKind, TreeError, TreeModel, MAX_DIR_ENTRIES, MAX_TREE_DEPTH,
};
use made_explorer::watch::WatchEvent;

const GENERATION: RootGeneration = RootGeneration::new(1);

fn fixture_root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn names(entries: &[made_explorer::tree::TreeEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

#[test]
fn construction_is_lazy_and_orders_directories_first_case_insensitively() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    for dir in ["Zeta", "alpha", "Beta"] {
        fs::create_dir(root.join(dir)).unwrap();
    }
    for file in ["c.txt", "A.txt", "b.txt"] {
        fs::write(root.join(file), file).unwrap();
    }

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    assert_eq!(tree.root(), fs::canonicalize(&root).unwrap());
    assert!(
        tree.children(tree.root()).is_none(),
        "construction must not list the root"
    );

    tree.expand_root().unwrap();
    let children = tree.children(tree.root()).unwrap();
    assert_eq!(
        names(children),
        vec!["alpha", "Beta", "Zeta", "A.txt", "b.txt", "c.txt"]
    );
    assert!(children[..3]
        .iter()
        .all(|entry| entry.kind == EntryKind::Directory));
    assert!(children[3..]
        .iter()
        .all(|entry| entry.kind == EntryKind::File));
}

#[test]
fn ten_thousand_file_directory_is_not_read_until_expanded() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let bulk = root.join("bulk");
    fs::create_dir(&bulk).unwrap();
    for index in 0..10_000 {
        fs::write(bulk.join(format!("file-{index:05}.txt")), b"").unwrap();
    }

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();

    assert!(tree.listing(&bulk).is_none());
    assert_eq!(tree.children(tree.root()).unwrap().len(), 1);
    assert_eq!(tree.children(tree.root()).unwrap()[0].name, "bulk");
    assert!(!tree.rows().iter().any(|row| row.name.starts_with("file-")));

    tree.expand(&bulk).unwrap();
    let loaded = tree.children(&bulk).unwrap();
    assert_eq!(loaded.len(), 10_000);
    assert_eq!(loaded[0].name, "file-00000.txt");
    assert_eq!(loaded[10_000 - 1].name, "file-09999.txt");
}

#[test]
fn expansion_and_selection_survive_unrelated_updates() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let src = root.join("src");
    fs::create_dir(&src).unwrap();
    let selected = src.join("selected.rs");
    fs::write(&selected, "fn selected() {}").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.expand(&src).unwrap();
    tree.select(&selected).unwrap();
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&selected).unwrap().as_path())
    );

    let created = src.join("new-file.rs");
    fs::write(&created, "fn new_file() {}").unwrap();
    let update = tree
        .apply_watch_events(&[WatchEvent::created(&created, GENERATION)])
        .unwrap();
    assert_eq!(update.removed, Vec::<std::path::PathBuf>::new());
    assert!(!update.reload_dirs.is_empty());
    tree.apply_watch_events_and_reload(&[WatchEvent::created(&created, GENERATION)])
        .unwrap();

    assert!(tree.is_expanded(&src));
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&selected).unwrap().as_path())
    );
    assert!(tree
        .children(&src)
        .unwrap()
        .iter()
        .any(|entry| entry.name == "new-file.rs"));
    assert!(tree
        .rows()
        .iter()
        .any(|row| { row.path == fs::canonicalize(&created).unwrap() && row.depth == 1 }));
}

#[test]
fn deleting_the_selected_path_clears_selection_and_expansion() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let selected = nested.join("selected.txt");
    fs::write(&selected, "selected").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.expand(&nested).unwrap();
    tree.select(&selected).unwrap();

    fs::remove_dir_all(&nested).unwrap();
    let update = tree
        .apply_watch_events(&[WatchEvent::deleted(&nested, GENERATION)])
        .unwrap();
    assert!(update.selection_changed);
    assert!(tree.selection().is_none());
    assert!(!tree.is_expanded(&nested));
    assert!(tree.listing(&nested).is_none());
}

#[test]
fn rename_update_removes_old_path_and_remaps_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let old_path = root.join("old.txt");
    fs::write(&old_path, "content").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.select(&old_path).unwrap();

    let new_path = root.join("new.txt");
    fs::rename(&old_path, &new_path).unwrap();
    let update = tree
        .apply_watch_events(&[WatchEvent::renamed(&old_path, &new_path, GENERATION)])
        .unwrap();
    assert!(update.selection_changed);
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&new_path).unwrap().as_path())
    );

    // The parent listing was cleared for reload and can be loaded again.
    let root = tree.root().to_path_buf();
    tree.load_dir(&root).unwrap();
    let children = tree.children(tree.root()).unwrap();
    assert!(!children.iter().any(|entry| entry.name == "old.txt"));
    assert!(children.iter().any(|entry| entry.name == "new.txt"));
}

#[test]
fn stale_listing_is_rejected_by_apply_load() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    fs::write(root.join("file.txt"), "content").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    let tree_root = tree.root().to_path_buf();
    let stale = DirListing::load(tree.path_guard(), tree_root, GENERATION.next()).unwrap();
    let error = tree.apply_load(stale).unwrap_err();
    assert!(matches!(error, TreeError::StaleGeneration(_)));
    assert!(tree.children(tree.root()).is_none());
}

#[test]
fn event_outside_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "outside").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    let error = tree
        .apply_watch_events(&[WatchEvent::created(&outside, GENERATION)])
        .unwrap_err();
    assert!(matches!(error, TreeError::Path(_)));
    assert!(tree.children(tree.root()).is_none());

    let traversal = tree.root().join("../outside.txt");
    let error = tree
        .apply_watch_events(&[WatchEvent::created(traversal, GENERATION)])
        .unwrap_err();
    assert!(matches!(error, TreeError::Path(_)));
}

#[test]
fn loading_a_file_as_a_directory_is_an_error_row_not_a_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("file.txt");
    fs::write(&file, "content").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    // A direct listing of a file surfaces the read failure as a row.
    let listing = DirListing::load(tree.path_guard(), &file, GENERATION).unwrap();
    assert!(listing.is_error());
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].kind, EntryKind::Error);

    // Expansion still demands a directory.
    let error = tree.expand(&file).unwrap_err();
    assert!(matches!(error, TreeError::NotDirectory { .. }));
}

#[test]
fn path_guard_is_exposed_for_background_loads() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    fs::write(root.join("one.txt"), "one").unwrap();
    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    let tree_root = tree.root().to_path_buf();
    let request = tree.begin_load(tree_root).unwrap();
    assert_eq!(request.generation(), GENERATION);
    assert!(tree.is_loading(request.dir()));

    let guard = PathGuard::new(&root).unwrap();
    let listing = request.load(&guard).unwrap();
    tree.finish_load(&request, listing).unwrap();
    assert!(!tree.is_loading(request.dir()));
    assert!(tree
        .children(tree.root())
        .unwrap()
        .iter()
        .any(|entry| entry.name == "one.txt"));
}

#[test]
fn rows_include_only_expanded_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let src = root.join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("main.rs"), "fn main() {}").unwrap();
    fs::write(root.join("README.md"), "# readme").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    let initial = tree.rows();
    assert_eq!(initial.len(), 2);
    assert_eq!(initial[0].name, "src");
    assert_eq!(initial[1].name, "README.md");

    tree.expand(&src).unwrap();
    let expanded = tree.rows();
    assert_eq!(expanded.len(), 3);
    assert_eq!(expanded[0].name, "src");
    assert!(expanded[0].expanded);
    assert_eq!(expanded[1].name, "main.rs");
    assert_eq!(expanded[1].depth, 1);
}

#[test]
fn loading_a_missing_directory_is_an_error_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let missing = root.join("gone");
    let mut tree = TreeModel::new(&root, GENERATION).unwrap();

    tree.load_dir(&missing).unwrap();
    let listing = tree.listing(&missing).expect("error listing is cached");
    assert!(listing.is_error());
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].kind, EntryKind::Error);
    assert_eq!(tree.selection(), None);
}

#[cfg(unix)]
#[test]
fn unreadable_directory_is_an_error_row() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let locked = root.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("hidden.txt"), "hidden").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipping unreadable-directory assertion when running as root");
        return;
    }

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    let result = tree.expand(&locked);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    result.expect("an unreadable directory must not fail the tree");
    let listing = tree.listing(&locked).expect("error listing is cached");
    assert!(listing.is_error());
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].kind, EntryKind::Error);
    assert!(listing.entries[0].error.is_some());
}

#[test]
fn renaming_an_expanded_directory_preserves_its_loaded_contents() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let old_dir = root.join("src");
    fs::create_dir(&old_dir).unwrap();
    let selected = old_dir.join("main.rs");
    fs::write(&selected, "fn main() {}").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.expand(&old_dir).unwrap();
    tree.select(&selected).unwrap();

    let new_dir = root.join("lib");
    fs::rename(&old_dir, &new_dir).unwrap();
    tree.apply_watch_events_and_reload(&[WatchEvent::renamed(&old_dir, &new_dir, GENERATION)])
        .unwrap();

    assert!(!tree.is_expanded(&old_dir));
    assert!(tree.is_expanded(&new_dir));
    let new_selected = new_dir.join("main.rs");
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&new_selected).unwrap().as_path())
    );
    assert!(tree
        .children(&new_dir)
        .unwrap()
        .iter()
        .any(|entry| entry.name == "main.rs"));
}

#[test]
fn unknown_rename_reload_removes_old_path_and_clears_stale_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let old_path = root.join("old.txt");
    fs::write(&old_path, "content").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.select(&old_path).unwrap();

    let new_path = root.join("new.txt");
    fs::rename(&old_path, &new_path).unwrap();
    let update = tree
        .apply_watch_events(&[WatchEvent::renamed_unknown(&new_path, GENERATION)])
        .unwrap();
    assert!(update.removed.is_empty());
    for dir in update.reload_dirs {
        tree.load_dir(dir).unwrap();
    }

    let children = tree.children(tree.root()).unwrap();
    assert!(!children.iter().any(|entry| entry.name == "old.txt"));
    assert!(children.iter().any(|entry| entry.name == "new.txt"));
    assert!(tree.selection().is_none());
}

#[test]
fn deleting_a_path_under_a_collapsed_directory_still_clears_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let selected = nested.join("selected.txt");
    fs::write(&selected, "selected").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.select(&selected).unwrap();
    tree.collapse(&nested);

    fs::remove_file(&selected).unwrap();
    let update = tree
        .apply_watch_events(&[WatchEvent::deleted(&selected, GENERATION)])
        .unwrap();
    assert!(update.selection_changed);
    assert!(tree.selection().is_none());
}

#[test]
fn rescan_reloads_cached_directories_without_losing_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let src = root.join("src");
    fs::create_dir(&src).unwrap();
    let selected = src.join("selected.rs");
    fs::write(&selected, "fn selected() {}").unwrap();

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    tree.expand(&src).unwrap();
    tree.select(&selected).unwrap();

    let update = tree
        .apply_watch_events(&[WatchEvent::rescan(tree.root(), GENERATION)])
        .unwrap();

    assert!(update.reload_dirs.iter().any(|dir| dir == tree.root()));
    assert!(update.reload_dirs.iter().any(|dir| dir == &src));
    assert!(tree.is_expanded(&src));
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&selected).unwrap().as_path())
    );

    let reloaded = update
        .reload_dirs
        .iter()
        .filter(|dir| tree.listing(dir).is_some())
        .count();
    assert!(reloaded >= 2, "cached listings should all be scheduled");
}

#[test]
fn directory_over_the_entry_cap_is_truncated_with_an_error_row() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    for index in 0..=MAX_DIR_ENTRIES {
        fs::write(root.join(format!("file-{index:05}.txt")), b"").unwrap();
    }

    let mut tree = TreeModel::new(&root, GENERATION).unwrap();
    tree.expand_root().unwrap();
    let listing = tree.listing(tree.root()).unwrap();
    assert!(listing.is_truncated());
    assert_eq!(listing.entries.len(), MAX_DIR_ENTRIES + 1);
    let truncation = listing.entries.last().unwrap();
    assert_eq!(truncation.kind, EntryKind::Error);
    assert!(truncation.is_error());
    assert!(truncation
        .error
        .as_deref()
        .unwrap()
        .contains(&MAX_DIR_ENTRIES.to_string()));
}

#[test]
fn documented_watcher_and_tree_caps_are_bounded() {
    assert_eq!(MAX_DIR_ENTRIES, 10_000);
    assert_eq!(MAX_TREE_DEPTH, 64);
}

#[test]
fn root_stays_canonical_even_with_dot_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let tree = TreeModel::new(root.join("."), GENERATION).unwrap();
    assert_eq!(tree.root(), fs::canonicalize(&root).unwrap());
    assert_eq!(Path::new(tree.root()), fs::canonicalize(&root).unwrap());
}
