//! In-app update capability for installed builds (REL-06 to REL-12).
//!
//! Made is installed and updated by Velopack. This module owns everything
//! that touches the network or the installation for updates; scheduling,
//! preference and UI state live in `made-app`.
//!
//! - **Installed detection.** [`Installation::detect`] uses Velopack's locator
//!   from the running executable. Any other build (`cargo run`, development
//!   artifacts, raw staging trees) is non-installed: [`Updater::detect`]
//!   returns `None` and no update request is ever made.
//! - **Startup hook.** [`run_startup_hook`] runs Velopack's install/update
//!   hooks with auto-apply on startup disabled, so a downloaded update is never
//!   applied without the user's confirmation (REL-07).
//! - **Source.** The update repository is the compile-time constant
//!   [`UPDATE_REPOSITORY`]; nothing overrides it. Probe 1.2 showed Velopack's
//!   built-in sources follow redirects to any host and send an install
//!   identifier (`stagingId`), so Made uses its own source: the stable feed at
//!   `https://github.com/<repo>/releases/latest/download/releases.<channel>.json`
//!   (GitHub never resolves `latest` to a prerelease or draft) and the
//!   package at `https://github.com/<repo>/releases/download/v<version>/<file>`.
//! - **Integrity (REL-08).** HTTPS only with certificate validation; redirects
//!   are followed by hand and every hop must be an exact allowlisted host, with
//!   `github.com` URLs pinned to the repository path. A package is ready only
//!   after its size, SHA-256 and embedded version match the feed entry; a
//!   kept package is re-verified against the private ready record before it
//!   is offered or applied. This proves the package is the one listed by that
//!   GitHub Release, not who published it.
//! - **Apply.** [`Updater::restart_into`] hands the verified package to the
//!   Velopack updater, which waits for Made to exit and relaunches it.
//!   [`Updater::apply_from_cli`] applies without restarting, under an install
//!   lock, and refuses while any Made instance of the install is running.
//!
//! The injectable transport used by the tamper, redirect and allowlist tests
//! exists only behind the non-default `update-test-seam` feature.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use sha2::{Digest, Sha256};
use url::Url;
use velopack::locator::{auto_locate_app_manifest, LocationContext, VelopackLocatorConfig};
use velopack::{VelopackApp, VelopackAsset, VelopackAssetFeed};

/// The production repository whose GitHub Releases feed updates. Fixed at
/// compile time; there is no environment, file or build-time override.
pub const UPDATE_REPOSITORY: &str = "wcgomes/made";

/// Velopack channel of this target. Each target has its own feed file.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub const UPDATE_CHANNEL: &str = "win-x64";
/// Velopack channel of this target. Each target has its own feed file.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const UPDATE_CHANNEL: &str = "osx-arm64";
/// Velopack channel of this target. Each target has its own feed file.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const UPDATE_CHANNEL: &str = "osx-x64";
/// Velopack channel of this target. Each target has its own feed file.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const UPDATE_CHANNEL: &str = "linux-x64";
/// Velopack channel of this target. Unsupported targets have no feed.
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
pub const UPDATE_CHANNEL: &str = "unsupported";

/// Whether this build contains the injectable test transport. Release builds
/// report `false`; the installed smoke check asserts it.
pub const TEST_SEAM_ENABLED: bool = cfg!(feature = "update-test-seam");

/// Exact hosts every update request and redirect hop may reach (probe 1.8:
/// `github.com` redirects release downloads to
/// `release-assets.githubusercontent.com`; `objects.githubusercontent.com`
/// is GitHub's older asset host). The API host is never used.
pub const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Velopack package id of every Made release.
pub const PACKAGE_ID: &str = "Made";

const MAX_REDIRECTS: usize = 5;
const MAX_FEED_BYTES: u64 = 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const READY_RECORD: &str = "update-ready.json";
const INSTALL_LOCK: &str = "made-install.lock";
const DOWNLOAD_LOCK: &str = "made-download.lock";
const INSTANCE_PREFIX: &str = "made-instance-";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Why an update operation did not complete. Every variant leaves the running
/// version untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// Offline, unreachable, rate-limited or an HTTP error status.
    Unreachable(String),
    /// A request or redirect hop outside the allowlist, or plain HTTP.
    Refused(String),
    /// The feed was not valid update data.
    MalformedFeed(String),
    /// The download stopped or failed before the package was complete.
    Download(String),
    /// Size, SHA-256 or embedded version did not match; the package was
    /// discarded.
    Verification(String),
    /// A Made instance of this install is running (process ids).
    InstanceRunning(Vec<u32>),
    /// Another update operation holds the install lock.
    Busy,
    /// Local file or process failure.
    Local(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(detail) => {
                write!(f, "the update service could not be reached: {detail}")
            }
            Self::Refused(detail) => write!(f, "refused an update location: {detail}"),
            Self::MalformedFeed(detail) => write!(f, "the update data is malformed: {detail}"),
            Self::Download(detail) => write!(f, "the download failed: {detail}"),
            Self::Verification(detail) => write!(
                f,
                "the package failed verification and was discarded: {detail}"
            ),
            Self::InstanceRunning(pids) => {
                let list: Vec<String> = pids.iter().map(u32::to_string).collect();
                write!(f, "Made is running from this install (process {}); quit it before applying the update", list.join(", "))
            }
            Self::Busy => f.write_str("another update operation is in progress for this install"),
            Self::Local(detail) => write!(f, "update failed: {detail}"),
        }
    }
}

impl std::error::Error for UpdateError {}

// ---------------------------------------------------------------------------
// URL policy
// ---------------------------------------------------------------------------

/// Accept a URL only if it is HTTPS on the default port, has no credentials,
/// targets an allowlisted host, and, on `github.com`, stays under
/// `/<owner>/<repo>/` of [`UPDATE_REPOSITORY`].
pub fn check_url(raw: &str) -> Result<Url, UpdateError> {
    let url = Url::parse(raw).map_err(|_| UpdateError::Refused("unparseable URL".into()))?;
    if url.scheme() != "https" {
        return Err(UpdateError::Refused(format!(
            "{} is not HTTPS",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UpdateError::Refused("credentials in URL".into()));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(UpdateError::Refused("non-default port".into()));
    }
    let host = url.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(UpdateError::Refused(format!("host {host} is not allowed")));
    }
    if host == "github.com" {
        let mut segments = url.path_segments().into_iter().flatten();
        let owner = segments.next().unwrap_or_default();
        let repo = segments.next().unwrap_or_default();
        let (want_owner, want_repo) = UPDATE_REPOSITORY.split_once('/').unwrap_or_default();
        let next = segments.next();
        if !owner.eq_ignore_ascii_case(want_owner)
            || !repo.eq_ignore_ascii_case(want_repo)
            || next.is_none()
        {
            return Err(UpdateError::Refused(
                "path outside the project repository".into(),
            ));
        }
    }
    Ok(url)
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// One GET request: the URL and every header Made sets on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// Absolute URL.
    pub url: String,
    /// Headers set by Made. The HTTP client adds only `Host` and
    /// `Accept-Encoding` (probe 1.2).
    pub headers: Vec<(String, String)>,
}

/// One HTTP response, redirects not followed.
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// `Location` header of a redirect.
    pub location: Option<String>,
    /// Response body.
    pub body: Box<dyn Read + Send>,
}

/// The network edge of the update source. Production uses one HTTPS-only
/// client that never follows redirects on its own.
pub trait Transport: Send + Sync {
    /// Perform one request; the caller follows redirects.
    fn get(&self, request: &HttpRequest, timeout: Duration) -> Result<HttpResponse, String>;
}

struct UreqTransport {
    https_only: bool,
}

impl Transport for UreqTransport {
    fn get(&self, request: &HttpRequest, timeout: Duration) -> Result<HttpResponse, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(self.https_only)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .into();
        let mut call = agent.get(&request.url);
        for (name, value) in &request.headers {
            call = call.header(name, value);
        }
        let response = call.call().map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.into_body().into_reader();
        Ok(HttpResponse {
            status,
            location,
            body: Box::new(body),
        })
    }
}

/// Test-only constructors. Compiled only with the non-default
/// `update-test-seam` feature; release builds never enable it.
#[cfg(feature = "update-test-seam")]
pub mod seam {
    use super::*;

    /// The production HTTP client with HTTPS enforcement lifted so a loopback
    /// listener can capture the exact request bytes a check sends.
    pub fn loopback_transport() -> Arc<dyn Transport> {
        Arc::new(UreqTransport { https_only: false })
    }

    /// The production HTTP client, HTTPS only.
    pub fn production_transport() -> Arc<dyn Transport> {
        Arc::new(UreqTransport { https_only: true })
    }

    /// A source using the given transport.
    pub fn source(transport: Arc<dyn Transport>, current_version: &str) -> ReleaseSource {
        ReleaseSource::new(transport, current_version)
    }

    /// An installation rooted in test directories.
    pub fn installation(packages_dir: PathBuf, data_dir: PathBuf, current: &str) -> Installation {
        Installation {
            root: packages_dir.clone(),
            update_exe: packages_dir.join("Update"),
            packages_dir,
            current_bin_dir: PathBuf::new(),
            manifest_path: PathBuf::new(),
            data_dir,
            current: Version::parse(current).expect("test version"),
            portable: true,
        }
    }

    /// An updater from test parts.
    pub fn updater(installation: Installation, source: ReleaseSource) -> Updater {
        Updater {
            installation,
            source,
        }
    }

    /// The updater from the running executable with an injected transport.
    pub fn detect_with(transport: Arc<dyn Transport>) -> Option<Updater> {
        let installation = Installation::detect()?;
        let source = ReleaseSource::new(transport, &installation.current.to_string());
        Some(Updater {
            installation,
            source,
        })
    }

    /// Run the startup hook against an explicit locator.
    pub fn startup_hook_with(config: VelopackLocatorConfig) {
        startup_app(Some(config)).run();
    }
}

// ---------------------------------------------------------------------------
// Release source
// ---------------------------------------------------------------------------

/// Made's GitHub Releases update source.
#[derive(Clone)]
pub struct ReleaseSource {
    transport: Arc<dyn Transport>,
    user_agent: String,
}

impl ReleaseSource {
    fn new(transport: Arc<dyn Transport>, current_version: &str) -> Self {
        Self {
            transport,
            user_agent: user_agent(current_version),
        }
    }

    fn production(current_version: &str) -> Self {
        Self::new(
            Arc::new(UreqTransport { https_only: true }),
            current_version,
        )
    }

    /// URL of this target's stable feed.
    pub fn feed_url() -> String {
        format!("https://github.com/{UPDATE_REPOSITORY}/releases/latest/download/releases.{UPDATE_CHANNEL}.json")
    }

    /// URL of a feed entry's package in its own release.
    pub fn package_url(asset: &VelopackAsset) -> String {
        format!(
            "https://github.com/{UPDATE_REPOSITORY}/releases/download/v{}/{}",
            asset.Version, asset.FileName
        )
    }

    /// The request a check sends for the feed.
    pub fn feed_request(&self) -> HttpRequest {
        self.request(Self::feed_url())
    }

    fn request(&self, url: String) -> HttpRequest {
        HttpRequest {
            url,
            headers: vec![
                ("User-Agent".into(), self.user_agent.clone()),
                ("Accept".into(), "application/octet-stream".into()),
            ],
        }
    }

    /// Follow redirects by hand, checking every hop against the allowlist.
    fn fetch(&self, url: String, timeout: Duration) -> Result<HttpResponse, UpdateError> {
        let mut current = check_url(&url)?;
        for _ in 0..=MAX_REDIRECTS {
            let response = self
                .transport
                .get(&self.request(current.to_string()), timeout)
                .map_err(UpdateError::Unreachable)?;
            match response.status {
                200 => return Ok(response),
                301 | 302 | 303 | 307 | 308 => {
                    let location = response.location.ok_or_else(|| {
                        UpdateError::Unreachable("redirect without a location".into())
                    })?;
                    let next = current
                        .join(&location)
                        .map_err(|_| UpdateError::Refused("unparseable redirect".into()))?;
                    current = check_url(next.as_str())?;
                }
                status => return Err(UpdateError::Unreachable(format!("HTTP status {status}"))),
            }
        }
        Err(UpdateError::Unreachable("too many redirects".into()))
    }

    /// Fetch and validate this target's stable feed. Entries that are not
    /// full stable packages with a SHA-256 and a safe file name are dropped.
    pub fn fetch_feed(&self) -> Result<Vec<VelopackAsset>, UpdateError> {
        let response = self.fetch(Self::feed_url(), REQUEST_TIMEOUT)?;
        let mut text = String::new();
        response
            .body
            .take(MAX_FEED_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| UpdateError::Unreachable(error.to_string()))?;
        if text.len() as u64 > MAX_FEED_BYTES {
            return Err(UpdateError::MalformedFeed("feed is too large".into()));
        }
        let feed: VelopackAssetFeed = serde_json::from_str(&text)
            .map_err(|error| UpdateError::MalformedFeed(error.to_string()))?;
        Ok(feed.Assets.into_iter().filter(acceptable_asset).collect())
    }

    /// Download one feed entry to `destination`, refusing more bytes than the
    /// feed lists and failing when fewer arrive.
    pub fn download(&self, asset: &VelopackAsset, destination: &Path) -> Result<(), UpdateError> {
        let response = self.fetch(Self::package_url(asset), DOWNLOAD_TIMEOUT)?;
        let mut file =
            File::create(destination).map_err(|error| UpdateError::Local(error.to_string()))?;
        let mut limited = response.body.take(asset.Size + 1);
        let copied = io::copy(&mut limited, &mut file)
            .map_err(|error| UpdateError::Download(error.to_string()))?;
        file.sync_all()
            .map_err(|error| UpdateError::Local(error.to_string()))?;
        if copied != asset.Size {
            return Err(UpdateError::Download(format!(
                "received {copied} bytes, expected {}",
                asset.Size
            )));
        }
        Ok(())
    }
}

/// `Made/<version> (<os>; <arch>)`: the only client detail an update request
/// carries besides what HTTPS inherently reveals (REL-06).
fn user_agent(current_version: &str) -> String {
    format!(
        "Made/{current_version} ({}; {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn acceptable_asset(asset: &VelopackAsset) -> bool {
    let stable = Version::parse(&asset.Version)
        .is_ok_and(|version| version.pre.is_empty() && version.build.is_empty());
    let digest = asset.SHA256.len() == 64 && asset.SHA256.bytes().all(|b| b.is_ascii_hexdigit());
    let name = !asset.FileName.is_empty()
        && asset.FileName.ends_with("-full.nupkg")
        && asset
            .FileName
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    asset.Type.eq_ignore_ascii_case("Full")
        && asset.PackageId == PACKAGE_ID
        && stable
        && digest
        && name
        && asset.Size > 0
        && asset.Size <= MAX_PACKAGE_BYTES
}

/// The newest entry strictly newer than `current`, if any. Older, equal and
/// prerelease versions are never offered.
pub fn select_newer(assets: &[VelopackAsset], current: &Version) -> Option<VelopackAsset> {
    assets
        .iter()
        .filter(|asset| acceptable_asset(asset))
        .filter_map(|asset| {
            Version::parse(&asset.Version)
                .ok()
                .map(|version| (version, asset))
        })
        .filter(|(version, _)| version > current)
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, asset)| asset.clone())
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// What a verified package must be. Persisted privately once a download is
/// verified, so a kept package is re-verified before it is offered again.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReadyRecord {
    /// Offered version.
    pub version: String,
    /// Package file name in the packages directory.
    pub file: String,
    /// Exact size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

impl ReadyRecord {
    fn from_asset(asset: &VelopackAsset) -> Self {
        Self {
            version: asset.Version.clone(),
            file: asset.FileName.clone(),
            size: asset.Size,
            sha256: asset.SHA256.to_ascii_lowercase(),
        }
    }
}

/// Verify size, SHA-256 and the version embedded in the package manifest.
pub fn verify_package(path: &Path, expected: &ReadyRecord) -> Result<(), UpdateError> {
    let fail = |detail: String| UpdateError::Verification(detail);
    let metadata =
        fs::metadata(path).map_err(|error| fail(format!("unreadable package: {error}")))?;
    if metadata.len() != expected.size {
        return Err(fail(format!(
            "size {} differs from {}",
            metadata.len(),
            expected.size
        )));
    }
    let mut file =
        File::open(path).map_err(|error| fail(format!("unreadable package: {error}")))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .map_err(|error| fail(format!("unreadable package: {error}")))?;
    let digest = format!("{:x}", hasher.finalize());
    if !digest.eq_ignore_ascii_case(&expected.sha256) {
        return Err(fail(
            "SHA-256 differs from the release's update data".into(),
        ));
    }
    let manifest = velopack::bundle::load_bundle_from_file(path)
        .and_then(|mut bundle| bundle.read_manifest())
        .map_err(|error| fail(format!("unreadable package manifest: {error}")))?;
    let expected_version = Version::parse(&expected.version)
        .map_err(|_| fail("offered version is not semantic".into()))?;
    if manifest.version != expected_version {
        return Err(fail(format!(
            "embedded version {} differs from offered {}",
            manifest.version, expected.version
        )));
    }
    if manifest.id != PACKAGE_ID {
        return Err(fail(format!(
            "embedded package id {} is not {PACKAGE_ID}",
            manifest.id
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

/// Paths of an installed build, from Velopack's locator.
#[derive(Debug, Clone)]
pub struct Installation {
    root: PathBuf,
    update_exe: PathBuf,
    packages_dir: PathBuf,
    current_bin_dir: PathBuf,
    manifest_path: PathBuf,
    data_dir: PathBuf,
    current: Version,
    portable: bool,
}

impl Installation {
    /// Detect the installation of the running executable. `None` for every
    /// non-installed build.
    pub fn detect() -> Option<Self> {
        let located = auto_locate_app_manifest(LocationContext::FromCurrentExe).ok()?;
        let data_dir = crate::host_platform_data_dir()?;
        let current_bin_dir = located.get_current_bin_dir();
        #[cfg(target_os = "linux")]
        let (root, packages_dir) = (located.get_appimage_path(), data_dir.join("updates"));
        #[cfg(not(target_os = "linux"))]
        let (root, packages_dir) = (located.get_root_dir(), located.get_packages_dir());
        // macOS keeps a sq.version symlink beside the executable.
        let manifest_path = current_bin_dir.join("sq.version");
        Some(Self {
            root,
            update_exe: located.get_update_path(),
            packages_dir,
            current_bin_dir,
            manifest_path,
            data_dir,
            current: located.get_manifest_version(),
            portable: located.get_is_portable(),
        })
    }

    /// Windows helper path: the installation rooted at `root`, located without
    /// the running executable (which is a temporary copy).
    #[cfg(target_os = "windows")]
    pub fn from_windows_root(root: &Path) -> Option<Self> {
        let config = velopack::locator::create_config_from_root_dir(root);
        let located = velopack::locator::VelopackLocator::new(&config).ok()?;
        Some(Self {
            root: located.get_root_dir(),
            update_exe: located.get_update_path(),
            packages_dir: located.get_packages_dir(),
            current_bin_dir: located.get_current_bin_dir(),
            manifest_path: config.ManifestPath,
            data_dir: crate::host_platform_data_dir()?,
            current: located.get_manifest_version(),
            portable: located.get_is_portable(),
        })
    }

    /// Installed version.
    pub fn current_version(&self) -> String {
        self.current.to_string()
    }

    /// Install root (the Windows install directory, the macOS `.app`, or the
    /// Linux AppImage file).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding downloaded packages.
    pub fn packages_dir(&self) -> &Path {
        &self.packages_dir
    }

    fn locator_config(&self) -> VelopackLocatorConfig {
        VelopackLocatorConfig {
            RootAppDir: self.root.clone(),
            UpdateExePath: self.update_exe.clone(),
            PackagesDir: self.packages_dir.clone(),
            ManifestPath: self.manifest_path.clone(),
            CurrentBinaryDir: self.current_bin_dir.clone(),
            IsPortable: self.portable,
        }
    }

    fn ensure_packages_dir(&self) -> Result<(), UpdateError> {
        if cfg!(target_os = "linux") {
            crate::create_private_dir_all(&self.packages_dir)
                .map_err(|error| UpdateError::Local(error.to_string()))
        } else {
            fs::create_dir_all(&self.packages_dir)
                .map_err(|error| UpdateError::Local(error.to_string()))
        }
    }

    fn read_record(&self) -> Option<ReadyRecord> {
        let bytes = crate::read_private_metadata(&self.data_dir, READY_RECORD, 4096).ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    fn write_record(&self, record: &ReadyRecord) -> Result<(), UpdateError> {
        let bytes =
            serde_json::to_vec(record).map_err(|error| UpdateError::Local(error.to_string()))?;
        crate::write_private_metadata_atomic(&self.data_dir, READY_RECORD, &bytes, 4096)
            .map_err(|error| UpdateError::Local(error.to_string()))
    }

    fn clear_record(&self) {
        let _ = fs::remove_file(self.data_dir.join(READY_RECORD));
    }
}

fn startup_app(config: Option<VelopackLocatorConfig>) -> VelopackApp<'static> {
    let app = VelopackApp::build().set_auto_apply_on_startup(false);
    match config {
        Some(config) => app.set_locator(config),
        None => app,
    }
}

/// Run Velopack's startup hooks. Must be the first thing `main` does. Never
/// applies a downloaded update on its own; in a non-installed build it does
/// nothing. Install/uninstall hook invocations exit the process here.
pub fn run_startup_hook() {
    let config = Installation::detect().map(|installation| installation.locator_config());
    startup_app(config).run();
}

// ---------------------------------------------------------------------------
// Updater
// ---------------------------------------------------------------------------

/// A newer stable version offered by the feed, not yet downloaded.
#[derive(Debug, Clone)]
pub struct Candidate {
    asset: VelopackAsset,
}

impl Candidate {
    /// Offered version.
    pub fn version(&self) -> &str {
        &self.asset.Version
    }
}

/// A downloaded package that passed verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    record: ReadyRecord,
}

impl Ready {
    /// Version the package installs.
    pub fn version(&self) -> &str {
        &self.record.version
    }
}

/// What a command-line apply did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliApplyOutcome {
    /// The new version is installed; Made was not started.
    Applied(String),
    /// Windows: the apply continues in a helper process outside the install
    /// directory, which holds the install lock until it finishes.
    HandedOff {
        /// Helper process id.
        pid: u32,
        /// Version being applied.
        version: String,
    },
}

/// The update capability of an installed build.
pub struct Updater {
    installation: Installation,
    source: ReleaseSource,
}

impl Updater {
    /// The updater of the running installed build, or `None` for a
    /// non-installed build (which then makes no update request at all).
    pub fn detect() -> Option<Self> {
        let installation = Installation::detect()?;
        let source = ReleaseSource::production(&installation.current_version());
        Some(Self {
            installation,
            source,
        })
    }

    /// The installation this updater serves.
    pub fn installation(&self) -> &Installation {
        &self.installation
    }

    /// Installed version.
    pub fn current_version(&self) -> String {
        self.installation.current_version()
    }

    /// Check the feed once. `Ok(None)` means up to date.
    pub fn check(&self) -> Result<Option<Candidate>, UpdateError> {
        let assets = self.source.fetch_feed()?;
        Ok(select_newer(&assets, &self.installation.current).map(|asset| Candidate { asset }))
    }

    /// Download and verify a candidate. A verified package becomes the only
    /// kept package; until then any earlier ready package stays untouched.
    pub fn download(&self, candidate: &Candidate) -> Result<Ready, UpdateError> {
        let installation = &self.installation;
        installation.ensure_packages_dir()?;
        let _lock = match FileLock::try_lock(&installation.packages_dir.join(DOWNLOAD_LOCK)) {
            Ok(Some(lock)) => lock,
            Ok(None) => return Err(UpdateError::Busy),
            Err(error) => return Err(UpdateError::Local(error.to_string())),
        };
        let record = ReadyRecord::from_asset(&candidate.asset);
        let final_path = installation.packages_dir.join(&record.file);
        if final_path.exists() && verify_package(&final_path, &record).is_ok() {
            installation.write_record(&record)?;
            return Ok(Ready { record });
        }
        let partial = installation
            .packages_dir
            .join(format!("{}.partial", record.file));
        let result = self
            .source
            .download(&candidate.asset, &partial)
            .and_then(|()| verify_package(&partial, &record));
        if let Err(error) = result {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        fs::rename(&partial, &final_path).map_err(|error| UpdateError::Local(error.to_string()))?;
        installation.write_record(&record)?;
        remove_other_packages(&installation.packages_dir, &record.file);
        #[cfg(target_os = "windows")]
        refresh_windows_updater(&final_path, &installation.update_exe);
        Ok(Ready { record })
    }

    /// A previously downloaded package that still verifies and is newer than
    /// the installed version. Makes no network request.
    pub fn kept(&self) -> Option<Ready> {
        let installation = &self.installation;
        let record = installation.read_record()?;
        let newer =
            Version::parse(&record.version).is_ok_and(|version| version > installation.current);
        let path = installation.packages_dir.join(&record.file);
        if !newer || !acceptable_file_name(&record.file) || verify_package(&path, &record).is_err()
        {
            installation.clear_record();
            return None;
        }
        Some(Ready { record })
    }

    fn reverify(&self, ready: &Ready) -> Result<PathBuf, UpdateError> {
        let path = self.installation.packages_dir.join(&ready.record.file);
        if let Err(error) = verify_package(&path, &ready.record) {
            let _ = fs::remove_file(&path);
            self.installation.clear_record();
            return Err(error);
        }
        Ok(path)
    }

    /// Hand a verified package to the Velopack updater, which waits for this
    /// process to exit, applies it and relaunches Made. The caller must close
    /// every workspace first, then exit promptly when this returns `Ok`.
    pub fn restart_into(&self, ready: &Ready) -> Result<(), UpdateError> {
        let package = self.reverify(ready)?;
        let mut args = apply_args(&self.installation, &package, false);
        args.push("--waitPid".into());
        args.push(std::process::id().to_string().into());
        spawn_updater(&self.installation.update_exe, args).map(|_| ())
    }

    /// Apply a verified package from the command line without restarting.
    /// Refuses while any Made instance of this install runs; holds the install
    /// lock for the whole apply. On Windows the running executable lives in
    /// the directory being replaced, so the apply is handed to a temporary
    /// copy of this executable that waits for this process to exit.
    pub fn apply_from_cli(&self, ready: &Ready) -> Result<CliApplyOutcome, UpdateError> {
        let lock = InstallLock::acquire(&self.installation.packages_dir)?;
        let running = running_instances(&self.installation.packages_dir);
        if !running.is_empty() {
            return Err(UpdateError::InstanceRunning(running));
        }
        let package = self.reverify(ready)?;
        #[cfg(target_os = "windows")]
        {
            let _ = &package;
            drop(lock);
            let pid = spawn_windows_helper(&self.installation)?;
            Ok(CliApplyOutcome::HandedOff {
                pid,
                version: ready.version().to_owned(),
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            run_updater_to_completion(&self.installation, &package)?;
            drop(lock);
            self.installation.clear_record();
            Ok(CliApplyOutcome::Applied(ready.version().to_owned()))
        }
    }
}

fn acceptable_file_name(name: &str) -> bool {
    name.ends_with("-full.nupkg")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn remove_other_packages(packages_dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(packages_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != keep && (name.ends_with(".nupkg") || name.ends_with(".partial")) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(target_os = "windows")]
fn refresh_windows_updater(package: &Path, update_exe: &Path) {
    // Velopack's own download path does the same: the package carries the
    // updater that should apply it.
    if let Ok(bundle) = velopack::bundle::load_bundle_from_file(package) {
        let _ =
            bundle.extract_zip_predicate_to_path(|name| name.ends_with("Squirrel.exe"), update_exe);
    }
}

fn apply_args(installation: &Installation, package: &Path, silent: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["apply".into(), "--package".into(), package.into()];
    if silent {
        args.push("--silent".into());
        args.push("--norestart".into());
    }
    args.push("--root".into());
    args.push(installation.root.clone().into());
    args.push("--packageDir".into());
    args.push(installation.packages_dir.clone().into());
    args
}

fn spawn_updater(
    update_exe: &Path,
    args: Vec<OsString>,
) -> Result<std::process::Child, UpdateError> {
    std::process::Command::new(update_exe)
        .args(args)
        .spawn()
        .map_err(|error| UpdateError::Local(format!("cannot start the updater: {error}")))
}

#[cfg(not(target_os = "windows"))]
fn run_updater_to_completion(
    installation: &Installation,
    package: &Path,
) -> Result<(), UpdateError> {
    let status = spawn_updater(
        &installation.update_exe,
        apply_args(installation, package, true),
    )?
    .wait()
    .map_err(|error| UpdateError::Local(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::Local(format!(
            "the updater exited with {status}"
        )))
    }
}

#[cfg(target_os = "windows")]
fn spawn_windows_helper(installation: &Installation) -> Result<u32, UpdateError> {
    let exe = std::env::current_exe().map_err(|error| UpdateError::Local(error.to_string()))?;
    let dir = std::env::temp_dir().join(format!("made-update-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|error| UpdateError::Local(error.to_string()))?;
    let copy = dir.join("made-app.exe");
    fs::copy(&exe, &copy).map_err(|error| UpdateError::Local(error.to_string()))?;
    let child = std::process::Command::new(&copy)
        .arg(WINDOWS_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(installation.root())
        .spawn()
        .map_err(|error| UpdateError::Local(format!("cannot start the apply helper: {error}")))?;
    Ok(child.id())
}

/// Hidden flag that runs the Windows apply helper:
/// `<flag> <wait-pid> <install-root>`.
pub const WINDOWS_HELPER_FLAG: &str = "--update-apply-helper";

/// Windows apply helper: wait for the invoking process to exit, then apply
/// the kept package under the install lock, without restarting Made.
#[cfg(target_os = "windows")]
pub fn run_windows_helper(wait_pid: u32, root: &Path) -> Result<String, UpdateError> {
    let _ = velopack::process::wait_for_pid_to_exit(wait_pid, Some(Duration::from_secs(60)));
    let installation = Installation::from_windows_root(root)
        .ok_or_else(|| UpdateError::Local("the install root is not a Made installation".into()))?;
    let updater = Updater {
        source: ReleaseSource::production(&installation.current_version()),
        installation,
    };
    let ready = updater
        .kept()
        .ok_or_else(|| UpdateError::Local("no verified update is kept for this install".into()))?;
    let lock = InstallLock::acquire(&updater.installation.packages_dir)?;
    let running = running_instances(&updater.installation.packages_dir);
    if !running.is_empty() {
        return Err(UpdateError::InstanceRunning(running));
    }
    let package = updater.reverify(&ready)?;
    let status = spawn_updater(
        &updater.installation.update_exe,
        apply_args(&updater.installation, &package, true),
    )?
    .wait()
    .map_err(|error| UpdateError::Local(error.to_string()))?;
    drop(lock);
    if !status.success() {
        return Err(UpdateError::Local(format!(
            "the updater exited with {status}"
        )));
    }
    updater.installation.clear_record();
    Ok(ready.version().to_owned())
}

/// Windows has no apply helper elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn run_windows_helper(_wait_pid: u32, _root: &Path) -> Result<String, UpdateError> {
    Err(UpdateError::Local(
        "the apply helper exists only on Windows".into(),
    ))
}

// ---------------------------------------------------------------------------
// Install lock and instance registry
// ---------------------------------------------------------------------------

/// An exclusive, non-blocking lock on one file, released on drop.
struct FileLock {
    _file: File,
}

impl FileLock {
    fn try_lock(path: &Path) -> io::Result<Option<Self>> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            // SAFETY: flock on a file descriptor this function owns.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Some(Self { _file: file }));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Ok(None)
            } else {
                Err(error)
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const ERROR_SHARING_VIOLATION: i32 = 32;
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(path)
            {
                Ok(file) => Ok(Some(Self { _file: file })),
                Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Ok(None),
                Err(error) => Err(error),
            }
        }
    }
}

/// The per-install apply lock. Held by a command-line apply for its whole
/// duration; GUI startup refuses while it is held. Downloads use a separate
/// lock so a background download never blocks another window from starting.
pub struct InstallLock {
    _lock: FileLock,
}

impl InstallLock {
    fn acquire(packages_dir: &Path) -> Result<Self, UpdateError> {
        fs::create_dir_all(packages_dir).map_err(|error| UpdateError::Local(error.to_string()))?;
        match FileLock::try_lock(&packages_dir.join(INSTALL_LOCK)) {
            Ok(Some(lock)) => Ok(Self { _lock: lock }),
            Ok(None) => Err(UpdateError::Busy),
            Err(error) => Err(UpdateError::Local(error.to_string())),
        }
    }
}

/// Registration of one running GUI instance, released on drop.
pub struct InstanceGuard {
    _lock: Option<FileLock>,
    path: Option<PathBuf>,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self._lock = None;
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Why GUI startup must not continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRefused;

impl fmt::Display for StartupRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "an update is being applied to this Made install; start Made again when it finishes",
        )
    }
}

/// Register a GUI instance of `installation`: the instance is announced first,
/// then startup refuses if a command-line apply holds the install lock, so a
/// concurrent apply always sees either the instance or its own lock.
pub fn register_instance(installation: &Installation) -> Result<InstanceGuard, StartupRefused> {
    let dir = &installation.packages_dir;
    if fs::create_dir_all(dir).is_err() {
        return Ok(InstanceGuard {
            _lock: None,
            path: None,
        });
    }
    let path = dir.join(format!("{INSTANCE_PREFIX}{}.lock", std::process::id()));
    let lock = FileLock::try_lock(&path).ok().flatten();
    let guard = InstanceGuard {
        _lock: lock,
        path: Some(path),
    };
    match FileLock::try_lock(&dir.join(INSTALL_LOCK)) {
        Ok(None) => Err(StartupRefused),
        _ => Ok(guard),
    }
}

/// Process ids of running GUI instances of the install whose packages
/// directory is `packages_dir`. Stale registrations are removed.
fn running_instances(packages_dir: &Path) -> Vec<u32> {
    let mut running = Vec::new();
    let Ok(entries) = fs::read_dir(packages_dir) else {
        return running;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix(INSTANCE_PREFIX)
            .and_then(|rest| rest.strip_suffix(".lock"))
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        match FileLock::try_lock(&entry.path()) {
            Ok(Some(lock)) => {
                drop(lock);
                let _ = fs::remove_file(entry.path());
            }
            _ => running.push(pid),
        }
    }
    running.sort_unstable();
    running
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_registration_is_seen_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let installation = Installation {
            root: dir.path().into(),
            update_exe: dir.path().join("Update"),
            packages_dir: dir.path().join("packages"),
            current_bin_dir: PathBuf::new(),
            manifest_path: PathBuf::new(),
            data_dir: dir.path().join("data"),
            current: Version::new(0, 1, 0),
            portable: true,
        };
        // A registration under another pid is what a CLI apply must see.
        fs::create_dir_all(&installation.packages_dir).unwrap();
        let other = installation
            .packages_dir
            .join(format!("{INSTANCE_PREFIX}4242.lock"));
        let held = FileLock::try_lock(&other).unwrap().unwrap();
        assert_eq!(running_instances(&installation.packages_dir), vec![4242]);
        drop(held);
        assert!(running_instances(&installation.packages_dir).is_empty());
        assert!(!other.exists(), "stale registrations are removed");

        let guard = register_instance(&installation).unwrap();
        let lock = InstallLock::acquire(&installation.packages_dir).unwrap();
        assert_eq!(register_instance(&installation).err(), Some(StartupRefused));
        drop(lock);
        drop(guard);
    }
}
