//! Bounded asynchronous client. A transport break invalidates queues, not the
//! remote session. Reconnection always constructs a fresh client/epoch.
use crate::protocol::*;
use made_platform::ssh::{SshEvent, SshProcess};
use made_platform::ssh_askpass::{PromptAnswer, PromptRequest};
use made_terminal::{PtyError, PtyEvent, TerminalTransport, TransportStatus};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

/// Nonblocking byte-stream seam; production uses installed OpenSSH. Test
/// implementations are not evidence of real SSH/TUI parity.
pub trait ClientStream: Send + 'static {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn poll(&mut self) -> Option<SshEvent>;
    fn detach(&mut self);
    fn poll_prompt(&mut self) -> Option<PromptRequest> {
        None
    }
    fn answer_prompt(&mut self, _: u64, _: Option<PromptAnswer>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
impl ClientStream for SshProcess {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        SshProcess::write(self, bytes)
    }
    fn poll(&mut self) -> Option<SshEvent> {
        SshProcess::poll(self)
    }
    fn detach(&mut self) {
        self.cancel();
    }
    fn poll_prompt(&mut self) -> Option<PromptRequest> {
        SshProcess::poll_prompt(self)
    }
    fn answer_prompt(&mut self, id: u64, answer: Option<PromptAnswer>) -> io::Result<()> {
        SshProcess::answer_prompt(self, id, answer)
    }
}

struct Pending {
    request: WireEnvelope,
    deadline: Instant,
    progress: ReplyProgress,
}

/// One `TerminalOutput` frame. `done` is not implied by an empty poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalFrame {
    pub bytes: Vec<u8>,
    pub done: bool,
}

enum QueuedTerminal {
    Output(TerminalFrame),
    Other(PtyEvent),
}
struct State {
    identity: Identity,
    root: RemotePath,
    connected: bool,
    external_id: u64,
    internal_id: u64,
    /// External Open/Attach request id that owns unsolicited plugin events.
    attach_request: Option<u64>,
    discard_through: u64,
    discard_internal_through: u64,
    control: VecDeque<WireEnvelope>,
    data: VecDeque<WireEnvelope>,
    pending: HashMap<u64, Pending>,
    terminal_pending: HashSet<u64>,
    replies: VecDeque<WireEnvelope>,
    roots: VecDeque<RootResolution>,
    sessions: VecDeque<WireEnvelope>,
    terminal: VecDeque<QueuedTerminal>,
    /// Set only when a `done: true` frame is accepted. An empty poll does not set it.
    resnapshot_done: bool,
    status: TransportStatus,
    prompt: Option<PromptRequest>,
    answer: Option<(u64, Option<PromptAnswer>)>,
    /// Called after terminal output arrives or the transport drops, so the UI
    /// can paint without waiting for its next idle poll.
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    wake_pending: bool,
}
impl State {
    fn disconnect(&mut self) {
        self.wake_pending = true;
        self.connected = false;
        self.status = TransportStatus::Disconnected;
        self.control.clear();
        self.data.clear();
        self.pending.clear();
        self.terminal_pending.clear();
        self.replies.clear();
        self.roots.clear();
        self.sessions.clear();
        self.terminal.clear();
        self.resnapshot_done = false;
        self.prompt.take();
        self.answer.take();
        self.attach_request = None;
    }
    fn internal_id(&mut self) -> Result<u64, ProtocolError> {
        self.internal_id = self
            .internal_id
            .checked_add(1)
            .ok_or(ProtocolError::Limit)?;
        Ok(self.internal_id)
    }
    fn submit(
        &mut self,
        frame: WireEnvelope,
        grant: Option<&RemotePath>,
        now: Instant,
    ) -> Result<(), ProtocolError> {
        if !self.connected
            || !frame.identity.matches(&self.identity)
            || frame.request_id <= self.external_id
            || frame.request_id >= (1u64 << 63)
        {
            return Err(ProtocolError::Identity);
        }
        frame.validate_request(&self.root, grant)?;
        if self.pending.len() >= MAX_IN_FLIGHT || self.control.len() >= MAX_IN_FLIGHT {
            return Err(ProtocolError::Limit);
        }
        let _ = encode(&frame)?;
        self.external_id = frame.request_id;
        let attach_request = matches!(
            &frame.operation,
            Operation::Open { .. } | Operation::Attach { .. }
        );
        self.pending.insert(
            frame.request_id,
            Pending {
                request: frame.clone(),
                deadline: now
                    + if matches!(frame.operation, Operation::Close) {
                        CLOSE_TIMEOUT
                    } else {
                        RPC_TIMEOUT
                    },
                progress: ReplyProgress::new(frame.clone())?,
            },
        );
        if attach_request {
            self.attach_request = Some(frame.request_id);
        }
        self.control.push_back(frame);
        Ok(())
    }
    fn deliver(&mut self, frame: WireEnvelope) -> Result<(), ProtocolError> {
        if self.replies.len() >= MAX_QUEUED_CHUNKS
            || self.roots.len() >= MAX_IN_FLIGHT
            || self.sessions.len() >= MAX_IN_FLIGHT
        {
            return Err(ProtocolError::Limit);
        }
        // A GrokSnapshot/GrokEvent is a reply only when it completes a pending
        // GrokState request; every other Grok frame is an unsolicited
        // session-sink event for the owning tab.
        let grok_state_reply = matches!(
            frame.operation,
            Operation::GrokSnapshot { .. } | Operation::GrokEvent { .. }
        ) && self
            .pending
            .get(&frame.request_id)
            .is_some_and(|pending| matches!(pending.request.operation, Operation::GrokState));
        match &frame.operation {
            Operation::RootResult {
                path, generation, ..
            } => self.roots.push_back(RootResolution {
                identity: frame.identity.clone(),
                request_id: frame.request_id,
                result: Ok(path.clone()),
                generation: *generation,
            }),
            Operation::SessionSnapshot { .. } | Operation::FocusResult { .. } => {
                self.sessions.push_back(frame.clone())
            }
            Operation::GrokSnapshot { .. } | Operation::GrokEvent { .. } if !grok_state_reply => {
                self.sessions.push_back(frame.clone())
            }
            Operation::Error { code } => {
                if let Some(p) = self.pending.get(&frame.request_id) {
                    match p.request.operation {
                        Operation::RootValidate { .. } | Operation::RootSelect { .. } => {
                            self.roots.push_back(RootResolution {
                                identity: frame.identity.clone(),
                                request_id: frame.request_id,
                                result: Err(*code),
                                generation: frame.identity.generation,
                            })
                        }
                        Operation::SessionState | Operation::Focus { .. } => {
                            self.sessions.push_back(frame.clone())
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        // Dedicated ports consume their own replies: do not require draining a
        // duplicate generic queue for normal root/session/heartbeat operation.
        // Unsolicited Grok pushes belong to the session sink only; a Grok
        // snapshot reply to a pending GrokState request keeps the generic
        // reply queue for the workspace id map.
        if !matches!(
            frame.operation,
            Operation::RootResult { .. }
                | Operation::SessionSnapshot { .. }
                | Operation::FocusResult { .. }
        ) && !(matches!(
            frame.operation,
            Operation::GrokSnapshot { .. } | Operation::GrokEvent { .. }
        ) && !grok_state_reply)
            && !self.pending.get(&frame.request_id).is_some_and(|p| {
                matches!(
                    p.request.operation,
                    Operation::Heartbeat
                        | Operation::RootValidate { .. }
                        | Operation::RootSelect { .. }
                        | Operation::SessionState
                        | Operation::Focus { .. }
                )
            })
        {
            self.replies.push_back(frame);
        }
        Ok(())
    }
    fn receive(&mut self, frame: WireEnvelope) -> Result<(), ProtocolError> {
        frame.validate()?;
        if !transport_current(&frame.identity, &self.identity) {
            return Err(ProtocolError::Identity);
        }
        // Filesystem generation/root must not drop the attachment or discard
        // terminal bytes. Stale non-terminal events are ignored.
        let generation_current = frame.identity.generation == self.identity.generation;
        match &frame.operation {
            Operation::TerminalOutput { data, done } => {
                // A second terminator before the next attach is malformed and
                // is not applied. This is not ReplyProgress completion.
                if *done && self.resnapshot_done {
                    return Ok(());
                }
                if self.terminal.len() >= MAX_QUEUED_CHUNKS {
                    return Err(ProtocolError::Limit);
                }
                if *done {
                    self.resnapshot_done = true;
                }
                self.wake_pending = true;
                self.terminal
                    .push_back(QueuedTerminal::Output(TerminalFrame {
                        bytes: data.as_bytes().to_vec(),
                        done: *done,
                    }));
                return Ok(());
            }
            Operation::TerminalExit { code } => {
                if self.terminal.len() >= MAX_QUEUED_CHUNKS {
                    return Err(ProtocolError::Limit);
                }
                let status = made_platform::ExitStatus {
                    code: *code,
                    signal: None,
                };
                self.status = TransportStatus::Exited(status.clone());
                self.terminal
                    .push_back(QueuedTerminal::Other(PtyEvent::Exited(status)));
                return Ok(());
            }
            _ => {}
        }
        // A reset or accepted root transition deliberately abandons old-root
        // work. Late replies for those request ids are drained, not treated as
        // forged replies that would disconnect the new root. Unsolicited
        // SessionSnapshot and Grok push events reuse the current Open/Attach
        // request id, so only that event source survives the discard window;
        // downstream revision/identity validation still rejects stale or
        // foreign state.
        let unsolicited_plugin = matches!(
            frame.operation,
            Operation::SessionSnapshot { .. }
                | Operation::GrokSnapshot { .. }
                | Operation::GrokEvent { .. }
        ) && self.attach_request == Some(frame.request_id);
        if !unsolicited_plugin
            && !self.pending.contains_key(&frame.request_id)
            && ((frame.request_id != 0 && frame.request_id <= self.discard_through)
                || (frame.request_id >= (1u64 << 63)
                    && frame.request_id <= self.discard_internal_through))
        {
            return Ok(());
        }
        match &frame.operation {
            Operation::ShellCwd { .. } => return self.deliver(frame),
            Operation::Changed { path } if !self.pending.contains_key(&frame.request_id) => {
                if !generation_current || !path.is_within(&self.root) {
                    return Ok(());
                }
                return self.deliver(frame);
            }
            Operation::SessionSnapshot { .. } if !self.pending.contains_key(&frame.request_id) => {
                return self.deliver(frame);
            }
            // Genuine Grok pushes are session-sink events, never replies to a
            // pending workspace request. When the attach is still in flight, a
            // push riding its id must not consume it; a reply to a pending
            // GrokState request keeps the normal correlation path below.
            Operation::GrokSnapshot { .. } | Operation::GrokEvent { .. }
                if !self.pending.get(&frame.request_id).is_some_and(|pending| {
                    matches!(pending.request.operation, Operation::GrokState)
                }) =>
            {
                return self.deliver(frame);
            }
            _ => {}
        }
        if !generation_current && !self.pending.contains_key(&frame.request_id) {
            return Ok(());
        }
        if self.replies.len() >= MAX_QUEUED_CHUNKS {
            return Err(ProtocolError::Limit);
        }
        let mut correlated = frame;
        if let Some(pending) = self.pending.get(&correlated.request_id) {
            // A root change must not leave the in-flight heartbeat Ack unmatched.
            if transport_current(&correlated.identity, &pending.request.identity)
                && correlated.identity.generation != pending.request.identity.generation
            {
                correlated.identity.generation = pending.request.identity.generation;
            }
        }
        if matches!(correlated.operation, Operation::Ack)
            && self.terminal_pending.remove(&correlated.request_id)
        {
            return Ok(());
        }
        let pending = self
            .pending
            .get_mut(&correlated.request_id)
            .ok_or(ProtocolError::Identity)?;
        pending.progress.accept(&correlated)?;
        let done = pending.progress.is_done();
        if done {
            pending.progress.finish()?;
        }
        let attachment_failed =
            done && matches!(
                pending.request.operation,
                Operation::Open { .. } | Operation::Attach { .. }
            ) && !matches!(correlated.operation, Operation::Attached);
        let id = correlated.request_id;
        let closed = matches!(correlated.operation, Operation::Closed { .. });
        self.deliver(correlated)?;
        if done {
            self.pending.remove(&id);
            if attachment_failed {
                self.attach_request = None;
            }
        }
        if closed {
            self.connected = false;
            self.status = TransportStatus::Disconnected;
            self.pending.clear();
            self.control.clear();
            self.data.clear();
            self.terminal_pending.clear();
            self.attach_request = None;
        }
        Ok(())
    }
    fn cancel(&mut self, id: u64) {
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        if self.attach_request == Some(id) {
            self.attach_request = None;
        }
        let unsent = self.control.iter().any(|f| f.request_id == id);
        self.control.retain(|f| f.request_id != id);
        if !unsent
            && self.control.len() < MAX_IN_FLIGHT
            && self.terminal_pending.len() < MAX_QUEUED_CHUNKS
        {
            if let Ok(request_id) = self.internal_id() {
                self.control.push_back(WireEnvelope::new(
                    pending.request.identity,
                    request_id,
                    Operation::Cancel { request_id: id },
                ));
                self.terminal_pending.insert(request_id);
            }
        }
    }
}

/// Cloneable handles share exactly one tab, one transport and bounded queues.
#[derive(Clone)]
pub struct RemoteClient {
    state: Arc<Mutex<State>>,
}
impl std::fmt::Debug for RemoteClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteClient(<private>)")
    }
}
impl RemoteClient {
    pub fn new(
        stream: impl ClientStream,
        identity: Identity,
        root: RemotePath,
    ) -> Result<Self, ProtocolError> {
        identity.validate()?;
        let state = Arc::new(Mutex::new(State {
            identity,
            root,
            connected: true,
            external_id: 0,
            internal_id: 1u64 << 63,
            attach_request: None,
            discard_through: 0,
            discard_internal_through: 0,
            control: VecDeque::new(),
            data: VecDeque::new(),
            pending: HashMap::new(),
            terminal_pending: HashSet::new(),
            replies: VecDeque::new(),
            roots: VecDeque::new(),
            sessions: VecDeque::new(),
            terminal: VecDeque::new(),
            resnapshot_done: false,
            status: TransportStatus::Connected,
            prompt: None,
            answer: None,
            wake: None,
            wake_pending: false,
        }));
        let weak = Arc::downgrade(&state);
        std::thread::spawn(move || run(stream, weak));
        Ok(Self { state })
    }
    /// Register a callback run on the transport thread, outside the state lock,
    /// after terminal output arrives or the transport drops. It must be cheap
    /// and must not call back into this client.
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut state) = self.state.lock() {
            state.wake = Some(Arc::new(wake));
        }
    }
    pub fn identity(&self) -> Identity {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .identity
            .clone()
    }
    pub fn request(
        &self,
        operation: Operation,
        grant: Option<&RemotePath>,
    ) -> Result<u64, ProtocolError> {
        let mut state = self.state.lock().map_err(|_| ProtocolError::Identity)?;
        let id = state
            .external_id
            .checked_add(1)
            .ok_or(ProtocolError::Limit)?;
        let frame = WireEnvelope::new(state.identity.clone(), id, operation);
        state.submit(frame, grant, Instant::now())?;
        Ok(id)
    }
    /// Discard every queued and pending old-root request without sending
    /// Cancel frames. A root transition resets here before RootSelect so the
    /// old root's full in-flight queue cannot starve the transition.
    pub fn reset_pending(&self) -> Result<(), ProtocolError> {
        let mut state = self.state.lock().map_err(|_| ProtocolError::Identity)?;
        state.discard_through = state.external_id;
        state.discard_internal_through = state.internal_id;
        if state
            .attach_request
            .is_some_and(|id| state.pending.contains_key(&id))
        {
            state.attach_request = None;
        }
        state.control.clear();
        state.data.clear();
        state.pending.clear();
        state.terminal_pending.clear();
        state.replies.clear();
        state.roots.clear();
        state.sessions.clear();
        state.resnapshot_done = false;
        Ok(())
    }

    /// Applying an accepted RootSelect is explicit and discards all old-root
    /// work. The caller must first validate/consume its RootResolution.
    pub fn set_root(&self, root: RemotePath, generation: u64) -> Result<(), ProtocolError> {
        let mut state = self.state.lock().map_err(|_| ProtocolError::Identity)?;
        if generation <= state.identity.generation {
            return Err(ProtocolError::Identity);
        }
        state.discard_through = state.external_id;
        state.discard_internal_through = state.internal_id;
        if state
            .attach_request
            .is_some_and(|id| state.pending.contains_key(&id))
        {
            state.attach_request = None;
        }
        state.control.clear();
        state.data.clear();
        state.pending.clear();
        state.terminal_pending.clear();
        state.replies.clear();
        state.roots.clear();
        state.sessions.clear();
        state.root = root;
        state.identity.generation = generation;
        Ok(())
    }
    pub fn poll_reply(&self) -> Option<WireEnvelope> {
        self.state.lock().ok()?.replies.pop_front()
    }
    pub fn poll_prompt(&self) -> Option<PromptRequest> {
        self.state.lock().ok()?.prompt.take()
    }
    pub fn answer_prompt(&self, id: u64, answer: Option<PromptAnswer>) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::ErrorKind::BrokenPipe)?;
        if !state.connected {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if state.answer.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        state.answer = Some((id, answer));
        Ok(())
    }
    pub fn cancel_request(&self, id: u64) {
        if let Ok(mut s) = self.state.lock() {
            s.cancel(id);
            s.replies.retain(|f| f.request_id != id);
            s.roots.retain(|f| f.request_id != id);
            s.sessions.retain(|f| f.request_id != id);
        }
    }
    pub fn disconnect(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.disconnect();
        }
    }
    pub fn is_connected(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.connected)
    }
    pub fn root_resolver(&self) -> RemoteRootResolver {
        RemoteRootResolver(self.clone())
    }
    pub fn session_sink(&self) -> RemoteSessionSink {
        RemoteSessionSink(self.clone())
    }
    pub fn terminal_transport(&self) -> RemoteTerminalTransport {
        RemoteTerminalTransport(self.clone())
    }
}

/// Due heartbeats are not blocked by a full user queue, and a single in-flight
/// heartbeat is enough — a second one would sit behind the first forever.
pub fn heartbeat_ready(due: bool, inflight: bool, control_len: usize, pending_len: usize) -> bool {
    due && !inflight && control_len <= MAX_IN_FLIGHT && pending_len <= MAX_IN_FLIGHT
}

fn transport_current(frame: &Identity, current: &Identity) -> bool {
    frame.authority == current.authority
        && frame.tab.authenticates(&current.tab)
        && frame.epoch == current.epoch
}

fn run(mut stream: impl ClientStream, weak: Weak<Mutex<State>>) {
    let mut decoder = FrameDecoder::new();
    let mut inbound = Vec::new();
    let mut sending: Option<(Vec<u8>, usize)> = None;
    let mut heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
    let mut held: Option<WireEnvelope> = None;
    loop {
        let Some(shared) = weak.upgrade() else { break };
        let Ok(mut state) = shared.lock() else { break };
        if !state.connected {
            break;
        }
        let now = Instant::now();
        if let Some((id, answer)) = state.answer.take() {
            if stream.answer_prompt(id, answer).is_err() {
                state.disconnect();
                break;
            }
        }
        if state.prompt.is_none() {
            state.prompt = stream.poll_prompt();
        }
        let expired: Vec<_> = state
            .pending
            .iter()
            .filter(|(_, p)| now >= p.deadline)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            let stop = state
                .pending
                .get(&id)
                .is_some_and(|p| matches!(p.request.operation, Operation::Close));
            let frame = WireEnvelope::new(
                state.identity.clone(),
                id,
                Operation::Error {
                    code: ErrorCode::Timeout,
                },
            );
            if let Err(error) = state.deliver(frame) {
                // A full reply queue is backpressure, not a dead transport.
                if error != ProtocolError::Limit {
                    state.disconnect();
                }
                break;
            }
            state.cancel(id);
            if stop {
                state.connected = false;
                state.status = TransportStatus::Disconnected;
                state.control.clear();
                state.data.clear();
                state.pending.clear();
                state.terminal_pending.clear();
                break;
            }
        }
        let heartbeat_inflight = state
            .pending
            .values()
            .any(|pending| matches!(pending.request.operation, Operation::Heartbeat));
        if heartbeat_ready(
            now >= heartbeat,
            heartbeat_inflight,
            state.control.len(),
            state.pending.len(),
        ) {
            if let Ok(id) = state.internal_id() {
                let frame = WireEnvelope::new(state.identity.clone(), id, Operation::Heartbeat);
                let progress = match ReplyProgress::new(frame.clone()) {
                    Ok(p) => p,
                    Err(_) => {
                        state.disconnect();
                        break;
                    }
                };
                state.pending.insert(
                    id,
                    Pending {
                        request: frame.clone(),
                        deadline: now + RPC_TIMEOUT,
                        progress,
                    },
                );
                state.control.push_back(frame);
            }
            heartbeat = now + HEARTBEAT_INTERVAL;
        }
        // Finish the current frame before priority changes; never interleave
        // frame bytes or terminal chunks midway through an escape sequence.
        if sending.is_none() {
            if let Some(frame) = state.control.pop_front().or_else(|| state.data.pop_front()) {
                match encode(&frame) {
                    Ok(bytes) => sending = Some((bytes, 0)),
                    Err(_) => state.disconnect(),
                }
            }
        }
        if let Some((bytes, offset)) = sending.as_mut() {
            let end = (*offset + made_platform::ssh::SSH_CHUNK_BYTES).min(bytes.len());
            match stream.write(&bytes[*offset..end]) {
                Ok(()) => {
                    *offset = end;
                    if end == bytes.len() {
                        sending = None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => state.disconnect(),
            }
        }
        if let Some(frame) = held.take() {
            match state.receive(frame.clone()) {
                Ok(()) => {}
                Err(ProtocolError::Limit) => held = Some(frame),
                Err(_) => state.disconnect(),
            }
        }
        if held.is_none() && state.connected {
            for _ in 0..16 {
                match stream.poll() {
                    None => break,
                    Some(SshEvent::Data(bytes)) => inbound.extend_from_slice(&bytes),
                    Some(_) => {
                        state.disconnect();
                        break;
                    }
                }
            }
        }
        if held.is_none() && state.connected && !inbound.is_empty() {
            let mut offset = 0;
            while offset < inbound.len() && state.connected && held.is_none() {
                match decoder.push(&inbound[offset..]) {
                    Ok((n, parsed)) if n > 0 => {
                        offset += n;
                        if let Some(frame) = parsed {
                            match state.receive(frame.clone()) {
                                Ok(()) => {}
                                Err(ProtocolError::Limit) => held = Some(frame),
                                Err(_) => state.disconnect(),
                            }
                        }
                    }
                    _ => {
                        state.disconnect();
                        break;
                    }
                }
            }
            inbound.drain(..offset);
        }
        let wake = if std::mem::take(&mut state.wake_pending) {
            state.wake.clone()
        } else {
            None
        };
        drop(state);
        drop(shared);
        if let Some(wake) = wake {
            wake();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // Discard partially sent frames and all uncertain input. There is no retry
    // or replay path; a new epoch requires a new transport/client instance.
    stream.detach();
    let wake = weak
        .upgrade()
        .and_then(|shared| shared.lock().ok().and_then(|state| state.wake.clone()));
    if let Some(wake) = wake {
        wake();
    }
}

pub struct RemoteRootResolver(RemoteClient);
impl RootResolver for RemoteRootResolver {
    fn resolve(&mut self, request: RootRequest) -> Result<(), ProtocolError> {
        let grant = request.path.clone();
        self.0
            .state
            .lock()
            .map_err(|_| ProtocolError::Identity)?
            .submit(request.into_envelope(), Some(&grant), Instant::now())
    }
    fn poll(&mut self) -> Option<RootResolution> {
        self.0.state.lock().ok()?.roots.pop_front()
    }
    fn cancel(&mut self, id: u64) {
        self.0.cancel_request(id);
    }
}
pub struct RemoteSessionSink(RemoteClient);
impl SessionCommandSink for RemoteSessionSink {
    fn submit(&mut self, request: SessionCommandRequest) -> Result<(), ProtocolError> {
        self.0
            .state
            .lock()
            .map_err(|_| ProtocolError::Identity)?
            .submit(request.into_envelope(), None, Instant::now())
    }
    fn poll(&mut self) -> Option<WireEnvelope> {
        self.0.state.lock().ok()?.sessions.pop_front()
    }
    fn cancel(&mut self, id: u64) {
        self.0.cancel_request(id);
    }
}
pub struct RemoteTerminalTransport(RemoteClient);
impl RemoteTerminalTransport {
    /// Next output frame, including `done`. `None` is an empty poll, not the
    /// end of a resnapshot.
    pub fn poll_output(&mut self) -> Option<TerminalFrame> {
        let mut state = self.0.state.lock().ok()?;
        match state.terminal.front() {
            Some(QueuedTerminal::Output(_)) => match state.terminal.pop_front() {
                Some(QueuedTerminal::Output(frame)) => Some(frame),
                _ => None,
            },
            _ => None,
        }
    }

    /// True only after a completing frame was accepted. An empty poll does not
    /// set this, and a later `done: false` frame does not clear it.
    pub fn resnapshot_complete(&self) -> bool {
        self.0.state.lock().is_ok_and(|state| state.resnapshot_done)
    }

    fn enqueue(&self, operation: Operation) -> Result<(), PtyError> {
        let mut state = self.0.state.lock().map_err(|_| PtyError::Closed)?;
        if !state.connected || !matches!(state.status, TransportStatus::Connected) {
            return Err(PtyError::Closed);
        }
        if state.data.len() >= MAX_QUEUED_CHUNKS
            || state.terminal_pending.len() >= MAX_QUEUED_CHUNKS
        {
            return Err(PtyError::WouldBlock);
        }
        operation.validate().map_err(|_| PtyError::WouldBlock)?;
        let id = state.internal_id().map_err(|_| PtyError::Closed)?;
        let frame = WireEnvelope::new(state.identity.clone(), id, operation);
        state.terminal_pending.insert(id);
        state.data.push_back(frame);
        Ok(())
    }
}
impl TerminalTransport for RemoteTerminalTransport {
    fn poll(&mut self) -> Option<PtyEvent> {
        // Byte view only. `done` stays on [`Self::poll_output`]; an empty poll
        // is not completion.
        match self.0.state.lock().ok()?.terminal.pop_front()? {
            QueuedTerminal::Output(frame) => Some(PtyEvent::Data(frame.bytes)),
            QueuedTerminal::Other(event) => Some(event),
        }
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let chunks = bytes.len().div_ceil(MAX_CHUNK_BYTES);
        let mut state = self.0.state.lock().map_err(|_| PtyError::Closed)?;
        if !state.connected || !matches!(state.status, TransportStatus::Connected) {
            return Err(PtyError::Closed);
        }
        if chunks > MAX_QUEUED_CHUNKS.saturating_sub(state.data.len())
            || chunks > MAX_QUEUED_CHUNKS.saturating_sub(state.terminal_pending.len())
            || state.internal_id.checked_add(chunks as u64).is_none()
        {
            return Err(PtyError::WouldBlock);
        }
        for bytes in bytes.chunks(MAX_CHUNK_BYTES) {
            let data = Chunk::new(bytes.to_vec()).map_err(|_| PtyError::WouldBlock)?;
            let id = state.internal_id().map_err(|_| PtyError::Closed)?;
            let frame = WireEnvelope::new(
                state.identity.clone(),
                id,
                Operation::TerminalInput { data },
            );
            state.terminal_pending.insert(id);
            state.data.push_back(frame);
        }
        Ok(())
    }
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.enqueue(Operation::TerminalResize { rows, cols })
    }
    fn status(&self) -> TransportStatus {
        self.0
            .state
            .lock()
            .map(|s| s.status.clone())
            .unwrap_or(TransportStatus::Failed)
    }
    fn detach(&mut self) -> Result<(), PtyError> {
        self.0.disconnect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use made_platform::ssh::SshEvent;
    use std::io;

    struct NullStream;

    impl ClientStream for NullStream {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
            Ok(())
        }
        fn poll(&mut self) -> Option<SshEvent> {
            None
        }
        fn detach(&mut self) {}
    }

    fn identity() -> Identity {
        Identity {
            authority: AuthorityId([7; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        }
    }

    fn snapshot(revision: u64) -> made_bridge::state::SessionSnapshot {
        made_bridge::state::SessionSnapshot {
            revision,
            active_session_id: None,
            tabs_enabled: true,
            view_directory: None,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn reset_pending_frees_the_in_flight_queue_for_root_select() {
        let root = RemotePath::new(b"/project".to_vec()).unwrap();
        let client = RemoteClient::new(NullStream, identity(), root).unwrap();
        for _ in 0..MAX_IN_FLIGHT {
            client
                .request(
                    Operation::GitRead {
                        command: GitRead::Status,
                    },
                    None,
                )
                .unwrap();
        }
        assert!(matches!(
            client.request(
                Operation::GitRead {
                    command: GitRead::Status,
                },
                None,
            ),
            Err(ProtocolError::Limit)
        ));
        client.reset_pending().unwrap();
        let next = RemotePath::new(b"/next".to_vec()).unwrap();
        client
            .request(Operation::RootSelect { path: next.clone() }, Some(&next))
            .unwrap();
    }

    #[test]
    fn reset_pending_discards_late_old_replies() {
        let root = RemotePath::new(b"/project".to_vec()).unwrap();
        let client = RemoteClient::new(NullStream, identity(), root).unwrap();
        let old_id = client
            .request(
                Operation::GitRead {
                    command: GitRead::Status,
                },
                None,
            )
            .unwrap();
        client.reset_pending().unwrap();
        let mut state = client.state.lock().unwrap();
        let late = WireEnvelope::new(state.identity.clone(), old_id, Operation::Ack);
        assert!(
            state.receive(late).is_ok(),
            "late old-root replies must be drained, not disconnect the new root"
        );
        let late_internal =
            WireEnvelope::new(state.identity.clone(), state.internal_id, Operation::Ack);
        assert!(
            state.receive(late_internal).is_ok(),
            "late heartbeat/cancel replies must also be drained"
        );
    }

    #[test]
    fn failed_attach_clears_the_unsolicited_event_source() {
        let root = RemotePath::new(b"/project".to_vec()).unwrap();
        let client = RemoteClient::new(NullStream, identity(), root).unwrap();
        let attach = client
            .request(Operation::Attach { rows: 24, cols: 80 }, None)
            .unwrap();
        let mut state = client.state.lock().unwrap();
        assert_eq!(state.attach_request, Some(attach));
        let identity = state.identity.clone();
        state
            .receive(WireEnvelope::new(
                identity,
                attach,
                Operation::Error {
                    code: ErrorCode::Unavailable,
                },
            ))
            .unwrap();
        assert_eq!(state.attach_request, None);
    }

    #[test]
    fn new_attachment_replaces_the_unsolicited_event_source() {
        let root = RemotePath::new(b"/project".to_vec()).unwrap();
        let client = RemoteClient::new(NullStream, identity(), root.clone()).unwrap();
        let first = client
            .request(
                Operation::Open {
                    directory: root,
                    target: LaunchTarget::Shell,
                    rows: 24,
                    cols: 80,
                },
                None,
            )
            .unwrap();
        {
            let mut state = client.state.lock().unwrap();
            let identity = state.identity.clone();
            state
                .receive(WireEnvelope::new(identity, first, Operation::Attached))
                .unwrap();
            assert_eq!(state.attach_request, Some(first));
        }

        let second = client
            .request(Operation::Attach { rows: 24, cols: 80 }, None)
            .unwrap();
        {
            let mut state = client.state.lock().unwrap();
            let identity = state.identity.clone();
            state
                .receive(WireEnvelope::new(identity, second, Operation::Attached))
                .unwrap();
            assert_eq!(state.attach_request, Some(second));
        }

        client.reset_pending().unwrap();
        let mut state = client.state.lock().unwrap();
        let old = WireEnvelope::new(
            state.identity.clone(),
            first,
            Operation::SessionSnapshot {
                snapshot: snapshot(1),
            },
        );
        assert!(state.receive(old).is_ok());
        assert!(
            state.sessions.is_empty(),
            "an old attach id must not remain an authorized event source"
        );
        let current = WireEnvelope::new(
            state.identity.clone(),
            second,
            Operation::SessionSnapshot {
                snapshot: snapshot(2),
            },
        );
        assert!(state.receive(current).is_ok());
        assert_eq!(state.sessions.len(), 1);
    }
}
