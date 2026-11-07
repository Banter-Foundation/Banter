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
    has_permission, top_hoisted_role, top_role_color, Account, BadgeItem, CallPhase, CallSnapshot,
    CategoryLayout, Channel, ChannelKind, ChatTarget, ClientSignal, CosmeticItem, CosmeticKind,
    DirectChat, Friend, FriendRequest, GuildRole, LayerId, LayerPreferences, MediaKind, MediaPlace,
    Mention, Message, Presence, ProfileActivityInput, ProfileConnectionInput, ProfilePatch,
    ProfileWidgetInput, ProfileWidgetKind, ProfileWishlistInput, ServerDebug, ServerFolderPatch,
    ServerRail, ServerSummary, UserProfile, VoiceFlags, VoiceMember, CHANNEL_ICON_CHOICES,
    MAX_AVATAR_BYTES, MAX_BANNER_BYTES, PERM_ADMINISTRATOR, PERM_CREATE_INVITE, PERM_MANAGE_BOTS,
    PERM_MANAGE_CHANNELS, PERM_MANAGE_PLUGINS, PERM_MANAGE_ROLES,
};
use dear_imgui_rs::{
    fonts::FontId, ChildFlags, Condition, Context, DrawFlags, Key, ManagedTextureId, MouseButton,
    OwnedTextureData, PopupQueryFlags, Style, StyleColor, StyleVar, TextureFormat, TreeNodeFlags,
    Ui, WindowFlags,
};

use crate::api::{ApiError, Client, MemberView};
use crate::bfmd;
use crate::bfmd_paint;
use crate::icons::rasterize_lucide;
use crate::layout::discord_shell_columns;
use crate::logic::{
    api_base_for, apply_voice_roster, camera_preview_mirrored, capture_voice_rejoin_intent,
    channel_context_menu, channel_default_icon, channel_display_icon, chat_date_separator,
    chat_time_hover, chat_time_label, clamp_context_menu_pos, clamp_window_size, comment_tree_rows,
    context_menu_height, crop_avatar_rgba, crop_window, default_stream_fps,
    default_stream_resolution, dismiss_stream_overlays, encode_jpeg_rgba, folder_context_menu,
    encoder_availability_note, encoder_mode_from_key, encoder_mode_key, fps_block, fps_from_key,
    fps_key, grouped_mix_sources, hardware_encoder_block,
    home_section_is_visible, home_section_label, missing_encoder_block,
    insert_mention, is_debug_command, lerp_size, load_json, local_api_base, member_context_menu,
    message_context_menu, message_starts_new_day, mirror_rgba, move_home_section, next_reconnect_at,
    parse_folder_color, password_blocks_copy, password_is_masked, plan_member_groups,
    push_home_recent, push_to_talk_held, rail_tree_rows, rate_control_from_key, rate_control_key,
    rate_control_label, reconnect_retry_ready, remove_message, resolution_block, resolution_from_key,
    resolution_key, save_json, server_rail_context_menu, server_rail_item_menu,
    set_home_section_visible, speaking_from_level, stream_fps_label, stream_frame_rates,
    stream_interval_ms, stream_max_edge, stream_resolution_label,
    stream_resolutions, toggle_comment_collapsed, upsert_message, visible_home_sections,
    voice_flags_retry_ready, voice_occupancy_line, voice_rtc_mesh, voice_rtc_session_ready,
    window_size_label,
    window_size_presets, CameraChoice, CaptureKind, ContextMenuAction, ContextMenuEntry,
    ContextMenuTone, DevServerPrefs, HomeLayout, HomeRecentEntry, HomeRecentKind, HomeSectionId,
    MixSource, MuteMemory, RailTreeRow, ScreenShare, ServerTarget, SettingsSection, StreamDefaults,
    StreamEncoderMode, StreamFps, StreamRateControl, StreamResolution, VideoPrefs, VoicePrefs,
    VoiceRejoinIntent, OPEN_MIC_RMS, RECONNECTING_STATUS,
};
use crate::media::{self, EngineUpdate, Media, StreamPick, VideoFrame};
use crate::sfx::{ringtone_duration, Sfx, SoundEvent};
use crate::profile::{
    display_name_style_color, games_from_payload, games_to_payload, games_widget_cap,
    profile_badge_row_visible_cap, profile_board_tabs, profile_column_content_width,
    profile_editor_layout, profile_editor_slot_rows, profile_heading, profile_primary_actions,
    profile_view_layout, rgb_u32_to_rgba, ProfileAction, ProfileBoardTab, ProfileCard,
    ProfileEditScope, ProfileEditorSlot, ProfileKind, ProfileMediaDraft, ProfileMode,
    ProfilePanelRect,
};
use crate::profile_paint;
use crate::profiler;
use crate::realtime::Realtime;
use crate::session::{clear_token, load_token, persist_login};
use crate::settings::{
    account_avatar_actions_place, db_to_rms, device_error_text, display_name_dirty,
    display_name_error, meter_fraction, percent_to_volume, rms_to_db, settings_layout,
    settings_nav_list_w, settings_nav_list_x, slider_fraction, slider_track_width,
    voice_bar_error_lines, volume_to_percent, wrap_rows, SettingsState, SETTINGS_NAV_HEADING_GAP,
    SETTINGS_NAV_SECTION_GAP, SETTINGS_TOP_PAD,
};
use crate::shell_sync::{
    snapshot_applies, AvatarFile, AvatarWant, FetchKind, ShellHost, ShellQuery, ShellSnapshot,
};
use crate::skin::{self, MenuTone, Tone};
use crate::coverflow::{
    coverflow_brightness, coverflow_cover_uv, coverflow_layout, coverflow_nearest,
    coverflow_nudge_focus, coverflow_present_alpha, coverflow_rise_offset, coverflow_step_focus,
    coverflow_step_present, COVERFLOW_FOCUS_BORDER, COVERFLOW_RISE_PX,
};
use crate::stream_watch::{
    cycle_streamer, resolve_focused_streamer, stream_chrome_alpha, stream_watch_avatar_x,
    streaming_user_ids, STREAM_CHROME_HIDE_ALPHA, STREAM_POPOUT_WINDOW_TITLE,
};
use crate::theme::*;

#[derive(Clone, Debug)]
struct PresenceInfo {
    state: Presence,
    custom_status: Option<String>,
}

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

/// What the Home place shows in the stage (BA-81).
/// Rail Home opens the customizable landing; the sidebar Friends row opens the people list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HomePane {
    Landing,
    Friends,
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
    EditChannel,
    CreateCategory,
    Roles,
    MemberRoles,
    /// Create or edit a server-rail folder (name + hex color).
    ServerFolder,
    /// Chat file attach is gated off (BA-61); shows Owl's exact unavailable copy.
    UploadsUnavailable,
    /// Install a package from the data directory's `plugins` folder onto one server.
    AddPlugin,
    /// Permission list, then the file warning when the plugin checked `io` or `os`.
    PluginAllow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FolderModalMode {
    Create,
    Rename,
    Recolor,
}

/// Exact popup copy Owl specified for disabled chat file uploads (BA-61).
const UPLOADS_UNAVAILABLE_MESSAGE: &str = "Currently Unavailable";

#[derive(Clone, Copy, PartialEq, Eq)]
enum RailTone {
    Server,
    Folder,
    Add,
}

/// Floating Discord-style context menu opened by right-click (or the message "More" button).
#[derive(Clone)]
struct OpenContextMenu {
    entries: Vec<ContextMenuEntry>,
    pos: [f32; 2],
    /// Skips the opening click so the same right-click does not dismiss the menu.
    frames: u32,
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
    /// When true, successful sign-in writes `client-session.json` for the next launch.
    stay_signed_in: bool,
    register_display: String,
    friends: Vec<Friend>,
    incoming: Vec<FriendRequest>,
    outgoing: Vec<FriendRequest>,
    servers: Vec<ServerSummary>,
    /// Per-user nested rail folders + placements (BA-79).
    server_rail: ServerRail,
    folder_name: String,
    folder_color: String,
    folder_parent: Option<i64>,
    editing_folder: Option<i64>,
    folder_mode: FolderModalMode,
    folder_error: String,
    plugin_server: Option<i64>,
    plugin_pending: Option<String>,
    plugin_confirm_step: u8,
    plugin_notice: String,
    local_plugins: Vec<crate::plugins::LocalPlugin>,
    installed_plugins: Vec<(String, String, String)>,
    dms: Vec<DirectChat>,
    mentions: Vec<Mention>,
    presence: HashMap<i64, PresenceInfo>,
    place: Place,
    layout: Vec<CategoryLayout>,
    members: Vec<MemberView>,
    selected: Option<Channel>,
    messages: Vec<Message>,
    voice_members: Vec<VoiceMember>,
    voice_rosters: HashMap<i64, Vec<VoiceMember>>,
    voice_channel: Option<i64>,
    call: Option<CallSnapshot>,
    mute: MuteMemory,
    push_to_talk: bool,
    camera: bool,
    screen: bool,
    flags: VoiceFlags,
    sent_flags: VoiceFlags,
    /// Last GuildP2pVideo preference pushed over the websocket (None until first send).
    sent_guild_p2p_video: Option<bool>,
    /// Last failed `POST /v1/voice/flags` time; backs off retries so the notice is not spammed.
    flags_failed_at: Option<Instant>,
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
    /// Optional Lucide id while creating or editing a channel.
    channel_icon: Option<String>,
    /// Channel being edited in [`Modal::EditChannel`].
    channel_edit_id: Option<i64>,
    nickname: String,
    about_draft: String,
    status_draft: String,
    guild_roles: Vec<GuildRole>,
    role_name: String,
    role_color: String,
    role_hoist: bool,
    role_admin: bool,
    role_channels: bool,
    role_manage: bool,
    role_invite: bool,
    role_plugins: bool,
    role_bots: bool,
    editing_role: Option<i64>,
    role_error: String,
    assign_user: Option<i64>,
    assign_role_ids: Vec<i64>,
    display_name: String,
    composer: String,
    search: String,
    friend_tab: u8,
    /// Home stage pane: customizable landing vs Friends people list (BA-81).
    home_pane: HomePane,
    /// Layout order, visibility, and recent destinations for the Home landing (`home.json`).
    home_layout: HomeLayout,
    /// When true, the Home landing shows reorder / show-hide controls.
    home_editing: bool,
    settings: SettingsState,
    settings_error: String,
    /// Last successful health report's version label (empty if unknown / unreachable).
    server_version_label: String,
    /// Last successful health report's protocol (None if host omitted it).
    server_protocol: Option<u32>,
    /// Soft mismatch notice when protocols match but release labels differ.
    version_warn: String,
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
    /// Continuous cover-flow focus (source index); eases toward `share_cover_target`.
    share_cover_focus: f32,
    /// Integer (or nudged) cover-flow target the carousel settles on.
    share_cover_target: f32,
    /// Soft open/close presentation for the cover-flow picker (`0` hidden → `1` shown).
    share_present: f32,
    /// Expanded glass chip shows full quality + stream audio; collapsed is a compact summary.
    share_chip_expanded: bool,
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
    /// Separate Banter Stream OS window showing the focused share.
    stream_popout: bool,
    /// Stage theater mode: stream fills the window; chrome fades when the pointer idles.
    stream_fullscreen: bool,
    /// Preferred live streamer (`None` = first available).
    focused_streamer: Option<i64>,
    /// Last pointer/key activity used for fullscreen chrome fade.
    stream_pointer_at: Instant,
    /// CPU copy of the focused screen frame for the Banter Stream ImGui context.
    stream_popout_rgba: Option<(u32, u32, Vec<u8>)>,
    stream_popout_icons: HashMap<String, Shot>,
    stream_popout_icons_ready: bool,
    /// Avatars uploaded on the Banter Stream ImGui context (swapped while that surface draws).
    stream_popout_avatars: HashMap<i64, Shot>,
    stream_popout_avatar_rev: HashMap<i64, i64>,
    stream_popout_avatar_miss: HashMap<i64, i64>,
    stream_popout_shot: Option<Shot>,
    stream_popout_styled: bool,
    stream_popout_font_backup: Option<(
        Option<FontId>,
        Option<FontId>,
        Option<FontId>,
        Option<FontId>,
    )>,
    members_open: bool,
    bold_font: Option<FontId>,
    semibold_font: Option<FontId>,
    italic_font: Option<FontId>,
    bold_italic_font: Option<FontId>,
    modal: Option<Modal>,
    modal_frames: u32,
    server_menu: bool,
    collapsed: HashSet<i64>,
    channel_category: Option<i64>,
    invite_copied: bool,
    icons: HashMap<String, Shot>,
    icons_ready: bool,
    /// GPU resources for the dedicated Banter Settings ImGui context (BA-86).
    /// Swapped into the active `icons` / `avatars` / … fields while that surface draws.
    settings_icons: HashMap<String, Shot>,
    settings_icons_ready: bool,
    settings_avatars: HashMap<i64, Shot>,
    settings_avatar_rev: HashMap<i64, i64>,
    settings_avatar_miss: HashMap<i64, i64>,
    settings_crop_preview: Option<Shot>,
    settings_crop_preview_key: [i32; 3],
    settings_local_camera: Option<Shot>,
    settings_font_backup: Option<(
        Option<FontId>,
        Option<FontId>,
        Option<FontId>,
        Option<FontId>,
    )>,
    /// Last local camera RGBA mirrored for the Settings ImGui context upload.
    settings_camera_rgba: Option<(u32, u32, Vec<u8>)>,
    settings_styled: bool,
    /// Parent message for an inline Reddit-style reply composer (`None` = top-level only).
    reply_to: Option<i64>,
    /// Draft text for the inline reply box (separate from the bottom top-level composer).
    reply_draft: String,
    /// Locally collapsed comment threads (Reddit fold); not persisted.
    collapsed_comments: HashSet<i64>,
    editing: Option<i64>,
    pending_file: Option<(String, Vec<u8>)>,
    avatars: HashMap<i64, Shot>,
    avatar_rev: HashMap<i64, i64>,
    avatar_miss: HashMap<i64, i64>,
    banners: HashMap<i64, Shot>,
    banner_rev: HashMap<i64, i64>,
    banner_miss: HashMap<i64, i64>,
    /// Banner bytes waiting for a Context upload pass.
    pending_banner: Option<(i64, i64, Vec<u8>)>,
    crop_open: bool,
    crop_rgba: Vec<u8>,
    crop_size: [u32; 2],
    crop_zoom: f32,
    crop_pan: [f32; 2],
    crop_drag: Option<[f32; 2]>,
    crop_preview: Option<Shot>,
    crop_preview_key: [i32; 3],
    /// Floating Discord-parity profile (view + customization editor).
    profile: ProfileCard,
    /// Last profile payload from `/v1/users/{id}` for the open card.
    profile_view: Option<UserProfile>,
    /// Banter cosmetics catalog for the editor (fetched once per open).
    profile_cosmetics: Vec<CosmeticItem>,
    /// Banter badge catalog for the editor.
    profile_badges_catalog: Vec<BadgeItem>,
    /// Draft widgets while customizing (real user content only).
    profile_widget_drafts: Vec<ProfileWidgetInput>,
    /// Draft connections while customizing.
    profile_connection_drafts: Vec<ProfileConnectionInput>,
    /// Draft equipped badge ids while customizing.
    profile_badge_drafts: Vec<String>,
    /// Draft Activity tab rows while customizing.
    profile_activity_drafts: Vec<ProfileActivityInput>,
    /// Draft Wishlist tab rows while customizing.
    profile_wishlist_drafts: Vec<ProfileWishlistInput>,
    /// Deferred banner change (applied on Save).
    profile_draft_banner: ProfileMediaDraft,
    /// Deferred avatar change (applied on Save).
    profile_draft_avatar: ProfileMediaDraft,
    /// Which left-column slot picker is expanded.
    profile_editor_slot: Option<ProfileEditorSlot>,
    /// Frames the profile card has been open (skips the opening click for dismiss).
    profile_frames: u32,
    /// Discord-style right-click menu for messages, members, and channels.
    context_menu: Option<OpenContextMenu>,
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
    /// Local UI sounds (join/leave/mute/stream) and DM call ringtone.
    sfx: Sfx,
    /// When the one-shot ringtone started; unanswered calls time out at start + ringtone length.
    ring_deadline: Option<Instant>,
    /// When set, the websocket is down and the next connect attempt runs at this instant.
    reconnect_at: Option<Instant>,
    /// Guild VC or DM call to rejoin after a successful reconnect (server clears seats on drop).
    pending_voice_rejoin: Option<VoiceRejoinIntent>,
    shell: ShellHost,
    last_activity: Instant,
    styled: bool,
    profiler_metrics: bool,
    profiler_demo: bool,
    /// Hidden `/debugnow` window. Not listed in any menu.
    debug_open: bool,
    debug_view: Option<ServerDebug>,
    debug_error: String,
    debug_fetched: Instant,
    /// Last time we re-subscribed to live remote screen/camera (late-join / stall recovery).
    stream_resub_at: Instant,
    layers: LayerPreferences,
    delivered: HashMap<(i64, MediaKind), LayerId>,
    quality_for: Option<(i64, MediaKind)>,
    quality_draft: f32,
    mods: crate::mods::ModHost,
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
                    let _ = clear_token(&data_dir);
                }
            }
        }
        crate::plugins::ensure_plugin_samples(&data_dir);
        let mods = crate::mods::ModHost::open(&data_dir);
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
            stay_signed_in: true,
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
            voice_rosters: HashMap::new(),
            voice_channel: None,
            call: None,
            mute: MuteMemory::new(),
            push_to_talk: false,
            camera: false,
            screen: false,
            flags: VoiceFlags::default(),
            sent_flags: VoiceFlags::default(),
            sent_guild_p2p_video: None,
            flags_failed_at: None,
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
            channel_icon: None,
            channel_edit_id: None,
            nickname: String::new(),
            about_draft: String::new(),
            status_draft: String::new(),
            server_rail: ServerRail {
                folders: Vec::new(),
                servers: Vec::new(),
            },
            folder_name: String::new(),
            folder_color: String::new(),
            folder_parent: None,
            editing_folder: None,
            folder_mode: FolderModalMode::Create,
            folder_error: String::new(),
            plugin_server: None,
            plugin_pending: None,
            plugin_confirm_step: 0,
            plugin_notice: String::new(),
            local_plugins: Vec::new(),
            installed_plugins: Vec::new(),
            guild_roles: Vec::new(),
            role_name: String::new(),
            role_color: String::new(),
            role_hoist: false,
            role_admin: false,
            role_channels: false,
            role_manage: false,
            role_invite: false,
            role_plugins: false,
            role_bots: false,
            editing_role: None,
            role_error: String::new(),
            assign_user: None,
            assign_role_ids: Vec::new(),
            display_name: String::new(),
            composer: String::new(),
            search: String::new(),
            friend_tab: 0,
            home_pane: HomePane::Landing,
            home_layout: HomeLayout::default(),
            home_editing: false,
            settings: SettingsState::new(),
            settings_error: String::new(),
            server_version_label: String::new(),
            server_protocol: None,
            version_warn: String::new(),
            dev_server: DevServerPrefs::default(),
            login_options: false,
            login_options_frames: 0,
            settings_popup_open: false,
            share_open: false,
            share: ScreenShare::new(),
            share_tab: 0,
            share_cover_focus: 0.0,
            share_cover_target: 0.0,
            share_present: 0.0,
            share_chip_expanded: false,
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
            stream_popout: false,
            stream_fullscreen: false,
            focused_streamer: None,
            stream_pointer_at: Instant::now(),
            stream_popout_rgba: None,
            stream_popout_icons: HashMap::new(),
            stream_popout_icons_ready: false,
            stream_popout_avatars: HashMap::new(),
            stream_popout_avatar_rev: HashMap::new(),
            stream_popout_avatar_miss: HashMap::new(),
            stream_popout_shot: None,
            stream_popout_styled: false,
            stream_popout_font_backup: None,
            members_open: true,
            bold_font: None,
            semibold_font: None,
            italic_font: None,
            bold_italic_font: None,
            modal: None,
            modal_frames: 0,
            server_menu: false,
            collapsed: HashSet::new(),
            channel_category: None,
            invite_copied: false,
            icons: HashMap::new(),
            icons_ready: false,
            settings_icons: HashMap::new(),
            settings_icons_ready: false,
            settings_avatars: HashMap::new(),
            settings_avatar_rev: HashMap::new(),
            settings_avatar_miss: HashMap::new(),
            settings_crop_preview: None,
            settings_crop_preview_key: [0, 0, 0],
            settings_local_camera: None,
            settings_font_backup: None,
            settings_camera_rgba: None,
            settings_styled: false,
            reply_to: None,
            reply_draft: String::new(),
            collapsed_comments: HashSet::new(),
            editing: None,
            pending_file: None,
            avatars: HashMap::new(),
            avatar_rev: HashMap::new(),
            avatar_miss: HashMap::new(),
            banners: HashMap::new(),
            banner_rev: HashMap::new(),
            banner_miss: HashMap::new(),
            pending_banner: None,
            crop_open: false,
            crop_rgba: Vec::new(),
            crop_size: [0, 0],
            crop_zoom: 1.0,
            crop_pan: [0.0, 0.0],
            crop_drag: None,
            crop_preview: None,
            crop_preview_key: [0, 0, 0],
            profile: ProfileCard::closed(),
            profile_view: None,
            profile_cosmetics: Vec::new(),
            profile_badges_catalog: Vec::new(),
            profile_widget_drafts: Vec::new(),
            profile_connection_drafts: Vec::new(),
            profile_badge_drafts: Vec::new(),
            profile_activity_drafts: Vec::new(),
            profile_wishlist_drafts: Vec::new(),
            profile_draft_banner: ProfileMediaDraft::Unchanged,
            profile_draft_avatar: ProfileMediaDraft::Unchanged,
            profile_editor_slot: None,
            profile_frames: 0,
            context_menu: None,
            history_h: 0.0,
            chat_jump: false,
            copy_message: None,
            local_camera: None,
            local_screen: None,
            remote_video: HashMap::new(),
            realtime: None,
            media: None,
            sfx: Sfx::open(),
            ring_deadline: None,
            reconnect_at: None,
            pending_voice_rejoin: None,
            shell: ShellHost::start(),
            last_activity: Instant::now(),
            styled: false,
            profiler_metrics: false,
            profiler_demo: false,
            debug_open: false,
            debug_view: None,
            debug_error: String::new(),
            debug_fetched: Instant::now(),
            stream_resub_at: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            layers: LayerPreferences::new(),
            delivered: HashMap::new(),
            quality_for: None,
            quality_draft: 1.0,
            mods,
        };
        app.load_saved();
        app.sync_sfx_output();
        if app.account.is_some() {
            if app.apply_version_check(true) {
                app.display_name = app
                    .account
                    .as_ref()
                    .map(|account| account.display_name.clone())
                    .unwrap_or_default();
                app.refresh_shell();
                app.connect_live();
            }
        } else {
            // Reachable but incompatible hosts should surface before sign-in.
            let _ = app.apply_version_check(false);
        }
        app
    }

    pub(crate) fn set_bold_font(&mut self, font: Option<FontId>) {
        self.bold_font = font;
    }

    pub(crate) fn set_semibold_font(&mut self, font: Option<FontId>) {
        self.semibold_font = font;
    }

    pub(crate) fn set_italic_font(&mut self, font: Option<FontId>) {
        self.italic_font = font;
    }

    pub(crate) fn set_bold_italic_font(&mut self, font: Option<FontId>) {
        self.bold_italic_font = font;
    }

    fn bfmd_fonts(&self) -> bfmd_paint::Fonts {
        bfmd_paint::Fonts {
            body: None,
            bold: self.bold_font,
            italic: self.italic_font,
            bold_italic: self.bold_italic_font.or(self.bold_font),
        }
    }

    /// Open the sign-in Server options overlay and show why Banter is asking for a remote.
    pub(crate) fn prompt_remote_server(&mut self, message: &str) {
        self.dev_server.target = ServerTarget::Remote;
        self.login_options = true;
        self.login_options_frames = 0;
        self.settings_error = message.to_string();
        // Keep Developer prefs Local/Remote honest once the user applies a host.
        // Until then the active base may still be the local placeholder.
    }

    pub(crate) fn set_v_held(&mut self, held: bool) {
        self.v_held = held;
    }

    pub(crate) fn take_video_apply(&mut self) -> Option<VideoPrefs> {
        self.pending_video.take()
    }

    pub(crate) fn note_input(&mut self) {
        self.user_acted = true;
        self.stream_pointer_at = Instant::now();
    }

    pub(crate) fn prepare(&mut self, imgui: &mut Context) {
        self.pump_shell();
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
        self.tick_call_ringtone();
        if !self.stream.show_preview {
            if let Some(shot) = self.local_screen.take() {
                let _ = imgui.remove_texture(shot.texture);
            }
        }
        let frames = self
            .media
            .as_ref()
            .map(|media| media.take_video())
            .unwrap_or_default();
        for frame in frames {
            // Keep a CPU copy so the Settings ImGui context can upload the same preview.
            if self.settings_os_window_desired()
                && frame.sender.is_none()
                && frame.kind == MediaKind::Camera
            {
                if let Some((width, height, rgba)) = frame.rgba.as_ref() {
                    self.settings_camera_rgba = Some((*width, *height, rgba.clone()));
                }
            }
            self.maybe_keep_stream_popout_frame(&frame);
            self.install_video(imgui, frame);
        }
        self.install_avatars(imgui);
        self.refresh_crop_preview(imgui);
    }

    /// Swap App draw resources onto the Banter Settings ImGui context's GPU maps/fonts.
    pub(crate) fn begin_settings_surface(
        &mut self,
        bold: Option<FontId>,
        semibold: Option<FontId>,
        italic: Option<FontId>,
        bold_italic: Option<FontId>,
    ) {
        assert!(
            self.settings_font_backup.is_none(),
            "begin_settings_surface must not nest"
        );
        self.settings_font_backup = Some((
            self.bold_font,
            self.semibold_font,
            self.italic_font,
            self.bold_italic_font,
        ));
        self.bold_font = bold;
        self.semibold_font = semibold;
        self.italic_font = italic;
        self.bold_italic_font = bold_italic;
        std::mem::swap(&mut self.icons, &mut self.settings_icons);
        std::mem::swap(&mut self.icons_ready, &mut self.settings_icons_ready);
        std::mem::swap(&mut self.avatars, &mut self.settings_avatars);
        std::mem::swap(&mut self.avatar_rev, &mut self.settings_avatar_rev);
        std::mem::swap(&mut self.avatar_miss, &mut self.settings_avatar_miss);
        std::mem::swap(&mut self.crop_preview, &mut self.settings_crop_preview);
        std::mem::swap(
            &mut self.crop_preview_key,
            &mut self.settings_crop_preview_key,
        );
        std::mem::swap(&mut self.local_camera, &mut self.settings_local_camera);
    }

    pub(crate) fn end_settings_surface(&mut self) {
        let Some((bold, semibold, italic, bold_italic)) = self.settings_font_backup.take() else {
            panic!("end_settings_surface without begin_settings_surface");
        };
        self.bold_font = bold;
        self.semibold_font = semibold;
        self.italic_font = italic;
        self.bold_italic_font = bold_italic;
        std::mem::swap(&mut self.icons, &mut self.settings_icons);
        std::mem::swap(&mut self.icons_ready, &mut self.settings_icons_ready);
        std::mem::swap(&mut self.avatars, &mut self.settings_avatars);
        std::mem::swap(&mut self.avatar_rev, &mut self.settings_avatar_rev);
        std::mem::swap(&mut self.avatar_miss, &mut self.settings_avatar_miss);
        std::mem::swap(&mut self.crop_preview, &mut self.settings_crop_preview);
        std::mem::swap(
            &mut self.crop_preview_key,
            &mut self.settings_crop_preview_key,
        );
        std::mem::swap(&mut self.local_camera, &mut self.settings_local_camera);
    }

    /// Prepare textures on the Settings ImGui context. Call only while `begin_settings_surface` is active.
    pub(crate) fn prepare_settings(&mut self, imgui: &mut Context) {
        if !self.settings_styled {
            apply_discord_style(imgui.style_mut());
            self.settings_styled = true;
        }
        self.ensure_icons(imgui);
        self.flush_shots(imgui);
        self.install_avatars(imgui);
        self.refresh_crop_preview(imgui);
        if let Some((width, height, rgba)) = self.settings_camera_rgba.take() {
            let shot = if camera_preview_mirrored() {
                mirrored_camera_rgba(imgui, width, height, &rgba)
            } else {
                upload_rgba(imgui, width, height, &rgba)
            };
            if let Some(shot) = shot {
                replace_shot(imgui, &mut self.local_camera, shot);
            }
        }
    }

    /// Drop Settings-context textures when the Banter Settings OS window closes.
    pub(crate) fn release_settings_gpu(&mut self, imgui: &mut Context) {
        for (_, shot) in self.settings_icons.drain() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.settings_icons_ready = false;
        for (_, shot) in self.settings_avatars.drain() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.settings_avatar_rev.clear();
        self.settings_avatar_miss.clear();
        if let Some(shot) = self.settings_crop_preview.take() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.settings_crop_preview_key = [0, 0, 0];
        if let Some(shot) = self.settings_local_camera.take() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.settings_camera_rgba = None;
        self.settings_styled = false;
    }

    /// True while User Settings should be shown in the dedicated Banter Settings OS window.
    pub(crate) fn settings_os_window_desired(&self) -> bool {
        self.settings.os_window_desired()
    }

    pub(crate) fn take_settings_window_focus(&mut self) -> bool {
        self.settings.take_needs_focus()
    }

    /// OS window chrome closed the settings window — clear modal/recording state on the main app.
    pub(crate) fn notify_settings_window_closed(&mut self) {
        self.settings.window_closed();
        self.settings_popup_open = false;
        self.settings_error.clear();
    }

    /// True while a stream should be shown in the dedicated Banter Stream OS window.
    pub(crate) fn stream_popout_os_window_desired(&self) -> bool {
        self.stream_popout && self.has_live_stream()
    }

    /// OS window chrome closed the Banter Stream pop-out.
    pub(crate) fn notify_stream_popout_window_closed(&mut self) {
        self.stream_popout = false;
        self.stream_popout_rgba = None;
    }

    /// Main Banter window. User Settings lives in a separate GLFW window so this shell stays usable.
    pub(crate) fn draw_main(&mut self, ui: &Ui) {
        self.pump_shell();
        if !ui.io().want_capture_keyboard() && ui.is_key_pressed(Key::F3) {
            profiler::global().toggle();
        }
        if self.account.is_none() {
            self.draw_login(ui);
            self.draw_profiler(ui);
            return;
        }
        if self.user_acted || self.last_activity.elapsed() >= Duration::from_secs(20) {
            if let Some(realtime) = &self.realtime {
                realtime.poke_activity();
            }
            self.user_acted = false;
            self.last_activity = Instant::now();
        }
        self.mods.tick();
        self.draw_shell(ui);
        self.draw_server_dropdown(ui);
        self.draw_context_menu(ui);
        self.draw_modal(ui);
        if self.share_open || self.share_present > 0.01 {
            self.draw_share(ui);
        }
        if self.stream_menu {
            self.draw_stream_menu(ui);
        }
        self.draw_profile_card(ui);
        self.draw_profiler(ui);
        self.draw_server_debug(ui);
        if (self.stream_popout || self.stream_fullscreen) && !self.has_live_stream() {
            self.stream_popout = false;
            self.stream_fullscreen = false;
            self.stream_popout_rgba = None;
        }
        if self.stream_fullscreen {
            self.draw_stream_theater(ui);
        }
        if ui.is_key_pressed(Key::Escape) {
            if self.stream_fullscreen {
                self.stream_fullscreen = false;
            } else if self.debug_open {
                self.debug_open = false;
            } else if self.crop_open && !self.settings.is_open() {
                self.crop_open = false;
            } else if self.profile.is_open() {
                self.close_profile();
            } else if self.modal.is_some() {
                self.modal = None;
            } else if self.server_menu {
                self.server_menu = false;
            } else if self.context_menu.is_some() {
                self.context_menu = None;
            } else if self.stream_menu {
                self.stream_menu = false;
            } else if self.share_open {
                self.share_open = false;
            } else if profiler::global().is_open() {
                profiler::global().set_open(false);
            }
            // Escape on the main window does not close User Settings — that belongs to the settings window.
        }
        // Avatar crop opened from My Account draws on the settings window, not here.
        if self.crop_open && !self.settings.is_open() {
            self.draw_avatar_crop(ui);
        }
        self.finish_voice_flags(ui);
    }

    /// Dedicated Banter Settings window surface (My Account / Voice & Video / Developer).
    pub(crate) fn draw_settings_window(&mut self, ui: &Ui) {
        if !self.settings.is_open() {
            return;
        }
        self.draw_settings(ui);
        if self.crop_open {
            self.draw_avatar_crop(ui);
        }
        if ui.is_key_pressed(Key::Escape) {
            if self.crop_open {
                self.crop_open = false;
            } else {
                self.settings.escape(self.settings_popup_open);
            }
        }
    }

    /// Shell token from an enabled theme mod, or the Banter default.
    fn shell(&self, token: &str, fallback: [f32; 4]) -> [f32; 4] {
        self.mods.shell_color(token).map(hex).unwrap_or(fallback)
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
                let rail = self.shell("rail_bg", RAIL_BG);
                let sidebar = self.shell("sidebar_bg", SIDEBAR_BG);
                let stage = self.shell("stage_bg", STAGE_BG);
                let panel = self.shell("panel_bg", SIDEBAR_BG);
                let header_bg = if media_stage { CALL_BG } else { stage };
                self.paint_column(
                    ui,
                    "rail",
                    [columns.rail_x, 0.0],
                    [columns.rail_w, size[1]],
                    rail,
                );
                self.paint_column(
                    ui,
                    "sidebar",
                    [columns.sidebar_x, 0.0],
                    [columns.sidebar_w, size[1]],
                    sidebar,
                );
                let header_w = columns.stage_w + columns.members_w;
                self.paint_column(
                    ui,
                    "stage-header",
                    [columns.stage_x, 0.0],
                    [header_w, HEADER_H],
                    header_bg,
                );
                self.paint_column(
                    ui,
                    "stage",
                    [columns.stage_x, HEADER_H],
                    [columns.stage_w, body_h],
                    stage,
                );
                if columns.members_w > 0.0 {
                    self.paint_column(
                        ui,
                        "members",
                        [columns.members_x, HEADER_H],
                        [columns.members_w, body_h],
                        panel,
                    );
                }
            });
    }

    fn paint_column(
        &mut self,
        ui: &Ui,
        name: &str,
        pos: [f32; 2],
        size: [f32; 2],
        color: [f32; 4],
    ) {
        let _color = ui.push_style_color(StyleColor::ChildBg, color);
        let flags = match name {
            "rail" => WindowFlags::NO_SCROLLBAR,
            "stage-header" | "sidebar" => {
                WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE
            }
            _ => WindowFlags::empty(),
        };
        ui.set_cursor_screen_pos(pos);
        ui.child_window(name)
            .size(size)
            .flags(flags)
            .build(ui, || match name {
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
                let body_h = 28.0
                    + 44.0
                    + 4.0
                    + 44.0
                    + 16.0
                    + 18.0
                    + 36.0
                    + INPUT_H
                    + 16.0
                    + 18.0
                    + 22.0
                    + 16.0;
                let footer_h = MODAL_FOOTER_H;
                let title_h = 24.0 + FONT_LG + 8.0 + 16.0 + 16.0;
                let height = title_h + body_h + footer_h;
                let min = [
                    ((display[0] - width) * 0.5).floor(),
                    ((display[1] - height) * 0.5).max(16.0).floor(),
                ];
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
                skin::text(
                    ui,
                    self.semibold_font,
                    FONT_LG,
                    [inner_x, y],
                    TEXT,
                    "Server options",
                );
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
                if settings_radio_row(
                    ui,
                    "login-dev-server-local",
                    "Local",
                    local_selected,
                    inner_w,
                ) && !local_selected
                {
                    self.dev_server.target = ServerTarget::Local;
                    self.apply_dev_server();
                }
                ui.set_cursor_screen_pos([inner_x, y + 48.0]);
                let remote_selected = self.dev_server.target == ServerTarget::Remote;
                if settings_radio_row(
                    ui,
                    "login-dev-server-remote",
                    "Remote",
                    remote_selected,
                    inner_w,
                ) && !remote_selected
                {
                    self.dev_server.target = ServerTarget::Remote;
                    self.apply_dev_server();
                }
                y += 48.0 + 48.0 + 8.0;

                skin::text(
                    ui,
                    self.bold_font,
                    FONT_XS,
                    [inner_x, y],
                    MUTED,
                    "REMOTE HOST",
                );
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
                let apply_w = (skin::text_size(ui, self.semibold_font, FONT_SM, apply_label)[0]
                    + 32.0)
                    .max(96.0);
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

                skin::text(
                    ui,
                    self.bold_font,
                    FONT_XS,
                    [inner_x, y],
                    MUTED,
                    "ACTIVE API BASE",
                );
                y += 18.0;
                skin::text(ui, None, FONT_SM, [inner_x, y], TEXT_NORMAL, &self.base);

                if !self.settings_error.is_empty() {
                    let err_y = max[1] - footer_h + 12.0;
                    skin::text(
                        ui,
                        None,
                        FONT_SM,
                        [inner_x, err_y],
                        RED,
                        &self.settings_error,
                    );
                }

                let done_w =
                    (skin::text_size(ui, self.semibold_font, FONT_SM, "Done")[0] + 32.0).max(96.0);
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
                    let inside = mouse[0] >= min[0]
                        && mouse[0] <= max[0]
                        && mouse[1] >= min[1]
                        && mouse[1] <= max[1];
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
        ui.get_window_draw_list().add_rect_filled_multicolor(
            [0.0, 0.0],
            size,
            LOGIN_TOP,
            LOGIN_TOP,
            LOGIN_BOTTOM,
            LOGIN_BOTTOM,
        );
        let register = self.register_mode;
        let card_w = 480.0_f32.min(size[0] - 32.0).max(320.0);
        let pad = 32.0;
        let field_h = 20.0 + INPUT_H + 20.0;
        let fields = if register { 3.0 } else { 2.0 };
        let subtitle_h = if register { 0.0 } else { 28.0 };
        // Two checkboxes: Show password + Stay signed in (28 each) then gap before the button.
        let card_h = pad
            + 30.0
            + subtitle_h
            + 20.0
            + fields * field_h
            + 28.0
            + 28.0
            + 44.0
            + 12.0
            + 20.0
            + pad;
        let min = [
            ((size[0] - card_w) * 0.5).floor(),
            ((size[1] - card_h) * 0.5).max(16.0).floor(),
        ];
        let max = [min[0] + card_w, min[1] + card_h];
        skin::shadow(ui, min, max, 5.0);
        skin::rect(ui, min, max, STAGE_BG, 5.0);
        let center_x = min[0] + card_w * 0.5;
        let inner_x = min[0] + pad;
        let inner_w = card_w - pad * 2.0;
        let mut y = min[1] + pad;
        let title = if register {
            "Create an account"
        } else {
            "Welcome back!"
        };
        skin::text_centered(ui, semi, FONT_XL, center_x, y, TEXT, title);
        y += 30.0;
        if !register {
            skin::text_centered(
                ui,
                None,
                FONT_MD,
                center_x,
                y + 8.0,
                TEXT_SECONDARY,
                "We're so excited to see you again!",
            );
            y += subtitle_h;
        }
        y += 20.0;
        let error = (!self.status.is_empty()).then(|| self.status.clone());
        let mut submit = false;
        if register {
            skin::field_label(ui, bold, [inner_x, y], "Display name", false, None);
            submit |= skin::input(
                ui,
                "##display",
                &mut self.register_display,
                "",
                [inner_x, y + 20.0],
                inner_w,
                INPUT_H,
                FONT_MD,
                false,
            );
            y += field_h;
        }
        skin::field_label(ui, bold, [inner_x, y], "Username", true, error.as_deref());
        submit |= skin::input(
            ui,
            "##username",
            &mut self.username,
            "",
            [inner_x, y + 20.0],
            inner_w,
            INPUT_H,
            FONT_MD,
            false,
        );
        y += field_h;
        skin::field_label(
            ui,
            bold,
            [inner_x, y],
            "Password",
            true,
            if register { None } else { error.as_deref() },
        );
        {
            // The password flag hides characters, but it also disables copy.
            let masked = password_is_masked(self.show_password);
            let _block_copy = password_blocks_copy(self.show_password);
            let _mask = masked.then(StarMask::push);
            submit |= skin::input(
                ui,
                "##password",
                &mut self.password,
                "",
                [inner_x, y + 20.0],
                inner_w,
                INPUT_H,
                FONT_MD,
                false,
            );
        }
        y += 20.0 + INPUT_H + 8.0;
        let check = self.icon("check");
        skin::checkbox(
            ui,
            "show-password",
            "Show password",
            &mut self.show_password,
            [inner_x, y],
            check,
            None,
        );
        y += 28.0;
        skin::checkbox(
            ui,
            "stay-signed-in",
            "Stay signed in",
            &mut self.stay_signed_in,
            [inner_x, y],
            check,
            None,
        );
        y += 28.0 + 12.0;
        let label = if register { "Continue" } else { "Log In" };
        if skin::button(
            ui,
            "auth-submit",
            label,
            Tone::Brand,
            [inner_x, y],
            [inner_w, 44.0],
            true,
            semi,
        ) {
            submit = true;
        }
        y += 44.0 + 12.0;
        if register {
            if skin::link(
                ui,
                "to-login",
                "Already have an account?",
                [inner_x, y],
                None,
                FONT_SM,
                LINK,
            ) {
                self.register_mode = false;
                self.status.clear();
            }
        } else {
            let lead = "Need an account? ";
            skin::text(ui, None, FONT_SM, [inner_x, y], MUTED, lead);
            let lead_w = skin::text_size(ui, None, FONT_SM, lead)[0];
            if skin::link(
                ui,
                "to-register",
                "Register",
                [inner_x + lead_w, y],
                None,
                FONT_SM,
                LINK,
            ) {
                self.register_mode = true;
                self.status.clear();
            }
        }
        if submit {
            self.submit_auth();
        }
    }

    fn submit_auth(&mut self) {
        if !self.apply_version_check(true) {
            return;
        }
        let result = if self.register_mode {
            self.client
                .register(&self.username, &self.register_display, &self.password)
        } else {
            self.client.login(&self.username, &self.password)
        };
        match result {
            Ok(session) => {
                if let Err(err) = persist_login(&self.data_dir, &session.token, self.stay_signed_in)
                {
                    self.status = if self.stay_signed_in {
                        format!("could not remember the login: {err}")
                    } else {
                        format!("could not clear the saved login: {err}")
                    };
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
        // Top corner control: Lucide home icon → customizable Home landing (BA-81).
        // DMs stay reachable from the sidebar and as a Home section, not as this button's sole destination.
        let (home_click, _) = self.rail_icon(
            ui,
            "home",
            "",
            Some("home"),
            None,
            home,
            "Home",
            RailTone::Server,
            mentions,
        );
        if home_click {
            self.place = Place::Home;
            self.home_pane = HomePane::Landing;
            self.home_editing = false;
            self.selected = None;
            self.messages.clear();
            self.server_menu = false;
            self.shell.bump();
        }
        self.rail_separator(ui);
        let rows = rail_tree_rows(&self.server_rail);
        let folders: Vec<(String, i64)> = self
            .server_rail
            .folders
            .iter()
            .map(|folder| (folder.name.clone(), folder.id))
            .collect();
        let mut toggle_folder: Option<i64> = None;
        let mut open_server: Option<i64> = None;
        let mut folder_menu: Option<(i64, [f32; 2])> = None;
        let mut server_menu: Option<(i64, Option<i64>, [f32; 2])> = None;
        let mut rail_menu: Option<[f32; 2]> = None;
        for row in rows {
            match row {
                RailTreeRow::Folder { id, depth } => {
                    let Some(folder) = self
                        .server_rail
                        .folders
                        .iter()
                        .find(|folder| folder.id == id)
                    else {
                        continue;
                    };
                    let tip = if folder.collapsed {
                        format!("{} (collapsed)", folder.name)
                    } else {
                        folder.name.clone()
                    };
                    let color = if folder.color > 0 {
                        Some(rgb_u32_to_rgba(folder.color as u32))
                    } else {
                        None
                    };
                    let origin = ui.cursor_screen_pos();
                    let (left, right) = self.rail_icon(
                        ui,
                        &format!("folder{id}"),
                        "",
                        Some("folder"),
                        color,
                        false,
                        &tip,
                        RailTone::Folder,
                        0,
                    );
                    let _ = depth;
                    if left {
                        toggle_folder = Some(id);
                    }
                    if right {
                        folder_menu = Some((id, [origin[0] + RAIL_W, origin[1]]));
                    }
                }
                RailTreeRow::Server { id, depth } => {
                    let Some(row) = self
                        .server_rail
                        .servers
                        .iter()
                        .find(|row| row.server.id == id)
                    else {
                        continue;
                    };
                    let selected = self.place == Place::Server(id);
                    let mark = server_acronym(&row.server.name);
                    let origin = ui.cursor_screen_pos();
                    let (left, right) = self.rail_icon(
                        ui,
                        &format!("sv{id}"),
                        &mark,
                        None,
                        None,
                        selected,
                        &row.server.name,
                        RailTone::Server,
                        0,
                    );
                    let _ = depth;
                    if left {
                        open_server = Some(id);
                    }
                    if right {
                        server_menu = Some((id, row.folder_id, [origin[0] + RAIL_W, origin[1]]));
                    }
                }
            }
        }
        let add_origin = ui.cursor_screen_pos();
        let (add_left, add_right) = self.rail_icon(
            ui,
            "add-server",
            "",
            Some("plus"),
            None,
            false,
            "Add a Server",
            RailTone::Add,
            0,
        );
        if add_left {
            self.server_name.clear();
            self.invite_code.clear();
            self.open_modal(Modal::AddServer);
        }
        if add_right {
            rail_menu = Some([add_origin[0] + RAIL_W, add_origin[1]]);
        }
        if let Some(folder_id) = toggle_folder {
            self.toggle_rail_folder(folder_id);
        }
        if let Some(server_id) = open_server {
            self.server_menu = false;
            self.open_server(server_id);
        }
        if let Some((folder_id, anchor)) = folder_menu {
            self.open_context_menu(ui, folder_context_menu(folder_id, true), anchor);
        } else if let Some((server_id, folder_id, anchor)) = server_menu {
            self.open_context_menu(
                ui,
                server_rail_item_menu(server_id, folder_id, &folders),
                anchor,
            );
        } else if let Some(anchor) = rail_menu {
            self.open_context_menu(ui, server_rail_context_menu(), anchor);
        }
    }

    /// One 48 px rail button with its pill, squircle morph, tooltip, and optional mention badge.
    /// Returns `(left_click, right_click)`.
    fn rail_icon(
        &self,
        ui: &Ui,
        id: &str,
        letters: &str,
        icon: Option<&str>,
        fill_override: Option<[f32; 4]>,
        selected: bool,
        tip: &str,
        tone: RailTone,
        badge: usize,
    ) -> (bool, bool) {
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button(id, [RAIL_W, RAIL_ICON]);
        let right = ui.is_item_clicked_with_button(MouseButton::Right);
        let hovered = ui.is_item_hovered();
        let active = selected || hovered;
        let center = [origin[0] + RAIL_W * 0.5, origin[1] + RAIL_ICON * 0.5];
        let half = RAIL_ICON * 0.5;
        let min = [center[0] - half, center[1] - half];
        let max = [center[0] + half, center[1] + half];
        let (fill, ink) = match (tone, fill_override) {
            (RailTone::Folder, Some(color)) => (color, WHITE),
            (RailTone::Folder, None) if active => (BLURPLE, WHITE),
            (RailTone::Folder, None) => (STAGE_BG, TEXT_NORMAL),
            (RailTone::Server, _) if active => (BLURPLE, WHITE),
            (RailTone::Server, _) => (STAGE_BG, TEXT_NORMAL),
            (RailTone::Add, _) if active => (GREEN, WHITE),
            (RailTone::Add, _) => (STAGE_BG, GREEN),
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
            skin::image(
                ui,
                self.icon(name),
                [center[0] - 12.0, center[1] - 12.0],
                24.0,
                ink,
            );
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
            let label = if badge > 99 {
                "99+".to_string()
            } else {
                badge.to_string()
            };
            let label_w = skin::text_size(ui, self.bold_font, FONT_XS, &label)[0];
            let badge_w = (label_w + 8.0).max(16.0);
            let badge_min = [max[0] + 4.0 - badge_w, max[1] - 12.0];
            let badge_max = [max[0] + 4.0, max[1] + 4.0];
            skin::rect(
                ui,
                [badge_min[0] - 3.0, badge_min[1] - 3.0],
                [badge_max[0] + 3.0, badge_max[1] + 3.0],
                RAIL_BG,
                11.0,
            );
            skin::rect(ui, badge_min, badge_max, RED, 8.0);
            skin::text_in_row(
                ui,
                self.bold_font,
                FONT_XS,
                badge_min[0] + (badge_w - label_w) * 0.5,
                badge_min[1],
                16.0,
                WHITE,
                &label,
            );
        }
        if hovered {
            skin::tooltip(
                ui,
                self.semibold_font,
                FONT_MD,
                tip,
                min,
                max,
                TooltipSide::Right,
            );
        }
        ui.dummy([RAIL_W, RAIL_ICON_GAP]);
        (clicked, right)
    }

    fn rail_separator(&self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        ui.dummy([RAIL_W, RAIL_SEPARATOR_H + RAIL_ICON_GAP]);
        let x = origin[0] + (RAIL_W - RAIL_SEPARATOR_W) * 0.5;
        skin::rect(
            ui,
            [x, origin[1]],
            [x + RAIL_SEPARATOR_W, origin[1] + RAIL_SEPARATOR_H],
            RAIL_SEPARATOR,
            1.0,
        );
    }

    fn draw_sidebar_column(&mut self, ui: &Ui) {
        let connected = self.voice_channel.is_some() || self.call.is_some();
        let connection_h = if connected { VOICE_PANEL_H } else { 0.0 };
        let list_h =
            (ui.content_region_avail()[1] - HEADER_H - connection_h - USER_PANEL_H).max(40.0);
        let fixed = WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE;
        ui.child_window("sidebar-header")
            .size([0.0, HEADER_H])
            .flags(fixed)
            .build(ui, || self.draw_sidebar_header(ui));
        ui.child_window("sidebar-list")
            .size([0.0, list_h])
            .build(ui, || match self.place {
                Place::Home | Place::Direct(_) => self.draw_dm_list(ui),
                Place::Server(server_id) => self.draw_channels(ui, server_id),
            });
        let _panel = ui.push_style_color(StyleColor::ChildBg, PANEL_BG);
        if connected {
            ui.child_window("sidebar-voice")
                .size([0.0, connection_h])
                .flags(fixed)
                .build(ui, || self.draw_connection(ui));
        }
        ui.child_window("sidebar-user")
            .size([0.0, USER_PANEL_H])
            .flags(fixed)
            .build(ui, || self.draw_user_bar(ui));
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
                    skin::rect(
                        ui,
                        origin,
                        [origin[0] + width, origin[1] + HEADER_H - 1.0],
                        ROW_HOVER,
                        0.0,
                    );
                }
                let shown = skin::ellipsize(
                    ui,
                    self.semibold_font,
                    FONT_MD,
                    &name,
                    width - 16.0 - 18.0 - 16.0 - 8.0,
                );
                skin::text_in_row(
                    ui,
                    self.semibold_font,
                    FONT_MD,
                    origin[0] + 16.0,
                    origin[1],
                    HEADER_H,
                    TEXT,
                    &shown,
                );
                let chevron = if self.server_menu {
                    "x"
                } else {
                    "chevron-down"
                };
                skin::image(
                    ui,
                    self.icon(chevron),
                    [origin[0] + width - 16.0 - 18.0, origin[1] + 15.0],
                    18.0,
                    TEXT_NORMAL,
                );
            }
            Place::Home | Place::Direct(_) => {
                ui.dummy([width, HEADER_H]);
                skin::text_in_row(
                    ui,
                    self.semibold_font,
                    FONT_MD,
                    origin[0] + 16.0,
                    origin[1],
                    HEADER_H,
                    TEXT,
                    "Direct Messages",
                );
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
        let can_invite = self.can_permission(PERM_CREATE_INVITE);
        let can_channels = self.can_permission(PERM_MANAGE_CHANNELS);
        let can_roles = self.can_permission(PERM_MANAGE_ROLES);
        let mut items = 1usize;
        if can_invite {
            items += 1;
        }
        if can_channels {
            items += 2;
        }
        if can_roles {
            items += 1;
        }
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
                if can_invite {
                    if skin::menu_item(
                        ui,
                        "menu-invite",
                        "Invite People",
                        self.icon("user-plus"),
                        MenuTone::Brand,
                        [x, y],
                        item_w,
                        semi,
                    ) {
                        action = Some(Modal::Invite);
                    }
                    y += MENU_ITEM_H + 2.0;
                }
                if can_channels {
                    if skin::menu_item(
                        ui,
                        "menu-channel",
                        "Create Channel",
                        self.icon("circle-plus"),
                        MenuTone::Normal,
                        [x, y],
                        item_w,
                        semi,
                    ) {
                        action = Some(Modal::CreateChannel);
                    }
                    y += MENU_ITEM_H + 2.0;
                    if skin::menu_item(
                        ui,
                        "menu-category",
                        "Create Category",
                        self.icon("folder-plus"),
                        MenuTone::Normal,
                        [x, y],
                        item_w,
                        semi,
                    ) {
                        action = Some(Modal::CreateCategory);
                    }
                    y += MENU_ITEM_H + 2.0;
                }
                if can_roles {
                    if skin::menu_item(
                        ui,
                        "menu-roles",
                        "Server Roles",
                        self.icon("users"),
                        MenuTone::Normal,
                        [x, y],
                        item_w,
                        semi,
                    ) {
                        action = Some(Modal::Roles);
                    }
                    y += MENU_ITEM_H + 2.0;
                }
                skin::menu_separator(ui, [x, y], item_w);
                y += 9.0;
                if skin::menu_item(
                    ui,
                    "menu-nickname",
                    "Change Nickname",
                    self.icon("pencil"),
                    MenuTone::Normal,
                    [x, y],
                    item_w,
                    semi,
                ) {
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
                    self.channel_icon = None;
                    self.channel_edit_id = None;
                    self.channel_is_voice = false;
                    self.open_modal(Modal::CreateChannel);
                }
                Modal::CreateCategory => {
                    self.category_name.clear();
                    self.open_modal(Modal::CreateCategory);
                }
                Modal::Nickname => {
                    let me = self.account.as_ref().map(|account| account.id);
                    if let Some(member) = self
                        .members
                        .iter()
                        .find(|member| Some(member.user_id) == me)
                    {
                        self.nickname = member.nickname.clone().unwrap_or_default();
                    }
                    self.open_modal(Modal::Nickname);
                }
                Modal::Roles => self.open_roles(),
                _ => {}
            }
            return;
        }
        if ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
            let mouse = ui.mouse_pos();
            let in_menu = mouse[0] >= pos[0]
                && mouse[0] <= pos[0] + width
                && mouse[1] >= pos[1]
                && mouse[1] <= pos[1] + height;
            let in_header =
                mouse[0] >= RAIL_W && mouse[0] <= RAIL_W + SIDEBAR_W && mouse[1] <= HEADER_H;
            if !in_menu && !in_header {
                self.server_menu = false;
            }
        }
    }

    fn open_modal(&mut self, modal: Modal) {
        self.modal = Some(modal);
        self.modal_frames = 0;
    }

    fn open_context_menu(&mut self, ui: &Ui, entries: Vec<ContextMenuEntry>, anchor: [f32; 2]) {
        if entries.is_empty() {
            self.context_menu = None;
            return;
        }
        let height = context_menu_height(&entries);
        let pos = clamp_context_menu_pos(anchor, [MENU_W, height], ui.io().display_size());
        self.context_menu = Some(OpenContextMenu {
            entries,
            pos,
            frames: 0,
        });
        self.server_menu = false;
    }

    fn context_menu_icon(&self, id: &str) -> Option<ManagedTextureId> {
        let name = match id {
            "ctx-reply" => "reply",
            "ctx-edit" | "ctx-rename-folder" => "pencil",
            "ctx-delete" | "ctx-delete-folder" => "trash-2",
            "ctx-message" => "message-circle",
            "ctx-call" => "phone",
            "ctx-add-friend" | "ctx-invite" => "user-plus",
            "ctx-copy-channel-id" | "ctx-copy-msg-id" => "hash",
            "ctx-create-folder"
            | "ctx-create-subfolder"
            | "ctx-move-folder"
            | "ctx-remove-folder" => "folder",
            "ctx-recolor-folder" => "circle-plus",
            "ctx-add-plugin" => "puzzle",
            _ => return None,
        };
        self.icon(name)
    }

    fn draw_context_menu(&mut self, ui: &Ui) {
        let Some(menu) = self.context_menu.clone() else {
            return;
        };
        let height = context_menu_height(&menu.entries);
        let width = MENU_W;
        let pos = menu.pos;
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
        let mut chosen: Option<ContextMenuAction> = None;
        ui.window("##context-menu")
            .flags(flags)
            .position(pos, Condition::Always)
            .size([width, height], Condition::Always)
            .build(|| {
                let semi = self.semibold_font;
                let x = pos[0] + MENU_PAD;
                let item_w = width - MENU_PAD * 2.0;
                let mut y = pos[1] + MENU_PAD;
                for entry in &menu.entries {
                    match entry {
                        ContextMenuEntry::Separator => {
                            skin::menu_separator(ui, [x, y], item_w);
                            y += 9.0;
                        }
                        ContextMenuEntry::Item {
                            id,
                            label,
                            tone,
                            action,
                        } => {
                            let menu_tone = match tone {
                                ContextMenuTone::Normal => MenuTone::Normal,
                                ContextMenuTone::Brand => MenuTone::Brand,
                                ContextMenuTone::Danger => MenuTone::Danger,
                            };
                            if skin::menu_item(
                                ui,
                                id,
                                label,
                                self.context_menu_icon(id),
                                menu_tone,
                                [x, y],
                                item_w,
                                semi,
                            ) {
                                chosen = Some(action.clone());
                            }
                            y += MENU_ITEM_H + 2.0;
                        }
                    }
                }
            });
        if let Some(action) = chosen {
            self.context_menu = None;
            self.apply_context_action(action);
            return;
        }
        let frames = if let Some(open) = self.context_menu.as_mut() {
            open.frames = open.frames.saturating_add(1);
            open.frames
        } else {
            0
        };
        if frames > 1
            && (ui.is_mouse_clicked(MouseButton::Left) || ui.is_mouse_clicked(MouseButton::Right))
        {
            let mouse = ui.mouse_pos();
            let in_menu = mouse[0] >= pos[0]
                && mouse[0] <= pos[0] + width
                && mouse[1] >= pos[1]
                && mouse[1] <= pos[1] + height;
            if !in_menu {
                self.context_menu = None;
            }
        }
    }

    fn apply_context_action(&mut self, action: ContextMenuAction) {
        match action {
            ContextMenuAction::Reply { message_id } => {
                self.editing = None;
                self.reply_to = Some(message_id);
                self.reply_draft.clear();
            }
            ContextMenuAction::Edit { message_id } => {
                if let Some(message) = self
                    .messages
                    .iter()
                    .find(|message| message.id == message_id)
                {
                    self.reply_to = None;
                    self.reply_draft.clear();
                    self.editing = Some(message_id);
                    self.composer = message.body.clone();
                }
            }
            ContextMenuAction::Delete { message_id } => {
                if self.note(self.client.delete_message(message_id)).is_some() {
                    remove_message(&mut self.messages, message_id);
                }
            }
            ContextMenuAction::CopyText { text }
            | ContextMenuAction::CopyUsername { username: text }
            | ContextMenuAction::CopyChannelName { name: text } => {
                self.copy_message = Some(text);
            }
            ContextMenuAction::CopyMessageId { message_id } => {
                self.copy_message = Some(message_id.to_string());
            }
            ContextMenuAction::CopyChannelId { channel_id } => {
                self.copy_message = Some(channel_id.to_string());
            }
            ContextMenuAction::Mention { username } => {
                insert_mention(&mut self.composer, &username);
            }
            ContextMenuAction::MessageUser { user_id } => {
                self.open_dm_for_user(user_id);
            }
            ContextMenuAction::CallUser { user_id } => {
                if let Some(dm_id) = self.open_dm_for_user(user_id) {
                    self.start_call(dm_id);
                }
            }
            ContextMenuAction::AddFriend { username } => {
                if self
                    .note(self.client.send_friend_request(&username))
                    .is_some()
                {
                    self.refresh_shell();
                }
            }
            ContextMenuAction::InvitePeople { server_id } => {
                self.open_invite(server_id);
            }
            ContextMenuAction::AssignRoles { user_id } => {
                self.open_member_roles(user_id);
            }
            ContextMenuAction::EditChannel { channel_id } => {
                self.open_edit_channel(channel_id);
            }
            ContextMenuAction::CreateServerFolder { parent_id } => {
                self.open_folder_modal(FolderModalMode::Create, None, parent_id);
            }
            ContextMenuAction::RenameServerFolder { folder_id } => {
                self.open_folder_modal(FolderModalMode::Rename, Some(folder_id), None);
            }
            ContextMenuAction::RecolorServerFolder { folder_id } => {
                self.open_folder_modal(FolderModalMode::Recolor, Some(folder_id), None);
            }
            ContextMenuAction::DeleteServerFolder { folder_id } => {
                if self
                    .note(self.client.delete_server_folder(folder_id))
                    .is_some()
                {
                    self.refresh_shell();
                }
            }
            ContextMenuAction::MoveServerToFolder {
                server_id,
                folder_id,
            } => {
                if self
                    .note(self.client.move_server_to_folder(server_id, folder_id, 0))
                    .is_some()
                {
                    self.refresh_shell();
                }
            }
            ContextMenuAction::AddPlugin { server_id } => {
                self.open_add_plugin(server_id);
            }
        }
    }

    fn open_add_plugin(&mut self, server_id: i64) {
        self.plugin_server = Some(server_id);
        self.plugin_pending = None;
        self.plugin_confirm_step = 0;
        self.plugin_notice.clear();
        self.local_plugins = crate::plugins::local_plugins(&self.data_dir.join("plugins"));
        self.installed_plugins = self
            .client
            .list_plugins(server_id)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.id, row.name, row.version))
                    .collect()
            })
            .unwrap_or_default();
        self.open_modal(Modal::AddPlugin);
    }

    fn begin_plugin_allow(&mut self, id: &str) {
        self.plugin_pending = Some(id.to_string());
        self.plugin_confirm_step = 0;
        self.open_modal(Modal::PluginAllow);
    }

    fn allow_plugin(&mut self) {
        let Some(id) = self.plugin_pending.clone() else {
            return;
        };
        let invasive = self
            .local_plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .map(|plugin| !crate::plugins::plugin_file_lines(&plugin.permissions).is_empty())
            .unwrap_or(false);
        if self.plugin_confirm_step == 0 && invasive {
            self.plugin_confirm_step = 1;
            return;
        }
        self.install_local_plugin(&id);
        self.plugin_pending = None;
        self.plugin_confirm_step = 0;
        self.open_modal(Modal::AddPlugin);
    }

    fn install_local_plugin(&mut self, id: &str) {
        let Some(server_id) = self.plugin_server else {
            return;
        };
        let Some(plugin) = self
            .local_plugins
            .iter()
            .find(|plugin| plugin.id == id && plugin.failed.is_none())
        else {
            return;
        };
        let zip = plugin.zip.clone();
        let name = plugin.name.clone();
        let version = plugin.version.clone();
        let usage = crate::plugins::plugin_usage(&plugin.id);
        if self
            .note(self.client.install_plugin(server_id, &zip))
            .is_some()
        {
            self.installed_plugins
                .retain(|(installed_id, _, _)| installed_id != id);
            self.installed_plugins
                .push((id.to_string(), name.clone(), version));
            self.plugin_notice = format!("{name} is on this server. {usage} Remove takes it off.");
            self.status = self.plugin_notice.clone();
        }
    }

    fn remove_server_plugin(&mut self, id: &str) {
        let Some(server_id) = self.plugin_server else {
            return;
        };
        let name = self
            .installed_plugins
            .iter()
            .find(|(installed_id, _, _)| installed_id == id)
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| id.to_string());
        if self
            .note(self.client.remove_plugin(server_id, id))
            .is_some()
        {
            self.installed_plugins
                .retain(|(installed_id, _, _)| installed_id != id);
            self.plugin_notice = format!(
                "{name} was removed from this server. New messages stay as you wrote them."
            );
            self.status = self.plugin_notice.clone();
        }
    }

    fn open_folder_modal(
        &mut self,
        mode: FolderModalMode,
        folder_id: Option<i64>,
        parent_id: Option<i64>,
    ) {
        self.folder_mode = mode;
        self.folder_error.clear();
        self.editing_folder = folder_id;
        self.folder_parent = parent_id;
        self.folder_name.clear();
        self.folder_color.clear();
        if let Some(id) = folder_id {
            if let Some(folder) = self
                .server_rail
                .folders
                .iter()
                .find(|folder| folder.id == id)
            {
                self.folder_name = folder.name.clone();
                self.folder_color = if folder.color == 0 {
                    String::new()
                } else {
                    format!("{:06X}", folder.color)
                };
                self.folder_parent = folder.parent_id;
            }
        }
        self.open_modal(Modal::ServerFolder);
    }

    fn toggle_rail_folder(&mut self, folder_id: i64) {
        let Some(folder) = self
            .server_rail
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
        else {
            return;
        };
        let collapsed = !folder.collapsed;
        let patch = ServerFolderPatch {
            collapsed: Some(collapsed),
            ..ServerFolderPatch::default()
        };
        if self
            .note(self.client.update_server_folder(folder_id, patch))
            .is_some()
        {
            self.refresh_shell();
        }
    }

    fn submit_server_folder(&mut self) {
        self.folder_error.clear();
        let color = match parse_folder_color(&self.folder_color) {
            Ok(value) => value,
            Err(err) => {
                self.folder_error = err;
                return;
            }
        };
        match self.folder_mode {
            FolderModalMode::Create => {
                let name = self.folder_name.clone();
                if name.trim().is_empty() {
                    self.folder_error = "name must be 1-100 characters".into();
                    return;
                }
                if self
                    .note(
                        self.client
                            .create_server_folder(&name, self.folder_parent, color),
                    )
                    .is_some()
                {
                    self.modal = None;
                    self.refresh_shell();
                }
            }
            FolderModalMode::Rename | FolderModalMode::Recolor => {
                let Some(folder_id) = self.editing_folder else {
                    return;
                };
                let patch = match self.folder_mode {
                    FolderModalMode::Rename => ServerFolderPatch {
                        name: Some(self.folder_name.clone()),
                        color: Some(color),
                        ..ServerFolderPatch::default()
                    },
                    FolderModalMode::Recolor => ServerFolderPatch {
                        color: Some(color),
                        ..ServerFolderPatch::default()
                    },
                    FolderModalMode::Create => unreachable!(),
                };
                if self
                    .note(self.client.update_server_folder(folder_id, patch))
                    .is_some()
                {
                    self.modal = None;
                    self.refresh_shell();
                }
            }
        }
    }

    fn open_edit_channel(&mut self, channel_id: i64) {
        let Some(channel) = self
            .layout
            .iter()
            .flat_map(|category| category.channels.iter())
            .find(|channel| channel.id == channel_id)
            .cloned()
        else {
            self.status = "channel not found".into();
            return;
        };
        self.channel_edit_id = Some(channel.id);
        self.channel_name = channel.name;
        self.channel_icon = channel.icon;
        self.channel_is_voice = channel.kind == ChannelKind::Voice;
        self.channel_category = Some(channel.category_id);
        self.open_modal(Modal::EditChannel);
    }

    fn open_dm_for_user(&mut self, user_id: i64) -> Option<i64> {
        if let Some(chat) = self
            .dms
            .iter()
            .find(|chat| chat.peer_id == user_id)
            .cloned()
        {
            self.open_dm(chat.id);
            return Some(chat.id);
        }
        let chat = self.note(self.client.open_dm(user_id))?;
        self.refresh_shell();
        self.open_dm(chat.id);
        Some(chat.id)
    }

    fn is_friend(&self, user_id: i64) -> bool {
        self.friends.iter().any(|friend| friend.id == user_id)
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
        let friends_selected = self.place == Place::Home && self.home_pane == HomePane::Friends;
        let (friends_left, _) = self.dm_row(ui, "friends", None, "Friends", None, friends_selected);
        if friends_left {
            self.place = Place::Home;
            self.home_pane = HomePane::Friends;
            self.home_editing = false;
            self.selected = None;
            self.messages.clear();
            self.shell.bump();
        }
        list_label(ui, "Direct Messages", self.semibold_font);
        let dms = self.dms.clone();
        for chat in dms {
            let selected = self.place == Place::Direct(chat.id);
            let presence = self
                .presence
                .get(&chat.peer_id)
                .map(|p| p.state)
                .unwrap_or(Presence::Offline);
            let (left, right) = self.dm_row(
                ui,
                &format!("dm{}", chat.id),
                Some(chat.peer_id),
                &chat.peer_display_name,
                Some(presence),
                selected,
            );
            if left {
                self.open_dm(chat.id);
            }
            if right {
                let mouse = ui.mouse_pos();
                let mine = self.account.as_ref().map(|account| account.id);
                self.open_context_menu(
                    ui,
                    member_context_menu(
                        chat.peer_id,
                        &chat.peer_username,
                        mine,
                        self.is_friend(chat.peer_id),
                        false,
                    ),
                    mouse,
                );
            }
        }
        if !self.mentions.is_empty() {
            list_label(ui, "Mentions", self.semibold_font);
            let mentions = self.mentions.clone();
            for mention in mentions {
                let label = format!("{}: {}", mention.from_username, mention.body);
                let (left, _) =
                    self.channel_row(ui, &format!("men{}", mention.id), None, &label, false);
                if left {
                    self.open_mention(&mention);
                }
            }
        }
    }

    /// A 42 px direct-message row. `user` is None for the Friends row, which shows the people icon.
    /// Returns `(left_click, right_click)`.
    fn dm_row(
        &self,
        ui: &Ui,
        id: &str,
        user: Option<i64>,
        label: &str,
        presence: Option<Presence>,
        selected: bool,
    ) -> (bool, bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let row_h = MEMBER_ROW_H;
        let clicked = ui.invisible_button(id, [width, row_h + CHANNEL_ROW_GAP]);
        let right = ui.is_item_clicked_with_button(MouseButton::Right);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + ROW_INSET, origin[1] + 1.0];
        let max = [origin[0] + width - ROW_INSET, min[1] + row_h];
        let selected_fill = self.shell("row_selected", ROW_SELECTED);
        let hover_fill = self.shell("row_hover", ROW_HOVER);
        let sidebar_bg = self.shell("sidebar_bg", SIDEBAR_BG);
        let muted = self.shell("muted", MUTED);
        let text = self.shell("text", TEXT_NORMAL);
        let fill = if selected {
            Some(selected_fill)
        } else if hovered {
            Some(hover_fill)
        } else {
            None
        };
        if let Some(fill) = fill {
            skin::rect(ui, min, max, fill, ROW_RADIUS);
        }
        let color = if selected {
            WHITE
        } else if hovered {
            text
        } else {
            muted
        };
        let avatar_center = [min[0] + 8.0 + 16.0, min[1] + row_h * 0.5];
        match user {
            Some(user_id) => {
                let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
                paint_avatar(ui, avatar, avatar_center, 16.0, &initials(label), None);
                if let Some(presence) = presence {
                    paint_status_dot(
                        ui,
                        avatar_center,
                        16.0,
                        presence,
                        fill.unwrap_or(sidebar_bg),
                    );
                }
            }
            None => skin::image(
                ui,
                self.icon("users"),
                [avatar_center[0] - 12.0, avatar_center[1] - 12.0],
                24.0,
                color,
            ),
        }
        let text_x = min[0] + 8.0 + 32.0 + 12.0;
        let shown = skin::ellipsize(ui, None, FONT_MD, label, max[0] - 8.0 - text_x);
        skin::text_in_row(ui, None, FONT_MD, text_x, min[1], row_h, color, &shown);
        (clicked, right)
    }

    fn draw_user_bar(&mut self, ui: &Ui) {
        let username = self
            .account
            .as_ref()
            .map(|account| account.username.clone())
            .unwrap_or_default();
        let account_id = self.account.as_ref().map(|account| account.id);
        let presence = account_id
            .and_then(|id| self.presence.get(&id).map(|p| p.state))
            .unwrap_or(Presence::Online);
        let origin = ui.window_pos();
        let width = ui.window_size()[0];
        let center = [origin[0] + 8.0 + 16.0, origin[1] + USER_PANEL_H * 0.5];
        ui.set_cursor_screen_pos([center[0] - 16.0, center[1] - 16.0]);
        if ui.invisible_button("profile-avatar", [32.0, 32.0]) {
            if let Some(account_id) = account_id {
                self.open_profile(account_id, [center[0], center[1]]);
            }
        }
        let avatar = account_id.and_then(|id| self.avatars.get(&id).map(|shot| shot.texture));
        paint_avatar(
            ui,
            avatar,
            center,
            16.0,
            &initials(&self.display_name),
            None,
        );
        paint_status_dot(ui, center, 16.0, presence, PANEL_BG);
        let buttons_x = origin[0] + width - 8.0 - 32.0 * 3.0;
        let name_x = origin[0] + 8.0 + 32.0 + 8.0;
        let name_w = (buttons_x - 4.0 - name_x).max(16.0);
        let name = skin::ellipsize(ui, self.semibold_font, FONT_SM, &self.display_name, name_w);
        skin::text(
            ui,
            self.semibold_font,
            FONT_SM,
            [name_x, origin[1] + 8.0],
            TEXT,
            &name,
        );
        let custom = account_id.and_then(|id| {
            self.presence.get(&id).and_then(|info| {
                info.custom_status
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
            })
        });
        let (second, second_color) = match custom {
            Some(status) => (status, TEXT),
            None => (username, TEXT_SECONDARY),
        };
        let handle = skin::ellipsize(ui, None, FONT_XS, &second, name_w);
        skin::text(
            ui,
            None,
            FONT_XS,
            [name_x, origin[1] + 27.0],
            second_color,
            &handle,
        );
        let y = origin[1] + (USER_PANEL_H - 32.0) * 0.5;
        let muted = self.mute.muted();
        let (mic_tint, mic_icon) = if muted {
            (RED, "mic-off")
        } else {
            (TEXT_SECONDARY, "mic")
        };
        let mic_hover = if muted { RED } else { TEXT_NORMAL };
        if skin::icon_button(
            ui,
            "mute",
            self.icon(mic_icon),
            [buttons_x, y],
            32.0,
            20.0,
            mic_tint,
            mic_hover,
            Some(ICON_HOVER_BG),
        ) {
            let next = !muted;
            self.mute.set_muted(next);
            self.play_mute_toggle(next);
        }
        skin::item_tooltip(
            ui,
            self.semibold_font,
            if muted { "Unmute" } else { "Mute" },
        );
        let deafened = self.mute.deafened();
        let (deaf_tint, deaf_icon) = if deafened {
            (RED, "headphone-off")
        } else {
            (TEXT_SECONDARY, "headphones")
        };
        let deaf_hover = if deafened { RED } else { TEXT_NORMAL };
        if skin::icon_button(
            ui,
            "deafen",
            self.icon(deaf_icon),
            [buttons_x + 32.0, y],
            32.0,
            20.0,
            deaf_tint,
            deaf_hover,
            Some(ICON_HOVER_BG),
        ) {
            self.mute.set_deafened(!deafened);
        }
        skin::item_tooltip(
            ui,
            self.semibold_font,
            if deafened { "Undeafen" } else { "Deafen" },
        );
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
        skin::line_h(
            ui,
            origin[0] + 8.0,
            origin[0] + width - 8.0,
            origin[1] + VOICE_PANEL_H - 1.0,
            LIST_DIVIDER,
        );
        let me = self.account.as_ref().map(|account| account.id);
        let call = self.call.clone();
        let (title, title_color, subtitle) = if let Some(channel_id) = self.voice_channel {
            let channel = self.find_channel(channel_id);
            let name = channel
                .as_ref()
                .map(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("channel {channel_id}"));
            let server = channel
                .as_ref()
                .and_then(|channel| {
                    self.servers
                        .iter()
                        .find(|server| server.id == channel.server_id)
                })
                .map(|server| server.name.clone())
                .unwrap_or_default();
            let subtitle = if server.is_empty() {
                name
            } else {
                format!("{name} / {server}")
            };
            ("Voice Connected", GREEN, subtitle)
        } else if let Some(call) = &call {
            let peer = if Some(call.caller) == me {
                call.callee
            } else {
                call.caller
            };
            match call.phase {
                CallPhase::Ringing if Some(call.callee) == me => {
                    ("Incoming Call", GREEN, self.name_of(peer))
                }
                CallPhase::Ringing => ("Calling…", YELLOW, self.name_of(peer)),
                CallPhase::Active => ("Voice Connected", GREEN, self.name_of(peer)),
            }
        } else {
            return;
        };
        // One plain device error replaces the place name, like Discord. Settings still lists every error.
        let media_errors = self
            .media
            .as_ref()
            .map(|media| media.errors())
            .unwrap_or_default();
        let voice_errors = voice_bar_error_lines(&media_errors);
        let error = media_errors.first().cloned();
        skin::image(
            ui,
            self.icon("signal"),
            [origin[0] + 8.0, origin[1] + 12.0],
            16.0,
            title_color,
        );
        skin::text(
            ui,
            semi,
            FONT_SM,
            [origin[0] + 8.0 + 16.0 + 6.0, origin[1] + 10.0],
            title_color,
            title,
        );
        let sub_w = width - 8.0 - 8.0 - 40.0;
        let (sub_text, sub_color) = match voice_errors.first() {
            Some(short) => ((*short).to_string(), RED),
            None => (subtitle, TEXT_SECONDARY),
        };
        let sub_text = skin::ellipsize(ui, None, FONT_XS, &sub_text, sub_w);
        skin::text(
            ui,
            None,
            FONT_XS,
            [origin[0] + 8.0, origin[1] + 30.0],
            sub_color,
            &sub_text,
        );
        if let Some(error) = &error {
            ui.set_cursor_screen_pos([origin[0] + 8.0, origin[1] + 28.0]);
            ui.invisible_button("connection-error", [sub_w, 16.0]);
            let detail =
                skin::wrap_lines(ui, None, FONT_SM, device_error_text(error), 360.0).join("\n");
            skin::item_tooltip(ui, None, &detail);
        }
        let end_tip = match &call {
            Some(call) if call.phase == CallPhase::Ringing && Some(call.callee) != me => {
                "Cancel Call"
            }
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
                    if skin::button(
                        ui,
                        "accept-call",
                        "Accept",
                        Tone::Green,
                        [origin[0] + 8.0, row_y],
                        [half, 32.0],
                        true,
                        semi,
                    ) {
                        self.accept_call(call.dm_id);
                    }
                    if skin::button(
                        ui,
                        "decline-call",
                        "Decline",
                        Tone::Danger,
                        [origin[0] + 16.0 + half, row_y],
                        [half, 32.0],
                        true,
                        semi,
                    ) {
                        if self.note(self.client.decline_call(call.dm_id)).is_some() {
                            self.stop_ringing();
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
        skin::item_tooltip(
            ui,
            semi,
            if camera {
                "Turn Off Camera"
            } else {
                "Turn On Camera"
            },
        );
        x += slot_w + 8.0;
        let live = self.share.is_live();
        let share_clicked =
            self.panel_button(ui, "screen-toggle", "monitor", [x, row_y], slot_w, live);
        let share_custom = ui.is_item_clicked_with_button(dear_imgui_rs::MouseButton::Right)
            && ui.io().key_shift();
        if share_clicked || share_custom {
            self.on_share_click(share_clicked, share_custom);
        }
        skin::item_tooltip(ui, semi, share_tip(live));
        x += slot_w + 8.0;
        if self.push_to_talk {
            ui.set_cursor_screen_pos([x, row_y]);
            ui.invisible_button("talk", [slot_w, 32.0]);
            self.talk_held = ui.is_item_active();
            let fill = if self.talk_held {
                GREEN_BUTTON
            } else if ui.is_item_hovered() {
                CONTROL_HOVER
            } else {
                CONTROL_BG
            };
            skin::rect(ui, [x, row_y], [x + slot_w, row_y + 32.0], fill, 4.0);
            let label_w = skin::text_size(ui, semi, FONT_SM, "Talk")[0];
            skin::text_in_row(
                ui,
                semi,
                FONT_SM,
                x + (slot_w - label_w) * 0.5,
                row_y,
                32.0,
                TEXT_NORMAL,
                "Talk",
            );
        } else {
            self.talk_held = false;
        }
        self.video_popout = false;
    }

    fn panel_button(
        &self,
        ui: &Ui,
        id: &str,
        icon: &str,
        min: [f32; 2],
        width: f32,
        active: bool,
    ) -> bool {
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
        skin::image(
            ui,
            self.icon(icon),
            [min[0] + (width - 20.0) * 0.5, min[1] + 6.0],
            20.0,
            tint,
        );
        clicked
    }

    fn end_voice_or_call(&mut self) {
        // Explicit hang-up cancels any post-reconnect rejoin.
        self.pending_voice_rejoin = None;
        if self.voice_channel.is_some() {
            self.leave_voice();
            return;
        }
        let me = self.account.as_ref().map(|account| account.id);
        let Some(call) = self.call.clone() else {
            return;
        };
        let ended = if call.phase == CallPhase::Ringing && Some(call.callee) == me {
            self.note(self.client.decline_call(call.dm_id)).is_some()
        } else {
            self.note(self.client.end_call(call.dm_id)).is_some()
        };
        if ended || self.reconnect_at.is_some() {
            // While reconnecting the server already ended the call; clear local UI anyway.
            let was_active = call.phase == CallPhase::Active;
            self.call = None;
            self.stop_ringing();
            if was_active {
                self.sfx.play(SoundEvent::Leave);
            }
            self.end_media_place();
        }
    }

    fn start_call(&mut self, dm_id: i64) {
        if let Some(call) = self.note(self.client.start_call(dm_id)) {
            self.apply_call_snapshot(call, true);
        }
    }

    fn accept_call(&mut self, dm_id: i64) {
        if let Some(call) = self.note(self.client.accept_call(dm_id)) {
            self.apply_call_snapshot(call, true);
        }
    }

    /// Seed DM call roster from snapshot flags so Go Live / camera are visible to the peer.
    fn apply_call_snapshot(&mut self, call: CallSnapshot, open_dm_if_needed: bool) {
        let dm_id = call.dm_id;
        let was_active = self
            .call
            .as_ref()
            .is_some_and(|prior| prior.phase == CallPhase::Active && prior.dm_id == dm_id);
        let active = call.phase == CallPhase::Active;
        let ringing = call.phase == CallPhase::Ringing;
        if active {
            if !was_active {
                self.voice_channel = None;
                self.drop_self_from_rosters(None);
                self.stop_ringing();
                self.sfx.play(SoundEvent::Join);
            }
            self.voice_members = vec![
                VoiceMember {
                    user_id: call.caller,
                    flags: call.caller_flags,
                    prefer_p2p_video: false,
                },
                VoiceMember {
                    user_id: call.callee,
                    flags: call.callee_flags,
                    prefer_p2p_video: false,
                },
            ];
        } else if ringing {
            self.begin_ringing();
        } else {
            self.stop_ringing();
        }
        self.call = Some(call);
        if open_dm_if_needed && !matches!(self.place, Place::Direct(id) if id == dm_id) {
            self.open_dm(dm_id);
        }
    }

    fn play_mute_toggle(&self, now_muted: bool) {
        self.sfx.play(if now_muted {
            SoundEvent::Mute
        } else {
            SoundEvent::Unmute
        });
    }

    fn sync_sfx_output(&self) {
        self.sfx
            .configure(&self.voice.output_device, self.voice.output_volume);
    }

    fn begin_ringing(&mut self) {
        if self.ring_deadline.is_some() {
            return;
        }
        self.sfx.set_ringing(true);
        self.ring_deadline = Some(Instant::now() + ringtone_duration());
    }

    fn stop_ringing(&mut self) {
        self.sfx.set_ringing(false);
        self.ring_deadline = None;
    }

    /// Keep ringtone in sync with call phase; time out unanswered calls when the clip ends.
    fn tick_call_ringtone(&mut self) {
        let ringing = self
            .call
            .as_ref()
            .is_some_and(|call| call.phase == CallPhase::Ringing);
        if !ringing {
            if self.ring_deadline.is_some() {
                self.stop_ringing();
            }
            return;
        }
        if self.ring_deadline.is_none() {
            self.begin_ringing();
        }
        if self
            .ring_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.timeout_ringing_call();
        }
    }

    /// Ringtone finished with nobody answering — cancel/decline the ringing call.
    fn timeout_ringing_call(&mut self) {
        let me = self.account.as_ref().map(|account| account.id);
        let Some(call) = self.call.clone() else {
            self.stop_ringing();
            return;
        };
        if call.phase != CallPhase::Ringing {
            self.stop_ringing();
            return;
        }
        let ended = if Some(call.callee) == me {
            self.note(self.client.decline_call(call.dm_id)).is_some()
        } else {
            self.note(self.client.end_call(call.dm_id)).is_some()
        };
        self.stop_ringing();
        if ended || self.reconnect_at.is_some() {
            self.call = None;
            self.end_media_place();
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
            Place::Home if self.home_pane == HomePane::Friends => {
                skin::image(ui, self.icon("users"), [x, mid - 12.0], 24.0, CHANNEL_ICON);
                x += 24.0 + 8.0;
                skin::text_in_row(ui, semi, FONT_MD, x, origin[1], HEADER_H, TEXT, "Friends");
                x += skin::text_size(ui, semi, FONT_MD, "Friends")[0] + 16.0;
                skin::rect(ui, [x, mid - 12.0], [x + 1.0, mid + 12.0], DIVIDER, 0.0);
                self.draw_friend_tabs(ui, x + 1.0 + 16.0, mid);
            }
            Place::Home => {
                skin::image(ui, self.icon("home"), [x, mid - 12.0], 24.0, CHANNEL_ICON);
                x += 24.0 + 8.0;
                skin::text_in_row(ui, semi, FONT_MD, x, origin[1], HEADER_H, TEXT, "Home");
                let edit_label = if self.home_editing {
                    "Done"
                } else {
                    "Customize"
                };
                let edit_w = skin::text_size(ui, semi, FONT_SM, edit_label)[0] + 24.0;
                let edit_x = origin[0] + width - 16.0 - edit_w;
                if skin::button(
                    ui,
                    "home-customize",
                    edit_label,
                    Tone::Secondary,
                    [edit_x, mid - 14.0],
                    [edit_w, 28.0],
                    true,
                    semi,
                ) {
                    self.home_editing = !self.home_editing;
                }
            }
            Place::Direct(dm_id) => {
                if let Some(chat) = self.dms.iter().find(|chat| chat.id == dm_id).cloned() {
                    let center = [x + 12.0, mid];
                    let avatar = self.avatars.get(&chat.peer_id).map(|shot| shot.texture);
                    paint_avatar(
                        ui,
                        avatar,
                        center,
                        12.0,
                        &initials(&chat.peer_display_name),
                        None,
                    );
                    let presence = self
                        .presence
                        .get(&chat.peer_id)
                        .map(|p| p.state)
                        .unwrap_or(Presence::Offline);
                    paint_status_dot(ui, center, 12.0, presence, STAGE_BG);
                    x += 24.0 + 8.0;
                    skin::text_in_row(
                        ui,
                        semi,
                        FONT_MD,
                        x,
                        origin[1],
                        HEADER_H,
                        TEXT,
                        &chat.peer_display_name,
                    );
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
            if skin::icon_button(
                ui,
                "members-toggle",
                self.icon("users"),
                [right, mid - 12.0],
                HEADER_ICON,
                HEADER_ICON,
                tint,
                TEXT_NORMAL,
                None,
            ) {
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
                if skin::icon_button(
                    ui,
                    "start-call",
                    self.icon("phone"),
                    [right, mid - 12.0],
                    HEADER_ICON,
                    HEADER_ICON,
                    TEXT_SECONDARY,
                    TEXT_NORMAL,
                    None,
                ) {
                    self.start_call(dm_id);
                }
                skin::item_tooltip(ui, semi, "Start Voice Call");
            }
        }
        self.paint_mod_header(ui, origin, width);
        header_rule(ui);
    }

    fn paint_mod_header(&mut self, ui: &Ui, origin: [f32; 2], width: f32) {
        let widgets = self.mods.header_widgets();
        if widgets.is_empty() {
            return;
        }
        let mut x = origin[0] + width - 16.0;
        for widget in widgets.iter().rev() {
            let text = widget.label.clone();
            let id = widget.id.clone();
            let size = skin::text_size(ui, self.semibold_font, FONT_SM, &text);
            x -= size[0] + 16.0;
            ui.set_cursor_screen_pos([x, origin[1] + (HEADER_H - 24.0) * 0.5]);
            if widget.kind == "button" {
                if ui.button(format!("{text}##{id}")) {
                    self.mods.click(&id);
                }
            } else {
                skin::text_in_row(
                    ui,
                    self.semibold_font,
                    FONT_SM,
                    x,
                    origin[1],
                    HEADER_H,
                    TEXT,
                    &text,
                );
            }
        }
    }

    fn draw_header_search(&mut self, ui: &Ui, min: [f32; 2]) {
        let submit = skin::input(
            ui,
            "##search",
            &mut self.search,
            "Search",
            min,
            SEARCH_W,
            SEARCH_H,
            FONT_SM,
            false,
        );
        skin::image(
            ui,
            self.icon("search"),
            [min[0] + SEARCH_W - 20.0, min[1] + 4.0],
            16.0,
            TEXT_SECONDARY,
        );
        if submit {
            self.navigate_shell(FetchKind::messages());
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
                if selected {
                    (None, GREEN_TEXT)
                } else {
                    (Some(GREEN_BUTTON), WHITE)
                }
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
                skin::text_in_row(
                    ui,
                    self.bold_font,
                    FONT_XS,
                    bx + (16.0 - count_w) * 0.5,
                    mid - 8.0,
                    16.0,
                    WHITE,
                    &count,
                );
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
            match self.home_pane {
                HomePane::Landing => self.draw_home_landing(ui),
                HomePane::Friends => self.draw_people(ui),
            }
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
        skin::rect(
            ui,
            origin,
            [origin[0] + width, origin[1] + height],
            NOTICE_BG,
            0.0,
        );
        let message = skin::ellipsize(ui, self.semibold_font, FONT_SM, &self.status, width - 80.0);
        let message_w = skin::text_size(ui, self.semibold_font, FONT_SM, &message)[0];
        skin::text_in_row(
            ui,
            self.semibold_font,
            FONT_SM,
            origin[0] + (width - message_w) * 0.5,
            origin[1],
            height,
            WHITE,
            &message,
        );
        if skin::icon_button(
            ui,
            "dismiss-notice",
            self.icon("x"),
            [origin[0] + width - 8.0 - 24.0, origin[1] + 6.0],
            24.0,
            16.0,
            WHITE,
            WHITE,
            None,
        ) {
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
            Place::Home if self.home_pane == HomePane::Friends => "Friends".to_string(),
            Place::Home => "Home".to_string(),
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

    fn stage_icon(&self) -> Option<&str> {
        if let Some(channel) = &self.selected {
            if matches!(self.place, Place::Server(_)) {
                return Some(channel_display_icon(&channel.kind, channel.icon.as_deref()));
            }
        }
        match self.place {
            Place::Home if self.home_pane == HomePane::Friends => Some("users"),
            Place::Home => Some("home"),
            Place::Direct(_) => Some("user"),
            Place::Server(_) => None,
        }
    }

    /// Customizable Home landing: Friends, DMs, Servers, and Recent sections (BA-81).
    fn draw_home_landing(&mut self, ui: &Ui) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let height = ui.content_region_avail()[1];
        let left = origin[0] + 30.0;
        let right = origin[0] + width - 20.0;
        ui.dummy([width, 16.0]);
        skin::text(
            ui,
            None,
            FONT_SM,
            [left, origin[1] + 8.0],
            TEXT_SECONDARY,
            "Your Home page. Open Friends from the sidebar, or customize which sections show here.",
        );
        ui.set_cursor_screen_pos([origin[0], origin[1] + 40.0]);

        if self.home_editing {
            self.draw_home_layout_editor(ui, left, right);
        }

        let sections = if self.home_editing {
            self.home_layout.sections.clone()
        } else {
            visible_home_sections(&self.home_layout)
        };
        let mut open_dm: Option<i64> = None;
        let mut open_server: Option<i64> = None;
        let mut open_friends = false;
        for section in &sections {
            if self.home_editing && !home_section_is_visible(&self.home_layout, *section) {
                continue;
            }
            match *section {
                HomeSectionId::Friends => {
                    if self.draw_home_friends_section(ui, left, right) {
                        open_friends = true;
                    }
                }
                HomeSectionId::DirectMessages => {
                    if let Some(id) = self.draw_home_dms_section(ui, left, right) {
                        open_dm = Some(id);
                    }
                }
                HomeSectionId::Servers => {
                    if let Some(id) = self.draw_home_servers_section(ui, left, right) {
                        open_server = Some(id);
                    }
                }
                HomeSectionId::Recent => match self.draw_home_recent_section(ui, left, right) {
                    Some(HomeRecentEntry {
                        kind: HomeRecentKind::DirectMessage,
                        id,
                    }) => open_dm = Some(id),
                    Some(HomeRecentEntry {
                        kind: HomeRecentKind::Server,
                        id,
                    }) => open_server = Some(id),
                    None => {}
                },
            }
        }

        if sections.is_empty() {
            ui.dummy([width, (height - 80.0).max(40.0)]);
            skin::text_centered(
                ui,
                None,
                FONT_MD,
                origin[0] + width * 0.5,
                origin[1] + height * 0.42,
                MUTED,
                "No Home sections are visible. Click Customize to show some.",
            );
        }

        if open_friends {
            self.home_pane = HomePane::Friends;
            self.friend_tab = 0;
            self.home_editing = false;
        }
        if let Some(dm_id) = open_dm {
            self.open_dm(dm_id);
        }
        if let Some(server_id) = open_server {
            self.open_server(server_id);
        }
    }

    fn draw_home_layout_editor(&mut self, ui: &Ui, left: f32, right: f32) {
        let semi = self.semibold_font;
        let origin = ui.cursor_screen_pos();
        let width = (right - left).max(80.0);
        skin::text(
            ui,
            semi,
            FONT_SM,
            [left, origin[1]],
            TEXT,
            "Arrange sections",
        );
        skin::text(
            ui,
            None,
            FONT_XS,
            [left, origin[1] + 20.0],
            TEXT_SECONDARY,
            "Show or hide each block and move it up or down. Changes save on this PC.",
        );
        ui.set_cursor_screen_pos([origin[0], origin[1] + 44.0]);
        let order = self.home_layout.sections.clone();
        let mut dirty = false;
        for (index, id) in order.iter().copied().enumerate() {
            let row_y = ui.cursor_screen_pos()[1];
            let label = home_section_label(id);
            let mut visible = home_section_is_visible(&self.home_layout, id);
            skin::checkbox(
                ui,
                &format!("home-vis-{index}"),
                label,
                &mut visible,
                [left, row_y],
                self.icon("check"),
                semi,
            );
            if visible != home_section_is_visible(&self.home_layout, id) {
                set_home_section_visible(&mut self.home_layout, id, visible);
                dirty = true;
            }
            let btn_y = row_y + 2.0;
            if skin::icon_button(
                ui,
                &format!("home-up-{index}"),
                self.icon("chevron-up"),
                [right - 64.0, btn_y],
                28.0,
                20.0,
                TEXT_SECONDARY,
                TEXT_NORMAL,
                None,
            ) && move_home_section(&mut self.home_layout, id, -1)
            {
                dirty = true;
            }
            if skin::icon_button(
                ui,
                &format!("home-down-{index}"),
                self.icon("chevron-down"),
                [right - 32.0, btn_y],
                28.0,
                20.0,
                TEXT_SECONDARY,
                TEXT_NORMAL,
                None,
            ) && move_home_section(&mut self.home_layout, id, 1)
            {
                dirty = true;
            }
            ui.set_cursor_screen_pos([origin[0], row_y + 36.0]);
            let _ = width;
        }
        if dirty {
            self.save_home_layout();
        }
        ui.dummy([0.0, 12.0]);
        skin::line_h(ui, left, right, ui.cursor_screen_pos()[1], LIST_DIVIDER);
        ui.dummy([0.0, 16.0]);
    }

    fn draw_home_section_header(&self, ui: &Ui, left: f32, title: &str, subtitle: &str) {
        let origin = ui.cursor_screen_pos();
        let semi = self.semibold_font;
        skin::text(
            ui,
            semi,
            FONT_XS,
            [left, origin[1]],
            TEXT_SECONDARY,
            &title.to_uppercase(),
        );
        if !subtitle.is_empty() {
            skin::text(
                ui,
                None,
                FONT_SM,
                [left, origin[1] + 18.0],
                TEXT_SECONDARY,
                subtitle,
            );
            ui.set_cursor_screen_pos([origin[0], origin[1] + 42.0]);
        } else {
            ui.set_cursor_screen_pos([origin[0], origin[1] + 22.0]);
        }
    }

    /// Returns true when the user asks to open the full Friends pane.
    fn draw_home_friends_section(&mut self, ui: &Ui, left: f32, right: f32) -> bool {
        let online = self
            .friends
            .iter()
            .filter(|friend| {
                self.presence
                    .get(&friend.id)
                    .map(|info| info.state != Presence::Offline)
                    .unwrap_or(false)
            })
            .count();
        let pending = self.incoming.len();
        let subtitle = if pending > 0 {
            format!("{online} online · {pending} pending")
        } else {
            format!("{online} online · {} friends", self.friends.len())
        };
        self.draw_home_section_header(ui, left, "Friends", &subtitle);
        let mut open = false;
        let semi = self.semibold_font;
        let y = ui.cursor_screen_pos()[1];
        if skin::button(
            ui,
            "home-open-friends",
            "Open Friends",
            Tone::Brand,
            [left, y],
            [140.0, 32.0],
            true,
            semi,
        ) {
            open = true;
        }
        let preview: Vec<_> = self
            .friends
            .iter()
            .filter(|friend| {
                self.presence
                    .get(&friend.id)
                    .map(|info| info.state != Presence::Offline)
                    .unwrap_or(false)
            })
            .take(5)
            .cloned()
            .collect();
        let mut x = left + 156.0;
        for friend in preview {
            let center = [x + 16.0, y + 16.0];
            let avatar = self.avatars.get(&friend.id).map(|shot| shot.texture);
            paint_avatar(
                ui,
                avatar,
                center,
                16.0,
                &initials(&friend.display_name),
                None,
            );
            let presence = self
                .presence
                .get(&friend.id)
                .map(|p| p.state)
                .unwrap_or(Presence::Offline);
            paint_status_dot(ui, center, 16.0, presence, STAGE_BG);
            x += 40.0;
            if x + 32.0 > right {
                break;
            }
        }
        ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], y + 48.0]);
        ui.dummy([0.0, 8.0]);
        open
    }

    fn draw_home_dms_section(&mut self, ui: &Ui, left: f32, right: f32) -> Option<i64> {
        let count = self.dms.len();
        let subtitle = if count == 0 {
            "No direct messages yet.".into()
        } else {
            format!("{count} conversations")
        };
        self.draw_home_section_header(ui, left, "Direct Messages", &subtitle);
        let mut chosen = None;
        let dms: Vec<_> = self.dms.iter().take(8).cloned().collect();
        for chat in dms {
            let row_min = [left - 10.0, ui.cursor_screen_pos()[1]];
            let row_max = [right + 10.0, row_min[1] + 48.0];
            let hovered = ui.is_window_hovered() && ui.is_mouse_hovering_rect(row_min, row_max);
            if hovered {
                skin::rect(ui, row_min, row_max, ROW_HOVER, 8.0);
            }
            let center = [left + 16.0, row_min[1] + 24.0];
            let avatar = self.avatars.get(&chat.peer_id).map(|shot| shot.texture);
            paint_avatar(
                ui,
                avatar,
                center,
                16.0,
                &initials(&chat.peer_display_name),
                None,
            );
            let presence = self
                .presence
                .get(&chat.peer_id)
                .map(|p| p.state)
                .unwrap_or(Presence::Offline);
            paint_status_dot(
                ui,
                center,
                16.0,
                presence,
                if hovered { ROW_HOVER } else { STAGE_BG },
            );
            skin::text(
                ui,
                self.semibold_font,
                FONT_MD,
                [left + 44.0, row_min[1] + 14.0],
                TEXT,
                &chat.peer_display_name,
            );
            ui.set_cursor_screen_pos(row_min);
            if ui.invisible_button(
                format!("home-dm-{}", chat.id),
                [row_max[0] - row_min[0], 48.0],
            ) {
                chosen = Some(chat.id);
            }
        }
        ui.dummy([0.0, 12.0]);
        chosen
    }

    fn draw_home_servers_section(&mut self, ui: &Ui, left: f32, right: f32) -> Option<i64> {
        let count = self.servers.len();
        let subtitle = if count == 0 {
            "Join or create a server to pin shortcuts here.".into()
        } else {
            format!("{count} servers")
        };
        self.draw_home_section_header(ui, left, "Servers", &subtitle);
        let mut chosen = None;
        let mut x = left;
        let y = ui.cursor_screen_pos()[1];
        let servers = self.servers.clone();
        for server in servers {
            if x + 48.0 > right {
                break;
            }
            ui.set_cursor_screen_pos([x, y]);
            if ui.invisible_button(format!("home-server-{}", server.id), [44.0, 44.0]) {
                chosen = Some(server.id);
            }
            let hovered = ui.is_item_hovered();
            let fill = if hovered { BLURPLE } else { STAGE_BG };
            skin::rect(ui, [x, y], [x + 44.0, y + 44.0], fill, 12.0);
            let letters = initials(&server.name);
            let measured = skin::text_size(ui, self.semibold_font, FONT_SM, &letters);
            skin::text(
                ui,
                self.semibold_font,
                FONT_SM,
                [
                    x + (44.0 - measured[0]) * 0.5,
                    y + (44.0 - measured[1]) * 0.5,
                ],
                WHITE,
                &letters,
            );
            if hovered {
                skin::item_tooltip(ui, self.semibold_font, &server.name);
            }
            x += 52.0;
        }
        ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], y + 56.0]);
        ui.dummy([0.0, 8.0]);
        chosen
    }

    fn draw_home_recent_section(
        &mut self,
        ui: &Ui,
        left: f32,
        right: f32,
    ) -> Option<HomeRecentEntry> {
        self.draw_home_section_header(ui, left, "Recent", "");
        let mut chosen = None;
        let recent = self.home_layout.recent.clone();
        if recent.is_empty() {
            skin::text(
                ui,
                None,
                FONT_SM,
                [left, ui.cursor_screen_pos()[1]],
                MUTED,
                "Servers and DMs you open show up here.",
            );
            ui.dummy([0.0, 28.0]);
            return None;
        }
        for entry in recent {
            let label = match entry.kind {
                HomeRecentKind::DirectMessage => self
                    .dms
                    .iter()
                    .find(|chat| chat.id == entry.id)
                    .map(|chat| format!("DM · {}", chat.peer_display_name)),
                HomeRecentKind::Server => self
                    .servers
                    .iter()
                    .find(|server| server.id == entry.id)
                    .map(|server| format!("Server · {}", server.name)),
            };
            let Some(label) = label else {
                continue;
            };
            let row_min = [left - 10.0, ui.cursor_screen_pos()[1]];
            let row_max = [right + 10.0, row_min[1] + 40.0];
            let hovered = ui.is_window_hovered() && ui.is_mouse_hovering_rect(row_min, row_max);
            if hovered {
                skin::rect(ui, row_min, row_max, ROW_HOVER, 8.0);
            }
            let icon = match entry.kind {
                HomeRecentKind::DirectMessage => "message-circle",
                HomeRecentKind::Server => "hash",
            };
            skin::image(
                ui,
                self.icon(icon),
                [left, row_min[1] + 8.0],
                20.0,
                TEXT_SECONDARY,
            );
            skin::text(
                ui,
                self.semibold_font,
                FONT_MD,
                [left + 28.0, row_min[1] + 10.0],
                TEXT,
                &label,
            );
            ui.set_cursor_screen_pos(row_min);
            if ui.invisible_button(
                format!("home-recent-{:?}-{}", entry.kind, entry.id),
                [row_max[0] - row_min[0], 40.0],
            ) {
                chosen = Some(entry);
            }
        }
        ui.dummy([0.0, 12.0]);
        chosen
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
            skin::text(
                ui,
                semi,
                FONT_LG,
                [left, origin[1] + 20.0],
                TEXT,
                "Add Friend",
            );
            skin::text(
                ui,
                None,
                FONT_SM,
                [left, origin[1] + 52.0],
                TEXT_SECONDARY,
                "You can add friends with their Banter username.",
            );
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
                    self.open_dm_after_adding_friend(&name);
                }
            }
            skin::line_h(
                ui,
                origin[0],
                origin[0] + width,
                origin[1] + 156.0,
                LIST_DIVIDER,
            );
            ui.set_cursor_screen_pos([origin[0], origin[1] + 160.0]);
            ui.dummy([width, 1.0]);
            return;
        }
        let mut rows: Vec<(i64, String, String, Presence, String, bool)> = Vec::new();
        let title;
        let empty;
        if self.friend_tab == 2 {
            for request in &self.incoming {
                rows.push((
                    request.id,
                    request.from_username.clone(),
                    request.from_username.clone(),
                    Presence::Offline,
                    "Incoming Friend Request".into(),
                    true,
                ));
            }
            for request in &self.outgoing {
                rows.push((
                    request.id,
                    request.to_username.clone(),
                    request.to_username.clone(),
                    Presence::Offline,
                    "Outgoing Friend Request".into(),
                    false,
                ));
            }
            title = "Pending";
            empty = "There are no pending friend requests.";
        } else {
            let online_only = self.friend_tab == 0;
            for friend in &self.friends {
                let presence = self
                    .presence
                    .get(&friend.id)
                    .map(|p| p.state)
                    .unwrap_or(Presence::Offline);
                if online_only && presence == Presence::Offline {
                    continue;
                }
                let subtitle = self
                    .presence
                    .get(&friend.id)
                    .and_then(|info| info.custom_status.clone())
                    .filter(|status| !status.trim().is_empty())
                    .unwrap_or_else(|| presence_label(presence).into());
                rows.push((
                    friend.id,
                    friend.display_name.clone(),
                    friend.username.clone(),
                    presence,
                    subtitle,
                    false,
                ));
            }
            title = if online_only { "Online" } else { "All Friends" };
            empty = if online_only {
                "No one's around right now."
            } else {
                "You have not added anyone yet."
            };
        }
        if rows.is_empty() {
            ui.dummy([width, height.max(1.0)]);
            let mid_y = origin[1] + height * 0.42;
            skin::text_centered(
                ui,
                None,
                FONT_MD,
                origin[0] + width * 0.5,
                mid_y,
                MUTED,
                empty,
            );
            return;
        }
        let label = format!("{} — {}", title.to_uppercase(), rows.len());
        skin::text(
            ui,
            semi,
            FONT_XS,
            [left, origin[1] + 24.0],
            TEXT_SECONDARY,
            &label,
        );
        ui.dummy([width, 48.0]);
        let pending_tab = self.friend_tab == 2;
        let mut open_chat = None;
        let mut open_profile = None;
        let mut accept = None;
        let mut decline = None;
        let mut context_user: Option<(i64, String)> = None;
        for (index, (id, name, username, presence, subtitle, incoming)) in rows.iter().enumerate() {
            let row_min = [left - 10.0, ui.cursor_screen_pos()[1]];
            let row_max = [right + 10.0, row_min[1] + 62.0];
            let hovered = ui.is_window_hovered() && ui.is_mouse_hovering_rect(row_min, row_max);
            if hovered && !pending_tab && ui.is_mouse_clicked(MouseButton::Right) {
                context_user = Some((*id, username.clone()));
            }
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
                paint_status_dot(
                    ui,
                    center,
                    16.0,
                    *presence,
                    if hovered { ROW_HOVER } else { STAGE_BG },
                );
            }
            let text_x = left + 32.0 + 12.0;
            skin::text(ui, semi, FONT_MD, [text_x, row_min[1] + 12.0], TEXT, name);
            if hovered && !pending_tab {
                let name_w = skin::text_size(ui, semi, FONT_MD, name)[0];
                skin::text(
                    ui,
                    None,
                    FONT_SM,
                    [text_x + name_w + 6.0, row_min[1] + 14.0],
                    TEXT_SECONDARY,
                    username,
                );
            }
            skin::text(
                ui,
                None,
                FONT_SM,
                [text_x, row_min[1] + 33.0],
                TEXT_SECONDARY,
                subtitle,
            );
            let button_y = row_min[1] + 13.0;
            let mut bx = right - 36.0;
            if pending_tab {
                if *incoming {
                    if self.round_action(
                        ui,
                        &format!("decline-{id}"),
                        "x",
                        [bx, button_y],
                        hovered,
                        RED,
                    ) {
                        decline = Some(*id);
                    }
                    skin::item_tooltip(ui, semi, "Ignore");
                    bx -= 36.0 + 10.0;
                    if self.round_action(
                        ui,
                        &format!("accept-{id}"),
                        "check",
                        [bx, button_y],
                        hovered,
                        GREEN,
                    ) {
                        accept = Some(*id);
                    }
                    skin::item_tooltip(ui, semi, "Accept");
                }
            } else {
                if self.round_action(
                    ui,
                    &format!("message-{id}"),
                    "message-circle",
                    [bx, button_y],
                    hovered,
                    TEXT_NORMAL,
                ) {
                    open_chat = Some(*id);
                }
                skin::item_tooltip(ui, semi, "Message");
            }
            let action_zone = if pending_tab { 90.0 } else { 50.0 };
            ui.set_cursor_screen_pos(row_min);
            if ui.invisible_button(
                format!("friend-row-{id}-{index}"),
                [(row_max[0] - row_min[0] - action_zone).max(8.0), 62.0],
            ) && !pending_tab
            {
                open_profile = Some((*id, [row_max[0], row_min[1]]));
            }
        }
        if let Some((user_id, anchor)) = open_profile {
            self.open_profile(user_id, anchor);
        }
        if let Some(friend_id) = open_chat {
            if let Some(chat) = self.note(self.client.open_dm(friend_id)) {
                self.refresh_shell();
                self.open_dm(chat.id);
            }
        }
        if let Some(id) = accept {
            let peer_username = self
                .incoming
                .iter()
                .find(|request| request.id == id)
                .map(|request| request.from_username.clone());
            if self.note(self.client.accept_friend_request(id)).is_some() {
                self.refresh_shell();
                if let Some(username) = peer_username {
                    self.open_dm_after_adding_friend(&username);
                }
            }
        }
        if let Some(id) = decline {
            if self.note(self.client.decline_friend_request(id)).is_some() {
                self.refresh_shell();
            }
        }
        if let Some((user_id, username)) = context_user {
            let mouse = ui.mouse_pos();
            let mine = self.account.as_ref().map(|account| account.id);
            self.open_context_menu(
                ui,
                member_context_menu(user_id, &username, mine, true, false),
                mouse,
            );
        }
    }

    /// Round 36 px action button used in the friends list.
    fn round_action(
        &self,
        ui: &Ui,
        id: &str,
        icon: &str,
        min: [f32; 2],
        row_hovered: bool,
        hover_tint: [f32; 4],
    ) -> bool {
        ui.set_cursor_screen_pos(min);
        let clicked = ui.invisible_button(id, [36.0, 36.0]);
        let hovered = ui.is_item_hovered();
        let fill = if row_hovered { RAIL_BG } else { SIDEBAR_BG };
        skin::rect(ui, min, [min[0] + 36.0, min[1] + 36.0], fill, 18.0);
        skin::image(
            ui,
            self.icon(icon),
            [min[0] + 8.0, min[1] + 8.0],
            20.0,
            if hovered { hover_tint } else { TEXT_SECONDARY },
        );
        clicked
    }

    fn draw_channels(&mut self, ui: &Ui, server_id: i64) {
        let layout = self.layout.clone();
        let voice_rosters = self.voice_rosters.clone();
        let can_manage = self.can_permission(PERM_MANAGE_CHANNELS);
        let selected_id = self.selected.as_ref().map(|channel| channel.id);
        ui.dummy([0.0, 4.0]);
        for category in layout {
            let category_id = category.category.id;
            let collapsed = self.collapsed.contains(&category_id);
            let (toggle, create) = self.category_row(
                ui,
                category_id,
                &category.category.name,
                collapsed,
                can_manage,
            );
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
                self.channel_icon = None;
                self.channel_edit_id = None;
                self.channel_is_voice = false;
                self.open_modal(Modal::CreateChannel);
            }
            for channel in category.channels {
                let joined =
                    channel.kind == ChannelKind::Voice && self.voice_channel == Some(channel.id);
                let selected = selected_id == Some(channel.id);
                if collapsed && !selected && !joined {
                    continue;
                }
                let icon_name = channel_display_icon(&channel.kind, channel.icon.as_deref());
                let (left, right) = self.channel_row(
                    ui,
                    &format!("ch{}", channel.id),
                    Some(icon_name),
                    &channel.name,
                    selected,
                );
                if left {
                    self.selected = Some(channel.clone());
                    self.search.clear();
                    self.reply_to = None;
                    self.reply_draft.clear();
                    self.collapsed_comments.clear();
                    self.editing = None;
                    self.messages.clear();
                    if channel.kind == ChannelKind::Text {
                        self.navigate_shell(FetchKind::messages());
                    } else if channel.kind == ChannelKind::Voice
                        && self.voice_channel != Some(channel.id)
                    {
                        self.shell.bump();
                        self.join_voice_channel(channel.id);
                    } else {
                        self.shell.bump();
                    }
                }
                if right {
                    let mouse = ui.mouse_pos();
                    self.open_context_menu(
                        ui,
                        channel_context_menu(channel.id, &channel.name, server_id, can_manage),
                        mouse,
                    );
                }
                if channel.kind == ChannelKind::Voice {
                    if let Some(members) = voice_rosters.get(&channel.id) {
                        for member in members {
                            self.draw_voice_member(ui, channel.id, member);
                        }
                    }
                }
            }
        }
        ui.dummy([0.0, 8.0]);
    }

    /// Category header: chevron, 12 px uppercase name, and a Create Channel plus for the owner.
    fn category_row(
        &self,
        ui: &Ui,
        category_id: i64,
        name: &str,
        collapsed: bool,
        can_create: bool,
    ) -> (bool, bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let text_top = origin[1] + 16.0;
        let text_h = CATEGORY_H - 16.0;
        let mut create = false;
        let plus_min = [
            origin[0] + width - 8.0 - 18.0,
            text_top + (text_h - 18.0) * 0.5,
        ];
        if can_create {
            create = skin::icon_button(
                ui,
                &format!("cat-add-{category_id}"),
                self.icon("plus"),
                plus_min,
                18.0,
                18.0,
                MUTED,
                TEXT_NORMAL,
                None,
            );
            skin::item_tooltip(ui, self.semibold_font, "Create Channel");
        }
        ui.set_cursor_screen_pos(origin);
        let toggle_w = if can_create {
            width - 8.0 - 18.0 - 4.0
        } else {
            width
        };
        let toggle = ui.invisible_button(format!("cat-{category_id}"), [toggle_w, CATEGORY_H]);
        let color = if ui.is_item_hovered() {
            TEXT_NORMAL
        } else {
            MUTED
        };
        let chevron = if collapsed {
            "chevron-right"
        } else {
            "chevron-down"
        };
        skin::image(
            ui,
            self.icon(chevron),
            [origin[0] + 4.0, text_top + (text_h - 12.0) * 0.5],
            12.0,
            color,
        );
        let label = name.to_uppercase();
        let shown = skin::ellipsize(ui, self.semibold_font, FONT_XS, &label, toggle_w - 22.0);
        skin::text_in_row(
            ui,
            self.semibold_font,
            FONT_XS,
            origin[0] + 18.0,
            text_top,
            text_h,
            color,
            &shown,
        );
        (toggle, create)
    }

    /// A 32 px channel row with 8 px insets, rounded hover and selected fills, icon, then name.
    /// Returns `(left_click, right_click)`.
    fn channel_row(
        &self,
        ui: &Ui,
        id: &str,
        icon: Option<&str>,
        label: &str,
        selected: bool,
    ) -> (bool, bool) {
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button(id, [width, CHANNEL_ROW_H + CHANNEL_ROW_GAP]);
        let right = ui.is_item_clicked_with_button(MouseButton::Right);
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + ROW_INSET, origin[1] + CHANNEL_ROW_GAP * 0.5];
        let max = [origin[0] + width - ROW_INSET, min[1] + CHANNEL_ROW_H];
        let selected_fill = self.shell("row_selected", ROW_SELECTED);
        let hover_fill = self.shell("row_hover", ROW_HOVER);
        let muted = self.shell("muted", MUTED);
        let text = self.shell("text", TEXT_NORMAL);
        if selected || hovered {
            skin::rect(
                ui,
                min,
                max,
                if selected { selected_fill } else { hover_fill },
                ROW_RADIUS,
            );
        }
        let color = if selected {
            WHITE
        } else if hovered {
            text
        } else {
            muted
        };
        let mut text_x = min[0] + 8.0;
        if let Some(icon) = icon {
            skin::image(
                ui,
                self.icon(icon),
                [text_x, min[1] + 6.0],
                20.0,
                if selected { text } else { muted },
            );
            text_x += 20.0 + 6.0;
        }
        let shown = skin::ellipsize(ui, None, FONT_MD, label, max[0] - 8.0 - text_x);
        skin::text_in_row(
            ui,
            None,
            FONT_MD,
            text_x,
            min[1],
            CHANNEL_ROW_H,
            color,
            &shown,
        );
        (clicked, right)
    }

    fn draw_members(&mut self, ui: &Ui) {
        match self.place {
            Place::Server(_) => {
                let members = self.members.clone();
                let rows: Vec<(bool, Option<(i64, String)>)> = members
                    .iter()
                    .map(|member| {
                        let presence = self
                            .presence
                            .get(&member.user_id)
                            .map(|info| info.state)
                            .unwrap_or(member.presence);
                        let online = presence != Presence::Offline;
                        let hoist = top_hoisted_role(&member.roles)
                            .map(|role| (role.position, role.name.clone()));
                        (online, hoist)
                    })
                    .collect();
                for (title, indexes) in plan_member_groups(&rows) {
                    let grouped: Vec<_> = indexes
                        .into_iter()
                        .filter_map(|index| {
                            let member = members.get(index)?.clone();
                            let presence = self
                                .presence
                                .get(&member.user_id)
                                .map(|info| info.state)
                                .unwrap_or(member.presence);
                            Some((member, presence))
                        })
                        .collect();
                    self.member_group(ui, &title, &grouped);
                }
                ui.dummy([0.0, 20.0]);
            }
            Place::Direct(dm_id) => {
                if let Some(chat) = self.dms.iter().find(|chat| chat.id == dm_id).cloned() {
                    let presence = self
                        .presence
                        .get(&chat.peer_id)
                        .map(|p| p.state)
                        .unwrap_or(Presence::Offline);
                    let status = self.presence.get(&chat.peer_id).and_then(|info| {
                        info.custom_status
                            .as_deref()
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                            .map(str::to_string)
                    });
                    ui.dummy([0.0, 16.0]);
                    if self.person_row(
                        ui,
                        chat.peer_id,
                        &chat.peer_display_name,
                        presence,
                        false,
                        status.as_deref(),
                        None,
                    ) {
                        let mouse = ui.mouse_pos();
                        let mine = self.account.as_ref().map(|account| account.id);
                        self.open_context_menu(
                            ui,
                            member_context_menu(
                                chat.peer_id,
                                &chat.peer_username,
                                mine,
                                self.is_friend(chat.peer_id),
                                false,
                            ),
                            mouse,
                        );
                    }
                }
            }
            Place::Home => {}
        }
    }

    fn member_group(
        &mut self,
        ui: &Ui,
        title: &str,
        members: &[(crate::api::MemberView, Presence)],
    ) {
        if members.is_empty() {
            return;
        }
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        ui.dummy([width, MEMBER_GROUP_H]);
        let label = format!("{title} — {}", members.len());
        skin::text_in_row(
            ui,
            self.semibold_font,
            FONT_XS,
            origin[0] + 16.0,
            origin[1] + 24.0,
            16.0,
            MUTED,
            &label,
        );
        for (member, presence) in members {
            let name = member
                .nickname
                .clone()
                .unwrap_or(member.display_name.clone());
            let live_status = self
                .presence
                .get(&member.user_id)
                .and_then(|info| info.custom_status.clone());
            let status = live_status
                .or_else(|| member.custom_status.clone())
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string);
            let color = top_role_color(&member.roles).map(|rgb| rgb_u32_to_rgba(rgb as u32));
            if self.person_row(
                ui,
                member.user_id,
                &name,
                *presence,
                member.is_owner,
                status.as_deref(),
                color,
            ) {
                let mouse = ui.mouse_pos();
                let mine = self.account.as_ref().map(|account| account.id);
                self.open_context_menu(
                    ui,
                    member_context_menu(
                        member.user_id,
                        &member.username,
                        mine,
                        self.is_friend(member.user_id),
                        self.can_permission(PERM_MANAGE_ROLES),
                    ),
                    mouse,
                );
            }
        }
    }

    /// Member row: avatar, name (role color when set), optional custom status, and the owner crown.
    /// Left-click opens the profile card. Returns true when right-clicked for a context menu.
    fn person_row(
        &mut self,
        ui: &Ui,
        user_id: i64,
        name: &str,
        presence: Presence,
        owner: bool,
        custom_status: Option<&str>,
        role_color: Option<[f32; 4]>,
    ) -> bool {
        let row_h = if custom_status.is_some() {
            MEMBER_ROW_H + 14.0
        } else {
            MEMBER_ROW_H
        };
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button(format!("member-{user_id}"), [width, row_h]);
        let right = ui.is_item_clicked_with_button(MouseButton::Right);
        let hovered = ui.is_item_hovered();
        if clicked {
            self.open_profile(user_id, [origin[0] + width, origin[1]]);
        }
        let min = [origin[0] + ROW_INSET, origin[1] + 1.0];
        let max = [origin[0] + width - ROW_INSET, origin[1] + row_h - 1.0];
        if hovered {
            skin::rect(ui, min, max, ROW_HOVER, ROW_RADIUS);
        }
        let backdrop = if hovered { ROW_HOVER } else { SIDEBAR_BG };
        let center = [min[0] + 8.0 + 16.0, origin[1] + row_h * 0.5];
        let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
        paint_avatar(ui, avatar, center, 16.0, &initials(name), None);
        let offline = presence == Presence::Offline;
        if offline {
            ui.get_window_draw_list()
                .add_circle(center, 16.5, with_alpha(backdrop, 0.7))
                .filled(true)
                .build();
        } else {
            paint_status_dot(ui, center, 16.0, presence, backdrop);
        }
        let text_x = min[0] + 8.0 + 32.0 + 12.0;
        let fallback = if offline {
            with_alpha(MUTED, 0.5)
        } else if hovered {
            TEXT_NORMAL
        } else {
            MUTED
        };
        let color = if offline {
            fallback
        } else {
            role_color.unwrap_or(fallback)
        };
        let crown_w = if owner { 18.0 } else { 0.0 };
        let shown = skin::ellipsize(ui, None, FONT_MD, name, max[0] - 8.0 - crown_w - text_x);
        let name_y = if custom_status.is_some() {
            origin[1] + 4.0
        } else {
            origin[1]
        };
        let name_h = if custom_status.is_some() { 20.0 } else { row_h };
        skin::text_in_row(ui, None, FONT_MD, text_x, name_y, name_h, color, &shown);
        if let Some(status) = custom_status {
            let status = skin::ellipsize(ui, None, FONT_XS, status, max[0] - 8.0 - text_x);
            skin::text(
                ui,
                None,
                FONT_XS,
                [text_x, origin[1] + 24.0],
                if offline {
                    with_alpha(MUTED, 0.5)
                } else {
                    TEXT_SECONDARY
                },
                &status,
            );
        }
        if owner {
            let name_w = skin::text_size(ui, None, FONT_MD, &shown)[0];
            let crown_min = [text_x + name_w + 4.0, name_y + name_h * 0.5 - 7.0];
            skin::image(ui, self.icon("crown"), crown_min, 14.0, WHITE);
            if ui.is_mouse_hovering_rect(crown_min, [crown_min[0] + 14.0, crown_min[1] + 14.0]) {
                skin::tooltip(
                    ui,
                    self.semibold_font,
                    FONT_SM,
                    "Server Owner",
                    crown_min,
                    [crown_min[0] + 14.0, crown_min[1] + 14.0],
                    TooltipSide::Top,
                );
            }
        }
        right
    }

    fn draw_message_rows(
        &mut self,
        ui: &Ui,
        mine: Option<i64>,
        reply: &mut Option<i64>,
        edit: &mut Option<i64>,
        delete: &mut Option<i64>,
        save: &mut Option<(i64, String)>,
        collapse: &mut Option<i64>,
        inline_submit: &mut bool,
        inline_cancel: &mut bool,
    ) {
        let offset = zone_offset_secs();
        let now = chrono::Local::now().timestamp_millis();
        let width = ui.content_region_avail()[0].max(160.0);
        let messages = self.messages.clone();
        let by_id: HashMap<i64, Message> = messages.iter().cloned().map(|m| (m.id, m)).collect();
        let rows = comment_tree_rows(&messages, &self.collapsed_comments);
        let mut previous_at: Option<i64> = None;
        let mut previous_depth: Option<u32> = None;
        for row in &rows {
            let Some(message) = by_id.get(&row.message_id) else {
                continue;
            };
            if previous_at.is_some_and(|at| message_starts_new_day(at, message.created_at, offset))
            {
                let label = chat_date_separator(message.created_at, offset);
                let origin = ui.cursor_screen_pos();
                ui.dummy([width, 28.0]);
                let _small = ui.push_font_with_size(None, 12.0);
                let label_w = ui.calc_text_size(&label)[0];
                let mid_y = origin[1] + 14.0;
                let draw = ui.get_window_draw_list();
                draw.add_line(
                    [origin[0] + 16.0, mid_y],
                    [origin[0] + (width - label_w) * 0.5 - 8.0, mid_y],
                    MUTED,
                )
                .build();
                draw.add_text(
                    [origin[0] + (width - label_w) * 0.5, origin[1] + 6.0],
                    MUTED,
                    &label,
                );
                draw.add_line(
                    [origin[0] + (width + label_w) * 0.5 + 8.0, mid_y],
                    [origin[0] + width - 16.0, mid_y],
                    MUTED,
                )
                .build();
            }
            // Roots get more air; nested replies stay tighter so a thread reads as one block.
            let gap = match (previous_depth, row.depth) {
                (Some(0), 0) | (Some(_), 0) => COMMENT_ROW_GAP_ROOT,
                (Some(_), _) => COMMENT_ROW_GAP_NESTED,
                (None, _) => 0.0,
            };
            if gap > 0.0 {
                ui.dummy([width, gap]);
            }
            let visual_depth = row.depth.min(COMMENT_MAX_VISUAL_DEPTH);
            let indent = COMMENT_INDENT * visual_depth as f32;
            let shows_fold = comment_shows_collapse(row.child_count);
            let collapse_gutter = if shows_fold || row.depth > 0 {
                COMMENT_COLLAPSE_W
            } else {
                0.0
            };
            let avatar_x = COMMENT_LEFT_PAD + indent + collapse_gutter;
            let text_x = avatar_x + 40.0 + 16.0;
            let text_w = (width - text_x - 24.0).max(40.0);
            let name = message
                .author_nickname
                .clone()
                .unwrap_or_else(|| message.author_display_name.clone());
            let edited = if message.edited_at.is_some() {
                " (edited)"
            } else {
                ""
            };
            let body_h = if message.body.is_empty() {
                0.0
            } else if edited.is_empty() {
                bfmd_paint::measure(ui, self.bfmd_fonts(), &message.body, text_w)
            } else {
                bfmd_paint::measure(
                    ui,
                    self.bfmd_fonts(),
                    &format!("{}{edited}", message.body),
                    text_w,
                )
            };
            let collapsed_note_h = if row.collapsed && row.child_count > 0 {
                18.0
            } else {
                0.0
            };
            let action_h = COMMENT_ACTION_H;
            let row_h = (22.0 + body_h.max(20.0) + collapsed_note_h + action_h + 4.0).max(52.0);
            let origin = ui.cursor_screen_pos();
            ui.dummy([width, row_h + 2.0]);
            let row_max = [origin[0] + width, origin[1] + row_h];
            let hot = pointer_in(ui, [origin[0], origin[1] - 16.0], row_max);
            let replying_here = self.reply_to == Some(message.id);
            if hot || replying_here {
                let fill = if replying_here {
                    with_alpha(BLURPLE, 0.10)
                } else {
                    ROW_HOVER
                };
                ui.get_window_draw_list()
                    .add_rect(
                        [origin[0] + 4.0, origin[1]],
                        [origin[0] + width - 4.0, origin[1] + row_h],
                        fill,
                    )
                    .filled(true)
                    .build();
            }
            if hot && ui.is_mouse_clicked(MouseButton::Right) {
                let mouse = ui.mouse_pos();
                self.open_context_menu(
                    ui,
                    message_context_menu(message.id, message.author_id, &message.body, mine),
                    mouse,
                );
            }
            // Reddit dark-mode thread lines: straight vertical gutters only (no L-hooks).
            // Drop DrawListMut before fold / line hit targets — they acquire the same window draw list (#31).
            let content_left = origin[0] + COMMENT_LEFT_PAD;
            let pointer = ui.mouse_pos();
            let pointer_in_row = pointer[1] >= origin[1] && pointer[1] <= origin[1] + row_h;
            {
                let draw = ui.get_window_draw_list();
                // Ancestor guides: one muted vertical line per depth, full row height.
                for (level, _) in row.ancestor_ids.iter().enumerate() {
                    let depth = level as u32;
                    let guide_x = comment_thread_guide_x(content_left, depth);
                    let hot_line = pointer_in_row && comment_thread_line_hit(guide_x, pointer[0]);
                    let color = if hot_line {
                        COMMENT_THREAD_HOVER
                    } else {
                        COMMENT_THREAD_LINE
                    };
                    draw.add_line([guide_x, origin[1]], [guide_x, origin[1] + row_h], color)
                        .thickness(COMMENT_GUIDE_THICK)
                        .build();
                }
                // Own branch line starts under this row's collapse circle when expanded.
                if shows_fold && !row.collapsed {
                    let guide_x = comment_thread_guide_x(content_left, visual_depth);
                    let (line_y0, line_y1) = comment_own_branch_line_y(origin[1], row_h);
                    let hot_line = pointer_in_row
                        && pointer[1] >= line_y0
                        && comment_thread_line_hit(guide_x, pointer[0]);
                    let color = if hot_line {
                        COMMENT_THREAD_HOVER
                    } else {
                        COMMENT_THREAD_LINE
                    };
                    draw.add_line([guide_x, line_y0], [guide_x, line_y1], color)
                        .thickness(COMMENT_GUIDE_THICK)
                        .build();
                }
            }
            // Line-click collapses the branch that owns that guide (Reddit).
            for (level, &ancestor_id) in row.ancestor_ids.iter().enumerate() {
                let guide_x = comment_thread_guide_x(content_left, level as u32);
                let hit_x = guide_x - COMMENT_LINE_HIT_W * 0.5;
                ui.set_cursor_screen_pos([hit_x, origin[1]]);
                if ui.invisible_button(
                    format!("thread-line-{ancestor_id}-{}", message.id),
                    [COMMENT_LINE_HIT_W, row_h],
                ) {
                    *collapse = Some(ancestor_id);
                }
                if ui.is_item_hovered() {
                    skin::item_tooltip(ui, self.semibold_font, "Collapse thread");
                }
            }
            if shows_fold {
                let guide_x = comment_thread_guide_x(content_left, visual_depth);
                let fold_min = [
                    guide_x - COMMENT_FOLD_HIT * 0.5,
                    origin[1] + COMMENT_FOLD_PAD_Y,
                ];
                let tip = if row.collapsed {
                    format!("Expand {} replies", row.child_count)
                } else {
                    format!("Collapse {} replies", row.child_count)
                };
                if self.comment_fold_control(ui, message.id, fold_min, row.collapsed, &tip) {
                    *collapse = Some(message.id);
                }
                if !row.collapsed {
                    let (line_y0, line_y1) = comment_own_branch_line_y(origin[1], row_h);
                    let hit_h = (line_y1 - line_y0).max(1.0);
                    ui.set_cursor_screen_pos([guide_x - COMMENT_LINE_HIT_W * 0.5, line_y0]);
                    if ui.invisible_button(
                        format!("thread-own-{}-{}", message.id, message.id),
                        [COMMENT_LINE_HIT_W, hit_h],
                    ) {
                        *collapse = Some(message.id);
                    }
                }
            }
            let center = [origin[0] + avatar_x + 20.0, origin[1] + 22.0];
            let avatar = self
                .avatars
                .get(&message.author_id)
                .map(|shot| shot.texture);
            ui.set_cursor_screen_pos([center[0] - 20.0, center[1] - 20.0]);
            if ui.invisible_button(format!("msg-avatar-{}", message.id), [40.0, 40.0]) {
                self.open_profile(message.author_id, [center[0] + 24.0, center[1]]);
            }
            paint_avatar(ui, avatar, center, 20.0, &initials(&name), None);
            let name_w = {
                let _bold = ui.push_font_with_size(self.bold_font, 16.0);
                ui.calc_text_size(&name)[0]
            };
            ui.set_cursor_screen_pos([origin[0] + text_x, origin[1] + 2.0]);
            if ui.invisible_button(format!("msg-name-{}", message.id), [name_w.max(8.0), 18.0]) {
                self.open_profile(message.author_id, [origin[0] + text_x + name_w, origin[1]]);
            }
            {
                let _bold = ui.push_font_with_size(self.bold_font, 16.0);
                ui.get_window_draw_list().add_text(
                    [origin[0] + text_x, origin[1] + 2.0],
                    TEXT,
                    &name,
                );
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
            if !message.body.is_empty() {
                let body_origin = [origin[0] + text_x, origin[1] + 22.0];
                bfmd_paint::draw(
                    ui,
                    self.bfmd_fonts(),
                    &message.body,
                    body_origin,
                    text_w,
                    edited,
                );
            }
            if row.collapsed && row.child_count > 0 {
                let note_y = origin[1] + 22.0 + body_h.max(20.0);
                let label = if row.child_count == 1 {
                    "▸ 1 more reply".to_string()
                } else {
                    format!("▸ {} more replies", row.child_count)
                };
                let label_w = {
                    let _small = ui.push_font_with_size(None, 13.0);
                    ui.calc_text_size(&label)[0]
                };
                ui.set_cursor_screen_pos([origin[0] + text_x, note_y]);
                if ui.invisible_button(
                    format!("expand-note-{}", message.id),
                    [label_w.max(8.0) + 4.0, 16.0],
                ) {
                    *collapse = Some(message.id);
                }
                {
                    let _small = ui.push_font_with_size(None, 13.0);
                    ui.get_window_draw_list().add_text(
                        [origin[0] + text_x, note_y],
                        BLURPLE_TEXT,
                        &label,
                    );
                }
            }
            let action_y = origin[1] + 22.0 + body_h.max(20.0) + collapsed_note_h + 2.0;
            // Always-visible Reddit-style Reply link (hover toolbar still has the icon).
            {
                let reply_label = if replying_here {
                    "Replying…"
                } else {
                    "Reply"
                };
                let reply_w = {
                    let _small = ui.push_font_with_size(self.semibold_font, 12.0);
                    ui.calc_text_size(reply_label)[0]
                };
                ui.set_cursor_screen_pos([origin[0] + text_x, action_y]);
                if ui.invisible_button(
                    format!("reply-link-{}", message.id),
                    [reply_w.max(8.0) + 4.0, COMMENT_ACTION_H],
                ) {
                    *reply = Some(message.id);
                }
                let reply_hot = ui.is_item_hovered();
                let reply_color = if replying_here || reply_hot {
                    BLURPLE_TEXT
                } else {
                    MUTED
                };
                {
                    let _small = ui.push_font_with_size(self.semibold_font, 12.0);
                    ui.get_window_draw_list().add_text(
                        [origin[0] + text_x, action_y + 2.0],
                        reply_color,
                        reply_label,
                    );
                }
            }
            let mut attachment_y = action_y + action_h;
            for attachment in &message.attachments {
                ui.set_cursor_screen_pos([origin[0] + text_x, attachment_y]);
                ui.text(format!(
                    "file: {} ({} bytes)",
                    attachment.filename, attachment.byte_size
                ));
                ui.same_line();
                if ui.button(format!("Save##att{}", attachment.id)) {
                    *save = Some((attachment.id, attachment.filename.clone()));
                }
                attachment_y += 22.0;
            }
            let mut row_end = attachment_y.max(origin[1] + row_h + 2.0);
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
                        "reply" => "Reply in thread",
                        "pencil" => "Edit",
                        "trash-2" => "Delete",
                        _ => "More",
                    };
                    let active = *name == "reply" && replying_here;
                    if self.tool(
                        ui,
                        &format!("{name}-{}", message.id),
                        name,
                        active,
                        Some(tip),
                    ) {
                        match *name {
                            "reply" => *reply = Some(message.id),
                            "pencil" => *edit = Some(message.id),
                            "trash-2" => *delete = Some(message.id),
                            _ => {
                                let anchor = [bar_x, bar_y + 36.0];
                                self.open_context_menu(
                                    ui,
                                    message_context_menu(
                                        message.id,
                                        message.author_id,
                                        &message.body,
                                        mine,
                                    ),
                                    anchor,
                                );
                            }
                        }
                    }
                }
            }
            // Inline Reddit reply composer — accent stripe + header so it is not the bottom bar.
            if replying_here {
                let box_x = origin[0] + text_x;
                let box_w = (width - text_x - 24.0).max(160.0);
                let box_top = row_end + 4.0;
                let box_h =
                    COMMENT_INLINE_HEADER_H + COMMENT_INLINE_INPUT_H + COMMENT_INLINE_FOOTER_H;
                ui.set_cursor_screen_pos([box_x, box_top]);
                ui.dummy([box_w, box_h]);
                skin::rect_corners(
                    ui,
                    [box_x, box_top],
                    [box_x + box_w, box_top + box_h],
                    COMMENT_INLINE_BG,
                    COMPOSER_RADIUS,
                    DrawFlags::ROUND_ALL,
                );
                {
                    let draw = ui.get_window_draw_list();
                    draw.add_rect(
                        [box_x, box_top],
                        [box_x + COMMENT_INLINE_ACCENT_W, box_top + box_h],
                        BLURPLE,
                    )
                    .filled(true)
                    .build();
                }
                let header = format!("Replying to {name}");
                skin::text(
                    ui,
                    self.semibold_font,
                    12.0,
                    [box_x + 12.0, box_top + 4.0],
                    BLURPLE_TEXT,
                    &header,
                );
                let input_y = box_top + COMMENT_INLINE_HEADER_H;
                skin::rect_corners(
                    ui,
                    [box_x + 8.0, input_y],
                    [box_x + box_w - 8.0, input_y + COMMENT_INLINE_INPUT_H],
                    COMPOSER_BG,
                    6.0,
                    DrawFlags::ROUND_ALL,
                );
                ui.set_cursor_screen_pos([box_x + 16.0, input_y + 8.0]);
                let submit = {
                    let _font = ui.push_font_with_size(None, FONT_MD);
                    let _border = ui.push_style_var(StyleVar::FrameBorderSize(0.0));
                    let _bg = ui.push_style_color(StyleColor::FrameBg, skin::CLEAR);
                    let _hint = ui.push_style_color(StyleColor::TextDisabled, PLACEHOLDER);
                    let _text = ui.push_style_color(StyleColor::Text, TEXT_NORMAL);
                    ui.set_next_item_width((box_w - 32.0).max(40.0));
                    ui.input_text(
                        &format!("##inline-reply-{}", message.id),
                        &mut self.reply_draft,
                    )
                    .hint(format!("Write a reply to {name}"))
                    .enter_returns_true(true)
                    .build()
                };
                if submit {
                    *inline_submit = true;
                }
                let footer_y = input_y + COMMENT_INLINE_INPUT_H + 4.0;
                let cancel_w = 72.0;
                let send_w = 72.0;
                if skin::button(
                    ui,
                    &format!("inline-cancel-{}", message.id),
                    "Cancel",
                    Tone::Secondary,
                    [box_x + box_w - 8.0 - send_w - 8.0 - cancel_w, footer_y],
                    [cancel_w, 24.0],
                    true,
                    self.semibold_font,
                ) {
                    *inline_cancel = true;
                }
                let can_send = !self.reply_draft.trim().is_empty() || self.pending_file.is_some();
                if skin::button(
                    ui,
                    &format!("inline-go-{}", message.id),
                    "Reply",
                    Tone::Brand,
                    [box_x + box_w - 8.0 - send_w, footer_y],
                    [send_w, 24.0],
                    can_send,
                    self.semibold_font,
                ) {
                    *inline_submit = true;
                }
                row_end = box_top + box_h + 4.0;
            }
            // Keep each row ending at its own bottom edge for scroll measurement.
            ui.set_cursor_screen_pos([origin[0], row_end]);
            ui.dummy([0.0, 0.0]);
            previous_at = Some(message.created_at);
            previous_depth = Some(row.depth);
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
        let mut collapse = None;
        let mut inline_submit = false;
        let mut inline_cancel = false;
        let footer = self.composer_footer_h();
        ui.child_window("history")
            .size([0.0, -footer])
            .build(ui, || {
                let stick = crate::theme::chat_should_stick(
                    ui.scroll_y(),
                    ui.scroll_max_y(),
                    self.chat_jump,
                );
                // Pad with last frame's history height so few messages sit on the composer, like Discord.
                let view_h = ui.content_region_avail()[1];
                ui.dummy([0.0, crate::theme::chat_top_pad(self.history_h, view_h)]);
                let history_top = ui.cursor_pos()[1];
                let mine = self.account.as_ref().map(|account| account.id);
                if self.messages.is_empty() && self.search.trim().is_empty() {
                    if let Some(channel) = &self.selected {
                        if matches!(self.place, Place::Server(_))
                            && channel.kind == ChannelKind::Text
                        {
                            ui.dummy([0.0, 24.0]);
                            ui.indent_by(16.0);
                            let _title = ui.push_font_with_size(self.bold_font, 28.0);
                            ui.text(format!("Welcome to #{}!", channel.name));
                            drop(_title);
                            ui.text_colored(
                                MUTED,
                                format!("This is the start of the #{} channel.", channel.name),
                            );
                            ui.unindent_by(16.0);
                        }
                    }
                }
                self.draw_message_rows(
                    ui,
                    mine,
                    &mut reply,
                    &mut edit,
                    &mut delete,
                    &mut save,
                    &mut collapse,
                    &mut inline_submit,
                    &mut inline_cancel,
                );
                self.history_h = ui.cursor_pos()[1] - history_top;
                if stick {
                    ui.set_scroll_here_y(1.0);
                }
                self.chat_jump = false;
            });
        if let Some(id) = reply {
            self.reply_to = Some(id);
            self.reply_draft.clear();
            self.editing = None;
        }
        if let Some(id) = collapse {
            toggle_comment_collapsed(&mut self.collapsed_comments, id);
        }
        if inline_cancel {
            self.reply_to = None;
            self.reply_draft.clear();
        }
        if inline_submit {
            self.submit_inline_reply(target);
        }
        if let Some(id) = edit {
            if let Some(message) = self.messages.iter().find(|message| message.id == id) {
                self.composer = message.body.clone();
            }
            self.editing = Some(id);
            self.reply_to = None;
            self.reply_draft.clear();
            self.pending_file = None;
        }
        if let Some(id) = delete {
            if self.note(self.client.delete_message(id)).is_some() {
                remove_message(&mut self.messages, id);
                self.collapsed_comments.remove(&id);
                if self.reply_to == Some(id) {
                    self.reply_to = None;
                    self.reply_draft.clear();
                }
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
        skin::text_centered(
            ui,
            self.bold_font,
            FONT_MD,
            center_x,
            top,
            TEXT_SECONDARY,
            "NO TEXT CHANNELS",
        );
        let body = "You find yourself in a strange place. You don't have access to any text channels, or there are none in this server.";
        let mut y = top + 28.0;
        for line in skin::wrap_lines(ui, None, FONT_SM, body, (width - 64.0).min(440.0)) {
            skin::text_centered(ui, None, FONT_SM, center_x, y, MUTED, &line);
            y += 20.0;
        }
    }

    fn composer_footer_h(&self) -> f32 {
        // Replies use an inline composer under the parent comment (Reddit). Bottom strip is edit/file/reply-hint.
        let strip =
            if self.editing.is_some() || self.pending_file.is_some() || self.reply_to.is_some() {
                34.0
            } else {
                0.0
            };
        8.0 + strip + COMPOSER_H + COMPOSER_BOTTOM
    }

    /// Rounded message bar for top-level posts: upload button, `Message #channel` placeholder,
    /// and a strip above it while editing, holding a file, or while an inline reply is open.
    /// Nested replies use the accented inline box under the parent comment.
    fn draw_composer(&mut self, ui: &Ui, target: ChatTarget) {
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let bar_x = origin[0] + COMPOSER_MARGIN;
        let bar_w = (width - COMPOSER_MARGIN * 2.0).max(120.0);
        let mut y = origin[1] + 8.0;
        ui.dummy([width, self.composer_footer_h()]);
        let strip = if self.editing.is_some() {
            Some(("Editing message".to_string(), false))
        } else if let Some((name, bytes)) = &self.pending_file {
            Some((format!("{name} ({} KB)", bytes.len().div_ceil(1024)), false))
        } else if self.reply_to.is_some() {
            Some((
                "Replying in the thread above — this bar starts a new top-level post".to_string(),
                true,
            ))
        } else {
            None
        };
        if let Some((label, reply_hint)) = &strip {
            skin::rect_corners(
                ui,
                [bar_x, y],
                [bar_x + bar_w, y + 34.0],
                SIDEBAR_BG,
                COMPOSER_RADIUS,
                DrawFlags::ROUND_TOP,
            );
            let shown = skin::ellipsize(ui, None, FONT_SM, label, bar_w - 64.0);
            let color = if *reply_hint {
                BLURPLE_TEXT
            } else {
                TEXT_SECONDARY
            };
            skin::text_in_row(ui, None, FONT_SM, bar_x + 16.0, y, 34.0, color, &shown);
            if !*reply_hint {
                if skin::icon_button(
                    ui,
                    "composer-cancel",
                    self.icon("x"),
                    [bar_x + bar_w - 8.0 - 24.0, y + 5.0],
                    24.0,
                    16.0,
                    TEXT_SECONDARY,
                    TEXT_NORMAL,
                    None,
                ) {
                    if self.editing.take().is_some() {
                        self.composer.clear();
                    } else {
                        self.pending_file = None;
                    }
                }
            }
            y += 34.0;
        }
        let corners = if strip.is_some() {
            DrawFlags::ROUND_BOTTOM
        } else {
            DrawFlags::ROUND_ALL
        };
        skin::rect_corners(
            ui,
            [bar_x, y],
            [bar_x + bar_w, y + COMPOSER_H],
            self.shell("composer_bg", COMPOSER_BG),
            COMPOSER_RADIUS,
            corners,
        );
        let input_x = if self.editing.is_none() {
            let tint = if self.pending_file.is_some() {
                TEXT_NORMAL
            } else {
                TEXT_SECONDARY
            };
            if skin::icon_button(
                ui,
                "attach",
                self.icon("circle-plus"),
                [bar_x + 12.0, y + 8.0],
                28.0,
                24.0,
                tint,
                TEXT_NORMAL,
                None,
            ) {
                self.pick_attachment();
            }
            skin::item_tooltip(ui, self.semibold_font, "Upload a File");
            bar_x + 12.0 + 28.0 + 12.0
        } else {
            bar_x + 8.0
        };
        let channel_mark = if self.stage_icon() == Some("hash") {
            "#"
        } else {
            "@"
        };
        let hint = if self.reply_to.is_some() {
            format!("New post in {channel_mark}{}", self.stage_title())
        } else {
            format!("Message {channel_mark}{}", self.stage_title())
        };
        ui.set_cursor_screen_pos([input_x, y]);
        let submit = {
            let _font = ui.push_font_with_size(None, FONT_MD);
            let _pad =
                ui.push_style_var(StyleVar::FramePadding([4.0, (COMPOSER_H - FONT_MD) * 0.5]));
            let _border = ui.push_style_var(StyleVar::FrameBorderSize(0.0));
            let _bg = ui.push_style_color(StyleColor::FrameBg, skin::CLEAR);
            let _bg_hover = ui.push_style_color(StyleColor::FrameBgHovered, skin::CLEAR);
            let _bg_active = ui.push_style_color(StyleColor::FrameBgActive, skin::CLEAR);
            let _hint = ui.push_style_color(StyleColor::TextDisabled, PLACEHOLDER);
            let _text = ui.push_style_color(StyleColor::Text, TEXT_NORMAL);
            ui.set_next_item_width((bar_x + bar_w - 16.0 - input_x).max(40.0));
            ui.input_text("##composer", &mut self.composer)
                .hint(hint)
                .enter_returns_true(true)
                .build()
        };
        if submit {
            self.submit_composer(target);
        }
    }

    fn join_voice_channel(&mut self, channel_id: i64) {
        if let Some(members) = self.note(self.client.join_voice(channel_id)) {
            self.drop_self_from_rosters(Some(channel_id));
            apply_voice_roster(&mut self.voice_rosters, channel_id, members.clone());
            self.voice_members = members;
            self.voice_channel = Some(channel_id);
            self.call = None;
            self.stop_ringing();
            self.sfx.play(SoundEvent::Join);
            self.flags_failed_at = None;
            self.sent_flags = VoiceFlags {
                muted: !self.flags.muted,
                ..self.flags
            };
        }
    }

    fn draw_voice_stage(&mut self, ui: &Ui) {
        let channel = self.selected.clone();
        let joined = channel
            .as_ref()
            .is_some_and(|channel| self.voice_channel == Some(channel.id));
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0].max(1.0);
        let height = ui.content_region_avail()[1].max(1.0);
        ui.dummy([width, height]);
        skin::rect(
            ui,
            origin,
            [origin[0] + width, origin[1] + height],
            CALL_BG,
            0.0,
        );
        let semi = self.semibold_font;
        if !joined {
            if let Some(channel) = channel {
                let names: Vec<String> = self
                    .voice_rosters
                    .get(&channel.id)
                    .map(|members| {
                        members
                            .iter()
                            .map(|member| self.name_of(member.user_id))
                            .collect()
                    })
                    .unwrap_or_default();
                let line = voice_occupancy_line(&names);
                let center_x = origin[0] + width * 0.5;
                let title_y = origin[1] + height * 0.4;
                skin::text_centered(ui, semi, FONT_XL, center_x, title_y, TEXT, &channel.name);
                skin::text_centered(
                    ui,
                    None,
                    FONT_MD,
                    center_x,
                    title_y + 36.0,
                    TEXT_SECONDARY,
                    &line,
                );
                if skin::button(
                    ui,
                    "join-voice",
                    "Join Voice",
                    Tone::Green,
                    [center_x - 60.0, title_y + 72.0],
                    [120.0, BUTTON_H],
                    true,
                    semi,
                ) {
                    self.join_voice_channel(channel.id);
                }
            }
            return;
        }
        let members = self.voice_members.clone();
        let people = if members.is_empty() {
            let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
            vec![banter_server::VoiceMember {
                user_id: me,
                flags: self.flags,
                prefer_p2p_video: self.voice.guild_p2p_video,
            }]
        } else {
            members
        };
        let pad = 16.0;
        let area_w = (width - pad * 2.0).max(80.0);
        let area_h = (height - CALL_BAR_H - pad).max(80.0);
        let sharing = self.screen || people.iter().any(|member| member.flags.screen);
        let featured_h = if sharing {
            (area_h * 0.62).floor()
        } else {
            0.0
        };
        if sharing {
            self.draw_featured_screen_at(
                ui,
                [origin[0] + pad, origin[1] + pad],
                area_w,
                featured_h,
            );
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
            self.draw_voice_tile_at(
                ui,
                member,
                tile_origin,
                grid.tile_w.floor(),
                grid.tile_h.floor(),
            );
        }
        self.draw_stage_controls(ui, [origin[0], origin[1] + height - CALL_BAR_H], width);
        if self.push_to_talk {
            let note = format!("Hold {} to talk.", self.voice.ptt_key);
            skin::text_centered(
                ui,
                None,
                FONT_XS,
                origin[0] + width * 0.5,
                origin[1] + height - 18.0,
                MUTED,
                &note,
            );
        }
    }

    /// Discord-style 1:1 DM call stage: large avatar while ringing, two tiles when active.
    fn draw_call_stage(&mut self, ui: &Ui) {
        let Some(call) = self.call.clone() else {
            return;
        };
        let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
        let peer = if call.caller == me {
            call.callee
        } else {
            call.caller
        };
        let origin = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0].max(1.0);
        let height = ui.content_region_avail()[1].max(1.0);
        ui.dummy([width, height]);
        skin::rect(
            ui,
            origin,
            [origin[0] + width, origin[1] + height],
            CALL_BG,
            0.0,
        );
        let semi = self.semibold_font;
        match call.phase {
            CallPhase::Ringing => {
                let center = call_ring_avatar_center(origin, width, height, CALL_BAR_H);
                let radius = CALL_RING_AVATAR * 0.5;
                let name = self.name_of(peer);
                let avatar = self.avatars.get(&peer).map(|shot| shot.texture);
                paint_avatar(ui, avatar, center, radius, &initials(&name), None);
                let status = if call.callee == me {
                    "Incoming Call"
                } else {
                    "Calling…"
                };
                skin::text_centered(
                    ui,
                    semi,
                    FONT_XL,
                    center[0],
                    center[1] + radius + 24.0,
                    TEXT,
                    &name,
                );
                skin::text_centered(
                    ui,
                    None,
                    FONT_MD,
                    center[0],
                    center[1] + radius + 52.0,
                    TEXT_SECONDARY,
                    status,
                );
                if call.callee == me {
                    let y = origin[1] + height - CALL_BAR_H - CALL_BUTTON - 24.0;
                    let xs =
                        centered_row(width, &[CALL_BUTTON, CALL_BUTTON], CALL_BUTTON_GAP * 2.0);
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
                    if self.round_control(
                        ui,
                        "call-decline",
                        "phone-off",
                        [origin[0] + xs[1], y],
                        true,
                        WHITE,
                    ) {
                        if self.note(self.client.decline_call(call.dm_id)).is_some() {
                            self.stop_ringing();
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
                let sharing = self.screen
                    || people.iter().any(|member| member.flags.screen)
                    || self.has_live_stream();
                let featured_h = if sharing {
                    (area_h * 0.62).floor()
                } else {
                    0.0
                };
                if sharing {
                    self.draw_featured_screen_at(
                        ui,
                        [origin[0] + pad, origin[1] + pad],
                        area_w,
                        featured_h,
                    );
                }
                let tiles_top = origin[1] + pad + if sharing { featured_h + TILE_GAP } else { 0.0 };
                let tiles_h =
                    (area_h - if sharing { featured_h + TILE_GAP } else { 0.0 }).max(60.0);
                let count = dm_call_participant_tiles(true);
                let grid = voice_tile_grid(count, area_w, tiles_h, TILE_GAP);
                let grid_w =
                    grid.cols as f32 * grid.tile_w + grid.cols.saturating_sub(1) as f32 * TILE_GAP;
                let grid_h =
                    grid.rows as f32 * grid.tile_h + grid.rows.saturating_sub(1) as f32 * TILE_GAP;
                let start_x = origin[0] + pad + (area_w - grid_w) * 0.5;
                let start_y = tiles_top + (tiles_h - grid_h) * 0.5;
                for (index, member) in people.iter().enumerate().take(count) {
                    let col = index % grid.cols.max(1);
                    let row = index / grid.cols.max(1);
                    let tile_origin = [
                        (start_x + col as f32 * (grid.tile_w + TILE_GAP)).floor(),
                        (start_y + row as f32 * (grid.tile_h + TILE_GAP)).floor(),
                    ];
                    self.draw_voice_tile_at(
                        ui,
                        member,
                        tile_origin,
                        grid.tile_w.floor(),
                        grid.tile_h.floor(),
                    );
                }
            }
        }
        self.draw_stage_controls(ui, [origin[0], origin[1] + height - CALL_BAR_H], width);
        if self.push_to_talk {
            let note = format!("Hold {} to talk.", self.voice.ptt_key);
            skin::text_centered(
                ui,
                None,
                FONT_XS,
                origin[0] + width * 0.5,
                origin[1] + height - 18.0,
                MUTED,
                &note,
            );
        }
    }

    fn dm_call_members(&self, me: i64, peer: i64) -> Vec<VoiceMember> {
        let mut peer_flags = self
            .voice_members
            .iter()
            .find(|member| member.user_id == peer)
            .map(|member| member.flags)
            .unwrap_or_default();
        // Flag WS can lag; remote textures already prove camera / Go Live.
        if self.remote_video.contains_key(&(peer, 3)) {
            peer_flags.screen = true;
        }
        if self.remote_video.contains_key(&(peer, 2)) {
            peer_flags.camera = true;
        }
        vec![
            VoiceMember {
                user_id: peer,
                flags: peer_flags,
                prefer_p2p_video: false,
            },
            VoiceMember {
                user_id: me,
                flags: self.flags,
                prefer_p2p_video: false,
            },
        ]
    }

    /// Centered call controls: camera, screen share with its options chevron, mic, and the red disconnect.
    fn draw_stage_controls(&mut self, ui: &Ui, origin: [f32; 2], width: f32) {
        let semi = self.semibold_font;
        let widths = [
            CALL_BUTTON,
            CALL_BUTTON + CALL_CHEVRON_W,
            CALL_BUTTON,
            CALL_BUTTON,
        ];
        let xs = centered_row(width, &widths, CALL_BUTTON_GAP);
        let y = origin[1] + (CALL_BAR_H - CALL_BUTTON) * 0.5;
        let camera = self.camera;
        let camera_icon = if camera { "video" } else { "video-off" };
        if self.round_control(
            ui,
            "stage-camera",
            camera_icon,
            [origin[0] + xs[0], y],
            false,
            WHITE,
        ) {
            self.camera = !self.camera;
        }
        skin::item_tooltip(
            ui,
            semi,
            if camera {
                "Turn Off Camera"
            } else {
                "Turn On Camera"
            },
        );
        let live = self.share.is_live();
        let screen_min = [origin[0] + xs[1], y];
        let screen_icon = if live { "monitor" } else { "monitor-off" };
        let (share_clicked, options_clicked, share_custom) =
            self.split_control(ui, "stage-screen", screen_icon, screen_min);
        if share_clicked || share_custom {
            self.on_share_click(share_clicked, share_custom);
        }
        if options_clicked {
            self.stream_anchor = screen_min;
            self.stream_menu = !self.stream_menu;
        }
        let muted = self.mute.muted();
        let (mic_icon, mic_tint) = if muted {
            ("mic-off", RED)
        } else {
            ("mic", WHITE)
        };
        if self.round_control(
            ui,
            "stage-mic",
            mic_icon,
            [origin[0] + xs[2], y],
            false,
            mic_tint,
        ) {
            let next = !muted;
            self.mute.set_muted(next);
            self.play_mute_toggle(next);
        }
        skin::item_tooltip(ui, semi, if muted { "Unmute" } else { "Mute" });
        if self.round_control(
            ui,
            "stage-disconnect",
            "phone-off",
            [origin[0] + xs[3], y],
            true,
            WHITE,
        ) {
            self.end_voice_or_call();
        }
        let end_tip = match &self.call {
            Some(call)
                if call.phase == CallPhase::Ringing
                    && self.account.as_ref().map(|a| a.id) != Some(call.callee) =>
            {
                "Cancel"
            }
            Some(call) if call.phase == CallPhase::Ringing => "Decline",
            Some(_) => "Disconnect",
            None => "Disconnect",
        };
        skin::item_tooltip(ui, semi, end_tip);
    }

    fn round_control(
        &self,
        ui: &Ui,
        id: &str,
        icon: &str,
        origin: [f32; 2],
        danger: bool,
        tint: [f32; 4],
    ) -> bool {
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
        skin::image(
            ui,
            self.icon(icon),
            [origin[0] + inset, origin[1] + inset],
            24.0,
            tint,
        );
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
            self.stop_streaming();
            return;
        }
        if media::native_screen_picker() {
            self.share_open = false;
            self.share.go_live_portal();
            self.save_stream();
            self.sfx.play(SoundEvent::StartStream);
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
        let custom = ui.is_item_clicked_with_button(dear_imgui_rs::MouseButton::Right)
            && ui.io().key_shift();
        if ui.is_item_hovered() {
            skin::rect_corners(
                ui,
                origin,
                [origin[0] + CALL_BUTTON, max[1]],
                CONTROL_HOVER,
                CALL_BUTTON * 0.5,
                DrawFlags::ROUND_LEFT,
            );
        }
        let live = self.share.is_live();
        skin::item_tooltip(ui, semi, share_tip(live));
        let chevron_min = [origin[0] + CALL_BUTTON, origin[1]];
        ui.set_cursor_screen_pos(chevron_min);
        let options = ui.invisible_button(format!("{id}-options"), [CALL_CHEVRON_W, CALL_BUTTON]);
        if ui.is_item_hovered() || self.stream_menu {
            skin::rect_corners(
                ui,
                chevron_min,
                max,
                CONTROL_HOVER,
                CALL_BUTTON * 0.5,
                DrawFlags::ROUND_RIGHT,
            );
        }
        skin::item_tooltip(ui, semi, "Stream Options");
        skin::rect(
            ui,
            [chevron_min[0], origin[1] + 14.0],
            [chevron_min[0] + 1.0, max[1] - 14.0],
            CALL_BG,
            0.0,
        );
        let inset = (CALL_BUTTON - 24.0) * 0.5;
        skin::image(
            ui,
            self.icon(icon),
            [origin[0] + inset, origin[1] + inset],
            24.0,
            WHITE,
        );
        skin::image(
            ui,
            self.icon("chevron-up"),
            [chevron_min[0] + 2.0, origin[1] + (CALL_BUTTON - 16.0) * 0.5],
            16.0,
            TEXT_NORMAL,
        );
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
                    Modal::EditChannel => self.modal_edit_channel(ui, display),
                    Modal::CreateCategory => self.modal_create_category(ui, display),
                    Modal::Roles => self.modal_roles(ui, display),
                    Modal::MemberRoles => self.modal_member_roles(ui, display),
                    Modal::ServerFolder => self.modal_server_folder(ui, display),
                    Modal::UploadsUnavailable => self.modal_uploads_unavailable(ui, display),
                    Modal::AddPlugin => self.modal_add_plugin(ui, display),
                    Modal::PluginAllow => self.modal_plugin_allow(ui, display),
                };
                if self.modal_frames > 0 && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    let mouse = ui.mouse_pos();
                    let inside = mouse[0] >= card.0[0]
                        && mouse[0] <= card.1[0]
                        && mouse[1] >= card.0[1]
                        && mouse[1] <= card.1[1];
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
        let lines = subtitle
            .map(|text| {
                skin::wrap_lines(
                    ui,
                    None,
                    if left_aligned { FONT_XS } else { FONT_MD },
                    text,
                    inner_w - 24.0,
                )
            })
            .unwrap_or_default();
        let line_h = if left_aligned { 16.0 } else { 20.0 };
        let header_h = 24.0 + title_size + 8.0 + lines.len() as f32 * line_h + 16.0;
        let height = header_h + body_h + footer_h;
        let min = [
            ((display[0] - width) * 0.5).floor(),
            ((display[1] - height) * 0.5).max(16.0).floor(),
        ];
        let max = [min[0] + width, min[1] + height];
        skin::rect(ui, min, max, STAGE_BG, MODAL_RADIUS);
        if footer_h > 0.0 {
            skin::rect_corners(
                ui,
                [min[0], max[1] - footer_h],
                max,
                MODAL_FOOTER_BG,
                MODAL_RADIUS,
                DrawFlags::ROUND_BOTTOM,
            );
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
                skin::text(
                    ui,
                    None,
                    FONT_XS,
                    [min[0] + MODAL_PAD, y],
                    TEXT_SECONDARY,
                    line,
                );
            } else {
                skin::text_centered(ui, None, FONT_MD, center_x, y, TEXT_SECONDARY, line);
            }
            y += line_h;
        }
        if skin::icon_button(
            ui,
            "modal-close",
            self.icon("x"),
            [max[0] - 16.0 - 24.0, min[1] + 16.0],
            24.0,
            24.0,
            TEXT_SECONDARY,
            TEXT_NORMAL,
            None,
        ) {
            self.modal = None;
        }
        ([min[0] + MODAL_PAD, min[1] + header_h], min, max)
    }

    /// Footer with a text link on the left and the primary button on the right.
    fn modal_footer(
        &mut self,
        ui: &Ui,
        max: [f32; 2],
        min_x: f32,
        back: &str,
        primary: &str,
        tone: Tone,
        enabled: bool,
    ) -> (bool, bool) {
        let semi = self.semibold_font;
        let y = max[1] - MODAL_FOOTER_H + (MODAL_FOOTER_H - BUTTON_H) * 0.5;
        let back_w = skin::text_size(ui, semi, FONT_SM, back)[0] + 32.0;
        let back_clicked = skin::button(
            ui,
            "modal-back",
            back,
            Tone::Link,
            [min_x + MODAL_PAD - 16.0, y],
            [back_w, BUTTON_H],
            true,
            semi,
        );
        let primary_w = (skin::text_size(ui, semi, FONT_SM, primary)[0] + 32.0).max(96.0);
        let primary_clicked = skin::button(
            ui,
            "modal-primary",
            primary,
            tone,
            [max[0] - MODAL_PAD - primary_w, y],
            [primary_w, BUTTON_H],
            enabled,
            semi,
        );
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
            Some(
                "Your server is where you and your friends hang out. Make yours and start talking.",
            ),
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
        skin::rect(
            ui,
            option_min,
            option_max,
            if hovered { ROW_HOVER } else { STAGE_BG },
            8.0,
        );
        ui.get_window_draw_list()
            .add_rect(option_min, option_max, LIST_DIVIDER)
            .rounding(8.0)
            .thickness(1.0)
            .build();
        skin::image(
            ui,
            self.icon("circle-plus"),
            [option_min[0] + 16.0, option_min[1] + 16.0],
            32.0,
            GREEN,
        );
        skin::text_in_row(
            ui,
            semi,
            FONT_MD,
            option_min[0] + 64.0,
            option_min[1],
            64.0,
            TEXT,
            "Create My Own",
        );
        skin::image(
            ui,
            self.icon("chevron-right"),
            [option_max[0] - 16.0 - 20.0, option_min[1] + 22.0],
            20.0,
            TEXT_SECONDARY,
        );
        let footer_top = max[1] - footer_h;
        skin::text_centered(
            ui,
            semi,
            FONT_LG,
            (min[0] + max[0]) * 0.5,
            footer_top + 16.0,
            TEXT,
            "Have an invite already?",
        );
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
        let enter = skin::input(
            ui,
            "##server-name",
            &mut self.server_name,
            "",
            [body[0], body[1] + 20.0],
            inner_w,
            INPUT_H,
            FONT_MD,
            false,
        );
        let enabled = !self.server_name.trim().is_empty();
        let (back, create) =
            self.modal_footer(ui, max, min[0], "Back", "Create", Tone::Brand, enabled);
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
        let enter = skin::input(
            ui,
            "##invite-code",
            &mut self.invite_code,
            "Invite code",
            [body[0], body[1] + 20.0],
            inner_w,
            INPUT_H,
            FONT_MD,
            false,
        );
        let enabled = !self.invite_code.trim().is_empty();
        let (back, join) =
            self.modal_footer(ui, max, min[0], "Back", "Join Server", Tone::Brand, enabled);
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
            Place::Server(id) => self
                .servers
                .iter()
                .find(|server| server.id == id)
                .map(|server| server.name.clone()),
            _ => None,
        }
        .unwrap_or_else(|| "your server".into());
        let channel = self
            .selected
            .as_ref()
            .filter(|channel| channel.kind == ChannelKind::Text)
            .map(|channel| format!("# {}", channel.name));
        let title = format!("Invite friends to {server}");
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            &title,
            channel.as_deref(),
            true,
            20.0 + INPUT_H + 24.0,
            0.0,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(
            ui,
            self.bold_font,
            body,
            "Or, send a server invite link to a friend",
            false,
            None,
        );
        let field_min = [body[0], body[1] + 20.0];
        skin::rect(
            ui,
            field_min,
            [field_min[0] + inner_w, field_min[1] + INPUT_H],
            INPUT_BG,
            INPUT_RADIUS,
        );
        let mut code = self.shown_invite.clone();
        skin::input(
            ui,
            "##invite-link",
            &mut code,
            "",
            field_min,
            inner_w - 84.0,
            INPUT_H,
            FONT_MD,
            true,
        );
        let copied = self.invite_copied;
        let enabled = !self.shown_invite.is_empty();
        let button_min = [field_min[0] + inner_w - 4.0 - 75.0, field_min[1] + 4.0];
        let tone = if copied { Tone::Green } else { Tone::Brand };
        if skin::button(
            ui,
            "copy-invite",
            if copied { "Copied" } else { "Copy" },
            tone,
            button_min,
            [75.0, 32.0],
            enabled,
            self.semibold_font,
        ) {
            crate::platform::copy_to_clipboard(&self.shown_invite);
            self.invite_copied = true;
        }
        (min, max)
    }

    fn modal_server_folder(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let title = match self.folder_mode {
            FolderModalMode::Create => "Create Folder",
            FolderModalMode::Rename => "Rename Folder",
            FolderModalMode::Recolor => "Folder Color",
        };
        let show_name = self.folder_mode != FolderModalMode::Recolor;
        let picker_h = 8.0 + 22.0 * 3.0;
        let body_h = if show_name {
            20.0 + INPUT_H + 12.0 + 20.0 + INPUT_H + picker_h + 28.0
        } else {
            20.0 + INPUT_H + picker_h + 28.0
        };
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            title,
            Some("Hex color like 5865F2"),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let mut y = body[1];
        if show_name {
            skin::field_label(ui, self.bold_font, [body[0], y], "Folder name", false, None);
            y += 20.0;
            ui.set_cursor_screen_pos([body[0], y]);
            ui.set_next_item_width(inner_w);
            ui.input_text("##folder-name", &mut self.folder_name)
                .build();
            y += INPUT_H + 12.0;
        }
        skin::field_label(ui, self.bold_font, [body[0], y], "Color", false, None);
        y += 20.0;
        let swatch = match parse_folder_color(&self.folder_color) {
            Ok(0) => STAGE_BG,
            Ok(rgb) => rgb_u32_to_rgba(rgb as u32),
            Err(_) => RED,
        };
        skin::rect(
            ui,
            [body[0], y],
            [body[0] + INPUT_H, y + INPUT_H],
            swatch,
            8.0,
        );
        ui.set_cursor_screen_pos([body[0] + INPUT_H + 8.0, y]);
        ui.set_next_item_width(inner_w - INPUT_H - 8.0);
        ui.input_text("##folder-color", &mut self.folder_color)
            .hint("5865F2")
            .build();
        // Compact RGB sliders act as the color picker; values sync into the hex field.
        let mut rgb = match parse_folder_color(&self.folder_color) {
            Ok(value) if value > 0 => [
                ((value >> 16) & 0xFF) as f32 / 255.0,
                ((value >> 8) & 0xFF) as f32 / 255.0,
                (value & 0xFF) as f32 / 255.0,
            ],
            _ => [0.345, 0.396, 0.949],
        };
        y += INPUT_H + 8.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.set_next_item_width(inner_w);
        let mut changed = false;
        changed |= ui.slider_f32("##folder-r", &mut rgb[0], 0.0, 1.0);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.set_next_item_width(inner_w);
        changed |= ui.slider_f32("##folder-g", &mut rgb[1], 0.0, 1.0);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.set_next_item_width(inner_w);
        changed |= ui.slider_f32("##folder-b", &mut rgb[2], 0.0, 1.0);
        if changed {
            let packed = (((rgb[0].clamp(0.0, 1.0) * 255.0).round() as i64) << 16)
                | (((rgb[1].clamp(0.0, 1.0) * 255.0).round() as i64) << 8)
                | ((rgb[2].clamp(0.0, 1.0) * 255.0).round() as i64);
            self.folder_color = format!("{packed:06X}");
        }
        if !self.folder_error.is_empty() {
            y += 28.0;
            ui.set_cursor_screen_pos([body[0], y]);
            ui.text_colored(RED, &self.folder_error);
        }
        let primary = match self.folder_mode {
            FolderModalMode::Create => "Create Folder",
            FolderModalMode::Rename => "Save",
            FolderModalMode::Recolor => "Save Color",
        };
        let enabled = match self.folder_mode {
            FolderModalMode::Create | FolderModalMode::Rename => {
                !self.folder_name.trim().is_empty()
            }
            FolderModalMode::Recolor => true,
        };
        let (cancel, save) =
            self.modal_footer(ui, max, min[0], "Cancel", primary, Tone::Brand, enabled);
        if cancel {
            self.modal = None;
        } else if save {
            self.submit_server_folder();
        }
        (min, max)
    }

    /// Notice when the composer attach control (or any other chat upload path) is used.
    fn modal_add_plugin(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let folder = self.data_dir.join("plugins");
        let subtitle = format!(
            "Add a package from {}, or remove one that is already on this server.",
            folder.display()
        );
        let list_h = (self.local_plugins.len().max(1) as f32 * 52.0).min(280.0);
        let notice = self.plugin_notice.clone();
        let notice_h = if notice.is_empty() { 0.0 } else { 40.0 };
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Plugins",
            Some(&subtitle),
            true,
            list_h + notice_h + 8.0,
            0.0,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let plugins = self.local_plugins.clone();
        let installed = self.installed_plugins.clone();
        let mut chosen = None;
        let mut removed = None;
        if !notice.is_empty() {
            skin::text(ui, None, FONT_SM, body, TEXT, &notice);
        }
        ui.set_cursor_screen_pos([body[0], body[1] + notice_h]);
        ui.child_window("local-plugins")
            .size([inner_w, list_h])
            .build(ui, || {
                if plugins.is_empty() && installed.is_empty() {
                    skin::text(
                        ui,
                        None,
                        FONT_SM,
                        ui.cursor_screen_pos(),
                        MUTED,
                        "No plugins in that folder, and none on this server.",
                    );
                    return;
                }
                for plugin in &plugins {
                    let origin = ui.cursor_screen_pos();
                    ui.dummy([inner_w, 48.0]);
                    let next = ui.cursor_screen_pos();
                    skin::text(ui, self.semibold_font, FONT_SM, origin, TEXT, &plugin.name);
                    let installed_version = installed
                        .iter()
                        .find(|(id, _, _)| id == &plugin.id)
                        .map(|(_, _, version)| version.as_str());
                    let newer = installed_version.is_some_and(|version| {
                        crate::plugins::plugin_version_is_newer(&plugin.version, version)
                    });
                    let detail = if let Some(reason) = &plugin.failed {
                        reason.clone()
                    } else {
                        crate::plugins::plugin_usage(&plugin.id).to_string()
                    };
                    skin::text(
                        ui,
                        None,
                        FONT_XS,
                        [origin[0], origin[1] + 18.0],
                        MUTED,
                        &detail,
                    );
                    let on_server = installed_version.is_some();
                    let can_add = plugin.failed.is_none() && (!on_server || newer);
                    if on_server
                        && skin::button(
                            ui,
                            &format!("remove-plugin-{}", plugin.id),
                            "Remove",
                            Tone::Danger,
                            [origin[0] + inner_w - 80.0, origin[1] + 8.0],
                            [72.0, 32.0],
                            true,
                            self.semibold_font,
                        )
                    {
                        removed = Some(plugin.id.clone());
                    }
                    if can_add {
                        let add_x = if on_server {
                            origin[0] + inner_w - 160.0
                        } else {
                            origin[0] + inner_w - 88.0
                        };
                        let label = if newer { "Update" } else { "Add" };
                        if skin::button(
                            ui,
                            &format!("add-plugin-{}", plugin.id),
                            label,
                            Tone::Brand,
                            [add_x, origin[1] + 8.0],
                            [72.0, 32.0],
                            true,
                            self.semibold_font,
                        ) {
                            chosen = Some(plugin.id.clone());
                        }
                    }
                    ui.set_cursor_screen_pos(next);
                }
                for (id, name, version) in &installed {
                    if plugins.iter().any(|plugin| plugin.id == *id) {
                        continue;
                    }
                    let origin = ui.cursor_screen_pos();
                    ui.dummy([inner_w, 48.0]);
                    let next = ui.cursor_screen_pos();
                    skin::text(ui, self.semibold_font, FONT_SM, origin, TEXT, name);
                    skin::text(
                        ui,
                        None,
                        FONT_XS,
                        [origin[0], origin[1] + 18.0],
                        MUTED,
                        &format!("On this server · {version}"),
                    );
                    if skin::button(
                        ui,
                        &format!("remove-installed-{id}"),
                        "Remove",
                        Tone::Danger,
                        [origin[0] + inner_w - 80.0, origin[1] + 8.0],
                        [72.0, 32.0],
                        true,
                        self.semibold_font,
                    ) {
                        removed = Some(id.clone());
                    }
                    ui.set_cursor_screen_pos(next);
                }
            });
        if let Some(id) = removed {
            self.remove_server_plugin(&id);
        } else if let Some(id) = chosen {
            self.begin_plugin_allow(&id);
        }
        (min, max)
    }

    fn modal_plugin_allow(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let pending = self.plugin_pending.clone();
        let plugin = pending
            .as_ref()
            .and_then(|id| self.local_plugins.iter().find(|plugin| plugin.id == *id))
            .cloned();
        let Some(plugin) = plugin else {
            let (body, min, max) =
                self.modal_frame(ui, display, "Add Plugin", None, true, 24.0, 0.0);
            let _ = body;
            return (min, max);
        };
        let second = self.plugin_confirm_step == 1;
        let mut lines = if second {
            crate::plugins::plugin_file_lines(&plugin.permissions)
        } else {
            let mut lines = crate::plugins::plugin_server_lines(&plugin.permissions);
            if lines.is_empty() {
                lines.push("This plugin asks for nothing.".into());
            }
            lines.push(crate::plugins::plugin_usage(&plugin.id).to_string());
            lines
        };
        if lines.is_empty() {
            lines.push("Allow this plugin to run on this server.".into());
        }
        let body_h = lines.len() as f32 * 22.0 + 8.0;
        let subtitle = if second {
            "Banter is not responsible for addons that modify your files."
        } else {
            "Allow this plugin to run on this server?"
        };
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            &plugin.name,
            Some(subtitle),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let mut y = body[1];
        for (index, line) in lines.iter().enumerate() {
            if second && index == 0 && line.starts_with("Banter is not responsible") {
                continue;
            }
            skin::text(ui, None, FONT_SM, [body[0], y], TEXT, line);
            y += 22.0;
        }
        let (cancel, allow) =
            self.modal_footer(ui, max, min[0], "Cancel", "Allow", Tone::Brand, true);
        if cancel {
            self.plugin_pending = None;
            self.plugin_confirm_step = 0;
            self.open_modal(Modal::AddPlugin);
        } else if allow {
            self.allow_plugin();
        }
        (min, max)
    }

    fn modal_uploads_unavailable(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (_body, min, max) = self.modal_frame(
            ui,
            display,
            UPLOADS_UNAVAILABLE_MESSAGE,
            None,
            false,
            16.0,
            MODAL_FOOTER_H,
        );
        let (dismiss, ok) = self.modal_footer(ui, max, min[0], "Dismiss", "OK", Tone::Brand, true);
        if dismiss || ok {
            self.modal = None;
        }
        (min, max)
    }

    fn modal_roles(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let body_h = 112.0 + 20.0 + INPUT_H + 12.0 + 20.0 + INPUT_H + 12.0 + 22.0 * 6.0 + 72.0;
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Server Roles",
            Some("Color, hoist, and permissions"),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let roles = self.guild_roles.clone();
        let list_bg = ui.push_style_color(StyleColor::ChildBg, SIDEBAR_BG);
        ui.set_cursor_screen_pos(body);
        ui.child_window("role-list")
            .size([inner_w, 104.0])
            .build(ui, || {
                if ui
                    .selectable_config("New role")
                    .selected(self.editing_role.is_none())
                    .build()
                {
                    self.clear_role_form();
                }
                for role in &roles {
                    let label = format!("{}##role-{}", role.name, role.id);
                    let selected = self.editing_role == Some(role.id);
                    if ui.selectable_config(&label).selected(selected).build() {
                        self.load_role_form(role);
                    }
                }
            });
        drop(list_bg);
        let mut y = body[1] + 112.0;
        skin::field_label(ui, self.bold_font, [body[0], y], "Role name", false, None);
        y += 20.0;
        let everyone = self
            .editing_role
            .and_then(|id| {
                self.guild_roles
                    .iter()
                    .find(|role| role.id == id)
                    .map(|role| role.everyone)
            })
            .unwrap_or(false);
        ui.set_cursor_screen_pos([body[0], y]);
        ui.set_next_item_width(inner_w);
        let _disabled = ui.begin_disabled_with_cond(everyone);
        ui.input_text("##role-name", &mut self.role_name).build();
        drop(_disabled);
        y += INPUT_H + 12.0;
        skin::field_label(ui, self.bold_font, [body[0], y], "Color", false, None);
        y += 20.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.set_next_item_width(inner_w);
        ui.input_text("##role-color", &mut self.role_color)
            .hint("5865F2")
            .build();
        y += INPUT_H + 8.0;
        ui.set_cursor_screen_pos([body[0], y]);
        let _disabled = ui.begin_disabled_with_cond(everyone);
        ui.checkbox("Show separately in the member list", &mut self.role_hoist);
        drop(_disabled);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Create invite", &mut self.role_invite);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Manage channels", &mut self.role_channels);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Manage roles", &mut self.role_manage);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Manage plugins", &mut self.role_plugins);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Manage bots", &mut self.role_bots);
        y += 22.0;
        ui.set_cursor_screen_pos([body[0], y]);
        ui.checkbox("Administrator", &mut self.role_admin);
        y += 24.0;
        if self.editing_role.is_some() && !everyone {
            ui.set_cursor_screen_pos([body[0], y]);
            if ui.small_button("Delete role") {
                self.delete_editing_role();
            }
            y += 22.0;
        }
        if !self.role_error.is_empty() {
            ui.set_cursor_screen_pos([body[0], y]);
            ui.text_colored(RED, &self.role_error);
        }
        let primary = if self.editing_role.is_some() {
            "Save Role"
        } else {
            "Create Role"
        };
        let (cancel, save) = self.modal_footer(
            ui,
            max,
            min[0],
            "Cancel",
            primary,
            Tone::Brand,
            !self.role_name.trim().is_empty() || everyone,
        );
        if cancel {
            self.modal = None;
        } else if save {
            self.submit_role(everyone);
        }
        (min, max)
    }

    fn modal_member_roles(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let user_id = self.assign_user;
        let name = user_id
            .and_then(|id| self.members.iter().find(|member| member.user_id == id))
            .map(|member| {
                member
                    .nickname
                    .clone()
                    .unwrap_or_else(|| member.display_name.clone())
            })
            .unwrap_or_else(|| "Member".into());
        let roles: Vec<GuildRole> = self
            .guild_roles
            .iter()
            .filter(|role| !role.everyone)
            .cloned()
            .collect();
        let body_h = 24.0 + (roles.len().max(1) as f32 * 24.0) + 16.0;
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Roles",
            Some(&name),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let mut y = body[1];
        if roles.is_empty() {
            ui.set_cursor_screen_pos([body[0], y]);
            ui.text_colored(MUTED, "No roles yet. Create one from Server Roles.");
        }
        for role in &roles {
            ui.set_cursor_screen_pos([body[0], y]);
            let mut on = self.assign_role_ids.contains(&role.id);
            let label = format!("{}##assign-{}", role.name, role.id);
            if ui.checkbox(&label, &mut on) {
                if on {
                    if !self.assign_role_ids.contains(&role.id) {
                        self.assign_role_ids.push(role.id);
                    }
                } else {
                    self.assign_role_ids.retain(|id| *id != role.id);
                }
            }
            y += 24.0;
        }
        if !self.role_error.is_empty() {
            ui.set_cursor_screen_pos([body[0], y]);
            ui.text_colored(RED, &self.role_error);
        }
        let (cancel, save) = self.modal_footer(
            ui,
            max,
            min[0],
            "Cancel",
            "Save",
            Tone::Brand,
            user_id.is_some(),
        );
        if cancel {
            self.modal = None;
        } else if save {
            if let (Place::Server(server_id), Some(user_id)) = (self.place, user_id) {
                let ids = self.assign_role_ids.clone();
                match self.client.set_member_roles(server_id, user_id, &ids) {
                    Ok(()) => {
                        self.role_error.clear();
                        self.reload_members();
                        self.modal = None;
                    }
                    Err(err) => self.role_error = err.to_string(),
                }
            }
        }
        (min, max)
    }

    fn modal_nickname(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Change Nickname",
            None,
            false,
            20.0 + INPUT_H + 8.0 + 20.0 + 16.0,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Nickname", false, None);
        self.focus_modal_field(ui);
        let hint = self.display_name.clone();
        let enter = skin::input(
            ui,
            "##nickname",
            &mut self.nickname,
            &hint,
            [body[0], body[1] + 20.0],
            inner_w,
            INPUT_H,
            FONT_MD,
            false,
        );
        if skin::link(
            ui,
            "reset-nickname",
            "Reset Nickname",
            [body[0], body[1] + 20.0 + INPUT_H + 8.0],
            None,
            FONT_SM,
            LINK,
        ) {
            self.nickname.clear();
        }
        let (cancel, save) =
            self.modal_footer(ui, max, min[0], "Cancel", "Save", Tone::Brand, true);
        if cancel {
            self.modal = None;
        } else if save || enter {
            if let Place::Server(server_id) = self.place {
                let nickname = {
                    let trimmed = self.nickname.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                };
                if self
                    .note(self.client.set_nickname(server_id, nickname.as_deref()))
                    .is_some()
                {
                    self.reload_members();
                    self.modal = None;
                }
            }
        }
        (min, max)
    }

    fn channel_icon_picker_height(&self) -> f32 {
        let cols = 8usize;
        // Default tile plus every curated Lucide choice.
        let tiles = CHANNEL_ICON_CHOICES.len() + 1;
        let rows = tiles.div_ceil(cols).max(1);
        20.0 + rows as f32 * 36.0 + 8.0
    }

    /// Lucide picker: Default (kind glyph) plus curated channel icons. Returns true if changed.
    fn draw_channel_icon_picker(
        &mut self,
        ui: &Ui,
        origin: [f32; 2],
        width: f32,
        kind: &ChannelKind,
    ) -> bool {
        skin::field_label(ui, self.bold_font, origin, "Channel icon", false, None);
        let y = origin[1] + 20.0;
        let cols = 8usize;
        let cell = 32.0;
        let gap = 4.0;
        let default_name = channel_default_icon(kind);
        let mut changed = false;
        let mut tiles: Vec<(Option<&str>, &str)> = vec![(None, default_name)];
        for name in CHANNEL_ICON_CHOICES {
            tiles.push((Some(*name), *name));
        }
        for (index, (value, preview)) in tiles.into_iter().enumerate() {
            let col = index % cols;
            let row = index / cols;
            let x = origin[0] + col as f32 * (cell + gap);
            let top = y + row as f32 * (cell + gap);
            let min = [x, top];
            let max = [x + cell, top + cell];
            ui.set_cursor_screen_pos(min);
            let id = match value {
                None => "##channel-icon-default".to_string(),
                Some(name) => format!("##channel-icon-{name}"),
            };
            if ui.invisible_button(id, [cell, cell]) {
                self.channel_icon = value.map(str::to_string);
                changed = true;
            }
            let hovered = ui.is_item_hovered();
            let selected = self.channel_icon.as_deref() == value;
            let fill = if selected {
                ROW_SELECTED
            } else if hovered {
                ROW_HOVER
            } else {
                INPUT_BG
            };
            skin::rect(ui, min, max, fill, 4.0);
            skin::image(
                ui,
                self.icon(preview),
                [min[0] + 8.0, min[1] + 8.0],
                16.0,
                if selected { WHITE } else { TEXT_SECONDARY },
            );
            if hovered {
                let tip = if value.is_none() { "Default" } else { preview };
                skin::item_tooltip(ui, self.semibold_font, tip);
            }
        }
        let _ = width;
        let rows = (CHANNEL_ICON_CHOICES.len() + 1).div_ceil(cols).max(1);
        let _end = y + rows as f32 * (cell + gap);
        changed
    }

    fn modal_create_channel(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let category = self
            .channel_category
            .or_else(|| self.selected.as_ref().map(|channel| channel.category_id))
            .or_else(|| self.layout.first().map(|category| category.category.id));
        let category_name = category
            .and_then(|id| self.layout.iter().find(|layout| layout.category.id == id))
            .map(|layout| format!("in {}", layout.category.name));
        let icon_h = self.channel_icon_picker_height();
        let body_h = 20.0 + 2.0 * 68.0 + 16.0 + 20.0 + INPUT_H + 16.0 + icon_h + 24.0;
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Create Channel",
            category_name.as_deref(),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        let semi = self.semibold_font;
        skin::field_label(ui, self.bold_font, body, "Channel type", false, None);
        let mut y = body[1] + 20.0;
        let options = [
            (
                false,
                "hash",
                "Text",
                "Send messages, images, GIFs, emoji, opinions, and puns",
            ),
            (
                true,
                "volume-2",
                "Voice",
                "Hang out together with voice, video, and screen share",
            ),
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
            skin::image(
                ui,
                self.icon(icon),
                [option_min[0] + 12.0, option_min[1] + 18.0],
                24.0,
                TEXT_SECONDARY,
            );
            skin::text(
                ui,
                semi,
                FONT_MD,
                [option_min[0] + 48.0, option_min[1] + 10.0],
                TEXT_NORMAL,
                title,
            );
            let line = skin::ellipsize(ui, None, FONT_SM, description, inner_w - 48.0 - 44.0);
            skin::text(
                ui,
                None,
                FONT_SM,
                [option_min[0] + 48.0, option_min[1] + 32.0],
                TEXT_SECONDARY,
                &line,
            );
            let radio = [option_max[0] - 22.0, option_min[1] + 30.0];
            ui.get_window_draw_list()
                .add_circle(radio, 10.0, if chosen { WHITE } else { TEXT_SECONDARY })
                .thickness(2.0)
                .build();
            if chosen {
                ui.get_window_draw_list()
                    .add_circle(radio, 5.0, WHITE)
                    .filled(true)
                    .build();
            }
            y += 68.0;
        }
        y += 8.0;
        skin::field_label(
            ui,
            self.bold_font,
            [body[0], y],
            "Channel name",
            false,
            None,
        );
        let field_min = [body[0], y + 20.0];
        skin::rect(
            ui,
            field_min,
            [field_min[0] + inner_w, field_min[1] + INPUT_H],
            INPUT_BG,
            INPUT_RADIUS,
        );
        let kind = if self.channel_is_voice {
            ChannelKind::Voice
        } else {
            ChannelKind::Text
        };
        let prefix = channel_display_icon(&kind, self.channel_icon.as_deref());
        skin::image(
            ui,
            self.icon(prefix),
            [field_min[0] + 10.0, field_min[1] + 12.0],
            16.0,
            TEXT_NORMAL,
        );
        self.focus_modal_field(ui);
        let enter = skin::input(
            ui,
            "##channel-name",
            &mut self.channel_name,
            "new-channel",
            [field_min[0] + 28.0, field_min[1]],
            inner_w - 28.0,
            INPUT_H,
            FONT_MD,
            false,
        );
        y = field_min[1] + INPUT_H + 16.0;
        self.draw_channel_icon_picker(ui, [body[0], y], inner_w, &kind);
        let enabled = !self.channel_name.trim().is_empty();
        let (cancel, create) = self.modal_footer(
            ui,
            max,
            min[0],
            "Cancel",
            "Create Channel",
            Tone::Brand,
            enabled,
        );
        if cancel {
            self.modal = None;
        } else if (create || enter) && enabled {
            if let (Place::Server(server_id), Some(category_id)) = (self.place, category) {
                let name = self.channel_name.trim().to_string();
                let icon = self.channel_icon.clone();
                if self
                    .note(self.client.create_channel(
                        server_id,
                        category_id,
                        &name,
                        kind,
                        icon.as_deref(),
                    ))
                    .is_some()
                {
                    self.channel_name.clear();
                    self.channel_icon = None;
                    self.modal = None;
                    self.reload_layout();
                }
            } else {
                self.status = "create a category first".into();
            }
        }
        (min, max)
    }

    fn modal_edit_channel(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let channel = self.channel_edit_id.and_then(|id| {
            self.layout
                .iter()
                .flat_map(|category| category.channels.iter())
                .find(|channel| channel.id == id)
                .cloned()
        });
        let kind = channel
            .as_ref()
            .map(|channel| channel.kind.clone())
            .unwrap_or(if self.channel_is_voice {
                ChannelKind::Voice
            } else {
                ChannelKind::Text
            });
        let subtitle = channel.as_ref().map(|channel| match &channel.kind {
            ChannelKind::Text => "Text channel".to_string(),
            ChannelKind::Voice => "Voice channel".to_string(),
            ChannelKind::Board => "Board channel".to_string(),
            ChannelKind::Plugin(kind) => kind.clone(),
        });
        let icon_h = self.channel_icon_picker_height();
        let body_h = 20.0 + INPUT_H + 16.0 + icon_h + 24.0;
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Edit Channel",
            subtitle.as_deref(),
            true,
            body_h,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Channel name", false, None);
        let field_min = [body[0], body[1] + 20.0];
        skin::rect(
            ui,
            field_min,
            [field_min[0] + inner_w, field_min[1] + INPUT_H],
            INPUT_BG,
            INPUT_RADIUS,
        );
        let prefix = channel_display_icon(&kind, self.channel_icon.as_deref());
        skin::image(
            ui,
            self.icon(prefix),
            [field_min[0] + 10.0, field_min[1] + 12.0],
            16.0,
            TEXT_NORMAL,
        );
        self.focus_modal_field(ui);
        let enter = skin::input(
            ui,
            "##edit-channel-name",
            &mut self.channel_name,
            "channel-name",
            [field_min[0] + 28.0, field_min[1]],
            inner_w - 28.0,
            INPUT_H,
            FONT_MD,
            false,
        );
        let picker_y = field_min[1] + INPUT_H + 16.0;
        self.draw_channel_icon_picker(ui, [body[0], picker_y], inner_w, &kind);
        let enabled = !self.channel_name.trim().is_empty() && self.channel_edit_id.is_some();
        let (cancel, save) = self.modal_footer(
            ui,
            max,
            min[0],
            "Cancel",
            "Save Changes",
            Tone::Brand,
            enabled,
        );
        if cancel {
            self.modal = None;
            self.channel_edit_id = None;
        } else if (save || enter) && enabled {
            if let (Place::Server(server_id), Some(channel_id)) = (self.place, self.channel_edit_id)
            {
                let name = self.channel_name.trim().to_string();
                let icon = self.channel_icon.clone();
                if let Some(updated) = self.note(self.client.update_channel(
                    server_id,
                    channel_id,
                    &name,
                    icon.as_deref(),
                )) {
                    if self
                        .selected
                        .as_ref()
                        .is_some_and(|channel| channel.id == channel_id)
                    {
                        self.selected = Some(updated);
                    }
                    self.channel_name.clear();
                    self.channel_icon = None;
                    self.channel_edit_id = None;
                    self.modal = None;
                    self.reload_layout();
                }
            }
        }
        (min, max)
    }

    fn modal_create_category(&mut self, ui: &Ui, display: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (body, min, max) = self.modal_frame(
            ui,
            display,
            "Create Category",
            None,
            true,
            20.0 + INPUT_H + 24.0,
            MODAL_FOOTER_H,
        );
        let inner_w = max[0] - min[0] - MODAL_PAD * 2.0;
        skin::field_label(ui, self.bold_font, body, "Category name", false, None);
        self.focus_modal_field(ui);
        let enter = skin::input(
            ui,
            "##category-name",
            &mut self.category_name,
            "New Category",
            [body[0], body[1] + 20.0],
            inner_w,
            INPUT_H,
            FONT_MD,
            false,
        );
        let enabled = !self.category_name.trim().is_empty();
        let (cancel, create) = self.modal_footer(
            ui,
            max,
            min[0],
            "Cancel",
            "Create Category",
            Tone::Brand,
            enabled,
        );
        if cancel {
            self.modal = None;
        } else if (create || enter) && enabled {
            if let Place::Server(server_id) = self.place {
                let name = self.category_name.trim().to_string();
                if self
                    .note(self.client.create_category(server_id, &name))
                    .is_some()
                {
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
            self.ptt_until =
                Some(Instant::now() + Duration::from_millis(self.voice.release_ms as u64));
        }
        let ptt_hot = held || self.ptt_until.is_some_and(|until| Instant::now() < until);
        let rms = self
            .media
            .as_ref()
            .map(|media| media.input_rms())
            .unwrap_or(0.0);
        let threshold = if self.voice.auto_sensitivity {
            OPEN_MIC_RMS
        } else {
            self.voice.sensitivity
        };
        let level_hot = speaking_from_level(self.push_to_talk, ptt_hot, rms, threshold);
        let in_place = self.voice_channel.is_some()
            || self
                .call
                .as_ref()
                .is_some_and(|call| call.phase == CallPhase::Active);
        if self
            .media
            .as_ref()
            .is_some_and(|media| media.take_capture_failure())
        {
            self.stop_streaming();
        }
        self.screen = self.share.is_live() && in_place;
        // voice_flags clears transmitting while muted/deafened (guild VC and DM calls).
        self.flags = self.mute.voice_flags(
            self.push_to_talk,
            level_hot,
            self.camera && in_place,
            self.screen,
        );
        if let Some(id) = self.account.as_ref().map(|account| account.id) {
            if let Some(member) = self
                .voice_members
                .iter_mut()
                .find(|member| member.user_id == id)
            {
                member.flags.transmitting = self.flags.transmitting;
            }
        }
        let preview = self.camera || self.video_popout || self.voice_settings_visible();
        if let Some(media) = &self.media {
            media.set_camera_choice(self.camera_choice.device_index(), preview);
            let pick = if self.share.is_portal() {
                Some(StreamPick::Portal)
            } else {
                self.share
                    .live_target()
                    .map(|(kind, id)| StreamPick::Source(kind, id))
            };
            media.set_stream_choice(
                pick,
                stream_max_edge(self.stream_resolution),
                stream_interval_ms(self.stream_fps),
            );
            media.apply_encode(self.encode_settings());
            media.note_publishing(self.flags.camera, self.flags.screen);
            media.apply_engine(&self.engine_update());
            media.set_mic_test(self.settings.mic_test());
            media.set_state(self.flags, in_place);
            let place = if let Some(channel_id) = self.voice_channel {
                Some(MediaPlace::VoiceChannel(channel_id))
            } else if self
                .call
                .as_ref()
                .is_some_and(|call| call.phase == CallPhase::Active)
            {
                self.call
                    .as_ref()
                    .map(|call| MediaPlace::DirectCall(call.dm_id))
            } else {
                None
            };
            let me = self.account.as_ref().map(|account| account.id).unwrap_or(0);
            let offerer = self.webrtc_offerer(me);
            let mesh = voice_rtc_mesh(
                matches!(place, Some(MediaPlace::DirectCall(_))),
                self.guild_p2p_video_active(),
            );
            // Do not offer WebRTC while reconnect is armed: the server seat is gone until
            // finish_reconnect HTTP-rejoins, and a premature offer is never retried.
            let rtc_ready = voice_rtc_session_ready(in_place, self.reconnect_at.is_some());
            media.set_session(place.filter(|_| rtc_ready), me, offerer, mesh);
        }
        if in_place && self.reconnect_at.is_none() && self.media.is_some() {
            self.ensure_remote_stream_subs();
        }
        self.sync_guild_p2p_video_pref();
        // Skip flag posts while the websocket is down — the server already cleared the seat.
        if in_place && self.reconnect_at.is_none() && self.flags != self.sent_flags {
            let now = Instant::now();
            if voice_flags_retry_ready(self.flags_failed_at, now, Duration::from_secs(2)) {
                match self.client.set_flags(self.flags) {
                    Ok(()) => {
                        self.sent_flags = self.flags;
                        self.flags_failed_at = None;
                    }
                    Err(err) => {
                        self.status = err.to_string();
                        self.flags_failed_at = Some(now);
                    }
                }
            }
        }
    }

    fn submit_composer(&mut self, target: ChatTarget) {
        let body = self.composer.clone();
        if self.consume_debug_command(&body) {
            return;
        }
        if let Some(id) = self.editing {
            if let Some(message) = self.note(self.client.edit_message(id, &body)) {
                upsert_message(&mut self.messages, message);
                self.composer.clear();
                self.editing = None;
            }
            return;
        }
        // Chat attachments are disabled (BA-61); never send pending file bytes.
        // Bottom bar always posts a top-level comment (Reddit main composer).
        if self.pending_file.take().is_some() {
            self.open_modal(Modal::UploadsUnavailable);
            return;
        }
        if let Some(message) = self.note(self.client.send_message(target, &body, None, &[])) {
            if self.search.trim().is_empty() {
                upsert_message(&mut self.messages, message);
            }
            self.chat_jump = true;
            self.composer.clear();
            self.pending_file = None;
        }
    }

    /// Send a reply nested under `reply_to` from the inline Reddit composer.
    fn submit_inline_reply(&mut self, target: ChatTarget) {
        let Some(parent) = self.reply_to else {
            return;
        };
        let body = self.reply_draft.clone();
        if self.consume_debug_command(&body) {
            return;
        }
        // Chat attachments are disabled (BA-61); never send pending file bytes.
        if self.pending_file.take().is_some() {
            self.open_modal(Modal::UploadsUnavailable);
            return;
        }
        if let Some(message) = self.note(self.client.send_message(target, &body, Some(parent), &[]))
        {
            if self.search.trim().is_empty() {
                upsert_message(&mut self.messages, message);
            }
            self.chat_jump = true;
            self.reply_draft.clear();
            self.reply_to = None;
            self.pending_file = None;
            // Expand the parent so the new reply is visible.
            self.collapsed_comments.remove(&parent);
        }
    }

    /// `/debugnow` opens the server debug window and is never stored as a message.
    fn consume_debug_command(&mut self, body: &str) -> bool {
        if !is_debug_command(&bfmd::plain_text(body)) {
            return false;
        }
        self.composer.clear();
        self.reply_draft.clear();
        self.editing = None;
        self.reply_to = None;
        self.pending_file = None;
        self.debug_open = true;
        self.debug_error.clear();
        self.refresh_server_debug();
        true
    }

    fn refresh_server_debug(&mut self) {
        match self.client.server_debug() {
            Ok(view) => {
                self.debug_view = Some(view);
                self.debug_error.clear();
            }
            Err(err) => self.debug_error = err.to_string(),
        }
        self.debug_fetched = Instant::now();
    }

    fn draw_server_debug(&mut self, ui: &Ui) {
        if !self.debug_open {
            return;
        }
        if self.debug_fetched.elapsed() >= Duration::from_secs(1) {
            self.refresh_server_debug();
        }
        let display = ui.io().display_size();
        let width = 720.0_f32.min(display[0] - 48.0).max(320.0);
        let height = 520.0_f32.min(display[1] - 48.0).max(240.0);
        let pos = [(display[0] - width) * 0.5, (display[1] - height) * 0.5];
        let _bg = ui.push_style_color(StyleColor::WindowBg, FLOATING_BG);
        let _border = ui.push_style_color(StyleColor::Border, DIVIDER);
        let _pad = ui.push_style_var(StyleVar::WindowPadding([16.0, 12.0]));
        let _round = ui.push_style_var(StyleVar::WindowRounding(MENU_RADIUS));
        let mut open = self.debug_open;
        ui.window("Server Debug")
            .flags(
                WindowFlags::NO_COLLAPSE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_DOCKING,
            )
            .position(pos, Condition::FirstUseEver)
            .size([width, height], Condition::FirstUseEver)
            .opened(&mut open)
            .build(|| {
                ui.text_colored(TEXT, "Server Debug");
                ui.same_line();
                if ui.small_button("Refresh") {
                    self.refresh_server_debug();
                }
                ui.same_line();
                if ui.small_button("Close") {
                    self.debug_open = false;
                }
                if !self.debug_error.is_empty() {
                    ui.text_colored(RED, &self.debug_error);
                }
                if let Some(view) = self.debug_view.clone() {
                    ui.separator();
                    ui.text_colored(TEXT_SECONDARY, "Stats");
                    let version = if view.version.is_empty() {
                        "unknown".to_string()
                    } else {
                        view.version.clone()
                    };
                    ui.text(format!("version {}    protocol {}", version, view.protocol));
                    ui.text(format!(
                        "uptime {}    rss {}",
                        format_uptime(view.uptime_ms),
                        crate::profiler::format_bytes(view.rss_bytes)
                    ));
                    ui.text(format!(
                        "online {}    idle {}    sockets {}",
                        view.online, view.idle, view.sockets
                    ));
                    ui.text(format!(
                        "voice members {}    voice channels {}    calls {}",
                        view.voice_members, view.voice_channels, view.calls
                    ));
                    ui.text(format!(
                        "media slots {}    forwards {}",
                        view.media_slots, view.media_forwards
                    ));
                    if view.udp_min > 0 && view.udp_max >= view.udp_min {
                        ui.text(format!(
                            "SFU UDP range {}–{}",
                            view.udp_min, view.udp_max
                        ));
                    } else {
                        ui.text_colored(MUTED, "SFU UDP range  (ephemeral / unset)");
                    }
                    if view.udp_ports.is_empty() {
                        ui.text_colored(MUTED, "SFU UDP in use  (none)");
                    } else {
                        let ports = view
                            .udp_ports
                            .iter()
                            .map(|port| port.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        ui.text(format!("SFU UDP in use  {ports}"));
                    }
                    ui.text(format!(
                        "users {}    servers {}    channels {}    messages {}",
                        view.users, view.servers, view.channels, view.messages
                    ));
                    ui.text(format!(
                        "db pool size {}    idle {}",
                        view.pool_size, view.pool_idle
                    ));
                    ui.separator();
                    ui.text_colored(TEXT_SECONDARY, "Network (server in / out)");
                    let traffic = &view.traffic;
                    ui.text(format!(
                        "HTTP  {}/s in  {}/s out  {} req/s",
                        crate::profiler::format_bytes(traffic.http_in_per_s),
                        crate::profiler::format_bytes(traffic.http_out_per_s),
                        traffic.http_reqs_per_s
                    ));
                    ui.text(format!(
                        "total  {} in  {} out  {} requests",
                        crate::profiler::format_bytes(traffic.http_in),
                        crate::profiler::format_bytes(traffic.http_out),
                        traffic.http_reqs
                    ));
                    ui.text(format!(
                        "WS  {}/s in  {}/s out",
                        crate::profiler::format_bytes(traffic.ws_in_per_s),
                        crate::profiler::format_bytes(traffic.ws_out_per_s)
                    ));
                    ui.text(format!(
                        "total  {} in  {} out  {} in-msg  {} out-msg",
                        crate::profiler::format_bytes(traffic.ws_in),
                        crate::profiler::format_bytes(traffic.ws_out),
                        traffic.ws_msgs_in,
                        traffic.ws_msgs_out
                    ));
                    ui.text(format!(
                        "Media  {}/s in  {}/s out",
                        crate::profiler::format_bytes(traffic.media_in_per_s),
                        crate::profiler::format_bytes(traffic.media_out_per_s)
                    ));
                    ui.text(format!(
                        "total  {} in  {} out  {} in-pkt  {} out-pkt",
                        crate::profiler::format_bytes(traffic.media_in),
                        crate::profiler::format_bytes(traffic.media_out),
                        traffic.media_packets_in,
                        traffic.media_packets_out
                    ));
                    ui.separator();
                    self.draw_debug_stream_section(ui);
                    ui.separator();
                    ui.text_colored(TEXT_SECONDARY, "Logs");
                    let _log_bg = ui.push_style_color(StyleColor::ChildBg, SIDEBAR_BG);
                    ui.child_window("server-debug-logs")
                        .size([0.0, 0.0])
                        .build(ui, || {
                            let stick = ui.scroll_max_y() <= 1.0
                                || ui.scroll_y() >= ui.scroll_max_y() - 8.0;
                            if view.logs.is_empty() {
                                ui.text_colored(MUTED, "No log lines yet.");
                            }
                            for line in &view.logs {
                                let color = match line.level.as_str() {
                                    "error" => RED,
                                    "warn" => YELLOW,
                                    _ => TEXT_SECONDARY,
                                };
                                ui.text_colored(
                                    color,
                                    format!("{}  {}", format_log_clock(line.at_ms), line.message),
                                );
                            }
                            if stick {
                                ui.set_scroll_here_y(1.0);
                            }
                        });
                } else if self.debug_error.is_empty() {
                    ui.text_colored(MUTED, "Loading...");
                }
            });
        self.debug_open = open;
    }

    /// Local Go Live / camera publish + any remote video tile Banter is decoding.
    fn draw_debug_stream_section(&self, ui: &Ui) {
        ui.text_colored(TEXT_SECONDARY, "Stream");
        let qos = profiler::qos();
        let local_live = self.flags.screen || self.flags.camera || qos.publishing_screen || qos.publishing_camera;
        let remote_live = !qos.remote_stream.is_empty()
            || self
                .voice_members
                .iter()
                .any(|member| member.flags.screen || member.flags.camera);
        if !local_live && !remote_live {
            ui.text_colored(MUTED, "No local or remote stream right now.");
            return;
        }
        if local_live {
            ui.text_colored(TEXT, "Local publish");
            let parts: Vec<&str> = [
                self.flags.screen.then_some("screen"),
                self.flags.camera.then_some("camera"),
            ]
            .into_iter()
            .flatten()
            .collect();
            ui.text(format!(
                "publishing {}    topology {}    ladder {}",
                if parts.is_empty() {
                    "idle".into()
                } else {
                    parts.join("+")
                },
                qos.topology,
                qos.ladder
            ));
            ui.text(format!(
                "encoder {} ({})    rate {}    keyframe {}s",
                qos.backend, qos.encoder_pref, qos.rate_control, qos.keyframe_secs
            ));
            if qos.encode_width > 0 && qos.encode_height > 0 {
                ui.text(format!(
                    "encode {}x{} @ {} FPS    target {} kbps",
                    qos.encode_width, qos.encode_height, qos.encode_fps, qos.target_kbps
                ));
            } else {
                ui.text(format!(
                    "target {} kbps    fps {}    (waiting for first frame)",
                    qos.target_kbps, qos.encode_fps
                ));
            }
            ui.text(format!(
                "measured  screen {} kbps   camera {} kbps   stream-audio {} kbps",
                qos.screen_kbps, qos.camera_kbps, qos.stream_kbps
            ));
            ui.text(format!(
                "encode {:.1} ms   keyframes {}   layers {}",
                qos.encode_ms, qos.keyframes, qos.layers
            ));
        }
        if remote_live {
            if local_live {
                settings_spacer(ui);
            }
            ui.text_colored(TEXT, "Remote / watching");
            if !qos.remote_stream.is_empty() {
                ui.text(&qos.remote_stream);
            }
            for member in &self.voice_members {
                if member.user_id == self.account.as_ref().map(|a| a.id).unwrap_or(-1) {
                    continue;
                }
                if !(member.flags.screen || member.flags.camera) {
                    continue;
                }
                let name = self.name_of(member.user_id);
                let kinds = [
                    member.flags.screen.then_some("screen"),
                    member.flags.camera.then_some("camera"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("+");
                ui.text(format!("{name}  ({kinds})"));
            }
            if qos.decode_ms > 0.0 {
                ui.text(format!("decode {:.1} ms", qos.decode_ms));
            }
            ui.text_colored(
                MUTED,
                "Remote encoder/bitrate is not signaled; numbers above are what this client receives.",
            );
        }
    }

    fn encode_settings(&self) -> media::EncodeSettings {
        let mode = encoder_mode_from_key(&self.stream.encoder);
        let rate = rate_control_from_key(&self.stream.rate_control);
        let fps = match self.stream_fps {
            StreamFps::Fps15 => 15,
            StreamFps::Fps30 => 30,
            StreamFps::Fps60 => 60,
            StreamFps::Fps120 => 120,
            StreamFps::Fps144 => 144,
        };
        media::EncodeSettings {
            full_bps: crate::logic::stream_full_bps_for(
                self.stream.bitrate_auto,
                self.stream.bitrate_kbps,
                self.stream_resolution,
                self.stream_fps,
            ),
            pick: encoder_mode_to_pick(mode),
            keyframe_ms: u64::from(self.stream.keyframe_secs.clamp(1, 10)) * 1000,
            fps,
            cbr: rate == StreamRateControl::Cbr,
            pref_label: encoder_mode_key(mode),
            rate_label: rate_control_key(rate),
        }
    }

    fn connect_live(&mut self) {
        let Some(token) = self.client.token().map(str::to_string) else {
            return;
        };
        let realtime = Realtime::connect(websocket_url(&self.base, &token));
        let media = Media::start(realtime.signal_sender());
        self.media = Some(media);
        self.realtime = Some(realtime);
        self.sent_guild_p2p_video = None;
        self.sync_guild_p2p_video_pref();
    }

    /// Drop the dead socket/media, keep local voice/call for UI, and schedule reconnect.
    fn begin_reconnect(&mut self, notice: String) {
        if self.pending_voice_rejoin.is_none() {
            let call = self.call.as_ref().map(|call| (call.dm_id, call.phase));
            self.pending_voice_rejoin = capture_voice_rejoin_intent(self.voice_channel, call);
        }
        self.media = None;
        self.realtime = None;
        self.reconnect_at = Some(next_reconnect_at(Instant::now()));
        self.status = if notice.is_empty() {
            RECONNECTING_STATUS.to_string()
        } else {
            format!("{RECONNECTING_STATUS} ({notice})")
        };
    }

    fn clear_reconnect_state(&mut self) {
        self.reconnect_at = None;
        self.pending_voice_rejoin = None;
    }

    /// Attempt a websocket reconnect when the scheduled wait has elapsed.
    fn tick_reconnect(&mut self) {
        if self.account.is_none() || self.client.token().is_none() {
            self.clear_reconnect_state();
            return;
        }
        if self.reconnect_at.is_none() {
            return;
        }
        if self.status.is_empty() || !self.status.starts_with(RECONNECTING_STATUS) {
            self.status = RECONNECTING_STATUS.to_string();
        }
        if self
            .realtime
            .as_ref()
            .is_some_and(|realtime| realtime.is_connected())
        {
            return;
        }
        if !reconnect_retry_ready(self.reconnect_at, Instant::now()) {
            return;
        }
        // Drop a hung handshake so the next connect can start cleanly.
        self.media = None;
        self.realtime = None;
        self.connect_live();
        // Handshake runs on the socket thread; failure/success is observed in `pump_events`.
        // Schedule the next attempt so a hung or failed connect still retries every 30s.
        self.reconnect_at = Some(next_reconnect_at(Instant::now()));
        self.status = RECONNECTING_STATUS.to_string();
    }

    /// Socket is live again — clear the banner and rejoin the prior voice seat or DM call.
    fn finish_reconnect(&mut self) {
        self.reconnect_at = None;
        if self.status.starts_with(RECONNECTING_STATUS) {
            self.status.clear();
        }
        // Drop any RTC session started while the seat was still cleared server-side
        // (connect_live recreates Media before HTTP rejoin). Same place would otherwise
        // dedupe and never re-offer after join_voice / start_call.
        if let Some(media) = &self.media {
            media.set_session(None, 0, false, false);
        }
        let Some(intent) = self.pending_voice_rejoin.take() else {
            return;
        };
        match intent {
            VoiceRejoinIntent::GuildChannel(channel_id) => {
                self.join_voice_channel(channel_id);
            }
            VoiceRejoinIntent::DirectCall(dm_id) => {
                // Server ended the call on disconnect; start a fresh ring to the same DM.
                self.call = None;
                self.start_call(dm_id);
            }
        }
    }

    fn logout(&mut self) {
        let _ = self.client.logout();
        let _ = clear_token(&self.data_dir);
        self.clear_reconnect_state();
        self.media = None;
        self.realtime = None;
        self.client.set_token(None);
        self.shell.invalidate();
        self.account = None;
        self.messages.clear();
        self.friends.clear();
        self.servers.clear();
        self.server_rail = ServerRail {
            folders: Vec::new(),
            servers: Vec::new(),
        };
        self.status.clear();
        self.settings.close();
        self.share_open = false;
        self.voice_channel = None;
        self.voice_members.clear();
        self.voice_rosters.clear();
        self.end_media_place();
    }

    fn refresh_shell(&mut self) {
        self.shell.request(FetchKind::refresh());
    }

    fn navigate_shell(&mut self, kind: FetchKind) {
        self.shell.bump();
        self.shell.request(kind);
    }

    fn pump_shell(&mut self) {
        let received = self.shell.try_recv();
        let restart = if received.is_some() {
            self.shell.finish_job()
        } else {
            false
        };
        if let Some(snap) = received {
            if snapshot_applies(snap.generation, self.shell.generation()) {
                self.apply_shell_snapshot(snap);
            }
        }
        if self.client.token().is_some()
            && self.account.is_some()
            && self.shell.poll_due(Instant::now())
        {
            self.shell.mark_polled(Instant::now());
            self.shell.queued.merge(FetchKind::refresh());
        }
        let start = (restart || !self.shell.in_flight())
            && self.shell.has_work()
            && self.client.token().is_some();
        if start {
            if !self.shell.in_flight() {
                let _ = self.shell.begin_job();
            }
            self.dispatch_shell();
        }
    }

    fn dispatch_shell(&mut self) {
        let kind = std::mem::take(&mut self.shell.queued);
        let search = {
            let trimmed = self.search.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let query = ShellQuery {
            generation: self.shell.generation(),
            base: self.base.clone(),
            token: self.client.token().unwrap_or("").to_string(),
            kind,
            server_id: match self.place {
                Place::Server(id) => Some(id),
                _ => None,
            },
            chat: self.chat_target(),
            search,
            avatars: self.shell.take_avatars(),
        };
        self.shell.submit(query);
    }

    fn apply_shell_snapshot(&mut self, snap: ShellSnapshot) {
        if let Some(Ok(account)) = snap.account {
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
                    self.shell.forget_avatar(id);
                }
            }
            self.account = Some(account);
        }
        if let Some(friends) = snap.friends {
            if let Some(friends) = self.note(friends) {
                self.friends = friends;
            }
        }
        if let Some(requests) = snap.requests {
            if let Some((incoming, outgoing)) = self.note(requests) {
                self.incoming = incoming;
                self.outgoing = outgoing;
            }
        }
        if let Some(servers) = snap.servers {
            if let Some(servers) = self.note(servers) {
                self.servers = servers;
            }
        }
        if let Some(rail) = snap.rail {
            if let Some(rail) = self.note(rail) {
                self.server_rail = rail;
                if !self.server_rail.servers.is_empty() {
                    self.servers = self
                        .server_rail
                        .servers
                        .iter()
                        .map(|row| row.server.clone())
                        .collect();
                }
            }
        }
        if let Some(dms) = snap.dms {
            if let Some(dms) = self.note(dms) {
                self.dms = dms;
            }
        }
        if let Some(mentions) = snap.mentions {
            if let Some(mentions) = self.note(mentions) {
                self.mentions = mentions;
            }
        }
        if let Some(rows) = snap.presence {
            if let Some(rows) = self.note(rows) {
                self.presence = rows
                    .into_iter()
                    .map(|(user_id, state, custom_status)| {
                        (
                            user_id,
                            PresenceInfo {
                                state,
                                custom_status,
                            },
                        )
                    })
                    .collect();
            }
        }
        if let Some(server_id) = snap.server_id {
            if self.place == Place::Server(server_id) {
                if let Some(layout) = snap.layout {
                    if let Some(layout) = self.note(layout) {
                        if let Some(selected) = &self.selected {
                            self.selected = layout
                                .iter()
                                .flat_map(|category| category.channels.iter())
                                .find(|channel| channel.id == selected.id)
                                .cloned();
                        }
                        self.layout = layout;
                    }
                }
                if self.selected.is_none() {
                    if let Some(channel) = snap.picked_channel {
                        if channel.server_id == server_id {
                            self.selected = Some(channel);
                        }
                    }
                }
                if let Some(members) = snap.members {
                    if let Some(members) = self.note(members) {
                        for member in &members {
                            self.presence.insert(
                                member.user_id,
                                PresenceInfo {
                                    state: member.presence,
                                    custom_status: member.custom_status.clone(),
                                },
                            );
                        }
                        self.members = members;
                        if self.modal != Some(Modal::Nickname) {
                            if let Some(member) = self.members.iter().find(|member| {
                                Some(member.user_id)
                                    == self.account.as_ref().map(|account| account.id)
                            }) {
                                self.nickname = member.nickname.clone().unwrap_or_default();
                            }
                        }
                    }
                }
                if let Some(rows) = snap.voice {
                    if let Some(rows) = self.note(rows) {
                        self.apply_voice_roster_rows(rows);
                    }
                }
            }
        }
        if let Some(messages) = snap.messages {
            if self.chat_target() == snap.chat {
                if let Some(messages) = self.note(messages) {
                    self.messages = messages;
                }
            }
        }
        for avatar in snap.avatars {
            self.shell
                .store_avatar(avatar.user_id, avatar.rev, avatar.bytes);
        }
    }

    fn apply_voice_roster_rows(&mut self, rows: Vec<banter_server::VoiceRoster>) {
        let mine: Vec<i64> = self
            .layout
            .iter()
            .flat_map(|category| category.channels.iter())
            .filter(|channel| channel.kind == ChannelKind::Voice)
            .map(|channel| channel.id)
            .collect();
        for channel_id in mine {
            self.voice_rosters.remove(&channel_id);
        }
        for row in rows {
            apply_voice_roster(&mut self.voice_rosters, row.channel_id, row.members);
        }
        if let Some(channel_id) = self.voice_channel {
            if let Some(members) = self.voice_rosters.get(&channel_id) {
                self.voice_members = members.clone();
            }
        }
    }

    fn open_server(&mut self, server_id: i64) {
        self.place = Place::Server(server_id);
        self.selected = None;
        self.messages.clear();
        self.search.clear();
        self.reply_to = None;
        self.reply_draft.clear();
        self.collapsed_comments.clear();
        self.editing = None;
        self.shown_invite.clear();
        self.layout.clear();
        self.members.clear();
        self.remember_home_recent(HomeRecentEntry {
            kind: HomeRecentKind::Server,
            id: server_id,
        });
        self.navigate_shell(FetchKind::open_server());
    }

    fn open_dm(&mut self, dm_id: i64) {
        self.place = Place::Direct(dm_id);
        self.selected = None;
        self.search.clear();
        self.reply_to = None;
        self.reply_draft.clear();
        self.collapsed_comments.clear();
        self.editing = None;
        self.messages.clear();
        self.remember_home_recent(HomeRecentEntry {
            kind: HomeRecentKind::DirectMessage,
            id: dm_id,
        });
        self.navigate_shell(FetchKind::messages());
    }

    fn remember_home_recent(&mut self, entry: HomeRecentEntry) {
        push_home_recent(&mut self.home_layout, entry);
        self.save_home_layout();
    }

    /// Opens the DM for a newly added friend when they are on the friends list.
    fn open_dm_after_adding_friend(&mut self, peer_username: &str) {
        let Some(peer_id) = crate::open_dm_peer_after_friend_add(
            self.friends
                .iter()
                .map(|friend| (friend.id, friend.username.as_str())),
            peer_username,
        ) else {
            return;
        };
        if let Some(chat) = self.note(self.client.open_dm(peer_id)) {
            self.refresh_shell();
            self.open_dm(chat.id);
        }
    }

    fn open_mention(&mut self, mention: &Mention) {
        if self
            .note(self.client.mark_mention_read(mention.id))
            .is_some()
        {
            self.mentions.retain(|item| item.id != mention.id);
        }
        if let Some(channel_id) = mention.channel_id {
            if let Some(channel) = self.find_channel(channel_id) {
                let server_id = channel.server_id;
                self.open_server(server_id);
                self.selected = Some(channel);
                self.navigate_shell(FetchKind::messages());
            }
        } else if let Some(dm_id) = mention.dm_id {
            self.open_dm(dm_id);
        }
    }

    fn reload_layout(&mut self) {
        self.navigate_shell(FetchKind::place());
    }

    fn reload_members(&mut self) {
        self.navigate_shell(FetchKind {
            place: true,
            ..FetchKind::default()
        });
    }

    fn can_permission(&self, flag: i64) -> bool {
        let Place::Server(server_id) = self.place else {
            return false;
        };
        let Some(me) = self.account.as_ref().map(|account| account.id) else {
            return false;
        };
        if self
            .servers
            .iter()
            .any(|server| server.id == server_id && server.owner_id == me)
        {
            return true;
        }
        let Some(member) = self.members.iter().find(|member| member.user_id == me) else {
            return false;
        };
        let bits = member
            .roles
            .iter()
            .fold(0, |acc, role| acc | role.permissions);
        has_permission(bits, flag)
    }

    fn reload_role_list(&mut self) {
        let Place::Server(server_id) = self.place else {
            return;
        };
        if let Ok(roles) = self.client.roles(server_id) {
            self.guild_roles = roles;
        }
    }

    fn open_roles(&mut self) {
        self.role_error.clear();
        self.reload_role_list();
        self.clear_role_form();
        self.open_modal(Modal::Roles);
    }

    fn open_member_roles(&mut self, user_id: i64) {
        self.reload_role_list();
        self.assign_user = Some(user_id);
        self.assign_role_ids = self
            .members
            .iter()
            .find(|member| member.user_id == user_id)
            .map(|member| {
                member
                    .roles
                    .iter()
                    .filter(|role| !role.everyone)
                    .map(|role| role.id)
                    .collect()
            })
            .unwrap_or_default();
        self.role_error.clear();
        self.open_modal(Modal::MemberRoles);
    }

    fn clear_role_form(&mut self) {
        self.editing_role = None;
        self.role_name.clear();
        self.role_color.clear();
        self.role_hoist = false;
        self.role_admin = false;
        self.role_channels = false;
        self.role_manage = false;
        self.role_invite = false;
        self.role_plugins = false;
        self.role_bots = false;
        self.role_error.clear();
    }

    fn load_role_form(&mut self, role: &GuildRole) {
        self.editing_role = Some(role.id);
        self.role_name = role.name.clone();
        self.role_color = if role.color == 0 {
            String::new()
        } else {
            format!("{:06X}", role.color)
        };
        self.role_hoist = role.hoist;
        self.role_admin = role.permissions & PERM_ADMINISTRATOR != 0;
        self.role_channels = role.permissions & PERM_MANAGE_CHANNELS != 0;
        self.role_manage = role.permissions & PERM_MANAGE_ROLES != 0;
        self.role_invite = role.permissions & PERM_CREATE_INVITE != 0;
        self.role_plugins = role.permissions & PERM_MANAGE_PLUGINS != 0;
        self.role_bots = role.permissions & PERM_MANAGE_BOTS != 0;
        self.role_error.clear();
    }

    fn role_form_bits(&self) -> i64 {
        let mut bits = 0;
        if self.role_invite {
            bits |= PERM_CREATE_INVITE;
        }
        if self.role_channels {
            bits |= PERM_MANAGE_CHANNELS;
        }
        if self.role_manage {
            bits |= PERM_MANAGE_ROLES;
        }
        if self.role_plugins {
            bits |= PERM_MANAGE_PLUGINS;
        }
        if self.role_bots {
            bits |= PERM_MANAGE_BOTS;
        }
        if self.role_admin {
            bits |= PERM_ADMINISTRATOR;
        }
        bits
    }

    fn submit_role(&mut self, everyone: bool) {
        let Place::Server(server_id) = self.place else {
            return;
        };
        let color = match parse_role_color(&self.role_color) {
            Ok(color) => color,
            Err(err) => {
                self.role_error = err;
                return;
            }
        };
        let name = if everyone {
            "@everyone".to_string()
        } else {
            self.role_name.trim().to_string()
        };
        if name.is_empty() {
            self.role_error = "name is required".into();
            return;
        }
        let bits = self.role_form_bits();
        let hoist = self.role_hoist && !everyone;
        let result = if let Some(role_id) = self.editing_role {
            self.client
                .update_role(server_id, role_id, &name, color, hoist, bits)
        } else {
            self.client
                .create_role(server_id, &name, color, hoist, bits)
        };
        match result {
            Ok(role) => {
                self.role_error.clear();
                self.reload_role_list();
                self.reload_members();
                self.load_role_form(&role);
            }
            Err(err) => self.role_error = err.to_string(),
        }
    }

    fn delete_editing_role(&mut self) {
        let Place::Server(server_id) = self.place else {
            return;
        };
        let Some(role_id) = self.editing_role else {
            return;
        };
        match self.client.delete_role(server_id, role_id) {
            Ok(()) => {
                self.clear_role_form();
                self.reload_role_list();
                self.reload_members();
            }
            Err(err) => self.role_error = err.to_string(),
        }
    }

    fn chat_target(&self) -> Option<ChatTarget> {
        match self.place {
            Place::Direct(id) => Some(ChatTarget::Direct(id)),
            Place::Server(_) => self
                .selected
                .as_ref()
                .filter(|channel| channel.kind == ChannelKind::Text)
                .map(|channel| ChatTarget::Channel(channel.id)),
            Place::Home => None,
        }
    }

    fn voice_selected(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|channel| channel.kind.is_voice())
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
        self.layout
            .iter()
            .flat_map(|category| category.channels.iter())
            .find(|channel| channel.id == channel_id)
            .cloned()
    }

    fn pump_events(&mut self) {
        self.tick_reconnect();
        let Some(realtime) = &self.realtime else {
            return;
        };
        let events = realtime.take_events();
        if let Some(error) = realtime.take_error() {
            self.begin_reconnect(error);
            return;
        }
        let recovering = self.reconnect_at.is_some() || self.pending_voice_rejoin.is_some();
        if recovering && realtime.is_connected() {
            self.finish_reconnect();
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
            ServerEvent::Presence {
                user_id,
                state,
                custom_status,
            } => {
                self.presence.insert(
                    user_id,
                    PresenceInfo {
                        state,
                        custom_status: custom_status.clone(),
                    },
                );
                if let Some(member) = self
                    .members
                    .iter_mut()
                    .find(|member| member.user_id == user_id)
                {
                    member.presence = state;
                    member.custom_status = custom_status;
                }
            }
            ServerEvent::MessageCreated { target, message }
            | ServerEvent::MessageEdited { target, message } => {
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
            ServerEvent::Voice {
                channel_id,
                members,
            } => {
                apply_voice_roster(&mut self.voice_rosters, channel_id, members.clone());
                if self.voice_channel == Some(channel_id) {
                    self.voice_members = members;
                }
            }
            ServerEvent::Call { call } => {
                let me = self.account.as_ref().map(|account| account.id);
                if Some(call.caller) == me || Some(call.callee) == me {
                    // Rebroadcasts carry live Go Live / camera flags during an active call.
                    self.apply_call_snapshot(call, true);
                }
            }
            ServerEvent::CallEnded { dm_id } => {
                if self.call.as_ref().map(|call| call.dm_id) == Some(dm_id) {
                    let was_active = self
                        .call
                        .as_ref()
                        .is_some_and(|call| call.phase == CallPhase::Active);
                    self.stop_ringing();
                    if was_active {
                        self.sfx.play(SoundEvent::Leave);
                    }
                    // Peer hang-up / remote end: same cleanup as Disconnect (drops stream overlays).
                    self.end_media_place();
                }
            }
            ServerEvent::RtcOffer { .. }
            | ServerEvent::RtcAnswer { .. }
            | ServerEvent::RtcIce { .. }
            | ServerEvent::SubscribeLayer { .. }
            | ServerEvent::WantedLayers { .. }
            | ServerEvent::BoardChanged { .. }
            | ServerEvent::Ready { .. }
            | ServerEvent::PluginSync { .. } => {}
        }
    }

    fn install_video(&mut self, imgui: &mut Context, frame: VideoFrame) {
        if let Some(sender) = frame.sender {
            self.delivered.insert((sender, frame.kind), frame.layer);
        }
        let shot = if let Some((width, height, rgba)) = frame.rgba.as_ref() {
            if frame.sender.is_none()
                && frame.kind == MediaKind::Camera
                && camera_preview_mirrored()
            {
                mirrored_camera_rgba(imgui, *width, *height, rgba)
            } else {
                upload_rgba(imgui, *width, *height, rgba)
            }
        } else if !frame.jpeg.is_empty() {
            upload_jpeg(imgui, &frame.jpeg)
        } else {
            None
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
        if let Some((user_id, rev, bytes)) = self.pending_banner.take() {
            if let Some(shot) = self.banners.remove(&user_id) {
                self.retired_shots.push(shot);
            }
            if let Some(shot) =
                upload_jpeg(imgui, &bytes).or_else(|| upload_rgba_png(imgui, &bytes))
            {
                self.banners.insert(user_id, shot);
                self.banner_rev.insert(user_id, rev);
                self.banner_miss.remove(&user_id);
            } else {
                self.banner_miss.insert(user_id, rev);
            }
        }
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
        let mut noted = false;
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
            if self.avatar_miss.get(&user_id) == Some(&rev) {
                continue;
            }
            let step = match self.shell.avatar_file(user_id) {
                Some(AvatarFile::Ready { rev: got, bytes }) if *got == rev => {
                    AvatarStep::Upload(bytes.clone())
                }
                Some(AvatarFile::Failed { rev: got }) if *got == rev => AvatarStep::Miss,
                _ => AvatarStep::Want,
            };
            match step {
                AvatarStep::Upload(bytes) => {
                    if let Some(shot) = self.avatars.remove(&user_id) {
                        self.retired_shots.push(shot);
                    }
                    if let Some(shot) =
                        upload_jpeg(imgui, &bytes).or_else(|| upload_rgba_png(imgui, &bytes))
                    {
                        self.avatars.insert(user_id, shot);
                        self.avatar_rev.insert(user_id, rev);
                        self.avatar_miss.remove(&user_id);
                    } else {
                        self.avatar_miss.insert(user_id, rev);
                    }
                }
                AvatarStep::Miss => {
                    self.avatar_miss.insert(user_id, rev);
                }
                AvatarStep::Want => {
                    if self.shell.note_want(AvatarWant { user_id, rev }) {
                        noted = true;
                    }
                }
            }
        }
        if noted && self.shell.in_flight() {
            let _ = self.shell.begin_job();
        }
        self.pump_shell();
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
        let rgba = crop_avatar_rgba(
            self.crop_size[0],
            self.crop_size[1],
            &self.crop_rgba,
            self.crop_zoom,
            self.crop_pan[0],
            self.crop_pan[1],
            edge,
        );
        let Some(png) = encode_png(edge, edge, &rgba) else {
            self.status = "could not save that picture".into();
            return;
        };
        if png.len() > MAX_AVATAR_BYTES {
            self.status = "avatar is larger than 2 MB".into();
            return;
        }
        if self.profile.customizing() {
            self.profile_draft_avatar = ProfileMediaDraft::Bytes(png);
            self.crop_open = false;
            self.status.clear();
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
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING;
        ui.window("Avatar crop")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .bg_alpha(0.72)
            .build(|| {
                let panel = [420.0, 460.0];
                let origin = [(size[0] - panel[0]) * 0.5, (size[1] - panel[1]) * 0.5];
                ui.get_window_draw_list()
                    .add_rect(
                        origin,
                        [origin[0] + panel[0], origin[1] + panel[1]],
                        STAGE_BG,
                    )
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
                let drag = pointer_in(
                    ui,
                    [center[0] - 96.0, center[1] - 96.0],
                    [center[0] + 96.0, center[1] + 96.0],
                );
                if drag && ui.is_mouse_down(MouseButton::Left) {
                    let mouse = ui.io().mouse_pos();
                    if let Some(previous) = self.crop_drag {
                        let window = crop_window(
                            self.crop_size[0],
                            self.crop_size[1],
                            self.crop_zoom,
                            0.0,
                            0.0,
                        )
                        .map(|(_, _, side)| side)
                        .unwrap_or(1.0);
                        let scale = window / 192.0;
                        self.crop_pan[0] = (self.crop_pan[0]
                            - (mouse[0] - previous[0]) * scale
                                / (self.crop_size[0] as f32).max(1.0))
                        .clamp(-1.0, 1.0);
                        self.crop_pan[1] = (self.crop_pan[1]
                            - (mouse[1] - previous[1]) * scale
                                / (self.crop_size[1] as f32).max(1.0))
                        .clamp(-1.0, 1.0);
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

    /// Composer paperclip / attach entry point. Chat uploads are gated off (BA-61).
    fn pick_attachment(&mut self) {
        self.pending_file = None;
        self.open_modal(Modal::UploadsUnavailable);
    }

    fn save_attachment(&mut self, id: i64, filename: &str) {
        let Some(bytes) = self.note(self.client.attachment(id)) else {
            return;
        };
        let Some(path) = rfd::FileDialog::new().set_file_name(filename).save_file() else {
            return;
        };
        if let Err(err) = std::fs::write(path, bytes) {
            self.status = err.to_string();
        }
    }

    fn name_of(&self, user_id: i64) -> String {
        if let Some(member) = self.members.iter().find(|member| member.user_id == user_id) {
            return member
                .nickname
                .clone()
                .unwrap_or_else(|| member.display_name.clone());
        }
        if let Some(account) = self
            .account
            .as_ref()
            .filter(|account| account.id == user_id)
        {
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
        {
            let draw = ui.get_window_draw_list();
            if active || ui.is_item_hovered() {
                let fill = if active {
                    [88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, 0.45]
                } else {
                    ROW_HOVER
                };
                draw.add_rect(min, [min[0] + size, min[1] + size], fill)
                    .rounding(6.0)
                    .filled(true)
                    .build();
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
        }
        if let Some(tip) = tip {
            if ui.is_item_hovered() {
                skin::item_tooltip(ui, self.semibold_font, tip);
            }
        }
        clicked
    }

    /// Reddit-style circular collapse control: (−) expanded, (+) collapsed.
    fn comment_fold_control(
        &self,
        ui: &Ui,
        message_id: i64,
        min: [f32; 2],
        collapsed: bool,
        tip: &str,
    ) -> bool {
        let size = COMMENT_FOLD_HIT;
        ui.set_cursor_screen_pos(min);
        let clicked = ui.invisible_button(format!("fold-{message_id}"), [size, size]);
        let hovered = ui.is_item_hovered();
        let icon = if collapsed { "plus" } else { "minus" };
        {
            let draw = ui.get_window_draw_list();
            let center = [min[0] + size * 0.5, min[1] + size * 0.5];
            let rim = if hovered {
                COMMENT_THREAD_HOVER
            } else {
                COMMENT_THREAD_LINE
            };
            // Subtle fill so the circle reads on dark chat without looking loud.
            draw.add_circle(center, COMMENT_FOLD_RADIUS, with_alpha(rim, 0.18))
                .filled(true)
                .build();
            draw.add_circle(center, COMMENT_FOLD_RADIUS, rim)
                .thickness(1.0)
                .build();
            if let Some(texture) = self.icon(icon) {
                let pad = size * 0.28;
                let tint = if hovered { COMMENT_THREAD_HOVER } else { MUTED };
                draw.add_image(
                    texture,
                    [min[0] + pad, min[1] + pad],
                    [min[0] + size - pad, min[1] + size - pad],
                    [0.0, 0.0],
                    [1.0, 1.0],
                    tint,
                );
            }
        }
        if hovered {
            skin::item_tooltip(ui, self.semibold_font, tip);
        }
        clicked
    }

    fn open_settings(&mut self) {
        self.open_settings_section(self.settings.section());
    }

    fn open_settings_section(&mut self, section: SettingsSection) {
        self.settings.open_section(section);
        self.settings_error.clear();
        self.display_name = self
            .account
            .as_ref()
            .map(|account| account.display_name.clone())
            .unwrap_or_default();
        if section == SettingsSection::Account {
            if let Some(id) = self.account.as_ref().map(|account| account.id) {
                if let Ok(profile) = self.client.user_profile(id, None) {
                    self.about_draft = profile.about_me.unwrap_or_default();
                    self.status_draft = profile.custom_status.unwrap_or_default();
                }
            }
        }
        if section == SettingsSection::Voice {
            self.devices_ready = false;
        }
        if !self.cameras_loaded {
            self.refresh_cameras();
        }
    }

    fn close_profile(&mut self) {
        self.profile.close();
        self.profile_view = None;
        self.profile_cosmetics.clear();
        self.profile_badges_catalog.clear();
        self.profile_widget_drafts.clear();
        self.profile_connection_drafts.clear();
        self.profile_badge_drafts.clear();
        self.profile_activity_drafts.clear();
        self.profile_wishlist_drafts.clear();
        self.profile_draft_banner = ProfileMediaDraft::Unchanged;
        self.profile_draft_avatar = ProfileMediaDraft::Unchanged;
        self.profile_editor_slot = None;
        self.profile_frames = 0;
    }

    fn profile_kind_for(&self, user_id: i64) -> ProfileKind {
        if self
            .account
            .as_ref()
            .is_some_and(|account| account.id == user_id)
        {
            return ProfileKind::SelfAccount;
        }
        let is_friend = self.friends.iter().any(|friend| friend.id == user_id)
            || self.dms.iter().any(|chat| chat.peer_id == user_id);
        if matches!(self.place, Place::Server(_))
            || self.members.iter().any(|member| member.user_id == user_id)
        {
            ProfileKind::ServerMember { is_friend }
        } else if is_friend {
            ProfileKind::Friend
        } else {
            ProfileKind::ServerMember { is_friend: false }
        }
    }

    fn open_profile(&mut self, user_id: i64, anchor: [f32; 2]) {
        let kind = self.profile_kind_for(user_id);
        let server_id = match self.place {
            Place::Server(id) => Some(id),
            _ => None,
        };
        match self.client.user_profile(user_id, server_id) {
            Ok(view) => {
                self.profile.open(user_id, kind);
                self.profile.set_anchor(anchor);
                self.profile.set_server_id(server_id);
                self.profile_view = Some(view);
                self.profile_frames = 0;
            }
            Err(err) => {
                self.status = err.to_string();
                self.close_profile();
            }
        }
    }

    fn draw_profile_card(&mut self, ui: &Ui) {
        if !self.profile.is_open() {
            return;
        }
        let Some(view) = self.profile_view.clone() else {
            self.close_profile();
            return;
        };
        self.ensure_profile_banner(ui, &view);
        if self.profile.mode() == ProfileMode::Customize {
            self.draw_profile_customize(ui, &view);
        } else {
            self.draw_profile_view(ui, &view);
        }
    }

    fn draw_profile_view(&mut self, ui: &Ui, view: &UserProfile) {
        let kind = self.profile.kind().unwrap_or(ProfileKind::Friend);
        let display = ui.io().display_size();
        let layout = profile_view_layout(display, self.profile.anchor());
        let mut close_after = false;
        let mut open_settings = false;
        let mut message_user = None;
        let mut add_friend = None;
        let mut begin_customize = false;

        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("##profile-view")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .build(|| {
                {
                    let draw = ui.get_window_draw_list();
                    draw.add_rect(layout.card.min, layout.card.max, FLOATING_BG)
                        .rounding(PROFILE_RADIUS)
                        .filled(true)
                        .build();
                }

                let theme_primary = view.theme_primary;
                let theme_secondary = view.theme_secondary;
                self.paint_profile_banner(
                    ui,
                    layout.identity.min,
                    [
                        layout.identity.max[0],
                        layout.identity.min[1] + PROFILE_BANNER_H,
                    ],
                    view,
                    theme_primary,
                    theme_secondary,
                );

                let heading = profile_heading(&view.display_name, view.nickname.as_deref());
                let avatar_center = [
                    layout.identity.min[0] + PROFILE_PAD + PROFILE_AVATAR,
                    layout.identity.min[1] + PROFILE_BANNER_H,
                ];
                {
                    let draw = ui.get_window_draw_list();
                    draw.add_circle(avatar_center, PROFILE_AVATAR + 6.0, FLOATING_BG)
                        .filled(true)
                        .build();
                }
                profile_paint::paint_profile_frame_art(
                    ui,
                    avatar_center,
                    PROFILE_AVATAR,
                    view.profile_frame_id.as_deref(),
                );
                let avatar = self.avatars.get(&view.id).map(|shot| shot.texture);
                paint_avatar(
                    ui,
                    avatar,
                    avatar_center,
                    PROFILE_AVATAR,
                    &initials(&heading),
                    None,
                );
                profile_paint::paint_avatar_decoration_art(
                    ui,
                    avatar_center,
                    PROFILE_AVATAR,
                    view.avatar_decoration_id.as_deref(),
                );
                let presence = self
                    .presence
                    .get(&view.id)
                    .map(|p| p.state)
                    .unwrap_or(Presence::Offline);
                paint_status_dot(ui, avatar_center, PROFILE_AVATAR, presence, FLOATING_BG);

                if let Some(status) = view
                    .custom_status
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let bubble_min = [
                        avatar_center[0] + PROFILE_AVATAR + 10.0,
                        avatar_center[1] - 18.0,
                    ];
                    let size = ui.calc_text_size(status);
                    let bubble_max = [
                        bubble_min[0] + size[0] + 16.0,
                        bubble_min[1] + size[1] + 10.0,
                    ];
                    let draw = ui.get_window_draw_list();
                    draw.add_rect(bubble_min, bubble_max, STAGE_BG)
                        .rounding(8.0)
                        .filled(true)
                        .build();
                    draw.add_text([bubble_min[0] + 8.0, bubble_min[1] + 5.0], TEXT, status);
                }

                let text_x = layout.identity.min[0] + PROFILE_PAD;
                let name_y = layout.identity.min[1] + PROFILE_BANNER_H + PROFILE_AVATAR + 14.0;
                let name_color = display_name_style_color(view.display_name_style_id.as_deref());
                {
                    let _font = ui.push_font_with_size(self.bold_font, FONT_LG);
                    ui.get_window_draw_list()
                        .add_text([text_x, name_y], name_color, &heading);
                }
                let mut meta = format!("@{}", view.username);
                if let Some(pronouns) = view
                    .pronouns
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    meta.push_str(" • ");
                    meta.push_str(pronouns);
                }
                ui.get_window_draw_list()
                    .add_text([text_x, name_y + 26.0], TEXT_SECONDARY, &meta);
                let mut y = name_y + 52.0;
                if !view.badges.is_empty() {
                    let cap = profile_badge_row_visible_cap();
                    let mut bx = text_x;
                    for badge in view.badges.iter().take(cap) {
                        profile_paint::paint_badge_icon(
                            ui,
                            [bx + PROFILE_BADGE, y + PROFILE_BADGE],
                            badge,
                        );
                        bx += PROFILE_BADGE * 2.0 + 6.0;
                    }
                    if view.badges.len() > cap {
                        let extra = view.badges.len() - cap;
                        ui.get_window_draw_list().add_text(
                            [bx, y + 4.0],
                            MUTED,
                            &format!("+{extra}"),
                        );
                    }
                    y += PROFILE_BADGE * 2.0 + 10.0;
                }
                ui.get_window_draw_list()
                    .add_text([text_x, y], MUTED, presence_label(presence));
                y += 24.0;

                let body_min = [layout.identity.min[0], y];
                let body_size = [
                    layout.identity.width(),
                    (layout.identity.max[1] - body_min[1]).max(0.0),
                ];
                let child_bg = ui.push_style_color(StyleColor::ChildBg, FLOATING_BG);
                let child_pad = ui.push_style_var(StyleVar::WindowPadding([PROFILE_PAD, 8.0]));
                let child_gap = ui.push_style_var(StyleVar::ItemSpacing([8.0, 6.0]));
                ui.set_cursor_screen_pos(body_min);
                ui.child_window("profile-view-identity")
                    .size(body_size)
                    .build(ui, || {
                        for action in profile_primary_actions(kind) {
                            let (id, label) = match action {
                                ProfileAction::EditProfile => ("profile-edit", "Edit Profile"),
                                ProfileAction::Message => ("profile-message", "Message"),
                                ProfileAction::AddFriend => ("profile-add-friend", "Add Friend"),
                            };
                            let btn_w = (ui.content_region_avail()[0]).max(80.0);
                            let origin = ui.cursor_screen_pos();
                            if skin::button(
                                ui,
                                id,
                                label,
                                Tone::Brand,
                                origin,
                                [btn_w, BUTTON_H],
                                true,
                                self.semibold_font,
                            ) {
                                match action {
                                    ProfileAction::EditProfile => begin_customize = true,
                                    ProfileAction::Message => message_user = Some(view.id),
                                    ProfileAction::AddFriend => {
                                        add_friend = Some(view.username.clone())
                                    }
                                }
                            }
                            ui.dummy([0.0, BUTTON_H + 4.0]);
                        }
                        ui.separator();
                        if let Some(about) = view
                            .about_me
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            ui.text_colored(MUTED, "ABOUT ME");
                            let _wrap = ui.push_text_wrap_pos(
                                ui.cursor_screen_pos()[0] + ui.content_region_avail()[0],
                            );
                            ui.text_colored(TEXT, about);
                            ui.dummy([0.0, 6.0]);
                        }
                        ui.text_colored(MUTED, "MEMBER SINCE");
                        ui.text_colored(TEXT, &format_member_since(view.member_since));
                        ui.dummy([0.0, 6.0]);
                        if !view.roles.is_empty() {
                            ui.text_colored(MUTED, "ROLES");
                            for role in &view.roles {
                                let color = if role.color > 0 {
                                    rgb_u32_to_rgba(role.color as u32)
                                } else {
                                    MUTED
                                };
                                ui.text_colored(color, &format!("● {}", role.name));
                            }
                            ui.dummy([0.0, 6.0]);
                        }
                        if !view.connections.is_empty() {
                            ui.text_colored(MUTED, "CONNECTIONS");
                            for connection in &view.connections {
                                self.draw_profile_connection_row(ui, connection);
                            }
                        }
                        if kind == ProfileKind::SelfAccount {
                            ui.dummy([0.0, 4.0]);
                            if ui.small_button("Open My Account") {
                                open_settings = true;
                            }
                        }
                    });
                drop(child_gap);
                drop(child_pad);
                drop(child_bg);

                let board_bg = ui.push_style_color(StyleColor::ChildBg, STAGE_BG);
                let board_pad = ui.push_style_var(StyleVar::WindowPadding([
                    PROFILE_BOARD_CARD_PAD,
                    PROFILE_BOARD_CARD_PAD,
                ]));
                let board_gap = ui.push_style_var(StyleVar::ItemSpacing([8.0, 8.0]));
                ui.set_cursor_screen_pos(layout.board.min);
                ui.child_window("profile-view-board")
                    .size(layout.board.size())
                    .build(ui, || {
                        self.draw_profile_board_contents(ui, layout.board, view, false);
                    });
                drop(board_gap);
                drop(board_pad);
                drop(board_bg);

                let over_card = ui.is_mouse_hovering_rect(layout.card.min, layout.card.max);
                if self.profile_frames > 0 && ui.is_mouse_clicked(MouseButton::Left) && !over_card {
                    close_after = true;
                }
            });
        self.profile_frames = self.profile_frames.saturating_add(1);

        if begin_customize {
            self.begin_profile_customize(view);
        }
        if open_settings {
            self.close_profile();
            self.open_settings_section(SettingsSection::Account);
        }
        if let Some(user_id) = message_user {
            if let Some(chat) = self.note(self.client.open_dm(user_id)) {
                self.refresh_shell();
                self.open_dm(chat.id);
            }
            self.close_profile();
        }
        if let Some(username) = add_friend {
            if self
                .note(self.client.send_friend_request(&username))
                .is_some()
            {
                self.refresh_shell();
                self.status = format!("Friend request sent to @{username}");
            }
            self.close_profile();
        }
        if close_after {
            self.close_profile();
        }
    }

    fn draw_profile_scope_switch(&mut self, ui: &Ui, left_w: f32) {
        let current = self.profile.edit_scope().label();
        ui.set_next_item_width(left_w - 8.0);
        if let Some(_token) = ui.begin_combo("##profile-scope", &current) {
            let mut selected = None;
            if ui
                .selectable_config("Main Profile")
                .selected(self.profile.edit_scope().is_main())
                .build()
            {
                selected = Some(ProfileEditScope::Main);
            }
            for server in self.servers.clone() {
                let is_sel = self.profile.edit_scope().server_id() == Some(server.id);
                if ui.selectable_config(&server.name).selected(is_sel).build() {
                    selected = Some(ProfileEditScope::Server {
                        id: server.id,
                        name: server.name.clone(),
                    });
                }
            }
            if let Some(scope) = selected {
                self.apply_profile_edit_scope(scope);
            }
        }
    }

    fn apply_profile_edit_scope(&mut self, scope: ProfileEditScope) {
        self.profile.set_edit_scope(scope.clone());
        match scope {
            ProfileEditScope::Main => {
                if let Some(view) = self.profile_view.clone() {
                    self.profile
                        .set_draft_nickname(view.nickname.clone().unwrap_or_default());
                }
            }
            ProfileEditScope::Server { id, .. } => {
                if let Some(account) = self.account.clone() {
                    if let Ok(view) = self.client.user_profile(account.id, Some(id)) {
                        self.profile
                            .set_draft_nickname(view.nickname.unwrap_or_default());
                    }
                }
            }
        }
    }

    fn begin_profile_customize(&mut self, view: &UserProfile) {
        self.profile.begin_customize_from(view);
        self.profile_widget_drafts = view
            .widgets
            .iter()
            .map(|widget| ProfileWidgetInput {
                kind: widget.kind,
                title: widget.title.clone(),
                payload: widget.payload.clone(),
            })
            .collect();
        self.profile_connection_drafts = view
            .connections
            .iter()
            .map(|connection| ProfileConnectionInput {
                kind: connection.kind.clone(),
                name: connection.name.clone(),
                url: connection.url.clone(),
            })
            .collect();
        self.profile_badge_drafts = view.badges.iter().map(|badge| badge.id.clone()).collect();
        self.profile_activity_drafts = view
            .activity
            .iter()
            .map(|entry| ProfileActivityInput {
                kind: entry.kind.clone(),
                title: entry.title.clone(),
                detail: entry.detail.clone(),
            })
            .collect();
        self.profile_wishlist_drafts = view
            .wishlist
            .iter()
            .map(|entry| ProfileWishlistInput {
                title: entry.title.clone(),
                tag: entry.tag.clone(),
            })
            .collect();
        self.profile_draft_banner = ProfileMediaDraft::Unchanged;
        self.profile_draft_avatar = ProfileMediaDraft::Unchanged;
        self.profile_editor_slot = None;
        if self.profile_cosmetics.is_empty() {
            if let Ok(items) = self.client.profile_cosmetics() {
                self.profile_cosmetics = items;
            }
        }
        if self.profile_badges_catalog.is_empty() {
            if let Ok(items) = self.client.profile_badges() {
                self.profile_badges_catalog = items;
            }
        }
    }

    fn draw_profile_customize(&mut self, ui: &Ui, view: &UserProfile) {
        let display = ui.io().display_size();
        let layout = profile_editor_layout(display);
        let preview = self.profile_preview_from_drafts(view);
        let mut save = false;
        let mut cancel = false;

        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _round = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("##profile-customize")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .build(|| {
                {
                    let draw = ui.get_window_draw_list();
                    draw.add_rect([0.0, 0.0], display, [0.0, 0.0, 0.0, 0.72])
                        .filled(true)
                        .build();
                    draw.add_rect(layout.frame.min, layout.frame.max, FLOATING_BG)
                        .rounding(PROFILE_RADIUS)
                        .filled(true)
                        .build();
                    draw.add_rect(layout.footer.min, layout.footer.max, MODAL_FOOTER_BG)
                        .rounding(PROFILE_RADIUS)
                        .flags(DrawFlags::ROUND_BOTTOM)
                        .filled(true)
                        .build();
                }

                if skin::icon_button(
                    ui,
                    "profile-editor-close",
                    self.icon("x"),
                    layout.close.min,
                    layout.close.size()[0],
                    18.0,
                    TEXT_SECONDARY,
                    TEXT_NORMAL,
                    Some(ICON_HOVER_BG),
                ) {
                    cancel = true;
                }

                let left_w = layout.left.width();
                let content_w = profile_column_content_width(left_w);
                let left_bg = ui.push_style_color(StyleColor::ChildBg, FLOATING_BG);
                let left_pad = ui.push_style_var(StyleVar::WindowPadding([8.0, 8.0]));
                let left_gap = ui.push_style_var(StyleVar::ItemSpacing([8.0, 6.0]));
                ui.set_cursor_screen_pos(layout.left.min);
                ui.child_window("profile-editor-left")
                    .size(layout.left.size())
                    .build(ui, || {
                        self.draw_profile_scope_switch(ui, content_w);
                        ui.dummy([0.0, 8.0]);
                        if self.profile.edit_scope().is_main() {
                            // Discord-like: cosmetics first, then profile text fields.
                            self.draw_profile_editor_slots(ui, view, content_w);
                            ui.separator();
                            ui.text_colored(MUTED, "DISPLAY NAME");
                            ui.set_next_item_width(content_w);
                            ui.input_text("##profile-display-name", self.profile.draft_display_name_mut())
                                .build();
                            ui.text_colored(MUTED, "PRONOUNS");
                            ui.set_next_item_width(content_w);
                            ui.input_text("##profile-pronouns", self.profile.draft_pronouns_mut())
                                .build();
                            ui.text_colored(MUTED, "CUSTOM STATUS");
                            ui.set_next_item_width(content_w);
                            ui.input_text("##profile-status", self.profile.draft_custom_status_mut())
                                .build();
                            ui.text_colored(MUTED, "ABOUT ME");
                            ui.input_text_multiline(
                                "##profile-about",
                                self.profile.draft_about_me_mut(),
                                [content_w, 64.0],
                            )
                            .build();
                        } else {
                            ui.text_colored(MUTED, "SERVER NICKNAME");
                            ui.set_next_item_width(content_w);
                            ui.input_text("##profile-nickname", self.profile.draft_nickname_mut())
                                .build();
                            let _wrap = ui.push_text_wrap_pos(ui.cursor_screen_pos()[0] + content_w);
                            ui.text_colored(
                                MUTED,
                                "Server profiles use a nickname here. Cosmetics stay on Main Profile.",
                            );
                        }
                    });
                drop(left_gap);
                drop(left_pad);
                drop(left_bg);

                let preview_bg = ui.push_style_color(StyleColor::ChildBg, STAGE_BG);
                ui.set_cursor_screen_pos(layout.preview.min);
                ui.child_window("profile-editor-preview")
                    .size(layout.preview.size())
                    .flags(WindowFlags::NO_SCROLLBAR)
                    .build(ui, || {
                        let origin = ui.cursor_screen_pos();
                        let avail = ui.content_region_avail();
                        let preview_max = [origin[0] + avail[0].min(PROFILE_W), origin[1] + avail[1]];
                        self.paint_profile_preview_card(ui, origin, preview_max, &preview);
                    });
                drop(preview_bg);

                let board_bg = ui.push_style_color(StyleColor::ChildBg, STAGE_BG);
                let board_pad =
                    ui.push_style_var(StyleVar::WindowPadding([PROFILE_BOARD_CARD_PAD, PROFILE_BOARD_CARD_PAD]));
                let board_gap = ui.push_style_var(StyleVar::ItemSpacing([8.0, 8.0]));
                ui.set_cursor_screen_pos(layout.board.min);
                ui.child_window("profile-editor-board")
                    .size(layout.board.size())
                    .build(ui, || {
                        self.draw_profile_board_contents(ui, layout.board, &preview, true);
                    });
                drop(board_gap);
                drop(board_pad);
                drop(board_bg);

                if skin::button(
                    ui,
                    "profile-cancel",
                    "Cancel",
                    Tone::Secondary,
                    layout.cancel.min,
                    layout.cancel.size(),
                    true,
                    self.semibold_font,
                ) {
                    cancel = true;
                }
                if skin::button(
                    ui,
                    "profile-save",
                    "Save",
                    Tone::Brand,
                    layout.save.min,
                    layout.save.size(),
                    true,
                    self.semibold_font,
                ) {
                    save = true;
                }
            });

        if cancel {
            self.profile.cancel_customize();
            self.profile_widget_drafts.clear();
            self.profile_connection_drafts.clear();
            self.profile_badge_drafts.clear();
            self.profile_activity_drafts.clear();
            self.profile_wishlist_drafts.clear();
            self.profile_draft_banner = ProfileMediaDraft::Unchanged;
            self.profile_draft_avatar = ProfileMediaDraft::Unchanged;
            self.profile_editor_slot = None;
        }
        if save {
            self.save_profile_customize(view);
        }
        self.profile_frames = self.profile_frames.saturating_add(1);
    }

    fn profile_preview_from_drafts(&self, view: &UserProfile) -> UserProfile {
        let mut preview = view.clone();
        if self.profile.edit_scope().is_main() {
            preview.display_name = self.profile.draft_display_name().to_string();
            preview.about_me = non_empty_opt(self.profile.draft_about_me());
            preview.pronouns = non_empty_opt(self.profile.draft_pronouns());
            preview.custom_status = non_empty_opt(self.profile.draft_custom_status());
            preview.theme_primary = self.profile.draft_theme_primary();
            preview.theme_secondary = self.profile.draft_theme_secondary();
            preview.nameplate_id = self.profile.draft_nameplate().clone();
            preview.avatar_decoration_id = self.profile.draft_avatar_decoration().clone();
            preview.display_name_style_id = self.profile.draft_display_name_style().clone();
            preview.profile_effect_id = self.profile.draft_profile_effect().clone();
            preview.profile_frame_id = self.profile.draft_profile_frame().clone();
            preview.nickname = None;
        } else {
            preview.nickname = non_empty_opt(self.profile.draft_nickname());
        }
        preview.widgets = self
            .profile_widget_drafts
            .iter()
            .enumerate()
            .map(|(index, widget)| banter_server::ProfileWidget {
                id: index as i64 + 1,
                kind: widget.kind,
                title: widget.title.clone(),
                payload: widget.payload.clone(),
            })
            .collect();
        preview.connections = self
            .profile_connection_drafts
            .iter()
            .enumerate()
            .map(|(index, connection)| banter_server::ProfileConnection {
                id: index as i64 + 1,
                kind: connection.kind.clone(),
                name: connection.name.clone(),
                url: connection.url.clone(),
            })
            .collect();
        preview.badges = self
            .profile_badge_drafts
            .iter()
            .filter_map(|id| {
                self.profile_badges_catalog
                    .iter()
                    .find(|item| item.id == *id)
                    .map(|item| banter_server::ProfileBadge {
                        id: item.id.clone(),
                        label: item.label.clone(),
                        color: item.color,
                        shape: item.shape,
                    })
            })
            .collect();
        preview.activity = self
            .profile_activity_drafts
            .iter()
            .enumerate()
            .map(|(index, entry)| banter_server::ProfileActivity {
                id: index as i64 + 1,
                kind: entry.kind.clone(),
                title: entry.title.clone(),
                detail: entry.detail.clone(),
            })
            .collect();
        preview.wishlist = self
            .profile_wishlist_drafts
            .iter()
            .enumerate()
            .map(|(index, entry)| banter_server::ProfileWishlistItem {
                id: index as i64 + 1,
                title: entry.title.clone(),
                tag: entry.tag.clone(),
            })
            .collect();
        match &self.profile_draft_banner {
            ProfileMediaDraft::Clear => {
                preview.has_banner = false;
                preview.banner_rev = 0;
            }
            ProfileMediaDraft::Bytes(_) => {
                preview.has_banner = true;
            }
            ProfileMediaDraft::Unchanged => {}
        }
        preview
    }

    fn draw_profile_editor_slots(&mut self, ui: &Ui, view: &UserProfile, left_w: f32) {
        for (row_label, slots) in profile_editor_slot_rows() {
            ui.text_colored(MUTED, row_label);
            let mut x = ui.cursor_screen_pos()[0];
            let y = ui.cursor_screen_pos()[1];
            for slot in *slots {
                let size = if matches!(slot, ProfileEditorSlot::Nameplate) {
                    [left_w - 16.0, 40.0]
                } else {
                    [PROFILE_SLOT, PROFILE_SLOT]
                };
                let slot_min = [x, y];
                let slot_max = [x + size[0], y + size[1]];
                self.paint_editor_slot_thumb(ui, slot_min, slot_max, *slot, view);
                if ui.is_mouse_hovering_rect(slot_min, slot_max)
                    && ui.is_mouse_clicked(MouseButton::Left)
                {
                    self.profile_editor_slot = match self.profile_editor_slot {
                        Some(open) if open == *slot => None,
                        _ => Some(*slot),
                    };
                }
                x += size[0] + PROFILE_SLOT_GAP;
            }
            ui.dummy([
                0.0,
                if slots.len() == 1 && matches!(slots[0], ProfileEditorSlot::Nameplate) {
                    48.0
                } else {
                    PROFILE_SLOT + 8.0
                },
            ]);
            if let Some(open) = self.profile_editor_slot {
                if slots.contains(&open) {
                    self.draw_editor_slot_picker(ui, open, view, left_w);
                }
            }
        }
    }

    fn paint_editor_slot_thumb(
        &self,
        ui: &Ui,
        min: [f32; 2],
        max: [f32; 2],
        slot: ProfileEditorSlot,
        view: &UserProfile,
    ) {
        {
            let draw = ui.get_window_draw_list();
            draw.add_rect(min, max, STAGE_BG)
                .rounding(6.0)
                .filled(true)
                .build();
            let selected = self.profile_editor_slot == Some(slot);
            if selected {
                draw.add_rect(min, max, BLURPLE)
                    .rounding(6.0)
                    .thickness(2.0)
                    .build();
            }
        }
        match slot {
            ProfileEditorSlot::Nameplate => {
                profile_paint::paint_nameplate_art(
                    ui,
                    min,
                    max,
                    self.profile.draft_nameplate().as_deref(),
                );
            }
            ProfileEditorSlot::Avatar => {
                let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
                let avatar = self.avatars.get(&view.id).map(|shot| shot.texture);
                paint_avatar(
                    ui,
                    avatar,
                    center,
                    22.0,
                    &initials(&view.display_name),
                    None,
                );
            }
            ProfileEditorSlot::AvatarDecoration => {
                let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
                paint_avatar(ui, None, center, 16.0, "", None);
                profile_paint::paint_avatar_decoration_art(
                    ui,
                    center,
                    16.0,
                    self.profile.draft_avatar_decoration().as_deref(),
                );
                if self.profile.draft_avatar_decoration().is_none() {
                    ui.get_window_draw_list()
                        .add_text([min[0] + 22.0, min[1] + 22.0], MUTED, "+");
                }
            }
            ProfileEditorSlot::DisplayNameStyle => {
                let color =
                    display_name_style_color(self.profile.draft_display_name_style().as_deref());
                let label = truncate_ui(self.profile.draft_display_name(), 10);
                ui.get_window_draw_list()
                    .add_text([min[0] + 8.0, min[1] + 22.0], color, &label);
            }
            ProfileEditorSlot::Theme => {
                let primary = rgb_u32_to_rgba(self.profile.draft_theme_primary());
                let secondary = rgb_u32_to_rgba(self.profile.draft_theme_secondary());
                let draw = ui.get_window_draw_list();
                draw.add_rect(min, [(min[0] + max[0]) * 0.5, max[1]], primary)
                    .rounding(6.0)
                    .flags(DrawFlags::ROUND_LEFT)
                    .filled(true)
                    .build();
                draw.add_rect([(min[0] + max[0]) * 0.5, min[1]], max, secondary)
                    .rounding(6.0)
                    .flags(DrawFlags::ROUND_RIGHT)
                    .filled(true)
                    .build();
            }
            ProfileEditorSlot::Banner => {
                let draw = ui.get_window_draw_list();
                if matches!(self.profile_draft_banner, ProfileMediaDraft::Clear) {
                    draw.add_text([min[0] + 18.0, min[1] + 22.0], MUTED, "+");
                } else if let Some(shot) = self.banners.get(&view.id) {
                    draw.add_image_rounded(
                        shot.texture,
                        min,
                        max,
                        [0.0, 0.0],
                        [1.0, 1.0],
                        WHITE,
                        6.0,
                        DrawFlags::ROUND_ALL,
                    );
                } else {
                    let primary = rgb_u32_to_rgba(self.profile.draft_theme_primary());
                    draw.add_rect(min, max, primary)
                        .rounding(6.0)
                        .filled(true)
                        .build();
                    if !view.has_banner {
                        draw.add_text([min[0] + 18.0, min[1] + 22.0], MUTED, "+");
                    }
                }
            }
            ProfileEditorSlot::ProfileEffect => {
                profile_paint::paint_profile_effect_art(
                    ui,
                    min,
                    max,
                    self.profile.draft_profile_effect().as_deref(),
                );
                if self.profile.draft_profile_effect().is_none() {
                    ui.get_window_draw_list()
                        .add_text([min[0] + 22.0, min[1] + 22.0], MUTED, "+");
                }
            }
            ProfileEditorSlot::ProfileFrame => {
                let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
                profile_paint::paint_profile_frame_art(
                    ui,
                    center,
                    18.0,
                    self.profile.draft_profile_frame().as_deref(),
                );
                if self.profile.draft_profile_frame().is_none() {
                    ui.get_window_draw_list()
                        .add_text([min[0] + 22.0, min[1] + 22.0], MUTED, "+");
                }
            }
            ProfileEditorSlot::Badges => {
                let mut bx = min[0] + 10.0;
                for id in self.profile_badge_drafts.iter().take(3) {
                    if let Some(item) = self.profile_badges_catalog.iter().find(|b| b.id == *id) {
                        let badge = banter_server::ProfileBadge {
                            id: item.id.clone(),
                            label: item.label.clone(),
                            color: item.color,
                            shape: item.shape,
                        };
                        profile_paint::paint_badge_icon(ui, [bx, (min[1] + max[1]) * 0.5], &badge);
                        bx += 22.0;
                    }
                }
                if self.profile_badge_drafts.is_empty() {
                    ui.get_window_draw_list()
                        .add_text([min[0] + 22.0, min[1] + 22.0], MUTED, "+");
                }
            }
        }
    }

    fn draw_editor_slot_picker(
        &mut self,
        ui: &Ui,
        slot: ProfileEditorSlot,
        view: &UserProfile,
        left_w: f32,
    ) {
        match slot {
            ProfileEditorSlot::Nameplate => {
                self.draw_cosmetic_picker(ui, "Nameplate", CosmeticKind::Nameplate, left_w);
            }
            ProfileEditorSlot::Avatar => {
                if ui.small_button("Change Avatar") {
                    self.pick_avatar();
                }
                ui.same_line();
                if ui.small_button("Remove Avatar") {
                    self.profile_draft_avatar = ProfileMediaDraft::Clear;
                }
            }
            ProfileEditorSlot::AvatarDecoration => {
                self.draw_cosmetic_picker(
                    ui,
                    "Avatar Decoration",
                    CosmeticKind::AvatarDecoration,
                    left_w,
                );
            }
            ProfileEditorSlot::DisplayNameStyle => {
                self.draw_cosmetic_picker(
                    ui,
                    "Display Name Style",
                    CosmeticKind::DisplayNameStyle,
                    left_w,
                );
            }
            ProfileEditorSlot::Theme => {
                ui.text_colored(MUTED, "THEME");
                let themes = [
                    (0x2d4a3e_u32, 0x111214_u32),
                    (0x3a2218, 0x1a1210),
                    (0x1a3344, 0x0e151c),
                    (0x4a2d5e, 0x141018),
                ];
                for (i, (primary, secondary)) in themes.iter().enumerate() {
                    if i > 0 {
                        ui.same_line();
                    }
                    let color = rgb_u32_to_rgba(*primary);
                    if ui.color_button(&format!("##theme{i}"), color) {
                        self.profile.set_draft_theme(*primary, *secondary);
                    }
                }
            }
            ProfileEditorSlot::Banner => {
                if ui.small_button("Change Banner") {
                    self.pick_banner();
                }
                ui.same_line();
                if ui.small_button("Remove Banner") {
                    self.profile_draft_banner = ProfileMediaDraft::Clear;
                }
                let _ = view;
            }
            ProfileEditorSlot::ProfileEffect => {
                self.draw_cosmetic_picker(
                    ui,
                    "Profile Effect",
                    CosmeticKind::ProfileEffect,
                    left_w,
                );
            }
            ProfileEditorSlot::ProfileFrame => {
                self.draw_cosmetic_picker(ui, "Profile Frame", CosmeticKind::ProfileFrame, left_w);
            }
            ProfileEditorSlot::Badges => {
                ui.text_colored(MUTED, "Equipped badges");
                let catalog = self.profile_badges_catalog.clone();
                for item in catalog {
                    let mut equipped = self.profile_badge_drafts.iter().any(|id| id == &item.id);
                    if ui.checkbox(&format!("{}##badge-{}", item.label, item.id), &mut equipped) {
                        if equipped {
                            if !self.profile_badge_drafts.iter().any(|id| id == &item.id)
                                && self.profile_badge_drafts.len() < 12
                            {
                                self.profile_badge_drafts.push(item.id.clone());
                            }
                        } else {
                            self.profile_badge_drafts.retain(|id| id != &item.id);
                        }
                    }
                }
            }
        }
    }

    fn draw_cosmetic_picker(&mut self, ui: &Ui, label: &str, kind: CosmeticKind, width: f32) {
        ui.text_colored(MUTED, label);
        let current = match kind {
            CosmeticKind::Nameplate => self.profile.draft_nameplate().clone(),
            CosmeticKind::AvatarDecoration => self.profile.draft_avatar_decoration().clone(),
            CosmeticKind::DisplayNameStyle => self.profile.draft_display_name_style().clone(),
            CosmeticKind::ProfileEffect => self.profile.draft_profile_effect().clone(),
            CosmeticKind::ProfileFrame => self.profile.draft_profile_frame().clone(),
        };
        let none_selected = current.is_none();
        if ui.radio_button_bool(&format!("None##{label}"), none_selected) && !none_selected {
            match kind {
                CosmeticKind::Nameplate => self.profile.set_draft_nameplate(None),
                CosmeticKind::AvatarDecoration => self.profile.set_draft_avatar_decoration(None),
                CosmeticKind::DisplayNameStyle => self.profile.set_draft_display_name_style(None),
                CosmeticKind::ProfileEffect => self.profile.set_draft_profile_effect(None),
                CosmeticKind::ProfileFrame => self.profile.set_draft_profile_frame(None),
            }
        }
        let row_w = profile_column_content_width(width);
        for item in self
            .profile_cosmetics
            .iter()
            .filter(|item| item.kind == kind)
            .cloned()
        {
            let selected = current.as_deref() == Some(item.id.as_str());
            let swatch = rgb_u32_to_rgba(item.color);
            let origin = ui.cursor_screen_pos();
            let _ = ui.color_button(&format!("##swatch-{}-{}", label, item.id), swatch);
            ui.set_cursor_screen_pos([origin[0] + 28.0, origin[1]]);
            ui.set_next_item_width((row_w - 28.0).max(60.0));
            if ui.radio_button_bool(&format!("{}##{label}", item.label), selected) && !selected {
                let id = Some(item.id.clone());
                match kind {
                    CosmeticKind::Nameplate => self.profile.set_draft_nameplate(id),
                    CosmeticKind::AvatarDecoration => self.profile.set_draft_avatar_decoration(id),
                    CosmeticKind::DisplayNameStyle => self.profile.set_draft_display_name_style(id),
                    CosmeticKind::ProfileEffect => self.profile.set_draft_profile_effect(id),
                    CosmeticKind::ProfileFrame => self.profile.set_draft_profile_frame(id),
                }
            }
        }
    }

    /// Board / Activity / Wishlist body. Caller supplies a clipped child with padding.
    fn draw_profile_board_contents(
        &mut self,
        ui: &Ui,
        bounds: ProfilePanelRect,
        view: &UserProfile,
        editing: bool,
    ) {
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        for (index, tab) in profile_board_tabs().iter().enumerate() {
            if index > 0 {
                ui.same_line();
            }
            let label = match tab {
                ProfileBoardTab::Board => "Board",
                ProfileBoardTab::Activity => "Activity",
                ProfileBoardTab::Wishlist => "Wishlist",
            };
            let active = self.profile.board_tab() == *tab;
            if ui.radio_button_bool(label, active) {
                self.profile.set_board_tab(*tab);
            }
        }
        ui.dummy([0.0, 4.0]);
        match self.profile.board_tab() {
            ProfileBoardTab::Board => self.draw_profile_board_tab(ui, bounds, view, editing),
            ProfileBoardTab::Activity => self.draw_profile_activity_tab(ui, view, editing),
            ProfileBoardTab::Wishlist => self.draw_profile_wishlist_tab(ui, bounds, view, editing),
        }
    }

    fn draw_profile_board_tab(
        &mut self,
        ui: &Ui,
        bounds: ProfilePanelRect,
        view: &UserProfile,
        editing: bool,
    ) {
        let content_w = profile_column_content_width(bounds.width());
        if editing {
            {
                let header = ui.cursor_screen_pos();
                ui.text_colored(TEXT, "Your Widgets");
                ui.set_cursor_screen_pos([header[0] + content_w.max(120.0) - 100.0, header[1]]);
                if ui.small_button("+ Add Widget") {
                    self.profile_widget_drafts.push(ProfileWidgetInput {
                        kind: ProfileWidgetKind::FavoriteGame,
                        title: "Favorite Game".into(),
                        payload: serde_json::json!({ "title": "", "quote": "" }),
                    });
                }
            }
            let mut remove = None;
            for (index, widget) in self.profile_widget_drafts.iter_mut().enumerate() {
                ui.dummy([0.0, 6.0]);
                let card_bg = ui.push_style_color(StyleColor::ChildBg, INPUT_BG);
                let card_pad = ui.push_style_var(StyleVar::WindowPadding([8.0, 8.0]));
                let card_round = ui.push_style_var(StyleVar::ChildRounding(PROFILE_RADIUS));
                ui.child_window(format!("profile-widget-card-{index}"))
                    .size([content_w, 0.0])
                    .child_flags(ChildFlags::BORDERS | ChildFlags::AUTO_RESIZE_Y)
                    .build(ui, || {
                        ui.text_colored(MUTED, &format!("Widget {}", index + 1));
                        // Stack kind radios — same_line overflow was breaking the board column.
                        if ui.radio_button_bool(
                            &format!("Favorite Game##w{index}"),
                            widget.kind == ProfileWidgetKind::FavoriteGame,
                        ) {
                            if widget.kind != ProfileWidgetKind::FavoriteGame {
                                widget.payload = serde_json::json!({ "title": "", "quote": "" });
                            }
                            widget.kind = ProfileWidgetKind::FavoriteGame;
                            widget.title = "Favorite Game".into();
                        }
                        if ui.radio_button_bool(
                            &format!("Games in Rotation##w{index}"),
                            widget.kind == ProfileWidgetKind::GamesInRotation,
                        ) {
                            if widget.kind != ProfileWidgetKind::GamesInRotation {
                                widget.payload = games_to_payload(&[]);
                            }
                            widget.kind = ProfileWidgetKind::GamesInRotation;
                            widget.title = "Games in Rotation".into();
                        }
                        if ui.radio_button_bool(
                            &format!("Games I Like##w{index}"),
                            widget.kind == ProfileWidgetKind::GamesILike,
                        ) {
                            if widget.kind != ProfileWidgetKind::GamesILike {
                                widget.payload = games_to_payload(&[]);
                            }
                            widget.kind = ProfileWidgetKind::GamesILike;
                            widget.title = "Games I Like".into();
                        }
                        let field_w = (ui.content_region_avail()[0]).max(80.0);
                        ui.set_next_item_width(field_w);
                        ui.input_text(&format!("##widget-title-{index}"), &mut widget.title)
                            .build();
                        match widget.kind {
                            ProfileWidgetKind::FavoriteGame => {
                                let mut title = widget
                                    .payload
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let mut quote = widget
                                    .payload
                                    .get("quote")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                ui.set_next_item_width(field_w);
                                ui.input_text(&format!("##fav-title-{index}"), &mut title)
                                    .build();
                                ui.set_next_item_width(field_w);
                                ui.input_text(&format!("##fav-quote-{index}"), &mut quote)
                                    .build();
                                widget.payload =
                                    serde_json::json!({ "title": title, "quote": quote });
                            }
                            ProfileWidgetKind::GamesInRotation | ProfileWidgetKind::GamesILike => {
                                let cap = games_widget_cap(widget.kind);
                                let mut games = games_from_payload(&widget.payload);
                                let mut remove_game = None;
                                for (game_i, (title, tag)) in games.iter_mut().enumerate() {
                                    let half = ((field_w - 8.0) * 0.5).max(70.0);
                                    ui.set_next_item_width(half);
                                    ui.input_text(&format!("##game-title-{index}-{game_i}"), title)
                                        .build();
                                    ui.same_line();
                                    ui.set_next_item_width(half);
                                    ui.input_text(&format!("##game-tag-{index}-{game_i}"), tag)
                                        .build();
                                    if ui.small_button(&format!("x##game-rm-{index}-{game_i}")) {
                                        remove_game = Some(game_i);
                                    }
                                }
                                if let Some(game_i) = remove_game {
                                    games.remove(game_i);
                                }
                                if games.len() < cap
                                    && ui.small_button(&format!("+ Add Game##w{index}"))
                                {
                                    games.push((String::new(), String::new()));
                                }
                                widget.payload = games_to_payload(&games);
                            }
                        }
                        if ui.small_button(&format!("Remove##widget-{index}")) {
                            remove = Some(index);
                        }
                    });
                drop(card_round);
                drop(card_pad);
                drop(card_bg);
            }
            if let Some(index) = remove {
                self.profile_widget_drafts.remove(index);
            }
            ui.separator();
            ui.text_colored(TEXT, "Connections");
            if ui.small_button("+ Add Connection") {
                self.profile_connection_drafts.push(ProfileConnectionInput {
                    kind: "website".into(),
                    name: String::new(),
                    url: "https://".into(),
                });
            }
            let mut remove_conn = None;
            for (index, connection) in self.profile_connection_drafts.iter_mut().enumerate() {
                let field_w = ((content_w - 16.0) * 0.45).max(70.0);
                ui.set_next_item_width(field_w);
                ui.input_text(&format!("##conn-kind-{index}"), &mut connection.kind)
                    .build();
                ui.same_line();
                ui.set_next_item_width(field_w);
                ui.input_text(&format!("##conn-name-{index}"), &mut connection.name)
                    .build();
                ui.set_next_item_width(content_w - 16.0);
                ui.input_text(&format!("##conn-url-{index}"), &mut connection.url)
                    .build();
                if ui.small_button(&format!("Remove##conn-{index}")) {
                    remove_conn = Some(index);
                }
            }
            if let Some(index) = remove_conn {
                self.profile_connection_drafts.remove(index);
            }
        } else if view.widgets.is_empty() {
            ui.text_colored(MUTED, "No widgets yet.");
        } else {
            for widget in &view.widgets {
                ui.dummy([0.0, 4.0]);
                ui.text_colored(TEXT, &widget.title);
                match widget.kind {
                    ProfileWidgetKind::FavoriteGame => {
                        let title = widget
                            .payload
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let quote = widget
                            .payload
                            .get("quote")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !title.is_empty() {
                            let origin = ui.cursor_screen_pos();
                            profile_paint::paint_game_card(ui, origin, title, "");
                            ui.dummy([PROFILE_GAME_CARD_W + 8.0, PROFILE_GAME_CARD_H + 4.0]);
                        }
                        if !quote.is_empty() {
                            let _wrap =
                                ui.push_text_wrap_pos(ui.cursor_screen_pos()[0] + content_w);
                            ui.text_colored(MUTED, quote);
                        }
                    }
                    ProfileWidgetKind::GamesInRotation => {
                        let games = games_from_payload(&widget.payload);
                        let mut x = ui.cursor_screen_pos()[0];
                        let mut y = ui.cursor_screen_pos()[1];
                        let right = ui.cursor_screen_pos()[0] + content_w;
                        for (title, tag) in &games {
                            if x + PROFILE_GAME_CARD_W > right && x > ui.cursor_screen_pos()[0] {
                                x = ui.cursor_screen_pos()[0];
                                y += PROFILE_GAME_CARD_H + 8.0;
                            }
                            profile_paint::paint_game_card(ui, [x, y], title, tag);
                            x += PROFILE_GAME_CARD_W + 8.0;
                        }
                        let rows = ((games.len().max(1) as f32) * (PROFILE_GAME_CARD_W + 8.0)
                            / content_w.max(1.0))
                        .ceil()
                        .max(1.0);
                        ui.dummy([0.0, rows * (PROFILE_GAME_CARD_H + 8.0)]);
                    }
                    ProfileWidgetKind::GamesILike => {
                        let games = games_from_payload(&widget.payload);
                        let mut x = ui.cursor_screen_pos()[0];
                        let mut y = ui.cursor_screen_pos()[1];
                        let right = ui.cursor_screen_pos()[0] + content_w;
                        for (title, _) in &games {
                            if x + PROFILE_GAME_THUMB > right && x > ui.cursor_screen_pos()[0] {
                                x = ui.cursor_screen_pos()[0];
                                y += PROFILE_GAME_THUMB + 6.0;
                            }
                            profile_paint::paint_game_thumb(ui, [x, y], title);
                            x += PROFILE_GAME_THUMB + 6.0;
                        }
                        let per_row = (content_w / (PROFILE_GAME_THUMB + 6.0)).floor().max(1.0);
                        let rows = ((games.len() as f32) / per_row).ceil().max(1.0);
                        ui.dummy([0.0, rows * (PROFILE_GAME_THUMB + 6.0)]);
                    }
                }
            }
            // Connections stay on the identity column in view mode; skip duplicate board list.
        }
    }

    fn draw_profile_connection_row(
        &mut self,
        ui: &Ui,
        connection: &banter_server::ProfileConnection,
    ) {
        let label = format!(
            "{}  {}  ↗",
            connection_kind_glyph(&connection.kind),
            connection.name
        );
        if ui.small_button(&format!("{label}##conn-{}", connection.id)) {
            if let Err(err) = open_http_url(&connection.url) {
                self.status = err;
            }
        }
        skin::item_tooltip(ui, self.semibold_font, &connection.url);
    }

    fn draw_profile_activity_tab(&mut self, ui: &Ui, view: &UserProfile, editing: bool) {
        let field_w = (ui.content_region_avail()[0]).max(80.0);
        if editing {
            if ui.small_button("+ Add Activity") {
                self.profile_activity_drafts.push(ProfileActivityInput {
                    kind: "custom".into(),
                    title: String::new(),
                    detail: String::new(),
                });
            }
            let mut remove = None;
            for (index, entry) in self.profile_activity_drafts.iter_mut().enumerate() {
                ui.separator();
                ui.set_next_item_width(field_w);
                ui.input_text(&format!("##act-kind-{index}"), &mut entry.kind)
                    .build();
                ui.set_next_item_width(field_w);
                ui.input_text(&format!("##act-title-{index}"), &mut entry.title)
                    .build();
                ui.set_next_item_width(field_w);
                ui.input_text(&format!("##act-detail-{index}"), &mut entry.detail)
                    .build();
                if ui.small_button(&format!("Remove##act-{index}")) {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                self.profile_activity_drafts.remove(index);
            }
        } else if view.activity.is_empty() {
            ui.text_colored(MUTED, "No recent activity.");
        } else {
            for entry in &view.activity {
                ui.separator();
                ui.text_colored(TEXT, &entry.title);
                let _wrap = ui.push_text_wrap_pos(ui.cursor_screen_pos()[0] + field_w);
                if !entry.detail.is_empty() {
                    ui.text_colored(MUTED, &format!("{} · {}", entry.kind, entry.detail));
                } else {
                    ui.text_colored(MUTED, &entry.kind);
                }
            }
        }
    }

    fn draw_profile_wishlist_tab(
        &mut self,
        ui: &Ui,
        bounds: ProfilePanelRect,
        view: &UserProfile,
        editing: bool,
    ) {
        let content_w = profile_column_content_width(bounds.width());
        if editing {
            if ui.small_button("+ Add Wishlist Game") {
                self.profile_wishlist_drafts.push(ProfileWishlistInput {
                    title: String::new(),
                    tag: String::new(),
                });
            }
            let mut remove = None;
            for (index, entry) in self.profile_wishlist_drafts.iter_mut().enumerate() {
                let half = ((content_w - 40.0) * 0.5).max(70.0);
                ui.set_next_item_width(half);
                ui.input_text(&format!("##wish-title-{index}"), &mut entry.title)
                    .build();
                ui.same_line();
                ui.set_next_item_width(half);
                ui.input_text(&format!("##wish-tag-{index}"), &mut entry.tag)
                    .build();
                if ui.small_button(&format!("x##wish-{index}")) {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                self.profile_wishlist_drafts.remove(index);
            }
        } else if view.wishlist.is_empty() {
            ui.text_colored(MUTED, "Wishlist is empty.");
        } else {
            let mut x = ui.cursor_screen_pos()[0];
            let mut y = ui.cursor_screen_pos()[1];
            let left = x;
            let right = left + content_w;
            for entry in &view.wishlist {
                if x + PROFILE_GAME_THUMB > right && x > left {
                    x = left;
                    y += PROFILE_GAME_THUMB + 6.0;
                }
                profile_paint::paint_game_thumb(ui, [x, y], &entry.title);
                x += PROFILE_GAME_THUMB + 6.0;
            }
            let per_row = (content_w / (PROFILE_GAME_THUMB + 6.0)).floor().max(1.0);
            let rows = ((view.wishlist.len() as f32) / per_row).ceil().max(1.0);
            ui.dummy([0.0, rows * (PROFILE_GAME_THUMB + 6.0)]);
            for entry in &view.wishlist {
                if entry.tag.is_empty() {
                    ui.text_colored(TEXT_SECONDARY, &entry.title);
                } else {
                    ui.text_colored(TEXT_SECONDARY, &format!("{} — {}", entry.title, entry.tag));
                }
            }
        }
    }

    fn paint_profile_banner(
        &self,
        ui: &Ui,
        min: [f32; 2],
        max: [f32; 2],
        view: &UserProfile,
        theme_primary: u32,
        theme_secondary: u32,
    ) {
        {
            let draw = ui.get_window_draw_list();
            let primary = rgb_u32_to_rgba(theme_primary);
            let secondary = rgb_u32_to_rgba(theme_secondary);
            draw.add_rect(min, max, secondary)
                .rounding(PROFILE_RADIUS)
                .flags(DrawFlags::ROUND_TOP)
                .filled(true)
                .build();
            let mid = [max[0], min[1] + (max[1] - min[1]) * 0.45];
            draw.add_rect([min[0], mid[1]], max, primary)
                .filled(true)
                .build();
            let show_banner =
                view.has_banner && !matches!(self.profile_draft_banner, ProfileMediaDraft::Clear);
            if show_banner {
                if let Some(shot) = self.banners.get(&view.id) {
                    draw.add_image_rounded(
                        shot.texture,
                        min,
                        max,
                        [0.0, 0.0],
                        [1.0, 1.0],
                        WHITE,
                        PROFILE_RADIUS,
                        DrawFlags::ROUND_TOP,
                    );
                }
            }
        }
        if let Some(plate) = view.nameplate_id.as_deref() {
            let plate_min = [min[0] + 12.0, max[1] - PROFILE_NAMEPLATE_H - 8.0];
            let plate_max = [max[0] - 12.0, max[1] - 8.0];
            profile_paint::paint_nameplate_art(ui, plate_min, plate_max, Some(plate));
        }
    }

    fn paint_profile_preview_card(
        &self,
        ui: &Ui,
        min: [f32; 2],
        max: [f32; 2],
        view: &UserProfile,
    ) {
        let width = (max[0] - min[0]).min(PROFILE_W);
        let card_max = [min[0] + width, max[1]];
        {
            let draw = ui.get_window_draw_list();
            draw.add_rect(min, card_max, FLOATING_BG)
                .rounding(PROFILE_RADIUS)
                .filled(true)
                .build();
        }
        self.paint_profile_banner(
            ui,
            min,
            [card_max[0], min[1] + PROFILE_BANNER_H],
            view,
            view.theme_primary,
            view.theme_secondary,
        );
        profile_paint::paint_profile_effect_art(
            ui,
            [min[0], min[1] + PROFILE_BANNER_H],
            card_max,
            view.profile_effect_id.as_deref(),
        );
        let heading = profile_heading(&view.display_name, view.nickname.as_deref());
        let avatar_center = [
            min[0] + PROFILE_PAD + PROFILE_AVATAR,
            min[1] + PROFILE_BANNER_H,
        ];
        {
            let draw = ui.get_window_draw_list();
            draw.add_circle(avatar_center, PROFILE_AVATAR + 6.0, FLOATING_BG)
                .filled(true)
                .build();
        }
        profile_paint::paint_profile_frame_art(
            ui,
            avatar_center,
            PROFILE_AVATAR,
            view.profile_frame_id.as_deref(),
        );
        let avatar = self.avatars.get(&view.id).map(|shot| shot.texture);
        paint_avatar(
            ui,
            avatar,
            avatar_center,
            PROFILE_AVATAR,
            &initials(&heading),
            None,
        );
        profile_paint::paint_avatar_decoration_art(
            ui,
            avatar_center,
            PROFILE_AVATAR,
            view.avatar_decoration_id.as_deref(),
        );
        paint_status_dot(
            ui,
            avatar_center,
            PROFILE_AVATAR,
            Presence::Online,
            FLOATING_BG,
        );
        if let Some(status) = view
            .custom_status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let bubble_min = [
                avatar_center[0] + PROFILE_AVATAR + 10.0,
                avatar_center[1] - 18.0,
            ];
            let size = ui.calc_text_size(status);
            let bubble_max = [
                bubble_min[0] + size[0] + 16.0,
                bubble_min[1] + size[1] + 10.0,
            ];
            let draw = ui.get_window_draw_list();
            draw.add_rect(bubble_min, bubble_max, STAGE_BG)
                .rounding(8.0)
                .filled(true)
                .build();
            draw.add_text([bubble_min[0] + 8.0, bubble_min[1] + 5.0], TEXT, status);
        }
        let text_x = min[0] + PROFILE_PAD;
        let name_y = min[1] + PROFILE_BANNER_H + PROFILE_AVATAR + 14.0;
        let name_color = display_name_style_color(view.display_name_style_id.as_deref());
        {
            let _font = ui.push_font_with_size(self.bold_font, FONT_LG);
            ui.get_window_draw_list()
                .add_text([text_x, name_y], name_color, &heading);
        }
        let mut meta = format!("@{}", view.username);
        if let Some(pronouns) = view
            .pronouns
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            meta.push_str(" • ");
            meta.push_str(pronouns);
        }
        ui.get_window_draw_list()
            .add_text([text_x, name_y + 26.0], TEXT_SECONDARY, &meta);
        let mut y = name_y + 52.0;
        if !view.badges.is_empty() {
            let cap = profile_badge_row_visible_cap();
            let mut bx = text_x;
            for badge in view.badges.iter().take(cap) {
                profile_paint::paint_badge_icon(ui, [bx + PROFILE_BADGE, y + PROFILE_BADGE], badge);
                bx += PROFILE_BADGE * 2.0 + 6.0;
            }
            if view.badges.len() > cap {
                let extra = view.badges.len() - cap;
                ui.get_window_draw_list()
                    .add_text([bx, y + 4.0], MUTED, &format!("+{extra}"));
            }
            y += PROFILE_BADGE * 2.0 + 8.0;
        }
        let wrap_w = (card_max[0] - text_x - 8.0).max(80.0);
        let about_lines = view
            .about_me
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|about| skin::wrap_lines(ui, None, FONT_SM, about, wrap_w))
            .unwrap_or_default();
        if y < card_max[1] - 8.0 {
            let _clip = ui.push_clip_rect([min[0], y], card_max, true);
            {
                let draw = ui.get_window_draw_list();
                if !about_lines.is_empty() {
                    draw.add_text([text_x, y], MUTED, "ABOUT ME");
                    y += 18.0;
                    for line in about_lines.iter().take(6) {
                        if y + 16.0 > card_max[1] {
                            break;
                        }
                        draw.add_text([text_x, y], TEXT, line);
                        y += 16.0;
                    }
                    y += 8.0;
                }
                if y + 36.0 <= card_max[1] {
                    draw.add_text([text_x, y], MUTED, "MEMBER SINCE");
                    y += 18.0;
                    draw.add_text([text_x, y], TEXT, &format_member_since(view.member_since));
                    y += 24.0;
                }
                if !view.roles.is_empty() && y + 18.0 <= card_max[1] {
                    draw.add_text([text_x, y], MUTED, "ROLES");
                    y += 18.0;
                    for role in view.roles.iter().take(6) {
                        if y + 16.0 > card_max[1] {
                            break;
                        }
                        let color = if role.color > 0 {
                            rgb_u32_to_rgba(role.color as u32)
                        } else {
                            TEXT
                        };
                        draw.add_text([text_x, y], color, &role.name);
                        y += 16.0;
                    }
                    y += 8.0;
                }
                if !view.connections.is_empty() && y + 18.0 <= card_max[1] {
                    draw.add_text([text_x, y], MUTED, "CONNECTIONS");
                    y += 18.0;
                    for connection in &view.connections {
                        if y + 16.0 > card_max[1] {
                            break;
                        }
                        draw.add_text(
                            [text_x, y],
                            TEXT_SECONDARY,
                            &format!("{} — {}", connection.kind, connection.name),
                        );
                        y += 18.0;
                    }
                }
            }
        }
    }

    fn save_profile_customize(&mut self, previous: &UserProfile) {
        if let Some(server_id) = self.profile.edit_scope().server_id() {
            let nickname = {
                let trimmed = self.profile.draft_nickname().trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            };
            match self.client.set_nickname(server_id, nickname.as_deref()) {
                Ok(()) => {
                    self.profile.cancel_customize();
                    self.profile_editor_slot = None;
                    self.status.clear();
                    if matches!(self.place, Place::Server(id) if id == server_id) {
                        self.reload_members();
                    }
                }
                Err(err) => self.status = err.to_string(),
            }
            let _ = previous;
            return;
        }
        let draft_name = self.profile.draft_display_name().trim().to_string();
        if let Some(problem) = display_name_error(&draft_name) {
            self.status = problem.to_string();
            return;
        }
        match &self.profile_draft_banner {
            ProfileMediaDraft::Clear => {
                if let Err(err) = self.client.clear_banner() {
                    self.status = err.to_string();
                    return;
                }
            }
            ProfileMediaDraft::Bytes(bytes) => {
                if let Err(err) = self.client.set_banner(bytes) {
                    self.status = err.to_string();
                    return;
                }
            }
            ProfileMediaDraft::Unchanged => {}
        }
        match &self.profile_draft_avatar {
            ProfileMediaDraft::Clear => {
                if let Err(err) = self.client.clear_avatar() {
                    self.status = err.to_string();
                    return;
                }
                self.drop_own_avatar();
            }
            ProfileMediaDraft::Bytes(bytes) => {
                if let Err(err) = self.client.set_avatar(bytes) {
                    self.status = err.to_string();
                    return;
                }
                self.drop_own_avatar();
            }
            ProfileMediaDraft::Unchanged => {}
        }
        let patch = ProfilePatch {
            display_name: Some(draft_name),
            about_me: Some(self.profile.draft_about_me().to_string()),
            pronouns: Some(self.profile.draft_pronouns().to_string()),
            custom_status: Some(self.profile.draft_custom_status().to_string()),
            theme_primary: Some(self.profile.draft_theme_primary()),
            theme_secondary: Some(self.profile.draft_theme_secondary()),
            nameplate_id: Some(self.profile.draft_nameplate().clone().unwrap_or_default()),
            avatar_decoration_id: Some(
                self.profile
                    .draft_avatar_decoration()
                    .clone()
                    .unwrap_or_default(),
            ),
            display_name_style_id: Some(
                self.profile
                    .draft_display_name_style()
                    .clone()
                    .unwrap_or_default(),
            ),
            profile_effect_id: Some(
                self.profile
                    .draft_profile_effect()
                    .clone()
                    .unwrap_or_default(),
            ),
            profile_frame_id: Some(
                self.profile
                    .draft_profile_frame()
                    .clone()
                    .unwrap_or_default(),
            ),
            widgets: Some(self.profile_widget_drafts.clone()),
            connections: Some(self.profile_connection_drafts.clone()),
            badge_ids: Some(self.profile_badge_drafts.clone()),
            activity: Some(
                self.profile_activity_drafts
                    .iter()
                    .filter(|entry| !entry.title.trim().is_empty())
                    .cloned()
                    .collect(),
            ),
            wishlist: Some(
                self.profile_wishlist_drafts
                    .iter()
                    .filter(|entry| !entry.title.trim().is_empty())
                    .cloned()
                    .collect(),
            ),
        };
        match self.client.update_profile(&patch) {
            Ok(updated) => {
                self.display_name = updated.display_name.clone();
                if let Some(account) = self.account.as_mut() {
                    account.display_name = updated.display_name.clone();
                }
                self.banner_rev.remove(&updated.id);
                self.banner_miss.remove(&updated.id);
                if let Some(account_id) = self.account.as_ref().map(|account| account.id) {
                    let state = self
                        .presence
                        .get(&account_id)
                        .map(|info| info.state)
                        .unwrap_or(Presence::Online);
                    self.presence.insert(
                        account_id,
                        PresenceInfo {
                            state,
                            custom_status: updated.custom_status.clone(),
                        },
                    );
                }
                self.profile_view = Some(updated);
                self.profile.finish_edit();
                self.profile_widget_drafts.clear();
                self.profile_connection_drafts.clear();
                self.profile_badge_drafts.clear();
                self.profile_activity_drafts.clear();
                self.profile_wishlist_drafts.clear();
                self.profile_draft_banner = ProfileMediaDraft::Unchanged;
                self.profile_draft_avatar = ProfileMediaDraft::Unchanged;
                self.profile_editor_slot = None;
                self.status.clear();
            }
            Err(err) => {
                self.status = err.to_string();
                let _ = previous;
            }
        }
    }

    fn ensure_profile_banner(&mut self, imgui: &Ui, view: &UserProfile) {
        let _ = imgui;
        if matches!(self.profile_draft_banner, ProfileMediaDraft::Clear) || !view.has_banner {
            if let Some(shot) = self.banners.remove(&view.id) {
                self.retired_shots.push(shot);
            }
            self.banner_rev.remove(&view.id);
            return;
        }
        if self.banner_rev.get(&view.id) == Some(&view.banner_rev)
            && self.banners.contains_key(&view.id)
        {
            return;
        }
        if self.banner_miss.get(&view.id) == Some(&view.banner_rev) {
            return;
        }
        match self.client.banner(view.id) {
            Ok(bytes) => {
                self.pending_banner = Some((view.id, view.banner_rev, bytes));
            }
            Err(_) => {
                self.banner_miss.insert(view.id, view.banner_rev);
            }
        }
    }

    fn pick_banner(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
        else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            self.status = "could not read that banner".into();
            return;
        };
        if bytes.len() > MAX_BANNER_BYTES {
            self.status = "banner is too large".into();
            return;
        }
        if self.profile.customizing() {
            self.profile_draft_banner = ProfileMediaDraft::Bytes(bytes);
            self.status.clear();
            return;
        }
        match self.client.set_banner(&bytes) {
            Ok(()) => {
                if let Ok(profile) = self
                    .client
                    .me()
                    .and_then(|account| self.client.user_profile(account.id, None))
                {
                    if let Some(view) = self.profile_view.as_mut() {
                        view.has_banner = profile.has_banner;
                        view.banner_rev = profile.banner_rev;
                    }
                    self.banner_rev.remove(&profile.id);
                    self.banner_miss.remove(&profile.id);
                }
                self.status.clear();
            }
            Err(err) => self.status = err.to_string(),
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
        if section == SettingsSection::Mods {
            self.mods.reload();
        }
    }

    fn voice_settings_visible(&self) -> bool {
        self.settings.is_open() && self.settings.section() == SettingsSection::Voice
    }

    fn leave_voice(&mut self) {
        self.pending_voice_rejoin = None;
        let left = self.note(self.client.leave_voice()).is_some();
        // Server already dropped the seat on websocket close; still clear local UI while reconnecting.
        if left || self.reconnect_at.is_some() {
            self.voice_channel = None;
            self.voice_members.clear();
            self.drop_self_from_rosters(None);
            self.flags_failed_at = None;
            self.sfx.play(SoundEvent::Leave);
            self.end_media_place();
        }
    }

    /// Drop the local user from sidebar rosters. `keep` is the channel they just joined.
    fn drop_self_from_rosters(&mut self, keep: Option<i64>) {
        let Some(me) = self.account.as_ref().map(|account| account.id) else {
            return;
        };
        let stale: Vec<i64> = self
            .voice_rosters
            .iter()
            .filter_map(|(channel_id, members)| {
                if Some(*channel_id) != keep && members.iter().any(|member| member.user_id == me) {
                    Some(*channel_id)
                } else {
                    None
                }
            })
            .collect();
        for channel_id in stale {
            let remaining = self
                .voice_rosters
                .get(&channel_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|member| member.user_id != me)
                .collect();
            apply_voice_roster(&mut self.voice_rosters, channel_id, remaining);
        }
    }

    fn end_media_place(&mut self) {
        self.call = None;
        self.camera = false;
        self.screen = false;
        self.share.stop();
        dismiss_stream_overlays(&mut self.share_open, &mut self.stream_menu);
        self.close_stream_watch();
    }

    fn stop_streaming(&mut self) {
        let was_live = self.share.is_live();
        self.share.stop();
        dismiss_stream_overlays(&mut self.share_open, &mut self.stream_menu);
        self.save_stream();
        if was_live {
            self.sfx.play(SoundEvent::StopStream);
        }
    }

    fn close_stream_watch(&mut self) {
        self.stream_popout = false;
        self.stream_fullscreen = false;
        self.focused_streamer = None;
        self.stream_popout_rgba = None;
    }

    fn open_share(&mut self) {
        self.share_open = true;
        self.reload_share_sources();
        self.sync_share_cover_focus();
    }

    fn refresh_cameras(&mut self) {
        self.cameras = media::camera_names();
        self.cameras_loaded = true;
        if !self
            .cameras
            .iter()
            .any(|(index, _)| *index == self.camera_choice.device_index())
        {
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
            "mic",
            "mic-off",
            "headphones",
            "headphone-off",
            "video",
            "video-off",
            "monitor",
            "monitor-off",
            "phone-off",
            "settings",
            "chevron-down",
            "chevron-up",
            "chevron-left",
            "plus",
            "hash",
            "volume-2",
            "users",
            "search",
            "paperclip",
            "x",
            "user",
            "log-out",
            "pencil",
            "trash-2",
            "reply",
            "check",
            "chevron-right",
            "circle-plus",
            "folder",
            "folder-plus",
            "message-circle",
            "phone",
            "user-plus",
            "signal",
            "ellipsis",
            "minus",
            "home",
            "image",
            "file-text",
            "sticky-note",
            "circle",
            "square",
            "type",
            "mouse-pointer-2",
            "maximize",
            "minimize",
            "picture-in-picture-2",
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
                self.settings_popup_open = ui
                    .is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP)
                    || ui.is_any_item_active();
                ui.set_cursor_pos([layout.nav_x, 0.0]);
                let nav_bg = ui.push_style_color(StyleColor::ChildBg, SIDEBAR_BG);
                ui.child_window("settings-nav")
                    .size([layout.nav_w, size[1]])
                    .build(ui, || {
                        self.draw_settings_nav(ui, layout.nav_w);
                    });
                drop(nav_bg);
                ui.set_cursor_pos([layout.body_x, 0.0]);
                let body_bg = ui.push_style_color(StyleColor::ChildBg, STAGE_BG);
                ui.child_window("settings-body")
                    .size([layout.body_w, size[1]])
                    .build(ui, || {
                        let window_pos = ui.window_pos();
                        ui.set_cursor_pos([layout.content_x, SETTINGS_TOP_PAD]);
                        ui.indent_by(layout.content_x);
                        let _wrap = ui.push_text_wrap_pos(layout.content_x + layout.content_w);
                        match self.settings.section() {
                            SettingsSection::Account => {
                                self.draw_account_settings(ui, layout.content_w)
                            }
                            SettingsSection::Voice => {
                                self.draw_voice_settings(ui, layout.content_w)
                            }
                            SettingsSection::Mods => self.draw_mods_settings(ui, layout.content_w),
                            SettingsSection::Developer => {
                                self.draw_developer_settings(ui, layout.content_w)
                            }
                        }
                        ui.dummy([layout.content_w, 80.0]);
                        ui.unindent_by(layout.content_x);
                        // Screen position, so the button stays put while the page scrolls.
                        ui.set_cursor_screen_pos([
                            window_pos[0] + layout.close_x - layout.body_x,
                            window_pos[1] + layout.close_y,
                        ]);
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
        let list_x = settings_nav_list_x(nav_w);
        let list_w = settings_nav_list_w(nav_w);
        ui.set_cursor_pos([list_x, SETTINGS_TOP_PAD]);
        ui.indent_by(list_x);
        ui.child_window("settings-nav-list")
            .size([list_w, 0.0])
            .build(ui, || {
                settings_form_label(ui, self.bold_font, "USER SETTINGS");
                ui.dummy([0.0, SETTINGS_NAV_HEADING_GAP]);
                for section in [SettingsSection::Account, SettingsSection::Voice] {
                    let (id, icon, label) = match section {
                        SettingsSection::Account => ("settings-account", "user", "My Account"),
                        SettingsSection::Voice => ("settings-voice", "headphones", "Voice & Video"),
                        SettingsSection::Mods => unreachable!("mods is under its own heading"),
                        SettingsSection::Developer => {
                            unreachable!("developer is under its own heading")
                        }
                    };
                    if nav_row(
                        ui,
                        id,
                        self.icon(icon),
                        label,
                        self.settings.section() == section,
                    ) {
                        self.select_settings(section);
                    }
                    ui.dummy([0.0, 4.0]);
                }
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                settings_divider(ui, ui.content_region_avail()[0]);
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                settings_form_label(ui, self.bold_font, "MODS");
                ui.dummy([0.0, SETTINGS_NAV_HEADING_GAP]);
                if nav_row(
                    ui,
                    "settings-mods",
                    self.icon("puzzle"),
                    "Mods",
                    self.settings.section() == SettingsSection::Mods,
                ) {
                    self.select_settings(SettingsSection::Mods);
                }
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                settings_divider(ui, ui.content_region_avail()[0]);
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                settings_form_label(ui, self.bold_font, "DEVELOPER");
                ui.dummy([0.0, SETTINGS_NAV_HEADING_GAP]);
                if nav_row(
                    ui,
                    "settings-developer",
                    self.icon("signal"),
                    "Developer",
                    self.settings.section() == SettingsSection::Developer,
                ) {
                    self.select_settings(SettingsSection::Developer);
                }
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                settings_divider(ui, ui.content_region_avail()[0]);
                ui.dummy([0.0, SETTINGS_NAV_SECTION_GAP]);
                if nav_row(
                    ui,
                    "settings-logout",
                    self.icon("log-out"),
                    "Log Out",
                    false,
                ) {
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
            draw.add_circle(center, size * 0.5, ROW_HOVER)
                .filled(true)
                .build();
        }
        draw.add_circle(center, size * 0.5 - 1.0, ring)
            .thickness(2.0)
            .build();
        if let Some(texture) = self.icon("x") {
            draw.add_image(
                texture,
                [center[0] - 9.0, center[1] - 9.0],
                [center[0] + 9.0, center[1] + 9.0],
                [0.0, 0.0],
                [1.0, 1.0],
                ring,
            );
        }
        let _font = ui.push_font_with_size(self.bold_font, 13.0);
        let label_w = ui.calc_text_size("ESC")[0];
        draw.add_text(
            [center[0] - label_w * 0.5, origin[1] + size + 6.0],
            MUTED,
            "ESC",
        );
        clicked
    }

    fn draw_mods_settings(&mut self, ui: &Ui, width: f32) {
        self.mods.reload();
        settings_page_title(ui, self.bold_font, "Mods");
        ui.text_colored(
            MUTED,
            "Mods run on this computer only. Enabled mods run from top to bottom. The last one wins a shared shell color.",
        );
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "FOLDER");
        ui.text(self.data_dir.join("mods").display().to_string());
        ui.text_colored(
            MUTED,
            "Each mod is a folder with banter.mod.json. This page reloads when you open it.",
        );
        settings_spacer(ui);

        let rows = self.mods.rows();
        if rows.is_empty() {
            ui.text_colored(MUTED, "No mods installed.");
            return;
        }
        let enabled: Vec<_> = rows.iter().filter(|row| row.enabled).cloned().collect();
        let available: Vec<_> = rows.iter().filter(|row| !row.enabled).cloned().collect();

        settings_form_label(ui, self.bold_font, "LOAD ORDER");
        if enabled.is_empty() {
            ui.text_colored(MUTED, "No mods are enabled.");
        }
        for (index, row) in enabled.iter().enumerate() {
            self.draw_mod_card(ui, width, row, Some((index, enabled.len())));
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "INSTALLED");
        if available.is_empty() {
            ui.text_colored(MUTED, "Every installed mod is in the load order.");
        }
        for row in &available {
            self.draw_mod_card(ui, width, row, None);
        }
        if !self.settings_error.is_empty() {
            settings_spacer(ui);
            ui.text_colored(DANGER, &self.settings_error);
        }
    }

    fn draw_mod_card(
        &mut self,
        ui: &Ui,
        width: f32,
        row: &crate::mods::ModRow,
        place: Option<(usize, usize)>,
    ) {
        let _title = ui.push_font_with_size(self.semibold_font, FONT_MD);
        ui.text(&row.name);
        drop(_title);
        let detail = format!("{} · {} MB", row.id, row.memory_mb);
        ui.text_colored(MUTED, detail);
        if let Some(reason) = &row.failed {
            ui.text_colored(DANGER, format!("This mod did not load. {reason}"));
        }
        if let Some(hard) = &row.hard {
            ui.text_colored(DANGER, format!("Stopped: {hard}"));
        }
        for soft in &row.soft {
            ui.text_colored(DANGER, soft);
        }
        if let Some(warning) = &row.warning {
            ui.text_colored(MUTED, warning);
        }
        if row.failed.is_none() {
            let mut on = row.enabled;
            if settings_toggle_row(
                ui,
                &format!("mod-enabled-{}", row.id),
                "Enabled",
                &mut on,
                width,
            ) {
                let before = self.mods.rows().iter().filter(|item| item.enabled).count();
                self.mods.set_enabled(&row.id, on);
                let after = self.mods.rows().iter().filter(|item| item.enabled).count();
                if on && after == before {
                    self.settings_error = "32 mods are already enabled.".into();
                } else {
                    self.settings_error.clear();
                }
            }
        }
        if row.enabled && row.failed.is_none() {
            let mut unlocked = row.unlocked;
            if settings_toggle_row(
                ui,
                &format!("mod-unlock-{}", row.id),
                "Unlock process rate",
                &mut unlocked,
                width,
            ) {
                self.mods.set_unlocked(&row.id, unlocked);
            }
            ui.text_colored(MUTED, "Off stops this mod after 1,000,000 instructions or 50 ms. On applies only on this computer.");
            if let Some((index, len)) = place {
                ui.dummy([0.0, 4.0]);
                if index > 0 && settings_text_button(ui, &format!("mod-up-{}", row.id), "Move up") {
                    self.mods.move_by(&row.id, true);
                }
                if index > 0 && index + 1 < len {
                    ui.same_line();
                }
                if index + 1 < len
                    && settings_text_button(ui, &format!("mod-down-{}", row.id), "Move down")
                {
                    self.mods.move_by(&row.id, false);
                }
            }
        }
        settings_divider(ui, width);
    }

    fn draw_developer_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "Developer");
        ui.text_colored(
            MUTED,
            "Choose which Banter server this client talks to. Changes apply without rebuilding.",
        );
        settings_spacer(ui);

        settings_form_label(ui, self.bold_font, "SERVER");
        let local_selected = self.dev_server.target == ServerTarget::Local;
        if settings_radio_row(ui, "dev-server-local", "Local", local_selected, width)
            && !local_selected
        {
            self.dev_server.target = ServerTarget::Local;
            self.apply_dev_server();
        }
        let remote_selected = self.dev_server.target == ServerTarget::Remote;
        if settings_radio_row(ui, "dev-server-remote", "Remote", remote_selected, width)
            && !remote_selected
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
            let _disabled =
                ui.begin_disabled_with_cond(self.dev_server.target != ServerTarget::Remote);
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
        settings_spacer(ui);

        settings_form_label(ui, self.bold_font, "VERSION");
        ui.text(&format!(
            "Client {} (protocol {})",
            crate::client_version(),
            crate::client_protocol()
        ));
        settings_spacer(ui);
        let server_line = match (self.server_version_label.as_str(), self.server_protocol) {
            ("", None) => "Server unknown (health not checked yet)".to_string(),
            ("", Some(protocol)) => format!("Server (no label, protocol {protocol})"),
            (label, None) => format!("Server {label} (no protocol — treat as too old)"),
            (label, Some(protocol)) => format!("Server {label} (protocol {protocol})"),
        };
        ui.text(&server_line);
        ui.text_colored(
            MUTED,
            &format!(
                "Client needs server protocol {}+.",
                crate::min_server_protocol()
            ),
        );
        if !self.version_warn.is_empty() {
            settings_spacer(ui);
            ui.text_colored(MUTED, &self.version_warn);
        }

        if !self.settings_error.is_empty() {
            settings_spacer(ui);
            ui.text_colored(RED, &self.settings_error);
        }
    }

    fn draw_account_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "My Account");
        ui.text_colored(
            MUTED,
            "Display name, username, and avatar for this Banter account.",
        );
        settings_spacer(ui);
        let Some(account) = self.account.clone() else {
            return;
        };
        let avatar = self.avatars.get(&account.id).map(|shot| shot.texture);

        // Profile card: banner, avatar, and name, then the editable fields on an inner panel.
        let card = ui.cursor_screen_pos();
        let banner_h = 100.0;
        let change_label = "Change Avatar";
        let remove_label = "Remove Avatar";
        let frame_pad = 24.0;
        let change_w = ui.calc_text_size(change_label)[0] + frame_pad;
        let remove_w = ui.calc_text_size(remove_label)[0] + frame_pad;
        let gap = 8.0;
        let name_left = card[0] + 124.0;
        let name_reserve = {
            let _font = ui.push_font_with_size(self.bold_font, 20.0);
            ui.calc_text_size(&account.display_name)[0]
                .min(width * 0.35)
                .max(48.0)
        };
        let place = account_avatar_actions_place(
            card[0],
            card[1],
            width,
            banner_h,
            16.0,
            name_left,
            name_reserve,
            change_w,
            remove_w,
            gap,
        );
        let header_h = if place.stacked { 112.0 } else { 72.0 };
        let fields_h = 190.0;
        let card_h = banner_h + header_h + fields_h + 16.0;
        let avatar_center = [card[0] + 16.0 + 46.0, card[1] + banner_h + 8.0];
        let panel = [card[0] + 16.0, card[1] + banner_h + header_h];
        let panel_w = width - 32.0;
        {
            let draw = ui.get_window_draw_list();
            draw.add_rect(card, [card[0] + width, card[1] + card_h], RAIL_BG)
                .rounding(8.0)
                .filled(true)
                .build();
            draw.add_rect(card, [card[0] + width, card[1] + banner_h], BLURPLE)
                .rounding(8.0)
                .flags(DrawFlags::ROUND_TOP)
                .filled(true)
                .build();
            draw.add_circle(avatar_center, 46.0, RAIL_BG)
                .filled(true)
                .build();
            draw.add_rect(panel, [panel[0] + panel_w, panel[1] + fields_h], SIDEBAR_BG)
                .rounding(8.0)
                .filled(true)
                .build();
            let _font = ui.push_font_with_size(self.bold_font, 20.0);
            draw.add_text(
                [card[0] + 124.0, card[1] + banner_h + 14.0],
                TEXT,
                &account.display_name,
            );
        }
        paint_avatar(
            ui,
            avatar,
            avatar_center,
            40.0,
            &initials(&account.display_name),
            None,
        );
        ui.set_cursor_screen_pos([place.x, place.y]);
        if ui.button(change_label) {
            self.status.clear();
            self.pick_avatar();
            if !self.status.is_empty() {
                self.settings_error = std::mem::take(&mut self.status);
            }
        }
        ui.same_line();
        if ui.button(remove_label) {
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
        settings_form_label(ui, self.bold_font, "CUSTOM STATUS");
        ui.set_next_item_width(width);
        ui.input_text("##account-status", &mut self.status_draft)
            .hint("What are you up to?")
            .build();
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "ABOUT ME");
        ui.input_text_multiline("##account-about", &mut self.about_draft, [width, 72.0])
            .build();
        ui.text_colored(
            MUTED,
            "Shown on your profile. Custom status also sits under your name.",
        );
        settings_spacer(ui);
        if ui.button("Save Profile") {
            let patch = ProfilePatch {
                about_me: Some(self.about_draft.clone()),
                custom_status: Some(self.status_draft.clone()),
                ..ProfilePatch::default()
            };
            match self.client.update_profile(&patch) {
                Ok(profile) => {
                    self.about_draft = profile.about_me.clone().unwrap_or_default();
                    self.status_draft = profile.custom_status.clone().unwrap_or_default();
                    if let Some(id) = self.account.as_ref().map(|account| account.id) {
                        let state = self
                            .presence
                            .get(&id)
                            .map(|info| info.state)
                            .unwrap_or(Presence::Online);
                        let custom_status = profile
                            .custom_status
                            .clone()
                            .filter(|text| !text.trim().is_empty());
                        self.presence.insert(
                            id,
                            PresenceInfo {
                                state,
                                custom_status,
                            },
                        );
                    }
                    self.settings_error.clear();
                }
                Err(err) => self.settings_error = err.to_string(),
            }
        }
        settings_spacer(ui);
        if !self.settings_error.is_empty() {
            ui.text_colored(RED, &self.settings_error);
        }
    }

    fn draw_voice_settings(&mut self, ui: &Ui, width: f32) {
        settings_page_title(ui, self.bold_font, "Voice & Video");
        ui.text_colored(
            MUTED,
            "Devices, volumes, input mode, and camera for voice and video.",
        );
        settings_spacer(ui);
        self.ensure_audio_devices();
        let gap = 16.0;
        let two_columns = width >= 460.0;
        let column_w = if two_columns {
            (width - gap) * 0.5
        } else {
            width
        };
        let row = ui.cursor_pos();

        settings_form_label(ui, self.bold_font, "INPUT DEVICE");
        self.draw_named_combo(
            ui,
            "input-device",
            "microphone",
            column_w,
            &self.audio_inputs.clone(),
            true,
        );
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
                self.draw_named_combo(
                    ui,
                    "output-device",
                    "speaker",
                    column_w,
                    &self.audio_outputs.clone(),
                    false,
                );
                settings_spacer(ui);
                settings_form_label(ui, self.bold_font, "OUTPUT VOLUME");
                if settings_volume_slider(
                    ui,
                    "output-volume",
                    &mut self.voice.output_volume,
                    column_w,
                ) {
                    self.save_voice();
                }
            });
            ui.set_cursor_pos([row[0], left_end.max(ui.cursor_pos()[1])]);
        } else {
            settings_spacer(ui);
            settings_form_label(ui, self.bold_font, "OUTPUT DEVICE");
            self.draw_named_combo(
                ui,
                "output-device",
                "speaker",
                column_w,
                &self.audio_outputs.clone(),
                false,
            );
            settings_spacer(ui);
            settings_form_label(ui, self.bold_font, "OUTPUT VOLUME");
            if settings_volume_slider(ui, "output-volume", &mut self.voice.output_volume, column_w)
            {
                self.save_voice();
            }
        }
        settings_spacer(ui);
        let device_errors = self
            .media
            .as_ref()
            .map(|media| media.errors())
            .unwrap_or_default();
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
        let test_label = if testing {
            "Stop Testing"
        } else {
            "Let's Check"
        };
        let test_w = ui.calc_text_size("Stop Testing")[0] + 24.0;
        let test_color =
            ui.push_style_color(StyleColor::Button, if testing { RED } else { BLURPLE });
        let pressed = ui.button_with_size(test_label, [test_w, 0.0]);
        drop(test_color);
        if pressed {
            self.settings.toggle_mic_test();
        }
        ui.same_line_with_spacing(0.0, 12.0);
        let meter_w = (width - test_w - 12.0).max(40.0);
        let threshold = if self.voice.auto_sensitivity || self.push_to_talk {
            None
        } else {
            Some(self.voice.sensitivity)
        };
        self.draw_input_meter(ui, meter_w, 36.0, threshold);
        settings_spacer(ui);
        settings_divider(ui, width);

        settings_form_label(ui, self.bold_font, "INPUT MODE");
        if settings_radio_row(
            ui,
            "mode-activity",
            "Voice Activity",
            !self.push_to_talk,
            width,
        ) && self.push_to_talk
        {
            self.push_to_talk = false;
            self.voice.push_to_talk = false;
            self.ptt_until = None;
            self.settings.finish_recording();
            self.save_voice();
        }
        if settings_radio_row(ui, "mode-ptt", "Push to Talk", self.push_to_talk, width)
            && !self.push_to_talk
        {
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

        if settings_panel(ui, "Guild voice video") {
            if settings_toggle_row(
                ui,
                "guild-p2p-video",
                "Prefer P2P video (1:1 voice channels)",
                &mut self.voice.guild_p2p_video,
                width,
            ) {
                self.save_voice();
            }
            ui.text_colored(
                MUTED,
                "Off by default. When both people enable this and you are alone together in a voice channel, screen and camera mesh peer-to-peer instead of through the SFU. Groups of three or more always use the SFU.",
            );
            settings_spacer(ui);
        }
        if settings_panel(ui, "Voice processing") {
            if settings_toggle_row(
                ui,
                "echo-cancellation",
                "Echo Cancellation",
                &mut self.voice.echo_cancellation,
                width,
            ) {
                self.save_voice();
            }
            settings_divider(ui, width);
            if settings_toggle_row(
                ui,
                "noise-suppression",
                "Noise Suppression",
                &mut self.voice.noise_suppression,
                width,
            ) {
                self.save_voice();
            }
            settings_divider(ui, width);
            if settings_toggle_row(
                ui,
                "auto-gain",
                "Automatic Gain Control",
                &mut self.voice.auto_gain,
                width,
            ) {
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
            self.draw_input_meter(
                ui,
                slider_track_width(width),
                12.0,
                Some(self.voice.sensitivity),
            );
            ui.text_colored(
                MUTED,
                "Your voice is sent when the bar passes the white mark.",
            );
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
        let button_label = if recording {
            "Stop Recording"
        } else {
            "Edit Keybind"
        };
        let button_w = ui.calc_text_size("Stop Recording")[0] + 24.0;
        let clicked = ui.invisible_button("ptt-keybind", [width, height]);
        let hovered = ui.is_item_hovered();
        let draw = ui.get_window_draw_list();
        let outline = if recording {
            RED
        } else if hovered {
            MUTED
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };
        draw.add_rect(origin, [origin[0] + width, origin[1] + height], RAIL_BG)
            .rounding(4.0)
            .filled(true)
            .build();
        draw.add_rect(origin, [origin[0] + width, origin[1] + height], outline)
            .rounding(4.0)
            .thickness(1.0)
            .build();
        let key_text = if recording {
            "Press a keyΓÇª".to_string()
        } else {
            self.voice.ptt_key.clone()
        };
        let text_h = ui.calc_text_size(&key_text)[1];
        draw.add_text(
            [origin[0] + 12.0, origin[1] + (height - text_h) * 0.5],
            if recording { RED } else { TEXT },
            &key_text,
        );
        let button_min = [origin[0] + width - button_w - 4.0, origin[1] + 4.0];
        let button_max = [origin[0] + width - 4.0, origin[1] + height - 4.0];
        draw.add_rect(
            button_min,
            button_max,
            if recording { RED } else { ROW_SELECTED },
        )
        .rounding(4.0)
        .filled(true)
        .build();
        let label_size = ui.calc_text_size(button_label);
        draw.add_text(
            [
                button_min[0] + (button_w - label_size[0]) * 0.5,
                origin[1] + (height - label_size[1]) * 0.5,
            ],
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
        draw.add_rect(origin, [origin[0] + box_w, origin[1] + box_h], RAIL_BG)
            .rounding(8.0)
            .filled(true)
            .build();
        if let Some(shot) = &self.local_camera {
            let fitted = fit_inside(shot.size, box_w, box_h);
            let min = [
                origin[0] + (box_w - fitted[0]) * 0.5,
                origin[1] + (box_h - fitted[1]) * 0.5,
            ];
            draw.add_image(
                shot.texture,
                min,
                [min[0] + fitted[0], min[1] + fitted[1]],
                [0.0, 0.0],
                [1.0, 1.0],
                WHITE,
            );
        } else {
            let text = if self.cameras.is_empty() {
                "No camera"
            } else {
                "Starting cameraΓÇª"
            };
            let size = ui.calc_text_size(text);
            draw.add_text(
                [
                    origin[0] + (box_w - size[0]) * 0.5,
                    origin[1] + (box_h - size[1]) * 0.5,
                ],
                MUTED,
                text,
            );
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
                (
                    format!("fps-{}", fps_key(*fps)),
                    stream_fps_label(*fps).to_string(),
                    self.stream_fps == *fps,
                    blocked.is_none(),
                )
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
        let scanned = media::list_available_encoders_preferring(Some(self.stream.encoder.as_str()));
        let current = encoder_mode_from_key(&self.stream.encoder);
        let choices: Vec<(String, String, bool, bool)> = scanned
            .iter()
            .map(|entry| {
                let mode = encoder_mode_from_key(entry.key);
                (
                    format!("enc-{}", entry.key),
                    entry.label.to_string(),
                    current == mode,
                    entry.available,
                )
            })
            .collect();
        if let Some(index) = settings_chip_row(ui, &choices, width) {
            let next = scanned[index].key.to_string();
            if scanned[index].available && self.stream.encoder != next {
                self.stream.encoder = next;
                self.save_stream();
            }
        }
        let found = media::hardware_encoder_names();
        let current_available = scanned
            .iter()
            .find(|entry| encoder_mode_from_key(entry.key) == current)
            .map(|entry| entry.available)
            .unwrap_or(true);
        if let Some(reason) = missing_encoder_block(current, current_available) {
            ui.text_colored(MUTED, reason);
        } else if let Some(reason) = hardware_encoder_block(self.stream.hardware) {
            ui.text_colored(MUTED, reason);
        } else {
            ui.text_colored(MUTED, encoder_availability_note(&found));
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "RATE CONTROL");
        let rates = [StreamRateControl::Cbr, StreamRateControl::Vbr];
        let current_rate = rate_control_from_key(&self.stream.rate_control);
        let rate_choices: Vec<(String, String, bool, bool)> = rates
            .iter()
            .map(|mode| {
                (
                    format!("rc-{}", rate_control_key(*mode)),
                    rate_control_label(*mode).to_string(),
                    current_rate == *mode,
                    true,
                )
            })
            .collect();
        if let Some(index) = settings_chip_row(ui, &rate_choices, width) {
            let next = rate_control_key(rates[index]).to_string();
            if self.stream.rate_control != next {
                self.stream.rate_control = next;
                self.save_stream();
            }
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "BITRATE");
        let mut auto = self.stream.bitrate_auto;
        if settings_toggle_row(ui, "bitrate-auto", "Automatic bitrate", &mut auto, width) {
            self.stream.bitrate_auto = auto;
            self.save_stream();
        }
        if !self.stream.bitrate_auto {
            let mut kbps = self.stream.bitrate_kbps as f32;
            if ui.slider_f32("Bitrate (kbps)", &mut kbps, 200.0, 40_000.0) {
                self.stream.bitrate_kbps = kbps.round() as u32;
            }
            if ui.is_item_deactivated_after_edit() {
                self.save_stream();
            }
            ui.text_colored(
                MUTED,
                format!("Target ~{} kbps on the full layer.", self.stream.bitrate_kbps),
            );
        } else {
            ui.text_colored(MUTED, "Auto targets ~2500 kbps on the full layer.");
        }
        settings_spacer(ui);
        settings_form_label(ui, self.bold_font, "KEYFRAME INTERVAL");
        let mut secs = self.stream.keyframe_secs.clamp(1, 10) as f32;
        if ui.slider_f32("Keyframe (seconds)", &mut secs, 1.0, 10.0) {
            self.stream.keyframe_secs = secs.round() as u32;
        }
        if ui.is_item_deactivated_after_edit() {
            self.save_stream();
        }
        ui.text_colored(
            MUTED,
            "Force an IDR this often while Go Live / camera is publishing (OBS-style).",
        );
    }

    /// Full-bleed Screen Share picker: soft fade, cover-flow rising from the bottom,
    /// and a frosted glass stream-settings chip at the bottom middle.
    fn draw_share(&mut self, ui: &Ui) {
        let display = ui.io().display_size();
        let dt = ui.io().delta_time();
        self.share_present = coverflow_step_present(self.share_present, self.share_open, dt);
        let alpha = coverflow_present_alpha(self.share_present);
        if alpha < 0.01 {
            return;
        }
        let rise = coverflow_rise_offset(self.share_present, COVERFLOW_RISE_PX);
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
            .focused(self.share_open)
            .build(|| {
                skin::rect(
                    ui,
                    [0.0, 0.0],
                    display,
                    with_alpha(SHARE_BACKDROP, SHARE_BACKDROP[3] * alpha),
                    0.0,
                );

                let semi = self.semibold_font;
                let tab_y = 18.0 + rise * 0.15;
                let screens_w = skin::text_size(ui, semi, FONT_SM, "Screens")[0] + 8.0;
                let tabs_x = (display[0] - (screens_w + 16.0 + 120.0)) * 0.5;
                if self.share_tab_label(
                    ui,
                    "share-screens",
                    "Screens",
                    self.share_tab == 0,
                    [tabs_x, tab_y],
                ) && self.share_tab != 0
                    && self.share_open
                {
                    self.share_tab = 0;
                    self.reload_share_sources();
                    self.sync_share_cover_focus();
                }
                if self.share_tab_label(
                    ui,
                    "share-apps",
                    "Applications",
                    self.share_tab == 1,
                    [tabs_x + screens_w + 16.0, tab_y],
                ) && self.share_tab != 1
                    && self.share_open
                {
                    self.share_tab = 1;
                    self.reload_share_sources();
                    self.sync_share_cover_focus();
                }
                if skin::icon_button(
                    ui,
                    "close-share",
                    self.icon("x"),
                    [display[0] - SHARE_PAD - 28.0, 14.0],
                    28.0,
                    18.0,
                    with_alpha(TEXT_SECONDARY, alpha),
                    with_alpha(TEXT_NORMAL, alpha),
                    Some(with_alpha(ICON_HOVER_BG, alpha)),
                ) && self.share_open
                {
                    self.share_open = false;
                }

                let chip_h = if self.share_chip_expanded {
                    SHARE_CHIP_H_EXPANDED.min(display[1] * 0.42)
                } else {
                    SHARE_CHIP_H_COLLAPSED
                };
                let cover_h = SHARE_COVER_H
                    .min(display[1] - SHARE_HEADER_H - chip_h - SHARE_CHIP_MARGIN_BOTTOM - 48.0)
                    .max(220.0);
                let cover_origin = [
                    SHARE_PAD,
                    ((display[1] - chip_h - SHARE_CHIP_MARGIN_BOTTOM - cover_h) * 0.42).max(56.0)
                        + rise,
                ];
                let cover_area = [display[0] - SHARE_PAD * 2.0, cover_h];
                self.draw_share_coverflow(ui, cover_origin, cover_area, alpha);

                self.draw_share_settings_chip(ui, display, chip_h, alpha);

                self.queue_next_thumb();
            });
    }

    /// Frosted glass stream-settings chip anchored at the bottom middle of the picker.
    fn draw_share_settings_chip(&mut self, ui: &Ui, display: [f32; 2], chip_h: f32, alpha: f32) {
        let semi = self.semibold_font;
        let chip_w = SHARE_CHIP_W.min(display[0] - 48.0).max(320.0);
        let min = [
            ((display[0] - chip_w) * 0.5).floor(),
            (display[1] - SHARE_CHIP_MARGIN_BOTTOM - chip_h).floor(),
        ];
        let max = [min[0] + chip_w, min[1] + chip_h];
        skin::glass_chip(ui, min, max, SHARE_CHIP_RADIUS, alpha);

        let inner_w = chip_w - SHARE_CHIP_PAD * 2.0;
        let header_h = 36.0;
        ui.set_cursor_screen_pos([min[0] + SHARE_CHIP_PAD, min[1] + 10.0]);
        let summary = format!(
            "Stream settings · {} · {}",
            stream_resolution_label(self.stream_resolution),
            stream_fps_label(self.stream_fps)
        );
        let shown = skin::ellipsize(ui, semi, FONT_SM, &summary, inner_w - 36.0);
        if skin::button(
            ui,
            "share-chip-toggle",
            if self.share_chip_expanded { "▾" } else { "▴" },
            Tone::Link,
            [max[0] - SHARE_CHIP_PAD - 28.0, min[1] + 8.0],
            [28.0, 28.0],
            self.share_open,
            semi,
        ) && self.share_open
        {
            self.share_chip_expanded = !self.share_chip_expanded;
        }
        skin::text(
            ui,
            semi,
            FONT_SM,
            [min[0] + SHARE_CHIP_PAD, min[1] + 14.0],
            with_alpha(TEXT, alpha),
            &shown,
        );

        let actions_y = max[1] - SHARE_CHIP_PAD - BUTTON_H;
        if skin::button(
            ui,
            "share-cancel",
            "Cancel",
            Tone::Link,
            [min[0] + SHARE_CHIP_PAD - 4.0, actions_y],
            [72.0, BUTTON_H],
            self.share_open,
            semi,
        ) && self.share_open
        {
            self.share_open = false;
        }
        let mut right = max[0] - SHARE_CHIP_PAD;
        if self.share.is_live() {
            let stop_w =
                (skin::text_size(ui, semi, FONT_SM, "Stop")[0] + 28.0).max(88.0);
            right -= stop_w;
            if skin::button(
                ui,
                "share-stop",
                "Stop",
                Tone::Danger,
                [right, actions_y],
                [stop_w, BUTTON_H],
                self.share_open,
                semi,
            ) && self.share_open
            {
                self.stop_streaming();
            }
            right -= 8.0;
        }
        let go_w = (skin::text_size(ui, semi, FONT_SM, "Go Live")[0] + 32.0).max(112.0);
        right -= go_w;
        let can_go = self.share_open && self.share.selected().is_some();
        if skin::button(
            ui,
            "share-go-live",
            "Go Live",
            Tone::Green,
            [right, actions_y],
            [go_w, BUTTON_H],
            can_go,
            semi,
        ) && self.share_open
        {
            if self.share.go_live() {
                self.share_open = false;
                self.status.clear();
                self.sfx.play(SoundEvent::StartStream);
            } else {
                self.status = "choose a screen or an application first".into();
            }
        }

        if !self.status.is_empty() {
            skin::text(
                ui,
                None,
                FONT_XS,
                [min[0] + SHARE_CHIP_PAD + 76.0, actions_y + 10.0],
                with_alpha(ERROR_TEXT, alpha),
                &self.status,
            );
        }

        if !self.share_chip_expanded {
            return;
        }
        let body_top = min[1] + header_h + 4.0;
        let body_h = (actions_y - 8.0 - body_top).max(40.0);
        ui.set_cursor_screen_pos([min[0] + SHARE_CHIP_PAD, body_top]);
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _gap = ui.push_style_var(StyleVar::ItemSpacing([0.0, 4.0]));
        let _bg = ui.push_style_color(StyleColor::ChildBg, skin::CLEAR);
        ui.child_window("share-glass-body")
            .size([inner_w, body_h])
            .build(ui, || {
                let share_w = ui.content_region_avail()[0];
                self.draw_stream_quality(ui, share_w);
                settings_spacer(ui);
                self.draw_audio_sources(ui, share_w);
            });
    }

    /// Underline tab like Discord's Screens / Applications row.
    fn share_tab_label(
        &self,
        ui: &Ui,
        id: &str,
        label: &str,
        selected: bool,
        min: [f32; 2],
    ) -> bool {
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
        skin::text_in_row(
            ui,
            semi,
            FONT_SM,
            min[0] + 4.0,
            min[1],
            SHARE_TAB_H - 2.0,
            color,
            label,
        );
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

    /// Arched cover-flow of tall portrait source previews; wheel / arrows / click cycle focus.
    fn draw_share_coverflow(&mut self, ui: &Ui, origin: [f32; 2], area: [f32; 2], alpha: f32) {
        let count = self.share_sources.len();
        ui.set_cursor_screen_pos(origin);
        let hit = ui.invisible_button("share-coverflow", area) && self.share_open;
        let hovered = ui.is_item_hovered() && self.share_open;
        let max = [origin[0] + area[0], origin[1] + area[1]];
        skin::rect(
            ui,
            origin,
            max,
            with_alpha(CALL_BG, 0.28 * alpha),
            SHARE_TILE_RADIUS,
        );

        if count == 0 {
            let empty = if self.share_tab == 0 {
                "No screens were found."
            } else {
                "No application windows were found."
            };
            let tw = skin::text_size(ui, None, FONT_SM, empty)[0];
            skin::text(
                ui,
                None,
                FONT_SM,
                [
                    origin[0] + (area[0] - tw) * 0.5,
                    origin[1] + area[1] * 0.5 - 8.0,
                ],
                with_alpha(MUTED, alpha),
                empty,
            );
            return;
        }

        let dt = ui.io().delta_time();
        if hovered {
            let wheel = ui.io().mouse_wheel();
            if wheel.abs() > 0.0 {
                // ImGui: scroll up is positive; treat that as earlier sources (left).
                self.share_cover_target =
                    coverflow_nudge_focus(self.share_cover_target, -wheel, count);
            }
        }
        if self.share_open && !ui.io().want_capture_keyboard() {
            if ui.is_key_pressed(Key::LeftArrow) {
                self.share_cover_target =
                    coverflow_nudge_focus(self.share_cover_target, -1.0, count);
            }
            if ui.is_key_pressed(Key::RightArrow) {
                self.share_cover_target =
                    coverflow_nudge_focus(self.share_cover_target, 1.0, count);
            }
        }
        self.share_cover_focus =
            coverflow_step_focus(self.share_cover_focus, self.share_cover_target, dt);

        if self.share_open {
            if let Some(nearest) = coverflow_nearest(self.share_cover_focus, count) {
                if (self.share_cover_focus - nearest as f32).abs() < 0.08 {
                    self.apply_share_cover_index(nearest);
                }
            }
        }

        let mut panels = coverflow_layout(count, self.share_cover_focus, origin, area);
        panels.sort_by_key(|panel| panel.depth);
        let mouse = ui.io().mouse_pos();
        let mut clicked_index = None;
        let mut hover_name: Option<(String, [f32; 2], [f32; 2])> = None;
        for panel in &panels {
            let source = &self.share_sources[panel.index];
            let key = source_key(source.kind, source.id);
            let offset = panel.index as f32 - self.share_cover_focus;
            let bright = coverflow_brightness(offset);
            skin::rect(
                ui,
                panel.min,
                panel.max,
                with_alpha(SHARE_COVER_EMPTY, SHARE_COVER_EMPTY[3] * alpha),
                SHARE_TILE_RADIUS,
            );
            let w = panel.max[0] - panel.min[0];
            let h = panel.max[1] - panel.min[1];
            let tint = [bright, bright, bright, alpha];
            if let Some(shot) = self.share_thumbs.get(&key) {
                let (uv0, uv1) = coverflow_cover_uv(shot.size, w, h);
                ui.get_window_draw_list().add_image_rounded(
                    shot.texture,
                    panel.min,
                    panel.max,
                    uv0,
                    uv1,
                    tint,
                    SHARE_TILE_RADIUS,
                    DrawFlags::ROUND_ALL,
                );
            }
            // Edge panels dim further so the arch reads against dark chrome (ref GIF).
            if bright < 0.98 {
                skin::rect(
                    ui,
                    panel.min,
                    panel.max,
                    with_alpha(CALL_BG, (1.0 - bright) * 0.55 * alpha),
                    SHARE_TILE_RADIUS,
                );
            }
            if panel.focused {
                ui.get_window_draw_list()
                    .add_rect(
                        panel.min,
                        panel.max,
                        with_alpha(SHARE_FOCUS_BORDER, SHARE_FOCUS_BORDER[3] * alpha),
                    )
                    .rounding(SHARE_TILE_RADIUS)
                    .thickness(COVERFLOW_FOCUS_BORDER)
                    .build();
            }
            let under = mouse[0] >= panel.min[0]
                && mouse[0] <= panel.max[0]
                && mouse[1] >= panel.min[1]
                && mouse[1] <= panel.max[1];
            if under {
                hover_name = Some((source.name.clone(), panel.min, panel.max));
                if hit {
                    clicked_index = Some(panel.index);
                }
            }
        }
        if let Some(index) = clicked_index {
            self.share_cover_target = index as f32;
            self.apply_share_cover_index(index);
        }
        // Reference has no on-panel titles; Banter still needs the source name on hover.
        if let Some((name, amin, amax)) = hover_name {
            if hovered {
                skin::tooltip(
                    ui,
                    self.semibold_font,
                    FONT_SM,
                    &name,
                    amin,
                    amax,
                    crate::theme::TooltipSide::Top,
                );
            }
        }
    }

    fn apply_share_cover_index(&mut self, index: usize) {
        let Some(source) = self.share_sources.get(index) else {
            return;
        };
        let kind = source.kind;
        let id = source.id;
        if self.share.selected() == Some((kind, id)) {
            return;
        }
        if self.share.is_live() {
            self.share.apply_while_live(kind, id);
        } else {
            self.share.select(kind, id);
        }
        self.save_stream();
    }

    fn sync_share_cover_focus(&mut self) {
        let count = self.share_sources.len();
        if count == 0 {
            self.share_cover_focus = 0.0;
            self.share_cover_target = 0.0;
            return;
        }
        let index = self
            .share
            .selected()
            .and_then(|(kind, id)| {
                self.share_sources
                    .iter()
                    .position(|source| source.kind == kind && source.id == id)
            })
            .unwrap_or(0)
            .min(count - 1);
        self.share_cover_focus = index as f32;
        self.share_cover_target = index as f32;
    }

    fn reload_share_sources(&mut self) {
        self.retired_shots
            .extend(self.share_thumbs.drain().map(|(_, shot)| shot));
        self.share_miss.clear();
        self.thumb_queue.clear();
        self.share_thumb_next = 0;
        self.share_sources = if self.share_tab == 0 {
            list_screens()
        } else {
            list_windows()
        };
        if self.share_cover_target >= self.share_sources.len() as f32 {
            let max = self.share_sources.len().saturating_sub(1) as f32;
            self.share_cover_target = max;
            self.share_cover_focus = max;
        }
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
    fn draw_voice_member(&self, ui: &Ui, channel_id: i64, member: &VoiceMember) {
        let name = self.name_of(member.user_id);
        let width = ui.content_region_avail()[0].max(32.0);
        let origin = ui.cursor_screen_pos();
        ui.invisible_button(
            format!("voice-member-{channel_id}-{}", member.user_id),
            [width, VOICE_MEMBER_H],
        );
        let hovered = ui.is_item_hovered();
        let min = [origin[0] + 32.0, origin[1] + 2.0];
        let max = [
            origin[0] + width - ROW_INSET,
            origin[1] + VOICE_MEMBER_H - 2.0,
        ];
        if hovered {
            skin::rect(ui, min, max, ROW_HOVER, ROW_RADIUS);
        }
        let center = [
            min[0] + 4.0 + VOICE_AVATAR * 0.5,
            origin[1] + VOICE_MEMBER_H * 0.5,
        ];
        let avatar = self.avatars.get(&member.user_id).map(|shot| shot.texture);
        paint_avatar(
            ui,
            avatar,
            center,
            VOICE_AVATAR * 0.5,
            &initials(&name),
            None,
        );
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
            let icon = if member.flags.deafened {
                "headphone-off"
            } else {
                "mic-off"
            };
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
            skin::rect(
                ui,
                live_min,
                [live_min[0] + live_w, live_min[1] + 16.0],
                RED,
                8.0,
            );
            skin::text_in_row(
                ui,
                self.bold_font,
                11.0,
                live_min[0] + 6.0,
                live_min[1],
                16.0,
                WHITE,
                "LIVE",
            );
            icons_w += live_w + 4.0;
        }
        let text_x = center[0] + VOICE_AVATAR * 0.5 + 8.0;
        let color = if hovered { TEXT_NORMAL } else { MUTED };
        let shown = skin::ellipsize(
            ui,
            None,
            FONT_SM,
            &name,
            (max[0] - 4.0 - icons_w - text_x).max(16.0),
        );
        skin::text_in_row(
            ui,
            None,
            FONT_SM,
            text_x,
            origin[1],
            VOICE_MEMBER_H,
            color,
            &shown,
        );
    }

    fn draw_featured_screen_at(&mut self, ui: &Ui, origin: [f32; 2], width: f32, height: f32) {
        let max = [origin[0] + width, origin[1] + height];
        skin::rect(ui, origin, max, TILE_BG, TILE_RADIUS);
        let Some(user_id) = self.focused_live_streamer() else {
            return;
        };
        let name = self.name_of(user_id);
        if let Some((texture, size)) = self.stream_texture_for(user_id) {
            let shown = if self.screen_shown[0] > 1.0 {
                self.screen_shown
            } else {
                size
            };
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
        }
        // Theater mode draws its own fading chrome on top of the shell.
        if self.stream_fullscreen {
            return;
        }
        self.name_tag(
            ui,
            [origin[0] + 8.0, max[1] - 8.0 - 24.0],
            &name,
            None,
            true,
        );
        self.draw_stream_watch_controls(ui, origin, width, 1.0);
        let me = self.account.as_ref().map(|account| account.id);
        if Some(user_id) != me {
            self.quality_control(
                ui,
                user_id,
                MediaKind::Screen,
                [origin[0] + 8.0, origin[1] + 8.0],
            );
        }
    }

    fn live_streamer_ids(&self) -> Vec<i64> {
        let me = self.account.as_ref().map(|account| account.id);
        let members: Vec<(i64, bool)> = if self.voice_channel.is_some() {
            self.voice_members
                .iter()
                .map(|member| (member.user_id, member.flags.screen))
                .collect()
        } else if let Some(call) = &self.call {
            let peer = if Some(call.caller) == me {
                call.callee
            } else {
                call.caller
            };
            let mut rows = vec![(peer, false)];
            if let Some(me) = me {
                rows.push((me, self.screen));
            }
            // Remote screen flag is not on DM call members; infer from remote video / voice_members.
            for member in &self.voice_members {
                if let Some(row) = rows.iter_mut().find(|(id, _)| *id == member.user_id) {
                    row.1 = member.flags.screen;
                } else {
                    rows.push((member.user_id, member.flags.screen));
                }
            }
            // Any remote screen texture counts as live even if the roster flag lags.
            for ((user_id, code), _) in &self.remote_video {
                if *code == 3 {
                    if let Some(row) = rows.iter_mut().find(|(id, _)| *id == *user_id) {
                        row.1 = true;
                    } else {
                        rows.push((*user_id, true));
                    }
                }
            }
            rows
        } else {
            self.voice_members
                .iter()
                .map(|member| (member.user_id, member.flags.screen))
                .collect()
        };
        streaming_user_ids(self.screen, me, &members)
    }

    fn focused_live_streamer(&self) -> Option<i64> {
        resolve_focused_streamer(self.focused_streamer, &self.live_streamer_ids())
    }

    fn has_live_stream(&self) -> bool {
        !self.live_streamer_ids().is_empty()
    }

    fn stream_texture_for(&self, user_id: i64) -> Option<(ManagedTextureId, [f32; 2])> {
        let me = self.account.as_ref().map(|account| account.id);
        if Some(user_id) == me {
            return self
                .local_screen
                .as_ref()
                .map(|shot| (shot.texture, shot.size));
        }
        self.remote_video
            .get(&(user_id, 3))
            .map(|shot| (shot.texture, shot.size))
    }

    fn maybe_keep_stream_popout_frame(&mut self, frame: &VideoFrame) {
        if !self.stream_popout || frame.kind != MediaKind::Screen {
            return;
        }
        let Some((width, height, rgba)) = frame.rgba.as_ref() else {
            return;
        };
        let me = self.account.as_ref().map(|account| account.id);
        let focus = self.focused_live_streamer();
        let frame_user = frame.sender.or(me);
        if frame_user == focus {
            self.stream_popout_rgba = Some((*width, *height, rgba.clone()));
        }
    }

    fn cycle_focused_streamer(&mut self) {
        let live = self.live_streamer_ids();
        self.focused_streamer = cycle_streamer(self.focused_streamer, &live);
        self.stream_pointer_at = Instant::now();
    }

    /// Top-right watch controls on the featured stream: cycle, pop-out, fullscreen.
    fn draw_stream_watch_controls(&mut self, ui: &Ui, origin: [f32; 2], width: f32, alpha: f32) {
        let live = self.live_streamer_ids();
        let mut buttons: Vec<(&str, &str, &str)> = Vec::new();
        if live.len() > 1 {
            buttons.push(("stream-cycle", "chevron-right", "Next Stream"));
        }
        buttons.push((
            "stream-popout",
            "picture-in-picture-2",
            if self.stream_popout {
                "Close Pop Out"
            } else {
                "Pop Out"
            },
        ));
        buttons.push((
            "stream-fullscreen",
            if self.stream_fullscreen {
                "minimize"
            } else {
                "maximize"
            },
            if self.stream_fullscreen {
                "Exit Fullscreen"
            } else {
                "Fullscreen"
            },
        ));
        let total_w = buttons.len() as f32 * STREAM_WATCH_BTN
            + buttons.len().saturating_sub(1) as f32 * STREAM_WATCH_BTN_GAP;
        let mut x = origin[0] + width - STREAM_WATCH_PAD - total_w;
        let y = origin[1] + STREAM_WATCH_PAD;
        let fill = [
            CONTROL_BG[0],
            CONTROL_BG[1],
            CONTROL_BG[2],
            CONTROL_BG[3] * alpha,
        ];
        let hover = [
            CONTROL_HOVER[0],
            CONTROL_HOVER[1],
            CONTROL_HOVER[2],
            CONTROL_HOVER[3] * alpha,
        ];
        let tint = [WHITE[0], WHITE[1], WHITE[2], alpha];
        let semi = self.semibold_font;
        for (id, icon, tip) in buttons {
            ui.set_cursor_screen_pos([x, y]);
            let clicked = ui.invisible_button(id, [STREAM_WATCH_BTN, STREAM_WATCH_BTN]);
            let color = if ui.is_item_hovered() { hover } else { fill };
            skin::rect(
                ui,
                [x, y],
                [x + STREAM_WATCH_BTN, y + STREAM_WATCH_BTN],
                color,
                STREAM_WATCH_BTN * 0.5,
            );
            let inset = (STREAM_WATCH_BTN - STREAM_WATCH_ICON) * 0.5;
            skin::image(
                ui,
                self.icon(icon),
                [x + inset, y + inset],
                STREAM_WATCH_ICON,
                tint,
            );
            if alpha > 0.5 {
                skin::item_tooltip(ui, semi, tip);
            }
            if clicked {
                match id {
                    "stream-cycle" => self.cycle_focused_streamer(),
                    "stream-popout" => {
                        self.stream_popout = !self.stream_popout;
                        if !self.stream_popout {
                            self.stream_popout_rgba = None;
                        }
                        self.stream_pointer_at = Instant::now();
                    }
                    "stream-fullscreen" => {
                        self.stream_fullscreen = !self.stream_fullscreen;
                        self.stream_pointer_at = Instant::now();
                    }
                    _ => {}
                }
            }
            x += STREAM_WATCH_BTN + STREAM_WATCH_BTN_GAP;
        }
    }

    /// Full-window theater view for the focused stream; chrome fades after pointer idle.
    fn draw_stream_theater(&mut self, ui: &Ui) {
        let display = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE
            | WindowFlags::NO_NAV;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, skin::CLEAR);
        ui.window("##stream-theater")
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .build(|| {
                let origin = [0.0, 0.0];
                skin::rect(ui, origin, display, CALL_BG, 0.0);
                let Some(user_id) = self.focused_live_streamer() else {
                    return;
                };
                if let Some((texture, size)) = self.stream_texture_for(user_id) {
                    let shown = if self.screen_shown[0] > 1.0 {
                        self.screen_shown
                    } else {
                        size
                    };
                    let fitted = fit_inside(shown, display[0], display[1]);
                    let x = (display[0] - fitted[0]) * 0.5;
                    let y = (display[1] - fitted[1]) * 0.5;
                    ui.get_window_draw_list().add_image(
                        texture,
                        [x, y],
                        [x + fitted[0], y + fitted[1]],
                        [0.0, 0.0],
                        [1.0, 1.0],
                        WHITE,
                    );
                }
                let chrome =
                    stream_chrome_alpha(self.stream_pointer_at.elapsed().as_secs_f32(), true);
                if chrome > STREAM_CHROME_HIDE_ALPHA {
                    self.draw_stream_watch_people(
                        ui,
                        [
                            STREAM_WATCH_PAD,
                            display[1] - STREAM_WATCH_PAD - STREAM_WATCH_AVATAR,
                        ],
                        chrome,
                    );
                    self.draw_stream_watch_controls(ui, origin, display[0], chrome);
                }
            });
    }

    /// Banter Stream OS window content (own ImGui context / textures).
    pub(crate) fn draw_stream_popout_window(&mut self, ui: &Ui) {
        let size = ui.io().display_size();
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SCROLL_WITH_MOUSE;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _bg = ui.push_style_color(StyleColor::WindowBg, CALL_BG);
        ui.window(STREAM_POPOUT_WINDOW_TITLE)
            .flags(flags)
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                let origin = [0.0, 0.0];
                skin::rect(ui, origin, size, CALL_BG, 0.0);
                if let Some(shot) = self.stream_popout_shot.as_ref() {
                    let fitted = fit_inside(shot.size, size[0], size[1]);
                    let x = (size[0] - fitted[0]) * 0.5;
                    let y = (size[1] - fitted[1]) * 0.5;
                    ui.get_window_draw_list().add_image(
                        shot.texture,
                        [x, y],
                        [x + fitted[0], y + fitted[1]],
                        [0.0, 0.0],
                        [1.0, 1.0],
                        WHITE,
                    );
                }
                let chrome =
                    stream_chrome_alpha(self.stream_pointer_at.elapsed().as_secs_f32(), true);
                if chrome > STREAM_CHROME_HIDE_ALPHA {
                    self.draw_stream_watch_people(
                        ui,
                        [
                            STREAM_WATCH_PAD,
                            size[1] - STREAM_WATCH_PAD - STREAM_WATCH_AVATAR,
                        ],
                        chrome,
                    );
                    // Pop-out window: cycle + close pop-out (no nested OS fullscreen here).
                    let live = self.live_streamer_ids();
                    let mut x = size[0] - STREAM_WATCH_PAD - STREAM_WATCH_BTN;
                    if live.len() > 1 {
                        x -= STREAM_WATCH_BTN + STREAM_WATCH_BTN_GAP;
                    }
                    let y = STREAM_WATCH_PAD;
                    let fill = [
                        CONTROL_BG[0],
                        CONTROL_BG[1],
                        CONTROL_BG[2],
                        CONTROL_BG[3] * chrome,
                    ];
                    let hover = [
                        CONTROL_HOVER[0],
                        CONTROL_HOVER[1],
                        CONTROL_HOVER[2],
                        CONTROL_HOVER[3] * chrome,
                    ];
                    let tint = [WHITE[0], WHITE[1], WHITE[2], chrome];
                    if live.len() > 1 {
                        ui.set_cursor_screen_pos([x, y]);
                        if ui.invisible_button("popout-cycle", [STREAM_WATCH_BTN, STREAM_WATCH_BTN])
                        {
                            self.cycle_focused_streamer();
                        }
                        let color = if ui.is_item_hovered() { hover } else { fill };
                        skin::rect(
                            ui,
                            [x, y],
                            [x + STREAM_WATCH_BTN, y + STREAM_WATCH_BTN],
                            color,
                            STREAM_WATCH_BTN * 0.5,
                        );
                        let inset = (STREAM_WATCH_BTN - STREAM_WATCH_ICON) * 0.5;
                        skin::image(
                            ui,
                            self.icon("chevron-right"),
                            [x + inset, y + inset],
                            STREAM_WATCH_ICON,
                            tint,
                        );
                        x += STREAM_WATCH_BTN + STREAM_WATCH_BTN_GAP;
                    }
                    ui.set_cursor_screen_pos([x, y]);
                    if ui.invisible_button("popout-close", [STREAM_WATCH_BTN, STREAM_WATCH_BTN]) {
                        self.stream_popout = false;
                        self.stream_popout_rgba = None;
                    }
                    let color = if ui.is_item_hovered() { hover } else { fill };
                    skin::rect(
                        ui,
                        [x, y],
                        [x + STREAM_WATCH_BTN, y + STREAM_WATCH_BTN],
                        color,
                        STREAM_WATCH_BTN * 0.5,
                    );
                    let inset = (STREAM_WATCH_BTN - STREAM_WATCH_ICON) * 0.5;
                    skin::image(
                        ui,
                        self.icon("x"),
                        [x + inset, y + inset],
                        STREAM_WATCH_ICON,
                        tint,
                    );
                }
            });
    }

    pub(crate) fn prepare_stream_popout(&mut self, imgui: &mut Context) {
        if !self.stream_popout_styled {
            apply_discord_style(imgui.style_mut());
            self.stream_popout_styled = true;
        }
        if !self.stream_popout_icons_ready {
            for name in [
                "chevron-right",
                "x",
                "picture-in-picture-2",
                "maximize",
                "minimize",
                "mic-off",
                "headphone-off",
            ] {
                if let Ok(rgba) = rasterize_lucide(name, 32, [255, 255, 255]) {
                    if let Some(shot) = upload_rgba(imgui, 32, 32, &rgba) {
                        self.stream_popout_icons.insert(name.to_string(), shot);
                    }
                }
            }
            self.stream_popout_icons_ready = true;
        }
        // Flush only retired textures — do not process share thumbs on this context.
        for shot in self.retired_shots.drain(..) {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.install_avatars(imgui);
        for shot in self.retired_shots.drain(..) {
            let _ = imgui.remove_texture(shot.texture);
        }
        if let Some((width, height, rgba)) = self.stream_popout_rgba.take() {
            if let Some(shot) = upload_rgba(imgui, width, height, &rgba) {
                replace_shot(imgui, &mut self.stream_popout_shot, shot);
            }
        }
    }

    pub(crate) fn begin_stream_popout_surface(
        &mut self,
        bold: Option<FontId>,
        semibold: Option<FontId>,
        italic: Option<FontId>,
        bold_italic: Option<FontId>,
    ) {
        assert!(
            self.stream_popout_font_backup.is_none(),
            "begin_stream_popout_surface must not nest"
        );
        self.stream_popout_font_backup = Some((
            self.bold_font,
            self.semibold_font,
            self.italic_font,
            self.bold_italic_font,
        ));
        self.bold_font = bold;
        self.semibold_font = semibold;
        self.italic_font = italic;
        self.bold_italic_font = bold_italic;
        std::mem::swap(&mut self.icons, &mut self.stream_popout_icons);
        std::mem::swap(&mut self.icons_ready, &mut self.stream_popout_icons_ready);
        std::mem::swap(&mut self.avatars, &mut self.stream_popout_avatars);
        std::mem::swap(&mut self.avatar_rev, &mut self.stream_popout_avatar_rev);
        std::mem::swap(&mut self.avatar_miss, &mut self.stream_popout_avatar_miss);
    }

    pub(crate) fn end_stream_popout_surface(&mut self) {
        let Some((bold, semibold, italic, bold_italic)) = self.stream_popout_font_backup.take()
        else {
            panic!("end_stream_popout_surface without begin_stream_popout_surface");
        };
        self.bold_font = bold;
        self.semibold_font = semibold;
        self.italic_font = italic;
        self.bold_italic_font = bold_italic;
        std::mem::swap(&mut self.icons, &mut self.stream_popout_icons);
        std::mem::swap(&mut self.icons_ready, &mut self.stream_popout_icons_ready);
        std::mem::swap(&mut self.avatars, &mut self.stream_popout_avatars);
        std::mem::swap(&mut self.avatar_rev, &mut self.stream_popout_avatar_rev);
        std::mem::swap(&mut self.avatar_miss, &mut self.stream_popout_avatar_miss);
    }

    pub(crate) fn release_stream_popout_gpu(&mut self, imgui: &mut Context) {
        for (_, shot) in self.stream_popout_icons.drain() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.stream_popout_icons_ready = false;
        for (_, shot) in self.stream_popout_avatars.drain() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.stream_popout_avatar_rev.clear();
        self.stream_popout_avatar_miss.clear();
        if let Some(shot) = self.stream_popout_shot.take() {
            let _ = imgui.remove_texture(shot.texture);
        }
        self.stream_popout_rgba = None;
        self.stream_popout_styled = false;
        self.stream_popout_font_backup = None;
    }

    /// Bottom-left stream-watch chrome: profile icons + talking rings / mute badges for live sharers.
    ///
    /// Fades with the same idle alpha as the rest of theater / pop-out chrome. Clicking an
    /// avatar focuses that streamer when more than one person is sharing.
    fn draw_stream_watch_people(&mut self, ui: &Ui, origin: [f32; 2], alpha: f32) {
        let live = self.live_streamer_ids();
        if live.is_empty() {
            return;
        }
        let focus = self.focused_live_streamer();
        let radius = STREAM_WATCH_AVATAR * 0.5;
        let mut focus_index = 0usize;
        for (index, &user_id) in live.iter().enumerate() {
            let x = stream_watch_avatar_x(origin[0], index);
            let center = [x + radius, origin[1] + radius];
            let name = self.name_of(user_id);
            let (talking, state_icon) = self.stream_watch_voice_state(user_id);
            ui.set_cursor_screen_pos([x, origin[1]]);
            let id = format!("stream-watch-avatar-{user_id}");
            if ui.invisible_button(&id, [STREAM_WATCH_AVATAR, STREAM_WATCH_AVATAR])
                && Some(user_id) != focus
            {
                self.focused_streamer = Some(user_id);
                self.stream_pointer_at = Instant::now();
            }
            let avatar = self.avatars.get(&user_id).map(|shot| shot.texture);
            let tint = [WHITE[0], WHITE[1], WHITE[2], alpha];
            let fill = [BLURPLE[0], BLURPLE[1], BLURPLE[2], BLURPLE[3] * alpha];
            paint_avatar_tinted(ui, avatar, center, radius, &initials(&name), fill, tint);
            if talking {
                let ring = [GREEN[0], GREEN[1], GREEN[2], GREEN[3] * alpha];
                ui.get_window_draw_list()
                    .add_circle(center, radius + 2.0, ring)
                    .thickness(2.0)
                    .build();
            }
            if let Some(icon) = state_icon {
                let badge = STREAM_WATCH_BADGE;
                let bx = center[0] + radius - badge * 0.55;
                let by = center[1] + radius - badge * 0.55;
                skin::rect(
                    ui,
                    [bx, by],
                    [bx + badge, by + badge],
                    [0.0, 0.0, 0.0, 0.75 * alpha],
                    badge * 0.5,
                );
                let inset = (badge - 10.0) * 0.5;
                skin::image(
                    ui,
                    self.icon(icon),
                    [bx + inset, by + inset],
                    10.0,
                    [MUTED[0], MUTED[1], MUTED[2], alpha],
                );
            }
            if Some(user_id) == focus {
                focus_index = index;
            }
        }
        if let Some(user_id) = focus {
            let name = self.name_of(user_id);
            let tag_x = stream_watch_avatar_x(origin[0], focus_index) + STREAM_WATCH_AVATAR + 8.0;
            let tag_y = origin[1] + (STREAM_WATCH_AVATAR - 24.0) * 0.5;
            // Mute/deafen is on the avatar badge; the tag keeps LIVE + name only.
            self.name_tag_alpha(ui, [tag_x, tag_y], &name, None, true, alpha);
        }
    }

    /// Talking ring + mute/deafen icon for a live streamer on watch chrome.
    fn stream_watch_voice_state(&self, user_id: i64) -> (bool, Option<&'static str>) {
        let me = self.account.as_ref().map(|account| account.id);
        let flags = if Some(user_id) == me {
            self.flags
        } else if let Some(member) = self
            .voice_members
            .iter()
            .find(|member| member.user_id == user_id)
        {
            member.flags
        } else {
            VoiceFlags::default()
        };
        let member = VoiceMember {
            user_id,
            flags,
            prefer_p2p_video: false,
        };
        let talking = self.member_talking(&member);
        let icon = if flags.deafened {
            Some("headphone-off")
        } else if flags.muted {
            Some("mic-off")
        } else {
            None
        };
        (talking, icon)
    }

    /// Bottom-left tile label: name, then a mute or deafen icon, and a red LIVE badge for a stream.
    fn name_tag(&self, ui: &Ui, min: [f32; 2], name: &str, state_icon: Option<&str>, live: bool) {
        self.name_tag_alpha(ui, min, name, state_icon, live, 1.0);
    }

    fn name_tag_alpha(
        &self,
        ui: &Ui,
        min: [f32; 2],
        name: &str,
        state_icon: Option<&str>,
        live: bool,
        alpha: f32,
    ) {
        let semi = self.semibold_font;
        let name_w = skin::text_size(ui, semi, FONT_SM, name)[0];
        let live_w = if live {
            skin::text_size(ui, self.bold_font, 11.0, "LIVE")[0] + 12.0
        } else {
            0.0
        };
        let icon_w = if state_icon.is_some() { 20.0 } else { 0.0 };
        let tag_w = 8.0 + name_w + 8.0 + icon_w + if live { live_w + 6.0 } else { 0.0 };
        let bg = [0.0, 0.0, 0.0, 0.6 * alpha];
        let text = [WHITE[0], WHITE[1], WHITE[2], alpha];
        let live_fill = [RED[0], RED[1], RED[2], RED[3] * alpha];
        skin::rect(ui, min, [min[0] + tag_w, min[1] + 24.0], bg, 4.0);
        let mut x = min[0] + 8.0;
        if live {
            skin::rect(
                ui,
                [x, min[1] + 4.0],
                [x + live_w, min[1] + 20.0],
                live_fill,
                4.0,
            );
            skin::text_in_row(
                ui,
                self.bold_font,
                11.0,
                x + 6.0,
                min[1] + 4.0,
                16.0,
                text,
                "LIVE",
            );
            x += live_w + 6.0;
        }
        skin::text_in_row(ui, semi, FONT_SM, x, min[1], 24.0, text, name);
        if let Some(icon) = state_icon {
            skin::image(
                ui,
                self.icon(icon),
                [x + name_w + 6.0, min[1] + 4.0],
                16.0,
                text,
            );
        }
    }

    fn draw_voice_tile_at(
        &mut self,
        ui: &Ui,
        member: &VoiceMember,
        origin: [f32; 2],
        tile_w: f32,
        tile_h: f32,
    ) {
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
                .add_rect(
                    [origin[0] + 1.0, origin[1] + 1.0],
                    [max[0] - 1.0, max[1] - 1.0],
                    GREEN,
                )
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
        self.name_tag(
            ui,
            [origin[0] + 8.0, max[1] - 8.0 - 24.0],
            &name,
            state_icon,
            false,
        );
        let me = self.account.as_ref().map(|account| account.id);
        if member.flags.camera && Some(member.user_id) != me {
            self.quality_control(
                ui,
                member.user_id,
                MediaKind::Camera,
                [origin[0] + 8.0, origin[1] + 8.0],
            );
        }
    }

    fn quality_control(&mut self, ui: &Ui, publisher: i64, track: MediaKind, origin: [f32; 2]) {
        let key = (publisher, track);
        let open = self.quality_for == Some(key);
        let chosen = self.layers.get(publisher, track);
        ui.set_cursor_screen_pos(origin);
        let label = format!(
            "{}##quality-open-{}-{}",
            layer_word(chosen),
            publisher,
            track.code()
        );
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
                    format!(
                        "{} (showing {})",
                        layer_word(snapped),
                        layer_word(delivered)
                    )
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
        } else if self
            .call
            .as_ref()
            .is_some_and(|call| call.phase == CallPhase::Active)
        {
            self.call
                .as_ref()
                .map(|call| MediaPlace::DirectCall(call.dm_id))
        } else {
            None
        };
        let Some(place) = place else { return };
        let Some(realtime) = &self.realtime else {
            return;
        };
        realtime.send(ClientSignal::SubscribeLayer {
            place,
            publisher,
            track,
            max_layer: layer,
        });
    }

    /// Re-subscribe to peers who are Go Live / camera so the SFU PLIs an IDR.
    /// Covers late join (publisher already mid-GOP) and receive stalls with no tiles.
    fn ensure_remote_stream_subs(&mut self) {
        if self.stream_resub_at.elapsed() < Duration::from_secs(2) {
            return;
        }
        let me = self.account.as_ref().map(|account| account.id);
        let mut sent = false;
        for member in &self.voice_members {
            if Some(member.user_id) == me {
                continue;
            }
            if member.flags.screen {
                let layer = self.layers.get(member.user_id, MediaKind::Screen);
                self.send_subscribe(member.user_id, MediaKind::Screen, layer);
                sent = true;
            }
            if member.flags.camera {
                let layer = self.layers.get(member.user_id, MediaKind::Camera);
                self.send_subscribe(member.user_id, MediaKind::Camera, layer);
                sent = true;
            }
        }
        if sent {
            self.stream_resub_at = Instant::now();
        }
    }

    fn tile_camera(
        &self,
        member: &VoiceMember,
        me: Option<i64>,
    ) -> Option<(ManagedTextureId, [f32; 2])> {
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
            self.stream.hardware = media::hardware_encoder_available();
        } else {
            self.stream.hardware = media::hardware_encoder_available();
        }
        if let Some(dev) = load_json::<DevServerPrefs>(&self.data_dir.join("dev-server.json")) {
            self.dev_server = dev;
        }
        if let Some(home) = load_json::<HomeLayout>(&self.data_dir.join("home.json")) {
            self.home_layout = HomeLayout::normalize(home);
        }
    }

    fn save_home_layout(&self) {
        let _ = save_json(&self.data_dir.join("home.json"), &self.home_layout);
    }

    fn save_voice(&mut self) {
        let mut voice = self.voice.clone();
        voice.push_to_talk = self.push_to_talk;
        let _ = save_json(&self.data_dir.join("voice.json"), &voice);
        self.sync_sfx_output();
        self.sync_guild_p2p_video_pref();
    }

    /// Lower user id offers when guild P2P video is active; otherwise SFU (everyone) or DM caller.
    fn webrtc_offerer(&self, me: i64) -> bool {
        if self.guild_p2p_video_active() {
            let peer = self
                .voice_members
                .iter()
                .map(|member| member.user_id)
                .find(|id| *id != me)
                .unwrap_or(me);
            return me < peer;
        }
        self.voice_channel.is_some()
            || self.call.as_ref().is_some_and(|call| call.caller == me)
    }

    /// Local setting on, exactly two seated, and the peer also opted in (roster field).
    fn guild_p2p_video_active(&self) -> bool {
        if !self.voice.guild_p2p_video
            || self.voice_channel.is_none()
            || self.voice_members.len() != 2
        {
            return false;
        }
        let me = self.account.as_ref().map(|account| account.id);
        self.voice_members.iter().all(|member| {
            if Some(member.user_id) == me {
                true
            } else {
                member.prefer_p2p_video
            }
        })
    }

    fn sync_guild_p2p_video_pref(&mut self) {
        let enabled = self.voice.guild_p2p_video;
        if self.sent_guild_p2p_video == Some(enabled) {
            return;
        }
        let Some(realtime) = &self.realtime else {
            return;
        };
        realtime.send(ClientSignal::GuildP2pVideo { enabled });
        self.sent_guild_p2p_video = Some(enabled);
        if let Some(me) = self.account.as_ref().map(|account| account.id) {
            for member in &mut self.voice_members {
                if member.user_id == me {
                    member.prefer_p2p_video = enabled;
                }
            }
        }
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
        // Capability flag for the UI — wire encode is always H.264 (hardware when present, else OpenH264).
        self.stream.hardware = media::hardware_encoder_available();
        self.stream.encoder = encoder_mode_key(encoder_mode_from_key(&self.stream.encoder)).into();
        self.stream.rate_control =
            rate_control_key(rate_control_from_key(&self.stream.rate_control)).into();
        self.stream.keyframe_secs = self.stream.keyframe_secs.clamp(1, 10);
        self.stream.bitrate_kbps = self.stream.bitrate_kbps.clamp(200, 40_000);
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
            "Window size and sync. With VSync on, the UI follows the current monitor’s refresh rate. Turning it off can raise FPS; tearing may show up.",
        );
        settings_spacer(ui);
        if settings_toggle_row(ui, "video-vsync", "VSync", &mut self.video.vsync, width) {
            self.save_video();
        }
        settings_divider(ui, width);
        if settings_toggle_row(
            ui,
            "video-fullscreen",
            "Fullscreen",
            &mut self.video.fullscreen,
            width,
        ) {
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
            labels.push(format!(
                "{} (current)",
                window_size_label(self.video.width, self.video.height)
            ));
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
        self.clear_reconnect_state();
        self.voice_channel = None;
        self.voice_members.clear();
        self.voice_rosters.clear();
        self.end_media_place();
        self.media = None;
        self.realtime = None;

        let token = self.client.token().map(str::to_string);
        self.base = base.clone();
        self.client.set_base(&base);
        self.shell.invalidate();

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

        if !self.apply_version_check(true) {
            self.client.set_token(None);
            let _ = clear_token(&self.data_dir);
            self.account = None;
            self.display_name.clear();
            return;
        }

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
                    let _ = clear_token(&self.data_dir);
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

    /// Probe `/v1/health` and compare protocol floors. Returns false when the client must not stay connected.
    ///
    /// When `require_reachable` is false, a transport failure is ignored (startup may still be asking for a remote).
    fn apply_version_check(&mut self, require_reachable: bool) -> bool {
        match self.client.health() {
            Ok(report) => {
                self.server_version_label = report.version.clone().unwrap_or_default();
                self.server_protocol = report.protocol;
                match banter_server::check_remote_version(&report) {
                    banter_server::VersionCheck::Compatible { warn } => {
                        self.version_warn = warn.unwrap_or_default();
                        true
                    }
                    banter_server::VersionCheck::Block { code, message } => {
                        let notice = banter_server::format_version_notice(code, &message);
                        self.drop_session_for_version(&notice);
                        false
                    }
                }
            }
            Err(err) => {
                self.server_version_label.clear();
                self.server_protocol = None;
                self.version_warn.clear();
                if !require_reachable {
                    return true;
                }
                match banter_server::unreachable_version_check(err.to_string()) {
                    banter_server::VersionCheck::Block { code, message } => {
                        let notice = banter_server::format_version_notice(code, &message);
                        self.settings_error = notice.clone();
                        self.status = notice;
                        false
                    }
                    banter_server::VersionCheck::Compatible { .. } => true,
                }
            }
        }
    }

    fn drop_session_for_version(&mut self, notice: &str) {
        self.version_warn.clear();
        self.settings_error = notice.to_string();
        self.status = notice.to_string();
        self.clear_reconnect_state();
        self.voice_channel = None;
        self.voice_members.clear();
        self.voice_rosters.clear();
        self.end_media_place();
        self.media = None;
        self.realtime = None;
        self.shell.invalidate();
        if self.account.is_some() || self.client.token().is_some() {
            self.client.set_token(None);
            let _ = clear_token(&self.data_dir);
            self.account = None;
            self.display_name.clear();
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
        }
    }

    fn engine_update(&self) -> EngineUpdate {
        EngineUpdate {
            input_device: self.voice.input_device.clone(),
            output_device: self.voice.output_device.clone(),
            input_volume: self.voice.input_volume,
            output_volume: self.voice.output_volume,
            sensitivity: if self.voice.auto_sensitivity {
                OPEN_MIC_RMS
            } else {
                self.voice.sensitivity
            },
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
            ("A", Key::A),
            ("B", Key::B),
            ("C", Key::C),
            ("D", Key::D),
            ("E", Key::E),
            ("F", Key::F),
            ("G", Key::G),
            ("H", Key::H),
            ("I", Key::I),
            ("J", Key::J),
            ("K", Key::K),
            ("L", Key::L),
            ("M", Key::M),
            ("N", Key::N),
            ("O", Key::O),
            ("P", Key::P),
            ("Q", Key::Q),
            ("R", Key::R),
            ("S", Key::S),
            ("T", Key::T),
            ("U", Key::U),
            ("V", Key::V),
            ("W", Key::W),
            ("X", Key::X),
            ("Y", Key::Y),
            ("Z", Key::Z),
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
        if let Some(source) = self
            .stream
            .sources
            .iter_mut()
            .find(|source| source.id == id)
        {
            source.name = name;
            return;
        }
        self.stream.sources.push(MixSource {
            id,
            name,
            enabled: false,
            volume: 1.0,
        });
    }

    fn draw_named_combo(
        &mut self,
        ui: &Ui,
        id: &str,
        kind: &str,
        width: f32,
        devices: &[String],
        input: bool,
    ) {
        if devices.is_empty() {
            ui.text_colored(MUTED, format!("No {kind} is available."));
            return;
        }
        let mut labels = vec!["Default".to_string()];
        labels.extend(devices.iter().cloned());
        let current = if input {
            &self.voice.input_device
        } else {
            &self.voice.output_device
        };
        let mut index = devices
            .iter()
            .position(|name| name == current)
            .map(|index| index + 1)
            .unwrap_or(0);
        if settings_combo(ui, &format!("##{id}"), width, &labels, &mut index) {
            let chosen = if index == 0 {
                String::new()
            } else {
                devices[index - 1].clone()
            };
            if input {
                self.voice.input_device = chosen;
            } else {
                self.voice.output_device = chosen;
            }
            self.save_voice();
        }
    }

    fn draw_input_meter(&self, ui: &Ui, width: f32, height: f32, threshold: Option<f32>) {
        let rms = self
            .media
            .as_ref()
            .map(|media| media.input_rms())
            .unwrap_or(0.0);
        let level = meter_fraction(rms);
        let origin = ui.cursor_screen_pos();
        ui.dummy([width, height]);
        let bar_h = 8.0;
        let top = origin[1] + (height - bar_h) * 0.5;
        let draw = ui.get_window_draw_list();
        draw.add_rect([origin[0], top], [origin[0] + width, top + bar_h], RAIL_BG)
            .rounding(4.0)
            .filled(true)
            .build();
        if level > 0.0 {
            let color = match threshold {
                Some(gate) if rms < gate => LOW_LEVEL,
                _ => GREEN,
            };
            draw.add_rect(
                [origin[0], top],
                [origin[0] + width * level, top + bar_h],
                color,
            )
            .rounding(4.0)
            .filled(true)
            .build();
        }
        if let Some(gate) = threshold {
            // Same 5 px inset as the settings slider knob, so the mark sits under the knob.
            let mark_x = origin[0] + 5.0 + (width - 10.0).max(0.0) * meter_fraction(gate);
            draw.add_rect(
                [mark_x - 1.0, top - 4.0],
                [mark_x + 1.0, top + bar_h + 4.0],
                WHITE,
            )
            .filled(true)
            .build();
        }
    }

    fn draw_audio_sources(&mut self, ui: &Ui, width: f32) {
        self.ensure_audio_devices();
        settings_form_label(ui, self.bold_font, "STREAM AUDIO");
        ui.text_colored(
            MUTED,
            "Turn on every app or device to mix into the stream. Each one has its own volume.",
        );
        settings_spacer(ui);
        let groups = grouped_mix_sources(&self.stream.sources);
        if groups.is_empty() {
            ui.text_colored(MUTED, "No audio sources were found.");
            return;
        }
        for (group_index, (group, indices)) in groups.into_iter().enumerate() {
            if group_index > 0 {
                settings_spacer(ui);
            }
            settings_form_label(ui, self.bold_font, group.label());
            for index in indices {
                let mut enabled = self.stream.sources[index].enabled;
                let name = self.stream.sources[index].name.clone();
                let id = self.stream.sources[index].id.clone();
                if settings_toggle_row(ui, &format!("src-{id}"), &name, &mut enabled, width) {
                    self.stream.sources[index].enabled = enabled;
                    self.save_stream();
                }
                if enabled {
                    let mut volume = self.stream.sources[index].volume;
                    let release =
                        settings_volume_slider(ui, &format!("vol-{id}"), &mut volume, width);
                    self.stream.sources[index].volume = volume;
                    if release {
                        self.save_stream();
                    }
                    settings_spacer(ui);
                }
                settings_divider(ui, width);
            }
        }
    }

    fn draw_stream_menu(&mut self, ui: &Ui) {
        // Tall enough for resolution / FPS chips while live; stage chrome stays unchanged.
        let width = 320.0;
        let height = 420.0;
        let x = self.stream_anchor[0];
        let y = (self.stream_anchor[1] - height - 8.0).max(8.0);
        ui.window("Stream options")
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::NO_DOCKING,
            )
            .position([x, y], Condition::Always)
            .size([width, height], Condition::Always)
            .build(|| {
                ui.text("Stream options");
                ui.child_window("stream-options-body")
                    .size([0.0, -36.0])
                    .build(ui, || {
                        // Same chips as Voice & Video / Stream Select; finish_voice_flags applies live.
                        // Always visible so res/FPS are one click from the stage chevron while sharing.
                        let quality_w = ui.content_region_avail()[0];
                        self.draw_stream_quality(ui, quality_w);
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
                        self.stop_streaming();
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
                    "pref {}   rate {}   keyframe {}s   target {} kbps",
                    qos.encoder_pref, qos.rate_control, qos.keyframe_secs, qos.target_kbps
                ));
                if qos.encode_width > 0 {
                    ui.text(format!(
                        "encode {}x{} @ {} FPS",
                        qos.encode_width, qos.encode_height, qos.encode_fps
                    ));
                }
                ui.text(format!(
                    "{}   loss {:.1}%   rtt {} ms   jitter {} ms",
                    qos.step,
                    qos.loss * 100.0,
                    qos.rtt_ms,
                    qos.jitter_ms
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
                if !qos.remote_stream.is_empty() {
                    ui.text(format!("remote {}", qos.remote_stream));
                }
                ui.separator();
                ui.checkbox("ImGui Metrics", &mut self.profiler_metrics);
                ui.same_line();
                ui.checkbox("ImGui Demo", &mut self.profiler_demo);
                ui.separator();
                ui.text_colored(TEXT_SECONDARY, "Recent expensive ops");
                ui.child_window("profiler_ops")
                    .size([0.0, 0.0])
                    .build(ui, || {
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
            if let Some(source) = self
                .share_sources
                .iter()
                .find(|source| source.kind == kind && source.id == id)
            {
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
        self.screen_from = if self.screen_shown[0] > 1.0 {
            self.screen_shown
        } else {
            size
        };
        self.screen_target = size;
        self.screen_resize_at = Instant::now();
        self.screen_shown = self.screen_from;
    }

    fn member_talking(&self, member: &VoiceMember) -> bool {
        // Never draw a talking glow over a muted or deafened mic, even if a stale
        // transmitting bit slipped through before sanitize_voice_flags.
        if self.account.as_ref().map(|account| account.id) == Some(member.user_id) {
            self.flags.transmitting && !self.flags.muted && !self.flags.deafened
        } else {
            member.flags.transmitting && !member.flags.muted && !member.flags.deafened
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

fn encoder_mode_to_pick(mode: StreamEncoderMode) -> media::EncoderPick {
    match mode {
        StreamEncoderMode::Auto => media::EncoderPick::Auto,
        StreamEncoderMode::Hardware => media::EncoderPick::Hardware,
        StreamEncoderMode::Software => media::EncoderPick::Software,
        StreamEncoderMode::Nvenc => media::EncoderPick::Nvenc,
        StreamEncoderMode::Vaapi => media::EncoderPick::Vaapi,
        StreamEncoderMode::MediaFoundation => media::EncoderPick::MediaFoundation,
    }
}

fn format_uptime(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn format_log_clock(at_ms: i64) -> String {
    let secs = at_ms.div_euclid(1000).rem_euclid(86_400);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02} UTC")
}

fn parse_role_color(raw: &str) -> Result<i64, String> {
    let text = raw.trim().trim_start_matches('#');
    if text.is_empty() {
        return Ok(0);
    }
    if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("color is a 6 digit hex value".into());
    }
    i64::from_str_radix(text, 16).map_err(|_| "color is a 6 digit hex value".into())
}

fn mirrored_camera_rgba(imgui: &mut Context, width: u32, height: u32, rgba: &[u8]) -> Option<Shot> {
    let mirrored = mirror_rgba(width, height, rgba).ok()?;
    upload_rgba(imgui, width, height, &mirrored)
}

fn source_key(kind: CaptureKind, id: u32) -> (u8, u32) {
    let code = match kind {
        CaptureKind::Screen => 1,
        CaptureKind::Window => 2,
    };
    (code, id)
}

fn list_screens() -> Vec<ShareSource> {
    let Ok(monitors) = xcap::Monitor::all() else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for monitor in monitors {
        let Ok(id) = monitor.id() else { continue };
        let name = monitor.name().unwrap_or_else(|_| format!("Screen {id}"));
        let primary = monitor.is_primary().unwrap_or(false);
        let label = if primary {
            format!("{name} (Primary)")
        } else {
            name
        };
        let height = monitor.height().unwrap_or(0);
        let hz = monitor.frequency().unwrap_or(0.0).round() as u32;
        sources.push((
            primary,
            ShareSource {
                kind: CaptureKind::Screen,
                id,
                name: label,
                height,
                hz,
            },
        ));
    }
    sources.sort_by(|left, right| right.0.cmp(&left.0));
    sources.into_iter().map(|(_, source)| source).collect()
}

fn list_windows() -> Vec<ShareSource> {
    let Ok(windows) = xcap::Window::all() else {
        return Vec::new();
    };
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
            .map(|monitor| {
                (
                    monitor.height().unwrap_or(0),
                    monitor.frequency().unwrap_or(0.0).round() as u32,
                )
            })
            .unwrap_or((0, 0));
        sources.push(ShareSource {
            kind: CaptureKind::Window,
            id,
            name,
            height,
            hz,
        });
    }
    sources
}

fn capture_source_jpeg(kind: CaptureKind, id: u32) -> Option<Vec<u8>> {
    match kind {
        CaptureKind::Screen => {
            let monitor = xcap::Monitor::all()
                .ok()?
                .into_iter()
                .find(|monitor| monitor.id().ok() == Some(id))?;
            let image = monitor.capture_image().ok()?;
            encode_jpeg_rgba(image.width(), image.height(), image.as_raw(), 360).ok()
        }
        CaptureKind::Window => {
            let window = xcap::Window::all()
                .ok()?
                .into_iter()
                .find(|window| window.id().ok() == Some(id))?;
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

fn settings_text_button(ui: &Ui, id: &str, label: &str) -> bool {
    let origin = ui.cursor_screen_pos();
    let size = ui.calc_text_size(label);
    let clicked = ui.invisible_button(id, [size[0], 22.0]);
    let color = if ui.is_item_hovered() { WHITE } else { BLURPLE };
    ui.get_window_draw_list().add_text(
        [origin[0], origin[1] + (22.0 - size[1]) * 0.5],
        color,
        label,
    );
    clicked
}

fn settings_divider(ui: &Ui, width: f32) {
    let origin = ui.cursor_screen_pos();
    ui.dummy([width, 17.0]);
    ui.get_window_draw_list()
        .add_rect(
            [origin[0], origin[1] + 8.0],
            [origin[0] + width, origin[1] + 9.0],
            SETTINGS_DIVIDER,
        )
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
    draw.add_text(
        [origin[0], origin[1] + (height - text_h) * 0.5],
        TEXT,
        &shown,
    );
    let x = origin[0] + width - switch_w;
    let y = origin[1] + (height - switch_h) * 0.5;
    draw.add_rect(
        [x, y],
        [x + switch_w, y + switch_h],
        if *value { GREEN } else { SWITCH_OFF },
    )
    .rounding(switch_h * 0.5)
    .filled(true)
    .build();
    let knob_x = if *value {
        x + switch_w - switch_h * 0.5
    } else {
        x + switch_h * 0.5
    };
    draw.add_circle([knob_x, y + switch_h * 0.5], switch_h * 0.5 - 3.0, WHITE)
        .filled(true)
        .build();
    clicked
}

fn settings_radio_row(ui: &Ui, id: &str, label: &str, selected: bool, width: f32) -> bool {
    let origin = ui.cursor_screen_pos();
    let height = 44.0;
    let clicked = ui.invisible_button(id, [width, height]);
    let hovered = ui.is_item_hovered();
    let draw = ui.get_window_draw_list();
    let fill = if selected {
        ROW_SELECTED
    } else if hovered {
        ROW_HOVER
    } else {
        SIDEBAR_BG
    };
    draw.add_rect(origin, [origin[0] + width, origin[1] + height], fill)
        .rounding(4.0)
        .filled(true)
        .build();
    let center = [origin[0] + 22.0, origin[1] + height * 0.5];
    draw.add_circle(center, 10.0, if selected { WHITE } else { MUTED })
        .thickness(2.0)
        .build();
    if selected {
        draw.add_circle(center, 5.0, WHITE).filled(true).build();
    }
    let text_h = ui.calc_text_size(label)[1];
    draw.add_text(
        [origin[0] + 44.0, origin[1] + (height - text_h) * 0.5],
        if selected { WHITE } else { TEXT },
        label,
    );
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
            if ui
                .selectable_config(format!("{label}##{position}"))
                .selected(selected)
                .build()
            {
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

fn settings_slider_i32(
    ui: &Ui,
    id: &str,
    value: &mut i32,
    min: i32,
    max: i32,
    width: f32,
    unit: &str,
) -> bool {
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
    draw.add_rect(
        [item_min[0], center_y - 4.0],
        [item_min[0] + track_w, center_y + 4.0],
        SWITCH_OFF,
    )
    .rounding(4.0)
    .filled(true)
    .build();
    draw.add_rect(
        [item_min[0], center_y - 4.0],
        [knob_x, center_y + 4.0],
        BLURPLE,
    )
    .rounding(4.0)
    .filled(true)
    .build();
    draw.add_rect(
        [knob_x - 5.0, center_y - 12.0],
        [knob_x + 5.0, center_y + 12.0],
        if active || hovered { WHITE } else { TEXT },
    )
    .rounding(3.0)
    .filled(true)
    .build();
    let label = format!("{value}{unit}");
    let label_size = ui.calc_text_size(&label);
    draw.add_text(
        [
            item_min[0] + width - label_size[0],
            center_y - label_size[1] * 0.5,
        ],
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

fn settings_chip_row(
    ui: &Ui,
    choices: &[(String, String, bool, bool)],
    width: f32,
) -> Option<usize> {
    let spacing = 8.0;
    let widths: Vec<f32> = choices
        .iter()
        .map(|(_, label, _, _)| ui.calc_text_size(label)[0] + 28.0)
        .collect();
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
            ui.get_window_draw_list()
                .add_text([origin[0] + 14.0, origin[1] + 7.0], dim, label);
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
        draw.add_rect(
            [origin[0] + 8.0, origin[1]],
            [origin[0] + width - 8.0, origin[1] + 32.0],
            fill,
        )
        .rounding(4.0)
        .filled(true)
        .build();
    }
    let mut text_x = origin[0] + 16.0;
    let icon_tint = if selected { TEXT } else { MUTED };
    if let Some(texture) = icon {
        draw.add_image(
            texture,
            [text_x, origin[1] + 8.0],
            [text_x + 16.0, origin[1] + 24.0],
            [0.0, 0.0],
            [1.0, 1.0],
            icon_tint,
        );
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
    let fill = if selected {
        BLURPLE
    } else {
        [0.0, 0.0, 0.0, 0.0]
    };
    if selected || ui.is_item_hovered() {
        ui.get_window_draw_list()
            .add_rect(
                [origin[0], origin[1] + 2.0],
                [origin[0] + width, origin[1] + 30.0],
                if selected { fill } else { ROW_HOVER },
            )
            .rounding(4.0)
            .filled(true)
            .build();
    }
    let color = if selected { WHITE } else { MUTED };
    ui.get_window_draw_list()
        .add_text([origin[0] + 14.0, origin[1] + 7.0], color, label);
    clicked
}

enum AvatarStep {
    Upload(Vec<u8>),
    Miss,
    Want,
}

fn header_rule(ui: &Ui) {
    let pos = ui.window_pos();
    let size = ui.window_size();
    ui.get_window_draw_list()
        .add_rect(
            [pos[0], pos[1] + HEADER_H - 1.0],
            [pos[0] + size[0], pos[1] + HEADER_H],
            HEADER_SHADOW,
        )
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
    ui.get_window_draw_list().add_text(
        [origin[0] + 18.0, origin[1] + CATEGORY_H - height - 6.0],
        MUTED,
        &label,
    );
}

fn connection_kind_glyph(kind: &str) -> &'static str {
    match kind {
        "github" => "GH",
        "steam" => "ST",
        "twitch" => "TW",
        "youtube" => "YT",
        "twitter" => "X",
        "website" => "WEB",
        _ => "LINK",
    }
}

fn open_http_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("connection URL must be http(s)".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", trimmed])
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(trimmed)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())?;
        return Ok(());
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("opening links is not supported on this platform".into())
    }
}

fn truncate_ui(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

fn non_empty_opt(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_member_since(ms: i64) -> String {
    let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) else {
        return "Unknown".into();
    };
    let month = dt.format("%b").to_string();
    let day = dt
        .format("%d")
        .to_string()
        .trim_start_matches('0')
        .to_string();
    let year = dt.format("%Y").to_string();
    format!("{month} {day}, {year}")
}

fn paint_avatar(
    ui: &Ui,
    texture: Option<ManagedTextureId>,
    center: [f32; 2],
    radius: f32,
    letters: &str,
    ring: Option<[f32; 4]>,
) {
    paint_avatar_tinted(ui, texture, center, radius, letters, BLURPLE, WHITE);
    if let Some(color) = ring {
        ui.get_window_draw_list()
            .add_circle(center, radius + 3.0, color)
            .thickness(2.0)
            .build();
    }
}

fn paint_avatar_tinted(
    ui: &Ui,
    texture: Option<ManagedTextureId>,
    center: [f32; 2],
    radius: f32,
    letters: &str,
    fill: [f32; 4],
    tint: [f32; 4],
) {
    let draw = ui.get_window_draw_list();
    if let Some(texture) = texture {
        draw.add_image_rounded(
            texture,
            [center[0] - radius, center[1] - radius],
            [center[0] + radius, center[1] + radius],
            [0.0, 0.0],
            [1.0, 1.0],
            tint,
            radius,
            DrawFlags::ROUND_ALL,
        );
    } else {
        draw.add_circle(center, radius, fill).filled(true).build();
        let size = ui.calc_text_size(letters);
        draw.add_text(
            [center[0] - size[0] * 0.5, center[1] - size[1] * 0.5],
            tint,
            letters,
        );
    }
}

fn paint_status_dot(
    ui: &Ui,
    center: [f32; 2],
    radius: f32,
    presence: Presence,
    backdrop: [f32; 4],
) {
    let dot = crate::theme::status_dot(center, radius);
    let draw = ui.get_window_draw_list();
    draw.add_circle(dot.center, dot.cutout, backdrop)
        .filled(true)
        .build();
    let color = presence_color(presence);
    draw.add_circle(dot.center, dot.radius, color)
        .filled(true)
        .build();
    match presence {
        Presence::Online => {}
        // Idle is a crescent: a backdrop-colored bite out of the upper left.
        Presence::Idle => {
            let bite = dot.radius * 0.62;
            draw.add_circle(
                [
                    dot.center[0] - dot.radius * 0.42,
                    dot.center[1] - dot.radius * 0.42,
                ],
                bite,
                backdrop,
            )
            .filled(true)
            .build();
        }
        // Offline is a hollow ring.
        Presence::Offline => {
            draw.add_circle(dot.center, dot.radius * 0.45, backdrop)
                .filled(true)
                .build();
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
    let parts: Vec<&str> = name
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect();
    let source = if parts.len() >= 2 {
        format!(
            "{}{}",
            parts[0].chars().next().unwrap_or('?'),
            parts[1].chars().next().unwrap_or('?')
        )
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

fn pointer_in(ui: &Ui, min: [f32; 2], max: [f32; 2]) -> bool {
    let mouse = ui.io().mouse_pos();
    mouse[0] >= min[0] && mouse[0] <= max[0] && mouse[1] >= min[1] && mouse[1] <= max[1]
}

#[cfg(test)]
mod uploads_unavailable_tests {
    use super::UPLOADS_UNAVAILABLE_MESSAGE;

    #[test]
    fn unavailable_popup_copy_matches_owl() {
        assert_eq!(UPLOADS_UNAVAILABLE_MESSAGE, "Currently Unavailable");
    }
}
