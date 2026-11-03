//! Request-scoped OpenSSH askpass IPC. No TCP, terminal or persistent answers.
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

pub const MAX_PROMPT_BYTES: usize = 8192;
pub const MAX_ANSWER_BYTES: usize = 8192;

/// File name of the packaged askpass helper beside the application executable.
///
/// Windows packages `made-askpass.exe`. Every other host packages `made-askpass`.
pub const fn askpass_file_name() -> &'static str {
    if cfg!(windows) {
        "made-askpass.exe"
    } else {
        "made-askpass"
    }
}

/// Intentionally neither Clone nor printable. Ownership transfers once.
pub struct PromptAnswer(Vec<u8>);
impl PromptAnswer {
    pub fn new(bytes: Vec<u8>) -> io::Result<Self> {
        if bytes.len() > MAX_ANSWER_BYTES || bytes.iter().any(|b| matches!(b, 0 | b'\n' | b'\r')) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSH prompt answer",
            ));
        }
        Ok(Self(bytes))
    }
}
impl Drop for PromptAnswer {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
impl fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PromptAnswer(<redacted>)")
    }
}

/// Display only in the owning private authentication card, never diagnostics.
pub struct PromptRequest {
    pub id: u64,
    pub text: String,
    pub expires_at: Instant,
}
impl fmt::Debug for PromptRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PromptRequest {{ id: {}, text: <redacted> }}", self.id)
    }
}
struct Reply {
    id: u64,
    answer: Option<PromptAnswer>,
}

pub struct AskpassBroker {
    directory: PathBuf,
    socket: PathBuf,
    requests: Receiver<PromptRequest>,
    replies: SyncSender<Reply>,
    cancelled: Arc<AtomicBool>,
    outstanding: Option<(u64, Instant)>,
    /// Non-owning interrupt token. The serve thread owns the pipe handle.
    #[cfg(windows)]
    pipe: windows_pipe::PipeInterrupt,
}
impl fmt::Debug for AskpassBroker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AskpassBroker(<private>)")
    }
}
impl AskpassBroker {
    /// `parent` must already be app-owned. Atomic mkdir refuses collisions.
    #[cfg(unix)]
    pub fn new(parent: &Path, timeout: Duration) -> io::Result<Self> {
        use std::os::unix::{fs::DirBuilderExt, net::UnixListener};
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if timeout.is_zero() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let directory = parent.join(format!(
            "askpass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let meta = std::fs::symlink_metadata(&directory)?;
        if meta.file_type().is_symlink()
            || !meta.is_dir()
            || meta.permissions().mode() & 0o777 != 0o700
            || meta.uid() != current_euid()
        {
            let _ = std::fs::remove_dir(&directory);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "askpass directory is not owner-only",
            ));
        }
        let socket = directory.join("prompt.sock");
        let listener = match UnixListener::bind(&socket) {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_dir(&directory);
                return Err(e);
            }
        };
        if let Err(error) = publish_socket(listener.as_raw_fd(), &socket) {
            let _ = std::fs::remove_file(&socket);
            let _ = std::fs::remove_dir(&directory);
            return Err(error);
        }
        listener.set_nonblocking(true)?;
        let (request_tx, requests) = mpsc::sync_channel(1);
        let (replies, reply_rx) = mpsc::sync_channel::<Reply>(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::spawn(move || {
            static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);
            while !stop.load(Ordering::Acquire) {
                let mut stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                // A pathname socket in a private directory is not enough: the
                // same user can still connect. SO_PEERCRED (or the platform
                // equivalent) rejects a foreign uid before any prompt is read.
                if same_user_peer(stream.as_raw_fd()).is_err() {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
                let prompt = match read_blob(&mut stream, MAX_PROMPT_BYTES) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let Ok(text) = String::from_utf8(prompt) else {
                    continue;
                };
                let id = NEXT_PROMPT.fetch_add(1, Ordering::Relaxed);
                if id == 0 {
                    break;
                }
                let text: String = text
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .collect();
                let deadline = Instant::now() + timeout;
                if request_tx
                    .try_send(PromptRequest {
                        id,
                        text,
                        expires_at: deadline,
                    })
                    .is_err()
                {
                    continue;
                }
                while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                    match reply_rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(reply) if reply.id == id => {
                            if let Some(answer) = reply.answer {
                                let _ = write_blob(&mut stream, &answer.0);
                            }
                            break;
                        }
                        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        Ok(Self {
            directory,
            socket,
            requests,
            replies,
            cancelled,
            outstanding: None,
        })
    }
    #[cfg(windows)]
    pub fn new(_parent: &Path, timeout: Duration) -> io::Result<Self> {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if timeout.is_zero() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let name = format!(
            r"\\.\pipe\made-askpass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let pipe = windows_pipe::create(&name)?;
        let socket = PathBuf::from(&name);
        // The spawned thread receives the only owner. This token can interrupt
        // ConnectNamedPipe; it must not close the handle.
        let interrupt = windows_pipe::interrupt(&pipe);
        let (request_tx, requests) = mpsc::sync_channel(1);
        let (replies, reply_rx) = mpsc::sync_channel::<Reply>(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::spawn(move || windows_pipe::serve(pipe, stop, timeout, request_tx, reply_rx));
        Ok(Self {
            directory: PathBuf::new(),
            socket,
            requests,
            replies,
            cancelled,
            outstanding: None,
            pipe: interrupt,
        })
    }
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }
    pub fn poll(&mut self) -> Option<PromptRequest> {
        if self.outstanding.is_some_and(|(_, d)| Instant::now() >= d) {
            self.outstanding = None;
        }
        if self.cancelled.load(Ordering::Acquire) || self.outstanding.is_some() {
            return None;
        }
        let request = self.requests.try_recv().ok()?;
        if Instant::now() >= request.expires_at {
            return None;
        }
        self.outstanding = Some((request.id, request.expires_at));
        Some(request)
    }
    /// A stale, duplicate or foreign response is never used for another prompt.
    pub fn respond(&mut self, id: u64, answer: Option<PromptAnswer>) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire)
            || !self
                .outstanding
                .is_some_and(|(current, deadline)| current == id && Instant::now() < deadline)
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.replies
            .try_send(Reply { id, answer })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        self.outstanding = None;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.outstanding = None;
        #[cfg(windows)]
        windows_pipe::cancel(&self.pipe);
    }
}
impl Drop for AskpassBroker {
    fn drop(&mut self) {
        self.cancel();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
#[cfg(unix)]
fn current_euid() -> u32 {
    // SAFETY: geteuid has no arguments or pointer preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn publish_socket(fd: std::os::fd::RawFd, path: &Path) -> io::Result<()> {
    crate::shell_integration::publish_private_socket(fd, path)
}

#[cfg(unix)]
fn same_user_peer(fd: std::os::fd::RawFd) -> io::Result<()> {
    let peer = peer_uid(fd)?;
    if peer != current_euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "askpass peer uid mismatch",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn peer_uid(fd: std::os::fd::RawFd) -> io::Result<u32> {
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred is a libc::ucred-sized buffer. getsockopt writes it only
    // when it returns 0. SO_PEERCRED is the Linux peer-credential option.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            cred.as_mut_ptr().cast::<libc::c_void>(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if len < std::mem::size_of::<libc::ucred>() as libc::socklen_t {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short SO_PEERCRED",
        ));
    }
    // SAFETY: getsockopt succeeded and wrote a full ucred.
    let cred = unsafe { cred.assume_init() };
    Ok(cred.uid)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn peer_uid(fd: std::os::fd::RawFd) -> io::Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: fd is a connected unix stream. getpeereid writes the peer euid
    // and egid; it is the macOS equivalent of SO_PEERCRED for this socket.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

fn read_blob(reader: &mut impl io::Read, cap: usize) -> io::Result<Vec<u8>> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size > cap {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn write_blob(writer: &mut impl io::Write, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}
/// Only the packaged askpass executable calls this. Answers go directly to SSH.
#[cfg(unix)]
pub fn run_helper(socket: &Path, prompt: &str, output: &mut impl io::Write) -> io::Result<()> {
    use std::os::unix::net::UnixStream;
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    write_blob(&mut stream, prompt.as_bytes())?;
    let answer = PromptAnswer::new(read_blob(&mut stream, MAX_ANSWER_BYTES)?)?;
    output.write_all(&answer.0)?;
    output.write_all(b"\n")?;
    output.flush()
}
#[cfg(windows)]
pub fn run_helper(pipe: &Path, prompt: &str, output: &mut impl io::Write) -> io::Result<()> {
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let handle = windows_pipe::connect(pipe)?;
    windows_pipe::write_blob(handle, prompt.as_bytes())?;
    let answer = PromptAnswer::new(windows_pipe::read_blob(handle, MAX_ANSWER_BYTES)?)?;
    output.write_all(&answer.0)?;
    output.write_all(b"\n")?;
    output.flush()?;
    windows_pipe::close(handle);
    Ok(())
}

#[cfg(windows)]
mod windows_pipe {
    use super::{read_blob as decode_blob, write_blob as encode_blob, Reply};
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::Path;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender},
    };
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Foundation::{LocalFree, GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
        PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Non-owning name of the pipe the serve thread owns.
    ///
    /// `CancelIoEx` must use the same handle that issued `ConnectNamedPipe`. A
    /// duplicated handle would not wake that call, so this token stores the
    /// raw value without taking ownership or closing it.
    pub(super) struct PipeInterrupt(RawHandle);

    // SAFETY: the token never closes or frees the handle. `CancelIoEx` on a
    // live named pipe is safe from any thread, and the broker uses this token
    // only while the serve thread still owns that pipe.
    unsafe impl Send for PipeInterrupt {}

    pub(super) fn interrupt(pipe: &OwnedHandle) -> PipeInterrupt {
        PipeInterrupt(pipe.as_raw_handle())
    }

    fn as_handle(pipe: &OwnedHandle) -> HANDLE {
        pipe.as_raw_handle() as HANDLE
    }

    pub fn create(name: &str) -> io::Result<OwnedHandle> {
        let wide = wide(std::ffi::OsStr::new(name));
        let sddl = wide_literal("D:P(A;;GA;;;CU)");
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: sddl is NUL-terminated. The API allocates the descriptor,
        // which is released with LocalFree after the pipe is created.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: name and attributes are valid for the call. PIPE_REJECT_REMOTE_CLIENTS
        // refuses a remote client. The returned handle is uniquely owned.
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                8192,
                8192,
                0,
                &mut attributes,
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateNamedPipeW returned a unique live handle. This is the
        // only owner. The serve thread receives it and closes it on drop.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
    }

    pub fn serve(
        owner: OwnedHandle,
        stop: std::sync::Arc<AtomicBool>,
        timeout: Duration,
        requests: SyncSender<super::PromptRequest>,
        replies: Receiver<Reply>,
    ) {
        let pipe = as_handle(&owner);
        use std::sync::atomic::AtomicU64;
        static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);
        while !stop.load(Ordering::Acquire) {
            // SAFETY: pipe is the server handle created above. A null overlapped
            // pointer makes the call synchronous.
            let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) };
            if stop.load(Ordering::Acquire) {
                break;
            }
            if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
                break;
            }
            if !same_user(pipe) {
                unsafe {
                    DisconnectNamedPipe(pipe);
                }
                continue;
            }
            let mut io = PipeIo(pipe);
            let prompt = match decode_blob(&mut io, super::MAX_PROMPT_BYTES) {
                Ok(bytes) => bytes,
                Err(_) => {
                    unsafe {
                        DisconnectNamedPipe(pipe);
                    }
                    continue;
                }
            };
            let Ok(text) = String::from_utf8(prompt) else {
                unsafe {
                    DisconnectNamedPipe(pipe);
                }
                continue;
            };
            let id = NEXT_PROMPT.fetch_add(1, Ordering::Relaxed);
            if id == 0 {
                break;
            }
            let text: String = text
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect();
            let deadline = Instant::now() + timeout;
            if requests
                .try_send(super::PromptRequest {
                    id,
                    text,
                    expires_at: deadline,
                })
                .is_err()
            {
                unsafe {
                    DisconnectNamedPipe(pipe);
                }
                continue;
            }
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match replies.recv_timeout(Duration::from_millis(20)) {
                    Ok(reply) if reply.id == id => {
                        if let Some(answer) = reply.answer {
                            let mut io = PipeIo(pipe);
                            let _ = encode_blob(&mut io, &answer.0);
                        }
                        break;
                    }
                    Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        return;
                    }
                }
            }
            unsafe {
                DisconnectNamedPipe(pipe);
            }
        }
        // `owner` drops here and is the only closer of the server pipe.
    }

    pub fn cancel(pipe: &PipeInterrupt) {
        let handle = pipe.0 as HANDLE;
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return;
        }
        // SAFETY: `handle` names the server pipe the serve thread still owns.
        // A null overlapped pointer cancels the blocked ConnectNamedPipe so
        // cancellation does not wait for a client. This token does not close it.
        unsafe {
            windows_sys::Win32::System::IO::CancelIoEx(handle, std::ptr::null());
        }
    }

    pub fn connect(path: &Path) -> io::Result<HANDLE> {
        let wide = wide(path.as_os_str());
        // SAFETY: the path is NUL-terminated. OPEN_EXISTING refuses to create
        // a different file. The handle is uniquely owned by the helper.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    pub fn read_blob(handle: HANDLE, cap: usize) -> io::Result<Vec<u8>> {
        decode_blob(&mut PipeIo(handle), cap)
    }

    pub fn write_blob(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
        encode_blob(&mut PipeIo(handle), bytes)
    }

    pub fn close(handle: HANDLE) {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
        }
    }

    fn same_user(pipe: HANDLE) -> bool {
        let mut pid = 0u32;
        // SAFETY: pipe is a connected server handle. The pid is written only
        // on success. A failed query rejects the client.
        if unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) } == 0 || pid == 0 {
            return false;
        }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return false;
        }
        let client = token_user(process);
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(process);
        }
        let current = token_user(unsafe { GetCurrentProcess() });
        match (client, current) {
            (Some(client), Some(current)) => sids_equal(&client, &current),
            _ => false,
        }
    }

    fn token_user(process: HANDLE) -> Option<Vec<u8>> {
        let mut token = std::ptr::null_mut();
        // SAFETY: process is a live process handle. The token is a new owned handle.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 || needed > 1024 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(token);
            }
            return None;
        }
        let mut bytes = vec![0u8; needed as usize];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(token);
        }
        (ok != 0).then_some(bytes)
    }

    fn sids_equal(left: &[u8], right: &[u8]) -> bool {
        if left.len() < std::mem::size_of::<TOKEN_USER>()
            || right.len() < std::mem::size_of::<TOKEN_USER>()
        {
            return false;
        }
        // SAFETY: both buffers were filled by GetTokenInformation as TOKEN_USER
        // and are at least that large. EqualSid reads the SIDs those headers point at.
        unsafe {
            let left = &*(left.as_ptr() as *const TOKEN_USER);
            let right = &*(right.as_ptr() as *const TOKEN_USER);
            EqualSid(left.User.Sid, right.User.Sid) != 0
        }
    }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn wide_literal(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct PipeIo(HANDLE);

    impl Read for PipeIo {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut read = 0u32;
            // SAFETY: buf is a valid writable slice and the pipe handle is connected.
            let ok = unsafe {
                ReadFile(
                    self.0,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(read as usize)
            }
        }
    }

    impl Write for PipeIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut written = 0u32;
            // SAFETY: buf is a valid readable slice and the pipe handle is connected.
            let ok = unsafe {
                WriteFile(
                    self.0,
                    buf.as_ptr(),
                    buf.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
