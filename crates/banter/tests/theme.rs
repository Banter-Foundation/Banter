//! Shared Discord-skin metrics. Every screen reads its sizes and colors from `banter::theme`.

use banter::{discord_shell_columns, rasterize_lucide};
use banter::theme::{
    self, TooltipSide, call_ring_avatar_center, centered_row, chat_should_stick, chat_top_pad,
    dm_call_participant_tiles, dm_call_takes_stage, hex, rail_corner_radius, rail_pill_height, server_acronym,
    status_dot, tooltip_rect, voice_tile_grid,
};

#[test]
fn a_short_history_is_pushed_to_the_bottom_of_the_pane() {
    assert!(close(chat_top_pad(120.0, 600.0), 480.0));
    assert!(close(chat_top_pad(600.0, 600.0), 0.0));
    assert!(close(chat_top_pad(900.0, 600.0), 0.0));
    assert!(close(chat_top_pad(0.0, 0.0), 0.0));
}

#[test]
fn chat_follows_new_messages_only_from_the_bottom_or_after_sending() {
    // At the bottom (within one line), new messages keep the view pinned.
    assert!(chat_should_stick(980.0, 1000.0, false));
    assert!(chat_should_stick(0.0, 0.0, false));
    // Scrolled up reading history: a received message does not yank the view.
    assert!(!chat_should_stick(200.0, 1000.0, false));
    // Sending always jumps to the newest message, like Discord.
    assert!(chat_should_stick(200.0, 1000.0, true));
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn hex_colors_are_normalized_rgba() {
    let color = hex(0x313338);
    assert!(close(color[0], 49.0 / 255.0));
    assert!(close(color[1], 51.0 / 255.0));
    assert!(close(color[2], 56.0 / 255.0));
    assert_eq!(color[3], 1.0);
    assert_eq!(hex(0xFFFFFF), [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn theme_columns_match_the_shell_layout() {
    let columns = discord_shell_columns(1280.0, true);
    assert_eq!(theme::RAIL_W, columns.rail_w);
    assert_eq!(theme::SIDEBAR_W, columns.sidebar_w);
    assert_eq!(theme::MEMBERS_W, columns.members_w);
    assert_eq!(theme::HEADER_H, 48.0);
    assert_eq!(theme::USER_PANEL_H, 52.0);
    assert_eq!(theme::CHANNEL_ROW_H, 32.0);
    assert_eq!(theme::MEMBER_ROW_H, 42.0);
    assert_eq!(theme::RAIL_ICON, 48.0);
}

#[test]
fn rail_pill_is_tall_when_selected_and_short_on_hover() {
    assert_eq!(rail_pill_height(true, false), 40.0);
    assert_eq!(rail_pill_height(true, true), 40.0);
    assert_eq!(rail_pill_height(false, true), 20.0);
    assert_eq!(rail_pill_height(false, false), 0.0);
}

#[test]
fn rail_icon_is_a_circle_until_hovered_or_selected() {
    assert_eq!(rail_corner_radius(false), theme::RAIL_ICON * 0.5);
    assert_eq!(rail_corner_radius(true), 16.0);
}

#[test]
fn server_acronym_takes_the_first_letter_of_each_word() {
    assert_eq!(server_acronym("Cool Test Server"), "CTS");
    assert_eq!(server_acronym("Owl's Place"), "OP");
    assert_eq!(server_acronym("banter"), "b");
    assert_eq!(server_acronym("  spaced   out  "), "so");
    assert_eq!(server_acronym(""), "");
}

#[test]
fn one_voice_tile_fills_the_stage_at_sixteen_by_nine() {
    let grid = voice_tile_grid(1, 1000.0, 600.0, 8.0);
    assert_eq!((grid.cols, grid.rows), (1, 1));
    assert!(close(grid.tile_w / grid.tile_h, 16.0 / 9.0));
    assert!(grid.tile_w <= 1000.0 && grid.tile_h <= 600.0);
    assert!(close(grid.tile_w, 1000.0) || close(grid.tile_h, 600.0));
}

#[test]
fn four_voice_tiles_make_a_two_by_two_grid_that_fits() {
    let grid = voice_tile_grid(4, 1000.0, 600.0, 8.0);
    assert_eq!((grid.cols, grid.rows), (2, 2));
    assert!(close(grid.tile_w / grid.tile_h, 16.0 / 9.0));
    assert!(grid.cols as f32 * grid.tile_w + (grid.cols - 1) as f32 * 8.0 <= 1000.0 + 0.01);
    assert!(grid.rows as f32 * grid.tile_h + (grid.rows - 1) as f32 * 8.0 <= 600.0 + 0.01);
}

#[test]
fn a_wide_stage_puts_three_tiles_in_one_row() {
    let grid = voice_tile_grid(3, 1600.0, 300.0, 8.0);
    assert_eq!((grid.cols, grid.rows), (3, 1));
}

#[test]
fn no_voice_tiles_for_an_empty_room() {
    let grid = voice_tile_grid(0, 1000.0, 600.0, 8.0);
    assert_eq!((grid.cols, grid.rows), (0, 0));
    assert_eq!((grid.tile_w, grid.tile_h), (0.0, 0.0));
}

#[test]
fn call_controls_are_centered_with_a_fixed_gap() {
    let xs = centered_row(400.0, &[56.0, 56.0, 56.0], 16.0);
    assert_eq!(xs, vec![100.0, 172.0, 244.0]);
    assert!(centered_row(400.0, &[], 16.0).is_empty());
}

#[test]
fn a_dm_call_takes_the_stage_only_for_that_conversation() {
    assert!(dm_call_takes_stage(Some(7), Some(7)));
    assert!(!dm_call_takes_stage(Some(7), Some(8)));
    assert!(!dm_call_takes_stage(Some(7), None));
    assert!(!dm_call_takes_stage(None, Some(7)));
    assert!(!dm_call_takes_stage(None, None));
}

#[test]
fn an_active_one_on_one_call_draws_two_participant_tiles() {
    assert_eq!(dm_call_participant_tiles(true), 2);
    assert_eq!(dm_call_participant_tiles(false), 0);
    let grid = voice_tile_grid(dm_call_participant_tiles(true), 1000.0, 500.0, theme::TILE_GAP);
    assert_eq!((grid.cols, grid.rows), (2, 1));
    assert!(close(grid.tile_w / grid.tile_h, 16.0 / 9.0));
}

#[test]
fn ringing_call_avatar_sits_above_the_control_bar() {
    assert_eq!(theme::CALL_RING_AVATAR, 120.0);
    let center = call_ring_avatar_center([100.0, 50.0], 800.0, 600.0, theme::CALL_BAR_H);
    assert!(close(center[0], 500.0));
    // 42% of the area above the 96 px bar: 50 + 0.42 * (600 - 96) = 261.68
    assert!(close(center[1], 50.0 + 0.42 * (600.0 - theme::CALL_BAR_H)));
    let tight = call_ring_avatar_center([0.0, 0.0], 400.0, theme::CALL_BAR_H, theme::CALL_BAR_H);
    assert!(close(tight[0], 200.0));
    assert!(close(tight[1], 0.0));
}

#[test]
fn status_dot_sits_bottom_right_with_a_cutout_ring() {
    let dot = status_dot([16.0, 16.0], 16.0);
    assert!(close(dot.center[0], 27.0));
    assert!(close(dot.center[1], 27.0));
    assert!(close(dot.radius, 5.0));
    assert!(close(dot.cutout, 8.0));
}

#[test]
fn rail_tooltip_opens_to_the_right_and_is_vertically_centered() {
    let (min, max) = tooltip_rect([12.0, 100.0], [60.0, 148.0], [80.0, 20.0], TooltipSide::Right, [1280.0, 800.0]);
    assert!(close(min[0], 60.0 + theme::TOOLTIP_GAP));
    assert!(close(max[0] - min[0], 80.0 + theme::TOOLTIP_PAD_X * 2.0));
    assert!(close(max[1] - min[1], 20.0 + theme::TOOLTIP_PAD_Y * 2.0));
    assert!(close((min[1] + max[1]) * 0.5, 124.0));
}

#[test]
fn every_skin_icon_is_vendored_and_has_ink() {
    for name in [
        "check", "chevron-down", "chevron-right", "chevron-up", "circle-plus", "crown", "folder-plus", "hash",
        "headphone-off", "headphones", "message-circle", "mic", "mic-off", "monitor", "monitor-off", "pencil",
        "phone", "phone-off", "plus", "search", "settings", "signal", "user", "user-plus", "users", "video",
        "video-off", "volume-2", "x",
    ] {
        let rgba = rasterize_lucide(name, 32, [255, 255, 255]).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(rgba.chunks(4).any(|pixel| pixel[3] > 0), "{name} has no ink");
    }
}

#[test]
fn top_tooltip_is_centered_above_and_kept_on_screen() {
    let (min, max) = tooltip_rect([600.0, 8.0], [632.0, 40.0], [60.0, 18.0], TooltipSide::Top, [1280.0, 800.0]);
    assert!(close((min[0] + max[0]) * 0.5, 616.0));
    assert!(close(max[1], 8.0 - theme::TOOLTIP_GAP) || min[1] >= 0.0);
    assert!(min[1] >= 0.0);
    let (edge_min, _) = tooltip_rect([0.0, 400.0], [10.0, 420.0], [120.0, 18.0], TooltipSide::Top, [1280.0, 800.0]);
    assert!(edge_min[0] >= 0.0);
}
