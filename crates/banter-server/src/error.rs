use thiserror::Error;

/// Recoverable Banter failure. Storage and bind failures carry the underlying message.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BanterError {
    #[error("that username is taken")]
    UsernameTaken,
    #[error("username must be 2-32 characters of lowercase letters, digits, underscore, or period")]
    InvalidUsername,
    #[error("display name must be 1-32 characters")]
    InvalidDisplayName,
    #[error("password must be 8-128 characters")]
    InvalidPassword,
    #[error("username or password is wrong")]
    InvalidCredentials,
    #[error("name must be 1-100 characters")]
    InvalidName,
    #[error("avatar must be a png, jpeg, gif, or webp image")]
    InvalidAvatar,
    #[error("banner must be a png, jpeg, gif, or webp image")]
    InvalidBanner,
    #[error("that profile customization is invalid")]
    InvalidProfile,
    #[error("not found")]
    NotFound,
    #[error("you are already friends")]
    AlreadyFriends,
    #[error("that friend request is already pending")]
    RequestExists,
    #[error("that friend request is not waiting on you")]
    NotPending,
    #[error("you cannot friend yourself")]
    CannotFriendSelf,
    #[error("you are not a member")]
    NotMember,
    #[error("you do not have permission to do that")]
    NotOwner,
    #[error("that role is above yours")]
    RoleHierarchy,
    #[error("this server already has the maximum number of roles")]
    TooManyRoles,
    #[error("you did not write that message")]
    NotAuthor,
    #[error("message is empty")]
    EmptyMessage,
    #[error("message is too long")]
    MessageTooLong,
    #[error("file is empty or larger than the attachment limit")]
    FileTooLarge,
    #[error("that channel does not accept text messages")]
    NotTextChannel,
    #[error("that channel is not a voice channel")]
    NotVoiceChannel,
    #[error("that channel is not a drawing board")]
    NotBoardChannel,
    #[error("the board changed; reload and try again")]
    BoardConflict,
    #[error("reply target is not in this conversation")]
    ReplyTargetMissing,
    #[error("direct messages are between friends")]
    NotFriend,
    #[error("you are already in that server")]
    AlreadyMember,
    #[error("invite is not valid")]
    InvalidInvite,
    #[error("there is no call to update")]
    NotInCall,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("could not bind the server: {0}")]
    Bind(String),
    #[error("that request is invalid")]
    Invalid,
    #[error("that version is not newer")]
    Conflict,
    #[error("slow down")]
    RateLimited,
}

impl BanterError {
    /// Stable `BANTER_*` code for HTTP JSON (`code` field) and the shared catalog.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UsernameTaken => "BANTER_USERNAME_TAKEN",
            Self::InvalidUsername => "BANTER_INVALID_USERNAME",
            Self::InvalidDisplayName => "BANTER_INVALID_DISPLAY_NAME",
            Self::InvalidPassword => "BANTER_INVALID_PASSWORD",
            Self::InvalidCredentials => "BANTER_INVALID_CREDENTIALS",
            Self::InvalidName => "BANTER_INVALID_NAME",
            Self::InvalidAvatar => "BANTER_INVALID_AVATAR",
            Self::InvalidBanner => "BANTER_INVALID_BANNER",
            Self::InvalidProfile => "BANTER_INVALID_PROFILE",
            Self::NotFound => "BANTER_NOT_FOUND",
            Self::AlreadyFriends => "BANTER_ALREADY_FRIENDS",
            Self::RequestExists => "BANTER_REQUEST_EXISTS",
            Self::NotPending => "BANTER_NOT_PENDING",
            Self::CannotFriendSelf => "BANTER_CANNOT_FRIEND_SELF",
            Self::NotMember => "BANTER_NOT_MEMBER",
            Self::NotOwner => "BANTER_NOT_OWNER",
            Self::RoleHierarchy => "BANTER_ROLE_HIERARCHY",
            Self::TooManyRoles => "BANTER_TOO_MANY_ROLES",
            Self::NotAuthor => "BANTER_NOT_AUTHOR",
            Self::EmptyMessage => "BANTER_EMPTY_MESSAGE",
            Self::MessageTooLong => "BANTER_MESSAGE_TOO_LONG",
            Self::FileTooLarge => "BANTER_FILE_TOO_LARGE",
            Self::NotTextChannel => "BANTER_NOT_TEXT_CHANNEL",
            Self::NotVoiceChannel => "BANTER_NOT_VOICE_CHANNEL",
            Self::NotBoardChannel => "BANTER_NOT_BOARD_CHANNEL",
            Self::BoardConflict => "BANTER_BOARD_CONFLICT",
            Self::ReplyTargetMissing => "BANTER_REPLY_TARGET_MISSING",
            Self::NotFriend => "BANTER_NOT_FRIEND",
            Self::AlreadyMember => "BANTER_ALREADY_MEMBER",
            Self::InvalidInvite => "BANTER_INVALID_INVITE",
            Self::NotInCall => "BANTER_NOT_IN_CALL",
            Self::Storage(_) => "BANTER_STORAGE",
            Self::Bind(_) => "BANTER_BIND",
            Self::Invalid => "BANTER_INVALID",
            Self::Conflict => "BANTER_CONFLICT",
            Self::RateLimited => "BANTER_RATE_LIMITED",
        }
    }
}
