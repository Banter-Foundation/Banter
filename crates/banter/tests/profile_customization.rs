//! Client contracts for Discord-parity profile view + customization editor.
//! Written before the profile editor state landed.

use banter::{
    ProfileAction, ProfileBoardTab, ProfileCard, ProfileEditScope, ProfileEditorSlot, ProfileKind,
    ProfileMediaDraft, ProfileMode, ProfileWidgetKind, display_name_style_color, games_from_payload,
    games_to_payload, games_widget_cap, profile_badge_row_visible_cap, profile_board_tabs,
    profile_column_content_width, profile_editor_layout, profile_editor_sections,
    profile_editor_slot_rows, profile_heading, profile_primary_actions, profile_view_layout,
    profile_view_shows_board,
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

#[test]
fn editor_can_switch_main_and_server_profile_scope() {
    let mut card = ProfileCard::closed();
    card.open(1, ProfileKind::SelfAccount);
    assert!(card.begin_customize());
    assert!(card.edit_scope().is_main());
    card.set_edit_scope(ProfileEditScope::Server {
        id: 9,
        name: "Owl Nest".into(),
    });
    assert_eq!(card.edit_scope().server_id(), Some(9));
    assert_eq!(card.edit_scope().label(), "Owl Nest");
    card.set_draft_nickname("Night Owl");
    assert_eq!(card.draft_nickname(), "Night Owl");
    card.set_edit_scope(ProfileEditScope::Main);
    assert!(card.edit_scope().is_main());
}

#[test]
fn profile_view_layout_keeps_identity_and_board_disjoint_on_screen() {
    let layout = profile_view_layout([1280.0, 720.0], [200.0, 100.0]);
    assert!(layout.identity.disjoint(layout.board));
    assert!(layout.card.contains(layout.identity.min));
    assert!(layout.card.contains([
        layout.identity.max[0] - 1.0,
        layout.identity.max[1] - 1.0
    ]));
    assert!(layout.card.contains(layout.board.min));
    assert!(layout.card.contains([layout.board.max[0] - 1.0, layout.board.max[1] - 1.0]));
    assert!(layout.card.max[0] <= 1280.0);
    assert!(layout.card.max[1] <= 720.0);
    assert!(layout.board.width() > 0.0);
    assert!(layout.board.height() > 0.0);
}

#[test]
fn profile_view_layout_flips_when_anchor_is_near_right_edge() {
    let layout = profile_view_layout([1280.0, 720.0], [1200.0, 80.0]);
    assert!(layout.card.max[0] <= 1280.0 - 8.0 + 0.1);
    assert!(layout.card.min[0] < 1200.0);
}

#[test]
fn profile_editor_layout_has_three_columns_footer_and_save_cancel() {
    let layout = profile_editor_layout([1440.0, 900.0]);
    assert!(layout.left.disjoint(layout.preview));
    assert!(layout.preview.disjoint(layout.board));
    assert!(layout.left.disjoint(layout.board));
    assert!(layout.footer.min[1] >= layout.left.max[1] - 0.1);
    assert!(layout.footer.contains([
        (layout.save.min[0] + layout.save.max[0]) * 0.5,
        (layout.save.min[1] + layout.save.max[1]) * 0.5
    ]));
    assert!(layout.footer.contains([
        (layout.cancel.min[0] + layout.cancel.max[0]) * 0.5,
        (layout.cancel.min[1] + layout.cancel.max[1]) * 0.5
    ]));
    assert!(layout.save.disjoint(layout.cancel));
    assert!(layout.frame.contains(layout.close.min));
    assert!(layout.preview.width() >= 280.0);
    assert!(layout.board.width() > 100.0);
    assert_eq!(profile_column_content_width(320.0), 304.0);
}

#[test]
fn profile_editor_layout_still_fits_narrow_displays() {
    let layout = profile_editor_layout([1024.0, 700.0]);
    assert!(layout.frame.max[0] <= 1024.0);
    assert!(layout.frame.max[1] <= 700.0);
    assert!(layout.left.width() > 0.0);
    assert!(layout.preview.width() > 0.0);
    assert!(layout.board.width() > 0.0);
    assert!(layout.left.disjoint(layout.preview));
    assert!(layout.preview.disjoint(layout.board));
}
