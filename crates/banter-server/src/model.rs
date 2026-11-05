use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::BanterError;

pub const MAX_MESSAGE_CHARS: usize = 2000;
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BANNER_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ABOUT_ME_CHARS: usize = 190;
pub const MAX_PRONOUNS_CHARS: usize = 40;
pub const MAX_CUSTOM_STATUS_CHARS: usize = 128;
pub const MAX_PROFILE_WIDGETS: usize = 8;
pub const MAX_PROFILE_CONNECTIONS: usize = 10;
pub const MAX_PROFILE_ACTIVITY: usize = 20;
pub const MAX_PROFILE_WISHLIST: usize = 20;
pub const IDLE_AFTER_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub has_avatar: bool,
    pub avatar_rev: i64,
    /// True for bot accounts. Ordinary users stay false when the field is absent.
    #[serde(default)]
    pub bot: bool,
}

/// Board widget kinds matching Discord's profile Board cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileWidgetKind {
    GamesInRotation,
    FavoriteGame,
    GamesILike,
}

/// One user-authored board widget on a profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileWidget {
    pub id: i64,
    pub kind: ProfileWidgetKind,
    pub title: String,
    pub payload: serde_json::Value,
}

/// Input used when replacing the widget list on a profile patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileWidgetInput {
    pub kind: ProfileWidgetKind,
    pub title: String,
    pub payload: serde_json::Value,
}

/// Linked account / website row under Connections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileConnection {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
}

/// Input used when replacing the connections list on a profile patch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileConnectionInput {
    pub kind: String,
    pub name: String,
    pub url: String,
}

/// One equipped Banter badge shown on the profile badge row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBadge {
    pub id: String,
    pub label: String,
    pub color: u32,
    pub shape: crate::BadgeShape,
}

/// User-authored Activity tab row (empty until the owner adds entries).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileActivity {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileActivityInput {
    pub kind: String,
    pub title: String,
    pub detail: String,
}

/// User-authored Wishlist tab game (empty until the owner adds entries).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileWishlistItem {
    pub id: i64,
    pub title: String,
    pub tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileWishlistInput {
    pub title: String,
    pub tag: String,
}

/// Partial update for Discord-parity profile customization.
///
/// Omitted fields stay unchanged. Empty strings clear optional text / cosmetic
/// ids. When `widgets`, `connections`, `badge_ids`, `activity`, or `wishlist`
/// is present, the full list is replaced.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfilePatch {
    pub display_name: Option<String>,
    pub about_me: Option<String>,
    pub pronouns: Option<String>,
    pub custom_status: Option<String>,
    pub theme_primary: Option<u32>,
    pub theme_secondary: Option<u32>,
    pub nameplate_id: Option<String>,
    pub avatar_decoration_id: Option<String>,
    pub display_name_style_id: Option<String>,
    pub profile_effect_id: Option<String>,
    pub profile_frame_id: Option<String>,
    pub widgets: Option<Vec<ProfileWidgetInput>>,
    pub connections: Option<Vec<ProfileConnectionInput>>,
    #[serde(default)]
    pub badge_ids: Option<Vec<String>>,
    #[serde(default)]
    pub activity: Option<Vec<ProfileActivityInput>>,
    #[serde(default)]
    pub wishlist: Option<Vec<ProfileWishlistInput>>,
}

/// Public profile a signed-in viewer is allowed to see.
///
/// Visible for self, friends, or anyone who shares a server. `nickname` is
/// filled only when the viewer asks with a `server_id` they belong to.
/// Customization fields match Discord's profile editor baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub has_avatar: bool,
    pub avatar_rev: i64,
    pub nickname: Option<String>,
    pub is_self: bool,
    pub is_friend: bool,
    pub about_me: Option<String>,
    pub pronouns: Option<String>,
    pub custom_status: Option<String>,
    pub member_since: i64,
    pub has_banner: bool,
    pub banner_rev: i64,
    pub theme_primary: u32,
    pub theme_secondary: u32,
    pub nameplate_id: Option<String>,
    pub avatar_decoration_id: Option<String>,
    pub display_name_style_id: Option<String>,
    pub profile_effect_id: Option<String>,
    pub profile_frame_id: Option<String>,
    pub widgets: Vec<ProfileWidget>,
    pub connections: Vec<ProfileConnection>,
    #[serde(default)]
    pub badges: Vec<ProfileBadge>,
    #[serde(default)]
    pub activity: Vec<ProfileActivity>,
    #[serde(default)]
    pub wishlist: Vec<ProfileWishlistItem>,
    /// Guild roles for this profile when the viewer asked with `server_id`.
    #[serde(default)]
    pub roles: Vec<GuildRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub account: Account,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friend {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub has_avatar: bool,
    pub avatar_rev: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequest {
    pub id: i64,
    pub from_id: i64,
    pub from_username: String,
    pub to_id: i64,
    pub to_username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSummary {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
}

/// One per-user folder on the server rail. `color` is RGB (`0` = default look).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerFolder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub color: i64,
    pub position: i64,
    pub collapsed: bool,
}

/// A server membership placed in the signed-in user's rail tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRailServer {
    pub server: ServerSummary,
    pub folder_id: Option<i64>,
    pub position: i64,
}

/// Private rail layout for one account: nested folders plus server placements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRail {
    pub folders: Vec<ServerFolder>,
    pub servers: Vec<ServerRailServer>,
}

/// Partial update for a rail folder. Nested `parent_id: Some(None)` means top-level.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerFolderPatch {
    pub name: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub color: Option<i64>,
    pub collapsed: Option<bool>,
    pub position: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerHome {
    pub server: ServerSummary,
    pub categories: Vec<CategoryLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub server_id: i64,
    pub name: String,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Board,
    /// Plugin kind stored as `<plugin id>.<name>`, for example `chess.match`.
    Plugin(String),
}

impl ChannelKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Board => "board",
            Self::Plugin(kind) => kind.as_str(),
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text)
    }

    pub fn is_voice(&self) -> bool {
        matches!(self, Self::Voice)
    }

    pub fn is_board(&self) -> bool {
        matches!(self, Self::Board)
    }

    pub fn is_plugin(&self) -> bool {
        matches!(self, Self::Plugin(_))
    }

    pub fn parse(value: &str) -> Result<Self, BanterError> {
        match value {
            "text" => Ok(Self::Text),
            "voice" => Ok(Self::Voice),
            "board" => Ok(Self::Board),
            other if crate::plugins::plugin_kind_ok(other) => Ok(Self::Plugin(other.to_string())),
            _ => Err(BanterError::InvalidName),
        }
    }
}

impl Serialize for ChannelKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ChannelKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        // Public create rejects plugin kinds in the store. Parsing them here lets
        // layout responses and database rows round-trip the stored string.
        ChannelKind::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: i64,
    pub server_id: i64,
    pub category_id: i64,
    pub name: String,
    pub kind: ChannelKind,
    pub position: i64,
    /// Optional Lucide icon id. `None` means the client uses the kind default.
    #[serde(default)]
    pub icon: Option<String>,
    /// Set when a plugin created this channel. That plugin is the only one that may delete it.
    #[serde(default)]
    pub created_by_plugin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryLayout {
    pub category: Category,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub is_owner: bool,
    pub has_avatar: bool,
    pub avatar_rev: i64,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub roles: Vec<GuildRole>,
}

/// A role on a server. @everyone is `everyone` and is not stored in `member_roles`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildRole {
    pub id: i64,
    pub server_id: i64,
    pub name: String,
    #[serde(default)]
    pub color: i64,
    pub position: i64,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub permissions: i64,
    #[serde(default)]
    pub everyone: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ChatTarget {
    Channel(i64),
    Direct(i64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: i64,
    pub filename: String,
    pub byte_size: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub author_id: i64,
    pub author_username: String,
    pub author_display_name: String,
    pub author_nickname: Option<String>,
    pub body: String,
    pub reply_to: Option<i64>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub attachments: Vec<Attachment>,
    /// `user` for people and bots. `plugin` when `author_id` is empty and `plugin_id` is set.
    #[serde(default = "user_author_kind")]
    pub author_kind: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
}

fn user_author_kind() -> String {
    "user".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub id: i64,
    pub message_id: i64,
    pub channel_id: Option<i64>,
    pub dm_id: Option<i64>,
    pub from_username: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectChat {
    pub id: i64,
    pub peer_id: i64,
    pub peer_username: String,
    pub peer_display_name: String,
    pub peer_has_avatar: bool,
    pub peer_avatar_rev: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    Idle,
    Offline,
}

/// Live presence plus optional custom status text (Discord-style).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceSnapshot {
    pub state: Presence,
    #[serde(default)]
    pub custom_status: Option<String>,
}

impl PresenceSnapshot {
    pub fn offline() -> Self {
        Self {
            state: Presence::Offline,
            custom_status: None,
        }
    }

    pub fn with_state(state: Presence, custom_status: Option<String>) -> Self {
        Self {
            state,
            custom_status,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceFlags {
    pub muted: bool,
    pub deafened: bool,
    pub push_to_talk: bool,
    pub transmitting: bool,
    pub camera: bool,
    pub screen: bool,
}

impl Default for VoiceFlags {
    fn default() -> Self {
        Self {
            muted: false,
            deafened: false,
            push_to_talk: false,
            transmitting: false,
            camera: false,
            screen: false,
        }
    }
}

pub fn should_send_audio(flags: &VoiceFlags) -> bool {
    if flags.muted || flags.deafened {
        return false;
    }
    if flags.push_to_talk && !flags.transmitting {
        return false;
    }
    true
}

pub fn should_receive_audio(flags: &VoiceFlags) -> bool {
    !flags.deafened
}

pub fn should_send_camera(flags: &VoiceFlags) -> bool {
    flags.camera
}

pub fn should_send_screen(flags: &VoiceFlags) -> bool {
    flags.screen
}

/// Desktop and app audio while Go Live. Mute and push-to-talk gate the mic, not this track.
pub fn should_send_stream_audio(flags: &VoiceFlags) -> bool {
    flags.screen
}

/// Four publishable tracks. User audio is the protected mic. Stream audio is the Go Live mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    UserAudio,
    StreamAudio,
    Camera,
    Screen,
}

impl MediaKind {
    pub fn is_audio(self) -> bool {
        matches!(self, Self::UserAudio | Self::StreamAudio)
    }

    pub fn is_video(self) -> bool {
        matches!(self, Self::Camera | Self::Screen)
    }

    /// Stable texture and metrics code. Video codes match the tiles the UI already keys on.
    pub fn code(self) -> u8 {
        match self {
            Self::UserAudio => 1,
            Self::StreamAudio => 4,
            Self::Camera => 2,
            Self::Screen => 3,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::UserAudio),
            4 => Some(Self::StreamAudio),
            2 => Some(Self::Camera),
            3 => Some(Self::Screen),
            _ => None,
        }
    }

    pub fn publish_allowed(self, flags: &VoiceFlags) -> bool {
        match self {
            Self::UserAudio => should_send_audio(flags),
            Self::StreamAudio => should_send_stream_audio(flags),
            Self::Camera => should_send_camera(flags),
            Self::Screen => should_send_screen(flags),
        }
    }
}

/// Simulcast spatial layer. `Off` stops video for that track only.
///
/// RIDs are `f`, `h`, and `q` on every offer, on Linux and Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerId {
    Full,
    Half,
    Quarter,
    Off,
}

impl LayerId {
    pub const RIDS: [&'static str; 3] = ["f", "h", "q"];

    pub fn rid(self) -> Option<&'static str> {
        match self {
            Self::Full => Some("f"),
            Self::Half => Some("h"),
            Self::Quarter => Some("q"),
            Self::Off => None,
        }
    }

    pub fn from_rid(rid: &str) -> Option<Self> {
        match rid {
            "f" => Some(Self::Full),
            "h" => Some(Self::Half),
            "q" => Some(Self::Quarter),
            _ => None,
        }
    }

    /// Higher is a larger picture. `Off` is the floor.
    pub fn rank(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Quarter => 1,
            Self::Half => 2,
            Self::Full => 3,
        }
    }

    pub fn from_rank(rank: u8) -> Self {
        match rank {
            0 => Self::Off,
            1 => Self::Quarter,
            2 => Self::Half,
            _ => Self::Full,
        }
    }

    /// Linear resolution scale of the publisher cap. Off contributes no picture.
    pub fn scale(self) -> f32 {
        match self {
            Self::Full => 1.0,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
            Self::Off => 0.0,
        }
    }
}

/// Highest layer that is published, at or below the viewer's cap, and at or below the sender ladder.
pub fn deliverable_layer(wanted: LayerId, sender_max: LayerId, published: &[LayerId]) -> LayerId {
    let cap = wanted.rank().min(sender_max.rank());
    let mut best = LayerId::Off;
    for layer in published {
        if layer.rank() <= cap && layer.rank() > best.rank() {
            best = *layer;
        }
    }
    best
}

/// Continuous slider position snaps onto the layers a viewer can actually select.
pub fn snap_quality_slider(position: f32) -> LayerId {
    let position = position.clamp(0.0, 1.0);
    if position < 0.125 {
        LayerId::Off
    } else if position < 0.375 {
        LayerId::Quarter
    } else if position < 0.75 {
        LayerId::Half
    } else {
        LayerId::Full
    }
}

pub fn slider_position(layer: LayerId) -> f32 {
    match layer {
        LayerId::Off => 0.0,
        LayerId::Quarter => 0.25,
        LayerId::Half => 0.5,
        LayerId::Full => 1.0,
    }
}

/// Sender congestion steps. User audio is not a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LadderStep {
    Nominal,
    CameraHalf,
    StreamAudioHalf,
    ScreenHalf,
    CameraCut,
    StreamAudioCut,
    ScreenStop,
}

impl LadderStep {
    pub fn next_down(self) -> Self {
        match self {
            Self::Nominal => Self::CameraHalf,
            Self::CameraHalf => Self::StreamAudioHalf,
            Self::StreamAudioHalf => Self::ScreenHalf,
            Self::ScreenHalf => Self::CameraCut,
            Self::CameraCut => Self::StreamAudioCut,
            Self::StreamAudioCut => Self::ScreenStop,
            Self::ScreenStop => Self::ScreenStop,
        }
    }

    pub fn next_up(self) -> Self {
        match self {
            Self::Nominal => Self::Nominal,
            Self::CameraHalf => Self::Nominal,
            Self::StreamAudioHalf => Self::CameraHalf,
            Self::ScreenHalf => Self::StreamAudioHalf,
            Self::CameraCut => Self::ScreenHalf,
            Self::StreamAudioCut => Self::CameraCut,
            Self::ScreenStop => Self::StreamAudioCut,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::CameraHalf => "camera half",
            Self::StreamAudioHalf => "stream audio half",
            Self::ScreenHalf => "screen half",
            Self::CameraCut => "camera cut",
            Self::StreamAudioCut => "stream audio cut",
            Self::ScreenStop => "screen stop",
        }
    }
}

/// Target fraction of the configured cap for each track at a ladder step.
/// User audio stays at 1. Camera and screen use the spatial layer; 0 means the track is unpublished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderTargets {
    pub user_audio: f32,
    pub stream_audio: f32,
    pub camera: LayerId,
    pub screen: LayerId,
    pub screen_published: bool,
}

pub fn ladder_targets(step: LadderStep) -> LadderTargets {
    let (camera, stream_audio, screen, screen_published) = match step {
        LadderStep::Nominal => (LayerId::Full, 1.0, LayerId::Full, true),
        LadderStep::CameraHalf => (LayerId::Half, 1.0, LayerId::Full, true),
        LadderStep::StreamAudioHalf => (LayerId::Half, 0.5, LayerId::Full, true),
        LadderStep::ScreenHalf => (LayerId::Half, 0.5, LayerId::Half, true),
        LadderStep::CameraCut => (LayerId::Off, 0.5, LayerId::Half, true),
        LadderStep::StreamAudioCut => (LayerId::Off, 0.0, LayerId::Half, true),
        LadderStep::ScreenStop => (LayerId::Off, 0.0, LayerId::Off, false),
    };
    LadderTargets {
        user_audio: 1.0,
        stream_audio,
        camera,
        screen,
        screen_published,
    }
}

/// Steps down after this many consecutive congested samples, and up after this many clear samples.
pub const LADDER_DOWN_TICKS: u32 = 3;
pub const LADDER_UP_TICKS: u32 = 8;

/// Sender ladder. `note` is one stats sample. Congestion steps down; a clear path steps up with hysteresis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CongestionLadder {
    step: LadderStep,
    down: u32,
    up: u32,
}

impl CongestionLadder {
    pub fn new() -> Self {
        Self {
            step: LadderStep::Nominal,
            down: 0,
            up: 0,
        }
    }

    pub fn step(&self) -> LadderStep {
        self.step
    }

    pub fn targets(&self) -> LadderTargets {
        ladder_targets(self.step)
    }

    pub fn note(&mut self, congested: bool) -> LadderStep {
        if congested {
            self.up = 0;
            self.down = self.down.saturating_add(1);
            if self.down >= LADDER_DOWN_TICKS && self.step != LadderStep::ScreenStop {
                self.step = self.step.next_down();
                self.down = 0;
            }
        } else {
            self.down = 0;
            self.up = self.up.saturating_add(1);
            if self.up >= LADDER_UP_TICKS && self.step != LadderStep::Nominal {
                self.step = self.step.next_up();
                self.up = 0;
            }
        }
        self.step
    }
}

impl Default for CongestionLadder {
    fn default() -> Self {
        Self::new()
    }
}

/// Sticky per publisher and track for one session.
#[derive(Clone, Debug, Default)]
pub struct LayerPreferences {
    chosen: HashMap<(i64, MediaKind), LayerId>,
}

impl LayerPreferences {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, publisher: i64, track: MediaKind) -> LayerId {
        self.chosen.get(&(publisher, track)).copied().unwrap_or(LayerId::Full)
    }

    pub fn set(&mut self, publisher: i64, track: MediaKind, layer: LayerId) {
        self.chosen.insert((publisher, track), layer);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum MediaPlace {
    VoiceChannel(i64),
    DirectCall(i64),
}

impl MediaPlace {
    pub fn code(self) -> (u8, i64) {
        match self {
            Self::VoiceChannel(id) => (1, id),
            Self::DirectCall(id) => (2, id),
        }
    }

    pub fn from_parts(code: u8, id: i64) -> Option<Self> {
        match code {
            1 => Some(Self::VoiceChannel(id)),
            2 => Some(Self::DirectCall(id)),
            _ => None,
        }
    }
}

/// Client messages on the authenticated websocket. Media itself is UDP, not these frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientSignal {
    Activity,
    RtcOffer {
        place: MediaPlace,
        sdp: String,
    },
    RtcAnswer {
        place: MediaPlace,
        sdp: String,
    },
    RtcIce {
        place: MediaPlace,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    SubscribeLayer {
        place: MediaPlace,
        publisher: i64,
        track: MediaKind,
        max_layer: LayerId,
    },
    BoardOp {
        channel_id: i64,
        op: BoardOp,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    Ringing,
    Active,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSnapshot {
    pub dm_id: i64,
    pub phase: CallPhase,
    pub caller: i64,
    pub callee: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceMember {
    pub user_id: i64,
    pub flags: VoiceFlags,
}

/// Who is in one voice channel. Empty `members` means the channel is clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRoster {
    pub channel_id: i64,
    pub members: Vec<VoiceMember>,
}

/// Realtime events pushed over the websocket. Media is UDP, not a binary frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    Presence {
        user_id: i64,
        state: Presence,
        #[serde(default)]
        custom_status: Option<String>,
    },
    MessageCreated {
        target: ChatTarget,
        message: Message,
    },
    MessageEdited {
        target: ChatTarget,
        message: Message,
    },
    MessageDeleted {
        target: ChatTarget,
        message_id: i64,
    },
    FriendRequest {
        request: FriendRequest,
    },
    FriendsChanged,
    Voice {
        channel_id: i64,
        members: Vec<VoiceMember>,
    },
    Call {
        call: CallSnapshot,
    },
    CallEnded {
        dm_id: i64,
    },
    /// Voice: the SFU's answer or a renegotiation offer. DM: relayed from the other peer (`from`).
    RtcOffer {
        place: MediaPlace,
        from: i64,
        sdp: String,
    },
    RtcAnswer {
        place: MediaPlace,
        from: i64,
        sdp: String,
    },
    RtcIce {
        place: MediaPlace,
        from: i64,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    /// DM only. The SFU applies `SubscribeLayer` itself and does not echo it.
    SubscribeLayer {
        place: MediaPlace,
        subscriber: i64,
        publisher: i64,
        track: MediaKind,
        max_layer: LayerId,
    },
    /// Dynacast. Layers with at least one subscriber. The publisher pauses the rest.
    WantedLayers {
        place: MediaPlace,
        track: MediaKind,
        layers: Vec<LayerId>,
        subscribers: u32,
    },
    BoardChanged {
        channel_id: i64,
        revision: i64,
        ops: Vec<BoardOp>,
        from: i64,
    },
    /// First frame on a bot websocket.
    Ready {
        bot: bool,
        user: Account,
    },
    PluginSync {
        server_id: i64,
        plugin_id: String,
        channel_id: i64,
        state: Option<serde_json::Value>,
    },
}

pub const MAX_BOARD_ASSET_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardElementKind {
    Rect,
    Ellipse,
    Diamond,
    Triangle,
    Line,
    Arrow,
    Freehand,
    Text,
    Sticky,
    Image,
    FileCard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardStrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

fn default_stroke_width() -> f64 {
    2.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardElement {
    pub id: String,
    pub kind: BoardElementKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    pub stroke: String,
    pub fill: String,
    #[serde(default = "default_stroke_width")]
    pub stroke_width: f64,
    #[serde(default)]
    pub stroke_style: BoardStrokeStyle,
    #[serde(default)]
    pub rotation: f64,
    pub z: i64,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub asset_id: Option<i64>,
    #[serde(default)]
    pub filename: Option<String>,
    pub updated_by: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardScene {
    pub revision: i64,
    pub elements: Vec<BoardElement>,
}

impl BoardScene {
    pub fn empty() -> Self {
        Self {
            revision: 0,
            elements: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoardOp {
    Upsert {
        base_revision: i64,
        elements: Vec<BoardElement>,
    },
    Delete {
        base_revision: i64,
        ids: Vec<String>,
    },
    Reorder {
        base_revision: i64,
        ids: Vec<String>,
    },
    Group {
        base_revision: i64,
        ids: Vec<String>,
        group_id: String,
    },
    Ungroup {
        base_revision: i64,
        group_ids: Vec<String>,
    },
}

impl BoardOp {
    pub fn base_revision(&self) -> i64 {
        match self {
            Self::Upsert { base_revision, .. }
            | Self::Delete { base_revision, .. }
            | Self::Reorder { base_revision, .. }
            | Self::Group { base_revision, .. }
            | Self::Ungroup { base_revision, .. } => *base_revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardAsset {
    pub id: i64,
    pub channel_id: i64,
    pub filename: String,
    pub byte_size: i64,
    pub content_type: String,
}

/// One line from the in-memory server log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugLogLine {
    pub at_ms: i64,
    pub level: String,
    pub message: String,
}

/// Process-wide network counters for the hidden server debug window.
///
/// `in` is bytes received by the server. `out` is bytes sent. Rates are since
/// the previous `/v1/debug` poll.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTraffic {
    #[serde(default)]
    pub http_in: u64,
    #[serde(default)]
    pub http_out: u64,
    #[serde(default)]
    pub http_reqs: u64,
    #[serde(default)]
    pub http_in_per_s: u64,
    #[serde(default)]
    pub http_out_per_s: u64,
    #[serde(default)]
    pub http_reqs_per_s: u64,
    #[serde(default)]
    pub ws_in: u64,
    #[serde(default)]
    pub ws_out: u64,
    #[serde(default)]
    pub ws_msgs_in: u64,
    #[serde(default)]
    pub ws_msgs_out: u64,
    #[serde(default)]
    pub ws_in_per_s: u64,
    #[serde(default)]
    pub ws_out_per_s: u64,
    #[serde(default)]
    pub media_in: u64,
    #[serde(default)]
    pub media_out: u64,
    #[serde(default)]
    pub media_packets_in: u64,
    #[serde(default)]
    pub media_packets_out: u64,
    #[serde(default)]
    pub media_in_per_s: u64,
    #[serde(default)]
    pub media_out_per_s: u64,
}

/// Snapshot for the hidden server debug window. Any signed-in client can read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDebug {
    /// Banter release label (`BANTER_VERSION`). Empty when an older host omits it.
    #[serde(default)]
    pub version: String,
    /// Wire / feature protocol revision. `0` when an older host omits it.
    #[serde(default)]
    pub protocol: u32,
    pub uptime_ms: i64,
    pub rss_bytes: u64,
    pub online: i64,
    pub idle: i64,
    pub sockets: i64,
    pub voice_members: i64,
    pub voice_channels: i64,
    pub calls: i64,
    pub media_slots: i64,
    pub media_forwards: i64,
    pub users: i64,
    pub servers: i64,
    pub channels: i64,
    pub messages: i64,
    pub pool_size: i64,
    pub pool_idle: i64,
    #[serde(default)]
    pub traffic: ServerTraffic,
    pub logs: Vec<DebugLogLine>,
}

#[cfg(test)]
mod server_debug_tests {
    use super::{ServerDebug, ServerTraffic};

    #[test]
    fn server_debug_deserializes_without_version_protocol_or_traffic() {
        let json = r#"{
            "uptime_ms": 1000,
            "rss_bytes": 2048,
            "online": 1,
            "idle": 0,
            "sockets": 1,
            "voice_members": 0,
            "voice_channels": 0,
            "calls": 0,
            "media_slots": 0,
            "media_forwards": 0,
            "users": 1,
            "servers": 1,
            "channels": 1,
            "messages": 0,
            "pool_size": 4,
            "pool_idle": 4,
            "logs": []
        }"#;
        let view: ServerDebug = serde_json::from_str(json).expect("legacy ServerDebug JSON");
        assert_eq!(view.version, "");
        assert_eq!(view.protocol, 0);
        assert_eq!(view.traffic, ServerTraffic::default());
        assert_eq!(view.uptime_ms, 1000);
    }
}
