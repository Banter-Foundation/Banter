//! Procedural Banter-owned cosmetics / badge / board-card paint for Discord-parity profiles.
//!
//! These are not Discord assets — shapes and gradients are Banter-drawn.

use banter_server::{BadgeShape, ProfileBadge};
use dear_imgui_rs::{DrawFlags, Ui};

use crate::profile::{cosmetic_color, rgb_u32_to_rgba};
use crate::theme::{
    PROFILE_BADGE, PROFILE_GAME_CARD_H, PROFILE_GAME_CARD_W, PROFILE_GAME_THUMB, PROFILE_RADIUS,
};

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn paint_nameplate_art(ui: &Ui, min: [f32; 2], max: [f32; 2], id: Option<&str>) {
    let draw = ui.get_window_draw_list();
    let color = id.and_then(cosmetic_color).unwrap_or(0x2d4a3e);
    let rgba = rgb_u32_to_rgba(color);
    let mid = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    draw.add_rect(min, max, rgba)
        .rounding(6.0)
        .filled(true)
        .build();
    let highlight = [
        rgba[0] * 1.25 + 0.05,
        rgba[1] * 1.25 + 0.05,
        rgba[2] * 1.25 + 0.05,
        0.55,
    ];
    draw.add_rect([min[0], min[1]], [max[0], mid[1]], highlight)
        .rounding(6.0)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    // Soft “forest mist” blobs for nameplate atmosphere.
    for (dx, dy, r) in [(-0.28, -0.1, 10.0), (0.22, 0.15, 8.0), (0.0, 0.05, 12.0)] {
        let c = [
            mid[0] + (max[0] - min[0]) * dx,
            mid[1] + (max[1] - min[1]) * dy,
        ];
        draw.add_circle(c, r, [1.0, 1.0, 1.0, 0.08])
            .filled(true)
            .build();
    }
}

pub fn paint_avatar_decoration_art(
    ui: &Ui,
    center: [f32; 2],
    radius: f32,
    decoration_id: Option<&str>,
) {
    let Some(id) = decoration_id else { return };
    let Some(color) = cosmetic_color(id) else {
        return;
    };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    let t = ui.time() as f32;
    draw.add_circle(center, radius + 9.0, rgba)
        .thickness(3.5)
        .build();
    if id.contains("leaf") {
        for i in 0..6 {
            let angle = 0.2 + i as f32 * (std::f32::consts::TAU / 6.0) + t * 0.35;
            let dist = 1.28 + (t * 1.7 + i as f32).sin() * 0.04;
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
            let a = i as f32 * std::f32::consts::TAU / 8.0 + t * 1.2;
            let orbit = radius + 12.0 + (t * 3.0 + i as f32).sin() * 2.0;
            let tip = [center[0] + orbit * a.cos(), center[1] + orbit * a.sin()];
            draw.add_circle(tip, 2.4, rgba).filled(true).build();
        }
    } else {
        let pulse = 14.0 + (t * 2.5).sin().abs() * 3.0;
        draw.add_circle(center, radius + pulse, [rgba[0], rgba[1], rgba[2], 0.35])
            .thickness(6.0)
            .build();
    }
}

pub fn paint_profile_frame_art(ui: &Ui, center: [f32; 2], radius: f32, frame_id: Option<&str>) {
    let Some(id) = frame_id else { return };
    let Some(color) = cosmetic_color(id) else {
        return;
    };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    let thick = if id.contains("thick") { 5.5 } else { 2.2 };
    draw.add_circle(center, radius + 12.0, rgba)
        .thickness(thick)
        .build();
    if id.contains("gold") {
        draw.add_circle(center, radius + 16.0, [rgba[0], rgba[1], rgba[2], 0.35])
            .thickness(1.5)
            .build();
    }
}

pub fn paint_profile_effect_art(ui: &Ui, min: [f32; 2], max: [f32; 2], effect_id: Option<&str>) {
    let Some(id) = effect_id else { return };
    let Some(color) = cosmetic_color(id) else {
        return;
    };
    let draw = ui.get_window_draw_list();
    let rgba = rgb_u32_to_rgba(color);
    let t = ui.time() as f32;
    let wash = 0.08 + (t * 1.4).sin().abs() * 0.06;
    draw.add_rect(min, max, [rgba[0], rgba[1], rgba[2], wash])
        .rounding(8.0)
        .filled(true)
        .build();
    if id.contains("particle") {
        let w = max[0] - min[0];
        let h = max[1] - min[1];
        for i in 0..7 {
            let phase = t * 0.9 + i as f32 * 0.85;
            let fx = (0.12 + i as f32 * 0.12 + phase.sin() * 0.04).rem_euclid(1.0);
            let fy = (0.18 + i as f32 * 0.11 + (phase * 0.7).cos() * 0.08).rem_euclid(1.0);
            draw.add_circle(
                [min[0] + w * fx, min[1] + h * fy],
                2.0 + (phase).sin().abs(),
                [rgba[0], rgba[1], rgba[2], 0.55],
            )
            .filled(true)
            .build();
        }
    } else {
        let band = 0.22 + (t * 0.8).sin().abs() * 0.08;
        draw.add_rect(
            [min[0], min[1]],
            [max[0], min[1] + (max[1] - min[1]) * band],
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
            draw.add_circle(center, r * 0.45, WHITE)
                .filled(true)
                .build();
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
            draw.add_circle(
                [center[0] + 2.0, center[1] - 2.0],
                r * 0.55,
                [rgba[0] * 0.7, rgba[1], rgba[2] * 0.7, 1.0],
            )
            .filled(true)
            .build();
        }
        BadgeShape::Eye => {
            draw.add_circle(center, r, rgba).filled(true).build();
            draw.add_circle(center, r * 0.45, BLACK)
                .filled(true)
                .build();
            draw.add_circle([center[0] + 1.5, center[1] - 1.5], r * 0.18, WHITE)
                .filled(true)
                .build();
        }
    }
}

/// Vertical game card (Games in Rotation / Favorite) using Banter procedural box art — not scraped art.
pub fn paint_game_card(ui: &Ui, min: [f32; 2], title: &str, tag: &str) {
    let draw = ui.get_window_draw_list();
    let max = [min[0] + PROFILE_GAME_CARD_W, min[1] + PROFILE_GAME_CARD_H];
    let cover = game_cover_color(title);
    let cover_hi = [
        cover[0] * 1.25 + 0.05,
        cover[1] * 1.2 + 0.05,
        cover[2] * 1.15 + 0.05,
        1.0,
    ];
    let cover_lo = [cover[0] * 0.55, cover[1] * 0.55, cover[2] * 0.6, 1.0];
    draw.add_rect(min, max, [0.08, 0.09, 0.10, 1.0])
        .rounding(PROFILE_RADIUS)
        .filled(true)
        .build();
    let art_max_y = min[1] + PROFILE_GAME_CARD_H * 0.62;
    draw.add_rect(min, [max[0], art_max_y], cover_lo)
        .rounding(PROFILE_RADIUS)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    draw.add_rect(min, [max[0], min[1] + PROFILE_GAME_CARD_H * 0.28], cover_hi)
        .rounding(PROFILE_RADIUS)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    // Spine / box-art bevel.
    draw.add_rect(
        [min[0], min[1]],
        [min[0] + 7.0, art_max_y],
        [0.0, 0.0, 0.0, 0.28],
    )
    .filled(true)
    .build();
    draw.add_rect(
        [min[0] + 10.0, min[1] + 10.0],
        [max[0] - 8.0, min[1] + 18.0],
        [1.0, 1.0, 1.0, 0.18],
    )
    .rounding(2.0)
    .filled(true)
    .build();
    let initials = game_initials(title);
    let size = ui.calc_text_size(&initials);
    draw.add_text(
        [
            min[0] + (PROFILE_GAME_CARD_W - size[0]) * 0.5,
            min[1] + PROFILE_GAME_CARD_H * 0.30 - size[1] * 0.5,
        ],
        WHITE,
        &initials,
    );
    let label = truncate(title, 10);
    draw.add_text(
        [min[0] + 6.0, min[1] + PROFILE_GAME_CARD_H * 0.66],
        WHITE,
        &label,
    );
    if !tag.is_empty() {
        let tag_min = [min[0] + 6.0, max[1] - 22.0];
        let tag_size = ui.calc_text_size(tag);
        let tag_max = [
            tag_min[0] + tag_size[0] + 10.0,
            tag_min[1] + tag_size[1] + 4.0,
        ];
        draw.add_rect(tag_min, tag_max, cover)
            .rounding(4.0)
            .filled(true)
            .build();
        draw.add_text([tag_min[0] + 5.0, tag_min[1] + 2.0], WHITE, tag);
    }
}

pub fn paint_game_thumb(ui: &Ui, min: [f32; 2], title: &str) {
    let draw = ui.get_window_draw_list();
    let max = [min[0] + PROFILE_GAME_THUMB, min[1] + PROFILE_GAME_THUMB];
    let cover = game_cover_color(title);
    let cover_hi = [
        cover[0] * 1.2 + 0.05,
        cover[1] * 1.15 + 0.05,
        cover[2] * 1.1 + 0.05,
        1.0,
    ];
    draw.add_rect(min, max, cover)
        .rounding(6.0)
        .filled(true)
        .build();
    draw.add_rect(min, [max[0], min[1] + PROFILE_GAME_THUMB * 0.35], cover_hi)
        .rounding(6.0)
        .flags(DrawFlags::ROUND_TOP)
        .filled(true)
        .build();
    draw.add_rect(
        [min[0], min[1]],
        [min[0] + 5.0, max[1]],
        [0.0, 0.0, 0.0, 0.25],
    )
    .filled(true)
    .build();
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
