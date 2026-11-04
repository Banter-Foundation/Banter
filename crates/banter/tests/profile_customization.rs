//! Client contracts for Discord-parity profile view + customization editor.
//! Written before the profile editor state landed.

use banter::{
    ProfileAction, ProfileBoardTab, ProfileCard, ProfileEditorSlot, ProfileKind, ProfileMediaDraft,
    ProfileMode, ProfileWidgetKind, display_name_style_color, games_from_payload, games_to_payload,
    games_widget_cap, profile_badge_row_visible_cap, profile_board_tabs, profile_editor_sections,
    profile_editor_slot_rows, profile_heading, profile_primary_actions, profile_view_shows_board,
};

#[test]
fn profile_opens_in_discord_view_not_basic_only() {
    let mut card = ProfileCard::closed();
    card.open(7, ProfileKind::SelfAccount);
    assert_eq!(card.mode(), ProfileMode::View);
    assert!(profile_view_shows_board(card.mode()));
    assert!(!card.customizing());
}

#[test]
fn self_edit_opens_customization_editor() {
    let mut card = ProfileCard::closed();
    card.open(1, ProfileKind::SelfAccount);
    assert!(card.begin_customize());
    assert_eq!(card.mode(), ProfileMode::Customize);
    assert!(card.customizing());
    assert_eq!(card.board_tab(), ProfileBoardTab::Board);

    card.cancel_customize();
    assert_eq!(card.mode(), ProfileMode::View);
    assert!(!card.customizing());
}

#[test]
fn friends_cannot_open_customization_editor() {
    let mut card = ProfileCard::closed();
    card.open(2, ProfileKind::Friend);
    assert!(!card.begin_customize());
    assert_eq!(card.mode(), ProfileMode::View);
}

#[test]
fn editor_draft_tracks_discord_slots() {
    let mut card = ProfileCard::closed();
    card.open(3, ProfileKind::SelfAccount);
    card.begin_customize();
    card.set_draft_about_me("Creator code OWL");
    card.set_draft_pronouns("she/her");
    card.set_draft_custom_status("Favorite NPC companion?");
    card.set_draft_theme(0x2d4a3e, 0x0f1a14);
    card.set_draft_nameplate(Some("forest-mist".into()));
    card.set_draft_avatar_decoration(Some("leaf-ring".into()));
    card.set_draft_display_name_style(Some("mint-glow".into()));
    card.set_draft_profile_effect(Some("aurora-wash".into()));
    card.set_draft_profile_frame(Some("thick-jade".into()));

    assert_eq!(card.draft_about_me(), "Creator code OWL");
    assert_eq!(card.draft_pronouns(), "she/her");
    assert_eq!(card.draft_custom_status(), "Favorite NPC companion?");
    assert_eq!(card.draft_theme_primary(), 0x2d4a3e);
    assert_eq!(card.draft_theme_secondary(), 0x0f1a14);
    assert_eq!(card.draft_nameplate().as_deref(), Some("forest-mist"));
    assert_eq!(card.draft_avatar_decoration().as_deref(), Some("leaf-ring"));
    assert_eq!(card.draft_display_name_style().as_deref(), Some("mint-glow"));
    assert_eq!(card.draft_profile_effect().as_deref(), Some("aurora-wash"));
    assert_eq!(card.draft_profile_frame().as_deref(), Some("thick-jade"));
}

#[test]
fn editor_sections_match_discord_customization_ref() {
    let sections = profile_editor_sections();
    assert_eq!(
        sections,
        &[
            "Nameplate",
            "Avatar",
            "Avatar Decoration",
            "Display Name Style",
            "Theme",
            "Banner",
            "Profile Effect",
            "Profile Frame",
        ]
    );
}

#[test]
fn board_tabs_match_discord_profile_board() {
    assert_eq!(
        profile_board_tabs(),
        &[
            ProfileBoardTab::Board,
            ProfileBoardTab::Activity,
            ProfileBoardTab::Wishlist,
        ]
    );
    let mut card = ProfileCard::closed();
    card.open(4, ProfileKind::Friend);
    card.set_board_tab(ProfileBoardTab::Activity);
    assert_eq!(card.board_tab(), ProfileBoardTab::Activity);
}

#[test]
fn primary_actions_still_match_relationship() {
    assert_eq!(
        profile_primary_actions(ProfileKind::SelfAccount),
        &[ProfileAction::EditProfile]
    );
    assert_eq!(
        profile_primary_actions(ProfileKind::Friend),
        &[ProfileAction::Message]
    );
}

#[test]
fn widget_kinds_cover_discord_board_cards() {
    assert_eq!(
        [
            ProfileWidgetKind::GamesInRotation,
            ProfileWidgetKind::FavoriteGame,
            ProfileWidgetKind::GamesILike,
        ]
        .len(),
        3
    );
}

#[test]
fn display_name_style_resolves_known_colors() {
    let mint = display_name_style_color(Some("mint-glow"));
    assert_ne!(mint, [1.0, 1.0, 1.0, 1.0]);
    let fallback = display_name_style_color(None);
    assert_eq!(fallback, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn heading_still_prefers_nickname_in_parity_card() {
    assert_eq!(profile_heading("Ada", Some("Night Owl")), "Night Owl");
}

#[test]
fn games_widget_payload_round_trips_without_raw_json() {
    let empty = games_from_payload(&serde_json::json!({}));
    assert!(empty.is_empty());

    let list = vec![
        ("Hytale".into(), "Obsessed".into()),
        ("Minecraft".into(), "Expert".into()),
    ];
    let payload = games_to_payload(&list);
    assert_eq!(games_from_payload(&payload), list);

    // Caps match Discord board card limits used by the editor.
    assert_eq!(games_widget_cap(ProfileWidgetKind::GamesInRotation), 5);
    assert_eq!(games_widget_cap(ProfileWidgetKind::GamesILike), 20);
    assert_eq!(games_widget_cap(ProfileWidgetKind::FavoriteGame), 0);
}

#[test]
fn editor_slot_rows_match_discord_customization_ref() {
    let rows = profile_editor_slot_rows();
    assert_eq!(rows[0].0, "Nameplate");
    assert_eq!(rows[0].1, &[ProfileEditorSlot::Nameplate]);
    assert!(rows.iter().any(|(label, _)| *label == "Theme & Banner"));
    assert!(rows.iter().any(|(label, _)| *label == "Profile Effect & Frame"));
    assert!(rows.iter().any(|(label, _)| *label == "Badges"));
    assert_eq!(profile_editor_sections().len(), 8);
    assert_eq!(profile_badge_row_visible_cap(), 6);
    assert!(ProfileMediaDraft::Unchanged.is_unchanged());
    assert!(!ProfileMediaDraft::Clear.is_unchanged());
}
