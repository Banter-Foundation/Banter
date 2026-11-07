//! Server Settings contracts. Written before the Discord-like server settings window (BA-106).

use banter::{
    context_menu_actions, context_menu_labels, server_rail_item_menu, server_settings_layout,
    server_settings_nav_sections, server_settings_section_backend, server_settings_window_default_size,
    settings_layout, ContextMenuAction, ServerSettingsBackend, ServerSettingsSection,
    ServerSettingsState, SERVER_SETTINGS_WINDOW_TITLE,
};

#[test]
fn server_settings_window_title_and_default_size_match_user_settings_chrome() {
    assert_eq!(SERVER_SETTINGS_WINDOW_TITLE, "Banter Server Settings");
    let (w, h) = server_settings_window_default_size();
    assert_eq!((w, h), (1280, 800));
}

#[test]
fn server_settings_layout_reuses_discord_nav_content_geometry() {
    let user = settings_layout(1280.0, 800.0);
    let server = server_settings_layout(1280.0, 800.0);
    assert_eq!(user.nav_w, server.nav_w);
    assert_eq!(user.content_w, server.content_w);
    assert_eq!(user.close_x, server.close_x);
    assert_eq!(user.close_size, server.close_size);
}

#[test]
fn server_settings_nav_order_is_overview_roles_badges_members() {
    assert_eq!(
        server_settings_nav_sections(),
        [
            ServerSettingsSection::Overview,
            ServerSettingsSection::Roles,
            ServerSettingsSection::Badges,
            ServerSettingsSection::Members,
        ]
    );
}

#[test]
fn server_settings_backends_match_what_apis_already_support() {
    assert_eq!(
        server_settings_section_backend(ServerSettingsSection::Overview),
        ServerSettingsBackend::Live
    );
    assert_eq!(
        server_settings_section_backend(ServerSettingsSection::Roles),
        ServerSettingsBackend::Live
    );
    assert_eq!(
        server_settings_section_backend(ServerSettingsSection::Badges),
        ServerSettingsBackend::ComingSoon
    );
    assert_eq!(
        server_settings_section_backend(ServerSettingsSection::Members),
        ServerSettingsBackend::Live
    );
}

#[test]
fn server_settings_open_close_nav_and_server_id() {
    let mut state = ServerSettingsState::new();
    assert!(!state.is_open());
    assert!(state.server_id().is_none());
    state.open(42, ServerSettingsSection::Overview);
    assert!(state.is_open());
    assert!(state.os_window_desired());
    assert_eq!(state.server_id(), Some(42));
    assert_eq!(state.section(), ServerSettingsSection::Overview);
    state.select(ServerSettingsSection::Roles);
    assert_eq!(state.section(), ServerSettingsSection::Roles);
    state.close();
    assert!(!state.is_open());
    assert!(state.server_id().is_none());
}

#[test]
fn server_settings_escape_closes_when_no_popup() {
    let mut state = ServerSettingsState::new();
    state.open(7, ServerSettingsSection::Members);
    state.escape(true);
    assert!(state.is_open(), "escape with a popup open only closes the popup");
    state.escape(false);
    assert!(!state.is_open());
}

#[test]
fn server_settings_focus_is_one_shot_on_open() {
    let mut state = ServerSettingsState::new();
    state.open(1, ServerSettingsSection::Overview);
    assert!(state.take_needs_focus());
    assert!(!state.take_needs_focus());
    state.select(ServerSettingsSection::Badges);
    assert!(!state.take_needs_focus(), "section change does not re-focus");
    state.open(1, ServerSettingsSection::Badges);
    assert!(state.take_needs_focus(), "re-open requests focus again");
}

#[test]
fn server_settings_open_replaces_server_and_section() {
    let mut state = ServerSettingsState::new();
    state.open(1, ServerSettingsSection::Overview);
    state.open(9, ServerSettingsSection::Roles);
    assert_eq!(state.server_id(), Some(9));
    assert_eq!(state.section(), ServerSettingsSection::Roles);
}

#[test]
fn server_rail_item_menu_offers_server_settings() {
    let bare = server_rail_item_menu(4, None, &[]);
    assert!(context_menu_labels(&bare).contains(&"Server Settings"));
    assert!(
        context_menu_actions(&bare).contains(&ContextMenuAction::ServerSettings { server_id: 4 })
    );

    let in_folder = server_rail_item_menu(4, Some(9), &[("Games".into(), 9)]);
    assert!(context_menu_labels(&in_folder).contains(&"Server Settings"));
    assert!(context_menu_actions(&in_folder)
        .contains(&ContextMenuAction::ServerSettings { server_id: 4 }));
}
