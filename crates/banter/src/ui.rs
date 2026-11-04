//! Banter window.
//!
//! The signed-in shell is a temporary copy of Discord's layout and colors so the
//! controls sit where they are familiar. It is not Banter's final look, and the
//! home mark is not Discord's logo.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use banter_server::{
    Account, CallPhase, CallSnapshot, CategoryLayout, Channel, ChannelKind, ChatTarget, DirectChat,
    ClientSignal, Friend, FriendRequest, LayerId, LayerPreferences, MAX_ATTACHMENT_BYTES, MAX_AVATAR_BYTES, MediaKind, MediaPlace, Mention,
    Message, Presence, ServerSummary, VoiceFlags, VoiceMember,
};
use dear_imgui_rs::{
    Condition, Context, DrawFlags, Key, ManagedTextureId, MouseButton, OwnedTextureData, PopupQueryFlags, Style,
    StyleColor, StyleVar, TextureFormat, TreeNodeFlags, Ui, WindowFlags, fonts::FontId,
};

use crate::api::{ApiError, Client, MemberView};
use crate::icons::rasterize_lucide;
use crate::logic::{
    CameraChoice, CaptureKind, DevServerPrefs, MixSource, MuteMemory, ScreenShare, ServerTarget, SettingsSection,
    StreamDefaults, StreamFps, StreamResolution, VideoPrefs, VoicePrefs, api_base_for, camera_preview_mirrored,
    clamp_window_size, default_stream_fps, default_stream_resolution, discord_shell_columns, encode_jpeg_rgba,
    fps_block, fps_from_key, fps_key, hardware_encoder_block, lerp_size, load_json, local_api_base, mirror_rgba,
    password_blocks_copy, password_is_masked, push_to_talk_held, remove_message, resolution_block,
    resolution_from_key, resolution_key, save_json, speaking_from_level, stream_fps_label,
    stream_frame_rates, stream_interval_ms, stream_max_edge, stream_resolution_label, stream_resolutions,
    upsert_message, chat_date_separator, chat_time_hover, chat_time_label, crop_avatar_rgba, crop_window,
    message_is_followup, message_starts_new_day, window_size_label, window_size_presets, OPEN_MIC_RMS,
};
use crate::media::{self, EngineUpdate, Media, StreamPick, VideoFrame};
use crate::profiler;
use crate::realtime::Realtime;
use crate::session::{load_token, save_token, session_path};
use crate::settings::{
    SettingsState, db_to_rms, device_error_text, display_name_dirty, display_name_error, meter_fraction,
    percent_to_volume, rms_to_db, settings_layout, slider_fraction, slider_track_width,
    voice_bar_error_lines, volume_to_percent, wrap_rows, SETTINGS_NAV_LIST_W, SETTINGS_TOP_PAD,
};
use crate::skin::{self, MenuTone, Tone};
use crate::theme::*;
use crate::websocket_url;

/// Draws the active field as `*` without Dear ImGui's password flag, so copy and paste still work.
struct StarMask;

impl StarMask {
    fn push() -> Self {
        unsafe { dear_imgui_rs::sys::igPushPasswordFont() }
        Self
    }
}

impl Drop for StarMask {
    fn drop(&mut self) {
        unsafe { dear_imgui_rs::sys::igPopPasswordFont() }
    }
}

struct Shot {
    texture: ManagedTextureId,
    size: [f32; 2],
}

struct ShareSource {
    kind: CaptureKind,
    id: u32,
    name: String,
    height: u32,
    hz: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Home,
    Server(i64),
    Direct(i64),
}

/// Discord's centered dialogs. Each one maps to an existing API call.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Modal {
    AddServer,
    CreateServer,
    JoinServer,
    Invite,
    Nickname,
    CreateChannel,
    CreateCategory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RailTone {
    Server,
    Add,
}

pub(crate) struct App {
    base: String,
    data_dir: PathBuf,
    client: Client,
    account: Option<Account>,
    status: String,
    register_mode: bool,
    username: String,
    password: String,
    show_password: bool,
    register_display: String,
    friends: Vec<Friend>,
    incoming: Vec<FriendRequest>,
    outgoing: Vec<FriendRequest>,
    servers: Vec<ServerSummary>,
    dms: Vec<DirectChat>,
    mentions: Vec<Mention>,
    presence: HashMap<i64, Presence>,
    place: Place,
    layout: Vec<CategoryLayout>,
    members: Vec<MemberView>,
    selected: Option<Channel>,
    messages: Vec<Message>,
    voice_members: Vec<VoiceMember>,
    voice_channel: Option<i64>,
    call: Option<CallSnapshot>,
    mute: MuteMemory,
    push_to_talk: bool,
    camera: bool,
    screen: bool,
    flags: VoiceFlags,
    sent_flags: VoiceFlags,
    v_held: bool,
    talk_held: bool,
    user_acted: bool,
    friend_name: String,
    server_name: String,
    invite_code: String,
    shown_invite: String,
    category_name: String,
    channel_name: String,
    channel_is_voice: bool,
    nickname: String,
    display_name: String,
    composer: String,
    search: String,
    friend_tab: u8,
    settings: SettingsState,
    settings_error: String,
    /// Local vs remote API target from User Settings → Developer.
    dev_server: DevServerPrefs,
    /// Server options panel on the sign-in / register screen.
    login_options: bool,
    /// Frames the login options panel has been open (skips the opening click for dismiss).
    login_options_frames: u32,
    /// A dropdown or text field was active at the end of the last frame, so Escape belongs to it.
    settings_popup_open: bool,
    share_open: bool,
    share: ScreenShare,
    share_tab: u8,
    share_sources: Vec<ShareSource>,
    share_thumb_next: usize,
    share_thumbs: HashMap<(u8, u32), Shot>,
    share_miss: HashSet<(u8, u32)>,
    thumb_queue: Vec<(u8, u32, Vec<u8>)>,
    thumb_tx: Sender<(u8, u32, Option<Vec<u8>>)>,
    thumb_rx: Receiver<(u8, u32, Option<Vec<u8>>)>,
    thumb_busy: bool,
    retired_shots: Vec<Shot>,
    camera_choice: CameraChoice,
    cameras: Vec<(u32, String)>,
    cameras_loaded: bool,
    stream_resolution: StreamResolution,
    stream_fps: StreamFps,
    voice: VoicePrefs,
    video: VideoPrefs,
    pending_video: Option<VideoPrefs>,
    stream: StreamDefaults,
    stream_menu: bool,
    stream_anchor: [f32; 2],
    capture_ptt: bool,
    ptt_until: Option<Instant>,
    devices_ready: bool,
    audio_inputs: Vec<String>,
    audio_outputs: Vec<String>,
    app_sources: Vec<(u32, String)>,
    screen_from: [f32; 2],
    screen_shown: [f32; 2],
    screen_target: [f32; 2],
    screen_resize_at: Instant,
    video_popout: bool,
    members_open: bool,
    bold_font: Option<FontId>,
    semibold_font: Option<FontId>,
    modal: Option<Modal>,
    modal_frames: u32,
    server_menu: bool,
    collapsed: HashSet<i64>,
    channel_category: Option<i64>,
    invite_copied: bool,
    icons: HashMap<String, Shot>,
    icons_ready: bool,
    reply_to: Option<i64>,
    editing: Option<i64>,
    pending_file: Option<(String, Vec<u8>)>,
    avatars: HashMap<i64, Shot>,
    avatar_rev: HashMap<i64, i64>,
    avatar_miss: HashMap<i64, i64>,
    crop_open: bool,
    crop_rgba: Vec<u8>,
    crop_size: [u32; 2],
    crop_zoom: f32,
    crop_pan: [f32; 2],
    crop_drag: Option<[f32; 2]>,
    crop_preview: Option<Shot>,
    crop_preview_key: [i32; 3],
    more_for: Option<i64>,
    /// Height of the drawn message history last frame; pads short histories down to the composer.
    history_h: f32,
    /// Set after sending so the list jumps to the newest message even when scrolled up.
    chat_jump: bool,
    copy_message: Option<String>,
    local_camera: Option<Shot>,
    local_screen: Option<Shot>,
    remote_video: HashMap<(i64, u8), Shot>,
    realtime: Option<Realtime>,
    media: Option<Media>,
    last_poll: Instant,
    last_activity: Instant,
    styled: bool,
    profiler_metrics: bool,
    profiler_demo: bool,
    layers: LayerPreferences,
    delivered: HashMap<(i64, MediaKind), LayerId>,
    quality_for: Option<(i64, MediaKind)>,
    quality_draft: f32,
}

impl App {
    pub(crate) fn open(base: String, data_dir: PathBuf) -> Self {
        let (thumb_tx, thumb_rx) = mpsc::channel();
        let mut client = Client::new(&base);
        let mut account = None;
        if let Some(token) = load_token(&data_dir) {
            client.set_token(Some(token));
            match client.me() {
                Ok(me) => account = Some(me),
                Err(_) => {
                    client.set_token(None);
                    let _ = std::fs::remove_file(session_path(&data_dir));
                }
            }
        }
        let mut app = Self {
            base,
            data_dir,
            client,
            account,
            status: String::new(),
            register_mode: false,
            username: String::new(),
            password: String::new(),
            show_password: false,
            register_display: String::new(),
            friends: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
            servers: Vec::new(),
            dms: Vec::new(),
            mentions: Vec::new(),
            presence: HashMap::new(),
            place: Place::Home,
            layout: Vec::new(),
            members: Vec::new(),
            selected: None,
            messages: Vec::new(),
            voice_members: Vec::new(),
            voice_channel: None,
            call: None,
            mute: MuteMemory::new(),
            push_to_talk: false,
            camera: false,
            screen: false,
            flags: VoiceFlags::default(),
            sent_flags: VoiceFlags::default(),
            v_held: false,
            talk_held: false,
            user_acted: false,
            friend_name: String::new(),
            server_name: String::new(),
            invite_code: String::new(),
            shown_invite: String::new(),
            category_name: String::new(),
            channel_name: String::new(),
            channel_is_voice: false,
            nickname: String::new(),
            display_name: String::new(),
            composer: String::new(),
            search: String::new(),
            friend_tab: 0,
            settings: SettingsState::new(),
            settings_error: String::new(),
            dev_server: DevServerPrefs::default(),
            login_options: false,
            login_options_frames: 0,
            settings_popup_open: false,
            share_open: false,
            share: ScreenShare::new(),
            share_tab: 0,
            share_sources: Vec::new(),
            share_thumb_next: 0,
            share_thumbs: HashMap::new(),
            share_miss: HashSet::new(),
            thumb_queue: Vec::new(),
            thumb_tx,
            thumb_rx,
            thumb_busy: false,
            retired_shots: Vec::new(),
            camera_choice: CameraChoice::new(),
            cameras: Vec::new(),
            cameras_loaded: false,
            stream_resolution: default_stream_resolution(),
            stream_fps: default_stream_fps(),
            voice: VoicePrefs::default(),
            video: VideoPrefs::default(),
            pending_video: None,
            stream: StreamDefaults::default(),
            stream_menu: false,
            stream_anchor: [0.0, 0.0],
            capture_ptt: false,
            ptt_until: None,
            devices_ready: false,
            audio_inputs: Vec::new(),
            audio_outputs: Vec::new(),
            app_sources: Vec::new(),
            screen_from: [0.0, 0.0],
            screen_shown: [0.0, 0.0],
            screen_target: [0.0, 0.0],
            screen_resize_at: Instant::now(),
            video_popout: false,
            members_open: true,
            bold_font: None,
            semibold_font: None,
            modal: None,
            modal_frames: 0,
            server_menu: false,
            collapsed: HashSet::new(),
            channel_category: None,
            invite_copied: false,
            icons: HashMap::new(),
            icons_ready: false,
            reply_to: None,
            editing: None,
            pending_file: None,
            avatars: HashMap::new(),
            avatar_rev: HashMap::new(),
            avatar_miss: HashMap::new(),
            crop_open: false,
            crop_rgba: Vec::new(),
            crop_size: [0, 0],
            crop_zoom: 1.0,
            crop_pan: [0.0, 0.0],
            crop_drag: None,
            crop_preview: None,
            crop_preview_key: [0, 0, 0],
            more_for: None,
            history_h: 0.0,
            chat_jump: false,
            copy_message: None,
            local_camera: None,
            local_screen: None,
            remote_video: HashMap::new(),
            realtime: None,
            media: None,
            last_poll: Instant::now(),
            last_activity: Instant::now(),
            styled: false,
            profiler_metrics: false,
            profiler_demo: false,
            layers: LayerPreferences::new(),
            delivered: HashMap::new(),
            quality_for: None,
            quality_draft: 1.0,
        };
        app.load_saved();
        if app.account.is_some() {
            app.display_name = app.account.as_ref().map(|account| account.display_name.clone()).unwrap_or_default();
            app.refresh_shell();
            app.connect_live();
        }
        app
    }

    pub(crate) fn set_bold_font(&mut self, font: Option<FontId>) {
        self.bold_font = font;
    }

    pub(crate) fn set_semibold_font(&mut self, font: Option<FontId>) {
        self.semibold_font = font;
    }

    pub(crate) fn set_v_held(&mut self, held: bool) {
        self.v_held = held;
    }

    pub(crate) fn take_video_apply(&mut self) -> Option<VideoPrefs> {
        self.pending_video.take()
    }

    pub(crate) fn note_input(&mut self) {
        self.user_acted = true;
    }

    pub(crate) fn prepare(&mut self, imgui: &mut Context) {
        if let Some(text) = self.copy_message.take() {
            imgui.set_clipboard_text(text);
        }
        if !self.styled {
            apply_discord_style(imgui.style_mut());
            self.styled = true;
        }
        self.ensure_icons(imgui);
        self.flush_shots(imgui);
        if self.account.is_none() {
            return;
        }
        self.pump_events();
        if !self.stream.show_preview {
            if let Some(shot) = self.local_screen.take() {
                let _ = imgui.remove_texture(shot.texture);
            }
        }
        let frames = self.media.as_ref().map(|media| media.take_video()).unwrap_or_default();
        for frame in frames {
            self.install_video(imgui, frame);
        }
        self.install_avatars(imgui);
        self.refresh_crop_preview(imgui);
    }

    pub(crate) fn draw(&mut self, ui: &Ui) {
        if !ui.io().want_capture_keyboard() && ui.is_key_pressed(Key::F3) {
            profiler::global().toggle();
        }
        if self.account.is_none() {
            self.draw_login(ui);
            self.draw_profiler(ui);
            return;
        }
        if self.last_poll.elapsed() >= Duration::from_secs(2) {
            self.refresh_shell();
            self.reload_messages();
            self.last_poll = Instant::now();
        }
        if self.user_acted || self.last_activity.elapsed() >= Duration::from_secs(20) {
            if let Some(realtime) = &self.realtime {
                realtime.poke_activity();
            }
            self.user_acted = false;
            self.last_activity = Instant::now();
        }
        self.draw_shell(ui);
        self.draw_server_dropdown(ui);
        self.draw_modal(ui);
        if self.settings.is_open() {
            self.draw_settings(ui);
        }
        if self.share_open {
            self.draw_share(ui);
        }
        if self.stream_menu {
            self.draw_stream_menu(ui);
        }
        self.draw_profiler(ui);
        if ui.is_key_pressed(Key::Escape) {
            if self.crop_open {
                self.crop_open = false;
            } else if self.modal.is_some() {
                self.modal = None;
            } else if self.server_menu {
                self.server_menu = false;
            } else if self.more_for.is_some() {
                self.more_for = None;
            } else if self.stream_menu {
                self.stream_menu = false;
            } else if self.share_open {
                self.share_open = false;
            } else if profiler::global().is_open() {
                profiler::global().set_open(false);
            } else if self.settings.is_open() {
                self.settings.escape(self.settings_popup_open);
            }
        }
        if self.crop_open {
            self.draw_avatar_crop(ui);
        }
        self.finish_voice_flags(ui);
    }

    fn draw_shell(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let media_stage = self.voice_selected() || self.call_stage_open();
        let show_members = self.place != Place::Home && self.members_open && !media_stage;
        let columns = discord_shell_columns(size[0], show_members);
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR;
        // Padding is read when the window begins. Pushing it inside the window
        // leaves an 8 px frame and makes the 240 px member column miss the edge.
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _child = ui.push_style_var(StyleVar::ChildBorderSize(0.0));
        let _dock = ui.push_style_var(StyleVar::DockingSeparatorSize(0.0));
        ui.window("Banter")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                // The chat header spans the stage and the member list, as in Discord.
                let body_h = (size[1] - HEADER_H).max(0.0);
                let header_bg = if media_stage { CALL_BG } else { STAGE_BG };
                self.paint_column(ui, "rail", [columns.rail_x, 0.0], [columns.rail_w, size[1]], RAIL_BG);
                self.paint_column(ui, "sidebar", [columns.sidebar_x, 0.0], [columns.sidebar_w, size[1]], SIDEBAR_BG);
                let header_w = columns.stage_w + columns.members_w;
                self.paint_column(ui, "stage-header", [columns.stage_x, 0.0], [header_w, HEADER_H], header_bg);
                self.paint_column(ui, "stage", [columns.stage_x, HEADER_H], [columns.stage_w, body_h], STAGE_BG);
                if columns.members_w > 0.0 {
                    self.paint_column(ui, "members", [columns.members_x, HEADER_H], [columns.members_w, body_h], SIDEBAR_BG);
                }
            });
    }

    fn paint_column(&mut self, ui: &Ui, name: &str, pos: [f32; 2], size: [f32; 2], color: [f32; 4]) {
        let _color = ui.push_style_color(StyleColor::ChildBg, color);
        let flags = match name {
            "rail" => WindowFlags::NO_SCROLLBAR,
            "stage-header" | "sidebar" => WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE,
            _ => WindowFlags::empty(),
        };
        ui.set_cursor_screen_pos(pos);
        ui.child_window(name).size(size).flags(flags).build(ui, || match name {
            "rail" => self.draw_rail(ui),
            "sidebar" => self.draw_sidebar_column(ui),
            "stage-header" => self.draw_stage_header(ui),
            "members" => self.draw_members(ui),
            _ => self.draw_stage(ui),
        });
    }

    fn draw_login(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        ui.window("Banter sign in")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                self.draw_login_card(ui, size);
                self.draw_login_options_button(ui, size);
            });
        if self.login_options {
            self.draw_login_options(ui, size);
        } else {
            self.login_options_frames = 0;
        }
    }

    /// Gear in the top-right of the auth screen opens Local / Remote server options.
    fn draw_login_options_button(&mut self, ui: &Ui, size: [f32; 2]) {
        let box_size = 36.0;
        let min = [size[0] - 16.0 - box_size, 16.0];
        if skin::icon_button(
            ui,
            "login-options",
            self.icon("settings"),
            min,
            box_size,
            22.0,
            WHITE,
            WHITE,
            Some(with_alpha(WHITE, 0.12)),
        ) {
            if self.login_options {
                self.login_options = false;
                self.login_options_frames = 0;
                self.settings_error.clear();
            } else {
                self.login_options = true;
                self.login_options_frames = 0;
                self.settings_error.clear();
            }
        }
        skin::item_tooltip(ui, self.semibold_font, "Server options");
    }

    fn close_login_options(&mut self) {
        self.login_options = false;
        self.login_options_frames = 0;
        self.settings_error.clear();
    }

    /// Overlay editor for the same prefs as User Settings → Developer.
    /// Drawn in its own window so controls are not fighting a full-screen dismiss hit-target.
    fn draw_login_options(&mut self, ui: &Ui, display: [f32; 2]) {
        if ui.is_key_pressed(Key::Escape) {
            self.close_login_options();
            return;
        }

        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("##login-options")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .build(|| {
                skin::rect(ui, [0.0, 0.0], display, OVERLAY, 0.0);

                let width = MODAL_W.min(display[0] - 32.0);
                let pad = MODAL_PAD;
                let body_h = 28.0 + 44.0 + 4.0 + 44.0 + 16.0 + 18.0 + 36.0 + INPUT_H + 16.0 + 18.0 + 22.0 + 16.0;
                let footer_h = MODAL_FOOTER_H;
                let title_h = 24.0 + FONT_LG + 8.0 + 16.0 + 16.0;
                let height = title_h + body_h + footer_h;
                let min = [((display[0] - width) * 0.5).floor(), ((display[1] - height) * 0.5).max(16.0).floor()];
                let max = [min[0] + width, min[1] + height];

                skin::shadow(ui, min, max, MODAL_RADIUS);
                skin::rect(ui, min, max, STAGE_BG, MODAL_RADIUS);
                skin::rect_corners(
                    ui,
                    [min[0], max[1] - footer_h],
                    max,
                    MODAL_FOOTER_BG,
                    MODAL_RADIUS,
                    DrawFlags::ROUND_BOTTOM,
                );

                let inner_x = min[0] + pad;
                let inner_w = width - pad * 2.0;
                let mut y = min[1] + 24.0;
                skin::text(ui, self.semibold_font, FONT_LG, [inner_x, y], TEXT, "Server options");
                if skin::icon_button(
                    ui,
                    "login-options-close",
                    self.icon("x"),
                    [max[0] - 16.0 - 24.0, min[1] + 16.0],
                    24.0,
                    24.0,
                    TEXT_SECONDARY,
                    TEXT_NORMAL,
                    None,
                ) {
                    self.close_login_options();
                    return;
                }
                y += FONT_LG + 8.0;
                skin::text(
                    ui,
                    None,
                    FONT_XS,
                    [inner_x, y],
                    TEXT_SECONDARY,
                    "Choose which Banter server this client talks to.",
                );
                y += 16.0 + 16.0;

                ui.set_cursor_screen_pos([inner_x, y]);
                skin::text(ui, self.bold_font, FONT_XS, [inner_x, y], MUTED, "SERVER");
                y += 18.0;
                ui.set_cursor_screen_pos([inner_x, y]);
                let local_selected = self.dev_server.target == ServerTarget::Local;
                if settings_radio_row(ui, "login-dev-server-local", "Local", local_selected, inner_w) && !local_selected {
                    self.dev_server.target = ServerTarget::Local;
                    self.apply_dev_server();
                }
                ui.set_cursor_screen_pos([inner_x, y + 48.0]);
                let remote_selected = self.dev_server.target == ServerTarget::Remote;
                if settings_radio_row(ui, "login-dev-server-remote", "Remote", remote_selected, inner_w) && !remote_selected
                {
                    self.dev_server.target = ServerTarget::Remote;
                    self.apply_dev_server();
                }
                y += 48.0 + 48.0 + 8.0;

                skin::text(ui, self.bold_font, FONT_XS, [inner_x, y], MUTED, "REMOTE HOST");
                y += 18.0;
                skin::text(
                    ui,
                    None,
                    FONT_XS,
                    [inner_x, y],
                    MUTED,
                    "Host, host:port, or http(s) URL. Example: 174.103.73.97:3970",
                );
                y += 36.0;
                let apply_label = "Apply";
                let apply_w = (skin::text_size(ui, self.semibold_font, FONT_SM, apply_label)[0] + 32.0).max(96.0);
                let field_w = (inner_w - apply_w - 8.0).max(120.0);
                let entered = skin::input(
                    ui,
                    "##login-dev-remote-host",
                    &mut self.dev_server.remote_host,
                    "host:3970",
                    [inner_x, y],
                    field_w,
                    INPUT_H,
                    FONT_MD,
                    false,
                );
                let apply_enabled = self.dev_server.target == ServerTarget::Remote;
                let apply_clicked = skin::button(
                    ui,
                    "login-dev-apply",
                    apply_label,
                    Tone::Brand,
                    [inner_x + field_w + 8.0, y],
                    [apply_w, INPUT_H],
                    apply_enabled,
                    self.semibold_font,
                );
                if (entered || apply_clicked) && apply_enabled {
                    self.apply_dev_server();
                }
                y += INPUT_H + 16.0;

                skin::text(ui, self.bold_font, FONT_XS, [inner_x, y], MUTED, "ACTIVE API BASE");
                y += 18.0;
                skin::text(ui, None, FONT_SM, [inner_x, y], TEXT_NORMAL, &self.base);

                if !self.settings_error.is_empty() {
                    let err_y = max[1] - footer_h + 12.0;
                    skin::text(ui, None, FONT_SM, [inner_x, err_y], RED, &self.settings_error);
                }

                let done_w = (skin::text_size(ui, self.semibold_font, FONT_SM, "Done")[0] + 32.0).max(96.0);
                let done_y = max[1] - footer_h + (footer_h - BUTTON_H) * 0.5;
                if skin::button(
                    ui,
                    "login-options-done",
                    "Done",
                    Tone::Brand,
                    [max[0] - pad - done_w, done_y],
                    [done_w, BUTTON_H],
                    true,
                    self.semibold_font,
                ) {
                    self.close_login_options();
                    return;
                }

                // Same dismiss rule as other modals: outside the card, after the opening frame.
                if self.login_options_frames > 0 && ui.is_mouse_clicked(MouseButton::Left) {
                    let mouse = ui.mouse_pos();
                    let inside =
                        mouse[0] >= min[0] && mouse[0] <= max[0] && mouse[1] >= min[1] && mouse[1] <= max[1];
                    if !inside {
                        self.close_login_options();
                    }
                }
            });
        if self.login_options {
            self.login_options_frames = self.login_options_frames.saturating_add(1);
        }
    }

    fn draw_login_card(&mut self, ui: &Ui, size: [f32; 2]) {
        let semi = self.semibold_font;
        let bold = self.bold_font;
        ui.get_window_draw_list().add_rect_filled_multicolor([0.0, 0.0], size, LOGIN_TOP, LOGIN_TOP, LOGIN_BOTTOM, LOGIN_BOTTOM);
        let register = self.register_mode;
        let card_w = 480.0_f32.min(size[0] - 32.0).max(320.0);
        let pad = 32.0;
        let field_h = 20.0 + INPUT_H + 20.0;
        let fields = if register { 3.0 } else { 2.0 };
        let subtitle_h = if register { 0.0 } else { 28.0 };
        let card_h = pad + 30.0 + subtitle_h + 20.0 + fields * field_h + 28.0 + 44.0 + 12.0 + 20.0 + pad;
        let min = [((size[0] - card_w) * 0.5).floor(), ((size[1] - card_h) * 0.5).max(16.0).floor()];
        let max = [min[0] + card_w, min[1] + card_h];
        skin::shadow(ui, min, max, 5.0);
        skin::rect(ui, min, max, STAGE_BG, 5.0);
        let center_x = min[0] + card_w * 0.5;
        let inner_x = min[0] + pad;
        let inner_w = card_w - pad * 2.0;
        let mut y = min[1] + pad;
        let title = if register { "Create an account" } else { "Welcome back!" };
        skin::text_centered(ui, semi, FONT_XL, center_x, y, TEXT, title);
        y += 30.0;
        if !register {
            skin::text_centered(ui, None, FONT_MD, center_x, y + 8.0, TEXT_SECONDARY, "We're so excited to see you again!");
            y += subtitle_h;
        }
        y += 20.0;
        let error = (!self.status.is_empty()).then(|| self.status.clone());
        let mut submit = false;
        if register {
            skin::field_label(ui, bold, [inner_x, y], "Display name", false, None);
            submit |= skin::input(ui, "##display", &mut self.register_display, "", [inner_x, y + 20.0], inner_w, INPUT_H, FONT_MD, false);
            y += field_h;
        }
        skin::field_label(ui, bold, [inner_x, y], "Username", true, error.as_deref());
        submit |= skin::input(ui, "##username", &mut self.username, "", [inner_x, y + 20.0], inner_w, INPUT_H, FONT_MD, false);
        y += field_h;
        skin::field_label(ui, bold, [inner_x, y], "Password", true, if register { None } else { error.as_deref() });
        {
            // The password flag hides characters, but it also disables copy.
            let masked = password_is_masked(self.show_password);
            let _block_copy = password_blocks_copy(self.show_password);
            let _mask = masked.then(StarMask::push);
            submit |= skin::input(ui, "##password", &mut self.password, "", [inner_x, y + 20.0], inner_w, INPUT_H, FONT_MD, false);
        }
        y += 20.0 + INPUT_H + 8.0;
        let check = self.icon("check");
        skin::checkbox(ui, "show-password", "Show password", &mut self.show_password, [inner_x, y], check, None);
        y += 28.0 + 12.0;
        let label = if register { "Continue" } else { "Log In" };
        if skin::button(ui, "auth-submit", label, Tone::Brand, [inner_x, y], [inner_w, 44.0], true, semi) {
            submit = true;
        }
        y += 44.0 + 12.0;
        if register {
            if skin::link(ui, "to-login", "Already have an account?", [inner_x, y], None, FONT_SM, LINK) {
                self.register_mode = false;
                self.status.clear();
            }
        } else {
            let lead = "Need an account? ";
            skin::text(ui, None, FONT_SM, [inner_x, y], MUTED, lead);
            let lead_w = skin::text_size(ui, None, FONT_SM, lead)[0];
            if skin::link(ui, "to-register", "Register", [inner_x + lead_w, y], None, FONT_SM, LINK) {
                self.register_mode = true;
                self.status.clear();
            }
        }
        if submit {
            self.submit_auth();
        }
    }

    fn submit_auth(&mut self) {
        let result = if self.register_mode {
            self.client.register(&self.username, &self.register_display, &self.password)
        } else {
            self.client.login(&self.username, &self.password)
        };
        match result {
            Ok(session) => {
                if let Err(err) = save_token(&self.data_dir, &session.token) {
                    self.status = format!("could not remember the login: {err}");
                }
                self.display_name = session.account.display_name.clone();
                self.account = Some(session.account);
                self.password.clear();
                self.status.clear();
                self.refresh_shell();
                self.connect_live();
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn draw_rail(&mut self, ui: &Ui) {
        ui.dummy([RAIL_W, RAIL_TOP]);
        let home = matches!(self.place, Place::Home | Place::Direct(_));
        let mentions = self.mentions.len();
        if self.rail_icon(ui, "home", "B", None, home, "Direct Messages", RailTone::Server, mentions) {
            self.place = Place::Home;
            self.selected = None;
            self.messages.clear();
            self.server_menu = false;
        }
        self.rail_separator(ui);
        let servers = self.servers.clone();
        for server in servers {
            let selected = self.place == Place::Server(server.id);
            let mark = server_acronym(&server.name);
            if self.rail_icon(ui, &format!("sv{}", server.id), &mark, None, selected, &server.name, RailTone::Server, 0) {
                self.server_menu = false;
                self.open_server(server.id);
            }
        }
        if self.rail_icon(ui, "add-server", "", Some("plus"), false, "Add a Server", RailTone::Add, 0) {
            self.server_name.clear();
            self.invite_code.clear();
            self.open_modal(Modal::AddServer);
        }
    }

    /// One 48 px rail button with its pill, squircle morph, tooltip, and optional mention badge.
    fn rail_icon(
        &self,
        ui: &Ui,
        id: &str,
        letters: &str,
        icon: Option<&str>,
        selected: bool,
        tip: &str,
        tone: RailTone,
        badge: usize,
    ) -> bool {
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button(id, [RAIL_W, RAIL_ICON]);
        let hovered = ui.is_item_hovered();
        let active = selected || hovered;
        let center = [origin[0] + RAIL_W * 0.5, origin[1] + RAIL_ICON * 0.5];
        let half = RAIL_ICON * 0.5;
        let min = [center[0] - half, center[1] - half];
        let max = [center[0] + half, center[1] + half];
        let (fill, ink) = match tone {
            RailTone::Server if active => (BLURPLE, WHITE),
            RailTone::Server => (STAGE_BG, TEXT_NORMAL),
            RailTone::Add if active => (GREEN, WHITE),
            RailTone::Add => (STAGE_BG, GREEN),
        };
        skin::rect(ui, min, max, fill, rail_corner_radius(active));
        let pill = rail_pill_height(selected, hovered);
        if pill > 0.0 {
            skin::rect_corners(
                ui,
                [origin[0], center[1] - pill * 0.5],
                [origin[0] + RAIL_PILL_W, center[1] + pill * 0.5],
                WHITE,
                RAIL_PILL_W,
                DrawFlags::ROUND_RIGHT,
            );
        }
        if let Some(name) = icon {
            skin::image(ui, self.icon(name), [center[0] - 12.0, center[1] - 12.0], 24.0, ink);
        } else if !letters.is_empty() {
            let size = match letters.chars().count() {
                0..=3 => FONT_MD,
                4 => FONT_SM,
                _ => FONT_XS,
            };
            let measured = skin::text_size(ui, self.semibold_font, size, letters);
            skin::text(
                ui,
                self.semibold_font,
                size,
                [center[0] - measured[0] * 0.5, center[1] - measured[1] * 0.5],
                ink,
                letters,
            );
        }
        if badge > 0 {
            let label = if badge > 99 { "99+".to_string() } else { badge.to_string() };
            let label_w = skin::text_size(ui, self.bold_font, FONT_XS, &label)[0];
            let badge_w = (label_w + 8.0).max(16.0);
            let badge_min = [max[0] + 4.0 - badge_w, max[1] - 12.0];
            let badge_max = [max[0] + 4.0, max[1] + 4.0];
            skin::rect(ui, [badge_min[0] - 3.0, badge_min[1] - 3.0], [badge_max[0] + 3.0, badge_max[1] + 3.0], RAIL_BG, 11.0);
            skin::rect(ui, badge_min, badge_max, RED, 8.0);
            skin::text_in_row(ui, self.bold_font, FONT_XS, badge_min[0] + (badge_w - label_w) * 0.5, badge_min[1], 16.0, WHITE, &label);
        }
        if hovered {
            skin::tooltip(ui, self.semibold_font, FONT_MD, tip, min, max, TooltipSide::Right);
        }
        ui.dummy([RAIL_W, RAIL_ICON_GAP]);
        clicked
    }

    fn rail_separator(&self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        ui.dummy([RAIL_W, RAIL_SEPARATOR_H + RAIL_ICON_GAP]);
        let x = origin[0] + (RAIL_W - RAIL_SEPARATOR_W) * 0.5;
        skin::rect(ui, [x, origin[1]], [x + RAIL_SEPARATOR_W, origin[1] + RAIL_SEPARATOR_H], RAIL_SEPARATOR, 1.0);
    }

    fn draw_sidebar_column(&mut self, ui: &Ui) {
        let connected = self.voice_channel.is_some() || self.call.is_some();
        let connection_h = if connected { VOICE_PANEL_H } else { 0.0 };
        let list_h = (ui.content_region_avail()[1] - HEADER_H - connection_h - USER_PANEL_H).max(40.0);
        let fixed = WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE;
        ui.child_window("sidebar-header").size([0.0, HEADER_H]).flags(fixed).build(ui, || self.draw_sidebar_header(ui));
        ui.child_window("sidebar-list").size([0.0, list_h]).build(ui, || match self.place {
            Place::Home | Place::Direct(_) => self.draw_dm_list(ui),
            Place::Server(server_id) => self.draw_channels(ui, server_id),
        });
        let _panel = ui.push_style_color(StyleColor::ChildBg, PANEL_BG);
        if connected {
            ui.child_window("sidebar-voice").size([0.0, connection_h]).flags(fixed).build(ui, || self.draw_connection(ui));
        }
        ui.child_window("sidebar-user").size([0.0, USER_PANEL_H]).flags(fixed).build(ui, || self.draw_user_bar(ui));
    }

    fn draw_sidebar_header(&mut self, ui: &Ui) {
        let width = ui.window_size()[0];
        let origin = ui.window_pos();
        match self.place {
            Place::Server(server_id) => {
                let name = self
                    .servers
                    .iter()
                    .find(|server| server.id == server_id)
                    .map(|server| server.name.clone())
                    .unwrap_or_else(|| "Server".to_string());
                ui.set_cursor_screen_pos(origin);
                if ui.invisible_button("server-header", [width, HEADER_H - 1.0]) {
                    self.server_menu = !self.server_menu;
                }
                if ui.is_item_hovered() || self.server_menu {
                    skin::rect(ui, origin, [origin[0] + width, origin[1] + HEADER_H - 1.0], ROW_HOVER, 0.0);
                }
                let shown = skin::ellipsize(ui, self.semibold_font, FONT_MD, &name, width - 16.0 - 18.0 - 16.0 - 8.0);
                skin::text_in_row(ui, self.semibold_font, FONT_MD, origin[0] + 16.0, origin[1], HEADER_H, TEXT, &shown);
                let chevron = if self.server_menu { "x" } else { "chevron-down" };
                skin::image(ui, self.icon(chevron), [origin[0] + width - 16.0 - 18.0, origin[1] + 15.0], 18.0, TEXT_NORMAL);
            }
            Place::Home | Place::Direct(_) => {
                ui.dummy([width, HEADER_H]);
                skin::text_in_row(ui, self.semibold_font, FONT_MD, origin[0] + 16.0, origin[1], HEADER_H, TEXT, "Direct Messages");
            }
        }
        header_rule(ui);
    }

    /// The server-name dropdown. It sits under the header, 10 px in from each side of the channel list.
    fn draw_server_dropdown(&mut self, ui: &Ui) {
        if !self.server_menu {
            return;
        }
        let Place::Server(server_id) = self.place else {
            self.server_menu = false;
            return;
        };
        let owner = self.servers.iter().find(|server| server.id == server_id).map(|server| server.owner_id);
        let mine = self.account.as_ref().map(|account| account.id);
        let is_owner = owner.is_some() && owner == mine;
        let items: usize = if is_owner { 4 } else { 2 };
        let separators = 1.0;
        let height = MENU_PAD * 2.0 + items as f32 * (MENU_ITEM_H + 2.0) + separators * 9.0;
        let pos = [RAIL_W + 10.0, HEADER_H + 4.0];
        let width = MENU_W;
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _round = ui.push_style_var(StyleVar::WindowRounding(MENU_RADIUS));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, FLOATING_BG);
        let mut action: Option<Modal> = None;
        ui.window("##server-menu")
            .flags(flags)
            .position(pos, Condition::Always)
            .size([width, height], Condition::Always)
            .build(|| {
                let semi = self.semibold_font;
                let x = pos[0] + MENU_PAD;
                let item_w = width - MENU_PAD * 2.0;
                let mut y = pos[1] + MENU_PAD;
                if skin::menu_item(ui, "menu-invite", "Invite People", self.icon("user-plus"), MenuTone::Brand, [x, y], item_w, semi) {
                    action = Some(Modal::Invite);
                }
                y += MENU_ITEM_H + 2.0;
                if is_owner {
                    if skin::menu_item(ui, "menu-channel", "Create Channel", self.icon("circle-plus"), MenuTone::Normal, [x, y], item_w, semi) {
                        action = Some(Modal::CreateChannel);
                    }
                    y += MENU_ITEM_H + 2.0;
                    if skin::menu_item(ui, "menu-category", "Create Category", self.icon("folder-plus"), MenuTone::Normal, [x, y], item_w, semi) {
                        action = Some(Modal::CreateCategory);
                    }
                    y += MENU_ITEM_H + 2.0;
                }
                skin::menu_separator(ui, [x, y], item_w);
                y += 9.0;
                if skin::menu_item(ui, "menu-nickname", "Change Nickname", self.icon("pencil"), MenuTone::Normal, [x, y], item_w, semi) {
                    action = Some(Modal::Nickname);
                }
            });
        if let Some(modal) = action {
            self.server_menu = false;
            match modal {
                Modal::Invite => self.open_invite(server_id),
                Modal::CreateChannel => {
                    self.channel_category = None;
                    self.channel_name.clear();
                    self.open_modal(Modal::CreateChannel);
                }
                Modal::CreateCategory => {
                    self.category_name.clear();
                    self.open_modal(Modal::CreateCategory);
                }
                Modal::Nickname => {
                    let me = self.account.as_ref().map(|account| account.id);
                    if let Some(member) = self.members.iter().find(|member| Some(member.user_id) == me) {
                        self.nickname = member.nickname.clone().unwrap_or_default();
                    }
                    self.open_modal(Modal::Nickname);
                }
                _ => {}
            }
            return;
        }
        if ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
            let mouse = ui.mouse_pos();
            let in_menu = mouse[0] >= pos[0] && mouse[0] <= pos[0] + width && mouse[1] >= pos[1] && mouse[1] <= pos[1] + height;
            let in_header = mouse[0] >= RAIL_W && mouse[0] <= RAIL_W + SIDEBAR_W && mouse[1] <= HEADER_H;
            if !in_menu && !in_header {
                self.server_menu = false;
            }
        }
    }

    fn open_modal(&mut self, modal: Modal) {
        self.modal = Some(modal);
        self.modal_frames = 0;
    }

    fn open_invite(&mut self, server_id: i64) {
        self.invite_copied = false;
        self.shown_invite.clear();
        if let Some(code) = self.note(self.client.create_invite(server_id)) {
            self.shown_invite = code;
        }
        self.open_modal(Modal::Invite);
    }

    fn draw_dm_list(&mut self, ui: &Ui) {
        ui.dummy([0.0, 8.0]);
        if self.dm_row(ui, "friends", None, "Friends", None, self.place == Place::Home) {
            self.place = Place::Home;
            self.selected = None;
            self.messages.clear();
        }
        list_label(ui, "Direct Messages", self.semibold_font);
        let dms = self.dms.clone();
        for chat in dms {
            let selected = self.place == Place::Direct(chat.id);
            let presence = self.presence.get(&chat.peer_id).copied().unwrap_or(Presence::Offline);
            if self.dm_row(ui, &format!("dm{}", chat.id), Some(chat.peer_id), &chat.peer_display_name, Some(presence), selected) {
                self.open_dm(chat.id);
            }
        }
        if !self.mentions.is_empty() {
            list_label(ui, "Mentions", self.semibold_font);
            let mentions = self.mentions.clone();
            for mention in mentions {
                let label = format!("{}: {}", mention.from_username, mention.body);
                if self.channel_row(ui, &format!("men{}", mention.id), None, &label, false) {
                    self.open_mention(&mention);
                }
            }
        }
    }

    /// A 42 px direct-message row. `user` is None for the Friends row, which shows the people icon.
    fn dm_row(&self, ui: &Ui, id: &str, user: Option<i64>, label: &str, presence: Option<Presence>, selected: bool) -> bool {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let row_h = MEMBER_ROW_H;
        let clicked = ui.invisible_button(id, [width, row_h + CHANNEL_ROW_GAP]);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + ROW_INSET, origin[1] + 1.0];
        let max = [origin[0] + width - ROW_INSET, min[1] + row_h];
        let fill = if selected {
            Some(ROW_SELECTED)
        } else if hovered {
            Some(ROW_HOVER)
        } else {
            None
        };
        if let Some(fill) = fill {
            skin::rect(ui, min, max, fill, ROW_RADIUS);
        }
        let color = if selected {
            WHITE
        } else if hovered {
            TEXT_NORMAL
        } else {
            MUTED
        };
        let avatar_center = [min[0] + 8.0 + 16.0, min[1] + row_h * 0.5];
        match user {
            Some(user_id) => {
                let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
                paint_avatar(ui, avatar, avatar_center, 16.0, &initials(label), None);
                if let Some(presence) = presence {
                    paint_status_dot(ui, avatar_center, 16.0, presence, fill.unwrap_or(SIDEBAR_BG));
                }
            }
            None => skin::image(ui, self.icon("users"), [avatar_center[0] - 12.0, avatar_center[1] - 12.0], 24.0, color),
        }
        let text_x = min[0] + 8.0 + 32.0 + 12.0;
        let shown = skin::ellipsize(ui, None, FONT_MD, label, max[0] - 8.0 - text_x);
        skin::text_in_row(ui, None, FONT_MD, text_x, min[1], row_h, color, &shown);
        clicked
    }

    fn draw_user_bar(&mut self, ui: &Ui) {
        let username = self.account.as_ref().map(|account| account.username.clone()).unwrap_or_default();
        let account_id = self.account.as_ref().map(|account| account.id);
        let presence = account_id
            .and_then(|id| self.presence.get(&id).copied())
            .unwrap_or(Presence::Online);
        let origin = ui.window_pos();
        let width = ui.window_size()[0];
        let center = [origin[0] + 8.0 + 16.0, origin[1] + USER_PANEL_H * 0.5];
        ui.set_cursor_screen_pos([center[0] - 16.0, center[1] - 16.0]);
        if ui.invisible_button("profile-avatar", [32.0, 32.0]) {
            self.open_settings();
            self.settings.select(SettingsSection::Account);
        }
        let avatar = account_id.and_then(|id| self.avatars.get(&id).map(|shot| shot.texture));
        paint_avatar(ui, avatar, center, 16.0, &initials(&self.display_name), None);
        paint_status_dot(ui, center, 16.0, presence, PANEL_BG);
        let buttons_x = origin[0] + width - 8.0 - 32.0 * 3.0;
        let name_x = origin[0] + 8.0 + 32.0 + 8.0;
        let name_w = (buttons_x - 4.0 - name_x).max(16.0);
        let name = skin::ellipsize(ui, self.semibold_font, FONT_SM, &self.display_name, name_w);
        skin::text(ui, self.semibold_font, FONT_SM, [name_x, origin[1] + 8.0], TEXT, &name);
        let handle = skin::ellipsize(ui, None, FONT_XS, &username, name_w);
        skin::text(ui, None, FONT_XS, [name_x, origin[1] + 27.0], TEXT_SECONDARY, &handle);
        let y = origin[1] + (USER_PANEL_H - 32.0) * 0.5;
        let muted = self.mute.muted();
        let (mic_tint, mic_icon) = if muted { (RED, "mic-off") } else { (TEXT_SECONDARY, "mic") };
        let mic_hover = if muted { RED } else { TEXT_NORMAL };
        if skin::icon_button(ui, "mute", self.icon(mic_icon), [buttons_x, y], 32.0, 20.0, mic_tint, mic_hover, Some(ICON_HOVER_BG)) {
            self.mute.set_muted(!muted);
        }
        skin::item_tooltip(ui, self.semibold_font, if muted { "Unmute" } else { "Mute" });
        let deafened = self.mute.deafened();
        let (deaf_tint, deaf_icon) = if deafened { (RED, "headphone-off") } else { (TEXT_SECONDARY, "headphones") };
        let deaf_hover = if deafened { RED } else { TEXT_NORMAL };
        if skin::icon_button(ui, "deafen", self.icon(deaf_icon), [buttons_x + 32.0, y], 32.0, 20.0, deaf_tint, deaf_hover, Some(ICON_HOVER_BG)) {
            self.mute.set_deafened(!deafened);
        }
        skin::item_tooltip(ui, self.semibold_font, if deafened { "Undeafen" } else { "Deafen" });
        if skin::icon_button(
            ui,
            "open-settings",
            self.icon("settings"),
            [buttons_x + 64.0, y],
            32.0,
            20.0,
            TEXT_SECONDARY,
            TEXT_NORMAL,
            Some(ICON_HOVER_BG),
        ) {
            self.open_settings();
        }
        skin::item_tooltip(ui, self.semibold_font, "User Settings");
    }

    /// The Voice Connected panel above the user panel: status, place, disconnect, then Video and Screen.
    fn draw_connection(&mut self, ui: &Ui) {
        let origin = ui.window_pos();
        let width = ui.window_size()[0];
        let semi = self.semibold_font;
        skin::line_h(ui, origin[0] + 8.0, origin[0] + width - 8.0, origin[1] + VOICE_PANEL_H - 1.0, LIST_DIVIDER);
        let me = self.account.as_ref().map(|account| account.id);
        let call = self.call.clone();
        let (title, title_color, subtitle) = if let Some(channel_id) = self.voice_channel {
            let channel = self.find_channel(channel_id);
            let name = channel.as_ref().map(|channel| channel.name.clone()).unwrap_or_else(|| format!("channel {channel_id}"));
            let server = channel
                .as_ref()
                .and_then(|channel| self.servers.iter().find(|server| server.id == channel.server_id))
                .map(|server| server.name.clone())
                .unwrap_or_default();
            let subtitle = if server.is_empty() { name } else { format!("{name} / {server}") };
            ("Voice Connected", GREEN, subtitle)
        } else if let Some(call) = &call {
            let peer = if Some(call.caller) == me { call.callee } else { call.caller };
            match call.phase {
                CallPhase::Ringing if Some(call.callee) == me => ("Incoming Call", GREEN, self.name_of(peer)),
                CallPhase::Ringing => ("Calling…", YELLOW, self.name_of(peer)),
                CallPhase::Active => ("Voice Connected", GREEN, self.name_of(peer)),
            }
        } else {
            return;
        };
        // One plain device error replaces the place name, like Discord. Settings still lists every error.
        let media_errors = self.media.as_ref().map(|media| media.errors()).unwrap_or_default();
        let voice_errors = voice_bar_error_lines(&media_errors);
        let error = media_errors.first().cloned();
        skin::image(ui, self.icon("signal"), [origin[0] + 8.0, origin[1] + 12.0], 16.0, title_color);
        skin::text(ui, semi, FONT_SM, [origin[0] + 8.0 + 16.0 + 6.0, origin[1] + 10.0], title_color, title);
        let sub_w = width - 8.0 - 8.0 - 40.0;
        let (sub_text, sub_color) = match voice_errors.first() {
            Some(short) => ((*short).to_string(), RED),
            None => (subtitle, TEXT_SECONDARY),
        };
        let sub_text = skin::ellipsize(ui, None, FONT_XS, &sub_text, sub_w);
        skin::text(ui, None, FONT_XS, [origin[0] + 8.0, origin[1] + 30.0], sub_color, &sub_text);
        if let Some(error) = &error {
            ui.set_cursor_screen_pos([origin[0] + 8.0, origin[1] + 28.0]);
            ui.invisible_button("connection-error", [sub_w, 16.0]);
            let detail = skin::wrap_lines(ui, None, FONT_SM, device_error_text(error), 360.0).join("\n");
            skin::item_tooltip(ui, None, &detail);
        }
        let end_tip = match &call {
            Some(call) if call.phase == CallPhase::Ringing && Some(call.callee) != me => "Cancel Call",
            Some(call) if call.phase == CallPhase::Ringing => "Decline",
            _ => "Disconnect",
        };
        if skin::icon_button(
            ui,
            "disconnect",
            self.icon("phone-off"),
            [origin[0] + width - 8.0 - 32.0, origin[1] + 8.0],
            32.0,
            20.0,
            TEXT_NORMAL,
            WHITE,
            Some(ICON_HOVER_BG),
        ) {
            self.end_voice_or_call();
        }
        skin::item_tooltip(ui, semi, end_tip);
        let row_y = origin[1] + 52.0;
        let row_w = width - 16.0;
        if let Some(call) = &call {
            if call.phase == CallPhase::Ringing {
                if Some(call.callee) == me {
                    let half = (row_w - 8.0) * 0.5;
                    if skin::button(ui, "accept-call", "Accept", Tone::Green, [origin[0] + 8.0, row_y], [half, 32.0], true, semi) {
                        self.accept_call(call.dm_id);
                    }
                    if skin::button(ui, "decline-call", "Decline", Tone::Danger, [origin[0] + 16.0 + half, row_y], [half, 32.0], true, semi) {
                        if self.note(self.client.decline_call(call.dm_id)).is_some() {
                            self.end_media_place();
                        }
                    }
                } else {
                    ui.set_cursor_screen_pos([origin[0], row_y]);
                    ui.dummy([width, 32.0]);
                }
                self.talk_held = false;
                return;
            }
        }
        let slots = if self.push_to_talk { 3.0 } else { 2.0 };
        let slot_w = (row_w - 8.0 * (slots - 1.0)) / slots;
        let mut x = origin[0] + 8.0;
        let camera = self.camera;
        if self.panel_button(ui, "camera-toggle", "video", [x, row_y], slot_w, camera) {
            self.camera = !self.camera;
        }
        skin::item_tooltip(ui, semi, if camera { "Turn Off Camera" } else { "Turn On Camera" });
        x += slot_w + 8.0;
        let live = self.share.is_live();
        let share_clicked = self.panel_button(ui, "screen-toggle", "monitor", [x, row_y], slot_w, live);
        let share_custom = ui.is_item_clicked_with_button(dear_imgui_rs::MouseButton::Right) && ui.io().key_shift();
        if share_clicked || share_custom {
            self.on_share_click(share_clicked, share_custom);
        }
        skin::item_tooltip(ui, semi, share_tip(live));
        x += slot_w + 8.0;
        if self.push_to_talk {
            ui.set_cursor_screen_pos([x, row_y]);
            ui.invisible_button("talk", [slot_w, 32.0]);
            self.talk_held = ui.is_item_active();
            let fill = if self.talk_held { GREEN_BUTTON } else if ui.is_item_hovered() { CONTROL_HOVER } else { CONTROL_BG };
            skin::rect(ui, [x, row_y], [x + slot_w, row_y + 32.0], fill, 4.0);
            let label_w = skin::text_size(ui, semi, FONT_SM, "Talk")[0];
            skin::text_in_row(ui, semi, FONT_SM, x + (slot_w - label_w) * 0.5, row_y, 32.0, TEXT_NORMAL, "Talk");
        } else {
            self.talk_held = false;
        }
        self.video_popout = false;
    }

    fn panel_button(&self, ui: &Ui, id: &str, icon: &str, min: [f32; 2], width: f32, active: bool) -> bool {
        ui.set_cursor_screen_pos(min);
        let clicked = ui.invisible_button(id, [width, 32.0]);
        let fill = if active {
            ROW_SELECTED
        } else if ui.is_item_hovered() {
            CONTROL_HOVER
        } else {
            CONTROL_BG
        };
        skin::rect(ui, min, [min[0] + width, min[1] + 32.0], fill, 4.0);
        let tint = if active { WHITE } else { TEXT_NORMAL };
        skin::image(ui, self.icon(icon), [min[0] + (width - 20.0) * 0.5, min[1] + 6.0], 20.0, tint);
        clicked
    }

    fn end_voice_or_call(&mut self) {
        if self.voice_channel.is_some() {
            self.leave_voice();
            return;
        }
        let me = self.account.as_ref().map(|account| account.id);
        let Some(call) = self.call.clone() else { return };
        let ended = if call.phase == CallPhase::Ringing && Some(call.callee) == me {
            self.note(self.client.decline_call(call.dm_id)).is_some()
        } else {
            self.note(self.client.end_call(call.dm_id)).is_some()
        };
        if ended {
            self.end_media_place();
        }
    }

    fn start_call(&mut self, dm_id: i64) {
        if let Some(call) = self.note(self.client.start_call(dm_id)) {
            self.call = Some(call);
            self.voice_channel = None;
            self.voice_members.clear();
            // Keep (or open) the DM so the center stage can show the 1:1 call view.
            if !matches!(self.place, Place::Direct(id) if id == dm_id) {
                self.open_dm(dm_id);
            }
        }
    }

    fn accept_call(&mut self, dm_id: i64) {
        if let Some(call) = self.note(self.client.accept_call(dm_id)) {
            self.call = Some(call);
            self.voice_channel = None;
            self.voice_members.clear();
            if !matches!(self.place, Place::Direct(id) if id == dm_id) {
                self.open_dm(dm_id);
            }
        }
    }

    /// The 48 px bar over the stage and the member list.
    fn draw_stage_header(&mut self, ui: &Ui) {
        let width = ui.window_size()[0];
        let origin = ui.window_pos();
        let semi = self.semibold_font;
        let mid = origin[1] + HEADER_H * 0.5;
        ui.dummy([width, HEADER_H]);
        let mut x = origin[0] + 16.0;
        match self.place {
            Place::Home => {
                skin::image(ui, self.icon("users"), [x, mid - 12.0], 24.0, CHANNEL_ICON);
                x += 24.0 + 8.0;
                skin::text_in_row(ui, semi, FONT_MD, x, origin[1], HEADER_H, TEXT, "Friends");
                x += skin::text_size(ui, semi, FONT_MD, "Friends")[0] + 16.0;
                skin::rect(ui, [x, mid - 12.0], [x + 1.0, mid + 12.0], DIVIDER, 0.0);
                self.draw_friend_tabs(ui, x + 1.0 + 16.0, mid);
            }
            Place::Direct(dm_id) => {
                if let Some(chat) = self.dms.iter().find(|chat| chat.id == dm_id).cloned() {
                    let center = [x + 12.0, mid];
                    let avatar = self.avatars.get(&chat.peer_id).map(|shot| shot.texture);
                    paint_avatar(ui, avatar, center, 12.0, &initials(&chat.peer_display_name), None);
                    let presence = self.presence.get(&chat.peer_id).copied().unwrap_or(Presence::Offline);
                    paint_status_dot(ui, center, 12.0, presence, STAGE_BG);
                    x += 24.0 + 8.0;
                    skin::text_in_row(ui, semi, FONT_MD, x, origin[1], HEADER_H, TEXT, &chat.peer_display_name);
                }
            }
            Place::Server(_) => {
                if let Some(name) = self.stage_icon() {
                    skin::image(ui, self.icon(name), [x, mid - 12.0], 24.0, CHANNEL_ICON);
                    x += 24.0 + 8.0;
                }
                let title = self.stage_title();
                skin::text_in_row(ui, semi, FONT_MD, x, origin[1], HEADER_H, TEXT, &title);
            }
        }
        let mut right = origin[0] + width - 16.0;
        let media_stage = self.voice_selected() || self.call_stage_open();
        if self.chat_target().is_some() && !media_stage {
            right -= SEARCH_W;
            self.draw_header_search(ui, [right, mid - SEARCH_H * 0.5]);
            right -= HEADER_ICON_GAP;
        }
        if self.place != Place::Home && !media_stage {
            right -= HEADER_ICON;
            let open = self.members_open;
            let tint = if open { TEXT } else { TEXT_SECONDARY };
            if skin::icon_button(ui, "members-toggle", self.icon("users"), [right, mid - 12.0], HEADER_ICON, HEADER_ICON, tint, TEXT_NORMAL, None) {
                self.members_open = !self.members_open;
            }
            let tip = match (self.place, open) {
                (Place::Direct(_), true) => "Hide User Profile",
                (Place::Direct(_), false) => "Show User Profile",
                (_, true) => "Hide Member List",
                (_, false) => "Show Member List",
            };
            skin::item_tooltip(ui, semi, tip);
            right -= HEADER_ICON_GAP;
        }
        if let Place::Direct(dm_id) = self.place {
            if self.call.is_none() {
                right -= HEADER_ICON;
                if skin::icon_button(ui, "start-call", self.icon("phone"), [right, mid - 12.0], HEADER_ICON, HEADER_ICON, TEXT_SECONDARY, TEXT_NORMAL, None) {
                    self.start_call(dm_id);
                }
                skin::item_tooltip(ui, semi, "Start Voice Call");
            }
        }
        header_rule(ui);
    }

    fn draw_header_search(&mut self, ui: &Ui, min: [f32; 2]) {
        let submit = skin::input(ui, "##search", &mut self.search, "Search", min, SEARCH_W, SEARCH_H, FONT_SM, false);
        skin::image(ui, self.icon("search"), [min[0] + SEARCH_W - 20.0, min[1] + 4.0], 16.0, TEXT_SECONDARY);
        if submit {
            self.reload_messages();
        }
    }

    fn draw_friend_tabs(&mut self, ui: &Ui, start_x: f32, mid: f32) {
        let labels = ["Online", "All", "Pending", "Add Friend"];
        let pending = self.incoming.len();
        let mut x = start_x;
        for (index, label) in labels.iter().enumerate() {
            let selected = self.friend_tab == index as u8;
            let label_w = skin::text_size(ui, None, FONT_MD, label)[0];
            let badge = index == 2 && pending > 0;
            let tab_w = label_w + 16.0 + if badge { 20.0 } else { 0.0 };
            let min = [x, mid - 12.0];
            let max = [x + tab_w, mid + 12.0];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("friend-tab-{index}"), [tab_w, 24.0]) {
                self.friend_tab = index as u8;
            }
            let hovered = ui.is_item_hovered();
            let (fill, color) = if index == 3 {
                if selected { (None, GREEN_TEXT) } else { (Some(GREEN_BUTTON), WHITE) }
            } else if selected {
                (Some(ROW_SELECTED), WHITE)
            } else if hovered {
                (Some(ROW_HOVER), TEXT_NORMAL)
            } else {
                (None, TEXT_SECONDARY)
            };
            if let Some(fill) = fill {
                skin::rect(ui, min, max, fill, 4.0);
            }
            skin::text_in_row(ui, None, FONT_MD, x + 8.0, min[1], 24.0, color, label);
            if badge {
                let count = pending.to_string();
                let bx = x + 8.0 + label_w + 4.0;
                skin::rect(ui, [bx, mid - 8.0], [bx + 16.0, mid + 8.0], RED, 8.0);
                let count_w = skin::text_size(ui, self.bold_font, FONT_XS, &count)[0];
                skin::text_in_row(ui, self.bold_font, FONT_XS, bx + (16.0 - count_w) * 0.5, mid - 8.0, 16.0, WHITE, &count);
            }
            x += tab_w + 16.0;
        }
    }

    fn draw_stage(&mut self, ui: &Ui) {
        if !self.status.is_empty() {
            self.draw_notice(ui);
        }
        if self.call_stage_open() {
            self.draw_call_stage(ui);
        } else if self.voice_selected() {
            self.draw_voice_stage(ui);
        } else if self.place == Place::Home {
            self.draw_people(ui);
        } else {
            self.draw_chat(ui);
        }
    }

    /// Discord's red notice strip for an error, with a close button.
    fn draw_notice(&mut self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let height = 36.0;
        ui.dummy([width, height]);
        skin::rect(ui, origin, [origin[0] + width, origin[1] + height], NOTICE_BG, 0.0);
        let message = skin::ellipsize(ui, self.semibold_font, FONT_SM, &self.status, width - 80.0);
        let message_w = skin::text_size(ui, self.semibold_font, FONT_SM, &message)[0];
        skin::text_in_row(ui, self.semibold_font, FONT_SM, origin[0] + (width - message_w) * 0.5, origin[1], height, WHITE, &message);
        if skin::icon_button(ui, "dismiss-notice", self.icon("x"), [origin[0] + width - 8.0 - 24.0, origin[1] + 6.0], 24.0, 16.0, WHITE, WHITE, None) {
            self.status.clear();
        }
        ui.set_cursor_screen_pos([origin[0], origin[1] + height]);
    }

    fn stage_title(&self) -> String {
        if let Some(channel) = &self.selected {
            if matches!(self.place, Place::Server(_)) {
                return channel.name.clone();
            }
        }
        match self.place {
            Place::Home => "Friends".to_string(),
            Place::Direct(dm_id) => self
                .dms
                .iter()
                .find(|chat| chat.id == dm_id)
                .map(|chat| chat.peer_display_name.clone())
                .unwrap_or_else(|| "Direct message".to_string()),
            Place::Server(server_id) => self
                .servers
                .iter()
                .find(|server| server.id == server_id)
                .map(|server| server.name.clone())
                .unwrap_or_else(|| "Server".to_string()),
        }
    }

    fn stage_icon(&self) -> Option<&'static str> {
        if let Some(channel) = &self.selected {
            if matches!(self.place, Place::Server(_)) {
                return Some(match channel.kind {
                    ChannelKind::Text => "hash",
                    ChannelKind::Voice => "volume-2",
                });
            }
        }
        match self.place {
            Place::Home => Some("users"),
            Place::Direct(_) => Some("user"),
            Place::Server(_) => None,
        }
    }

    /// Friends home: Online, All, Pending, and Add Friend, laid out like Discord's people list.
    fn draw_people(&mut self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let height = ui.content_region_avail()[1];
        let left = origin[0] + 30.0;
        let right = origin[0] + width - 20.0;
        let semi = self.semibold_font;
        if self.friend_tab == 3 {
            ui.dummy([width, 20.0]);
            skin::text(ui, semi, FONT_LG, [left, origin[1] + 20.0], TEXT, "Add Friend");
            skin::text(ui, None, FONT_SM, [left, origin[1] + 52.0], TEXT_SECONDARY, "You can add friends with their Banter username.");
            let box_min = [left, origin[1] + 80.0];
            let box_max = [right, box_min[1] + 52.0];
            skin::rect(ui, box_min, box_max, INPUT_BG, 8.0);
            let button_w = 156.0;
            let enabled = !self.friend_name.trim().is_empty();
            let field_w = (box_max[0] - box_min[0] - button_w - 32.0).max(80.0);
            let submit = skin::input(
                ui,
                "##friend",
                &mut self.friend_name,
                "You can add friends with their Banter username.",
                [box_min[0] + 6.0, box_min[1] + 6.0],
                field_w,
                40.0,
                FONT_MD,
                false,
            );
            let send = skin::button(
                ui,
                "send-friend",
                "Send Friend Request",
                Tone::Brand,
                [box_max[0] - 12.0 - button_w, box_min[1] + 10.0],
                [button_w, 32.0],
                enabled,
                semi,
            );
            if (submit || send) && enabled {
                let name = self.friend_name.trim().to_string();
                if self.note(self.client.send_friend_request(&name)).is_some() {
                    self.friend_name.clear();
                    self.refresh_shell();
                }
            }
            skin::line_h(ui, origin[0], origin[0] + width, origin[1] + 156.0, LIST_DIVIDER);
            ui.set_cursor_screen_pos([origin[0], origin[1] + 160.0]);
            ui.dummy([width, 1.0]);
            return;
        }
        let mut rows: Vec<(i64, String, String, Presence, String, bool)> = Vec::new();
        let title;
        let empty;
        if self.friend_tab == 2 {
            for request in &self.incoming {
                rows.push((request.id, request.from_username.clone(), request.from_username.clone(), Presence::Offline, "Incoming Friend Request".into(), true));
            }
            for request in &self.outgoing {
                rows.push((request.id, request.to_username.clone(), request.to_username.clone(), Presence::Offline, "Outgoing Friend Request".into(), false));
            }
            title = "Pending";
            empty = "There are no pending friend requests.";
        } else {
            let online_only = self.friend_tab == 0;
            for friend in &self.friends {
                let presence = self.presence.get(&friend.id).copied().unwrap_or(Presence::Offline);
                if online_only && presence == Presence::Offline {
                    continue;
                }
                rows.push((friend.id, friend.display_name.clone(), friend.username.clone(), presence, presence_label(presence).into(), false));
            }
            title = if online_only { "Online" } else { "All Friends" };
            empty = if online_only { "No one's around right now." } else { "You have not added anyone yet." };
        }
        if rows.is_empty() {
            ui.dummy([width, height.max(1.0)]);
            let mid_y = origin[1] + height * 0.42;
            skin::text_centered(ui, None, FONT_MD, origin[0] + width * 0.5, mid_y, MUTED, empty);
            return;
        }
        let label = format!("{} — {}", title.to_uppercase(), rows.len());
        skin::text(ui, semi, FONT_XS, [left, origin[1] + 24.0], TEXT_SECONDARY, &label);
        ui.dummy([width, 48.0]);
        let pending_tab = self.friend_tab == 2;
        let mut open_chat = None;
        let mut accept = None;
        let mut decline = None;
        for (index, (id, name, username, presence, subtitle, incoming)) in rows.iter().enumerate() {
            let row_min = [left - 10.0, ui.cursor_screen_pos()[1]];
            let row_max = [right + 10.0, row_min[1] + 62.0];
            let hovered = ui.is_window_hovered() && ui.is_mouse_hovering_rect(row_min, row_max);
            if hovered {
                skin::rect(ui, row_min, row_max, ROW_HOVER, 8.0);
            } else if index > 0 {
                skin::line_h(ui, left, right, row_min[1], LIST_DIVIDER);
            }
            let center = [left + 16.0, row_min[1] + 31.0];
            if pending_tab {
                paint_avatar(ui, None, center, 16.0, &initials(name), None);
            } else {
                let avatar = self.avatars.get(id).map(|shot| shot.texture);
                paint_avatar(ui, avatar, center, 16.0, &initials(name), None);
                paint_status_dot(ui, center, 16.0, *presence, if hovered { ROW_HOVER } else { STAGE_BG });
            }
            let text_x = left + 32.0 + 12.0;
            skin::text(ui, semi, FONT_MD, [text_x, row_min[1] + 12.0], TEXT, name);
            if hovered && !pending_tab {
                let name_w = skin::text_size(ui, semi, FONT_MD, name)[0];
                skin::text(ui, None, FONT_SM, [text_x + name_w + 6.0, row_min[1] + 14.0], TEXT_SECONDARY, username);
            }
            skin::text(ui, None, FONT_SM, [text_x, row_min[1] + 33.0], TEXT_SECONDARY, subtitle);
            let button_y = row_min[1] + 13.0;
            let mut bx = right - 36.0;
            if pending_tab {
                if *incoming {
                    if self.round_action(ui, &format!("decline-{id}"), "x", [bx, button_y], hovered, RED) {
                        decline = Some(*id);
                    }
                    skin::item_tooltip(ui, semi, "Ignore");
                    bx -= 36.0 + 10.0;
                    if self.round_action(ui, &format!("accept-{id}"), "check", [bx, button_y], hovered, GREEN) {
                        accept = Some(*id);
                    }
                    skin::item_tooltip(ui, semi, "Accept");
                }
            } else {
                if self.round_action(ui, &format!("message-{id}"), "message-circle", [bx, button_y], hovered, TEXT_NORMAL) {
                    open_chat = Some(*id);
                }
                skin::item_tooltip(ui, semi, "Message");
            }
            ui.set_cursor_screen_pos(row_min);
            if ui.invisible_button(format!("friend-row-{id}-{index}"), [row_max[0] - row_min[0], 62.0]) && !pending_tab {
                open_chat = Some(*id);
            }
        }
        if let Some(friend_id) = open_chat {
            if let Some(chat) = self.note(self.client.open_dm(friend_id)) {
                self.refresh_shell();
                self.open_dm(chat.id);
            }
        }
        if let Some(id) = accept {
            if self.note(self.client.accept_friend_request(id)).is_some() {
                self.refresh_shell();
            }
        }
        if let Some(id) = decline {
            if self.note(self.client.decline_friend_request(id)).is_some() {
                self.refresh_shell();
            }
        }
    }

    /// Round 36 px action button used in the friends list.
    fn round_action(&self, ui: &Ui, id: &str, icon: &str, min: [f32; 2], row_hovered: bool, hover_tint: [f32; 4]) -> bool {
        ui.set_cursor_screen_pos(min);
        let clicked = ui.invisible_button(id, [36.0, 36.0]);
        let hovered = ui.is_item_hovered();
        let fill = if row_hovered { RAIL_BG } else { SIDEBAR_BG };
        skin::rect(ui, min, [min[0] + 36.0, min[1] + 36.0], fill, 18.0);
        skin::image(ui, self.icon(icon), [min[0] + 8.0, min[1] + 8.0], 20.0, if hovered { hover_tint } else { TEXT_SECONDARY });
        clicked
    }

    fn draw_channels(&mut self, ui: &Ui, server_id: i64) {
        let layout = self.layout.clone();
        let voice_members = self.voice_members.clone();
        let owner = self.servers.iter().find(|server| server.id == server_id).map(|server| server.owner_id);
        let is_owner = owner.is_some() && owner == self.account.as_ref().map(|account| account.id);
        let selected_id = self.selected.as_ref().map(|channel| channel.id);
        ui.dummy([0.0, 4.0]);
        for category in layout {
            let category_id = category.category.id;
            let collapsed = self.collapsed.contains(&category_id);
            let (toggle, create) = self.category_row(ui, category_id, &category.category.name, collapsed, is_owner);
            if toggle {
                if collapsed {
                    self.collapsed.remove(&category_id);
                } else {
                    self.collapsed.insert(category_id);
                }
            }
            if create {
                self.channel_category = Some(category_id);
                self.channel_name.clear();
                self.channel_is_voice = false;
                self.open_modal(Modal::CreateChannel);
            }
            for channel in category.channels {
                let joined = channel.kind == ChannelKind::Voice && self.voice_channel == Some(channel.id);
                let selected = selected_id == Some(channel.id);
                if collapsed && !selected && !joined {
                    continue;
                }
                let icon_name = match channel.kind {
                    ChannelKind::Text => "hash",
                    ChannelKind::Voice => "volume-2",
                };
                if self.channel_row(ui, &format!("ch{}", channel.id), Some(icon_name), &channel.name, selected) {
                    self.selected = Some(channel.clone());
                    self.search.clear();
                    self.reply_to = None;
                    self.editing = None;
                    self.messages.clear();
                    if channel.kind == ChannelKind::Text {
                        self.reload_messages();
                    } else if self.voice_channel != Some(channel.id) {
                        self.join_voice_channel(channel.id);
                    }
                }
                if joined {
                    for member in &voice_members {
                        self.draw_voice_member(ui, member);
                    }
                }
            }
        }
        ui.dummy([0.0, 8.0]);
    }

    /// Category header: chevron, 12 px uppercase name, and a Create Channel plus for the owner.
    fn category_row(&self, ui: &Ui, category_id: i64, name: &str, collapsed: bool, can_create: bool) -> (bool, bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let text_top = origin[1] + 16.0;
        let text_h = CATEGORY_H - 16.0;
        let mut create = false;
        let plus_min = [origin[0] + width - 8.0 - 18.0, text_top + (text_h - 18.0) * 0.5];
        if can_create {
            create = skin::icon_button(ui, &format!("cat-add-{category_id}"), self.icon("plus"), plus_min, 18.0, 18.0, MUTED, TEXT_NORMAL, None);
            skin::item_tooltip(ui, self.semibold_font, "Create Channel");
        }
        ui.set_cursor_screen_pos(origin);
        let toggle_w = if can_create { width - 8.0 - 18.0 - 4.0 } else { width };
        let toggle = ui.invisible_button(format!("cat-{category_id}"), [toggle_w, CATEGORY_H]);
        let color = if ui.is_item_hovered() { TEXT_NORMAL } else { MUTED };
        let chevron = if collapsed { "chevron-right" } else { "chevron-down" };
        skin::image(ui, self.icon(chevron), [origin[0] + 4.0, text_top + (text_h - 12.0) * 0.5], 12.0, color);
        let label = name.to_uppercase();
        let shown = skin::ellipsize(ui, self.semibold_font, FONT_XS, &label, toggle_w - 22.0);
        skin::text_in_row(ui, self.semibold_font, FONT_XS, origin[0] + 18.0, text_top, text_h, color, &shown);
        (toggle, create)
    }

    /// A 32 px channel row with 8 px insets, rounded hover and selected fills, icon, then name.
    fn channel_row(&self, ui: &Ui, id: &str, icon: Option<&str>, label: &str, selected: bool) -> bool {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button(id, [width, CHANNEL_ROW_H + CHANNEL_ROW_GAP]);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + ROW_INSET, origin[1] + CHANNEL_ROW_GAP * 0.5];
        let max = [origin[0] + width - ROW_INSET, min[1] + CHANNEL_ROW_H];
        if selected || hovered {
            skin::rect(ui, min, max, if selected { ROW_SELECTED } else { ROW_HOVER }, ROW_RADIUS);
        }
        let color = if selected {
            WHITE
        } else if hovered {
            TEXT_NORMAL
        } else {
            MUTED
        };
        let mut text_x = min[0] + 8.0;
        if let Some(icon) = icon {
            skin::image(ui, self.icon(icon), [text_x, min[1] + 6.0], 20.0, if selected { TEXT_NORMAL } else { CHANNEL_ICON });
            text_x += 20.0 + 6.0;
        }
        let shown = skin::ellipsize(ui, None, FONT_MD, label, max[0] - 8.0 - text_x);
        skin::text_in_row(ui, None, FONT_MD, text_x, min[1], CHANNEL_ROW_H, color, &shown);
        clicked
    }

    fn draw_members(&mut self, ui: &Ui) {
        match self.place {
            Place::Server(_) => {
                let members = self.members.clone();
                let mut online = Vec::new();
                let mut offline = Vec::new();
                for member in members {
                    let presence = self.presence.get(&member.user_id).copied().unwrap_or(member.presence);
                    if presence == Presence::Offline {
                        offline.push((member, presence));
                    } else {
                        online.push((member, presence));
                    }
                }
                self.member_group(ui, "ONLINE", &online);
                self.member_group(ui, "OFFLINE", &offline);
                ui.dummy([0.0, 20.0]);
            }
            Place::Direct(dm_id) => {
                if let Some(chat) = self.dms.iter().find(|chat| chat.id == dm_id).cloned() {
                    let presence = self.presence.get(&chat.peer_id).copied().unwrap_or(Presence::Offline);
                    ui.dummy([0.0, 16.0]);
                    self.person_row(ui, chat.peer_id, &chat.peer_display_name, &chat.peer_username, presence, false);
                }
            }
            Place::Home => {}
        }
    }

    fn member_group(&mut self, ui: &Ui, title: &str, members: &[(crate::api::MemberView, Presence)]) {
        if members.is_empty() {
            return;
        }
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        ui.dummy([width, MEMBER_GROUP_H]);
        let label = format!("{title} — {}", members.len());
        skin::text_in_row(ui, self.semibold_font, FONT_XS, origin[0] + 16.0, origin[1] + 24.0, 16.0, MUTED, &label);
        for (member, presence) in members {
            let name = member.nickname.clone().unwrap_or(member.display_name.clone());
            self.person_row(ui, member.user_id, &name, &member.username, *presence, member.is_owner);
        }
    }

    /// 42 px member row: 32 px avatar with status, name, and the owner crown. Offline rows are dimmed.
    fn person_row(&self, ui: &Ui, user_id: i64, name: &str, _username: &str, presence: Presence, owner: bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let _clicked = ui.invisible_button(format!("member-{user_id}"), [width, MEMBER_ROW_H]);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + ROW_INSET, origin[1] + 1.0];
        let max = [origin[0] + width - ROW_INSET, origin[1] + MEMBER_ROW_H - 1.0];
        if hovered {
            skin::rect(ui, min, max, ROW_HOVER, ROW_RADIUS);
        }
        let backdrop = if hovered { ROW_HOVER } else { SIDEBAR_BG };
        let center = [min[0] + 8.0 + 16.0, origin[1] + MEMBER_ROW_H * 0.5];
        let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
        paint_avatar(ui, avatar, center, 16.0, &initials(name), None);
        let offline = presence == Presence::Offline;
        if offline {
            ui.get_window_draw_list().add_circle(center, 16.5, with_alpha(backdrop, 0.7)).filled(true).build();
        } else {
            paint_status_dot(ui, center, 16.0, presence, backdrop);
        }
        let text_x = min[0] + 8.0 + 32.0 + 12.0;
        let color = if offline {
            with_alpha(MUTED, 0.5)
        } else if hovered {
            TEXT_NORMAL
        } else {
            MUTED
        };
        let crown_w = if owner { 18.0 } else { 0.0 };
        let shown = skin::ellipsize(ui, None, FONT_MD, name, max[0] - 8.0 - crown_w - text_x);
        skin::text_in_row(ui, None, FONT_MD, text_x, origin[1], MEMBER_ROW_H, color, &shown);
        if owner {
            let name_w = skin::text_size(ui, None, FONT_MD, &shown)[0];
            let crown_min = [text_x + name_w + 4.0, center[1] - 7.0];
            skin::image(ui, self.icon("crown"), crown_min, 14.0, WHITE);
            if ui.is_mouse_hovering_rect(crown_min, [crown_min[0] + 14.0, crown_min[1] + 14.0]) {
                skin::tooltip(ui, self.semibold_font, FONT_SM, "Server Owner", crown_min, [crown_min[0] + 14.0, crown_min[1] + 14.0], TooltipSide::Top);
            }
        }
    }

    fn draw_message_rows(
        &mut self,
        ui: &Ui,
        mine: Option<i64>,
        reply: &mut Option<i64>,
        edit: &mut Option<i64>,
        delete: &mut Option<i64>,
        save: &mut Option<(i64, String)>,
    ) {
        let offset = zone_offset_secs();
        let now = chrono::Local::now().timestamp_millis();
        let width = ui.content_region_avail()[0].max(160.0);
        let text_x = 72.0;
        let text_w = (width - text_x - 24.0).max(40.0);
        let messages = self.messages.clone();
        let mut previous: Option<(i64, i64)> = None;
        for message in &messages {
            if previous.is_some_and(|(_, at)| message_starts_new_day(at, message.created_at, offset)) {
                let label = chat_date_separator(message.created_at, offset);
                let origin = ui.cursor_screen_pos();
                ui.dummy([width, 28.0]);
                let _small = ui.push_font_with_size(None, 12.0);
                let label_w = ui.calc_text_size(&label)[0];
                let mid_y = origin[1] + 14.0;
                let draw = ui.get_window_draw_list();
                draw.add_line([origin[0] + 16.0, mid_y], [origin[0] + (width - label_w) * 0.5 - 8.0, mid_y], MUTED).build();
                draw.add_text([origin[0] + (width - label_w) * 0.5, origin[1] + 6.0], MUTED, &label);
                draw.add_line([origin[0] + (width + label_w) * 0.5 + 8.0, mid_y], [origin[0] + width - 16.0, mid_y], MUTED).build();
            }
            let follow = previous
                .is_some_and(|(author, at)| message_is_followup(author, at, message.author_id, message.created_at, offset));
            let name = message.author_nickname.clone().unwrap_or_else(|| message.author_display_name.clone());
            let edited = if message.edited_at.is_some() { " (edited)" } else { "" };
            let body = format!("{}{edited}", message.body);
            let body_h = if message.body.is_empty() {
                0.0
            } else {
                let _body_font = ui.push_font_with_size(None, 16.0);
                ui.calc_text_size_with_opts(&body, false, text_w)[1]
            };
            let row_h = if follow { body_h.max(22.0) } else { 22.0 + body_h.max(20.0) }.max(if follow { 22.0 } else { 48.0 });
            let origin = ui.cursor_screen_pos();
            ui.dummy([width, row_h + 2.0]);
            let row_max = [origin[0] + width, origin[1] + row_h];
            let hot = pointer_in(ui, [origin[0], origin[1] - 16.0], row_max) || self.more_for == Some(message.id);
            if hot {
                ui.get_window_draw_list()
                    .add_rect([origin[0] + 4.0, origin[1]], [origin[0] + width - 4.0, origin[1] + row_h], ROW_HOVER)
                    .filled(true)
                    .build();
            }
            if follow {
                if hot {
                    let gutter = clock_short(message.created_at, offset);
                    let _small = ui.push_font_with_size(None, 12.0);
                    let gutter_w = ui.calc_text_size(&gutter)[0];
                    ui.get_window_draw_list().add_text([origin[0] + text_x - gutter_w - 8.0, origin[1] + 2.0], MUTED, &gutter);
                    if pointer_in(ui, [origin[0], origin[1]], [origin[0] + text_x, origin[1] + row_h]) {
                        ui.tooltip_text(chat_time_hover(message.created_at, offset));
                    }
                }
            } else {
                let center = [origin[0] + 36.0, origin[1] + 22.0];
                let avatar = self.avatars.get(&message.author_id).map(|shot| shot.texture);
                paint_avatar(ui, avatar, center, 20.0, &initials(&name), None);
                let name_w = {
                    let _bold = ui.push_font_with_size(self.bold_font, 16.0);
                    ui.calc_text_size(&name)[0]
                };
                {
                    let _bold = ui.push_font_with_size(self.bold_font, 16.0);
                    ui.get_window_draw_list().add_text([origin[0] + text_x, origin[1] + 2.0], TEXT, &name);
                }
                let stamp = chat_time_label(message.created_at, now, offset);
                let stamp_pos = [origin[0] + text_x + name_w + 8.0, origin[1] + 5.0];
                let stamp_w = {
                    let _small = ui.push_font_with_size(None, 12.0);
                    let width = ui.calc_text_size(&stamp)[0];
                    ui.get_window_draw_list().add_text(stamp_pos, MUTED, &stamp);
                    width
                };
                if pointer_in(ui, stamp_pos, [stamp_pos[0] + stamp_w, stamp_pos[1] + 16.0]) {
                    ui.tooltip_text(chat_time_hover(message.created_at, offset));
                }
            }
            if !message.body.is_empty() {
                ui.set_cursor_screen_pos([origin[0] + text_x, origin[1] + if follow { 0.0 } else { 22.0 }]);
                let _body_font = ui.push_font_with_size(None, 16.0);
                let _wrap = ui.push_text_wrap_pos(origin[0] + text_x + text_w);
                ui.text_wrapped(&body);
            }
            if let Some(reply_to) = message.reply_to {
                ui.set_cursor_screen_pos([origin[0] + text_x, origin[1] + row_h - 16.0]);
                ui.text_colored(BLURPLE, format!("replying to {reply_to}"));
            }
            let mut attachment_y = origin[1] + row_h;
            for attachment in &message.attachments {
                ui.set_cursor_screen_pos([origin[0] + text_x, attachment_y]);
                ui.text(format!("file: {} ({} bytes)", attachment.filename, attachment.byte_size));
                ui.same_line();
                if ui.button(format!("Save##att{}", attachment.id)) {
                    *save = Some((attachment.id, attachment.filename.clone()));
                }
                attachment_y += 22.0;
            }
            let row_end = attachment_y.max(origin[1] + row_h + 2.0);
            if hot {
                let mut tools = vec!["reply"];
                if mine == Some(message.author_id) {
                    tools.push("pencil");
                    tools.push("trash-2");
                }
                tools.push("ellipsis");
                let bar_w = tools.len() as f32 * 32.0 + 8.0;
                let bar_x = origin[0] + width - bar_w - 12.0;
                let bar_y = origin[1] - 16.0;
                ui.get_window_draw_list()
                    .add_rect([bar_x, bar_y], [bar_x + bar_w, bar_y + 32.0], PANEL_BG)
                    .rounding(6.0)
                    .filled(true)
                    .build();
                for (index, name) in tools.iter().enumerate() {
                    ui.set_cursor_screen_pos([bar_x + 4.0 + index as f32 * 32.0, bar_y]);
                    let tip = match *name {
                        "reply" => "Reply",
                        "pencil" => "Edit",
                        "trash-2" => "Delete",
                        _ => "More",
                    };
                    if self.tool(ui, &format!("{name}-{}", message.id), name, false, Some(tip)) {
                        match *name {
                            "reply" => *reply = Some(message.id),
                            "pencil" => *edit = Some(message.id),
                            "trash-2" => *delete = Some(message.id),
                            _ => self.more_for = if self.more_for == Some(message.id) { None } else { Some(message.id) },
                        }
                    }
                }
                if self.more_for == Some(message.id) {
                    ui.set_cursor_screen_pos([bar_x, bar_y + 36.0]);
                    if ui.button(format!("Copy text##{}", message.id)) {
                        self.copy_message = Some(message.body.clone());
                        self.more_for = None;
                    }
                }
            }
            // The toolbar and body text move the cursor; every row must end at its own bottom edge, or the
            // next row shifts while one is hovered and the measured history height stops matching the scroll extent.
            ui.set_cursor_screen_pos([origin[0], row_end]);
            ui.dummy([0.0, 0.0]);
            previous = Some((message.author_id, message.created_at));
        }
    }

    fn draw_chat(&mut self, ui: &Ui) {
        if self.voice_selected() || self.call_stage_open() {
            return;
        }
        let Some(target) = self.chat_target() else {
            self.draw_no_text_channels(ui);
            return;
        };
        let mut reply = None;
        let mut edit = None;
        let mut delete = None;
        let mut save = None;
        let footer = self.composer_footer_h();
        ui.child_window("history").size([0.0, -footer]).build(ui, || {
            let stick = crate::theme::chat_should_stick(ui.scroll_y(), ui.scroll_max_y(), self.chat_jump);
            // Pad with last frame's history height so few messages sit on the composer, like Discord.
            let view_h = ui.content_region_avail()[1];
            ui.dummy([0.0, crate::theme::chat_top_pad(self.history_h, view_h)]);
            let history_top = ui.cursor_pos()[1];
            let mine = self.account.as_ref().map(|account| account.id);
            if self.messages.is_empty() && self.search.trim().is_empty() {
                if let Some(channel) = &self.selected {
                    if matches!(self.place, Place::Server(_)) && channel.kind == ChannelKind::Text {
                        ui.dummy([0.0, 24.0]);
                        ui.indent_by(16.0);
                        let _title = ui.push_font_with_size(self.bold_font, 28.0);
                        ui.text(format!("Welcome to #{}!", channel.name));
                        drop(_title);
                        ui.text_colored(MUTED, format!("This is the start of the #{} channel.", channel.name));
                        ui.unindent_by(16.0);
                    }
                }
            }
            self.draw_message_rows(ui, mine, &mut reply, &mut edit, &mut delete, &mut save);
            self.history_h = ui.cursor_pos()[1] - history_top;
            if stick {
                ui.set_scroll_here_y(1.0);
            }
            self.chat_jump = false;
        });
        if let Some(id) = reply {
            self.reply_to = Some(id);
            self.editing = None;
        }
        if let Some(id) = edit {
            if let Some(message) = self.messages.iter().find(|message| message.id == id) {
                self.composer = message.body.clone();
            }
            self.editing = Some(id);
            self.reply_to = None;
            self.pending_file = None;
        }
        if let Some(id) = delete {
            if self.note(self.client.delete_message(id)).is_some() {
                remove_message(&mut self.messages, id);
            }
        }
        if let Some((id, filename)) = save {
            self.save_attachment(id, &filename);
        }
        self.draw_composer(ui, target);
    }

    fn draw_no_text_channels(&self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0].max(1.0);
        let height = ui.content_region_avail()[1].max(1.0);
        ui.dummy([width, height]);
        let center_x = origin[0] + width * 0.5;
        let top = origin[1] + height * 0.4;
        skin::text_centered(ui, self.bold_font, FONT_MD, center_x, top, TEXT_SECONDARY, "NO TEXT CHANNELS");
        let body = "You find yourself in a strange place. You don't have access to any text channels, or there are none in this server.";
        let mut y = top + 28.0;
        for line in skin::wrap_lines(ui, None, FONT_SM, body, (width - 64.0).min(440.0)) {
            skin::text_centered(ui, None, FONT_SM, center_x, y, MUTED, &line);
            y += 20.0;
        }
    }

    fn composer_footer_h(&self) -> f32 {
        let strip = if self.reply_to.is_some() || self.editing.is_some() || self.pending_file.is_some() { 34.0 } else { 0.0 };
        8.0 + strip + COMPOSER_H + COMPOSER_BOTTOM
    }

    /// Rounded message bar: upload button, `Message #channel` placeholder, and a strip above it
    /// while replying, editing, or holding a file.
    fn draw_composer(&mut self, ui: &Ui, target: ChatTarget) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let bar_x = origin[0] + COMPOSER_MARGIN;
        let bar_w = (width - COMPOSER_MARGIN * 2.0).max(120.0);
        let mut y = origin[1] + 8.0;
        ui.dummy([width, self.composer_footer_h()]);
        let strip = if let Some(id) = self.reply_to {
            let author = self
                .messages
                .iter()
                .find(|message| message.id == id)
                .map(|message| message.author_nickname.clone().unwrap_or_else(|| message.author_display_name.clone()));
            Some(match author {
                Some(name) => format!("Replying to {name}"),
                None => "Replying to a message".to_string(),
            })
        } else if self.editing.is_some() {
            Some("Editing message".to_string())
        } else {
            self.pending_file.as_ref().map(|(name, bytes)| format!("{name} ({} KB)", bytes.len().div_ceil(1024)))
        };
        if let Some(label) = &strip {
            skin::rect_corners(ui, [bar_x, y], [bar_x + bar_w, y + 34.0], SIDEBAR_BG, COMPOSER_RADIUS, DrawFlags::ROUND_TOP);
            let shown = skin::ellipsize(ui, None, FONT_SM, label, bar_w - 64.0);
            skin::text_in_row(ui, None, FONT_SM, bar_x + 16.0, y, 34.0, TEXT_SECONDARY, &shown);
            if skin::icon_button(ui, "composer-cancel", self.icon("x"), [bar_x + bar_w - 8.0 - 24.0, y + 5.0], 24.0, 16.0, TEXT_SECONDARY, TEXT_NORMAL, None) {
                if self.editing.take().is_some() {
                    self.composer.clear();
                }
                if self.reply_to.take().is_none() {
                    self.pending_file = None;
                }
            }
            y += 34.0;
        }
        let corners = if strip.is_some() { DrawFlags::ROUND_BOTTOM } else { DrawFlags::ROUND_ALL };
        skin::rect_corners(ui, [bar_x, y], [bar_x + bar_w, y + COMPOSER_H], COMPOSER_BG, COMPOSER_RADIUS, corners);
        let input_x = if self.editing.is_none() {
            let tint = if self.pending_file.is_some() { TEXT_NORMAL } else { TEXT_SECONDARY };
            if skin::icon_button(ui, "attach", self.icon("circle-plus"), [bar_x + 12.0, y + 8.0], 28.0, 24.0, tint, TEXT_NORMAL, None) {
                self.pick_attachment();
            }
            skin::item_tooltip(ui, self.semibold_font, "Upload a File");
            bar_x + 12.0 + 28.0 + 12.0
        } else {
            bar_x + 8.0
        };
        let channel_mark = if self.stage_icon() == Some("hash") { "#" } else { "@" };
        let hint = format!("Message {channel_mark}{}", self.stage_title());
        ui.set_cursor_screen_pos([input_x, y]);
        let submit = {
            let _font = ui.push_font_with_size(None, FONT_MD);
            let _pad = ui.push_style_var(StyleVar::FramePadding([4.0, (COMPOSER_H - FONT_MD) * 0.5]));
            let _border = ui.push_style_var(StyleVar::FrameBorderSize(0.0));
            let _bg = ui.push_style_color(StyleColor::FrameBg, skin::CLEAR);
            let _bg_hover = ui.push_style_color(StyleColor::FrameBgHovered, skin::CLEAR);
            let _bg_active = ui.push_style_color(StyleColor::FrameBgActive, skin::CLEAR);
            let _hint = ui.push_style_color(StyleColor::TextDisabled, PLACEHOLDER);
            let _text = ui.push_style_color(StyleColor::Text, TEXT_NORMAL);
            ui.set_next_item_width((bar_x + bar_w - 16.0 - input_x).max(40.0));
            ui.input_text("##composer", &mut self.composer).hint(hint).enter_returns_true(true).build()
        };
        if submit {
            self.submit_composer(target);
        }
    }

    fn join_voice_channel(&mut self, channel_id: i64) {
        if let Some(members) = self.note(self.client.join_voice(channel_id)) {
            self.voice_members = members;
            self.voice_channel = Some(channel_id);
            self.call = None;
            self.sent_flags = VoiceFlags { muted: !self.flags.muted, ..self.flags };
        }
    }

    fn draw_voice_stage(&mut self, ui: &Ui) {
        let channel = self.selected.clone();
        let joined = channel.as_ref().is_some_and(|channel| self.voice_channel == Some(channel.id));
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0].max(1.0);
        let height = ui.content_region_avail()[1].max(1.0);
        ui.dummy([width, height]);
        skin::rect(ui, origin, [origin[0] + width, origin[1] + height], CALL_BG, 0.0);
        let semi = self.semibold_font;
        if !joined {
            if let Some(channel) = channel {
                let center_x = origin[0] + width * 0.5;
                let title_y = origin[1] + height * 0.4;
                skin::text_centered(ui, semi, FONT_XL, center_x, title_y, TEXT, &channel.name);
                skin::text_centered(ui, None, FONT_MD, center_x, title_y + 36.0, TEXT_SECONDARY, "No one is currently in voice");
                if skin::button(ui, "join-voice", "Join Voice", Tone::Green, [center_x - 60.0, title_y + 72.0], [120.0, BUTTON_H], true, semi) {
                    self.join_voice_channel(channel.id);
                }
            }
            return;
        }
        let members = self.voice_members.clone();
        let people = if members.is_empty() {
            let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
            vec![banter_server::VoiceMember { user_id: me, flags: self.flags }]
        } else {
            members
        };
        let pad = 16.0;
        let area_w = (width - pad * 2.0).max(80.0);
        let area_h = (height - CALL_BAR_H - pad).max(80.0);
        let sharing = self.screen || people.iter().any(|member| member.flags.screen);
        let featured_h = if sharing { (area_h * 0.62).floor() } else { 0.0 };
        if sharing {
            self.draw_featured_screen_at(ui, [origin[0] + pad, origin[1] + pad], area_w, featured_h);
        }
        let tiles_top = origin[1] + pad + if sharing { featured_h + TILE_GAP } else { 0.0 };
        let tiles_h = (area_h - if sharing { featured_h + TILE_GAP } else { 0.0 }).max(60.0);
        let grid = voice_tile_grid(people.len(), area_w, tiles_h, TILE_GAP);
        let grid_w = grid.cols as f32 * grid.tile_w + grid.cols.saturating_sub(1) as f32 * TILE_GAP;
        let grid_h = grid.rows as f32 * grid.tile_h + grid.rows.saturating_sub(1) as f32 * TILE_GAP;
        let start_x = origin[0] + pad + (area_w - grid_w) * 0.5;
        let start_y = tiles_top + (tiles_h - grid_h) * 0.5;
        for (index, member) in people.iter().enumerate() {
            let col = index % grid.cols.max(1);
            let row = index / grid.cols.max(1);
            let tile_origin = [
                (start_x + col as f32 * (grid.tile_w + TILE_GAP)).floor(),
                (start_y + row as f32 * (grid.tile_h + TILE_GAP)).floor(),
            ];
            self.draw_voice_tile_at(ui, member, tile_origin, grid.tile_w.floor(), grid.tile_h.floor());
        }
        self.draw_stage_controls(ui, [origin[0], origin[1] + height - CALL_BAR_H], width);
        if self.push_to_talk {
            let note = format!("Hold {} to talk.", self.voice.ptt_key);
            skin::text_centered(ui, None, FONT_XS, origin[0] + width * 0.5, origin[1] + height - 18.0, MUTED, &note);
        }
    }

    /// Discord-style 1:1 DM call stage: large avatar while ringing, two tiles when active.
    fn draw_call_stage(&mut self, ui: &Ui) {
        let Some(call) = self.call.clone() else { return };
        let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
        let peer = if call.caller == me { call.callee } else { call.caller };
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0].max(1.0);
        let height = ui.content_region_avail()[1].max(1.0);
        ui.dummy([width, height]);
        skin::rect(ui, origin, [origin[0] + width, origin[1] + height], CALL_BG, 0.0);
        let semi = self.semibold_font;
        match call.phase {
            CallPhase::Ringing => {
                let center = call_ring_avatar_center(origin, width, height, CALL_BAR_H);
                let radius = CALL_RING_AVATAR * 0.5;
                let name = self.name_of(peer);
                let avatar = self.avatars.get(&peer).map(|shot| shot.texture);
                paint_avatar(ui, avatar, center, radius, &initials(&name), None);
                let status = if call.callee == me { "Incoming Call" } else { "Calling…" };
                skin::text_centered(ui, semi, FONT_XL, center[0], center[1] + radius + 24.0, TEXT, &name);
                skin::text_centered(ui, None, FONT_MD, center[0], center[1] + radius + 52.0, TEXT_SECONDARY, status);
                if call.callee == me {
                    let y = origin[1] + height - CALL_BAR_H - CALL_BUTTON - 24.0;
                    let xs = centered_row(width, &[CALL_BUTTON, CALL_BUTTON], CALL_BUTTON_GAP * 2.0);
                    if self.round_control_colored(
                        ui,
                        "call-accept",
                        "phone",
                        [origin[0] + xs[0], y],
                        GREEN_BUTTON,
                        GREEN_BUTTON_HOVER,
                        WHITE,
                    ) {
                        self.accept_call(call.dm_id);
                    }
                    skin::item_tooltip(ui, semi, "Accept");
                    if self.round_control(ui, "call-decline", "phone-off", [origin[0] + xs[1], y], true, WHITE) {
                        if self.note(self.client.decline_call(call.dm_id)).is_some() {
                            self.end_media_place();
                        }
                    }
                    skin::item_tooltip(ui, semi, "Decline");
                }
            }
            CallPhase::Active => {
                let people = self.dm_call_members(me, peer);
                let pad = 16.0;
                let area_w = (width - pad * 2.0).max(80.0);
                let area_h = (height - CALL_BAR_H - pad).max(80.0);
                let sharing = self.screen || people.iter().any(|member| member.flags.screen);
                let featured_h = if sharing { (area_h * 0.62).floor() } else { 0.0 };
                if sharing {
                    self.draw_featured_screen_at(ui, [origin[0] + pad, origin[1] + pad], area_w, featured_h);
                }
                let tiles_top = origin[1] + pad + if sharing { featured_h + TILE_GAP } else { 0.0 };
                let tiles_h = (area_h - if sharing { featured_h + TILE_GAP } else { 0.0 }).max(60.0);
                let count = dm_call_participant_tiles(true);
                let grid = voice_tile_grid(count, area_w, tiles_h, TILE_GAP);
                let grid_w = grid.cols as f32 * grid.tile_w + grid.cols.saturating_sub(1) as f32 * TILE_GAP;
                let grid_h = grid.rows as f32 * grid.tile_h + grid.rows.saturating_sub(1) as f32 * TILE_GAP;
                let start_x = origin[0] + pad + (area_w - grid_w) * 0.5;
                let start_y = tiles_top + (tiles_h - grid_h) * 0.5;
                for (index, member) in people.iter().enumerate().take(count) {
                    let col = index % grid.cols.max(1);
                    let row = index / grid.cols.max(1);
                    let tile_origin = [
                        (start_x + col as f32 * (grid.tile_w + TILE_GAP)).floor(),
                        (start_y + row as f32 * (grid.tile_h + TILE_GAP)).floor(),
                    ];
                    self.draw_voice_tile_at(ui, member, tile_origin, grid.tile_w.floor(), grid.tile_h.floor());
                }
            }
        }
        self.draw_stage_controls(ui, [origin[0], origin[1] + height - CALL_BAR_H], width);
        if self.push_to_talk {
            let note = format!("Hold {} to talk.", self.voice.ptt_key);
            skin::text_centered(ui, None, FONT_XS, origin[0] + width * 0.5, origin[1] + height - 18.0, MUTED, &note);
        }
    }

    fn dm_call_members(&self, me: i64, peer: i64) -> Vec<VoiceMember> {
        vec![
            VoiceMember {
                user_id: peer,
                flags: VoiceFlags::default(),
            },
            VoiceMember {
                user_id: me,
                flags: self.flags,
            },
        ]
    }

    /// Centered call controls: camera, screen share with its options chevron, mic, and the red disconnect.
    fn draw_stage_controls(&mut self, ui: &Ui, origin: [f32; 2], width: f32) {
        let semi = self.semibold_font;
        let widths = [CALL_BUTTON, CALL_BUTTON + CALL_CHEVRON_W, CALL_BUTTON, CALL_BUTTON];
        let xs = centered_row(width, &widths, CALL_BUTTON_GAP);
        let y = origin[1] + (CALL_BAR_H - CALL_BUTTON) * 0.5;
        let camera = self.camera;
        let camera_icon = if camera { "video" } else { "video-off" };
        if self.round_control(ui, "stage-camera", camera_icon, [origin[0] + xs[0], y], false, WHITE) {
            self.camera = !self.camera;
        }
        skin::item_tooltip(ui, semi, if camera { "Turn Off Camera" } else { "Turn On Camera" });
        let live = self.share.is_live();
        let screen_min = [origin[0] + xs[1], y];
        let screen_icon = if live { "monitor" } else { "monitor-off" };
        let (share_clicked, options_clicked, share_custom) = self.split_control(ui, "stage-screen", screen_icon, screen_min);
        if share_clicked || share_custom {
            self.on_share_click(share_clicked, share_custom);
        }
        if options_clicked {
            self.stream_anchor = screen_min;
            self.stream_menu = !self.stream_menu;
        }
        let muted = self.mute.muted();
        let (mic_icon, mic_tint) = if muted { ("mic-off", RED) } else { ("mic", WHITE) };
        if self.round_control(ui, "stage-mic", mic_icon, [origin[0] + xs[2], y], false, mic_tint) {
            self.mute.set_muted(!muted);
        }
        skin::item_tooltip(ui, semi, if muted { "Unmute" } else { "Mute" });
        if self.round_control(ui, "stage-disconnect", "phone-off", [origin[0] + xs[3], y], true, WHITE) {
            self.end_voice_or_call();
        }
        let end_tip = match &self.call {
            Some(call) if call.phase == CallPhase::Ringing && self.account.as_ref().map(|a| a.id) != Some(call.callee) => {
                "Cancel"
            }
            Some(call) if call.phase == CallPhase::Ringing => "Decline",
            Some(_) => "Disconnect",
            None => "Disconnect",
        };
        skin::item_tooltip(ui, semi, end_tip);
    }

    fn round_control(&self, ui: &Ui, id: &str, icon: &str, origin: [f32; 2], danger: bool, tint: [f32; 4]) -> bool {
        let (fill, hover) = if danger {
            (DANGER, DANGER_HOVER)
        } else {
            (CONTROL_BG, CONTROL_HOVER)
        };
        self.round_control_colored(ui, id, icon, origin, fill, hover, tint)
    }

    fn round_control_colored(
        &self,
        ui: &Ui,
        id: &str,
        icon: &str,
        origin: [f32; 2],
        fill: [f32; 4],
        hover: [f32; 4],
        tint: [f32; 4],
    ) -> bool {
        ui.set_cursor_screen_pos(origin);
        let clicked = ui.invisible_button(id, [CALL_BUTTON, CALL_BUTTON]);
        let hovered = ui.is_item_hovered();
        let color = if hovered { hover } else { fill };
        let max = [origin[0] + CALL_BUTTON, origin[1] + CALL_BUTTON];
        skin::rect(ui, origin, max, color, CALL_BUTTON * 0.5);
        let inset = (CALL_BUTTON - 24.0) * 0.5;
        skin::image(ui, self.icon(icon), [origin[0] + inset, origin[1] + inset], 24.0, tint);
        clicked
    }

    /// Screen share button joined to its stream-options chevron, as in Discord's call bar.
    fn on_share_click(&mut self, left: bool, custom: bool) {
        if custom {
            self.open_share();
            return;
        }
        if !left {
            return;
        }
        if self.share.is_live() {
            self.share.stop();
            self.save_stream();
            return;
        }
        if media::native_screen_picker() {
            self.share_open = false;
            self.share.go_live_portal();
            self.save_stream();
        } else {
            self.open_share();
        }
    }

    fn split_control(&self, ui: &Ui, id: &str, icon: &str, origin: [f32; 2]) -> (bool, bool, bool) {
        let semi = self.semibold_font;
        let total_w = CALL_BUTTON + CALL_CHEVRON_W;
        let max = [origin[0] + total_w, origin[1] + CALL_BUTTON];
        skin::rect(ui, origin, max, CONTROL_BG, CALL_BUTTON * 0.5);
        ui.set_cursor_screen_pos(origin);
        let main = ui.invisible_button(id, [CALL_BUTTON, CALL_BUTTON]);
        let custom = ui.is_item_clicked_with_button(dear_imgui_rs::MouseButton::Right) && ui.io().key_shift();
        if ui.is_item_hovered() {
            skin::rect_corners(ui, origin, [origin[0] + CALL_BUTTON, max[1]], CONTROL_HOVER, CALL_BUTTON * 0.5, DrawFlags::ROUND_LEFT);
        }
        let live = self.share.is_live();
        skin::item_tooltip(ui, semi, share_tip(live));
        let chevron_min = [origin[0] + CALL_BUTTON, origin[1]];
        ui.set_cursor_screen_pos(chevron_min);
        let options = ui.invisible_button(format!("{id}-options"), [CALL_CHEVRON_W, CALL_BUTTON]);
        if ui.is_item_hovered() || self.stream_menu {
            skin::rect_corners(ui, chevron_min, max, CONTROL_HOVER, CALL_BUTTON * 0.5, DrawFlags::ROUND_RIGHT);
        }
        skin::item_tooltip(ui, semi, "Stream Options");
        skin::rect(ui, [chevron_min[0], origin[1] + 14.0], [chevron_min[0] + 1.0, max[1] - 14.0], CALL_BG, 0.0);
        let inset = (CALL_BUTTON - 24.0) * 0.5;
        skin::image(ui, self.icon(icon), [origin[0] + inset, origin[1] + inset], 24.0, WHITE);
        skin::image(ui, self.icon("chevron-up"), [chevron_min[0] + 2.0, origin[1] + (CALL_BUTTON - 16.0) * 0.5], 16.0, TEXT_NORMAL);
        (main, options, custom)
    }

    /// Draws the open dialog over a dark overlay. A click outside the card or Escape closes it.
    fn draw_modal(&mut self, ui: &Ui) {
        let Some(kind) = self.modal else { return };
        let display = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("##modal")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .build(|| {
                skin::rect(ui, [0.0, 0.0], display, OVERLAY, 0.0);
                let card = match kind {
                    Modal::AddServer => self.modal_add_server(ui, display),
                    Modal::CreateServer => self.modal_create_server(ui, display),
                    Modal::JoinServer => self.modal_join_server(ui, display),
                    Modal::Invite => self.modal_invite(ui, display),
                    Modal::Nickname => self.modal_nickname(ui, display),
                    Modal::CreateChannel => self.modal_create_channel(ui, display),
                    Modal::CreateCategory => self.modal_create_category(ui, display),
                };
                if self.modal_frames > 0 && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    let mouse = ui.mouse_pos();
                    let inside = mouse[0] >= card.0[0] && mouse[0] <= card.1[0] && mouse[1] >= card.0[1] && mouse[1] <= card.1[1];
                    if !inside {
                        self.modal = None;
                    }
                }
            });
        // A dialog opened during this frame starts counting next frame, so it gets focus once.
        if self.modal == Some(kind) {
            self.modal_frames = self.modal_frames.saturating_add(1);
        }
    }

    /// Card, title, subtitle, close button, and footer band. Returns the body top-left corner.
    fn modal_frame(
        &mut self,
        ui: &Ui,
        display: [f32; 2],
        title: &str,
        subtitle: Option<&str>,
        left_aligned: bool,
        body_h: f32,
        footer_h: f32,
    ) -> ([f32; 2], [f32; 2], [f32; 2]) {
        let semi = self.semibold_font;
        let width = MODAL_W.min(display[0] - 32.0);
        let inner_w = width - MODAL_PAD * 2.0;
        let title_size = if left_aligned { FONT_LG } else { FONT_XL };
        let lines = subtitle.map(|text| skin::wrap_lines(ui, None, if left_aligned { FONT_XS } else { FONT_MD }, text, inner_w - 24.0)).unwrap_or_default();
        let line_h = if left_aligned { 16.0 } else { 20.0 };
        let header_h = 24.0 + title_size + 8.0 + lines.len() as f32 * line_h + 16.0;
        let height = header_h + body_h + footer_h;
        let min = [((display[0] - width) * 0.5).floor(), ((display[1] - height) * 0.5).max(16.0).floor()];
        let max = [min[0] + width, min[1] + height];
        skin::rect(ui, min, max, STAGE_BG, MODAL_RADIUS);
        if footer_h > 0.0 {
            skin::rect_corners(ui, [min[0], max[1] - footer_h], max, MODAL_FOOTER_BG, MODAL_RADIUS, DrawFlags::ROUND_BOTTOM);
        }
        let center_x = min[0] + width * 0.5;
        let mut y = min[1] + 24.0;
        if left_aligned {
            skin::text(ui, semi, title_size, [min[0] + MODAL_PAD, y], TEXT, title);
        } else {
            skin::text_centered(ui, semi, title_size, center_x, y, TEXT, title);
        }
        y += title_size + 8.0;
        for line in &lines {
            if left_aligned {
                skin::text(ui, None, FONT_XS, [min[0] + MODAL_PAD, y], TEXT_SECONDARY, line);
            } else {
                skin::text_centered(ui, None, FONT_MD, center_x, y, TEXT_SECONDARY, line);
            }
            y += line_h;
        }
        if skin::icon_button(ui, "modal-close", self.icon("x"), [max[0] - 16.0 - 24.0, min[1] + 16.0], 24.0, 24.0, TEXT_SECONDARY, TEXT_NORMAL, None) {
            self.modal = None;
        }
        ([min[0] + MODAL_PAD, min[1] + header_h], min, max)
    }

    /// Footer with a text link on the left and the primary button on the right.
    fn modal_footer(&mut self, ui: &Ui, max: [f32; 2], min_x: f32, back: &str, primary: &str, tone: Tone, enabled: bool) -> (bool, bool) {
        let semi = self.semibold_font;
        let y = max[1] - MODAL_FOOTER_H + (MODAL_FOOTER_H - BUTTON_H) * 0.5;
        let back_w = skin::text_size(ui, semi, FONT_SM, back)[0] + 32.0;
        let back_clicked = skin::button(ui, "modal-back", back, Tone::Link, [min_x + MODAL_PAD - 16.0, y], [back_w, BUTTON_H], true, semi);
        let primary_w = (skin::text_size(ui, semi, FONT_SM, primary)[0] + 32.0).max(96.0);
        let primary_clicked = skin::button(ui, "modal-primary", primary, tone, [max[0] - MODAL_PAD - primary_w, y], [primary_w, BUTTON_H], enabled, semi);
        (back_clicked, primary_clicked)
    }

    fn focus_modal_field(&self, ui: &Ui) {
        if self.modal_frames == 0 {
            ui.set_keyboard_focus_here();
        }
    }

    fn modal_add_server(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let semi = self.semibold_font;
        let footer_h = 16.0 + 24.0 + 8.0 + BUTTON_H + 16.0;
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Create Your Server",
            Some("Your server is where you and your friends hang out. Make yours and start talking."),
            false,
            64.0 + 16.0,
            footer_h,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let option_min = body;
        let option_max = [body[0] + inner_w, body[1] + 64.0];
        ui.set_cursor_screen_pos(option_min);
        let create = ui.invisible_button("create-my-own", [inner_w, 64.0]);
        let hovered = ui.is_item_hovered();
        skin::rect(ui, option_min, option_max, if hovered { ROW_HOVER } else { STAGE_BG }, 8.0);
        ui.get_window_draw_list()
            .add_rect(option_min, option_max, LIST_DIVIDER)
            .rounding(8.0)
            .thickness(1.0)
            .build();
        skin::image(ui, self.icon("circle-plus"), [option_min[0] + 16.0, option_min[1] + 16.0], 32.0, GREEN);
        skin::text_in_row(ui, semi, FONT_MD, option_min[0] + 64.0, option_min[1], 64.0, TEXT, "Create My Own");
        skin::image(ui, self.icon("chevron-right"), [option_max[0] - 16.0 - 20.0, option_min[1] + 22.0], 20.0, TEXT_SECONDARY);
        let footer_top = max[1] - footer_h;
        skin::text_centered(ui, semi, FONT_LG, (min[0] + max[0]) * 0.5, footer_top + 16.0, TEXT, "Have an invite already?");
        let join = skin::button(
            ui,
            "join-a-server",
            "Join a Server",
            Tone::Secondary,
            [min[0] + MODAL_PAD, footer_top + 16.0 + 24.0 + 8.0],
            [inner_w, BUTTON_H],
            true,
            semi,
        );
        if create {
            if self.server_name.trim().is_empty() {
                self.server_name = format!("{}'s server", self.display_name);
            }
            self.open_modal(Modal::CreateServer);
        } else if join {
            self.open_modal(Modal::JoinServer);
        }
        (min, max)
    }

    fn modal_create_server(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Customize Your Server",
            Some("Give your new server a personality with a name. You can always change it later."),
            false,
            20.0 + INPUT_H + 24.0,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Server name", false, None);
        self.focus_modal_field(ui);
        let enter = skin::input(ui, "##server-name", &mut self.server_name, "", [body[0], body[1] + 20.0], inner_w, INPUT_H, FONT_MD, false);
        let enabled = !self.server_name.trim().is_empty();
        let (back, create) = self.modal_footer(ui, max, min[0], "Back", "Create", Tone::Brand, enabled);
        if back {
            self.open_modal(Modal::AddServer);
        } else if (create || enter) && enabled {
            let name = self.server_name.trim().to_string();
            if let Some(home) = self.note(self.client.create_server(&name)) {
                self.server_name.clear();
                self.modal = None;
                self.refresh_shell();
                self.open_server(home.server.id);
            }
        }
        (min, max)
    }

    fn modal_join_server(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Join a Server",
            Some("Enter an invite below to join an existing server"),
            false,
            20.0 + INPUT_H + 24.0,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Invite link", true, None);
        self.focus_modal_field(ui);
        let enter = skin::input(ui, "##invite-code", &mut self.invite_code, "Invite code", [body[0], body[1] + 20.0], inner_w, INPUT_H, FONT_MD, false);
        let enabled = !self.invite_code.trim().is_empty();
        let (back, join) = self.modal_footer(ui, max, min[0], "Back", "Join Server", Tone::Brand, enabled);
        if back {
            self.open_modal(Modal::AddServer);
        } else if (join || enter) && enabled {
            let code = self.invite_code.trim().to_string();
            if let Some(home) = self.note(self.client.join_invite(&code)) {
                self.invite_code.clear();
                self.modal = None;
                self.refresh_shell();
                self.open_server(home.server.id);
            }
        }
        (min, max)
    }

    fn modal_invite(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let server = match self.place {
            Place::Server(id) => self.servers.iter().find(|server| server.id == id).map(|server| server.name.clone()),
            _ => None,
        }
        .unwrap_or_else(|| "your server".into());
        let channel = self
            .selected
            .as_ref()
            .filter(|channel| channel.kind == ChannelKind::Text)
            .map(|channel| format!("# {}", channel.name));
        let title = format!("Invite friends to {server}");
        let (body, min, max) = self.modal_frame(ui, display, &title, channel.as_deref(), true, 20.0 + INPUT_H + 24.0, 0.0);
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Or, send a server invite link to a friend", false, None);
        let field_min = [body[0], body[1] + 20.0];
        skin::rect(ui, field_min, [field_min[0] + inner_w, field_min[1] + INPUT_H], INPUT_BG, INPUT_RADIUS);
        let mut code = self.shown_invite.clone();
        skin::input(ui, "##invite-link", &mut code, "", field_min, inner_w - 84.0, INPUT_H, FONT_MD, true);
        let copied = self.invite_copied;
        let enabled = !self.shown_invite.is_empty();
        let button_min = [field_min[0] + inner_w - 4.0 - 75.0, field_min[1] + 4.0];
        let tone = if copied { Tone::Green } else { Tone::Brand };
        if skin::button(ui, "copy-invite", if copied { "Copied" } else { "Copy" }, tone, button_min, [75.0, 32.0], enabled, self.semibold_font) {
            crate::platform::copy_to_clipboard(&self.shown_invite);
            self.invite_copied = true;
        }
        (min, max)
    }

    fn modal_nickname(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(ui, display, "Change Nickname", None, false, 20.0 + INPUT_H + 8.0 + 20.0 + 16.0, MODAL_FOOTER_H);
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Nickname", false, None);
        self.focus_modal_field(ui);
        let hint = self.display_name.clone();
        let enter = skin::input(ui, "##nickname", &mut self.nickname, &hint, [body[0], body[1] + 20.0], inner_w, INPUT_H, FONT_MD, false);
        if skin::link(ui, "reset-nickname", "Reset Nickname", [body[0], body[1] + 20.0 + INPUT_H + 8.0], None, FONT_SM, LINK) {
            self.nickname.clear();
        }
        let (cancel, save) = self.modal_footer(ui, max, min[0], "Cancel", "Save", Tone::Brand, true);
        if cancel {
            self.modal = None;
        } else if save || enter {
            if let Place::Server(server_id) = self.place {
                let nickname = {
                    let trimmed = self.nickname.trim();
                    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
                };
                if self.note(self.client.set_nickname(server_id, nickname.as_deref())).is_some() {
                    self.reload_members();
                    self.modal = None;
                }
            }
        }
        (min, max)
    }

    fn modal_create_channel(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let category = self
            .channel_category
            .or_else(|| self.selected.as_ref().map(|channel| channel.category_id))
            .or_else(|| self.layout.first().map(|category| category.category.id));
        let category_name = category
            .and_then(|id| self.layout.iter().find(|layout| layout.category.id == id))
            .map(|layout| format!("in {}", layout.category.name));
        let body_h = 20.0 + 2.0 * 68.0 + 16.0 + 20.0 + INPUT_H + 24.0;
        let (body, min, max) = self.modal_frame(ui, display, "Create Channel", category_name.as_deref(), true, body_h, MODAL_FOOTER_H);
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let semi = self.semibold_font;
        skin::field_label(ui, self.bold_font, body, "Channel type", false, None);
        let mut y = body[1] + 20.0;
        let options = [
            (false, "hash", "Text", "Send messages, images, GIFs, emoji, opinions, and puns"),
            (true, "volume-2", "Voice", "Hang out together with voice, video, and screen share"),
        ];
        for (voice, icon, title, description) in options {
            let option_min = [body[0], y];
            let option_max = [body[0] + inner_w, y + 60.0];
            ui.set_cursor_screen_pos(option_min);
            if ui.invisible_button(format!("channel-type-{title}"), [inner_w, 60.0]) {
                self.channel_is_voice = voice;
            }
            let hovered = ui.is_item_hovered();
            let chosen = self.channel_is_voice == voice;
            let fill = if chosen {
                ROW_SELECTED
            } else if hovered {
                ROW_HOVER
            } else {
                SIDEBAR_BG
            };
            skin::rect(ui, option_min, option_max, fill, 4.0);
            skin::image(ui, self.icon(icon), [option_min[0] + 12.0, option_min[1] + 18.0], 24.0, TEXT_SECONDARY);
            skin::text(ui, semi, FONT_MD, [option_min[0] + 48.0, option_min[1] + 10.0], TEXT_NORMAL, title);
            let line = skin::ellipsize(ui, None, FONT_SM, description, inner_w - 48.0 - 44.0);
            skin::text(ui, None, FONT_SM, [option_min[0] + 48.0, option_min[1] + 32.0], TEXT_SECONDARY, &line);
            let radio = [option_max[0] - 22.0, option_min[1] + 30.0];
            ui.get_window_draw_list().add_circle(radio, 10.0, if chosen { WHITE } else { TEXT_SECONDARY }).thickness(2.0).build();
            if chosen {
                ui.get_window_draw_list().add_circle(radio, 5.0, WHITE).filled(true).build();
            }
            y += 68.0;
        }
        y += 8.0;
        skin::field_label(ui, self.bold_font, [body[0], y], "Channel name", false, None);
        let field_min = [body[0], y + 20.0];
        skin::rect(ui, field_min, [field_min[0] + inner_w, field_min[1] + INPUT_H], INPUT_BG, INPUT_RADIUS);
        let prefix = if self.channel_is_voice { "volume-2" } else { "hash" };
        skin::image(ui, self.icon(prefix), [field_min[0] + 10.0, field_min[1] + 12.0], 16.0, TEXT_NORMAL);
        self.focus_modal_field(ui);
        let enter = skin::input(ui, "##channel-name", &mut self.channel_name, "new-channel", [field_min[0] + 28.0, field_min[1]], inner_w - 28.0, INPUT_H, FONT_MD, false);
        let enabled = !self.channel_name.trim().is_empty();
        let (cancel, create) = self.modal_footer(ui, max, min[0], "Cancel", "Create Channel", Tone::Brand, enabled);
        if cancel {
            self.modal = None;
        } else if (create || enter) && enabled {
            if let (Place::Server(server_id), Some(category_id)) = (self.place, category) {
                let kind = if self.channel_is_voice { ChannelKind::Voice } else { ChannelKind::Text };
                let name = self.channel_name.trim().to_string();
                if self.note(self.client.create_channel(server_id, category_id, &name, kind)).is_some() {
                    self.channel_name.clear();
                    self.modal = None;
                    self.reload_layout();
                }
            } else {
                self.status = "create a category first".into();
            }
        }
        (min, max)
    }

    fn modal_create_category(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(ui, display, "Create Category", None, true, 20.0 + INPUT_H + 24.0, MODAL_FOOTER_H);
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Category name", false, None);
        self.focus_modal_field(ui);
        let enter = skin::input(ui, "##category-name", &mut self.category_name, "New Category", [body[0], body[1] + 20.0], inner_w, INPUT_H, FONT_MD, false);
        let enabled = !self.category_name.trim().is_empty();
        let (cancel, create) = self.modal_footer(ui, max, min[0], "Cancel", "Create Category", Tone::Brand, enabled);
        if cancel {
            self.modal = None;
        } else if (create || enter) && enabled {
            if let Place::Server(server_id) = self.place {
                let name = self.category_name.trim().to_string();
                if self.note(self.client.create_category(server_id, &name)).is_some() {
                    self.category_name.clear();
                    self.modal = None;
                    self.reload_layout();
                }
            }
        }
        (min, max)
    }

    fn finish_voice_flags(&mut self, ui: &Ui) {
        let key_held = self.ptt_down(ui);
        let held = push_to_talk_held(key_held, self.talk_held, ui.io().want_capture_keyboard());
        if self.push_to_talk && held {
            self.ptt_until = Some(Instant::now() + Duration::from_millis(self.voice.release_ms as u64));
        }
        let ptt_hot = held || self.ptt_until.is_some_and(|until| Instant::now() < until);
        let rms = self.media.as_ref().map(|media| media.input_rms()).unwrap_or(0.0);
        let threshold = if self.voice.auto_sensitivity { OPEN_MIC_RMS } else { self.voice.sensitivity };
        let transmitting = speaking_from_level(self.push_to_talk, ptt_hot, rms, threshold);
        let in_place = self.voice_channel.is_some()
            || self.call.as_ref().is_some_and(|call| call.phase == CallPhase::Active);
        if self.media.as_ref().is_some_and(|media| media.take_capture_failure()) {
            self.share.stop();
            self.save_stream();
        }
        self.screen = self.share.is_live() && in_place;
        self.flags = self.mute.voice_flags(self.push_to_talk, transmitting, self.camera && in_place, self.screen);
        if let Some(id) = self.account.as_ref().map(|account| account.id) {
            if let Some(member) = self.voice_members.iter_mut().find(|member| member.user_id == id) {
                member.flags.transmitting = transmitting;
            }
        }
        let preview = self.camera || self.video_popout || self.voice_settings_visible();
        if let Some(media) = &self.media {
            media.set_camera_choice(self.camera_choice.device_index(), preview);
            let pick = if self.share.is_portal() {
                Some(StreamPick::Portal)
            } else {
                self.share.live_target().map(|(kind, id)| StreamPick::Source(kind, id))
            };
            media.set_stream_choice(pick, stream_max_edge(self.stream_resolution), stream_interval_ms(self.stream_fps));
            media.apply_engine(&self.engine_update());
            media.set_mic_test(self.settings.mic_test());
            media.set_state(self.flags, in_place);
            let place = if let Some(channel_id) = self.voice_channel {
                Some(MediaPlace::VoiceChannel(channel_id))
            } else if self.call.as_ref().is_some_and(|call| call.phase == CallPhase::Active) {
                self.call.as_ref().map(|call| MediaPlace::DirectCall(call.dm_id))
            } else {
                None
            };
            let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
            let offerer = self.voice_channel.is_some() || self.call.as_ref().is_some_and(|call| call.caller == me);
            media.set_session(place.filter(|_| in_place), me, offerer);
        }
        if in_place && self.flags != self.sent_flags {
            if self.note(self.client.set_flags(self.flags)).is_some() {
                self.sent_flags = self.flags;
            }
        }
    }

    fn submit_composer(&mut self, target: ChatTarget) {
        let body = self.composer.clone();
        if let Some(id) = self.editing {
            if let Some(message) = self.note(self.client.edit_message(id, &body)) {
                upsert_message(&mut self.messages, message);
                self.composer.clear();
                self.editing = None;
            }
            return;
        }
        let files = self.pending_file.as_ref().map(|(name, bytes)| vec![(name.as_str(), bytes.as_slice())]).unwrap_or_default();
        if let Some(message) = self.note(self.client.send_message(target, &body, self.reply_to, &files)) {
            if self.search.trim().is_empty() {
                upsert_message(&mut self.messages, message);
            }
            self.chat_jump = true;
            self.composer.clear();
            self.reply_to = None;
            self.pending_file = None;
        }
    }

    fn connect_live(&mut self) {
        let Some(token) = self.client.token().map(str::to_string) else { return };
        let realtime = Realtime::connect(websocket_url(&self.base, &token));
        let media = Media::start(realtime.signal_sender());
        self.media = Some(media);
        self.realtime = Some(realtime);
    }

    fn logout(&mut self) {
        let _ = self.client.logout();
        let _ = std::fs::remove_file(session_path(&self.data_dir));
        self.media = None;
        self.realtime = None;
        self.client.set_token(None);
        self.account = None;
        self.messages.clear();
        self.friends.clear();
        self.servers.clear();
        self.status.clear();
        self.settings.close();
        self.share_open = false;
        self.end_media_place();
    }

    fn refresh_shell(&mut self) {
        if let Ok(account) = self.client.me() {
            let revision_changed = self.account.as_ref().is_none_or(|current| {
                current.avatar_rev != account.avatar_rev || current.has_avatar != account.has_avatar
            });
            if revision_changed {
                if let Some(id) = self.account.as_ref().map(|current| current.id) {
                    if let Some(shot) = self.avatars.remove(&id) {
                        self.retired_shots.push(shot);
                    }
                    self.avatar_rev.remove(&id);
                    self.avatar_miss.remove(&id);
                }
            }
            self.account = Some(account);
        }
        if let Some(friends) = self.note(self.client.friends()) {
            self.friends = friends;
        }
        if let Some((incoming, outgoing)) = self.note(self.client.friend_requests()) {
            self.incoming = incoming;
            self.outgoing = outgoing;
        }
        if let Some(servers) = self.note(self.client.servers()) {
            self.servers = servers;
        }
        if let Some(dms) = self.note(self.client.direct_messages()) {
            self.dms = dms;
        }
        if let Some(mentions) = self.note(self.client.mentions()) {
            self.mentions = mentions;
        }
        if let Some(rows) = self.note(self.client.presence()) {
            self.presence = rows.into_iter().collect();
        }
        if let Place::Server(_) = self.place {
            self.reload_layout();
            self.reload_members();
        }
    }

    fn open_server(&mut self, server_id: i64) {
        self.place = Place::Server(server_id);
        self.selected = None;
        self.messages.clear();
        self.search.clear();
        self.reply_to = None;
        self.editing = None;
        self.shown_invite.clear();
        self.reload_layout();
        self.reload_members();
        if let Some(member) = self.members.iter().find(|member| Some(member.user_id) == self.account.as_ref().map(|account| account.id)) {
            self.nickname = member.nickname.clone().unwrap_or_default();
        }
        self.select_first_text_channel();
    }

    fn select_first_text_channel(&mut self) {
        let channel = self
            .layout
            .iter()
            .flat_map(|category| category.channels.iter())
            .find(|channel| channel.kind == ChannelKind::Text)
            .cloned()
            .or_else(|| self.layout.iter().flat_map(|category| category.channels.iter()).next().cloned());
        let Some(channel) = channel else { return };
        let text = channel.kind == ChannelKind::Text;
        self.selected = Some(channel);
        if text {
            self.reload_messages();
        }
    }

    fn open_dm(&mut self, dm_id: i64) {
        self.place = Place::Direct(dm_id);
        self.selected = None;
        self.search.clear();
        self.reply_to = None;
        self.editing = None;
        self.reload_messages();
    }

    fn open_mention(&mut self, mention: &Mention) {
        if self.note(self.client.mark_mention_read(mention.id)).is_some() {
            self.mentions.retain(|item| item.id != mention.id);
        }
        if let Some(channel_id) = mention.channel_id {
            if let Some(channel) = self.find_channel(channel_id) {
                let server_id = channel.server_id;
                self.open_server(server_id);
                self.selected = Some(channel);
                self.reload_messages();
            }
        } else if let Some(dm_id) = mention.dm_id {
            self.open_dm(dm_id);
        }
    }

    fn reload_layout(&mut self) {
        let Place::Server(server_id) = self.place else { return };
        if let Some(layout) = self.note(self.client.layout(server_id)) {
            if let Some(selected) = &self.selected {
                self.selected = layout.iter().flat_map(|category| category.channels.iter()).find(|channel| channel.id == selected.id).cloned();
            }
            self.layout = layout;
        }
    }

    fn reload_members(&mut self) {
        let Place::Server(server_id) = self.place else { return };
        if let Some(members) = self.note(self.client.members(server_id)) {
            for member in &members {
                self.presence.insert(member.user_id, member.presence);
            }
            self.members = members;
        }
    }

    fn reload_messages(&mut self) {
        let Some(target) = self.chat_target() else { return };
        let query = {
            let trimmed = self.search.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        };
        if let Some(messages) = self.note(self.client.messages(target, 100, query.as_deref())) {
            self.messages = messages;
        }
    }

    fn chat_target(&self) -> Option<ChatTarget> {
        match self.place {
            Place::Direct(id) => Some(ChatTarget::Direct(id)),
            Place::Server(_) => self.selected.as_ref().filter(|channel| channel.kind == ChannelKind::Text).map(|channel| ChatTarget::Channel(channel.id)),
            Place::Home => None,
        }
    }

    fn voice_selected(&self) -> bool {
        self.selected.as_ref().map(|channel| channel.kind) == Some(ChannelKind::Voice)
    }

    /// Center stage shows the Discord-style 1:1 call view for the open DM that owns the call.
    fn call_stage_open(&self) -> bool {
        let call_dm = self.call.as_ref().map(|call| call.dm_id);
        let viewing_dm = match self.place {
            Place::Direct(id) => Some(id),
            _ => None,
        };
        dm_call_takes_stage(call_dm, viewing_dm)
    }

    fn find_channel(&self, channel_id: i64) -> Option<Channel> {
        self.layout.iter().flat_map(|category| category.channels.iter()).find(|channel| channel.id == channel_id).cloned()
    }

    fn pump_events(&mut self) {
        let Some(realtime) = &self.realtime else { return };
        let events = realtime.take_events();
        if let Some(error) = realtime.take_error() {
            self.status = error;
        }
        for event in events {
            if let Some(media) = &self.media {
                media.ingest(event.clone());
            }
            self.on_event(event);
        }
    }

    fn on_event(&mut self, event: banter_server::ServerEvent) {
        use banter_server::ServerEvent;
        match event {
            ServerEvent::Presence { user_id, state } => {
                self.presence.insert(user_id, state);
                if let Some(member) = self.members.iter_mut().find(|member| member.user_id == user_id) {
                    member.presence = state;
                }
            }
            ServerEvent::MessageCreated { target, message } | ServerEvent::MessageEdited { target, message } => {
                if self.search.trim().is_empty() && self.chat_target() == Some(target) {
                    upsert_message(&mut self.messages, message);
                }
            }
            ServerEvent::MessageDeleted { target, message_id } => {
                if self.chat_target() == Some(target) {
                    remove_message(&mut self.messages, message_id);
                }
            }
            ServerEvent::FriendRequest { .. } | ServerEvent::FriendsChanged => self.refresh_shell(),
            ServerEvent::Voice { channel_id, members } => {
                if self.voice_channel == Some(channel_id) {
                    self.voice_members = members;
                }
            }
            ServerEvent::Call { call } => {
                let me = self.account.as_ref().map(|account| account.id);
                if Some(call.caller) == me || Some(call.callee) == me {
                    if call.phase == CallPhase::Active {
                        self.voice_channel = None;
                    }
                    let dm_id = call.dm_id;
                    self.call = Some(call);
                    // Open the DM so the one-on-one call stage appears (incoming or accepted).
                    if !matches!(self.place, Place::Direct(id) if id == dm_id) {
                        self.open_dm(dm_id);
                    }
                }
            }
            ServerEvent::CallEnded { dm_id } => {
                if self.call.as_ref().map(|call| call.dm_id) == Some(dm_id) {
                    self.call = None;
                }
            }
            ServerEvent::RtcOffer { .. }
            | ServerEvent::RtcAnswer { .. }
            | ServerEvent::RtcIce { .. }
            | ServerEvent::SubscribeLayer { .. }
            | ServerEvent::WantedLayers { .. } => {}
        }
    }

    fn install_video(&mut self, imgui: &mut Context, frame: VideoFrame) {
        if let Some(sender) = frame.sender {
            self.delivered.insert((sender, frame.kind), frame.layer);
        }
        let shot = if let Some((width, height, rgba)) = frame.rgba.as_ref() {
            upload_rgba(imgui, *width, *height, rgba)
        } else if frame.sender.is_none() && frame.kind == MediaKind::Camera && camera_preview_mirrored() {
            mirrored_camera_shot(imgui, &frame.jpeg)
        } else {
            upload_jpeg(imgui, &frame.jpeg)
        };
        let Some(shot) = shot else { return };
        if frame.kind == MediaKind::Screen {
            self.note_screen_size(shot.size);
        }
        match (frame.sender, frame.kind) {
            (None, MediaKind::Camera) => replace_shot(imgui, &mut self.local_camera, shot),
            (None, MediaKind::Screen) => replace_shot(imgui, &mut self.local_screen, shot),
            (Some(sender), kind) => {
                let key = (sender, video_code(kind));
                if let Some(previous) = self.remote_video.insert(key, shot) {
                    let _ = imgui.remove_texture(previous.texture);
                }
            }
            _ => {}
        }
    }

    fn install_avatars(&mut self, imgui: &mut Context) {
        let mut wanted = Vec::new();
        if let Some(account) = &self.account {
            wanted.push((account.id, account.has_avatar, account.avatar_rev));
        }
        for friend in &self.friends {
            wanted.push((friend.id, friend.has_avatar, friend.avatar_rev));
        }
        for member in &self.members {
            wanted.push((member.user_id, member.has_avatar, member.avatar_rev));
        }
        for chat in &self.dms {
            wanted.push((chat.peer_id, chat.peer_has_avatar, chat.peer_avatar_rev));
        }
        let mut fetched = 0;
        for (user_id, has_avatar, rev) in wanted {
            if !has_avatar {
                if let Some(shot) = self.avatars.remove(&user_id) {
                    self.retired_shots.push(shot);
                }
                self.avatar_rev.remove(&user_id);
                self.avatar_miss.remove(&user_id);
                continue;
            }
            if self.avatar_rev.get(&user_id) == Some(&rev) && self.avatars.contains_key(&user_id) {
                continue;
            }
            if self.avatar_miss.get(&user_id) == Some(&rev) || fetched >= 4 {
                continue;
            }
            fetched += 1;
            if let Some(shot) = self.avatars.remove(&user_id) {
                self.retired_shots.push(shot);
            }
            match self.client.avatar(user_id) {
                Ok(bytes) => {
                    if let Some(shot) = upload_jpeg(imgui, &bytes).or_else(|| upload_rgba_png(imgui, &bytes)) {
                        self.avatars.insert(user_id, shot);
                        self.avatar_rev.insert(user_id, rev);
                        self.avatar_miss.remove(&user_id);
                    } else {
                        self.avatar_miss.insert(user_id, rev);
                    }
                }
                Err(_) => {
                    self.avatar_miss.insert(user_id, rev);
                }
            }
        }
    }

    fn pick_avatar(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
        else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            self.status = "could not read that image".into();
            return;
        };
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            self.status = "avatar is empty or larger than 2 MB".into();
            return;
        }
        let Ok(image) = image::load_from_memory(&bytes) else {
            self.status = "that file is not a png, jpeg, gif, or webp image".into();
            return;
        };
        let rgba = image.to_rgba8();
        self.crop_size = [rgba.width(), rgba.height()];
        self.crop_rgba = rgba.into_raw();
        self.crop_zoom = 1.0;
        self.crop_pan = [0.0, 0.0];
        self.crop_drag = None;
        self.crop_open = true;
    }

    fn save_avatar_crop(&mut self) {
        let edge = 512u32;
        let rgba = crop_avatar_rgba(self.crop_size[0], self.crop_size[1], &self.crop_rgba, self.crop_zoom, self.crop_pan[0], self.crop_pan[1], edge);
        let Some(png) = encode_png(edge, edge, &rgba) else {
            self.status = "could not save that picture".into();
            return;
        };
        if png.len() > MAX_AVATAR_BYTES {
            self.status = "avatar is larger than 2 MB".into();
            return;
        }
        if self.note(self.client.set_avatar(&png)).is_some() {
            self.drop_own_avatar();
            self.crop_open = false;
        }
    }

    fn remove_avatar(&mut self) {
        if self.note(self.client.clear_avatar()).is_some() {
            self.drop_own_avatar();
        }
    }

    fn drop_own_avatar(&mut self) {
        if let Some(id) = self.account.as_ref().map(|account| account.id) {
            if let Some(shot) = self.avatars.remove(&id) {
                self.retired_shots.push(shot);
            }
            self.avatar_rev.remove(&id);
            self.avatar_miss.remove(&id);
        }
        if let Some(account) = self.note(self.client.me()) {
            self.account = Some(account);
        }
    }

    fn draw_avatar_crop(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION | WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_DOCKING;
        ui.window("Avatar crop")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .bg_alpha(0.72)
            .build(|| {
                let panel = [420.0, 460.0];
                let origin = [(size[0] - panel[0]) * 0.5, (size[1] - panel[1]) * 0.5];
                ui.get_window_draw_list()
                    .add_rect(origin, [origin[0] + panel[0], origin[1] + panel[1]], STAGE_BG)
                    .rounding(8.0)
                    .filled(true)
                    .build();
                ui.set_cursor_screen_pos([origin[0] + 20.0, origin[1] + 16.0]);
                ui.text("Profile picture");
                let center = [origin[0] + panel[0] * 0.5, origin[1] + 180.0];
                self.paint_crop_preview(ui, center, 96.0);
                ui.set_cursor_screen_pos([origin[0] + 24.0, origin[1] + 300.0]);
                ui.set_next_item_width(panel[0] - 48.0);
                ui.slider_f32("Zoom", &mut self.crop_zoom, 1.0, 3.0);
                ui.set_cursor_screen_pos([origin[0] + 24.0, origin[1] + 360.0]);
                if ui.button("Save") {
                    self.save_avatar_crop();
                }
                ui.same_line();
                if ui.button("Cancel") {
                    self.crop_open = false;
                }
                let drag = pointer_in(ui, [center[0] - 96.0, center[1] - 96.0], [center[0] + 96.0, center[1] + 96.0]);
                if drag && ui.is_mouse_down(MouseButton::Left) {
                    let mouse = ui.io().mouse_pos();
                    if let Some(previous) = self.crop_drag {
                        let window = crop_window(self.crop_size[0], self.crop_size[1], self.crop_zoom, 0.0, 0.0)
                            .map(|(_, _, side)| side)
                            .unwrap_or(1.0);
                        let scale = window / 192.0;
                        self.crop_pan[0] = (self.crop_pan[0] - (mouse[0] - previous[0]) * scale / (self.crop_size[0] as f32).max(1.0)).clamp(-1.0, 1.0);
                        self.crop_pan[1] = (self.crop_pan[1] - (mouse[1] - previous[1]) * scale / (self.crop_size[1] as f32).max(1.0)).clamp(-1.0, 1.0);
                    }
                    self.crop_drag = Some(mouse);
                } else {
                    self.crop_drag = None;
                }
            });
    }

    fn paint_crop_preview(&self, ui: &Ui, center: [f32; 2], radius: f32) {
        let avatar = self.crop_preview.as_ref().map(|shot| shot.texture);
        paint_avatar(ui, avatar, center, radius, "?", None);
    }

    fn refresh_crop_preview(&mut self, imgui: &mut Context) {
        if !self.crop_open {
            if let Some(shot) = self.crop_preview.take() {
                self.retired_shots.push(shot);
            }
            return;
        }
        let key = [
            (self.crop_zoom * 100.0) as i32,
            (self.crop_pan[0] * 100.0) as i32,
            (self.crop_pan[1] * 100.0) as i32,
        ];
        if self.crop_preview.is_some() && self.crop_preview_key == key {
            return;
        }
        if let Some(shot) = self.crop_preview.take() {
            self.retired_shots.push(shot);
        }
        let rgba = crop_avatar_rgba(
            self.crop_size[0],
            self.crop_size[1],
            &self.crop_rgba,
            self.crop_zoom,
            self.crop_pan[0],
            self.crop_pan[1],
            160,
        );
        if let Some(shot) = upload_rgba(imgui, 160, 160, &rgba) {
            self.crop_preview = Some(shot);
            self.crop_preview_key = key;
        }
    }

    fn pick_attachment(&mut self) {
        let Some(path) = rfd::FileDialog::new().pick_file() else { return };
        let Ok(bytes) = std::fs::read(&path) else {
            self.status = "could not read that file".into();
            return;
        };
        if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_BYTES {
            self.status = "file is empty or larger than 10 MB".into();
            return;
        }
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("file").to_string();
        self.pending_file = Some((name, bytes));
    }

    fn save_attachment(&mut self, id: i64, filename: &str) {
        let Some(bytes) = self.note(self.client.attachment(id)) else { return };
        let Some(path) = rfd::FileDialog::new().set_file_name(filename).save_file() else { return };
        if let Err(err) = std::fs::write(path, bytes) {
            self.status = err.to_string();
        }
    }

    fn name_of(&self, user_id: i64) -> String {
        if let Some(member) = self.members.iter().find(|member| member.user_id == user_id) {
            return member.nickname.clone().unwrap_or_else(|| member.display_name.clone());
        }
        if let Some(account) = self.account.as_ref().filter(|account| account.id == user_id) {
            return account.display_name.clone();
        }
        if let Some(friend) = self.friends.iter().find(|friend| friend.id == user_id) {
            return friend.display_name.clone();
        }
        if let Some(chat) = self.dms.iter().find(|chat| chat.peer_id == user_id) {
            return chat.peer_display_name.clone();
        }
        format!("user {user_id}")
    }

    fn icon(&self, name: &str) -> Option<ManagedTextureId> {
        self.icons.get(name).map(|shot| shot.texture)
    }

    fn tool(&self, ui: &Ui, id: &str, name: &str, active: bool, tip: Option<&str>) -> bool {
        let size = 32.0;
        let clicked = ui.invisible_button(id, [size, size]);
        let min = ui.item_rect_min();
        let draw = ui.get_window_draw_list();
        if active || ui.is_item_hovered() {
            let fill = if active { [88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, 0.45] } else { ROW_HOVER };
            draw.add_rect(min, [min[0] + size, min[1] + size], fill).rounding(6.0).filled(true).build();
        }
        if let Some(texture) = self.icon(name) {
            let pad = 7.0;
            draw.add_image(
                texture,
                [min[0] + pad, min[1] + pad],
                [min[0] + size - pad, min[1] + size - pad],
                [0.0, 0.0],
                [1.0, 1.0],
                WHITE,
            );
        }
        if let Some(tip) = tip {
            if ui.is_item_hovered() {
                skin::item_tooltip(ui, self.semibold_font, tip);
            }
        }
        clicked
    }

    fn open_settings(&mut self) {
        self.settings.open();
        self.settings_error.clear();
        self.display_name = self.account.as_ref().map(|account| account.display_name.clone()).unwrap_or_default();
        if self.settings.section() == SettingsSection::Voice {
            self.devices_ready = false;
        }
        if !self.cameras_loaded {
            self.refresh_cameras();
        }
    }

    fn select_settings(&mut self, section: SettingsSection) {
        if section == self.settings.section() {
            return;
        }
        self.settings.select(section);
        self.settings_error.clear();
        if section == SettingsSection::Voice {
            // Devices plugged in while settings was closed show up when the page opens.
            self.devices_ready = false;
            self.refresh_cameras();
        }
    }

    fn voice_settings_visible(&self) -> bool {
        self.settings.is_open() && self.settings.section() == SettingsSection::Voice
    }

    fn leave_voice(&mut self) {
        if self.note(self.client.leave_voice()).is_some() {
            self.voice_channel = None;
            self.voice_members.clear();
            self.end_media_place();
        }
    }

    fn end_media_place(&mut self) {
        self.call = None;
        self.camera = false;
        self.screen = false;
        self.share.stop();
    }

    fn open_share(&mut self) {
        self.share_open = true;
        self.reload_share_sources();
    }

    fn refresh_cameras(&mut self) {
        self.cameras = media::camera_names();
        self.cameras_loaded = true;
        if !self.cameras.iter().any(|(index, _)| *index == self.camera_choice.device_index()) {
            if let Some((index, _)) = self.cameras.first() {
                self.camera_choice.select(*index);
            }
        }
    }

    fn ensure_icons(&mut self, imgui: &mut Context) {
        if self.icons_ready {
            return;
        }
        for name in [
            "mic", "mic-off", "headphones", "headphone-off", "video", "video-off", "monitor", "monitor-off",
            "phone-off", "settings", "chevron-down", "chevron-up", "plus", "hash", "volume-2", "users", "search", "paperclip",
            "x", "user", "log-out", "pencil", "trash-2", "reply", "check", "chevron-right", "circle-plus", "folder-plus",
            "message-circle", "phone", "user-plus", "signal", "ellipsis",
        ] {
            if let Ok(rgba) = rasterize_lucide(name, 32, [255, 255, 255]) {
                if let Some(shot) = upload_rgba(imgui, 32, 32, &rgba) {
                    self.icons.insert(name.to_string(), shot);
                }
            }
        }
        for name in ["mic-off", "headphone-off", "phone-off"] {
            if let Ok(rgba) = rasterize_lucide(name, 32, [242, 63, 66]) {
                if let Some(shot) = upload_rgba(imgui, 32, 32, &rgba) {
                    self.icons.insert(format!("{name}-red"), shot);
                }
            }
        }
        if let Ok(rgba) = rasterize_lucide("crown", 32, [240, 178, 50]) {
            if let Some(shot) = upload_rgba(imgui, 32, 32, &rgba) {
                self.icons.insert("crown".to_string(), shot);
            }
        }
        if let Ok(rgba) = rasterize_lucide("signal", 32, [35, 165, 89]) {
            if let Some(shot) = upload_rgba(imgui, 32, 32, &rgba) {
                self.icons.insert("signal-green".to_string(), shot);
            }
        }
        self.icons_ready = true;
    }

    fn flush_shots(&mut self, imgui: &mut Context) {
        while let Ok((code, id, jpeg)) = self.thumb_rx.try_recv() {
            self.thumb_busy = false;
            match jpeg {
                Some(bytes) => self.thumb_queue.push((code, id, bytes)),
                None => {
                    self.share_miss.insert((code, id));
                }
            }
        }
        for shot in self.retired_shots.drain(..) {
            let _ = imgui.remove_texture(shot.texture);
        }
        let queue = std::mem::take(&mut self.thumb_queue);
        for (code, id, jpeg) in queue {
            if let Some(shot) = upload_jpeg(imgui, &jpeg) {
                if let Some(previous) = self.share_thumbs.insert((code, id), shot) {
                    let _ = imgui.remove_texture(previous.texture);
                }
            } else {
                self.share_miss.insert((code, id));
            }
        }
    }

    fn draw_settings(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let layout = settings_layout(size[0], size[1]);
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _rounding = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, STAGE_BG);
        let mut close = false;
        ui.window("Settings")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                // Read before drawing: a dropdown that closes on this Escape is still open here.
                self.settings_popup_open =
                    ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP) || ui.is_any_item_active();
                ui.set_cursor_pos([layout.nav_x, 0.0]);
                let nav_bg = ui.push_style_color(StyleColor::ChildBg, SIDEBAR_BG);
                ui.child_window("settings-nav").size([layout.nav_w, size[1]]).build(ui, || {
                    self.draw_settings_nav(ui, layout.nav_w);
                });
                drop(nav_bg);
                ui.set_cursor_pos([layout.body_x, 0.0]);
                let body_bg = ui.push_style_color(StyleColor::ChildBg, STAGE_BG);
                ui.child_window("settings-body").size([layout.body_w, size[1]]).build(ui, || {
                    let window_pos = ui.window_pos();
                    ui.set_cursor_pos([layout.content_x, SETTINGS_TOP_PAD]);
                    ui.indent_by(layout.content_x);
                    let _wrap = ui.push_text_wrap_pos(layout.content_x + layout.content_w);
                    match self.settings.section() {
                        SettingsSection::Account => self.draw_account_settings(ui, layout.content_w),
                        SettingsSection::Voice => self.draw_voice_settings(ui, layout.content_w),
                        SettingsSection::Developer => self.draw_developer_settings(ui, layout.content_w),
                    }
                    ui.dummy([layout.content_w, 80.0]);
                    ui.unindent_by(layout.content_x);
                    // Screen position, so the button stays put while the page scrolls.
                    ui.set_cursor_screen_pos([window_pos[0] + layout.close_x - layout.body_x, window_pos[1] + layout.close_y]);
                    if self.settings_close_button(ui, layout.close_size) {
                        close = true;
                    }
                });
                drop(body_bg);
            });
        if close {
            self.settings.close();
        }
    }

    fn draw_settings_nav(&mut self, ui: &Ui, nav_w: f32) {
        let list_x = (nav_w - SETTINGS_NAV_LIST_W).max(0.0) + 20.0;
        let list_w = (nav_w - list_x - 6.0).max(32.0);
        ui.set_cursor_pos([list_x, SETTINGS_TOP_PAD]);
        ui.indent_by(list_x);
        ui.child_window("settings-nav-list").size([list_w, 0.0]).build(ui, || {
            settings_form_label(ui, self.bold_font, "USER SETTINGS");
            for section in [SettingsSection::Account, SettingsSection::Voice] {
                let (id, icon, label) = match section {
                    SettingsSection::Account => ("settings-account", "user", "My Account"),
                    SettingsSection::Voice => ("settings-voice", "headphones", "Voice & Video"),
                    SettingsSection::Developer => unreachable!("developer is under its own heading"),
                };
                if nav_row(ui, id, self.icon(icon), label, self.settings.section() == section) {
                    self.select_settings(section);
                }
                ui.dummy([0.0, 2.0]);
            }
            settings_divider(ui, ui.content_region_avail()[0]);
            settings_form_label(ui, self.bold_font, "DEVELOPER");
            if nav_row(
                ui,
                "settings-developer",
                self.icon("signal"),
                "Developer",
                self.settings.section() == SettingsSection::Developer,
            ) {
                self.select_settings(SettingsSection::Developer);
            }
            ui.dummy([0.0, 2.0]);
            settings_divider(ui, ui.content_region_avail()[0]);
            if nav_row(ui, "settings-logout", self.icon("log-out"), "Log Out", false) {
                self.logout();
            }
        });
        ui.unindent_by(list_x);
    }

    fn settings_close_button(&self, ui: &Ui, size: f32) -> bool {
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button("close-settings", [size, size + 22.0]);
        let hovered = ui.is_item_hovered();
        let draw = ui.get_window_draw_list();
        let center = [origin[0] + size * 0.5, origin[1] + size * 0.5];
        let ring = if hovered { TEXT } else { MUTED };
        if hovered {
            draw.add_circle(center, size * 0.5, ROW_HOVER).filled(true).build();
        }
        draw.add_circle(center, size * 0.5 - 1.0, ring).thickness(2.0).build();
        if let Some(texture) = self.icon("x") {
            draw.add_image(texture, [center[0] - 9.0, center[1] - 9.0], [center[0] + 9.0, center[1] + 9.0], [0.0, 0.0], [1.0, 1.0], ring);
        }
        let _font = ui.push_font_with_size(self.bold_font, 13.0);
        let label_w = ui.calc_text_size("ESC")[0];
        draw.add_text([center[0] - label_w * 0.5, origin[1] + size + 6.0], MUTED, "ESC");
        clicked
    }

    fn draw_developer_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "Developer");
        ui.text_colored(MUTED, "Choose which Banter server this client talks to.");
        settings_spacer(ui);

        settings_form_label(ui, self.bold_font, "SERVER");
        let local_selected = self.dev_server.target == ServerTarget::Local;
        if settings_radio_row(ui, "dev-server-local", "Local", local_selected, width) && !local_selected {
            self.dev_server.target = ServerTarget::Local;
            self.apply_dev_server();
        }
        let remote_selected = self.dev_server.target == ServerTarget::Remote;
        if settings_radio_row(ui, "dev-server-remote", "Remote", remote_selected, width) && !remote_selected
        {
            self.dev_server.target = ServerTarget::Remote;
            self.apply_dev_server();
        }
        settings_spacer(ui);

        settings_form_label(ui, self.bold_font, "REMOTE HOST");
        ui.text_colored(
            MUTED,
            "Host, host:port, or http(s) URL. Saved under this PC's Banter data folder.",
        );
        settings_spacer(ui);
        let apply_label = "Apply";
        let apply_w = ui.calc_text_size(apply_label)[0] + 24.0;
        let field_w = (width - apply_w - 8.0).max(120.0);
        ui.set_next_item_width(field_w);
        let entered = ui
            .input_text("##dev-remote-host", &mut self.dev_server.remote_host)
            .enter_returns_true(true)
            .build();
        ui.same_line();
        let apply_clicked = {
            let _disabled = ui.begin_disabled_with_cond(self.dev_server.target != ServerTarget::Remote);
            ui.button(apply_label)
        };
        if (entered || apply_clicked) && self.dev_server.target == ServerTarget::Remote {
            self.apply_dev_server();
        }
        settings_spacer(ui);

        settings_form_label(ui, self.bold_font, "ACTIVE API BASE");
        ui.text(&self.base);
        settings_spacer(ui);
        ui.text_colored(MUTED, &format!("Local default is {}.", local_api_base()));

        if !self.settings_error.is_empty() {
            settings_spacer(ui);
            ui.text_colored(RED, &self.settings_error);
        }
    }

    fn draw_account_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "My Account");
        let Some(account) = self.account.clone() else { return };
        let avatar = self.avatars.get(&account.id).map(|shot| shot.texture);

        // Profile card: banner, avatar, and name, then the editable fields on an inner panel.
        let card = ui.cursor_screen_pos();
        let banner_h = 100.0;
        let header_h = 72.0;
        let fields_h = 190.0;
        let card_h = banner_h + header_h + fields_h + 16.0;
        let avatar_center = [card[0] + 16.0 + 46.0, card[1] + banner_h + 8.0];
        let panel = [card[0] + 16.0, card[1] + banner_h + header_h];
        let panel_w = width - 32.0;
        {
            let draw = ui.get_window_draw_list();
            draw.add_rect(card, [card[0] + width, card[1] + card_h], RAIL_BG).rounding(8.0).filled(true).build();
            draw.add_rect(card, [card[0] + width, card[1] + banner_h], BLURPLE)
                .rounding(8.0)
                .flags(DrawFlags::ROUND_TOP)
                .filled(true)
                .build();
            draw.add_circle(avatar_center, 46.0, RAIL_BG).filled(true).build();
            draw.add_rect(panel, [panel[0] + panel_w, panel[1] + fields_h], SIDEBAR_BG).rounding(8.0).filled(true).build();
            let _font = ui.push_font_with_size(self.bold_font, 20.0);
            draw.add_text([card[0] + 124.0, card[1] + banner_h + 14.0], TEXT, &account.display_name);
        }
        paint_avatar(ui, avatar, avatar_center, 40.0, &initials(&account.display_name), None);
        let button_label = "Change Avatar";
        let button_w = ui.calc_text_size(button_label)[0] + 24.0;
        ui.set_cursor_screen_pos([card[0] + width - 16.0 - button_w, card[1] + banner_h + 14.0]);
        if ui.button(button_label) {
            self.status.clear();
            self.pick_avatar();
            if !self.status.is_empty() {
                self.settings_error = std::mem::take(&mut self.status);
            }
        }
        ui.same_line();
        if ui.button("Remove Avatar") {
            self.remove_avatar();
            if !self.status.is_empty() {
                self.settings_error = std::mem::take(&mut self.status);
            }
        }

        let inner_x = panel[0] + 16.0;
        let inner_w = panel_w - 32.0;
        ui.set_cursor_screen_pos([inner_x, panel[1] + 16.0]);
        settings_form_label(ui, self.bold_font, "DISPLAY NAME");
        let save_label = "Save";
        let save_w = ui.calc_text_size(save_label)[0] + 24.0;
        let reset_label = "Reset";
        let reset_w = ui.calc_text_size(reset_label)[0] + 24.0;
        let field_w = (inner_w - save_w - reset_w - 16.0).max(80.0);
        ui.set_cursor_screen_pos([inner_x, ui.cursor_screen_pos()[1]]);
        ui.set_next_item_width(field_w);
        let entered = ui
            .input_text("##display-name", &mut self.display_name)
            .enter_returns_true(true)
            .build();
        let dirty = display_name_dirty(&self.display_name, &account.display_name);
        let problem = display_name_error(&self.display_name);
        ui.same_line();
        let save_clicked = {
            let _disabled = ui.begin_disabled_with_cond(!dirty || problem.is_some());
            ui.button(save_label)
        };
        ui.same_line();
        let reset_clicked = {
            let _disabled = ui.begin_disabled_with_cond(!dirty);
            ui.button(reset_label)
        };
        if reset_clicked {
            self.display_name = account.display_name.clone();
            self.settings_error.clear();
        }
        if (save_clicked || entered) && dirty && problem.is_none() {
            let name = self.display_name.trim().to_string();
            match self.client.set_display_name(&name) {
                Ok(saved) => {
                    self.display_name = saved.display_name.clone();
                    self.account = Some(saved);
                    self.settings_error.clear();
                }
                Err(err) => self.settings_error = err.to_string(),
            }
        }
        ui.set_cursor_screen_pos([inner_x, ui.cursor_screen_pos()[1] + 4.0]);
        if let Some(problem) = problem.filter(|_| dirty) {
            ui.text_colored(RED, problem);
        } else if dirty {
            ui.text_colored(MUTED, "Press Save or Enter to keep this name.");
        } else {
            ui.dummy([0.0, ui.text_line_height()]);
        }
        ui.set_cursor_screen_pos([inner_x, ui.cursor_screen_pos()[1] + 12.0]);
        settings_form_label(ui, self.bold_font, "USERNAME");
        ui.set_cursor_screen_pos([inner_x, ui.cursor_screen_pos()[1]]);
        ui.text(format!("@{}", account.username));

        ui.set_cursor_screen_pos([card[0], card[1] + card_h + 16.0]);
        if !self.settings_error.is_empty() {
            ui.text_colored(RED, &self.settings_error);
        }
    }

    fn draw_voice_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "Voice & Video");
        self.ensure_audio_devices();
        let gap = 16.0;
        let two_columns = width >= 460.0;
        let column_w = if two_columns { (width - gap) * 0.5 } else { width };
        let row = ui.cursor_pos();

        settings_form_label(ui, self.bold_font, "INPUT DEVICE");
        self.draw_named_combo(ui, "input-device", "microphone", column_w, &self.audio_inputs.clone(), true);
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "INPUT VOLUME");
        if settings_volume_slider(ui, "input-volume", &mut self.voice.input_volume, column_w) {
            self.save_voice();
        }
        if two_columns {
            let left_end = ui.cursor_pos()[1];
            ui.set_cursor_pos([row[0] + column_w + gap, row[1]]);
            ui.group(|| {
                settings_form_label(ui, self.bold_font, "OUTPUT DEVICE");
                self.draw_named_combo(ui, "output-device", "speaker", column_w, &self.audio_outputs.clone(), false);
                settings_spacer(ui);
                settings_form_label(ui, self.bold_font, "OUTPUT VOLUME");
                if settings_volume_slider(ui, "output-volume", &mut self.voice.output_volume, column_w) {
                    self.save_voice();
                }
            });
            ui.set_cursor_pos([row[0], left_end.max(ui.cursor_pos()[1])]);
        } else {
            settings_spacer(ui);
            settings_form_label(ui, self.bold_font, "OUTPUT DEVICE");
            self.draw_named_combo(ui, "output-device", "speaker", column_w, &self.audio_outputs.clone(), false);
            settings_spacer(ui);
            settings_form_label(ui, self.bold_font, "OUTPUT VOLUME");
            if settings_volume_slider(ui, "output-volume", &mut self.voice.output_volume, column_w) {
                self.save_voice();
            }
        }
        settings_spacer(ui);
        let device_errors = self.media.as_ref().map(|media| media.errors()).unwrap_or_default();
        for error in &device_errors {
            ui.text_colored(RED, device_error_text(error));
        }
        if !device_errors.is_empty() {
            settings_spacer(ui);
        }

        settings_form_label(ui, self.bold_font, "MIC TEST");
        ui.text_colored(MUTED, "Having mic issues? Start a test and say something. Banter plays your voice back to you.");
        settings_spacer(ui);
        let testing = self.settings.mic_test();
        let test_label = if testing { "Stop Testing" } else { "Let's Check" };
        let test_w = ui.calc_text_size("Stop Testing")[0] + 24.0;
        let test_color = ui.push_style_color(StyleColor::Button, if testing { RED } else { BLURPLE });
        let pressed = ui.button_with_size(test_label, [test_w, 0.0]);
        drop(test_color);
        if pressed {
            self.settings.toggle_mic_test();
        }
        ui.same_line_with_spacing(0.0, 12.0);
        let meter_w = (width - test_w - 12.0).max(40.0);
        let threshold = if self.voice.auto_sensitivity || self.push_to_talk { None } else { Some(self.voice.sensitivity) };
        self.draw_input_meter(ui, meter_w, 36.0, threshold);
        settings_spacer(ui);
        settings_divider(ui, width);

        settings_form_label(ui, self.bold_font, "INPUT MODE");
        if settings_radio_row(ui, "mode-activity", "Voice Activity", !self.push_to_talk, width) && self.push_to_talk {
            self.push_to_talk = false;
            self.voice.push_to_talk = false;
            self.ptt_until = None;
            self.settings.finish_recording();
            self.save_voice();
        }
        if settings_radio_row(ui, "mode-ptt", "Push to Talk", self.push_to_talk, width) && !self.push_to_talk {
            self.push_to_talk = true;
            self.voice.push_to_talk = true;
            self.save_voice();
        }
        settings_spacer(ui);
        if self.push_to_talk {
            self.draw_push_to_talk_settings(ui, two_columns, column_w, gap);
        } else {
            self.draw_sensitivity_settings(ui, width);
        }
        settings_divider(ui, width);
        settings_spacer(ui);

        if settings_panel(ui, "Voice processing") {
            if settings_toggle_row(ui, "echo-cancellation", "Echo Cancellation", &mut self.voice.echo_cancellation, width) {
                self.save_voice();
            }
            settings_divider(ui, width);
            if settings_toggle_row(ui, "noise-suppression", "Noise Suppression", &mut self.voice.noise_suppression, width) {
                self.save_voice();
            }
            settings_divider(ui, width);
            if settings_toggle_row(ui, "auto-gain", "Automatic Gain Control", &mut self.voice.auto_gain, width) {
                self.save_voice();
            }
            settings_spacer(ui);
        }
        if settings_panel(ui, "Video") {
            self.draw_display_settings(ui, width);
            settings_spacer(ui);
        }
        if settings_panel(ui, "Camera") {
            self.draw_camera_picker(ui, width);
            settings_spacer(ui);
        }
        if settings_panel(ui, "Screen share") {
            self.draw_stream_quality(ui, width);
            settings_spacer(ui);
            self.draw_audio_sources(ui, width);
            settings_spacer(ui);
            ui.text_colored(
                MUTED,
                "Open Share Your Screen from a voice channel or a call. Pick a screen or an application, then press Go Live.",
            );
            settings_spacer(ui);
        }
    }

    fn draw_sensitivity_settings(&mut self, ui: &Ui, width: f32) {
        settings_form_label(ui, self.bold_font, "INPUT SENSITIVITY");
        if settings_toggle_row(
            ui,
            "sensitivity-auto",
            "Automatically determine input sensitivity",
            &mut self.voice.auto_sensitivity,
            width,
        ) {
            self.save_voice();
        }
        if self.voice.auto_sensitivity {
            self.draw_input_meter(ui, width, 12.0, None);
        } else {
            let mut db = rms_to_db(self.voice.sensitivity).round() as i32;
            if settings_slider_i32(ui, "##sensitivity", &mut db, -100, 0, width, " dB") {
                self.voice.sensitivity = db_to_rms(db as f32);
            }
            if ui.is_item_deactivated_after_edit() {
                self.save_voice();
            }
            self.draw_input_meter(ui, slider_track_width(width), 12.0, Some(self.voice.sensitivity));
            ui.text_colored(MUTED, "Your voice is sent when the bar passes the white mark.");
        }
        settings_spacer(ui);
    }

    fn draw_push_to_talk_settings(&mut self, ui: &Ui, two_columns: bool, column_w: f32, gap: f32) {
        let row = ui.cursor_pos();
        settings_form_label(ui, self.bold_font, "SHORTCUT");
        self.draw_keybind(ui, column_w);
        if two_columns {
            let left_end = ui.cursor_pos()[1];
            ui.set_cursor_pos([row[0] + column_w + gap, row[1]]);
            ui.group(|| self.draw_release_delay(ui, column_w));
            ui.set_cursor_pos([row[0], left_end.max(ui.cursor_pos()[1])]);
        } else {
            settings_spacer(ui);
            self.draw_release_delay(ui, column_w);
        }
        settings_spacer(ui);
    }

    fn draw_release_delay(&mut self, ui: &Ui, width: f32) {
        settings_form_label(ui, self.bold_font, "PUSH TO TALK RELEASE DELAY");
        let mut release = self.voice.release_ms.min(2000) as i32;
        if settings_slider_i32(ui, "##release-delay", &mut release, 0, 2000, width, " ms") {
            self.voice.release_ms = release.max(0) as u32;
        }
        if ui.is_item_deactivated_after_edit() {
            self.save_voice();
        }
    }

    fn draw_keybind(&mut self, ui: &Ui, width: f32) {
        let recording = self.settings.recording();
        let origin = ui.cursor_screen_pos();
        let height = 40.0;
        let button_label = if recording { "Stop Recording" } else { "Edit Keybind" };
        let button_w = ui.calc_text_size("Stop Recording")[0] + 24.0;
        let clicked = ui.invisible_button("ptt-keybind", [width, height]);
        let hovered = ui.is_item_hovered();
        let draw = ui.get_window_draw_list();
        let outline = if recording { RED } else if hovered { MUTED } else { [0.0, 0.0, 0.0, 0.0] };
        draw.add_rect(origin, [origin[0] + width, origin[1] + height], RAIL_BG).rounding(4.0).filled(true).build();
        draw.add_rect(origin, [origin[0] + width, origin[1] + height], outline).rounding(4.0).thickness(1.0).build();
        let key_text = if recording { "Press a keyΓÇª".to_string() } else { self.voice.ptt_key.clone() };
        let text_h = ui.calc_text_size(&key_text)[1];
        draw.add_text([origin[0] + 12.0, origin[1] + (height - text_h) * 0.5], if recording { RED } else { TEXT }, &key_text);
        let button_min = [origin[0] + width - button_w - 4.0, origin[1] + 4.0];
        let button_max = [origin[0] + width - 4.0, origin[1] + height - 4.0];
        draw.add_rect(button_min, button_max, if recording { RED } else { ROW_SELECTED }).rounding(4.0).filled(true).build();
        let label_size = ui.calc_text_size(button_label);
        draw.add_text(
            [button_min[0] + (button_w - label_size[0]) * 0.5, origin[1] + (height - label_size[1]) * 0.5],
            WHITE,
            button_label,
        );
        if clicked {
            if recording {
                self.settings.finish_recording();
            } else {
                self.settings.start_recording();
            }
        } else if recording {
            self.capture_ptt_key(ui);
        }
        if recording {
            ui.text_colored(MUTED, "Press Escape to cancel.");
        }
    }

    fn draw_camera_picker(&mut self, ui: &Ui, width: f32) {
        if !self.cameras_loaded {
            self.refresh_cameras();
        }
        settings_form_label(ui, self.bold_font, "CAMERA");
        if self.cameras.is_empty() {
            ui.text_colored(MUTED, "No camera is available.");
        } else {
            let labels: Vec<String> = self.cameras.iter().map(|(_, name)| name.clone()).collect();
            let mut index = self
                .cameras
                .iter()
                .position(|(id, _)| *id == self.camera_choice.device_index())
                .unwrap_or(0);
            if settings_combo(ui, "##camera", width.min(360.0), &labels, &mut index) {
                let (device, name) = self.cameras[index].clone();
                self.camera_choice.select(device);
                self.voice.camera = name;
                self.save_voice();
            }
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "PREVIEW");
        let box_w = width.min(400.0);
        let box_h = box_w * 9.0 / 16.0;
        let origin = ui.cursor_screen_pos();
        ui.dummy([box_w, box_h]);
        let draw = ui.get_window_draw_list();
        draw.add_rect(origin, [origin[0] + box_w, origin[1] + box_h], RAIL_BG).rounding(8.0).filled(true).build();
        if let Some(shot) = &self.local_camera {
            let fitted = fit_inside(shot.size, box_w, box_h);
            let min = [origin[0] + (box_w - fitted[0]) * 0.5, origin[1] + (box_h - fitted[1]) * 0.5];
            draw.add_image(shot.texture, min, [min[0] + fitted[0], min[1] + fitted[1]], [0.0, 0.0], [1.0, 1.0], WHITE);
        } else {
            let text = if self.cameras.is_empty() { "No camera" } else { "Starting cameraΓÇª" };
            let size = ui.calc_text_size(text);
            draw.add_text([origin[0] + (box_w - size[0]) * 0.5, origin[1] + (box_h - size[1]) * 0.5], MUTED, text);
        }
    }

    fn draw_stream_quality(&mut self, ui: &Ui, width: f32) {
        let (height, hz) = self.selected_capture_limits();
        settings_form_label(ui, self.bold_font, "RESOLUTION");
        let resolutions = stream_resolutions();
        let mut reasons = Vec::new();
        let choices: Vec<(String, String, bool, bool)> = resolutions
            .iter()
            .map(|resolution| {
                let blocked = resolution_block(*resolution, height);
                if let Some(reason) = &blocked {
                    reasons.push(reason.clone());
                }
                (
                    format!("res-{}", resolution_key(*resolution)),
                    stream_resolution_label(*resolution).to_string(),
                    self.stream_resolution == *resolution,
                    blocked.is_none(),
                )
            })
            .collect();
        if let Some(index) = settings_chip_row(ui, &choices, width) {
            if self.stream_resolution != resolutions[index] {
                self.stream_resolution = resolutions[index];
                self.save_stream();
            }
        }
        for reason in &reasons {
            ui.text_colored(MUTED, reason);
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "FRAME RATE");
        let rates = stream_frame_rates();
        let mut reasons = Vec::new();
        let choices: Vec<(String, String, bool, bool)> = rates
            .iter()
            .map(|fps| {
                let blocked = fps_block(*fps, hz);
                if let Some(reason) = &blocked {
                    reasons.push(reason.clone());
                }
                (format!("fps-{}", fps_key(*fps)), stream_fps_label(*fps).to_string(), self.stream_fps == *fps, blocked.is_none())
            })
            .collect();
        if let Some(index) = settings_chip_row(ui, &choices, width) {
            if self.stream_fps != rates[index] {
                self.stream_fps = rates[index];
                self.save_stream();
            }
        }
        for reason in &reasons {
            ui.text_colored(MUTED, reason);
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "ENCODER");
        if let Some(reason) = hardware_encoder_block(self.stream.hardware) {
            ui.text_colored(MUTED, reason);
        } else {
            ui.text_colored(MUTED, "Hardware H.264 when this PC can encode Constrained Baseline. Otherwise OpenH264.");
        }
    }

    /// Discord-style Screen Share / Go Live picker: overlay card, underline tabs,
    /// thumbnail source grid, quality + audio below, green Go Live in the footer.
    fn draw_share(&mut self, ui: &Ui) {
        let display = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("Screen Share")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .focused(true)
            .build(|| {
                skin::rect(ui, [0.0, 0.0], display, OVERLAY, 0.0);

                let panel_w = (display[0] - 80.0).clamp(720.0, SHARE_MODAL_W);
                let panel_h = (display[1] - 48.0).clamp(520.0, SHARE_MODAL_H);
                let min = [((display[0] - panel_w) * 0.5).floor(), ((display[1] - panel_h) * 0.5).max(16.0).floor()];
                let max = [min[0] + panel_w, min[1] + panel_h];
                let semi = self.semibold_font;
                let inner_w = panel_w - SHARE_PAD * 2.0;

                skin::shadow(ui, min, max, MODAL_RADIUS);
                skin::rect(ui, min, max, STAGE_BG, MODAL_RADIUS);
                skin::rect_corners(
                    ui,
                    [min[0], max[1] - SHARE_FOOTER_H],
                    max,
                    MODAL_FOOTER_BG,
                    MODAL_RADIUS,
                    DrawFlags::ROUND_BOTTOM,
                );

                skin::text(ui, semi, FONT_LG, [min[0] + SHARE_PAD, min[1] + 16.0], TEXT, "Screen Share");
                if skin::icon_button(
                    ui,
                    "close-share",
                    self.icon("x"),
                    [max[0] - SHARE_PAD - 28.0, min[1] + 12.0],
                    28.0,
                    18.0,
                    TEXT_SECONDARY,
                    TEXT_NORMAL,
                    Some(ICON_HOVER_BG),
                ) {
                    self.share_open = false;
                }
                let esc = "ESC";
                let esc_w = skin::text_size(ui, None, FONT_XS, esc)[0];
                skin::text(
                    ui,
                    None,
                    FONT_XS,
                    [max[0] - SHARE_PAD - 28.0 - 6.0 - esc_w, min[1] + 18.0],
                    MUTED,
                    esc,
                );

                let tab_y = min[1] + SHARE_HEADER_H;
                let screens_w = skin::text_size(ui, semi, FONT_SM, "Screens")[0] + 8.0;
                if self.share_tab_label(ui, "share-screens", "Screens", self.share_tab == 0, [min[0] + SHARE_PAD, tab_y])
                    && self.share_tab != 0
                {
                    self.share_tab = 0;
                    self.reload_share_sources();
                }
                if self.share_tab_label(
                    ui,
                    "share-apps",
                    "Applications",
                    self.share_tab == 1,
                    [min[0] + SHARE_PAD + screens_w + 16.0, tab_y],
                ) && self.share_tab != 1
                {
                    self.share_tab = 1;
                    self.reload_share_sources();
                }
                skin::line_h(ui, min[0] + SHARE_PAD, max[0] - SHARE_PAD, tab_y + SHARE_TAB_H - 1.0, DIVIDER);

                let body_top = tab_y + SHARE_TAB_H + 8.0;
                let body_h = (max[1] - SHARE_FOOTER_H - 8.0 - body_top).max(120.0);
                let body_origin = [min[0] + SHARE_PAD, body_top];
                ui.set_cursor_screen_pos(body_origin);
                let _body_pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
                let _body_gap = ui.push_style_var(StyleVar::ItemSpacing([SHARE_TILE_GAP, SHARE_TILE_GAP]));
                let _body_bg = ui.push_style_color(StyleColor::ChildBg, skin::CLEAR);
                ui.child_window("share-body").size([inner_w, body_h]).build(ui, || {
                    let sources = self
                        .share_sources
                        .iter()
                        .map(|source| (source.kind, source.id, source.name.clone()))
                        .collect::<Vec<_>>();
                    if sources.is_empty() {
                        let empty = if self.share_tab == 0 {
                            "No screens were found."
                        } else {
                            "No application windows were found."
                        };
                        ui.dummy([0.0, 24.0]);
                        let w = ui.content_region_avail()[0];
                        let tw = skin::text_size(ui, None, FONT_SM, empty)[0];
                        let origin = ui.cursor_screen_pos();
                        skin::text(ui, None, FONT_SM, [origin[0] + (w - tw) * 0.5, origin[1]], MUTED, empty);
                        ui.dummy([0.0, 40.0]);
                    } else {
                        let avail = ui.content_region_avail()[0];
                        let cols = ((avail + SHARE_TILE_GAP) / (SHARE_TILE_W + SHARE_TILE_GAP)).floor().max(1.0) as usize;
                        for (index, (kind, id, name)) in sources.iter().enumerate() {
                            if index % cols != 0 {
                                ui.same_line();
                            }
                            self.draw_share_tile(ui, *kind, *id, name);
                        }
                        ui.dummy([0.0, 8.0]);
                    }

                    settings_spacer(ui);
                    let share_w = ui.content_region_avail()[0];
                    if settings_panel(ui, "Quality") {
                        self.draw_stream_quality(ui, share_w);
                        settings_spacer(ui);
                    }
                    if settings_panel(ui, "Stream audio") {
                        self.draw_audio_sources(ui, share_w);
                    }
                });

                let footer_y = max[1] - SHARE_FOOTER_H + (SHARE_FOOTER_H - BUTTON_H) * 0.5;
                if skin::button(
                    ui,
                    "share-cancel",
                    "Cancel",
                    Tone::Link,
                    [min[0] + SHARE_PAD - 8.0, footer_y],
                    [80.0, BUTTON_H],
                    true,
                    semi,
                ) {
                    self.share_open = false;
                }

                if !self.status.is_empty() {
                    let status_x = min[0] + SHARE_PAD + 88.0;
                    skin::text_in_row(ui, None, FONT_SM, status_x, max[1] - SHARE_FOOTER_H, SHARE_FOOTER_H, ERROR_TEXT, &self.status);
                }

                let mut right = max[0] - SHARE_PAD;
                if self.share.is_live() {
                    let stop_w = (skin::text_size(ui, semi, FONT_SM, "Stop Streaming")[0] + 32.0).max(140.0);
                    right -= stop_w;
                    if skin::button(
                        ui,
                        "share-stop",
                        "Stop Streaming",
                        Tone::Danger,
                        [right, footer_y],
                        [stop_w, BUTTON_H],
                        true,
                        semi,
                    ) {
                        self.share.stop();
                        self.share_open = false;
                    }
                    right -= 8.0;
                }
                let go_w = (skin::text_size(ui, semi, FONT_SM, "Go Live")[0] + 32.0).max(120.0);
                right -= go_w;
                let can_go = self.share.selected().is_some();
                if skin::button(
                    ui,
                    "share-go-live",
                    "Go Live",
                    Tone::Green,
                    [right, footer_y],
                    [go_w, BUTTON_H],
                    can_go,
                    semi,
                ) {
                    if self.share.go_live() {
                        self.share_open = false;
                        self.status.clear();
                    } else {
                        self.status = "choose a screen or an application first".into();
                    }
                }

                self.queue_next_thumb();
            });
    }

    /// Underline tab like Discord's Screens / Applications row.
    fn share_tab_label(&self, ui: &Ui, id: &str, label: &str, selected: bool, min: [f32; 2]) -> bool {
        let semi = self.semibold_font;
        let text_w = skin::text_size(ui, semi, FONT_SM, label)[0];
        let width = text_w + 8.0;
        ui.set_cursor_screen_pos(min);
        let clicked = ui.invisible_button(id, [width, SHARE_TAB_H]);
        let hovered = ui.is_item_hovered();
        let color = if selected {
            TEXT
        } else if hovered {
            TEXT_NORMAL
        } else {
            MUTED
        };
        skin::text_in_row(ui, semi, FONT_SM, min[0] + 4.0, min[1], SHARE_TAB_H - 2.0, color, label);
        if selected {
            skin::rect(
                ui,
                [min[0], min[1] + SHARE_TAB_H - 3.0],
                [min[0] + width, min[1] + SHARE_TAB_H - 1.0],
                BLURPLE,
                1.0,
            );
        }
        clicked
    }

    /// One Screens / Applications thumbnail card with Discord selection chrome.
    fn draw_share_tile(&mut self, ui: &Ui, kind: CaptureKind, id: u32, name: &str) {
        let key = source_key(kind, id);
        let selected = self.share.selected() == Some((kind, id));
        let origin = ui.cursor_screen_pos();
        let size = [SHARE_TILE_W, SHARE_TILE_H];
        let clicked = ui.invisible_button(format!("pick-{}-{}", key.0, key.1), size);
        let hovered = ui.is_item_hovered();
        let max = [origin[0] + size[0], origin[1] + size[1]];
        let fill = if selected {
            ROW_SELECTED
        } else if hovered {
            ROW_HOVER
        } else {
            PANEL_BG
        };
        skin::rect(ui, origin, max, fill, SHARE_TILE_RADIUS);

        let thumb_pad = 8.0;
        let thumb_min = [origin[0] + thumb_pad, origin[1] + thumb_pad];
        let thumb_max = [max[0] - thumb_pad, origin[1] + thumb_pad + SHARE_THUMB_H];
        skin::rect(ui, thumb_min, thumb_max, INPUT_BG, SHARE_TILE_RADIUS - 2.0);
        let thumb_w = thumb_max[0] - thumb_min[0];
        let thumb_h = thumb_max[1] - thumb_min[1];
        if let Some(shot) = self.share_thumbs.get(&key) {
            let fitted = fit_inside(shot.size, thumb_w, thumb_h);
            let x = thumb_min[0] + (thumb_w - fitted[0]) * 0.5;
            let y = thumb_min[1] + (thumb_h - fitted[1]) * 0.5;
            ui.get_window_draw_list().add_image_rounded(
                shot.texture,
                [x, y],
                [x + fitted[0], y + fitted[1]],
                [0.0, 0.0],
                [1.0, 1.0],
                WHITE,
                SHARE_TILE_RADIUS - 2.0,
                DrawFlags::ROUND_ALL,
            );
        }

        if selected {
            ui.get_window_draw_list()
                .add_rect(origin, max, BLURPLE)
                .rounding(SHARE_TILE_RADIUS)
                .thickness(2.0)
                .build();
            let check_c = [thumb_max[0] - 4.0 - SHARE_CHECK * 0.5, thumb_min[1] + 4.0 + SHARE_CHECK * 0.5];
            ui.get_window_draw_list()
                .add_circle(check_c, SHARE_CHECK * 0.5, BLURPLE)
                .filled(true)
                .build();
            skin::image(
                ui,
                self.icon("check"),
                [check_c[0] - 6.0, check_c[1] - 6.0],
                12.0,
                WHITE,
            );
        }

        let label_y = thumb_max[1] + 6.0;
        let shown = skin::ellipsize(ui, self.semibold_font, FONT_SM, name, size[0] - thumb_pad * 2.0);
        skin::text(ui, self.semibold_font, FONT_SM, [origin[0] + thumb_pad, label_y], TEXT_NORMAL, &shown);

        if clicked {
            if self.share.is_live() {
                self.share.apply_while_live(kind, id);
            } else {
                self.share.select(kind, id);
            }
            self.save_stream();
        }
    }

    fn reload_share_sources(&mut self) {
        self.retired_shots.extend(self.share_thumbs.drain().map(|(_, shot)| shot));
        self.share_miss.clear();
        self.thumb_queue.clear();
        self.share_thumb_next = 0;
        self.share_sources = if self.share_tab == 0 { list_screens() } else { list_windows() };
    }

    fn queue_next_thumb(&mut self) {
        if self.thumb_busy || !self.thumb_queue.is_empty() {
            return;
        }
        while self.share_thumb_next < self.share_sources.len() {
            let source = &self.share_sources[self.share_thumb_next];
            let key = source_key(source.kind, source.id);
            let kind = source.kind;
            let id = source.id;
            self.share_thumb_next += 1;
            if self.share_thumbs.contains_key(&key) || self.share_miss.contains(&key) {
                continue;
            }
            self.thumb_busy = true;
            let tx = self.thumb_tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send((key.0, key.1, capture_source_jpeg(kind, id)));
            });
            return;
        }
    }

    /// Someone in the joined voice channel, listed under it: 24 px avatar, name, then state icons.
    fn draw_voice_member(&self, ui: &Ui, member: &VoiceMember) {
        let name = self.name_of(member.user_id);
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        ui.invisible_button(format!("voice-member-{}", member.user_id), [width, VOICE_MEMBER_H]);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + 32.0, origin[1] + 2.0];
        let max = [origin[0] + width - ROW_INSET, origin[1] + VOICE_MEMBER_H - 2.0];
        if hovered {
            skin::rect(ui, min, max, ROW_HOVER, ROW_RADIUS);
        }
        let center = [min[0] + 4.0 + VOICE_AVATAR * 0.5, origin[1] + VOICE_MEMBER_H * 0.5];
        let avatar = self.avatars.get(&member.user_id).map(|shot| shot.texture);
        paint_avatar(ui, avatar, center, VOICE_AVATAR * 0.5, &initials(&name), None);
        if self.member_talking(member) {
            ui.get_window_draw_list()
                .add_circle(center, VOICE_AVATAR * 0.5 + 1.0, GREEN)
                .thickness(2.0)
                .build();
        }
        let mut icon_x = max[0] - 4.0 - 16.0;
        let icon_y = center[1] - 8.0;
        let mut icons_w = 0.0;
        if member.flags.muted || member.flags.deafened {
            let icon = if member.flags.deafened { "headphone-off" } else { "mic-off" };
            skin::image(ui, self.icon(icon), [icon_x, icon_y], 16.0, MUTED);
            icon_x -= 20.0;
            icons_w += 20.0;
        }
        if member.flags.camera {
            skin::image(ui, self.icon("video"), [icon_x, icon_y], 16.0, MUTED);
            icon_x -= 20.0;
            icons_w += 20.0;
        }
        if member.flags.screen {
            let live_w = skin::text_size(ui, self.bold_font, 11.0, "LIVE")[0] + 12.0;
            let live_min = [icon_x + 16.0 - live_w, center[1] - 8.0];
            skin::rect(ui, live_min, [live_min[0] + live_w, live_min[1] + 16.0], RED, 8.0);
            skin::text_in_row(ui, self.bold_font, 11.0, live_min[0] + 6.0, live_min[1], 16.0, WHITE, "LIVE");
            icons_w += live_w + 4.0;
        }
        let text_x = center[0] + VOICE_AVATAR * 0.5 + 8.0;
        let color = if hovered { TEXT_NORMAL } else { MUTED };
        let shown = skin::ellipsize(ui, None, FONT_SM, &name, (max[0] - 4.0 - icons_w - text_x).max(16.0));
        skin::text_in_row(ui, None, FONT_SM, text_x, origin[1], VOICE_MEMBER_H, color, &shown);
    }

    fn draw_featured_screen_at(&mut self, ui: &Ui, origin: [f32; 2], width: f32, height: f32) {
        let featured = if self.screen {
            self.local_screen.as_ref().map(|shot| (self.name_of(self.account.as_ref().map(|account| account.id).unwrap_or(0)), shot.texture, shot.size))
        } else {
            None
        };
        let featured = featured.or_else(|| {
            self.voice_members.iter().find(|member| member.flags.screen).and_then(|member| {
                self.remote_video.get(&(member.user_id, 3)).map(|shot| (self.name_of(member.user_id), shot.texture, shot.size))
            })
        });
        let max = [origin[0] + width, origin[1] + height];
        skin::rect(ui, origin, max, TILE_BG, TILE_RADIUS);
        let Some((name, texture, size)) = featured else { return };
        let shown = if self.screen_shown[0] > 1.0 { self.screen_shown } else { size };
        let fitted = fit_inside(shown, width, height);
        let x = origin[0] + (width - fitted[0]) * 0.5;
        let y = origin[1] + (height - fitted[1]) * 0.5;
        ui.get_window_draw_list().add_image_rounded(
            texture,
            [x, y],
            [x + fitted[0], y + fitted[1]],
            [0.0, 0.0],
            [1.0, 1.0],
            WHITE,
            TILE_RADIUS,
            DrawFlags::ROUND_ALL,
        );
        self.name_tag(ui, [origin[0] + 8.0, max[1] - 8.0 - 24.0], &name, None, true);
        if let Some(member) = self.voice_members.iter().find(|member| member.flags.screen).cloned() {
            let me = self.account.as_ref().map(|account| account.id);
            if Some(member.user_id) != me {
                self.quality_control(ui, member.user_id, MediaKind::Screen, [origin[0] + 8.0, origin[1] + 8.0]);
            }
        }
    }

    /// Bottom-left tile label: name, then a mute or deafen icon, and a red LIVE badge for a stream.
    fn name_tag(&self, ui: &Ui, min: [f32; 2], name: &str, state_icon: Option<&str>, live: bool) {
        let semi = self.semibold_font;
        let name_w = skin::text_size(ui, semi, FONT_SM, name)[0];
        let live_w = if live { skin::text_size(ui, self.bold_font, 11.0, "LIVE")[0] + 12.0 } else { 0.0 };
        let icon_w = if state_icon.is_some() { 20.0 } else { 0.0 };
        let tag_w = 8.0 + name_w + 8.0 + icon_w + if live { live_w + 6.0 } else { 0.0 };
        skin::rect(ui, min, [min[0] + tag_w, min[1] + 24.0], [0.0, 0.0, 0.0, 0.6], 4.0);
        let mut x = min[0] + 8.0;
        if live {
            skin::rect(ui, [x, min[1] + 4.0], [x + live_w, min[1] + 20.0], RED, 4.0);
            skin::text_in_row(ui, self.bold_font, 11.0, x + 6.0, min[1] + 4.0, 16.0, WHITE, "LIVE");
            x += live_w + 6.0;
        }
        skin::text_in_row(ui, semi, FONT_SM, x, min[1], 24.0, WHITE, name);
        if let Some(icon) = state_icon {
            skin::image(ui, self.icon(icon), [x + name_w + 6.0, min[1] + 4.0], 16.0, WHITE);
        }
    }

    fn draw_voice_tile_at(&mut self, ui: &Ui, member: &VoiceMember, origin: [f32; 2], tile_w: f32, tile_h: f32) {
        let name = self.name_of(member.user_id);
        let me = self.account.as_ref().map(|account| account.id);
        let talking = self.member_talking(member);
        let max = [origin[0] + tile_w, origin[1] + tile_h];
        skin::rect(ui, origin, max, TILE_BG, TILE_RADIUS);
        if let Some((texture, size)) = self.tile_camera(member, me) {
            let fitted = fit_inside(size, tile_w, tile_h);
            let x = origin[0] + (tile_w - fitted[0]) * 0.5;
            let y = origin[1] + (tile_h - fitted[1]) * 0.5;
            ui.get_window_draw_list().add_image_rounded(
                texture,
                [x, y],
                [x + fitted[0], y + fitted[1]],
                [0.0, 0.0],
                [1.0, 1.0],
                WHITE,
                TILE_RADIUS,
                DrawFlags::ROUND_ALL,
            );
        } else {
            let radius = (TILE_AVATAR * 0.5).min(tile_h * 0.3);
            let center = [origin[0] + tile_w * 0.5, origin[1] + tile_h * 0.5];
            let avatar = self.avatars.get(&member.user_id).map(|shot| shot.texture);
            paint_avatar(ui, avatar, center, radius, &initials(&name), None);
        }
        if talking {
            ui.get_window_draw_list()
                .add_rect([origin[0] + 1.0, origin[1] + 1.0], [max[0] - 1.0, max[1] - 1.0], GREEN)
                .rounding(TILE_RADIUS)
                .thickness(2.0)
                .build();
        }
        let state_icon = if member.flags.deafened {
            Some("headphone-off")
        } else if member.flags.muted {
            Some("mic-off")
        } else {
            None
        };
        self.name_tag(ui, [origin[0] + 8.0, max[1] - 8.0 - 24.0], &name, state_icon, false);
        let me = self.account.as_ref().map(|account| account.id);
        if member.flags.camera && Some(member.user_id) != me {
            self.quality_control(ui, member.user_id, MediaKind::Camera, [origin[0] + 8.0, origin[1] + 8.0]);
        }
    }

    fn quality_control(&mut self, ui: &Ui, publisher: i64, track: MediaKind, origin: [f32; 2]) {
        let key = (publisher, track);
        let open = self.quality_for == Some(key);
        let chosen = self.layers.get(publisher, track);
        ui.set_cursor_screen_pos(origin);
        let label = format!("{}##quality-open-{}-{}", layer_word(chosen), publisher, track.code());
        if ui.small_button(&label) {
            if open {
                self.quality_for = None;
            } else {
                self.quality_for = Some(key);
                self.quality_draft = banter_server::slider_position(chosen);
            }
        }
        if !open {
            return;
        }
        let delivered = self.delivered.get(&key).copied().unwrap_or(chosen);
        let panel = [origin[0], origin[1] + 26.0];
        ui.window(format!("Quality {} {}", publisher, track.code()))
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::NO_DOCKING,
            )
            .position(panel, Condition::Always)
            .size([240.0, 96.0], Condition::Always)
            .build(|| {
                ui.text("Quality");
                let changed = ui.slider_f32("##quality-draft", &mut self.quality_draft, 0.0, 1.0);
                let snapped = banter_server::snap_quality_slider(self.quality_draft);
                if changed {
                    self.quality_draft = banter_server::slider_position(snapped);
                }
                let note = if delivered != snapped {
                    format!("{} (showing {})", layer_word(snapped), layer_word(delivered))
                } else {
                    layer_word(snapped).to_string()
                };
                if ui.button("Apply") {
                    self.layers.set(publisher, track, snapped);
                    self.send_subscribe(publisher, track, snapped);
                    self.quality_for = None;
                }
                ui.same_line();
                ui.text_colored(MUTED, note);
            });
    }

    fn send_subscribe(&self, publisher: i64, track: MediaKind, layer: LayerId) {
        let place = if let Some(channel_id) = self.voice_channel {
            Some(MediaPlace::VoiceChannel(channel_id))
        } else if self.call.as_ref().is_some_and(|call| call.phase == CallPhase::Active) {
            self.call.as_ref().map(|call| MediaPlace::DirectCall(call.dm_id))
        } else {
            None
        };
        let Some(place) = place else { return };
        let Some(realtime) = &self.realtime else { return };
        realtime.send(ClientSignal::SubscribeLayer {
            place,
            publisher,
            track,
            max_layer: layer,
        });
    }

    fn tile_camera(&self, member: &VoiceMember, me: Option<i64>) -> Option<(ManagedTextureId, [f32; 2])> {
        let mine = me == Some(member.user_id);
        if !(member.flags.camera || (mine && self.camera)) {
            return None;
        }
        let shot = if mine {
            self.local_camera.as_ref()
        } else {
            self.remote_video.get(&(member.user_id, 2))
        }?;
        Some((shot.texture, shot.size))
    }

    fn load_saved(&mut self) {
        if let Some(voice) = load_json::<VoicePrefs>(&self.data_dir.join("voice.json")) {
            self.push_to_talk = voice.push_to_talk;
            self.voice = voice;
        }
        if let Some(mut video) = load_json::<VideoPrefs>(&self.data_dir.join("video.json")) {
            let (width, height) = clamp_window_size(video.width, video.height);
            video.width = width;
            video.height = height;
            self.video = video;
        }
        if let Some(stream) = load_json::<StreamDefaults>(&self.data_dir.join("stream.json")) {
            self.stream_resolution = resolution_from_key(&stream.resolution);
            self.stream_fps = fps_from_key(&stream.fps);
            self.stream = stream;
            self.stream.hardware = false;
        }
        if let Some(dev) = load_json::<DevServerPrefs>(&self.data_dir.join("dev-server.json")) {
            self.dev_server = dev;
        }
    }

    fn save_voice(&self) {
        let mut voice = self.voice.clone();
        voice.push_to_talk = self.push_to_talk;
        let _ = save_json(&self.data_dir.join("voice.json"), &voice);
    }

    fn save_video(&mut self) {
        let (width, height) = clamp_window_size(self.video.width, self.video.height);
        self.video.width = width;
        self.video.height = height;
        let _ = save_json(&self.data_dir.join("video.json"), &self.video);
        self.pending_video = Some(self.video.clone());
    }

    fn save_stream(&mut self) {
        self.stream.resolution = resolution_key(self.stream_resolution).to_string();
        self.stream.fps = fps_key(self.stream_fps).to_string();
        self.stream.hardware = false;
        let _ = save_json(&self.data_dir.join("stream.json"), &self.stream);
    }

    fn save_dev_server(&self) {
        let _ = save_json(&self.data_dir.join("dev-server.json"), &self.dev_server);
    }

    /// Persist Developer prefs and point the HTTP/WebSocket client at the chosen API base.
    fn apply_dev_server(&mut self) {
        match api_base_for(&self.dev_server) {
            Ok(base) => {
                self.save_dev_server();
                if base == self.base {
                    self.settings_error.clear();
                    return;
                }
                self.reconnect_api_base(base);
            }
            Err(err) => {
                self.settings_error = err;
            }
        }
    }

    fn draw_display_settings(&mut self, ui: &Ui, width: f32) {
        settings_form_label(ui, self.bold_font, "DISPLAY");
        ui.text_colored(
            MUTED,
            "Window size and sync. Turning VSync off can raise FPS; tearing may show up.",
        );
        settings_spacer(ui);
        if settings_toggle_row(ui, "video-vsync", "VSync", &mut self.video.vsync, width) {
            self.save_video();
        }
        settings_divider(ui, width);
        if settings_toggle_row(ui, "video-fullscreen", "Fullscreen", &mut self.video.fullscreen, width) {
            self.save_video();
        }
        settings_divider(ui, width);
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "WINDOW RESOLUTION");
        if self.video.fullscreen {
            ui.text_colored(MUTED, "Resolution applies when you leave fullscreen.");
            settings_spacer(ui);
        }
        let presets = window_size_presets();
        let mut labels: Vec<String> = presets
            .iter()
            .map(|&(w, h)| window_size_label(w, h))
            .collect();
        let mut index = presets
            .iter()
            .position(|&(w, h)| w == self.video.width && h == self.video.height)
            .unwrap_or(usize::MAX);
        if index == usize::MAX {
            labels.push(format!("{} (current)", window_size_label(self.video.width, self.video.height)));
            index = labels.len() - 1;
        }
        if settings_combo(ui, "##window-resolution", width, &labels, &mut index) {
            if let Some(&(w, h)) = presets.get(index) {
                self.video.width = w;
                self.video.height = h;
                self.save_video();
            }
        }
    }

    fn reconnect_api_base(&mut self, base: String) {
        if self.dev_server.target == ServerTarget::Local {
            if let Err(err) = crate::platform::ensure_local_server() {
                self.settings_error = err;
                return;
            }
        }

        // Drop media against the previous host without waiting on it.
        self.voice_channel = None;
        self.voice_members.clear();
        self.end_media_place();
        self.media = None;
        self.realtime = None;

        let token = self.client.token().map(str::to_string);
        self.base = base.clone();
        self.client.set_base(&base);

        self.friends.clear();
        self.incoming.clear();
        self.outgoing.clear();
        self.servers.clear();
        self.dms.clear();
        self.mentions.clear();
        self.presence.clear();
        self.layout.clear();
        self.members.clear();
        self.selected = None;
        self.messages.clear();
        self.place = Place::Home;

        if let Some(token) = token {
            self.client.set_token(Some(token));
            match self.client.me() {
                Ok(me) => {
                    self.display_name = me.display_name.clone();
                    self.account = Some(me);
                    self.settings_error.clear();
                    self.status = format!("Connected to {base}");
                    self.refresh_shell();
                    self.connect_live();
                }
                Err(_) => {
                    self.client.set_token(None);
                    let _ = std::fs::remove_file(session_path(&self.data_dir));
                    self.account = None;
                    self.display_name.clear();
                    self.settings.close();
                    self.settings_error =
                        format!("Switched to {base}. Sign in again on this server.");
                }
            }
        } else {
            self.settings_error.clear();
            // On the sign-in screen `status` is the auth error line; keep it clear.
            if self.account.is_some() {
                self.status = format!("Using {base}");
            } else {
                self.status.clear();
            }
        }
    }

    fn engine_update(&self) -> EngineUpdate {
        EngineUpdate {
            input_device: self.voice.input_device.clone(),
            output_device: self.voice.output_device.clone(),
            input_volume: self.voice.input_volume,
            output_volume: self.voice.output_volume,
            sensitivity: if self.voice.auto_sensitivity { OPEN_MIC_RMS } else { self.voice.sensitivity },
            noise_suppression: self.voice.noise_suppression,
            echo_cancellation: self.voice.echo_cancellation,
            auto_gain: self.voice.auto_gain,
            show_cursor: self.stream.show_cursor,
            show_preview: self.stream.show_preview,
            sources: self.stream.sources.clone(),
        }
    }

    fn ptt_down(&self, ui: &Ui) -> bool {
        let key = match self.voice.ptt_key.as_str() {
            "A" => Key::A,
            "B" => Key::B,
            "C" => Key::C,
            "D" => Key::D,
            "E" => Key::E,
            "F" => Key::F,
            "G" => Key::G,
            "H" => Key::H,
            "I" => Key::I,
            "J" => Key::J,
            "K" => Key::K,
            "L" => Key::L,
            "M" => Key::M,
            "N" => Key::N,
            "O" => Key::O,
            "P" => Key::P,
            "Q" => Key::Q,
            "R" => Key::R,
            "S" => Key::S,
            "T" => Key::T,
            "U" => Key::U,
            "W" => Key::W,
            "X" => Key::X,
            "Y" => Key::Y,
            "Z" => Key::Z,
            _ => Key::V,
        };
        ui.is_key_down(key) || (self.voice.ptt_key == "V" && self.v_held)
    }

    fn capture_ptt_key(&mut self, ui: &Ui) {
        if !self.capture_ptt {
            return;
        }
        for (name, key) in [
            ("A", Key::A), ("B", Key::B), ("C", Key::C), ("D", Key::D), ("E", Key::E), ("F", Key::F),
            ("G", Key::G), ("H", Key::H), ("I", Key::I), ("J", Key::J), ("K", Key::K), ("L", Key::L),
            ("M", Key::M), ("N", Key::N), ("O", Key::O), ("P", Key::P), ("Q", Key::Q), ("R", Key::R),
            ("S", Key::S), ("T", Key::T), ("U", Key::U), ("V", Key::V), ("W", Key::W), ("X", Key::X),
            ("Y", Key::Y), ("Z", Key::Z),
        ] {
            if ui.is_key_pressed(key) {
                self.voice.ptt_key = name.to_string();
                self.capture_ptt = false;
                self.save_voice();
                break;
            }
        }
    }

    fn ensure_audio_devices(&mut self) {
        if self.devices_ready {
            return;
        }
        let inputs = media::input_device_names();
        let outputs = media::output_device_names();
        let apps = media::app_audio_sources();
        for name in &inputs {
            self.ensure_source(format!("mic:{name}"), name.clone());
        }
        for name in &outputs {
            self.ensure_source(format!("loop:{name}"), format!("{name} (speakers)"));
        }
        for (pid, name) in &apps {
            self.ensure_source(format!("app:{pid}"), name.clone());
        }
        self.audio_inputs = inputs;
        self.audio_outputs = outputs;
        self.app_sources = apps;
        self.devices_ready = true;
        self.save_stream();
    }

    fn ensure_source(&mut self, id: String, name: String) {
        if let Some(source) = self.stream.sources.iter_mut().find(|source| source.id == id) {
            source.name = name;
            return;
        }
        self.stream.sources.push(MixSource { id, name, enabled: false, volume: 1.0 });
    }

    fn draw_named_combo(&mut self, ui: &Ui, id: &str, kind: &str, width: f32, devices: &[String], input: bool) {
        if devices.is_empty() {
            ui.text_colored(MUTED, format!("No {kind} is available."));
            return;
        }
        let mut labels = vec!["Default".to_string()];
        labels.extend(devices.iter().cloned());
        let current = if input { &self.voice.input_device } else { &self.voice.output_device };
        let mut index = devices.iter().position(|name| name == current).map(|index| index + 1).unwrap_or(0);
        if settings_combo(ui, &format!("##{id}"), width, &labels, &mut index) {
            let chosen = if index == 0 { String::new() } else { devices[index - 1].clone() };
            if input {
                self.voice.input_device = chosen;
            } else {
                self.voice.output_device = chosen;
            }
            self.save_voice();
        }
    }

    fn draw_input_meter(&self, ui: &Ui, width: f32, height: f32, threshold: Option<f32>) {
        let rms = self.media.as_ref().map(|media| media.input_rms()).unwrap_or(0.0);
        let level = meter_fraction(rms);
        let origin = ui.cursor_screen_pos();
        ui.dummy([width, height]);
        let bar_h = 8.0;
        let top = origin[1] + (height - bar_h) * 0.5;
        let draw = ui.get_window_draw_list();
        draw.add_rect([origin[0], top], [origin[0] + width, top + bar_h], RAIL_BG).rounding(4.0).filled(true).build();
        if level > 0.0 {
            let color = match threshold {
                Some(gate) if rms < gate => LOW_LEVEL,
                _ => GREEN,
            };
            draw.add_rect([origin[0], top], [origin[0] + width * level, top + bar_h], color).rounding(4.0).filled(true).build();
        }
        if let Some(gate) = threshold {
            // Same 5 px inset as the settings slider knob, so the mark sits under the knob.
            let mark_x = origin[0] + 5.0 + (width - 10.0).max(0.0) * meter_fraction(gate);
            draw.add_rect([mark_x - 1.0, top - 4.0], [mark_x + 1.0, top + bar_h + 4.0], WHITE).filled(true).build();
        }
    }

    fn draw_audio_sources(&mut self, ui: &Ui, width: f32) {
        self.ensure_audio_devices();
        settings_form_label(ui, self.bold_font, "STREAM AUDIO");
        ui.text_colored(MUTED, "Turn on every app or device to mix into the stream. Each one has its own volume.");
        settings_spacer(ui);
        let count = self.stream.sources.len();
        if count == 0 {
            ui.text_colored(MUTED, "No audio sources were found.");
        }
        for index in 0..count {
            let mut enabled = self.stream.sources[index].enabled;
            let name = self.stream.sources[index].name.clone();
            let id = self.stream.sources[index].id.clone();
            if settings_toggle_row(ui, &format!("src-{id}"), &name, &mut enabled, width) {
                self.stream.sources[index].enabled = enabled;
                self.save_stream();
            }
            if enabled {
                let mut volume = self.stream.sources[index].volume;
                let release = settings_volume_slider(ui, &format!("vol-{id}"), &mut volume, width);
                self.stream.sources[index].volume = volume;
                if release {
                    self.save_stream();
                }
                settings_spacer(ui);
            }
            settings_divider(ui, width);
        }
    }

    fn draw_stream_menu(&mut self, ui: &Ui) {
        let width = 280.0;
        let height = 220.0;
        let x = self.stream_anchor[0];
        let y = (self.stream_anchor[1] - height - 8.0).max(8.0);
        ui.window("Stream options")
            .flags(WindowFlags::NO_DECORATION | WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_DOCKING)
            .position([x, y], Condition::Always)
            .size([width, height], Condition::Always)
            .build(|| {
                ui.text("Stream options");
                ui.child_window("stream-options-body").size([0.0, -36.0]).build(ui, || {
                    if settings_panel(ui, "Bitrate") {
                        if ui.checkbox("Automatic bitrate", &mut self.stream.bitrate_auto) {
                            self.save_stream();
                        }
                        if !self.stream.bitrate_auto {
                            let mut kbps = self.stream.bitrate_kbps as f32;
                            if ui.slider_f32("Bitrate kbps", &mut kbps, 200.0, 8000.0) {
                                self.stream.bitrate_kbps = kbps.round() as u32;
                                self.save_stream();
                            }
                        }
                        if let Some(reason) = hardware_encoder_block(self.stream.hardware) {
                            ui.text_wrapped(reason);
                        }
                    }
                    if settings_panel(ui, "Preview") {
                        if ui.checkbox("Show cursor", &mut self.stream.show_cursor) {
                            self.save_stream();
                        }
                        if ui.checkbox("Show my preview", &mut self.stream.show_preview) {
                            self.save_stream();
                        }
                    }
                });
                if ui.button("Change source") {
                    self.open_share();
                }
                if self.share.is_live() {
                    ui.same_line();
                    if ui.button("Stop streaming") {
                        self.share.stop();
                        self.stream_menu = false;
                        self.save_stream();
                    }
                }
            });
    }

    fn draw_profiler(&mut self, ui: &Ui) {
        if self.profiler_metrics {
            ui.show_metrics_window(&mut self.profiler_metrics);
        }
        if self.profiler_demo {
            ui.show_demo_window(&mut self.profiler_demo);
        }
        if !profiler::global().is_open() {
            return;
        }
        let size = ui.io().display_size();
        let x = (size[0] - PROFILER_W - PROFILER_MARGIN).max(PROFILER_MARGIN);
        let y = PROFILER_MARGIN;
        let snap = profiler::global().snapshot();
        let ops = profiler::global().recent_ops();
        let _bg = ui.push_style_color(StyleColor::WindowBg, FLOATING_BG);
        let _border = ui.push_style_color(StyleColor::Border, DIVIDER);
        let _pad = ui.push_style_var(StyleVar::WindowPadding([PROFILER_PAD, PROFILER_PAD]));
        let _round = ui.push_style_var(StyleVar::WindowRounding(MENU_RADIUS));
        ui.window("Profiler")
            .flags(
                WindowFlags::NO_COLLAPSE
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::NO_DOCKING
                    | WindowFlags::NO_FOCUS_ON_APPEARING,
            )
            .position([x, y], Condition::FirstUseEver)
            .size([PROFILER_W, PROFILER_H], Condition::FirstUseEver)
            .build(|| {
                ui.text_colored(TEXT, "Profiler");
                ui.same_line();
                ui.text_colored(MUTED, "F3");
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Frame");
                ui.text(format!(
                    "FPS  {:.0}  (smooth {:.0})",
                    snap.fps, snap.fps_smooth
                ));
                ui.text_colored(MUTED, "VSync (User Settings → Voice & Video) can cap FPS.");
                ui.text(format!(
                    "prepare {:.2} ms   draw {:.2} ms   gl {:.2} ms",
                    snap.prepare_ms, snap.draw_ms, snap.gl_ms
                ));
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Memory");
                ui.text(format!(
                    "RSS {}   Virtual {}",
                    profiler::format_bytes(snap.rss_bytes),
                    profiler::format_bytes(snap.vmsize_bytes)
                ));
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Network");
                ui.text(format!(
                    "HTTP  {}/s up  {}/s down  {} req/s",
                    profiler::format_bytes(snap.http_tx_per_s),
                    profiler::format_bytes(snap.http_rx_per_s),
                    snap.http_count_per_s
                ));
                ui.text(format!(
                    "total  {} up  {} down  {} requests",
                    profiler::format_bytes(snap.http_tx),
                    profiler::format_bytes(snap.http_rx),
                    snap.http_count
                ));
                ui.text(format!(
                    "WS  {}/s up  {}/s down  {} msg/s",
                    profiler::format_bytes(snap.ws_tx_per_s),
                    profiler::format_bytes(snap.ws_rx_per_s),
                    snap.ws_msgs_per_s
                ));
                ui.text(format!(
                    "total  {} up  {} down  {} messages",
                    profiler::format_bytes(snap.ws_tx),
                    profiler::format_bytes(snap.ws_rx),
                    snap.ws_msgs
                ));
                let qos = profiler::qos();
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Stream");
                ui.text(format!("{}   ladder {}", qos.topology, qos.ladder));
                ui.text(format!(
                    "subscribers {}   layers {}   {}",
                    qos.subscribers, qos.layers, qos.backend
                ));
                ui.text(format!(
                    "{}   loss {:.1}%   rtt {} ms   jitter {} ms",
                    qos.step, qos.loss * 100.0, qos.rtt_ms, qos.jitter_ms
                ));
                ui.text(format!(
                    "user {} kbps   stream {} kbps",
                    qos.user_kbps, qos.stream_kbps
                ));
                ui.text(format!(
                    "camera {} kbps   screen {} kbps   encode {:.1} ms   decode {:.1} ms",
                    qos.camera_kbps, qos.screen_kbps, qos.encode_ms, qos.decode_ms
                ));
                ui.text(format!("keyframes {}", qos.keyframes));
                ui.separator();
                ui.checkbox("ImGui Metrics", &mut self.profiler_metrics);
                ui.same_line();
                ui.checkbox("ImGui Demo", &mut self.profiler_demo);
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Recent expensive ops");
                ui.child_window("profiler_ops").size([0.0, 0.0]).build(ui, || {
                    if ops.is_empty() {
                        ui.text_colored(MUTED, "Nothing slow yet.");
                        return;
                    }
                    for op in &ops {
                        ui.text(format!("{:.1} ms  {}", op.duration_ms, op.label));
                        if !op.detail.is_empty() {
                            ui.text_colored(MUTED, format!("    {}", op.detail));
                        }
                    }
                });
            });
    }

    fn selected_capture_limits(&self) -> (u32, u32) {
        if let Some((kind, id)) = self.share.selected().or(self.share.live_target()) {
            if let Some(source) = self.share_sources.iter().find(|source| source.kind == kind && source.id == id) {
                if source.height > 0 || source.hz > 0 {
                    return (source.height, source.hz);
                }
            }
        }
        media::monitor_limits()
    }

    fn note_screen_size(&mut self, size: [f32; 2]) {
        if self.screen_target == size {
            let t = (self.screen_resize_at.elapsed().as_secs_f32() / 0.12).clamp(0.0, 1.0);
            self.screen_shown = lerp_size(self.screen_from, self.screen_target, t);
            return;
        }
        self.screen_from = if self.screen_shown[0] > 1.0 { self.screen_shown } else { size };
        self.screen_target = size;
        self.screen_resize_at = Instant::now();
        self.screen_shown = self.screen_from;
    }

    fn member_talking(&self, member: &VoiceMember) -> bool {
        if self.account.as_ref().map(|account| account.id) == Some(member.user_id) {
            self.flags.transmitting
        } else {
            member.flags.transmitting
        }
    }

    fn note<T>(&mut self, result: Result<T, ApiError>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(err) => {
                self.status = err.to_string();
                None
            }
        }
    }
}

fn mirrored_camera_shot(imgui: &mut Context, jpeg: &[u8]) -> Option<Shot> {
    let image = image::load_from_memory(jpeg).ok()?.into_rgba8();
    let mirrored = mirror_rgba(image.width(), image.height(), image.as_raw()).ok()?;
    upload_rgba(imgui, image.width(), image.height(), &mirrored)
}

fn source_key(kind: CaptureKind, id: u32) -> (u8, u32) {
    let code = match kind {
        CaptureKind::Screen => 1,
        CaptureKind::Window => 2,
    };
    (code, id)
}

fn list_screens() -> Vec<ShareSource> {
    let Ok(monitors) = xcap::Monitor::all() else { return Vec::new() };
    let mut sources = Vec::new();
    for monitor in monitors {
        let Ok(id) = monitor.id() else { continue };
        let name = monitor.name().unwrap_or_else(|_| format!("Screen {id}"));
        let primary = monitor.is_primary().unwrap_or(false);
        let label = if primary { format!("{name} (Primary)") } else { name };
        let height = monitor.height().unwrap_or(0);
        let hz = monitor.frequency().unwrap_or(0.0).round() as u32;
        sources.push((primary, ShareSource { kind: CaptureKind::Screen, id, name: label, height, hz }));
    }
    sources.sort_by(|left, right| right.0.cmp(&left.0));
    sources.into_iter().map(|(_, source)| source).collect()
}

fn list_windows() -> Vec<ShareSource> {
    let Ok(windows) = xcap::Window::all() else { return Vec::new() };
    let mut sources = Vec::new();
    for window in windows {
        if window.is_minimized().unwrap_or(true) {
            continue;
        }
        if window.width().unwrap_or(0) < 80 || window.height().unwrap_or(0) < 80 {
            continue;
        }
        let Ok(id) = window.id() else { continue };
        let title = window.title().unwrap_or_default();
        let app = window.app_name().unwrap_or_default();
        let name = if title.trim().is_empty() { app } else { title };
        if name.trim().is_empty() {
            continue;
        }
        let (height, hz) = window
            .current_monitor()
            .ok()
            .map(|monitor| (monitor.height().unwrap_or(0), monitor.frequency().unwrap_or(0.0).round() as u32))
            .unwrap_or((0, 0));
        sources.push(ShareSource { kind: CaptureKind::Window, id, name, height, hz });
    }
    sources
}

fn capture_source_jpeg(kind: CaptureKind, id: u32) -> Option<Vec<u8>> {
    match kind {
        CaptureKind::Screen => {
            let monitor = xcap::Monitor::all().ok()?.into_iter().find(|monitor| monitor.id().ok() == Some(id))?;
            let image = monitor.capture_image().ok()?;
            encode_jpeg_rgba(image.width(), image.height(), image.as_raw(), 360).ok()
        }
        CaptureKind::Window => {
            let window = xcap::Window::all().ok()?.into_iter().find(|window| window.id().ok() == Some(id))?;
            let image = window.capture_image().ok()?;
            encode_jpeg_rgba(image.width(), image.height(), image.as_raw(), 360).ok()
        }
    }
}

fn replace_shot(imgui: &mut Context, slot: &mut Option<Shot>, shot: Shot) {
    if let Some(previous) = slot.replace(shot) {
        let _ = imgui.remove_texture(previous.texture);
    }
}

fn upload_jpeg(imgui: &mut Context, bytes: &[u8]) -> Option<Shot> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    upload_rgba(imgui, image.width(), image.height(), image.as_raw())
}

fn upload_rgba_png(imgui: &mut Context, bytes: &[u8]) -> Option<Shot> {
    upload_jpeg(imgui, bytes)
}

fn upload_rgba(imgui: &mut Context, width: u32, height: u32, rgba: &[u8]) -> Option<Shot> {
    let texture = OwnedTextureData::from_pixels(TextureFormat::RGBA32, width, height, rgba).ok()?;
    Some(Shot {
        texture: imgui.register_texture(texture),
        size: [width as f32, height as f32],
    })
}

fn video_code(kind: MediaKind) -> u8 {
    kind.code()
}

fn layer_word(layer: LayerId) -> &'static str {
    match layer {
        LayerId::Full => "full",
        LayerId::Half => "half",
        LayerId::Quarter => "quarter",
        LayerId::Off => "off",
    }
}

fn fit_inside(size: [f32; 2], max_width: f32, max_height: f32) -> [f32; 2] {
    if size[0] <= 1.0 || size[1] <= 1.0 || max_width <= 1.0 || max_height <= 1.0 {
        return [32.0, 32.0];
    }
    let scale = (max_width / size[0]).min(max_height / size[1]);
    [size[0] * scale, size[1] * scale]
}

const SETTINGS_DIVIDER: [f32; 4] = [78.0 / 255.0, 80.0 / 255.0, 88.0 / 255.0, 0.48];
const SWITCH_OFF: [f32; 4] = [128.0 / 255.0, 132.0 / 255.0, 142.0 / 255.0, 1.0];
const LOW_LEVEL: [f32; 4] = [250.0 / 255.0, 166.0 / 255.0, 26.0 / 255.0, 1.0];

fn settings_page_title(ui: &Ui, bold: Option<FontId>, text: &str) {
    {
        let _font = ui.push_font_with_size(bold, 20.0);
        ui.text(text);
    }
    ui.dummy([0.0, 12.0]);
}

fn settings_form_label(ui: &Ui, bold: Option<FontId>, text: &str) {
    {
        let _font = ui.push_font_with_size(bold, 12.0);
        ui.text_colored(MUTED, text);
    }
    ui.dummy([0.0, 4.0]);
}

fn settings_spacer(ui: &Ui) {
    ui.dummy([0.0, 12.0]);
}

fn settings_divider(ui: &Ui, width: f32) {
    let origin = ui.cursor_screen_pos();
    ui.dummy([width, 17.0]);
    ui.get_window_draw_list()
        .add_rect([origin[0], origin[1] + 8.0], [origin[0] + width, origin[1] + 9.0], SETTINGS_DIVIDER)
        .filled(true)
        .build();
}

fn settings_panel(ui: &Ui, title: &str) -> bool {
    ui.collapsing_header(title, TreeNodeFlags::NONE)
}

fn settings_toggle_row(ui: &Ui, id: &str, label: &str, value: &mut bool, width: f32) -> bool {
    let origin = ui.cursor_screen_pos();
    let height = 40.0;
    let clicked = ui.invisible_button(id, [width, height]);
    if clicked {
        *value = !*value;
    }
    let draw = ui.get_window_draw_list();
    let switch_w = 40.0;
    let switch_h = 24.0;
    let label_w = width - switch_w - 16.0;
    let shown = clip_label(ui, label, label_w);
    let text_h = ui.calc_text_size(&shown)[1];
    draw.add_text([origin[0], origin[1] + (height - text_h) * 0.5], TEXT, &shown);
    let x = origin[0] + width - switch_w;
    let y = origin[1] + (height - switch_h) * 0.5;
    draw.add_rect([x, y], [x + switch_w, y + switch_h], if *value { GREEN } else { SWITCH_OFF })
        .rounding(switch_h * 0.5)
        .filled(true)
        .build();
    let knob_x = if *value { x + switch_w - switch_h * 0.5 } else { x + switch_h * 0.5 };
    draw.add_circle([knob_x, y + switch_h * 0.5], switch_h * 0.5 - 3.0, WHITE).filled(true).build();
    clicked
}

fn settings_radio_row(ui: &Ui, id: &str, label: &str, selected: bool, width: f32) -> bool {
    let origin = ui.cursor_screen_pos();
    let height = 44.0;
    let clicked = ui.invisible_button(id, [width, height]);
    let hovered = ui.is_item_hovered();
    let draw = ui.get_window_draw_list();
    let fill = if selected { ROW_SELECTED } else if hovered { ROW_HOVER } else { SIDEBAR_BG };
    draw.add_rect(origin, [origin[0] + width, origin[1] + height], fill).rounding(4.0).filled(true).build();
    let center = [origin[0] + 22.0, origin[1] + height * 0.5];
    draw.add_circle(center, 10.0, if selected { WHITE } else { MUTED }).thickness(2.0).build();
    if selected {
        draw.add_circle(center, 5.0, WHITE).filled(true).build();
    }
    let text_h = ui.calc_text_size(label)[1];
    draw.add_text([origin[0] + 44.0, origin[1] + (height - text_h) * 0.5], if selected { WHITE } else { TEXT }, label);
    ui.dummy([0.0, 4.0]);
    clicked
}

fn settings_combo(ui: &Ui, id: &str, width: f32, labels: &[String], index: &mut usize) -> bool {
    ui.set_next_item_width(width);
    let preview = labels.get(*index).map(String::as_str).unwrap_or("");
    let mut changed = false;
    if let Some(token) = ui.begin_combo(id, preview) {
        for (position, label) in labels.iter().enumerate() {
            let selected = position == *index;
            if ui.selectable_config(format!("{label}##{position}")).selected(selected).build() {
                *index = position;
                changed = true;
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
        if ui.is_key_pressed(Key::Escape) {
            ui.close_current_popup();
        }
        token.end();
    }
    changed
}

fn settings_slider_i32(ui: &Ui, id: &str, value: &mut i32, min: i32, max: i32, width: f32, unit: &str) -> bool {
    let track_w = slider_track_width(width);
    let changed = {
        // Separate bindings, because locals drop in reverse order and Dear ImGui pops its color stack last-in first-out.
        let clear = [0.0, 0.0, 0.0, 0.0];
        let _frame = ui.push_style_color(StyleColor::FrameBg, clear);
        let _frame_hovered = ui.push_style_color(StyleColor::FrameBgHovered, clear);
        let _frame_active = ui.push_style_color(StyleColor::FrameBgActive, clear);
        let _grab = ui.push_style_color(StyleColor::SliderGrab, clear);
        let _grab_active = ui.push_style_color(StyleColor::SliderGrabActive, clear);
        let _text = ui.push_style_color(StyleColor::Text, clear);
        ui.set_next_item_width(track_w);
        ui.slider(id, min, max, value)
    };
    let active = ui.is_item_active();
    let hovered = ui.is_item_hovered();
    let item_min = ui.item_rect_min();
    let item_max = ui.item_rect_max();
    let center_y = (item_min[1] + item_max[1]) * 0.5;
    let knob_x = item_min[0] + 5.0 + (track_w - 10.0).max(0.0) * slider_fraction(*value, min, max);
    let draw = ui.get_window_draw_list();
    draw.add_rect([item_min[0], center_y - 4.0], [item_min[0] + track_w, center_y + 4.0], SWITCH_OFF)
        .rounding(4.0)
        .filled(true)
        .build();
    draw.add_rect([item_min[0], center_y - 4.0], [knob_x, center_y + 4.0], BLURPLE).rounding(4.0).filled(true).build();
    draw.add_rect([knob_x - 5.0, center_y - 12.0], [knob_x + 5.0, center_y + 12.0], if active || hovered { WHITE } else { TEXT })
        .rounding(3.0)
        .filled(true)
        .build();
    let label = format!("{value}{unit}");
    let label_size = ui.calc_text_size(&label);
    draw.add_text(
        [item_min[0] + width - label_size[0], center_y - label_size[1] * 0.5],
        if active { WHITE } else { TEXT },
        &label,
    );
    changed
}

fn settings_volume_slider(ui: &Ui, id: &str, volume: &mut f32, width: f32) -> bool {
    let mut percent = volume_to_percent(*volume);
    if settings_slider_i32(ui, &format!("##{id}"), &mut percent, 0, 200, width, "%") {
        *volume = percent_to_volume(percent);
    }
    ui.is_item_deactivated_after_edit()
}

fn settings_chip_row(ui: &Ui, choices: &[(String, String, bool, bool)], width: f32) -> Option<usize> {
    let spacing = 8.0;
    let widths: Vec<f32> = choices.iter().map(|(_, label, _, _)| ui.calc_text_size(label)[0] + 28.0).collect();
    let rows = wrap_rows(&widths, width, spacing);
    let mut clicked = None;
    for (index, (id, label, selected, enabled)) in choices.iter().enumerate() {
        if index > 0 && rows[index] == rows[index - 1] {
            ui.same_line_with_spacing(0.0, spacing);
        }
        if *enabled {
            if chip(ui, id, label, *selected) {
                clicked = Some(index);
            }
        } else {
            let origin = ui.cursor_screen_pos();
            ui.dummy([widths[index], 32.0]);
            let dim = [MUTED[0], MUTED[1], MUTED[2], 0.45];
            ui.get_window_draw_list().add_text([origin[0] + 14.0, origin[1] + 7.0], dim, label);
        }
    }
    clicked
}

fn clip_label(ui: &Ui, label: &str, width: f32) -> String {
    if ui.calc_text_size(label)[0] <= width {
        return label.to_string();
    }
    let mut shown = String::new();
    for character in label.chars() {
        shown.push(character);
        if ui.calc_text_size(format!("{shown}ΓÇª"))[0] > width {
            shown.pop();
            break;
        }
    }
    format!("{shown}ΓÇª")
}

fn apply_discord_style(style: &mut Style) {
    style.set_color(StyleColor::Text, TEXT);
    style.set_color(StyleColor::TextDisabled, MUTED);
    style.set_color(StyleColor::WindowBg, STAGE_BG);
    style.set_color(StyleColor::ChildBg, STAGE_BG);
    style.set_color(StyleColor::PopupBg, FLOATING_BG);
    style.set_color(StyleColor::Border, with_alpha(FLOATING_BG, 0.6));
    style.set_color(StyleColor::FrameBg, INPUT_BG);
    style.set_color(StyleColor::FrameBgHovered, INPUT_BG);
    style.set_color(StyleColor::FrameBgActive, INPUT_BG);
    style.set_color(StyleColor::Button, BLURPLE);
    style.set_color(StyleColor::ButtonHovered, BLURPLE_HOVER);
    style.set_color(StyleColor::ButtonActive, BLURPLE_ACTIVE);
    style.set_color(StyleColor::Header, ROW_SELECTED);
    style.set_color(StyleColor::HeaderHovered, ROW_HOVER);
    style.set_color(StyleColor::HeaderActive, ROW_SELECTED);
    style.set_color(StyleColor::ScrollbarBg, skin::CLEAR);
    style.set_color(StyleColor::ScrollbarGrab, SCROLL_THUMB);
    style.set_color(StyleColor::ScrollbarGrabHovered, SCROLL_THUMB);
    style.set_color(StyleColor::ScrollbarGrabActive, SCROLL_THUMB);
    style.set_color(StyleColor::TextSelectedBg, with_alpha(BLURPLE, 0.45));
    style.set_frame_rounding(INPUT_RADIUS);
    style.set_window_rounding(MODAL_RADIUS);
    style.set_popup_rounding(MENU_RADIUS);
    style.set_scrollbar_rounding(4.0);
    style.set_scrollbar_size(8.0);
    style.set_window_border_size(0.0);
    style.set_child_border_size(0.0);
    style.set_docking_separator_size(0.0);
    style.set_frame_padding([12.0, 8.0]);
    style.set_item_spacing([8.0, 4.0]);
    style.set_window_padding([8.0, 8.0]);
}

fn nav_row(ui: &Ui, id: &str, icon: Option<ManagedTextureId>, label: &str, selected: bool) -> bool {
    let width = ui.content_region_avail()[0].max(32.0);
    let origin = ui.cursor_screen_pos();
    let clicked = ui.invisible_button(id, [width, 32.0]);
    let hovered = ui.is_item_hovered();
    let draw = ui.get_window_draw_list();
    if selected || hovered {
        let fill = if selected { ROW_SELECTED } else { ROW_HOVER };
        draw.add_rect([origin[0] + 8.0, origin[1]], [origin[0] + width - 8.0, origin[1] + 32.0], fill)
            .rounding(4.0)
            .filled(true)
            .build();
    }
    let mut text_x = origin[0] + 16.0;
    let icon_tint = if selected { TEXT } else { MUTED };
    if let Some(texture) = icon {
        draw.add_image(texture, [text_x, origin[1] + 8.0], [text_x + 16.0, origin[1] + 24.0], [0.0, 0.0], [1.0, 1.0], icon_tint);
        text_x += 24.0;
    }
    let color = if selected { TEXT } else { MUTED };
    let text_h = ui.calc_text_size(label)[1];
    draw.add_text([text_x, origin[1] + (32.0 - text_h) * 0.5], color, label);
    clicked
}

fn chip(ui: &Ui, id: &str, label: &str, selected: bool) -> bool {
    let text_size = ui.calc_text_size(label);
    let width = text_size[0] + 28.0;
    let origin = ui.cursor_screen_pos();
    let clicked = ui.invisible_button(id, [width, 32.0]);
    let fill = if selected { BLURPLE } else { [0.0, 0.0, 0.0, 0.0] };
    if selected || ui.is_item_hovered() {
        ui.get_window_draw_list()
            .add_rect([origin[0], origin[1] + 2.0], [origin[0] + width, origin[1] + 30.0], if selected { fill } else { ROW_HOVER })
            .rounding(4.0)
            .filled(true)
            .build();
    }
    let color = if selected { WHITE } else { MUTED };
    ui.get_window_draw_list().add_text([origin[0] + 14.0, origin[1] + 7.0], color, label);
    clicked
}


fn header_rule(ui: &Ui) {
    let pos = ui.window_pos();
    let size = ui.window_size();
    ui.get_window_draw_list()
        .add_rect([pos[0], pos[1] + HEADER_H - 1.0], [pos[0] + size[0], pos[1] + HEADER_H], HEADER_SHADOW)
        .filled(true)
        .build();
}

/// Discord's uppercase list heading ("DIRECT MESSAGES"): 12 px semibold, muted, 40 px tall.
fn list_label(ui: &Ui, text: &str, font: Option<FontId>) {
    let width = ui.content_region_avail()[0].max(32.0);
    let origin = ui.cursor_screen_pos();
    ui.dummy([width, CATEGORY_H]);
    let label = text.to_uppercase();
    let _font = ui.push_font_with_size(font, FONT_XS);
    let height = ui.calc_text_size(&label)[1];
    ui.get_window_draw_list().add_text([origin[0] + 18.0, origin[1] + CATEGORY_H - height - 6.0], MUTED, &label);
}

fn paint_avatar(ui: &Ui, texture: Option<ManagedTextureId>, center: [f32; 2], radius: f32, letters: &str, ring: Option<[f32; 4]>) {
    let draw = ui.get_window_draw_list();
    if let Some(color) = ring {
        draw.add_circle(center, radius + 3.0, color).thickness(2.0).build();
    }
    if let Some(texture) = texture {
        draw.add_image_rounded(
            texture,
            [center[0] - radius, center[1] - radius],
            [center[0] + radius, center[1] + radius],
            [0.0, 0.0],
            [1.0, 1.0],
            WHITE,
            radius,
            DrawFlags::ROUND_ALL,
        );
    } else {
        draw.add_circle(center, radius, BLURPLE).filled(true).build();
        let size = ui.calc_text_size(letters);
        draw.add_text([center[0] - size[0] * 0.5, center[1] - size[1] * 0.5], WHITE, letters);
    }
}

fn paint_status_dot(ui: &Ui, center: [f32; 2], radius: f32, presence: Presence, backdrop: [f32; 4]) {
    let dot = crate::theme::status_dot(center, radius);
    let draw = ui.get_window_draw_list();
    draw.add_circle(dot.center, dot.cutout, backdrop).filled(true).build();
    let color = presence_color(presence);
    draw.add_circle(dot.center, dot.radius, color).filled(true).build();
    match presence {
        Presence::Online => {}
        // Idle is a crescent: a backdrop-colored bite out of the upper left.
        Presence::Idle => {
            let bite = dot.radius * 0.62;
            draw.add_circle([dot.center[0] - dot.radius * 0.42, dot.center[1] - dot.radius * 0.42], bite, backdrop)
                .filled(true)
                .build();
        }
        // Offline is a hollow ring.
        Presence::Offline => {
            draw.add_circle(dot.center, dot.radius * 0.45, backdrop).filled(true).build();
        }
    }
}

fn share_tip(live: bool) -> &'static str {
    if live {
        "Stop Streaming"
    } else if media::native_screen_picker() {
        "Share Your Screen\nShift+Right Click to pick a source"
    } else {
        "Share Your Screen"
    }
}

fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().filter(|part| !part.is_empty()).collect();
    let source = if parts.len() >= 2 {
        format!("{}{}", parts[0].chars().next().unwrap_or('?'), parts[1].chars().next().unwrap_or('?'))
    } else {
        name.chars().take(1).collect()
    };
    source.to_uppercase()
}

fn presence_color(presence: Presence) -> [f32; 4] {
    match presence {
        Presence::Online => GREEN,
        Presence::Idle => YELLOW,
        Presence::Offline => OFFLINE,
    }
}

fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Online => "Online",
        Presence::Idle => "Idle",
        Presence::Offline => "Offline",
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let mut cursor = std::io::Cursor::new(Vec::new());
    image.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
    Some(cursor.into_inner())
}

fn zone_offset_secs() -> i32 {
    chrono::Local::now().offset().local_minus_utc()
}

fn clock_short(ms: i64, offset: i32) -> String {
    chat_time_label(ms, ms, offset).trim_start_matches("Today at ").to_string()
}

fn pointer_in(ui: &Ui, min: [f32; 2], max: [f32; 2]) -> bool {
    let mouse = ui.io().mouse_pos();
    mouse[0] >= min[0] && mouse[0] <= max[0] && mouse[1] >= min[1] && mouse[1] <= max[1]
}

