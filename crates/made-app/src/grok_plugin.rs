//! Install the shipped `grok-made` package after PATH detection reports grok.
//!
//! The lookup itself stays metadata-only. This worker compares dotted numeric
//! `plugin.json` versions and, when the installed copy is missing or older,
//! runs `grok plugin install <absolute shipped path> --trust`. It never edits
//! `config.toml` and never runs from an explicit catalog.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use made_bridge::config::CONFIG_CONTENT_ENV;
use made_platform::{find_executable_on_path, host_platform, CompatibleTui, Platform};

/// Outcome of the one-shot ensure worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnsureOutcome {
    /// The install command succeeded.
    Installed,
    /// The installed version is equal or newer, so the command was skipped.
    Skipped,
    /// The version check or the install command failed.
    Unavailable,
}

/// Start the ensure worker once per process.
///
/// `None` means this process already started it. A disconnected receiver means
/// the thread could not be spawned.
pub(crate) fn spawn_ensure() -> Option<Receiver<EnsureOutcome>> {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, AtomicOrdering::AcqRel) {
        return None;
    }
    let (sender, receiver) = mpsc::channel();
    match thread::Builder::new()
        .name("made-grok-plugin".to_owned())
        .spawn(move || {
            let _ = sender.send(ensure_shipped_grok_plugin());
        }) {
        Ok(_) => Some(receiver),
        Err(error) => {
            eprintln!("made: grok plugin install thread could not start ({error})");
            Some(receiver)
        }
    }
}

fn ensure_shipped_grok_plugin() -> EnsureOutcome {
    let Some(grok) = detected_grok() else {
        return EnsureOutcome::Unavailable;
    };
    let Some((package, shipped)) = shipped_grok_plugin() else {
        return EnsureOutcome::Unavailable;
    };
    let Some(home) = grok_home_dir() else {
        return EnsureOutcome::Unavailable;
    };
    if let Some(installed) = installed_grok_made_version(&home) {
        if cmp_dotted(&installed, &shipped) != Ordering::Less {
            return EnsureOutcome::Skipped;
        }
    }
    if install_shipped_plugin(&grok, &package) {
        EnsureOutcome::Installed
    } else {
        EnsureOutcome::Unavailable
    }
}

fn detected_grok() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    find_executable_on_path(OsStr::new(CompatibleTui::Grok.program()), &path)
}

fn shipped_grok_plugin() -> Option<(PathBuf, Vec<u64>)> {
    let paths = host_platform().app_paths().ok()?;
    let directory = std::fs::canonicalize(paths.plugin_dir.with_file_name("grok-made")).ok()?;
    if !directory.is_dir() {
        return None;
    }
    let version = manifest_version(&directory.join("plugin.json"))?;
    Some((directory, version))
}

fn grok_home_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("GROK_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".grok"))
}

fn installed_grok_made_version(home: &Path) -> Option<Vec<u64>> {
    let entries = std::fs::read_dir(home.join("installed-plugins")).ok()?;
    let mut best: Option<Vec<u64>> = None;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(version) = manifest_version(&entry.path().join("plugin.json")) else {
            continue;
        };
        best = Some(match best {
            Some(current) if cmp_dotted(&current, &version) != Ordering::Less => current,
            _ => version,
        });
    }
    best
}

fn manifest_version(path: &Path) -> Option<Vec<u64>> {
    let text = read_limited(path)?;
    if json_string(&text, "name").as_deref() != Some("grok-made") {
        return None;
    }
    parse_dotted_version(&json_string(&text, "version")?)
}

fn read_limited(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(65_536).read_to_string(&mut text).ok()?;
    Some(text)
}

fn install_shipped_plugin(grok: &Path, package: &Path) -> bool {
    let mut command = Command::new(grok);
    command
        .arg("plugin")
        .arg("install")
        .arg(package)
        .arg("--trust")
        .env_remove(CONFIG_CONTENT_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn parse_dotted_version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for part in text.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    Some(parts)
}

fn cmp_dotted(left: &[u64], right: &[u64]) -> Ordering {
    let len = left.len().max(right.len());
    for index in 0..len {
        let ordering = left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn json_string(object: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let rest = object.split(&pattern).nth(1)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => out.push(chars.next().unwrap_or('\\')),
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_versions_compare_numerically() {
        let older = parse_dotted_version("0.2.0").unwrap();
        let shipped = parse_dotted_version("0.10.0").unwrap();
        let equal_short = parse_dotted_version("0.10").unwrap();
        let newer = parse_dotted_version("0.10.1").unwrap();
        assert_eq!(cmp_dotted(&older, &shipped), Ordering::Less);
        assert_eq!(cmp_dotted(&equal_short, &shipped), Ordering::Equal);
        assert_eq!(cmp_dotted(&newer, &shipped), Ordering::Greater);
        assert!(parse_dotted_version("0.10.0-beta").is_none());
        assert!(parse_dotted_version("").is_none());
    }

    #[test]
    fn manifest_version_requires_the_grok_made_name() {
        let root = tempfile::tempdir().unwrap();
        let manifest = root.path().join("plugin.json");
        std::fs::write(
            &manifest,
            "{\n  \"name\": \"grok-made\",\n  \"version\": \"0.10.0\"\n}\n",
        )
        .unwrap();
        assert_eq!(manifest_version(&manifest).unwrap(), vec![0, 10, 0]);
        std::fs::write(
            &manifest,
            "{\n  \"name\": \"other\",\n  \"version\": \"9.0.0\"\n}\n",
        )
        .unwrap();
        assert!(manifest_version(&manifest).is_none());
    }
}
