use std::fs;

use made_explorer::root_switch::{GenerationGate, RootGeneration, StaleGeneration};
use made_explorer::tree::{DirListing, TreeError, TreeModel};
use made_explorer::watch::WatchEvent;

fn fixture_root(temp: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let root = temp.path().join(name);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn generation_tokens_are_monotonic_and_ordered() {
    let first = RootGeneration::INITIAL;
    let second = first.next();
    let third = second.next();

    assert!(first < second);
    assert!(second < third);
    assert!(first.is_stale(second));
    assert!(second.is_current(second));
    assert!(!third.is_current(second));
    assert_eq!(first.to_string(), "0");
    assert_eq!(third.get(), 2);
}

#[test]
fn generation_gate_rejects_old_and_unknown_tokens() {
    let mut gate = GenerationGate::new(RootGeneration::new(10));
    assert!(gate.accepts(RootGeneration::new(10)));
    assert_eq!(
        gate.check(RootGeneration::new(9)).unwrap_err(),
        StaleGeneration::new(RootGeneration::new(10), RootGeneration::new(9))
    );
    assert!(!gate.accepts(RootGeneration::new(11)));

    let next = gate.advance();
    assert_eq!(next, RootGeneration::new(11));
    assert!(!gate.accepts(RootGeneration::new(10)));
    assert_eq!(gate.accept(next, "value").unwrap(), "value");

    let error = gate.set_current(RootGeneration::new(10)).unwrap_err();
    assert_eq!(error.expected, next);
    assert_eq!(error.received, RootGeneration::new(10));
}

#[test]
fn stale_load_cannot_populate_the_new_tree() {
    let temp = tempfile::tempdir().unwrap();
    let old_root = fixture_root(&temp, "old-root");
    let new_root = fixture_root(&temp, "new-root");
    fs::write(old_root.join("old.txt"), "old").unwrap();
    fs::write(new_root.join("new.txt"), "new").unwrap();

    let old_generation = RootGeneration::new(1);
    let new_generation = old_generation.next();
    let mut tree = TreeModel::new(&old_root, old_generation).unwrap();
    let old_tree_root = tree.root().to_path_buf();
    let request = tree.begin_load(old_tree_root).unwrap();
    let stale_listing = request.load(tree.path_guard()).unwrap();
    assert_eq!(stale_listing.generation, old_generation);

    tree.switch_root(&new_root, new_generation).unwrap();
    let error = tree.apply_load(stale_listing.clone()).unwrap_err();
    assert!(matches!(error, TreeError::StaleGeneration(_)));
    assert!(tree.children(tree.root()).is_none());

    let error = tree.finish_load(&request, stale_listing).unwrap_err();
    assert!(matches!(error, TreeError::StaleGeneration(_)));

    tree.expand_root().unwrap();
    let names: Vec<&str> = tree
        .children(tree.root())
        .unwrap()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["new.txt"]);
}

#[test]
fn stale_event_cannot_modify_the_new_tree() {
    let temp = tempfile::tempdir().unwrap();
    let old_root = fixture_root(&temp, "old-root");
    let new_root = fixture_root(&temp, "new-root");
    let old_file = old_root.join("old.txt");
    let selected = new_root.join("selected.txt");
    fs::write(&old_file, "old").unwrap();
    fs::write(&selected, "new").unwrap();

    let old_generation = RootGeneration::new(4);
    let new_generation = old_generation.next();
    let mut tree = TreeModel::new(&new_root, new_generation).unwrap();
    tree.expand_root().unwrap();
    tree.select(&selected).unwrap();
    let rows_before = tree.rows();
    let selection_before = tree.selection().map(ToOwned::to_owned);

    let stale = WatchEvent::deleted(&old_file, old_generation);
    let error = tree.apply_watch_events(&[stale]).unwrap_err();
    assert!(matches!(error, TreeError::StaleGeneration(_)));
    assert_eq!(tree.rows(), rows_before);
    assert_eq!(tree.selection().map(ToOwned::to_owned), selection_before);
    assert!(tree.is_expanded(tree.root()));
}

#[test]
fn load_with_current_generation_but_old_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let old_root = fixture_root(&temp, "old-root");
    let new_root = fixture_root(&temp, "new-root");
    fs::write(old_root.join("old.txt"), "old").unwrap();
    fs::write(new_root.join("new.txt"), "new").unwrap();

    let generation = RootGeneration::new(9);
    let guard = made_explorer::paths::PathGuard::new(&old_root).unwrap();
    let mismatched = DirListing::load(&guard, &old_root, generation).unwrap();

    let mut tree = TreeModel::new(&new_root, generation).unwrap();
    let error = tree.apply_load(mismatched).unwrap_err();
    assert!(matches!(error, TreeError::Path(_)));
    assert!(tree.children(tree.root()).is_none());
}

#[test]
fn switching_roots_requires_a_newer_generation() {
    let temp = tempfile::tempdir().unwrap();
    let first = fixture_root(&temp, "first");
    let second = fixture_root(&temp, "second");

    let generation = RootGeneration::new(2);
    let mut tree = TreeModel::new(&first, generation).unwrap();
    tree.expand_root().unwrap();

    let error = tree.switch_root(&second, generation).unwrap_err();
    assert!(matches!(error, TreeError::NonMonotonicGeneration { .. }));

    let older = RootGeneration::new(1);
    let error = tree.switch_root(&second, older).unwrap_err();
    assert!(matches!(error, TreeError::NonMonotonicGeneration { .. }));

    tree.switch_root(&second, generation.next()).unwrap();
    assert_eq!(tree.generation(), generation.next());
    assert_eq!(tree.root(), fs::canonicalize(&second).unwrap());
    assert!(tree.selection().is_none());
    assert!(tree.children(tree.root()).is_none());
    assert!(!tree.is_expanded(tree.root()));
}

#[test]
fn stale_generation_display_is_actionable() {
    let error = StaleGeneration::new(RootGeneration::new(8), RootGeneration::new(3));
    let text = error.to_string();
    assert!(text.contains('8'));
    assert!(text.contains('3'));
}
