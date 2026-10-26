//! Keyboard, mouse, focus, and bracketed-paste encoding.
//!
//! The encoder is a pure function of the terminal's application-controlled
//! modes and one input event. It produces the exact byte sequence the child
//! expects; it never writes to the PTY itself.
//!
//! Legacy xterm sequences are the default. Setting [`InputModes::csi_u`] (the
//! emulator does this when an application enables the kitty keyboard protocol
//! disambiguation flag) switches modified keys to the CSI-u extended encoding,
//! which lets an application distinguish Ctrl+Enter, Ctrl+Tab, and similar keys
//! that legacy xterm collapses into control bytes or plain text.

/// Maximum payload bytes accepted from one clipboard paste.
///
/// A paste is capped before the bracketed-paste markers are added. The cap is
/// applied at a UTF-8 character boundary so the emitted bytes are always valid
/// UTF-8.
pub const PASTE_BYTE_LIMIT: usize = 1024 * 1024;

/// Bracketed-paste start marker.
pub const BRACKETED_PASTE_START: &str = "\x1b[200~";

/// Bracketed-paste end marker.
pub const BRACKETED_PASTE_END: &str = "\x1b[201~";

/// C1 (8-bit CSI) bracketed-paste start marker.
pub const C1_BRACKETED_PASTE_START: &str = "\u{009b}200~";

/// C1 (8-bit CSI) bracketed-paste end marker.
pub const C1_BRACKETED_PASTE_END: &str = "\u{009b}201~";

/// Replacement inserted for any embedded paste marker. A space deliberately
/// cannot combine with surrounding text to form a new ESC/C1 sequence.
const PASTE_MARKER_REPLACEMENT: &str = " ";

const PASTE_MARKERS: [&str; 4] = [
    BRACKETED_PASTE_START,
    BRACKETED_PASTE_END,
    C1_BRACKETED_PASTE_START,
    C1_BRACKETED_PASTE_END,
];

/// Result of encoding one paste with truncation and sanitization metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteEncoding {
    /// Bytes ready to write to the PTY.
    pub bytes: Vec<u8>,
    /// Whether the payload was capped at [`PASTE_BYTE_LIMIT`].
    pub truncated: bool,
    /// Size of the original clipboard text in bytes.
    pub source_bytes: usize,
    /// Size of the sanitized payload actually encoded (before bracket markers).
    pub payload_bytes: usize,
    /// Number of embedded legacy/C1 bracketed-paste markers replaced in the
    /// payload.
    pub stripped_markers: usize,
}

/// Key modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        alt: false,
        ctrl: false,
        super_key: false,
    };

    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub const fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub const fn with_super(mut self) -> Self {
        self.super_key = true;
        self
    }

    pub const fn is_empty(self) -> bool {
        !self.shift && !self.alt && !self.ctrl && !self.super_key
    }

    /// xterm/kitty modifier parameter: 1 + Shift + 2*Alt + 4*Ctrl + 8*Super.
    pub const fn code(self) -> u8 {
        1 + (self.shift as u8)
            + (self.alt as u8) * 2
            + (self.ctrl as u8) * 4
            + (self.super_key as u8) * 8
    }
}

/// A key independent of the windowing layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character. The caller supplies the final character; Shift is
    /// already reflected in it.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    /// F1-F12. Other values produce no bytes.
    Function(u8),
}

/// Application mouse reporting mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    /// Mouse reporting disabled.
    #[default]
    Off,
    /// Button press/release only.
    Click,
    /// Press/release and motion while a button is held.
    Drag,
    /// Every motion event, including hover.
    Motion,
}

/// Mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    None,
}

/// Mouse event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    Move,
}

/// A mouse event in zero-based viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub button: MouseButton,
    /// Zero-based row within the terminal viewport.
    pub row: u16,
    /// Zero-based column within the terminal viewport.
    pub col: u16,
    pub modifiers: Modifiers,
}

/// Terminal modes that affect input encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputModes {
    /// Application cursor keys (SS3 arrows/Home/End).
    pub app_cursor: bool,
    /// Application keypad mode (tracked for callers; this encoder does not emit
    /// keypad keys).
    pub app_keypad: bool,
    pub mouse_mode: MouseMode,
    /// SGR mouse encoding instead of legacy X10 bytes.
    pub sgr_mouse: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    /// CSI-u extended keyboard protocol for modified keys.
    pub csi_u: bool,
}

/// Stateless input encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputEncoder {
    modes: InputModes,
}

impl InputEncoder {
    pub fn new(modes: InputModes) -> Self {
        Self { modes }
    }

    pub fn modes(&self) -> InputModes {
        self.modes
    }

    pub fn set_modes(&mut self, modes: InputModes) {
        self.modes = modes;
    }

    /// Encode a key press.
    pub fn encode_key(&self, key: Key, modifiers: Modifiers) -> Vec<u8> {
        if self.modes.csi_u {
            if let Some(bytes) = encode_csi_u(key, modifiers) {
                return bytes;
            }
        }
        self.encode_legacy(key, modifiers)
    }

    /// Encode committed IME text. Composition preview text is never sent here;
    /// only the single committed string becomes terminal input.
    pub fn encode_text(&self, text: &str) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// Encode a clipboard paste, honoring bracketed paste mode.
    ///
    /// The payload is capped at [`PASTE_BYTE_LIMIT`] on a UTF-8 boundary, and
    /// embedded legacy/C1 bracketed-paste markers are replaced with spaces
    /// until none remain, so clipboard text cannot terminate the paste early or
    /// reconstruct a terminator from surrounding bytes. Use
    /// [`Self::encode_paste_limited`] when truncation or marker removal must be
    /// reported to the caller.
    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        self.encode_paste_limited(text).bytes
    }

    /// Encode a paste and report cap/sanitization metadata.
    pub fn encode_paste_limited(&self, text: &str) -> PasteEncoding {
        // Cap the source first so a multi-megabyte clipboard cannot make the
        // sanitizer scan unbounded input.
        let (bounded, truncated) = truncate_utf8(text, PASTE_BYTE_LIMIT);
        let (sanitized, stripped_markers) = sanitize_paste_markers(bounded);
        let payload = sanitized.as_str();

        let bytes = if self.modes.bracketed_paste {
            let mut bytes = Vec::with_capacity(payload.len() + 12);
            bytes.extend_from_slice(BRACKETED_PASTE_START.as_bytes());
            bytes.extend_from_slice(payload.as_bytes());
            bytes.extend_from_slice(BRACKETED_PASTE_END.as_bytes());
            bytes
        } else {
            payload.as_bytes().to_vec()
        };

        PasteEncoding {
            bytes,
            truncated,
            source_bytes: text.len(),
            payload_bytes: payload.len(),
            stripped_markers,
        }
    }

    /// Encode a focus change, honoring focus reporting mode.
    pub fn encode_focus(&self, focused: bool) -> Vec<u8> {
        if !self.modes.focus_reporting {
            return Vec::new();
        }
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    }

    /// Encode a mouse event, honoring the application's mouse mode.
    pub fn encode_mouse(&self, event: MouseEvent) -> Vec<u8> {
        if self.modes.mouse_mode == MouseMode::Off {
            return Vec::new();
        }
        let motion = event.kind == MouseEventKind::Move;
        if motion {
            match self.modes.mouse_mode {
                MouseMode::Off | MouseMode::Click => return Vec::new(),
                MouseMode::Drag if event.button == MouseButton::None => return Vec::new(),
                MouseMode::Drag | MouseMode::Motion => {}
            }
        }
        if matches!(event.button, MouseButton::WheelUp | MouseButton::WheelDown)
            && event.kind == MouseEventKind::Release
        {
            return Vec::new();
        }

        let mut button = match event.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::None => 3,
        };
        button += modifier_bits(event.modifiers);
        if motion {
            button += 32;
        }

        let release = event.kind == MouseEventKind::Release;
        if self.modes.sgr_mouse {
            let mut bytes = format!(
                "\x1b[<{};{};{}{}",
                button,
                event.col.saturating_add(1),
                event.row.saturating_add(1),
                if release { 'm' } else { 'M' }
            )
            .into_bytes();
            bytes.shrink_to_fit();
            bytes
        } else {
            if release {
                button = 3 + modifier_bits(event.modifiers);
            }
            let col = (event.col.saturating_add(1)).min(223) as u8 + 32;
            let row = (event.row.saturating_add(1)).min(223) as u8 + 32;
            vec![0x1b, b'[', b'M', 32 + button, col, row]
        }
    }

    fn encode_legacy(&self, key: Key, modifiers: Modifiers) -> Vec<u8> {
        let mut bytes = Vec::new();
        match key {
            Key::Char(character) => {
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                if modifiers.ctrl {
                    if let Some(control) = control_byte(character) {
                        bytes.push(control);
                        return bytes;
                    }
                }
                let mut encoded = [0u8; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            }
            Key::Enter => {
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                bytes.push(b'\r');
            }
            Key::Tab => {
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                if modifiers.shift {
                    bytes.extend_from_slice(b"\x1b[Z");
                } else {
                    bytes.push(b'\t');
                }
            }
            Key::Backspace => {
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                if modifiers.ctrl {
                    bytes.push(0x08);
                } else {
                    bytes.push(0x7f);
                }
            }
            Key::Escape => {
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                bytes.push(0x1b);
            }
            Key::ArrowUp => push_arrow(&mut bytes, self.modes.app_cursor, modifiers, b'A'),
            Key::ArrowDown => push_arrow(&mut bytes, self.modes.app_cursor, modifiers, b'B'),
            Key::ArrowRight => push_arrow(&mut bytes, self.modes.app_cursor, modifiers, b'C'),
            Key::ArrowLeft => push_arrow(&mut bytes, self.modes.app_cursor, modifiers, b'D'),
            Key::Home => push_home_end(&mut bytes, self.modes.app_cursor, modifiers, b'H'),
            Key::End => push_home_end(&mut bytes, self.modes.app_cursor, modifiers, b'F'),
            Key::Insert => push_csi_tilde(&mut bytes, 2, modifiers),
            Key::Delete => push_csi_tilde(&mut bytes, 3, modifiers),
            Key::PageUp => push_csi_tilde(&mut bytes, 5, modifiers),
            Key::PageDown => push_csi_tilde(&mut bytes, 6, modifiers),
            Key::Function(number) => push_function(&mut bytes, number, modifiers),
        }
        bytes
    }
}

/// Replace every embedded legacy/C1 paste marker with a safe space.
///
/// Deletion alone is unsafe: an attacker can split a terminator across a
/// deleted marker (`ESC[20` + `ESC[201~` + `1~` becomes `ESC[201~`). The
/// non-empty replacement breaks that reconstruction, and the loop re-checks the
/// result until no marker remains.
fn sanitize_paste_markers(text: &str) -> (String, usize) {
    let mut sanitized = text.to_owned();
    let mut stripped = 0usize;
    loop {
        let mut changed = false;
        for marker in PASTE_MARKERS {
            let count = sanitized.matches(marker).count();
            if count > 0 {
                stripped = stripped.saturating_add(count);
                sanitized = sanitized.replace(marker, PASTE_MARKER_REPLACEMENT);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    debug_assert!(
        !PASTE_MARKERS
            .iter()
            .any(|marker| sanitized.contains(marker)),
        "paste sanitizer must not leave any marker"
    );
    (sanitized, stripped)
}

fn truncate_utf8(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn encode_csi_u(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
    if modifiers.is_empty() {
        return None;
    }
    let code = match key {
        Key::Char(character) => {
            if !(modifiers.ctrl || modifiers.alt || modifiers.super_key) {
                return None;
            }
            character as u32
        }
        Key::Escape => 27,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        Key::ArrowLeft => 57350,
        Key::ArrowRight => 57351,
        Key::ArrowUp => 57352,
        Key::ArrowDown => 57353,
        Key::PageUp => 57354,
        Key::PageDown => 57355,
        Key::Home => 57356,
        Key::End => 57357,
        Key::Insert => 57348,
        Key::Delete => 57349,
        Key::Function(number) if (1..=12).contains(&number) => 57364 + (number as u32 - 1),
        Key::Function(_) => return None,
    };
    Some(format!("\x1b[{code};{}u", modifiers.code()).into_bytes())
}

fn control_byte(character: char) -> Option<u8> {
    match character {
        '@' | ' ' => Some(0x00),
        'a'..='z' => Some(character as u8 - b'a' + 1),
        'A'..='Z' => Some(character as u8 - b'A' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn modifier_bits(modifiers: Modifiers) -> u8 {
    (modifiers.shift as u8) * 4 + (modifiers.alt as u8) * 8 + (modifiers.ctrl as u8) * 16
}

fn push_arrow(bytes: &mut Vec<u8>, app_cursor: bool, modifiers: Modifiers, final_byte: u8) {
    if modifiers.is_empty() {
        if app_cursor {
            bytes.extend_from_slice(&[0x1b, b'O', final_byte]);
        } else {
            bytes.extend_from_slice(&[0x1b, b'[', final_byte]);
        }
    } else {
        bytes.extend_from_slice(
            format!("\x1b[1;{}{}", modifiers.code(), final_byte as char).as_bytes(),
        );
    }
}

fn push_home_end(bytes: &mut Vec<u8>, app_cursor: bool, modifiers: Modifiers, final_byte: u8) {
    if modifiers.is_empty() {
        if app_cursor {
            bytes.extend_from_slice(&[0x1b, b'O', final_byte]);
        } else {
            bytes.extend_from_slice(&[0x1b, b'[', final_byte]);
        }
    } else {
        bytes.extend_from_slice(
            format!("\x1b[1;{}{}", modifiers.code(), final_byte as char).as_bytes(),
        );
    }
}

fn push_csi_tilde(bytes: &mut Vec<u8>, number: u8, modifiers: Modifiers) {
    if modifiers.is_empty() {
        bytes.extend_from_slice(format!("\x1b[{number}~").as_bytes());
    } else {
        bytes.extend_from_slice(format!("\x1b[{number};{}~", modifiers.code()).as_bytes());
    }
}

fn push_function(bytes: &mut Vec<u8>, number: u8, modifiers: Modifiers) {
    match number {
        1..=4 => {
            let final_byte = b'P' + (number - 1);
            if modifiers.is_empty() {
                bytes.extend_from_slice(&[0x1b, b'O', final_byte]);
            } else {
                bytes.extend_from_slice(
                    format!("\x1b[1;{}{}", modifiers.code(), final_byte as char).as_bytes(),
                );
            }
        }
        5..=12 => {
            let code = match number {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return,
            };
            if modifiers.is_empty() {
                bytes.extend_from_slice(format!("\x1b[{code}~").as_bytes());
            } else {
                bytes.extend_from_slice(format!("\x1b[{code};{}~", modifiers.code()).as_bytes());
            }
        }
        _ => {}
    }
}
