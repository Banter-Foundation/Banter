//! Syntax highlighting for BFMD code blocks via syntect.
//!
//! Uses the default Sublime Text syntax pack (fancy-regex backend) so Banter
//! gets a broad language list without shipping custom grammars.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// One colored fragment inside a highlighted line.
#[derive(Clone, Debug, PartialEq)]
pub struct HighlightSpan {
    pub text: String,
    /// RGBA 0..1 for ImGui.
    pub color: [f32; 4],
}

/// One language from the syntect default pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightLanguage {
    pub name: String,
    pub extensions: Vec<String>,
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

/// Languages available for `{cb:lang}` highlighting (syntect defaults).
pub fn supported_languages() -> Vec<HighlightLanguage> {
    let mut out: Vec<HighlightLanguage> = syntax_set()
        .syntaxes()
        .iter()
        .map(|s| HighlightLanguage {
            name: s.name.clone(),
            extensions: s.file_extensions.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    out
}

/// Highlight `source` for an optional BFMD language ident / extension / name.
///
/// Unknown languages fall back to a single plain span per line (no theme colors
/// beyond the default foreground). Empty source yields one empty line.
pub fn highlight(language: Option<&str>, source: &str) -> Vec<Vec<HighlightSpan>> {
    if source.is_empty() {
        return vec![vec![HighlightSpan {
            text: String::new(),
            color: plain_fg(),
        }]];
    }

    let Some(syntax) = resolve_syntax(language) else {
        return plain_lines(source);
    };

    let theme = theme_set()
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set().themes.values().next())
        .expect("syntect default themes");

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(source) {
        let ranges = match highlighter.highlight_line(line, syntax_set()) {
            Ok(r) => r,
            Err(_) => {
                lines.push(vec![HighlightSpan {
                    text: trim_line_ending(line).to_string(),
                    color: plain_fg(),
                }]);
                continue;
            }
        };
        let mut spans = Vec::new();
        for (style, text) in ranges {
            let text = trim_line_ending(text);
            if text.is_empty() && !spans.is_empty() {
                continue;
            }
            spans.push(HighlightSpan {
                text: text.to_string(),
                color: syn_color(style),
            });
        }
        if spans.is_empty() {
            spans.push(HighlightSpan {
                text: String::new(),
                color: plain_fg(),
            });
        }
        lines.push(spans);
    }
    if lines.is_empty() {
        lines.push(vec![HighlightSpan {
            text: String::new(),
            color: plain_fg(),
        }]);
    }
    // LinesWithEndings keeps a trailing empty line when source ends with `\n`.
    // Match plain_lines / paint expectations: keep visual lines without a
    // dangling empty row after a final newline when the source is "a\n".
    if source.ends_with('\n') {
        if let Some(last) = lines.last() {
            if last.len() == 1 && last[0].text.is_empty() {
                lines.pop();
            }
        }
    }
    lines
}

fn resolve_syntax(
    language: Option<&str>,
) -> Option<&'static syntect::parsing::SyntaxReference> {
    let lang = language?.trim();
    if lang.is_empty() {
        return None;
    }
    let set = syntax_set();
    set.find_syntax_by_token(lang)
        .or_else(|| set.find_syntax_by_extension(lang))
        .or_else(|| {
            let lower = lang.to_ascii_lowercase();
            set.syntaxes()
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(&lower))
        })
        .or_else(|| {
            let lower = lang.to_ascii_lowercase();
            set.syntaxes().iter().find(|s| {
                s.name.to_ascii_lowercase().contains(&lower)
                    || s.file_extensions
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&lower))
            })
        })
}

fn plain_lines(source: &str) -> Vec<Vec<HighlightSpan>> {
    let mut lines: Vec<Vec<HighlightSpan>> = source
        .split('\n')
        .map(|line| {
            vec![HighlightSpan {
                text: line.to_string(),
                color: plain_fg(),
            }]
        })
        .collect();
    if source.ends_with('\n') {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(vec![HighlightSpan {
            text: String::new(),
            color: plain_fg(),
        }]);
    }
    lines
}

fn trim_line_ending(text: &str) -> &str {
    text.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(text)
}

fn syn_color(style: SynStyle) -> [f32; 4] {
    let c = style.foreground;
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    ]
}

fn plain_fg() -> [f32; 4] {
    // Matches theme::TEXT_NORMAL approximately without depending on theme here.
    [0.859, 0.871, 0.882, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_token_resolves() {
        assert!(resolve_syntax(Some("rust")).is_some());
        assert!(resolve_syntax(Some("rs")).is_some());
        assert!(resolve_syntax(Some("Rust")).is_some());
    }

    #[test]
    fn unknown_is_none() {
        assert!(resolve_syntax(Some("not-a-real-lang-xyz")).is_none());
        assert!(resolve_syntax(None).is_none());
        assert!(resolve_syntax(Some("")).is_none());
    }
}
