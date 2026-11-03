//! User Settings rules: page layout, open and close, mic test, keybinds, and control units.
//!
//! Kept free of drawing so `tests/settings.rs` can check every rule without a window.

use dear_imgui_rs::Key;

use crate::logic::SettingsSection;

/// Discord's settings sidebar list width.
pub const SETTINGS_NAV_LIST_W: f32 = 218.0;
/// Discord caps the settings content column at 740 px.
pub const SETTINGS_CONTENT_MAX_W: f32 = 740.0;
pub const SETTINGS_CLOSE_SIZE: f32 = 36.0;
pub const SETTINGS_TOP_PAD: f32 = 60.0;
const SETTINGS_PAD: f32 = 40.0;
const SETTINGS_NARROW_PAD: f32 = 16.0;
const SETTINGS_CLOSE_GAP: f32 = 18.0;
const SETTINGS_CLOSE_COL: f32 = SETTINGS_CLOSE_GAP * 2.0 + SETTINGS_CLOSE_SIZE;
const SETTINGS_MIN_CONTENT_W: f32 = 280.0;

/// Window-space rectangles for the settings overlay. `content_x` is relative to `body_x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsLayout {
    pub nav_x: f32,
    pub nav_w: f32,
    pub body_x: f32,
    pub body_w: f32,
    pub content_x: f32,
    pub content_w: f32,
    pub close_x: f32,
    pub close_y: f32,
    pub close_size: f32,
}

/// Wide windows center the nav list, content, and close button as one group, like Discord.
/// Narrow windows shrink the content first, then the nav, so nothing overlaps or leaves the window.
pub fn settings_layout(width: f32, height: f32) -> SettingsLayout {
    let width = width.max(1.0);
    let group = SETTINGS_NAV_LIST_W + SETTINGS_PAD + SETTINGS_CONTENT_MAX_W + SETTINGS_CLOSE_COL;
    let (nav_w, pad, content_w) = if width >= group {
        let side = (width - group) * 0.5;
        (side + SETTINGS_NAV_LIST_W, SETTINGS_PAD, SETTINGS_CONTENT_MAX_W)
    } else {
        let pad = if width >= 640.0 { SETTINGS_PAD } else { SETTINGS_NARROW_PAD };
        let fixed = pad + SETTINGS_CLOSE_COL;
        let nav_w = if width - SETTINGS_NAV_LIST_W - fixed >= SETTINGS_MIN_CONTENT_W {
            SETTINGS_NAV_LIST_W
        } else {
            (width * 0.32).min(SETTINGS_NAV_LIST_W)
        };
        let content_w = (width - nav_w - fixed).clamp(1.0, SETTINGS_CONTENT_MAX_W);
        (nav_w, pad, content_w)
    };
    let close_x = nav_w + pad + content_w + SETTINGS_CLOSE_GAP;
    let close_y = SETTINGS_TOP_PAD.min((height - SETTINGS_CLOSE_SIZE).max(0.0));
    SettingsLayout {
        nav_x: 0.0,
        nav_w,
        body_x: nav_w,
        body_w: width - nav_w,
        content_x: pad,
        content_w,
        close_x,
        close_y,
        close_size: SETTINGS_CLOSE_SIZE,
    }
}

/// Open page, keybind recording, and mic test. Leaving the page or closing settings stops both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsState {
    open: bool,
    section: SettingsSection,
    recording: bool,
    mic_test: bool,
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            open: false,
            section: SettingsSection::Account,
            recording: false,
            mic_test: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn section(&self) -> SettingsSection {
        self.section
    }

    pub fn recording(&self) -> bool {
        self.recording
    }

    pub fn mic_test(&self) -> bool {
        self.mic_test
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.recording = false;
        self.mic_test = false;
    }

    pub fn select(&mut self, section: SettingsSection) {
        if section != self.section {
            self.recording = false;
            self.mic_test = false;
        }
        self.section = section;
    }

    pub fn toggle_mic_test(&mut self) {
        self.mic_test = self.open && self.section == SettingsSection::Voice && !self.mic_test;
    }

    pub fn start_recording(&mut self) {
        self.recording = self.open && self.section == SettingsSection::Voice;
    }

    pub fn finish_recording(&mut self) {
        self.recording = false;
    }

    /// Dear ImGui closes an open dropdown on Escape by itself, so that press must not also close settings.
    pub fn escape(&mut self, popup_open: bool) {
        if !self.open || popup_open {
            return;
        }
        if self.recording {
            self.recording = false;
            return;
        }
        self.close();
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Stored volume is a 0.0 to 2.0 multiplier. Discord shows it as 0% to 200%.
pub fn volume_to_percent(volume: f32) -> i32 {
    ((volume.clamp(0.0, 2.0) as f64) * 100.0).round() as i32
}

pub fn percent_to_volume(percent: i32) -> f32 {
    percent.clamp(0, 200) as f32 / 100.0
}

const FULL_SCALE_RMS: f32 = 32768.0;
const SILENCE_DB: f32 = -100.0;

/// Discord's sensitivity slider runs from -100 dB to 0 dB of full scale.
pub fn rms_to_db(rms: f32) -> f32 {
    if !rms.is_finite() || rms <= 0.0 {
        return SILENCE_DB;
    }
    (20.0 * (rms / FULL_SCALE_RMS).log10()).clamp(SILENCE_DB, 0.0)
}

pub fn db_to_rms(db: f32) -> f32 {
    FULL_SCALE_RMS * 10f32.powf(db.clamp(SILENCE_DB, 0.0) / 20.0)
}

/// Meter fill on the same dB scale as the sensitivity slider, so the threshold mark lines up.
pub fn meter_fraction(rms: f32) -> f32 {
    (rms_to_db(rms) - SILENCE_DB) / -SILENCE_DB
}

const PTT_KEYS: [(&str, Key); 48] = [
    ("A", Key::A), ("B", Key::B), ("C", Key::C), ("D", Key::D), ("E", Key::E), ("F", Key::F),
    ("G", Key::G), ("H", Key::H), ("I", Key::I), ("J", Key::J), ("K", Key::K), ("L", Key::L),
    ("M", Key::M), ("N", Key::N), ("O", Key::O), ("P", Key::P), ("Q", Key::Q), ("R", Key::R),
    ("S", Key::S), ("T", Key::T), ("U", Key::U), ("V", Key::V), ("W", Key::W), ("X", Key::X),
    ("Y", Key::Y), ("Z", Key::Z),
    ("0", Key::Key0), ("1", Key::Key1), ("2", Key::Key2), ("3", Key::Key3), ("4", Key::Key4),
    ("5", Key::Key5), ("6", Key::Key6), ("7", Key::Key7), ("8", Key::Key8), ("9", Key::Key9),
    ("F1", Key::F1), ("F2", Key::F2), ("F3", Key::F3), ("F4", Key::F4), ("F5", Key::F5), ("F6", Key::F6),
    ("F7", Key::F7), ("F8", Key::F8), ("F9", Key::F9), ("F10", Key::F10), ("F11", Key::F11), ("F12", Key::F12),
];

/// Keys that can be bound to push to talk. Escape is left out because it cancels recording.
pub fn ptt_key_names() -> Vec<&'static str> {
    PTT_KEYS.iter().map(|(name, _)| *name).collect()
}

pub fn ptt_key(name: &str) -> Option<Key> {
    PTT_KEYS.iter().find(|(known, _)| *known == name).map(|(_, key)| *key)
}

/// The saved camera name wins, then the open camera, then the first one found.
pub fn pick_camera(cameras: &[(u32, String)], saved_name: &str, current: u32) -> Option<u32> {
    if !saved_name.is_empty() {
        if let Some((index, _)) = cameras.iter().find(|(_, name)| name == saved_name) {
            return Some(*index);
        }
    }
    if cameras.iter().any(|(index, _)| *index == current) {
        return Some(current);
    }
    cameras.first().map(|(index, _)| *index)
}

/// Row number for each item when items flow left to right and wrap at `available`.
pub fn wrap_rows(widths: &[f32], available: f32, spacing: f32) -> Vec<usize> {
    let mut rows = Vec::with_capacity(widths.len());
    let mut row = 0;
    let mut used = 0.0_f32;
    for (index, width) in widths.iter().enumerate() {
        let needed = if used > 0.0 { used + spacing + width } else { *width };
        if index > 0 && used > 0.0 && needed > available {
            row += 1;
            used = *width;
        } else {
            used = needed;
        }
        rows.push(row);
    }
    rows
}

/// One line per device, shown as `device: message`.
pub fn remember_device_error(list: &mut Vec<String>, device: &str, message: &str) {
    let line = format!("{device}: {message}");
    if !list.contains(&line) {
        list.push(line);
    }
}

/// Called once the device opens, so a fixed problem stops showing.
pub fn forget_device_errors(list: &mut Vec<String>, device: &str) {
    let prefix = format!("{device}:");
    list.retain(|line| !line.starts_with(&prefix));
}

/// Space to the right of a settings slider for its value, so the knob never covers the number.
pub const SETTINGS_SLIDER_VALUE_W: f32 = 64.0;

pub fn slider_track_width(width: f32) -> f32 {
    (width - SETTINGS_SLIDER_VALUE_W).max(1.0)
}

/// Where `value` sits along a slider from `min` to `max`, from 0.0 to 1.0.
pub fn slider_fraction(value: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((value.clamp(min, max) - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

/// The part of a stored error line the user sees; the device prefix only exists for `forget_device_errors`.
pub fn device_error_text(line: &str) -> &str {
    line.split_once(": ").map_or(line, |(_, message)| message)
}

/// Plain messages for the voice-connection panel. Prefixes stay in storage for `forget_device_errors`.
pub fn voice_bar_error_lines(errors: &[String]) -> Vec<&str> {
    errors.iter().map(|line| device_error_text(line)).collect()
}

/// Screen Y for each voice-bar error, stacked upward from the bottom of the panel so lines never share a row.
pub fn voice_bar_error_ys(panel_origin_y: f32, panel_h: f32, count: usize, line_h: f32) -> Vec<f32> {
    if count == 0 || line_h <= 0.0 {
        return Vec::new();
    }
    let bottom = panel_origin_y + panel_h - line_h;
    (0..count).map(|index| bottom - index as f32 * line_h).collect()
}

/// What the Voice & Video page says when a device will not open. Library errors go to stderr instead,
/// because they name internals (for example nokhwa's "Not available on WASM") rather than what to do.
pub fn friendly_device_error(device: &str) -> Option<&'static str> {
    match device {
        "camera" => Some("Banter could not use this camera. Another app may be using it. Pick another camera or close that app."),
        "microphone" => Some("Banter could not open this microphone. Pick another input device."),
        "speakers" => Some("Banter could not open these speakers. Pick another output device."),
        _ => None,
    }
}

/// Same rule as the server: 1 to 32 characters after trimming, no control characters.
pub fn display_name_error(draft: &str) -> Option<&'static str> {
    let name = draft.trim();
    let count = name.chars().count();
    if count == 0 {
        Some("Display name can't be empty.")
    } else if count > 32 {
        Some("Display name must be 32 characters or fewer.")
    } else if name.chars().any(|c| c.is_control()) {
        Some("Display name can't contain control characters.")
    } else {
        None
    }
}

pub fn display_name_dirty(draft: &str, saved: &str) -> bool {
    draft.trim() != saved
}
