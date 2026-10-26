//! Root-constrained path resolver (Phase 2, tasks 2.5-2.6).
//!
//! Canonicalizes the active root and requested target, resolves symlinks, and
//! rejects traversal or escapes outside the root or explicitly granted roots.
//!
//! The guard never writes. `resolve_for_create` exists so a future user action
//! can validate a path before a file is created; it still canonicalizes the
//! deepest existing ancestor and rejects symlink escapes.
//!
//! V1 exposes no "open outside workspace" UI, so the only default root is the
//! active workspace root. [`PathGuard::add_granted_root`] is the explicit
//! user-grant API reserved for a later action; adding a root never weakens the
//! containment checks for other paths.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Typed path resolution failure.
#[derive(Debug)]
pub enum PathError {
    /// The canonical target is not inside the active root or a granted root.
    OutsideRoot {
        /// Path that was requested.
        path: PathBuf,
    },
    /// The target does not exist (or a path component is not a directory).
    NotFound {
        /// Path that was requested.
        path: PathBuf,
    },
    /// The target or one of its components could not be read.
    Unreadable {
        /// Path that was requested.
        path: PathBuf,
        /// Underlying filesystem error.
        source: io::Error,
    },
    /// The configured root is missing, unreadable, or not a directory.
    InvalidRoot {
        /// Root that was requested.
        path: PathBuf,
        /// Underlying filesystem error.
        source: io::Error,
    },
}

impl PathError {
    /// Returns the requested path carried by the error.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::OutsideRoot { path }
            | Self::NotFound { path }
            | Self::Unreadable { path, .. }
            | Self::InvalidRoot { path, .. } => path,
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideRoot { path } => {
                write!(f, "path is outside the workspace root: {}", path.display())
            }
            Self::NotFound { path } => write!(f, "path does not exist: {}", path.display()),
            Self::Unreadable { path, source } => {
                write!(f, "path is unreadable ({}): {}", source, path.display())
            }
            Self::InvalidRoot { path, source } => {
                write!(f, "invalid workspace root ({}): {}", source, path.display())
            }
        }
    }
}

impl Error for PathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } | Self::InvalidRoot { source, .. } => Some(source),
            Self::OutsideRoot { .. } | Self::NotFound { .. } => None,
        }
    }
}

/// Canonical-path allowlist for one or more workspace roots.
///
/// `PathGuard::new` canonicalizes the root immediately. Every resolution
/// canonicalizes the requested target, which resolves symlinks; the resulting
/// path is then compared component-wise against the allowed roots. A lexical
/// `..` component is rejected even when it would resolve back inside the root.
#[derive(Debug, Clone)]
pub struct PathGuard {
    root: PathBuf,
    granted_roots: Vec<PathBuf>,
}

impl PathGuard {
    /// Creates a guard for `root`, canonicalizing and validating the root.
    ///
    /// Construction opens the canonical directory once with `read_dir` so an
    /// unreadable root is rejected immediately instead of producing a tree of
    /// unreadable rows later.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::InvalidRoot`] when the root is missing, unreadable,
    /// or is not a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PathError> {
        let requested = root.as_ref().to_path_buf();
        let root = canonicalize_root(&requested)?;
        Ok(Self {
            root,
            granted_roots: Vec::new(),
        })
    }

    /// Returns the canonical active root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the canonical roots explicitly granted by the user.
    #[must_use]
    pub fn granted_roots(&self) -> &[PathBuf] {
        &self.granted_roots
    }

    /// Returns every allowed root, starting with the active root.
    pub fn allowed_roots(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.root.as_path()).chain(self.granted_roots.iter().map(PathBuf::as_path))
    }

    /// Grants an explicit additional root for later resolutions.
    ///
    /// V1 has no UI entry point for this; it exists so a later user action can
    /// grant a folder without weakening the guard. The added root is
    /// canonicalized and must be an existing, readable directory.
    pub fn add_granted_root(&mut self, root: impl AsRef<Path>) -> Result<PathBuf, PathError> {
        let requested = root.as_ref().to_path_buf();
        let canonical = canonicalize_root(&requested)?;
        if canonical != self.root && !self.granted_roots.contains(&canonical) {
            self.granted_roots.push(canonical.clone());
        }
        Ok(canonical)
    }

    /// Returns `true` when `path` is absolute, has no `..` component, and is
    /// component-wise inside an allowed root.
    ///
    /// `path` is expected to be canonical. Use [`PathGuard::resolve_existing`]
    /// or [`PathGuard::resolve_for_create`] when validation is required.
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        if !path.is_absolute()
            || path
                .components()
                .any(|component| component == Component::ParentDir)
        {
            return false;
        }
        self.allowed_roots()
            .any(|root| path == root || path.starts_with(root))
    }

    /// Resolves an existing path and enforces the allowlist.
    ///
    /// Relative paths are interpreted relative to the active root. The target
    /// and every symlink are resolved before the containment check.
    pub fn resolve_existing(&self, path: impl AsRef<Path>) -> Result<PathBuf, PathError> {
        let requested = self.absolute_target(path.as_ref());
        reject_parent_components(&requested)?;

        let canonical =
            fs::canonicalize(&requested).map_err(|error| target_error(&requested, error))?;
        if !self.contains(&canonical) {
            return Err(PathError::OutsideRoot { path: requested });
        }
        Ok(canonical)
    }

    /// Resolves a path that may not exist yet, such as a future file creation.
    ///
    /// The deepest existing ancestor is canonicalized and checked; missing
    /// trailing components are appended lexically. Existing targets go through
    /// [`PathGuard::resolve_existing`]. `..` components are always rejected, and
    /// a symlinked ancestor that resolves outside the allowlist is rejected.
    pub fn resolve_for_create(&self, path: impl AsRef<Path>) -> Result<PathBuf, PathError> {
        let requested = self.absolute_target(path.as_ref());
        reject_parent_components(&requested)?;

        match fs::symlink_metadata(&requested) {
            Ok(_) => return self.resolve_existing(&requested),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                return Err(PathError::NotFound { path: requested });
            }
            Err(error) => {
                return Err(PathError::Unreadable {
                    path: requested,
                    source: error,
                });
            }
        }

        let mut ancestor = requested.as_path();
        let mut missing: Vec<OsString> = Vec::new();
        loop {
            match fs::symlink_metadata(ancestor) {
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let Some(name) = ancestor.file_name() else {
                        return Err(PathError::OutsideRoot { path: requested });
                    };
                    missing.push(name.to_os_string());
                    let Some(parent) = ancestor.parent() else {
                        return Err(PathError::OutsideRoot { path: requested });
                    };
                    ancestor = parent;
                }
                Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                    return Err(PathError::NotFound { path: requested });
                }
                Err(error) => {
                    return Err(PathError::Unreadable {
                        path: requested,
                        source: error,
                    });
                }
            }
        }

        let canonical_ancestor =
            fs::canonicalize(ancestor).map_err(|error| target_error(&requested, error))?;
        if !self.contains(&canonical_ancestor) {
            return Err(PathError::OutsideRoot { path: requested });
        }

        let mut resolved = canonical_ancestor;
        for name in missing.iter().rev() {
            resolved.push(name);
        }
        if !self.contains(&resolved) {
            return Err(PathError::OutsideRoot { path: requested });
        }
        Ok(resolved)
    }

    fn absolute_target(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

fn canonicalize_root(requested: &Path) -> Result<PathBuf, PathError> {
    let canonical = fs::canonicalize(requested).map_err(|source| PathError::InvalidRoot {
        path: requested.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| PathError::InvalidRoot {
        path: requested.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(PathError::InvalidRoot {
            path: requested.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "root is not a directory"),
        });
    }
    // `metadata` checks the type but not whether the directory can be listed.
    // Opening the directory here makes the "unreadable root" contract real;
    // the iterator is dropped immediately and no entries are read.
    fs::read_dir(&canonical).map_err(|source| PathError::InvalidRoot {
        path: requested.to_path_buf(),
        source,
    })?;
    Ok(canonical)
}

fn reject_parent_components(path: &Path) -> Result<(), PathError> {
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(PathError::OutsideRoot {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn target_error(path: &Path, error: io::Error) -> PathError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => PathError::NotFound {
            path: path.to_path_buf(),
        },
        _ => PathError::Unreadable {
            path: path.to_path_buf(),
            source: error,
        },
    }
}
