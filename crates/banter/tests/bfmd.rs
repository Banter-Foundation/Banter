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

#[test]
fn spoiler_flag_then_clear() {
    let runs = bfmd::parse("<{sp}>secret<{!sp}>visible").runs;
    assert_eq!(runs.len(), 2);
    assert!(runs[0].style.spoiler);
    assert_eq!(runs[0].text, "secret");
    assert!(!runs[1].style.spoiler);
    assert_eq!(runs[1].text, "visible");
    // Flag shorthand / explicit bool / close-via-false.
    assert!(bfmd::parse("<{sp:t}>x").runs[0].style.spoiler);
    assert!(!bfmd::parse("<{sp}>x<{sp:f}>y").runs[1].style.spoiler);
}

#[test]
fn spoiler_survives_other_closes_until_sp_or_close_all() {
    let runs = bfmd::parse("<{sp},{b:t}>x<{!b}>y<{!}>z").runs;
    assert!(runs[0].style.spoiler && runs[0].style.bold);
    assert!(runs[1].style.spoiler && !runs[1].style.bold);
    assert!(runs[2].style.is_default());
}

#[test]
fn spoiler_round_trip_serialize() {
    let src = "<{sp:t}>hidden<{!sp}>out";
    let runs = bfmd::parse(src).runs;
    let again = bfmd::serialize(&runs);
    let reruns = bfmd::parse(&again).runs;
    assert_eq!(reruns[0].text, "hidden");
    assert!(reruns[0].style.spoiler);
    assert_eq!(reruns[1].text, "out");
    assert!(!reruns[1].style.spoiler);
}

#[test]
fn subtext_bfmd_slot_requires_size() {
    let runs = bfmd::parse("<{st:12}>caption<{!st}>body").runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style.subtext, Some(12.0));
    assert_eq!(runs[0].style.size_or_default(), 12.0);
    assert_eq!(runs[0].text, "caption");
    assert_eq!(runs[1].style.subtext, None);
    assert_eq!(runs[1].text, "body");
    // Missing / non-positive size is a bad value — slot stays off.
    assert_eq!(bfmd::parse("<{st}>x").runs[0].style.subtext, None);
    assert_eq!(bfmd::parse("<{st:0}>x").runs[0].style.subtext, None);
    assert_eq!(bfmd::parse("<{st:-3}>x").runs[0].style.subtext, None);
}

#[test]
fn subtext_line_prefix_negative_hash_semicolon_size() {
    // Owl-required chat syntax: `-#;<size>` at line start (size mandatory).
    let runs = bfmd::parse("-#;12 quiet line\nnormal").runs;
    assert!(runs.len() >= 2);
    assert_eq!(runs[0].style.subtext, Some(12.0));
    assert_eq!(runs[0].style.size_or_default(), 12.0);
    assert_eq!(runs[0].text, "quiet line\n");
    let last = runs.last().unwrap();
    assert_eq!(last.style.subtext, None);
    assert_eq!(last.text, "normal");
    // Marker is not visible plain text.
    assert_eq!(bfmd::plain_text("-#;12 quiet"), "quiet");
}

#[test]
fn subtext_line_prefix_rejects_discord_bare_dash_hash() {
    // Discord `-# ` (no semicolon+size) must stay literal — not Banter subtext.
    let runs = bfmd::parse("-# bare discord").runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "-# bare discord");
    assert_eq!(runs[0].style.subtext, None);
}

#[test]
fn subtext_line_prefix_not_mid_line() {
    let runs = bfmd::parse("xx -#;12 no").runs;
    assert_eq!(runs[0].text, "xx -#;12 no");
    assert_eq!(runs[0].style.subtext, None);
}

#[test]
fn subtext_line_prefix_each_line_independent() {
    let runs = bfmd::parse("-#;10 one\n-#;14 two\nthree").runs;
    let sub: Vec<_> = runs
        .iter()
        .filter(|r| r.style.subtext.is_some())
        .collect();
    assert_eq!(sub.len(), 2);
    assert_eq!(sub[0].style.subtext, Some(10.0));
    assert!(sub[0].text.starts_with("one"));
    assert_eq!(sub[1].style.subtext, Some(14.0));
    assert!(sub[1].text.starts_with("two"));
    assert_eq!(runs.last().unwrap().style.subtext, None);
    assert_eq!(runs.last().unwrap().text, "three");
}

#[test]
fn subtext_slot_round_trip_serialize() {
    let src = "<{st:11}>note<{!st}>";
    let runs = bfmd::parse(src).runs;
    let again = bfmd::serialize(&runs);
    let reruns = bfmd::parse(&again).runs;
    assert_eq!(reruns[0].style.subtext, Some(11.0));
    assert_eq!(reruns[0].text, "note");
}
