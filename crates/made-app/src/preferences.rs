//! Persistent user preferences (REL-10).
//!
//! The only preference is whether automatic update checks run. It lives in
//! private, user-writable app data beside the SSH server registry, never in
//! the install directory, so installing or updating Made never resets it. A
//! missing file means a new install (checks on); an unreadable or corrupt file
//! is treated as on and reported once through [`UpdatePreference::diagnostic`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name inside the private app-data directory.
pub const PREFERENCES_FILE: &str = "preferences.json";
const MAX_PREFERENCES_BYTES: usize = 4096;

#[derive(Serialize, Deserialize)]
struct Document {
    schema: u32,
    automatic_update_checks: bool,
}

/// The loaded update preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePreference {
    /// Whether automatic update checks run at launch and every 24 hours.
    pub automatic_checks: bool,
    /// Why the stored choice could not be read, when it was treated as on.
    pub diagnostic: Option<String>,
}

/// Reads and writes the preference file in one private directory.
#[derive(Debug, Clone)]
pub struct PreferenceStore {
    directory: PathBuf,
}

impl PreferenceStore {
    /// A store in `directory` (the private app-data directory).
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// The store in the host app-data directory, beside the SSH registry.
    pub fn from_app_paths() -> Option<Self> {
        use made_platform::Platform;
        made_platform::host_platform()
            .app_paths()
            .ok()
            .map(|paths| Self::new(paths.data_dir))
    }

    /// Directory holding the preference file.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Load the update preference; never fails.
    pub fn load(&self) -> UpdatePreference {
        let on = |diagnostic: Option<&str>| UpdatePreference {
            automatic_checks: true,
            diagnostic: diagnostic.map(str::to_owned),
        };
        match made_platform::read_private_metadata(
            &self.directory,
            PREFERENCES_FILE,
            MAX_PREFERENCES_BYTES,
        ) {
            Ok(None) => on(None),
            Ok(Some(bytes)) => match serde_json::from_slice::<Document>(&bytes) {
                Ok(document) if document.schema == 1 => UpdatePreference {
                    automatic_checks: document.automatic_update_checks,
                    diagnostic: None,
                },
                _ => on(Some(
                    "the stored update preference is unreadable; automatic checks are on",
                )),
            },
            Err(_) => on(Some(
                "the stored update preference cannot be read safely; automatic checks are on",
            )),
        }
    }

    /// Store the automatic-check choice.
    pub fn set_automatic_checks(&self, enabled: bool) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&Document {
            schema: 1,
            automatic_update_checks: enabled,
        })
        .map_err(|error| error.to_string())?;
        made_platform::write_private_metadata_atomic(
            &self.directory,
            PREFERENCES_FILE,
            &bytes,
            MAX_PREFERENCES_BYTES,
        )
        .map_err(|_| "the update preference could not be saved".to_owned())
    }
}
