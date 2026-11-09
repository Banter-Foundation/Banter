//! Discord-style inline preview for text document attachments in chat (BA-121).
//!
//! When a message shows a text-document attachment, Banter offers an expandable
//! inline preview (filename header + body). There is **no** line-count threshold
//! that refuses preview; a later pass may add one. Collapsed chrome may soft-window
//! the body for layout — that is not an eligibility gate.
//!
//! Out of scope here: BFMD/code-block syntax highlighting and the text-block
//! message type (other workers).

use std::collections::HashSet;
use std::path::Path;

/// Soft collapsed viewport height in lines (Discord-like chrome).
/// Long files still get a preview; this only shapes the collapsed pane.
pub const COLLAPSED_VIEW_LINES: usize = 10;

/// Why text bytes could not become a preview body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextPreviewError {
    /// NUL bytes or invalid UTF-8 — treat as non-text.
    Binary,
}

/// Filename (or path) looks like a text document Banter should preview inline.
pub fn is_text_document(filename: &str) -> bool {
    let name = filename_only(filename);
    if name.is_empty() {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    let ext = match lower.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && !ext.contains('/') && !ext.contains('\\') => ext,
        // Dotfiles like `.env` — treat the whole name after the leading dot as the "ext".
        None if lower.starts_with('.') && lower.len() > 1 => &lower[1..],
        None => return false,
        Some(_) => return false,
    };
    TEXT_EXTENSIONS.binary_search(&ext).is_ok()
}

/// Decode attachment bytes as UTF-8 text for the preview pane.
/// Strips a leading UTF-8 BOM. Rejects NULs and invalid UTF-8 as binary.
pub fn decode_text_bytes(bytes: &[u8]) -> Result<String, TextPreviewError> {
    if bytes.contains(&0) {
        return Err(TextPreviewError::Binary);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|_| TextPreviewError::Binary)
}

/// Count logical lines (same rules as [`str::lines`]).
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.lines().count()
}

/// Whether the Expand / Collapse control should appear under the preview body.
/// Soft chrome only — never used to refuse showing a preview.
pub fn needs_expand_control(text: &str) -> bool {
    line_count(text) > COLLAPSED_VIEW_LINES
}

/// Body text drawn in the preview pane for the current expand state.
/// Expanded always returns the full document (no line cap).
pub fn preview_visible_text<'a>(text: &'a str, expanded: bool) -> &'a str {
    if expanded || !needs_expand_control(text) {
        return text;
    }
    take_first_lines(text, COLLAPSED_VIEW_LINES)
}

/// Toggle whether attachment `id` is expanded.
/// Returns whether it **was** expanded before the toggle (true → just collapsed).
pub fn toggle_expanded(expanded: &mut HashSet<i64>, id: i64) -> bool {
    if expanded.contains(&id) {
        expanded.remove(&id);
        true
    } else {
        expanded.insert(id);
        false
    }
}

/// Human-readable size for the attachment header (Discord-like).
pub fn format_byte_size(bytes: i64) -> String {
    let n = bytes.max(0) as u64;
    if n == 1 {
        return "1 byte".into();
    }
    if n < 1024 {
        return format!("{n} bytes");
    }
    let kb = n as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.1} KB");
    }
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        return format!("{mb:.1} MB");
    }
    let gb = mb / 1024.0;
    format!("{gb:.1} GB")
}

fn filename_only(filename: &str) -> &str {
    Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(filename)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
}

fn take_first_lines(text: &str, max_lines: usize) -> &str {
    if max_lines == 0 {
        return "";
    }
    let mut end = text.len();
    let mut seen = 0usize;
    for (idx, _) in text.match_indices('\n') {
        seen += 1;
        if seen == max_lines {
            // Keep the newline so the soft window ends cleanly on a line boundary
            // when the source had one; otherwise stop at the newline start.
            end = idx + 1;
            break;
        }
    }
    if seen < max_lines {
        return text;
    }
    // If we stopped on a newline and there is no more content need, trim the
    // trailing newline for display parity with `lines()` join — keep it when
    // more content follows so the collapsed pane reads like Discord's clip.
    &text[..end]
}

/// Sorted lowercase extensions Banter treats as text documents for inline preview.
/// Keep sorted for binary_search. Programming/text configs included; media/binaries excluded.
const TEXT_EXTENSIONS: &[&str] = &[
    "bash",
    "bat",
    "bfmd",
    "c",
    "cc",
    "cfg",
    "cmd",
    "conf",
    "cpp",
    "cs",
    "css",
    "csv",
    "cxx",
    "diff",
    "env",
    "fs",
    "go",
    "h",
    "hpp",
    "htm",
    "html",
    "ini",
    "java",
    "js",
    "json",
    "jsonl",
    "jsx",
    "kt",
    "kts",
    "log",
    "lua",
    "luau",
    "m",
    "markdown",
    "md",
    "mm",
    "patch",
    "php",
    "pl",
    "pm",
    "ps1",
    "py",
    "r",
    "rb",
    "rs",
    "rst",
    "sh",
    "sql",
    "swift",
    "text",
    "toml",
    "ts",
    "tsv",
    "tsx",
    "txt",
    "xml",
    "yaml",
    "yml",
    "zsh",
];
