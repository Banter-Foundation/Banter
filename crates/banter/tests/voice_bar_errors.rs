//! Voice-connection panel error layout. Written before the stacking fix and kept as written.
//! Device errors must not draw on the same pixel row; Discord shows a readable status, not layered text.

use banter::{device_error_text, remember_device_error, voice_bar_error_lines, voice_bar_error_ys};

#[test]
fn voice_bar_shows_plain_messages_without_device_prefixes() {
    let mut list = Vec::new();
    remember_device_error(&mut list, "camera", "Banter could not use this camera.");
    remember_device_error(&mut list, "microphone", "Banter could not open this microphone.");
    let lines = voice_bar_error_lines(&list);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], device_error_text(&list[0]));
    assert_eq!(lines[1], device_error_text(&list[1]));
    assert!(!lines[0].contains("camera:"));
}

#[test]
fn voice_bar_error_rows_do_not_share_a_y() {
    let ys = voice_bar_error_ys(100.0, 96.0, 3, 14.0);
    assert_eq!(ys.len(), 3);
    assert!(ys[0] > ys[1], "later errors stack upward");
    assert!(ys[1] > ys[2]);
    assert!((ys[0] - ys[1] - 14.0).abs() < f32::EPSILON);
    assert!((ys[1] - ys[2] - 14.0).abs() < f32::EPSILON);
}

#[test]
fn voice_bar_error_rows_sit_inside_the_panel() {
    let origin_y = 400.0;
    let panel_h = 96.0;
    let line_h = 14.0;
    let ys = voice_bar_error_ys(origin_y, panel_h, 2, line_h);
    for y in ys {
        assert!(y >= origin_y);
        assert!(y + line_h <= origin_y + panel_h + f32::EPSILON);
    }
}

#[test]
fn no_errors_means_no_voice_bar_lines() {
    assert!(voice_bar_error_lines(&[]).is_empty());
    assert!(voice_bar_error_ys(0.0, 96.0, 0, 14.0).is_empty());
}
