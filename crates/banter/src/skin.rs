//! Discord-skin widgets drawn on Dear ImGui draw lists.
//!
//! Sizes, radii, font sizes, and colors come from `theme` so every screen shares one grid.
//! Column placement comes from `layout`. Keep new shared widgets here; keep screen flow in `ui`.
//! Each helper takes and releases its own draw list, so helpers can be called back to back.

use dear_imgui_rs::{fonts::FontId, DrawFlags, ManagedTextureId, StyleColor, StyleVar, Ui};

use crate::theme::{self, TooltipSide};
use crate::theme::{
    BLURPLE, BLURPLE_HOVER, BLURPLE_TEXT, DANGER, DANGER_HOVER, ERROR_TEXT, FLOATING_BG,
    GREEN_BUTTON, GREEN_BUTTON_HOVER, INPUT_BG, INPUT_RADIUS, MENU_ITEM_H, MENU_SEPARATOR,
    PLACEHOLDER, RED, SECONDARY_BUTTON, SECONDARY_BUTTON_HOVER, TEXT_NORMAL, TEXT_SECONDARY,
    TITLE_BAR_BG, TITLE_BAR_H, TITLE_BAR_LABEL_SIZE, TITLE_BAR_TEXT, TITLE_BTN_CLOSE,
    TITLE_BTN_CLOSE_HOVER, TITLE_BTN_GLYPH, TITLE_BTN_MAXIMIZE, TITLE_BTN_MAXIMIZE_HOVER,
    TITLE_BTN_MINIMIZE, TITLE_BTN_MINIMIZE_HOVER, TITLE_BTN_R, TOOLTIP_ARROW, TOOLTIP_PAD_X,
    TOOLTIP_PAD_Y, TOOLTIP_RADIUS, WHITE,
};
use crate::titlebar::{traffic_light_centers, TrafficLight};

pub(crate) const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

pub(crate) fn text_size(ui: &Ui, font: Option<FontId>, size: f32, value: &str) -> [f32; 2] {
    let _font = ui.push_font_with_size(font, size);
    ui.calc_text_size(value)
}

pub(crate) fn text(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    pos: [f32; 2],
    color: [f32; 4],
    value: &str,
) {
    let _font = ui.push_font_with_size(font, size);
    ui.get_window_draw_list().add_text(pos, color, value);
}

/// Text vertically centered inside a row that starts at `top` and is `height` tall.
pub(crate) fn text_in_row(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    x: f32,
    top: f32,
    height: f32,
    color: [f32; 4],
    value: &str,
) {
    let measured = text_size(ui, font, size, value);
    text(
        ui,
        font,
        size,
        [x, top + (height - measured[1]) * 0.5],
        color,
        value,
    );
}

pub(crate) fn text_centered(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    center_x: f32,
    y: f32,
    color: [f32; 4],
    value: &str,
) {
    let measured = text_size(ui, font, size, value);
    text(
        ui,
        font,
        size,
        [center_x - measured[0] * 0.5, y],
        color,
        value,
    );
}

/// Greedy word wrap for draw-list text, which Dear ImGui does not wrap by itself.
pub(crate) fn wrap_lines(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    value: &str,
    width: f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && text_size(ui, font, size, &candidate)[0] > width {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Text cut to `width` with an ellipsis, as Discord does for long names.
pub(crate) fn ellipsize(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    value: &str,
    width: f32,
) -> String {
    if text_size(ui, font, size, value)[0] <= width {
        return value.to_string();
    }
    let mut cut: String = value.to_string();
    while !cut.is_empty() {
        cut.pop();
        let candidate = format!("{}…", cut.trim_end());
        if text_size(ui, font, size, &candidate)[0] <= width {
            return candidate;
        }
    }
    "…".to_string()
}

pub(crate) fn image(
    ui: &Ui,
    texture: Option<ManagedTextureId>,
    min: [f32; 2],
    size: f32,
    tint: [f32; 4],
) {
    if let Some(texture) = texture {
        ui.get_window_draw_list().add_image(
            texture,
            min,
            [min[0] + size, min[1] + size],
            [0.0, 0.0],
            [1.0, 1.0],
            tint,
        );
    }
}

pub(crate) fn rect(ui: &Ui, min: [f32; 2], max: [f32; 2], color: [f32; 4], radius: f32) {
    ui.get_window_draw_list()
        .add_rect(min, max, color)
        .rounding(radius)
        .filled(true)
        .build();
}

pub(crate) fn circle(ui: &Ui, center: [f32; 2], radius: f32, color: [f32; 4]) {
    ui.get_window_draw_list()
        .add_circle(center, radius, color)
        .filled(true)
        .build();
}

/// Paint the client caption strip (background, traffic lights, centered label).
/// Interaction is handled by the caller via invisible buttons over the same geometry.
pub(crate) fn paint_title_bar(
    ui: &Ui,
    font: Option<FontId>,
    window_w: f32,
    label: &str,
    hovered: Option<TrafficLight>,
) {
    let bar_h = TITLE_BAR_H;
    rect(ui, [0.0, 0.0], [window_w, bar_h], TITLE_BAR_BG, 0.0);
    let show_glyphs = hovered.is_some();
    let centers = traffic_light_centers(bar_h);
    for (index, light) in TrafficLight::ALL.into_iter().enumerate() {
        let center = centers[index];
        let hot = hovered == Some(light);
        let fill = match (light, hot) {
            (TrafficLight::Close, false) => TITLE_BTN_CLOSE,
            (TrafficLight::Close, true) => TITLE_BTN_CLOSE_HOVER,
            (TrafficLight::Minimize, false) => TITLE_BTN_MINIMIZE,
            (TrafficLight::Minimize, true) => TITLE_BTN_MINIMIZE_HOVER,
            (TrafficLight::Maximize, false) => TITLE_BTN_MAXIMIZE,
            (TrafficLight::Maximize, true) => TITLE_BTN_MAXIMIZE_HOVER,
        };
        circle(ui, center, TITLE_BTN_R, fill);
        if show_glyphs {
            paint_traffic_glyph(ui, light, center);
        }
    }
    let label_y = (bar_h - TITLE_BAR_LABEL_SIZE) * 0.5;
    text_centered(
        ui,
        font,
        TITLE_BAR_LABEL_SIZE,
        window_w * 0.5,
        label_y,
        TITLE_BAR_TEXT,
        label,
    );
}

fn paint_traffic_glyph(ui: &Ui, light: TrafficLight, center: [f32; 2]) {
    let draw = ui.get_window_draw_list();
    let c = TITLE_BTN_GLYPH;
    match light {
        TrafficLight::Close => {
            let s = 2.6;
            draw.add_line(
                [center[0] - s, center[1] - s],
                [center[0] + s, center[1] + s],
                c,
            )
            .thickness(1.2)
            .build();
            draw.add_line(
                [center[0] + s, center[1] - s],
                [center[0] - s, center[1] + s],
                c,
            )
            .thickness(1.2)
            .build();
        }
        TrafficLight::Minimize => {
            let s = 3.0;
            draw.add_line(
                [center[0] - s, center[1]],
                [center[0] + s, center[1]],
                c,
            )
            .thickness(1.2)
            .build();
        }
        TrafficLight::Maximize => {
            // Small + (Mac zoom affordance).
            let s = 2.8;
            draw.add_line(
                [center[0] - s, center[1]],
                [center[0] + s, center[1]],
                c,
            )
            .thickness(1.2)
            .build();
            draw.add_line(
                [center[0], center[1] - s],
                [center[0], center[1] + s],
                c,
            )
            .thickness(1.2)
            .build();
        }
    }
}

pub(crate) fn rect_corners(
    ui: &Ui,
    min: [f32; 2],
    max: [f32; 2],
    color: [f32; 4],
    radius: f32,
    corners: DrawFlags,
) {
    ui.get_window_draw_list()
        .add_rect(min, max, color)
        .rounding(radius)
        .flags(corners)
        .filled(true)
        .build();
}

/// Soft drop shadow (Discord's `0 2px 10px rgba(0,0,0,.2)`), faked with stacked translucent rounded rects.
pub(crate) fn shadow(ui: &Ui, min: [f32; 2], max: [f32; 2], radius: f32) {
    const LAYERS: usize = 6;
    let draw = ui.get_window_draw_list();
    for layer in 0..LAYERS {
        let spread = (LAYERS - layer) as f32 * 10.0 / LAYERS as f32;
        let color = [0.0, 0.0, 0.0, 0.2 / LAYERS as f32];
        draw.add_rect(
            [min[0] - spread, min[1] - spread + 2.0],
            [max[0] + spread, max[1] + spread + 2.0],
            color,
        )
        .rounding(radius + spread)
        .filled(true)
        .build();
    }
}

pub(crate) fn line_h(ui: &Ui, x0: f32, x1: f32, y: f32, color: [f32; 4]) {
    rect(ui, [x0, y], [x1, y + 1.0], color, 0.0);
}

/// Frosted glass / acrylic approximation for the stream-settings chip.
///
/// Real backdrop blur is not available in this ImGui path, so the look is layered
/// translucent fills, a top highlight, a light rim, and sparse noise speckles.
pub(crate) fn glass_chip(ui: &Ui, min: [f32; 2], max: [f32; 2], radius: f32, alpha: f32) {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.01 {
        return;
    }
    let draw = ui.get_window_draw_list();
    let mul = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * alpha];
    // Soft outer bloom.
    for layer in 0..4 {
        let spread = (4 - layer) as f32 * 3.0;
        draw.add_rect(
            [min[0] - spread, min[1] - spread],
            [max[0] + spread, max[1] + spread],
            [0.0, 0.0, 0.0, 0.06 * alpha / (layer + 1) as f32],
        )
        .rounding(radius + spread)
        .filled(true)
        .build();
    }
    draw.add_rect(min, max, mul(theme::SHARE_GLASS_FILL_INNER))
        .rounding(radius)
        .filled(true)
        .build();
    draw.add_rect(min, max, mul(theme::SHARE_GLASS_FILL))
        .rounding(radius)
        .filled(true)
        .build();
    // Specular top edge.
    let hi = (max[1] - min[1]).min(18.0);
    draw.add_rect(min, [max[0], min[1] + hi], mul(theme::SHARE_GLASS_HIGHLIGHT))
        .rounding(radius)
        .filled(true)
        .build();
    draw.add_rect(min, max, mul(theme::SHARE_GLASS_BORDER))
        .rounding(radius)
        .thickness(1.25)
        .build();
    // Sparse noise for acrylic grain (deterministic hash, no texture upload).
    let w = (max[0] - min[0]).max(1.0);
    let h = (max[1] - min[1]).max(1.0);
    let cols = (w / 7.0).floor().max(4.0) as i32;
    let rows = (h / 7.0).floor().max(3.0) as i32;
    for row in 0..rows {
        for col in 0..cols {
            let n = ((col.wrapping_mul(374761393) ^ row.wrapping_mul(668265263)) as u32)
                .wrapping_mul(0x9E37_79B9);
            if n & 0x7 != 0 {
                continue;
            }
            let x = min[0] + (col as f32 + 0.35) * (w / cols as f32);
            let y = min[1] + (row as f32 + 0.4) * (h / rows as f32);
            draw.add_rect(
                [x, y],
                [x + 1.2, y + 1.2],
                mul(theme::SHARE_GLASS_NOISE),
            )
            .filled(true)
            .build();
        }
    }
}

/// Discord's floating black tooltip with an arrow pointing at the anchor.
pub(crate) fn tooltip(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    value: &str,
    anchor_min: [f32; 2],
    anchor_max: [f32; 2],
    side: TooltipSide,
) {
    let measured = text_size(ui, font, size, value);
    let display = ui.io().display_size();
    let (min, max) = theme::tooltip_rect(anchor_min, anchor_max, measured, side, display);
    let _font = ui.push_font_with_size(font, size);
    let draw = ui.get_foreground_draw_list();
    draw.add_rect(min, max, FLOATING_BG)
        .rounding(TOOLTIP_RADIUS)
        .filled(true)
        .build();
    match side {
        TooltipSide::Right => {
            let cy = (min[1] + max[1]) * 0.5;
            draw.add_triangle(
                [min[0] - TOOLTIP_ARROW, cy],
                [min[0], cy - TOOLTIP_ARROW],
                [min[0], cy + TOOLTIP_ARROW],
                FLOATING_BG,
            )
            .filled(true)
            .build();
        }
        TooltipSide::Top => {
            let cx = ((anchor_min[0] + anchor_max[0]) * 0.5)
                .clamp(min[0] + 8.0, (max[0] - 8.0).max(min[0] + 8.0));
            if max[1] <= anchor_min[1] {
                draw.add_triangle(
                    [cx - TOOLTIP_ARROW, max[1]],
                    [cx + TOOLTIP_ARROW, max[1]],
                    [cx, max[1] + TOOLTIP_ARROW],
                    FLOATING_BG,
                )
                .filled(true)
                .build();
            } else {
                draw.add_triangle(
                    [cx - TOOLTIP_ARROW, min[1]],
                    [cx, min[1] - TOOLTIP_ARROW],
                    [cx + TOOLTIP_ARROW, min[1]],
                    FLOATING_BG,
                )
                .filled(true)
                .build();
            }
        }
    }
    draw.add_text(
        [min[0] + TOOLTIP_PAD_X, min[1] + TOOLTIP_PAD_Y],
        TEXT_NORMAL,
        value,
    );
}

/// Tooltip above the last item when it is hovered.
pub(crate) fn item_tooltip(ui: &Ui, font: Option<FontId>, value: &str) {
    if ui.is_item_hovered() {
        tooltip(
            ui,
            font,
            theme::FONT_SM,
            value,
            ui.item_rect_min(),
            ui.item_rect_max(),
            TooltipSide::Top,
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Brand,
    Green,
    Danger,
    Secondary,
    Link,
}

pub(crate) fn button(
    ui: &Ui,
    id: &str,
    label: &str,
    tone: Tone,
    min: [f32; 2],
    size: [f32; 2],
    enabled: bool,
    font: Option<FontId>,
) -> bool {
    ui.set_cursor_screen_pos(min);
    let pressed = ui.invisible_button(id, size);
    let hovered = enabled && ui.is_item_hovered();
    let max = [min[0] + size[0], min[1] + size[1]];
    let alpha = if enabled { 1.0 } else { 0.5 };
    let (fill, color) = match tone {
        Tone::Brand => (if hovered { BLURPLE_HOVER } else { BLURPLE }, WHITE),
        Tone::Green => (
            if hovered {
                GREEN_BUTTON_HOVER
            } else {
                GREEN_BUTTON
            },
            WHITE,
        ),
        Tone::Danger => (if hovered { DANGER_HOVER } else { DANGER }, WHITE),
        Tone::Secondary => (
            if hovered {
                SECONDARY_BUTTON_HOVER
            } else {
                SECONDARY_BUTTON
            },
            WHITE,
        ),
        Tone::Link => (CLEAR, TEXT_NORMAL),
    };
    if tone != Tone::Link {
        rect(ui, min, max, theme::with_alpha(fill, alpha), 3.0);
    }
    // Discord's large (44 px) buttons use 16 px labels; medium and small ones use 14 px.
    let font_size = if size[1] >= 44.0 {
        theme::FONT_MD
    } else {
        theme::FONT_SM
    };
    let measured = text_size(ui, font, font_size, label);
    let pos = [
        min[0] + (size[0] - measured[0]) * 0.5,
        min[1] + (size[1] - measured[1]) * 0.5,
    ];
    text(
        ui,
        font,
        font_size,
        pos,
        theme::with_alpha(color, alpha),
        label,
    );
    if tone == Tone::Link && hovered {
        line_h(
            ui,
            pos[0],
            pos[0] + measured[0],
            pos[1] + measured[1],
            color,
        );
    }
    pressed && enabled
}

/// Inline text link. Returns true when clicked.
pub(crate) fn link(
    ui: &Ui,
    id: &str,
    label: &str,
    pos: [f32; 2],
    font: Option<FontId>,
    size: f32,
    color: [f32; 4],
) -> bool {
    let measured = text_size(ui, font, size, label);
    ui.set_cursor_screen_pos(pos);
    let clicked = ui.invisible_button(id, measured);
    text(ui, font, size, pos, color, label);
    if ui.is_item_hovered() {
        line_h(
            ui,
            pos[0],
            pos[0] + measured[0],
            pos[1] + measured[1] - 1.0,
            color,
        );
    }
    clicked
}

/// A Discord text field: dark, borderless, 3 px corners. Returns true on Enter.
pub(crate) fn input(
    ui: &Ui,
    id: &str,
    value: &mut String,
    hint: &str,
    min: [f32; 2],
    width: f32,
    height: f32,
    font_size: f32,
    read_only: bool,
) -> bool {
    ui.set_cursor_screen_pos(min);
    let _font = ui.push_font_with_size(None, font_size);
    let pad_y = ((height - font_size) * 0.5).max(0.0);
    let _pad = ui.push_style_var(StyleVar::FramePadding([10.0, pad_y]));
    let _round = ui.push_style_var(StyleVar::FrameRounding(INPUT_RADIUS));
    let _border = ui.push_style_var(StyleVar::FrameBorderSize(0.0));
    let _bg = ui.push_style_color(StyleColor::FrameBg, INPUT_BG);
    let _bg_hover = ui.push_style_color(StyleColor::FrameBgHovered, INPUT_BG);
    let _bg_active = ui.push_style_color(StyleColor::FrameBgActive, INPUT_BG);
    let _hint = ui.push_style_color(StyleColor::TextDisabled, PLACEHOLDER);
    let _text = ui.push_style_color(StyleColor::Text, TEXT_NORMAL);
    ui.set_next_item_width(width);
    ui.input_text(id, value)
        .hint(hint)
        .read_only(read_only)
        .enter_returns_true(true)
        .build()
}

/// 12 px bold uppercase field label. A required field gets a red `*`; an error turns it red.
pub(crate) fn field_label(
    ui: &Ui,
    bold: Option<FontId>,
    pos: [f32; 2],
    label: &str,
    required: bool,
    error: Option<&str>,
) {
    let color = if error.is_some() {
        ERROR_TEXT
    } else {
        TEXT_SECONDARY
    };
    let upper = label.to_uppercase();
    text(ui, bold, theme::FONT_XS, pos, color, &upper);
    let mut x = pos[0] + text_size(ui, bold, theme::FONT_XS, &upper)[0];
    if required {
        text(ui, bold, theme::FONT_XS, [x + 4.0, pos[1]], RED, "*");
        x += 4.0 + text_size(ui, bold, theme::FONT_XS, "*")[0];
    }
    if let Some(error) = error {
        let tail = format!(" - {error}");
        text(ui, None, theme::FONT_XS, [x, pos[1]], ERROR_TEXT, &tail);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuTone {
    Normal,
    Brand,
    Danger,
}

/// One Discord context-menu row: label left, icon right, blurple fill on hover.
pub(crate) fn menu_item(
    ui: &Ui,
    id: &str,
    label: &str,
    icon: Option<ManagedTextureId>,
    tone: MenuTone,
    min: [f32; 2],
    width: f32,
    font: Option<FontId>,
) -> bool {
    ui.set_cursor_screen_pos(min);
    let clicked = ui.invisible_button(id, [width, MENU_ITEM_H]);
    let hovered = ui.is_item_hovered();
    let max = [min[0] + width, min[1] + MENU_ITEM_H];
    if hovered {
        let fill = if tone == MenuTone::Danger {
            DANGER
        } else {
            BLURPLE_HOVER
        };
        rect(ui, min, max, fill, 2.0);
    }
    let color = if hovered {
        WHITE
    } else {
        match tone {
            MenuTone::Normal => TEXT_SECONDARY,
            MenuTone::Brand => BLURPLE_TEXT,
            MenuTone::Danger => RED,
        }
    };
    text_in_row(
        ui,
        font,
        theme::FONT_SM,
        min[0] + 8.0,
        min[1],
        MENU_ITEM_H,
        color,
        label,
    );
    image(
        ui,
        icon,
        [max[0] - 8.0 - 18.0, min[1] + (MENU_ITEM_H - 18.0) * 0.5],
        18.0,
        color,
    );
    clicked
}

pub(crate) fn menu_separator(ui: &Ui, min: [f32; 2], width: f32) {
    line_h(
        ui,
        min[0] + 4.0,
        min[0] + width - 4.0,
        min[1] + 4.0,
        MENU_SEPARATOR,
    );
}

/// Square icon button. Icons are white rasters, so `tint` sets their color.
pub(crate) fn icon_button(
    ui: &Ui,
    id: &str,
    texture: Option<ManagedTextureId>,
    min: [f32; 2],
    box_size: f32,
    icon_size: f32,
    tint: [f32; 4],
    hover_tint: [f32; 4],
    hover_bg: Option<[f32; 4]>,
) -> bool {
    ui.set_cursor_screen_pos(min);
    let clicked = ui.invisible_button(id, [box_size, box_size]);
    let hovered = ui.is_item_hovered();
    if hovered {
        if let Some(bg) = hover_bg {
            rect(ui, min, [min[0] + box_size, min[1] + box_size], bg, 4.0);
        }
    }
    let pad = (box_size - icon_size) * 0.5;
    image(
        ui,
        texture,
        [min[0] + pad, min[1] + pad],
        icon_size,
        if hovered { hover_tint } else { tint },
    );
    clicked
}

/// Discord's rounded checkbox. Returns true when toggled.
pub(crate) fn checkbox(
    ui: &Ui,
    id: &str,
    label: &str,
    value: &mut bool,
    min: [f32; 2],
    check: Option<ManagedTextureId>,
    font: Option<FontId>,
) -> bool {
    let box_size = 20.0;
    let label_size = text_size(ui, font, theme::FONT_SM, label);
    ui.set_cursor_screen_pos(min);
    let clicked = ui.invisible_button(id, [box_size + 8.0 + label_size[0], box_size]);
    if clicked {
        *value = !*value;
    }
    let max = [min[0] + box_size, min[1] + box_size];
    if *value {
        rect(ui, min, max, BLURPLE, 4.0);
        image(ui, check, [min[0] + 3.0, min[1] + 3.0], 14.0, WHITE);
    } else {
        ui.get_window_draw_list()
            .add_rect(min, max, TEXT_SECONDARY)
            .rounding(4.0)
            .thickness(1.0)
            .build();
    }
    text_in_row(
        ui,
        font,
        theme::FONT_SM,
        max[0] + 8.0,
        min[1],
        box_size,
        TEXT_SECONDARY,
        label,
    );
    clicked
}
