//! Launch-time user-config hash verification.
//!
//! Made captures a SHA-256 fingerprint of the user's OpenCode configuration
//! before launch and verifies it after the session. The result is evidence for
//! diagnostics and tests; it is never used to rewrite or repair the file.
//!
//! A missing file is a valid, explicitly represented state. Missing before and
//! missing after is unchanged; appearing or disappearing is a change.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::security::hex_lower;

/// Algorithm name reported with hash evidence.
pub const HASH_ALGORITHM: &str = "sha256";

/// Whether the configuration file existed at capture time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFileState {
    /// The file existed and was hashed.
    Present,
    /// The file did not exist.
    Missing,
}

/// A captured configuration fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSnapshot {
    path: PathBuf,
    state: ConfigFileState,
    sha256: Option<[u8; 32]>,
}

impl ConfigSnapshot {
    /// Capture the current content hash of `path`.
    ///
    /// A missing file yields [`ConfigFileState::Missing`] and no hash. Any
    /// other I/O failure is returned with the path in the error message.
    pub fn capture(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let sha256 = hash_file(&path)?;
        let state = if sha256.is_some() {
            ConfigFileState::Present
        } else {
            ConfigFileState::Missing
        };
        Ok(Self {
            path,
            state,
            sha256,
        })
    }

    /// The captured path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file existed at capture time.
    pub fn state(&self) -> ConfigFileState {
        self.state
    }

    /// Raw SHA-256 bytes, or `None` when the file was missing.
    pub fn sha256(&self) -> Option<[u8; 32]> {
        self.sha256
    }

    /// Lowercase hexadecimal SHA-256, or `None` when the file was missing.
    pub fn sha256_hex(&self) -> Option<String> {
        self.sha256.as_ref().map(|digest| hex_lower(digest))
    }

    /// Re-hash the path and compare it with this capture.
    pub fn verify(&self) -> io::Result<ConfigVerification> {
        let after = Self::capture(self.path.clone())?;
        if self.state == after.state && self.sha256 == after.sha256 {
            Ok(ConfigVerification::Unchanged {
                state: after.state,
                sha256: after.sha256_hex(),
            })
        } else {
            Ok(ConfigVerification::Changed {
                before: self.state,
                after: after.state,
                before_sha256: self.sha256_hex(),
                after_sha256: after.sha256_hex(),
            })
        }
    }
}

/// Evidence produced by comparing a capture with the current file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigVerification {
    /// State and content hash are identical.
    Unchanged {
        /// State observed both before and after.
        state: ConfigFileState,
        /// Lowercase SHA-256, absent when the file was missing.
        sha256: Option<String>,
    },
    /// The file appeared, disappeared, or changed content.
    Changed {
        /// State before the session.
        before: ConfigFileState,
        /// State after the session.
        after: ConfigFileState,
        /// Hash before the session, when present.
        before_sha256: Option<String>,
        /// Hash after the session, when present.
        after_sha256: Option<String>,
    },
}

impl ConfigVerification {
    /// Whether the configuration was byte-for-byte identical.
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged { .. })
    }

    /// The hash algorithm used by this evidence.
    pub fn algorithm(&self) -> &'static str {
        HASH_ALGORITHM
    }
}

/// A before/after guard around a launch.
///
/// ```ignore
/// let guard = ConfigGuard::before_session(&cli_json)?;
/// // launch OpenCode with the composed in-memory configuration
/// let evidence = guard.after_session()?;
/// assert!(evidence.is_unchanged());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigGuard {
    before: ConfigSnapshot,
}

impl ConfigGuard {
    /// Capture the user configuration before a session.
    pub fn before_session(path: impl Into<PathBuf>) -> io::Result<Self> {
        Ok(Self {
            before: ConfigSnapshot::capture(path)?,
        })
    }

    /// The captured "before" snapshot.
    pub fn before(&self) -> &ConfigSnapshot {
        &self.before
    }

    /// Verify the user configuration after the session.
    pub fn after_session(&self) -> io::Result<ConfigVerification> {
        self.before.verify()
    }
}

fn hash_file(path: &Path) -> io::Result<Option<[u8; 32]>> {
    match fs::read(path) {
        Ok(content) => {
            let digest = Sha256::digest(&content);
            let mut bytes = [0_u8; 32];
            bytes.copy_from_slice(&digest);
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("could not read {}: {error}", path.display()),
        )),
    }
}
