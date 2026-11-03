//! Explicit, ordered PNG uploads. Completed images are not tab-owned resources.
use crate::{filesystem::remote, protocol::*};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const RETENTION: Duration = Duration::from_secs(7 * 24 * 3600);
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
struct Pending {
    progress: UploadProgress,
    path: PathBuf,
    file: File,
    prefix: Vec<u8>,
    _lock: File,
    lock_path: PathBuf,
}
pub struct Uploads {
    directory: PathBuf,
    pending: HashMap<u64, Pending>,
}
impl Uploads {
    pub fn new(directory: PathBuf) -> Result<Self, ErrorCode> {
        let location = remote(&directory)?;
        if location
            .to_utf8()
            .map_err(|_| ErrorCode::InvalidRequest)?
            .chars()
            .any(char::is_control)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        made_platform::ensure_private_directory(&directory).map_err(|_| ErrorCode::Io)?;
        let this = Self {
            directory,
            pending: HashMap::new(),
        };
        // Startup cleanup is best effort; successful uploads do not depend on it.
        let _ = this.cleanup(None, SystemTime::now());
        Ok(this)
    }
    pub fn begin(&mut self, id: u64, total: u64) -> Result<(), ErrorCode> {
        if id == 0 || self.pending.contains_key(&id) || self.pending.len() >= MAX_IN_FLIGHT {
            return Err(ErrorCode::Limit);
        }
        let progress = UploadProgress::new(total).map_err(|_| ErrorCode::Limit)?;
        let name = random_name()?;
        let path = self.directory.join(format!("clipboard-{name}.partial"));
        let lock_path = path.with_extension("partial.lock");
        let lock = made_platform::lock_private_file(&lock_path, Duration::from_secs(1))
            .map_err(|_| ErrorCode::Io)?;
        let file =
            made_platform::create_private_file_new(&path, false).map_err(|_| ErrorCode::Io)?;
        self.pending.insert(
            id,
            Pending {
                progress,
                path,
                file,
                prefix: Vec::with_capacity(24),
                _lock: lock,
                lock_path,
            },
        );
        Ok(())
    }
    pub fn chunk(&mut self, id: u64, offset: u64, data: &Chunk) -> Result<(), ErrorCode> {
        let pending = self.pending.get_mut(&id).ok_or(ErrorCode::InvalidRequest)?;
        pending
            .progress
            .append(offset, data)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let n = (24 - pending.prefix.len()).min(data.as_bytes().len());
        pending.prefix.extend_from_slice(&data.as_bytes()[..n]);
        if pending.file.write_all(data.as_bytes()).is_err() {
            self.abort(id);
            return Err(ErrorCode::Io);
        }
        Ok(())
    }
    pub fn commit(&mut self, id: u64) -> Result<RemotePath, ErrorCode> {
        let pending = self.pending.remove(&id).ok_or(ErrorCode::InvalidRequest)?;
        let result = (|| {
            pending
                .progress
                .commit()
                .map_err(|_| ErrorCode::InvalidRequest)?;
            validate_png_header(&pending.prefix)?;
            pending.file.sync_all().map_err(|_| ErrorCode::Io)?;
            validate_png_file(&pending.path)?;
            let path = pending.path.with_extension("png");
            // Atomic no-replace promotion; a pre-existing entry is never overwritten.
            fs::hard_link(&pending.path, &path).map_err(|_| ErrorCode::Io)?;
            let remote = remote(&path)?;
            remote.to_utf8().map_err(|_| ErrorCode::InvalidRequest)?;
            let _ = fs::remove_file(&pending.path);
            let _ = self.cleanup(Some(&path), SystemTime::now());
            Ok(remote)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&pending.path);
        }
        let _ = fs::remove_file(&pending.lock_path);
        result
    }
    pub fn abort(&mut self, id: u64) {
        if let Some(pending) = self.pending.remove(&id) {
            let _ = fs::remove_file(pending.path);
            let _ = fs::remove_file(pending.lock_path);
        }
    }
    pub fn pending_abort_all(&mut self) {
        let ids: Vec<_> = self.pending.keys().copied().collect();
        for id in ids {
            self.abort(id);
        }
    }
    /// Serializes cross-tab retention using a platform file lock. Only generated
    /// regular filenames qualify; symlinks and unrelated entries are untouched.
    pub fn cleanup(&self, current: Option<&Path>, now: SystemTime) -> Result<(), ErrorCode> {
        let _lock = made_platform::lock_private_file(
            &self.directory.join("retention.lock"),
            Duration::from_secs(1),
        )
        .map_err(|_| ErrorCode::Io)?;
        let mut images = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|_| ErrorCode::Io)? {
            let entry = entry.map_err(|_| ErrorCode::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(token) = name.strip_prefix("clipboard-") else {
                continue;
            };
            let Some((token, extension)) = token.rsplit_once('.') else {
                continue;
            };
            if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| ErrorCode::Io)?;
            if !metadata.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if current == Some(path.as_path()) {
                continue;
            }
            let modified = metadata.modified().map_err(|_| ErrorCode::Io)?;
            // A live sibling holds this lock for its entire upload. Crash
            // remnants can be swept immediately, without touching live uploads.
            if extension == "partial" {
                let lock_path = path.with_extension("partial.lock");
                if let Ok(_lock) = made_platform::lock_private_file(&lock_path, Duration::ZERO) {
                    let _ = fs::remove_file(path);
                    let _ = fs::remove_file(lock_path);
                }
                continue;
            }
            if extension != "png" {
                continue;
            }
            if now.duration_since(modified).unwrap_or_default() > RETENTION
                && fs::remove_file(&path).is_ok()
            {
                continue;
            }
            images.push((modified, path));
        }
        images.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let excess = images
            .len()
            .saturating_add(usize::from(current.is_some()))
            .saturating_sub(64);
        for (_, path) in images.into_iter().take(excess) {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }
}
impl Drop for Uploads {
    fn drop(&mut self) {
        for (_, pending) in self.pending.drain() {
            let _ = fs::remove_file(pending.path);
            let _ = fs::remove_file(pending.lock_path);
        }
    }
}
fn random_name() -> Result<String, ErrorCode> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| ErrorCode::Io)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn validate_png_header(bytes: &[u8]) -> Result<(), ErrorCode> {
    if bytes.len() < 24 || &bytes[..8] != PNG || &bytes[12..16] != b"IHDR" {
        return Err(ErrorCode::InvalidRequest);
    }
    let width = u32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| ErrorCode::InvalidRequest)?,
    );
    let height = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| ErrorCode::InvalidRequest)?,
    );
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) * 4 > MAX_UPLOAD_BYTES
    {
        return Err(ErrorCode::Limit);
    }
    Ok(())
}
fn validate_png_file(path: &Path) -> Result<(), ErrorCode> {
    let mut file = File::open(path).map_err(|_| ErrorCode::Io)?;
    let mut signature = [0; 8];
    file.read_exact(&mut signature)
        .map_err(|_| ErrorCode::InvalidRequest)?;
    if signature != PNG {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut table = [0u32; 256];
    for (i, value) in table.iter_mut().enumerate() {
        let mut crc = i as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb88320
            };
        }
        *value = crc;
    }
    let mut first = true;
    let mut image = false;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let mut header = [0; 8];
        file.read_exact(&mut header)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let length = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| ErrorCode::InvalidRequest)?,
        );
        let kind = &header[4..];
        if u64::from(length) > MAX_UPLOAD_BYTES
            || (first && (kind != b"IHDR" || length != 13))
            || (!first && kind == b"IHDR")
            || (kind == b"IEND" && length != 0)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        first = false;
        let mut crc = u32::MAX;
        for b in kind {
            crc = table[((crc ^ u32::from(*b)) & 255) as usize] ^ (crc >> 8);
        }
        let mut remaining = length as usize;
        while remaining > 0 {
            let count = remaining.min(buffer.len());
            file.read_exact(&mut buffer[..count])
                .map_err(|_| ErrorCode::InvalidRequest)?;
            for b in &buffer[..count] {
                crc = table[((crc ^ u32::from(*b)) & 255) as usize] ^ (crc >> 8);
            }
            remaining -= count;
        }
        let mut expected = [0; 4];
        file.read_exact(&mut expected)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        if !crc != u32::from_be_bytes(expected) {
            return Err(ErrorCode::InvalidRequest);
        }
        if kind == b"IDAT" {
            image = true;
        }
        if kind == b"IEND" {
            if !image || file.read(&mut buffer[..1]).map_err(|_| ErrorCode::Io)? != 0 {
                return Err(ErrorCode::InvalidRequest);
            }
            return Ok(());
        }
    }
}
