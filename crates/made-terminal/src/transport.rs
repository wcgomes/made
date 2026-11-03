//! Nonblocking terminal I/O seam. Detach releases the attachment, never sends
//! a remote Close/Kill. A local child has no durable remote owner: its existing
//! PtyProcess drop/kill behavior deliberately remains unchanged.
use crate::{PtyError, PtyEvent, PtyProcess};
use made_platform::ExitStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportStatus {
    Connected,
    Disconnected,
    Exited(ExitStatus),
    Failed,
}

/// All writes are bounded all-or-nothing enqueue operations. On disconnection,
/// reject new input and discard pending input/paste; reconnect never replays it.
/// Implementations must retain byte ordering/backpressure, not drop output in
/// the middle of an escape sequence. Explicit remote Close is outside this API.
pub trait TerminalTransport: Send {
    fn poll(&mut self) -> Option<PtyEvent>;
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError>;
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError>;
    fn status(&self) -> TransportStatus;
    fn detach(&mut self) -> Result<(), PtyError>;
}

#[derive(Debug)]
pub struct LocalPtyTransport {
    process: Option<PtyProcess>,
}
impl LocalPtyTransport {
    pub fn new(process: PtyProcess) -> Self {
        Self {
            process: Some(process),
        }
    }
    pub fn process(&self) -> Option<&PtyProcess> {
        self.process.as_ref()
    }
}
impl TerminalTransport for LocalPtyTransport {
    fn poll(&mut self) -> Option<PtyEvent> {
        self.process.as_ref()?.poll()
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.process.as_mut().ok_or(PtyError::Closed)?.write(bytes)
    }
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.process
            .as_mut()
            .ok_or(PtyError::Closed)?
            .resize(rows, cols)
    }
    fn status(&self) -> TransportStatus {
        match &self.process {
            None => TransportStatus::Disconnected,
            Some(process) if process.writer_error().is_some() => TransportStatus::Failed,
            Some(process) => match process.try_is_running() {
                Err(_) => TransportStatus::Failed,
                Ok(_) => process
                    .exit_status()
                    .map(TransportStatus::Exited)
                    .unwrap_or(TransportStatus::Connected),
            },
        }
    }
    fn detach(&mut self) -> Result<(), PtyError> {
        // Taking first makes subsequent input/polls fail even if OS cleanup is
        // slow. Drop closes/discards the existing queue and kills only this PTY.
        drop(self.process.take());
        Ok(())
    }
}
