//! Pure client rules: mute memory, push-to-talk, silence, resampling, and message lists.

use std::collections::{HashMap, HashSet};

use banter_server::{Message, VoiceFlags};
use serde::{Deserialize, Serialize};

/// One visible row in a Reddit-style comment tree (depth-first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentRow {
    pub message_id: i64,
    pub depth: u32,
    /// Total nested replies under this node (direct + deeper).
    pub child_count: usize,
    pub collapsed: bool,
}

/// Build children lists keyed by parent message id. Orphans (missing parent) are roots.
fn comment_children(messages: &[Message]) -> (Vec<i64>, HashMap<i64, Vec<i64>>) {
    let ids: HashSet<i64> = messages.iter().map(|m| m.id).collect();
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut roots = Vec::new();
    for message in messages {
        match message.reply_to {
            Some(parent) if ids.contains(&parent) => {
                children.entry(parent).or_default().push(message.id);
            }
            _ => roots.push(message.id),
        }
    }
    roots.sort_unstable();
    for list in children.values_mut() {
        list.sort_unstable();
    }
    (roots, children)
}

fn comment_descendant_total(children: &HashMap<i64, Vec<i64>>, id: i64, stack: &mut HashSet<i64>) -> usize {
    if !stack.insert(id) {
        return 0;
    }
    let mut total = 0;
    if let Some(kids) = children.get(&id) {
        total += kids.len();
        for kid in kids {
            total += comment_descendant_total(children, *kid, stack);
        }
    }
    stack.remove(&id);
    total
}

/// Count every nested reply under `root_id` (0 when it has none).
pub fn comment_descendant_count(messages: &[Message], root_id: i64) -> usize {
    let (_, children) = comment_children(messages);
    let mut stack = HashSet::new();
    comment_descendant_total(&children, root_id, &mut stack)
}

/// Depth-first Reddit comment rows. Collapsed nodes stay visible; their descendants do not.
pub fn comment_tree_rows(messages: &[Message], collapsed: &HashSet<i64>) -> Vec<CommentRow> {
    let (roots, children) = comment_children(messages);
    let mut rows = Vec::with_capacity(messages.len());
    let mut stack = HashSet::new();
    for root in roots {
        walk_comment_tree(root, 0, &children, collapsed, &mut stack, &mut rows);
    }
    rows
}

fn walk_comment_tree(
    id: i64,
    depth: u32,
    children: &HashMap<i64, Vec<i64>>,
    collapsed: &HashSet<i64>,
    stack: &mut HashSet<i64>,
    rows: &mut Vec<CommentRow>,
) {
    if !stack.insert(id) {
        return;
    }
    let child_count = {
        let mut inner = HashSet::new();
        comment_descendant_total(children, id, &mut inner)
    };
    let is_collapsed = collapsed.contains(&id);
    rows.push(CommentRow {
        message_id: id,
        depth,
        child_count,
        collapsed: is_collapsed,
    });
    if !is_collapsed {
        if let Some(kids) = children.get(&id) {
            for kid in kids {
                walk_comment_tree(*kid, depth.saturating_add(1), children, collapsed, stack, rows);
            }
        }
    }
    stack.remove(&id);
}

/// Collapse or expand a comment thread locally (Reddit client-side fold).
pub fn toggle_comment_collapsed(collapsed: &mut HashSet<i64>, id: i64) {
    if !collapsed.remove(&id) {
        collapsed.insert(id);
    }
}

/// Remembers mute across deafen. Deafen forces mute, then restores the earlier choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuteMemory {
    muted: bool,
    deafened: bool,
    mute_before_deafen: bool,
}

impl MuteMemory {
    pub fn new() -> Self {
        Self {
            muted: false,
            deafened: false,
            mute_before_deafen: false,
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn deafened(&self) -> bool {
        self.deafened
    }

    pub fn set_muted(&mut self, muted: bool) {
        if self.deafened {
            return;
        }
        self.muted = muted;
    }

    pub fn set_deafened(&mut self, deafened: bool) {
        if deafened == self.deafened {
            return;
        }
        if deafened {
            self.mute_before_deafen = self.muted;
            self.muted = true;
            self.deafened = true;
        } else {
            self.deafened = false;
            self.muted = self.mute_before_deafen;
        }
    }

    pub fn voice_flags(
        &self,
        push_to_talk: bool,
        transmitting: bool,
        camera: bool,
        screen: bool,
    ) -> VoiceFlags {
        VoiceFlags {
            muted: self.muted,
            deafened: self.deafened,
            push_to_talk,
            transmitting,
            camera,
            screen,
        }
    }
}

impl Default for MuteMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// The password field is masked until Show password is on. Dear ImGui draws each hidden character as `*`.
pub fn password_is_masked(show_password: bool) -> bool {
    !show_password
}

/// Dear ImGui's password flag blocks copy. The field must not use it, hidden or shown.
pub fn password_blocks_copy(_show_password: bool) -> bool {
    false
}

/// Ctrl, Shift, Alt, and Super shortcuts follow either the left or the right key.
pub fn either_key_held(left: bool, right: bool) -> bool {
    left || right
}

/// Column widths for the Discord-style shell. Left to right: server rail, channel sidebar, stage, members.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellColumns {
    pub rail_x: f32,
    pub rail_w: f32,
    pub sidebar_x: f32,
    pub sidebar_w: f32,
    pub stage_x: f32,
    pub stage_w: f32,
    pub members_x: f32,
    pub members_w: f32,
}

pub fn discord_shell_columns(width: f32, show_members: bool) -> ShellColumns {
    let width = width.max(0.0);
    let rail_w = 72.0_f32.min(width);
    let sidebar_w = 240.0_f32.min((width - rail_w).max(0.0));
    let mut members_w = if show_members { 240.0 } else { 0.0 };
    let mut stage_w = width - rail_w - sidebar_w - members_w;
    if stage_w < 0.0 {
        members_w = (members_w + stage_w).max(0.0);
        stage_w = 0.0;
    }
    let rail_x = 0.0;
    let sidebar_x = rail_x + rail_w;
    let stage_x = sidebar_x + sidebar_w;
    let members_x = stage_x + stage_w;
    ShellColumns {
        rail_x,
        rail_w,
        sidebar_x,
        sidebar_w,
        stage_x,
        stage_w,
        members_x,
        members_w,
    }
}

/// V is ignored while Dear ImGui is using the keyboard. The on-screen Talk button still counts.
pub fn push_to_talk_held(key_held: bool, button_held: bool, text_input_active: bool) -> bool {
    button_held || (key_held && !text_input_active)
}

/// Open-mic frames quieter than this RMS are not sent. Push-to-talk does not use this gate.
pub const OPEN_MIC_RMS: f32 = 180.0;

pub fn frame_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy: f64 = samples.iter().map(|sample| {
        let value = *sample as f64;
        value * value
    }).sum();
    (energy / samples.len() as f64).sqrt() as f32
}

pub fn open_mic_should_send(samples: &[i16]) -> bool {
    frame_rms(samples) >= OPEN_MIC_RMS
}

pub fn resample_mono_i16(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 || from_rate == to_rate {
        return input.to_vec();
    }
    let out_len = (input.len() as u64) * (to_rate as u64) / (from_rate as u64);
    let mut out = Vec::with_capacity(out_len as usize);
    for index in 0..out_len {
        let source = (index as f64) * (from_rate as f64) / (to_rate as f64);
        let left = source.floor() as usize;
        let fraction = (source - left as f64) as f32;
        let a = input[left.min(input.len() - 1)] as f32;
        let b = input[(left + 1).min(input.len() - 1)] as f32;
        let mixed = a + (b - a) * fraction;
        out.push(mixed.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
    }
    out
}

pub fn upsert_message(messages: &mut Vec<Message>, message: Message) {
    if let Some(slot) = messages.iter_mut().find(|existing| existing.id == message.id) {
        *slot = message;
    } else {
        messages.push(message);
        messages.sort_by_key(|existing| existing.id);
    }
}

pub fn remove_message(messages: &mut Vec<Message>, id: i64) {
    messages.retain(|message| message.id != id);
    for message in messages {
        if message.reply_to == Some(id) {
            message.reply_to = None;
        }
    }
}

pub fn encode_jpeg_rgba(width: u32, height: u32, rgba: &[u8], max_edge: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "frame is too large".to_string())?;
    if rgba.len() != expected {
        return Err("rgba size does not match the frame".into());
    }
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| "bad frame".to_string())?;
    let dynamic = image::DynamicImage::ImageRgba8(image);
    let scaled = if max_edge > 0 && (width > max_edge || height > max_edge) {
        dynamic.thumbnail(max_edge, max_edge)
    } else {
        dynamic
    };
    let mut bytes = Vec::new();
    scaled
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
        .map_err(|err| err.to_string())?;
    Ok(bytes)
}

/// 20 ms of 48 kHz mono PCM.
pub const AUDIO_FRAME_SAMPLES: usize = 960;

/// Settings pages, in the order the menu shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    Account,
    Voice,
    Developer,
}

pub fn settings_sections() -> [SettingsSection; 3] {
    [SettingsSection::Account, SettingsSection::Voice, SettingsSection::Developer]
}

/// Which Banter API the desktop client should talk to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerTarget {
    #[default]
    Local,
    Remote,
}

/// Developer server target. Lives in `dev-server.json` under the data directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevServerPrefs {
    #[serde(default)]
    pub target: ServerTarget,
    /// Host, `host:port`, or an `http(s)://` URL. Ignored while `target` is Local.
    #[serde(default)]
    pub remote_host: String,
}

impl Default for DevServerPrefs {
    fn default() -> Self {
        Self {
            target: ServerTarget::Local,
            remote_host: String::new(),
        }
    }
}

/// Local API base for the embedded / loopback server (`http://127.0.0.1:3970`).
pub fn local_api_base() -> String {
    format!("http://{}", banter_server::DEFAULT_ADDR)
}

/// Turn a typed remote host into an `http`/`https` API base with no trailing slash.
pub fn normalize_remote_base(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("enter a remote host".into());
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err("remote host cannot contain spaces".into());
    }

    let (scheme, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        ("http", rest)
    } else if trimmed.contains("://") {
        return Err("only http and https remotes are supported".into());
    } else {
        ("http", trimmed)
    };

    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Err("remote host is incomplete".into());
    }

    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if authority.is_empty() || authority.starts_with(':') {
        return Err("remote host is incomplete".into());
    }

    let host_ok = if let Some(end) = authority.strip_prefix('[').and_then(|_| authority.find(']')) {
        end > 1
    } else {
        let host = authority.split(':').next().unwrap_or("");
        !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    if !host_ok {
        return Err("remote host is incomplete".into());
    }
    if authority
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']')))
    {
        return Err("remote host has invalid characters".into());
    }

    Ok(format!("{scheme}://{authority}"))
}

/// Resolve the HTTP API base for the current Developer prefs.
pub fn api_base_for(prefs: &DevServerPrefs) -> Result<String, String> {
    match prefs.target {
        ServerTarget::Local => Ok(local_api_base()),
        ServerTarget::Remote => normalize_remote_base(&prefs.remote_host),
    }
}

/// How process startup should treat the loopback server.
///
/// The desktop window always opens. Local is only *started* when the user is already
/// logged in on Local, or when they explicitly choose Local in Developer settings.
/// A configured Remote never requires local. Fresh Local with nothing listening asks
/// for a remote instead of failing the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupLocalPolicy {
    /// Use the configured remote API; do not start or require local.
    SkipLocalUseRemote,
    /// Attempt to start the embedded local server (logged-in Local session).
    StartLocal,
    /// Use local only if something is already listening; otherwise prompt for remote.
    PreferRunningLocalElseAskRemote,
}

/// Decide whether startup should start, reuse, or skip the local server.
pub fn startup_local_policy(prefs: &DevServerPrefs, has_saved_session: bool) -> StartupLocalPolicy {
    match prefs.target {
        ServerTarget::Remote => StartupLocalPolicy::SkipLocalUseRemote,
        ServerTarget::Local if has_saved_session => StartupLocalPolicy::StartLocal,
        ServerTarget::Local => StartupLocalPolicy::PreferRunningLocalElseAskRemote,
    }
}

/// Whether the sign-in / server-options UI should open asking for a remote host.
///
/// `local_available` is true when the loopback health check succeeded (already running
/// or freshly started). Logged-in Local that failed to start still prompts rather than
/// exiting the process.
pub fn startup_should_prompt_remote(
    prefs: &DevServerPrefs,
    local_available: bool,
    _has_saved_session: bool,
) -> bool {
    match prefs.target {
        ServerTarget::Remote => api_base_for(prefs).is_err(),
        ServerTarget::Local => !local_available,
    }
}

/// API base string used while the window opens. Invalid Remote falls back to the
/// local placeholder until the user enters a usable host.
pub fn startup_api_base(prefs: &DevServerPrefs) -> String {
    api_base_for(prefs).unwrap_or_else(|_| local_api_base())
}

/// Screen-share resolutions in Discord's Go Live menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamResolution {
    P480,
    P720,
    P1080,
    P1440,
    P2160,
    Source,
}

/// Screen-share frame rates in Discord's Go Live menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFps {
    Fps15,
    Fps30,
    Fps60,
    Fps120,
    Fps144,
}

pub fn stream_resolutions() -> [StreamResolution; 6] {
    [
        StreamResolution::P480,
        StreamResolution::P720,
        StreamResolution::P1080,
        StreamResolution::P1440,
        StreamResolution::P2160,
        StreamResolution::Source,
    ]
}

pub fn stream_frame_rates() -> [StreamFps; 5] {
    [StreamFps::Fps15, StreamFps::Fps30, StreamFps::Fps60, StreamFps::Fps120, StreamFps::Fps144]
}

pub fn default_stream_resolution() -> StreamResolution {
    StreamResolution::P720
}

pub fn default_stream_fps() -> StreamFps {
    StreamFps::Fps30
}

pub fn stream_resolution_label(resolution: StreamResolution) -> &'static str {
    match resolution {
        StreamResolution::P480 => "480p",
        StreamResolution::P720 => "720p",
        StreamResolution::P1080 => "1080p",
        StreamResolution::P1440 => "1440p",
        StreamResolution::P2160 => "4K",
        StreamResolution::Source => "Source",
    }
}

pub fn stream_fps_label(fps: StreamFps) -> &'static str {
    match fps {
        StreamFps::Fps15 => "15 FPS",
        StreamFps::Fps30 => "30 FPS",
        StreamFps::Fps60 => "60 FPS",
        StreamFps::Fps120 => "120 FPS",
        StreamFps::Fps144 => "144 FPS",
    }
}

/// Longest edge of the encoded stream. Zero means the captured size, which is Source.
pub fn stream_max_edge(resolution: StreamResolution) -> u32 {
    match resolution {
        StreamResolution::P480 => 854,
        StreamResolution::P720 => 1280,
        StreamResolution::P1080 => 1920,
        StreamResolution::P1440 => 2560,
        StreamResolution::P2160 => 3840,
        StreamResolution::Source => 0,
    }
}

pub fn stream_interval_ms(fps: StreamFps) -> u64 {
    match fps {
        StreamFps::Fps15 => 66,
        StreamFps::Fps30 => 33,
        StreamFps::Fps60 => 16,
        StreamFps::Fps120 => 8,
        StreamFps::Fps144 => 7,
    }
}

/// A screen or a window, the same two tabs Discord uses for Go Live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Screen,
    Window,
}

/// Picking a source does not share it. Go Live commits the pick. Stop keeps the pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenShare {
    selected: Option<(CaptureKind, u32)>,
    live_target: Option<(CaptureKind, u32)>,
    live: bool,
    /// Wayland ScreenCast. The compositor picks the source; there is no Banter target id.
    portal: bool,
}

impl ScreenShare {
    pub fn new() -> Self {
        Self {
            selected: None,
            live_target: None,
            live: false,
            portal: false,
        }
    }

    pub fn select(&mut self, kind: CaptureKind, id: u32) {
        self.selected = Some((kind, id));
    }

    pub fn selected(&self) -> Option<(CaptureKind, u32)> {
        self.selected
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn live_target(&self) -> Option<(CaptureKind, u32)> {
        if self.live { self.live_target } else { None }
    }

    pub fn go_live(&mut self) -> bool {
        let Some(target) = self.selected else {
            return false;
        };
        self.portal = false;
        self.live_target = Some(target);
        self.live = true;
        true
    }

    /// Share through the system ScreenCast dialog. The compositor chooses the source.
    pub fn go_live_portal(&mut self) {
        self.portal = true;
        self.live = true;
        self.live_target = None;
    }

    pub fn is_portal(&self) -> bool {
        self.live && self.portal
    }

    pub fn stop(&mut self) {
        self.live = false;
        self.live_target = None;
        self.portal = false;
    }

    /// Change the live source without ending the stream. Viewers stay in the same session.
    pub fn apply_while_live(&mut self, kind: CaptureKind, id: u32) {
        self.portal = false;
        self.selected = Some((kind, id));
        if self.live {
            self.live_target = Some((kind, id));
        }
    }
}

impl Default for ScreenShare {
    fn default() -> Self {
        Self::new()
    }
}

/// Which camera is open. A new index means the device has to be opened again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraChoice {
    index: u32,
}

impl CameraChoice {
    pub fn new() -> Self {
        Self { index: 0 }
    }

    pub fn device_index(&self) -> u32 {
        self.index
    }

    pub fn select(&mut self, index: u32) {
        self.index = index;
    }

    pub fn needs_reopen(&self, opened: Option<u32>) -> bool {
        opened != Some(self.index)
    }
}

impl Default for CameraChoice {
    fn default() -> Self {
        Self::new()
    }
}

/// Discord mirrors the local preview and sends the camera the other way around.
pub fn camera_preview_mirrored() -> bool {
    true
}

pub fn mirror_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "frame is too large".to_string())?;
    if rgba.len() != expected {
        return Err("rgba size does not match the frame".into());
    }
    let mut out = vec![0; rgba.len()];
    let row = width as usize;
    for y in 0..height as usize {
        for x in 0..row {
            let src = (y * row + x) * 4;
            let dst = (y * row + (row - 1 - x)) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    Ok(out)
}

/// Output size for a capture. Resolution and frame rate are chosen separately.
pub fn stream_frame_size(resolution: StreamResolution, src_w: u32, src_h: u32) -> (u32, u32) {
    let edge = stream_max_edge(resolution);
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }
    if edge == 0 || (src_w <= edge && src_h <= edge) {
        return (src_w, src_h);
    }
    let scale = edge as f32 / src_w.max(src_h) as f32;
    let width = ((src_w as f32) * scale).round().max(1.0) as u32;
    let height = ((src_h as f32) * scale).round().max(1.0) as u32;
    (width, height)
}

pub fn resolution_block(resolution: StreamResolution, monitor_height: u32) -> Option<String> {
    if monitor_height == 0 || matches!(resolution, StreamResolution::Source) {
        return None;
    }
    let needed = match resolution {
        StreamResolution::P480 => 480,
        StreamResolution::P720 => 720,
        StreamResolution::P1080 => 1080,
        StreamResolution::P1440 => 1440,
        StreamResolution::P2160 => 2160,
        StreamResolution::Source => 0,
    };
    if monitor_height < needed {
        Some(format!("This screen is {monitor_height}p, so {needed}p is not available."))
    } else {
        None
    }
}

pub fn fps_block(fps: StreamFps, monitor_hz: u32) -> Option<String> {
    if monitor_hz == 0 {
        return None;
    }
    let wanted = match fps {
        StreamFps::Fps15 => 15,
        StreamFps::Fps30 => 30,
        StreamFps::Fps60 => 60,
        StreamFps::Fps120 => 120,
        StreamFps::Fps144 => 144,
    };
    if wanted > monitor_hz {
        Some(format!("This screen refreshes at {monitor_hz} Hz, so {wanted} FPS is not available."))
    } else {
        None
    }
}

pub fn hardware_encoder_block(found: bool) -> Option<&'static str> {
    if found {
        None
    } else {
        Some("No hardware H.264 encoder is available on this PC.")
    }
}

pub fn encode_jpeg_rgba_quality(width: u32, height: u32, rgba: &[u8], max_edge: u32, quality: u8) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "frame is too large".to_string())?;
    if rgba.len() != expected {
        return Err("rgba size does not match the frame".into());
    }
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| "bad frame".to_string())?;
    let dynamic = image::DynamicImage::ImageRgba8(image);
    let scaled = if max_edge > 0 && (width > max_edge || height > max_edge) {
        dynamic.thumbnail(max_edge, max_edge)
    } else {
        dynamic
    };
    let rgb = scaled.to_rgb8();
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality.clamp(1, 100));
    encoder
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|err| err.to_string())?;
    Ok(bytes)
}

pub fn lerp_size(from: [f32; 2], to: [f32; 2], t: f32) -> [f32; 2] {
    let t = t.clamp(0.0, 1.0);
    [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]
}

pub fn stamp_cursor(rgba: &mut [u8], width: u32, height: u32, x: i32, y: i32, show: bool) {
    if !show || width == 0 || height == 0 {
        return;
    }
    for dy in -1..=1 {
        for dx in -1..=1 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let index = ((py as u32) * width + px as u32) as usize * 4;
            if index + 3 < rgba.len() {
                rgba[index] = 255;
                rgba[index + 1] = 255;
                rgba[index + 2] = 255;
                rgba[index + 3] = 255;
            }
        }
    }
}

pub fn equal_power_gains(t: f32) -> (f32, f32) {
    let angle = t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

pub fn mix_equal_power(left: &[i16], right: &[i16], t: f32) -> Vec<i16> {
    let (gain_a, gain_b) = equal_power_gains(t);
    // Renormalize so two equal tones stay at the original level instead of summing.
    let norm = (gain_a + gain_b).max(0.001);
    let len = left.len().max(right.len());
    let mut out = Vec::with_capacity(len);
    for index in 0..len {
        let a = left.get(index).copied().unwrap_or(0) as f32 * gain_a / norm;
        let b = right.get(index).copied().unwrap_or(0) as f32 * gain_b / norm;
        out.push((a + b).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
    }
    out
}

pub fn speaking_from_level(push_to_talk: bool, held: bool, rms: f32, threshold: f32) -> bool {
    if push_to_talk {
        held
    } else {
        rms >= threshold
    }
}

/// PCM to feed the Opus encoder this capture tick.
///
/// Mute, deafen, and push-to-talk-up return `None` so the mic track stops.
/// Open-mic quiet frames still return samples: skipping them chopped words and
/// starved Opus DTX. The speaking glow keeps using `speaking_from_level`.
pub fn mic_opus_frame(flags: &VoiceFlags, samples: &[i16]) -> Option<Vec<i16>> {
    if !banter_server::should_send_audio(flags) {
        None
    } else {
        Some(samples.to_vec())
    }
}

pub fn scale_i16(samples: &[i16], gain: f32) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| ((*sample as f32) * gain).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
        .collect()
}

pub fn suppress_noise(samples: &[i16], enabled: bool, threshold: f32) -> Vec<i16> {
    if !enabled {
        return samples.to_vec();
    }
    let quiet = frame_rms(samples) < threshold;
    if !quiet {
        return samples.to_vec();
    }
    scale_i16(samples, 0.15)
}

pub fn automatic_gain(samples: &[i16], enabled: bool, target_rms: f32) -> Vec<i16> {
    if !enabled {
        return samples.to_vec();
    }
    let rms = frame_rms(samples);
    if rms < 1.0 {
        return samples.to_vec();
    }
    let gain = (target_rms / rms).clamp(0.25, 8.0);
    scale_i16(samples, gain)
}

pub fn cancel_echo(mic: &[i16], reference: &[i16], enabled: bool) -> Vec<i16> {
    if !enabled {
        return mic.to_vec();
    }
    mic.iter()
        .enumerate()
        .map(|(index, sample)| {
            let echo = reference.get(index).copied().unwrap_or(0) as f32 * 0.6;
            ((*sample as f32) - echo).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixSource {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub volume: f32,
}

/// Stream Select / stream-options grouping for checkable mix sources.
///
/// Windows WASAPI (and the Linux `loop:` / `app:` mirrors) split into system
/// devices versus per-process loopback. Several sources stay checkable so they
/// can mix into one stream-audio track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixSourceGroup {
    /// System capture and render devices (`mic:` microphones, `loop:` speaker loopback).
    AudioChannels,
    /// Per-process loopback (`app:` PID), e.g. WASAPI process loopback.
    Apps,
}

impl MixSourceGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::AudioChannels => "Audio channels",
            Self::Apps => "Apps",
        }
    }
}

/// Classify a mix source id into its Stream Select group.
pub fn mix_source_group(id: &str) -> MixSourceGroup {
    if id.starts_with("app:") {
        MixSourceGroup::Apps
    } else {
        MixSourceGroup::AudioChannels
    }
}

/// Group indices for the UI: Audio channels first, then Apps. Empty groups are omitted.
/// Relative order inside each group matches `sources`. Multi-select is unchanged.
pub fn grouped_mix_sources(sources: &[MixSource]) -> Vec<(MixSourceGroup, Vec<usize>)> {
    [
        MixSourceGroup::AudioChannels,
        MixSourceGroup::Apps,
    ]
    .into_iter()
    .filter_map(|group| {
        let indices: Vec<usize> = sources
            .iter()
            .enumerate()
            .filter(|(_, source)| mix_source_group(&source.id) == group)
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            None
        } else {
            Some((group, indices))
        }
    })
    .collect()
}

#[cfg(test)]
mod mix_source_group_tests {
    use super::{MixSource, MixSourceGroup, grouped_mix_sources, mix_source_group};

    #[test]
    fn classifies_and_groups_channel_and_app_sources() {
        assert_eq!(mix_source_group("mic:Headset"), MixSourceGroup::AudioChannels);
        assert_eq!(mix_source_group("loop:Speakers"), MixSourceGroup::AudioChannels);
        assert_eq!(mix_source_group("app:4242"), MixSourceGroup::Apps);
        assert_eq!(MixSourceGroup::AudioChannels.label(), "Audio channels");
        assert_eq!(MixSourceGroup::Apps.label(), "Apps");

        let sources = vec![
            MixSource {
                id: "mic:Mic".into(),
                name: "Mic".into(),
                enabled: true,
                volume: 1.0,
            },
            MixSource {
                id: "app:100".into(),
                name: "Game".into(),
                enabled: false,
                volume: 0.8,
            },
            MixSource {
                id: "loop:Speakers".into(),
                name: "Speakers (speakers)".into(),
                enabled: true,
                volume: 0.5,
            },
            MixSource {
                id: "app:200".into(),
                name: "Browser".into(),
                enabled: true,
                volume: 1.0,
            },
        ];
        let groups = grouped_mix_sources(&sources);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, MixSourceGroup::AudioChannels);
        assert_eq!(groups[0].1, vec![0, 2]);
        assert_eq!(groups[1].0, MixSourceGroup::Apps);
        assert_eq!(groups[1].1, vec![1, 3]);
        assert!(sources.iter().filter(|source| source.enabled).count() >= 2);
        assert!(grouped_mix_sources(&[]).is_empty());
    }
}

pub fn fade_gain(current: f32, target: f32, step: f32) -> f32 {
    if (target - current).abs() <= step {
        target
    } else if target > current {
        current + step
    } else {
        current - step
    }
}

/// Missing fields fall back to the defaults, so an older `stream.json` keeps the rest of its choices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDefaults {
    pub resolution: String,
    pub fps: String,
    pub hardware: bool,
    pub bitrate_auto: bool,
    pub bitrate_kbps: u32,
    pub show_cursor: bool,
    pub show_preview: bool,
    pub sources: Vec<MixSource>,
}

impl Default for StreamDefaults {
    fn default() -> Self {
        Self {
            resolution: "720p".into(),
            fps: "30".into(),
            hardware: false,
            bitrate_auto: true,
            bitrate_kbps: 2500,
            show_cursor: true,
            show_preview: true,
            sources: Vec::new(),
        }
    }
}

pub fn resolution_key(resolution: StreamResolution) -> &'static str {
    match resolution {
        StreamResolution::P480 => "480p",
        StreamResolution::P720 => "720p",
        StreamResolution::P1080 => "1080p",
        StreamResolution::P1440 => "1440p",
        StreamResolution::P2160 => "4k",
        StreamResolution::Source => "source",
    }
}

pub fn resolution_from_key(key: &str) -> StreamResolution {
    match key {
        "480p" => StreamResolution::P480,
        "1080p" => StreamResolution::P1080,
        "1440p" => StreamResolution::P1440,
        "4k" => StreamResolution::P2160,
        "source" => StreamResolution::Source,
        _ => StreamResolution::P720,
    }
}

pub fn fps_key(fps: StreamFps) -> &'static str {
    match fps {
        StreamFps::Fps15 => "15",
        StreamFps::Fps30 => "30",
        StreamFps::Fps60 => "60",
        StreamFps::Fps120 => "120",
        StreamFps::Fps144 => "144",
    }
}

pub fn fps_from_key(key: &str) -> StreamFps {
    match key {
        "15" => StreamFps::Fps15,
        "60" => StreamFps::Fps60,
        "120" => StreamFps::Fps120,
        "144" => StreamFps::Fps144,
        _ => StreamFps::Fps30,
    }
}

/// Window and display choices. Missing fields fall back to the defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoPrefs {
    pub vsync: bool,
    pub fullscreen: bool,
    pub width: i32,
    pub height: i32,
}

impl Default for VideoPrefs {
    fn default() -> Self {
        Self {
            vsync: true,
            fullscreen: false,
            width: 1280,
            height: 800,
        }
    }
}

/// Common windowed sizes for the Display settings combo.
pub fn window_size_presets() -> &'static [(i32, i32)] {
    &[
        (1280, 720),
        (1280, 800),
        (1600, 900),
        (1920, 1080),
        (2560, 1440),
    ]
}

pub fn window_size_label(width: i32, height: i32) -> String {
    format!("{width} × {height}")
}

pub fn clamp_window_size(width: i32, height: i32) -> (i32, i32) {
    (width.clamp(640, 7680), height.clamp(480, 4320))
}

/// Fallback UI refresh when GLFW reports no connected display or a 0 Hz mode.
pub const DEFAULT_UI_REFRESH_HZ: u32 = 60;

/// One connected display in screen coordinates, used to pick the UI refresh rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayRefresh {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub refresh_hz: u32,
}

/// Treat a missing or zero Hz report as [`DEFAULT_UI_REFRESH_HZ`].
pub fn normalize_refresh_hz(hz: u32) -> u32 {
    if hz == 0 {
        DEFAULT_UI_REFRESH_HZ
    } else {
        hz
    }
}

/// Refresh rate of the display that owns the UI window.
///
/// Fullscreen uses `fullscreen_hz` from the window's monitor video mode. Windowed mode
/// picks the display that contains the window center. If the center misses every display,
/// the first connected display wins, then [`DEFAULT_UI_REFRESH_HZ`].
pub fn pick_display_refresh_hz(
    window_center_x: i32,
    window_center_y: i32,
    fullscreen_hz: Option<u32>,
    displays: &[DisplayRefresh],
) -> u32 {
    if let Some(hz) = fullscreen_hz {
        return normalize_refresh_hz(hz);
    }
    for display in displays {
        let right = display.x.saturating_add(display.width);
        let bottom = display.y.saturating_add(display.height);
        if window_center_x >= display.x
            && window_center_y >= display.y
            && window_center_x < right
            && window_center_y < bottom
        {
            return normalize_refresh_hz(display.refresh_hz);
        }
    }
    displays
        .first()
        .map(|display| normalize_refresh_hz(display.refresh_hz))
        .unwrap_or(DEFAULT_UI_REFRESH_HZ)
}

/// True when GLFW swap interval must be reapplied for the current VSync / refresh pair.
pub fn ui_swap_needs_update(applied: Option<(bool, u32)>, vsync: bool, refresh_hz: u32) -> bool {
    applied != Some((vsync, refresh_hz))
}

/// Missing fields fall back to the defaults, so an older `voice.json` keeps the rest of its choices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoicePrefs {
    pub input_device: String,
    pub output_device: String,
    pub input_volume: f32,
    pub output_volume: f32,
    pub push_to_talk: bool,
    pub sensitivity: f32,
    pub auto_sensitivity: bool,
    pub release_ms: u32,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    /// Camera name, not index. Indexes change when cameras are plugged in or removed.
    pub camera: String,
    /// GLFW-style letter or "V". Missing older files keep V.
    #[serde(default = "default_ptt_key")]
    pub ptt_key: String,
}

fn default_ptt_key() -> String {
    "V".to_string()
}

impl Default for VoicePrefs {
    fn default() -> Self {
        Self {
            input_device: String::new(),
            output_device: String::new(),
            input_volume: 1.0,
            output_volume: 1.0,
            push_to_talk: false,
            sensitivity: OPEN_MIC_RMS,
            auto_sensitivity: true,
            release_ms: 200,
            noise_suppression: true,
            echo_cancellation: true,
            auto_gain: false,
            camera: String::new(),
            ptt_key: default_ptt_key(),
        }
    }
}

pub fn save_json<T: Serialize>(path: &std::path::Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| err.to_string())
}

pub fn load_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub const MESSAGE_GROUP_MS: i64 = 7 * 60 * 1000;

fn zoned(ms: i64, offset_secs: i32) -> chrono::DateTime<chrono::FixedOffset> {
    let offset = chrono::FixedOffset::east_opt(offset_secs).unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("utc"));
    let utc = chrono::DateTime::from_timestamp_millis(ms).unwrap_or(chrono::DateTime::UNIX_EPOCH);
    utc.with_timezone(&offset)
}

fn clock_face(ms: i64, offset_secs: i32) -> String {
    let time = zoned(ms, offset_secs);
    let hour = time.format("%I").to_string();
    let hour = hour.trim_start_matches('0');
    let hour = if hour.is_empty() { "12" } else { hour };
    format!("{hour}:{} {}", time.format("%M"), time.format("%p"))
}

fn long_date(ms: i64, offset_secs: i32) -> String {
    let time = zoned(ms, offset_secs);
    let day = time.format("%d").to_string();
    let day = day.trim_start_matches('0');
    format!("{} {day}, {}", time.format("%B"), time.format("%Y"))
}

pub fn chat_time_label(at_ms: i64, now_ms: i64, offset_secs: i32) -> String {
    let at = zoned(at_ms, offset_secs).date_naive();
    let now = zoned(now_ms, offset_secs).date_naive();
    let clock = clock_face(at_ms, offset_secs);
    if at == now {
        format!("Today at {clock}")
    } else if now.pred_opt() == Some(at) {
        format!("Yesterday at {clock}")
    } else {
        format!("{} at {clock}", long_date(at_ms, offset_secs))
    }
}

pub fn chat_time_hover(at_ms: i64, offset_secs: i32) -> String {
    let time = zoned(at_ms, offset_secs);
    format!("{}, {} at {}", time.format("%A"), long_date(at_ms, offset_secs), clock_face(at_ms, offset_secs))
}

pub fn chat_date_separator(at_ms: i64, offset_secs: i32) -> String {
    long_date(at_ms, offset_secs)
}

pub fn message_starts_new_day(previous_ms: i64, at_ms: i64, offset_secs: i32) -> bool {
    zoned(previous_ms, offset_secs).date_naive() != zoned(at_ms, offset_secs).date_naive()
}

pub fn message_is_followup(previous_author: i64, previous_ms: i64, author: i64, at_ms: i64, offset_secs: i32) -> bool {
    author == previous_author
        && at_ms >= previous_ms
        && at_ms - previous_ms <= MESSAGE_GROUP_MS
        && !message_starts_new_day(previous_ms, at_ms, offset_secs)
}

pub fn crop_avatar_rgba(width: u32, height: u32, rgba: &[u8], zoom: f32, pan_x: f32, pan_y: f32, edge: u32) -> Vec<u8> {
    let pixels = (edge as usize).saturating_mul(edge as usize).saturating_mul(4);
    let Some((origin_x, origin_y, window)) = crop_window(width, height, zoom, pan_x, pan_y) else {
        return vec![0; pixels];
    };
    if edge == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return vec![0; pixels];
    }
    let mut out = vec![0u8; pixels];
    for y in 0..edge {
        for x in 0..edge {
            let sx = origin_x + (x as f32 + 0.5) * window / edge as f32;
            let sy = origin_y + (y as f32 + 0.5) * window / edge as f32;
            let ix = (sx as u32).min(width - 1);
            let iy = (sy as u32).min(height - 1);
            let src = ((iy * width + ix) * 4) as usize;
            let dst = ((y * edge + x) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    out
}

pub fn crop_window(width: u32, height: u32, zoom: f32, pan_x: f32, pan_y: f32) -> Option<(f32, f32, f32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let zoom = zoom.max(1.0);
    let base = width.min(height) as f32;
    let window = (base / zoom).clamp(1.0, base);
    let pan_x = pan_x.clamp(-1.0, 1.0);
    let pan_y = pan_y.clamp(-1.0, 1.0);
    let slack_x = (width as f32 - window).max(0.0);
    let slack_y = (height as f32 - window).max(0.0);
    Some((slack_x * 0.5 + pan_x * slack_x * 0.5, slack_y * 0.5 + pan_y * slack_y * 0.5, window))
}


/// After a friend-add succeeds and the friends list is refreshed, return the peer
/// id whose DM should open. Pending-only requests leave the peer off the list.
pub fn open_dm_peer_after_friend_add<'a>(
    friends: impl IntoIterator<Item = (i64, &'a str)>,
    peer_username: &str,
) -> Option<i64> {
    let want = peer_username.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    friends
        .into_iter()
        .find(|(_, username)| username.eq_ignore_ascii_case(&want))
        .map(|(id, _)| id)
}

/// Discord-style right-click menu tone. Maps to the floating menu row colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuTone {
    Normal,
    Brand,
    Danger,
}

/// One row or separator in a Discord-style context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuEntry {
    Item {
        id: &'static str,
        label: &'static str,
        tone: ContextMenuTone,
        action: ContextMenuAction,
    },
    Separator,
}

/// Actions the signed-in shell can run from a context menu. Kick and ban stay deferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuAction {
    Reply { message_id: i64 },
    Edit { message_id: i64 },
    Delete { message_id: i64 },
    CopyText { text: String },
    CopyMessageId { message_id: i64 },
    CopyUsername { username: String },
    Mention { username: String },
    MessageUser { user_id: i64 },
    CallUser { user_id: i64 },
    AddFriend { username: String },
    InvitePeople { server_id: i64 },
    AssignRoles { user_id: i64 },
    CopyChannelName { name: String },
    CopyChannelId { channel_id: i64 },
}

fn menu_item(id: &'static str, label: &'static str, tone: ContextMenuTone, action: ContextMenuAction) -> ContextMenuEntry {
    ContextMenuEntry::Item { id, label, tone, action }
}

/// Right-click menu for a chat message. Own messages also get edit and delete.
pub fn message_context_menu(message_id: i64, author_id: i64, body: &str, mine: Option<i64>) -> Vec<ContextMenuEntry> {
    let mut entries = vec![menu_item(
        "ctx-reply",
        "Reply",
        ContextMenuTone::Normal,
        ContextMenuAction::Reply { message_id },
    )];
    if mine == Some(author_id) {
        entries.push(ContextMenuEntry::Separator);
        entries.push(menu_item(
            "ctx-edit",
            "Edit Message",
            ContextMenuTone::Normal,
            ContextMenuAction::Edit { message_id },
        ));
        entries.push(menu_item(
            "ctx-delete",
            "Delete Message",
            ContextMenuTone::Danger,
            ContextMenuAction::Delete { message_id },
        ));
    }
    entries.push(ContextMenuEntry::Separator);
    entries.push(menu_item(
        "ctx-copy-text",
        "Copy Text",
        ContextMenuTone::Normal,
        ContextMenuAction::CopyText { text: body.to_string() },
    ));
    entries.push(menu_item(
        "ctx-copy-msg-id",
        "Copy Message ID",
        ContextMenuTone::Normal,
        ContextMenuAction::CopyMessageId { message_id },
    ));
    entries
}

/// Right-click menu for a member, friend, or DM peer. Self gets mention and copy only.
pub fn member_context_menu(
    user_id: i64,
    username: &str,
    mine: Option<i64>,
    is_friend: bool,
    assign_roles: bool,
) -> Vec<ContextMenuEntry> {
    let mut entries = vec![menu_item(
        "ctx-mention",
        "Mention",
        ContextMenuTone::Normal,
        ContextMenuAction::Mention {
            username: username.to_string(),
        },
    )];
    if mine != Some(user_id) {
        entries.push(menu_item(
            "ctx-message",
            "Message",
            ContextMenuTone::Brand,
            ContextMenuAction::MessageUser { user_id },
        ));
        entries.push(menu_item(
            "ctx-call",
            "Call",
            ContextMenuTone::Normal,
            ContextMenuAction::CallUser { user_id },
        ));
        if !is_friend {
            entries.push(ContextMenuEntry::Separator);
            entries.push(menu_item(
                "ctx-add-friend",
                "Add Friend",
                ContextMenuTone::Normal,
                ContextMenuAction::AddFriend {
                    username: username.to_string(),
                },
            ));
        }
    }
    if assign_roles {
        entries.push(menu_item(
            "ctx-roles",
            "Roles",
            ContextMenuTone::Normal,
            ContextMenuAction::AssignRoles { user_id },
        ));
    }
    entries.push(ContextMenuEntry::Separator);
    entries.push(menu_item(
        "ctx-copy-user",
        "Copy Username",
        ContextMenuTone::Normal,
        ContextMenuAction::CopyUsername {
            username: username.to_string(),
        },
    ));
    entries
}

/// Online members with a hoisted role group under that role, highest position first.
/// Other online members go under ONLINE. Offline members go under OFFLINE even when hoisted.
pub fn plan_member_groups(rows: &[(bool, Option<(i64, String)>)]) -> Vec<(String, Vec<usize>)> {
    let mut hoist: Vec<(i64, String, Vec<usize>)> = Vec::new();
    let mut online = Vec::new();
    let mut offline = Vec::new();
    for (index, (is_online, key)) in rows.iter().enumerate() {
        if !is_online {
            offline.push(index);
            continue;
        }
        if let Some((position, name)) = key {
            if let Some(group) = hoist.iter_mut().find(|group| group.0 == *position && group.1 == *name) {
                group.2.push(index);
            } else {
                hoist.push((*position, name.clone(), vec![index]));
            }
        } else {
            online.push(index);
        }
    }
    hoist.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let mut groups = Vec::new();
    for (_, name, indexes) in hoist {
        groups.push((name.to_uppercase(), indexes));
    }
    if !online.is_empty() {
        groups.push(("ONLINE".into(), online));
    }
    if !offline.is_empty() {
        groups.push(("OFFLINE".into(), offline));
    }
    groups
}

/// Right-click menu for a text or voice channel in the sidebar.
pub fn channel_context_menu(channel_id: i64, name: &str, server_id: i64, _is_owner: bool) -> Vec<ContextMenuEntry> {
    vec![
        menu_item(
            "ctx-invite",
            "Invite People",
            ContextMenuTone::Brand,
            ContextMenuAction::InvitePeople { server_id },
        ),
        ContextMenuEntry::Separator,
        menu_item(
            "ctx-copy-channel-name",
            "Copy Name",
            ContextMenuTone::Normal,
            ContextMenuAction::CopyChannelName {
                name: name.to_string(),
            },
        ),
        menu_item(
            "ctx-copy-channel-id",
            "Copy Channel ID",
            ContextMenuTone::Normal,
            ContextMenuAction::CopyChannelId { channel_id },
        ),
    ]
}

/// Append `@username ` to the composer, matching Discord's mention insert.
pub fn insert_mention(composer: &mut String, username: &str) {
    let mention = format!("@{username}");
    if !composer.is_empty() && !composer.ends_with(' ') {
        composer.push(' ');
    }
    composer.push_str(&mention);
    composer.push(' ');
}

/// Labels only, skipping separators. Used by tests and UI smoke checks.
pub fn context_menu_labels(entries: &[ContextMenuEntry]) -> Vec<&'static str> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            ContextMenuEntry::Item { label, .. } => Some(*label),
            ContextMenuEntry::Separator => None,
        })
        .collect()
}

/// Actions only, skipping separators.
pub fn context_menu_actions(entries: &[ContextMenuEntry]) -> Vec<ContextMenuAction> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            ContextMenuEntry::Item { action, .. } => Some(action.clone()),
            ContextMenuEntry::Separator => None,
        })
        .collect()
}

/// Tone for a labeled item, if present.
pub fn context_menu_tone(entries: &[ContextMenuEntry], label: &str) -> Option<ContextMenuTone> {
    entries.iter().find_map(|entry| match entry {
        ContextMenuEntry::Item {
            label: item_label,
            tone,
            ..
        } if *item_label == label => Some(*tone),
        _ => None,
    })
}

/// Floating menu height for the Discord skin: pad, item gaps, and 9 px separators.
pub fn context_menu_height(entries: &[ContextMenuEntry]) -> f32 {
    use crate::theme::{MENU_ITEM_H, MENU_PAD};
    let mut height = MENU_PAD * 2.0;
    for entry in entries {
        match entry {
            ContextMenuEntry::Item { .. } => height += MENU_ITEM_H + 2.0,
            ContextMenuEntry::Separator => height += 9.0,
        }
    }
    height
}

#[cfg(test)]
mod member_group_tests {
    use super::plan_member_groups;

    #[test]
    fn hoists_online_members_and_keeps_offline_together() {
        let rows = vec![
            (true, Some((2, "Mods".into()))),
            (true, None),
            (false, Some((2, "Mods".into()))),
            (true, Some((5, "Admin".into()))),
            (true, Some((2, "Mods".into()))),
        ];
        let groups = plan_member_groups(&rows);
        assert_eq!(groups[0].0, "ADMIN");
        assert_eq!(groups[0].1, vec![3]);
        assert_eq!(groups[1].0, "MODS");
        assert_eq!(groups[1].1, vec![0, 4]);
        assert_eq!(groups[2].0, "ONLINE");
        assert_eq!(groups[2].1, vec![1]);
        assert_eq!(groups[3].0, "OFFLINE");
        assert_eq!(groups[3].1, vec![2]);
    }
}

/// Keep a floating menu inside the display so right-clicks near the edge still work.
pub fn clamp_context_menu_pos(pos: [f32; 2], size: [f32; 2], display: [f32; 2]) -> [f32; 2] {
    let x = pos[0].clamp(0.0, (display[0] - size[0]).max(0.0));
    let y = pos[1].clamp(0.0, (display[1] - size[1]).max(0.0));
    [x, y]
}
