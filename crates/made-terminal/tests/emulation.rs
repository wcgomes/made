//! Terminal core behavior tests: truecolor, alternate screen, scrollback,
//! title, OSC 7 cwd, Unicode width, damage, resize, and selection/copy.

use made_platform::{Clipboard, InMemoryClipboard};
use made_terminal::{CellSnapshot, QueriedColor, TerminalColor, TerminalCore, TerminalSize};

fn terminal(rows: usize, cols: usize) -> TerminalCore {
    TerminalCore::with_size(TerminalSize { rows, cols }, 200)
}

fn cell(snapshot: &made_terminal::TerminalSnapshot, row: usize, col: usize) -> &CellSnapshot {
    snapshot
        .cell(row, col)
        .unwrap_or_else(|| panic!("missing cell {row},{col}"))
}

#[test]
fn truecolor_sgr_reaches_the_snapshot() {
    let mut core = terminal(4, 20);
    core.feed(b"\x1b[38;2;10;20;30m\x1b[48;2;40;50;60mX");
    let snapshot = core.snapshot();
    let first = cell(&snapshot, 0, 0);
    assert_eq!(first.ch, 'X');
    assert_eq!(first.fg, TerminalColor::Rgb(10, 20, 30));
    assert_eq!(first.bg, TerminalColor::Rgb(40, 50, 60));

    core.feed(b"\x1b[0m");
    core.feed(b"\x1b[31mR\x1b[0m");
    assert_eq!(
        cell(&core.snapshot(), 0, 1).fg,
        TerminalColor::Indexed(1),
        "basic ANSI colors map to palette indices"
    );
}

#[test]
fn alternate_screen_switches_and_restores_primary_content() {
    let mut core = terminal(4, 20);
    core.feed(b"primary");
    assert_eq!(core.snapshot().text(), "primary");
    assert!(!core.snapshot().alternate_screen);

    core.feed(b"\x1b[?1049h\x1b[H");
    assert!(core.snapshot().alternate_screen);
    core.feed(b"alternate");
    assert_eq!(core.snapshot().text(), "alternate");
    assert!(core.alternate_screen());

    core.feed(b"\x1b[?1049l");
    assert!(!core.snapshot().alternate_screen);
    assert_eq!(core.snapshot().text(), "primary");
}

#[test]
fn scrollback_is_a_bounded_ring_and_keeps_the_newest_lines() {
    let mut core = TerminalCore::with_size(TerminalSize { rows: 3, cols: 24 }, 8);
    for index in 0..30 {
        core.feed(format!("line{index}\r\n").as_bytes());
    }
    assert_eq!(core.history_len(), 8);
    assert_eq!(core.scrollback_len(), 8);
    assert_eq!(core.scrollback_limit(), 8);
    let history = core.history(4);
    assert_eq!(history.len(), 4);
    let newest = history.last().expect("history line");
    assert!(
        newest.text.contains("line2"),
        "newest history line should be from the end of the stream: {:?}",
        newest.text
    );
}

#[test]
fn osc_2_and_osc_0_set_the_title() {
    let mut core = terminal(4, 20);
    core.feed(b"\x1b]2;Made Title\x07");
    assert_eq!(core.title().as_deref(), Some("Made Title"));

    core.feed(b"\x1b]0;Other Title\x1b\\");
    assert_eq!(core.title().as_deref(), Some("Other Title"));
    assert_eq!(core.snapshot().title.as_deref(), Some("Other Title"));
}

#[test]
fn osc_7_tracks_cwd_across_split_feeds_with_percent_decoding() {
    let mut core = terminal(4, 20);
    let path = std::env::temp_dir().join("made workspace");
    let uri_path = encoded_test_uri_path(&path);
    core.feed(b"\x1b]7;file://local");
    assert_eq!(core.cwd(), None);
    core.feed(format!("host{uri_path}\x1b\\").as_bytes());

    assert_eq!(core.cwd(), Some(path.as_path()));
    assert_eq!(core.snapshot().cwd, Some(path));

    let next = std::env::temp_dir().join("project");
    core.feed(format!("\x1b]7;file://{}\x07", encoded_test_uri_path(&next)).as_bytes());
    assert_eq!(core.cwd(), Some(next.as_path()));
}

#[test]
fn unicode_and_emoji_use_emulator_widths() {
    let mut core = terminal(4, 20);
    core.feed("中文😀a".as_bytes());
    let snapshot = core.snapshot();

    assert_eq!(cell(&snapshot, 0, 0).ch, '中');
    assert!(cell(&snapshot, 0, 0).wide);
    assert!(cell(&snapshot, 0, 1).wide_spacer);
    assert_eq!(cell(&snapshot, 0, 2).ch, '文');
    assert_eq!(cell(&snapshot, 0, 4).ch, '😀');
    assert!(cell(&snapshot, 0, 4).wide);
    assert!(cell(&snapshot, 0, 5).wide_spacer);
    assert_eq!(cell(&snapshot, 0, 6).ch, 'a');
    assert_eq!(snapshot.cursor.col, 7);
}

#[test]
fn resize_reflows_the_grid_and_bumps_damage() {
    let mut core = terminal(4, 20);
    core.feed(b"resize me");
    let before = core.version();
    core.resize(6, 30);
    let snapshot = core.snapshot();
    assert_eq!(snapshot.size, TerminalSize { rows: 6, cols: 30 });
    assert_eq!(snapshot.text(), "resize me");
    assert!(core.version() > before);
    assert!(core.take_damage().full, "resize must force a full redraw");
}

#[test]
fn damage_and_version_track_terminal_changes() {
    let mut core = terminal(4, 20);
    let initial = core.take_damage();
    assert!(initial.full, "initial state is fully damaged");

    let before = core.version();
    core.feed(b"hello");
    assert!(core.version() > before);
    let damage = core.take_damage();
    assert!(!damage.is_empty(), "content change must produce damage");
    assert!(core.take_damage().is_empty(), "take_damage clears state");

    core.scroll(-1);
    assert!(!core.take_damage().is_empty());
}

#[test]
fn selection_extracts_text_and_copies_to_clipboard() {
    let mut core = terminal(4, 30);
    core.feed(b"hello world\r\nsecond line");

    core.select_start(0, 0);
    core.select_update(0, 4);
    assert!(core.has_selection());
    assert_eq!(core.selection_text().as_deref(), Some("hello"));

    let mut clipboard = InMemoryClipboard::new();
    assert!(core.copy_selection(&mut clipboard).expect("copy selection"));
    assert_eq!(clipboard.get_text().expect("clipboard"), "hello");

    core.select_start(0, 6);
    core.select_update(1, 5);
    assert_eq!(
        core.selection_text().as_deref(),
        Some("world\nsecond"),
        "multi-line selection includes wrapped rows"
    );

    core.clear_selection();
    assert!(!core.has_selection());
    assert_eq!(core.selection_text(), None);
}

#[test]
fn scrollback_scrolling_changes_the_viewport_and_selection_uses_viewport_rows() {
    let mut core = TerminalCore::with_size(TerminalSize { rows: 3, cols: 20 }, 20);
    for index in 0..12 {
        core.feed(format!("row{index}\r\n").as_bytes());
    }
    assert_eq!(core.display_offset(), 0);
    core.scroll(5);
    assert_eq!(core.display_offset(), 5);
    let snapshot = core.snapshot();
    assert_eq!(snapshot.display_offset, 5);

    core.select_start(0, 0);
    core.select_update(0, 3);
    assert!(core.selection_text().is_some());

    core.scroll_to_bottom();
    assert_eq!(core.display_offset(), 0);
}

#[test]
fn bell_and_replies_are_surfaced_without_ui_state() {
    let mut core = terminal(4, 20);
    core.feed(b"\x07");
    assert!(core.take_bell());
    assert!(!core.take_bell());

    // Device status report (DSR 5) makes the emulator request a reply.
    core.feed(b"\x1b[5n");
    assert!(!core.take_replies(|_| [0, 0, 0]).is_empty());
}

#[test]
fn color_queries_encode_the_rgb_the_caller_supplies() {
    let mut core = terminal(4, 20);
    core.feed(b"\x1b]10;?\x07\x1b]11;?\x07\x1b]12;?\x1b\\\x1b]4;8;?\x07");
    let replies = String::from_utf8(core.take_replies(|query| match query {
        QueriedColor::Foreground => [1, 2, 3],
        QueriedColor::Background => [4, 5, 6],
        QueriedColor::Cursor => [7, 8, 9],
        QueriedColor::Indexed(8) => [10, 11, 12],
        QueriedColor::Indexed(_) => [0, 0, 0],
    }))
    .expect("reply bytes");
    assert_eq!(
        replies,
        "\u{1b}]10;rgb:0101/0202/0303\u{7}\
         \u{1b}]11;rgb:0404/0505/0606\u{7}\
         \u{1b}]12;rgb:0707/0808/0909\u{1b}\\\
         \u{1b}]4;8;rgb:0a0a/0b0b/0c0c\u{7}"
    );
    assert!(core.take_replies(|_| [0, 0, 0]).is_empty());
}

#[test]
fn debug_output_omits_screen_and_title_content() {
    let mut core = terminal(4, 40);
    core.feed(b"secret-screen-text");
    core.feed(b"\x1b]2;secret-title\x07");

    let snapshot = core.snapshot();
    let debug = format!("{snapshot:?}");
    assert!(
        !debug.contains("secret-screen-text"),
        "snapshot Debug must not print terminal content: {debug}"
    );
    assert!(
        !debug.contains("secret-title"),
        "snapshot Debug must not print the window title: {debug}"
    );
    assert!(debug.contains("cell_count"));
    assert!(debug.contains("title_len"));

    let line = core.line_at(0).expect("first row");
    let line_debug = format!("{line:?}");
    assert!(!line_debug.contains("secret-screen-text"));
    assert!(line_debug.contains("cell_count"));
}
#[test]
fn osc7_rejects_remote_malformed_and_nonlocal_paths() {
    for uri in [
        "file://remote.example/tmp",
        "file://user@localhost/tmp",
        "file://localhost:22/tmp",
        "file:///tmp/%00evil",
        "file:///tmp/%FF",
        "file:///tmp/%xy",
        "file:///tmp/raw space",
        "file:///tmp/a?query",
        "file:///tmp/a#fragment",
        "file:relative",
        "file:////server/share",
        "file:///C:relative",
        "file:///tmp/%5Cserver",
        "file:///tmp/<malformed>",
        "file:///tmp/{malformed}",
    ] {
        let mut core = made_terminal::TerminalCore::new();
        core.feed(format!("\x1b]7;{uri}\x07").as_bytes());
        assert!(core.cwd().is_none(), "accepted {uri}");
    }
}

#[test]
fn osc7_bounded_reports_and_titles_never_set_cwd() {
    let mut core = made_terminal::TerminalCore::new();
    core.feed(b"\x1b]2;/tmp/title-is-not-cwd\x07");
    core.feed(format!("\x1b]7;file:///{}\x07", "a".repeat(65536)).as_bytes());
    assert!(core.cwd().is_none());
    let path = std::env::temp_dir().join("a é");
    core.feed(
        format!(
            "\x1b]7;file://localhost{}\x1b\\",
            encoded_test_uri_path(&path)
        )
        .as_bytes(),
    );
    assert_eq!(core.cwd(), Some(path.as_path()));
}

fn encoded_test_uri_path(path: &std::path::Path) -> String {
    let normalized = path.to_str().unwrap().replace('\\', "/");
    let mut result = if normalized.starts_with('/') {
        String::new()
    } else {
        "/".into()
    };
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || b"/:-._~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").unwrap();
        }
    }
    result
}
