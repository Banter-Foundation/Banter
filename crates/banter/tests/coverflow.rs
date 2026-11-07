//! Cover-flow stream switcher layout: arched portrait previews, focus scroll, cover UV.

use banter::{
    coverflow_cover_uv, coverflow_layout, coverflow_nearest, coverflow_nudge_focus,
    coverflow_scale, coverflow_step_focus, COVERFLOW_ASPECT, COVERFLOW_CENTER_H_FRAC,
    COVERFLOW_FOCUS_BORDER, COVERFLOW_GAP_FRAC, COVERFLOW_MIN_SCALE, COVERFLOW_PITCH_FRAC,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

#[test]
fn coverflow_tokens_are_tall_portrait_panels() {
    assert!(COVERFLOW_ASPECT >= 2.4 && COVERFLOW_ASPECT <= 3.2);
    assert!(COVERFLOW_CENTER_H_FRAC > 0.7 && COVERFLOW_CENTER_H_FRAC <= 1.0);
    assert!(COVERFLOW_MIN_SCALE > 0.35 && COVERFLOW_MIN_SCALE < 0.65);
    assert!(COVERFLOW_GAP_FRAC >= 0.0 && COVERFLOW_GAP_FRAC < 0.2);
    assert!(COVERFLOW_PITCH_FRAC > 0.4 && COVERFLOW_PITCH_FRAC < 1.0);
    assert!(COVERFLOW_FOCUS_BORDER >= 1.0 && COVERFLOW_FOCUS_BORDER <= 3.0);
}

#[test]
fn scale_peaks_at_focus_and_falls_toward_the_edges() {
    assert!(close(coverflow_scale(0.0), 1.0));
    let near = coverflow_scale(1.0);
    let mid = coverflow_scale(3.0);
    let far = coverflow_scale(8.0);
    assert!(near < 1.0 && near > mid);
    assert!(mid > far);
    assert!(far >= COVERFLOW_MIN_SCALE - 0.02);
    assert!(close(coverflow_scale(-2.5), coverflow_scale(2.5)));
}

#[test]
fn layout_arches_taller_panels_in_the_center() {
    let origin = [0.0, 100.0];
    let area = [1200.0, 400.0];
    let panels = coverflow_layout(11, 5.0, origin, area);
    assert_eq!(panels.len(), 11);
    let center = panels.iter().find(|p| p.index == 5).expect("focus panel");
    let left = panels.iter().find(|p| p.index == 0).expect("left edge");
    let right = panels.iter().find(|p| p.index == 10).expect("right edge");
    let center_h = center.max[1] - center.min[1];
    let left_h = left.max[1] - left.min[1];
    let right_h = right.max[1] - right.min[1];
    assert!(center_h > left_h && center_h > right_h);
    assert!(close(center_h / (center.max[0] - center.min[0]), COVERFLOW_ASPECT));
    // Vertically centered arch: mid-Y of every panel lines up with the area mid.
    let mid_y = origin[1] + area[1] * 0.5;
    assert!(close((center.min[1] + center.max[1]) * 0.5, mid_y));
    assert!(close((left.min[1] + left.max[1]) * 0.5, mid_y));
    assert!(center.focused);
    assert!(!left.focused);
    // Center draws above the edges.
    assert!(center.depth > left.depth && center.depth > right.depth);
    // Focused panel is horizontally centered in the area.
    let mid_x = origin[0] + area[0] * 0.5;
    assert!(close((center.min[0] + center.max[0]) * 0.5, mid_x));
}

#[test]
fn fractional_focus_shifts_the_arch_smoothly() {
    let area = [1000.0, 360.0];
    let a = coverflow_layout(7, 3.0, [0.0, 0.0], area);
    let b = coverflow_layout(7, 3.4, [0.0, 0.0], area);
    let a3 = a.iter().find(|p| p.index == 3).unwrap();
    let b3 = b.iter().find(|p| p.index == 3).unwrap();
    let a3_cx = (a3.min[0] + a3.max[0]) * 0.5;
    let b3_cx = (b3.min[0] + b3.max[0]) * 0.5;
    assert!(b3_cx < a3_cx);
    assert!((a3.max[1] - a3.min[1]) > (b3.max[1] - b3.min[1]));
}

#[test]
fn nearest_and_nudge_keep_focus_in_range() {
    assert_eq!(coverflow_nearest(2.4, 5), Some(2));
    assert_eq!(coverflow_nearest(2.6, 5), Some(3));
    assert_eq!(coverflow_nearest(-1.0, 5), Some(0));
    assert_eq!(coverflow_nearest(99.0, 5), Some(4));
    assert_eq!(coverflow_nearest(0.0, 0), None);
    assert!(close(coverflow_nudge_focus(2.0, 1.0, 5), 3.0));
    assert!(close(coverflow_nudge_focus(0.0, -1.0, 5), 0.0));
    assert!(close(coverflow_nudge_focus(4.0, 1.0, 5), 4.0));
}

#[test]
fn focus_lerps_toward_the_target() {
    let stepped = coverflow_step_focus(0.0, 3.0, 1.0 / 60.0);
    assert!(stepped > 0.0 && stepped < 3.0);
    let settled = coverflow_step_focus(2.99, 3.0, 1.0);
    assert!(close(settled, 3.0));
}

#[test]
fn cover_uv_crops_landscape_into_portrait() {
    let (uv0, uv1) = coverflow_cover_uv([1920.0, 1080.0], 100.0, 275.0);
    assert!(uv0[0] > 0.0 && uv1[0] < 1.0);
    assert!(close(uv0[1], 0.0));
    assert!(close(uv1[1], 1.0));
    let (full0, full1) = coverflow_cover_uv([100.0, 275.0], 100.0, 275.0);
    assert!(close(full0[0], 0.0) && close(full0[1], 0.0));
    assert!(close(full1[0], 1.0) && close(full1[1], 1.0));
}
