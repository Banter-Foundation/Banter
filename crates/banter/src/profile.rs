//! User profile card rules: open/close, edit mode, headings, and primary actions.
//!
//! Kept free of drawing so `tests/profile.rs` can check every rule without a window.
//! About Me and custom status stay deferred — see `docs/foundation.md`.

/// How the viewer relates to the profile being shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKind {
    SelfAccount,
    Friend,
    ServerMember { is_friend: bool },
}

/// Primary buttons on a foundation profile card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileAction {
    EditProfile,
    Message,
    AddFriend,
}

/// Floating profile card state. Drawable-free so unit tests own the contract.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileCard {
    user_id: Option<i64>,
    kind: Option<ProfileKind>,
    editing: bool,
    draft_display_name: String,
    anchor: [f32; 2],
    server_id: Option<i64>,
}

impl ProfileCard {
    pub fn closed() -> Self {
        Self {
            user_id: None,
            kind: None,
            editing: false,
            draft_display_name: String::new(),
            anchor: [0.0, 0.0],
            server_id: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.user_id.is_some()
    }

    pub fn user_id(&self) -> Option<i64> {
        self.user_id
    }

    pub fn kind(&self) -> Option<ProfileKind> {
        self.kind
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    pub fn draft_display_name(&self) -> &str {
        &self.draft_display_name
    }

    pub fn anchor(&self) -> [f32; 2] {
        self.anchor
    }

    pub fn server_id(&self) -> Option<i64> {
        self.server_id
    }

    pub fn open(&mut self, user_id: i64, kind: ProfileKind) {
        self.user_id = Some(user_id);
        self.kind = Some(kind);
        self.editing = false;
        self.draft_display_name.clear();
        self.server_id = None;
    }

    pub fn set_anchor(&mut self, anchor: [f32; 2]) {
        self.anchor = anchor;
    }

    pub fn set_server_id(&mut self, server_id: Option<i64>) {
        self.server_id = server_id;
    }

    /// Start editing the display name. Only the viewer's own profile can edit.
    pub fn begin_edit(&mut self, current_display_name: &str) -> bool {
        if self.kind != Some(ProfileKind::SelfAccount) {
            return false;
        }
        self.editing = true;
        self.draft_display_name = current_display_name.to_string();
        true
    }

    pub fn set_draft_display_name(&mut self, draft: impl Into<String>) {
        self.draft_display_name = draft.into();
    }

    pub fn draft_display_name_mut(&mut self) -> &mut String {
        &mut self.draft_display_name
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.draft_display_name.clear();
    }

    pub fn finish_edit(&mut self) {
        self.editing = false;
        self.draft_display_name.clear();
    }

    pub fn close(&mut self) {
        *self = Self::closed();
    }
}

impl Default for ProfileCard {
    fn default() -> Self {
        Self::closed()
    }
}

/// Name shown at the top of the card: trimmed nickname wins over display name.
pub fn profile_heading(display_name: &str, nickname: Option<&str>) -> String {
    match nickname.map(str::trim).filter(|value| !value.is_empty()) {
        Some(nick) => nick.to_string(),
        None => display_name.to_string(),
    }
}

/// Foundation actions for a profile card. About Me / custom status stay deferred.
pub fn profile_primary_actions(kind: ProfileKind) -> &'static [ProfileAction] {
    match kind {
        ProfileKind::SelfAccount => &[ProfileAction::EditProfile],
        ProfileKind::Friend => &[ProfileAction::Message],
        ProfileKind::ServerMember { is_friend: true } => &[ProfileAction::Message],
        ProfileKind::ServerMember { is_friend: false } => &[ProfileAction::AddFriend],
    }
}
