//! Optional syntax highlighting with plain-text fallback (Phase 2, tasks
//! 2.13-2.14).
//!
//! Highlighting is a SHOULD. `syntect` is used with its pure-Rust regex engine
//! so the explorer needs no C or oniguruma dependency. Unsupported extensions,
//! binary-looking content, oversized content, and any highlighter error fall
//! back to plain text with the same owned span shape.
//!
//! The output is UI-neutral: one [`HighlightedLine`] per source line, each with
//! foreground/font-style spans. `Highlighter` performs no filesystem access; the
//! caller passes text that already came through the preview loader and path
//! guard.

use std::path::Path;

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::preview::BINARY_SNIFF_LEN;

/// Maximum text size that is syntax-highlighted (256 KiB).
///
/// This is deliberately much smaller than the 2 MiB preview limit: highlighting
/// is CPU-intensive and currently runs on the caller's thread. Larger files are
/// still previewed as plain text, but the UI thread never pays the highlighter
/// cost for them.
pub const HIGHLIGHT_SIZE_LIMIT: u64 = 256 * 1024;

/// Default theme name used when available.
pub const DEFAULT_THEME: &str = "base16-ocean.dark";

/// RGB foreground color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Creates an RGB color.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl Default for Rgb {
    fn default() -> Self {
        Self {
            r: 220,
            g: 223,
            b: 228,
        }
    }
}

/// Font style attached to a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    /// Foreground color.
    pub foreground: Rgb,
    /// Bold text.
    pub bold: bool,
    /// Italic text.
    pub italic: bool,
    /// Underlined text.
    pub underline: bool,
}

/// A contiguous run of text with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    /// Text in the run, excluding the line terminator.
    pub text: String,
    /// Span style.
    pub style: TextStyle,
}

/// One rendered source line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HighlightedLine {
    /// Styled runs. Empty source lines have no spans.
    pub spans: Vec<StyledSpan>,
}

impl HighlightedLine {
    /// Reconstructs the plain text of the line from its spans.
    #[must_use]
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

/// Result of highlighting one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightedFile {
    /// Syntax name when highlighting was applied.
    pub syntax: Option<String>,
    /// Visible lines in source order.
    pub lines: Vec<HighlightedLine>,
    /// `true` when the plain-text fallback was used.
    pub plain_text: bool,
}

impl HighlightedFile {
    /// Returns `true` when highlighting was applied.
    #[must_use]
    pub const fn is_highlighted(&self) -> bool {
        !self.plain_text
    }

    /// Reconstructs the plain text, one line per [`HighlightedFile::lines`]
    /// entry. Line terminators are not included.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(HighlightedLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Reusable pure-Rust syntax highlighter.
///
/// Construction loads the bundled default syntax and theme sets once. The value
/// is immutable afterwards and can be reused across preview loads, including on
/// a background thread.
#[derive(Debug, Clone)]
pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
    default_style: TextStyle,
}

impl Highlighter {
    /// Creates a highlighter with the default syntax and theme sets.
    #[must_use]
    pub fn new() -> Self {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .get(DEFAULT_THEME)
            .cloned()
            .or_else(|| themes.themes.values().next().cloned())
            .unwrap_or_default();
        let default_style = default_style(&theme);
        Self {
            syntaxes,
            theme,
            default_style,
        }
    }

    /// Returns the default foreground style used by the fallback.
    #[must_use]
    pub const fn default_style(&self) -> TextStyle {
        self.default_style
    }

    /// Highlights `text` for `path`, or returns a plain-text fallback.
    ///
    /// The extension selects the syntax. Unknown extensions, NUL bytes in the
    /// first 8 KiB, content over [`HIGHLIGHT_SIZE_LIMIT`], and parser errors all
    /// return owned plain spans instead of failing.
    #[must_use]
    pub fn highlight(&self, path: &Path, text: &str) -> HighlightedFile {
        if should_use_plain_text(text) {
            return self.plain(text);
        }

        let Some(syntax) = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .and_then(|extension| self.syntaxes.find_syntax_by_extension(&extension))
        else {
            return self.plain(text);
        };

        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        let mut lines = Vec::new();
        for line in source_lines(text) {
            let mut source = String::with_capacity(line.len() + 1);
            source.push_str(line);
            source.push('\n');

            match highlighter.highlight_line(&source, &self.syntaxes) {
                Ok(spans) => lines.push(styled_line(spans)),
                Err(_) => return self.plain(text),
            }
        }

        HighlightedFile {
            syntax: Some(syntax.name.clone()),
            lines,
            plain_text: false,
        }
    }

    /// Returns plain text spans without consulting the syntax set.
    #[must_use]
    pub fn plain(&self, text: &str) -> HighlightedFile {
        let lines = source_lines(text)
            .map(|line| HighlightedLine {
                spans: if line.is_empty() {
                    Vec::new()
                } else {
                    vec![StyledSpan {
                        text: line.to_string(),
                        style: self.default_style,
                    }]
                },
            })
            .collect();
        HighlightedFile {
            syntax: None,
            lines,
            plain_text: true,
        }
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

fn should_use_plain_text(text: &str) -> bool {
    text.len() as u64 > HIGHLIGHT_SIZE_LIMIT
        || text
            .as_bytes()
            .iter()
            .take(BINARY_SNIFF_LEN)
            .any(|byte| *byte == 0)
}

fn source_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn styled_line(spans: Vec<(Style, &str)>) -> HighlightedLine {
    let spans = spans
        .into_iter()
        .filter_map(|(style, text)| {
            let text = text.strip_suffix('\n').unwrap_or(text);
            if text.is_empty() {
                None
            } else {
                Some(StyledSpan {
                    text: text.to_string(),
                    style: text_style(style),
                })
            }
        })
        .collect();
    HighlightedLine { spans }
}

fn default_style(theme: &Theme) -> TextStyle {
    let foreground = theme
        .settings
        .foreground
        .map(|color| Rgb::new(color.r, color.g, color.b))
        .unwrap_or_default();
    TextStyle {
        foreground,
        bold: false,
        italic: false,
        underline: false,
    }
}

fn text_style(style: Style) -> TextStyle {
    TextStyle {
        foreground: Rgb::new(style.foreground.r, style.foreground.g, style.foreground.b),
        bold: style.font_style.contains(FontStyle::BOLD),
        italic: style.font_style.contains(FontStyle::ITALIC),
        underline: style.font_style.contains(FontStyle::UNDERLINE),
    }
}
