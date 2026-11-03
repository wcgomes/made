//! Native per-tab owner. The supervisor process, not the SSH frontend, holds the
//! PTY. Frontend death drops the attachment only.
//!
//! No tmux binary, socket, config, probe, or fallback.
//!
//! # Platform contracts observed in this tree
//!
//! - Detach is `made_platform::spawn_detached_command` (module `detach` is
//!   private; that function is the re-export). It starts a new session, ignores
//!   SIGHUP, closes inherited stdio, and uses a private directory. The frontend
//!   calls it. This module does not reimplement OS detach.
//! - Endpoint publish uses the existing `write_private_metadata_atomic` /
//!   `read_private_metadata`. The draft helpers `write_private` and
//!   `read_private_file` are not the atomic API.
//! - `PtyProcess` is the owner. It is the public adapter over
//!   `Platform::spawn_pty` / `PtySession`. Dropping it is the scoped close;
//!   dropping an attachment does not drop it.
//! - `CwdReportSink` exists on `made_platform::shell_integration` but that
//!   module is still private. The import is the expected public path once
//!   platform registers it (task 3.1). No `cfg(target_os)` is added here.
use crate::{
    clipboard::Uploads, filesystem::Filesystem, git::GitRunner, integration::Integration,
    protocol::*,
};
use made_bridge::remote_rpc::{RpcConnection, RpcListener};
use made_platform::shell_integration::{CwdReportSink, ShellLaunchPlan};
use made_platform::PtySize;
use made_terminal::{
    CellSnapshot, CursorShape, InputModes, LineSnapshot, MouseMode, PtyEvent, PtyOptions,
    PtyProcess, TerminalCore, TerminalSize, TerminalSnapshot,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Scrollback bound shared with the local terminal. Not a replay log.
pub const SCROLLBACK_LINES: usize = 10_000;

#[derive(Debug)]
pub struct Lease {
    renewed: Instant,
}
impl Lease {
    pub fn new(now: Instant) -> Self {
        Self { renewed: now }
    }
    pub fn renew(&mut self, now: Instant) {
        self.renewed = now;
    }
    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.renewed) >= DETACHED_LEASE
    }
}

#[derive(Serialize, Deserialize)]
pub struct ChildLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

#[derive(Serialize, Deserialize)]
pub struct Endpoint {
    pub addr: SocketAddr,
}

pub fn capability_bytes(identity: &Identity) -> Result<[u8; 32], ErrorCode> {
    serde_json::from_value(serde_json::to_value(&identity.tab).map_err(|_| ErrorCode::Io)?)
        .map_err(|_| ErrorCode::Io)
}

pub fn runtime_directory(base: &Path, identity: &Identity) -> Result<PathBuf, ErrorCode> {
    let capability = capability_bytes(identity)?;
    // Public one-way locator; never put the bearer capability in argv. A
    // collision refuses Open (exclusive directory creation), never aliases tabs.
    let name = made_bridge::Token::from_bytes(capability)
        .fingerprint()
        .replace(':', "-");
    Ok(base.join("remote/runtime").join(name))
}

fn io_error(_: impl std::fmt::Debug) -> ErrorCode {
    ErrorCode::Io
}

fn publish_private(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), ErrorCode> {
    made_platform::write_private_metadata_atomic(directory, name, bytes, bytes.len().max(1))
        .map_err(io_error)
}

/// Bounded VT plus the PTY child. The attachment does not own this value.
pub struct NativeSession {
    process: PtyProcess,
    vt: VtState,
}

impl NativeSession {
    pub fn spawn(
        platform: &dyn made_platform::Platform,
        options: PtyOptions,
    ) -> Result<Self, ErrorCode> {
        let rows = options.size.rows;
        let cols = options.size.cols;
        let process = PtyProcess::spawn(platform, options).map_err(io_error)?;
        Ok(Self {
            process,
            vt: VtState::new(rows, cols),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.pid()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), ErrorCode> {
        self.process.resize(rows, cols).map_err(io_error)?;
        self.vt.resize(rows, cols);
        Ok(())
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), ErrorCode> {
        self.process.write(bytes).map_err(|_| ErrorCode::Limit)
    }

    pub fn terminate(&mut self) -> bool {
        self.process.terminate().is_ok()
    }

    /// Feed already-read PTY bytes into the bounded core. Raw bytes are not kept.
    pub fn observe(&mut self, bytes: &[u8]) {
        self.vt.feed(bytes);
    }

    pub fn poll(&mut self) -> Option<PtyEvent> {
        self.process.poll()
    }

    pub fn resnapshot(&self) -> Vec<u8> {
        self.vt.resnapshot()
    }
}

/// Primary grid retained across alternate-screen entry. Alacritty does not
/// expose the inactive grid, so the transition is split before it is lost.
pub struct VtState {
    core: TerminalCore,
    primary: PrimaryRetention,
}

struct PrimaryRetention {
    history: Vec<LineSnapshot>,
    screen: TerminalSnapshot,
    modes: InputModes,
}

impl VtState {
    pub fn new(rows: u16, cols: u16) -> Self {
        let mut core = TerminalCore::with_size(size_of(rows, cols), SCROLLBACK_LINES);
        core.resize(usize::from(rows.max(1)), usize::from(cols.max(1)));
        let primary = PrimaryRetention::capture(&core);
        Self { core, primary }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.core
            .resize(usize::from(rows.max(1)), usize::from(cols.max(1)));
        if !self.core.alternate_screen() {
            self.primary = PrimaryRetention::capture(&self.core);
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0x1b {
                if !self.core.alternate_screen() {
                    self.primary = PrimaryRetention::capture(&self.core);
                }
                let end = index + sequence_end(&bytes[index..]);
                self.core.feed(&bytes[index..end]);
                index = end;
            } else {
                let next = bytes[index..]
                    .iter()
                    .position(|byte| *byte == 0x1b)
                    .map(|offset| index + offset)
                    .unwrap_or(bytes.len());
                self.core.feed(&bytes[index..next]);
                index = next;
            }
            if !self.core.alternate_screen() {
                self.primary = PrimaryRetention::capture(&self.core);
            }
        }
    }

    pub fn resnapshot(&self) -> Vec<u8> {
        encode_resnapshot(&self.core, &self.primary)
    }

    pub fn alternate_screen(&self) -> bool {
        self.core.alternate_screen()
    }

    pub fn modes(&self) -> InputModes {
        self.core.input_modes()
    }

    pub fn screen_text(&self) -> String {
        self.core.screen_text()
    }
}

impl PrimaryRetention {
    fn capture(core: &TerminalCore) -> Self {
        Self {
            history: core.history(SCROLLBACK_LINES),
            screen: core.snapshot(),
            modes: core.input_modes(),
        }
    }
}

fn size_of(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize {
        rows: usize::from(rows.max(1)),
        cols: usize::from(cols.max(1)),
    }
}

fn sequence_end(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    if bytes[0] != 0x1b {
        return 1;
    }
    if bytes.len() == 1 {
        return 1;
    }
    match bytes[1] {
        b'[' => {
            let mut index = 2;
            while index < bytes.len() && index < 66 {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            index
        }
        b']' => {
            let mut index = 2;
            while index < bytes.len() && index < 258 {
                if bytes[index] == 0x07 {
                    return index + 1;
                }
                if bytes[index] == 0x1b && index + 1 < bytes.len() && bytes[index + 1] == b'\\' {
                    return index + 2;
                }
                index += 1;
            }
            index
        }
        _ => 2.min(bytes.len()),
    }
}

fn encode_resnapshot(core: &TerminalCore, primary: &PrimaryRetention) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[0m");
    paint_history(&mut out, &primary.history, primary.screen.size.rows);
    out.extend_from_slice(b"\x1b[H\x1b[2J");
    paint_viewport(&mut out, &primary.screen);
    push_modes(&mut out, &primary.modes);
    push_cursor(&mut out, &primary.screen);
    if core.alternate_screen() {
        out.extend_from_slice(b"\x1b[?1049h");
        let alternate = core.snapshot();
        out.extend_from_slice(b"\x1b[H\x1b[2J");
        paint_viewport(&mut out, &alternate);
        push_modes(&mut out, &core.input_modes());
        push_cursor(&mut out, &alternate);
    }
    out
}

fn paint_history(out: &mut Vec<u8>, history: &[LineSnapshot], rows: usize) {
    if history.is_empty() || rows == 0 {
        return;
    }
    push_cup(out, rows - 1, 0);
    for line in history {
        push_line_text(out, line);
        if !line.wrapped {
            out.extend_from_slice(b"\r\n");
        }
    }
}

fn paint_viewport(out: &mut Vec<u8>, snapshot: &TerminalSnapshot) {
    let cols = snapshot.size.cols;
    for row in 0..snapshot.size.rows {
        push_cup(out, row, 0);
        let start = row * cols;
        push_cells(out, &snapshot.cells[start..start + cols]);
    }
}

fn push_line_text(out: &mut Vec<u8>, line: &LineSnapshot) {
    push_cells(out, &line.cells);
}

fn push_cells(out: &mut Vec<u8>, cells: &[CellSnapshot]) {
    for cell in cells {
        if cell.wide_spacer {
            continue;
        }
        push_char(out, cell.ch);
        for extra in &cell.combining {
            push_char(out, *extra);
        }
    }
}

fn push_char(out: &mut Vec<u8>, ch: char) {
    let ch = if ch.is_control() { ' ' } else { ch };
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

fn push_cup(out: &mut Vec<u8>, row: usize, col: usize) {
    let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
}

fn push_modes(out: &mut Vec<u8>, modes: &InputModes) {
    private_mode(out, 1, modes.app_cursor);
    if modes.app_keypad {
        out.extend_from_slice(b"\x1b=");
    } else {
        out.extend_from_slice(b"\x1b>");
    }
    private_mode(out, 1000, false);
    private_mode(out, 1002, false);
    private_mode(out, 1003, false);
    let mouse = match modes.mouse_mode {
        MouseMode::Click => Some(1000),
        MouseMode::Drag => Some(1002),
        MouseMode::Motion => Some(1003),
        MouseMode::Off => None,
    };
    if let Some(mode) = mouse {
        private_mode(out, mode, true);
    }
    private_mode(out, 1006, modes.sgr_mouse);
    private_mode(out, 1004, modes.focus_reporting);
    private_mode(out, 2004, modes.bracketed_paste);
    if modes.csi_u {
        out.extend_from_slice(b"\x1b[=1u");
    } else {
        out.extend_from_slice(b"\x1b[=0u");
    }
}

fn private_mode(out: &mut Vec<u8>, mode: u16, on: bool) {
    let flag = if on { 'h' } else { 'l' };
    let _ = write!(out, "\x1b[?{mode}{flag}");
}

fn push_cursor(out: &mut Vec<u8>, snapshot: &TerminalSnapshot) {
    let cursor = &snapshot.cursor;
    if cursor.shape == CursorShape::Hidden || !cursor.visible {
        out.extend_from_slice(b"\x1b[?25l");
    } else {
        out.extend_from_slice(b"\x1b[?25h");
        let style = match (cursor.shape, cursor.blinking) {
            (CursorShape::Underline, true) => 3,
            (CursorShape::Underline, false) => 4,
            (CursorShape::Bar, true) => 5,
            (CursorShape::Bar, false) => 6,
            (_, true) => 1,
            (_, false) => 2,
        };
        let _ = write!(out, "\x1b[{style} q");
    }
    push_cup(out, cursor.row, cursor.col);
}

struct OutputQueue {
    control: SyncSender<WireEnvelope>,
    data: SyncSender<WireEnvelope>,
}

impl OutputQueue {
    // The unsent frame has to travel in `TrySendError` so the caller can keep it.
    #[allow(clippy::result_large_err)]
    fn try_send(&self, frame: WireEnvelope) -> Result<(), TrySendError<WireEnvelope>> {
        if is_data(&frame.operation) {
            self.data.try_send(frame)
        } else {
            self.control.try_send(frame)
        }
    }
}

/// Next resnapshot frame. `completed` stays false until the single `done: true`
/// frame is produced. Live output must not be emitted before that.
pub fn next_snapshot_frame(rest: &[u8], completed: bool) -> Option<(Operation, usize, bool)> {
    if completed {
        return None;
    }
    if rest.is_empty() {
        let data = Chunk::new(Vec::new()).ok()?;
        return Some((Operation::TerminalOutput { data, done: true }, 0, true));
    }
    let take = MAX_CHUNK_BYTES.min(rest.len());
    let data = Chunk::new(rest[..take].to_vec()).ok()?;
    Some((Operation::TerminalOutput { data, done: false }, take, false))
}

/// Live PTY bytes after the resnapshot. Never a second `done: true`, never empty.
pub fn live_terminal_output(bytes: &[u8]) -> Option<Operation> {
    if bytes.is_empty() {
        return None;
    }
    let data = Chunk::new(bytes.to_vec()).ok()?;
    Some(Operation::TerminalOutput { data, done: false })
}

fn is_data(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::TerminalOutput { .. }
            | Operation::Listed { .. }
            | Operation::GitResult { .. }
            | Operation::PreviewResult { .. }
    )
}

struct Attachment {
    stream: TcpStream,
    input: Receiver<WireEnvelope>,
    output: OutputQueue,
    identity: Identity,
    request: u64,
    watch_request: Option<u64>,
    snapshot_rest: Vec<u8>,
    /// Set only after the single completing `done: true` frame is queued.
    snapshot_closed: bool,
    pending_relay: Option<Vec<u8>>,
    pending_control: Vec<WireEnvelope>,
}

/// Heartbeat, terminal I/O, and close are not filesystem-generation scoped.
pub fn generation_exempt(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::Heartbeat
            | Operation::Capabilities
            | Operation::Close
            | Operation::Cancel { .. }
            | Operation::TerminalInput { .. }
            | Operation::TerminalResize { .. }
            | Operation::SessionState
            | Operation::Focus { .. }
            | Operation::UploadBegin { .. }
            | Operation::UploadChunk { .. }
            | Operation::UploadCommit { .. }
            | Operation::UploadAbort { .. }
    )
}

pub fn transport_matches(request: &Identity, current: &Identity) -> bool {
    request.tab.authenticates(&current.tab)
        && request.authority == current.authority
        && request.epoch == current.epoch
}

pub struct Supervisor {
    identity: Identity,
    runtime: PathBuf,
    session: NativeSession,
    filesystem: Filesystem,
    uploads: Uploads,
    integration: Option<Integration>,
    lease: Lease,
    attachment: Option<Attachment>,
    attached_once: bool,
    cwd_sink: CwdReportSink,
    shell_plan: Option<ShellLaunchPlan>,
    shell_cwd: Option<RemotePath>,
    closed: bool,
}

impl Supervisor {
    pub fn create(
        first: &WireEnvelope,
        runtime: PathBuf,
        data: &Path,
        companion: &Path,
        launch: ChildLaunch,
        integration: Option<Integration>,
    ) -> Result<Self, ErrorCode> {
        let Operation::Open {
            directory,
            rows,
            cols,
            ..
        } = &first.operation
        else {
            return Err(ErrorCode::InvalidRequest);
        };
        first.validate().map_err(|_| ErrorCode::InvalidRequest)?;
        let filesystem = Filesystem::new(directory, first.identity.generation)?;
        let uploads = Uploads::new(data.join("remote/clipboard-images"))?;
        let cwd_sink = CwdReportSink::new(&runtime).map_err(io_error)?;
        let mut launch = launch;
        let shell_plan = if matches!(
            first.operation,
            Operation::Open {
                target: LaunchTarget::Shell,
                ..
            }
        ) {
            let mut request =
                made_platform::PtyRequest::new(launch.program.as_os_str(), &launch.cwd);
            request.args = launch.args;
            request.env = launch.env;
            let plan =
                ShellLaunchPlan::with_cwd_sink(request, companion.into(), cwd_sink.path().into());
            launch.args = plan.request().args.clone();
            launch.env = plan.request().env.clone();
            Some(plan)
        } else {
            None
        };
        let child_bytes = serde_json::to_vec(&launch).map_err(io_error)?;
        publish_private(&runtime, "child.json", &child_bytes)?;
        let child_path = runtime.join("child.json");
        let options = PtyOptions::new(&launch.cwd)
            .with_shell(companion)
            .with_args([OsString::from("--child"), child_path.into_os_string()])
            .with_size(PtySize {
                rows: *rows,
                cols: *cols,
            });
        let session = NativeSession::spawn(&made_platform::host_platform(), options)?;
        Ok(Self {
            identity: first.identity.clone(),
            runtime,
            session,
            filesystem,
            uploads,
            integration,
            lease: Lease::new(Instant::now()),
            attachment: None,
            attached_once: false,
            cwd_sink,
            shell_plan,
            shell_cwd: Some(directory.clone()),
            closed: false,
        })
    }

    fn install_attachment(&mut self, connection: RpcConnection) -> Result<(), ErrorCode> {
        let epoch = connection.epoch();
        let mut stream = connection.into_stream();
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(io_error)?;
        let first = read_envelope(&mut stream).map_err(io_error)?;
        // A rejected attach must be distinguishable from a dead transport.
        // Always answer with the code after the connection authenticated.
        let reject = |stream: &mut TcpStream, code: ErrorCode| {
            let frame = WireEnvelope::new(
                first.identity.clone(),
                first.request_id,
                Operation::Error { code },
            );
            let _ = write_envelope(stream, &frame);
            code
        };
        if epoch < self.identity.epoch || (self.attached_once && epoch <= self.identity.epoch) {
            return Err(reject(&mut stream, ErrorCode::Stale));
        }
        if !transport_matches(
            &first.identity,
            &Identity {
                epoch,
                ..self.identity.clone()
            },
        ) || first.identity.epoch != epoch
        {
            return Err(reject(&mut stream, ErrorCode::Unauthorized));
        }
        let (rows, cols) = match &first.operation {
            Operation::Open { rows, cols, .. }
                if self.attachment.is_none() && epoch == self.identity.epoch =>
            {
                (*rows, *cols)
            }
            Operation::Attach { rows, cols } => (*rows, *cols),
            _ => return Err(reject(&mut stream, ErrorCode::InvalidRequest)),
        };
        if let Err(code) = self.session.resize(rows, cols) {
            return Err(reject(&mut stream, code));
        }
        let (input_tx, input) = mpsc::sync_channel(MAX_IN_FLIGHT);
        let (data_tx, data_rx) = mpsc::sync_channel(MAX_QUEUED_CHUNKS);
        let (control_tx, control_rx) = mpsc::sync_channel(MAX_IN_FLIGHT);
        let output = OutputQueue {
            control: control_tx,
            data: data_tx,
        };
        let mut identity = first.identity.clone();
        identity.generation = self.filesystem.generation();
        let attached = WireEnvelope::new(
            first.identity.clone(),
            first.request_id,
            Operation::Attached,
        );
        // Epoch is not current until Attached is actually queued.
        output.try_send(attached).map_err(|_| ErrorCode::Limit)?;
        let snapshot = self.session.resnapshot();
        let mut reader = stream.try_clone().map_err(io_error)?;
        let writer = stream.try_clone().map_err(io_error)?;
        reader.set_read_timeout(None).map_err(io_error)?;
        // Attached is queued and the new streams exist. Only then replace the
        // previous frontend and confirm the epoch.
        self.detach_frontend();
        self.identity.epoch = epoch;
        self.attached_once = true;
        self.identity.generation = self.filesystem.generation();
        self.lease.renew(Instant::now());
        if let Some(integration) = &mut self.integration {
            integration.reset_attachment();
        }
        self.uploads.pending_abort_all();
        thread::spawn(move || {
            while let Ok(frame) = read_envelope(&mut reader) {
                if input_tx.send(frame).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || writer_loop(writer, control_rx, data_rx));
        self.attachment = Some(Attachment {
            stream,
            input,
            output,
            identity,
            request: first.request_id,
            watch_request: None,
            snapshot_rest: snapshot,
            snapshot_closed: false,
            pending_relay: None,
            pending_control: Vec::new(),
        });
        Ok(())
    }

    fn detach_frontend(&mut self) {
        if let Some(attachment) = self.attachment.take() {
            let _ = attachment.stream.shutdown(Shutdown::Both);
            drop(attachment);
        }
    }

    pub fn run(mut self) -> Result<(), ErrorCode> {
        let listener = Arc::new(
            RpcListener::bind(capability_bytes(&self.identity)?, self.identity.authority.0)
                .map_err(io_error)?,
        );
        let endpoint = serde_json::to_vec(&Endpoint {
            addr: listener.local_addr().map_err(io_error)?,
        })
        .map_err(io_error)?;
        publish_private(&self.runtime, "endpoint.json", &endpoint)?;
        let (authenticated, ready) = mpsc::sync_channel(8);
        let pending = Arc::new(AtomicUsize::new(0));
        loop {
            if self.lease.expired(Instant::now()) || self.closed {
                self.detach_frontend();
                let _ = self.session.terminate();
                break;
            }
            if let Some(stream) = listener.accept().map_err(io_error)? {
                if pending.load(Ordering::Relaxed) < 8 {
                    pending.fetch_add(1, Ordering::Relaxed);
                    let listener = Arc::clone(&listener);
                    let pending = Arc::clone(&pending);
                    let authenticated = authenticated.clone();
                    thread::spawn(move || {
                        if let Ok(connection) = listener.authenticate(stream) {
                            let _ = authenticated.try_send(connection);
                        }
                        pending.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
            if let Ok(connection) = ready.try_recv() {
                let _ = self.install_attachment(connection);
            }
            let request = match self
                .attachment
                .as_ref()
                .map(|attachment| attachment.input.try_recv())
            {
                Some(Ok(request)) => Some(request),
                Some(Err(TryRecvError::Disconnected)) => {
                    self.detach_frontend();
                    None
                }
                _ => None,
            };
            if let Some(request) = request {
                if !self.accept_request(&request) {
                    self.reply(
                        &request,
                        Operation::Error {
                            code: ErrorCode::Stale,
                        },
                    );
                } else {
                    let close = matches!(request.operation, Operation::Close);
                    match self.dispatch(&request) {
                        Ok(replies) => {
                            for operation in replies {
                                self.reply(&request, operation);
                            }
                        }
                        Err(code) => self.reply(&request, Operation::Error { code }),
                    }
                    if close {
                        thread::sleep(Duration::from_millis(20));
                        self.detach_frontend();
                        break;
                    }
                }
            }
            self.poll_events();
            thread::sleep(Duration::from_millis(1));
        }
        if let Some(integration) = &self.integration {
            let _ = integration.verify_config();
        }
        let _ = fs::remove_dir_all(&self.runtime);
        Ok(())
    }

    fn accept_request(&self, request: &WireEnvelope) -> bool {
        transport_matches(&request.identity, &self.identity)
            && (request.identity.generation == self.identity.generation
                || generation_exempt(&request.operation))
    }

    fn reply(&mut self, request: &WireEnvelope, operation: Operation) {
        let response = WireEnvelope::new(request.identity.clone(), request.request_id, operation);
        self.enqueue(response);
    }

    /// Full is backpressure: keep the frontend. Only a disconnected queue drops it.
    fn enqueue(&mut self, frame: WireEnvelope) {
        let Some(attachment) = self.attachment.as_mut() else {
            return;
        };
        match attachment.output.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(frame)) => {
                if attachment.pending_control.len() < MAX_IN_FLIGHT {
                    attachment.pending_control.push(frame);
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.detach_frontend();
            }
        }
    }

    fn dispatch(&mut self, request: &WireEnvelope) -> Result<Vec<Operation>, ErrorCode> {
        use Operation::*;
        let reply = match &request.operation {
            Capabilities => CapabilitiesResult {
                capabilities: crate::protocol::Capabilities {
                    target: current_target()?,
                    tmux: false,
                    opencode: self.integration.is_some(),
                    shell_cwd: self
                        .shell_plan
                        .as_ref()
                        .is_some_and(|plan| plan.capability().is_supported()),
                    helper_version: env!("CARGO_PKG_VERSION").into(),
                },
            },
            Heartbeat => {
                self.lease.renew(Instant::now());
                Ack
            }
            Close => {
                self.closed = true;
                Closed {
                    confirmed: self.session.terminate(),
                }
            }
            RootValidate { path } => self.filesystem.validate_root(path)?,
            RootSelect { path } => {
                let result = self.filesystem.select(path)?;
                self.uploads.pending_abort_all();
                self.identity.generation = self.filesystem.generation();
                if let Some(attachment) = &mut self.attachment {
                    attachment.identity.generation = self.identity.generation;
                    attachment.watch_request = None;
                }
                result
            }
            List { path } => return self.filesystem.list(path),
            Preview { path } | Refresh { path } => return self.filesystem.preview(path),
            Watch { path } => {
                self.filesystem.watch(path)?;
                if let Some(attachment) = &mut self.attachment {
                    attachment.watch_request = Some(request.request_id);
                }
                Changed { path: path.clone() }
            }
            Rescan { path } => {
                self.filesystem.list(path)?;
                Changed { path: path.clone() }
            }
            GitRead { command } => {
                let bytes = GitRunner::new()
                    .read_bytes((*command).into(), self.filesystem.root())
                    .map_err(|_| ErrorCode::Unavailable)?;
                return Ok(chunk_git(*command, &bytes));
            }
            SessionState => self
                .integration
                .as_ref()
                .ok_or(ErrorCode::Unavailable)?
                .state()?,
            Focus { session_id } => {
                self.integration
                    .as_mut()
                    .ok_or(ErrorCode::Unavailable)?
                    .focus(request.request_id, session_id)?;
                return Ok(vec![]);
            }
            TerminalInput { data } => {
                self.session.write(data.as_bytes())?;
                Ack
            }
            TerminalResize { rows, cols } => {
                self.session.resize(*rows, *cols)?;
                Ack
            }
            UploadBegin { upload_id, total } => {
                self.uploads.begin(*upload_id, *total)?;
                Ack
            }
            UploadChunk {
                upload_id,
                offset,
                data,
            } => {
                self.uploads.chunk(*upload_id, *offset, data)?;
                Ack
            }
            UploadCommit { upload_id } => UploadResult {
                upload_id: *upload_id,
                path: self.uploads.commit(*upload_id)?,
            },
            UploadAbort { upload_id } => {
                self.uploads.abort(*upload_id);
                Ack
            }
            Cancel { request_id } => {
                if let Some(attachment) = &mut self.attachment {
                    if attachment.watch_request == Some(*request_id) {
                        attachment.watch_request = None;
                        self.filesystem.cancel_watch();
                    }
                }
                if let Some(integration) = &mut self.integration {
                    integration.cancel(*request_id);
                }
                Ack
            }
            _ => return Err(ErrorCode::InvalidRequest),
        };
        Ok(vec![reply])
    }

    fn poll_events(&mut self) {
        let previous_cwd = self.shell_cwd.clone();
        if let Ok(Some(bytes)) = self.cwd_sink.poll() {
            if let Ok(path) = RemotePath::new(bytes) {
                if let Ok(native) = crate::filesystem::native(&path) {
                    if let Ok(guard) = made_explorer::paths::PathGuard::new(native) {
                        self.shell_cwd = crate::filesystem::remote(guard.root()).ok();
                    }
                }
            }
        }
        self.flush_attachment();
        self.observe_pty();
        if self.attachment.is_none() {
            if let Some(integration) = &mut self.integration {
                integration.poll();
            }
            return;
        }
        if previous_cwd != self.shell_cwd {
            if let Some(path) = self.shell_cwd.clone() {
                if let Some(attachment) = &self.attachment {
                    let frame = WireEnvelope::new(
                        attachment.identity.clone(),
                        attachment.request,
                        Operation::ShellCwd { path },
                    );
                    self.enqueue(frame);
                }
            }
        }
        let change = self.filesystem.poll_change();
        if let Some(operation) = change {
            if let Some(attachment) = &self.attachment {
                if let Some(id) = attachment.watch_request {
                    let frame = WireEnvelope::new(attachment.identity.clone(), id, operation);
                    self.enqueue(frame);
                }
            }
        }
        if let Some(integration) = &mut self.integration {
            let events = integration.poll();
            if let Some(attachment) = &self.attachment {
                let identity = attachment.identity.clone();
                let request = attachment.request;
                for (id, operation) in events {
                    let frame =
                        WireEnvelope::new(identity.clone(), id.unwrap_or(request), operation);
                    self.enqueue(frame);
                }
            }
        }
    }

    fn flush_attachment(&mut self) {
        let Some(attachment) = self.attachment.as_mut() else {
            return;
        };
        if !attachment.pending_control.is_empty() {
            let pending = std::mem::take(&mut attachment.pending_control);
            let output = OutputQueue {
                control: attachment.output.control.clone(),
                data: attachment.output.data.clone(),
            };
            let mut blocked = false;
            let mut rest = Vec::new();
            for frame in pending {
                if blocked {
                    rest.push(frame);
                    continue;
                }
                match output.try_send(frame) {
                    Ok(()) => {}
                    Err(TrySendError::Full(frame)) => {
                        rest.push(frame);
                        blocked = true;
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        self.detach_frontend();
                        return;
                    }
                }
            }
            if let Some(attachment) = self.attachment.as_mut() {
                attachment.pending_control = rest;
            }
        }
        self.flush_snapshot();
    }

    fn flush_snapshot(&mut self) {
        let Some(attachment) = self.attachment.as_mut() else {
            return;
        };
        while !attachment.snapshot_closed {
            let Some((operation, consumed, closed)) =
                next_snapshot_frame(&attachment.snapshot_rest, false)
            else {
                return;
            };
            let frame =
                WireEnvelope::new(attachment.identity.clone(), attachment.request, operation);
            match attachment.output.try_send(frame) {
                Ok(()) => {
                    if consumed > 0 {
                        attachment.snapshot_rest.drain(..consumed);
                    }
                    if closed {
                        attachment.snapshot_closed = true;
                    }
                }
                Err(TrySendError::Full(_)) => return,
                Err(TrySendError::Disconnected(_)) => {
                    self.detach_frontend();
                    return;
                }
            }
        }
    }

    fn observe_pty(&mut self) {
        const MAX_PTY_EVENTS_PER_TICK: usize = 256;
        let snapshot_pending = self
            .attachment
            .as_ref()
            .is_some_and(|attachment| !attachment.snapshot_closed);
        if snapshot_pending {
            return;
        }
        // A single event per main-loop tick puts every later keystroke echo
        // behind the shell's startup output and command output. Drain the
        // reader queue in one pass; the bounded output queue is the
        // backpressure point.
        for _ in 0..MAX_PTY_EVENTS_PER_TICK {
            if let Some(bytes) = self
                .attachment
                .as_mut()
                .and_then(|attachment| attachment.pending_relay.take())
            {
                if !self.relay_terminal(bytes.clone()) {
                    if let Some(attachment) = self.attachment.as_mut() {
                        attachment.pending_relay = Some(bytes);
                    }
                    return;
                }
            }
            let Some(event) = self.session.poll() else {
                return;
            };
            match event {
                PtyEvent::Data(bytes) => {
                    self.session.observe(&bytes);
                    if self.attachment.is_some() && !self.relay_terminal(bytes.clone()) {
                        if let Some(attachment) = self.attachment.as_mut() {
                            attachment.pending_relay = Some(bytes);
                        }
                        return;
                    }
                }
                PtyEvent::Exited(status) => {
                    if let Some(attachment) = &self.attachment {
                        let frame = WireEnvelope::new(
                            attachment.identity.clone(),
                            attachment.request,
                            Operation::TerminalExit { code: status.code },
                        );
                        self.enqueue(frame);
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    fn relay_terminal(&mut self, bytes: Vec<u8>) -> bool {
        let Some(attachment) = self.attachment.as_ref() else {
            return true;
        };
        let Some(operation) = live_terminal_output(&bytes) else {
            return true;
        };
        let frame = WireEnvelope::new(attachment.identity.clone(), attachment.request, operation);
        match attachment.output.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.detach_frontend();
                true
            }
        }
    }
}

fn writer_loop(
    mut writer: TcpStream,
    control_rx: Receiver<WireEnvelope>,
    data_rx: Receiver<WireEnvelope>,
) {
    loop {
        let frame = match control_rx.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Disconnected) => match data_rx.try_recv() {
                Ok(frame) => frame,
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(TryRecvError::Disconnected) => break,
            },
            Err(TryRecvError::Empty) => match data_rx.try_recv() {
                Ok(frame) => frame,
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    match control_rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(frame) => frame,
                        Err(_) => break,
                    }
                }
            },
        };
        if write_frame_retry(&mut writer, &frame).is_err() {
            break;
        }
    }
}

fn write_frame_retry(writer: &mut TcpStream, frame: &WireEnvelope) -> io::Result<()> {
    let bytes =
        encode(frame).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    let mut offset = 0;
    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "attachment closed",
                ))
            }
            Ok(n) => offset += n,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                ) =>
            {
                // Backpressure is not attachment death. A broken pipe still returns.
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
    writer.flush()
}

fn current_target() -> Result<RemoteTarget, ErrorCode> {
    match made_platform::remote_target().map_err(io_error)?.as_str() {
        "x86_64-unknown-linux-gnu" => Ok(RemoteTarget::LinuxX64),
        "aarch64-unknown-linux-gnu" => Ok(RemoteTarget::LinuxArm64),
        "x86_64-apple-darwin" => Ok(RemoteTarget::MacosX64),
        "aarch64-apple-darwin" => Ok(RemoteTarget::MacosArm64),
        _ => Err(ErrorCode::Unsupported),
    }
}

fn chunk_git(command: crate::protocol::GitRead, bytes: &[u8]) -> Vec<Operation> {
    let mut results = Vec::new();
    for (index, chunk) in bytes.chunks(MAX_CHUNK_BYTES).enumerate() {
        let Ok(data) = Chunk::new(chunk.to_vec()) else {
            break;
        };
        results.push(Operation::GitResult {
            command,
            offset: (index * MAX_CHUNK_BYTES) as u64,
            data,
            done: (index + 1) * MAX_CHUNK_BYTES >= bytes.len(),
        });
    }
    if results.is_empty() {
        if let Ok(data) = Chunk::new(Vec::new()) {
            results.push(Operation::GitResult {
                command,
                offset: 0,
                data,
                done: true,
            });
        }
    }
    results
}

pub fn read_envelope(reader: &mut impl Read) -> io::Result<WireEnvelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let n = u32::from_be_bytes(prefix) as usize;
    if n == 0 || n > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame limit"));
    }
    let mut payload = vec![0; n];
    reader.read_exact(&mut payload)?;
    let frame: WireEnvelope = serde_json::from_slice(&payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    frame
        .validate()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    Ok(frame)
}

pub fn write_envelope(writer: &mut impl Write, frame: &WireEnvelope) -> io::Result<()> {
    writer.write_all(
        &encode(frame).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?,
    )?;
    writer.flush()
}
