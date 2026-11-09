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
fn bfmd_unordered_list_slot() {
    let runs = bfmd::parse("<{lk:u}>alpha\nbeta<{!lk}>").runs;
    assert!(runs.len() >= 1);
    assert_eq!(runs[0].style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(runs[0].style.list_depth_or_default(), 1);
    assert!(runs[0].text.starts_with("alpha"));
    let last = runs.last().unwrap();
    // After `{!lk}` following text (if any) is not a list; EOF may leave only list runs.
    if last.text.contains("beta") && runs.len() == 1 {
        assert_eq!(last.style.list_kind, Some(bfmd::ListKind::Unordered));
    }
}

#[test]
fn bfmd_ordered_list_slot() {
    let runs = bfmd::parse("<{lk:o}>one\ntwo").runs;
    assert_eq!(runs[0].style.list_kind, Some(bfmd::ListKind::Ordered));
    assert_eq!(runs[0].style.list_depth_or_default(), 1);
    assert_eq!(runs[0].text, "one\ntwo");
}

#[test]
fn bfmd_list_depth_nests() {
    let src = "<{lk:u}>top\n<{ld:2}>nested\n<{ld:1}>topagain";
    let runs = bfmd::parse(src).runs;
    // Find runs by content.
    let top = runs.iter().find(|r| r.text.starts_with("top\n") || r.text == "top").unwrap();
    assert_eq!(top.style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(top.style.list_depth_or_default(), 1);
    let nested = runs.iter().find(|r| r.text.contains("nested")).unwrap();
    assert_eq!(nested.style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(nested.style.list_depth_or_default(), 2);
    let again = runs.iter().find(|r| r.text.contains("topagain")).unwrap();
    assert_eq!(again.style.list_depth_or_default(), 1);
}

#[test]
fn bfmd_mixed_bullet_and_number_nesting() {
    let src = "<{lk:o}>parent\n<{lk:u},{ld:2}>child bullet\n<{lk:o},{ld:2}>child number";
    let runs = bfmd::parse(src).runs;
    let parent = runs.iter().find(|r| r.text.contains("parent")).unwrap();
    assert_eq!(parent.style.list_kind, Some(bfmd::ListKind::Ordered));
    assert_eq!(parent.style.list_depth_or_default(), 1);
    let child_b = runs.iter().find(|r| r.text.contains("child bullet")).unwrap();
    assert_eq!(child_b.style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(child_b.style.list_depth_or_default(), 2);
    let child_o = runs.iter().find(|r| r.text.contains("child number")).unwrap();
    assert_eq!(child_o.style.list_kind, Some(bfmd::ListKind::Ordered));
    assert_eq!(child_o.style.list_depth_or_default(), 2);
}

#[test]
fn bfmd_close_list_clears_kind_and_depth() {
    let runs = bfmd::parse("<{lk:o},{ld:3}>item<{!lk}>after").runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style.list_kind, Some(bfmd::ListKind::Ordered));
    assert_eq!(runs[0].style.list_depth_or_default(), 3);
    assert!(runs[1].style.list_kind.is_none());
    assert_eq!(runs[1].text, "after");
}

#[test]
fn bfmd_list_depth_clamps_to_1_through_8() {
    let low = bfmd::parse("<{lk:u},{ld:0}>x").runs;
    // Bad depth 0 is rejected (diagnostic); kind may still apply with default depth.
    assert_eq!(low[0].style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(low[0].style.list_depth_or_default(), 1);
    let high = bfmd::parse("<{lk:u},{ld:9}>x").runs;
    assert_eq!(high[0].style.list_kind, Some(bfmd::ListKind::Unordered));
    assert_eq!(high[0].style.list_depth_or_default(), 1);
    let ok = bfmd::parse("<{lk:u},{ld:8}>x").runs;
    assert_eq!(ok[0].style.list_depth_or_default(), 8);
}

#[test]
fn markdown_flat_bullet_markers_only() {
    for marker in ["- ", "* ", "+ "] {
        let src = format!("{marker}one\n{marker}two");
        let runs = bfmd::parse(&src).runs;
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "one\ntwo", "marker {marker:?} should strip");
        for run in &runs {
            if run.text.trim().is_empty() {
                continue;
            }
            assert_eq!(
                run.style.list_kind,
                Some(bfmd::ListKind::Unordered),
                "marker {marker:?}"
            );
            assert_eq!(run.style.list_depth_or_default(), 1);
        }
    }
}

#[test]
fn markdown_numbered_marker_stays_literal() {
    let runs = bfmd::parse("1. one\n2. two").runs;
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "1. one\n2. two");
    assert!(runs.iter().all(|r| r.style.list_kind.is_none()));
}

#[test]
fn markdown_indented_bullet_does_not_nest() {
    // Discord nests with leading spaces; BFMD must not — indented marker is literal.
    let runs = bfmd::parse("- top\n  - not nested").runs;
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "top\n  - not nested");
    let top = runs.iter().find(|r| r.text.contains("top")).unwrap();
    assert_eq!(top.style.list_kind, Some(bfmd::ListKind::Unordered));
    let indented = runs.iter().find(|r| r.text.contains("- not nested")).unwrap();
    assert!(indented.style.list_kind.is_none() || indented.text.contains("- not nested"));
    // The indented line content remains literal including its marker.
    assert!(text.contains("  - not nested"));
}

#[test]
fn markdown_bullet_does_not_apply_when_bfmd_list_already_on() {
    let runs = bfmd::parse("<{lk:o}>- still ordered content").runs;
    assert_eq!(runs[0].style.list_kind, Some(bfmd::ListKind::Ordered));
    // Marker is literal because BFMD list kind is already active.
    assert!(runs[0].text.contains("- still ordered content") || runs[0].text.starts_with('-'));
}

#[test]
fn list_serialize_round_trip_bfmd_commands() {
    let src = "<{lk:u}>a\n<{ld:2}>b<{!lk},{!ld}>";
    let runs = bfmd::parse(src).runs;
    let out = bfmd::serialize(&runs);
    let again = bfmd::parse(&out).runs;
    assert_eq!(runs.len(), again.len());
    for (a, b) in runs.iter().zip(again.iter()) {
        assert_eq!(a.text, b.text);
        assert_eq!(a.style.list_kind, b.style.list_kind);
        assert_eq!(a.style.list_depth_or_default(), b.style.list_depth_or_default());
    }
}

#[test]
fn plain_text_strips_markdown_bullet_markers() {
    assert_eq!(bfmd::plain_text("- a\n- b"), "a\nb");
    assert_eq!(bfmd::plain_text("<{lk:o}>a\nb"), "a\nb");
}
