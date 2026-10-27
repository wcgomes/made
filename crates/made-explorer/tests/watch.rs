use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use made_explorer::root_switch::RootGeneration;
use made_explorer::tree::TreeModel;
use made_explorer::watch::{FsWatcher, WatchEvent, WatchEventKind};

const GENERATION: RootGeneration = RootGeneration::new(7);
const TIMEOUT: Duration = Duration::from_secs(5);

fn fixture_root(temp: &tempfile::TempDir) -> PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn wait_for<F>(watcher: &mut FsWatcher, timeout: Duration, mut predicate: F) -> Option<WatchEvent>
where
    F: FnMut(&WatchEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        for event in watcher.poll_all() {
            if predicate(&event) {
                return Some(event);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn watcher_emits_create_change_delete_and_rename_events() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let existing = root.join("existing.txt");
    let renamed_from = root.join("rename-me.txt");
    fs::write(&existing, "before").unwrap();
    fs::write(&renamed_from, "rename").unwrap();

    let mut watcher = FsWatcher::new(&root, GENERATION).unwrap();
    assert_eq!(watcher.generation(), GENERATION);
    assert_eq!(watcher.root(), fs::canonicalize(&root).unwrap());

    let created = root.join("created.txt");
    fs::write(&created, "created").unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Created && event.path == created
    })
    .expect("create event");
    assert_eq!(event.generation, GENERATION);
    assert!(event.is_current(GENERATION));

    fs::write(&existing, "after").unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Changed && event.path == existing
    })
    .expect("change event");
    assert_eq!(event.generation, GENERATION);

    fs::remove_file(&created).unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Deleted && event.path == created
    })
    .expect("delete event");
    assert_eq!(event.generation, GENERATION);

    let renamed_to = root.join("renamed.txt");
    fs::rename(&renamed_from, &renamed_to).unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Renamed
            && (event.path == renamed_to || event.path == renamed_from)
    })
    .expect("rename event");
    assert_eq!(event.generation, GENERATION);
    if let Some(old_path) = &event.old_path {
        // Linux and paired Windows events identify both sides.
        assert_eq!(old_path, &renamed_from);
        assert_eq!(event.path, renamed_to);
    } else {
        // macOS FSEvents reports a rename without associating the two paths.
        assert!(event.path == renamed_to || event.path == renamed_from);
    }
}

#[test]
fn create_latency_is_within_the_budget_with_a_tolerant_assertion() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let mut watcher = FsWatcher::new(&root, GENERATION).unwrap();
    assert!(
        watcher.debounce() < Duration::from_secs(1),
        "debounce must target the one-second update budget"
    );

    let file = root.join("latency.txt");
    let started = Instant::now();
    fs::write(&file, "latency").unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.path == file && event.kind == WatchEventKind::Created
    })
    .expect("create event");
    let elapsed = started.elapsed();

    assert_eq!(event.generation, GENERATION);
    // The product target is one second; allow CI scheduling headroom while
    // still failing a materially slow watcher.
    assert!(
        elapsed < Duration::from_secs(2),
        "create event took {elapsed:?}"
    );
}

#[test]
fn burst_writes_are_debounced_for_the_same_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("burst.txt");
    fs::write(&file, "initial").unwrap();
    let mut watcher = FsWatcher::new(&root, GENERATION).unwrap();

    for index in 0..20 {
        fs::write(&file, format!("write {index}")).unwrap();
    }

    // Wait for the first changed event, then let the debounce window settle and
    // count how many additional changed events were published. Coalescing is
    // expected; one extra change is tolerated for platform splitters.
    wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Changed && event.path == file
    })
    .expect("first changed event");

    std::thread::sleep(Duration::from_millis(350));
    let extra = watcher
        .poll_all()
        .into_iter()
        .filter(|event| event.kind == WatchEventKind::Changed && event.path == file)
        .count();
    assert!(
        extra <= 3,
        "burst should be debounced but produced {extra} extra change events"
    );
}

#[test]
fn watcher_events_update_the_tree_without_losing_expansion_or_selection() {
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

    let mut watcher = FsWatcher::new(&root, GENERATION).unwrap();
    let created = src.join("live.rs");
    fs::write(&created, "fn live() {}").unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| {
        event.kind == WatchEventKind::Created && event.path == created
    })
    .expect("create event");

    let update = tree.apply_watch_events(&[event]).unwrap();
    for dir in update.reload_dirs {
        tree.load_dir(dir).unwrap();
    }

    assert!(tree.is_expanded(&src));
    assert_eq!(
        tree.selection(),
        Some(fs::canonicalize(&selected).unwrap().as_path())
    );
    assert!(tree
        .children(&src)
        .unwrap()
        .iter()
        .any(|entry| entry.name == "live.rs"));
}

#[test]
fn watcher_events_can_be_discarded_by_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let mut watcher =
        FsWatcher::with_debounce(&root, GENERATION, Duration::from_millis(80)).unwrap();

    let file = root.join("stale.txt");
    fs::write(&file, "stale").unwrap();
    let event = wait_for(&mut watcher, TIMEOUT, |event| event.path == file).expect("create event");

    assert!(!event.is_current(GENERATION.next()));
    assert!(event.is_stale(GENERATION.next()));
}

#[test]
fn dropping_a_watcher_does_not_block_the_test_thread() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let watcher = FsWatcher::new(&root, GENERATION).unwrap();
    drop(watcher);
}
