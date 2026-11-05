//! Home layout contracts (BA-81). Written before the home landing UI and kept as written.
//!
//! The rail corner control opens a customizable home page. Section order and visibility
//! persist in `home.json` under the Banter data directory (same pattern as voice.json).

use banter::{
    home_section_label, load_json, move_home_section, push_home_recent, save_json, set_home_section_visible,
    visible_home_sections, HomeLayout, HomeRecentKind, HomeRecentEntry, HomeSectionId, HOME_RECENT_LIMIT,
};
use std::path::PathBuf;

fn temp_home_json() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("banter-home-test-{}-{}.json", std::process::id(), uuid_ish()));
    path
}

fn uuid_ish() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

#[test]
fn home_layout_defaults_show_all_sections_in_a_stable_order() {
    let layout = HomeLayout::default();
    assert_eq!(
        layout.sections,
        vec![
            HomeSectionId::Friends,
            HomeSectionId::DirectMessages,
            HomeSectionId::Servers,
            HomeSectionId::Recent,
        ]
    );
    assert!(layout.visible.friends);
    assert!(layout.visible.direct_messages);
    assert!(layout.visible.servers);
    assert!(layout.visible.recent);
    assert!(layout.recent.is_empty());
    assert_eq!(
        visible_home_sections(&layout),
        vec![
            HomeSectionId::Friends,
            HomeSectionId::DirectMessages,
            HomeSectionId::Servers,
            HomeSectionId::Recent,
        ]
    );
}

#[test]
fn home_section_labels_are_user_facing() {
    assert_eq!(home_section_label(HomeSectionId::Friends), "Friends");
    assert_eq!(home_section_label(HomeSectionId::DirectMessages), "Direct Messages");
    assert_eq!(home_section_label(HomeSectionId::Servers), "Servers");
    assert_eq!(home_section_label(HomeSectionId::Recent), "Recent");
}

#[test]
fn hiding_a_section_removes_it_from_visible_order_without_dropping_it_from_layout() {
    let mut layout = HomeLayout::default();
    set_home_section_visible(&mut layout, HomeSectionId::Servers, false);
    assert!(!layout.visible.servers);
    assert!(layout.sections.contains(&HomeSectionId::Servers));
    assert_eq!(
        visible_home_sections(&layout),
        vec![
            HomeSectionId::Friends,
            HomeSectionId::DirectMessages,
            HomeSectionId::Recent,
        ]
    );
}

#[test]
fn move_home_section_reorders_and_clamps() {
    let mut layout = HomeLayout::default();
    assert!(move_home_section(&mut layout, HomeSectionId::Recent, -1));
    assert_eq!(
        layout.sections,
        vec![
            HomeSectionId::Friends,
            HomeSectionId::DirectMessages,
            HomeSectionId::Recent,
            HomeSectionId::Servers,
        ]
    );
    assert!(!move_home_section(&mut layout, HomeSectionId::Friends, -1));
    assert!(move_home_section(&mut layout, HomeSectionId::Friends, 1));
    assert_eq!(layout.sections[0], HomeSectionId::DirectMessages);
    assert_eq!(layout.sections[1], HomeSectionId::Friends);
    assert!(!move_home_section(&mut layout, HomeSectionId::Servers, 1));
}

#[test]
fn push_home_recent_dedupes_and_caps() {
    let mut layout = HomeLayout::default();
    for id in 1..=(HOME_RECENT_LIMIT as i64 + 3) {
        push_home_recent(
            &mut layout,
            HomeRecentEntry {
                kind: HomeRecentKind::DirectMessage,
                id,
            },
        );
    }
    assert_eq!(layout.recent.len(), HOME_RECENT_LIMIT);
    assert_eq!(layout.recent[0].id, HOME_RECENT_LIMIT as i64 + 3);
    push_home_recent(
        &mut layout,
        HomeRecentEntry {
            kind: HomeRecentKind::DirectMessage,
            id: HOME_RECENT_LIMIT as i64 + 3,
        },
    );
    assert_eq!(layout.recent.len(), HOME_RECENT_LIMIT);
    assert_eq!(layout.recent[0].id, HOME_RECENT_LIMIT as i64 + 3);
    push_home_recent(
        &mut layout,
        HomeRecentEntry {
            kind: HomeRecentKind::Server,
            id: 99,
        },
    );
    assert_eq!(layout.recent[0].kind, HomeRecentKind::Server);
    assert_eq!(layout.recent[0].id, 99);
}

#[test]
fn home_json_round_trips_and_tolerates_partial_files() {
    let path = temp_home_json();
    let mut layout = HomeLayout::default();
    set_home_section_visible(&mut layout, HomeSectionId::Friends, false);
    move_home_section(&mut layout, HomeSectionId::Recent, -3);
    push_home_recent(
        &mut layout,
        HomeRecentEntry {
            kind: HomeRecentKind::Server,
            id: 7,
        },
    );
    save_json(&path, &layout).expect("home.json writes");
    let loaded: HomeLayout = load_json(&path).expect("home.json loads");
    assert_eq!(loaded, layout);

    let partial = r#"{ "visible": { "friends": false } }"#;
    std::fs::write(&path, partial).expect("partial write");
    let older: HomeLayout = load_json(&path).expect("an older or partial home.json still loads");
    assert!(!older.visible.friends);
    assert!(older.visible.direct_messages);
    assert_eq!(older.sections.len(), 4);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn visible_home_sections_skips_unknown_ids_and_fills_missing_defaults() {
    let mut layout = HomeLayout {
        sections: vec![HomeSectionId::DirectMessages],
        visible: Default::default(),
        recent: Vec::new(),
    };
    // Incomplete order still surfaces every known section once defaults are normalized.
    layout = HomeLayout::normalize(layout);
    assert_eq!(
        layout.sections,
        vec![
            HomeSectionId::DirectMessages,
            HomeSectionId::Friends,
            HomeSectionId::Servers,
            HomeSectionId::Recent,
        ]
    );
}
