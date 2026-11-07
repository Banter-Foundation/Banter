//! Custom Mac-style title bar geometry and chrome actions (BA-105).

use banter::theme;
use banter::titlebar::{
    caption_height, client_caption_enabled, traffic_light_centers, traffic_light_hit,
    title_bar_drag_rect, title_bar_rect, DoubleClickTracker, TitleBarAction, TrafficLight,
    TITLE_BAR_DOUBLE_CLICK_MS, TITLE_BAR_DOUBLE_CLICK_SLOP,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn linux_keeps_the_system_title_bar() {
    // Linux uses DE / compositor chrome; Windows (and friends) keep the client caption.
    #[cfg(target_os = "linux")]
    {
        assert!(!client_caption_enabled());
        assert!(close(caption_height(), 0.0));
        assert!(close(title_bar_rect(1280.0)[3], 0.0));
    }
    #[cfg(not(target_os = "linux"))]
    {
        assert!(client_caption_enabled());
        assert!(close(caption_height(), theme::TITLE_BAR_H));
    }
}

#[test]
fn title_bar_is_a_short_full_width_strip() {
    let rect = title_bar_rect(1280.0);
    assert!(close(rect[0], 0.0));
    assert!(close(rect[1], 0.0));
    assert!(close(rect[2], 1280.0));
    assert!(close(rect[3], caption_height()));
    assert!(theme::TITLE_BAR_H > 0.0);
    assert!(theme::TITLE_BAR_H < theme::HEADER_H);
}

#[test]
fn traffic_lights_sit_on_the_left_in_mac_order() {
    let [close_c, min_c, max_c] = traffic_light_centers(theme::TITLE_BAR_H);
    assert_eq!(TrafficLight::ALL, [
        TrafficLight::Close,
        TrafficLight::Minimize,
        TrafficLight::Maximize,
    ]);
    assert!(close_c[0] < min_c[0]);
    assert!(min_c[0] < max_c[0]);
    assert!(close_c[0] > 0.0);
    for center in [close_c, min_c, max_c] {
        assert!(close(center[1], theme::TITLE_BAR_H * 0.5));
    }
}

#[test]
fn traffic_light_hit_matches_each_button() {
    let [close_c, min_c, max_c] = traffic_light_centers(theme::TITLE_BAR_H);
    assert_eq!(
        traffic_light_hit([close_c[0], close_c[1]], theme::TITLE_BAR_H),
        Some(TrafficLight::Close)
    );
    assert_eq!(
        traffic_light_hit([min_c[0], min_c[1]], theme::TITLE_BAR_H),
        Some(TrafficLight::Minimize)
    );
    assert_eq!(
        traffic_light_hit([max_c[0], max_c[1]], theme::TITLE_BAR_H),
        Some(TrafficLight::Maximize)
    );
    assert_eq!(
        traffic_light_hit([400.0, theme::TITLE_BAR_H * 0.5], theme::TITLE_BAR_H),
        None
    );
    assert_eq!(
        traffic_light_hit([close_c[0], theme::TITLE_BAR_H + 4.0], theme::TITLE_BAR_H),
        None
    );
}

#[test]
fn traffic_light_actions_match_mac_controls() {
    assert_eq!(TrafficLight::Close.action(), TitleBarAction::Close);
    assert_eq!(TrafficLight::Minimize.action(), TitleBarAction::Minimize);
    assert_eq!(
        TrafficLight::Maximize.action(),
        TitleBarAction::ToggleMaximize
    );
}

#[test]
fn drag_region_excludes_the_traffic_lights() {
    let drag = title_bar_drag_rect(960.0, theme::TITLE_BAR_H);
    let [_, _, max_c] = traffic_light_centers(theme::TITLE_BAR_H);
    let lights_right = max_c[0] + theme::TITLE_BTN_R + theme::TITLE_BTN_HIT_PAD;
    assert!(drag[0] >= lights_right);
    assert!(close(drag[1], 0.0));
    assert!(close(drag[2], 960.0 - drag[0]));
    assert!(close(drag[3], theme::TITLE_BAR_H));
}

#[test]
fn window_move_keeps_the_grab_point_under_the_cursor() {
    // Window at (100, 200), cursor 40px into the caption → grab offset (40, 10).
    let (x, y) = banter::titlebar::dragged_window_pos(100.0, 200.0, 40.0, 10.0, 40.0, 10.0);
    assert!(close(x, 100.0));
    assert!(close(y, 200.0));
    // Cursor moved +30, +5 in window space before the window follows.
    let (x, y) = banter::titlebar::dragged_window_pos(100.0, 200.0, 70.0, 15.0, 40.0, 10.0);
    assert!(close(x, 130.0));
    assert!(close(y, 205.0));
}

#[test]
fn double_click_tracker_fires_within_slop_and_delay() {
    let mut tracker = DoubleClickTracker::default();
    assert!(!tracker.note_click(10.0, 8.0, 0));
    assert!(tracker.note_click(
        10.0 + TITLE_BAR_DOUBLE_CLICK_SLOP * 0.5,
        8.0,
        TITLE_BAR_DOUBLE_CLICK_MS / 2
    ));
    // Consumed; next click starts a new pair.
    assert!(!tracker.note_click(10.0, 8.0, TITLE_BAR_DOUBLE_CLICK_MS));
    assert!(!tracker.note_click(
        10.0 + TITLE_BAR_DOUBLE_CLICK_SLOP + 1.0,
        8.0,
        TITLE_BAR_DOUBLE_CLICK_MS + 10
    ));
}

#[test]
fn shell_content_sits_below_the_title_bar() {
    let columns = banter::shell_columns(1280.0, true);
    let top = caption_height();
    let rail = columns.rect(banter::Region::Rail, 720.0);
    assert!(close(rail[1], top));
    assert!(close(rail[3], 720.0 - top));
    let header = columns.rect(banter::Region::Header, 720.0);
    assert!(close(header[1], top));
    let title = columns.rect(banter::Region::TitleBar, 720.0);
    assert_eq!(title, title_bar_rect(columns.window_w()));
}
