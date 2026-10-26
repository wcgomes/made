//! Exact-byte input encoding tests.

use made_terminal::{
    InputEncoder, InputModes, Key, Modifiers, MouseButton, MouseEvent, MouseEventKind, MouseMode,
    TerminalCore, TerminalSize, PASTE_BYTE_LIMIT,
};

fn encoder() -> InputEncoder {
    InputEncoder::default()
}

fn encoded(key: Key, modifiers: Modifiers) -> Vec<u8> {
    encoder().encode_key(key, modifiers)
}

#[test]
fn printable_and_control_characters_use_legacy_sequences() {
    assert_eq!(encoded(Key::Char('a'), Modifiers::NONE), b"a");
    assert_eq!(encoded(Key::Char('A'), Modifiers::NONE), b"A");
    assert_eq!(encoded(Key::Char('é'), Modifiers::NONE), "é".as_bytes());
    assert_eq!(
        encoded(Key::Char('x'), Modifiers::NONE.with_alt()),
        b"\x1bx"
    );
    assert_eq!(
        encoded(Key::Char('a'), Modifiers::NONE.with_ctrl()),
        vec![0x01]
    );
    assert_eq!(
        encoded(Key::Char('a'), Modifiers::NONE.with_ctrl().with_shift()),
        vec![0x01]
    );
    assert_eq!(
        encoded(Key::Char('['), Modifiers::NONE.with_ctrl()),
        vec![0x1b]
    );
    assert_eq!(
        encoded(Key::Char('?'), Modifiers::NONE.with_ctrl()),
        vec![0x7f]
    );
    assert_eq!(
        encoded(Key::Char('c'), Modifiers::NONE.with_ctrl().with_alt()),
        vec![0x1b, 0x03]
    );
}

#[test]
fn enter_tab_backspace_and_escape_are_exact() {
    assert_eq!(encoded(Key::Enter, Modifiers::NONE), b"\r");
    assert_eq!(encoded(Key::Enter, Modifiers::NONE.with_alt()), b"\x1b\r");
    assert_eq!(encoded(Key::Tab, Modifiers::NONE), b"\t");
    assert_eq!(encoded(Key::Tab, Modifiers::NONE.with_shift()), b"\x1b[Z");
    assert_eq!(encoded(Key::Backspace, Modifiers::NONE), vec![0x7f]);
    assert_eq!(
        encoded(Key::Backspace, Modifiers::NONE.with_ctrl()),
        vec![0x08]
    );
    assert_eq!(
        encoded(Key::Backspace, Modifiers::NONE.with_alt()),
        vec![0x1b, 0x7f]
    );
    assert_eq!(encoded(Key::Escape, Modifiers::NONE), vec![0x1b]);
}

#[test]
fn cursor_and_editing_keys_use_xterm_sequences() {
    assert_eq!(encoded(Key::ArrowUp, Modifiers::NONE), b"\x1b[A");
    assert_eq!(encoded(Key::ArrowDown, Modifiers::NONE), b"\x1b[B");
    assert_eq!(encoded(Key::ArrowRight, Modifiers::NONE), b"\x1b[C");
    assert_eq!(encoded(Key::ArrowLeft, Modifiers::NONE), b"\x1b[D");
    assert_eq!(encoded(Key::Home, Modifiers::NONE), b"\x1b[H");
    assert_eq!(encoded(Key::End, Modifiers::NONE), b"\x1b[F");
    assert_eq!(encoded(Key::Insert, Modifiers::NONE), b"\x1b[2~");
    assert_eq!(encoded(Key::Delete, Modifiers::NONE), b"\x1b[3~");
    assert_eq!(encoded(Key::PageUp, Modifiers::NONE), b"\x1b[5~");
    assert_eq!(encoded(Key::PageDown, Modifiers::NONE), b"\x1b[6~");

    assert_eq!(
        encoded(Key::ArrowUp, Modifiers::NONE.with_ctrl()),
        b"\x1b[1;5A"
    );
    assert_eq!(
        encoded(Key::ArrowLeft, Modifiers::NONE.with_shift()),
        b"\x1b[1;2D"
    );
    assert_eq!(
        encoded(Key::ArrowRight, Modifiers::NONE.with_alt()),
        b"\x1b[1;3C"
    );
    assert_eq!(
        encoded(Key::ArrowDown, Modifiers::NONE.with_ctrl().with_shift()),
        b"\x1b[1;6B"
    );
    assert_eq!(
        encoded(Key::Home, Modifiers::NONE.with_ctrl()),
        b"\x1b[1;5H"
    );
    assert_eq!(encoded(Key::End, Modifiers::NONE.with_ctrl()), b"\x1b[1;5F");
    assert_eq!(
        encoded(Key::Delete, Modifiers::NONE.with_shift()),
        b"\x1b[3;2~"
    );
    assert_eq!(
        encoded(Key::PageUp, Modifiers::NONE.with_alt()),
        b"\x1b[5;3~"
    );
}

#[test]
fn application_cursor_mode_uses_ss3() {
    let encoder = InputEncoder::new(InputModes {
        app_cursor: true,
        ..InputModes::default()
    });
    assert_eq!(encoder.encode_key(Key::ArrowUp, Modifiers::NONE), b"\x1bOA");
    assert_eq!(
        encoder.encode_key(Key::ArrowLeft, Modifiers::NONE),
        b"\x1bOD"
    );
    assert_eq!(encoder.encode_key(Key::Home, Modifiers::NONE), b"\x1bOH");
    assert_eq!(encoder.encode_key(Key::End, Modifiers::NONE), b"\x1bOF");
    assert_eq!(
        encoder.encode_key(Key::ArrowUp, Modifiers::NONE.with_ctrl()),
        b"\x1b[1;5A",
        "modified arrows stay CSI even in application cursor mode"
    );
}

#[test]
fn function_keys_use_legacy_sequences() {
    assert_eq!(encoded(Key::Function(1), Modifiers::NONE), b"\x1bOP");
    assert_eq!(encoded(Key::Function(4), Modifiers::NONE), b"\x1bOS");
    assert_eq!(encoded(Key::Function(5), Modifiers::NONE), b"\x1b[15~");
    assert_eq!(encoded(Key::Function(12), Modifiers::NONE), b"\x1b[24~");
    assert_eq!(
        encoded(Key::Function(1), Modifiers::NONE.with_shift()),
        b"\x1b[1;2P"
    );
    assert_eq!(
        encoded(Key::Function(5), Modifiers::NONE.with_ctrl()),
        b"\x1b[15;5~"
    );
    assert_eq!(encoded(Key::Function(13), Modifiers::NONE), b"");
}

#[test]
fn csi_u_disambiguates_modified_keys() {
    let encoder = InputEncoder::new(InputModes {
        csi_u: true,
        ..InputModes::default()
    });

    assert_eq!(
        encoder.encode_key(Key::Enter, Modifiers::NONE.with_ctrl()),
        b"\x1b[13;5u"
    );
    assert_eq!(
        encoder.encode_key(Key::Tab, Modifiers::NONE.with_ctrl()),
        b"\x1b[9;5u"
    );
    assert_eq!(
        encoder.encode_key(Key::Tab, Modifiers::NONE.with_shift()),
        b"\x1b[9;2u"
    );
    assert_eq!(
        encoder.encode_key(Key::Char('a'), Modifiers::NONE.with_ctrl()),
        b"\x1b[97;5u"
    );
    assert_eq!(
        encoder.encode_key(Key::Char('a'), Modifiers::NONE.with_ctrl().with_shift()),
        b"\x1b[97;6u"
    );
    assert_eq!(
        encoder.encode_key(Key::Char('a'), Modifiers::NONE.with_alt()),
        b"\x1b[97;3u"
    );
    assert_eq!(
        encoder.encode_key(Key::ArrowUp, Modifiers::NONE.with_ctrl()),
        b"\x1b[57352;5u"
    );
    assert_eq!(
        encoder.encode_key(Key::Function(1), Modifiers::NONE.with_shift()),
        b"\x1b[57364;2u"
    );

    assert_eq!(
        encoder.encode_key(Key::Char('a'), Modifiers::NONE),
        b"a",
        "plain text is never wrapped in CSI-u"
    );
    assert_eq!(encoder.encode_key(Key::Enter, Modifiers::NONE), b"\r");
}

#[test]
fn bracketed_paste_wraps_only_when_enabled() {
    let plain = encoder();
    assert_eq!(plain.encode_paste("a\nb"), b"a\nb");

    let bracketed = InputEncoder::new(InputModes {
        bracketed_paste: true,
        ..InputModes::default()
    });
    assert_eq!(bracketed.encode_paste("a\nb"), b"\x1b[200~a\nb\x1b[201~");
    assert_eq!(bracketed.encode_paste(""), b"\x1b[200~\x1b[201~");
}

#[test]
fn bracketed_paste_strips_embedded_markers() {
    let encoder = InputEncoder::new(InputModes {
        bracketed_paste: true,
        ..InputModes::default()
    });
    let encoded = encoder.encode_paste("\x1b[201~rm -rf /\n\x1b[200~safe");
    assert_eq!(
        String::from_utf8(encoded).expect("UTF-8 paste"),
        "\x1b[200~ rm -rf /\n safe\x1b[201~",
        "embedded markers must be replaced leaving only the wrapper"
    );

    let report = encoder.encode_paste_limited("\x1b[201~rm -rf /");
    assert_eq!(report.stripped_markers, 1);
    assert!(!report.truncated);
    assert_eq!(
        String::from_utf8(report.bytes).expect("UTF-8 paste"),
        "\x1b[200~ rm -rf /\x1b[201~"
    );
}

#[test]
fn paste_sanitizer_prevents_marker_reconstruction() {
    let encoder = InputEncoder::new(InputModes {
        bracketed_paste: true,
        ..InputModes::default()
    });

    // Security-review input: deleting the end marker would reconstruct
    // `ESC[201~` from `ESC[20` + deleted marker + `1~`, terminating the paste
    // and executing the CR-prefixed command.
    let attack = "\x1b[20\x1b[201~1~\rtouch /tmp/made-paste-pwned";
    let report = encoder.encode_paste_limited(attack);
    let output = String::from_utf8(report.bytes).expect("UTF-8 paste");
    assert_eq!(
        output,
        "\x1b[200~\x1b[20 1~\rtouch /tmp/made-paste-pwned\x1b[201~"
    );
    let payload = &output[6..output.len() - 6];
    for marker in ["\x1b[200~", "\x1b[201~", "\u{009b}200~", "\u{009b}201~"] {
        assert!(
            !payload.contains(marker),
            "payload must never contain {marker:?}: {payload:?}"
        );
    }
    assert_eq!(report.stripped_markers, 1);
    assert_eq!(output.matches("\x1b[201~").count(), 1);

    // C1 variants must be handled the same way.
    let c1_attack = "\u{009b}20\u{009b}201~1~\rtouch /tmp/made-paste-pwned";
    let report = encoder.encode_paste_limited(c1_attack);
    let output = String::from_utf8(report.bytes).expect("UTF-8 paste");
    let payload = &output[6..output.len() - 6];
    assert_eq!(report.stripped_markers, 1);
    for marker in ["\x1b[200~", "\x1b[201~", "\u{009b}200~", "\u{009b}201~"] {
        assert!(
            !payload.contains(marker),
            "C1 payload must never contain {marker:?}: {payload:?}"
        );
    }
    assert_eq!(output.matches("\x1b[201~").count(), 1);
}

#[test]
fn paste_limit_is_utf8_safe_and_reported() {
    let encoder = InputEncoder::new(InputModes {
        bracketed_paste: true,
        ..InputModes::default()
    });

    let oversized = "a".repeat(PASTE_BYTE_LIMIT + 512);
    let report = encoder.encode_paste_limited(&oversized);
    assert!(report.truncated);
    assert_eq!(report.source_bytes, oversized.len());
    assert_eq!(report.payload_bytes, PASTE_BYTE_LIMIT);
    assert_eq!(report.bytes.len(), PASTE_BYTE_LIMIT + 12);
    assert!(report.bytes.starts_with(b"\x1b[200~"));
    assert!(report.bytes.ends_with(b"\x1b[201~"));
    let payload = &report.bytes[6..report.bytes.len() - 6];
    assert_eq!(payload.len(), PASTE_BYTE_LIMIT);
    assert_eq!(
        std::str::from_utf8(payload).expect("valid UTF-8"),
        &oversized[..PASTE_BYTE_LIMIT]
    );

    // The cap falls in the middle of a two-byte character; the encoder must
    // back off to the previous boundary rather than emit invalid UTF-8.
    let mut boundary = "a".repeat(PASTE_BYTE_LIMIT - 1);
    boundary.push('é');
    boundary.push_str("tail");
    let report = encoder.encode_paste_limited(&boundary);
    assert!(report.truncated);
    assert_eq!(report.payload_bytes, PASTE_BYTE_LIMIT - 1);
    assert_eq!(report.bytes.len(), PASTE_BYTE_LIMIT - 1 + 12);
    let payload = &report.bytes[6..report.bytes.len() - 6];
    let payload = std::str::from_utf8(payload).expect("cap must not split UTF-8");
    assert_eq!(payload, "a".repeat(PASTE_BYTE_LIMIT - 1));

    let report = encoder.encode_paste_limited(&oversized);
    assert_eq!(report.bytes, encoder.encode_paste(&oversized));
}

#[test]
fn focus_reporting_is_gated_by_mode() {
    assert_eq!(encoder().encode_focus(true), b"");
    let focus = InputEncoder::new(InputModes {
        focus_reporting: true,
        ..InputModes::default()
    });
    assert_eq!(focus.encode_focus(true), b"\x1b[I");
    assert_eq!(focus.encode_focus(false), b"\x1b[O");
}

#[test]
fn sgr_mouse_events_are_exact() {
    let modes = InputModes {
        mouse_mode: MouseMode::Click,
        sgr_mouse: true,
        ..InputModes::default()
    };
    let encoder = InputEncoder::new(modes);

    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::Left,
            row: 2,
            col: 4,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<0;5;3M"
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Release,
            button: MouseButton::Left,
            row: 2,
            col: 4,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<0;5;3m"
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::WheelUp,
            row: 0,
            col: 0,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<64;1;1M"
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::Left,
            row: 0,
            col: 0,
            modifiers: Modifiers::NONE.with_ctrl(),
        }),
        b"\x1b[<16;1;1M"
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Move,
            button: MouseButton::Left,
            row: 1,
            col: 1,
            modifiers: Modifiers::NONE,
        }),
        b"",
        "click mode ignores motion"
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Release,
            button: MouseButton::WheelDown,
            row: 1,
            col: 1,
            modifiers: Modifiers::NONE,
        }),
        b"",
        "wheel release is not reported"
    );
}

#[test]
fn mouse_modes_gate_motion_and_off_disables_everything() {
    let drag = InputEncoder::new(InputModes {
        mouse_mode: MouseMode::Drag,
        sgr_mouse: true,
        ..InputModes::default()
    });
    assert_eq!(
        drag.encode_mouse(MouseEvent {
            kind: MouseEventKind::Move,
            button: MouseButton::Left,
            row: 1,
            col: 1,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<32;2;2M"
    );
    assert_eq!(
        drag.encode_mouse(MouseEvent {
            kind: MouseEventKind::Move,
            button: MouseButton::None,
            row: 1,
            col: 1,
            modifiers: Modifiers::NONE,
        }),
        b""
    );

    let motion = InputEncoder::new(InputModes {
        mouse_mode: MouseMode::Motion,
        sgr_mouse: true,
        ..InputModes::default()
    });
    assert_eq!(
        motion.encode_mouse(MouseEvent {
            kind: MouseEventKind::Move,
            button: MouseButton::None,
            row: 1,
            col: 1,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<35;2;2M"
    );

    let off = encoder();
    assert_eq!(
        off.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::Left,
            row: 0,
            col: 0,
            modifiers: Modifiers::NONE,
        }),
        b""
    );
}

#[test]
fn legacy_mouse_encoding_is_exact() {
    let encoder = InputEncoder::new(InputModes {
        mouse_mode: MouseMode::Click,
        sgr_mouse: false,
        ..InputModes::default()
    });
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::Left,
            row: 2,
            col: 4,
            modifiers: Modifiers::NONE,
        }),
        vec![0x1b, b'[', b'M', 32, 37, 35]
    );
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Release,
            button: MouseButton::Left,
            row: 2,
            col: 4,
            modifiers: Modifiers::NONE,
        }),
        vec![0x1b, b'[', b'M', 35, 37, 35]
    );
}

#[test]
fn terminal_modes_drive_the_encoder() {
    let mut core = TerminalCore::with_size(TerminalSize { rows: 4, cols: 20 }, 100);
    core.feed(b"\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?2004h\x1b[?1004h");
    let modes = core.input_modes();
    assert!(modes.app_cursor);
    assert_eq!(modes.mouse_mode, MouseMode::Drag);
    assert!(modes.sgr_mouse);
    assert!(modes.bracketed_paste);
    assert!(modes.focus_reporting);

    let encoder = InputEncoder::new(modes);
    assert_eq!(encoder.encode_key(Key::ArrowUp, Modifiers::NONE), b"\x1bOA");
    assert_eq!(
        encoder.encode_mouse(MouseEvent {
            kind: MouseEventKind::Press,
            button: MouseButton::Left,
            row: 0,
            col: 0,
            modifiers: Modifiers::NONE,
        }),
        b"\x1b[<0;1;1M"
    );
    assert_eq!(encoder.encode_paste("x"), b"\x1b[200~x\x1b[201~");
    assert_eq!(encoder.encode_focus(true), b"\x1b[I");
}

#[test]
fn ime_commit_text_is_sent_without_wrapping() {
    let encoder = encoder();
    assert_eq!(encoder.encode_text("日本語"), "日本語".as_bytes());
    assert_eq!(encoder.encode_text(""), b"");
}
