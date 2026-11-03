//! Authenticated raw loopback relay for companion frontends. This module knows
//! no remote domain DTOs and cannot introduce a bridge -> remote dependency.
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

pub const MAX_FRAME: usize = 256 * 1024;
pub const AUTH_DEADLINE: Duration = Duration::from_secs(5);
/// One listener per live tab. Authentication happens before disclosing any byte.
pub struct RpcListener {
    listener: TcpListener,
    capability: [u8; 32],
    authority: [u8; 16],
}
impl RpcListener {
    pub fn bind(capability: [u8; 32], authority: [u8; 16]) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            capability,
            authority,
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    /// The caller schedules authentication on bounded workers (at most eight).
    pub fn accept(&self) -> io::Result<Option<TcpStream>> {
        match self.listener.accept() {
            Ok((stream, peer)) if peer.ip().is_loopback() => Ok(Some(stream)),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "non-loopback peer",
            )),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn authenticate(&self, mut stream: TcpStream) -> io::Result<RpcConnection> {
        let deadline = Instant::now() + AUTH_DEADLINE;
        let mut hello = [0; 56];
        read_deadline(&mut stream, &mut hello, deadline)?;
        let diff = hello[..32]
            .iter()
            .zip(self.capability)
            .fold(0u8, |d, (a, b)| d | (a ^ b));
        // Same constant-time fold as the capability. Slice `!=` would return at
        // the first mismatch and leak the authority prefix.
        let authority_diff = hello[32..48]
            .iter()
            .zip(self.authority)
            .fold(0u8, |d, (a, b)| d | (a ^ b));
        let epoch = u64::from_be_bytes(hello[48..].try_into().map_err(|_| invalid())?);
        if diff != 0 || authority_diff != 0 || epoch == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "authentication rejected",
            ));
        }
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(RpcConnection { stream, epoch })
    }
}
pub struct RpcConnection {
    stream: TcpStream,
    epoch: u64,
}
impl RpcConnection {
    pub fn connect(
        addr: SocketAddr,
        capability: &[u8; 32],
        authority: &[u8; 16],
        epoch: u64,
    ) -> io::Result<Self> {
        if !addr.ip().is_loopback() || epoch == 0 {
            return Err(invalid());
        }
        let mut stream = TcpStream::connect_timeout(&addr, AUTH_DEADLINE)?;
        stream.set_write_timeout(Some(AUTH_DEADLINE))?;
        stream.write_all(capability)?;
        stream.write_all(authority)?;
        stream.write_all(&epoch.to_be_bytes())?;
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(Self { stream, epoch })
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }
    /// A partial frame timeout is terminal: never resume at an uncertain boundary.
    pub fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut length = [0; 4];
        read_deadline(&mut self.stream, &mut length, deadline)?;
        let n = u32::from_be_bytes(length) as usize;
        if n == 0 || n > MAX_FRAME {
            return Err(invalid());
        }
        let mut frame = vec![0; n];
        read_deadline(&mut self.stream, &mut frame, deadline)?;
        Ok(frame)
    }
    pub fn write_frame(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(invalid());
        }
        self.stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.stream.write_all(bytes)
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid relay frame")
}
fn read_deadline(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "relay deadline"))?;
        stream.set_read_timeout(Some(remaining.min(Duration::from_millis(100))))?;
        match stream.read(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "relay closed")),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
