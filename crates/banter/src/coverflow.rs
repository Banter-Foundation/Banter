//! Cover-flow layout for stream / source switcher previews.
//!
//! Tall portrait panels sit on a horizontal arch: the focused slot is tallest and
//! centered; height falls off toward the left and right edges. Pure geometry so
//! tests can check the curve without a window.

/// Portrait height ÷ width for each preview panel.
pub const COVERFLOW_ASPECT: f32 = 2.75;
/// Center panel height as a fraction of the carousel area height.
pub const COVERFLOW_CENTER_H_FRAC: f32 = 0.92;
/// Smallest scale at the far edges of the arch (relative to the focused panel).
pub const COVERFLOW_MIN_SCALE: f32 = 0.48;
/// Gaussian falloff strength for scale vs distance-from-focus.
pub const COVERFLOW_FALLOFF: f32 = 0.045;
/// Gap between panels at full scale, as a fraction of the center panel width.
pub const COVERFLOW_GAP_FRAC: f32 = 0.05;
/// Horizontal pitch as a fraction of the center panel width (tight pack / slight overlap).
pub const COVERFLOW_PITCH_FRAC: f32 = 0.72;
/// Focused panel border thickness in pixels.
pub const COVERFLOW_FOCUS_BORDER: f32 = 1.5;
/// How fast focus eases toward its target (higher = snappier).
pub const COVERFLOW_FOCUS_LERP: f32 = 14.0;

/// One laid-out preview panel in the stream switcher cover-flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverflowPanel {
    pub index: usize,
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub scale: f32,
    /// Higher draws later (on top). The focused panel has the greatest depth.
    pub depth: i32,
    pub focused: bool,
}

/// Scale for a panel `offset` slots from focus (`0` = center). Symmetric in sign.
pub fn coverflow_scale(offset: f32) -> f32 {
    let t = (-COVERFLOW_FALLOFF * offset * offset).exp();
    COVERFLOW_MIN_SCALE + (1.0 - COVERFLOW_MIN_SCALE) * t
}

/// Lay out `count` portrait panels across `area` with continuous `focus` (slot index).
///
/// Panels are vertically centered so top and bottom edges form the arch. Horizontal
/// pitch is fixed from the center width so edge panels pack tighter as they shrink.
pub fn coverflow_layout(
    count: usize,
    focus: f32,
    origin: [f32; 2],
    area: [f32; 2],
) -> Vec<CoverflowPanel> {
    if count == 0 || area[0] <= 1.0 || area[1] <= 1.0 {
        return Vec::new();
    }
    let center_h = (area[1] * COVERFLOW_CENTER_H_FRAC).max(32.0);
    let center_w = (center_h / COVERFLOW_ASPECT).max(16.0);
    let pitch = center_w * (COVERFLOW_PITCH_FRAC + COVERFLOW_GAP_FRAC);
    let mid_x = origin[0] + area[0] * 0.5;
    let mid_y = origin[1] + area[1] * 0.5;
    let mut panels = Vec::with_capacity(count);
    for index in 0..count {
        let offset = index as f32 - focus;
        let scale = coverflow_scale(offset);
        let h = center_h * scale;
        let w = h / COVERFLOW_ASPECT;
        let cx = mid_x + offset * pitch;
        let min = [cx - w * 0.5, mid_y - h * 0.5];
        let max = [cx + w * 0.5, mid_y + h * 0.5];
        let depth = ((1.0 - offset.abs().min(32.0) / 32.0) * 10_000.0) as i32;
        panels.push(CoverflowPanel {
            index,
            min,
            max,
            scale,
            depth,
            focused: offset.abs() < 0.5,
        });
    }
    panels
}

/// Nearest source index for `focus`, or `None` when the carousel is empty.
pub fn coverflow_nearest(focus: f32, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(focus.round().clamp(0.0, (count - 1) as f32) as usize)
}

/// Apply a mouse-wheel / key nudge to the focus target, clamped to the source range.
///
/// Positive `wheel` advances toward later sources; negative goes earlier.
pub fn coverflow_nudge_focus(focus: f32, wheel: f32, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let max = (count - 1) as f32;
    (focus + wheel).clamp(0.0, max)
}

/// Ease `current` toward `target` by `dt` seconds.
pub fn coverflow_step_focus(current: f32, target: f32, dt: f32) -> f32 {
    if (current - target).abs() < 0.001 {
        return target;
    }
    let t = (COVERFLOW_FOCUS_LERP * dt.max(0.0)).clamp(0.0, 1.0);
    current + (target - current) * t
}

/// UV min/max that cover-fits `image` into a portrait `box` (crops overflow, no letterbox).
pub fn coverflow_cover_uv(image: [f32; 2], box_w: f32, box_h: f32) -> ([f32; 2], [f32; 2]) {
    if image[0] <= 1.0 || image[1] <= 1.0 || box_w <= 1.0 || box_h <= 1.0 {
        return ([0.0, 0.0], [1.0, 1.0]);
    }
    let image_aspect = image[0] / image[1];
    let box_aspect = box_w / box_h;
    if image_aspect > box_aspect {
        // Wider than the box: crop left/right.
        let visible = box_aspect / image_aspect;
        let u0 = (1.0 - visible) * 0.5;
        ([u0, 0.0], [u0 + visible, 1.0])
    } else {
        // Taller than the box: crop top/bottom.
        let visible = image_aspect / box_aspect;
        let v0 = (1.0 - visible) * 0.5;
        ([0.0, v0], [1.0, v0 + visible])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_carousel_yields_no_panels() {
        assert!(coverflow_layout(0, 0.0, [0.0, 0.0], [800.0, 400.0]).is_empty());
        assert!(coverflow_layout(3, 1.0, [0.0, 0.0], [0.0, 400.0]).is_empty());
    }

}
