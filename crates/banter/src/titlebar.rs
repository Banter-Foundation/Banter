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
//! Undecorated GLFW windows ignore `GLFW_RESIZABLE` (Win32 uses `WS_POPUP` without
//! `WS_THICKFRAME`), so Banter owns edge-resize hit testing when the client caption is
//! enabled. Geometry helpers stay drawable-free for unit tests. Platform applies
//! [`TitleBarAction`], live drag, and edge resize through GLFW when the client caption
//! is enabled.

use crate::theme::{
    TITLE_BAR_H, TITLE_BTN_GAP, TITLE_BTN_HIT_PAD, TITLE_BTN_PAD_LEFT, TITLE_BTN_R,
};

/// Max milliseconds between two caption clicks that count as a double-click.
pub const TITLE_BAR_DOUBLE_CLICK_MS: u64 = 400;
/// Max distance (px) between those clicks.
pub const TITLE_BAR_DOUBLE_CLICK_SLOP: f32 = 6.0;

/// Client-side resize hit thickness for undecorated windows (BA-111).
///
/// GLFW ignores `GLFW_RESIZABLE` when `GLFW_DECORATED` is false, so Banter must own
/// the border. Kept smaller than [`TITLE_BAR_H`] so caption drag still has room.
pub const RESIZE_BORDER_PX: f32 = 6.0;

/// Whether Banter draws its own caption instead of the OS title bar.
///
/// False on Linux so the window manager / compositor chrome stays in charge.
pub fn client_caption_enabled() -> bool {
    !cfg!(target_os = "linux")
}

/// GLFW `GLFW_DECORATED` value for Banter surfaces.
///
/// True when the client caption is off (Linux system chrome); false when Banter
/// draws traffic lights (Windows).
pub fn glfw_decorated() -> bool {
    !client_caption_enabled()
}

/// GLFW `GLFW_RESIZABLE` value for Banter surfaces.
///
/// Always true. Decorated hosts use the OS edge; undecorated hosts still request it
/// (hint is ignored there) and Banter handles resize via [`resize_edge_at`].
pub fn glfw_resizable() -> bool {
    true
}

/// Height reserved for the client caption (0 when the OS draws the title bar).
pub fn caption_height() -> f32 {
    if client_caption_enabled() {
        TITLE_BAR_H
    } else {
        0.0
    }
}

/// ImGui host rect for shell / settings / stream content below the caption.
///
/// `[x, y, w, h]` with `y == caption_height()`. Must not cover the caption strip or
/// caption drag / traffic lights lose hits (BA-111).
pub fn content_below_caption(window_w: f32, window_h: f32) -> [f32; 4] {
    let top = caption_height();
    [
        0.0,
        top,
        window_w.max(0.0),
        (window_h - top).max(0.0),
    ]
}

/// Center a windowed frame on a monitor work area (screen coordinates).
pub fn centered_window_pos(
    monitor_x: i32,
    monitor_y: i32,
    monitor_w: i32,
    monitor_h: i32,
    window_w: i32,
    window_h: i32,
) -> (i32, i32) {
    (
        monitor_x + (monitor_w - window_w) / 2,
        monitor_y + (monitor_h - window_h) / 2,
    )
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

/// Which border / corner the cursor is resizing on an undecorated window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeEdge {
    pub const fn affects_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    pub const fn affects_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    pub const fn affects_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    pub const fn affects_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
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
///
/// The top [`RESIZE_BORDER_PX`] is left for edge resize so drag and top-edge resize
/// do not fight.
pub fn title_bar_drag_rect(window_w: f32, bar_h: f32) -> [f32; 4] {
    let [_, _, max_c] = traffic_light_centers(bar_h);
    let left = max_c[0] + TITLE_BTN_R + TITLE_BTN_HIT_PAD;
    let width = (window_w - left).max(0.0);
    let top = RESIZE_BORDER_PX.min(bar_h * 0.5);
    let height = (bar_h - top).max(0.0);
    [left, top, width, height]
}

/// Which resize edge contains `pos` inside a window of size `window_w` × `window_h`.
pub fn resize_edge_at(
    pos: [f32; 2],
    window_w: f32,
    window_h: f32,
    border: f32,
) -> Option<ResizeEdge> {
    if window_w <= 0.0 || window_h <= 0.0 || border <= 0.0 {
        return None;
    }
    if pos[0] < 0.0 || pos[1] < 0.0 || pos[0] > window_w || pos[1] > window_h {
        return None;
    }
    let left = pos[0] <= border;
    let right = pos[0] >= window_w - border;
    let top = pos[1] <= border;
    let bottom = pos[1] >= window_h - border;
    match (left, right, top, bottom) {
        (true, false, true, false) => Some(ResizeEdge::TopLeft),
        (false, true, true, false) => Some(ResizeEdge::TopRight),
        (true, false, false, true) => Some(ResizeEdge::BottomLeft),
        (false, true, false, true) => Some(ResizeEdge::BottomRight),
        (true, false, false, false) => Some(ResizeEdge::Left),
        (false, true, false, false) => Some(ResizeEdge::Right),
        (false, false, true, false) => Some(ResizeEdge::Top),
        (false, false, false, true) => Some(ResizeEdge::Bottom),
        _ => None,
    }
}

/// Next window frame while dragging a resize edge.
///
/// `cursor_dx` / `cursor_dy` are deltas from the press point in screen space.
/// Only edges named by `edge` move; the opposite edge stays pinned. Dimensions are
/// clamped to `min_w` × `min_h` without moving the pinned edge past the floor.
pub fn resized_window_frame(
    edge: ResizeEdge,
    origin_x: i32,
    origin_y: i32,
    origin_w: i32,
    origin_h: i32,
    cursor_dx: f32,
    cursor_dy: f32,
    min_w: i32,
    min_h: i32,
) -> (i32, i32, i32, i32) {
    let min_w = min_w.max(1);
    let min_h = min_h.max(1);
    let dx = cursor_dx.round() as i32;
    let dy = cursor_dy.round() as i32;

    let right = origin_x + origin_w;
    let bottom = origin_y + origin_h;

    let mut x = origin_x;
    let mut y = origin_y;
    let mut w = origin_w;
    let mut h = origin_h;

    if edge.affects_left() {
        let new_x = origin_x + dx;
        let new_w = right - new_x;
        if new_w < min_w {
            x = right - min_w;
            w = min_w;
        } else {
            x = new_x;
            w = new_w;
        }
    } else if edge.affects_right() {
        w = (origin_w + dx).max(min_w);
    }

    if edge.affects_top() {
        let new_y = origin_y + dy;
        let new_h = bottom - new_y;
        if new_h < min_h {
            y = bottom - min_h;
            h = min_h;
        } else {
            y = new_y;
            h = new_h;
        }
    } else if edge.affects_bottom() {
        h = (origin_h + dy).max(min_h);
    }

    (x, y, w, h)
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

/// Active edge-resize grab for an undecorated window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeGrab {
    pub edge: ResizeEdge,
    /// Screen-space cursor at press.
    pub start_cursor_x: f64,
    pub start_cursor_y: f64,
    /// Window frame at press (screen coordinates).
    pub origin_x: i32,
    pub origin_y: i32,
    pub origin_w: i32,
    pub origin_h: i32,
}

/// Live caption interaction owned by the main / child surfaces.
#[derive(Clone, Debug, Default)]
pub struct TitleBarChrome {
    pub pending: Vec<TitleBarAction>,
    /// Window-relative cursor offset while dragging the caption.
    pub drag_grab: Option<(f32, f32)>,
    /// Edge resize while the client caption owns chrome (undecorated hosts).
    pub resize_grab: Option<ResizeGrab>,
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
        self.resize_grab = None;
        self.drag_grab = Some((cursor_x, cursor_y));
    }

    pub fn end_drag(&mut self) {
        self.drag_grab = None;
    }

    pub fn begin_resize(&mut self, grab: ResizeGrab) {
        self.drag_grab = None;
        self.resize_grab = Some(grab);
    }

    pub fn end_resize(&mut self) {
        self.resize_grab = None;
    }

    pub fn tick(&mut self, delta_secs: f32) {
        let add = (delta_secs.max(0.0) * 1000.0) as u64;
        self.clock_ms = self.clock_ms.saturating_add(add);
    }
}
