//! PTY adapter over the platform layer.
//!
//! Spawns the user's shell in the workspace folder, reads output on a
//! background thread into a bounded channel, forwards input/resize, and
//! surfaces child exit. All OS mechanics come from `made-platform`; this module
//! contains no OS-specific code and no UI dependency.
//!
//! Output is read on a background thread into a bounded channel: the UI drains
//! it with [`PtyProcess::poll`] or [`PtyProcess::recv_timeout`]. When the channel
//! is full the reader applies backpressure to the child instead of dropping
//! arbitrary bytes, which would corrupt escape sequences.
//!
//! Input uses a separate bounded FIFO queue drained by a dedicated writer
//! thread. [`PtyProcess::write`] never performs PTY I/O and returns
//! [`PtyError::WouldBlock`] when the whole slice cannot be enqueued; the UI can
//! retry without losing ordering or duplicating a partial write.
//!
//! Dropping a [`PtyProcess`] kills and waits for the child for a bounded time,
//! closes the input queue, and detaches the reader so neither a full output
//! channel nor a blocked writer can hang window close.
//!
//! The adapter is relaunch-friendly: [`PtyOptions`] is owned and cloneable, and
//! [`PtyProcess::relaunch`] tears down the old child and starts a fresh one with
//! the same shell, cwd, environment, and size.
//!
//! # Diagnostic output
//!
//! [`PtyOptions`], [`PtyProcess`], and [`PtyEvent`] implement redacted Debug
//! formatting. Environment values are never printed (secret-bearing keys such
//! as `OPENCODE_CLI_CONFIG_CONTENT` and `MADE_*TOKEN*` are marked redacted), and
//! output chunks report only their length. A child monitor failure is surfaced
//! through [`PtyEvent::MonitorError`], [`PtyProcess::monitor_error`], and
//! [`PtyProcess::try_is_running`]; it is never reported as a running child.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use made_platform::{
    ExitStatus, Platform, PlatformError, PtyRequest, PtySession, PtySize, RedactedEnv, ShellSpec,
};

const READ_BUFFER: usize = 16 * 1024;
const DEFAULT_CHANNEL_CAPACITY: usize = 256;
/// Maximum bytes in one queued write chunk.
pub const WRITE_CHUNK_MAX: usize = 16 * 1024;
/// Default maximum number of queued write chunks.
pub const DEFAULT_WRITE_QUEUE_CAPACITY: usize = 256;

/// Options for one PTY launch.
///
/// Debug and Display output never print environment values or argument
/// contents. A child environment can include `OPENCODE_CLI_CONFIG_CONTENT`
/// (whose plugin options carry the bridge token) and `MADE_*TOKEN*` values.
#[derive(Clone, PartialEq, Eq)]
pub struct PtyOptions {
    /// Explicit shell path. When `None`, the platform resolves the user's shell.
    pub shell: Option<OsString>,
    /// Extra arguments appended after the platform's shell arguments.
    pub args: Vec<OsString>,
    /// Initial working directory.
    pub cwd: PathBuf,
    /// Environment overrides applied on top of the platform environment.
    pub env: Vec<(OsString, OsString)>,
    /// Environment variables removed from the inherited environment.
    pub env_remove: Vec<OsString>,
    pub size: PtySize,
    /// Bounded output channel capacity in chunks.
    pub channel_capacity: usize,
    /// Bounded input queue capacity in chunks (each chunk is at most
    /// [`WRITE_CHUNK_MAX`] bytes).
    pub write_queue_capacity: usize,
}

impl fmt::Debug for PtyOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyOptions")
            .field("shell", &self.shell)
            .field("arg_count", &self.args.len())
            .field("cwd", &self.cwd)
            .field("env", &RedactedEnv::new(&self.env))
            .field("env_remove", &self.env_remove)
            .field("size", &self.size)
            .field("channel_capacity", &self.channel_capacity)
            .field("write_queue_capacity", &self.write_queue_capacity)
            .finish()
    }
}

impl fmt::Display for PtyOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PtyOptions {{ shell: {:?}, cwd: {}, size: {}x{}, env: {} vars }}",
            self.shell,
            self.cwd.display(),
            self.size.rows,
            self.size.cols,
            self.env.len()
        )
    }
}

impl PtyOptions {
    /// Default options for a shell in `cwd`.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            env_remove: Vec::new(),
            size: PtySize::default(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            write_queue_capacity: DEFAULT_WRITE_QUEUE_CAPACITY,
        }
    }

    pub fn with_shell(mut self, shell: impl Into<OsString>) -> Self {
        self.shell = Some(shell.into());
        self
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn without_env(mut self, key: impl Into<OsString>) -> Self {
        self.env_remove.push(key.into());
        self
    }

    pub fn with_size(mut self, size: PtySize) -> Self {
        self.size = size;
        self
    }

    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity.max(1);
        self
    }

    /// Set the bounded input queue capacity in chunks.
    pub fn with_write_queue_capacity(mut self, capacity: usize) -> Self {
        self.write_queue_capacity = capacity.max(1);
        self
    }
}

/// Events read from a PTY.
///
/// Debug output reports the size of output chunks rather than their contents:
/// terminal output can contain arbitrary user or agent text and must not be
/// copied into a log by an accidental `{:?}`.
#[derive(Clone, PartialEq, Eq)]
pub enum PtyEvent {
    /// Output bytes. Chunk boundaries have no protocol meaning.
    Data(Vec<u8>),
    /// The child side closed its output.
    Eof,
    /// The read failed; the child is usually gone.
    ReadError(String),
    /// The child exited and was reaped. Returned by [`PtyProcess::poll`] after
    /// queued output is drained; later polls may report it again.
    Exited(ExitStatus),
    /// The monitor thread could not observe the child. The child may still be
    /// running, but its exit state is unknown until it is killed.
    MonitorError(String),
}

impl fmt::Debug for PtyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(bytes) => write!(f, "Data(<{} bytes>)", bytes.len()),
            Self::Eof => f.write_str("Eof"),
            Self::ReadError(message) => f.debug_tuple("ReadError").field(message).finish(),
            Self::Exited(status) => f.debug_tuple("Exited").field(status).finish(),
            Self::MonitorError(message) => f.debug_tuple("MonitorError").field(message).finish(),
        }
    }
}

/// PTY adapter errors.
#[derive(Debug)]
pub enum PtyError {
    Platform(PlatformError),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Timeout,
    Closed,
    /// The input queue is full. No bytes were enqueued; retry the whole write
    /// later. The PTY was not touched.
    WouldBlock,
    Monitor(String),
}

impl PtyError {
    fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Platform(error) => write!(f, "{error}"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Timeout => f.write_str("PTY operation timed out"),
            Self::Closed => f.write_str("PTY channel closed"),
            Self::WouldBlock => f.write_str("PTY write queue is full"),
            Self::Monitor(message) => write!(f, "PTY child monitor failed: {message}"),
        }
    }
}

impl std::error::Error for PtyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Platform(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<PlatformError> for PtyError {
    fn from(error: PlatformError) -> Self {
        Self::Platform(error)
    }
}

impl From<io::Error> for PtyError {
    fn from(source: io::Error) -> Self {
        Self::io("io", source)
    }
}

/// A running shell in a PTY.
pub struct PtyProcess {
    session: Arc<dyn PtySession>,
    write_queue: Arc<WriteQueue>,
    options: PtyOptions,
    data_rx: Receiver<PtyEvent>,
    exit: Arc<ExitState>,
    reader_thread: Option<thread::JoinHandle<()>>,
    monitor_thread: Option<thread::JoinHandle<()>>,
    writer_thread: Option<thread::JoinHandle<()>>,
    launch_plan: made_platform::ShellLaunchPlan,
}

impl fmt::Debug for PtyProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyProcess")
            .field("pid", &self.pid())
            .field("options", &self.options)
            .field("exit", &self.exit_status())
            .field("monitor_error", &self.monitor_error())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for PtyProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PtyProcess {{ pid: {:?}, running: {} }}",
            self.pid(),
            self.is_running()
        )
    }
}

impl PtyProcess {
    /// Spawn the shell described by `options`.
    pub fn spawn(platform: &dyn Platform, options: PtyOptions) -> Result<Self, PtyError> {
        let ShellSpec { program, mut args } = platform.resolve_shell(options.shell.as_deref())?;
        args.extend(options.args.iter().cloned());

        let environment = platform.environment(&options.env, &options.env_remove);
        let request = PtyRequest {
            program,
            args,
            cwd: options.cwd.clone(),
            env: environment.into_vec(),
            env_remove: Vec::new(),
            size: options.size,
        };

        let launch_plan = made_platform::ShellLaunchPlan::new(request);
        let session: Arc<dyn PtySession> = Arc::from(platform.spawn_pty(launch_plan.request())?);
        let reader = match session.take_reader() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = session.kill();
                return Err(error.into());
            }
        };
        let writer = match session.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = session.kill();
                return Err(error.into());
            }
        };

        let capacity = options.channel_capacity.max(1);
        let (data_tx, data_rx) = mpsc::sync_channel(capacity);
        let exit = Arc::new(ExitState::default());
        let write_queue = Arc::new(WriteQueue::new(options.write_queue_capacity.max(1)));

        let reader_thread = match spawn_reader(reader, data_tx) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = session.kill();
                return Err(error);
            }
        };
        let monitor_thread = match spawn_monitor(session.clone(), exit.clone()) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = session.kill();
                return Err(error);
            }
        };
        let writer_thread = match spawn_writer(writer, write_queue.clone()) {
            Ok(handle) => handle,
            Err(error) => {
                write_queue.close();
                let _ = session.kill();
                return Err(error);
            }
        };

        Ok(Self {
            session,
            write_queue,
            options,
            data_rx,
            exit,
            reader_thread: Some(reader_thread),
            monitor_thread: Some(monitor_thread),
            writer_thread: Some(writer_thread),
            launch_plan,
        })
    }

    /// Options this process was launched with.
    pub fn options(&self) -> &PtyOptions {
        &self.options
    }

    /// Launch-local reporting capability; actual cwd is received by TerminalCore.
    pub fn shell_cwd_capability(&self) -> &made_platform::ShellCwdCapability {
        self.launch_plan.capability()
    }

    /// Private startup resource directory, retained until this process is dropped.
    pub fn bootstrap_directory(&self) -> Option<&std::path::Path> {
        self.launch_plan.bootstrap_directory()
    }

    /// Enqueue input bytes for the child without blocking on PTY I/O.
    ///
    /// Bytes are split into chunks of at most [`WRITE_CHUNK_MAX`] and appended
    /// atomically to a bounded FIFO queue drained by a dedicated writer thread
    /// in order. If the whole input does not fit, no bytes are enqueued and
    /// [`PtyError::WouldBlock`] is returned; retry the whole slice later. A
    /// single slice larger than `write_queue_capacity * WRITE_CHUNK_MAX` always
    /// returns `WouldBlock`; split it or raise the queue capacity. The method
    /// may briefly contend on the queue mutex but never waits for the child or
    /// performs a blocking write.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_queue.enqueue(bytes)
    }

    /// Resize the PTY and notify the child.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.session.resize(PtySize { rows, cols })?;
        self.options.size = PtySize { rows, cols };
        Ok(())
    }

    /// Query the kernel's current PTY size.
    pub fn size(&self) -> Result<PtySize, PtyError> {
        self.session.size().map_err(Into::into)
    }

    /// Non-blocking output poll.
    ///
    /// Once the monitor thread has failed, queued data is still delivered, but
    /// a terminal `Eof` is reported as [`PtyEvent::MonitorError`] instead. This
    /// keeps `poll`, [`Self::recv_timeout`], [`Self::is_running`], and
    /// [`Self::wait_timeout`] consistent when the reader reaches EOF before the
    /// monitor failure is observed.
    pub fn poll(&self) -> Option<PtyEvent> {
        match self.data_rx.try_recv() {
            Ok(event) => Some(self.bias_monitor_event(event)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                if let Some(error) = self.monitor_error() {
                    Some(PtyEvent::MonitorError(error))
                } else {
                    self.exit_status().map(PtyEvent::Exited)
                }
            }
        }
    }

    /// Wait for the next output event.
    ///
    /// A monitor failure is reported as [`PtyError::Monitor`] rather than a
    /// timeout, a closed channel, or a buffered `Eof`. Already-queued data is
    /// still returned first; the monitor error wins over the terminal EOF so
    /// callers cannot mistake a failed monitor for a clean child exit.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<PtyEvent, PtyError> {
        // Preserve already-buffered data, but if the monitor has already failed
        // and only an EOF remains, do not let the EOF hide the failure.
        match self.data_rx.try_recv() {
            Ok(event) => return self.monitor_result(event),
            Err(TryRecvError::Disconnected) => {
                return match self.monitor_error() {
                    Some(error) => Err(PtyError::Monitor(error)),
                    None => Err(PtyError::Closed),
                };
            }
            Err(TryRecvError::Empty) => {}
        }
        if let Some(error) = self.monitor_error() {
            return Err(PtyError::Monitor(error));
        }

        match self.data_rx.recv_timeout(timeout) {
            Ok(event) => self.monitor_result(event),
            Err(RecvTimeoutError::Timeout) => match self.monitor_error() {
                Some(error) => Err(PtyError::Monitor(error)),
                None => Err(PtyError::Timeout),
            },
            Err(RecvTimeoutError::Disconnected) => match self.monitor_error() {
                Some(error) => Err(PtyError::Monitor(error)),
                None => Err(PtyError::Closed),
            },
        }
    }

    /// Replace a terminal `Eof` with a monitor failure when one is present.
    fn bias_monitor_event(&self, event: PtyEvent) -> PtyEvent {
        match event {
            PtyEvent::Eof => match self.monitor_error() {
                Some(error) => PtyEvent::MonitorError(error),
                None => PtyEvent::Eof,
            },
            event => event,
        }
    }

    /// Like [`Self::bias_monitor_event`], but returns the monitor error as the
    /// typed error used by blocking reads.
    fn monitor_result(&self, event: PtyEvent) -> Result<PtyEvent, PtyError> {
        match self.bias_monitor_event(event) {
            PtyEvent::MonitorError(error) => Err(PtyError::Monitor(error)),
            event => Ok(event),
        }
    }

    /// Drain all immediately available output bytes.
    pub fn drain_output(&self) -> Vec<u8> {
        let mut output = Vec::new();
        while let Some(event) = self.poll() {
            match event {
                PtyEvent::Data(bytes) => output.extend_from_slice(&bytes),
                PtyEvent::Eof
                | PtyEvent::ReadError(_)
                | PtyEvent::Exited(_)
                | PtyEvent::MonitorError(_) => break,
            }
        }
        output
    }

    /// Current exit status, if the child has exited and been reaped.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit.status()
    }

    /// Monitor-thread error, if observing the child failed.
    ///
    /// A monitor failure is distinct from a child exit: [`Self::exit_status`]
    /// remains `None`, and callers must not treat the child as running or as
    /// cleanly exited.
    pub fn monitor_error(&self) -> Option<String> {
        self.exit.error()
    }

    /// Writer-thread error, if the PTY write side failed.
    ///
    /// Afterwards [`Self::write`] returns [`PtyError::Closed`]. The message
    /// contains only the OS error, never input bytes.
    pub fn writer_error(&self) -> Option<String> {
        self.write_queue.error()
    }

    /// Whether the child is still running.
    ///
    /// Returns `false` when the monitor thread has failed, because the child's
    /// state is then unknown. Use [`Self::try_is_running`],
    /// [`Self::monitor_error`], or [`PtyEvent::MonitorError`] to distinguish a
    /// monitor failure from a normal exit.
    pub fn is_running(&self) -> bool {
        self.try_is_running().unwrap_or(false)
    }

    /// Whether the child is still running, surfacing monitor failures.
    pub fn try_is_running(&self) -> Result<bool, PtyError> {
        self.exit.running()
    }

    /// Child process id.
    pub fn pid(&self) -> Option<u32> {
        self.session.pid()
    }

    /// Block until the child exits or `timeout` elapses.
    ///
    /// `None` waits without a deadline.
    pub fn wait_timeout(&self, timeout: Option<Duration>) -> Result<ExitStatus, PtyError> {
        self.exit.wait(timeout)
    }

    /// Kill the child immediately.
    ///
    /// Also closes the input queue, so later writes return
    /// [`PtyError::Closed`]. Pending queued input is discarded.
    pub fn kill(&mut self) -> Result<(), PtyError> {
        let result = self.session.kill().map_err(Into::into);
        self.write_queue.close();
        result
    }

    /// Kill the child and wait for it to be reaped.
    ///
    /// The input writer is closed as part of termination. On Unix the platform
    /// layer also terminates the stored PTY process group and the current
    /// foreground job group, including when the direct shell already exited but
    /// descendants remain; see [`made_platform::PtySession::kill`].
    pub fn terminate(&mut self) -> Result<ExitStatus, PtyError> {
        if let Some(status) = self.exit_status() {
            // The shell has exited, but the platform may still own descendants
            // in its process group. Ask it to clean up before reporting.
            let _ = self.kill();
            self.close_writer();
            return Ok(status);
        }
        if let Err(error) = self.kill() {
            // The child may have exited between the status check and the kill.
            if let Ok(status) = self.wait_timeout(Some(Duration::from_millis(250))) {
                self.close_writer();
                return Ok(status);
            }
            return Err(error);
        }
        let result = self.wait_timeout(Some(Duration::from_secs(3)));
        self.close_writer();
        result
    }

    /// Close the input queue and join the writer when it exits promptly.
    ///
    /// A writer blocked in a PTY write is released by child termination. The
    /// join is bounded anyway so a pathological platform write cannot hang
    /// window close; in that case the writer handle is detached.
    fn close_writer(&mut self) {
        self.write_queue.close();
        let done = self.write_queue.wait_done(Duration::from_secs(1));
        if let Some(handle) = self.writer_thread.take() {
            if done {
                let _ = handle.join();
            }
        }
    }

    /// Replace this process with a fresh shell using the same options.
    ///
    /// The old child is terminated by the drop of the replaced process. This is
    /// the relaunch path after a shell exits.
    pub fn relaunch(&mut self, platform: &dyn Platform) -> Result<(), PtyError> {
        let fresh = Self::spawn(platform, self.options.clone())?;
        let old = std::mem::replace(self, fresh);
        drop(old);
        Ok(())
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        // Run platform cleanup even when the direct shell already exited; its
        // process group may still contain descendants.
        let _ = self.session.kill();
        if self.exit_status().is_none() {
            let _ = self.exit.wait(Some(Duration::from_secs(2)));
        }
        self.close_writer();
        // The monitor exits as soon as the child is reaped. The reader may be
        // blocked on the bounded channel when the UI stopped draining; dropping
        // the receiver (at the end of this drop) unblocks it, so it is detached
        // rather than joined. This guarantees window close never hangs.
        if self.exit_status().is_some() {
            if let Some(handle) = self.monitor_thread.take() {
                let _ = handle.join();
            }
        }
        self.reader_thread.take();
    }
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    sender: SyncSender<PtyEvent>,
) -> Result<thread::JoinHandle<()>, PtyError> {
    thread::Builder::new()
        .name("made-pty-reader".into())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(PtyEvent::Eof);
                        break;
                    }
                    Ok(read) => {
                        if sender
                            .send(PtyEvent::Data(buffer[..read].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = sender.send(PtyEvent::ReadError(error.to_string()));
                        break;
                    }
                }
            }
        })
        .map_err(|error| PtyError::io("spawn PTY reader thread", error))
}

fn spawn_monitor(
    session: Arc<dyn PtySession>,
    exit: Arc<ExitState>,
) -> Result<thread::JoinHandle<()>, PtyError> {
    thread::Builder::new()
        .name("made-pty-monitor".into())
        .spawn(move || loop {
            match session.try_wait() {
                Ok(Some(status)) => {
                    exit.set_status(status);
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(15)),
                Err(error) => {
                    exit.set_error(error.to_string());
                    break;
                }
            }
        })
        .map_err(|error| PtyError::io("spawn PTY monitor thread", error))
}

fn spawn_writer(
    mut writer: Box<dyn Write + Send>,
    queue: Arc<WriteQueue>,
) -> Result<thread::JoinHandle<()>, PtyError> {
    thread::Builder::new()
        .name("made-pty-writer".into())
        .spawn(move || {
            while let Some(chunk) = queue.next() {
                if let Err(error) = writer.write_all(&chunk).and_then(|()| writer.flush()) {
                    queue.fail(format!("PTY write failed: {error}"));
                    break;
                }
            }
            queue.mark_done();
        })
        .map_err(|error| PtyError::io("spawn PTY writer thread", error))
}

/// Bounded FIFO input queue drained by the writer thread.
///
/// The queue is owned by `PtyProcess` (shared with the writer). Enqueueing is
/// all-or-nothing: a write that does not fit in the remaining capacity returns
/// `WouldBlock` without enqueueing a prefix, which keeps retries idempotent.
struct WriteQueue {
    state: Mutex<WriteQueueState>,
    ready: Condvar,
}

struct WriteQueueState {
    chunks: VecDeque<Vec<u8>>,
    capacity: usize,
    closed: bool,
    failed: Option<String>,
    done: bool,
}

impl WriteQueue {
    fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(WriteQueueState {
                chunks: VecDeque::new(),
                capacity: capacity.max(1),
                closed: false,
                failed: None,
                done: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn enqueue(&self, bytes: &[u8]) -> Result<(), PtyError> {
        if bytes.is_empty() {
            return Ok(());
        }

        let mut state = self.lock();
        if state.closed {
            return Err(PtyError::Closed);
        }
        // Reject input that cannot fit before allocating chunk copies. This
        // also keeps the caller's cost bounded for a huge paste.
        let max_bytes = state.capacity.saturating_mul(WRITE_CHUNK_MAX);
        let chunk_count = bytes.len().div_ceil(WRITE_CHUNK_MAX);
        if bytes.len() > max_bytes || state.chunks.len() + chunk_count > state.capacity {
            return Err(PtyError::WouldBlock);
        }
        for chunk in bytes.chunks(WRITE_CHUNK_MAX) {
            state.chunks.push_back(chunk.to_vec());
        }
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    fn next(&self) -> Option<Vec<u8>> {
        let mut state = self.lock();
        loop {
            // A close is a shutdown signal: discard queued-but-unwritten input
            // and let the current blocking write finish or fail. This keeps
            // window close bounded even when the child stopped reading.
            if state.closed {
                return None;
            }
            if let Some(chunk) = state.chunks.pop_front() {
                return Some(chunk);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        drop(state);
        self.ready.notify_all();
    }

    fn fail(&self, message: String) {
        let mut state = self.lock();
        state.failed = Some(message);
        state.closed = true;
        drop(state);
        self.ready.notify_all();
    }

    fn error(&self) -> Option<String> {
        self.lock().failed.clone()
    }

    fn mark_done(&self) {
        let mut state = self.lock();
        state.done = true;
        drop(state);
        self.ready.notify_all();
    }

    fn wait_done(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while !state.done {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, result) = self
                .ready
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(|error| error.into_inner());
            state = guard;
            if result.timed_out() && !state.done {
                return false;
            }
        }
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WriteQueueState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[derive(Default)]
struct ExitState {
    inner: Mutex<ExitInner>,
    changed: Condvar,
}

#[derive(Default)]
struct ExitInner {
    status: Option<ExitStatus>,
    error: Option<String>,
}

impl ExitState {
    fn status(&self) -> Option<ExitStatus> {
        self.lock().status.clone()
    }

    fn error(&self) -> Option<String> {
        self.lock().error.clone()
    }

    fn running(&self) -> Result<bool, PtyError> {
        let inner = self.lock();
        if let Some(error) = &inner.error {
            return Err(PtyError::Monitor(error.clone()));
        }
        Ok(inner.status.is_none())
    }

    fn set_status(&self, status: ExitStatus) {
        let mut inner = self.lock();
        inner.status = Some(status);
        drop(inner);
        self.changed.notify_all();
    }

    fn set_error(&self, error: String) {
        let mut inner = self.lock();
        inner.error = Some(error);
        drop(inner);
        self.changed.notify_all();
    }

    fn wait(&self, timeout: Option<Duration>) -> Result<ExitStatus, PtyError> {
        let deadline = timeout.map(|duration| Instant::now() + duration);
        let mut inner = self.lock();
        loop {
            if let Some(status) = &inner.status {
                return Ok(status.clone());
            }
            if let Some(error) = &inner.error {
                return Err(PtyError::Monitor(error.clone()));
            }
            inner = match deadline {
                None => self
                    .changed
                    .wait(inner)
                    .unwrap_or_else(|err| err.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(PtyError::Timeout);
                    }
                    let (guard, result) = self
                        .changed
                        .wait_timeout(inner, deadline.saturating_duration_since(now))
                        .unwrap_or_else(|err| err.into_inner());
                    if result.timed_out() && guard.status.is_none() && guard.error.is_none() {
                        return Err(PtyError::Timeout);
                    }
                    guard
                }
            };
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ExitInner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}
