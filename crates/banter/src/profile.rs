//! User profile card rules: Discord-parity view + customization editor state.
//!
//! Kept free of drawing so `tests/profile*.rs` can check every rule without a window.

use banter_server::cosmetics_catalog;

use crate::theme::{
    BUTTON_H, PROFILE_BANNER_H, PROFILE_BOARD_W, PROFILE_BODY_MIN_H, PROFILE_EDITOR_COL_GAP,
    PROFILE_EDITOR_FOOTER_H, PROFILE_EDITOR_OUTER_PAD, PROFILE_EDITOR_W, PROFILE_PAD,
    PROFILE_PREVIEW_MIN_W, PROFILE_W,
};

/// Axis-aligned panel used by the profile view and customization editor layouts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePanelRect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl ProfilePanelRect {
    pub fn width(self) -> f32 {
        self.max[0] - self.min[0]
    }

    pub fn height(self) -> f32 {
        self.max[1] - self.min[1]
    }

    pub fn size(self) -> [f32; 2] {
        [self.width(), self.height()]
    }

    pub fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.min[0]
            && point[0] <= self.max[0]
            && point[1] >= self.min[1]
            && point[1] <= self.max[1]
    }

    /// True when the interiors do not overlap (edges may touch).
    pub fn disjoint(self, other: Self) -> bool {
        self.max[0] <= other.min[0]
            || other.max[0] <= self.min[0]
            || self.max[1] <= other.min[1]
            || other.max[1] <= self.min[1]
    }
}

/// Floating profile view: identity column + board, anchored near the click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileViewLayout {
    pub card: ProfilePanelRect,
    pub identity: ProfilePanelRect,
    pub board: ProfilePanelRect,
}

/// Discord-style customization editor: left slots, center preview, right board, footer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileEditorLayout {
    pub frame: ProfilePanelRect,
    pub left: ProfilePanelRect,
    pub preview: ProfilePanelRect,
    pub board: ProfilePanelRect,
    pub footer: ProfilePanelRect,
    pub save: ProfilePanelRect,
    pub cancel: ProfilePanelRect,
    pub close: ProfilePanelRect,
}

/// Place the floating view card near `anchor`, flipping so it stays on-screen.
pub fn profile_view_layout(display: [f32; 2], anchor: [f32; 2]) -> ProfileViewLayout {
    let gap = 8.0;
    let margin = 8.0;
    let card_w = PROFILE_W + gap + PROFILE_BOARD_W;
    let card_h = (PROFILE_BANNER_H + PROFILE_BODY_MIN_H)
        .min(display[1] - margin * 2.0)
        .max(200.0);
    let mut min = [anchor[0] + gap, anchor[1]];
    if min[0] + card_w > display[0] - margin {
        min[0] = (anchor[0] - card_w - gap).max(margin);
    }
    if min[1] + card_h > display[1] - margin {
        min[1] = (display[1] - card_h - margin).max(margin);
    }
    let card = ProfilePanelRect {
        min,
        max: [min[0] + card_w, min[1] + card_h],
    };
    let identity = ProfilePanelRect {
        min: card.min,
        max: [card.min[0] + PROFILE_W, card.max[1]],
    };
    let board = ProfilePanelRect {
        min: [identity.max[0] + gap, card.min[1] + PROFILE_PAD],
        max: [card.max[0] - PROFILE_PAD, card.max[1] - PROFILE_PAD],
    };
    ProfileViewLayout {
        card,
        identity,
        board,
    }
}

/// Three-column customization chrome that fits inside `display` with a Save/Cancel footer.
pub fn profile_editor_layout(display: [f32; 2]) -> ProfileEditorLayout {
    let outer = PROFILE_EDITOR_OUTER_PAD;
    let frame = ProfilePanelRect {
        min: [outer, outer],
        max: [
            (display[0] - outer).max(outer + 320.0),
            (display[1] - outer).max(outer + 240.0),
        ],
    };
    let inner_min = [frame.min[0] + PROFILE_PAD, frame.min[1] + PROFILE_PAD];
    let inner_max = [
        frame.max[0] - PROFILE_PAD,
        frame.max[1] - PROFILE_EDITOR_FOOTER_H,
    ];
    let footer = ProfilePanelRect {
        min: [frame.min[0], frame.max[1] - PROFILE_EDITOR_FOOTER_H],
        max: frame.max,
    };
    let avail_w = (inner_max[0] - inner_min[0]).max(0.0);
    let gaps = PROFILE_EDITOR_COL_GAP * 2.0;
    let ideal = PROFILE_EDITOR_W + PROFILE_W + PROFILE_BOARD_W + gaps;
    let (left_w, preview_w) = if avail_w >= ideal {
        (PROFILE_EDITOR_W, PROFILE_W)
    } else {
        // Keep preview readable; shrink side columns first.
        let preview_w = PROFILE_PREVIEW_MIN_W.min(PROFILE_W).min(avail_w * 0.4);
        let side = ((avail_w - preview_w - gaps) * 0.5).max(140.0);
        (side, preview_w)
    };
    let body_h = (inner_max[1] - inner_min[1]).max(0.0);
    let left = ProfilePanelRect {
        min: inner_min,
        max: [inner_min[0] + left_w, inner_min[1] + body_h],
    };
    let preview = ProfilePanelRect {
        min: [left.max[0] + PROFILE_EDITOR_COL_GAP, inner_min[1]],
        max: [
            left.max[0] + PROFILE_EDITOR_COL_GAP + preview_w,
            inner_min[1] + body_h,
        ],
    };
    let board = ProfilePanelRect {
        min: [preview.max[0] + PROFILE_EDITOR_COL_GAP, inner_min[1]],
        max: [inner_max[0], inner_min[1] + body_h],
    };
    let btn_w = 120.0;
    let btn_y = footer.min[1] + (PROFILE_EDITOR_FOOTER_H - BUTTON_H) * 0.5;
    let cancel = ProfilePanelRect {
        min: [footer.min[0] + PROFILE_PAD, btn_y],
        max: [footer.min[0] + PROFILE_PAD + btn_w, btn_y + BUTTON_H],
    };
    let save = ProfilePanelRect {
        min: [footer.max[0] - PROFILE_PAD - btn_w, btn_y],
        max: [footer.max[0] - PROFILE_PAD, btn_y + BUTTON_H],
    };
    let close_size = 28.0;
    let close = ProfilePanelRect {
        min: [frame.max[0] - PROFILE_PAD - close_size, frame.min[1] + 10.0],
        max: [frame.max[0] - PROFILE_PAD, frame.min[1] + 10.0 + close_size],
    };
    ProfileEditorLayout {
        frame,
        left,
        preview,
        board,
        footer,
        save,
        cancel,
        close,
    }
}

/// Content width inside an editor/board column after horizontal pad.
pub fn profile_column_content_width(column_w: f32) -> f32 {
    (column_w - PROFILE_PAD).max(80.0)
}

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

/// View vs Discord-style customization editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileMode {
    View,
    Customize,
}

/// Board / Activity / Wishlist tabs on the profile panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileBoardTab {
    Board,
    Activity,
    Wishlist,
}

/// Left-column Discord editor slot currently being picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileEditorSlot {
    Nameplate,
    Avatar,
    AvatarDecoration,
    DisplayNameStyle,
    Theme,
    Banner,
    ProfileEffect,
    ProfileFrame,
    Badges,
}

/// Deferred banner/avatar change while the customization editor is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileMediaDraft {
    Unchanged,
    Clear,
    Bytes(Vec<u8>),
}

impl ProfileMediaDraft {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

/// Discord "Main Profile" vs a specific server profile in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileEditScope {
    Main,
    Server { id: i64, name: String },
}

impl ProfileEditScope {
    pub fn is_main(&self) -> bool {
        matches!(self, Self::Main)
    }

    pub fn server_id(&self) -> Option<i64> {
        match self {
            Self::Main => None,
            Self::Server { id, .. } => Some(*id),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Main => "Main Profile".into(),
            Self::Server { name, .. } => name.clone(),
        }
    }
}

pub use banter_server::ProfileWidgetKind;

/// Floating profile card state. Drawable-free so unit tests own the contract.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileCard {
    user_id: Option<i64>,
    kind: Option<ProfileKind>,
    mode: ProfileMode,
    draft_display_name: String,
    draft_about_me: String,
    draft_pronouns: String,
    draft_custom_status: String,
    draft_theme_primary: u32,
    draft_theme_secondary: u32,
    draft_nameplate: Option<String>,
    draft_avatar_decoration: Option<String>,
    draft_display_name_style: Option<String>,
    draft_profile_effect: Option<String>,
    draft_profile_frame: Option<String>,
    board_tab: ProfileBoardTab,
    anchor: [f32; 2],
    server_id: Option<i64>,
    edit_scope: ProfileEditScope,
    draft_nickname: String,
}

impl ProfileCard {
    pub fn closed() -> Self {
        Self {
            user_id: None,
            kind: None,
            mode: ProfileMode::View,
            draft_display_name: String::new(),
            draft_about_me: String::new(),
            draft_pronouns: String::new(),
            draft_custom_status: String::new(),
            draft_theme_primary: banter_server::DEFAULT_THEME_PRIMARY,
            draft_theme_secondary: banter_server::DEFAULT_THEME_SECONDARY,
            draft_nameplate: None,
            draft_avatar_decoration: None,
            draft_display_name_style: None,
            draft_profile_effect: None,
            draft_profile_frame: None,
            board_tab: ProfileBoardTab::Board,
            anchor: [0.0, 0.0],
            server_id: None,
            edit_scope: ProfileEditScope::Main,
            draft_nickname: String::new(),
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

    pub fn mode(&self) -> ProfileMode {
        self.mode
    }

    pub fn customizing(&self) -> bool {
        self.mode == ProfileMode::Customize
    }

    /// True while the Discord-style editor (or legacy display-name edit) is open.
    pub fn editing(&self) -> bool {
        self.customizing()
    }

    pub fn draft_display_name(&self) -> &str {
        &self.draft_display_name
    }

    pub fn draft_about_me(&self) -> &str {
        &self.draft_about_me
    }

    pub fn draft_pronouns(&self) -> &str {
        &self.draft_pronouns
    }

    pub fn draft_custom_status(&self) -> &str {
        &self.draft_custom_status
    }

    pub fn draft_theme_primary(&self) -> u32 {
        self.draft_theme_primary
    }

    pub fn draft_theme_secondary(&self) -> u32 {
        self.draft_theme_secondary
    }

    pub fn draft_nameplate(&self) -> &Option<String> {
        &self.draft_nameplate
    }

    pub fn draft_avatar_decoration(&self) -> &Option<String> {
        &self.draft_avatar_decoration
    }

    pub fn draft_display_name_style(&self) -> &Option<String> {
        &self.draft_display_name_style
    }

    pub fn draft_profile_effect(&self) -> &Option<String> {
        &self.draft_profile_effect
    }

    pub fn draft_profile_frame(&self) -> &Option<String> {
        &self.draft_profile_frame
    }

    pub fn board_tab(&self) -> ProfileBoardTab {
        self.board_tab
    }

    pub fn anchor(&self) -> [f32; 2] {
        self.anchor
    }

    pub fn server_id(&self) -> Option<i64> {
        self.server_id
    }

    pub fn edit_scope(&self) -> &ProfileEditScope {
        &self.edit_scope
    }

    pub fn set_edit_scope(&mut self, scope: ProfileEditScope) {
        self.edit_scope = scope;
    }

    pub fn draft_nickname(&self) -> &str {
        &self.draft_nickname
    }

    pub fn draft_nickname_mut(&mut self) -> &mut String {
        &mut self.draft_nickname
    }

    pub fn set_draft_nickname(&mut self, draft: impl Into<String>) {
        self.draft_nickname = draft.into();
    }

    pub fn open(&mut self, user_id: i64, kind: ProfileKind) {
        self.user_id = Some(user_id);
        self.kind = Some(kind);
        self.mode = ProfileMode::View;
        self.clear_drafts();
        self.board_tab = ProfileBoardTab::Board;
        self.server_id = None;
        self.edit_scope = ProfileEditScope::Main;
    }

    pub fn set_anchor(&mut self, anchor: [f32; 2]) {
        self.anchor = anchor;
    }

    pub fn set_server_id(&mut self, server_id: Option<i64>) {
        self.server_id = server_id;
    }

    pub fn set_board_tab(&mut self, tab: ProfileBoardTab) {
        self.board_tab = tab;
    }

    /// Open the Discord-style customization editor. Only the viewer's own profile.
    pub fn begin_customize(&mut self) -> bool {
        if self.kind != Some(ProfileKind::SelfAccount) {
            return false;
        }
        self.mode = ProfileMode::Customize;
        true
    }

    /// Seed drafts from the loaded profile payload, then open the editor.
    pub fn begin_customize_from(&mut self, view: &banter_server::UserProfile) -> bool {
        if !self.begin_customize() {
            return false;
        }
        self.draft_display_name = view.display_name.clone();
        self.draft_about_me = view.about_me.clone().unwrap_or_default();
        self.draft_pronouns = view.pronouns.clone().unwrap_or_default();
        self.draft_custom_status = view.custom_status.clone().unwrap_or_default();
        self.draft_theme_primary = view.theme_primary;
        self.draft_theme_secondary = view.theme_secondary;
        self.draft_nameplate = view.nameplate_id.clone();
        self.draft_avatar_decoration = view.avatar_decoration_id.clone();
        self.draft_display_name_style = view.display_name_style_id.clone();
        self.draft_profile_effect = view.profile_effect_id.clone();
        self.draft_profile_frame = view.profile_frame_id.clone();
        self.draft_nickname = view.nickname.clone().unwrap_or_default();
        self.edit_scope = ProfileEditScope::Main;
        true
    }

    /// Start editing (legacy entry used by older tests). Opens customize mode.
    pub fn begin_edit(&mut self, current_display_name: &str) -> bool {
        if !self.begin_customize() {
            return false;
        }
        self.draft_display_name = current_display_name.to_string();
        true
    }

    pub fn set_draft_display_name(&mut self, draft: impl Into<String>) {
        self.draft_display_name = draft.into();
    }

    pub fn draft_display_name_mut(&mut self) -> &mut String {
        &mut self.draft_display_name
    }

    pub fn set_draft_about_me(&mut self, draft: impl Into<String>) {
        self.draft_about_me = draft.into();
    }

    pub fn draft_about_me_mut(&mut self) -> &mut String {
        &mut self.draft_about_me
    }

    pub fn set_draft_pronouns(&mut self, draft: impl Into<String>) {
        self.draft_pronouns = draft.into();
    }

    pub fn draft_pronouns_mut(&mut self) -> &mut String {
        &mut self.draft_pronouns
    }

    pub fn set_draft_custom_status(&mut self, draft: impl Into<String>) {
        self.draft_custom_status = draft.into();
    }

    pub fn draft_custom_status_mut(&mut self) -> &mut String {
        &mut self.draft_custom_status
    }

    pub fn set_draft_theme(&mut self, primary: u32, secondary: u32) {
        self.draft_theme_primary = primary;
        self.draft_theme_secondary = secondary;
    }

    pub fn set_draft_nameplate(&mut self, id: Option<String>) {
        self.draft_nameplate = id;
    }

    pub fn set_draft_avatar_decoration(&mut self, id: Option<String>) {
        self.draft_avatar_decoration = id;
    }

    pub fn set_draft_display_name_style(&mut self, id: Option<String>) {
        self.draft_display_name_style = id;
    }

    pub fn set_draft_profile_effect(&mut self, id: Option<String>) {
        self.draft_profile_effect = id;
    }

    pub fn set_draft_profile_frame(&mut self, id: Option<String>) {
        self.draft_profile_frame = id;
    }

    pub fn cancel_edit(&mut self) {
        self.cancel_customize();
    }

    pub fn cancel_customize(&mut self) {
        self.mode = ProfileMode::View;
        self.clear_drafts();
    }

    pub fn finish_edit(&mut self) {
        self.mode = ProfileMode::View;
        self.clear_drafts();
    }

    pub fn close(&mut self) {
        *self = Self::closed();
    }

    fn clear_drafts(&mut self) {
        self.draft_display_name.clear();
        self.draft_about_me.clear();
        self.draft_pronouns.clear();
        self.draft_custom_status.clear();
        self.draft_theme_primary = banter_server::DEFAULT_THEME_PRIMARY;
        self.draft_theme_secondary = banter_server::DEFAULT_THEME_SECONDARY;
        self.draft_nameplate = None;
        self.draft_avatar_decoration = None;
        self.draft_display_name_style = None;
        self.draft_profile_effect = None;
        self.draft_profile_frame = None;
        self.draft_nickname.clear();
        self.edit_scope = ProfileEditScope::Main;
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

/// Foundation actions for a profile card.
pub fn profile_primary_actions(kind: ProfileKind) -> &'static [ProfileAction] {
    match kind {
        ProfileKind::SelfAccount => &[ProfileAction::EditProfile],
        ProfileKind::Friend => &[ProfileAction::Message],
        ProfileKind::ServerMember { is_friend: true } => &[ProfileAction::Message],
        ProfileKind::ServerMember { is_friend: false } => &[ProfileAction::AddFriend],
    }
}

/// Discord editor left-column slots from the customization reference.
pub fn profile_editor_sections() -> &'static [&'static str] {
    &[
        "Nameplate",
        "Avatar",
        "Avatar Decoration",
        "Display Name Style",
        "Theme",
        "Banner",
        "Profile Effect",
        "Profile Frame",
    ]
}

/// Slot pairs as laid out in Discord's customization left column.
pub fn profile_editor_slot_rows() -> &'static [(&'static str, &'static [ProfileEditorSlot])] {
    &[
        ("Nameplate", &[ProfileEditorSlot::Nameplate]),
        (
            "Avatar",
            &[
                ProfileEditorSlot::Avatar,
                ProfileEditorSlot::AvatarDecoration,
            ],
        ),
        ("Display Name Style", &[ProfileEditorSlot::DisplayNameStyle]),
        (
            "Theme & Banner",
            &[ProfileEditorSlot::Theme, ProfileEditorSlot::Banner],
        ),
        (
            "Profile Effect & Frame",
            &[
                ProfileEditorSlot::ProfileEffect,
                ProfileEditorSlot::ProfileFrame,
            ],
        ),
        ("Badges", &[ProfileEditorSlot::Badges]),
    ]
}

/// How many badges to show before a +N overflow chip (Discord profile row).
pub fn profile_badge_row_visible_cap() -> usize {
    6
}

pub fn profile_board_tabs() -> &'static [ProfileBoardTab] {
    &[
        ProfileBoardTab::Board,
        ProfileBoardTab::Activity,
        ProfileBoardTab::Wishlist,
    ]
}

pub fn profile_view_shows_board(mode: ProfileMode) -> bool {
    matches!(mode, ProfileMode::View | ProfileMode::Customize)
}

/// RGBA color for a display-name style id. Falls back to white.
pub fn display_name_style_color(id: Option<&str>) -> [f32; 4] {
    match banter_server::display_name_style_color(id) {
        Some(rgb) => rgb_u32_to_rgba(rgb),
        None => [1.0, 1.0, 1.0, 1.0],
    }
}

pub fn rgb_u32_to_rgba(rgb: u32) -> [f32; 4] {
    let r = ((rgb >> 16) & 0xff) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xff) as f32 / 255.0;
    let b = (rgb & 0xff) as f32 / 255.0;
    [r, g, b, 1.0]
}

pub fn cosmetic_label(id: &str) -> Option<String> {
    cosmetics_catalog()
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.label)
}

pub fn cosmetic_color(id: &str) -> Option<u32> {
    cosmetics_catalog()
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.color)
}

/// Max games a board widget may list in the editor (Discord board card caps).
pub fn games_widget_cap(kind: ProfileWidgetKind) -> usize {
    match kind {
        ProfileWidgetKind::GamesInRotation => 5,
        ProfileWidgetKind::GamesILike => 20,
        ProfileWidgetKind::FavoriteGame => 0,
    }
}

/// Read `(title, tag)` rows from a games widget payload. Missing/invalid entries are skipped.
pub fn games_from_payload(payload: &serde_json::Value) -> Vec<(String, String)> {
    let Some(games) = payload.get("games").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    games
        .iter()
        .filter_map(|game| {
            let title = game.get("title").and_then(|v| v.as_str())?.to_string();
            let tag = game
                .get("tag")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some((title, tag))
        })
        .collect()
}

/// Build a games widget payload from editor rows.
pub fn games_to_payload(games: &[(String, String)]) -> serde_json::Value {
    serde_json::json!({
        "games": games
            .iter()
            .map(|(title, tag)| serde_json::json!({ "title": title, "tag": tag }))
            .collect::<Vec<_>>()
    })
}
