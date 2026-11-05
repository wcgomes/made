//! `stage-grok-plugin.sh` produces the shipped grok-made package from a source
//! tree and a built hook. The test stages fixtures, not the repository source,
//! so it does not depend on the source `hooks.json` form.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn workspace_version() -> String {
    let manifest = std::fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    let section = manifest
        .split("[workspace.package]")
        .nth(1)
        .expect("workspace package");
    section
        .lines()
        .find_map(|line| line.strip_prefix("version = "))
        .expect("workspace version")
        .trim_matches('"')
        .to_string()
}

const FIXTURE_HOOKS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"bin/made-grok-status","timeout":5}]}]}}
"#;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("hooks/bin")).unwrap();
        std::fs::write(
            source.join("plugin.json"),
            "{\n  \"name\": \"grok-made\",\n  \"version\": \"9.9.9\",\n  \"description\": \"fixture\"\n}\n",
        )
        .unwrap();
        std::fs::write(source.join("README.md"), "# fixture\n").unwrap();
        std::fs::write(source.join("hooks/hooks.json"), FIXTURE_HOOKS).unwrap();
        // Files the staged package must never carry.
        std::fs::write(source.join("hooks/test_hook.py"), "print()\n").unwrap();
        std::fs::write(source.join("hooks/bin/hook.py"), "print()\n").unwrap();
        std::fs::write(root.path().join("hook"), b"\x7fELF fixture hook\n").unwrap();
        std::fs::set_permissions(
            root.path().join("hook"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        Self { root }
    }

    fn out(&self) -> PathBuf {
        self.root.path().join("out/plugins/grok-made")
    }

    fn stage(&self, target: &str) -> Output {
        Command::new("bash")
            .arg(repo_root().join(".github/scripts/stage-grok-plugin.sh"))
            .arg(target)
            .arg(self.root.path().join("hook"))
            .arg(self.out())
            .env("MADE_GROK_PLUGIN_SOURCE", self.root.path().join("source"))
            .output()
            .expect("run the staging script")
    }
}

fn files_under(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, found);
            } else {
                found.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found.sort();
    found
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn stages_the_exact_file_set_with_an_executable_hook() {
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ] {
        let fixture = Fixture::new();
        let output = fixture.stage(target);
        assert!(
            output.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let out = fixture.out();
        assert_eq!(
            files_under(&out),
            [
                "README.md",
                "hooks/bin/made-grok-status",
                "hooks/hooks.json",
                "plugin.json"
            ],
            "{target}"
        );
        assert_eq!(mode(&out.join("hooks/bin/made-grok-status")), 0o755);
        assert_eq!(
            std::fs::read(out.join("hooks/bin/made-grok-status")).unwrap(),
            std::fs::read(fixture.root.path().join("hook")).unwrap()
        );
        // The POSIX hooks.json is the source file, byte for byte.
        assert_eq!(
            std::fs::read_to_string(out.join("hooks/hooks.json")).unwrap(),
            FIXTURE_HOOKS
        );
        // The staged version is the workspace version, not the source's.
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("plugin.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["version"], workspace_version());
        assert_eq!(manifest["name"], "grok-made");
        assert_eq!(manifest["description"], "fixture");
    }
}

#[test]
fn windows_target_generates_the_exe_hooks_json() {
    let fixture = Fixture::new();
    let output = fixture.stage("x86_64-pc-windows-msvc");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = fixture.out();
    assert_eq!(
        files_under(&out),
        [
            "README.md",
            "hooks/bin/made-grok-status.exe",
            "hooks/hooks.json",
            "plugin.json"
        ]
    );
    let hooks = std::fs::read_to_string(out.join("hooks/hooks.json")).unwrap();
    assert!(
        hooks.contains("\"command\":\"bin/made-grok-status.exe\""),
        "{hooks}"
    );
    assert!(!hooks.contains("bin/made-grok-status\""), "{hooks}");
    // Everything but the command name is the source file.
    assert_eq!(
        hooks.replace("made-grok-status.exe", "made-grok-status"),
        FIXTURE_HOOKS
    );
}

#[test]
fn restaging_replaces_the_output_and_bad_input_fails() {
    let fixture = Fixture::new();
    assert!(fixture.stage("x86_64-apple-darwin").status.success());
    std::fs::write(fixture.out().join("stale.py"), "x").unwrap();
    assert!(fixture.stage("x86_64-apple-darwin").status.success());
    assert!(!fixture.out().join("stale.py").exists());

    let unsupported = fixture.stage("aarch64-pc-windows-msvc");
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported"));

    std::fs::remove_file(fixture.root.path().join("hook")).unwrap();
    let missing = fixture.stage("x86_64-apple-darwin");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("hook binary"));
}
