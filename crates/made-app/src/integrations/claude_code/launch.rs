//! Claude Code launch environment: the session-only plugin list and the
//! per-tab hook spool binding.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use made_platform::{AppPaths, PlatformFamily};

use crate::integrations::CLAUDE_CODE;
use crate::tui::TerminalTarget;

/// Environment variable naming a tab's Claude Code hook spool.
pub const CLAUDE_SPOOL_ENV: &str = "MADE_CLAUDE_SPOOL";

/// Claude Code's session-only plugin directory list variable.
pub const CLAUDE_CODE_PLUGIN_DIRS_ENV: &str = "CLAUDE_CODE_PLUGIN_DIRS";

/// The Claude Code environment a tab's terminal child receives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeLaunchEnv {
    /// Variables to set on the PTY child. Empty when the target gets none.
    pub env: Vec<(OsString, OsString)>,
    /// A target that loads the plugin could not: the shipped plugin is missing
    /// or unreadable, or the tab has no spool. Session-status hooks are then
    /// unavailable for that tab.
    pub hooks_unavailable: bool,
}

/// The `CLAUDE_CODE_PLUGIN_DIRS` entry and per-tab spool path for `target`.
///
/// `existing` is the inherited `CLAUDE_CODE_PLUGIN_DIRS` value; its entries
/// keep their order and Made's `shipped` entry is appended once. Targets
/// that do not load the plugin get an empty result. With no `shipped`
/// directory, or one whose path cannot be written into the list safely, no
/// entry or spool binding is added and hooks are reported unavailable.
/// Without a `spool` the entry is still added, but no spool binding is set
/// and hooks are reported unavailable.
pub fn claude_launch_env(
    target: TerminalTarget,
    shipped: Option<&Path>,
    existing: Option<&OsStr>,
    spool: Option<&Path>,
) -> ClaudeLaunchEnv {
    if !CLAUDE_CODE.env_policy().covers(CLAUDE_CODE, target) {
        return ClaudeLaunchEnv::default();
    }
    let Some(shipped) = shipped else {
        return ClaudeLaunchEnv {
            env: Vec::new(),
            hooks_unavailable: true,
        };
    };
    let separator = claude_plugin_dirs_separator(PlatformFamily::current());
    let composed = compose_claude_plugin_dirs(existing, shipped, separator);
    // An unsafe shipped path composes to nothing. An empty plugin list is
    // never set: it would only hide the user's own.
    if composed.is_empty() {
        return ClaudeLaunchEnv {
            env: Vec::new(),
            hooks_unavailable: true,
        };
    }
    let mut env = vec![(OsString::from(CLAUDE_CODE_PLUGIN_DIRS_ENV), composed)];
    if let Some(spool) = spool {
        env.push((
            OsString::from(CLAUDE_SPOOL_ENV),
            spool.as_os_str().to_owned(),
        ));
    }
    ClaudeLaunchEnv {
        env,
        hooks_unavailable: spool.is_none(),
    }
}

/// The shipped claude-code-made package when it is a readable directory.
///
/// `None` when the install root is unknown or the package is missing or
/// unreadable, or its path cannot be written into the plugin list safely, in
/// which case no `CLAUDE_CODE_PLUGIN_DIRS` entry may point at it.
pub fn shipped_claude_plugin_dir(paths: &AppPaths) -> Option<PathBuf> {
    let dir = paths.plugin_package_dir("claude-code-made")?;
    // Only this host's list separator can split the entry: a Windows drive
    // colon is part of an ordinary `C:\...` path.
    let separator = claude_plugin_dirs_separator(PlatformFamily::current());
    if !claude_plugin_entry_is_safe(&dir, separator) {
        return None;
    }
    std::fs::read_dir(&dir).ok()?;
    Some(dir)
}

/// Entry separator of [`CLAUDE_CODE_PLUGIN_DIRS_ENV`]: `:` on POSIX, `;` on
/// Windows (candidate; unverified on a Windows host).
pub const fn claude_plugin_dirs_separator(family: PlatformFamily) -> char {
    match family {
        PlatformFamily::Linux | PlatformFamily::MacOs => ':',
        PlatformFamily::Windows => ';',
    }
}

/// Whether `path` may be written into the plugin list: absolute, and free of
/// the list separator, quotes, backtick, `$`, newlines and, on POSIX (a `:`
/// separator), backslash, so no consumer can split or expand it.
fn claude_plugin_entry_is_safe(path: &Path, separator: char) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && !text
            .chars()
            .any(|c| c == separator || matches!(c, '"' | '`' | '$' | '\n' | '\r'))
        && !(separator == ':' && text.contains('\\'))
}

/// Compose the [`CLAUDE_CODE_PLUGIN_DIRS_ENV`] value for a Made-started process.
///
/// Every existing entry is kept byte-for-byte in its original order and
/// Made's `shipped` entry is appended, unless an entry already equals it
/// (ignoring trailing slashes). An absent or empty `existing` value yields
/// just `shipped`, with no leading separator. A `shipped` path that is not
/// safe (see `claude_plugin_entry_is_safe`) adds nothing: the result is
/// `existing` unchanged, or empty. `separator` must be ASCII.
pub fn compose_claude_plugin_dirs(
    existing: Option<&OsStr>,
    shipped: &Path,
    separator: char,
) -> OsString {
    debug_assert!(separator.is_ascii());
    let safe = claude_plugin_entry_is_safe(shipped, separator);
    let shipped = shipped.as_os_str();
    let Some(existing) = existing.filter(|value| !value.is_empty()) else {
        return if safe {
            shipped.to_owned()
        } else {
            OsString::new()
        };
    };
    if !safe {
        return existing.to_owned();
    }
    let separator = separator as u8;
    let bytes = existing.as_encoded_bytes();
    // A `;` list is a Windows one: both slashes end an entry and paths compare
    // case-insensitively.
    let windows = separator == b';';
    let trimmed = |entry: &[u8]| {
        let end = entry
            .iter()
            .rposition(|byte| *byte != b'/' && !(windows && *byte == b'\\'))
            .map_or(0, |i| i + 1);
        let mut entry = entry[..end].to_vec();
        if windows {
            entry.make_ascii_lowercase();
        }
        entry
    };
    let wanted = trimmed(shipped.as_encoded_bytes());
    if bytes
        .split(|byte| *byte == separator)
        .any(|entry| trimmed(entry) == wanted)
    {
        return existing.to_owned();
    }
    let mut composed = existing.to_owned();
    if bytes.last() != Some(&separator) {
        composed.push(char::from(separator).to_string());
    }
    composed.push(shipped);
    composed
}
