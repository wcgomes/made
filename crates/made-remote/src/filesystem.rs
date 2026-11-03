//! Server-side filesystem authority. All content reads reuse the local guards.
use crate::protocol::*;
use made_explorer::{
    paths::PathGuard,
    preview::{Preview, PreviewLoader},
    root_switch::RootGeneration,
    tree::{DirListing, EntryKind as TreeKind},
    watch::FsWatcher,
};
use std::path::{Path, PathBuf};

/// These lossless native conversions are platform capabilities, not display conversions.
pub fn native(path: &RemotePath) -> Result<PathBuf, ErrorCode> {
    made_platform::remote_path_from_bytes(path.as_bytes()).map_err(|_| ErrorCode::Unsupported)
}
pub fn remote(path: &Path) -> Result<RemotePath, ErrorCode> {
    RemotePath::new(made_platform::remote_path_bytes(path).map_err(|_| ErrorCode::Unsupported)?)
        .map_err(|_| ErrorCode::InvalidRequest)
}

pub struct Filesystem {
    guard: PathGuard,
    generation: u64,
    watcher: Option<(RemotePath, FsWatcher)>,
}
impl Filesystem {
    pub fn new(root: &RemotePath, generation: u64) -> Result<Self, ErrorCode> {
        if generation == 0 {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(Self {
            guard: PathGuard::new(native(root)?).map_err(|_| ErrorCode::Unavailable)?,
            generation,
            watcher: None,
        })
    }
    pub fn root(&self) -> &Path {
        self.guard.root()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn check(&self, generation: u64) -> Result<(), ErrorCode> {
        if generation == self.generation {
            Ok(())
        } else {
            Err(ErrorCode::Stale)
        }
    }
    /// Root transitions require an authenticated owning-tab selection/report.
    /// No file read API grants a root implicitly.
    pub fn select(&mut self, path: &RemotePath) -> Result<Operation, ErrorCode> {
        let guard = PathGuard::new(native(path)?).map_err(|_| ErrorCode::Unavailable)?;
        let next = self.generation.checked_add(1).ok_or(ErrorCode::Limit)?;
        let canonical = remote(guard.root())?;
        self.guard = guard;
        self.generation = next;
        self.watcher = None;
        Ok(Operation::RootResult {
            requested: path.clone(),
            path: canonical,
            generation: next,
        })
    }
    pub fn validate_root(&self, path: &RemotePath) -> Result<Operation, ErrorCode> {
        let guard = PathGuard::new(native(path)?).map_err(|_| ErrorCode::Unavailable)?;
        Ok(Operation::RootResult {
            requested: path.clone(),
            path: remote(guard.root())?,
            generation: self.generation,
        })
    }
    pub fn list(&self, path: &RemotePath) -> Result<Vec<Operation>, ErrorCode> {
        let listing = DirListing::load(
            &self.guard,
            native(path)?,
            RootGeneration::new(self.generation),
        )
        .map_err(|_| ErrorCode::Unauthorized)?;
        if listing.error.is_some() {
            return Err(ErrorCode::Unavailable);
        }
        let mut entries = Vec::with_capacity(listing.entries.len().min(MAX_DIRECTORY_ENTRIES));
        for entry in listing.entries {
            // Synthetic truncation/error rows do not have a usable wire path.
            if entry.kind == TreeKind::Error {
                continue;
            }
            let name = entry.path.file_name().ok_or(ErrorCode::InvalidRequest)?;
            // Preserve the requested spelling (a guarded in-root symlink parent
            // may canonicalize differently), never reconstruct from display name.
            let exact = native(path)?.join(name);
            entries.push(DirectoryEntry {
                path: remote(&exact)?,
                kind: match entry.kind {
                    TreeKind::Directory => EntryKind::Directory,
                    TreeKind::File => EntryKind::File,
                    TreeKind::Symlink => EntryKind::Symlink,
                    TreeKind::Other | TreeKind::Error => EntryKind::Other,
                },
            });
        }
        // Worst-case byte paths expand to JSON arrays; 128 entries alone is not
        // a safe frame bound. Eight max-sized paths fit the 256 KiB frame cap.
        let batches: Vec<_> = entries.chunks(8).collect();
        if batches.is_empty() {
            return Ok(vec![Operation::Listed {
                path: path.clone(),
                entries: vec![],
                truncated: listing.truncated,
                done: true,
            }]);
        }
        Ok(batches
            .iter()
            .enumerate()
            .map(|(i, batch)| Operation::Listed {
                path: path.clone(),
                entries: batch.to_vec(),
                truncated: listing.truncated,
                done: i + 1 == batches.len(),
            })
            .collect())
    }
    pub fn preview(&self, path: &RemotePath) -> Result<Vec<Operation>, ErrorCode> {
        let native = native(path)?;
        // Keep missing classification in PreviewLoader, but never open a special entry.
        if let Ok(canonical) = self.guard.resolve_existing(&native) {
            if std::fs::metadata(canonical).is_ok_and(|m| !m.is_file()) {
                return Ok(vec![preview_result(
                    path,
                    PreviewKind::NotRegular,
                    0,
                    &[],
                    true,
                )?]);
            }
        }
        let preview =
            PreviewLoader::from_guard(self.guard.clone(), RootGeneration::new(self.generation))
                .load(native);
        let kind = match &preview {
            Preview::Text { .. } => PreviewKind::Text,
            Preview::Binary { .. } => PreviewKind::Binary,
            Preview::TooLarge { .. } => PreviewKind::TooLarge,
            Preview::InvalidUtf8 { .. } => PreviewKind::InvalidUtf8,
            Preview::Missing { .. } => PreviewKind::Missing,
            Preview::Unreadable { .. } => PreviewKind::Unreadable,
            Preview::Forbidden { .. } => return Err(ErrorCode::Unauthorized),
        };
        let text = preview.text().unwrap_or("");
        let mut result = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = (start + MAX_CHUNK_BYTES).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            result.push(preview_result(
                path,
                kind,
                start as u64,
                &text.as_bytes()[start..end],
                end == text.len(),
            )?);
            start = end;
        }
        if result.is_empty() {
            result.push(preview_result(path, kind, 0, &[], true)?);
        }
        Ok(result)
    }
    pub fn watch(&mut self, path: &RemotePath) -> Result<(), ErrorCode> {
        let canonical = self
            .guard
            .resolve_existing(native(path)?)
            .map_err(|_| ErrorCode::Unauthorized)?;
        let watcher = FsWatcher::new(canonical, RootGeneration::new(self.generation))
            .map_err(|_| ErrorCode::Unavailable)?;
        self.watcher = Some((path.clone(), watcher));
        Ok(())
    }
    pub fn cancel_watch(&mut self) {
        self.watcher = None;
    }
    /// Coalesce all changes to the subscription path: clients resnapshot visible
    /// listings/preview/Git instead of trusting deleted or renamed paths as grants.
    pub fn poll_change(&mut self) -> Option<Operation> {
        let (path, watcher) = self.watcher.as_mut()?;
        let mut changed = false;
        for _ in 0..4096 {
            if watcher.poll().is_some() {
                changed = true;
            } else {
                break;
            }
        }
        changed.then(|| Operation::Changed { path: path.clone() })
    }
}
fn preview_result(
    path: &RemotePath,
    kind: PreviewKind,
    offset: u64,
    bytes: &[u8],
    done: bool,
) -> Result<Operation, ErrorCode> {
    Ok(Operation::PreviewResult {
        path: path.clone(),
        kind,
        offset,
        data: Chunk::new(bytes.to_vec()).map_err(|_| ErrorCode::Limit)?,
        done,
    })
}
