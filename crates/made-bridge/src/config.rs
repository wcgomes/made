//! In-memory OpenCode configuration composition for a Made launch.
//!
//! Composition never writes a file. The caller supplies the path of the user's
//! `cli.json`, the current `OPENCODE_CLI_CONFIG_CONTENT` value (if any), and
//! the app-owned plugin directory. The result is a JSON object suitable for the
//! child environment plus the canonical plugin path that was injected.
//!
//! Merge rules:
//!
//! 1. The user `cli.json` object is the base.
//! 2. An existing inline config overlays the base. Objects merge recursively;
//!    plugin arrays are concatenated so no existing plugin entry or option is
//!    lost; other values from the overlay win.
//! 3. Every entry identifying the Made plugin package is removed, then exactly
//!    one Made entry is appended with namespaced `options.made`. Relative
//!    package entries are resolved against the user `cli.json` parent
//!    directory; package-name/id references and the canonical package path are
//!    also recognized.
//! 4. `tabs.mode` is forced to `"on"` and `tabs.scope` to `"global"` while all
//!    other tab keys survive.
//!
//! The app-owned plugin path must be a directory inside the supplied
//! [`ComposeOptions::app_owned_root`] and must contain `package.json` and
//! `tui.ts` (OpenCode requires a package directory, not a direct `.ts` entry).
//!
//! If either input is invalid, the caller receives a typed error and should
//! launch OpenCode without the Made overlay. User files are never rewritten.

use std::{
    collections::HashMap,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};
use thiserror::Error;

use crate::{
    protocol::PROTOCOL_VERSION,
    security::{is_loopback_ws_url, redact_secret, MAX_APP_VERSION_CHARS, MAX_PANE_ID_CHARS},
    Token,
};

/// Environment variable consumed by OpenCode for an inline CLI configuration.
pub const CONFIG_CONTENT_ENV: &str = "OPENCODE_CLI_CONFIG_CONTENT";

/// Package name declared by the app-owned Made plugin.
pub const MADE_PLUGIN_PACKAGE_NAME: &str = "opencode-made";

/// TUI plugin identity declared by the app-owned Made plugin.
pub const MADE_PLUGIN_ID: &str = "made.opencode.bridge";

/// Files that must exist in an app-owned plugin package directory.
///
/// OpenCode does not load a direct `.ts` entry from `cli.json`; it loads a
/// directory package containing `tui.ts` (Phase 0 finding). `package.json`
/// keeps the package shape OpenCode expects.
pub const PLUGIN_REQUIRED_FILES: [&str; 2] = ["package.json", "tui.ts"];

/// Inputs for [`compose`].
#[derive(Clone)]
pub struct ComposeOptions {
    /// App-owned plugin package directory. Must canonicalize inside
    /// [`app_owned_root`](Self::app_owned_root).
    pub plugin_dir: PathBuf,
    /// App-owned installation/resource root that contains the plugin package.
    pub app_owned_root: PathBuf,
    /// User `cli.json` path. `None` composes from the inline overlay only.
    pub user_config_path: Option<PathBuf>,
    /// Existing `OPENCODE_CLI_CONFIG_CONTENT` value, if present.
    pub inline_config: Option<String>,
    /// Loopback bridge URL supplied to the plugin.
    pub bridge_url: String,
    /// Per-instance bridge token.
    pub token: Token,
    /// Terminal pane identity.
    pub pane_id: String,
    /// Made application version.
    pub app_version: String,
}

impl ComposeOptions {
    /// Build composition inputs with no user config path and no inline overlay.
    pub fn new(
        plugin_dir: impl Into<PathBuf>,
        app_owned_root: impl Into<PathBuf>,
        bridge_url: impl Into<String>,
        token: Token,
        pane_id: impl Into<String>,
        app_version: impl Into<String>,
    ) -> Self {
        Self {
            plugin_dir: plugin_dir.into(),
            app_owned_root: app_owned_root.into(),
            user_config_path: None,
            inline_config: None,
            bridge_url: bridge_url.into(),
            token,
            pane_id: pane_id.into(),
            app_version: app_version.into(),
        }
    }

    /// Set the user `cli.json` path to read as the base. The file is never written.
    pub fn with_user_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.user_config_path = Some(path.into());
        self
    }

    /// Set the existing inline configuration to overlay.
    pub fn with_inline_config(mut self, content: impl Into<String>) -> Self {
        self.inline_config = Some(content.into());
        self
    }

    /// Set the inline configuration from the current process environment.
    ///
    /// An unset or whitespace-only value leaves the overlay absent.
    pub fn with_inline_config_from_env(mut self) -> Self {
        self.inline_config = inline_config_from_env();
        self
    }
}

/// Read the current `OPENCODE_CLI_CONFIG_CONTENT` value, if it is set and not
/// empty. Composition never mutates the environment.
pub fn inline_config_from_env() -> Option<String> {
    std::env::var(CONFIG_CONTENT_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

impl fmt::Debug for ComposeOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposeOptions")
            .field("plugin_dir", &self.plugin_dir)
            .field("app_owned_root", &self.app_owned_root)
            .field("user_config_path", &self.user_config_path)
            .field(
                "inline_config",
                &self.inline_config.as_ref().map(|_| "<redacted>"),
            )
            .field("bridge_url", &redact_secret(&self.bridge_url, &self.token))
            .field("token", &crate::security::REDACTED)
            .field("pane_id", &self.pane_id)
            .field("app_version", &self.app_version)
            .finish()
    }
}

/// A successfully composed launch configuration.
///
/// The content contains the bridge token because the plugin must receive it.
/// `Debug` redacts the entire content and the token to keep accidental logging
/// safe.
#[derive(Clone, PartialEq, Eq)]
pub struct ComposedConfig {
    /// JSON content for `OPENCODE_CLI_CONFIG_CONTENT`.
    ///
    /// This value contains the bridge token and must never be logged. `Debug`
    /// redacts it.
    pub content: String,
    /// Canonical app-owned plugin package directory that was injected.
    pub plugin_dir: PathBuf,
    /// Non-fatal diagnostics for this composition.
    pub warnings: Vec<String>,
}

impl ComposedConfig {
    /// The environment variable and value to set on the terminal child.
    pub fn env_pair(&self) -> (&'static str, &str) {
        (CONFIG_CONTENT_ENV, &self.content)
    }

    /// Return a map containing only the composed configuration variable.
    ///
    /// This is convenient for platforms that accept an environment overlay
    /// rather than a single key/value pair. The value contains the token, so
    /// do not log the returned map.
    pub fn environment(&self) -> HashMap<String, String> {
        HashMap::from([(CONFIG_CONTENT_ENV.to_owned(), self.content.clone())])
    }
}

impl fmt::Debug for ComposedConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposedConfig")
            .field(
                "content",
                &format_args!("<redacted; {} bytes>", self.content.len()),
            )
            .field("plugin_dir", &self.plugin_dir)
            .field("warnings", &self.warnings)
            .finish()
    }
}

/// Typed configuration composition failures.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The user config file existed but could not be read.
    #[error("could not read user config {path}: {source}")]
    ReadBase {
        /// Supplied user config path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The user config file was not valid JSON.
    #[error("user config {path} is not valid JSON: {source}")]
    InvalidBaseJson {
        /// Supplied user config path.
        path: PathBuf,
        /// JSON parse error.
        #[source]
        source: serde_json::Error,
    },
    /// The user config root was not a JSON object.
    #[error("user config {path} must contain a JSON object")]
    BaseNotObject {
        /// Supplied user config path.
        path: PathBuf,
    },
    /// The inline overlay was not valid JSON.
    #[error("OPENCODE_CLI_CONFIG_CONTENT is not valid JSON: {0}")]
    InvalidOverlayJson(#[source] serde_json::Error),
    /// The inline overlay root was not a JSON object.
    #[error("OPENCODE_CLI_CONFIG_CONTENT must contain a JSON object")]
    OverlayNotObject,
    /// A field had an unsupported shape.
    #[error("`{field}` in {container} must be {expected}")]
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Expected shape.
        expected: &'static str,
        /// Which input contained the field.
        container: &'static str,
    },
    /// The app-owned root could not be canonicalized.
    #[error("could not canonicalize app-owned root {path}: {source}")]
    AppRootPath {
        /// Supplied root.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The plugin path could not be canonicalized.
    #[error("could not canonicalize plugin path {path}: {source}")]
    PluginPath {
        /// Supplied plugin path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The plugin path resolved outside the app-owned root.
    #[error("plugin path {plugin} is outside the app-owned root {root}")]
    PluginOutsideRoot {
        /// Canonical plugin path.
        plugin: PathBuf,
        /// Canonical app-owned root.
        root: PathBuf,
    },
    /// The plugin path resolved to something other than a directory.
    #[error("plugin path {0} is not a directory")]
    PluginNotDirectory(PathBuf),
    /// The plugin package directory is missing a required entry.
    #[error("plugin package {path} is missing required entry {file}")]
    PluginMissingFile {
        /// Canonical plugin package directory.
        path: PathBuf,
        /// Required file name.
        file: &'static str,
    },
    /// The bridge URL was not a loopback `ws://.../made` URL.
    #[error("bridge URL {url} is not a loopback ws:// URL with an explicit port and /made path")]
    InvalidBridgeUrl {
        /// Rejected URL.
        url: String,
    },
    /// A required option was empty or too long.
    #[error("{field} {reason}")]
    InvalidOption {
        /// Option name.
        field: &'static str,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// Serializing the composed object failed. This should be unreachable for
    /// a `serde_json::Value` object, but is reported rather than panicking.
    #[error("could not serialize composed config: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Compose a per-launch OpenCode CLI configuration in memory.
///
/// The user config is read only. See the module documentation for merge rules.
/// On success the returned plugin directory is canonical and verified to be
/// inside `options.app_owned_root`.
///
/// This function uses exactly the overlay supplied in `options`. Use
/// [`compose_with_environment`] to pick up the current process's
/// `OPENCODE_CLI_CONFIG_CONTENT`.
pub fn compose(options: &ComposeOptions) -> Result<ComposedConfig, ConfigError> {
    if options.bridge_url.is_empty() || !is_loopback_ws_url(&options.bridge_url) {
        return Err(ConfigError::InvalidBridgeUrl {
            url: redact_secret(&options.bridge_url, &options.token),
        });
    }
    if options.pane_id.is_empty() || options.pane_id.chars().count() > MAX_PANE_ID_CHARS {
        return Err(ConfigError::InvalidOption {
            field: "paneId",
            reason: "is missing or too long",
        });
    }
    if options.app_version.is_empty() || options.app_version.chars().count() > MAX_APP_VERSION_CHARS
    {
        return Err(ConfigError::InvalidOption {
            field: "appVersion",
            reason: "is missing or too long",
        });
    }

    let app_root =
        fs::canonicalize(&options.app_owned_root).map_err(|source| ConfigError::AppRootPath {
            path: options.app_owned_root.clone(),
            source,
        })?;
    let plugin_dir =
        fs::canonicalize(&options.plugin_dir).map_err(|source| ConfigError::PluginPath {
            path: options.plugin_dir.clone(),
            source,
        })?;
    if !plugin_dir.starts_with(&app_root) {
        return Err(ConfigError::PluginOutsideRoot {
            plugin: plugin_dir,
            root: app_root,
        });
    }
    let metadata = fs::metadata(&plugin_dir).map_err(|source| ConfigError::PluginPath {
        path: plugin_dir.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(ConfigError::PluginNotDirectory(plugin_dir));
    }
    for file in PLUGIN_REQUIRED_FILES {
        if !plugin_dir.join(file).is_file() {
            return Err(ConfigError::PluginMissingFile {
                path: plugin_dir.clone(),
                file,
            });
        }
    }

    let config_dir = options
        .user_config_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);

    let mut warnings = Vec::new();
    let mut config = read_base(options.user_config_path.as_deref(), &mut warnings)?;

    if let Some(inline) = options.inline_config.as_deref() {
        if !inline.trim().is_empty() {
            let overlay: Value =
                serde_json::from_str(inline).map_err(ConfigError::InvalidOverlayJson)?;
            let Value::Object(overlay) = overlay else {
                return Err(ConfigError::OverlayNotObject);
            };
            merge_objects(&mut config, overlay)?;
        }
    }

    force_tabs(&mut config, &mut warnings);

    let plugin_display = plugin_dir.to_string_lossy().into_owned();
    let mut plugins = match config.remove("plugins") {
        Some(Value::Array(entries)) => entries,
        Some(_) => {
            return Err(ConfigError::InvalidField {
                field: "plugins",
                expected: "an array",
                container: "the composed config",
            })
        }
        None => Vec::new(),
    };

    let mut removed = 0_usize;
    plugins.retain(|entry| {
        if identifies_plugin(entry, &plugin_dir, &plugin_display, config_dir.as_deref()) {
            removed += 1;
            false
        } else {
            true
        }
    });
    if removed > 0 {
        warnings.push(format!(
            "Removed {removed} existing Made plugin {} before appending the launch entry.",
            if removed == 1 { "entry" } else { "entries" }
        ));
    }

    plugins.push(json!({
        "package": plugin_display,
        "options": {
            "made": {
                "bridgeUrl": options.bridge_url,
                "token": options.token.expose_secret(),
                "paneId": options.pane_id,
                "protocolVersion": PROTOCOL_VERSION,
                "appVersion": options.app_version,
            }
        }
    }));
    config.insert("plugins".into(), Value::Array(plugins));

    let content = serde_json::to_string(&Value::Object(config)).map_err(ConfigError::Serialize)?;

    Ok(ComposedConfig {
        content,
        plugin_dir,
        warnings,
    })
}

/// Compose using the current process's `OPENCODE_CLI_CONFIG_CONTENT` overlay
/// when `options.inline_config` is absent.
///
/// The environment is read once at call time; composition still never writes a
/// file or mutates the environment.
pub fn compose_with_environment(options: &ComposeOptions) -> Result<ComposedConfig, ConfigError> {
    let mut options = options.clone();
    if options.inline_config.is_none() {
        options.inline_config = inline_config_from_env();
    }
    compose(&options)
}

fn read_base(
    path: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Result<Map<String, Value>, ConfigError> {
    let Some(path) = path else {
        return Ok(Map::new());
    };

    match fs::read_to_string(path) {
        Ok(content) => {
            let value: Value =
                serde_json::from_str(&content).map_err(|source| ConfigError::InvalidBaseJson {
                    path: path.to_path_buf(),
                    source,
                })?;
            let Value::Object(object) = value else {
                return Err(ConfigError::BaseNotObject {
                    path: path.to_path_buf(),
                });
            };
            Ok(object)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            warnings.push(format!(
                "User config {} was not found; composed from other inputs.",
                path.display()
            ));
            Ok(Map::new())
        }
        Err(source) => Err(ConfigError::ReadBase {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn merge_objects(
    base: &mut Map<String, Value>,
    overlay: Map<String, Value>,
) -> Result<(), ConfigError> {
    merge_object_fields(base, overlay, true)
}

fn merge_object_fields(
    base: &mut Map<String, Value>,
    overlay: Map<String, Value>,
    top_level: bool,
) -> Result<(), ConfigError> {
    for (key, value) in overlay {
        if top_level && key == "plugins" {
            let existing = base.remove("plugins");
            let mut combined = plugin_array(existing, "user config")?;
            combined.extend(plugin_array(Some(value), "OPENCODE_CLI_CONFIG_CONTENT")?);
            base.insert(key, Value::Array(combined));
            continue;
        }

        match (base.get_mut(&key), value) {
            (Some(Value::Object(base_object)), Value::Object(overlay_object)) => {
                merge_object_fields(base_object, overlay_object, false)?;
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
    Ok(())
}

fn plugin_array(value: Option<Value>, source: &'static str) -> Result<Vec<Value>, ConfigError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(entries)) => Ok(entries),
        Some(_) => Err(ConfigError::InvalidField {
            field: "plugins",
            expected: "an array",
            container: source,
        }),
    }
}

fn force_tabs(config: &mut Map<String, Value>, warnings: &mut Vec<String>) {
    let mut tabs = match config.remove("tabs") {
        Some(Value::Object(object)) => object,
        Some(Value::Null) | None => Map::new(),
        Some(_) => {
            warnings.push(
                "Replaced a non-object tabs configuration with the required v1 settings.".into(),
            );
            Map::new()
        }
    };

    if tabs.get("mode").and_then(Value::as_str) != Some("on") {
        tabs.insert("mode".into(), Value::String("on".into()));
        warnings.push("Forced tabs.mode to \"on\" for this Made launch.".into());
    }
    if tabs.get("scope").and_then(Value::as_str) != Some("global") {
        tabs.insert("scope".into(), Value::String("global".into()));
        warnings.push("Forced tabs.scope to \"global\" for this Made launch.".into());
    }

    config.insert("tabs".into(), Value::Object(tabs));
}

fn identifies_plugin(
    value: &Value,
    canonical: &Path,
    display: &str,
    config_dir: Option<&Path>,
) -> bool {
    let candidate = match value {
        Value::String(package) => package.as_str(),
        Value::Object(object) => match object.get("package").and_then(Value::as_str) {
            Some(package) => package,
            None => return false,
        },
        _ => return false,
    };

    if candidate == display || is_made_package_reference(candidate) {
        return true;
    }

    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return canonicalize(candidate_path).is_some_and(|resolved| resolved == canonical);
    }

    config_dir
        .and_then(|directory| canonicalize(&directory.join(candidate_path)))
        .is_some_and(|resolved| resolved == canonical)
}

fn is_made_package_reference(candidate: &str) -> bool {
    matches!(
        candidate,
        MADE_PLUGIN_PACKAGE_NAME | MADE_PLUGIN_ID | "@made/opencode-made"
    )
}

fn canonicalize(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}
