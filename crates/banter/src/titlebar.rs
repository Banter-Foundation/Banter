//! Client-drawn Mac-style window caption (BA-105).
//!
//! On Windows (and other non-Linux hosts), the OS title bar is disabled
//! (`GLFW_DECORATED` false) and Banter paints a short dark strip with traffic-light
//! controls on the left, drag-to-move on the remaining caption, and double-click to
//! maximize / restore.
//!
//! **Linux keeps the system title bar** so compositors / DEs can use their own chrome
//! (including user themes). Wayland also cannot reliably drag undecorated windows from
//! Banter, so a client caption there was empty space without working window controls.
//!
//! Geometry helpers stay drawable-free for unit tests. Platform applies
//! [`TitleBarAction`] and live drag through GLFW when the client caption is enabled.

use crate::theme::{
    TITLE_BAR_H, TITLE_BTN_GAP, TITLE_BTN_HIT_PAD, TITLE_BTN_PAD_LEFT, TITLE_BTN_R,
};

/// Max milliseconds between two caption clicks that count as a double-click.
pub const TITLE_BAR_DOUBLE_CLICK_MS: u64 = 400;
/// Max distance (px) between those clicks.
pub const TITLE_BAR_DOUBLE_CLICK_SLOP: f32 = 6.0;

/// Whether Banter draws its own caption instead of the OS title bar.
///
/// False on Linux so the window manager / compositor chrome stays in charge.
pub fn client_caption_enabled() -> bool {
    !cfg!(target_os = "linux")
}

/// Height reserved for the client caption (0 when the OS draws the title bar).
pub fn caption_height() -> f32 {
    if client_caption_enabled() {
        TITLE_BAR_H
    } else {
        0.0
    }
}

/// Chrome request from the ImGui caption to the GLFW host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarAction {
    Close,
    Minimize,
    ToggleMaximize,
}

/// Mac traffic-light order: close, minimize, maximize (left → right).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrafficLight {
    Close,
    Minimize,
    Maximize,
}

impl TrafficLight {
    pub const ALL: [TrafficLight; 3] = [
        TrafficLight::Close,
        TrafficLight::Minimize,
        TrafficLight::Maximize,
    ];

    pub const fn action(self) -> TitleBarAction {
        match self {
            Self::Close => TitleBarAction::Close,
            Self::Minimize => TitleBarAction::Minimize,
            Self::Maximize => TitleBarAction::ToggleMaximize,
        }
    }
}

/// Full-window caption strip `[x, y, w, h]` (height 0 when the OS draws the title bar).
pub fn title_bar_rect(window_w: f32) -> [f32; 4] {
    [0.0, 0.0, window_w.max(0.0), caption_height()]
}

/// Centers of the three traffic lights for a caption of height `bar_h`.
pub fn traffic_light_centers(bar_h: f32) -> [[f32; 2]; 3] {
    let y = bar_h * 0.5;
    let step = TITLE_BTN_R * 2.0 + TITLE_BTN_GAP;
    let first = TITLE_BTN_PAD_LEFT + TITLE_BTN_R;
    [
        [first, y],
        [first + step, y],
        [first + step * 2.0, y],
    ]
}

/// Which traffic light contains `pos`, if any.
pub fn traffic_light_hit(pos: [f32; 2], bar_h: f32) -> Option<TrafficLight> {
    if pos[1] < 0.0 || pos[1] > bar_h {
        return None;
    }
    let hit_r = TITLE_BTN_R + TITLE_BTN_HIT_PAD;
    let hit_r2 = hit_r * hit_r;
    for (light, center) in TrafficLight::ALL.into_iter().zip(traffic_light_centers(bar_h)) {
        let dx = pos[0] - center[0];
        let dy = pos[1] - center[1];
        if dx * dx + dy * dy <= hit_r2 {
            return Some(light);
        }
    }
    None
}

/// Caption area that starts a window drag (everything right of the lights).
pub fn title_bar_drag_rect(window_w: f32, bar_h: f32) -> [f32; 4] {
    let [_, _, max_c] = traffic_light_centers(bar_h);
    let left = max_c[0] + TITLE_BTN_R + TITLE_BTN_HIT_PAD;
    let width = (window_w - left).max(0.0);
    [left, 0.0, width, bar_h]
}

/// Whether `pos` lies inside `rect` (`[x, y, w, h]`).
pub fn point_in_rect(pos: [f32; 2], rect: [f32; 4]) -> bool {
    pos[0] >= rect[0]
        && pos[1] >= rect[1]
        && pos[0] < rect[0] + rect[2]
        && pos[1] < rect[1] + rect[3]
}

/// Next window origin so the grab point stays under the cursor.
///
/// `cursor_*` are window-relative; `grab_*` is the cursor offset recorded on press.
pub fn dragged_window_pos(
    window_x: f32,
    window_y: f32,
    cursor_x: f32,
    cursor_y: f32,
    grab_x: f32,
    grab_y: f32,
) -> (f32, f32) {
    (window_x + cursor_x - grab_x, window_y + cursor_y - grab_y)
}

/// Tracks caption double-clicks for maximize / restore.
#[derive(Clone, Debug, Default)]
pub struct DoubleClickTracker {
    last: Option<(f32, f32, u64)>,
}

impl DoubleClickTracker {
    /// Returns true when this click completes a double-click.
    pub fn note_click(&mut self, x: f32, y: f32, now_ms: u64) -> bool {
        if let Some((px, py, t)) = self.last {
            let dt = now_ms.saturating_sub(t);
            let dx = (x - px).abs();
            let dy = (y - py).abs();
            self.last = None;
            if dt <= TITLE_BAR_DOUBLE_CLICK_MS
                && dx <= TITLE_BAR_DOUBLE_CLICK_SLOP
                && dy <= TITLE_BAR_DOUBLE_CLICK_SLOP
            {
                return true;
            }
        }
        self.last = Some((x, y, now_ms));
        false
    }

    pub fn clear(&mut self) {
        self.last = None;
    }
}

/// Live caption interaction owned by the main / child surfaces.
#[derive(Clone, Debug, Default)]
pub struct TitleBarChrome {
    pub pending: Vec<TitleBarAction>,
    /// Window-relative cursor offset while dragging the caption.
    pub drag_grab: Option<(f32, f32)>,
    pub double_click: DoubleClickTracker,
    /// Soft clock for double-click (ms since process start); advanced by the UI each frame.
    pub clock_ms: u64,
}

impl TitleBarChrome {
    pub fn push(&mut self, action: TitleBarAction) {
        self.pending.push(action);
    }

    pub fn take_actions(&mut self) -> Vec<TitleBarAction> {
        std::mem::take(&mut self.pending)
    }

    pub fn begin_drag(&mut self, cursor_x: f32, cursor_y: f32) {
        self.drag_grab = Some((cursor_x, cursor_y));
    }

    pub fn end_drag(&mut self) {
        self.drag_grab = None;
    }

    pub fn tick(&mut self, delta_secs: f32) {
        let add = (delta_secs.max(0.0) * 1000.0) as u64;
        self.clock_ms = self.clock_ms.saturating_add(add);
    }
}
