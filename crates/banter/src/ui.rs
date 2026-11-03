//! Banter window.
//!
//! The signed-in shell is a temporary copy of Discord's layout and colors so the
//! controls sit where they are familiar. It is not Banter's final look, and the
//! home mark is not Discord's logo.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use banter_server::{
    Account, CallPhase, CallSnapshot, CategoryLayout, Channel, ChannelKind, ChatTarget, DirectChat,
    Friend, FriendRequest, MAX_ATTACHMENT_BYTES, MAX_AVATAR_BYTES, MediaKind, MediaPlace, Mention,
    Message, Presence, ServerSummary, VoiceFlags, VoiceMember,
};
use dear_imgui_rs::{
    Condition, Context, DrawFlags, Key, ManagedTextureId, OwnedTextureData, PopupQueryFlags, Style, StyleColor, StyleVar,
    TextureFormat, Ui, WindowFlags, fonts::FontId,
};

use crate::api::{ApiError, Client, MemberView};
use crate::icons::rasterize_lucide;
use crate::logic::{
    CameraChoice, CaptureKind, MixSource, MuteMemory, ScreenShare, SettingsSection, StreamDefaults, StreamFps,
    StreamResolution, VoicePrefs, camera_preview_mirrored, default_stream_fps, default_stream_resolution,
    discord_shell_columns, encode_jpeg_rgba, fps_block, fps_from_key, fps_key, hardware_encoder_block,
    jpeg_quality_for_bitrate, lerp_size, load_json, mirror_rgba, password_blocks_copy, password_is_masked,
    push_to_talk_held, remove_message, resolution_block, resolution_from_key, resolution_key, save_json,
    settings_sections, speaking_from_level, stream_fps_label, stream_frame_rates, stream_interval_ms,
    stream_max_edge, stream_resolution_label, stream_resolutions, unpack_picture, upsert_message, OPEN_MIC_RMS,
};
use crate::media::{self, EngineUpdate, Media, VideoFrame};
use crate::realtime::Realtime;
use crate::session::{load_token, save_token, session_path};
use crate::settings::{
    SettingsState, db_to_rms, device_error_text, display_name_dirty, slider_fraction, slider_track_width, display_name_error, meter_fraction, percent_to_volume, pick_camera,
    ptt_key, ptt_key_names, rms_to_db, settings_layout, volume_to_percent, wrap_rows, SETTINGS_NAV_LIST_W,
    SETTINGS_TOP_PAD,
};
use crate::websocket_url;

const RAIL_BG: [f32; 4] = [30.0 / 255.0, 31.0 / 255.0, 34.0 / 255.0, 1.0];
const SIDEBAR_BG: [f32; 4] = [43.0 / 255.0, 45.0 / 255.0, 49.0 / 255.0, 1.0];
const STAGE_BG: [f32; 4] = [49.0 / 255.0, 51.0 / 255.0, 56.0 / 255.0, 1.0];
const PANEL_BG: [f32; 4] = [35.0 / 255.0, 36.0 / 255.0, 40.0 / 255.0, 1.0];
const ROW_SELECTED: [f32; 4] = [64.0 / 255.0, 66.0 / 255.0, 73.0 / 255.0, 1.0];
const ROW_HOVER: [f32; 4] = [53.0 / 255.0, 55.0 / 255.0, 60.0 / 255.0, 1.0];
const TEXT: [f32; 4] = [242.0 / 255.0, 243.0 / 255.0, 245.0 / 255.0, 1.0];
const MUTED: [f32; 4] = [148.0 / 255.0, 155.0 / 255.0, 164.0 / 255.0, 1.0];
const BLURPLE: [f32; 4] = [88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, 1.0];
const GREEN: [f32; 4] = [35.0 / 255.0, 165.0 / 255.0, 89.0 / 255.0, 1.0];
const RED: [f32; 4] = [242.0 / 255.0, 63.0 / 255.0, 66.0 / 255.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

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
    retired_shots: Vec<Shot>,
    camera_choice: CameraChoice,
    cameras: Vec<(u32, String)>,
    cameras_loaded: bool,
    stream_resolution: StreamResolution,
    stream_fps: StreamFps,
    voice: VoicePrefs,
    stream: StreamDefaults,
    stream_menu: bool,
    stream_anchor: [f32; 2],
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
    icons: HashMap<String, Shot>,
    icons_ready: bool,
    reply_to: Option<i64>,
    editing: Option<i64>,
    pending_file: Option<(String, Vec<u8>)>,
    avatars: HashMap<i64, Shot>,
    avatar_miss: HashSet<i64>,
    local_camera: Option<Shot>,
    /// Device index `local_camera` was captured from, so switching cameras drops the old picture at once.
    local_camera_device: u32,
    local_screen: Option<Shot>,
    remote_video: HashMap<(i64, u8), Shot>,
    realtime: Option<Realtime>,
    media: Option<Media>,
    last_poll: Instant,
    last_activity: Instant,
    styled: bool,
}

impl App {
    pub(crate) fn open(base: String, data_dir: PathBuf) -> Self {
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
            settings_popup_open: false,
            share_open: false,
            share: ScreenShare::new(),
            share_tab: 0,
            share_sources: Vec::new(),
            share_thumb_next: 0,
            share_thumbs: HashMap::new(),
            share_miss: HashSet::new(),
            thumb_queue: Vec::new(),
            retired_shots: Vec::new(),
            camera_choice: CameraChoice::new(),
            cameras: Vec::new(),
            cameras_loaded: false,
            stream_resolution: default_stream_resolution(),
            stream_fps: default_stream_fps(),
            voice: VoicePrefs::default(),
            stream: StreamDefaults::default(),
            stream_menu: false,
            stream_anchor: [0.0, 0.0],
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
            icons: HashMap::new(),
            icons_ready: false,
            reply_to: None,
            editing: None,
            pending_file: None,
            avatars: HashMap::new(),
            avatar_miss: HashSet::new(),
            local_camera: None,
            local_camera_device: 0,
            local_screen: None,
            remote_video: HashMap::new(),
            realtime: None,
            media: None,
            last_poll: Instant::now(),
            last_activity: Instant::now(),
            styled: false,
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

    pub(crate) fn set_v_held(&mut self, held: bool) {
        self.v_held = held;
    }

    pub(crate) fn note_input(&mut self) {
        self.user_acted = true;
    }

    pub(crate) fn prepare(&mut self, imgui: &mut Context) {
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
        if self.local_camera_device != self.camera_choice.device_index() {
            self.local_camera_device = self.camera_choice.device_index();
            if let Some(shot) = self.local_camera.take() {
                let _ = imgui.remove_texture(shot.texture);
            }
        }
        let frames = self.media.as_ref().map(|media| media.take_video()).unwrap_or_default();
        for frame in frames {
            self.install_video(imgui, frame);
        }
        self.install_avatars(imgui);
    }

    pub(crate) fn draw(&mut self, ui: &Ui) {
        if self.account.is_none() {
            self.draw_login(ui);
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
        if self.settings.is_open() {
            self.draw_settings(ui);
        }
        if self.share_open {
            self.draw_share(ui);
        }
        if self.stream_menu {
            self.draw_stream_menu(ui);
        }
        if ui.is_key_pressed(Key::Escape) {
            if self.stream_menu {
                self.stream_menu = false;
            } else if self.share_open {
                self.share_open = false;
            } else if self.settings.is_open() {
                self.settings.escape(self.settings_popup_open);
            }
        }
        self.finish_voice_flags(ui);
    }

    fn draw_shell(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let show_members = self.place != Place::Home && self.members_open;
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
                self.paint_column(ui, "rail", columns.rail_w, RAIL_BG, true);
                ui.same_line_with_spacing(0.0, 0.0);
                self.paint_column(ui, "sidebar", columns.sidebar_w, SIDEBAR_BG, false);
                ui.same_line_with_spacing(0.0, 0.0);
                self.paint_column(ui, "stage", columns.stage_w, STAGE_BG, false);
                if columns.members_w > 0.0 {
                    ui.same_line_with_spacing(0.0, 0.0);
                    self.paint_column(ui, "members", columns.members_w, SIDEBAR_BG, false);
                }
            });
    }

    fn paint_column(&mut self, ui: &Ui, name: &str, width: f32, color: [f32; 4], rail: bool) {
        let _color = ui.push_style_color(StyleColor::ChildBg, color);
        ui.child_window(name).size([width, 0.0]).build(ui, || {
            if rail {
                self.draw_rail(ui);
            } else if name == "sidebar" {
                self.draw_sidebar_column(ui);
            } else if name == "members" {
                self.draw_members(ui);
            } else {
                self.draw_stage(ui);
            }
        });
    }

    fn draw_login(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        ui.window("Banter")
            .flags(WindowFlags::NO_DOCKING)
            .size([440.0, 340.0], Condition::Always)
            .position([(size[0] - 440.0) * 0.5, (size[1] - 340.0) * 0.5], Condition::Appearing)
            .build(|| {
                ui.text("Welcome back!");
                ui.text_colored(MUTED, "Sign in with your username.");
                ui.input_text("Username", &mut self.username).build();
                {
                    // The password flag hides characters, but it also disables copy.
                    let masked = password_is_masked(self.show_password);
                    let block_copy = password_blocks_copy(self.show_password);
                    let _mask = masked.then(StarMask::push);
                    ui.input_text("##password", &mut self.password).password(block_copy).build();
                }
                ui.same_line();
                ui.text("Password");
                ui.checkbox("Show password", &mut self.show_password);
                if self.register_mode {
                    ui.input_text("Display name", &mut self.register_display).build();
                }
                let label = if self.register_mode { "Create account" } else { "Log in" };
                if ui.button(label) {
                    self.submit_auth();
                }
                ui.same_line();
                if ui.button(if self.register_mode { "Have an account?" } else { "Create one" }) {
                    self.register_mode = !self.register_mode;
                }
                if !self.status.is_empty() {
                    ui.text_wrapped(&self.status);
                }
            });
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
        let home_mark = if self.mentions.is_empty() { "B".to_string() } else { self.mentions.len().to_string() };
        if rail_icon(ui, "home", &home_mark, None, BLURPLE, self.place == Place::Home) {
            self.place = Place::Home;
            self.selected = None;
            self.messages.clear();
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Home");
        }
        rail_gap(ui);
        let servers = self.servers.clone();
        for server in servers {
            let selected = self.place == Place::Server(server.id);
            let mark = initials(&server.name);
            if rail_icon(ui, &format!("sv{}", server.id), &mark, None, server_fill(server.id), selected) {
                self.open_server(server.id);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text(&server.name);
            }
        }
        rail_gap(ui);
        if rail_icon(ui, "add-server-icon", "", self.icon("plus"), GREEN, false) {
            ui.open_popup("add-server");
        }
        if let Some(_popup) = ui.begin_popup("add-server") {
            ui.input_text("Server", &mut self.server_name).build();
            if ui.button("Create server") {
                let name = self.server_name.clone();
                if let Some(home) = self.note(self.client.create_server(&name)) {
                    self.server_name.clear();
                    self.refresh_shell();
                    self.open_server(home.server.id);
                }
            }
            ui.input_text("Invite", &mut self.invite_code).build();
            if ui.button("Join invite") {
                let code = self.invite_code.clone();
                if let Some(home) = self.note(self.client.join_invite(&code)) {
                    self.invite_code.clear();
                    self.refresh_shell();
                    self.open_server(home.server.id);
                }
            }
        }
    }

    fn draw_sidebar_column(&mut self, ui: &Ui) {
        let connected = self.voice_channel.is_some() || self.call.is_some();
        let connection_h = if connected {
            if self.call.is_some() { 210.0 } else { 112.0 }
        } else {
            0.0
        };
        let user_h = 52.0;
        let header_h = 48.0;
        let list_h = (ui.content_region_avail()[1] - header_h - connection_h - user_h).max(40.0);
        ui.child_window("sidebar-header").size([0.0, header_h]).build(ui, || self.draw_sidebar_header(ui));
        ui.child_window("sidebar-list").size([0.0, list_h]).build(ui, || match self.place {
            Place::Home | Place::Direct(_) => self.draw_dm_list(ui),
            Place::Server(server_id) => self.draw_channels(ui, server_id),
        });
        if connected {
            let _panel = ui.push_style_color(StyleColor::ChildBg, PANEL_BG);
            ui.child_window("sidebar-voice").size([0.0, connection_h]).build(ui, || self.draw_connection(ui));
        }
        let _panel = ui.push_style_color(StyleColor::ChildBg, PANEL_BG);
        ui.child_window("sidebar-user").size([0.0, user_h]).build(ui, || self.draw_user_bar(ui));
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
                let text_h = ui.calc_text_size(&name)[1];
                ui.get_window_draw_list().add_text(
                    [origin[0] + 16.0, origin[1] + (48.0 - text_h) * 0.5],
                    TEXT,
                    &name,
                );
                ui.set_cursor_screen_pos([origin[0] + width - 44.0, origin[1] + 8.0]);
                if self.tool(ui, "server-menu-open", "chevron-down", false, Some("Server settings")) {
                    ui.open_popup("server-menu");
                }
                if let Some(_popup) = ui.begin_popup("server-menu") {
                    self.draw_server_menu(ui, server_id);
                }
            }
            Place::Home | Place::Direct(_) => {
                let text_h = ui.calc_text_size("Direct Messages")[1];
                ui.get_window_draw_list().add_text(
                    [origin[0] + 16.0, origin[1] + (48.0 - text_h) * 0.5],
                    TEXT,
                    "Direct Messages",
                );
            }
        }
        header_rule(ui);
    }

    fn draw_server_menu(&mut self, ui: &Ui, server_id: i64) {
        let owner = self.servers.iter().find(|server| server.id == server_id).map(|server| server.owner_id);
        let mine = self.account.as_ref().map(|account| account.id);
        if ui.button("Create invite") {
            if let Some(code) = self.note(self.client.create_invite(server_id)) {
                self.shown_invite = code;
            }
        }
        if !self.shown_invite.is_empty() {
            ui.text_wrapped(format!("Invite code: {}", self.shown_invite));
        }
        ui.input_text("Nickname", &mut self.nickname).build();
        if ui.button("Save nickname") {
            let nickname = {
                let trimmed = self.nickname.trim();
                if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
            };
            if self.note(self.client.set_nickname(server_id, nickname.as_deref())).is_some() {
                self.reload_members();
            }
        }
        if owner == mine {
            ui.separator();
            ui.input_text("Category", &mut self.category_name).build();
            if ui.button("Add category") {
                let name = self.category_name.clone();
                if self.note(self.client.create_category(server_id, &name)).is_some() {
                    self.category_name.clear();
                    self.reload_layout();
                }
            }
            ui.input_text("Channel", &mut self.channel_name).build();
            ui.checkbox("Voice channel", &mut self.channel_is_voice);
            if ui.button("Add channel") {
                self.add_channel(server_id);
            }
        }
    }

    fn draw_dm_list(&mut self, ui: &Ui) {
        if nav_row(ui, "friends", self.icon("users"), "Friends", self.place == Place::Home) {
            self.place = Place::Home;
            self.selected = None;
            self.messages.clear();
        }
        section_label(ui, "DIRECT MESSAGES", self.bold_font, None);
        let dms = self.dms.clone();
        for chat in dms {
            let selected = self.place == Place::Direct(chat.id);
            if nav_row(ui, &format!("dm{}", chat.id), self.icon("user"), &chat.peer_display_name, selected) {
                self.open_dm(chat.id);
            }
        }
        if !self.mentions.is_empty() {
            section_label(ui, "MENTIONS", self.bold_font, None);
            let mentions = self.mentions.clone();
            for mention in mentions {
                let label = format!("{}: {}", mention.from_username, mention.body);
                if nav_row(ui, &format!("men{}", mention.id), None, &label, false) {
                    self.open_mention(&mention);
                }
            }
        }
    }

    fn draw_user_bar(&mut self, ui: &Ui) {
        let username = self.account.as_ref().map(|account| account.username.clone()).unwrap_or_default();
        let account_id = self.account.as_ref().map(|account| account.id);
        let presence = account_id
            .and_then(|id| self.presence.get(&id).copied())
            .unwrap_or(Presence::Online);
        let origin = ui.window_pos();
        let width = ui.window_size()[0];
        let center = [origin[0] + 28.0, origin[1] + 26.0];
        let avatar = account_id.and_then(|id| self.avatars.get(&id).map(|shot| shot.texture));
        // The settings draft is not the saved name until Save succeeds.
        let shown_name = self.account.as_ref().map(|account| account.display_name.clone()).unwrap_or_default();
        paint_avatar(ui, avatar, center, 16.0, &initials(&shown_name), None);
        paint_status_dot(ui, center, 16.0, presence, PANEL_BG);
        let name_x = origin[0] + 52.0;
        let name_h = ui.calc_text_size(&shown_name)[1];
        ui.get_window_draw_list().add_text([name_x, origin[1] + 8.0], TEXT, &shown_name);
        {
            let _small = ui.push_font_with_size(None, 12.0);
            let handle = format!("@{username}");
            ui.get_window_draw_list().add_text([name_x, origin[1] + 10.0 + name_h], MUTED, &handle);
        }
        let mut x = origin[0] + width - 40.0;
        ui.set_cursor_screen_pos([x, origin[1] + 10.0]);
        if self.tool(ui, "open-settings", "settings", self.settings.is_open(), Some("User Settings")) {
            self.open_settings();
        }
        x -= 36.0;
        ui.set_cursor_screen_pos([x, origin[1] + 10.0]);
        let deaf = if self.mute.deafened() { "headphone-off-red" } else { "headphones" };
        if self.tool(ui, "deafen", deaf, false, Some(if self.mute.deafened() { "Undeafen" } else { "Deafen" })) {
            let deafened = !self.mute.deafened();
            self.mute.set_deafened(deafened);
        }
        x -= 36.0;
        ui.set_cursor_screen_pos([x, origin[1] + 10.0]);
        let mic = if self.mute.muted() { "mic-off-red" } else { "mic" };
        if self.tool(ui, "mute", mic, false, Some(if self.mute.muted() { "Unmute" } else { "Mute" })) {
            let muted = !self.mute.muted();
            self.mute.set_muted(muted);
        }
    }

    fn draw_connection(&mut self, ui: &Ui) {
        let origin = ui.window_pos();
        let width = ui.window_size()[0];
        if self.voice_channel.is_some() {
            let channel_id = self.voice_channel.expect("voice channel");
            let name = self
                .layout
                .iter()
                .flat_map(|category| category.channels.iter())
                .find(|channel| channel.id == channel_id)
                .map(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("channel {channel_id}"));
            let server = self
                .layout
                .iter()
                .flat_map(|category| category.channels.iter())
                .find(|channel| channel.id == channel_id)
                .and_then(|channel| self.servers.iter().find(|server| server.id == channel.server_id))
                .map(|server| server.name.clone())
                .unwrap_or_default();
            if let Some(texture) = self.icon("signal-green") {
                ui.get_window_draw_list().add_image(
                    texture,
                    [origin[0] + 8.0, origin[1] + 10.0],
                    [origin[0] + 24.0, origin[1] + 26.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    WHITE,
                );
            }
            ui.get_window_draw_list().add_text([origin[0] + 28.0, origin[1] + 8.0], GREEN, "Voice Connected");
            let subtitle = if server.is_empty() { name.clone() } else { format!("{name} / {server}") };
            {
                let _small = ui.push_font_with_size(None, 12.0);
                ui.get_window_draw_list().add_text([origin[0] + 8.0, origin[1] + 30.0], MUTED, &subtitle);
            }
            ui.set_cursor_screen_pos([origin[0] + width - 40.0, origin[1] + 6.0]);
            if self.tool(ui, "disconnect", "phone-off-red", false, Some("Disconnect")) {
                self.leave_voice();
            }
        }
        if self.call.is_some() || matches!(self.place, Place::Direct(_)) {
            ui.set_cursor_screen_pos([origin[0] + 8.0, origin[1] + 52.0]);
            self.draw_call(ui);
        }
        let button_y = if self.voice_channel.is_some() { origin[1] + 68.0 } else { origin[1] + 8.0 };
        ui.set_cursor_screen_pos([origin[0] + 8.0, button_y]);
        let camera_icon = if self.camera { "video" } else { "video-off" };
        if self.tool(ui, "camera-toggle", camera_icon, self.camera, Some(if self.camera { "Turn Off Camera" } else { "Turn On Camera" })) {
            self.camera = !self.camera;
        }
        ui.set_cursor_screen_pos([origin[0] + 44.0, button_y]);
        let live = self.share.is_live();
        let screen_icon = if live { "monitor" } else { "monitor-off" };
        if self.tool(ui, "screen-toggle", screen_icon, live, Some(if live { "Stop Streaming" } else { "Share Your Screen" })) {
            if live {
                self.share.stop();
                self.save_stream();
            } else {
                self.open_share();
            }
        }
        ui.set_cursor_screen_pos([origin[0] + 76.0, button_y]);
        if self.tool(ui, "stream-options", "chevron-up", self.stream_menu, Some("Stream options")) {
            self.stream_anchor = [origin[0] + 8.0, button_y];
            self.stream_menu = !self.stream_menu;
        }
        if self.push_to_talk {
            ui.set_cursor_screen_pos([origin[0] + 112.0, button_y + 4.0]);
            let _pad = ui.push_style_var(StyleVar::FramePadding([8.0, 4.0]));
            ui.button("Talk");
            self.talk_held = ui.is_item_active();
        } else {
            self.talk_held = false;
        }
        self.video_popout = false;
        if let Some(media) = &self.media {
            for error in media.errors() {
                ui.set_cursor_screen_pos([origin[0] + 8.0, button_y + 36.0]);
                ui.text_colored(RED, device_error_text(&error));
            }
        }
    }

    fn draw_stage(&mut self, ui: &Ui) {
        ui.child_window("stage-header")
            .size([0.0, 48.0])
            .flags(WindowFlags::NO_SCROLLBAR)
            .build(ui, || {
                let width = ui.window_size()[0];
                let origin = ui.window_pos();
                let mut text_x = origin[0] + 16.0;
                if let Some(name) = self.stage_icon() {
                    if let Some(texture) = self.icon(name) {
                        let tint = if name == "hash" { MUTED } else { TEXT };
                        ui.get_window_draw_list().add_image(
                            texture,
                            [text_x, origin[1] + 16.0],
                            [text_x + 18.0, origin[1] + 34.0],
                            [0.0, 0.0],
                            [1.0, 1.0],
                            tint,
                        );
                        text_x += 26.0;
                    }
                }
                let title = self.stage_title();
                let text_h = ui.calc_text_size(&title)[1];
                ui.get_window_draw_list().add_text([text_x, origin[1] + (48.0 - text_h) * 0.5], TEXT, &title);
                let mut right = origin[0] + width - 12.0;
                if self.place != Place::Home {
                    right -= 32.0;
                    ui.set_cursor_screen_pos([right, origin[1] + 8.0]);
                    if self.tool(ui, "members-toggle", "users", self.members_open, Some("Member List")) {
                        self.members_open = !self.members_open;
                    }
                    right -= 8.0;
                }
                if self.chat_target().is_some() && !self.voice_selected() {
                    right -= 196.0;
                    ui.set_cursor_screen_pos([right, origin[1] + 8.0]);
                    if self.tool(ui, "find", "search", false, Some("Search")) {
                        self.reload_messages();
                    }
                    ui.set_cursor_screen_pos([right + 36.0, origin[1] + 10.0]);
                    let _pad = ui.push_style_var(StyleVar::FramePadding([8.0, 4.0]));
                    ui.set_next_item_width(148.0);
                    let submit = ui.input_text("##search", &mut self.search).hint("Search").enter_returns_true(true).build();
                    if submit {
                        self.reload_messages();
                    }
                }
                header_rule(ui);
            });
        if self.voice_selected() {
            self.draw_voice_stage(ui);
        } else if self.place == Place::Home {
            self.draw_people(ui);
        } else {
            self.draw_chat(ui);
        }
        if !self.status.is_empty() {
            ui.separator();
            ui.text_wrapped(&self.status);
        }
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

    fn draw_people(&mut self, ui: &Ui) {
        let tabs = ["Online", "All", "Pending", "Add Friend"];
        for (index, label) in tabs.iter().enumerate() {
            if index > 0 {
                ui.same_line();
            }
            let selected = self.friend_tab == index as u8;
            if chip(ui, &format!("friend-tab-{index}"), label, selected) {
                self.friend_tab = index as u8;
            }
        }
        ui.separator();
        if self.friend_tab == 3 {
            ui.input_text("##friend", &mut self.friend_name).hint("Username").build();
            if ui.button("Send Friend Request") {
                let name = self.friend_name.clone();
                if self.note(self.client.send_friend_request(&name)).is_some() {
                    self.friend_name.clear();
                    self.refresh_shell();
                }
            }
            return;
        }
        if self.friend_tab == 2 {
            section_label(ui, "INCOMING", self.bold_font, None);
            let incoming = self.incoming.clone();
            for request in incoming {
                ui.text(&request.from_username);
                ui.same_line();
                if ui.button(format!("Accept##{}", request.id)) {
                    if self.note(self.client.accept_friend_request(request.id)).is_some() {
                        self.refresh_shell();
                    }
                }
                ui.same_line();
                if ui.button(format!("Decline##{}", request.id)) {
                    if self.note(self.client.decline_friend_request(request.id)).is_some() {
                        self.refresh_shell();
                    }
                }
            }
            if !self.outgoing.is_empty() {
                section_label(ui, "OUTGOING", self.bold_font, None);
                for request in &self.outgoing {
                    ui.text(&request.to_username);
                }
            }
            return;
        }
        let online_only = self.friend_tab == 0;
        section_label(ui, if online_only { "ONLINE" } else { "ALL FRIENDS" }, self.bold_font, None);
        let friends = self.friends.clone();
        let mut shown = 0;
        for friend in friends {
            let presence = self.presence.get(&friend.id).copied().unwrap_or(Presence::Offline);
            if online_only && presence == Presence::Offline {
                continue;
            }
            shown += 1;
            ui.text_colored(presence_color(presence), presence_label(presence));
            ui.same_line();
            if let Some(shot) = self.avatars.get(&friend.id) {
                ui.image(shot.texture, [22.0, 22.0]);
                ui.same_line();
            }
            ui.text(format!("{} (@{})", friend.display_name, friend.username));
            ui.same_line();
            if ui.button(format!("Message##{}", friend.id)) {
                if let Some(chat) = self.note(self.client.open_dm(friend.id)) {
                    self.refresh_shell();
                    self.open_dm(chat.id);
                }
            }
        }
        if shown == 0 {
            ui.text_colored(MUTED, if online_only { "No friends are online right now." } else { "You have not added anyone yet." });
        }
    }

    fn draw_channels(&mut self, ui: &Ui, _server_id: i64) {
        let layout = self.layout.clone();
        let voice_members = self.voice_members.clone();
        for category in layout {
            section_label(ui, &category.category.name.to_uppercase(), self.bold_font, self.icon("chevron-down"));
            for channel in category.channels {
                let icon_name = match channel.kind {
                    ChannelKind::Text => "hash",
                    ChannelKind::Voice => "volume-2",
                };
                let selected = self.selected.as_ref().map(|selected| selected.id) == Some(channel.id);
                if nav_row(ui, &format!("ch{}", channel.id), self.icon(icon_name), &channel.name, selected) {
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
                if channel.kind == ChannelKind::Voice && self.voice_channel == Some(channel.id) {
                    for member in &voice_members {
                        self.draw_voice_member(ui, member);
                    }
                }
            }
        }
    }

    fn draw_members(&mut self, ui: &Ui) {
        let width = ui.window_size()[0].max(1.0);
        ui.dummy([width, 48.0]);
        header_rule(ui);
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
            }
            Place::Direct(dm_id) => {
                if let Some(chat) = self.dms.iter().find(|chat| chat.id == dm_id).cloned() {
                    let presence = self.presence.get(&chat.peer_id).copied().unwrap_or(Presence::Offline);
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
        section_label(ui, &format!("{title} — {}", members.len()), self.bold_font, None);
        for (member, presence) in members {
            let name = member.nickname.clone().unwrap_or(member.display_name.clone());
            self.person_row(ui, member.user_id, &name, &member.username, *presence, member.is_owner);
        }
    }

    fn person_row(&self, ui: &Ui, user_id: i64, name: &str, _username: &str, presence: Presence, owner: bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let _clicked = ui.invisible_button(format!("member-{user_id}"), [width, 42.0]);
        let hovered = ui.is_item_hovered();
        let row_bg = if hovered { ROW_HOVER } else { [0.0, 0.0, 0.0, 0.0] };
        if hovered {
            ui.get_window_draw_list()
                .add_rect([origin[0] + 8.0, origin[1] + 1.0], [origin[0] + width - 8.0, origin[1] + 41.0], ROW_HOVER)
                .rounding(4.0)
                .filled(true)
                .build();
        }
        let center = [origin[0] + 28.0, origin[1] + 21.0];
        let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
        paint_avatar(ui, avatar, center, 16.0, &initials(name), None);
        paint_status_dot(ui, center, 16.0, presence, if hovered { row_bg } else { SIDEBAR_BG });
        let text_h = ui.calc_text_size(name)[1];
        let text_x = origin[0] + 52.0;
        ui.get_window_draw_list().add_text([text_x, origin[1] + (42.0 - text_h) * 0.5], if presence == Presence::Offline { MUTED } else { TEXT }, name);
        if owner {
            if let Some(texture) = self.icon("crown") {
                let name_w = ui.calc_text_size(name)[0];
                ui.get_window_draw_list().add_image(
                    texture,
                    [text_x + name_w + 6.0, origin[1] + 13.0],
                    [text_x + name_w + 20.0, origin[1] + 27.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    WHITE,
                );
            }
        }
    }

    fn draw_chat(&mut self, ui: &Ui) {
        if self.voice_selected() {
            return;
        }
        let Some(target) = self.chat_target() else {
            ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0] + 16.0, ui.cursor_screen_pos()[1] + 16.0]);
            ui.text_colored(MUTED, "Select a channel or a direct message.");
            return;
        };
        let mut reply = None;
        let mut edit = None;
        let mut delete = None;
        let mut save = None;
        let stick = self.messages.len();
        let footer = if self.reply_to.is_some() || self.editing.is_some() || self.pending_file.is_some() { 108.0 } else { 72.0 };
        ui.child_window("history").size([0.0, -footer]).build(ui, || {
            let near_bottom = ui.scroll_max_y() <= 0.0 || ui.scroll_y() >= ui.scroll_max_y() - 24.0;
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
            ui.indent_by(16.0);
            for message in &self.messages {
                let name = message.author_nickname.clone().unwrap_or_else(|| message.author_display_name.clone());
                let edited = if message.edited_at.is_some() { " (edited)" } else { "" };
                if let Some(shot) = self.avatars.get(&message.author_id) {
                    ui.image(shot.texture, [40.0, 40.0]);
                    ui.same_line();
                }
                ui.text(&name);
                ui.same_line();
                ui.text_colored(MUTED, format!("{}{edited}", clock(message.created_at)));
                if let Some(reply_to) = message.reply_to {
                    ui.text_colored(BLURPLE, format!("replying to {reply_to}"));
                }
                if !message.body.is_empty() {
                    ui.text_wrapped(&message.body);
                }
                for attachment in &message.attachments {
                    ui.text(format!("file: {} ({} bytes)", attachment.filename, attachment.byte_size));
                    ui.same_line();
                    if ui.button(format!("Save##att{}", attachment.id)) {
                        save = Some((attachment.id, attachment.filename.clone()));
                    }
                }
                if mine == Some(message.author_id) {
                    if self.tool(ui, &format!("edit-{}", message.id), "pencil", false, Some("Edit")) {
                        edit = Some(message.id);
                    }
                    ui.same_line();
                    if self.tool(ui, &format!("delete-{}", message.id), "trash-2", false, Some("Delete")) {
                        delete = Some(message.id);
                    }
                } else if self.tool(ui, &format!("reply-{}", message.id), "reply", false, Some("Reply")) {
                    reply = Some(message.id);
                }
                ui.separator();
            }
            if near_bottom {
                ui.set_scroll_here_y(1.0);
            }
            ui.unindent_by(16.0);
            let _ = stick;
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
        if let Some(id) = self.reply_to {
            ui.text(format!("Replying to {id}"));
            ui.same_line();
            if ui.button("Cancel reply") {
                self.reply_to = None;
            }
        }
        if let Some(id) = self.editing {
            ui.text(format!("Editing {id}"));
            ui.same_line();
            if ui.button("Cancel edit") {
                self.editing = None;
                self.composer.clear();
            }
        }
        let pending = self.pending_file.as_ref().map(|(name, bytes)| (name.clone(), bytes.len()));
        if let Some((name, len)) = pending {
            ui.text_colored(MUTED, format!("{name} ({len} bytes)"));
            ui.same_line();
            if ui.button("Remove file") {
                self.pending_file = None;
            }
        }
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let bar_h = 44.0;
        let margin = 16.0;
        let bar_x = origin[0] + margin;
        let bar_w = (width - margin * 2.0).max(120.0);
        ui.get_window_draw_list()
            .add_rect([bar_x, origin[1]], [bar_x + bar_w, origin[1] + bar_h], [56.0 / 255.0, 58.0 / 255.0, 64.0 / 255.0, 1.0])
            .rounding(8.0)
            .filled(true)
            .build();
        let input_x = if self.editing.is_none() {
            ui.set_cursor_screen_pos([bar_x + 6.0, origin[1] + 6.0]);
            if self.tool(ui, "attach", "plus", self.pending_file.is_some(), Some("Attach file")) {
                self.pick_attachment();
            }
            bar_x + 44.0
        } else {
            bar_x + 8.0
        };
        ui.set_cursor_screen_pos([input_x, origin[1] + 6.0]);
        let _pad = ui.push_style_var(StyleVar::FramePadding([8.0, 6.0]));
        let _round = ui.push_style_var(StyleVar::FrameRounding(8.0));
        let _border = ui.push_style_var(StyleVar::FrameBorderSize(0.0));
        let _bg = ui.push_style_color(StyleColor::FrameBg, [0.0, 0.0, 0.0, 0.0]);
        ui.set_next_item_width((bar_x + bar_w - 8.0 - input_x).max(40.0));
        let channel_mark = if self.stage_icon() == Some("hash") { "#" } else { "" };
        let hint = format!("Message {channel_mark}{}", self.stage_title());
        let submit = ui.input_text("##composer", &mut self.composer).hint(hint).enter_returns_true(true).build();
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
        ui.get_window_draw_list()
            .add_rect(origin, [origin[0] + width, origin[1] + height], [17.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 1.0])
            .filled(true)
            .build();
        let bar_h = 72.0;
        if !joined {
            if let Some(channel) = channel {
                let label = format!("Join {}", channel.name);
                let text_w = ui.calc_text_size(&label)[0];
                ui.set_cursor_screen_pos([origin[0] + (width - text_w - 28.0) * 0.5, origin[1] + height * 0.45]);
                if ui.button(&label) {
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
        let area_h = (height - bar_h).max(80.0);
        let sharing = self.screen || people.iter().any(|member| member.flags.screen);
        let tiles_top = if sharing { 220.0_f32.min(area_h * 0.45) } else { 0.0 };
        if sharing {
            self.draw_featured_screen_at(ui, origin, width, tiles_top);
        }
        let tile_area_h = (area_h - tiles_top).max(80.0);
        let tile_w = 320.0_f32.min(width - 48.0).max(160.0);
        let tile_h = 190.0_f32.min(tile_area_h - 24.0).max(120.0);
        let cols = ((width - 32.0) / (tile_w + 8.0)).floor().clamp(1.0, 4.0) as usize;
        let rows = people.len().div_ceil(cols).max(1);
        let grid_w = cols as f32 * tile_w + (cols.saturating_sub(1) as f32) * 8.0;
        let grid_h = rows as f32 * tile_h + (rows.saturating_sub(1) as f32) * 8.0;
        let start_x = origin[0] + (width - grid_w) * 0.5;
        let start_y = origin[1] + tiles_top + ((tile_area_h - grid_h) * 0.5).max(8.0);
        for (index, member) in people.iter().enumerate() {
            let col = index % cols;
            let row = index / cols;
            let tile_origin = [start_x + col as f32 * (tile_w + 8.0), start_y + row as f32 * (tile_h + 8.0)];
            self.draw_voice_tile_at(ui, member, tile_origin, tile_w, tile_h);
        }
        self.draw_stage_controls(ui, [origin[0], origin[1] + height - bar_h], width);
        if self.push_to_talk {
            let note = format!("Hold {} to talk.", self.voice.ptt_key);
            let note_w = ui.calc_text_size(&note)[0];
            ui.get_window_draw_list().add_text([origin[0] + (width - note_w) * 0.5, origin[1] + height - 18.0], MUTED, note);
        }
    }

    fn draw_stage_controls(&mut self, ui: &Ui, origin: [f32; 2], width: f32) {
        let count = 4.0;
        let row_w = count * 40.0 + (count - 1.0) * 12.0;
        let mut x = origin[0] + (width - row_w) * 0.5;
        let y = origin[1] + 8.0;
        let camera_icon = if self.camera { "video" } else { "video-off" };
        if self.round_control(ui, "stage-camera", camera_icon, [x, y], false, self.camera) {
            self.camera = !self.camera;
        }
        x += 52.0;
        let live = self.share.is_live();
        let screen_icon = if live { "monitor" } else { "monitor-off" };
        if self.round_control(ui, "stage-screen", screen_icon, [x, y], false, live) {
            if live {
                self.share.stop();
                self.save_stream();
            } else {
                self.open_share();
            }
        }
        ui.set_cursor_screen_pos([x + 28.0, y - 28.0]);
        if self.tool(ui, "stage-stream-options", "chevron-up", self.stream_menu, Some("Stream options")) {
            self.stream_anchor = [x, y];
            self.stream_menu = !self.stream_menu;
        }
        x += 52.0;
        let mic_icon = if self.mute.muted() { "mic-off-red" } else { "mic" };
        if self.round_control(ui, "stage-mic", mic_icon, [x, y], false, self.mute.muted()) {
            let muted = !self.mute.muted();
            self.mute.set_muted(muted);
        }
        x += 52.0;
        if self.round_control(ui, "stage-disconnect", "phone-off", [x, y], true, false) {
            self.leave_voice();
        }
    }

    fn round_control(&self, ui: &Ui, id: &str, icon: &str, origin: [f32; 2], danger: bool, active: bool) -> bool {
        ui.set_cursor_screen_pos(origin);
        let clicked = ui.invisible_button(id, [40.0, 40.0]);
        let min = ui.item_rect_min();
        let fill = if danger {
            RED
        } else if active {
            ROW_SELECTED
        } else if ui.is_item_hovered() {
            ROW_HOVER
        } else {
            PANEL_BG
        };
        let draw = ui.get_window_draw_list();
        draw.add_rect(min, [min[0] + 40.0, min[1] + 40.0], fill).rounding(20.0).filled(true).build();
        if let Some(texture) = self.icon(icon) {
            draw.add_image(texture, [min[0] + 10.0, min[1] + 10.0], [min[0] + 30.0, min[1] + 30.0], [0.0, 0.0], [1.0, 1.0], WHITE);
        }
        clicked
    }

    fn draw_call(&mut self, ui: &Ui) {
        let me = self.account.as_ref().map(|account| account.id);
        if let Some(call) = self.call.clone() {
            match call.phase {
                CallPhase::Ringing if Some(call.callee) == me => {
                    ui.text(format!("Incoming call from {}", self.name_of(call.caller)));
                    if ui.button("Accept call") {
                        if let Some(call) = self.note(self.client.accept_call(call.dm_id)) {
                            self.call = Some(call);
                            self.voice_channel = None;
                        }
                    }
                    ui.same_line();
                    if ui.button("Decline call") {
                        if self.note(self.client.decline_call(call.dm_id)).is_some() {
                            self.end_media_place();
                        }
                    }
                }
                CallPhase::Ringing => {
                    ui.text("Calling…");
                    if ui.button("Cancel call") {
                        if self.note(self.client.end_call(call.dm_id)).is_some() {
                            self.end_media_place();
                        }
                    }
                }
                CallPhase::Active => {
                    ui.text(format!("In a call with {}", self.name_of(if Some(call.caller) == me { call.callee } else { call.caller })));
                    if ui.button("End call") {
                        if self.note(self.client.end_call(call.dm_id)).is_some() {
                            self.end_media_place();
                        }
                    }
                }
            }
            ui.separator();
        } else if let Place::Direct(dm_id) = self.place {
            if ui.button("Start call") {
                if let Some(call) = self.note(self.client.start_call(dm_id)) {
                    self.call = Some(call);
                    self.voice_channel = None;
                    self.voice_members.clear();
                }
            }
            ui.separator();
        }
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
            media.set_stream_choice(self.share.live_target(), stream_max_edge(self.stream_resolution), stream_interval_ms(self.stream_fps));
            media.apply_engine(&self.engine_update());
            media.set_mic_test(self.settings.mic_test());
            media.set_state(self.flags, in_place);
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
            self.composer.clear();
            self.reply_to = None;
            self.pending_file = None;
        }
    }

    fn add_channel(&mut self, server_id: i64) {
        let category_id = self
            .selected
            .as_ref()
            .map(|channel| channel.category_id)
            .or_else(|| self.layout.first().map(|category| category.category.id));
        let Some(category_id) = category_id else {
            self.status = "create a category first".into();
            return;
        };
        let kind = if self.channel_is_voice { ChannelKind::Voice } else { ChannelKind::Text };
        let name = self.channel_name.clone();
        if self.note(self.client.create_channel(server_id, category_id, &name, kind)).is_some() {
            self.channel_name.clear();
            self.reload_layout();
        }
    }

    fn connect_live(&mut self) {
        let Some(token) = self.client.token().map(str::to_string) else { return };
        let realtime = Realtime::connect(websocket_url(&self.base, &token));
        let media = Media::start(realtime.binary_sender());
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

    fn find_channel(&self, channel_id: i64) -> Option<Channel> {
        self.layout.iter().flat_map(|category| category.channels.iter()).find(|channel| channel.id == channel_id).cloned()
    }

    fn pump_events(&mut self) {
        let Some(realtime) = &self.realtime else { return };
        let events = realtime.take_events();
        let frames = realtime.take_frames();
        if let Some(error) = realtime.take_error() {
            self.status = error;
        }
        for event in events {
            self.on_event(event);
        }
        for frame in frames {
            if !self.frame_place_matches(frame.place) {
                continue;
            }
            match frame.kind {
                MediaKind::Audio => {
                    if let Some(media) = &self.media {
                        media.push_remote_audio(&frame.payload);
                    }
                }
                MediaKind::Camera | MediaKind::Screen => {
                    if let Some(media) = &self.media {
                        media.push_remote_video(frame.sender_id, frame.kind, &frame.payload);
                    }
                }
            }
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
                    self.call = Some(call);
                }
            }
            ServerEvent::CallEnded { dm_id } => {
                if self.call.as_ref().map(|call| call.dm_id) == Some(dm_id) {
                    self.call = None;
                }
            }
        }
    }

    fn frame_place_matches(&self, place: MediaPlace) -> bool {
        match place {
            MediaPlace::VoiceChannel(id) => self.voice_channel == Some(id),
            MediaPlace::DirectCall(id) => self.call.as_ref().is_some_and(|call| call.dm_id == id && call.phase == CallPhase::Active),
        }
    }

    fn install_video(&mut self, imgui: &mut Context, frame: VideoFrame) {
        let (jpeg, picture) = picture_bytes(&frame.jpeg);
        let shot = if frame.sender.is_none() && frame.kind == MediaKind::Camera && camera_preview_mirrored() {
            mirrored_camera_shot(imgui, &jpeg)
        } else {
            upload_jpeg(imgui, &jpeg)
        };
        let Some(mut shot) = shot else { return };
        if picture[0] > 1.0 && picture[1] > 1.0 {
            shot.size = picture;
        }
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
            if account.has_avatar {
                wanted.push(account.id);
            }
        }
        for friend in &self.friends {
            if friend.has_avatar {
                wanted.push(friend.id);
            }
        }
        let mut fetched = 0;
        for user_id in wanted {
            if fetched >= 2 || self.avatars.contains_key(&user_id) || self.avatar_miss.contains(&user_id) {
                continue;
            }
            fetched += 1;
            match self.client.avatar(user_id) {
                Ok(bytes) => {
                    if let Some(shot) = upload_jpeg(imgui, &bytes).or_else(|| upload_rgba_png(imgui, &bytes)) {
                        self.avatars.insert(user_id, shot);
                    } else {
                        self.avatar_miss.insert(user_id);
                    }
                }
                Err(_) => {
                    self.avatar_miss.insert(user_id);
                }
            }
        }
    }

    fn pick_avatar(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "gif"]).pick_file() else { return };
        let Ok(bytes) = std::fs::read(&path) else {
            self.status = "could not read that image".into();
            return;
        };
        if bytes.len() > MAX_AVATAR_BYTES {
            self.status = "avatar is larger than 2 MB".into();
            return;
        }
        if self.note(self.client.set_avatar(&bytes)).is_some() {
            if let Some(id) = self.account.as_ref().map(|account| account.id) {
                self.avatars.remove(&id);
                self.avatar_miss.remove(&id);
            }
            if let Some(account) = self.note(self.client.me()) {
                self.account = Some(account);
            }
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
        if self.account.as_ref().map(|account| account.id) == Some(user_id) {
            return "You".into();
        }
        if let Some(member) = self.members.iter().find(|member| member.user_id == user_id) {
            return member.nickname.clone().unwrap_or_else(|| member.display_name.clone());
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
                ui.tooltip_text(tip);
            }
        }
        clicked
    }

    fn voice_settings_visible(&self) -> bool {
        self.settings.is_open() && self.settings.section() == SettingsSection::Voice
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
        if let Some(index) = pick_camera(&self.cameras, &self.voice.camera, self.camera_choice.device_index()) {
            self.camera_choice.select(index);
        }
    }

    fn ensure_icons(&mut self, imgui: &mut Context) {
        if self.icons_ready {
            return;
        }
        for name in [
            "mic", "mic-off", "headphones", "headphone-off", "video", "video-off", "monitor", "monitor-off",
            "phone-off", "settings", "chevron-down", "chevron-up", "plus", "hash", "volume-2", "users", "search", "paperclip",
            "x", "user", "log-out", "pencil", "trash-2", "reply",
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

    /// Full-window page like Discord's User Settings: sidebar on the left, one scrolling content column,
    /// and a close button with ESC beside it. Nothing from the shell shows through.
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
            for section in settings_sections() {
                let (id, icon, label) = match section {
                    SettingsSection::Account => ("settings-account", "user", "My Account"),
                    SettingsSection::Voice => ("settings-voice", "headphones", "Voice & Video"),
                };
                if nav_row(ui, id, self.icon(icon), label, self.settings.section() == section) {
                    self.select_settings(section);
                }
                ui.dummy([0.0, 2.0]);
            }
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

        settings_form_label(ui, self.bold_font, "VOICE PROCESSING");
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
        settings_divider(ui, width);
        settings_spacer(ui);

        settings_page_title(ui, self.bold_font, "Video Settings");
        self.draw_camera_picker(ui, width);
        settings_spacer(ui);
        settings_divider(ui, width);

        settings_page_title(ui, self.bold_font, "Screen Share");
        self.draw_stream_quality(ui, width);
        settings_spacer(ui);
        self.draw_audio_sources(ui, width);
        settings_spacer(ui);
        ui.text_colored(
            MUTED,
            "Open Share Your Screen from a voice channel or a call. Pick a screen or an application, then press Go Live. Nothing is shared before that.",
        );
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
        let key_text = if recording { "Press a key…".to_string() } else { self.voice.ptt_key.clone() };
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
            let text = if self.cameras.is_empty() { "No camera" } else { "Starting camera…" };
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
        if let Some(reason) = hardware_encoder_block(false) {
            ui.text_colored(MUTED, format!("Software JPEG. {reason}"));
        }
        self.stream.hardware = false;
    }

    fn draw_share(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR;
        ui.window("Screen Share")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .bg_alpha(0.72)
            .focused(true)
            .build(|| {
                let panel_w = (size[0] - 80.0).clamp(700.0, 960.0);
                let panel_h = (size[1] - 60.0).clamp(480.0, 680.0);
                ui.set_cursor_pos([(size[0] - panel_w) * 0.5, (size[1] - panel_h) * 0.5]);
                let _card = ui.push_style_color(StyleColor::ChildBg, SIDEBAR_BG);
                ui.child_window("share-card").size([panel_w, panel_h]).build(ui, || {
                    ui.text("Screen Share");
                    ui.same_line();
                    if self.tool(ui, "close-share", "x", false, Some("Close")) {
                        self.share_open = false;
                    }
                    if chip(ui, "share-screens", "Screens", self.share_tab == 0) && self.share_tab != 0 {
                        self.share_tab = 0;
                        self.reload_share_sources();
                    }
                    ui.same_line();
                    if chip(ui, "share-apps", "Applications", self.share_tab == 1) && self.share_tab != 1 {
                        self.share_tab = 1;
                        self.reload_share_sources();
                    }
                    ui.separator();
                    let sources = self
                        .share_sources
                        .iter()
                        .map(|source| (source.kind, source.id, source.name.clone()))
                        .collect::<Vec<_>>();
                    ui.child_window("share-grid").size([0.0, -128.0]).build(ui, || {
                        if sources.is_empty() {
                            ui.text_colored(
                                MUTED,
                                if self.share_tab == 0 { "No screens were found." } else { "No application windows were found." },
                            );
                        }
                        for (index, (kind, id, name)) in sources.iter().enumerate() {
                            if index % 3 != 0 {
                                ui.same_line();
                            }
                            let key = source_key(*kind, *id);
                            let selected = self.share.selected() == Some((*kind, *id));
                            let _fill = ui.push_style_color(StyleColor::ChildBg, if selected { ROW_SELECTED } else { PANEL_BG });
                            ui.child_window(format!("share-{}-{}", key.0, key.1))
                                .size([210.0, 156.0])
                                .border(selected)
                                .build(ui, || {
                                    let origin = ui.cursor_pos();
                                    if let Some(shot) = self.share_thumbs.get(&key) {
                                        ui.image(shot.texture, fit_inside(shot.size, 190.0, 96.0));
                                    } else {
                                        ui.dummy([190.0, 96.0]);
                                    }
                                    ui.text_wrapped(name);
                                    ui.set_cursor_pos(origin);
                                    if ui.invisible_button(format!("pick-{}-{}", key.0, key.1), [190.0, 140.0]) {
                                        if self.share.is_live() {
                                            self.share.apply_while_live(*kind, *id);
                                        } else {
                                            self.share.select(*kind, *id);
                                        }
                                        self.save_stream();
                                    }
                                });
                        }
                    });
                    let options_w = ui.content_region_avail()[0].max(120.0);
                    self.draw_stream_quality(ui, options_w);
                    self.draw_audio_sources(ui, options_w);
                    if ui.button("Go Live") {
                        if self.share.go_live() {
                            self.share_open = false;
                        } else {
                            self.status = "choose a screen or an application first".into();
                        }
                    }
                    if self.share.is_live() {
                        ui.same_line();
                        let _red = ui.push_style_color(StyleColor::Button, RED);
                        if ui.button("Stop Streaming") {
                            self.share.stop();
                            self.share_open = false;
                        }
                    }
                });
                self.queue_next_thumb();
            });
    }

    fn reload_share_sources(&mut self) {
        self.retired_shots.extend(self.share_thumbs.drain().map(|(_, shot)| shot));
        self.share_miss.clear();
        self.thumb_queue.clear();
        self.share_thumb_next = 0;
        self.share_sources = if self.share_tab == 0 { list_screens() } else { list_windows() };
    }

    fn queue_next_thumb(&mut self) {
        if !self.thumb_queue.is_empty() {
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
            if let Some(jpeg) = capture_source_jpeg(kind, id) {
                self.thumb_queue.push((key.0, key.1, jpeg));
            } else {
                self.share_miss.insert(key);
            }
            break;
        }
    }

    fn draw_voice_member(&self, ui: &Ui, member: &VoiceMember) {
        let name = self.name_of(member.user_id);
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        ui.dummy([width, 32.0]);
        let center = [origin[0] + 42.0, origin[1] + 16.0];
        let avatar = self.avatars.get(&member.user_id).map(|shot| shot.texture);
        let transmitting = self.member_talking(member);
        let ring = transmitting.then_some(GREEN);
        paint_avatar(ui, avatar, center, 12.0, &initials(&name), ring);
        let text_h = ui.calc_text_size(&name)[1];
        ui.get_window_draw_list().add_text(
            [origin[0] + 60.0, origin[1] + (32.0 - text_h) * 0.5],
            if self.member_talking(member) { TEXT } else { MUTED },
            &name,
        );
        let mut icon_x = origin[0] + width - 22.0;
        let draw_icon = |icon_x: f32, texture: ManagedTextureId| {
            ui.get_window_draw_list().add_image(
                texture,
                [icon_x, origin[1] + 9.0],
                [icon_x + 14.0, origin[1] + 23.0],
                [0.0, 0.0],
                [1.0, 1.0],
                MUTED,
            );
        };
        if member.flags.screen {
            if let Some(texture) = self.icon("monitor") {
                draw_icon(icon_x, texture);
                icon_x -= 18.0;
            }
        }
        if member.flags.camera {
            if let Some(texture) = self.icon("video") {
                draw_icon(icon_x, texture);
                icon_x -= 18.0;
            }
        }
        if member.flags.muted || member.flags.deafened {
            let icon = if member.flags.deafened { "headphone-off" } else { "mic-off" };
            if let Some(texture) = self.icon(icon) {
                draw_icon(icon_x, texture);
            }
        }
    }

    fn draw_featured_screen_at(&self, ui: &Ui, origin: [f32; 2], width: f32, height: f32) {
        let featured = if self.screen {
            self.local_screen.as_ref().map(|shot| ("Your screen".to_string(), shot.texture, shot.size))
        } else {
            None
        };
        let featured = featured.or_else(|| {
            self.voice_members.iter().find(|member| member.flags.screen).and_then(|member| {
                self.remote_video.get(&(member.user_id, 3)).map(|shot| {
                    (format!("{} is sharing", self.name_of(member.user_id)), shot.texture, shot.size)
                })
            })
        });
        let Some((label, texture, size)) = featured else { return };
        let shown = if self.screen_shown[0] > 1.0 { self.screen_shown } else { size };
        let fitted = fit_inside(shown, width - 48.0, (height - 28.0).max(32.0));
        let x = origin[0] + (width - fitted[0]) * 0.5;
        let y = origin[1] + 24.0;
        ui.get_window_draw_list().add_text([x, origin[1] + 4.0], GREEN, &label);
        ui.get_window_draw_list().add_image_rounded(
            texture,
            [x, y],
            [x + fitted[0], y + fitted[1]],
            [0.0, 0.0],
            [1.0, 1.0],
            WHITE,
            8.0,
            DrawFlags::ROUND_ALL,
        );
    }

    fn draw_voice_tile_at(&self, ui: &Ui, member: &VoiceMember, origin: [f32; 2], tile_w: f32, tile_h: f32) {
        let name = self.name_of(member.user_id);
        let me = self.account.as_ref().map(|account| account.id);
        let talking = self.member_talking(member);
        let max = [origin[0] + tile_w, origin[1] + tile_h];
        ui.get_window_draw_list()
            .add_rect(origin, max, [43.0 / 255.0, 45.0 / 255.0, 49.0 / 255.0, 1.0])
            .rounding(8.0)
            .filled(true)
            .build();
        if talking {
            ui.get_window_draw_list()
                .add_rect(origin, max, GREEN)
                .rounding(8.0)
                .filled(false)
                .build();
        }
        if let Some((texture, size)) = self.tile_camera(member, me) {
            let fitted = fit_inside(size, tile_w - 16.0, tile_h - 44.0);
            let x = origin[0] + (tile_w - fitted[0]) * 0.5;
            let y = origin[1] + 12.0;
            ui.get_window_draw_list().add_image_rounded(
                texture,
                [x, y],
                [x + fitted[0], y + fitted[1]],
                [0.0, 0.0],
                [1.0, 1.0],
                WHITE,
                8.0,
                DrawFlags::ROUND_ALL,
            );
        } else {
            let radius = 36.0;
            let center = [origin[0] + tile_w * 0.5, origin[1] + tile_h * 0.42];
            let avatar = self.avatars.get(&member.user_id).map(|shot| shot.texture);
            let ring = talking.then_some(GREEN);
            paint_avatar(ui, avatar, center, radius, &initials(&name), ring);
        }
        ui.get_window_draw_list().add_text([origin[0] + 12.0, max[1] - 28.0], TEXT, &name);
        let icon_x = max[0] - 28.0;
        if member.flags.deafened {
            if let Some(texture) = self.icon("headphone-off") {
                ui.get_window_draw_list().add_image(
                    texture,
                    [icon_x, max[1] - 28.0],
                    [icon_x + 16.0, max[1] - 12.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    MUTED,
                );
            }
        } else if member.flags.muted {
            if let Some(texture) = self.icon("mic-off") {
                ui.get_window_draw_list().add_image(
                    texture,
                    [icon_x, max[1] - 28.0],
                    [icon_x + 16.0, max[1] - 12.0],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    MUTED,
                );
            }
        }
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
        if let Some(stream) = load_json::<StreamDefaults>(&self.data_dir.join("stream.json")) {
            self.stream_resolution = resolution_from_key(&stream.resolution);
            self.stream_fps = fps_from_key(&stream.fps);
            self.stream = stream;
            self.stream.hardware = false;
        }
    }

    fn save_voice(&self) {
        let mut voice = self.voice.clone();
        voice.push_to_talk = self.push_to_talk;
        let _ = save_json(&self.data_dir.join("voice.json"), &voice);
    }

    fn save_stream(&mut self) {
        self.stream.resolution = resolution_key(self.stream_resolution).to_string();
        self.stream.fps = fps_key(self.stream_fps).to_string();
        self.stream.hardware = false;
        let _ = save_json(&self.data_dir.join("stream.json"), &self.stream);
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
            quality: jpeg_quality_for_bitrate(self.stream.bitrate_auto, self.stream.bitrate_kbps),
            show_cursor: self.stream.show_cursor,
            show_preview: self.stream.show_preview,
            sources: self.stream.sources.clone(),
        }
    }

    fn ptt_down(&self, ui: &Ui) -> bool {
        let key = ptt_key(&self.voice.ptt_key).unwrap_or(Key::V);
        ui.is_key_down(key) || (key == Key::V && self.v_held)
    }

    fn capture_ptt_key(&mut self, ui: &Ui) {
        if !self.settings.recording() {
            return;
        }
        for name in ptt_key_names() {
            let Some(key) = ptt_key(name) else { continue };
            if ui.is_key_pressed(key) {
                self.voice.ptt_key = name.to_string();
                self.settings.finish_recording();
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

    /// Live mic level on the same dB scale as the sensitivity slider. The white mark is the send threshold.
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
        let width = 300.0;
        let height = 250.0;
        let x = self.stream_anchor[0];
        let y = (self.stream_anchor[1] - height - 8.0).max(8.0);
        ui.window("Stream options")
            .flags(WindowFlags::NO_DECORATION | WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_DOCKING)
            .position([x, y], Condition::Always)
            .size([width, height], Condition::Always)
            .build(|| {
                ui.text("Stream options");
                if let Some(reason) = hardware_encoder_block(false) {
                    ui.text_wrapped(format!("Software JPEG. {reason}"));
                }
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
                if ui.checkbox("Show cursor", &mut self.stream.show_cursor) {
                    self.save_stream();
                }
                if ui.checkbox("Show my preview", &mut self.stream.show_preview) {
                    self.save_stream();
                }
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

fn picture_bytes(bytes: &[u8]) -> (Vec<u8>, [f32; 2]) {
    let (_key, width, height, jpeg) = unpack_picture(bytes);
    let size = if width > 0 && height > 0 { [width as f32, height as f32] } else { [0.0, 0.0] };
    (jpeg.to_vec(), size)
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
    match kind {
        MediaKind::Camera => 2,
        MediaKind::Screen => 3,
        MediaKind::Audio => 1,
    }
}

fn fit_inside(size: [f32; 2], max_width: f32, max_height: f32) -> [f32; 2] {
    if size[0] <= 1.0 || size[1] <= 1.0 || max_width <= 1.0 || max_height <= 1.0 {
        return [32.0, 32.0];
    }
    let scale = (max_width / size[0]).min(max_height / size[1]);
    [size[0] * scale, size[1] * scale]
}

fn apply_discord_style(style: &mut Style) {
    style.set_color(StyleColor::Text, TEXT);
    style.set_color(StyleColor::TextDisabled, MUTED);
    style.set_color(StyleColor::WindowBg, STAGE_BG);
    style.set_color(StyleColor::ChildBg, STAGE_BG);
    style.set_color(StyleColor::PopupBg, [17.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 1.0]);
    style.set_color(StyleColor::Border, [0.0, 0.0, 0.0, 0.45]);
    style.set_color(StyleColor::FrameBg, [56.0 / 255.0, 58.0 / 255.0, 64.0 / 255.0, 1.0]);
    style.set_color(StyleColor::FrameBgHovered, ROW_SELECTED);
    style.set_color(StyleColor::FrameBgActive, ROW_SELECTED);
    style.set_color(StyleColor::Button, BLURPLE);
    style.set_color(StyleColor::ButtonHovered, [71.0 / 255.0, 82.0 / 255.0, 196.0 / 255.0, 1.0]);
    style.set_color(StyleColor::ButtonActive, [60.0 / 255.0, 69.0 / 255.0, 165.0 / 255.0, 1.0]);
    style.set_color(StyleColor::Header, ROW_SELECTED);
    style.set_color(StyleColor::HeaderHovered, ROW_HOVER);
    style.set_color(StyleColor::HeaderActive, ROW_SELECTED);
    style.set_color(StyleColor::ScrollbarBg, [0.0, 0.0, 0.0, 0.0]);
    style.set_color(StyleColor::ScrollbarGrab, [26.0 / 255.0, 27.0 / 255.0, 30.0 / 255.0, 1.0]);
    style.set_color(StyleColor::TextSelectedBg, [88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, 0.45]);
    style.set_frame_rounding(8.0);
    style.set_window_rounding(8.0);
    style.set_popup_rounding(8.0);
    style.set_scrollbar_rounding(8.0);
    style.set_window_border_size(0.0);
    style.set_child_border_size(0.0);
    style.set_docking_separator_size(0.0);
    style.set_frame_padding([12.0, 8.0]);
    style.set_item_spacing([8.0, 4.0]);
    style.set_window_padding([8.0, 8.0]);
}

fn rail_icon(ui: &Ui, id: &str, letters: &str, icon: Option<ManagedTextureId>, fill: [f32; 4], selected: bool) -> bool {
    let width = ui.content_region_avail()[0].max(48.0);
    let origin = ui.cursor_screen_pos();
    let clicked = ui.invisible_button(id, [width, 56.0]);
    let hovered = ui.is_item_hovered();
    let draw = ui.get_window_draw_list();
    let center_x = origin[0] + width * 0.5;
    let center_y = origin[1] + 28.0;
    let squircle = selected || hovered;
    if squircle {
        draw.add_rect([center_x - 24.0, center_y - 24.0], [center_x + 24.0, center_y + 24.0], fill)
            .rounding(12.0)
            .filled(true)
            .build();
    } else {
        draw.add_circle([center_x, center_y], 24.0, fill).filled(true).build();
    }
    if selected || hovered {
        let pill_h = if selected { 40.0 } else { 20.0 };
        draw.add_rect([origin[0], center_y - pill_h * 0.5], [origin[0] + 4.0, center_y + pill_h * 0.5], WHITE)
            .rounding(2.0)
            .filled(true)
            .build();
    }
    if let Some(texture) = icon {
        draw.add_image(
            texture,
            [center_x - 12.0, center_y - 12.0],
            [center_x + 12.0, center_y + 12.0],
            [0.0, 0.0],
            [1.0, 1.0],
            WHITE,
        );
    } else if !letters.is_empty() {
        let text_size = ui.calc_text_size(letters);
        draw.add_text(
            [center_x - text_size[0] * 0.5, center_y - text_size[1] * 0.5],
            WHITE,
            letters,
        );
    }
    clicked
}

fn rail_gap(ui: &Ui) {
    let width = ui.content_region_avail()[0].max(48.0);
    let origin = ui.cursor_screen_pos();
    ui.dummy([width, 12.0]);
    let center_x = origin[0] + width * 0.5;
    ui.get_window_draw_list()
        .add_rect([center_x - 16.0, origin[1] + 5.0], [center_x + 16.0, origin[1] + 7.0], [79.0 / 255.0, 84.0 / 255.0, 92.0 / 255.0, 1.0])
        .rounding(1.0)
        .filled(true)
        .build();
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

/// Discord's 12 px bold uppercase label that sits above a control.
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

/// Label on the left, switch on the right. The whole row toggles.
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

/// Dear ImGui only closes a dropdown on Escape when keyboard navigation is on, and Banter leaves it off.
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

/// Discord-style slider: a thin track with a knob, and the value printed to the right so the knob never hides it.
/// Dear ImGui's slider still handles the dragging; its own frame, grab, and text are made transparent.
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

/// Edits the 0.0 to 2.0 multiplier live as 0% to 200%. Returns true once the drag ends, which is when to save.
fn settings_volume_slider(ui: &Ui, id: &str, volume: &mut f32, width: f32) -> bool {
    let mut percent = volume_to_percent(*volume);
    if settings_slider_i32(ui, &format!("##{id}"), &mut percent, 0, 200, width, "%") {
        *volume = percent_to_volume(percent);
    }
    ui.is_item_deactivated_after_edit()
}

/// Chips that wrap to the next line. A disabled chip stays visible but dim. Returns the clicked index.
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
        if ui.calc_text_size(format!("{shown}…"))[0] > width {
            shown.pop();
            break;
        }
    }
    format!("{shown}…")
}

fn section_label(ui: &Ui, text: &str, bold: Option<FontId>, chevron: Option<ManagedTextureId>) {
    let width = ui.content_region_avail()[0].max(32.0);
    let origin = ui.cursor_screen_pos();
    ui.dummy([width, 28.0]);
    let _font = ui.push_font_with_size(bold, 12.0);
    let mut x = origin[0] + 8.0;
    if let Some(texture) = chevron {
        ui.get_window_draw_list().add_image(
            texture,
            [x, origin[1] + 8.0],
            [x + 12.0, origin[1] + 20.0],
            [0.0, 0.0],
            [1.0, 1.0],
            MUTED,
        );
        x += 16.0;
    }
    let text_h = ui.calc_text_size(text)[1];
    ui.get_window_draw_list().add_text([x, origin[1] + (28.0 - text_h) * 0.5], MUTED, text);
}

fn header_rule(ui: &Ui) {
    let pos = ui.window_pos();
    let size = ui.window_size();
    ui.get_window_draw_list()
        .add_rect([pos[0], pos[1] + 47.0], [pos[0] + size[0], pos[1] + 48.0], [0.0, 0.0, 0.0, 0.45])
        .filled(true)
        .build();
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
    let dot = [center[0] + radius * 0.62, center[1] + radius * 0.62];
    let draw = ui.get_window_draw_list();
    draw.add_circle(dot, 7.0, backdrop).filled(true).build();
    draw.add_circle(dot, 5.0, presence_color(presence)).filled(true).build();
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

fn server_fill(id: i64) -> [f32; 4] {
    const COLORS: [[f32; 4]; 6] = [
        [88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, 1.0],
        [35.0 / 255.0, 165.0 / 255.0, 89.0 / 255.0, 1.0],
        [240.0 / 255.0, 178.0 / 255.0, 50.0 / 255.0, 1.0],
        [235.0 / 255.0, 69.0 / 255.0, 158.0 / 255.0, 1.0],
        [87.0 / 255.0, 170.0 / 255.0, 242.0 / 255.0, 1.0],
        [155.0 / 255.0, 89.0 / 255.0, 182.0 / 255.0, 1.0],
    ];
    COLORS[(id.unsigned_abs() as usize) % COLORS.len()]
}

fn presence_color(presence: Presence) -> [f32; 4] {
    match presence {
        Presence::Online => GREEN,
        Presence::Idle => [240.0 / 255.0, 178.0 / 255.0, 50.0 / 255.0, 1.0],
        Presence::Offline => MUTED,
    }
}

fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Online => "online",
        Presence::Idle => "idle",
        Presence::Offline => "offline",
    }
}

fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}
