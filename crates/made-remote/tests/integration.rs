use made_bridge::config_guard::ConfigSnapshot;
use made_remote::integration::{resolve_config, Integration};
#[test]
fn remote_v2_precedes_legacy_without_modifying_either() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = temp.path().join("v2.json");
    let legacy = temp.path().join("legacy.json");
    std::fs::write(&legacy, "{}").unwrap();
    assert_eq!(resolve_config(v2.clone(), legacy.clone()), legacy);
    std::fs::write(&v2, "{\"theme\":\"remote\"}").unwrap();
    let before = ConfigSnapshot::capture(&v2).unwrap();
    assert_eq!(resolve_config(v2.clone(), legacy), v2);
    assert!(before.verify().unwrap().is_unchanged());
}
#[test]
fn bridge_is_actual_remote_loopback_and_launch_config_is_immutable() {
    let temp = tempfile::tempdir().unwrap();
    let plugin = temp.path().join("plugin");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("package.json"), "{\"name\":\"opencode-made\"}").unwrap();
    std::fs::write(plugin.join("tui.ts"), "export default {}").unwrap();
    let config = temp.path().join("cli.json");
    std::fs::write(&config, "{\"theme\":\"remote\",\"plugins\":[\"existing\"]}").unwrap();
    let service = Integration::start(&plugin, temp.path(), config, None, "pane".into()).unwrap();
    assert!(service.url().starts_with("ws://127.0.0.1:"));
    let (key, value) = service.environment();
    assert_eq!(key, "OPENCODE_CLI_CONFIG_CONTENT");
    let value: serde_json::Value = serde_json::from_str(value).unwrap();
    assert_eq!(value["theme"], "remote");
    assert_eq!(value["plugins"][0], "existing");
    assert!(service.verify_config().unwrap().is_unchanged());
}
