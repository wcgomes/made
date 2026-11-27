//! User-confirmed URL open requests from terminal children.
//!
//! A terminal child that wants to open a URL runs `$BROWSER URL`. Made points
//! `BROWSER` at its own helper, which validates the URL and sends one bounded
//! request `{pane token, url}` over a private out-of-band endpoint named by
//! [`URL_SOCKET_ENV`]. The helper exits 0 only when an attached UI
//! acknowledges the request within [`HELPER_ACK_TIMEOUT`]; otherwise it exits
//! non-zero, and the child falls back to printing the URL. The helper never
//! opens anything itself, and nothing here logs a URL.
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The variable terminal children read to find a browser command.
pub const BROWSER_ENV: &str = "BROWSER";
/// The user's own `BROWSER`, preserved when Made replaces it.
pub const ORIGINAL_BROWSER_ENV: &str = "MADE_ORIGINAL_BROWSER";
/// The private endpoint the helper sends its request to.
pub const URL_SOCKET_ENV: &str = "MADE_URL_SOCKET";
/// The launching pane's token the helper presents with its request.
pub const URL_TOKEN_ENV: &str = "MADE_URL_TOKEN";
/// File name the remote companion answers to as the URL helper. A symlink of
/// this name beside the session runtime points at the companion executable.
pub const REMOTE_HELPER_NAME: &str = "made-open";
/// Longest URL accepted, in bytes.
pub const MAX_URL_BYTES: usize = 2048;
/// The whole helper exchange: connect, send, and wait for the ack.
pub const HELPER_ACK_TIMEOUT: Duration = Duration::from_secs(1);
/// A pane token is 128 random bits in lowercase hexadecimal.
pub const TOKEN_HEX_LEN: usize = 32;
/// Request body bound: token, separator and URL.
pub const MAX_REQUEST_BYTES: usize = TOKEN_HEX_LEN + 1 + MAX_URL_BYTES;

const ACK: u8 = b'1';
const NACK: u8 = b'0';
/// Connections whose request has not fully arrived yet.
#[cfg(unix)]
const MAX_PARTIAL: usize = 8;
/// Requests handed to the owner per poll.
const MAX_REQUESTS_PER_POLL: usize = 8;
/// A connected helper must deliver its whole request within this budget.
#[cfg(unix)]
const REQUEST_READ_BUDGET: Duration = Duration::from_millis(500);

/// File name of the packaged URL helper beside the application executable.
pub const fn url_helper_file_name() -> &'static str {
    if cfg!(windows) {
        "made-open.exe"
    } else {
        "made-open"
    }
}

/// Why a URL was refused. Carries no part of the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlRejection {
    Empty,
    TooLong,
    /// A control, bidirectional-override, whitespace or other non-printable
    /// ASCII character.
    Character,
    Scheme,
    Host,
    UserInfo,
    Port,
}

impl fmt::Display for UrlRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "empty URL",
            Self::TooLong => "URL is too long",
            Self::Character => "URL contains a disallowed character",
            Self::Scheme => "only http and https URLs are accepted",
            Self::Host => "URL has no valid host",
            Self::UserInfo => "URL carries user information",
            Self::Port => "URL has an invalid port",
        })
    }
}

impl std::error::Error for UrlRejection {}

impl From<UrlRejection> for io::Error {
    fn from(value: UrlRejection) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, value)
    }
}

/// An absolute `http`/`https` URL that passed [`validate_url`]. Debug output
/// is redacted so a URL cannot reach a log through formatting.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeUrl {
    text: String,
    host_start: usize,
    host_end: usize,
}

impl fmt::Debug for SafeUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SafeUrl(<redacted>)")
    }
}

impl SafeUrl {
    /// The validated URL with a lowercase scheme.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// `http://` or `https://`.
    pub fn prefix(&self) -> &str {
        &self.text[..self.host_start]
    }

    /// The host as written (a bracketed IPv6 literal keeps its brackets).
    pub fn host(&self) -> &str {
        &self.text[self.host_start..self.host_end]
    }

    /// Port, path, query and fragment after the host.
    pub fn rest(&self) -> &str {
        &self.text[self.host_end..]
    }
}

/// Accept only an absolute `http`/`https` URL of at most [`MAX_URL_BYTES`]
/// printable ASCII bytes, with a host, no user information and a valid port.
///
/// Printable ASCII excludes every control, whitespace and bidirectional
/// formatting character; an internationalized host must arrive in its
/// punycode form. A backslash is refused because browsers read it as a path
/// separator, which would let the displayed host differ from the opened one.
pub fn validate_url(input: &str) -> Result<SafeUrl, UrlRejection> {
    if input.is_empty() {
        return Err(UrlRejection::Empty);
    }
    if input.len() > MAX_URL_BYTES {
        return Err(UrlRejection::TooLong);
    }
    if input
        .bytes()
        .any(|b| !(0x21..=0x7e).contains(&b) || b == b'\\')
    {
        return Err(UrlRejection::Character);
    }
    let lower = input.get(..8).unwrap_or(input).to_ascii_lowercase();
    let (scheme, host_start) = if lower.starts_with("https://") {
        ("https://", 8)
    } else if lower.starts_with("http://") {
        ("http://", 7)
    } else {
        return Err(UrlRejection::Scheme);
    };
    let after = &input[host_start..];
    let authority_len = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..authority_len];
    if authority.contains('@') {
        return Err(UrlRejection::UserInfo);
    }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let close = inner.find(']').ok_or(UrlRejection::Host)?;
        let literal = &inner[..close];
        if literal.is_empty()
            || !literal.contains(':')
            || !literal
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return Err(UrlRejection::Host);
        }
        let tail = &inner[close + 1..];
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port),
            None if tail.is_empty() => None,
            None => return Err(UrlRejection::Host),
        };
        (&authority[..close + 2], port)
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty()
        || (!host.starts_with('[')
            && (host.starts_with(['.', '-'])
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))))
    {
        return Err(UrlRejection::Host);
    }
    if let Some(port) = port {
        if port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|b| b.is_ascii_digit())
            || !port.parse::<u32>().is_ok_and(|n| n <= 65_535)
        {
            return Err(UrlRejection::Port);
        }
    }
    let mut text = String::with_capacity(input.len());
    text.push_str(scheme);
    text.push_str(after);
    Ok(SafeUrl {
        host_end: host_start + host.len(),
        host_start,
        text,
    })
}

/// A pane's routing token: 128 random bits, lowercase hexadecimal. Debug is
/// redacted; the value only travels in the child environment.
#[derive(Clone, PartialEq, Eq)]
pub struct UrlToken(String);

impl fmt::Debug for UrlToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UrlToken(<redacted>)")
    }
}

impl UrlToken {
    /// A fresh token from the operating system random source.
    pub fn generate() -> io::Result<Self> {
        let mut bytes = [0u8; TOKEN_HEX_LEN / 2];
        getrandom::fill(&mut bytes).map_err(|_| io::Error::other("no system randomness"))?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `presented` is this token. The comparison does not stop at the
    /// first differing byte.
    pub fn matches(&self, presented: &str) -> bool {
        presented.len() == self.0.len()
            && presented
                .bytes()
                .zip(self.0.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

fn valid_token(token: &str) -> bool {
    token.len() == TOKEN_HEX_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn encode_request(token: &str, url: &SafeUrl) -> Vec<u8> {
    let mut body = Vec::with_capacity(token.len() + 1 + url.as_str().len());
    body.extend_from_slice(token.as_bytes());
    body.push(b'\n');
    body.extend_from_slice(url.as_str().as_bytes());
    body
}

fn decode_request(body: &[u8]) -> Option<(String, SafeUrl)> {
    if body.len() > MAX_REQUEST_BYTES {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?;
    let (token, url) = text.split_once('\n')?;
    if !valid_token(token) {
        return None;
    }
    Some((token.to_owned(), validate_url(url).ok()?))
}

/// The variables a terminal child gets so `$BROWSER URL` reaches `socket`
/// with `token`: `BROWSER` names `helper`, and a pre-existing user `BROWSER`
/// is preserved as [`ORIGINAL_BROWSER_ENV`]. An inherited `BROWSER` that is
/// already a Made helper (a nested launch) is not preserved over the earlier
/// original.
pub fn browser_environment(
    helper: &Path,
    socket: &Path,
    token: &UrlToken,
    inherited_browser: Option<&OsStr>,
) -> Vec<(OsString, OsString)> {
    let mut env = vec![
        (OsString::from(BROWSER_ENV), helper.as_os_str().to_owned()),
        (
            OsString::from(URL_SOCKET_ENV),
            socket.as_os_str().to_owned(),
        ),
        (
            OsString::from(URL_TOKEN_ENV),
            OsString::from(token.as_str()),
        ),
    ];
    if let Some(original) = inherited_browser.filter(|value| !value.is_empty()) {
        if !is_made_helper(original) {
            env.push((OsString::from(ORIGINAL_BROWSER_ENV), original.to_owned()));
        }
    }
    env
}

/// Whether a `BROWSER` value names a Made URL helper (local or remote).
pub fn is_made_helper(browser: &OsStr) -> bool {
    Path::new(browser)
        .file_name()
        .is_some_and(|name| name == url_helper_file_name() || name == REMOTE_HELPER_NAME)
}

/// What `BROWSER` must become for a process that cannot reach a Made helper
/// endpoint: `None` leaves it as is; `Some(Some(value))` restores the user's
/// original; `Some(None)` removes an inherited Made helper that would only
/// fail. Only an inherited Made helper is ever changed.
pub fn restored_browser(
    browser: Option<&OsStr>,
    original: Option<&OsStr>,
) -> Option<Option<OsString>> {
    if !browser.is_some_and(is_made_helper) {
        return None;
    }
    Some(
        original
            .filter(|value| !value.is_empty() && !is_made_helper(value))
            .map(OsStr::to_owned),
    )
}

/// Environment changes for the host opener so it cannot route back into a
/// card: the URL routing variables are removed and an inherited Made helper
/// `BROWSER` is restored to the user's original or removed. Returns
/// (variables to set, variables to remove).
pub fn opener_environment(
    browser: Option<&OsStr>,
    original: Option<&OsStr>,
) -> (Vec<(OsString, OsString)>, Vec<&'static str>) {
    let mut set = Vec::new();
    let mut remove = vec![URL_SOCKET_ENV, URL_TOKEN_ENV, ORIGINAL_BROWSER_ENV];
    match restored_browser(browser, original) {
        Some(Some(value)) => set.push((OsString::from(BROWSER_ENV), value)),
        Some(None) => remove.push(BROWSER_ENV),
        None => {}
    }
    (set, remove)
}

/// Create `link` as a symlink to the remote companion, so `BROWSER` can name
/// the companion's URL helper mode by file name (it cannot carry a mode
/// argument). Remote targets are Unix; other hosts report `Unsupported`.
pub fn link_remote_helper(companion: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(companion, link)
    }
    #[cfg(not(unix))]
    {
        let _ = (companion, link);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Outcome of [`UrlRateGate::admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// A new request: forward or show it.
    Fresh,
    /// The same URL as the last fresh one inside the window: coalesce.
    Duplicate,
    /// A different URL inside the window: drop it.
    Limited,
}

/// About one request per `window`, with repeats of the last URL coalesced.
#[derive(Debug)]
pub struct UrlRateGate {
    window: Duration,
    last: Option<(String, Instant)>,
}

impl UrlRateGate {
    pub fn new(window: Duration) -> Self {
        Self { window, last: None }
    }

    /// Classify `url` and record it when it is fresh.
    pub fn admit(&mut self, url: &SafeUrl, now: Instant) -> RateDecision {
        let decision = self.check(url, now);
        if decision == RateDecision::Fresh {
            self.record(url, now);
        }
        decision
    }

    /// Classify `url` without recording it. A caller that may fail to deliver
    /// a fresh request records it with [`record`](Self::record) only after
    /// delivery, so an undelivered request is never later coalesced as a
    /// duplicate and acknowledged.
    pub fn check(&self, url: &SafeUrl, now: Instant) -> RateDecision {
        match &self.last {
            Some((last, at)) if now.saturating_duration_since(*at) < self.window => {
                if last == url.as_str() {
                    RateDecision::Duplicate
                } else {
                    RateDecision::Limited
                }
            }
            _ => RateDecision::Fresh,
        }
    }

    /// Record a delivered request as the window's latest.
    pub fn record(&mut self, url: &SafeUrl, now: Instant) {
        self.last = Some((url.as_str().to_owned(), now));
    }
}

/// One validated request waiting for its owner's decision. Dropping it
/// without [`accept`](Self::accept) is a refusal: the helper sees the
/// connection close and exits non-zero.
pub struct UrlRequest {
    token: String,
    url: SafeUrl,
    reply: Responder,
}

impl fmt::Debug for UrlRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UrlRequest(<redacted>)")
    }
}

enum Responder {
    #[cfg(unix)]
    Stream(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Pipe(std::sync::mpsc::SyncSender<bool>),
}

impl Responder {
    fn send(self, accepted: bool) {
        match self {
            #[cfg(unix)]
            Self::Stream(mut stream) => {
                let _ = write_blob(&mut stream, &[if accepted { ACK } else { NACK }]);
            }
            #[cfg(windows)]
            Self::Pipe(sender) => {
                let _ = sender.try_send(accepted);
            }
        }
    }
}

impl UrlRequest {
    /// The token the helper presented. Compare it with [`UrlToken::matches`].
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn url(&self) -> &SafeUrl {
        &self.url
    }

    /// Acknowledge receipt; the helper exits 0.
    pub fn accept(self) {
        self.reply.send(true);
    }

    /// Refuse; the helper exits non-zero and the child prints the URL.
    pub fn reject(self) {
        self.reply.send(false);
    }
}

#[cfg(unix)]
fn write_blob(writer: &mut impl io::Write, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut frame = Vec::with_capacity(4 + bytes.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(bytes);
    writer.write_all(&frame)?;
    writer.flush()
}

#[cfg(unix)]
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

fn ack_result(reply: &[u8]) -> io::Result<()> {
    if reply == [ACK] {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "URL request refused",
        ))
    }
}

/// The private endpoint a pane (or a remote supervisor) owns. Unix binds a
/// stream socket in an owner-only directory and is polled without blocking
/// from the owner's loop; no thread is started. Windows serves a
/// same-user-only named pipe from one thread.
pub struct UrlRequestSink {
    #[cfg(unix)]
    listener: std::os::unix::net::UnixListener,
    #[cfg(unix)]
    directory: PathBuf,
    #[cfg(unix)]
    partial: Vec<Partial>,
    path: PathBuf,
    #[cfg(windows)]
    requests: std::sync::mpsc::Receiver<UrlRequest>,
    #[cfg(windows)]
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(windows)]
    pipe: crate::ssh_askpass::windows_pipe::PipeInterrupt,
}

impl fmt::Debug for UrlRequestSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UrlRequestSink(<private>)")
    }
}

#[cfg(unix)]
struct Partial {
    stream: std::os::unix::net::UnixStream,
    buffer: Vec<u8>,
    deadline: Instant,
}

#[cfg(unix)]
enum ReadState {
    Complete(Vec<u8>),
    Pending,
    Invalid,
}

#[cfg(unix)]
impl Partial {
    fn read(&mut self) -> ReadState {
        use std::io::Read;
        let mut chunk = [0u8; 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return ReadState::Invalid,
                Ok(n) => {
                    self.buffer.extend_from_slice(&chunk[..n]);
                    if self.buffer.len() >= 4 {
                        let mut size = [0; 4];
                        size.copy_from_slice(&self.buffer[..4]);
                        let size = u32::from_be_bytes(size) as usize;
                        if size > MAX_REQUEST_BYTES || self.buffer.len() > 4 + size {
                            return ReadState::Invalid;
                        }
                        if self.buffer.len() == 4 + size {
                            return ReadState::Complete(self.buffer.split_off(4));
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return ReadState::Pending
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return ReadState::Invalid,
            }
        }
    }
}

impl UrlRequestSink {
    /// Bind one private endpoint under `parent`, or in its own owner-only
    /// directory under the OS temp root when that path exceeds the AF_UNIX
    /// address limit. Dropping the sink removes the endpoint.
    #[cfg(unix)]
    pub fn new(parent: &Path) -> io::Result<Self> {
        use crate::shell_integration::{
            sink_directory_name, temp_root_sink_directory, AF_UNIX_ENDPOINT_CHARS,
        };
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut directory = None;
        let mut last = None;
        for _ in 0..16 {
            let name = format!(
                "url-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let attempt = if parent.join(&name).join("url.sock").as_os_str().len()
                <= AF_UNIX_ENDPOINT_CHARS
            {
                let directory = parent.join(name);
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&directory)
                    .map(|()| directory)
            } else {
                temp_root_sink_directory(&sink_directory_name(&NEXT))
            };
            match attempt {
                Ok(created) => {
                    directory = Some(created);
                    break;
                }
                Err(error) => last = Some(error),
            }
        }
        let Some(directory) = directory else {
            return Err(last.unwrap_or_else(|| io::Error::other("no URL sink directory")));
        };
        let path = directory.join("url.sock");
        let listener = match std::os::unix::net::UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = std::fs::remove_dir(&directory);
                return Err(error);
            }
        };
        let published =
            crate::shell_integration::publish_private_socket(listener.as_raw_fd(), &path)
                .and_then(|()| listener.set_nonblocking(true));
        if let Err(error) = published {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir(&directory);
            return Err(error);
        }
        Ok(Self {
            listener,
            directory,
            partial: Vec::new(),
            path,
        })
    }

    #[cfg(windows)]
    pub fn new(_parent: &Path) -> io::Result<Self> {
        use crate::ssh_askpass::windows_pipe;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|_| io::Error::other("no system randomness"))?;
        let name = format!(
            r"\\.\pipe\made-url-{}-{}-{:016x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            u64::from_be_bytes(nonce)
        );
        let pipe = windows_pipe::create(&name)?;
        let interrupt = windows_pipe::interrupt(&pipe);
        let (request_tx, requests) = std::sync::mpsc::sync_channel(MAX_REQUESTS_PER_POLL);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let serve_stop = stop.clone();
        std::thread::Builder::new()
            .name("made-url-sink".to_owned())
            .spawn(move || windows_serve(pipe, serve_stop, request_tx))?;
        Ok(Self {
            path: PathBuf::from(name),
            requests,
            stop,
            pipe: interrupt,
        })
    }

    /// The endpoint the helper is pointed at through [`URL_SOCKET_ENV`].
    pub fn socket_path(&self) -> &Path {
        &self.path
    }

    /// Bounded, non-blocking: the complete, validated requests that arrived
    /// since the last poll. Malformed, oversized, foreign-user and slow
    /// requests are dropped, which the helper reports as a refusal.
    #[cfg(unix)]
    pub fn poll(&mut self) -> Vec<UrlRequest> {
        use std::os::fd::AsRawFd;
        let now = Instant::now();
        for _ in 0..MAX_PARTIAL {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.partial.len() >= MAX_PARTIAL
                        || crate::ssh_askpass::same_user_peer(stream.as_raw_fd()).is_err()
                        || stream.set_nonblocking(true).is_err()
                    {
                        continue;
                    }
                    self.partial.push(Partial {
                        stream,
                        buffer: Vec::new(),
                        deadline: now + REQUEST_READ_BUDGET,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let mut ready = Vec::new();
        let mut waiting = Vec::new();
        for mut partial in std::mem::take(&mut self.partial) {
            match partial.read() {
                ReadState::Complete(body) if ready.len() < MAX_REQUESTS_PER_POLL => {
                    if let Some((token, url)) = decode_request(&body) {
                        ready.push(UrlRequest {
                            token,
                            url,
                            reply: Responder::Stream(partial.stream),
                        });
                    }
                }
                ReadState::Pending if now < partial.deadline => waiting.push(partial),
                _ => {}
            }
        }
        self.partial = waiting;
        ready
    }

    #[cfg(windows)]
    pub fn poll(&mut self) -> Vec<UrlRequest> {
        let mut ready = Vec::new();
        while ready.len() < MAX_REQUESTS_PER_POLL {
            match self.requests.try_recv() {
                Ok(request) => ready.push(request),
                Err(_) => break,
            }
        }
        ready
    }
}

impl Drop for UrlRequestSink {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_dir(&self.directory);
        }
        #[cfg(windows)]
        {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            crate::ssh_askpass::windows_pipe::cancel(&self.pipe);
        }
    }
}

#[cfg(windows)]
fn windows_serve(
    owner: std::os::windows::io::OwnedHandle,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    requests: std::sync::mpsc::SyncSender<UrlRequest>,
) {
    use crate::ssh_askpass::windows_pipe;
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED, HANDLE};
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe};
    let pipe = owner.as_raw_handle() as HANDLE;
    while !stop.load(Ordering::Acquire) {
        // SAFETY: pipe is the server handle `owner` keeps alive for this loop.
        // A null overlapped pointer makes the call synchronous; the sink's
        // drop cancels it through CancelIoEx.
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) };
        if stop.load(Ordering::Acquire) {
            break;
        }
        // SAFETY: GetLastError reads this thread's last-error value.
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            break;
        }
        let request = if windows_pipe::same_user(pipe) {
            windows_pipe::read_blob(pipe, MAX_REQUEST_BYTES)
                .ok()
                .and_then(|body| decode_request(&body))
        } else {
            None
        };
        if let Some((token, url)) = request {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            let accepted = requests
                .try_send(UrlRequest {
                    token,
                    url,
                    reply: Responder::Pipe(reply_tx),
                })
                .is_ok()
                && reply_rx.recv_timeout(HELPER_ACK_TIMEOUT).unwrap_or(false);
            let _ = windows_pipe::write_blob(pipe, &[if accepted { ACK } else { NACK }]);
        }
        // SAFETY: pipe is the connected server handle; disconnecting readies
        // it for the next client.
        unsafe {
            DisconnectNamedPipe(pipe);
        }
    }
    // `owner` drops here and is the only closer of the server pipe.
}

/// The helper side: validate `url`, send `{token, url}` to `socket`, and
/// return `Ok` only when the owner acknowledged it within `timeout`.
#[cfg(unix)]
pub fn send_request(socket: &Path, token: &str, url: &str, timeout: Duration) -> io::Result<()> {
    use std::os::unix::net::UnixStream;
    let url = validate_url(url)?;
    if !valid_token(token) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let deadline = Instant::now() + timeout;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
    };
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(remaining()?))?;
    write_blob(&mut stream, &encode_request(token, &url))?;
    stream.set_read_timeout(Some(remaining()?))?;
    ack_result(&read_blob(&mut stream, 1)?)
}

#[cfg(windows)]
pub fn send_request(socket: &Path, token: &str, url: &str, timeout: Duration) -> io::Result<()> {
    use crate::ssh_askpass::windows_pipe;
    let url = validate_url(url)?;
    if !valid_token(token) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let body = encode_request(token, &url);
    let socket = socket.to_path_buf();
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    // Blocking pipe reads have no timeout; the exchange runs on its own
    // thread so the helper still returns at the deadline. The process exits
    // right after, which ends the thread.
    std::thread::spawn(move || {
        let result = (|| {
            let handle = windows_pipe::connect(&socket)?;
            let result = windows_pipe::write_blob(handle, &body)
                .and_then(|()| windows_pipe::read_blob(handle, 1));
            windows_pipe::close(handle);
            ack_result(&result?)
        })();
        let _ = done_tx.send(result);
    });
    done_rx
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
}

/// The whole helper entry point: read the endpoint and token from the
/// environment, take exactly one URL argument, and report success only on an
/// ack. Prints nothing.
pub fn run_helper(args: &[OsString]) -> io::Result<()> {
    let [url] = args else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    let url = url.to_str().ok_or(io::ErrorKind::InvalidInput)?;
    let socket = std::env::var_os(URL_SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .ok_or(io::ErrorKind::NotFound)?;
    let token = std::env::var(URL_TOKEN_ENV).map_err(|_| io::ErrorKind::NotFound)?;
    send_request(Path::new(&socket), &token, url, HELPER_ACK_TIMEOUT)
}

/// Open a validated URL in the host's default browser. The URL is passed as
/// one argument, never through a shell: macOS `/usr/bin/open`, Linux
/// `xdg-open` from PATH, Windows `ShellExecuteW`.
pub fn open_url(url: &SafeUrl) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        spawn_opener(std::process::Command::new("/usr/bin/open").arg(url.as_str()))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        spawn_opener(std::process::Command::new("xdg-open").arg(url.as_str()))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide =
            |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
        let verb = wide("open");
        let target = wide(url.as_str());
        // SAFETY: both strings are NUL-terminated UTF-16 and outlive the
        // call. No parameters or directory are passed; the URL is the file.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success with a value greater than 32.
        if result as isize > 32 {
            Ok(())
        } else {
            Err(io::Error::other("the default browser could not be started"))
        }
    }
}

#[cfg(unix)]
fn spawn_opener(command: &mut std::process::Command) -> io::Result<()> {
    use std::process::Stdio;
    let (set, remove) = opener_environment(
        std::env::var_os(BROWSER_ENV).as_deref(),
        std::env::var_os(ORIGINAL_BROWSER_ENV).as_deref(),
    );
    for key in remove {
        command.env_remove(key);
    }
    command.envs(set);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap the opener off the UI thread; it normally exits at once.
    std::thread::Builder::new()
        .name("made-url-opener".to_owned())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_table() {
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
        // Input, then the accepted (prefix, host) or the rejection.
        type Case<'a> = (&'a str, Result<(&'a str, &'a str), UrlRejection>);
        let cases: &[Case] = &[
            ("https://example.com", Ok(("https://", "example.com"))),
            ("http://example.com/a?b=c#d", Ok(("http://", "example.com"))),
            ("HTTPS://Example.COM/x", Ok(("https://", "Example.COM"))),
            ("https://localhost:8080/cb", Ok(("https://", "localhost"))),
            ("http://127.0.0.1:65535/", Ok(("http://", "127.0.0.1"))),
            ("http://[::1]:3000/x", Ok(("http://", "[::1]"))),
            (
                "https://xn--bcher-kva.example/",
                Ok(("https://", "xn--bcher-kva.example")),
            ),
            (
                "https://claude.ai/oauth?code=1%202",
                Ok(("https://", "claude.ai")),
            ),
            ("", Err(UrlRejection::Empty)),
            (&long, Err(UrlRejection::TooLong)),
            ("file:///etc/passwd", Err(UrlRejection::Scheme)),
            ("javascript:alert(1)", Err(UrlRejection::Scheme)),
            ("vscode://open", Err(UrlRejection::Scheme)),
            ("ftp://example.com", Err(UrlRejection::Scheme)),
            ("data:text/html,x", Err(UrlRejection::Scheme)),
            ("//example.com", Err(UrlRejection::Scheme)),
            ("example.com", Err(UrlRejection::Scheme)),
            ("https:example.com", Err(UrlRejection::Scheme)),
            ("https://user:pw@example.com", Err(UrlRejection::UserInfo)),
            ("https://evil.com@good.com/", Err(UrlRejection::UserInfo)),
            ("https:///path", Err(UrlRejection::Host)),
            ("https://", Err(UrlRejection::Host)),
            ("https://:80/", Err(UrlRejection::Host)),
            ("https://-bad.example/", Err(UrlRejection::Host)),
            ("https://ex%41mple.com/", Err(UrlRejection::Host)),
            ("https://[zz]/", Err(UrlRejection::Host)),
            ("https://example.com:/", Err(UrlRejection::Port)),
            ("https://example.com:99999/", Err(UrlRejection::Port)),
            ("https://example.com:8a/", Err(UrlRejection::Port)),
            ("https://example.com/a b", Err(UrlRejection::Character)),
            ("https://example.com/\u{7}", Err(UrlRejection::Character)),
            ("https://example.com/\n", Err(UrlRejection::Character)),
            (
                "https://example.com/\u{1b}[31m",
                Err(UrlRejection::Character),
            ),
            ("https://exa\u{202e}moc.com/", Err(UrlRejection::Character)),
            (
                "https://example.com/\u{2066}x",
                Err(UrlRejection::Character),
            ),
            ("https://ex\u{e4}mple.com/", Err(UrlRejection::Character)),
            ("https://evil.com\\@good.com/", Err(UrlRejection::Character)),
        ];
        for (input, expected) in cases {
            let got = validate_url(input);
            match expected {
                Ok((prefix, host)) => {
                    let url = got.unwrap_or_else(|e| panic!("{input:?} rejected: {e}"));
                    assert_eq!(url.prefix(), *prefix, "{input:?}");
                    assert_eq!(url.host(), *host, "{input:?}");
                    assert_eq!(
                        format!("{}{}{}", url.prefix(), url.host(), url.rest()),
                        url.as_str()
                    );
                }
                Err(reason) => assert_eq!(got.err(), Some(*reason), "{input:?}"),
            }
        }
        let max = format!("https://e.co/{}", "a".repeat(MAX_URL_BYTES - 13));
        assert_eq!(max.len(), MAX_URL_BYTES);
        assert!(validate_url(&max).is_ok());
    }

    #[test]
    fn safe_url_and_token_debug_is_redacted() {
        let url = validate_url("https://secret.example/token=abc").unwrap();
        assert!(!format!("{url:?}").contains("secret"));
        let token = UrlToken::generate().unwrap();
        assert!(!format!("{token:?}").contains(token.as_str()));
        assert!(valid_token(token.as_str()));
        assert!(token.matches(token.as_str()));
        assert!(!token.matches(&"0".repeat(TOKEN_HEX_LEN)));
        assert!(!token.matches(""));
    }

    #[test]
    fn request_frame_round_trips_and_rejects_malformed_bodies() {
        let token = UrlToken::generate().unwrap();
        let url = validate_url("https://example.com/x").unwrap();
        let body = encode_request(token.as_str(), &url);
        let (decoded_token, decoded_url) = decode_request(&body).unwrap();
        assert_eq!(decoded_token, token.as_str());
        assert_eq!(decoded_url, url);
        for bad in [
            b"no-separator".to_vec(),
            format!("{}\nhttps://example.com", "g".repeat(TOKEN_HEX_LEN)).into_bytes(),
            format!("{}\nfile:///etc", token.as_str()).into_bytes(),
            b"short\nhttps://example.com".to_vec(),
            vec![0xff; 40],
            vec![b'a'; MAX_REQUEST_BYTES + 1],
        ] {
            assert!(decode_request(&bad).is_none());
        }
    }

    #[test]
    fn browser_environment_preserves_a_user_browser_once() {
        let token = UrlToken::generate().unwrap();
        let helper = Path::new("/opt/made/made-open");
        let socket = Path::new("/tmp/url.sock");
        let get = |env: &[(OsString, OsString)], key: &str| {
            env.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        let env = browser_environment(helper, socket, &token, Some(OsStr::new("firefox")));
        assert_eq!(
            get(&env, BROWSER_ENV).as_deref(),
            Some("/opt/made/made-open")
        );
        assert_eq!(get(&env, URL_SOCKET_ENV).as_deref(), Some("/tmp/url.sock"));
        assert_eq!(get(&env, URL_TOKEN_ENV).as_deref(), Some(token.as_str()));
        assert_eq!(get(&env, ORIGINAL_BROWSER_ENV).as_deref(), Some("firefox"));
        let unset = browser_environment(helper, socket, &token, None);
        assert_eq!(get(&unset, ORIGINAL_BROWSER_ENV), None);
        let nested =
            browser_environment(helper, socket, &token, Some(OsStr::new("/other/made-open")));
        assert_eq!(get(&nested, ORIGINAL_BROWSER_ENV), None);
    }

    #[test]
    fn the_opener_never_routes_back_into_a_card() {
        let helper = OsStr::new("/opt/made/made-open");
        let has = |set: &[(OsString, OsString)], key: &str| {
            set.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        // Made itself was started from a Made pane with a user browser.
        let (set, remove) = opener_environment(Some(helper), Some(OsStr::new("firefox")));
        assert_eq!(has(&set, BROWSER_ENV).as_deref(), Some("firefox"));
        for key in [URL_SOCKET_ENV, URL_TOKEN_ENV, ORIGINAL_BROWSER_ENV] {
            assert!(remove.contains(&key));
        }
        // No original: the dead helper is removed.
        let (set, remove) = opener_environment(Some(helper), None);
        assert!(set.is_empty());
        assert!(remove.contains(&BROWSER_ENV));
        // A user's own BROWSER is left alone.
        let (set, remove) = opener_environment(Some(OsStr::new("lynx")), None);
        assert!(set.is_empty());
        assert!(!remove.contains(&BROWSER_ENV));
        assert_eq!(restored_browser(None, Some(OsStr::new("x"))), None);
        assert_eq!(
            restored_browser(Some(helper), Some(OsStr::new("/a/made-open"))),
            Some(None)
        );
    }

    #[test]
    fn rate_gate_coalesces_duplicates_and_limits_distinct_urls() {
        let mut gate = UrlRateGate::new(Duration::from_secs(1));
        let a = validate_url("https://a.example/").unwrap();
        let b = validate_url("https://b.example/").unwrap();
        let start = Instant::now();
        assert_eq!(gate.admit(&a, start), RateDecision::Fresh);
        assert_eq!(
            gate.admit(&a, start + Duration::from_millis(100)),
            RateDecision::Duplicate
        );
        assert_eq!(
            gate.admit(&b, start + Duration::from_millis(200)),
            RateDecision::Limited
        );
        assert_eq!(
            gate.admit(&b, start + Duration::from_millis(1000)),
            RateDecision::Fresh
        );
    }

    #[test]
    fn an_unrecorded_fresh_request_is_not_coalesced_later() {
        let mut gate = UrlRateGate::new(Duration::from_secs(1));
        let a = validate_url("https://a.example/").unwrap();
        let start = Instant::now();
        // Delivery failed: the caller checked but did not record.
        assert_eq!(gate.check(&a, start), RateDecision::Fresh);
        let retry = start + Duration::from_millis(100);
        assert_eq!(gate.check(&a, retry), RateDecision::Fresh);
        gate.record(&a, retry);
        assert_eq!(
            gate.check(&a, retry + Duration::from_millis(100)),
            RateDecision::Duplicate
        );
    }
}
