//! OpenCode integration bridge: protocol, loopback server, config composition,
//! and state validation.
//!
//! The bridge exposes a normalized session snapshot and contains no UI code.
//! Network binding is confined to this crate, and the only permitted bind
//! address is loopback with an OS-assigned ephemeral port. See
//! `wiki/architecture.md` for the protocol boundary.
//!
//! Security invariants:
//!
//! - the listener is bound to `127.0.0.1:0` only;
//! - the per-instance token is 256 bits, compared in constant time, and never
//!   appears in logs, events, or `Debug` output;
//! - configuration composition is in-memory and never writes a user file;
//! - every plugin-supplied string that reaches an event is bounded and
//!   token-redacted.

#![forbid(unsafe_code)]

mod events;

pub mod config;
pub mod config_guard;
pub mod protocol;
pub mod security;
pub mod server;
pub mod state;

pub use security::Token;
