use std::{fs, path::PathBuf};

use made_bridge::{
    config::{compose, ComposeOptions},
    config_guard::{ConfigFileState, ConfigGuard, ConfigSnapshot, ConfigVerification},
    Token,
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    plugin: PathBuf,
    user_config: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("app-owned");
    let plugin = root.join("plugins").join("opencode-made");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(plugin.join("package.json"), r#"{"name":"opencode-made"}"#).unwrap();
    fs::write(plugin.join("tui.ts"), "export default {};\n").unwrap();
    let user_config = temp.path().join("cli.json");
    Fixture {
        _temp: temp,
        root,
        plugin,
        user_config,
    }
}

fn options(fixture: &Fixture) -> ComposeOptions {
    ComposeOptions::new(
        &fixture.plugin,
        &fixture.root,
        "ws://127.0.0.1:43210/made",
        Token::from_hex("0123456789abcdef".repeat(4)).unwrap(),
        "pane-test",
        "0.1.0-test",
    )
    .with_user_config_path(&fixture.user_config)
}

#[test]
fn sha256_matches_the_known_digest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cli.json");
    fs::write(&path, b"hello").unwrap();

    let snapshot = ConfigSnapshot::capture(&path).unwrap();
    assert_eq!(snapshot.state(), ConfigFileState::Present);
    assert_eq!(
        snapshot.sha256_hex().as_deref(),
        Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );
}

#[test]
fn user_config_is_unchanged_across_composition() {
    let fixture = fixture();
    let content = "{\n  \"plugins\": [{\"package\": \"./user\", \"options\": {\"keep\": true}}],\n  \"tabs\": {\"mode\": \"off\"}\n}\n";
    fs::write(&fixture.user_config, content).unwrap();

    let before = ConfigSnapshot::capture(&fixture.user_config).unwrap();
    compose(&options(&fixture)).expect("compose");
    let verification = before.verify().unwrap();

    assert!(verification.is_unchanged());
    assert_eq!(verification.algorithm(), "sha256");
    match verification {
        ConfigVerification::Unchanged { state, sha256 } => {
            assert_eq!(state, ConfigFileState::Present);
            assert_eq!(sha256, before.sha256_hex());
        }
        other => panic!("expected unchanged, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(&fixture.user_config).unwrap(), content);
}

#[test]
fn changed_content_is_reported_with_both_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cli.json");
    fs::write(&path, "{\"a\":1}").unwrap();

    let snapshot = ConfigSnapshot::capture(&path).unwrap();
    let before_hash = snapshot.sha256_hex();
    fs::write(&path, "{\"a\":2}").unwrap();

    match snapshot.verify().unwrap() {
        ConfigVerification::Changed {
            before,
            after,
            before_sha256,
            after_sha256,
        } => {
            assert_eq!(before, ConfigFileState::Present);
            assert_eq!(after, ConfigFileState::Present);
            assert_eq!(before_sha256, before_hash);
            assert_ne!(before_sha256, after_sha256);
            assert!(after_sha256.is_some());
        }
        other => panic!("expected changed, got {other:?}"),
    }
}

#[test]
fn missing_file_is_explicitly_represented() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing.json");

    let snapshot = ConfigSnapshot::capture(&path).unwrap();
    assert_eq!(snapshot.state(), ConfigFileState::Missing);
    assert!(snapshot.sha256().is_none());
    assert!(snapshot.sha256_hex().is_none());
    match snapshot.verify().unwrap() {
        ConfigVerification::Unchanged { state, sha256 } => {
            assert_eq!(state, ConfigFileState::Missing);
            assert!(sha256.is_none());
        }
        other => panic!("expected missing unchanged, got {other:?}"),
    }
}

#[test]
fn a_file_appearing_or_disappearing_is_a_change() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("appears.json");

    let missing = ConfigSnapshot::capture(&path).unwrap();
    fs::write(&path, "{}").unwrap();
    match missing.verify().unwrap() {
        ConfigVerification::Changed {
            before,
            after,
            before_sha256,
            after_sha256,
        } => {
            assert_eq!(before, ConfigFileState::Missing);
            assert_eq!(after, ConfigFileState::Present);
            assert!(before_sha256.is_none());
            assert!(after_sha256.is_some());
        }
        other => panic!("expected appeared, got {other:?}"),
    }

    let present = ConfigSnapshot::capture(&path).unwrap();
    fs::remove_file(&path).unwrap();
    match present.verify().unwrap() {
        ConfigVerification::Changed {
            before,
            after,
            before_sha256,
            after_sha256,
        } => {
            assert_eq!(before, ConfigFileState::Present);
            assert_eq!(after, ConfigFileState::Missing);
            assert!(before_sha256.is_some());
            assert!(after_sha256.is_none());
        }
        other => panic!("expected disappeared, got {other:?}"),
    }
}

#[test]
fn config_guard_wraps_before_and_after_evidence() {
    let fixture = fixture();
    fs::write(&fixture.user_config, "{}").unwrap();

    let guard = ConfigGuard::before_session(&fixture.user_config).unwrap();
    assert_eq!(guard.before().state(), ConfigFileState::Present);
    compose(&options(&fixture)).expect("compose");
    let evidence = guard.after_session().unwrap();
    assert!(evidence.is_unchanged());
}

#[test]
fn failed_composition_also_leaves_the_file_unchanged() {
    let fixture = fixture();
    let content = "{ invalid json";
    fs::write(&fixture.user_config, content).unwrap();
    let guard = ConfigGuard::before_session(&fixture.user_config).unwrap();

    assert!(compose(&options(&fixture)).is_err());
    assert!(guard.after_session().unwrap().is_unchanged());
    assert_eq!(fs::read_to_string(&fixture.user_config).unwrap(), content);
}
