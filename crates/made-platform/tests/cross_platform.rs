//! Cross-platform delivery contract for Phase 4 (tasks 4.1-4.6).
//!
//! These tests are static contracts over the repository layout. Compiling the
//! workspace cannot prove that a second UI implementation or an OS branch was
//! not added, so the test inspects source, manifests, documentation, and CI
//! definitions directly. They run on every supported host; only the assertion
//! that the Linux backend was selected is OS-gated.
//!
//! Test-only `cfg` is acceptable per `wiki/architecture.md`. Shipped Rust code
//! outside `crates/made-platform` must not contain OS-specific `cfg` branches.

use std::fs;
use std::path::{Path, PathBuf};

/// The four delivery targets from the change spec. The triples are the
/// identifiers used by CI and documented in `README.md`.
const SUPPORTED_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

const WORKSPACE_CRATES: &[&str] = &[
    "made-app",
    "made-bridge",
    "made-explorer",
    "made-platform",
    "made-terminal",
];

/// UI modules that must exist once, in `made-app`, independent of the OS.
const APP_UI_MODULES: &[&str] = &[
    "session_map",
    "session_follow",
    "explorer_view",
    "terminal_view",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root must resolve from crates/made-platform")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    let source =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    // Windows checkouts may use CRLF. Normalize so source and workflow
    // contracts are asserted against the same text on every runner.
    source.replace("\r\n", "\n")
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn crate_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root.join("crates")).expect("crates/ must exist") {
        let entry = entry.expect("crate entry");
        if entry.file_type().expect("crate file type").is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    dirs
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .map(|entry| entry.expect("directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(dir, &mut files);
    files
}

/// Shipped code precedes the `#[cfg(test)]` module at the end of a module file.
/// The repository follows that convention, so removing the test tail lets the
/// isolation check allow test-only platform selection without weakening the
/// rule for code that ships.
fn shipped_source(source: &str) -> &str {
    match source.find("#[cfg(test)]\nmod ") {
        Some(index) => &source[..index],
        None => source,
    }
}

fn is_os_specific_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
        return false;
    }

    let has_cfg_attribute = line.contains("#[cfg(") || line.contains("cfg!(");
    let has_os_term = ["target_os", "target_family", "unix", "windows"]
        .iter()
        .any(|term| line.contains(term));

    (has_cfg_attribute && has_os_term)
        || line.contains("std::os::unix")
        || line.contains("std::os::windows")
}

#[test]
fn workspace_has_one_app_crate_and_one_ui_source_tree() {
    let root = repo_root();
    let crates = crate_dirs(&root);

    for expected in WORKSPACE_CRATES {
        assert!(
            crates
                .iter()
                .any(|dir| dir.file_name().and_then(|name| name.to_str()) == Some(expected)),
            "workspace must contain the {expected} crate"
        );
    }

    let binaries: Vec<_> = crates
        .iter()
        .filter(|dir| dir.join("src/main.rs").is_file())
        .collect();
    assert_eq!(
        binaries.len(),
        1,
        "there must be exactly one application binary target"
    );
    assert_eq!(
        binaries[0].file_name().and_then(|name| name.to_str()),
        Some("made-app"),
        "the only application binary must live in made-app"
    );

    for crate_dir in &crates {
        let name = crate_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name == "made-app" {
            continue;
        }

        let manifest = read(crate_dir.join("Cargo.toml"));
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with("made-app =")),
            "{name} must not depend on the app composition root"
        );

        for module in APP_UI_MODULES {
            assert!(
                !crate_dir.join("src").join(format!("{module}.rs")).exists(),
                "{name} must not own the app UI module {module}"
            );
        }
    }

    let app_manifest = read(root.join("crates/made-app/Cargo.toml"));
    for dependency in [
        "made-bridge",
        "made-explorer",
        "made-platform",
        "made-terminal",
    ] {
        assert!(
            app_manifest.contains(&format!("{dependency} =")),
            "made-app must compose {dependency}"
        );
    }

    let app_lib = read(root.join("crates/made-app/src/lib.rs"));
    for module in APP_UI_MODULES {
        assert!(
            app_lib.contains(&format!("pub mod {module};")),
            "made-app must declare the shared UI module {module} unconditionally"
        );
    }
}

#[test]
fn platform_specific_code_lives_only_in_made_platform() {
    let root = repo_root();
    let platform_dir = root.join("crates/made-platform");
    let mut violations = Vec::new();

    for crate_dir in crate_dirs(&root) {
        if crate_dir == platform_dir {
            continue;
        }

        for file in rust_files(&crate_dir.join("src")) {
            let source = read(&file);
            for (index, line) in shipped_source(&source).lines().enumerate() {
                if is_os_specific_line(line) {
                    violations.push(format!(
                        "{}:{}: {}",
                        display(&root, &file),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }

        let build_script = crate_dir.join("build.rs");
        if build_script.is_file() {
            for (index, line) in read(&build_script).lines().enumerate() {
                if is_os_specific_line(line) {
                    violations.push(format!(
                        "{}:{}: {}",
                        display(&root, &build_script),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }

        let manifest_path = crate_dir.join("Cargo.toml");
        for (index, line) in read(&manifest_path).lines().enumerate() {
            if line.trim_start().starts_with("[target.") {
                violations.push(format!(
                    "{}:{}: {}",
                    display(&root, &manifest_path),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "OS-specific Rust code or target tables outside made-platform:\n{}",
        violations.join("\n")
    );

    let platform_lib = read(platform_dir.join("src/lib.rs"));
    for backend in [
        "#[cfg(target_os = \"linux\")]\npub mod linux;",
        "#[cfg(target_os = \"macos\")]\npub mod macos;",
        "#[cfg(target_os = \"windows\")]\npub mod windows;",
    ] {
        assert!(
            platform_lib.contains(backend),
            "made-platform must cfg-select its backend with {backend:?}"
        );
    }
    for backend in ["linux.rs", "macos.rs", "windows.rs"] {
        assert!(
            platform_dir.join("src").join(backend).is_file(),
            "made-platform must provide src/{backend}"
        );
    }
}

#[test]
fn session_follow_data_model_is_shared_across_targets() {
    let root = repo_root();
    let state = read(root.join("crates/made-bridge/src/state.rs"));

    assert!(
        state.contains("pub struct SessionSnapshot"),
        "the bridge owns the normalized snapshot model"
    );
    for field in [
        "pub revision: u64",
        "pub active_session_id: Option<String>",
        "pub tabs_enabled: bool",
        "pub sessions: Vec<SessionInfo>",
    ] {
        assert!(
            state.contains(field),
            "SessionSnapshot must keep the shared field {field:?}"
        );
    }
    assert!(state.contains("pub struct SessionInfo"));
    for field in [
        "pub session_id: String",
        "pub title: String",
        "pub active: bool",
        "pub busy: bool",
        "pub attention: bool",
        "pub unread: bool",
        "pub directory: Option<PathBuf>",
        "pub subpath: Option<String>",
        "pub project_id: Option<String>",
    ] {
        assert!(
            state.contains(field),
            "SessionInfo must keep the shared field {field:?}"
        );
    }

    let bridge_lib = read(root.join("crates/made-bridge/src/lib.rs"));
    assert!(
        bridge_lib.contains("pub mod state;"),
        "made-bridge must compile the snapshot model on every target"
    );

    let follow = read(root.join("crates/made-app/src/session_follow.rs"));
    assert!(
        follow.contains("use made_bridge::state::{SessionSnapshot"),
        "made-app must consume the bridge snapshot model rather than a local copy"
    );
    assert!(
        follow.contains("pub fn apply_snapshot"),
        "the session-follow coordinator must apply the shared snapshot"
    );
    assert!(
        !follow.contains("pub struct SessionSnapshot"),
        "made-app must not define a second session snapshot model"
    );

    let app_lib = read(root.join("crates/made-app/src/lib.rs"));
    assert!(
        app_lib.contains("pub mod session_follow;"),
        "session follow must be compiled on every target"
    );
    assert!(
        !app_lib.contains("pub struct SessionSnapshot"),
        "made-app must not define a second session snapshot model"
    );

    for (name, source) in [
        ("made-bridge/src/state.rs", &state),
        ("made-app/src/session_follow.rs", &follow),
    ] {
        for (index, line) in shipped_source(source).lines().enumerate() {
            assert!(
                !is_os_specific_line(line),
                "{name}:{} must not branch on the operating system: {}",
                index + 1,
                line.trim()
            );
        }
    }
}

#[test]
fn supported_targets_are_documented_and_covered_by_ci() {
    let root = repo_root();
    let readme = read(root.join("README.md"));
    let architecture = read(root.join("wiki/architecture.md"));

    for target in SUPPORTED_TARGETS {
        assert!(
            readme.contains(target),
            "README.md must document the supported target identifier {target}"
        );
    }
    for phrase in ["X11", "Wayland", "native OpenCode"] {
        assert!(
            readme.contains(phrase),
            "README.md must document the {phrase} delivery constraint"
        );
    }
    for phrase in [
        "macOS arm64/x64",
        "Linux x64 on X11 and Wayland",
        "Windows x64",
    ] {
        assert!(
            architecture.contains(phrase),
            "wiki/architecture.md must document {phrase}"
        );
    }

    let ci = read(root.join(".github/workflows/ci.yml"));
    let artifacts = read(root.join(".github/workflows/ci-artifacts.yml"));
    for target in SUPPORTED_TARGETS {
        assert!(
            ci.contains(target),
            ".github/workflows/ci.yml must build and test {target}"
        );
        assert!(
            artifacts.contains(target),
            ".github/workflows/ci-artifacts.yml must package {target}"
        );
    }
}

#[test]
fn development_artifact_uses_the_release_plugin_layout() {
    let root = repo_root();
    let artifacts = read(root.join(".github/workflows/ci-artifacts.yml"));

    assert!(
        artifacts.contains("$stage/plugins/opencode-made"),
        "the artifact root must stage the plugin at <install_root>/plugins/opencode-made"
    );
    assert!(
        !artifacts.contains("$stage/plugin/opencode-made"),
        "the artifact must not use the obsolete singular plugin directory"
    );
}

#[test]
fn platform_smoke_workflows_do_not_silently_skip_validation() {
    let root = repo_root();
    let linux = read(root.join(".github/workflows/ci-linux-display.yml")).to_lowercase();
    let windows = read(root.join(".github/workflows/ci-windows.yml")).to_lowercase();
    let macos = read(root.join(".github/workflows/ci-macos.yml")).to_lowercase();

    assert!(
        linux.contains("xvfb-run") && linux.contains("weston"),
        "the Linux workflow must cover X11 under xvfb and Wayland under a headless compositor"
    );
    assert!(
        linux.contains("--smoke-test"),
        "the Linux workflow must invoke the hidden app smoke-test mode"
    );
    assert!(
        linux.contains("manual validation"),
        "the Linux workflow must record the validation scope that CI cannot cover"
    );

    for (name, workflow) in [("Windows", &windows), ("macOS", &macos)] {
        assert!(
            workflow.contains("--smoke-test"),
            "the {name} workflow must invoke the hidden app smoke-test mode"
        );
        assert!(
            workflow.contains("manual validation"),
            "the {name} workflow must record the OpenCode TUI validation step when CI cannot run it"
        );
        assert!(
            workflow.contains("opencode"),
            "the {name} workflow must attempt or record native OpenCode v2 validation"
        );
    }
}

#[test]
fn platform_layer_names_all_supported_families() {
    use made_platform::PlatformFamily;

    assert_eq!(PlatformFamily::Linux.name(), "linux");
    assert_eq!(PlatformFamily::MacOs.name(), "macos");
    assert_eq!(PlatformFamily::Windows.name(), "windows");
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn supported_linux_x64_host_selects_the_linux_backend() {
    use made_platform::PlatformFamily;

    assert_eq!(PlatformFamily::current(), PlatformFamily::Linux);
}
