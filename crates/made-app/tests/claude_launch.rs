//! Claude Code launch composition tests: the plugin list, its separator, entry
//! safety and the shipped plugin directory. Copied from the `made-platform`
//! contract tests with their assertions unchanged.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use made_app::integrations::claude_code::launch::{
    claude_plugin_dirs_separator, compose_claude_plugin_dirs, shipped_claude_plugin_dir,
    CLAUDE_CODE_PLUGIN_DIRS_ENV,
};
use made_platform::{AppPaths, PathsInputs, PlatformFamily};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn unique_temp(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "made-claude-launch-{label}-{}-{nanos}-{}",
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
        shipped_plugins_dir: Some(PathBuf::from("/opt/made/plugins")),
        temp_dir: PathBuf::from("/tmp"),
        user_token: "1000".to_owned(),
        plugin_override: None,
        dev_plugin_dir: None,
        development: false,
    }
}

#[test]
fn shipped_claude_plugin_dir_requires_a_readable_directory() {
    let root = unique_temp("claude-plugin-dir");
    let plugin = root.join("plugins").join("claude-code-made");
    let inputs = PathsInputs {
        shipped_plugins_dir: Some(root.join("plugins")),
        ..path_inputs(PlatformFamily::Linux)
    };
    let paths = AppPaths::resolve(&inputs).expect("paths");
    assert_eq!(shipped_claude_plugin_dir(&paths), None, "missing");

    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(&plugin, "file").unwrap();
    assert_eq!(shipped_claude_plugin_dir(&paths), None, "not a directory");

    std::fs::remove_file(&plugin).unwrap();
    std::fs::create_dir_all(&plugin).unwrap();
    assert_eq!(shipped_claude_plugin_dir(&paths), Some(plugin.clone()));

    let unresolved = PathsInputs {
        shipped_plugins_dir: None,
        ..path_inputs(PlatformFamily::Linux)
    };
    assert_eq!(
        shipped_claude_plugin_dir(&AppPaths::resolve(&unresolved).expect("paths")),
        None
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn claude_plugin_dirs_composition_preserves_entries_and_never_duplicates() {
    let shipped = Path::new("/opt/made/plugins/claude-code-made");
    let compose = |existing: Option<&str>, separator| {
        compose_claude_plugin_dirs(existing.map(OsStr::new), shipped, separator)
    };
    let made = "/opt/made/plugins/claude-code-made";

    assert_eq!(compose(None, ':'), OsString::from(made));
    assert_eq!(compose(Some(""), ':'), OsString::from(made));
    assert_eq!(
        compose(Some("/u/a:/u/b"), ':'),
        OsString::from(format!("/u/a:/u/b:{made}"))
    );
    assert_eq!(
        compose(Some("/u/b:/u/a"), ':'),
        OsString::from(format!("/u/b:/u/a:{made}")),
        "order is kept"
    );
    assert_eq!(
        compose(Some("/u/a:"), ':'),
        OsString::from(format!("/u/a:{made}")),
        "a trailing separator is not doubled"
    );
    for existing in [
        made.to_owned(),
        format!("{made}:/u/a"),
        format!("/u/a:{made}:/u/b"),
    ] {
        assert_eq!(
            compose(Some(&existing), ':'),
            OsString::from(&existing),
            "already present: {existing}"
        );
    }
    assert_eq!(
        compose(Some("/u/a:/opt/made/plugins/claude-code-made-extra"), ':'),
        OsString::from(format!(
            "/u/a:/opt/made/plugins/claude-code-made-extra:{made}"
        )),
        "a prefix match is not a duplicate"
    );
    assert_eq!(
        compose(Some("C:\\u\\a"), ';'),
        OsString::from(format!("C:\\u\\a;{made}")),
        "a drive colon is not a separator under ;"
    );
    assert_eq!(
        compose(Some(&format!("C:\\u\\a;{made}")), ';'),
        OsString::from(format!("C:\\u\\a;{made}"))
    );

    assert_eq!(CLAUDE_CODE_PLUGIN_DIRS_ENV, "CLAUDE_CODE_PLUGIN_DIRS");
    assert_eq!(claude_plugin_dirs_separator(PlatformFamily::Linux), ':');
    assert_eq!(claude_plugin_dirs_separator(PlatformFamily::MacOs), ':');
    assert_eq!(claude_plugin_dirs_separator(PlatformFamily::Windows), ';');
}

#[cfg(unix)]
#[test]
fn claude_plugin_dirs_composition_treats_a_trailing_slash_as_the_same_entry() {
    let shipped = Path::new("/opt/made/plugins/claude-code-made");
    for existing in [
        "/opt/made/plugins/claude-code-made/",
        "/u/a:/opt/made/plugins/claude-code-made//:/u/b",
    ] {
        assert_eq!(
            compose_claude_plugin_dirs(Some(OsStr::new(existing)), shipped, ':'),
            OsString::from(existing),
            "already present: {existing}"
        );
    }
    let slashed = Path::new("/opt/made/plugins/claude-code-made/");
    assert_eq!(
        compose_claude_plugin_dirs(
            Some(OsStr::new("/opt/made/plugins/claude-code-made")),
            slashed,
            ':'
        ),
        OsString::from("/opt/made/plugins/claude-code-made")
    );
}

#[cfg(unix)]
#[test]
fn claude_plugin_dirs_composition_adds_nothing_for_an_unsafe_shipped_path() {
    for bad in [
        "relative/plugins/claude-code-made",
        "/opt/ma:de/plugins",
        "/opt/ma\"de/plugins",
        "/opt/ma`de/plugins",
        "/opt/ma$de/plugins",
        "/opt/ma\\de/plugins",
        "/opt/ma\nde/plugins",
    ] {
        let shipped = Path::new(bad);
        assert_eq!(
            compose_claude_plugin_dirs(None, shipped, ':'),
            OsString::new(),
            "{bad}"
        );
        assert_eq!(
            compose_claude_plugin_dirs(Some(OsStr::new("/u/a:/u/b")), shipped, ':'),
            OsString::from("/u/a:/u/b"),
            "{bad}"
        );
    }
}

#[cfg(unix)]
#[test]
fn shipped_claude_plugin_dir_refuses_an_unsafe_path() {
    let root = unique_temp("claude-unsafe");
    let plugin = root.join("a:b").join("claude-code-made");
    std::fs::create_dir_all(&plugin).unwrap();
    let inputs = PathsInputs {
        shipped_plugins_dir: Some(root.join("a:b")),
        ..path_inputs(PlatformFamily::Linux)
    };
    assert_eq!(
        shipped_claude_plugin_dir(&AppPaths::resolve(&inputs).expect("paths")),
        None
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_windows_list_compares_entries_without_case_or_trailing_backslashes() {
    // Platform-neutral: a `;` list is compared the way Windows paths are.
    let shipped = Path::new(if cfg!(windows) {
        "C:\\Made\\plugins\\claude-code-made"
    } else {
        "/Made/plugins/claude-code-made"
    });
    let made = shipped.to_str().unwrap();
    for existing in [
        made.to_ascii_uppercase(),
        format!("{made}\\"),
        format!("{made}/"),
        format!("{};{}", made.to_ascii_lowercase(), "D:\\u"),
    ] {
        assert_eq!(
            compose_claude_plugin_dirs(Some(OsStr::new(&existing)), shipped, ';'),
            OsString::from(&existing),
            "already present: {existing}"
        );
    }
    assert_eq!(
        compose_claude_plugin_dirs(Some(OsStr::new("D:\\u")), shipped, ';'),
        OsString::from(format!("D:\\u;{made}")),
        "a drive colon is not a separator under ;"
    );
}

#[cfg(windows)]
#[test]
fn shipped_claude_plugin_dir_accepts_a_drive_letter_path() {
    let root = unique_temp("claude-drive");
    let plugin = root.join("claude-code-made");
    std::fs::create_dir_all(&plugin).unwrap();
    assert!(plugin.to_str().unwrap().contains(':'));
    let inputs = PathsInputs {
        shipped_plugins_dir: Some(root.clone()),
        ..path_inputs(PlatformFamily::Windows)
    };
    let paths = AppPaths::resolve(&inputs).expect("paths");
    assert_eq!(shipped_claude_plugin_dir(&paths), Some(plugin.clone()));
    let _ = std::fs::remove_dir_all(&root);
}
