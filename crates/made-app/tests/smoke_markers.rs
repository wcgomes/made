//! Pin of the smoke-layout markers line, written against the code before the
//! integration-module refactor (agent-integration-modules, phase 0).
//!
//! The installed-layout smoke check (REL-03, REL-11) parses this one line, so
//! its format is a fixed template:
//!
//! `version={} opencode_plugin={} grok_status={} companions={}/4 askpass={}
//! updates={} update_repo={} update_seam={}`
//!
//! Field names, order and separators are exact, the slots are filled from a
//! fixed packaged layout, and there is no Claude Code field. The expected text
//! below does not change during the refactor; only the adapter block does.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

// BEGIN characterization adapter
use made_app::app::{AppModel, AppOptions};
use made_app::workspace::WorkspaceLaunch;
use made_app::CompatibleTui;

/// The smoke line of a model whose catalog lists every integration: the line
/// must not depend on which integrations are detected.
fn smoke_line() -> String {
    AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([
            CompatibleTui::OpenCode,
            CompatibleTui::Grok,
            CompatibleTui::ClaudeCode,
        ]),
    )
    .smoke_layout_markers()
}

/// The same line for a model with an empty catalog.
fn smoke_line_without_catalog() -> String {
    AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new())
        .smoke_layout_markers()
}

/// Write the files of the packaged layout that belong to integrations: the
/// OpenCode plugin package, and with `full` the Grok Build hook and plugin
/// manifest.
fn stage_integration_files(install: &Path, full: bool) {
    for file in made_bridge::config::PLUGIN_REQUIRED_FILES {
        write_file(&install.join("plugins/opencode-made").join(file), "{}\n");
    }
    if full {
        write_file(
            &install.join("plugins/grok-made/hooks/bin/made-grok-status"),
            "#!/bin/sh\nexit 0\n",
        );
        write_file(&install.join("plugins/grok-made/plugin.json"), "{}\n");
    }
}
// END characterization adapter

const CHILD_ENV: &str = "MADE_SMOKE_MARKERS_CHILD";
const LAYOUT_ENV: &str = "MADE_SMOKE_MARKERS_LAYOUT";

const COMPANION_TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Plugin package, Grok Build hook, askpass helper and all four companions.
    Packaged,
    /// Plugin package only.
    PluginOnly,
}

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Re-run `test` in a child process whose install root is a fresh directory
/// holding exactly `layout`. Returns `true` in the parent after the child
/// passed.
fn isolated(test: &str, layout: Layout) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let install = home.path().join("install");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("smoke-markers");
    let current = std::env::current_exe().unwrap();
    if fs::hard_link(&current, &executable).is_err() {
        fs::copy(&current, &executable).unwrap();
    }
    stage_integration_files(&install, layout == Layout::Packaged);
    if layout == Layout::Packaged {
        write_file(
            &install.join(made_platform::askpass_file_name()),
            "#!/bin/sh\nexit 0\n",
        );
        for target in COMPANION_TARGETS {
            write_file(
                &install.join("remote").join(target).join("made-remote"),
                "#!/bin/sh\nexit 0\n",
            );
        }
    }
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    for directory in [&tmp, &runtime] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(&executable)
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(
            LAYOUT_ENV,
            if layout == Layout::Packaged {
                "packaged"
            } else {
                "plugin-only"
            },
        )
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove(made_platform::PLUGIN_DIR_ENV)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// The fixed template with its slots filled.
fn template(
    plugin: &str,
    grok_status: &str,
    companions: usize,
    askpass: &str,
    updates: &str,
) -> String {
    format!(
        "version={} opencode_plugin={plugin} grok_status={grok_status} companions={companions}/4 askpass={askpass} updates={updates} update_repo={} update_seam={}",
        made_app::APP_VERSION,
        made_platform::update::UPDATE_REPOSITORY,
        u8::from(made_platform::update::TEST_SEAM_ENABLED),
    )
}

#[test]
fn the_smoke_line_of_a_packaged_layout_matches_the_fixed_template() {
    if isolated(
        "the_smoke_line_of_a_packaged_layout_matches_the_fixed_template",
        Layout::Packaged,
    ) {
        return;
    }
    assert_eq!(
        std::env::var(LAYOUT_ENV).as_deref(),
        Ok("packaged"),
        "the child runs the packaged layout"
    );
    let expected = template("installed", "present", 4, "found", "disabled");
    assert_eq!(smoke_line(), expected);
    assert_eq!(smoke_line_without_catalog(), expected);
    assert_eq!(
        expected
            .split(' ')
            .map(|field| field.split('=').next().unwrap())
            .collect::<Vec<_>>(),
        [
            "version",
            "opencode_plugin",
            "grok_status",
            "companions",
            "askpass",
            "updates",
            "update_repo",
            "update_seam"
        ],
        "field names and order are exact"
    );
    assert!(
        !expected.to_lowercase().contains("claude"),
        "the smoke line has no Claude Code field"
    );
}

#[test]
fn the_smoke_line_of_a_plugin_only_layout_reports_the_missing_parts() {
    if isolated(
        "the_smoke_line_of_a_plugin_only_layout_reports_the_missing_parts",
        Layout::PluginOnly,
    ) {
        return;
    }
    assert_eq!(
        std::env::var(LAYOUT_ENV).as_deref(),
        Ok("plugin-only"),
        "the child runs the plugin-only layout"
    );
    let expected = template("installed", "missing", 0, "missing", "disabled");
    assert_eq!(smoke_line(), expected);
    assert_eq!(smoke_line_without_catalog(), expected);
}
