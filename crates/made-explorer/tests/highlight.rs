use std::path::Path;

use made_explorer::highlight::{Highlighter, HIGHLIGHT_SIZE_LIMIT};
use made_explorer::preview::PREVIEW_SIZE_LIMIT;

#[test]
fn supported_extension_returns_styled_spans_per_line() {
    let highlighter = Highlighter::new();
    let source = "fn main() {\n    let x = 42;\n}\n";
    let result = highlighter.highlight(Path::new("src/main.rs"), source);

    assert!(!result.plain_text);
    assert!(result.is_highlighted());
    assert_eq!(result.syntax.as_deref(), Some("Rust"));
    assert_eq!(result.lines.len(), 3);
    assert_eq!(result.text(), source.trim_end_matches('\n'));
    assert!(
        result.lines[0].spans.len() > 1,
        "Rust tokens should produce multiple styled spans"
    );
    assert!(result
        .lines
        .iter()
        .all(|line| line.spans.iter().all(|span| !span.text.contains('\n'))));
}

#[test]
fn unknown_extension_falls_back_to_plain_text() {
    let highlighter = Highlighter::new();
    let source = "plain text\nsecond line\n";
    let result = highlighter.highlight(Path::new("notes.made-unknown"), source);

    assert!(result.plain_text);
    assert!(!result.is_highlighted());
    assert!(result.syntax.is_none());
    assert_eq!(result.text(), "plain text\nsecond line");
    assert!(result.lines.iter().all(|line| line
        .spans
        .iter()
        .all(|span| span.style == highlighter.default_style())));
}

#[test]
fn binary_looking_text_falls_back_to_plain_text() {
    let highlighter = Highlighter::new();
    let source = "fn main() {\0}";
    let result = highlighter.highlight(Path::new("main.rs"), source);

    assert!(result.plain_text);
    assert!(result.syntax.is_none());
    assert_eq!(result.text(), source);
}

#[test]
fn oversized_text_falls_back_to_plain_text() {
    let highlighter = Highlighter::new();
    let source = "a".repeat(PREVIEW_SIZE_LIMIT as usize + 1);
    let result = highlighter.highlight(Path::new("main.rs"), &source);

    assert!(result.plain_text);
    assert!(result.syntax.is_none());
    assert_eq!(result.lines.len(), 1);
    assert_eq!(result.lines[0].text(), source);
}

#[test]
fn text_just_under_the_preview_limit_is_not_highlighted() {
    let highlighter = Highlighter::new();
    let source = "a".repeat(PREVIEW_SIZE_LIMIT as usize - 1);

    let result = highlighter.highlight(Path::new("main.rs"), &source);

    assert!(
        result.plain_text,
        "highlighting must stop below the preview limit to avoid a UI stall"
    );
    assert!(result.syntax.is_none());
    assert_eq!(result.lines.len(), 1);
    assert_eq!(result.lines[0].text(), source);
}

#[test]
fn highlight_threshold_is_lower_than_the_preview_limit() {
    const {
        assert!(HIGHLIGHT_SIZE_LIMIT < PREVIEW_SIZE_LIMIT);
        assert!(HIGHLIGHT_SIZE_LIMIT == 256 * 1024);
    }
}

#[test]
fn extension_matching_is_case_insensitive() {
    let highlighter = Highlighter::new();
    let result = highlighter.highlight(Path::new("MAIN.RS"), "fn main() {}\n");
    assert!(!result.plain_text);
    assert_eq!(result.syntax.as_deref(), Some("Rust"));
}

#[test]
fn empty_text_has_no_lines_in_either_path() {
    let highlighter = Highlighter::new();
    let highlighted = highlighter.highlight(Path::new("empty.rs"), "");
    assert!(!highlighted.plain_text);
    assert!(highlighted.lines.is_empty());
    assert_eq!(highlighted.text(), "");

    let plain = highlighter.highlight(Path::new("empty.unknown"), "");
    assert!(plain.plain_text);
    assert!(plain.lines.is_empty());
}
