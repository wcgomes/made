//! Read-only preview loader with size and binary guards (Phase 2, tasks
//! 2.7-2.8).
//!
//! The preview size limit is 2 MiB and binary detection checks the first 8 KiB
//! for a NUL byte. There is no write path or editor model in this module: every
//! operation takes `&self` and returns owned display data. Do not add a
//! write/save API here.
//!
//! Reads go through [`PathGuard`] and therefore reject traversal, symlink
//! escapes, and paths outside the active root. Files that change between the
//! guard check and the open are rechecked by size and content classification
//! through the opened handle.
//!
//! # Residual TOCTOU risk
//!
//! Canonicalization and `open` are separate filesystem operations. A same-user
//! process can replace the canonical path with a symlink or FIFO after the
//! guard check and before `File::open`. V1 accepts this residual risk: the
//! threat model explicitly excludes a same-user attacker, and closing the race
//! requires non-blocking opens plus opened-descriptor identity checks, which
//! would be OS-specific code owned by `made-platform`. The metadata check below
//! rejects a path that is *already* a FIFO or device without opening it, so
//! ordinary previews cannot hang on non-regular files; it does not make the
//! race atomic.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::paths::{PathError, PathGuard};
use crate::root_switch::{RootGeneration, StaleGeneration};

/// Maximum preview size, in bytes (2 MiB).
pub const PREVIEW_SIZE_LIMIT: u64 = 2 * 1024 * 1024;

/// Prefix inspected for NUL bytes when detecting binary files (8 KiB).
pub const BINARY_SNIFF_LEN: usize = 8 * 1024;

/// Safe classification of a preview result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    /// Valid UTF-8 text within the size limit.
    Text,
    /// A NUL byte was found in the sniff window.
    Binary,
    /// The file is larger than [`PREVIEW_SIZE_LIMIT`].
    TooLarge,
    /// The bytes are not valid UTF-8.
    InvalidUtf8,
    /// The file no longer exists.
    Missing,
    /// The file exists but could not be read.
    Unreadable,
    /// The path guard rejected the path.
    Forbidden,
}

impl PreviewKind {
    /// Returns `true` for renderable text.
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(self, Self::Text)
    }
}

/// Read-only preview content or a safe non-content message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// UTF-8 text content.
    Text {
        /// Canonical file path.
        path: PathBuf,
        /// File content.
        content: String,
        /// Content size in bytes.
        size: u64,
    },
    /// Binary file; raw content is intentionally absent.
    Binary {
        /// Canonical file path.
        path: PathBuf,
        /// File size in bytes.
        size: u64,
        /// Safe message for display.
        message: String,
    },
    /// File over the preview size limit; content was not read.
    TooLarge {
        /// File path.
        path: PathBuf,
        /// File size in bytes.
        size: u64,
        /// The configured limit in bytes.
        limit: u64,
        /// Safe message containing the size and limit.
        message: String,
    },
    /// File is not valid UTF-8; raw bytes are not exposed.
    InvalidUtf8 {
        /// Canonical file path.
        path: PathBuf,
        /// File size in bytes.
        size: u64,
        /// Safe message for display.
        message: String,
    },
    /// File was removed before it could be read.
    Missing {
        /// Requested or canonical path.
        path: PathBuf,
        /// Safe message for display.
        message: String,
    },
    /// File exists but could not be read.
    Unreadable {
        /// Requested or canonical path.
        path: PathBuf,
        /// Safe message for display.
        message: String,
    },
    /// Path guard rejected the target.
    Forbidden {
        /// Rejected path.
        path: PathBuf,
        /// Safe message for display.
        message: String,
    },
}

impl Preview {
    /// Returns the preview classification.
    #[must_use]
    pub const fn kind(&self) -> PreviewKind {
        match self {
            Self::Text { .. } => PreviewKind::Text,
            Self::Binary { .. } => PreviewKind::Binary,
            Self::TooLarge { .. } => PreviewKind::TooLarge,
            Self::InvalidUtf8 { .. } => PreviewKind::InvalidUtf8,
            Self::Missing { .. } => PreviewKind::Missing,
            Self::Unreadable { .. } => PreviewKind::Unreadable,
            Self::Forbidden { .. } => PreviewKind::Forbidden,
        }
    }

    /// Returns the path associated with the result.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Text { path, .. }
            | Self::Binary { path, .. }
            | Self::TooLarge { path, .. }
            | Self::InvalidUtf8 { path, .. }
            | Self::Missing { path, .. }
            | Self::Unreadable { path, .. }
            | Self::Forbidden { path, .. } => path,
        }
    }

    /// Returns text content only for [`Preview::Text`].
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text { content, .. } => Some(content),
            _ => None,
        }
    }

    /// Returns the file size when known.
    #[must_use]
    pub const fn size(&self) -> Option<u64> {
        match self {
            Self::Text { size, .. }
            | Self::Binary { size, .. }
            | Self::TooLarge { size, .. }
            | Self::InvalidUtf8 { size, .. } => Some(*size),
            Self::Missing { .. } | Self::Unreadable { .. } | Self::Forbidden { .. } => None,
        }
    }

    /// Returns the safe display message for non-text results.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Text { .. } => None,
            Self::Binary { message, .. }
            | Self::TooLarge { message, .. }
            | Self::InvalidUtf8 { message, .. }
            | Self::Missing { message, .. }
            | Self::Unreadable { message, .. }
            | Self::Forbidden { message, .. } => Some(message),
        }
    }

    /// Returns `true` when the result is renderable text.
    #[must_use]
    pub const fn is_text(&self) -> bool {
        self.kind().is_text()
    }
}

/// Read-only preview loader bound to one canonical root and generation.
///
/// The loader is cheap to clone and can be used from a background thread. It
/// exposes no write, save, or mutation method.
#[derive(Debug, Clone)]
pub struct PreviewLoader {
    guard: PathGuard,
    generation: RootGeneration,
}

impl PreviewLoader {
    /// Creates a loader for `root` at [`RootGeneration::INITIAL`].
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PathError> {
        Self::with_generation(root, RootGeneration::INITIAL)
    }

    /// Creates a loader for `root` at an explicit generation.
    pub fn with_generation(
        root: impl AsRef<Path>,
        generation: RootGeneration,
    ) -> Result<Self, PathError> {
        Ok(Self {
            guard: PathGuard::new(root)?,
            generation,
        })
    }

    /// Creates a loader from an existing guard.
    #[must_use]
    pub fn from_guard(guard: PathGuard, generation: RootGeneration) -> Self {
        Self { guard, generation }
    }

    /// Returns the canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.guard.root()
    }

    /// Returns the generation stamped onto loads.
    #[must_use]
    pub const fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Returns the path guard used for reads.
    #[must_use]
    pub fn path_guard(&self) -> &PathGuard {
        &self.guard
    }

    /// Loads a preview for the loader's current generation.
    pub fn load(&self, path: impl AsRef<Path>) -> Preview {
        self.load_inner(path.as_ref())
    }

    /// Loads a preview only when `generation` is still current.
    ///
    /// A background preview read can carry the generation it was started with;
    /// a result from an old root is rejected before any filesystem access.
    pub fn load_for(
        &self,
        path: impl AsRef<Path>,
        generation: RootGeneration,
    ) -> Result<Preview, StaleGeneration> {
        if !generation.is_current(self.generation) {
            return Err(StaleGeneration::new(self.generation, generation));
        }
        Ok(self.load_inner(path.as_ref()))
    }

    fn load_inner(&self, requested: &Path) -> Preview {
        let canonical = match self.guard.resolve_existing(requested) {
            Ok(path) => path,
            Err(error) => return preview_from_path_error(error),
        };

        // Reject an already-present non-regular file before opening so a FIFO or
        // device cannot block the preview worker waiting for a writer. This is
        // not race-free: the documented same-user TOCTOU residual applies.
        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(error) => return preview_from_io_error(&canonical, error),
        };
        if !metadata.is_file() {
            return Preview::Unreadable {
                path: canonical,
                message: "File is unreadable: not a regular file.".to_string(),
            };
        }
        let metadata_size = metadata.len();
        if metadata_size > PREVIEW_SIZE_LIMIT {
            return Preview::TooLarge {
                path: canonical,
                size: metadata_size,
                limit: PREVIEW_SIZE_LIMIT,
                message: too_large_message(metadata_size, PREVIEW_SIZE_LIMIT),
            };
        }

        let mut file = match File::open(&canonical) {
            Ok(file) => file,
            Err(error) => return preview_from_io_error(&canonical, error),
        };

        // Recheck through the open handle so a path that changed between the
        // metadata call and the open is still classified from the opened file.
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return preview_from_io_error(&canonical, error),
        };
        if !metadata.is_file() {
            return Preview::Unreadable {
                path: canonical,
                message: "File is unreadable: not a regular file.".to_string(),
            };
        }

        let metadata_size = metadata.len();
        if metadata_size > PREVIEW_SIZE_LIMIT {
            return Preview::TooLarge {
                path: canonical,
                size: metadata_size,
                limit: PREVIEW_SIZE_LIMIT,
                message: too_large_message(metadata_size, PREVIEW_SIZE_LIMIT),
            };
        }

        let mut content = Vec::with_capacity(metadata_size as usize);
        let mut prefix = [0_u8; BINARY_SNIFF_LEN];
        let prefix_len = match read_up_to(&mut file, &mut prefix) {
            Ok(read) => read,
            Err(error) => return preview_from_io_error(&canonical, error),
        };
        if prefix[..prefix_len].contains(&0) {
            return Preview::Binary {
                path: canonical,
                size: metadata_size,
                message: binary_message(metadata_size),
            };
        }
        content.extend_from_slice(&prefix[..prefix_len]);

        let remaining = (PREVIEW_SIZE_LIMIT + 1).saturating_sub(content.len() as u64);
        if let Err(error) = file.take(remaining).read_to_end(&mut content) {
            return preview_from_io_error(&canonical, error);
        }
        if content.len() as u64 > PREVIEW_SIZE_LIMIT {
            let size = metadata_size.max(content.len() as u64);
            return Preview::TooLarge {
                path: canonical,
                size,
                limit: PREVIEW_SIZE_LIMIT,
                message: too_large_message(size, PREVIEW_SIZE_LIMIT),
            };
        }

        let size = content.len() as u64;
        match String::from_utf8(content) {
            Ok(content) => Preview::Text {
                path: canonical,
                content,
                size,
            },
            Err(_) => Preview::InvalidUtf8 {
                path: canonical,
                size,
                message: invalid_utf8_message(size),
            },
        }
    }
}

fn read_up_to(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn preview_from_path_error(error: PathError) -> Preview {
    match error {
        PathError::OutsideRoot { path } => Preview::Forbidden {
            path,
            message: "Path is outside the workspace root and cannot be previewed.".to_string(),
        },
        PathError::NotFound { path } => Preview::Missing {
            path,
            message: "File is missing or no longer available.".to_string(),
        },
        PathError::Unreadable { path, source } => Preview::Unreadable {
            path,
            message: format!("File is unreadable: {source}"),
        },
        PathError::InvalidRoot { path, source } => Preview::Unreadable {
            path,
            message: format!("File is unreadable: {source}"),
        },
    }
}

fn preview_from_io_error(path: &Path, error: io::Error) -> Preview {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Preview::Missing {
            path: path.to_path_buf(),
            message: "File is missing or no longer available.".to_string(),
        },
        _ => Preview::Unreadable {
            path: path.to_path_buf(),
            message: format!("File is unreadable: {error}"),
        },
    }
}

fn too_large_message(size: u64, limit: u64) -> String {
    format!("File is {size} bytes; previews are limited to {limit} bytes (2 MiB).")
}

fn binary_message(size: u64) -> String {
    format!("Binary file ({size} bytes) is not rendered in the preview.")
}

fn invalid_utf8_message(size: u64) -> String {
    format!("File is not valid UTF-8 text ({size} bytes); the preview is unavailable.")
}
