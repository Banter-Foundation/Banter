//! Custom Mac-style title bar geometry and chrome actions (BA-105).
//! Window chrome repair: client edge-resize + initial placement (BA-111).

use banter::theme;
use banter::titlebar::{
    caption_height, centered_window_pos, client_caption_enabled, content_below_caption,
    dragged_window_pos, resize_edge_at, resized_window_frame, title_bar_drag_rect, title_bar_rect,
    traffic_light_centers, traffic_light_hit, DoubleClickTracker, ResizeEdge, TitleBarAction,
    TrafficLight, RESIZE_BORDER_PX, TITLE_BAR_DOUBLE_CLICK_MS, TITLE_BAR_DOUBLE_CLICK_SLOP,
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
fn glfw_decorated_matches_client_caption_policy() {
    // Decorated must be the inverse of the client caption: Linux gets OS chrome;
    // Windows stays undecorated so traffic lights can replace the native bar.
    assert_eq!(
        banter::titlebar::glfw_decorated(),
        !client_caption_enabled()
    );
}

#[test]
fn glfw_windows_stay_user_resizable() {
    // Always request GLFW_RESIZABLE. On decorated hosts the OS edge works; on
    // undecorated hosts GLFW ignores the hint and Banter owns edge hits instead.
    assert!(banter::titlebar::glfw_resizable());
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
fn drag_region_excludes_the_traffic_lights_and_top_resize_border() {
    let drag = title_bar_drag_rect(960.0, theme::TITLE_BAR_H);
    let [_, _, max_c] = traffic_light_centers(theme::TITLE_BAR_H);
    let lights_right = max_c[0] + theme::TITLE_BTN_R + theme::TITLE_BTN_HIT_PAD;
    assert!(drag[0] >= lights_right);
    // Top strip is reserved for client edge-resize on undecorated hosts.
    assert!(close(drag[1], RESIZE_BORDER_PX));
    assert!(close(drag[2], 960.0 - drag[0]));
    assert!(close(drag[3], theme::TITLE_BAR_H - RESIZE_BORDER_PX));
    assert!(RESIZE_BORDER_PX > 0.0);
    assert!(RESIZE_BORDER_PX < theme::TITLE_BAR_H);
}

#[test]
fn window_move_keeps_the_grab_point_under_the_cursor() {
    // Window at (100, 200), cursor 40px into the caption → grab offset (40, 10).
    let (x, y) = dragged_window_pos(100.0, 200.0, 40.0, 10.0, 40.0, 10.0);
    assert!(close(x, 100.0));
    assert!(close(y, 200.0));
    // Cursor moved +30, +5 in window space before the window follows.
    let (x, y) = dragged_window_pos(100.0, 200.0, 70.0, 15.0, 40.0, 10.0);
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

#[test]
fn imgui_shell_host_does_not_cover_the_caption_strip() {
    // Full-bleed shell at [0,0] stole caption hits and broke drag. Host rect starts
    // at caption_height() so traffic lights / drag keep the strip.
    let host = content_below_caption(1280.0, 800.0);
    assert!(close(host[0], 0.0));
    assert!(close(host[1], caption_height()));
    assert!(close(host[2], 1280.0));
    assert!(close(host[3], 800.0 - caption_height()));
}

#[test]
fn resize_edge_at_hits_sides_and_corners() {
    let w = 400.0;
    let h = 300.0;
    let b = RESIZE_BORDER_PX;
    assert_eq!(resize_edge_at([b * 0.5, h * 0.5], w, h, b), Some(ResizeEdge::Left));
    assert_eq!(
        resize_edge_at([w - b * 0.5, h * 0.5], w, h, b),
        Some(ResizeEdge::Right)
    );
    assert_eq!(resize_edge_at([w * 0.5, b * 0.5], w, h, b), Some(ResizeEdge::Top));
    assert_eq!(
        resize_edge_at([w * 0.5, h - b * 0.5], w, h, b),
        Some(ResizeEdge::Bottom)
    );
    assert_eq!(
        resize_edge_at([b * 0.5, b * 0.5], w, h, b),
        Some(ResizeEdge::TopLeft)
    );
    assert_eq!(
        resize_edge_at([w - b * 0.5, b * 0.5], w, h, b),
        Some(ResizeEdge::TopRight)
    );
    assert_eq!(
        resize_edge_at([b * 0.5, h - b * 0.5], w, h, b),
        Some(ResizeEdge::BottomLeft)
    );
    assert_eq!(
        resize_edge_at([w - b * 0.5, h - b * 0.5], w, h, b),
        Some(ResizeEdge::BottomRight)
    );
    assert_eq!(resize_edge_at([w * 0.5, h * 0.5], w, h, b), None);
}

#[test]
fn resized_window_frame_moves_origin_on_left_and_top() {
    // From a frame already above mins: right +20 grows width only.
    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Right, 100, 80, 800, 600, 20.0, 0.0, 640, 480);
    assert_eq!((x, y, w, h), (100, 80, 820, 600));

    // Left −30 (cursor moved left): origin shifts left, width grows.
    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Left, 100, 80, 800, 600, -30.0, 0.0, 640, 480);
    assert_eq!((x, y, w, h), (70, 80, 830, 600));

    // Top −10: origin shifts up, height grows.
    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Top, 100, 80, 800, 600, 0.0, -10.0, 640, 480);
    assert_eq!((x, y, w, h), (100, 70, 800, 610));

    // Bottom-right corner +15,+20.
    let (x, y, w, h) = resized_window_frame(
        ResizeEdge::BottomRight,
        100,
        80,
        800,
        600,
        15.0,
        20.0,
        640,
        480,
    );
    assert_eq!((x, y, w, h), (100, 80, 815, 620));
}

#[test]
fn resized_window_frame_respects_minimum_size() {
    // Shrinking below the floor clamps; left/top origins stay put when clamp blocks.
    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Right, 50, 40, 800, 600, -500.0, 0.0, 640, 480);
    assert_eq!((x, y, w, h), (50, 40, 640, 600));

    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Bottom, 50, 40, 800, 600, 0.0, -400.0, 640, 480);
    assert_eq!((x, y, w, h), (50, 40, 800, 480));

    // Left edge pull inward that would go below min width: keep min width, pin right edge.
    let (x, y, w, h) =
        resized_window_frame(ResizeEdge::Left, 50, 40, 800, 600, 300.0, 0.0, 640, 480);
    assert_eq!(w, 640);
    assert_eq!(x + w, 50 + 800);
    assert_eq!((y, h), (40, 600));
}

#[test]
fn centered_window_pos_places_frame_on_monitor() {
    let (x, y) = centered_window_pos(0, 0, 1920, 1080, 1280, 800);
    assert_eq!((x, y), (320, 140));
    let (x, y) = centered_window_pos(100, 50, 1600, 900, 800, 600);
    assert_eq!((x, y), (500, 200));
}
