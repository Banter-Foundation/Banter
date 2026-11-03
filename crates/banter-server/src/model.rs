use serde::{Deserialize, Serialize};

use crate::error::BanterError;

pub const MAX_MESSAGE_CHARS: usize = 2000;
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
pub const IDLE_AFTER_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub has_avatar: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
        }
    }

    pub fn parse(value: &str) -> Result<Self, BanterError> {
        match value {
            "text" => Ok(Self::Text),
            "voice" => Ok(Self::Voice),
            _ => Err(BanterError::InvalidName),
        }
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    Idle,
    Offline,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Audio,
    Camera,
    Screen,
}

impl MediaKind {
    pub(crate) fn code(self) -> u8 {
        match self {
            Self::Audio => 1,
            Self::Camera => 2,
            Self::Screen => 3,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Audio),
            2 => Some(Self::Camera),
            3 => Some(Self::Screen),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum MediaPlace {
    VoiceChannel(i64),
    DirectCall(i64),
}

impl MediaPlace {
    pub(crate) fn code(self) -> (u8, i64) {
        match self {
            Self::VoiceChannel(id) => (1, id),
            Self::DirectCall(id) => (2, id),
        }
    }

    pub(crate) fn from_parts(code: u8, id: i64) -> Option<Self> {
        match code {
            1 => Some(Self::VoiceChannel(id)),
            2 => Some(Self::DirectCall(id)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedMedia {
    pub user_id: i64,
    pub kind: MediaKind,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaFrame {
    pub kind: MediaKind,
    pub place: MediaPlace,
    pub sender_id: i64,
    pub payload: Vec<u8>,
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

/// Realtime events pushed over the websocket. Binary media uses a separate frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    Presence {
        user_id: i64,
        state: Presence,
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
}
