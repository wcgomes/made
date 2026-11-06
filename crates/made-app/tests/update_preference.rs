//! REL-10 update preference store (add-release-auto-update task 5.1).

use made_app::preferences::{PreferenceStore, PREFERENCES_FILE};

#[test]
fn defaults_to_on_for_a_new_install() {
    let dir = tempfile::tempdir().unwrap();
    let preference = PreferenceStore::new(dir.path().join("data")).load();
    assert!(preference.automatic_checks);
    assert_eq!(preference.diagnostic, None);
}

#[test]
fn persists_across_restart_and_survives_replacement_of_app_files() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let install = dir.path().join("install");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::write(install.join("made-app"), b"v1").unwrap();

    PreferenceStore::new(&data)
        .set_automatic_checks(false)
        .unwrap();
    // A restart is a fresh store; an update replaces every app file.
    std::fs::remove_dir_all(&install).unwrap();
    std::fs::create_dir_all(&install).unwrap();
    std::fs::write(install.join("made-app"), b"v2").unwrap();
    let reloaded = PreferenceStore::new(&data).load();
    assert!(!reloaded.automatic_checks);
    assert!(!install.join(PREFERENCES_FILE).exists());

    PreferenceStore::new(&data)
        .set_automatic_checks(true)
        .unwrap();
    assert!(PreferenceStore::new(&data).load().automatic_checks);
}

#[test]
fn a_corrupt_file_reads_as_on_and_reports_a_diagnostic() {
    for corrupt in [
        &b"{not json"[..],
        br#"{"schema":9,"automatic_update_checks":false}"#,
        b"",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let store = PreferenceStore::new(&data);
        store.set_automatic_checks(false).unwrap();
        std::fs::write(data.join(PREFERENCES_FILE), corrupt).unwrap();
        let preference = store.load();
        assert!(preference.automatic_checks, "{corrupt:?}");
        let diagnostic = preference.diagnostic.expect("a diagnostic");
        assert!(diagnostic.contains("automatic checks are on"));
        assert!(
            !diagnostic.contains(dir.path().to_str().unwrap()),
            "no paths in the diagnostic"
        );
    }
}

#[cfg(unix)]
#[test]
fn the_preference_file_is_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    PreferenceStore::new(&data)
        .set_automatic_checks(false)
        .unwrap();
    let file = std::fs::metadata(data.join(PREFERENCES_FILE)).unwrap();
    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    let directory = std::fs::metadata(&data).unwrap();
    assert_eq!(
        directory.permissions().mode() & 0o077,
        0,
        "the data directory is owner-only"
    );
}
