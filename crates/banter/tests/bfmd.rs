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

/// Cited Quote (`{cq}`): named quote feature — not "block quote".
#[test]
fn cited_quote_flag_forces_effective_italic() {
    let runs = bfmd::parse("<{cq}>to be or not to be<{!cq}>plain").runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "to be or not to be");
    assert!(runs[0].style.cited_quote.is_some());
    assert!(runs[0].style.effective_italic());
    // Underlying `{i}` slot may stay off; italic is composed on by the quote.
    assert!(!runs[0].style.italic);
    assert_eq!(runs[1].text, "plain");
    assert!(runs[1].style.cited_quote.is_none());
    assert!(!runs[1].style.effective_italic());
}

#[test]
fn cited_quote_cannot_turn_italics_off_while_active() {
    let runs = bfmd::parse("<{cq}>quoted<{!i}>still quote<{!cq}>after").runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "quotedstill quote");
    assert!(runs[0].style.cited_quote.is_some());
    assert!(runs[0].style.effective_italic());
    assert_eq!(runs[1].text, "after");
    assert!(!runs[1].style.effective_italic());
}

#[test]
fn cited_quote_with_message_id_for_reply_jump() {
    let runs = bfmd::parse("<{cq:42}>parent snippet<{!cq}>my reply").runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].style.cited_quote,
        Some(bfmd::CitedQuote::Message(42))
    );
    assert_eq!(runs[0].style.cite_message_id(), Some(42));
    assert!(runs[0].style.effective_italic());
    assert_eq!(runs[1].style.cite_message_id(), None);
}

#[test]
fn cited_quote_plain_and_false_clear() {
    let on = bfmd::parse("<{cq:t}>line").runs;
    assert_eq!(on[0].style.cited_quote, Some(bfmd::CitedQuote::Plain));
    let off = bfmd::parse("<{cq}>a<{cq:f}>b").runs;
    assert!(off[0].style.cited_quote.is_some());
    assert!(off[1].style.cited_quote.is_none());
}

#[test]
fn cited_quote_round_trips_canonical() {
    let src = "<{cq:7}>famous line<{!cq}>ok";
    let runs = bfmd::parse(src).runs;
    let again = bfmd::serialize(&runs);
    let reruns = bfmd::parse(&again).runs;
    assert_eq!(runs, reruns);
    assert!(again.contains("{cq:7}"));
    assert!(again.contains("{!cq}"));
}

#[test]
fn cited_quote_helper_wraps_message_excerpt() {
    let src = bfmd::cited_quote(99, "hello <world>");
    let runs = bfmd::parse(&src).runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "hello <world>");
    assert_eq!(
        runs[0].style.cited_quote,
        Some(bfmd::CitedQuote::Message(99))
    );
    assert!(runs[0].style.effective_italic());
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
