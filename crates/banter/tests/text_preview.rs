//! Contract tests for Discord-style text document inline preview in chat (BA-121).
//!
//! Ask item 2: when a text document attachment is shown, offer an expandable
//! inline preview. No line-count gate refuses preview (threshold later).
//! Do not cover code-block highlighting or the text-block message type.
//!
//! These tests lock the preview helpers before UI wiring. Do not weaken them.

use banter::text_preview::{
    decode_text_bytes, format_byte_size, is_text_document, line_count, needs_expand_control,
    preview_visible_text, toggle_expanded, COLLAPSED_VIEW_LINES, TextPreviewError,
};
use std::collections::HashSet;

#[test]
fn text_extensions_are_documents() {
    for name in [
        "notes.txt",
        "README.MD",
        "changelog.markdown",
        "dump.log",
        "data.csv",
        "sheet.tsv",
        "config.json",
        "doc.xml",
        "prefs.yaml",
        "prefs.yml",
        "Cargo.toml",
        "app.ini",
        "server.cfg",
        "nginx.conf",
        ".env",
        "letter.bfmd",
        "query.sql",
        "change.diff",
        "fix.patch",
        "script.sh",
        "main.rs",
        "app.py",
        "index.js",
        "Page.tsx",
        "plain.text",
    ] {
        assert!(
            is_text_document(name),
            "{name} should be treated as a text document"
        );
    }
}

#[test]
fn binary_and_media_extensions_are_not_documents() {
    for name in [
        "photo.png",
        "shot.jpg",
        "clip.webp",
        "movie.mp4",
        "track.mp3",
        "archive.zip",
        "lib.so",
        "app.exe",
        "font.ttf",
        "blob.bin",
        "secret",
        "noext",
    ] {
        assert!(
            !is_text_document(name),
            "{name} must not open a text preview"
        );
    }
}

#[test]
fn path_uses_final_filename_only() {
    assert!(is_text_document("../secret.txt"));
    assert!(is_text_document(r"C:\Users\Owl\notes.md"));
    assert!(!is_text_document("../photo.png"));
}

#[test]
fn decode_utf8_and_strip_bom() {
    assert_eq!(decode_text_bytes(b"hello\nworld").unwrap(), "hello\nworld");
    assert_eq!(
        decode_text_bytes(b"\xEF\xBB\xBFhello").unwrap(),
        "hello"
    );
}

#[test]
fn decode_rejects_binary_nul_and_invalid_utf8() {
    assert_eq!(
        decode_text_bytes(b"ok\0no"),
        Err(TextPreviewError::Binary)
    );
    assert_eq!(
        decode_text_bytes(&[0xFF, 0xFE, 0x00]),
        Err(TextPreviewError::Binary)
    );
}

#[test]
fn no_line_cap_refuses_preview_eligibility() {
    // Owl: no threshold yet — long text documents still get a preview.
    let long = "line\n".repeat(5_000);
    assert!(line_count(&long) > COLLAPSED_VIEW_LINES);
    assert!(needs_expand_control(&short_enough()));
    assert!(needs_expand_control(&long));
    // Expanded always returns the full document; collapsed may soft-window.
    assert_eq!(preview_visible_text(&long, true), long);
    let collapsed = preview_visible_text(&long, false);
    assert!(collapsed.lines().count() <= COLLAPSED_VIEW_LINES);
    assert!(!collapsed.is_empty());
}

#[test]
fn short_document_needs_no_expand_when_within_collapsed_view() {
    let short = "a\nb\nc";
    assert!(line_count(short) <= COLLAPSED_VIEW_LINES);
    assert!(!needs_expand_control(short));
    assert_eq!(preview_visible_text(short, false), short);
    assert_eq!(preview_visible_text(short, true), short);
}

#[test]
fn expand_toggle_is_per_attachment_id() {
    let mut open = HashSet::new();
    assert!(!toggle_expanded(&mut open, 7));
    assert!(open.contains(&7));
    assert!(toggle_expanded(&mut open, 7));
    assert!(!open.contains(&7));
    assert!(!toggle_expanded(&mut open, 9));
    assert!(open.contains(&9));
    assert!(!open.contains(&7));
}

#[test]
fn byte_size_label_is_human_readable() {
    assert_eq!(format_byte_size(0), "0 bytes");
    assert_eq!(format_byte_size(1), "1 byte");
    assert_eq!(format_byte_size(999), "999 bytes");
    assert_eq!(format_byte_size(1024), "1.0 KB");
    assert_eq!(format_byte_size(1536), "1.5 KB");
    assert_eq!(format_byte_size(1_048_576), "1.0 MB");
}

fn short_enough() -> String {
    // One line past the collapsed chrome so Expand is offered, without any
    // eligibility gate that would refuse the preview entirely.
    "line\n".repeat(COLLAPSED_VIEW_LINES + 1)
}
