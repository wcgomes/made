//! Update capability contract (add-release-auto-update task 4.2).
//!
//! The cases that inject a transport (tamper, redirect, allowlist, the
//! captured check request, download and kept-update round trips) compile only
//! with the non-default `update-test-seam` feature:
//! `cargo test -p made-platform --features update-test-seam --test update`.

use std::io::Write;
use std::path::Path;

use made_platform::update::{
    check_url, select_newer, verify_package, ReadyRecord, ReleaseSource, UpdateError, Updater,
    ALLOWED_HOSTS, TEST_SEAM_ENABLED, UPDATE_CHANNEL, UPDATE_REPOSITORY,
};
use semver::Version;
use sha2::{Digest, Sha256};
use velopack::VelopackAsset;

fn nuspec(id: &str, version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2010/07/nuspec.xsd">
  <metadata><id>{id}</id><version>{version}</version><title>{id}</title>
  <authors>Made</authors><description>Made</description><mainExe>made-app</mainExe></metadata>
</package>"#
    )
}

fn nupkg(id: &str, version: &str) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("{id}.nuspec"), options).unwrap();
        zip.write_all(nuspec(id, version).as_bytes()).unwrap();
        zip.start_file("lib/app/payload.bin", options).unwrap();
        zip.write_all(&[7u8; 4096]).unwrap();
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn asset(version: &str, bytes: &[u8]) -> VelopackAsset {
    VelopackAsset {
        PackageId: "Made".into(),
        Version: version.into(),
        Type: "Full".into(),
        FileName: format!("Made-{version}-{UPDATE_CHANNEL}-full.nupkg"),
        SHA256: sha256(bytes).to_uppercase(),
        SHA1: String::new(),
        Size: bytes.len() as u64,
        ..VelopackAsset::default()
    }
}

fn record(version: &str, file: &Path, bytes: &[u8]) -> ReadyRecord {
    ReadyRecord {
        version: version.into(),
        file: file.file_name().unwrap().to_string_lossy().into_owned(),
        size: bytes.len() as u64,
        sha256: sha256(bytes),
    }
}

#[test]
fn non_installed_builds_report_not_applicable() {
    // The test binary is a non-installed build: no installation, no updater.
    assert!(made_platform::update::Installation::detect().is_none());
    assert!(Updater::detect().is_none());
}

#[test]
fn each_target_uses_its_own_channel() {
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "win-x64",
        ("macos", "aarch64") => "osx-arm64",
        ("macos", "x86_64") => "osx-x64",
        ("linux", "x86_64") => "linux-x64",
        _ => "unsupported",
    };
    assert_eq!(UPDATE_CHANNEL, expected);
    assert!(ReleaseSource::feed_url().ends_with(&format!("/releases.{expected}.json")));
}

#[test]
fn only_strictly_newer_stable_versions_are_offered() {
    let bytes = nupkg("Made", "0.0.0");
    let feed = vec![
        asset("0.2.1", &bytes),
        asset("0.3.0", &bytes),
        asset("9.0.0-beta.1", &bytes),
        asset("0.3.1+build.7", &bytes),
        VelopackAsset {
            SHA256: String::new(),
            ..asset("0.4.0", &bytes)
        },
        VelopackAsset {
            Type: "Delta".into(),
            ..asset("0.5.0", &bytes)
        },
        VelopackAsset {
            PackageId: "Other".into(),
            ..asset("0.6.0", &bytes)
        },
        VelopackAsset {
            FileName: "../evil-full.nupkg".into(),
            ..asset("0.7.0", &bytes)
        },
    ];
    let current = Version::parse("0.3.0").unwrap();
    assert!(
        select_newer(&feed, &current).is_none(),
        "older, equal, prerelease, build, digestless, delta and foreign entries are never offered"
    );
    let mut newer = feed.clone();
    newer.push(asset("0.3.2", &bytes));
    newer.push(asset("0.3.10", &bytes));
    assert_eq!(select_newer(&newer, &current).unwrap().Version, "0.3.10");
}

#[test]
fn url_policy_is_https_exact_hosts_and_the_repository_path() {
    assert_eq!(
        ALLOWED_HOSTS,
        [
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com"
        ]
    );
    for good in [
        "https://github.com/wcgomes/made/releases/latest/download/releases.linux-x64.json",
        "https://github.com/WCGomes/Made/releases/download/v0.2.0/Made-0.2.0-linux-x64-full.nupkg",
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/abc?sp=r",
        "https://objects.githubusercontent.com/github-production-release-asset-2e65be/1/abc",
    ] {
        assert!(check_url(good).is_ok(), "{good}");
    }
    for bad in [
        "http://github.com/wcgomes/made/releases/latest/download/releases.linux-x64.json",
        "http://release-assets.githubusercontent.com/x",
        "https://evil.example/wcgomes/made/releases/x",
        "https://github.com.evil.example/wcgomes/made/x",
        "https://api.github.com/repos/wcgomes/made/releases",
        "https://github.com/someone/made/releases/download/v1/x",
        "https://github.com/wcgomes/made-fork/releases/download/v1/x",
        "https://github.com/wcgomes/made",
        "https://github.com:8443/wcgomes/made/releases/x",
        "https://user:pass@github.com/wcgomes/made/releases/x",
        "file:///etc/passwd",
    ] {
        assert!(
            matches!(check_url(bad), Err(UpdateError::Refused(_))),
            "{bad}"
        );
    }
}

#[test]
fn tampered_truncated_and_wrongly_versioned_packages_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let good = nupkg("Made", "0.3.0");
    let path = dir.path().join("Made-0.3.0-full.nupkg");
    std::fs::write(&path, &good).unwrap();
    let expected = record("0.3.0", &path, &good);
    verify_package(&path, &expected).unwrap();

    let mut tampered = good.clone();
    let last = tampered.len() - 30;
    tampered[last] ^= 0xff;
    std::fs::write(&path, &tampered).unwrap();
    assert!(matches!(
        verify_package(&path, &expected),
        Err(UpdateError::Verification(_))
    ));

    std::fs::write(&path, &good[..good.len() / 2]).unwrap();
    assert!(matches!(
        verify_package(&path, &expected),
        Err(UpdateError::Verification(_))
    ));

    let wrong = nupkg("Made", "0.2.9");
    std::fs::write(&path, &wrong).unwrap();
    let digest_matches = record("0.3.0", &path, &wrong);
    match verify_package(&path, &digest_matches) {
        Err(UpdateError::Verification(detail)) => {
            assert!(detail.contains("embedded version"), "{detail}")
        }
        other => panic!("wrong embedded version accepted: {other:?}"),
    }

    let foreign = nupkg("Other", "0.3.0");
    std::fs::write(&path, &foreign).unwrap();
    assert!(verify_package(&path, &record("0.3.0", &path, &foreign)).is_err());

    std::fs::remove_file(&path).unwrap();
    assert!(
        verify_package(&path, &expected).is_err(),
        "unreadable package"
    );
}

#[test]
fn the_update_repository_is_a_fixed_constant_with_no_override() {
    assert_eq!(UPDATE_REPOSITORY, "wcgomes/made");
    std::env::set_var("MADE_UPDATE_REPOSITORY", "evil/repo");
    assert!(ReleaseSource::feed_url()
        .starts_with("https://github.com/wcgomes/made/releases/latest/download/"));
    std::env::remove_var("MADE_UPDATE_REPOSITORY");
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/update.rs"))
            .unwrap();
    for forbidden in ["env::var", "var_os", "option_env!", "env!("] {
        assert!(
            !source.contains(forbidden),
            "update.rs must not read `{forbidden}`"
        );
    }
}

#[test]
fn the_test_seam_is_a_non_default_feature() {
    assert_eq!(TEST_SEAM_ENABLED, cfg!(feature = "update-test-seam"));
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let default = manifest
        .lines()
        .find(|line| line.starts_with("default = "))
        .unwrap();
    let desktop = manifest
        .lines()
        .find(|line| line.starts_with("desktop = "))
        .unwrap();
    assert!(!default.contains("update-test-seam") && !desktop.contains("update-test-seam"));
}

/// Child half of `startup_hook_never_applies_on_its_own`.
#[cfg(feature = "update-test-seam")]
#[test]
fn startup_hook_child() {
    let Ok(root) = std::env::var("MADE_HOOK_CHILD_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    made_platform::update::seam::startup_hook_with(velopack::locator::VelopackLocatorConfig {
        RootAppDir: root.to_path_buf(),
        UpdateExePath: root.join(UPDATER),
        PackagesDir: root.join("packages"),
        ManifestPath: root.join("current/sq.version"),
        CurrentBinaryDir: root.join("current"),
        IsPortable: true,
    });
    println!("MADE-HOOK-RETURNED");
}

#[cfg(feature = "update-test-seam")]
const UPDATER: &str = if cfg!(windows) {
    "Update.exe"
} else {
    "Update"
};

/// A pending newer package and a working updater: Velopack's default would
/// apply it at startup. Windows cannot run the shell-script updater, so the
/// assertion that nothing was started is meaningful on Unix only.
#[cfg(all(unix, feature = "update-test-seam"))]
#[test]
fn startup_hook_never_applies_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("current")).unwrap();
    std::fs::create_dir_all(root.join("packages")).unwrap();
    std::fs::write(root.join("current/sq.version"), nuspec("Made", "0.1.0")).unwrap();
    std::fs::write(
        root.join("packages/Made-0.2.0-full.nupkg"),
        nupkg("Made", "0.2.0"),
    )
    .unwrap();
    let marker = root.join("applied");
    let updater = root.join(UPDATER);
    std::fs::write(
        &updater,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&updater, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "startup_hook_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("MADE_HOOK_CHILD_ROOT", root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("MADE-HOOK-RETURNED"),
        "the startup hook exited instead of returning: {stdout}"
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!marker.exists(), "the startup hook started the updater");
    assert!(
        root.join("packages/Made-0.2.0-full.nupkg").exists(),
        "the pending package is kept"
    );
}

#[cfg(feature = "update-test-seam")]
mod seam {
    use super::*;
    use std::collections::HashMap;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use made_platform::update::{seam, HttpRequest, HttpResponse, Transport};

    const FEED: &str =
        "https://github.com/wcgomes/made/releases/latest/download/releases.CHANNEL.json";

    fn feed_url() -> String {
        FEED.replace("CHANNEL", UPDATE_CHANNEL)
    }

    #[derive(Clone)]
    enum Reply {
        Ok(Vec<u8>),
        Redirect(String),
        Status(u16),
    }

    #[derive(Default)]
    struct FakeTransport {
        routes: Mutex<HashMap<String, Reply>>,
        seen: Mutex<Vec<HttpRequest>>,
    }

    impl FakeTransport {
        fn route(&self, url: &str, reply: Reply) {
            self.routes.lock().unwrap().insert(url.to_owned(), reply);
        }
        fn seen_urls(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }
    }

    impl Transport for FakeTransport {
        fn get(&self, request: &HttpRequest, _timeout: Duration) -> Result<HttpResponse, String> {
            self.seen.lock().unwrap().push(request.clone());
            let reply = self.routes.lock().unwrap().get(&request.url).cloned();
            let (status, location, body) = match reply {
                Some(Reply::Ok(body)) => (200, None, body),
                Some(Reply::Redirect(to)) => (302, Some(to), Vec::new()),
                Some(Reply::Status(status)) => (status, None, Vec::new()),
                None => return Err("offline".into()),
            };
            Ok(HttpResponse {
                status,
                location,
                body: Box::new(std::io::Cursor::new(body)),
            })
        }
    }

    fn feed_json(assets: &[VelopackAsset]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "Assets": assets })).unwrap()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        transport: Arc<FakeTransport>,
        packages: std::path::PathBuf,
        data: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let packages = dir.path().join("packages");
            let data = dir.path().join("data");
            Self {
                _dir: dir,
                transport: Arc::new(FakeTransport::default()),
                packages,
                data,
            }
        }
        fn updater(&self, current: &str) -> Updater {
            let transport: Arc<dyn Transport> = self.transport.clone();
            seam::updater(
                seam::installation(self.packages.clone(), self.data.clone(), current),
                seam::source(transport, current),
            )
        }
        fn publish(&self, version: &str, package: &[u8], listed: &VelopackAsset) {
            let tag = format!(
                "https://github.com/wcgomes/made/releases/download/v{version}/{}",
                listed.FileName
            );
            let cdn = format!("https://release-assets.githubusercontent.com/github-production-release-asset/1/{version}");
            self.transport.route(&tag, Reply::Redirect(cdn.clone()));
            self.transport.route(&cdn, Reply::Ok(package.to_vec()));
        }
        fn feed(&self, assets: &[VelopackAsset]) {
            let tag = format!("https://github.com/wcgomes/made/releases/download/v9/releases.{UPDATE_CHANNEL}.json");
            let cdn = "https://release-assets.githubusercontent.com/github-production-release-asset/1/feed";
            self.transport
                .route(&feed_url(), Reply::Redirect(tag.clone()));
            self.transport.route(&tag, Reply::Redirect(cdn.into()));
            self.transport.route(cdn, Reply::Ok(feed_json(assets)));
        }
    }

    #[test]
    fn non_installed_detection_makes_no_request() {
        let transport = Arc::new(FakeTransport::default());
        let dynamic: Arc<dyn Transport> = transport.clone();
        assert!(seam::detect_with(dynamic).is_none());
        assert!(transport.seen_urls().is_empty());
    }

    #[test]
    fn check_download_and_kept_update_round_trip() {
        let fixture = Fixture::new();
        let package = nupkg("Made", "0.3.0");
        let listed = asset("0.3.0", &package);
        fixture.feed(&[
            asset("0.2.0", &package),
            listed.clone(),
            asset("9.0.0-beta.1", &package),
        ]);
        fixture.publish("0.3.0", &package, &listed);

        let updater = fixture.updater("0.2.0");
        assert!(updater.kept().is_none());
        let candidate = updater.check().unwrap().expect("0.3.0 offered");
        assert_eq!(candidate.version(), "0.3.0");
        let ready = updater.download(&candidate).unwrap();
        assert_eq!(ready.version(), "0.3.0");

        // A relaunch finds the kept update with no request at all.
        let before = fixture.transport.seen_urls().len();
        let relaunched = fixture.updater("0.2.0");
        assert_eq!(relaunched.kept(), Some(ready.clone()));
        assert_eq!(fixture.transport.seen_urls().len(), before);
        // The installed version catching up retires the record.
        assert!(fixture.updater("0.3.0").kept().is_none());
    }

    #[test]
    fn up_to_date_and_older_or_prerelease_entries_offer_nothing() {
        let fixture = Fixture::new();
        let package = nupkg("Made", "0.3.0");
        fixture.feed(&[
            asset("0.3.0", &package),
            asset("0.2.1", &package),
            asset("1.0.0-rc.1", &package),
        ]);
        assert!(fixture.updater("0.3.0").check().unwrap().is_none());
    }

    #[test]
    fn tampered_download_is_discarded_and_an_earlier_ready_update_survives() {
        let fixture = Fixture::new();
        let first = nupkg("Made", "0.3.0");
        let first_listed = asset("0.3.0", &first);
        fixture.feed(std::slice::from_ref(&first_listed));
        fixture.publish("0.3.0", &first, &first_listed);
        let updater = fixture.updater("0.2.0");
        let ready = updater
            .download(&updater.check().unwrap().unwrap())
            .unwrap();

        let second = nupkg("Made", "0.4.0");
        let second_listed = asset("0.4.0", &second);
        let mut tampered = second.clone();
        let index = tampered.len() - 40;
        tampered[index] ^= 0x55;
        fixture.feed(&[first_listed, second_listed.clone()]);
        fixture.publish("0.4.0", &tampered, &second_listed);
        let candidate = updater.check().unwrap().unwrap();
        assert_eq!(candidate.version(), "0.4.0");
        assert!(matches!(
            updater.download(&candidate),
            Err(UpdateError::Verification(_))
        ));
        assert_eq!(
            updater.kept(),
            Some(ready),
            "the older ready version stays offered"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&fixture.packages)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("0.4.0"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "discarded package left behind: {leftovers:?}"
        );
    }

    #[test]
    fn truncated_oversized_and_wrong_version_downloads_are_rejected() {
        let package = nupkg("Made", "0.3.0");
        let listed = asset("0.3.0", &package);
        for (body, kind) in [
            (package[..package.len() - 10].to_vec(), "download"),
            ([package.clone(), vec![0u8; 16]].concat(), "download"),
        ] {
            let fixture = Fixture::new();
            fixture.feed(std::slice::from_ref(&listed));
            fixture.publish("0.3.0", &body, &listed);
            let updater = fixture.updater("0.2.0");
            let result = updater.download(&updater.check().unwrap().unwrap());
            assert!(
                matches!(result, Err(UpdateError::Download(_))),
                "{kind}: {result:?}"
            );
            assert!(updater.kept().is_none());
        }
        let fixture = Fixture::new();
        let wrong = nupkg("Made", "0.2.5");
        let listed = asset("0.3.0", &wrong);
        fixture.feed(std::slice::from_ref(&listed));
        fixture.publish("0.3.0", &wrong, &listed);
        let updater = fixture.updater("0.2.0");
        assert!(matches!(
            updater.download(&updater.check().unwrap().unwrap()),
            Err(UpdateError::Verification(_))
        ));
        assert!(updater.kept().is_none());
    }

    #[test]
    fn redirects_outside_the_allowlist_are_never_fetched() {
        for target in [
            "https://evil.example/releases.json",
            "http://release-assets.githubusercontent.com/feed",
            "https://github.com/attacker/made/releases/download/v1/releases.json",
            "https://api.github.com/repos/wcgomes/made/releases",
        ] {
            let fixture = Fixture::new();
            fixture
                .transport
                .route(&feed_url(), Reply::Redirect(target.into()));
            fixture.transport.route(target, Reply::Ok(feed_json(&[])));
            let result = fixture.updater("0.1.0").check();
            assert!(
                matches!(result, Err(UpdateError::Refused(_))),
                "{target}: {:?}",
                result.map(|c| c.is_some())
            );
            assert_eq!(
                fixture.transport.seen_urls(),
                vec![feed_url()],
                "{target} was fetched"
            );
        }
        let fixture = Fixture::new();
        let package = nupkg("Made", "0.3.0");
        let listed = asset("0.3.0", &package);
        fixture.feed(std::slice::from_ref(&listed));
        let tag = format!(
            "https://github.com/wcgomes/made/releases/download/v0.3.0/{}",
            listed.FileName
        );
        fixture
            .transport
            .route(&tag, Reply::Redirect("https://evil.example/pkg".into()));
        fixture
            .transport
            .route("https://evil.example/pkg", Reply::Ok(package));
        let updater = fixture.updater("0.2.0");
        assert!(matches!(
            updater.download(&updater.check().unwrap().unwrap()),
            Err(UpdateError::Refused(_))
        ));
        assert!(!fixture
            .transport
            .seen_urls()
            .iter()
            .any(|u| u.contains("evil")));
    }

    #[test]
    fn unreachable_rate_limited_and_malformed_feeds_offer_nothing() {
        let fixture = Fixture::new();
        assert!(matches!(
            fixture.updater("0.1.0").check(),
            Err(UpdateError::Unreachable(_))
        ));
        fixture.transport.route(&feed_url(), Reply::Status(403));
        assert!(matches!(
            fixture.updater("0.1.0").check(),
            Err(UpdateError::Unreachable(_))
        ));
        fixture
            .transport
            .route(&feed_url(), Reply::Ok(b"{not json".to_vec()));
        assert!(matches!(
            fixture.updater("0.1.0").check(),
            Err(UpdateError::MalformedFeed(_))
        ));
    }

    /// Sends the real client's request to a loopback listener (by rewriting
    /// only the scheme and authority) and records the bytes on the wire.
    struct LoopbackCapture {
        port: u16,
        inner: Arc<dyn Transport>,
    }

    impl Transport for LoopbackCapture {
        fn get(&self, request: &HttpRequest, timeout: Duration) -> Result<HttpResponse, String> {
            let url = url::Url::parse(&request.url).unwrap();
            let rewritten = format!(
                "http://127.0.0.1:{}{}",
                self.port,
                &request.url[url.origin().ascii_serialization().len()..]
            );
            self.inner.get(
                &HttpRequest {
                    url: rewritten,
                    headers: request.headers.clone(),
                },
                timeout,
            )
        }
    }

    #[test]
    fn a_check_request_carries_only_repository_platform_architecture_and_version() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                head.push(byte[0]);
            }
            let body = br#"{"Assets":[]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            String::from_utf8(head).unwrap()
        });
        let transport: Arc<dyn Transport> = Arc::new(LoopbackCapture {
            port,
            inner: seam::loopback_transport(),
        });
        let installed = "0.2.0";
        let dir = tempfile::tempdir().unwrap();
        let updater = seam::updater(
            seam::installation(dir.path().join("p"), dir.path().join("d"), installed),
            seam::source(transport, installed),
        );
        assert!(updater.check().unwrap().is_none());
        let head = server.join().unwrap();
        let mut lines = head.lines();
        assert_eq!(
            lines.next().unwrap(),
            format!("GET /wcgomes/made/releases/latest/download/releases.{UPDATE_CHANNEL}.json HTTP/1.1")
        );
        let headers: Vec<(String, String)> = lines
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.trim().to_ascii_lowercase(), value.trim().to_owned())
            })
            .collect();
        let names: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
        for name in &names {
            assert!(
                ["host", "user-agent", "accept", "accept-encoding"].contains(name),
                "unexpected header {name}: {head}"
            );
        }
        let agent = &headers
            .iter()
            .find(|(name, _)| name == "user-agent")
            .unwrap()
            .1;
        assert_eq!(
            agent,
            &format!(
                "Made/{installed} ({}; {})",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        );
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        for private in [user.as_str(), home.as_str(), dir.path().to_str().unwrap()] {
            if private.len() > 2 {
                assert!(!head.contains(private), "check request leaks {private}");
            }
        }
        assert!(!head.contains('?'), "no query string: {head}");
    }

    /// Child half of the CLI apply test: a running GUI instance.
    #[test]
    fn instance_holder_child() {
        let Ok(dirs) = std::env::var("MADE_INSTANCE_CHILD") else {
            return;
        };
        let (packages, data) = dirs.split_once('|').unwrap();
        let installation = seam::installation(packages.into(), data.into(), "0.2.0");
        let _guard = made_platform::update::register_instance(&installation).unwrap();
        std::fs::write(Path::new(packages).join("child-ready"), b"").unwrap();
        let stop = Path::new(packages).join("child-stop");
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !stop.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn cli_apply_refuses_while_made_runs_holds_the_install_lock_and_never_restarts() {
        use made_platform::update::{register_instance, CliApplyOutcome};
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new();
        let package = nupkg("Made", "0.3.0");
        let listed = asset("0.3.0", &package);
        fixture.feed(std::slice::from_ref(&listed));
        fixture.publish("0.3.0", &package, &listed);
        let updater = fixture.updater("0.2.0");
        let ready = updater
            .download(&updater.check().unwrap().unwrap())
            .unwrap();
        let args = fixture.packages.join("updater-args");
        let script = fixture.packages.join("Update");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho \"$@\" > '{}'\nsleep 2\n", args.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        // A running GUI instance of this install refuses the apply.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["seam::instance_holder_child", "--exact", "--nocapture"])
            .env(
                "MADE_INSTANCE_CHILD",
                format!("{}|{}", fixture.packages.display(), fixture.data.display()),
            )
            .spawn()
            .unwrap();
        let ready_flag = fixture.packages.join("child-ready");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !ready_flag.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the instance child never registered"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        match updater.apply_from_cli(&ready) {
            Err(UpdateError::InstanceRunning(pids)) => assert_eq!(pids, vec![child.id()]),
            other => panic!("apply was not refused: {other:?}"),
        }
        assert!(!args.exists(), "a refused apply changes no installed files");
        std::fs::write(fixture.packages.join("child-stop"), b"").unwrap();
        assert!(child.wait().unwrap().success());

        // While the apply runs it holds the install lock; GUI startup refuses.
        let installation =
            seam::installation(fixture.packages.clone(), fixture.data.clone(), "0.2.0");
        let applying = std::thread::spawn(move || updater.apply_from_cli(&ready));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !args.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the updater never started"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let refused = register_instance(&installation);
        assert!(
            refused.is_err(),
            "GUI startup must refuse during a command-line apply"
        );
        assert!(refused.err().unwrap().to_string().contains("being applied"));
        assert_eq!(
            applying.join().unwrap().unwrap(),
            CliApplyOutcome::Applied("0.3.0".into())
        );
        let recorded = std::fs::read_to_string(&args).unwrap();
        for expected in ["apply", "--package", "--silent", "--norestart"] {
            assert!(recorded.contains(expected), "{recorded}");
        }
        assert!(
            !recorded.contains("--waitPid"),
            "the CLI apply waits for no GUI and starts none"
        );
        assert!(
            register_instance(&installation).is_ok(),
            "startup works again after the apply"
        );
    }

    #[test]
    fn the_production_transport_refuses_plain_http() {
        let transport = seam::production_transport();
        let request = HttpRequest {
            url: "http://127.0.0.1:9/".into(),
            headers: Vec::new(),
        };
        assert!(transport.get(&request, Duration::from_secs(2)).is_err());
    }
}
