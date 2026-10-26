//! Terminal domain: VT screen state, PTY adapter, and input encoding.
//!
//! The crate exposes the `TerminalCore` boundary described in the change design
//! so the VT implementation can be replaced without touching the app shell.
//! PTY/ConPTY mechanics are delegated to `made-platform`; this crate contains no
//! OS-specific code.
//!
//! # VT implementation deviation (design D6)
//!
//! The design names `libghostty-vt` as the preferred terminal core and permits
//! a fallback behind `TerminalCore` when a platform gap blocks it. This build
//! environment has no Zig toolchain, so `libghostty-vt` cannot be compiled here.
//! [`emulation::TerminalCore`] wraps `alacritty_terminal` 0.26 instead. The
//! public boundary exposes only Made-owned owned snapshots and scalar state, so
//! replacing the emulator later is a change inside [`emulation`] only. The
//! upgrade path is documented in the module and in the change notes:
//!
//! 1. replace the private `Term`/`Processor` fields with the new VT core;
//! 2. keep `TerminalSnapshot`, `CellSnapshot`, selection, title, cwd, and damage
//!    mapping identical;
//! 3. rerun `tests/emulation.rs` and the OpenCode TUI smoke test.
//!
//! Known fallback limits: `alacritty_terminal` applies Unicode width per code
//! point rather than full grapheme-cluster segmentation, so multi-code-point
//! ZWJ emoji sequences can occupy multiple cells even though single emoji and
//! CJK width are correct. This is the documented fallback behavior for v1; the
//! renderer must draw the cells it is given rather than re-measure text.
//!
//! # Public surface
//!
//! - [`emulation::TerminalCore`] — bytes in, owned screen snapshots out.
//! - [`pty::PtyProcess`] — shell spawn, bounded non-blocking reads, resize,
//!   input, child-exit and monitor-error events, and relaunch. Its debug
//!   output redacts environment values and terminal output.
//! - [`input::InputEncoder`] — legacy xterm and optional CSI-u keyboard
//!   sequences, SGR mouse reporting, bracketed paste, and focus reporting.
//!
//! There is no UI dependency, no global mutable state, and no OS-specific code.

pub mod emulation;
pub mod input;
pub mod pty;

pub use emulation::{
    CellAttributes, CellSnapshot, CursorShape, CursorSnapshot, DamageLine, LineSnapshot,
    TerminalColor, TerminalCore, TerminalDamage, TerminalSize, TerminalSnapshot,
};
pub use input::{
    InputEncoder, InputModes, Key, Modifiers, MouseButton, MouseEvent, MouseEventKind, MouseMode,
    PasteEncoding, BRACKETED_PASTE_END, BRACKETED_PASTE_START, C1_BRACKETED_PASTE_END,
    C1_BRACKETED_PASTE_START, PASTE_BYTE_LIMIT,
};
pub use pty::{
    PtyError, PtyEvent, PtyOptions, PtyProcess, DEFAULT_WRITE_QUEUE_CAPACITY, WRITE_CHUNK_MAX,
};
