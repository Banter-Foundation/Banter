//! Theme tokens for the temporary Discord skin.
//!
//! Values follow Discord's desktop dark theme. Every screen reads its sizes, radii,
//! font sizes, and colors from here so columns and rows line up across the window.
//! Do not hard-code colors or pixel sizes in `ui.rs` — add a token here instead.
//!
//! Token groups (search these section comments):
//! - **Backgrounds** — rail / sidebar / stage / panel / input fills
//! - **Text and icons** — primary, muted, links, placeholders
//! - **Accents** — blurple, green, danger, status
//! - **Columns and bars** — `RAIL_W`, `SIDEBAR_W`, `MEMBERS_W`, header / user / voice heights
//! - **Server rail / channel list / member list / chat / call / floating / type scale**
//!
//! Column geometry and named regions live in [`crate::layout`]. Draw helpers live in `skin`.
//! Branch purpose: `docs/ui-theme-layout.md`.

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

// Client-drawn Mac-style title bar (BA-105). Replaces native OS chrome.
/// Short caption strip above the Discord-like shell.
pub const TITLE_BAR_H: f32 = 28.0;
pub const TITLE_BAR_BG: [f32; 4] = hex(0x1A1B1E);
pub const TITLE_BAR_TEXT: [f32; 4] = TEXT_SECONDARY;
/// Traffic-light fill colors (close / minimize / maximize).
pub const TITLE_BTN_CLOSE: [f32; 4] = hex(0xFF5F57);
pub const TITLE_BTN_MINIMIZE: [f32; 4] = hex(0xFEBC2E);
pub const TITLE_BTN_MAXIMIZE: [f32; 4] = hex(0x28C840);
pub const TITLE_BTN_CLOSE_HOVER: [f32; 4] = hex(0xFF8A84);
pub const TITLE_BTN_MINIMIZE_HOVER: [f32; 4] = hex(0xFFD35A);
pub const TITLE_BTN_MAXIMIZE_HOVER: [f32; 4] = hex(0x5ADE72);
pub const TITLE_BTN_GLYPH: [f32; 4] = hex(0x4D0000);
pub const TITLE_BTN_R: f32 = 6.0;
pub const TITLE_BTN_GAP: f32 = 8.0;
pub const TITLE_BTN_PAD_LEFT: f32 = 12.0;
/// Extra hit padding beyond the drawn circle radius.
pub const TITLE_BTN_HIT_PAD: f32 = 4.0;
pub const TITLE_BAR_LABEL_SIZE: f32 = 12.0;

// Server rail.
pub const RAIL_ICON: f32 = 48.0;
pub const RAIL_ICON_GAP: f32 = 8.0;
pub const RAIL_TOP: f32 = 12.0;
pub const RAIL_SEPARATOR_W: f32 = 32.0;
pub const RAIL_SEPARATOR_H: f32 = 2.0;
pub const RAIL_PILL_W: f32 = 4.0;

// Reddit-style comment threads in text chat / DMs (BA-82 thread lines).
/// Fixed horizontal step per nesting depth (~20–30 px like Reddit dark mode).
pub const COMMENT_INDENT: f32 = 24.0;
pub const COMMENT_MAX_VISUAL_DEPTH: u32 = 12;
/// Left pad before the depth-0 collapse column (matches chat row inset).
pub const COMMENT_LEFT_PAD: f32 = 16.0;
/// Gutter width for one collapse circle + vertical guide column.
pub const COMMENT_COLLAPSE_W: f32 = 18.0;
/// Radius of the circular (−) collapse control.
pub const COMMENT_FOLD_RADIUS: f32 = 7.0;
/// Hit box for the collapse circle (slightly larger than the drawn circle).
pub const COMMENT_FOLD_HIT: f32 = 16.0;
/// Top padding from the row origin down to the collapse circle.
pub const COMMENT_FOLD_PAD_Y: f32 = 4.0;
/// Thin solid vertical gutters only — no L-hooks or rounded connectors.
pub const COMMENT_GUIDE_THICK: f32 = 1.0;
/// Click / hover half-width around a vertical thread guide.
pub const COMMENT_LINE_HIT_W: f32 = 10.0;
/// Extra space before a new root comment so threads scan as blocks.
pub const COMMENT_ROW_GAP_ROOT: f32 = 12.0;
/// Tighter gap between nested replies inside one thread.
pub const COMMENT_ROW_GAP_NESTED: f32 = 4.0;
/// Always-visible Reddit-style action row under a comment (Reply link).
pub const COMMENT_ACTION_H: f32 = 18.0;
pub const COMMENT_INLINE_HEADER_H: f32 = 22.0;
pub const COMMENT_INLINE_INPUT_H: f32 = 40.0;
pub const COMMENT_INLINE_FOOTER_H: f32 = 30.0;
pub const COMMENT_INLINE_ACCENT_W: f32 = 3.0;
/// Muted blue-gray guide on dark chat (Reddit dark mode). Hover brightens.
pub const COMMENT_THREAD_LINE: [f32; 4] = hex(0x343C42);
pub const COMMENT_THREAD_HOVER: [f32; 4] = hex(0x818384);
/// Inline reply composer fill — darker than the bottom top-level bar (`COMPOSER_BG`).
pub const COMMENT_INLINE_BG: [f32; 4] = hex(0x2B2D31);

/// Screen X of the vertical thread guide / collapse circle center at `depth` (0 = root).
pub fn comment_thread_guide_x(content_left: f32, depth: u32) -> f32 {
    content_left + COMMENT_INDENT * depth as f32 + COMMENT_COLLAPSE_W * 0.5
}

/// Collapse circle only when the node has nested replies.
pub fn comment_shows_collapse(child_count: usize) -> bool {
    child_count > 0
}

/// Own-row branch line: from the bottom of the collapse circle to the row bottom.
pub fn comment_own_branch_line_y(row_top: f32, row_h: f32) -> (f32, f32) {
    let circle_bottom = row_top + COMMENT_FOLD_PAD_Y + COMMENT_FOLD_RADIUS * 2.0;
    (circle_bottom, row_top + row_h)
}

/// True when `pointer_x` is close enough to click / hover a vertical guide at `guide_x`.
pub fn comment_thread_line_hit(guide_x: f32, pointer_x: f32) -> bool {
    (pointer_x - guide_x).abs() <= COMMENT_LINE_HIT_W * 0.5
}

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

// Stream watch chrome (pop-out / cycle / fullscreen).
pub const STREAM_WATCH_BTN: f32 = 36.0;
pub const STREAM_WATCH_BTN_GAP: f32 = 8.0;
pub const STREAM_WATCH_PAD: f32 = 10.0;
pub const STREAM_WATCH_ICON: f32 = 20.0;
/// Profile icon size on stream watch chrome (pop-out / fullscreen).
pub const STREAM_WATCH_AVATAR: f32 = 32.0;
pub const STREAM_WATCH_AVATAR_GAP: f32 = 8.0;
/// Mute / deafen badge on a stream-watch avatar.
pub const STREAM_WATCH_BADGE: f32 = 14.0;

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

// Screen Share / Go Live picker — cover-flow stream switcher.
pub const SHARE_PAD: f32 = 16.0;
pub const SHARE_HEADER_H: f32 = 48.0;
pub const SHARE_TAB_H: f32 = 36.0;
/// Cover-flow carousel band height on the full-bleed picker.
pub const SHARE_COVER_H: f32 = 420.0;
pub const SHARE_TILE_RADIUS: f32 = 6.0;
/// Thin light border on the focused cover-flow panel (translucent chrome).
pub const SHARE_FOCUS_BORDER: [f32; 4] = [1.0, 1.0, 1.0, 0.82];
/// Dim fill behind a panel that has no thumbnail yet.
pub const SHARE_COVER_EMPTY: [f32; 4] = with_alpha(hex(0x1A1B1E), 0.92);
/// Soft dim behind the full-bleed cover-flow (fades with presentation).
pub const SHARE_BACKDROP: [f32; 4] = [0.02, 0.03, 0.05, 0.55];
/// Bottom-middle frosted glass settings chip.
pub const SHARE_CHIP_W: f32 = 560.0;
pub const SHARE_CHIP_H_COLLAPSED: f32 = 64.0;
pub const SHARE_CHIP_H_EXPANDED: f32 = 320.0;
pub const SHARE_CHIP_PAD: f32 = 14.0;
pub const SHARE_CHIP_RADIUS: f32 = 22.0;
pub const SHARE_CHIP_MARGIN_BOTTOM: f32 = 28.0;
/// Apple-glass / acrylic approximation: cool translucent fill + rim + highlight.
pub const SHARE_GLASS_FILL: [f32; 4] = [0.78, 0.84, 0.92, 0.16];
pub const SHARE_GLASS_FILL_INNER: [f32; 4] = [0.12, 0.14, 0.18, 0.42];
pub const SHARE_GLASS_HIGHLIGHT: [f32; 4] = [1.0, 1.0, 1.0, 0.20];
pub const SHARE_GLASS_BORDER: [f32; 4] = [1.0, 1.0, 1.0, 0.32];
pub const SHARE_GLASS_NOISE: [f32; 4] = [1.0, 1.0, 1.0, 0.035];

pub const PROFILER_W: f32 = 360.0;
pub const PROFILER_H: f32 = 420.0;
pub const PROFILER_PAD: f32 = 12.0;
pub const PROFILER_MARGIN: f32 = 16.0;
/// Floating user profile — Discord-parity identity column.
pub const PROFILE_W: f32 = 340.0;
/// Board / widgets column beside the identity card.
pub const PROFILE_BOARD_W: f32 = 320.0;
/// Left editor column in the Discord-style customization layout.
pub const PROFILE_EDITOR_W: f32 = 280.0;
pub const PROFILE_BANNER_H: f32 = 105.0;
pub const PROFILE_AVATAR: f32 = 40.0;
pub const PROFILE_PAD: f32 = 16.0;
pub const PROFILE_RADIUS: f32 = 8.0;
pub const PROFILE_BODY_MIN_H: f32 = 320.0;
pub const PROFILE_SLOT: f32 = 64.0;
pub const PROFILE_NAMEPLATE_H: f32 = 48.0;
pub const PROFILE_BADGE: f32 = 18.0;
pub const PROFILE_GAME_CARD_W: f32 = 92.0;
pub const PROFILE_GAME_CARD_H: f32 = 128.0;
pub const PROFILE_GAME_THUMB: f32 = 56.0;
pub const PROFILE_SLOT_GAP: f32 = 8.0;
/// Outer inset of the customization editor frame from the display edge.
pub const PROFILE_EDITOR_OUTER_PAD: f32 = 24.0;
/// Gap between editor left / preview / board columns.
pub const PROFILE_EDITOR_COL_GAP: f32 = 16.0;
/// Save / Cancel footer band inside the customization frame.
pub const PROFILE_EDITOR_FOOTER_H: f32 = 56.0;
/// Inner pad for board widget cards.
pub const PROFILE_BOARD_CARD_PAD: f32 = 12.0;
/// Minimum width kept for the live preview column when the window is narrow.
pub const PROFILE_PREVIEW_MIN_W: f32 = 280.0;
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
    if active {
        16.0
    } else {
        RAIL_ICON * 0.5
    }
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
    if active {
        2
    } else {
        0
    }
}

/// Center of the large peer avatar while a 1:1 call is ringing, above the control bar.
pub fn call_ring_avatar_center(origin: [f32; 2], width: f32, height: f32, bar_h: f32) -> [f32; 2] {
    let usable_h = (height - bar_h).max(0.0);
    [origin[0] + width * 0.5, origin[1] + usable_h * 0.42]
}

/// The column count that gives the largest 16:9 tiles inside the stage.
pub fn voice_tile_grid(count: usize, area_w: f32, area_h: f32, gap: f32) -> TileGrid {
    let mut best = TileGrid {
        cols: 0,
        rows: 0,
        tile_w: 0.0,
        tile_h: 0.0,
    };
    if count == 0 || area_w <= 0.0 || area_h <= 0.0 {
        return best;
    }
    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let fit_w = (area_w - gap * (cols - 1) as f32) / cols as f32;
        let fit_h = (area_h - gap * (rows - 1) as f32) / rows as f32;
        let tile_w = fit_w.min(fit_h * 16.0 / 9.0);
        if tile_w > best.tile_w {
            best = TileGrid {
                cols,
                rows,
                tile_w,
                tile_h: tile_w * 9.0 / 16.0,
            };
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
        TooltipSide::Right => (
            anchor_max[0] + TOOLTIP_GAP,
            (anchor_min[1] + anchor_max[1]) * 0.5 - h * 0.5,
        ),
        TooltipSide::Top => {
            let x = (anchor_min[0] + anchor_max[0]) * 0.5 - w * 0.5;
            let above = anchor_min[1] - TOOLTIP_GAP - h;
            (
                x,
                if above < 0.0 {
                    anchor_max[1] + TOOLTIP_GAP
                } else {
                    above
                },
            )
        }
    };
    x = x.clamp(0.0, (display[0] - w).max(0.0));
    y = y.clamp(0.0, (display[1] - h).max(0.0));
    ([x, y], [x + w, y + h])
}
