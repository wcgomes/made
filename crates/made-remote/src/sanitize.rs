//! Display sanitization shared by the companion and the desktop app, so
//! untrusted titles, names and paths are cleaned by one implementation.

/// The one display sanitization, shared by the companion and `made-app`'s
/// `sanitize_display`: control characters and line separators become a space,
/// bidirectional and format controls are removed, and text over `max_chars`
/// ends in an ellipsis.
pub fn sanitize_display(text: &str, max_chars: usize) -> String {
    let mut sanitized = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            sanitized.push(' ');
        } else if !is_format_control(character) {
            sanitized.push(character);
        }
    }
    if max_chars == 0 {
        return String::new();
    }
    if sanitized.chars().count() <= max_chars {
        return sanitized;
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut truncated: String = sanitized.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

/// Bidirectional, zero-width and other format controls that display text
/// never keeps.
pub fn is_format_control(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{206A}'..='\u{206F}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
            | '\u{FEFF}'
    )
}
