//! Platform contract tests.
//!
//! The PTY portions run on POSIX hosts and spawn a real `/bin/sh` in a real
//! PTY: cwd, echo, resize, termination, and child reaping are observed, not
//! mocked. Clipboard tests use the in-memory implementation and never require a
//! display server. Path policy is exercised for all three OS families without
//! mutating process-wide environment variables.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use made_platform::{
    append_private_log_rotated, append_private_log_rotated_with_rename,
    append_private_log_rotated_with_timeout, host_platform, install_root_for_executable,
    is_sensitive_env_key, open_private_log, previous_private_log_path, private_log_lock_path,
    repository_plugin_dir, repository_root, resolve_user_home, validate_user_home, AppPaths,
    Clipboard, Environment, HomeDirectoryErrorKind, ImeComposition, ImeEvent, InMemoryClipboard,
    LogAppendOutcome, LogIssue, LogIssueSeverity, PathsInputs, Platform, PlatformFamily,
    PluginDirSource, PtyRequest, PtySize, REDACTED_VALUE,
};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn unique_temp(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "made-contract-{label}-{}-{nanos}-{}",
        std::process::id(),
        UNIQUE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    let path = path.canonicalize().unwrap();
    path
}

fn path_inputs(family: PlatformFamily) -> PathsInputs {
    PathsInputs {
        family,
        home: Some(PathBuf::from("/home/tester")),
        xdg_data_home: Some(PathBuf::from("/xdg/data")),
        xdg_state_home: Some(PathBuf::from("/xdg/state")),
        xdg_config_home: Some(PathBuf::from("/xdg/config")),
        xdg_runtime_dir: Some(PathBuf::from("/run/user/1000")),
        app_data: Some(PathBuf::from("/windows/roaming")),
        local_app_data: Some(PathBuf::from("/windows/local")),
        install_root: Some(PathBuf::from("/opt/made")),
        temp_dir: PathBuf::from("/tmp"),
        user_token: "1000".to_owned(),
        plugin_override: None,
        dev_plugin_dir: None,
        development: false,
    }
}

#[test]
fn home_resolution_prefers_home_on_linux_and_macos() {
    for family in [PlatformFamily::Linux, PlatformFamily::MacOs] {
        let home = resolve_user_home(
            family,
            Some(OsStr::new("/home/primary")),
            Some(OsStr::new("/home/fallback")),
        )
        .expect("home resolves");
        assert_eq!(home, PathBuf::from("/home/primary"), "{family}");
    }
}

#[test]
fn home_resolution_prefers_userprofile_on_windows() {
    let home = resolve_user_home(
        PlatformFamily::Windows,
        Some(OsStr::new(r"C:\Users\fallback")),
        Some(OsStr::new(r"C:\Users\primary")),
    )
    .expect("home resolves");
    assert_eq!(home, PathBuf::from(r"C:\Users\primary"));
}

#[test]
fn home_resolution_falls_back_and_ignores_empty_values() {
    assert_eq!(
        resolve_user_home(
            PlatformFamily::Windows,
            Some(OsStr::new("/home/fallback")),
            None,
        )
        .expect("windows falls back to HOME"),
        PathBuf::from("/home/fallback")
    );
    assert_eq!(
        resolve_user_home(
            PlatformFamily::Linux,
            Some(OsStr::new("")),
            Some(OsStr::new("/home/fallback")),
        )
        .expect("empty HOME is ignored"),
        PathBuf::from("/home/fallback")
    );
    assert_eq!(
        resolve_user_home(PlatformFamily::MacOs, None, Some(OsStr::new("   ")),)
            .expect("a whitespace path is not empty"),
        PathBuf::from("   ")
    );
}

#[test]
fn home_resolution_reports_unset_values() {
    for family in [
        PlatformFamily::Linux,
        PlatformFamily::MacOs,
        PlatformFamily::Windows,
    ] {
        let error = resolve_user_home(family, None, None).unwrap_err();
        assert_eq!(error.kind(), HomeDirectoryErrorKind::Unresolved);
        assert_eq!(error.kind().code(), "home_unresolved");
        assert!(error.path().is_none());
        assert!(error.detail().contains("unset or empty"));
    }
}

#[test]
fn validate_user_home_accepts_an_existing_directory() {
    let home = temp_dir("home-valid");
    std::fs::create_dir_all(&home).unwrap();
    let validated = validate_user_home(&home).expect("valid home");
    assert_eq!(validated, home);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn validate_user_home_classifies_missing_and_non_directory_paths() {
    let root = temp_dir("home-invalid");
    std::fs::create_dir_all(&root).unwrap();
    let missing = root.join("missing");
    let file = root.join("file");
    std::fs::write(&file, "content").unwrap();

    let error = validate_user_home(&missing).unwrap_err();
    assert_eq!(error.kind(), HomeDirectoryErrorKind::Missing);
    assert_eq!(error.kind().code(), "home_missing");
    assert_eq!(error.path(), Some(missing.as_path()));

    let error = validate_user_home(&file).unwrap_err();
    assert_eq!(error.kind(), HomeDirectoryErrorKind::NotADirectory);
    assert_eq!(error.kind().code(), "home_not_a_directory");

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn validate_user_home_classifies_an_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;

    let home = temp_dir("home-unreadable");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::read_dir(&home).is_ok();
    if readable {
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        eprintln!("skipping unreadable home assertion: the current user can read mode-000");
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let error = validate_user_home(&home).unwrap_err();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.kind(), HomeDirectoryErrorKind::Unreadable);
    assert_eq!(error.kind().code(), "home_unreadable");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn host_platform_home_capability_is_usable_or_classified() {
    match host_platform().home_dir() {
        Ok(home) => assert!(home.is_dir()),
        Err(error) => assert!(matches!(
            error.kind(),
            HomeDirectoryErrorKind::Unresolved
                | HomeDirectoryErrorKind::Missing
                | HomeDirectoryErrorKind::NotADirectory
                | HomeDirectoryErrorKind::Unreadable
        )),
    }
}

#[test]
fn workspace_home_policy_stays_behind_the_platform_boundary() {
    let source = include_str!("../../made-app/src/workspace.rs");
    assert!(!source.contains("\"HOME\""));
    assert!(!source.contains("\"USERPROFILE\""));
    assert!(!source.contains("env::var_os(\"HOME\")"));
}

fn temp_dir(label: &str) -> PathBuf {
    unique_temp(label)
}

#[test]
fn host_backend_reports_a_family() {
    let platform = host_platform();
    assert_eq!(platform.family(), PlatformFamily::current());
}

#[test]
fn shell_resolution_prefers_the_explicit_request() {
    let platform = host_platform();
    let default = platform.resolve_shell(None).expect("default shell");
    assert!(!default.program.is_empty());

    let explicit = platform
        .resolve_shell(Some(OsStr::new("/made/test/shell")))
        .expect("explicit shell");
    assert_eq!(explicit.program, OsString::from("/made/test/shell"));
    assert!(explicit.args.is_empty());
}

#[cfg(unix)]
#[test]
fn posix_default_shell_is_an_existing_executable() {
    let platform = host_platform();
    let shell = platform.resolve_shell(None).expect("default shell");
    assert!(
        std::path::Path::new(&shell.program).is_file(),
        "default shell {:?} must exist",
        shell.program
    );
}

#[test]
fn environment_contains_terminal_defaults_and_honors_overrides() {
    let platform = host_platform();
    let environment = platform.environment(
        &[(OsString::from("MADE_CONTRACT_KEEP"), OsString::from("kept"))],
        &[OsString::from("TERM")],
    );

    assert_eq!(
        environment.get("TERM"),
        None,
        "explicit removal must win over terminal defaults"
    );
    assert_eq!(environment.get("COLORTERM"), Some(OsStr::new("truecolor")));
    assert_eq!(environment.get("TERM_PROGRAM"), Some(OsStr::new("Made")));
    assert_eq!(
        environment.get("MADE_CONTRACT_KEEP"),
        Some(OsStr::new("kept"))
    );
    assert!(environment
        .get("PATH")
        .or_else(|| environment.get("Path"))
        .is_some());
}

#[test]
fn environment_snapshot_round_trips() {
    let environment = Environment::from_pairs([("A", "1"), ("B", "2")]);
    assert_eq!(environment.get("A"), Some(OsStr::new("1")));
    assert_eq!(environment.vars().len(), 2);
    let owned = environment.into_vec();
    assert!(owned.iter().any(|(key, value)| key == "B" && value == "2"));
}

#[test]
fn linux_paths_follow_xdg() {
    let paths = AppPaths::resolve(&path_inputs(PlatformFamily::Linux)).expect("linux paths");
    assert_eq!(paths.install_root, PathBuf::from("/opt/made"));
    assert_eq!(paths.resource_dir, paths.install_root);
    assert_eq!(paths.data_dir, PathBuf::from("/xdg/data/made"));
    assert_eq!(paths.log_dir, PathBuf::from("/xdg/state/made/log"));
    assert_eq!(paths.temp_dir, PathBuf::from("/run/user/1000/made-1000"));
    assert_eq!(
        paths.plugin_dir,
        PathBuf::from("/opt/made/plugins/opencode-made")
    );
    assert_eq!(paths.plugin_root, PathBuf::from("/opt/made"));
    assert_eq!(paths.app_owned_root(), std::path::Path::new("/opt/made"));
    assert_eq!(paths.plugin_source, PluginDirSource::Installed);
    assert!(!paths.plugin_is_development_only());
    assert_eq!(
        paths.opencode_config_path,
        PathBuf::from("/xdg/config/opencode/cli.json")
    );
}

#[test]
fn linux_paths_fall_back_to_home_without_xdg() {
    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.xdg_data_home = None;
    inputs.xdg_state_home = None;
    inputs.xdg_config_home = None;
    inputs.xdg_runtime_dir = None;
    let paths = AppPaths::resolve(&inputs).expect("linux paths");
    assert_eq!(
        paths.data_dir,
        PathBuf::from("/home/tester/.local/share/made")
    );
    assert_eq!(
        paths.log_dir,
        PathBuf::from("/home/tester/.local/state/made/log")
    );
    assert_eq!(
        paths.opencode_config_path,
        PathBuf::from("/home/tester/.config/opencode/cli.json")
    );
    assert_eq!(paths.resource_dir, PathBuf::from("/opt/made"));
    assert_eq!(
        paths.temp_dir,
        PathBuf::from("/home/tester/.local/share/made/temp-1000"),
        "Linux without XDG_RUNTIME_DIR must fall back under the private data dir"
    );
}

#[test]
fn macos_paths_use_library_directories() {
    let paths = AppPaths::resolve(&path_inputs(PlatformFamily::MacOs)).expect("macos paths");
    assert_eq!(paths.install_root, PathBuf::from("/opt/made"));
    assert_eq!(
        paths.data_dir,
        PathBuf::from("/home/tester/Library/Application Support/Made")
    );
    assert_eq!(
        paths.log_dir,
        PathBuf::from("/home/tester/Library/Logs/Made")
    );
    assert_eq!(
        paths.plugin_dir,
        PathBuf::from("/opt/made/plugins/opencode-made")
    );
    assert_eq!(
        paths.opencode_config_path,
        PathBuf::from("/home/tester/Library/Application Support/opencode/cli.json")
    );
    assert_eq!(paths.temp_dir, PathBuf::from("/tmp/made-1000"));
}

#[test]
fn windows_paths_use_appdata_directories() {
    let paths = AppPaths::resolve(&path_inputs(PlatformFamily::Windows)).expect("windows paths");
    assert_eq!(paths.install_root, PathBuf::from("/opt/made"));
    assert_eq!(paths.data_dir, PathBuf::from("/windows/roaming/Made"));
    assert_eq!(paths.log_dir, PathBuf::from("/windows/local/Made/logs"));
    assert_eq!(
        paths.plugin_dir,
        PathBuf::from("/opt/made/plugins/opencode-made")
    );
    assert_eq!(
        paths.opencode_config_path,
        PathBuf::from("/windows/roaming/opencode/cli.json")
    );
    assert_eq!(paths.temp_dir, PathBuf::from("/tmp/made-1000"));
}

#[test]
fn xdg_config_override_is_linux_only() {
    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.xdg_config_home = Some(PathBuf::from("/custom/config"));
    let linux = AppPaths::resolve(&inputs).expect("linux paths");
    assert_eq!(
        linux.opencode_config_path,
        PathBuf::from("/custom/config/opencode/cli.json")
    );

    // macOS and Windows use their platform directories, not XDG.
    for family in [PlatformFamily::MacOs, PlatformFamily::Windows] {
        let paths = AppPaths::resolve(&path_inputs(family)).expect("platform paths");
        assert!(!paths.opencode_config_path.starts_with("/xdg/config"));
    }
}

#[test]
fn install_root_comes_from_the_executable_or_bundle() {
    assert_eq!(
        install_root_for_executable(
            PlatformFamily::MacOs,
            std::path::Path::new("/Applications/Made.app/Contents/MacOS/made")
        ),
        Some(PathBuf::from("/Applications/Made.app/Contents/Resources"))
    );
    assert_eq!(
        install_root_for_executable(
            PlatformFamily::MacOs,
            std::path::Path::new("/usr/local/bin/made")
        ),
        Some(PathBuf::from("/usr/local/bin"))
    );
    assert_eq!(
        install_root_for_executable(
            PlatformFamily::Windows,
            std::path::Path::new("C:/Program Files/Made/made.exe")
        ),
        Some(PathBuf::from("C:/Program Files/Made"))
    );
    assert_eq!(
        install_root_for_executable(
            PlatformFamily::Linux,
            std::path::Path::new("/opt/made/bin/made")
        ),
        Some(PathBuf::from("/opt/made/bin"))
    );
}

#[test]
fn development_plugin_uses_the_repository_package() {
    let repo_plugin = repository_plugin_dir();
    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = true;
    inputs.dev_plugin_dir = None;
    let paths = AppPaths::resolve(&inputs).expect("development paths");
    assert_eq!(paths.plugin_dir, repo_plugin);
    assert_eq!(paths.plugin_root, repository_root());
    assert_eq!(paths.plugin_source, PluginDirSource::DevelopmentRepository);
    assert!(paths.plugin_is_development_only());
}

fn write_plugin_package(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create plugin package");
    std::fs::write(dir.join("package.json"), "{\"name\":\"staged\"}").expect("write package.json");
    std::fs::write(dir.join("tui.ts"), "export default {};").expect("write tui.ts");
}

#[test]
fn development_prefers_a_complete_staged_install_plugin() {
    let root = unique_temp("plugin-staged");
    let install_root = root.join("install");
    let staged = install_root.join("plugins").join("opencode-made");
    write_plugin_package(&staged);

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = true;
    inputs.install_root = Some(install_root.clone());
    inputs.dev_plugin_dir = Some(repository_plugin_dir());
    let paths = AppPaths::resolve(&inputs).expect("staged development plugin");
    assert_eq!(paths.plugin_dir, staged);
    assert_eq!(paths.plugin_root, install_root);
    assert_eq!(paths.plugin_source, PluginDirSource::Installed);
    assert!(!paths.plugin_is_development_only());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn development_falls_back_to_the_repository_plugin_when_staged_is_incomplete() {
    let root = unique_temp("plugin-staged-incomplete");
    let install_root = root.join("install");
    let staged = install_root.join("plugins").join("opencode-made");
    std::fs::create_dir_all(&staged).expect("create plugin directory");
    std::fs::write(staged.join("package.json"), "{}").expect("write package.json");

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = true;
    inputs.install_root = Some(install_root);
    inputs.dev_plugin_dir = Some(repository_plugin_dir());
    let paths = AppPaths::resolve(&inputs).expect("repository fallback");
    assert_eq!(paths.plugin_dir, repository_plugin_dir());
    assert_eq!(paths.plugin_root, repository_root());
    assert_eq!(paths.plugin_source, PluginDirSource::DevelopmentRepository);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn development_plugin_override_is_marked_development_only() {
    let root = unique_temp("plugin-dev-override");
    let outside = root.join("staged").join("opencode-made");
    std::fs::create_dir_all(&outside).expect("create staged plugin");

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = true;
    inputs.dev_plugin_dir = Some(repository_plugin_dir());
    inputs.plugin_override = Some(outside.clone());
    let paths = AppPaths::resolve(&inputs).expect("development override");
    assert_eq!(paths.plugin_dir, outside);
    assert_eq!(paths.plugin_source, PluginDirSource::DevelopmentOverride);
    assert!(paths.plugin_is_development_only());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn release_override_must_be_inside_the_install_root() {
    let root = unique_temp("plugin-release");
    let install_root = root.join("install");
    let installed_plugin = install_root.join("plugins").join("opencode-made");
    std::fs::create_dir_all(&installed_plugin).expect("create installed plugin");
    let outside = root.join("user-writable").join("opencode-made");
    std::fs::create_dir_all(&outside).expect("create outside plugin");

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.install_root = Some(install_root.clone());
    inputs.plugin_override = Some(installed_plugin.clone());
    let paths = AppPaths::resolve(&inputs).expect("contained override");
    assert_eq!(
        paths.plugin_dir,
        std::fs::canonicalize(&installed_plugin).expect("canonical plugin")
    );
    assert_eq!(paths.plugin_source, PluginDirSource::Installed);
    assert_eq!(paths.plugin_root, install_root);
    assert!(!paths.plugin_is_development_only());

    inputs.plugin_override = Some(outside);
    assert!(
        AppPaths::resolve(&inputs).is_err(),
        "release builds must reject a plugin override outside the install root"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn release_override_symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("plugin-symlink");
    let install_root = root.join("install");
    let plugin_dir = install_root.join("plugins").join("opencode-made");
    std::fs::create_dir_all(&plugin_dir).expect("create install plugin");
    let outside = root.join("outside").join("opencode-made");
    std::fs::create_dir_all(&outside).expect("create outside plugin");
    let escape = install_root.join("plugins").join("escape");
    symlink(&outside, &escape).expect("create symlink");

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.install_root = Some(install_root);
    inputs.plugin_override = Some(escape);
    assert!(
        AppPaths::resolve(&inputs).is_err(),
        "a symlink that resolves outside the install root must be rejected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn packaged_plugin_lives_under_platform_install_roots() {
    let cases = [
        (
            PlatformFamily::MacOs,
            "/Applications/Made.app/Contents/MacOS/made",
            "/Applications/Made.app/Contents/Resources/plugins/opencode-made",
        ),
        (
            PlatformFamily::Windows,
            "C:/Program Files/Made/made.exe",
            "C:/Program Files/Made/plugins/opencode-made",
        ),
        (
            PlatformFamily::Linux,
            "/opt/made/bin/made",
            "/opt/made/bin/plugins/opencode-made",
        ),
    ];

    for (family, executable, expected_plugin) in cases {
        let install_root = install_root_for_executable(family, std::path::Path::new(executable))
            .expect("install root");
        let mut inputs = path_inputs(family);
        inputs.install_root = Some(install_root.clone());
        let paths = AppPaths::resolve(&inputs).expect("packaged paths");
        assert_eq!(paths.install_root, install_root);
        assert_eq!(paths.plugin_dir, PathBuf::from(expected_plugin));
        assert_eq!(paths.plugin_source, PluginDirSource::Installed);
    }
}

#[test]
fn release_paths_require_an_install_root() {
    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = false;
    inputs.install_root = None;
    assert!(
        AppPaths::resolve(&inputs).is_err(),
        "release resolution must fail closed without an install root"
    );
}

#[test]
fn development_paths_can_derive_the_install_root_from_the_repository() {
    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.development = true;
    inputs.install_root = None;
    inputs.dev_plugin_dir = None;
    let paths = AppPaths::resolve(&inputs).expect("development fallback");
    assert_eq!(paths.install_root, repository_root());
    assert_eq!(paths.plugin_dir, repository_plugin_dir());
    assert!(paths.plugin_is_development_only());
}

#[test]
fn development_policy_switch_ignores_installed_only_fields() {
    let root = unique_temp("plugin-release-installed");
    let install_root = root.join("install");
    std::fs::create_dir_all(install_root.join("plugins").join("opencode-made"))
        .expect("create installed plugin");

    let mut inputs = path_inputs(PlatformFamily::Linux);
    inputs.install_root = Some(install_root.clone());
    inputs.development = false;
    // A stale development path must not influence release resolution.
    inputs.dev_plugin_dir = Some(PathBuf::from("/repo/plugins/opencode-made"));
    let paths = AppPaths::resolve(&inputs).expect("release paths");
    assert_eq!(
        paths.plugin_dir,
        install_root.join("plugins").join("opencode-made")
    );
    assert_eq!(paths.plugin_source, PluginDirSource::Installed);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn repository_plugin_package_has_the_shape_opencode_loads() {
    let plugin_dir = repository_plugin_dir();
    assert!(
        plugin_dir.is_dir(),
        "development plugin package must exist at {}",
        plugin_dir.display()
    );
    assert!(
        plugin_dir.join("package.json").is_file(),
        "OpenCode loads a directory package, so package.json must be present"
    );
    assert!(
        plugin_dir.join("tui.ts").is_file(),
        "OpenCode's TUI entrypoint must live inside the package directory"
    );
    assert!(
        plugin_dir.join("src").join("integration.ts").is_file(),
        "implementation modules must be inside the package, not a cross-directory re-export"
    );
}

#[test]
fn host_paths_are_absolute_and_contain_the_plugin_location() {
    let platform = host_platform();
    let paths = platform.app_paths().expect("host paths");
    assert!(paths.install_root.is_absolute());
    assert_eq!(paths.resource_dir, paths.install_root);
    assert!(paths.data_dir.is_absolute());
    assert!(paths.plugin_dir.is_absolute());
    assert!(paths.log_dir.is_absolute());
    assert!(paths.temp_dir.is_absolute());
    assert!(paths.opencode_config_path.is_absolute());
    assert!(
        paths
            .temp_dir
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("made-")),
        "temp dir must carry a per-user suffix: {}",
        paths.temp_dir.display()
    );

    if paths.plugin_source == PluginDirSource::Installed {
        assert!(
            paths.plugin_dir.starts_with(&paths.install_root),
            "installed plugin must live under the install root"
        );
    } else {
        assert!(
            paths.plugin_is_development_only(),
            "non-installed plugin sources must be development-only"
        );
    }
}

#[test]
fn app_paths_creates_user_writable_directories_only() {
    let root = unique_temp("paths");
    let install_root = root.join("install");
    let inputs = PathsInputs {
        temp_dir: root.join("tmp"),
        home: Some(root.join("home")),
        xdg_data_home: Some(root.join("data")),
        xdg_state_home: Some(root.join("state")),
        xdg_runtime_dir: Some(root.join("runtime")),
        install_root: Some(install_root.clone()),
        ..path_inputs(PlatformFamily::Linux)
    };
    let paths = AppPaths::resolve(&inputs).expect("paths");
    paths.ensure_directories().expect("create app directories");
    assert!(paths.data_dir.is_dir());
    assert!(paths.log_dir.is_dir());
    assert!(paths.temp_dir.is_dir());
    assert!(
        !install_root.exists(),
        "installation data must never be created by the running app"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_appends_regular_file() {
    use std::io::Write as _;

    let root = unique_temp("private-log");
    let log = root.join("logs").join("made.log");
    {
        let mut file = open_private_log(&log).expect("create private log");
        file.write_all(b"one").expect("first append");
    }
    {
        let mut file = open_private_log(&log).expect("reopen private log");
        file.write_all(b"two").expect("second append");
    }
    assert_eq!(std::fs::read(&log).expect("read log"), b"onetwo");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_log_is_owner_only_and_rejects_symlinks_and_directories() {
    use std::io::Write as _;
    use std::os::unix::fs::{symlink, PermissionsExt};

    let root = unique_temp("private-log-mode");
    let log = root.join("logs").join("made.log");
    let mut file = open_private_log(&log).expect("create private log");
    file.write_all(b"secret").expect("append");
    drop(file);

    let log_mode = std::fs::metadata(&log)
        .expect("log metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(log_mode, 0o600, "log file must not be group/world readable");
    let dir_mode = std::fs::metadata(log.parent().expect("log parent"))
        .expect("log directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700, "log directory must be owner-only");

    let link = root.join("link.log");
    symlink(&log, &link).expect("create symlink");
    assert!(
        open_private_log(&link).is_err(),
        "a symlink must be rejected even when its target is a regular file"
    );

    let directory = root.join("directory.log");
    std::fs::create_dir_all(&directory).expect("create directory path");
    assert!(
        open_private_log(&directory).is_err(),
        "a directory must be rejected as a log file"
    );

    let _ = std::fs::remove_dir_all(&root);
}

fn seed_private_log(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write as _;
    let mut file = open_private_log(path).expect("seed private log");
    file.write_all(bytes).expect("seed append");
}

fn written(outcome: LogAppendOutcome) -> (bool, Option<LogIssue>) {
    match outcome {
        LogAppendOutcome::Written { rotated, issue } => (rotated, issue),
        LogAppendOutcome::Refused { issue } => {
            panic!("append was refused: {issue:?}")
        }
    }
}

fn refused(outcome: LogAppendOutcome) -> LogIssue {
    match outcome {
        LogAppendOutcome::Refused { issue } => {
            assert_eq!(issue.severity, LogIssueSeverity::Error, "{issue:?}");
            issue
        }
        LogAppendOutcome::Written { rotated, issue } => {
            panic!("append unexpectedly succeeded (rotated={rotated}, issue={issue:?})")
        }
    }
}

#[test]
fn private_log_rotation_respects_exact_threshold_and_replaces_only_backup() {
    let root = unique_temp("private-log-threshold");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    let lock = private_log_lock_path(&log);
    seed_private_log(&log, &[b'a'; 44]);
    std::fs::write(&backup, b"older").expect("seed backup");

    let at_limit = append_private_log_rotated(&log, &[b'b'; 20], 64).expect("append at limit");
    let (rotated, issue) = written(at_limit);
    assert!(!rotated, "equal to the limit must not rotate");
    assert!(issue.is_none());
    assert_eq!(std::fs::read(&log).expect("read current").len(), 64);
    assert_eq!(std::fs::read(&backup).expect("read backup"), b"older");

    let previous_current = std::fs::read(&log).expect("read current before rotation");
    #[cfg(unix)]
    let previous_inode = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&log).expect("current metadata").ino()
    };

    let over_limit =
        append_private_log_rotated(&log, b"c", 64).expect("append one byte over the limit");
    let (rotated, issue) = written(over_limit);
    assert!(rotated, "one byte over the limit must rotate");
    assert!(issue.is_none());
    assert_eq!(std::fs::read(&log).expect("read new current"), b"c");
    assert_eq!(
        std::fs::read(&backup).expect("read rotated backup"),
        previous_current,
        "the previous log must be byte-identical to the former current log"
    );
    assert!(lock.is_file(), "the persistent lock file must exist");

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let backup_metadata = std::fs::metadata(&backup).expect("backup metadata");
        assert_eq!(
            backup_metadata.ino(),
            previous_inode,
            "Unix rename must preserve the current log's inode as the previous log"
        );
        assert_eq!(backup_metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(&log)
                .expect("current metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&lock)
                .expect("lock metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(lock.parent().expect("lock parent"))
                .expect("lock dir metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_append_tightens_an_existing_previous_log_without_rotation() {
    let root = unique_temp("private-log-tighten-previous");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    seed_private_log(&log, b"current");
    std::fs::write(&backup, b"previous record\n").expect("seed previous");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644))
            .expect("loosen previous mode");
    }

    let outcome = append_private_log_rotated(&log, b"next", 1024).expect("append");
    let (rotated, issue) = written(outcome);
    assert!(!rotated, "the record must fit without rotation");
    assert!(issue.is_none(), "{issue:?}");
    assert_eq!(std::fs::read(&log).expect("read current"), b"currentnext");
    assert_eq!(
        std::fs::read(&backup).expect("read previous"),
        b"previous record\n",
        "tightening must not rewrite previous content"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&backup)
                .expect("previous metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_log_append_ignores_unsafe_previous_when_no_rotation_is_needed() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("private-log-unsafe-previous-no-rotation");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    seed_private_log(&log, b"current");
    let target = root.join("previous-target");
    std::fs::write(&target, b"target").expect("seed target");
    symlink(&target, &backup).expect("create previous symlink");

    let outcome = append_private_log_rotated(&log, b"next", 1024).expect("append");
    let (rotated, issue) = written(outcome);
    assert!(!rotated);
    assert!(issue.is_none(), "{issue:?}");
    assert_eq!(std::fs::read(&log).expect("read current"), b"currentnext");
    assert!(std::fs::symlink_metadata(&backup)
        .expect("previous metadata")
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&target).expect("read target"), b"target");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_rotation_creates_a_missing_current_log() {
    let root = unique_temp("private-log-missing");
    let log = root.join("logs").join("integration.log");

    let outcome = append_private_log_rotated(&log, b"first\n", 64).expect("append");
    let (rotated, issue) = written(outcome);
    assert!(!rotated);
    assert!(issue.is_none());
    assert_eq!(std::fs::read(&log).expect("read current"), b"first\n");
    assert!(!previous_private_log_path(&log).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_rotation_rotates_an_oversized_legacy_log() {
    let root = unique_temp("private-log-legacy");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    std::fs::create_dir_all(log.parent().expect("log parent")).expect("create log parent");
    {
        let file = std::fs::File::create(&log).expect("create legacy log");
        file.set_len(made_platform::DIAGNOSTICS_LOG_MAX_BYTES + 1)
            .expect("size legacy log");
    }

    let outcome = append_private_log_rotated(
        &log,
        b"new record\n",
        made_platform::DIAGNOSTICS_LOG_MAX_BYTES,
    )
    .expect("append after legacy log");
    let (rotated, issue) = written(outcome);
    assert!(rotated, "a legacy log above 1 MiB must rotate");
    assert!(issue.is_none());
    assert_eq!(
        std::fs::metadata(&backup).expect("backup metadata").len(),
        made_platform::DIAGNOSTICS_LOG_MAX_BYTES + 1,
        "a legacy oversized active file may survive as .1 without truncation"
    );
    assert!(
        std::fs::metadata(&log).expect("active metadata").len()
            < made_platform::DIAGNOSTICS_LOG_MAX_BYTES
    );
    assert_eq!(std::fs::read(&log).expect("read current"), b"new record\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&log, &backup, &private_log_lock_path(&log)] {
            assert_eq!(
                std::fs::metadata(path)
                    .expect("log metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
                "{} must be 0600",
                path.display()
            );
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_log_rotation_rejects_a_symlink_current_log() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("private-log-rotation-symlink");
    let log = root.join("logs").join("integration.log");
    std::fs::create_dir_all(log.parent().expect("log parent")).expect("create log parent");
    let target = root.join("target.log");
    std::fs::write(&target, b"target").expect("seed target");
    symlink(&target, &log).expect("create log symlink");

    let issue =
        refused(append_private_log_rotated(&log, b"record", 64).expect("symlink current append"));
    assert!(issue.message.contains("symlink"), "{issue:?}");
    assert_eq!(std::fs::read(&target).expect("read target"), b"target");
    assert!(!previous_private_log_path(&log).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_log_rotation_refuses_an_unsafe_backup_without_appending() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("private-log-unsafe-backup");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    seed_private_log(&log, &[b'a'; 61]);
    let target = root.join("backup-target.log");
    std::fs::write(&target, b"target").expect("seed target");
    symlink(&target, &backup).expect("create backup symlink");

    let issue =
        refused(append_private_log_rotated(&log, b"next", 64).expect("unsafe backup append"));
    assert!(issue.message.contains("not a regular file"), "{issue:?}");
    assert_eq!(std::fs::read(&target).expect("read target"), b"target");
    assert_eq!(
        std::fs::read(&log).expect("read current"),
        vec![b'a'; 61],
        "a refused rotation must not append"
    );
    assert!(std::fs::symlink_metadata(&backup)
        .expect("backup metadata")
        .file_type()
        .is_symlink());

    std::fs::remove_file(&backup).expect("remove backup symlink");
    std::fs::create_dir(&backup).expect("create backup directory");
    let issue =
        refused(append_private_log_rotated(&log, b"more", 64).expect("backup directory append"));
    assert!(issue.message.contains("not a regular file"), "{issue:?}");
    assert_eq!(
        std::fs::read(&log).expect("read current"),
        vec![b'a'; 61],
        "a refused rotation must not append"
    );
    assert!(
        backup.is_dir(),
        "the backup directory must be left untouched"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_refuses_a_failing_rename_without_touching_either_log() {
    let root = unique_temp("private-log-rename-failure");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    seed_private_log(&log, &[b'a'; 61]);

    let outcome = append_private_log_rotated_with_rename(
        &log,
        b"next",
        64,
        Duration::from_secs(1),
        |_, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected rename failure",
            ))
        },
    )
    .expect("public rename-failure append");

    let issue = refused(outcome);
    assert!(issue.message.contains("rotation refused"), "{issue:?}");
    assert_eq!(
        std::fs::read(&log).expect("read current"),
        vec![b'a'; 61],
        "a refused rename must leave the active log byte-identical"
    );
    assert!(!backup.exists(), "a refused rename must not create .1");
    assert!(!std::fs::read_dir(log.parent().unwrap())
        .unwrap()
        .any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with(".1")));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_log_refuses_unsafe_or_unreadable_lock_paths() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("private-log-lock-unsafe");
    let log = root.join("logs").join("integration.log");
    let lock = private_log_lock_path(&log);
    seed_private_log(&log, b"unchanged");
    let target = root.join("lock-target");
    std::fs::write(&target, b"target").expect("seed lock target");
    symlink(&target, &lock).expect("create lock symlink");

    let issue =
        refused(append_private_log_rotated(&log, b"record", 1024).expect("symlink lock append"));
    assert!(issue.message.contains("lock"), "{issue:?}");
    assert_eq!(std::fs::read(&target).expect("read target"), b"target");
    assert_eq!(std::fs::read(&log).expect("read current"), b"unchanged");

    std::fs::remove_file(&lock).expect("remove lock symlink");
    std::fs::create_dir(&lock).expect("create lock directory");
    let issue =
        refused(append_private_log_rotated(&log, b"record", 1024).expect("directory lock append"));
    assert!(issue.message.contains("lock"), "{issue:?}");
    assert_eq!(std::fs::read(&log).expect("read current"), b"unchanged");

    if unsafe { libc::geteuid() } != 0 {
        std::fs::remove_dir(&lock).expect("remove lock directory");
        std::fs::write(&lock, b"").expect("create lock file");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o000))
            .expect("make lock unreadable");
        let issue = refused(
            append_private_log_rotated(&log, b"record", 1024).expect("permission lock append"),
        );
        assert!(issue.message.contains("lock"), "{issue:?}");
        assert_eq!(std::fs::read(&log).expect("read current"), b"unchanged");
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600))
            .expect("restore lock mode");
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_lock_contention_times_out_without_appending() {
    let root = unique_temp("private-log-lock-timeout");
    let log = root.join("logs").join("integration.log");
    let lock = private_log_lock_path(&log);
    seed_private_log(&log, b"unchanged");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock)
        .expect("open held lock");
    held.lock().expect("hold lock");

    let started = std::time::Instant::now();
    let issue = refused(
        append_private_log_rotated_with_timeout(&log, b"record", 1024, Duration::from_millis(20))
            .expect("contended append"),
    );
    let waited = started.elapsed();
    assert!(issue.message.contains("lock"), "{issue:?}");
    assert!(waited >= Duration::from_millis(20), "waited {waited:?}");
    assert!(waited < Duration::from_secs(1), "waited {waited:?}");
    assert_eq!(std::fs::read(&log).expect("read current"), b"unchanged");

    held.unlock().expect("unlock held lock");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_concurrent_appends_are_record_atomic() {
    use std::sync::{Arc, Barrier};

    let root = unique_temp("private-log-concurrent");
    let log = root.join("logs").join("integration.log");
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for thread in 0..8 {
        let log = log.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let record = format!("record-{thread}-{}\n", "x".repeat(64));
            barrier.wait();
            let outcome = append_private_log_rotated(&log, record.as_bytes(), 1024 * 1024)
                .expect("concurrent append");
            let (rotated, issue) = written(outcome);
            assert!(!rotated);
            assert!(issue.is_none());
        }));
    }
    for handle in handles {
        handle.join().expect("append thread");
    }

    let text = std::fs::read_to_string(&log).expect("read concurrent log");
    for thread in 0..8 {
        let record = format!("record-{thread}-{}\n", "x".repeat(64));
        assert_eq!(
            text.matches(&record).count(),
            1,
            "record for thread {thread} must appear exactly once"
        );
    }
    assert_eq!(
        text.lines().count(),
        8,
        "no partial or interleaved lines: {text:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_concurrent_rotation_keeps_both_records_and_rotates_once() {
    use std::sync::{Arc, Barrier};

    let root = unique_temp("private-log-concurrent-rotation");
    let log = root.join("logs").join("integration.log");
    let backup = previous_private_log_path(&log);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for marker in ["thread-a\n", "thread-b\n"] {
        let log = log.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let outcome =
                append_private_log_rotated(&log, marker.as_bytes(), 12).expect("rotating append");
            let (rotated, issue) = written(outcome);
            assert!(issue.is_none());
            rotated
        }));
    }
    let rotations: usize = handles
        .into_iter()
        .map(|handle| usize::from(handle.join().expect("rotation thread")))
        .sum();

    assert_eq!(rotations, 1, "exactly one append must rotate");
    assert!(backup.is_file(), "rotation must create the previous log");
    let current = std::fs::read_to_string(&log).expect("read current");
    let previous = std::fs::read_to_string(&backup).expect("read previous");
    let combined = format!("{current}{previous}");
    assert_eq!(combined.matches("thread-a\n").count(), 1, "{combined:?}");
    assert_eq!(combined.matches("thread-b\n").count(), 1, "{combined:?}");
    let mut lines: Vec<&str> = combined.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        ["thread-a", "thread-b"],
        "no partial or interleaved lines: {combined:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_log_refuses_a_record_at_the_threshold() {
    let root = unique_temp("private-log-record-threshold");
    let log = root.join("logs").join("integration.log");
    let issue = refused(
        append_private_log_rotated(&log, &[b'x'; 64], 64).expect("record-at-threshold append"),
    );
    assert!(issue.message.contains("refusing"), "{issue:?}");
    assert!(!log.exists());
    assert!(!previous_private_log_path(&log).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn private_temp_path_rejects_a_regular_file() {
    let root = unique_temp("private-temp-file");
    let runtime = root.join("runtime");
    std::fs::create_dir_all(&runtime).expect("create runtime");
    let temp = runtime.join("made-1000");
    std::fs::write(&temp, b"not a directory").expect("create file at temp path");

    let inputs = PathsInputs {
        xdg_runtime_dir: Some(runtime),
        user_token: "1000".to_owned(),
        home: Some(root.join("home")),
        install_root: Some(root.join("install")),
        ..path_inputs(PlatformFamily::Linux)
    };
    assert!(
        AppPaths::resolve(&inputs).is_err(),
        "a regular file at the private temp path must be rejected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn private_temp_path_rejects_a_symlink() {
    use std::os::unix::fs::symlink;

    let root = unique_temp("private-temp-symlink");
    let runtime = root.join("runtime");
    std::fs::create_dir_all(&runtime).expect("create runtime");
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).expect("create outside");
    let temp = runtime.join("made-1000");
    symlink(&outside, &temp).expect("create temp symlink");

    let inputs = PathsInputs {
        xdg_runtime_dir: Some(runtime),
        user_token: "1000".to_owned(),
        home: Some(root.join("home")),
        install_root: Some(root.join("install")),
        ..path_inputs(PlatformFamily::Linux)
    };
    assert!(
        AppPaths::resolve(&inputs).is_err(),
        "a symlink at the private temp path must be rejected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn app_directories_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_temp("paths-mode");
    let inputs = PathsInputs {
        temp_dir: root.join("tmp"),
        home: Some(root.join("home")),
        xdg_data_home: Some(root.join("data")),
        xdg_state_home: Some(root.join("state")),
        xdg_runtime_dir: Some(root.join("runtime")),
        install_root: Some(root.join("install")),
        ..path_inputs(PlatformFamily::Linux)
    };
    let paths = AppPaths::resolve(&inputs).expect("paths");
    paths.ensure_directories().expect("create private dirs");
    for dir in [&paths.data_dir, &paths.log_dir, &paths.temp_dir] {
        let mode = std::fs::metadata(dir)
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "{} must be owner-only", dir.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn debug_output_redacts_config_content_and_tokens() {
    let token = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let config = format!(
        r#"{{"plugin":[{{"package":"made-opencode","options":{{"made":{{"token":"{token}"}}}}}}]}}"#
    );

    let environment = Environment::from_pairs([
        ("OPENCODE_CLI_CONFIG_CONTENT", config.as_str()),
        ("MADE_BRIDGE_TOKEN", token),
        ("PATH", "/usr/bin:/bin"),
    ]);
    let debug = format!("{environment:?}");
    assert!(
        !debug.contains(token),
        "token leaked through Environment: {debug}"
    );
    assert!(
        !debug.contains(&config),
        "config content leaked through Environment"
    );
    assert!(
        !debug.contains("/usr/bin:/bin"),
        "Debug must not print the full environment: {debug}"
    );
    assert!(debug.contains("OPENCODE_CLI_CONFIG_CONTENT"));
    assert!(debug.contains("MADE_BRIDGE_TOKEN"));
    assert!(debug.contains(REDACTED_VALUE));
    assert!(!format!("{environment}").contains(token));

    let mut request = PtyRequest::new("/bin/sh", "/work");
    request.env = environment.into_vec();
    request.args = vec![OsString::from("--token"), OsString::from(token)];
    request.size = PtySize { rows: 24, cols: 80 };
    let request_debug = format!("{request:?}");
    assert!(
        !request_debug.contains(token),
        "token leaked through PtyRequest: {request_debug}"
    );
    assert!(!request_debug.contains(&config));
    assert!(!request_debug.contains("--token"));
    assert!(request_debug.contains(REDACTED_VALUE));
    assert!(!format!("{request}").contains(token));

    assert!(is_sensitive_env_key("OPENCODE_CLI_CONFIG_CONTENT"));
    assert!(is_sensitive_env_key("MADE_BRIDGE_TOKEN"));
    assert!(is_sensitive_env_key("MADE_SESSION_TOKEN_PATH"));
    assert!(!is_sensitive_env_key("PATH"));
    assert!(!is_sensitive_env_key("MADE_PLUGIN_DIR"));
}

#[test]
fn in_memory_clipboard_round_trips_without_a_display() {
    let mut clipboard = InMemoryClipboard::new();
    assert_eq!(clipboard.get_text().expect("empty read"), "");
    clipboard.set_text("made clipboard").expect("write");
    assert_eq!(clipboard.get_text().expect("read"), "made clipboard");
    clipboard.set_text("replaced").expect("replace");
    assert_eq!(clipboard.get_text().expect("read"), "replaced");
    assert_eq!(
        InMemoryClipboard::with_text("seed").get_text().unwrap(),
        "seed"
    );

    let debug = format!("{:?}", InMemoryClipboard::with_text("clipboard-secret"));
    assert!(!debug.contains("clipboard-secret"));
    assert!(debug.contains("text_len"));
}

#[test]
fn ime_composition_commits_once_and_suppresses_duplicate_commit() {
    let platform = host_platform();
    let mut composition = ImeComposition::new();

    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Preedit("に".into())),
        None
    );
    assert!(composition.is_composing());
    assert_eq!(composition.preedit(), "に");

    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Commit("に".into())),
        Some("に".to_string())
    );
    assert!(!composition.is_composing());

    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Commit("に".into())),
        None,
        "a repeated commit that did not come from a new pre-edit must be dropped"
    );

    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Preedit("に".into())),
        None
    );
    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Commit("に".into())),
        Some("に".to_string()),
        "a new composition must be allowed to commit the same text again"
    );
}

#[test]
fn ime_cancel_and_empty_commit_emit_nothing() {
    let platform = host_platform();
    let mut composition = ImeComposition::new();
    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Preedit("composing".into())),
        None
    );
    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Commit(String::new())),
        None
    );
    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Preedit("again".into())),
        None
    );
    assert_eq!(
        platform.normalize_ime(&mut composition, ImeEvent::Cancel),
        None
    );
    assert!(!composition.is_composing());
    assert_eq!(composition.preedit(), "");
}

#[cfg(unix)]
mod posix_pty {
    use std::io::{self, Read, Write};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;
    use std::time::{Duration, Instant};

    use made_platform::{PtyRequest, PtySession, PtySize};

    use super::*;

    /// Keep the PTY output drained so a blocking write cannot wedge a shell
    /// that produces prompts, echoes, or job-control notices.
    fn drain_output(session: &dyn PtySession) {
        let mut reader = session.take_reader().expect("take reader");
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
    }

    fn shell_request(cwd: PathBuf) -> PtyRequest {
        let platform = host_platform();
        let mut request = PtyRequest::new("/bin/sh", cwd);
        request.env = platform.environment(&[], &[]).into_vec();
        request.size = PtySize { rows: 24, cols: 80 };
        request
    }

    fn collect_until(
        receiver: &mpsc::Receiver<Vec<u8>>,
        output: &mut Vec<u8>,
        needle: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if String::from_utf8_lossy(output).contains(needle) {
                return true;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => output.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        String::from_utf8_lossy(output).contains(needle)
    }

    #[test]
    fn pty_spawns_real_shell_with_cwd_echo_resize_and_reaping() {
        let root = unique_temp("pty");
        std::fs::create_dir_all(&root).expect("create cwd");

        let platform = host_platform();
        let session = platform
            .spawn_pty(&shell_request(root.clone()))
            .expect("spawn PTY");
        assert_eq!(
            session.size().expect("initial size"),
            PtySize { rows: 24, cols: 80 }
        );

        session
            .resize(PtySize {
                rows: 30,
                cols: 100,
            })
            .expect("resize PTY");
        assert_eq!(
            session.size().expect("resized size"),
            PtySize {
                rows: 30,
                cols: 100
            }
        );

        let mut reader = session.take_reader().expect("take reader");
        let mut writer = session.take_writer().expect("take writer");

        let (sender, receiver) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        writer
            .write_all(b"stty size\npwd\nprintf 'CONTRACT%s\\n' OK\n")
            .expect("write commands");
        writer.flush().expect("flush commands");

        let mut output = Vec::new();
        assert!(
            collect_until(&receiver, &mut output, "CONTRACTOK", Duration::from_secs(5)),
            "shell did not execute commands; output: {}",
            String::from_utf8_lossy(&output)
        );
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("30 100"),
            "PTY window size was not visible to the shell: {text}"
        );
        let root_text = root.to_string_lossy();
        assert!(
            text.contains(root_text.as_ref()),
            "shell did not start in the requested cwd: {text}"
        );

        writer.write_all(b"exit\n").expect("exit command");
        writer.flush().expect("flush exit");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut status = None;
        while Instant::now() < deadline {
            status = session.try_wait().expect("try_wait");
            if status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let status = status.expect("child must exit after `exit`");
        assert!(status.success(), "unexpected exit status: {status}");
        let _ = reader_thread.join();

        drop(writer);
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn wait_for_file_len(path: &std::path::Path, minimum: u64, timeout: Duration) -> u64 {
        let deadline = Instant::now() + timeout;
        loop {
            let len = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            if len >= minimum || Instant::now() >= deadline {
                return len;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn bash_request(cwd: PathBuf) -> PtyRequest {
        let platform = host_platform();
        let mut request = PtyRequest::new("/bin/bash", cwd);
        request.args = vec![
            OsString::from("--noprofile"),
            OsString::from("--norc"),
            OsString::from("-i"),
        ];
        request.env = platform.environment(&[], &[]).into_vec();
        request.size = PtySize { rows: 24, cols: 80 };
        request
    }

    fn read_pid(path: &std::path::Path) -> libc::pid_t {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<libc::pid_t>() {
                    return pid;
                }
            }
            assert!(Instant::now() < deadline, "pid file was not written");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_heartbeat_stops(path: &std::path::Path) {
        thread::sleep(Duration::from_millis(250));
        let stopped = std::fs::metadata(path).expect("heartbeat metadata").len();
        thread::sleep(Duration::from_millis(600));
        let after = std::fs::metadata(path).expect("heartbeat metadata").len();
        assert_eq!(
            after, stopped,
            "a descendant survived process-group cleanup"
        );
    }

    #[test]
    fn pty_terminate_kills_hup_ignoring_interactive_bash_foreground_job() {
        let root = unique_temp("pty-bash-fg");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let pid_file = root.join("foreground.pid");
        let script = format!(
            "echo \\$\\$ > '{}'; trap '' HUP; while :; do echo tick >> '{}'; sleep 0.1; done",
            pid_file.display(),
            heartbeat.display()
        );

        let platform = host_platform();
        let request = bash_request(root.clone());
        let session = platform
            .spawn_pty(&request)
            .expect("spawn interactive bash");
        drain_output(session.as_ref());
        let mut writer = session.take_writer().expect("take writer");

        // Let interactive bash initialize job control and readline. Job control
        // is on by default for interactive bash; do not override it here.
        thread::sleep(Duration::from_millis(250));
        writer
            .write_all(format!("sh -c \"{}\"\n", script).as_bytes())
            .expect("start foreground job");
        writer.flush().expect("flush command");

        let foreground_pid = read_pid(&pid_file);
        let shell_pid = session.pid().expect("shell pid") as libc::pid_t;
        let foreground_group = unsafe { libc::getpgid(foreground_pid) };
        let shell_group = unsafe { libc::getpgid(shell_pid) };
        assert!(foreground_group > 1);
        assert!(
            foreground_group != shell_group,
            "interactive bash should put the foreground job in its own process group"
        );
        assert!(
            wait_for_file_len(&heartbeat, 8, Duration::from_secs(5)) > 0,
            "HUP-ignoring foreground job never started"
        );

        let status = session
            .terminate()
            .expect("terminate interactive bash process groups");
        assert!(
            status.signal.is_some() || !status.success(),
            "terminated interactive shell should report a signal: {status}"
        );
        assert_heartbeat_stops(&heartbeat);

        drop(writer);
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_kills_hup_ignoring_interactive_bash_background_job() {
        let root = unique_temp("pty-bash-bg");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let pid_file = root.join("background.pid");
        let script = format!(
            "echo \\$\\$ > '{}'; trap '' HUP; while :; do echo tick >> '{}'; sleep 0.1; done",
            pid_file.display(),
            heartbeat.display()
        );

        let platform = host_platform();
        let request = bash_request(root.clone());
        let session = platform
            .spawn_pty(&request)
            .expect("spawn interactive bash");
        drain_output(session.as_ref());
        let mut writer = session.take_writer().expect("take writer");

        thread::sleep(Duration::from_millis(250));
        // Interactive bash enables job control by default, so the background
        // job splits into its own process group while staying in the shell's
        // session. Cleanup must find it without touching another tab.
        writer
            .write_all(format!("sh -c \"{}\" &\n", script).as_bytes())
            .expect("start background job");
        writer.flush().expect("flush command");

        let background_pid = read_pid(&pid_file);
        let shell_pid = session.pid().expect("shell pid") as libc::pid_t;
        let background_group = unsafe { libc::getpgid(background_pid) };
        let shell_group = unsafe { libc::getpgid(shell_pid) };
        assert_ne!(
            background_group, shell_group,
            "default interactive job control should put the background job in its own process group"
        );
        assert!(
            wait_for_file_len(&heartbeat, 8, Duration::from_secs(5)) > 0,
            "HUP-ignoring background job never started"
        );

        let status = session
            .terminate()
            .expect("terminate interactive bash process groups");
        assert!(
            status.signal.is_some() || !status.success(),
            "terminated interactive shell should report a signal: {status}"
        );
        assert_heartbeat_stops(&heartbeat);

        drop(writer);
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_cleans_group_after_shell_exit() {
        let root = unique_temp("pty-group-after-exit");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let pid_file = root.join("background.pid");
        let script = format!(
            "sh -c 'echo $$ > \"{}\"; trap \"\" HUP; while :; do echo tick >> \"{}\"; sleep 0.1; done' &\n\
             while [ ! -s \"{}\" ]; do sleep 0.05; done\n\
             exit 0\n",
            pid_file.display(),
            heartbeat.display(),
            heartbeat.display()
        );

        let platform = host_platform();
        let mut request = shell_request(root.clone());
        request.args = vec![OsString::from("-c"), OsString::from(script)];
        let session = platform.spawn_pty(&request).expect("spawn exiting shell");

        assert!(
            wait_for_file_len(&heartbeat, 8, Duration::from_secs(5)) > 0,
            "HUP-ignoring background descendant never wrote a heartbeat"
        );
        let background_pid = read_pid(&pid_file);
        let background_group = unsafe { libc::getpgid(background_pid) };
        assert!(background_group > 1);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut status = None;
        while Instant::now() < deadline {
            status = session.try_wait().expect("try_wait");
            if status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let status = status.expect("direct shell should exit while descendant stays alive");
        assert!(status.success(), "unexpected shell status: {status}");

        // The direct shell is already reaped. terminate must still signal the
        // stored process group instead of returning early.
        let status = session.terminate().expect("terminate after shell exit");
        assert!(status.success());
        assert_heartbeat_stops(&heartbeat);

        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_cleans_job_control_background_after_interactive_shell_exit() {
        let root = unique_temp("pty-bash-bg-after-exit");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let pid_file = root.join("background.pid");
        let script = format!(
            "echo \\$\\$ > '{}'; trap '' HUP; while :; do echo tick >> '{}'; sleep 0.1; done",
            pid_file.display(),
            heartbeat.display()
        );

        let platform = host_platform();
        let session = platform
            .spawn_pty(&bash_request(root.clone()))
            .expect("spawn interactive bash");
        drain_output(session.as_ref());
        let mut writer = session.take_writer().expect("take writer");

        thread::sleep(Duration::from_millis(250));
        writer
            .write_all(format!("sh -c \"{}\" &\n", script).as_bytes())
            .expect("start background job");
        writer.flush().expect("flush command");

        let background_pid = read_pid(&pid_file);
        let shell_pid = session.pid().expect("shell pid") as libc::pid_t;
        assert_ne!(
            unsafe { libc::getpgid(background_pid) },
            unsafe { libc::getpgid(shell_pid) },
            "default interactive job control should put the background job in its own process group"
        );
        assert!(
            wait_for_file_len(&heartbeat, 8, Duration::from_secs(5)) > 0,
            "HUP-ignoring background job never started"
        );

        // The interactive shell exits first. The background job ignores HUP
        // and remains in the shell's session, so tab-local cleanup must still
        // find it when the workspace is closed later.
        writer.write_all(b"exit\n").expect("exit command");
        writer.flush().expect("flush exit");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut status = None;
        while Instant::now() < deadline {
            status = session.try_wait().expect("try_wait");
            if status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let status = status.expect("interactive shell must exit while its job remains");
        assert!(status.success(), "unexpected shell status: {status}");

        session
            .terminate()
            .expect("terminate session after interactive shell exit");
        assert_heartbeat_stops(&heartbeat);

        drop(writer);
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_does_not_signal_sibling_workspace_jobs() {
        let root = unique_temp("pty-sibling");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat_a = root.join("heartbeat-a.log");
        let heartbeat_b = root.join("heartbeat-b.log");
        let pid_a = root.join("a.pid");
        let pid_b = root.join("b.pid");
        let script_a = format!(
            "echo \\$\\$ > '{}'; trap '' HUP; while :; do echo tick >> '{}'; sleep 0.1; done",
            pid_a.display(),
            heartbeat_a.display()
        );
        let script_b = format!(
            "echo \\$\\$ > '{}'; trap '' HUP; while :; do echo tick >> '{}'; sleep 0.1; done",
            pid_b.display(),
            heartbeat_b.display()
        );

        let platform = host_platform();
        let session_a = platform
            .spawn_pty(&bash_request(root.clone()))
            .expect("spawn first interactive bash");
        let session_b = platform
            .spawn_pty(&bash_request(root.clone()))
            .expect("spawn sibling interactive bash");
        drain_output(session_a.as_ref());
        drain_output(session_b.as_ref());
        let mut writer_a = session_a.take_writer().expect("take first writer");
        let mut writer_b = session_b.take_writer().expect("take second writer");

        thread::sleep(Duration::from_millis(250));
        writer_a
            .write_all(format!("sh -c \"{}\" &\n", script_a).as_bytes())
            .expect("start first background job");
        writer_a.flush().expect("flush first command");
        writer_b
            .write_all(format!("sh -c \"{}\" &\n", script_b).as_bytes())
            .expect("start second background job");
        writer_b.flush().expect("flush second command");

        read_pid(&pid_a);
        read_pid(&pid_b);
        assert!(
            wait_for_file_len(&heartbeat_a, 8, Duration::from_secs(5)) > 0,
            "first HUP-ignoring background job never started"
        );
        assert!(
            wait_for_file_len(&heartbeat_b, 8, Duration::from_secs(5)) > 0,
            "sibling HUP-ignoring background job never started"
        );

        session_a
            .terminate()
            .expect("terminate the first workspace session");
        assert_heartbeat_stops(&heartbeat_a);

        thread::sleep(Duration::from_millis(250));
        let sibling_before = std::fs::metadata(&heartbeat_b)
            .expect("sibling heartbeat metadata")
            .len();
        thread::sleep(Duration::from_millis(600));
        let sibling_after = std::fs::metadata(&heartbeat_b)
            .expect("sibling heartbeat metadata")
            .len();
        assert!(
            sibling_after > sibling_before,
            "closing one workspace must not terminate a sibling workspace's job"
        );

        session_b
            .terminate()
            .expect("terminate the sibling workspace session");
        assert_heartbeat_stops(&heartbeat_b);

        drop(writer_a);
        drop(writer_b);
        drop(session_a);
        drop(session_b);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_kills_process_group_descendants() {
        let root = unique_temp("pty-group");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let pid_file = root.join("grandchild.pid");
        let script = format!(
            "while :; do echo tick >> '{}'; sleep 0.1; done &\n\
             echo $! > '{}'\n\
             wait\n",
            heartbeat.display(),
            pid_file.display()
        );

        let platform = host_platform();
        let mut request = shell_request(root.clone());
        request.args = vec![OsString::from("-c"), OsString::from(script)];
        let session = platform
            .spawn_pty(&request)
            .expect("spawn PTY with grandchild");

        let started = wait_for_file_len(&heartbeat, 8, Duration::from_secs(5));
        assert!(started > 0, "background descendant never wrote a heartbeat");
        thread::sleep(Duration::from_millis(250));
        let before = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        thread::sleep(Duration::from_millis(250));
        let running = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        assert!(
            running > before,
            "background descendant was not running before termination"
        );

        let status = session.terminate().expect("terminate process group");
        assert!(
            status.signal.is_some() || !status.success(),
            "terminated shell should report a signal: {status}"
        );

        thread::sleep(Duration::from_millis(250));
        let stopped = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        thread::sleep(Duration::from_millis(600));
        let after = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        assert_eq!(
            after, stopped,
            "a descendant in the PTY process group survived termination"
        );

        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pty_terminate_kills_and_reaps_an_idle_child() {
        let root = unique_temp("pty-kill");
        std::fs::create_dir_all(&root).expect("create cwd");
        let platform = host_platform();
        let mut request = shell_request(root.clone());
        request.args = vec![OsString::from("-c"), OsString::from("sleep 30")];
        let session = platform.spawn_pty(&request).expect("spawn idle PTY");
        assert!(session.pid().is_some());

        let status = session.terminate().expect("terminate and reap");
        assert!(
            status.signal.is_some() || !status.success(),
            "idle child should not report a clean exit: {status}"
        );
        assert!(
            session.try_wait().expect("post-terminate wait").is_some(),
            "terminated child must stay reaped"
        );

        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(windows)]
mod windows_pty {
    use std::io::{self, Read};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use made_platform::{PtyRequest, PtySize};

    use super::*;

    fn wait_for_file_len(path: &std::path::Path, minimum: u64, timeout: Duration) -> u64 {
        let deadline = Instant::now() + timeout;
        loop {
            let len = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            if len >= minimum || Instant::now() >= deadline {
                return len;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn assert_heartbeat_stops(path: &std::path::Path) {
        thread::sleep(Duration::from_millis(250));
        let stopped = std::fs::metadata(path).expect("heartbeat metadata").len();
        thread::sleep(Duration::from_millis(600));
        let after = std::fs::metadata(path).expect("heartbeat metadata").len();
        assert_eq!(after, stopped, "a descendant survived Job Object cleanup");
    }

    #[test]
    fn pty_terminate_cleans_job_descendants_after_shell_exit() {
        let root = unique_temp("pty-windows-job");
        std::fs::create_dir_all(&root).expect("create cwd");
        let heartbeat = root.join("heartbeat.log");
        let child_pid = root.join("child.pid");
        let script_path = root.join("descendant.ps1");
        let child_script = root.join("child.ps1");
        std::fs::write(
            &child_script,
            r#"param(
    [string]$Heartbeat
)
for (;;) {
    Add-Content -LiteralPath $Heartbeat -Value tick
    Start-Sleep -Milliseconds 100
}
"#,
        )
        .expect("write child script");
        std::fs::write(
            &script_path,
            r#"param(
    [string]$Heartbeat,
    [string]$ChildPid,
    [string]$ChildScript
)
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = Join-Path $PSHOME 'powershell.exe'
$psi.Arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "' + $ChildScript + '" -Heartbeat "' + $Heartbeat + '"'
$psi.UseShellExecute = $false
$child = [System.Diagnostics.Process]::Start($psi)
$child.Id | Set-Content -LiteralPath $ChildPid
while (-not (Test-Path -LiteralPath $Heartbeat)) { Start-Sleep -Milliseconds 50 }
"#,
        )
        .expect("write descendant script");

        let platform = host_platform();
        let mut request = PtyRequest::new("powershell.exe", root.clone());
        request.args = vec![
            OsString::from("-NoLogo"),
            OsString::from("-NoProfile"),
            OsString::from("-ExecutionPolicy"),
            OsString::from("Bypass"),
            OsString::from("-File"),
            script_path.into_os_string(),
            OsString::from("-Heartbeat"),
            heartbeat.clone().into_os_string(),
            OsString::from("-ChildPid"),
            child_pid.clone().into_os_string(),
            OsString::from("-ChildScript"),
            child_script.into_os_string(),
        ];
        request.env = platform.environment(&[], &[]).into_vec();
        request.size = PtySize { rows: 24, cols: 80 };
        let session = platform
            .spawn_pty(&request)
            .expect("spawn PowerShell in a Job Object");

        // Drain output so a chatty child cannot fill the ConPTY buffer, and
        // retain the tail for a useful failure message.
        let mut reader = session.take_reader().expect("take reader");
        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let reader_captured = Arc::clone(&captured);
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let mut output = reader_captured
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        output.extend_from_slice(&buffer[..read]);
                        if output.len() > 8192 {
                            let excess = output.len() - 8192;
                            output.drain(..excess);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let heartbeat_len = wait_for_file_len(&heartbeat, 8, Duration::from_secs(10));
        let captured_text = {
            let output = captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            String::from_utf8_lossy(output.as_slice()).into_owned()
        };
        assert!(
            heartbeat_len > 0,
            "descendant never wrote a heartbeat; PowerShell output: {captured_text}"
        );

        // The direct shell exits while its descendant stays in the job.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut status = None;
        while Instant::now() < deadline {
            status = session.try_wait().expect("try_wait");
            if status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let status = status.expect("direct PowerShell should exit while its descendant remains");
        assert!(status.success(), "unexpected shell status: {status}");

        let before = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        thread::sleep(Duration::from_millis(400));
        let running = std::fs::metadata(&heartbeat)
            .expect("heartbeat metadata")
            .len();
        assert!(
            running > before,
            "descendant was not running after the direct shell exited"
        );

        session
            .terminate()
            .expect("terminate the workspace Job Object after shell exit");
        assert_heartbeat_stops(&heartbeat);

        drop(session);
        let _ = reader_thread.join();
        let _ = std::fs::remove_dir_all(&root);
    }
}
