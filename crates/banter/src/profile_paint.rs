//! Procedural Banter-owned cosmetics / badge / board-card paint for Discord-parity profiles.
//!
//! These are not Discord assets — shapes and gradients are Banter-drawn.

use banter_server::{BadgeShape, ProfileBadge};
use dear_imgui_rs::{DrawFlags, Ui};

use crate::profile::{cosmetic_color, rgb_u32_to_rgba};
use crate::theme::{PROFILE_BADGE, PROFILE_GAME_CARD_H, PROFILE_GAME_CARD_W, PROFILE_GAME_THUMB, PROFILE_RADIUS};

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn paint_nameplate_art(ui: &Ui, min: [f32; 2], max: [f32; 2], id: Option<&str>) {
    let draw = ui.get_window_draw_list();
    let color = id.and_then(cosmetic_color).unwrap_or(0x2d4a3e);
    let rgba = rgb_u32_to_rgba(color);
    let mid = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    draw.add_rect(min, max, rgba).rounding(6.0).filled(true).build();
    let highlight = [rgba[0] * 1.25 + 0.05, rgba[1] * 1.25 + 0.05, rgba[2] * 1.25 + 0.05, 0.55];
    draw.add_rect([min[0], min[1]], [max[0], mid[1]], highlight)
        .rounding(6.0)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    // Soft “forest mist” blobs for nameplate atmosphere.
    for (dx, dy, r) in [(-0.28, -0.1, 10.0), (0.22, 0.15, 8.0), (0.0, 0.05, 12.0)] {
        let c = [mid[0] + (max[0] - min[0]) * dx, mid[1] + (max[1] - min[1]) * dy];
        draw.add_circle(c, r, [1.0, 1.0, 1.0, 0.08]).filled(true).build();
    }
}

pub fn paint_avatar_decoration_art(ui: &Ui, center: [f32; 2], radius: f32, decoration_id: Option<&str>) {
    let Some(id) = decoration_id else { return };
    let Some(color) = cosmetic_color(id) else { return };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    draw.add_circle(center, radius + 9.0, rgba).thickness(3.5).build();
    if id.contains("leaf") {
        for (angle, dist) in [
            (0.2_f32, 1.28),
            (1.1, 1.32),
            (2.0, 1.26),
            (3.2, 1.3),
            (4.1, 1.28),
            (5.0, 1.3),
        ] {
            let tip = [
                center[0] + radius * dist * angle.cos(),
                center[1] + radius * dist * angle.sin(),
            ];
            draw.add_circle(tip, 5.0, rgba).filled(true).build();
            draw.add_circle(tip, 2.2, [rgba[0] * 0.6, rgba[1] * 0.7, rgba[2] * 0.5, 1.0])
                .filled(true)
                .build();
        }
    } else if id.contains("spark") {
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let tip = [center[0] + (radius + 12.0) * a.cos(), center[1] + (radius + 12.0) * a.sin()];
            draw.add_circle(tip, 2.4, rgba).filled(true).build();
        }
    } else {
        draw.add_circle(center, radius + 14.0, [rgba[0], rgba[1], rgba[2], 0.35])
            .thickness(6.0)
            .build();
    }
}

pub fn paint_profile_frame_art(ui: &Ui, center: [f32; 2], radius: f32, frame_id: Option<&str>) {
    let Some(id) = frame_id else { return };
    let Some(color) = cosmetic_color(id) else { return };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    let thick = if id.contains("thick") { 5.5 } else { 2.2 };
    draw.add_circle(center, radius + 12.0, rgba).thickness(thick).build();
    if id.contains("gold") {
        draw.add_circle(center, radius + 16.0, [rgba[0], rgba[1], rgba[2], 0.35])
            .thickness(1.5)
            .build();
    }
}

pub fn paint_profile_effect_art(ui: &Ui, min: [f32; 2], max: [f32; 2], effect_id: Option<&str>) {
    let Some(id) = effect_id else { return };
    let Some(color) = cosmetic_color(id) else { return };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    draw.add_rect(min, max, [rgba[0], rgba[1], rgba[2], 0.10])
        .rounding(8.0)
        .filled(true)
        .build();
    if id.contains("particle") {
        let w = max[0] - min[0];
        let h = max[1] - min[1];
        for (fx, fy) in [(0.15, 0.2), (0.7, 0.35), (0.4, 0.7), (0.85, 0.55), (0.25, 0.85)] {
            draw.add_circle([min[0] + w * fx, min[1] + h * fy], 2.5, [rgba[0], rgba[1], rgba[2], 0.55])
                .filled(true)
                .build();
        }
    } else {
        // Aurora wash band across the top of the body.
        draw.add_rect(
            [min[0], min[1]],
            [max[0], min[1] + (max[1] - min[1]) * 0.28],
            [rgba[0], rgba[1], rgba[2], 0.18],
        )
        .rounding(8.0)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    }
}

pub fn paint_badge_icon(ui: &Ui, center: [f32; 2], badge: &ProfileBadge) {
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(badge.color);
    let r = PROFILE_BADGE;
    match badge.shape {
        BadgeShape::Circle => {
            draw.add_circle(center, r, rgba).filled(true).build();
        }
        BadgeShape::Hex | BadgeShape::Gem => {
            draw.add_circle(center, r, rgba).filled(true).build();
            draw.add_circle(center, r * 0.45, WHITE).filled(true).build();
        }
        BadgeShape::Shield => {
            draw.add_rect(
                [center[0] - r * 0.7, center[1] - r],
                [center[0] + r * 0.7, center[1] + r * 0.85],
                rgba,
            )
            .rounding(3.0)
            .filled(true)
            .build();
        }
        BadgeShape::Leaf => {
            draw.add_circle(center, r, rgba).filled(true).build();
            draw.add_circle([center[0] + 2.0, center[1] - 2.0], r * 0.55, [rgba[0] * 0.7, rgba[1], rgba[2] * 0.7, 1.0])
                .filled(true)
                .build();
        }
        BadgeShape::Eye => {
            draw.add_circle(center, r, rgba).filled(true).build();
            draw.add_circle(center, r * 0.45, BLACK).filled(true).build();
            draw.add_circle([center[0] + 1.5, center[1] - 1.5], r * 0.18, WHITE)
                .filled(true)
                .build();
        }
    }
}

/// Vertical game card (Games in Rotation / Favorite) using Banter color covers — not scraped art.
pub fn paint_game_card(ui: &Ui, min: [f32; 2], title: &str, tag: &str) {
    let draw = ui.get_window_draw_list();
    let max = [min[0] + PROFILE_GAME_CARD_W, min[1] + PROFILE_GAME_CARD_H];
    let cover = game_cover_color(title);
    draw.add_rect(min, max, [0.08, 0.09, 0.10, 1.0])
        .rounding(PROFILE_RADIUS)
        .filled(true)
        .build();
    draw.add_rect(min, [max[0], min[1] + PROFILE_GAME_CARD_H * 0.62], cover)
        .rounding(PROFILE_RADIUS)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    let initials = game_initials(title);
    let size = ui.calc_text_size(&initials);
    draw.add_text(
        [
            min[0] + (PROFILE_GAME_CARD_W - size[0]) * 0.5,
            min[1] + PROFILE_GAME_CARD_H * 0.28 - size[1] * 0.5,
        ],
        WHITE,
        &initials,
    );
    let label = truncate(title, 10);
    draw.add_text([min[0] + 6.0, min[1] + PROFILE_GAME_CARD_H * 0.66], WHITE, &label);
    if !tag.is_empty() {
        let tag_min = [min[0] + 6.0, max[1] - 22.0];
        let tag_size = ui.calc_text_size(tag);
        let tag_max = [tag_min[0] + tag_size[0] + 10.0, tag_min[1] + tag_size[1] + 4.0];
        draw.add_rect(tag_min, tag_max, cover).rounding(4.0).filled(true).build();
        draw.add_text([tag_min[0] + 5.0, tag_min[1] + 2.0], WHITE, tag);
    }
}

pub fn paint_game_thumb(ui: &Ui, min: [f32; 2], title: &str) {
    let draw = ui.get_window_draw_list();
    let max = [min[0] + PROFILE_GAME_THUMB, min[1] + PROFILE_GAME_THUMB];
    let cover = game_cover_color(title);
    draw.add_rect(min, max, cover).rounding(6.0).filled(true).build();
    let initials = game_initials(title);
    let size = ui.calc_text_size(&initials);
    draw.add_text(
        [
            min[0] + (PROFILE_GAME_THUMB - size[0]) * 0.5,
            min[1] + (PROFILE_GAME_THUMB - size[1]) * 0.5,
        ],
        WHITE,
        &initials,
    );
}

fn game_cover_color(title: &str) -> [f32; 4] {
    let mut hash = 0u32;
    for b in title.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as u32);
    }
    let r = 0.25 + ((hash >> 16) & 0xff) as f32 / 255.0 * 0.55;
    let g = 0.25 + ((hash >> 8) & 0xff) as f32 / 255.0 * 0.55;
    let b = 0.25 + (hash & 0xff) as f32 / 255.0 * 0.55;
    [r, g, b, 1.0]
}

fn game_initials(title: &str) -> String {
    let mut out = String::new();
    for part in title.split_whitespace().take(2) {
        if let Some(ch) = part.chars().next() {
            out.push(ch.to_ascii_uppercase());
        }
    }
    if out.is_empty() {
        "?".into()
    } else {
        out
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}
