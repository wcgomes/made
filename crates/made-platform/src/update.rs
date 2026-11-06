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
const APPLY_OUTCOME: &str = "update-apply-outcome.json";
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

    /// Take the install lock of `packages_dir`, as a command-line apply does.
    pub fn hold_install_lock(packages_dir: &Path) -> Result<InstallLock, UpdateError> {
        InstallLock::acquire(packages_dir)
    }

    /// Make a held install lock inheritable by the next child; its handle.
    #[cfg(windows)]
    pub fn share_install_lock(lock: &InstallLock) -> io::Result<u64> {
        lock.share_with_child()
    }

    /// Adopt an inherited install-lock handle, as the Windows helper does.
    #[cfg(windows)]
    pub fn adopt_install_lock(handle: u64, packages_dir: &Path) -> io::Result<InstallLock> {
        InstallLock::adopt(handle, packages_dir)
    }

    /// A command-line apply whose Windows helper is copied under
    /// `helper_root` instead of the temporary directory.
    pub fn apply_from_cli_with_helper_root(
        updater: &Updater,
        ready: &Ready,
        helper_root: &Path,
    ) -> Result<CliApplyOutcome, UpdateError> {
        updater.apply_from_cli_in(ready, helper_root)
    }

    /// Whether the install lock of `packages_dir` is held.
    pub fn apply_in_progress_at(packages_dir: &Path) -> bool {
        apply_in_progress(packages_dir)
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

    fn write_apply_outcome(&self, version: Option<&str>, applied: bool) {
        let outcome = ApplyOutcome {
            version: version.map(str::to_owned),
            applied: applied && version.is_some(),
        };
        if let Ok(bytes) = serde_json::to_vec(&outcome) {
            let _ =
                crate::write_private_metadata_atomic(&self.data_dir, APPLY_OUTCOME, &bytes, 4096);
        }
    }

    fn read_apply_outcome(&self) -> Option<ApplyOutcome> {
        let bytes = crate::read_private_metadata(&self.data_dir, APPLY_OUTCOME, 4096).ok()??;
        let outcome: ApplyOutcome = serde_json::from_slice(&bytes).ok()?;
        // Only a plain stable version is ever shown; anything else is dropped.
        match &outcome.version {
            None if !outcome.applied => Some(outcome),
            None => None,
            Some(version) => Version::parse(version)
                .ok()
                .filter(|version| version.pre.is_empty() && version.build.is_empty())
                .map(|_| outcome),
        }
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
        let others = self.other_instances();
        if !others.is_empty() {
            return Err(UpdateError::InstanceRunning(others));
        }
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
        self.apply_from_cli_in(ready, &std::env::temp_dir())
    }

    /// [`Self::apply_from_cli`] with the Windows helper copied under
    /// `helper_root`.
    fn apply_from_cli_in(
        &self,
        ready: &Ready,
        helper_root: &Path,
    ) -> Result<CliApplyOutcome, UpdateError> {
        let lock = InstallLock::acquire(&self.installation.packages_dir)?;
        let running = running_instances(&self.installation.packages_dir);
        if !running.is_empty() {
            return Err(UpdateError::InstanceRunning(running));
        }
        let package = self.reverify(ready)?;
        #[cfg(target_os = "windows")]
        {
            let _ = &package;
            // The helper inherits this open lock handle, so the lock stays
            // held from these checks through the swap even after this
            // process exits.
            let pid = spawn_windows_helper(&self.installation, &lock, helper_root)?;
            drop(lock);
            Ok(CliApplyOutcome::HandedOff {
                pid,
                version: ready.version().to_owned(),
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = helper_root;
            let result = run_updater_to_completion(&self.installation, &package);
            self.installation
                .write_apply_outcome(Some(ready.version()), result.is_ok());
            drop(lock);
            result?;
            self.installation.clear_record();
            Ok(CliApplyOutcome::Applied(ready.version().to_owned()))
        }
    }

    /// Whether a command-line apply holds this install's lock right now.
    pub fn apply_in_progress(&self) -> bool {
        apply_in_progress(&self.installation.packages_dir)
    }

    /// Process ids of other Made windows of this install.
    pub fn other_instances(&self) -> Vec<u32> {
        running_instances(&self.installation.packages_dir)
    }

    /// The outcome the last command-line apply recorded, if any.
    pub fn last_cli_apply(&self) -> Option<ApplyOutcome> {
        self.installation.read_apply_outcome()
    }
}

/// The recorded result of a command-line apply (written by the apply itself or
/// the Windows helper), kept in private app data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApplyOutcome {
    /// Version that was applied, when the apply got far enough to know it.
    #[serde(default)]
    pub version: Option<String>,
    /// Whether the updater finished successfully.
    pub applied: bool,
}

impl ApplyOutcome {
    /// One fixed-wording line for terminals and diagnostics.
    pub fn describe(&self) -> String {
        match (&self.version, self.applied) {
            (Some(version), true) => format!("installed Made {version}"),
            (Some(version), false) => {
                format!("failed to install Made {version}; the previous version was kept")
            }
            (None, _) => "failed to install the update; the previous version was kept".to_owned(),
        }
    }
}

fn apply_in_progress(packages_dir: &Path) -> bool {
    matches!(
        FileLock::probe_shared(&packages_dir.join(INSTALL_LOCK)),
        Ok(false)
    )
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
fn spawn_windows_helper(
    installation: &Installation,
    lock: &InstallLock,
    helper_root: &Path,
) -> Result<u32, UpdateError> {
    let local = |error: io::Error| UpdateError::Local(error.to_string());
    let exe = std::env::current_exe().map_err(local)?;
    remove_stale_helper_copies(helper_root);
    // A fresh directory created by this call only: never an existing one
    // whose contents (for example a planted DLL) we did not write.
    let dir = helper_root.join(format!(
        "{HELPER_DIR_PREFIX}{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    fs::create_dir(&dir).map_err(local)?;
    let copy = dir.join("made-app.exe");
    fs::copy(&exe, &copy).map_err(local)?;
    let handle = lock.share_with_child().map_err(local)?;
    let child = std::process::Command::new(&copy)
        .arg(WINDOWS_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(installation.root())
        .arg(handle.to_string())
        .current_dir(&dir)
        .spawn()
        .map_err(|error| UpdateError::Local(format!("cannot start the apply helper: {error}")))?;
    Ok(child.id())
}

/// Remove helper copies left by earlier hand-offs. A helper still running
/// keeps its directory, because Windows refuses to delete a running image.
#[cfg(target_os = "windows")]
fn remove_stale_helper_copies(helper_root: &Path) {
    let Ok(entries) = fs::read_dir(helper_root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(HELPER_DIR_PREFIX)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(target_os = "windows")]
const HELPER_DIR_PREFIX: &str = "made-update-";

/// Hidden flag that runs the Windows apply helper:
/// `<flag> <wait-pid> <install-root> <inherited-lock-handle>`.
pub const WINDOWS_HELPER_FLAG: &str = "--update-apply-helper";

/// Windows apply helper: adopt the install lock inherited from the
/// command-line apply, wait for that process to exit, then verify and apply
/// the kept package without restarting Made, and record the outcome.
#[cfg(target_os = "windows")]
pub fn run_windows_helper(
    wait_pid: u32,
    root: &Path,
    lock_handle: u64,
) -> Result<String, UpdateError> {
    restrict_dll_search();
    let installation = Installation::from_windows_root(root)
        .ok_or_else(|| UpdateError::Local("the install root is not a Made installation".into()))?;
    let lock = InstallLock::adopt(lock_handle, &installation.packages_dir).map_err(|error| {
        UpdateError::Local(format!("the inherited install lock is invalid: {error}"))
    })?;
    let _ = velopack::process::wait_for_pid_to_exit(wait_pid, Some(Duration::from_secs(60)));
    let updater = Updater {
        source: ReleaseSource::production(&installation.current_version()),
        installation,
    };
    // From here on every exit records an outcome, so `--update` never shows
    // an older result while this apply failed.
    let result = helper_apply(&updater);
    let (version, applied) = match &result {
        Ok(version) => (Some(version.as_str()), true),
        Err((version, _)) => (version.as_deref(), false),
    };
    updater.installation.write_apply_outcome(version, applied);
    if applied {
        updater.installation.clear_record();
    }
    drop(lock);
    result.map_err(|(_, error)| error)
}

/// The helper's apply steps; an error carries the version when it is known.
#[cfg(target_os = "windows")]
fn helper_apply(updater: &Updater) -> Result<String, (Option<String>, UpdateError)> {
    let ready = updater.kept().ok_or_else(|| {
        (
            None,
            UpdateError::Local("no verified update is kept for this install".into()),
        )
    })?;
    let version = ready.version().to_owned();
    let fail = |error| (Some(version.clone()), error);
    let running = running_instances(&updater.installation.packages_dir);
    if !running.is_empty() {
        return Err(fail(UpdateError::InstanceRunning(running)));
    }
    let package = updater.reverify(&ready).map_err(fail)?;
    let status = spawn_updater(
        &updater.installation.update_exe,
        apply_args(&updater.installation, &package, true),
    )
    .map_err(fail)?
    .wait()
    .map_err(|error| fail(UpdateError::Local(error.to_string())))?;
    if status.success() {
        Ok(version)
    } else {
        Err(fail(UpdateError::Local(format!(
            "the updater exited with {status}"
        ))))
    }
}

/// Load later DLLs only from System32, never from the helper's directory.
#[cfg(target_os = "windows")]
fn restrict_dll_search() {
    use windows_sys::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, SetDllDirectoryW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    let empty: [u16; 1] = [0];
    // SAFETY: plain Win32 calls with valid arguments.
    unsafe {
        SetDllDirectoryW(empty.as_ptr());
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }
}

/// Windows has no apply helper elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn run_windows_helper(
    _wait_pid: u32,
    _root: &Path,
    _lock_handle: u64,
) -> Result<String, UpdateError> {
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

    /// Probe `path` with a shared lock, released at once. `Ok(true)` when no
    /// process holds it exclusively (or it does not exist); probes from
    /// several processes never block each other, only an exclusive holder.
    fn probe_shared(path: &Path) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = match OpenOptions::new().read(true).open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
                Err(error) => return Err(error),
            };
            // SAFETY: flock on a file descriptor this function owns.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
            if result == 0 {
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Ok(false)
            } else {
                Err(error)
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const ERROR_SHARING_VIOLATION: i32 = 32;
            const FILE_SHARE_READ: u32 = 0x1;
            match OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(path)
            {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
                Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Ok(false),
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

    /// Make the lock's open handle inheritable by the next child process and
    /// return its value. The OS releases the lock once every process holding
    /// the handle has exited, for any reason.
    #[cfg(windows)]
    fn share_with_child(&self) -> io::Result<u64> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
        let handle = self._lock._file.as_raw_handle();
        // SAFETY: the handle is owned by this lock and stays open.
        if unsafe { SetHandleInformation(handle as _, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(handle as usize as u64)
    }

    /// Take ownership of an inherited lock handle after checking it is the
    /// lock file of `packages_dir`.
    #[cfg(windows)]
    fn adopt(handle: u64, packages_dir: &Path) -> io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let raw = handle as usize as *mut std::ffi::c_void;
        let mut buffer = vec![0u16; 1024];
        // SAFETY: querying a handle value; failure is reported, not trusted.
        let length = unsafe {
            GetFinalPathNameByHandleW(raw as _, buffer.as_mut_ptr(), buffer.len() as u32, 0)
        };
        if length == 0 || length as usize >= buffer.len() {
            return Err(io::Error::last_os_error());
        }
        let actual = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        let expected = fs::canonicalize(packages_dir.join(INSTALL_LOCK))?;
        if actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "handle is not this install's lock",
            ));
        }
        // SAFETY: the handle was inherited from the parent for this purpose
        // and is owned by this process from here on.
        let file = unsafe { File::from_raw_handle(raw) };
        // Never pass the lock on to processes this helper starts (the updater).
        // SAFETY: plain Win32 call on the handle `file` now owns.
        if unsafe { SetHandleInformation(raw as _, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            _lock: FileLock { _file: file },
        })
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
pub enum StartupRefused {
    /// A command-line apply holds the install lock.
    ApplyInProgress,
}

impl fmt::Display for StartupRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "An update is being applied to this Made install. Start Made again when it finishes.",
        )
    }
}

/// How this GUI window is known to its install.
pub enum Registration {
    /// Announced; a command-line apply sees it and refuses while it runs.
    Registered(InstanceGuard),
    /// The update data folder could not hold the registration (read-only,
    /// full, or without file locks). Made still starts, with updates off for
    /// this run, so a command-line apply can never replace files under a
    /// window it cannot see from this window's side.
    UpdatesUnavailable(String),
}

/// Register a GUI instance of `installation`: the instance is announced first,
/// then startup refuses if a command-line apply holds the install lock, so a
/// concurrent apply always sees either the instance or its own lock.
pub fn register_instance(installation: &Installation) -> Result<Registration, StartupRefused> {
    let dir = &installation.packages_dir;
    let unavailable = |detail: &str| Ok(Registration::UpdatesUnavailable(detail.to_owned()));
    // An apply in progress is refused first, even when registration would fail.
    if matches!(FileLock::probe_shared(&dir.join(INSTALL_LOCK)), Ok(false)) {
        return Err(StartupRefused::ApplyInProgress);
    }
    if fs::create_dir_all(dir).is_err() {
        return unavailable("the update data folder cannot be created");
    }
    let path = dir.join(format!("{INSTANCE_PREFIX}{}.lock", std::process::id()));
    let Ok(Some(lock)) = FileLock::try_lock(&path) else {
        return unavailable("this window cannot be registered in the update data folder");
    };
    let guard = InstanceGuard {
        _lock: Some(lock),
        path: Some(path),
    };
    // Announce first, then probe again: a concurrent apply sees either this
    // registration or refuses us here.
    match FileLock::probe_shared(&dir.join(INSTALL_LOCK)) {
        Ok(true) => Ok(Registration::Registered(guard)),
        Ok(false) => Err(StartupRefused::ApplyInProgress),
        Err(_) => unavailable("the install lock cannot be checked"),
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
        drop(guard);
        let lock = InstallLock::acquire(&installation.packages_dir).unwrap();
        assert_eq!(
            register_instance(&installation).err(),
            Some(StartupRefused::ApplyInProgress)
        );
        drop(lock);
        assert!(register_instance(&installation).is_ok());
    }
}
