//! Shared metrics and colors for the temporary Discord skin.
//!
//! Values follow Discord's desktop dark theme. Every screen reads its sizes, radii,
//! font sizes, and colors from here so columns and rows line up across the window.

/// `0xRRGGBB` as normalized RGBA.
pub const fn hex(rgb: u32) -> [f32; 4] {
    [
        ((rgb >> 16) & 0xFF) as f32 / 255.0,
        ((rgb >> 8) & 0xFF) as f32 / 255.0,
        (rgb & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

pub const fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], alpha]
}

// Backgrounds.
pub const RAIL_BG: [f32; 4] = hex(0x1E1F22);
pub const SIDEBAR_BG: [f32; 4] = hex(0x2B2D31);
pub const STAGE_BG: [f32; 4] = hex(0x313338);
pub const PANEL_BG: [f32; 4] = hex(0x232428);
pub const FLOATING_BG: [f32; 4] = hex(0x111214);
pub const INPUT_BG: [f32; 4] = hex(0x1E1F22);
pub const COMPOSER_BG: [f32; 4] = hex(0x383A40);
pub const CALL_BG: [f32; 4] = hex(0x000000);
pub const TILE_BG: [f32; 4] = hex(0x2B2D31);
pub const CONTROL_BG: [f32; 4] = hex(0x2B2D31);
pub const CONTROL_HOVER: [f32; 4] = hex(0x393B41);
pub const MODAL_FOOTER_BG: [f32; 4] = hex(0x2B2D31);
pub const OVERLAY: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
pub const ROW_HOVER: [f32; 4] = hex(0x35373C);
pub const ROW_SELECTED: [f32; 4] = hex(0x404249);
pub const ICON_HOVER_BG: [f32; 4] = hex(0x3D3E45);
pub const RAIL_SEPARATOR: [f32; 4] = hex(0x35363C);
pub const DIVIDER: [f32; 4] = hex(0x3F4147);
pub const HEADER_SHADOW: [f32; 4] = [4.0 / 255.0, 4.0 / 255.0, 5.0 / 255.0, 0.32];
pub const HEADER_SHADOW_SOFT: [f32; 4] = [4.0 / 255.0, 4.0 / 255.0, 5.0 / 255.0, 0.08];
pub const SCROLL_THUMB: [f32; 4] = hex(0x1A1B1E);
pub const SCROLL_TRACK: [f32; 4] = hex(0x2B2D31);
pub const MENU_SEPARATOR: [f32; 4] = [78.0 / 255.0, 80.0 / 255.0, 88.0 / 255.0, 0.48];
pub const LIST_DIVIDER: [f32; 4] = [78.0 / 255.0, 80.0 / 255.0, 88.0 / 255.0, 0.48];
pub const SECONDARY_BUTTON: [f32; 4] = hex(0x4E5058);
pub const SECONDARY_BUTTON_HOVER: [f32; 4] = hex(0x6D6F78);
pub const NOTICE_BG: [f32; 4] = hex(0xDA373C);
pub const ERROR_TEXT: [f32; 4] = hex(0xFA777C);
pub const GREEN_TEXT: [f32; 4] = hex(0x2DC770);
pub const LOGIN_TOP: [f32; 4] = hex(0x404EED);
pub const LOGIN_BOTTOM: [f32; 4] = hex(0x2C2F8F);

// Text and icons.
pub const TEXT: [f32; 4] = hex(0xF2F3F5);
pub const TEXT_NORMAL: [f32; 4] = hex(0xDBDEE1);
pub const TEXT_SECONDARY: [f32; 4] = hex(0xB5BAC1);
pub const MUTED: [f32; 4] = hex(0x949BA4);
pub const CHANNEL_ICON: [f32; 4] = hex(0x80848E);
pub const PLACEHOLDER: [f32; 4] = hex(0x6D6F78);
pub const LINK: [f32; 4] = hex(0x00A8FC);
pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

// Accents.
pub const BLURPLE: [f32; 4] = hex(0x5865F2);
pub const BLURPLE_HOVER: [f32; 4] = hex(0x4752C4);
pub const BLURPLE_ACTIVE: [f32; 4] = hex(0x3C45A5);
pub const BLURPLE_TEXT: [f32; 4] = hex(0x949CF7);
pub const GREEN: [f32; 4] = hex(0x23A55A);
pub const GREEN_BUTTON: [f32; 4] = hex(0x248046);
pub const GREEN_BUTTON_HOVER: [f32; 4] = hex(0x1A6334);
pub const RED: [f32; 4] = hex(0xF23F43);
pub const DANGER: [f32; 4] = hex(0xDA373C);
pub const DANGER_HOVER: [f32; 4] = hex(0xA12828);
pub const YELLOW: [f32; 4] = hex(0xF0B232);
pub const OFFLINE: [f32; 4] = hex(0x80848E);

// Columns and bars.
pub const RAIL_W: f32 = 72.0;
pub const SIDEBAR_W: f32 = 240.0;
pub const MEMBERS_W: f32 = 240.0;
pub const HEADER_H: f32 = 48.0;
pub const USER_PANEL_H: f32 = 52.0;
pub const VOICE_PANEL_H: f32 = 96.0;

// Server rail.
pub const RAIL_ICON: f32 = 48.0;
pub const RAIL_ICON_GAP: f32 = 8.0;
pub const RAIL_TOP: f32 = 12.0;
pub const RAIL_SEPARATOR_W: f32 = 32.0;
pub const RAIL_SEPARATOR_H: f32 = 2.0;
pub const RAIL_PILL_W: f32 = 4.0;

// Channel list.
pub const CHANNEL_ROW_H: f32 = 32.0;
pub const CHANNEL_ROW_GAP: f32 = 2.0;
pub const ROW_INSET: f32 = 8.0;
pub const ROW_RADIUS: f32 = 4.0;
pub const CATEGORY_H: f32 = 40.0;
pub const VOICE_MEMBER_H: f32 = 32.0;
pub const VOICE_AVATAR: f32 = 24.0;

// Member list.
pub const MEMBER_ROW_H: f32 = 42.0;
pub const MEMBER_GROUP_H: f32 = 40.0;
pub const AVATAR: f32 = 32.0;

// Chat.
pub const COMPOSER_H: f32 = 44.0;
pub const COMPOSER_MARGIN: f32 = 16.0;
pub const COMPOSER_BOTTOM: f32 = 24.0;
pub const COMPOSER_RADIUS: f32 = 8.0;
pub const SEARCH_W: f32 = 144.0;
pub const SEARCH_H: f32 = 24.0;
pub const HEADER_ICON: f32 = 24.0;
pub const HEADER_ICON_GAP: f32 = 16.0;

// Call view.
pub const CALL_BUTTON: f32 = 56.0;
pub const CALL_CHEVRON_W: f32 = 24.0;
pub const CALL_BUTTON_GAP: f32 = 16.0;
pub const CALL_BAR_H: f32 = 96.0;
pub const TILE_GAP: f32 = 8.0;
pub const TILE_RADIUS: f32 = 8.0;
pub const TILE_AVATAR: f32 = 80.0;
/// Large peer avatar while a 1:1 DM call is ringing (Discord-style center stage).
pub const CALL_RING_AVATAR: f32 = 120.0;

// Floating surfaces.
pub const TOOLTIP_PAD_X: f32 = 12.0;
pub const TOOLTIP_PAD_Y: f32 = 8.0;
pub const TOOLTIP_GAP: f32 = 8.0;
pub const TOOLTIP_RADIUS: f32 = 4.0;
pub const TOOLTIP_ARROW: f32 = 5.0;
pub const MENU_W: f32 = 220.0;
pub const MENU_PAD: f32 = 8.0;
pub const MENU_ITEM_H: f32 = 32.0;
pub const MENU_RADIUS: f32 = 4.0;
pub const MODAL_W: f32 = 440.0;
pub const MODAL_PAD: f32 = 16.0;
pub const MODAL_RADIUS: f32 = 8.0;
pub const MODAL_FOOTER_H: f32 = 70.0;

// Screen Share / Go Live picker (Discord Stream Select).
pub const SHARE_MODAL_W: f32 = 880.0;
pub const SHARE_MODAL_H: f32 = 660.0;
pub const SHARE_PAD: f32 = 16.0;
pub const SHARE_HEADER_H: f32 = 52.0;
pub const SHARE_TAB_H: f32 = 36.0;
pub const SHARE_FOOTER_H: f32 = 70.0;
pub const SHARE_TILE_W: f32 = 248.0;
pub const SHARE_TILE_H: f32 = 178.0;
pub const SHARE_TILE_GAP: f32 = 12.0;
pub const SHARE_THUMB_H: f32 = 132.0;
pub const SHARE_TILE_RADIUS: f32 = 8.0;
pub const SHARE_CHECK: f32 = 22.0;

pub const PROFILER_W: f32 = 360.0;
pub const PROFILER_H: f32 = 420.0;
pub const PROFILER_PAD: f32 = 12.0;
pub const PROFILER_MARGIN: f32 = 16.0;
/// Floating user profile card (Discord-like popout).
pub const PROFILE_W: f32 = 340.0;
pub const PROFILE_BANNER_H: f32 = 60.0;
pub const PROFILE_AVATAR: f32 = 40.0;
pub const PROFILE_PAD: f32 = 16.0;
pub const PROFILE_RADIUS: f32 = 8.0;
pub const PROFILE_BODY_MIN_H: f32 = 180.0;
pub const BUTTON_H: f32 = 38.0;
pub const INPUT_H: f32 = 40.0;
pub const INPUT_RADIUS: f32 = 3.0;

// Type scale.
pub const FONT_XS: f32 = 12.0;
pub const FONT_SM: f32 = 14.0;
pub const FONT_MD: f32 = 16.0;
pub const FONT_LG: f32 = 20.0;
pub const FONT_XL: f32 = 24.0;

/// The white pill on the rail's left edge: tall for the open server, short on hover.
pub fn rail_pill_height(selected: bool, hovered: bool) -> f32 {
    if selected {
        40.0
    } else if hovered {
        20.0
    } else {
        0.0
    }
}

/// Rail icons are circles at rest and morph into a rounded square on hover or select.
pub fn rail_corner_radius(active: bool) -> f32 {
    if active { 16.0 } else { RAIL_ICON * 0.5 }
}

/// Discord's acronym for a server without an icon: `'s ` is dropped, each word
/// collapses to its first character, punctuation stays, and spaces go away.
pub fn server_acronym(name: &str) -> String {
    let cleaned = name.replace("'s ", " ");
    let mut acronym = String::new();
    for word in cleaned.split_whitespace() {
        let mut in_word = false;
        for ch in word.chars() {
            let word_char = ch.is_alphanumeric() || ch == '_';
            if word_char {
                if !in_word {
                    acronym.push(ch);
                }
                in_word = true;
            } else {
                acronym.push(ch);
                in_word = false;
            }
        }
    }
    acronym
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGrid {
    pub cols: usize,
    pub rows: usize,
    pub tile_w: f32,
    pub tile_h: f32,
}

/// Whether the DM stage should show the one-on-one call view instead of the chat.
///
/// Only the conversation that owns the call replaces the stage; other DMs and Home stay as chat.
pub fn dm_call_takes_stage(call_dm_id: Option<i64>, viewing_dm_id: Option<i64>) -> bool {
    match (call_dm_id, viewing_dm_id) {
        (Some(call), Some(place)) => call == place,
        _ => false,
    }
}

/// Participant tiles for an active 1:1 call (caller and callee). Ringing uses the large avatar layout.
pub fn dm_call_participant_tiles(active: bool) -> usize {
    if active { 2 } else { 0 }
}

/// Center of the large peer avatar while a 1:1 call is ringing, above the control bar.
pub fn call_ring_avatar_center(origin: [f32; 2], width: f32, height: f32, bar_h: f32) -> [f32; 2] {
    let usable_h = (height - bar_h).max(0.0);
    [origin[0] + width * 0.5, origin[1] + usable_h * 0.42]
}

/// The column count that gives the largest 16:9 tiles inside the stage.
pub fn voice_tile_grid(count: usize, area_w: f32, area_h: f32, gap: f32) -> TileGrid {
    let mut best = TileGrid { cols: 0, rows: 0, tile_w: 0.0, tile_h: 0.0 };
    if count == 0 || area_w <= 0.0 || area_h <= 0.0 {
        return best;
    }
    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let fit_w = (area_w - gap * (cols - 1) as f32) / cols as f32;
        let fit_h = (area_h - gap * (rows - 1) as f32) / rows as f32;
        let tile_w = fit_w.min(fit_h * 16.0 / 9.0);
        if tile_w > best.tile_w {
            best = TileGrid { cols, rows, tile_w, tile_h: tile_w * 9.0 / 16.0 };
        }
    }
    best
}

/// Empty space above the message list so a short history sits on the composer, as in Discord.
pub fn chat_top_pad(content_h: f32, view_h: f32) -> f32 {
    (view_h - content_h).max(0.0)
}

/// How far from the bottom (px) still counts as "at the bottom" for following new messages.
pub const CHAT_STICK_SLACK: f32 = 24.0;

/// Whether the message list should jump to the newest message this frame.
pub fn chat_should_stick(scroll_y: f32, scroll_max_y: f32, just_sent: bool) -> bool {
    just_sent || scroll_max_y <= 0.0 || scroll_y >= scroll_max_y - CHAT_STICK_SLACK
}

/// Left x offsets for a row of items centered in `total`.
pub fn centered_row(total: f32, widths: &[f32], gap: f32) -> Vec<f32> {
    if widths.is_empty() {
        return Vec::new();
    }
    let row: f32 = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
    let mut x = (total - row) * 0.5;
    widths
        .iter()
        .map(|width| {
            let left = x;
            x += width + gap;
            left
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusDot {
    pub center: [f32; 2],
    pub radius: f32,
    /// Radius of the background-colored ring cut out of the avatar around the dot.
    pub cutout: f32,
}

/// Presence dot geometry, scaled from Discord's 10 px dot with a 3 px ring on a 32 px avatar.
pub fn status_dot(avatar_center: [f32; 2], avatar_radius: f32) -> StatusDot {
    let offset = avatar_radius * 11.0 / 16.0;
    StatusDot {
        center: [avatar_center[0] + offset, avatar_center[1] + offset],
        radius: avatar_radius * 5.0 / 16.0,
        cutout: avatar_radius * 8.0 / 16.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipSide {
    Right,
    Top,
}

/// Tooltip box for an anchor. A top tooltip that would leave the screen flips below the anchor.
pub fn tooltip_rect(
    anchor_min: [f32; 2],
    anchor_max: [f32; 2],
    text: [f32; 2],
    side: TooltipSide,
    display: [f32; 2],
) -> ([f32; 2], [f32; 2]) {
    let w = text[0] + TOOLTIP_PAD_X * 2.0;
    let h = text[1] + TOOLTIP_PAD_Y * 2.0;
    let (mut x, mut y) = match side {
        TooltipSide::Right => (anchor_max[0] + TOOLTIP_GAP, (anchor_min[1] + anchor_max[1]) * 0.5 - h * 0.5),
        TooltipSide::Top => {
            let x = (anchor_min[0] + anchor_max[0]) * 0.5 - w * 0.5;
            let above = anchor_min[1] - TOOLTIP_GAP - h;
            (x, if above < 0.0 { anchor_max[1] + TOOLTIP_GAP } else { above })
        }
    };
    x = x.clamp(0.0, (display[0] - w).max(0.0));
    y = y.clamp(0.0, (display[1] - h).max(0.0));
    ([x, y], [x + w, y + h])
}
