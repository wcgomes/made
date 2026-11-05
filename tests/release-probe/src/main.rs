//! Velopack probe for add-release-auto-update tasks 1.2-1.7.
//!
//! The binary is named `made-app` so packaging mirrors the product layout.
//! It is never shipped and is removed by task 10.2.
//!
//! - no arguments / `report`: print a JSON report (installed detection,
//!   channel, version, layout resolution, pending update) and append one line
//!   to `~/made-probe-launch.txt` so a relaunch after apply is observable.
//! - `update <base-url> [--download-only|--apply|--apply-norestart] [--stable-only]`:
//!   check a local feed through a logging source that follows redirects by
//!   hand and prints every hop and every request header it sets.
//! - `capture`: run the built-in sources and a plain ureq agent against a
//!   loopback listener and print the raw request heads seen on the wire.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use velopack::bundle::Manifest;
use velopack::locator::{auto_locate_app_manifest, LocationContext, VelopackLocatorConfig};
use velopack::sources::{GithubSource, HttpSource, NoneSource, UpdateSource};
use velopack::{
    Error, UpdateCheck, UpdateManager, UpdateOptions, VelopackApp, VelopackAsset, VelopackAssetFeed,
};

const USER_AGENT: &str = "made-probe";

fn main() {
    // REL-07 gate: the startup hook must never apply on its own.
    let mut app = VelopackApp::build().set_auto_apply_on_startup(false);
    if let Some(config) = linux_locator() {
        app = app.set_locator(config);
    }
    app.run();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("report") => report(),
        Some("update") => update(&args[1..]),
        Some("capture") => capture(),
        Some(other) => {
            eprintln!("probe: unknown command {other}");
            2
        }
    };
    std::process::exit(code);
}

/// The product keeps Linux update packages in private user data on the same
/// filesystem as the user's files instead of the shared `/var/tmp`.
fn linux_locator() -> Option<VelopackLocatorConfig> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let packages = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".local/share/made-probe/packages");
    let located = auto_locate_app_manifest(LocationContext::FromCurrentExe).ok()?;
    let bin = located.get_current_bin_dir();
    Some(VelopackLocatorConfig {
        RootAppDir: root_of(&located),
        UpdateExePath: located.get_update_path(),
        PackagesDir: packages,
        ManifestPath: bin.join("sq.version"),
        CurrentBinaryDir: bin,
        IsPortable: located.get_is_portable(),
    })
}

#[cfg(target_os = "linux")]
fn root_of(locator: &velopack::locator::VelopackLocator) -> PathBuf {
    locator.get_appimage_path()
}

#[cfg(not(target_os = "linux"))]
fn root_of(locator: &velopack::locator::VelopackLocator) -> PathBuf {
    locator.get_root_dir()
}

fn manager<S: UpdateSource + 'static>(source: S) -> Result<UpdateManager, Error> {
    let options = UpdateOptions {
        MaximumDeltasBeforeFallback: -1,
        ..UpdateOptions::default()
    };
    UpdateManager::new(source, Some(options), linux_locator())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn exists(path: &Path) -> serde_json::Value {
    serde_json::json!({ "path": path.display().to_string(), "exists": path.exists() })
}

fn report() -> i32 {
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let resources = exe_dir.join("..").join("Resources");
    let askpass = if cfg!(windows) { "made-askpass.exe" } else { "made-askpass" };
    let mut report = serde_json::json!({
        "probe": "release-probe",
        "exe": exe.display().to_string(),
        "plugin_in_resources": exists(&resources.join("plugins/opencode-made/package.json")),
        "plugin_next_to_exe": exists(&exe_dir.join("plugins/opencode-made/package.json")),
        "companion_next_to_exe": exists(&exe_dir.join("remote/x86_64-unknown-linux-gnu/made-remote")),
        "askpass_next_to_exe": exists(&exe_dir.join(askpass)),
        "appimage_env": std::env::var("APPIMAGE").ok(),
    });
    match auto_locate_app_manifest(LocationContext::FromCurrentExe) {
        Ok(locator) => {
            report["installed"] = true.into();
            report["version"] = locator.get_manifest_version_full_string().into();
            report["channel"] = locator.get_manifest_channel().into();
            report["id"] = locator.get_manifest_id().into();
            report["root"] = root_of(&locator).display().to_string().into();
            report["update_exe"] = locator.get_update_path().display().to_string().into();
            report["default_packages_dir"] = locator.get_packages_dir().display().to_string().into();
            report["portable"] = locator.get_is_portable().into();
        }
        Err(error) => {
            report["installed"] = false.into();
            report["not_installed_reason"] = error.to_string().into();
        }
    }
    if let Ok(um) = manager(NoneSource {}) {
        report["pending"] = um
            .get_update_pending_restart()
            .map(|asset| asset.Version)
            .into();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    if let Some(home) = home() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!(
            "{stamp} pid={} version={} exe={}\n",
            std::process::id(),
            report["version"].as_str().unwrap_or("none"),
            exe.display()
        );
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("made-probe-launch.txt"))
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
    0
}

/// A feed source that never lets ureq follow a redirect: each hop is printed
/// with its status and location so the redirect chain is observable.
#[derive(Clone)]
struct LoggingSource {
    base: String,
    stable_only: bool,
}

impl LoggingSource {
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build()
            .into()
    }

    fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, Error> {
        let agent = Self::agent();
        let mut current = url.to_owned();
        for hop in 0..8 {
            println!("probe hop {hop}: GET {current} (User-Agent: {USER_AGENT}, Accept: */*)");
            let response = agent
                .get(&current)
                .header("Accept", "*/*")
                .call()
                .map_err(Error::from)?;
            let status = response.status();
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            println!("probe hop {hop}: status {} location {:?}", status.as_u16(), location);
            if status.is_redirection() {
                let next = location.ok_or_else(|| Error::Other("redirect without location".into()))?;
                current = url::Url::parse(&current)
                    .and_then(|base| base.join(&next))
                    .map(|url| url.to_string())
                    .map_err(|e| Error::Other(e.to_string()))?;
                continue;
            }
            if !status.is_success() {
                return Err(Error::Other(format!("HTTP {}", status.as_u16())));
            }
            return Ok(response);
        }
        Err(Error::Other("too many redirects".into()))
    }

    fn join(&self, name: &str) -> String {
        format!("{}/{}", self.base.trim_end_matches('/'), name)
    }
}

impl UpdateSource for LoggingSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &Manifest,
        staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, Error> {
        println!(
            "probe feed: channel={channel} app={} {} staged_user_id_offered={}",
            app.id,
            app.version,
            !staged_user_id.is_empty()
        );
        let mut response = self.get(&self.join(&format!("releases.{channel}.json")))?;
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(Error::from)?;
        let mut feed: VelopackAssetFeed = serde_json::from_str(&text)?;
        for asset in &feed.Assets {
            println!(
                "probe feed asset: {} {} {} size={} sha256={}",
                asset.Type, asset.Version, asset.FileName, asset.Size, asset.SHA256
            );
        }
        if self.stable_only {
            feed.Assets.retain(|asset| {
                semver::Version::parse(&asset.Version)
                    .map(|version| version.pre.is_empty())
                    .unwrap_or(false)
            });
        }
        Ok(feed)
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        _progress: Option<Sender<i16>>,
    ) -> Result<(), Error> {
        let response = self.get(&self.join(&asset.FileName))?;
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(local_file)?;
        std::io::copy(&mut reader, &mut file)?;
        Ok(())
    }
}

fn update(args: &[String]) -> i32 {
    let Some(base) = args.first() else {
        eprintln!("probe: update needs a base URL");
        return 2;
    };
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let source = LoggingSource {
        base: base.clone(),
        stable_only: flag("--stable-only"),
    };
    let um = match manager(source) {
        Ok(um) => um,
        Err(error) => {
            println!("probe update: not installed: {error}");
            return 3;
        }
    };
    println!("probe update: current {}", um.get_current_version_as_string());
    let update = match um.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(update)) => update,
        Ok(UpdateCheck::NoUpdateAvailable) => {
            println!("probe update: no update available");
            return 0;
        }
        Ok(UpdateCheck::RemoteIsEmpty) => {
            println!("probe update: remote is empty");
            return 0;
        }
        Err(error) => {
            println!("probe update: check failed: {error}");
            return 4;
        }
    };
    let target = update.TargetFullRelease.clone();
    println!("probe update: offered {} ({})", target.Version, target.FileName);
    if let Err(error) = um.download_updates(&update, None) {
        println!("probe update: download failed: {error}");
        return 5;
    }
    println!(
        "probe update: downloaded; pending={:?}",
        um.get_update_pending_restart().map(|asset| asset.Version)
    );
    if flag("--apply") {
        println!("probe update: apply and restart");
        let _ = std::io::stdout().flush();
        if let Err(error) = um.apply_updates_and_restart(&target) {
            println!("probe update: apply failed: {error}");
            return 6;
        }
    } else if flag("--apply-norestart") {
        println!("probe update: apply without restart after this process exits");
        if let Err(error) = um.wait_exit_then_apply_updates(&target, true, false, Vec::<String>::new()) {
            println!("probe update: apply failed: {error}");
            return 6;
        }
    }
    0
}

/// One loopback listener that answers each connection with a fixed response
/// and returns the raw request heads it received.
fn listen(responses: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let handle = std::thread::spawn(move || {
        let mut heads = Vec::new();
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buffer = Vec::new();
            let mut byte = [0u8; 1];
            while !buffer.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => buffer.push(byte[0]),
                    _ => break,
                }
            }
            heads.push(String::from_utf8_lossy(&buffer).into_owned());
            let _ = stream.write_all(response.as_bytes());
        }
        heads
    });
    (port, handle)
}

fn ok_json(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn print_heads(label: &str, heads: Vec<String>) {
    for (index, head) in heads.into_iter().enumerate() {
        println!("probe capture {label} #{index}:\n{}", head.trim_end());
    }
}

fn capture() -> i32 {
    let app = Manifest {
        id: "Made".into(),
        version: semver::Version::new(0, 1, 0),
        ..Manifest::default()
    };
    let staged = "11111111-2222-3333-4444-555555555555";

    let (port, handle) = listen(vec![ok_json(r#"{"Assets":[]}"#)]);
    let result = HttpSource::new(format!("http://127.0.0.1:{port}/feed/")).get_release_feed("linux-x64", &app, staged);
    println!("probe capture builtin-http result ok={}", result.is_ok());
    print_heads("builtin-http", handle.join().unwrap_or_default());

    let (port, handle) = listen(vec![ok_json("[]")]);
    let result = GithubSource::new(&format!("http://127.0.0.1:{port}/owner/repo"), None, false)
        .get_release_feed("linux-x64", &app, staged);
    println!("probe capture builtin-github result ok={}", result.is_ok());
    print_heads("builtin-github", handle.join().unwrap_or_default());

    let (target_port, target) = listen(vec![ok_json(r#"{"Assets":[]}"#)]);
    let (port, handle) = listen(vec![format!(
        "HTTP/1.1 302 Found\r\nLocation: http://localhost:{target_port}/elsewhere/releases.linux-x64.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )]);
    let result = HttpSource::new(format!("http://127.0.0.1:{port}/")).get_release_feed("linux-x64", &app, staged);
    println!("probe capture builtin-redirect result ok={} (ok means a cross-host redirect was followed)", result.is_ok());
    print_heads("builtin-redirect-first", handle.join().unwrap_or_default());
    print_heads("builtin-redirect-target", target.join().unwrap_or_default());

    let (port, handle) = listen(vec![ok_json(r#"{"Assets":[]}"#)]);
    let agent = LoggingSource::agent();
    let result = agent
        .get(&format!("http://127.0.0.1:{port}/o/r/releases/latest/download/releases.linux-x64.json"))
        .header("Accept", "*/*")
        .call();
    println!("probe capture custom result ok={}", result.is_ok());
    print_heads("custom", handle.join().unwrap_or_default());

    let https_only: ureq::Agent = ureq::Agent::config_builder().https_only(true).build().into();
    let refused = https_only.get("http://127.0.0.1:9/").call();
    println!("probe capture https-only plain-http refused={}", refused.is_err());
    0
}
