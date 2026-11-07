//! Server Settings rules: page layout, open and close, and section backends.
//!
//! Kept free of drawing so `tests/server_settings.rs` can check every rule without a window.
//! Server Settings opens in its own GLFW window (same chrome as User Settings) so the main
//! Banter window stays usable. This is separate from User Settings (`settings.rs`).

use crate::settings::{
    settings_layout, settings_window_default_size, SettingsLayout, SETTINGS_WINDOW_DEFAULT_H,
    SETTINGS_WINDOW_DEFAULT_W,
};

/// OS window title for the dedicated Server Settings window.
pub const SERVER_SETTINGS_WINDOW_TITLE: &str = "Banter Server Settings";

/// Default client size for a new Banter Server Settings window (matches User Settings).
pub fn server_settings_window_default_size() -> (i32, i32) {
    (SETTINGS_WINDOW_DEFAULT_W, SETTINGS_WINDOW_DEFAULT_H)
}

/// Same Discord-like nav + content geometry as User Settings.
pub fn server_settings_layout(width: f32, height: f32) -> SettingsLayout {
    settings_layout(width, height)
}

/// Left-nav pages in Discord Server Settings order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerSettingsSection {
    Overview,
    Roles,
    Badges,
    Members,
}

/// Whether the section has a Banter API / Postgres backend today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerSettingsBackend {
    Live,
    ComingSoon,
}

/// Nav order shown in the Server Settings sidebar.
pub fn server_settings_nav_sections() -> [ServerSettingsSection; 4] {
    [
        ServerSettingsSection::Overview,
        ServerSettingsSection::Roles,
        ServerSettingsSection::Badges,
        ServerSettingsSection::Members,
    ]
}

/// Live when Postgres/API already support the section; ComingSoon only for empty backends.
pub fn server_settings_section_backend(section: ServerSettingsSection) -> ServerSettingsBackend {
    match section {
        ServerSettingsSection::Overview => ServerSettingsBackend::Live,
        ServerSettingsSection::Roles => ServerSettingsBackend::Live,
        // Community / server-granted badges are not in the API yet (profile badges are account-level).
        ServerSettingsSection::Badges => ServerSettingsBackend::ComingSoon,
        ServerSettingsSection::Members => ServerSettingsBackend::Live,
    }
}

impl ServerSettingsSection {
    /// Sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Roles => "Roles",
            Self::Badges => "Badges",
            Self::Members => "Member List",
        }
    }

    /// Lucide icon id already vendored for Banter.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Overview => "settings",
            Self::Roles => "users",
            Self::Badges => "crown",
            Self::Members => "user",
        }
    }

    /// Stable ImGui id fragment.
    pub fn nav_id(self) -> &'static str {
        match self {
            Self::Overview => "server-settings-overview",
            Self::Roles => "server-settings-roles",
            Self::Badges => "server-settings-badges",
            Self::Members => "server-settings-members",
        }
    }
}

/// Open page and target server. Closing clears the server id so the next open is explicit.
///
/// `open` also means the dedicated Banter Server Settings OS window should exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerSettingsState {
    open: bool,
    server_id: Option<i64>,
    section: ServerSettingsSection,
    /// Platform focuses the settings window once after open, then clears this.
    needs_focus: bool,
}

impl ServerSettingsState {
    pub fn new() -> Self {
        Self {
            open: false,
            server_id: None,
            section: ServerSettingsSection::Overview,
            needs_focus: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// True while Server Settings should be hosted in its own GLFW window.
    pub fn os_window_desired(&self) -> bool {
        self.open
    }

    pub fn section(&self) -> ServerSettingsSection {
        self.section
    }

    pub fn server_id(&self) -> Option<i64> {
        self.server_id
    }

    pub fn open(&mut self, server_id: i64, section: ServerSettingsSection) {
        self.open = true;
        self.server_id = Some(server_id);
        self.section = section;
        self.needs_focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.server_id = None;
        self.needs_focus = false;
    }

    /// Called when the OS settings window is closed via its chrome (not the in-UI X).
    pub fn window_closed(&mut self) {
        self.close();
    }

    /// Take-and-clear the one-shot focus request for the settings OS window.
    pub fn take_needs_focus(&mut self) -> bool {
        let needs = self.needs_focus;
        self.needs_focus = false;
        needs
    }

    pub fn select(&mut self, section: ServerSettingsSection) {
        self.section = section;
    }

    /// Dear ImGui closes an open dropdown on Escape by itself, so that press must not also close.
    pub fn escape(&mut self, popup_open: bool) {
        if !self.open || popup_open {
            return;
        }
        self.close();
    }
}

impl Default for ServerSettingsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Which Banter settings OS window should exist (User and Server are mutually exclusive hosts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsOsKind {
    User,
    Server,
}

pub fn settings_os_window_title(kind: SettingsOsKind) -> &'static str {
    match kind {
        SettingsOsKind::User => crate::settings::SETTINGS_WINDOW_TITLE,
        SettingsOsKind::Server => SERVER_SETTINGS_WINDOW_TITLE,
    }
}

pub fn settings_os_window_default_size(kind: SettingsOsKind) -> (i32, i32) {
    match kind {
        SettingsOsKind::User => settings_window_default_size(),
        SettingsOsKind::Server => server_settings_window_default_size(),
    }
}
