//! Central sanitization for user-visible and logged text.
//!
//! All display and log text is untrusted: session titles, paths, GitHub/OpenCode
//! error strings, worktree branch names, and bridge close reasons. Control
//! characters can inject terminal escape sequences when a log is printed;
//! bidi/format controls can visually reorder a title or path. This module is the
//! single place that strips them before any text reaches a label or a log.

use made_bridge::security::{redact_secret, Token};

/// Sanitize arbitrary display text without truncating it.
///
/// Newlines, tabs, escape sequences, and other control characters become a
/// space so words remain separated. Bidi and zero-width format controls are
/// removed because they have no legitimate place in a title/path/status line.
pub fn sanitize_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            output.push(' ');
        } else if is_format_control(character) {
            // Remove bidi/zero-width/format controls entirely.
        } else {
            output.push(character);
        }
    }
    output
}

/// Sanitize a log message. Logs are single-line, so the same control stripping
/// as display text is required.
pub fn sanitize_log_text(text: &str) -> String {
    sanitize_text(text)
}

/// Sanitize text and bound it to `max_chars`, appending an ellipsis when cut.
pub fn sanitize_display(text: &str, max_chars: usize) -> String {
    let sanitized = sanitize_text(text);
    if max_chars == 0 {
        return String::new();
    }
    let count = sanitized.chars().count();
    if count <= max_chars {
        return sanitized;
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut truncated: String = sanitized.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

/// Prepare text for the clipboard: sanitize display controls, optionally
/// redact the bridge secret, then bound it.
///
/// Used for the copyable diagnostics/log path and the full diagnostics report.
pub fn sanitize_copy_text(text: &str, token: Option<&Token>, max_chars: usize) -> String {
    let sanitized = sanitize_display(text, max_chars);
    match token {
        Some(token) => redact_secret(&sanitized, token),
        None => sanitized,
    }
}

fn is_format_control(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}' // soft hyphen
            | '\u{061C}' // Arabic letter mark
            | '\u{180E}' // Mongolian vowel separator
            | '\u{200B}'..='\u{200F}' // zero-width space/joiners, LRM, RLM
            | '\u{202A}'..='\u{202E}' // bidi embedding/override
            | '\u{2060}'..='\u{2064}' // word joiner and invisible operators
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{206A}'..='\u{206F}' // deprecated format controls
            | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation controls
            | '\u{1D173}'..='\u{1D17A}' // musical format controls
            | '\u{E0001}' // language tag
            | '\u{E0020}'..='\u{E007F}' // tag characters
            | '\u{FEFF}' // zero-width no-break space
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use made_bridge::security::Token;

    #[test]
    fn controls_escape_sequences_bidi_and_zero_width_are_stripped() {
        let raw = "safe\u{1b}]0;owned\u{7}\nnext\u{202e}reversed\u{200b}zero\u{feff}end";
        let clean = sanitize_display(raw, 200);
        assert!(!clean.contains('\u{1b}'));
        assert!(!clean.contains('\u{7}'));
        assert!(!clean.contains('\n'));
        assert!(!clean.contains('\u{202e}'));
        assert!(!clean.contains('\u{200b}'));
        assert!(!clean.contains('\u{feff}'));
        assert!(clean.contains("safe"));
        assert!(clean.contains("next"));
    }

    #[test]
    fn copy_text_redacts_token_after_sanitizing() {
        let token = Token::from_bytes([7; 32]);
        let secret = token.expose_secret();
        let raw = format!("path/{secret}\u{202e}\n");
        let copied = sanitize_copy_text(&raw, Some(&token), 4096);
        assert!(!copied.contains(secret));
        assert!(!copied.contains('\u{202e}'));
        assert!(!copied.contains('\n'));
        assert!(copied.contains("[redacted]"));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(sanitize_display("日本語テキスト", 4), "日本語…");
    }
}
