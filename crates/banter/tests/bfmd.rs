//! Conformance tests for BFMD parse/render (spec §12).
//!
//! Inputs are exact source strings from `docs/bfmd.md`. This is Banter's chat
//! formatting language — not CommonMark.

use banter::bfmd::{self, Flag, Rgba};

#[test]
fn example_a_bold_remains_after_close_s_c() {
    let src = "<{s:36},{c=0xFFFFFF},{b:t}> Text Goes Here <{!s},{!c}> This is still bold.";
    let runs = bfmd::parse(src).runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, " Text Goes Here ");
    assert_eq!(runs[0].style.size, Some(36.0));
    assert_eq!(runs[0].style.color, Some(Rgba::rgb(255, 255, 255)));
    assert!(runs[0].style.bold);
    assert_eq!(runs[1].text, " This is still bold.");
    assert_eq!(runs[1].style.size, None);
    assert_eq!(runs[1].style.color, None);
    assert!(runs[1].style.bold);
}

#[test]
fn example_b_size_replace_no_stack() {
    let src = "<{s:36}> Text is size 36 <{s:42}> This is size 42.";
    let runs = bfmd::parse(src).runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style.size, Some(36.0));
    assert_eq!(runs[1].style.size, Some(42.0));
}

#[test]
fn example_c_eof_closes_without_trailing_group() {
    let runs = bfmd::parse("<{b:t},{i:t}>hello").runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "hello");
    assert!(runs[0].style.bold && runs[0].style.italic);
}

#[test]
fn example_d_escaped_angle_is_literal() {
    let runs = bfmd::parse(r"use \<{s:12}> as text").runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "use <{s:12}> as text");
    assert!(runs[0].style.is_default());
}

#[test]
fn close_size_after_replace_restores_default() {
    let src = "<{s:36}>a<{s:42}>b<{!s}>c";
    let runs = bfmd::parse(src).runs;
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].style.size, Some(36.0));
    assert_eq!(runs[1].style.size, Some(42.0));
    assert_eq!(runs[2].style.size, None);
}

#[test]
fn close_all_clears_bold_and_size() {
    let runs = bfmd::parse("<{s:20},{b:t}>x<{!}>y").runs;
    assert_eq!(runs.len(), 2);
    assert!(runs[0].style.bold);
    assert!(runs[1].style.is_default());
}

#[test]
fn unknown_command_ignored_bold_still_applies() {
    let runs = bfmd::parse("<{z:1},{b:t}>hi").runs;
    assert_eq!(runs.len(), 1);
    assert!(runs[0].style.bold);
}

#[test]
fn angle_not_followed_by_brace_is_literal() {
    let runs = bfmd::parse("a < b").runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "a < b");
}

#[test]
fn hex_0x_and_hash_same_rgb() {
    let a = bfmd::parse("<{c=0xFFFFFF}>x").runs;
    let b = bfmd::parse("<{c:#FFFFFF}>x").runs;
    assert_eq!(a[0].style.color, b[0].style.color);
}

#[test]
fn hr_flag_then_clear() {
    let runs = bfmd::parse("<{hr}>\n<{!hr}>after").runs;
    assert!(runs[0].style.hr);
    assert_eq!(runs[0].text, "\n");
    let last = runs.last().unwrap();
    assert!(!last.style.hr);
    assert_eq!(last.text, "after");
}

#[test]
fn plain_text_remains_valid_bfmd() {
    let src = "hello from banter";
    let runs = bfmd::parse(src).runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(bfmd::serialize(&runs), src);
    assert_eq!(bfmd::plain_text(src), src);
}

#[test]
fn toggle_bold_italic_underline_round_trip() {
    let mut src = "alpha beta".to_string();
    src = bfmd::toggle_flag(&src, 0, 5, Flag::Bold);
    src = bfmd::toggle_flag(&src, 6, 10, Flag::Italic);
    let runs = bfmd::parse(&src).runs;
    assert!(runs.iter().any(|r| r.text == "alpha" && r.style.bold && !r.style.italic));
    assert!(runs.iter().any(|r| r.text == "beta" && r.style.italic && !r.style.bold));
    let again = bfmd::serialize(&runs);
    let reruns = bfmd::parse(&again).runs;
    assert_eq!(runs.len(), reruns.len());
    for (a, b) in runs.iter().zip(reruns.iter()) {
        assert_eq!(a.text, b.text);
        assert_eq!(a.style.bold, b.style.bold);
        assert_eq!(a.style.italic, b.style.italic);
    }
}

#[test]
fn render_plain_helper_strips_commands() {
    assert_eq!(
        bfmd::plain_text("<{b:t}>bold <{!b}>plain"),
        "bold plain"
    );
}

#[test]
fn emote_size_slot_defaults_and_clamps() {
    assert_eq!(bfmd::DEFAULT_EMOTE_PX, 22.0);
    let runs = bfmd::parse("<{e:48}>x").runs;
    assert_eq!(runs[0].style.emote_size, Some(48.0));
    let runs = bfmd::parse("<{e:48}>x<{!e}>y").runs;
    assert_eq!(runs[0].style.emote_size, Some(48.0));
    assert_eq!(runs[1].style.emote_size, None);
    // Below 16 and above 128 clamp into range.
    let low = bfmd::parse("<{e:8}>x").runs;
    assert_eq!(low[0].style.emote_size, Some(16.0));
    let high = bfmd::parse("<{e:200}>x").runs;
    assert_eq!(high[0].style.emote_size, Some(128.0));
    // Independent from text size `{s}`.
    let both = bfmd::parse("<{s:36},{e:48}>x").runs;
    assert_eq!(both[0].style.size, Some(36.0));
    assert_eq!(both[0].style.emote_size, Some(48.0));
    assert_eq!(both[0].style.emote_size_or_default(), 48.0);
    assert_eq!(bfmd::Style::default().emote_size_or_default(), 22.0);
}

#[test]
fn emote_token_scan_finds_stable_asset_ids() {
    let text = "hi :{e:42}: and :{e:7}: done";
    let hits = bfmd::scan_emote_tokens(text);
    // (byte_start, byte_len, asset_id)
    assert_eq!(hits, vec![(3, 8, 42), (16, 7, 7)]);
    // Unresolved / malformed stay text — scanner only matches `:{e:digits}:`.
    assert!(bfmd::scan_emote_tokens(":party_parrot:").is_empty());
    assert!(bfmd::scan_emote_tokens(":{e:}:").is_empty());
}

// --- Timestamps (Owl-approved; Discord + easy plain-text) ---

#[test]
fn discord_timestamp_default_style_is_f() {
    let runs = bfmd::parse("<t:0>").runs;
    assert_eq!(runs.len(), 1);
    let ts = runs[0].timestamp.expect("timestamp run");
    assert_eq!(ts.unix_secs, 0);
    assert_eq!(ts.style, bfmd::TimestampStyle::ShortDateTime);
    assert!(runs[0].text.is_empty());
}

#[test]
fn discord_timestamp_all_usual_style_flags() {
    let cases = [
        ('t', bfmd::TimestampStyle::ShortTime),
        ('T', bfmd::TimestampStyle::LongTime),
        ('d', bfmd::TimestampStyle::ShortDate),
        ('D', bfmd::TimestampStyle::LongDate),
        ('f', bfmd::TimestampStyle::ShortDateTime),
        ('F', bfmd::TimestampStyle::LongDateTime),
        ('R', bfmd::TimestampStyle::Relative),
    ];
    for (flag, style) in cases {
        let src = format!("<t:1700000000:{flag}>");
        let runs = bfmd::parse(&src).runs;
        assert_eq!(runs.len(), 1, "flag {flag}");
        let ts = runs[0].timestamp.expect("timestamp");
        assert_eq!(ts.unix_secs, 1_700_000_000);
        assert_eq!(ts.style, style);
    }
}

#[test]
fn discord_timestamp_amid_text_and_styles() {
    let src = "see you <t:1700000000:R> then <{b}>bold";
    let runs = bfmd::parse(src).runs;
    assert!(runs.len() >= 3);
    assert_eq!(runs[0].text, "see you ");
    assert!(runs[0].timestamp.is_none());
    let ts = runs[1].timestamp.expect("mid timestamp");
    assert_eq!(ts.unix_secs, 1_700_000_000);
    assert_eq!(ts.style, bfmd::TimestampStyle::Relative);
    let bold = runs.iter().find(|r| r.text == "bold").expect("bold run");
    assert!(bold.style.bold);
}

#[test]
fn discord_timestamp_invalid_stays_literal() {
    for src in ["<t:abc>", "<t:>", "<t:12:z>", "<t:12:ff>", "a <t:not> b"] {
        let runs = bfmd::parse(src).runs;
        assert!(
            runs.iter().all(|r| r.timestamp.is_none()),
            "expected literal for {src:?}, got {runs:?}"
        );
        // Entire source remains visible plain text (no token consumption).
        assert_eq!(bfmd::plain_text(src), src);
    }
}

#[test]
fn discord_timestamp_allows_negative_unix() {
    let ts = bfmd::parse("<t:-60:R>").runs[0]
        .timestamp
        .expect("negative unix");
    assert_eq!(ts.unix_secs, -60);
}

#[test]
fn escaped_discord_timestamp_is_literal() {
    let runs = bfmd::parse(r"\<t:0:R>").runs;
    assert_eq!(runs.len(), 1);
    assert!(runs[0].timestamp.is_none());
    assert_eq!(runs[0].text, "<t:0:R>");
}

#[test]
fn easy_timestamp_utc_z_parses_to_unix() {
    // Deterministic: Zulu / offset forms do not depend on writer local TZ.
    let runs = bfmd::parse("@t[1970-01-01 00:00:00Z]").runs;
    assert_eq!(runs.len(), 1);
    let ts = runs[0].timestamp.expect("easy @t");
    assert_eq!(ts.unix_secs, 0);
    assert_eq!(ts.style, bfmd::TimestampStyle::ShortDateTime);
}

#[test]
fn easy_timestamp_iso_t_separator_and_offset() {
    let runs = bfmd::parse("@t[1970-01-01T00:00:00+00:00 R]").runs;
    let ts = runs[0].timestamp.expect("iso easy");
    assert_eq!(ts.unix_secs, 0);
    assert_eq!(ts.style, bfmd::TimestampStyle::Relative);
}

#[test]
fn easy_timestamp_ampm_with_offset() {
    let runs = bfmd::parse("@t[1970-01-01 12:00 AM +00:00]").runs;
    let ts = runs[0].timestamp.expect("ampm");
    assert_eq!(ts.unix_secs, 0);
    let runs = bfmd::parse("@t[1970-01-01 12:00 PM +00:00 t]").runs;
    let ts = runs[0].timestamp.expect("ampm noon");
    assert_eq!(ts.unix_secs, 12 * 3600);
    assert_eq!(ts.style, bfmd::TimestampStyle::ShortTime);
}

#[test]
fn easy_timestamp_style_flags() {
    for (flag, style) in [
        ('t', bfmd::TimestampStyle::ShortTime),
        ('T', bfmd::TimestampStyle::LongTime),
        ('d', bfmd::TimestampStyle::ShortDate),
        ('D', bfmd::TimestampStyle::LongDate),
        ('f', bfmd::TimestampStyle::ShortDateTime),
        ('F', bfmd::TimestampStyle::LongDateTime),
        ('R', bfmd::TimestampStyle::Relative),
    ] {
        let src = format!("@t[1970-01-01 00:00:00Z {flag}]");
        let ts = bfmd::parse(&src).runs[0]
            .timestamp
            .expect("styled easy");
        assert_eq!(ts.style, style, "flag {flag}");
    }
}

#[test]
fn easy_timestamp_no_false_positives_on_ordinary_sentences() {
    let samples = [
        "meet on 2026-11-09 at 13:00 please",
        "email me at owl@test.com",
        "price is $t[99]",
        "@t missing brackets",
        "@t [2026-11-09 13:00]", // space after t — not the marker
        "see @tomorrow",
        "array[1970-01-01]",
    ];
    for src in samples {
        let runs = bfmd::parse(src).runs;
        assert!(
            runs.iter().all(|r| r.timestamp.is_none()),
            "false positive in {src:?}: {runs:?}"
        );
    }
}

#[test]
fn easy_timestamp_invalid_inside_stays_literal() {
    for src in [
        "@t[not-a-date]",
        "@t[2026-13-40 99:99]",
        "@t[2026-11-09]",
        "@t[2026-11-09 13:00 zz]",
        "@t[]",
    ] {
        let runs = bfmd::parse(src).runs;
        assert!(
            runs.iter().all(|r| r.timestamp.is_none()),
            "expected literal for {src:?}"
        );
        assert!(bfmd::plain_text(src).contains("@t["));
    }
}

#[test]
fn timestamp_serialize_canonical_discord_form() {
    let runs = bfmd::parse("@t[1970-01-01 00:00:00Z R]").runs;
    assert_eq!(bfmd::serialize(&runs), "<t:0:R>");
    let again = bfmd::parse("<t:1700000000:f>").runs;
    assert_eq!(bfmd::serialize(&again), "<t:1700000000:f>");
    // Default Discord omit-style still serializes with explicit f.
    let def = bfmd::parse("<t:42>").runs;
    assert_eq!(bfmd::serialize(&def), "<t:42:f>");
}

#[test]
fn timestamp_format_absolute_styles_use_reader_local() {
    // Discord timestamps render in the reader's local zone. Pin expectations
    // to the same Local conversion the formatter must use.
    let unix = 946_684_800i64; // 2000-01-01 00:00:00 UTC
    let local = chrono::DateTime::from_timestamp(unix, 0)
        .expect("unix")
        .with_timezone(&chrono::Local);
    // Padded numeric fields (`%I` / `%d`) — portable across Windows/Linux strftime.
    let cases = [
        (
            bfmd::TimestampStyle::ShortTime,
            local.format("%I:%M %p").to_string(),
        ),
        (
            bfmd::TimestampStyle::LongTime,
            local.format("%I:%M:%S %p").to_string(),
        ),
        (
            bfmd::TimestampStyle::ShortDate,
            local.format("%m/%d/%Y").to_string(),
        ),
        (
            bfmd::TimestampStyle::LongDate,
            local.format("%B %d, %Y").to_string(),
        ),
        (
            bfmd::TimestampStyle::ShortDateTime,
            local.format("%B %d, %Y %I:%M %p").to_string(),
        ),
        (
            bfmd::TimestampStyle::LongDateTime,
            local.format("%A, %B %d, %Y %I:%M %p").to_string(),
        ),
    ];
    for (style, expect) in cases {
        let ts = bfmd::Timestamp {
            unix_secs: unix,
            style,
        };
        // Absolute styles ignore `now`.
        let rendered = bfmd::format_timestamp(&ts, 0);
        assert_eq!(rendered, expect, "style {style:?}");
    }
}

#[test]
fn timestamp_format_relative_past_and_future() {
    let past = bfmd::Timestamp {
        unix_secs: 1_000,
        style: bfmd::TimestampStyle::Relative,
    };
    assert_eq!(bfmd::format_timestamp(&past, 1_000 + 90), "a minute ago");
    assert_eq!(bfmd::format_timestamp(&past, 1_000 + 3600 * 3), "3 hours ago");

    let future = bfmd::Timestamp {
        unix_secs: 5_000,
        style: bfmd::TimestampStyle::Relative,
    };
    assert_eq!(bfmd::format_timestamp(&future, 5_000 - 45), "in a minute");
    assert_eq!(bfmd::format_timestamp(&future, 5_000 - 86_400 * 2), "in 2 days");
}

#[test]
fn timestamp_plain_text_uses_formatted_display() {
    let src = "go <t:946684800:d> now";
    let plain = bfmd::plain_text(src);
    assert!(plain.contains("01/01/2000"), "plain was {plain:?}");
    assert!(!plain.contains("<t:"));
}

#[test]
fn timestamp_inherits_surrounding_style_slots() {
    let src = "<{b}>when <t:0:t><{!b}> after";
    let runs = bfmd::parse(src).runs;
    let ts_run = runs
        .iter()
        .find(|r| r.timestamp.is_some())
        .expect("ts run");
    assert!(ts_run.style.bold);
    let after = runs.iter().find(|r| r.text == " after").expect("after");
    assert!(!after.style.bold);
}
