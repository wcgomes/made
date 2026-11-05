use made_remote::{
    protocol::*,
    supervisor::{read_envelope, runtime_directory, write_envelope},
};
#[test]
fn frontend_handshake_is_bounded_versioned_and_secret_is_not_a_runtime_locator() {
    let identity = Identity {
        authority: AuthorityId::random().unwrap(),
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let request = WireEnvelope::new(
        identity.clone(),
        1,
        Operation::Open {
            directory: RemotePath::new(b"/project".to_vec()).unwrap(),
            target: LaunchTarget::Shell,
            rows: 24,
            cols: 80,
        },
    );
    let mut bytes = Vec::new();
    write_envelope(&mut bytes, &request).unwrap();
    assert_eq!(read_envelope(&mut bytes.as_slice()).unwrap(), request);
    let path = runtime_directory(std::path::Path::new("/private/made"), &identity).unwrap();
    assert!(path.starts_with("/private/made/remote/runtime"));
    assert!(!path
        .to_string_lossy()
        .contains(&serde_json::to_string(&identity.tab).unwrap()));
    let oversized = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
    assert!(read_envelope(&mut oversized.as_slice()).is_err());
}

#[cfg(unix)]
mod legacy_stage {
    use made_remote::protocol::RemoteTarget;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        fs,
        io::Write,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        process::{Command, Output, Stdio},
    };

    pub const ARTIFACTS: [&str; 11] = [
        "made-remote",
        "plugins/opencode-made/package.json",
        "plugins/opencode-made/tui.ts",
        "plugins/opencode-made/index.ts",
        "plugins/opencode-made/src/protocol.ts",
        "plugins/opencode-made/src/integration.ts",
        "plugins/grok-made/plugin.json",
        "plugins/grok-made/README.md",
        "plugins/grok-made/hooks/hooks.json",
        "plugins/grok-made/hooks/status_test.py",
        "plugins/grok-made/hooks/bin/status.py",
    ];
    pub const UPDATE_REQUIRED: &str = "MADE-STAGE-UPDATE-REQUIRED";

    pub fn target() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
            ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("macos", "aarch64") => "aarch64-apple-darwin",
            _ => "x86_64-unknown-linux-gnu",
        }
    }

    fn target_record() -> RemoteTarget {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => RemoteTarget::LinuxX64,
            ("linux", "aarch64") => RemoteTarget::LinuxArm64,
            ("macos", "x86_64") => RemoteTarget::MacosX64,
            ("macos", "aarch64") => RemoteTarget::MacosArm64,
            _ => RemoteTarget::LinuxX64,
        }
    }

    pub fn sha256_hex(bytes: &[u8]) -> String {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blob");
        fs::write(&path, bytes).unwrap();
        made_bridge::config_guard::ConfigSnapshot::capture(&path)
            .unwrap()
            .sha256_hex()
            .unwrap()
    }

    pub fn home() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        (temp, home)
    }

    pub fn data_root(home: &Path) -> PathBuf {
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Made")
        } else {
            home.join("data/made")
        }
    }

    pub fn version_root(home: &Path, version: &str) -> PathBuf {
        data_root(home).join("remote").join(version)
    }

    pub fn destination(home: &Path, version: &str) -> PathBuf {
        version_root(home, version).join(target())
    }

    pub fn files(content: &[u8]) -> Vec<(&'static str, &[u8])> {
        ARTIFACTS.iter().map(|name| (*name, content)).collect()
    }

    pub fn manifest(
        update: bool,
        version: &str,
        files: &[(&'static str, &[u8])],
    ) -> (String, Vec<u8>) {
        manifest_with_declared_hash(update, version, files, None)
    }

    pub fn manifest_with_declared_hash(
        update: bool,
        version: &str,
        files: &[(&'static str, &[u8])],
        declared: Option<&str>,
    ) -> (String, Vec<u8>) {
        let mut records = Vec::new();
        let mut payload = Vec::new();
        for (name, content) in files {
            let hash = sha256_hex(content);
            records.push(json!({
                "name": name,
                "size": content.len(),
                "sha256": declared.unwrap_or(&hash),
            }));
            payload.extend_from_slice(content);
        }
        let manifest = json!({
            "consent": true,
            "update": update,
            "version": version,
            "target": target_record(),
            "files": records,
        });
        (manifest.to_string(), payload)
    }

    /// Run the shipped `--stage` mode with a framed manifest followed by the
    /// exact file stream, isolated from the real user data directory.
    pub fn run(home: &Path, manifest: &str, payload: &[u8]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_made-remote"))
            .arg("--stage")
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = (manifest.len() as u32).to_be_bytes().to_vec();
        input.extend_from_slice(manifest.as_bytes());
        input.extend_from_slice(payload);
        child.stdin.take().unwrap().write_all(&input).unwrap();
        child.wait_with_output().unwrap()
    }

    pub type Fingerprint = BTreeMap<PathBuf, (u64, i64, i64, u32, u64)>;

    pub fn fingerprint(root: &Path) -> Fingerprint {
        let mut map = Fingerprint::new();
        collect(root, root, &mut map);
        map
    }

    fn collect(root: &Path, path: &Path, map: &mut Fingerprint) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            map.insert(
                relative,
                (
                    meta.len(),
                    meta.mtime(),
                    meta.mtime_nsec(),
                    meta.permissions().mode() & 0o777,
                    meta.ino(),
                ),
            );
            if meta.is_dir() {
                collect(root, &path, map);
            }
        }
    }

    pub fn entries(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    pub fn version_root_entries_for(target: &str) -> Vec<String> {
        let mut entries = vec![target.to_owned(), "stage.lock".to_owned()];
        entries.sort();
        entries
    }

    #[test]
    fn version_root_entries_assume_sorted_order_for_x86_64_targets() {
        assert_eq!(
            version_root_entries_for("x86_64-apple-darwin"),
            vec!["stage.lock".to_owned(), "x86_64-apple-darwin".to_owned()]
        );
        assert_eq!(
            version_root_entries_for("x86_64-unknown-linux-gnu"),
            vec![
                "stage.lock".to_owned(),
                "x86_64-unknown-linux-gnu".to_owned()
            ]
        );
        assert_eq!(
            version_root_entries_for("aarch64-apple-darwin"),
            vec!["aarch64-apple-darwin".to_owned(), "stage.lock".to_owned()]
        );
    }

    pub fn assert_no_temporaries(root: &Path) {
        for entry in fs::read_dir(root).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.starts_with(".stage-") && !name.starts_with(".replace-"),
                "temporary directory accumulated: {name}"
            );
        }
    }

    pub fn child_pids(pid: u32) -> Vec<u32> {
        let Ok(output) = Command::new("/bin/ps")
            .args(["-o", "pid=", "-P", &pid.to_string()])
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect()
    }

    pub fn pid_alive(pid: u32) -> bool {
        Command::new("/bin/ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[test]
#[cfg(unix)]
fn legacy_stage_exact_reuse_and_update_required_are_distinct_without_writes() {
    use legacy_stage::*;
    let (_temp, home) = home();
    let (initial_manifest, initial_payload) = manifest(false, "1.0", &files(b"a"));
    let initial = run(&home, &initial_manifest, &initial_payload);
    assert!(
        initial.status.success(),
        "initial legacy staging failed: {:?}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let destination = destination(&home, "1.0");
    let version_root = version_root(&home, "1.0");
    let before = fingerprint(&destination);

    let reuse = run(&home, &initial_manifest, &initial_payload);
    assert!(reuse.status.success(), "exact reuse must stay success");
    assert_eq!(
        fingerprint(&destination),
        before,
        "exact reuse must not rewrite any artifact"
    );

    let (changed_manifest, changed_payload) = manifest(false, "1.0", &files(b"b"));
    let changed = run(&home, &changed_manifest, &changed_payload);
    assert_eq!(changed.status.code(), Some(74));
    let stderr = String::from_utf8_lossy(&changed.stderr);
    assert!(stderr.contains(UPDATE_REQUIRED));
    assert!(!stderr.contains("operation failed"));
    assert!(!stderr.contains(&sha256_hex(b"b")));
    assert!(!stderr.contains(home.to_str().unwrap()));
    assert_eq!(
        fingerprint(&destination),
        before,
        "an unapproved same-version mismatch must write and delete nothing"
    );
    assert_no_temporaries(&version_root);
    assert_eq!(entries(&version_root), version_root_entries_for(target()));
}

#[test]
#[cfg(unix)]
fn legacy_approved_update_swaps_cleans_keeps_the_old_process_and_future_paths() {
    use legacy_stage::*;
    use std::{
        fs,
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };
    let (_temp, home) = home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let (old_manifest, old_payload) = manifest(false, "1.0", &files(old));
    let initial = run(&home, &old_manifest, &old_payload);
    assert!(initial.status.success(), "initial legacy staging failed");
    let destination = destination(&home, "1.0");
    let version_root = version_root(&home, "1.0");
    let mut old_process = Command::new(destination.join("made-remote"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let old_pid = old_process.id();
    assert!(
        child_pids(old_pid).is_empty(),
        "the old-process fixture must exec its sleeper so no orphan remains"
    );

    let before_approval = fingerprint(&destination);
    let (refused_manifest, refused_payload) = manifest(false, "1.0", &files(new));
    let refused = run(&home, &refused_manifest, &refused_payload);
    assert_eq!(refused.status.code(), Some(74));
    assert_eq!(
        fingerprint(&destination),
        before_approval,
        "no write may happen before the separate update approval"
    );
    assert!(
        old_process.try_wait().unwrap().is_none(),
        "an unapproved attempt must not signal a running companion"
    );

    let (new_manifest, new_payload) = manifest(true, "1.0", &files(new));
    let update = run(&home, &new_manifest, &new_payload);
    assert!(
        update.status.success(),
        "approved legacy update failed: {:?}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert_eq!(fs::read(destination.join("made-remote")).unwrap(), new);
    assert_eq!(
        fs::metadata(destination.join("made-remote"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(destination.join("plugins/grok-made/hooks/bin/status.py"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(destination.join("plugins/opencode-made/tui.ts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        old_process.try_wait().unwrap().is_none(),
        "the update must not signal a running companion"
    );

    // Companion and Grok install resolve the same current version/target path.
    for command in [
        made_platform::ssh::SshCommand::Companion,
        made_platform::ssh::SshCommand::GrokPluginInstall,
    ] {
        let mut child = Command::new("/bin/sh")
            .args(["-c", command.script()])
            .env_clear()
            .env("HOME", &home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("1.0\n{}\n", target()).as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.stdout, b"updated\n",
            "{command:?} must run the updated companion at the same path"
        );
    }

    assert_eq!(
        entries(&version_root),
        version_root_entries_for(target()),
        "no build or content directory may accumulate"
    );
    assert_eq!(
        entries(&data_root(&home).join("remote")),
        vec!["1.0".to_owned()]
    );
    assert_no_temporaries(&version_root);

    // A fourth attempt over the updated bytes is exact reuse without writes.
    let (reuse_manifest, reuse_payload) = manifest(false, "1.0", &files(new));
    let updated = fingerprint(&destination);
    let reuse = run(&home, &reuse_manifest, &reuse_payload);
    assert!(reuse.status.success(), "post-update exact reuse failed");
    assert_eq!(
        fingerprint(&destination),
        updated,
        "post-update exact reuse must not rewrite artifacts"
    );

    let _ = old_process.kill();
    let _ = old_process.wait();
    assert!(
        !pid_alive(old_pid),
        "the recorded old-process PID must be the sleeper itself"
    );
    assert!(
        child_pids(old_pid).is_empty(),
        "killing the recorded PID must not leave an orphan sleep child"
    );
}

#[test]
#[cfg(unix)]
fn legacy_approved_update_failure_before_swap_keeps_the_original_byte_identical() {
    use legacy_stage::*;
    let (_temp, home) = home();
    let old = b"#!/bin/sh\nexec sleep 30\n";
    let new = b"#!/bin/sh\nprintf 'updated\\n'\n";
    let (old_manifest, old_payload) = manifest(false, "1.0", &files(old));
    let initial = run(&home, &old_manifest, &old_payload);
    assert!(initial.status.success(), "initial legacy staging failed");
    let destination = destination(&home, "1.0");
    let version_root = version_root(&home, "1.0");
    let before = fingerprint(&destination);

    let (bad_manifest, bad_payload) =
        manifest_with_declared_hash(true, "1.0", &files(new), Some(&"0".repeat(64)));
    let bad = run(&home, &bad_manifest, &bad_payload);
    assert!(!bad.status.success(), "a corrupt approved update must fail");
    assert_eq!(
        fingerprint(&destination),
        before,
        "a pre-swap hash failure must leave the original untouched"
    );
    assert_no_temporaries(&version_root);

    let (short_manifest, mut short_payload) = manifest(true, "1.0", &files(new));
    short_payload.truncate(short_payload.len() - 1);
    let short = run(&home, &short_manifest, &short_payload);
    assert!(!short.status.success(), "a truncated update must fail");
    let stderr = String::from_utf8_lossy(&short.stderr);
    assert!(stderr.contains("made-remote: operation failed"));
    assert!(!stderr.contains(home.to_str().unwrap()));
    assert_eq!(
        fingerprint(&destination),
        before,
        "a pre-swap size failure must leave the original untouched"
    );
    assert_no_temporaries(&version_root);
}

#[test]
#[cfg(unix)]
fn legacy_leaf_symlink_is_update_required_but_a_symlinked_ancestor_is_unsafe() {
    use legacy_stage::*;
    use std::{fs, os::unix::fs::symlink};
    let (_temp, home) = home();
    let (manifest, payload) = manifest(false, "1.0", &files(b"a"));
    let initial = run(&home, &manifest, &payload);
    assert!(initial.status.success(), "initial legacy staging failed");
    let destination = destination(&home, "1.0");
    let version_root = version_root(&home, "1.0");

    // A leaf symlink is an artifact mismatch: update-required, but it must not
    // be followed, hashed or rewritten.
    let leaf = destination.join("plugins/opencode-made/package.json");
    let sibling = destination.join("plugins/opencode-made/tui.ts");
    fs::remove_file(&leaf).unwrap();
    symlink(&sibling, &leaf).unwrap();
    let leaf_attempt = run(&home, &manifest, &payload);
    assert_eq!(
        leaf_attempt.status.code(),
        Some(74),
        "a leaf artifact symlink must remain update-required: {:?}",
        String::from_utf8_lossy(&leaf_attempt.stderr)
    );
    assert!(String::from_utf8_lossy(&leaf_attempt.stderr).contains(UPDATE_REQUIRED));
    assert!(fs::symlink_metadata(&leaf)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&sibling).unwrap(), b"a");

    // A symlinked ancestor is a security failure, distinct from
    // update-required, and must not be hashed through.
    fs::remove_file(&leaf).unwrap();
    fs::write(&leaf, b"a").unwrap();
    let parent = destination.join("plugins/opencode-made");
    let moved = home.join("attacker-opencode-made");
    fs::rename(&parent, &moved).unwrap();
    symlink(&moved, &parent).unwrap();

    let attempt = run(&home, &manifest, &payload);
    assert!(
        !attempt.status.success(),
        "a symlinked artifact parent must fail: {:?}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_ne!(
        attempt.status.code(),
        Some(74),
        "an ancestor symlink is unsafe, not update-required"
    );
    let stderr = String::from_utf8_lossy(&attempt.stderr);
    assert!(!stderr.contains(UPDATE_REQUIRED));
    assert!(stderr.contains("made-remote: operation failed"));
    assert!(fs::symlink_metadata(&parent)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&parent).unwrap(), moved);
    assert_eq!(
        fs::read(moved.join("package.json")).unwrap(),
        b"a",
        "the hostile target must not be modified"
    );
    assert_no_temporaries(&version_root);
}

#[test]
#[cfg(unix)]
fn legacy_recovers_a_preserved_original_before_the_next_attempt() {
    use legacy_stage::*;
    use std::fs;
    let (_temp, home) = home();
    let old = b"#!old\n";
    let new = b"#!new\n";
    let (old_manifest, old_payload) = manifest(false, "1.0", &files(old));
    let initial = run(&home, &old_manifest, &old_payload);
    assert!(initial.status.success(), "initial legacy staging failed");
    let destination = destination(&home, "1.0");
    let version_root = version_root(&home, "1.0");
    let replaced = version_root.join(".replace");
    fs::rename(&destination, &replaced).unwrap();

    let (new_manifest, new_payload) = manifest(false, "1.0", &files(new));
    let attempt = run(&home, &new_manifest, &new_payload);
    assert_eq!(attempt.status.code(), Some(74));
    assert!(String::from_utf8_lossy(&attempt.stderr).contains(UPDATE_REQUIRED));
    assert_eq!(
        fs::read(destination.join("made-remote")).unwrap(),
        old,
        "the next attempt must restore the preserved original"
    );
    assert!(
        !replaced.exists(),
        "restoration must consume the recovery path"
    );
    assert_no_temporaries(&version_root);
}
