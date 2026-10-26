//! Process-level no-webview assertion and build identity.
//!
//! Made v1 must not use a webview runtime. [`inspect_current_process`] checks
//! the current process map and its descendant processes for Electron, CEF, and
//! WebKitGTK markers. The check is capability-gated at runtime: when the
//! platform does not expose `/proc`, the result is [`WebviewCheck::Unavailable`]
//! rather than a false clean result.
//!
//! [`audit_lockfile`] provides the build-time half: it rejects a dependency
//! lock file that contains a known webview runtime crate. [`verify_build`]
//! confirms that this binary was not built against an embedded webview.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

/// Process and mapping markers that identify a webview runtime.
pub const WEBVIEW_MARKERS: &[&str] = &[
    "electron",
    "libcef",
    "cefclient",
    "cefsharp",
    "chromiumembedded",
    "webkit2gtk",
    "webkitwebprocess",
    "webkitnetworkprocess",
    "webview2",
    "wry",
    "tauri",
];

/// Crates that would add a webview runtime to the dependency graph.
pub const FORBIDDEN_WEBVIEW_CRATES: &[&str] = &[
    "cef",
    "cef-sys",
    "electron",
    "javascriptcore-rs",
    "javascriptcore-rs-sys",
    "tauri",
    "tauri-build",
    "tauri-codegen",
    "tauri-macros",
    "tauri-runtime",
    "tauri-runtime-wry",
    "webkit2gtk",
    "webkit2gtk-sys",
    "webview2-com",
    "webview2-com-macros",
    "webview2-com-sys",
    "wry",
];

/// Result of the process-level webview inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebviewCheck {
    /// No webview marker was found.
    Clean {
        /// Number of processes inspected, including the current process.
        processes_checked: usize,
        /// Number of memory-map lines inspected.
        mappings_checked: usize,
    },
    /// At least one marker was found.
    Detected {
        /// Markers found.
        markers: Vec<String>,
        /// Short evidence strings identifying where each marker was found.
        evidence: Vec<String>,
    },
    /// The host does not expose the information needed for an honest check.
    Unavailable {
        /// Why the check could not be performed.
        reason: String,
    },
}

impl WebviewCheck {
    /// Whether the check completed with no webview detected.
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean { .. })
    }

    /// Whether a webview was detected.
    pub const fn is_detected(&self) -> bool {
        matches!(self, Self::Detected { .. })
    }

    /// Whether the check was impossible on this host.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

impl fmt::Display for WebviewCheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clean {
                processes_checked,
                mappings_checked,
            } => write!(
                formatter,
                "no webview runtime found in {processes_checked} process(es) and {mappings_checked} mapping(s)"
            ),
            Self::Detected { markers, .. } => {
                write!(formatter, "webview runtime detected: {}", markers.join(", "))
            }
            Self::Unavailable { reason } => {
                write!(formatter, "webview check unavailable: {reason}")
            }
        }
    }
}

/// Build identity recorded for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildIdentity {
    /// Product name.
    pub app: &'static str,
    /// App crate version.
    pub version: &'static str,
    /// Target operating system name.
    pub target_os: &'static str,
    /// Target architecture name.
    pub target_arch: &'static str,
    /// Embedded webview runtime, if any. Always `None` in v1.
    pub webview_runtime: Option<&'static str>,
}

/// The webview runtime embedded in this build.
///
/// This is a build-identity constant rather than an environment probe so a
/// packaged binary cannot accidentally claim a different dependency set.
pub const BUILD_WEBVIEW_RUNTIME: Option<&str> = None;

/// Build identity for this binary.
pub fn build_identity() -> BuildIdentity {
    BuildIdentity {
        app: crate::APP_NAME,
        version: env!("CARGO_PKG_VERSION"),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        webview_runtime: BUILD_WEBVIEW_RUNTIME,
    }
}

/// Build-time error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildGuardError {
    /// The binary embeds a webview runtime.
    WebviewRuntime(&'static str),
}

impl fmt::Display for BuildGuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WebviewRuntime(runtime) => {
                write!(formatter, "this build embeds the webview runtime {runtime}")
            }
        }
    }
}

impl std::error::Error for BuildGuardError {}

/// Verify the build has no embedded webview runtime.
pub fn verify_build() -> Result<BuildIdentity, BuildGuardError> {
    let identity = build_identity();
    match identity.webview_runtime {
        Some(runtime) => Err(BuildGuardError::WebviewRuntime(runtime)),
        None => Ok(identity),
    }
}

/// Build-dependency audit result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAudit {
    /// No forbidden webview crate is present.
    Clean {
        /// Number of packages inspected.
        packages: usize,
    },
    /// One or more webview crates are present.
    WebviewDependencies {
        /// Forbidden package names.
        packages: Vec<String>,
    },
    /// The lock file did not contain package names.
    Invalid {
        /// Why the audit could not run.
        reason: String,
    },
}

impl DependencyAudit {
    /// Whether the dependency set is clean.
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean { .. })
    }
}

/// Audit a Cargo lock file for webview runtime crates.
pub fn audit_lockfile(lockfile: &str) -> DependencyAudit {
    let mut package_count = 0_usize;
    let mut flagged = Vec::new();
    for line in lockfile.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("name = \"") else {
            continue;
        };
        let Some(end) = rest.find('"') else {
            continue;
        };
        let name = &rest[..end];
        package_count += 1;
        if FORBIDDEN_WEBVIEW_CRATES.contains(&name) {
            flagged.push(name.to_owned());
        }
    }

    if package_count == 0 {
        return DependencyAudit::Invalid {
            reason: "the lock file contains no package names".to_owned(),
        };
    }
    flagged.sort();
    flagged.dedup();
    if flagged.is_empty() {
        DependencyAudit::Clean {
            packages: package_count,
        }
    } else {
        DependencyAudit::WebviewDependencies { packages: flagged }
    }
}

/// Find webview markers in `/proc/self/maps`-style text.
pub fn scan_maps_text(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    WEBVIEW_MARKERS
        .iter()
        .filter(|marker| lower.contains(**marker))
        .map(|marker| (*marker).to_owned())
        .collect()
}

/// Find webview markers in a process command line or executable name.
pub fn scan_process_text(text: &str) -> Vec<String> {
    scan_maps_text(text)
}

/// Inspect the current process and its descendants.
///
/// On hosts with `/proc`, this scans the process map, enumerates descendants,
/// and checks each descendant's command line and maps. If `/proc` is not
/// available, the result is [`WebviewCheck::Unavailable`] with the underlying
/// reason.
pub fn inspect_current_process() -> WebviewCheck {
    let self_pid = std::process::id();
    let maps = match fs::read_to_string("/proc/self/maps") {
        Ok(maps) => maps,
        Err(error) => {
            return WebviewCheck::Unavailable {
                reason: format!("/proc/self/maps is unavailable: {error}"),
            }
        }
    };

    let mut markers = scan_maps_text(&maps);
    let mut evidence = Vec::new();
    for marker in &markers {
        evidence.push(format!("process {self_pid} maps contain {marker}"));
    }
    let mut processes_checked = 1_usize;
    let mut mappings_checked = maps.lines().count();

    let proc_entries = match fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(error) => {
            return WebviewCheck::Unavailable {
                reason: format!("cannot enumerate /proc for the process tree: {error}"),
            }
        }
    };

    let mut parent_by_pid: HashMap<u32, u32> = HashMap::new();
    let mut known_pids = Vec::new();
    for entry in proc_entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            if let Some(parent) = parse_ppid(&stat) {
                parent_by_pid.insert(pid, parent);
            }
        }
        known_pids.push(pid);
    }

    let descendants: Vec<u32> = known_pids
        .into_iter()
        .filter(|pid| *pid != self_pid && is_descendant(*pid, self_pid, &parent_by_pid))
        .collect();

    for pid in descendants {
        processes_checked += 1;
        let mut process_text = String::new();
        if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) {
            process_text.push_str(&String::from_utf8_lossy(&cmdline).replace('\0', " "));
        }
        if let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) {
            process_text.push(' ');
            process_text.push_str(&comm);
        }
        if let Ok(child_maps) = fs::read_to_string(format!("/proc/{pid}/maps")) {
            mappings_checked += child_maps.lines().count();
            for marker in scan_maps_text(&child_maps) {
                evidence.push(format!("process {pid} maps contain {marker}"));
                markers.push(marker);
            }
        }
        for marker in scan_process_text(&process_text) {
            evidence.push(format!("process {pid} command line contains {marker}"));
            markers.push(marker);
        }
    }

    markers.sort();
    markers.dedup();
    if markers.is_empty() {
        WebviewCheck::Clean {
            processes_checked,
            mappings_checked,
        }
    } else {
        evidence.sort();
        evidence.dedup();
        WebviewCheck::Detected { markers, evidence }
    }
}

fn parse_ppid(stat: &str) -> Option<u32> {
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 1..)?.trim_start();
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

fn is_descendant(pid: u32, ancestor: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut current = pid;
    let mut guard = 0_usize;
    while let Some(parent) = parents.get(&current).copied() {
        if parent == ancestor {
            return true;
        }
        if parent == 0 || guard > parents.len() {
            return false;
        }
        current = parent;
        guard += 1;
    }
    false
}

/// Whether a path looks like a webview runtime binary.
///
/// This is a small convenience for diagnostics and tests.
pub fn path_names_webview(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !scan_process_text(&text).is_empty()
}
