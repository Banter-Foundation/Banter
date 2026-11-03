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
    #[error("only the server owner can do that until roles exist")]
    NotOwner,
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
}

impl BanterError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UsernameTaken => "username_taken",
            Self::InvalidUsername => "invalid_username",
            Self::InvalidDisplayName => "invalid_display_name",
            Self::InvalidPassword => "invalid_password",
            Self::InvalidCredentials => "invalid_credentials",
            Self::InvalidName => "invalid_name",
            Self::InvalidAvatar => "invalid_avatar",
            Self::NotFound => "not_found",
            Self::AlreadyFriends => "already_friends",
            Self::RequestExists => "request_exists",
            Self::NotPending => "not_pending",
            Self::CannotFriendSelf => "cannot_friend_self",
            Self::NotMember => "not_member",
            Self::NotOwner => "not_owner",
            Self::NotAuthor => "not_author",
            Self::EmptyMessage => "empty_message",
            Self::MessageTooLong => "message_too_long",
            Self::FileTooLarge => "file_too_large",
            Self::NotTextChannel => "not_text_channel",
            Self::NotVoiceChannel => "not_voice_channel",
            Self::ReplyTargetMissing => "reply_target_missing",
            Self::NotFriend => "not_friend",
            Self::AlreadyMember => "already_member",
            Self::InvalidInvite => "invalid_invite",
            Self::NotInCall => "not_in_call",
            Self::Storage(_) => "storage",
            Self::Bind(_) => "bind",
        }
    }
}
