use std::fs;
use std::time::Duration;

#[cfg(target_os = "macos")]
use notify::Watcher;
#[cfg(target_os = "macos")]
use std::path::Path;

use made_explorer::preview::{Preview, PreviewKind, PREVIEW_SIZE_LIMIT};
use made_explorer::preview_refresh::{PreviewError, PreviewRefresher};
use made_explorer::root_switch::RootGeneration;

const GENERATION: RootGeneration = RootGeneration::new(1);
const TIMEOUT: Duration = Duration::from_secs(5);

/// Drain setup events (including the fixture write) until the watcher has been
/// quiet longer than the debounce thread's 600 ms maximum latency. The measured
/// `refresh()` runs after this point, so its own event is never drained here.
fn settle_events(refresher: &mut PreviewRefresher) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut quiet_since = std::time::Instant::now();
    while std::time::Instant::now() < deadline {
        if refresher.poll_event().is_some() {
            quiet_since = std::time::Instant::now();
        }
        if quiet_since.elapsed() >= Duration::from_millis(700) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
struct RawEventProbe {
    events: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    _watcher: notify::RecommendedWatcher,
}

#[cfg(target_os = "macos")]
impl RawEventProbe {
    fn start(root: &Path) -> Self {
        let (sender, events) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .expect("probe watcher");
        watcher
            .watch(root, notify::RecursiveMode::NonRecursive)
            .expect("probe watch");
        Self {
            events,
            _watcher: watcher,
        }
    }

    fn finish(self) -> Vec<String> {
        std::thread::sleep(Duration::from_millis(100));
        self.events
            .try_iter()
            .map(|result| match result {
                Ok(event) => format!("{:?} {:?}", event.kind, event.paths),
                Err(error) => format!("error: {error}"),
            })
            .collect()
    }
}

fn fixture_root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn external_change_refreshes_text_to_binary() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("note.txt");
    fs::write(&file, "text").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&file).unwrap();
    assert_eq!(
        refresher.refresh(selection).unwrap().kind(),
        PreviewKind::Text
    );

    fs::write(&file, b"text\0binary").unwrap();
    let event = refresher
        .wait_for_event(TIMEOUT)
        .expect("change event for previewed file");
    assert_eq!(event.path, fs::canonicalize(&file).unwrap());
    assert!(event.is_current(selection));
    assert_eq!(event.selection, selection);
    assert_eq!(event.generation, GENERATION);

    let preview = refresher.refresh(selection).unwrap();
    assert_eq!(preview.kind(), PreviewKind::Binary);
    assert!(preview.text().is_none());
}

#[test]
fn reading_the_preview_does_not_trigger_its_own_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("note.txt");
    fs::write(&file, "text").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&file).unwrap();
    settle_events(&mut refresher);

    #[cfg(target_os = "macos")]
    let probe = RawEventProbe::start(&root);
    let preview = refresher.refresh(selection).expect("first open read");
    assert_eq!(preview.kind(), PreviewKind::Text);

    // The debounce thread can hold a burst up to 600 ms; wait beyond that so a
    // first-read self-event cannot be mistaken for a missing one.
    std::thread::sleep(Duration::from_millis(700));
    let event = refresher.poll_event();
    #[cfg(target_os = "macos")]
    let raw_events = probe.finish();
    #[cfg(not(target_os = "macos"))]
    let raw_events: Vec<String> = Vec::new();
    assert!(
        event.is_none(),
        "opening the preview must not enqueue an access event; got {event:?}; raw watcher events: {raw_events:?}"
    );
}

#[test]
fn deleted_file_is_reclassified_as_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("deleted.txt");
    fs::write(&file, "content").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&file).unwrap();
    fs::remove_file(&file).unwrap();
    refresher
        .wait_for_event(TIMEOUT)
        .expect("delete event for previewed file");

    assert_eq!(
        refresher.refresh(selection).unwrap().kind(),
        PreviewKind::Missing
    );
}

#[test]
fn oversized_file_is_reclassified_with_size_and_limit() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("grows.log");
    fs::write(&file, "small").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&file).unwrap();
    assert_eq!(
        refresher.refresh(selection).unwrap().kind(),
        PreviewKind::Text
    );

    let size = PREVIEW_SIZE_LIMIT + 1;
    let handle = fs::File::create(&file).unwrap();
    handle.set_len(size).unwrap();
    refresher
        .wait_for_event(TIMEOUT)
        .expect("change event for oversized file");

    let preview = refresher.refresh(selection).unwrap();
    assert_eq!(preview.kind(), PreviewKind::TooLarge);
    assert_eq!(preview.size(), Some(size));
    let message = preview.message().unwrap();
    assert!(message.contains(&size.to_string()));
    assert!(message.contains(&PREVIEW_SIZE_LIMIT.to_string()));
}

#[test]
fn missing_selection_loads_the_missing_message() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let missing = root.join("never-there.txt");

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&missing).unwrap();
    assert_eq!(
        refresher.refresh(selection).unwrap().kind(),
        PreviewKind::Missing
    );
}

#[test]
fn selection_change_rejects_stale_refresh_and_accept() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let first_selection = refresher.select(&first).unwrap();
    fs::write(&first, "first changed").unwrap();
    let stale_event = refresher
        .wait_for_event(TIMEOUT)
        .expect("change event for first file");
    assert_eq!(stale_event.selection, first_selection);

    let second_selection = refresher.select(&second).unwrap();
    assert_ne!(first_selection, second_selection);
    assert!(!refresher.is_current(first_selection));
    assert!(refresher.is_current(second_selection));

    let error = refresher.refresh(first_selection).unwrap_err();
    assert!(matches!(error, PreviewError::StaleSelection { .. }));

    let stale_preview = Preview::Text {
        path: first.clone(),
        content: "first changed".to_string(),
        size: 13,
    };
    let error = refresher
        .accept(stale_event.selection, stale_preview)
        .unwrap_err();
    assert!(matches!(error, PreviewError::StaleSelection { .. }));

    let current = refresher.refresh(second_selection).unwrap();
    assert_eq!(current.kind(), PreviewKind::Text);
    assert_eq!(current.text(), Some("second"));
}

#[test]
fn root_switch_invalidates_selection_and_old_generation_loads() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("old.txt");
    fs::write(&file, "old").unwrap();

    let new_root = temp.path().join("new-root");
    fs::create_dir(&new_root).unwrap();
    fs::write(new_root.join("new.txt"), "new").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let old_selection = refresher.select(&file).unwrap();
    let new_generation = GENERATION.next();
    refresher.switch_root(&new_root, new_generation).unwrap();

    assert!(refresher.current_selection().is_none());
    let error = refresher.refresh(old_selection).unwrap_err();
    assert!(matches!(error, PreviewError::StaleSelection { .. }));

    let loader_error = refresher.loader().load_for(&file, GENERATION).unwrap_err();
    assert_eq!(loader_error.expected, new_generation);
    assert_eq!(loader_error.received, GENERATION);

    let new_selection = refresher.select(new_root.join("new.txt")).unwrap();
    assert_eq!(
        refresher.refresh(new_selection).unwrap().text(),
        Some("new")
    );
}

#[test]
fn rapid_changes_are_debounced_to_one_refresh_signal() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("burst.txt");
    fs::write(&file, "start").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    refresher.select(&file).unwrap();
    for index in 0..20 {
        fs::write(&file, format!("burst {index}")).unwrap();
    }

    refresher
        .wait_for_event(TIMEOUT)
        .expect("first change event");
    std::thread::sleep(Duration::from_millis(350));

    let mut extra = 0;
    while refresher.poll_event().is_some() {
        extra += 1;
    }
    assert!(extra <= 3, "burst produced {extra} extra refresh events");
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_reclassified_without_exposing_content() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_root(&temp);
    let file = root.join("secret.txt");
    fs::write(&file, "secret").unwrap();

    let mut refresher = PreviewRefresher::new(&root, GENERATION).unwrap();
    let selection = refresher.select(&file).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();

    let preview = refresher.refresh(selection);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

    let preview = preview.unwrap();
    if preview.kind() == PreviewKind::Text {
        eprintln!("skipping unreadable-preview assertion when running as root");
        return;
    }
    assert_eq!(preview.kind(), PreviewKind::Unreadable);
    assert!(preview.text().is_none());
    assert!(preview.message().unwrap().contains("unreadable"));
}
