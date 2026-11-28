//! UI-free companion contracts and remote runtime.
//!
//! The executable is `made-remote`. It does not probe, require, or fall back to tmux.
pub mod claude;
pub mod client;
pub mod clipboard;
pub mod filesystem;
pub mod git;
pub mod grok;
pub mod integration;
pub mod launch;
pub mod protocol;
pub mod sanitize;
pub mod supervisor;
pub use protocol::*;
