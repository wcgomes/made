use std::{fs, path::PathBuf};

use made_bridge::{
    config::{compose, ComposeOptions, ConfigError},
    Token,
};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    plugin: PathBuf,
    user_config: PathBuf,
    token: Token,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("app-owned");
    let plugin = root.join("plugins").join("opencode-made");
    fs::create_dir_all(&plugin).expect("plugin dir");
    fs::write(plugin.join("package.json"), r#"{"name":"opencode-made"}"#).expect("package");
    fs::write(plugin.join("tui.ts"), "export default {};\n").expect("plugin entry");
    let user_config = temp.path().join("cli.json");

    Fixture {
        _temp: temp,
        root,
        plugin,
        user_config,
        token: Token::from_hex("0123456789abcdef".repeat(4)).expect("token"),
    }
}

fn options(fixture: &Fixture) -> ComposeOptions {
    ComposeOptions::new(
        &fixture.plugin,
        &fixture.root,
        "ws://127.0.0.1:43210/made",
        fixture.token.clone(),
        "pane-test",
        "0.1.0-test",
    )
}

fn write_json(path: &PathBuf, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write fixture");
}

fn parse(composed: &made_bridge::config::ComposedConfig) -> Value {
    serde_json::from_str(&composed.content).expect("composed JSON")
}

fn canonical_plugin(fixture: &Fixture) -> String {
    fs::canonicalize(&fixture.plugin)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn existing_plugins_and_options_are_preserved_in_order() {
    let fixture = fixture();
    let base = json!({
        "plugins": [
            {"package": "./plugins/user-one", "options": {"keep": true, "nested": {"a": 1}}},
            "plain-plugin",
            {"package": "./plugins/user-two", "options": {"order": 2}}
        ],
        "tabs": {"mode": "off", "scope": "session", "layout": "tabs", "pinned": ["x"]},
        "theme": "dark",
        "nested": {"a": 1, "b": 2}
    });
    write_json(&fixture.user_config, &base);

    let composed =
        compose(&options(&fixture).with_user_config_path(&fixture.user_config)).expect("compose");
    let config = parse(&composed);

    let plugins = config["plugins"].as_array().expect("plugins array");
    assert_eq!(plugins.len(), 4);
    assert_eq!(plugins[0], base["plugins"][0]);
    assert_eq!(plugins[1], base["plugins"][1]);
    assert_eq!(plugins[2], base["plugins"][2]);
    assert_eq!(plugins[3]["package"], canonical_plugin(&fixture));
    assert_eq!(
        plugins[3]["options"]["made"]["token"],
        fixture.token.expose_secret()
    );

    assert_eq!(config["tabs"]["mode"], "on");
    assert_eq!(config["tabs"]["scope"], "global");
    assert_eq!(config["tabs"]["layout"], "tabs");
    assert_eq!(config["tabs"]["pinned"], json!(["x"]));
    assert_eq!(config["theme"], "dark");
    assert_eq!(config["nested"], json!({"a": 1, "b": 2}));

    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("tabs.mode")));
    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("tabs.scope")));
}

#[test]
fn made_plugin_options_are_namespaced_and_complete() {
    let fixture = fixture();
    let composed = compose(&options(&fixture)).expect("compose");
    let config = parse(&composed);
    let made = &config["plugins"][0]["options"]["made"];
    assert_eq!(made["bridgeUrl"], "ws://127.0.0.1:43210/made");
    assert_eq!(made["token"], fixture.token.expose_secret());
    assert_eq!(made["paneId"], "pane-test");
    assert_eq!(made["protocolVersion"], 1);
    assert_eq!(made["appVersion"], "0.1.0-test");
    assert_eq!(config["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(config["tabs"]["mode"], "on");
    assert_eq!(config["tabs"]["scope"], "global");
    assert_eq!(
        composed.plugin_dir,
        fs::canonicalize(&fixture.plugin).unwrap()
    );
}

#[test]
fn inline_overlay_is_deep_merged_and_does_not_drop_plugins() {
    let fixture = fixture();
    let base = json!({
        "plugins": [
            {"package": "./plugins/user-one", "options": {"keep": true}}
        ],
        "tabs": {"layout": "tabs"},
        "nested": {"a": 1, "c": 3},
        "theme": "dark"
    });
    let overlay = json!({
        "plugins": [
            {"package": "./plugins/overlay", "options": {"overlay": true}},
            "overlay-string"
        ],
        "tabs": {"scope": "tab"},
        "nested": {"a": 9, "b": 2},
        "theme": "light"
    });
    write_json(&fixture.user_config, &base);

    let composed = compose(
        &options(&fixture)
            .with_user_config_path(&fixture.user_config)
            .with_inline_config(overlay.to_string()),
    )
    .expect("compose");
    let config = parse(&composed);

    let plugins = config["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 4);
    assert_eq!(plugins[0], base["plugins"][0]);
    assert_eq!(plugins[1], overlay["plugins"][0]);
    assert_eq!(plugins[2], overlay["plugins"][1]);
    assert_eq!(plugins[3]["package"], canonical_plugin(&fixture));

    assert_eq!(config["tabs"]["layout"], "tabs");
    assert_eq!(config["tabs"]["mode"], "on");
    assert_eq!(config["tabs"]["scope"], "global");
    assert_eq!(config["nested"], json!({"a": 9, "b": 2, "c": 3}));
    assert_eq!(config["theme"], "light");
}

#[test]
fn duplicate_made_entries_are_removed_by_identity() {
    let fixture = fixture();
    let made = canonical_plugin(&fixture);
    let base = json!({
        "plugins": [
            {"package": made, "options": {"made": {"token": "old"}}},
            {"package": "./plugins/user", "options": {"keep": true}},
            made
        ]
    });
    let overlay = json!({
        "plugins": [
            {"package": made, "options": {"made": {"token": "older"}}},
            "overlay-string"
        ]
    });
    write_json(&fixture.user_config, &base);

    let composed = compose(
        &options(&fixture)
            .with_user_config_path(&fixture.user_config)
            .with_inline_config(overlay.to_string()),
    )
    .expect("compose");
    let config = parse(&composed);
    let plugins = config["plugins"].as_array().unwrap();

    let made_count = plugins
        .iter()
        .filter(|entry| entry.get("package").and_then(Value::as_str) == Some(made.as_str()))
        .count();
    assert_eq!(made_count, 1);
    assert_eq!(plugins.last().unwrap()["package"], made.as_str());
    assert_eq!(plugins[0]["package"], "./plugins/user");
    assert_eq!(plugins[1], "overlay-string");
    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("Removed 3 existing Made plugin")));
}

#[test]
fn invalid_base_json_is_a_typed_error_and_is_not_rewritten() {
    let fixture = fixture();
    fs::write(&fixture.user_config, "{ not valid json").unwrap();
    let before = fs::read(&fixture.user_config).unwrap();

    let error = compose(&options(&fixture).with_user_config_path(&fixture.user_config))
        .expect_err("invalid base");
    assert!(matches!(error, ConfigError::InvalidBaseJson { .. }));
    assert_eq!(fs::read(&fixture.user_config).unwrap(), before);
}

#[test]
fn invalid_inline_json_is_a_typed_error() {
    let fixture = fixture();
    let error = compose(&options(&fixture).with_inline_config("{\"plugins\": [}"))
        .expect_err("invalid overlay");
    assert!(matches!(error, ConfigError::InvalidOverlayJson(_)));
}

#[test]
fn non_object_roots_are_typed_errors() {
    let fixture = fixture();
    write_json(&fixture.user_config, &json!(["not", "an", "object"]));
    let error = compose(&options(&fixture).with_user_config_path(&fixture.user_config))
        .expect_err("array base");
    assert!(matches!(error, ConfigError::BaseNotObject { .. }));

    let error =
        compose(&options(&fixture).with_inline_config("[1, 2, 3]")).expect_err("array overlay");
    assert!(matches!(error, ConfigError::OverlayNotObject));
}

#[test]
fn plugins_must_be_an_array() {
    let fixture = fixture();
    write_json(
        &fixture.user_config,
        &json!({"plugins": {"package": "not-an-array"}}),
    );
    let error = compose(&options(&fixture).with_user_config_path(&fixture.user_config))
        .expect_err("plugins object");
    assert!(matches!(
        error,
        ConfigError::InvalidField {
            field: "plugins",
            ..
        }
    ));

    let error = compose(
        &options(&fixture).with_inline_config(json!({"plugins": "not-an-array"}).to_string()),
    )
    .expect_err("overlay plugins string");
    assert!(matches!(
        error,
        ConfigError::InvalidField {
            field: "plugins",
            ..
        }
    ));
}

#[test]
fn plugin_path_outside_app_owned_root_is_rejected() {
    let fixture = fixture();
    let outside = fixture._temp.path().join("outside-plugin");
    fs::create_dir_all(&outside).unwrap();

    let mut options = options(&fixture);
    options.plugin_dir = outside;
    let error = compose(&options).expect_err("outside plugin");
    assert!(matches!(error, ConfigError::PluginOutsideRoot { .. }));
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = fixture();
    let outside = fixture._temp.path().join("outside-plugin");
    fs::create_dir_all(&outside).unwrap();
    let link = fixture.root.join("plugins").join("escaped");
    symlink(&outside, &link).unwrap();

    let mut options = options(&fixture);
    options.plugin_dir = link;
    let error = compose(&options).expect_err("symlink escape");
    assert!(matches!(error, ConfigError::PluginOutsideRoot { .. }));
}

#[test]
fn missing_plugin_path_is_rejected() {
    let fixture = fixture();
    let mut options = options(&fixture);
    options.plugin_dir = fixture.root.join("plugins").join("does-not-exist");
    let error = compose(&options).expect_err("missing plugin");
    assert!(matches!(error, ConfigError::PluginPath { .. }));
}

#[test]
fn missing_user_config_is_allowed_with_a_warning() {
    let fixture = fixture();
    let composed = compose(&options(&fixture).with_user_config_path(&fixture.user_config))
        .expect("missing base");
    assert_eq!(parse(&composed)["plugins"].as_array().unwrap().len(), 1);
    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("was not found")));
    assert!(!fixture.user_config.exists());
}

#[test]
fn no_user_config_path_composes_from_the_inline_overlay() {
    let fixture = fixture();
    let composed = compose(
        &options(&fixture).with_inline_config(
            json!({
                "plugins": [{"package": "./plugins/overlay"}],
                "tabs": {"mode": "tab"}
            })
            .to_string(),
        ),
    )
    .expect("compose");
    let config = parse(&composed);
    assert_eq!(config["plugins"][0]["package"], "./plugins/overlay");
    assert_eq!(config["plugins"][1]["package"], canonical_plugin(&fixture));
}

#[test]
fn non_object_tabs_are_replaced_with_a_warning() {
    let fixture = fixture();
    write_json(&fixture.user_config, &json!({"tabs": "enabled"}));
    let composed =
        compose(&options(&fixture).with_user_config_path(&fixture.user_config)).expect("compose");
    assert_eq!(parse(&composed)["tabs"]["mode"], "on");
    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("non-object tabs")));
}

#[test]
fn invalid_bridge_url_is_rejected_before_injection() {
    let fixture = fixture();
    let mut options = options(&fixture);
    options.bridge_url = "ws://example.com:1234/made".into();
    let error = compose(&options).expect_err("non-loopback URL");
    assert!(matches!(error, ConfigError::InvalidBridgeUrl { .. }));
}

#[test]
fn compose_never_writes_the_user_config() {
    let fixture = fixture();
    let content = format!(
        "{{\n  \"plugins\": [],\n  \"theme\": \"dark\"\n}}\n{}",
        " ".repeat(32)
    );
    fs::write(&fixture.user_config, &content).unwrap();
    let before = fs::read(&fixture.user_config).unwrap();

    compose(&options(&fixture).with_user_config_path(&fixture.user_config)).expect("compose");

    assert_eq!(fs::read(&fixture.user_config).unwrap(), before);
    assert_eq!(fs::read_to_string(&fixture.user_config).unwrap(), content);
}

#[test]
fn environment_pair_targets_opencode_inline_config() {
    use made_bridge::config::CONFIG_CONTENT_ENV;

    let fixture = fixture();
    let composed = compose(&options(&fixture)).expect("compose");
    let (key, value) = composed.env_pair();
    assert_eq!(key, CONFIG_CONTENT_ENV);
    assert_eq!(value, composed.content);
    let environment = composed.environment();
    assert_eq!(environment.get(CONFIG_CONTENT_ENV), Some(&composed.content));
}

#[test]
fn debug_output_never_contains_the_token() {
    let fixture = fixture();
    let token = fixture.token.expose_secret().to_owned();
    let mut options = options(&fixture);
    options.inline_config = Some(json!({"token": token.clone()}).to_string());

    let options_debug = format!("{options:?}");
    assert!(!options_debug.contains(&token));

    let composed = compose(&options).expect("compose");
    let composed_debug = format!("{composed:?}");
    assert!(!composed_debug.contains(&token));

    let token_debug = format!("{:?}", fixture.token);
    let token_display = format!("{}", fixture.token);
    assert!(!token_debug.contains(&token));
    assert!(!token_display.contains(&token));
}

#[test]
fn long_but_valid_titles_do_not_affect_unrelated_config() {
    let fixture = fixture();
    let title = "t".repeat(512);
    write_json(
        &fixture.user_config,
        &json!({"plugins": [{"package": "./user", "options": {"title": title}}]}),
    );
    let composed =
        compose(&options(&fixture).with_user_config_path(&fixture.user_config)).expect("compose");
    assert_eq!(parse(&composed)["plugins"][0]["options"]["title"], title);
}

#[test]
fn relative_made_entry_is_resolved_against_the_user_config_parent() {
    let fixture = fixture();
    let user_config = fixture.root.join("cli.json");
    write_json(
        &user_config,
        &json!({
            "plugins": [
                {"package": "./plugins/opencode-made", "options": {"made": {"token": "old"}}},
                {"package": "./plugins/user-plugin", "options": {"keep": true}}
            ]
        }),
    );

    let composed = compose(&options(&fixture).with_user_config_path(&user_config))
        .expect("relative duplicate composes");
    let config = parse(&composed);
    let plugins = config["plugins"].as_array().expect("plugins array");

    assert_eq!(
        plugins.len(),
        2,
        "the Made entry must be replaced: {plugins:?}"
    );
    assert_eq!(plugins[0]["package"], "./plugins/user-plugin");
    assert_eq!(plugins[0]["options"]["keep"], true);
    assert_eq!(plugins[1]["package"], canonical_plugin(&fixture));
    assert!(composed
        .warnings
        .iter()
        .any(|warning| warning.contains("Removed 1 existing Made plugin")));
}

#[test]
fn made_package_name_and_id_references_are_removed() {
    let fixture = fixture();
    write_json(
        &fixture.user_config,
        &json!({
            "plugins": [
                "opencode-made",
                "made.opencode.bridge",
                {"package": "@made/opencode-made", "options": {"made": {"token": "old"}}},
                {"package": "user-plugin", "options": {"keep": true}}
            ]
        }),
    );

    let composed =
        compose(&options(&fixture).with_user_config_path(&fixture.user_config)).expect("compose");
    let plugins = parse(&composed)["plugins"].as_array().unwrap().clone();

    assert_eq!(
        plugins.len(),
        2,
        "all Made references must be removed: {plugins:?}"
    );
    assert_eq!(plugins[0]["package"], "user-plugin");
    assert_eq!(plugins[1]["package"], canonical_plugin(&fixture));
}

#[test]
fn plugin_package_shape_requires_package_json_and_tui_ts() {
    let without_package = fixture();
    fs::remove_file(without_package.plugin.join("package.json")).unwrap();
    let error = compose(&options(&without_package)).expect_err("missing package.json");
    assert!(matches!(
        error,
        ConfigError::PluginMissingFile {
            file: "package.json",
            ..
        }
    ));

    let without_tui = fixture();
    fs::remove_file(without_tui.plugin.join("tui.ts")).unwrap();
    let error = compose(&options(&without_tui)).expect_err("missing tui.ts");
    assert!(matches!(
        error,
        ConfigError::PluginMissingFile { file: "tui.ts", .. }
    ));
}

#[test]
fn debug_output_never_contains_composed_content() {
    let fixture = fixture();
    let marker = "COMPOSED-CONFIG-CONTENT-MARKER";
    let overlay = json!({"marker": marker, "token": fixture.token.expose_secret()}).to_string();
    let options = options(&fixture).with_inline_config(overlay);

    let options_debug = format!("{options:?}");
    assert!(!options_debug.contains(marker));

    let composed = compose(&options).expect("compose");
    assert!(composed.content.contains(marker));
    let composed_debug = format!("{composed:?}");
    assert!(!composed_debug.contains(marker));
    assert!(!composed_debug.contains(fixture.token.expose_secret()));
}
