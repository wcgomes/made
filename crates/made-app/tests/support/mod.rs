#![allow(dead_code)] // each test binary uses a different subset of these helpers

//! Shared support for made-app integration tests that run the app in an
//! isolated child process.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use made_bridge::config::PLUGIN_REQUIRED_FILES;
use made_platform::{install_root_from_current_executable, repository_root, PlatformFamily};

/// Plugin packages staged inside the running test executable's install root.
///
/// The app resolves its plugin package from the platform install root: a
/// release build always loads `<install_root>/plugins/opencode-made`, and the
/// documented [`PLUGIN_DIR_ENV`] override is accepted only when it
/// canonicalizes inside that root. For a test executable the install root is
/// the cargo `deps` directory, and a cargo test layout has no packaged plugin
/// beside the test binary, so release children must be handed an explicit
/// fixture. Development builds accept any override and otherwise fall back to
/// the repository package. Staging the repository package files inside the
/// install root gives every profile the same fixture through the same seam.
pub struct StagedPlugins {
    root: PathBuf,
}

static STAGED_SERIAL: AtomicU64 = AtomicU64::new(0);

impl StagedPlugins {
    /// Copy the files the app requires from the repository plugin packages
    /// into a unique directory inside the install root of the running test
    /// executable.
    ///
    /// The directory name is unique per staging so parallel tests never share
    /// state, and [`Drop`] removes it once the child that consumed it is done.
    pub fn stage(label: &str) -> Self {
        let install_root = install_root_from_current_executable(PlatformFamily::current())
            .expect("test executable install root");
        let serial = STAGED_SERIAL.fetch_add(1, Ordering::Relaxed);
        let root = install_root.join(format!(
            ".made-plugins-{label}-{}-{serial}",
            std::process::id()
        ));
        let repository = repository_root().join("plugins");
        for (package, files) in [
            ("opencode-made", PLUGIN_REQUIRED_FILES.as_slice()),
            ("grok-made", ["plugin.json"].as_slice()),
        ] {
            let package_dir = root.join(package);
            std::fs::create_dir_all(&package_dir).expect("staged package directory");
            for file in files {
                let source = repository.join(package).join(file);
                let contents = std::fs::read(&source).unwrap_or_else(|error| {
                    panic!("repository plugin file {source:?} is required: {error}")
                });
                std::fs::write(package_dir.join(file), contents).expect("staged plugin file");
            }
        }
        Self { root }
    }

    /// A staged package directory to hand the app or the test.
    pub fn package(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for StagedPlugins {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Stage the shipped grok-made copy where the app resolves it: beside the
/// plugins of the running test executable's install root. The content is the
/// same for every caller and is written through a rename, so parallel tests in
/// one binary can all call this without observing a partial file.
pub fn stage_shipped_grok_plugin() -> PathBuf {
    let directory = install_root_from_current_executable(PlatformFamily::current())
        .expect("test executable install root")
        .join("plugins")
        .join("grok-made");
    std::fs::create_dir_all(&directory).expect("staged grok-made directory");
    let source = repository_root()
        .join("plugins")
        .join("grok-made")
        .join("plugin.json");
    let contents = std::fs::read(&source)
        .unwrap_or_else(|error| panic!("repository plugin file {source:?} is required: {error}"));
    let scratch = directory.join(format!(".plugin.json.{}", std::process::id()));
    std::fs::write(&scratch, contents).expect("staged grok-made manifest");
    std::fs::rename(&scratch, directory.join("plugin.json")).expect("staged grok-made manifest");
    directory
}
