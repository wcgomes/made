#![cfg(unix)]
//! Shipped `grok-made` install after PATH detection reports grok.
//!
//! The fake `grok` and `GROK_HOME` live under a temporary directory. Nothing
//! here writes the user's real `~/.grok`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions};
use made_app::workspace::{LaunchPlan, WorkspaceLaunch, WorkspaceSource};
use made_app::{CompatibleTui, ExplorerPane, Workbench};
use made_platform::Platform;

#[test]
fn shipped_grok_plugin_install_follows_the_detected_version() {
    if std::env::var_os("MADE_GROK_PLUGIN_CASE").is_some() {
        run_case();
        return;
    }
    for case in ["missing", "lexical-older", "equal", "newer"] {
        spawn_case(case);
    }
}

fn spawn_case(case: &str) {
    let home = tempfile::tempdir().expect("temp home");
    let bin = home.path().join("bin");
    let grok_home = home.path().join("grok-home");
    let install_args = home.path().join("install-args");
    let install_argv0 = home.path().join("install-argv0");
    let install_phase = home.path().join("install-phase");
    let install_release = home.path().join("install-release");
    let opencode_env = home.path().join("opencode-env");
    let launch_args = home.path().join("launch-args");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&grok_home).unwrap();
    let grok_bin = bin.join("grok");
    std::fs::write(
        &grok_bin,
        "#!/bin/sh\n\
if [ \"$1\" = plugin ] && [ \"$2\" = install ]; then\n\
  printf '%s\\n' \"$0\" >> \"$MADE_GROK_INSTALL_ARGV0\"\n\
  printf '%s\\n' \"$*\" >> \"$MADE_GROK_INSTALL_ARGS\"\n\
  if [ -n \"${OPENCODE_CLI_CONFIG_CONTENT+x}\" ]; then printf set > \"$MADE_GROK_OPENCODE_ENV\"; else printf unset > \"$MADE_GROK_OPENCODE_ENV\"; fi\n\
  printf start > \"$MADE_GROK_INSTALL_PHASE\"\n\
  i=0\n\
  while [ ! -f \"$MADE_GROK_INSTALL_RELEASE\" ] && [ \"$i\" -lt 200 ]; do\n\
    sleep 0.05\n\
    i=$((i + 1))\n\
  done\n\
  exit 0\n\
fi\n\
printf 'args:%s\\n' \"$*\" >> \"$MADE_GROK_LAUNCH_ARGS\"\n\
sleep 30\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&grok_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin.clone()).chain(std::env::split_paths(&path));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "shipped_grok_plugin_install_follows_the_detected_version",
            "--nocapture",
        ])
        .env("MADE_GROK_PLUGIN_CASE", case)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GROK_HOME", &grok_home)
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", home.path().join("tmp"))
        .env("XDG_RUNTIME_DIR", home.path().join("runtime"))
        .env("APPDATA", home.path().join("AppData").join("Roaming"))
        .env("LOCALAPPDATA", home.path().join("AppData").join("Local"))
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SHELL", "/bin/bash")
        .env("MADE_GROK_BIN", &grok_bin)
        .env("MADE_GROK_INSTALL_ARGS", &install_args)
        .env("MADE_GROK_INSTALL_ARGV0", &install_argv0)
        .env("MADE_GROK_INSTALL_PHASE", &install_phase)
        .env("MADE_GROK_INSTALL_RELEASE", &install_release)
        .env("MADE_GROK_OPENCODE_ENV", &opencode_env)
        .env("MADE_GROK_LAUNCH_ARGS", &launch_args);
    command.env_remove(made_bridge::config::CONFIG_CONTENT_ENV);
    command.env_remove(made_platform::SHELL_ENV);
    command.env_remove("MADE_PLUGIN_DIR");
    for directory in [
        home.path().join("tmp"),
        home.path().join("runtime"),
        home.path().join("AppData").join("Roaming"),
        home.path().join("AppData").join("Local"),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            home.path().join("runtime"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{case}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_case() {
    let case = std::env::var("MADE_GROK_PLUGIN_CASE").unwrap();
    std::env::set_var(
        made_bridge::config::CONFIG_CONTENT_ENV,
        "leaked-plugin-config",
    );
    let _release = Release(PathBuf::from(
        std::env::var_os("MADE_GROK_INSTALL_RELEASE").unwrap(),
    ));
    match case.as_str() {
        "missing" => expect_install(None, None),
        "lexical-older" => {
            let shipped = plant_controlled_shipped("0.10.0");
            expect_install(Some("0.2.0"), Some(shipped));
        }
        "equal" => {
            let shipped = plant_controlled_shipped("0.10.0");
            expect_skip("0.10.0", &shipped);
        }
        "newer" => {
            let shipped = plant_controlled_shipped("0.10.0");
            expect_skip("0.10.1", &shipped);
        }
        other => panic!("unknown case {other}"),
    }
}

struct Release(PathBuf);

impl Drop for Release {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, b"release");
    }
}

fn expect_install(installed: Option<&str>, shipped: Option<PathBuf>) {
    let config = grok_config_path();
    if let Some(version) = installed {
        plant_installed(version);
    }
    let before = std::fs::read(&config).ok();
    let shipped = shipped.unwrap_or_else(resolved_grok_plugin_dir);
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    model.open_plan(LaunchPlan::open(root.path(), WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    let ctx = egui::Context::default();
    let shell_id = model.workspace_ids().next().unwrap();
    let startup = Instant::now();
    wait_until(
        Duration::from_secs(6),
        "shell and explorer waited on plugin install",
        || {
            model.logic(&ctx);
            shell_ready(&model)
        },
    );
    assert!(
        startup.elapsed() < Duration::from_secs(6) && !release_exists(),
        "shell and explorer must be up while grok plugin install is still running"
    );
    let install = Instant::now();
    wait_until(
        Duration::from_secs(3),
        "detected grok did not run plugin install",
        || {
            model.logic(&ctx);
            install_started()
        },
    );
    assert!(
        install.elapsed() < Duration::from_secs(3) && !release_exists(),
        "the install command must be running, not finished, before Grok starts"
    );
    model.begin_tui_workspace(CompatibleTui::Grok);
    let grok_startup = Instant::now();
    wait_until(
        Duration::from_secs(6),
        "Grok startup waited on plugin install",
        || {
            model.logic(&ctx);
            model.workspace_ids().any(|id| {
                id != shell_id
                    && model
                        .workspace(id)
                        .and_then(Workbench::terminal)
                        .is_some_and(|terminal| terminal.is_running())
            })
        },
    );
    assert!(
        grok_startup.elapsed() < Duration::from_secs(6) && !release_exists(),
        "menu-launched Grok must start while grok plugin install is still running"
    );
    let grok_id = model
        .workspace_ids()
        .find(|id| *id != shell_id)
        .expect("grok tab");
    let terminal = model
        .workspace(grok_id)
        .and_then(Workbench::terminal)
        .expect("grok terminal");
    assert_eq!(
        terminal.options().args,
        vec![OsString::from("dashboard")],
        "a menu-launched Grok child is the absolute path plus dashboard"
    );
    assert!(
        terminal
            .options()
            .env
            .iter()
            .all(|(key, _)| key != made_bridge::config::CONFIG_CONTENT_ENV),
        "Grok must not receive OpenCode's plugin environment"
    );
    let program = terminal
        .options()
        .shell
        .clone()
        .expect("absolute grok path");
    assert!(
        Path::new(&program).is_absolute(),
        "menu-launched Grok must be the absolute detected path"
    );
    let args = std::fs::read_to_string(env_path("MADE_GROK_INSTALL_ARGS")).unwrap();
    let argv0 = std::fs::read_to_string(env_path("MADE_GROK_INSTALL_ARGV0")).unwrap();
    assert_eq!(args.lines().count(), 1, "install must run once: {args}");
    assert_eq!(
        args.trim(),
        format!("plugin install {} --trust", shipped.display())
    );
    assert_eq!(argv0.trim(), program.to_string_lossy());
    assert_eq!(
        std::fs::read_to_string(env_path("MADE_GROK_OPENCODE_ENV")).unwrap(),
        "unset"
    );
    assert_eq!(
        std::fs::read(&config).ok(),
        before,
        "Made must not hand-edit config.toml"
    );
}

fn expect_skip(installed: &str, shipped: &Path) {
    plant_installed(installed);
    let manifest = installed_manifest();
    let before_manifest = std::fs::read(&manifest).unwrap();
    let config = grok_config_path();
    let before_config = std::fs::read(&config).ok();
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path()).unwrap();
    model.open_plan(LaunchPlan::open(root.path(), WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        model.logic(&ctx);
        assert!(
            !install_started(),
            "version {} must not run grok plugin install (shipped {})",
            installed,
            shipped.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell_ready(&model),
        "shell and explorer must start without an install"
    );
    assert_eq!(std::fs::read(&manifest).unwrap(), before_manifest);
    assert_eq!(std::fs::read(&config).ok(), before_config);
    assert!(
        !env_path("MADE_GROK_LAUNCH_ARGS").exists(),
        "skipping install must not launch grok"
    );
}

fn wait_until(limit: Duration, message: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn shell_ready(model: &AppModel) -> bool {
    model.workbench().is_some_and(|workbench| {
        workbench.terminal().is_some()
            && workbench
                .explorer()
                .is_some_and(ExplorerPane::is_root_ready)
    })
}

fn install_started() -> bool {
    std::fs::read_to_string(env_path("MADE_GROK_INSTALL_PHASE"))
        .ok()
        .is_some_and(|text| text.trim() == "start")
}

fn release_exists() -> bool {
    env_path("MADE_GROK_INSTALL_RELEASE").is_file()
}

fn env_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key} missing")))
}

fn grok_config_path() -> PathBuf {
    env_path_var("GROK_HOME").join("config.toml")
}

fn env_path_var(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap())
}

fn installed_manifest() -> PathBuf {
    env_path_var("GROK_HOME")
        .join("installed-plugins")
        .join("grok-made-test")
        .join("plugin.json")
}

fn plant_installed(version: &str) {
    let manifest = installed_manifest();
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest,
        format!("{{\n  \"name\": \"grok-made\",\n  \"version\": \"{version}\"\n}}\n"),
    )
    .unwrap();
}

fn plant_controlled_shipped(version: &str) -> PathBuf {
    let root = env_path_var("HOME").join("shipped-plugins");
    let opencode = root.join("opencode-made");
    let grok = root.join("grok-made");
    std::fs::create_dir_all(&opencode).unwrap();
    std::fs::create_dir_all(&grok).unwrap();
    std::fs::write(
        grok.join("plugin.json"),
        format!("{{\n  \"name\": \"grok-made\",\n  \"version\": \"{version}\"\n}}\n"),
    )
    .unwrap();
    std::env::set_var("MADE_PLUGIN_DIR", &opencode);
    std::fs::canonicalize(&grok).unwrap()
}

fn resolved_grok_plugin_dir() -> PathBuf {
    let paths = made_platform::host_platform()
        .app_paths()
        .expect("app paths");
    std::fs::canonicalize(paths.plugin_dir.with_file_name("grok-made")).expect("shipped grok-made")
}
