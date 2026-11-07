//! Local Banter server.
//!
//! Accounts, servers, channels, and messages live in PostgreSQL
//! (`BANTER_DATABASE_URL`). Uploaded files stay under `BANTER_DATA` and survive
//! after the process exits. Voice membership, calls, and presence live in
//! memory and reset when the server stops.
//!
//! Deferred, and not implemented here: reactions, text chat inside a voice
//! channel, group chats, kick, and ban. Profile About Me, custom status, and
//! guild roles are implemented. See `docs/foundation.md`.

mod badges;
mod bots;
mod commands;
mod plugins;
mod script;
mod channel_icons;
mod codec;
mod cosmetics;
mod debuglog;
mod errcode;
mod error;
mod folders;
mod http;
mod live;
mod migrate;
mod model;
mod roles;
mod room;
mod sfu;
mod store;
mod version;

pub use badges::{badge_exists, badge_item, badges_catalog, BadgeItem, BadgeShape, MAX_EQUIPPED_BADGES};
pub use channel_icons::{is_channel_icon, normalize_channel_icon, CHANNEL_ICON_CHOICES};
pub use commands::{
    default_command_settings, normalize_command_aliases, validate_command_name,
    validate_command_prefix, DEFAULT_COMMAND_PREFIX,
};
pub use codec::{sdp_fixture, sdp_matches_contract, stun_url, turn_server, H264_FMTP, H264_PROFILE_LEVEL_ID};
pub use cosmetics::{
    cosmetics_catalog, display_name_style_color, CosmeticItem, CosmeticKind, DEFAULT_THEME_PRIMARY,
    DEFAULT_THEME_SECONDARY,
};
pub use errcode::{
    format_banter_notice, BanterIssue, ERR_CLIENT_TOO_OLD, ERR_HTTP_PARSE, ERR_HTTP_STATUS,
    ERR_HTTP_TRANSPORT, ERR_SERVER_TOO_OLD, ERR_VC_DEVICE, ERR_VC_FLAGS, ERR_VC_JOIN, ERR_VC_LEAVE,
    ERR_VC_LOOPBACK, ERR_VC_PUBLISH, ERR_VERSION_UNREACHABLE, ERR_WS_CONNECT, ERR_WS_DROPPED,
};
pub use error::BanterError;
pub use folders::{folder_color_ok, folder_depth, would_cycle, MAX_SERVER_FOLDERS, MAX_SERVER_FOLDER_DEPTH};
pub use live::Live;
pub use model::*;
pub use bots::{call_is_null, event_gate, known_intent, EventGate, INTENTS};
pub use plugins::{
    ident_ok, inspect_plugin_zip, parse_manifest, parse_version, plugin_kind_ok, zip_files, PluginInfo, PluginManifest,
};
pub use roles::{
    combine_permissions, grants_only_held, has_permission, next_role_position, role_color_ok, top_hoisted_role,
    top_role_color, EVERYONE_NAME, EVERYONE_PERMISSIONS, MAX_GUILD_ROLES, PERM_ADMINISTRATOR, PERM_CREATE_INVITE,
    PERM_MANAGE_BOTS, PERM_MANAGE_CHANNELS, PERM_MANAGE_PLUGINS, PERM_MANAGE_ROLES, ROLE_PERMISSION_MASK,
};
pub use script::{export_allowed, memory_warning, run_client_script, Addon, Limits, RunReport, Widget};
pub use room::{
    attach_publish_tracks, inbound_rtp_layer, inbound_rtp_layer_for_packet, mids_by_track, publish_offer_sdp,
    rewrite_advertised_candidate, rewrite_forward_rtp, sample_bytes, simulcast_extensions, video_rtp_duration_ms,
    PublishTracks, CAMERA_SSRC_Q, H264_PT, OPUS_PT, SCREEN_SSRC_Q, STREAM_AUDIO_SSRC, USER_AUDIO_SSRC,
};
pub use sfu::{bench_eight_publishers, BenchReport, ForwardTable};
pub use store::Store;
pub use version::{
    check_remote_version, format_version_notice, health_payload, unreachable_version_check,
    BanterVersionReport, VersionCheck, BANTER_VERSION, MIN_CLIENT_PROTOCOL, MIN_SERVER_PROTOCOL,
    PROTOCOL,
};

pub const DEFAULT_ADDR: &str = "127.0.0.1:3970";

/// Default Postgres URL for Local and bare-metal runs (Compose uses host `postgres`).
pub const DEFAULT_DATABASE_URL: &str = "postgres://banter:banter@127.0.0.1:5432/banter";

/// Address the server binary should bind.
///
/// A non-empty command-line address wins. Otherwise a non-empty `BANTER_BIND`
/// or `BANTER_PORT` selects `{bind or 0.0.0.0}:{port or 3970}`. When the
/// argument and both variables are absent, the desktop default is used.
pub fn listen_addr(cli: Option<&str>, bind: Option<&str>, port: Option<&str>) -> String {
    if let Some(addr) = present(cli) {
        return addr.to_string();
    }
    let bind = present(bind);
    let port = present(port);
    if bind.is_none() && port.is_none() {
        return DEFAULT_ADDR.to_string();
    }
    format!("{}:{}", bind.unwrap_or("0.0.0.0"), port.unwrap_or("3970"))
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

pub fn serve(addr: &str, data_dir: &std::path::Path, database_url: &str) -> Result<(), BanterError> {
    http::serve(addr, data_dir, database_url)
}

pub fn default_data_dir() -> std::path::PathBuf {
    resolve_data_dir(std::env::var("BANTER_DATA").ok().as_deref(), &platform_data_dir())
}

/// `BANTER_DATABASE_URL` wins when non-empty; otherwise [`DEFAULT_DATABASE_URL`].
pub fn database_url() -> String {
    resolve_database_url(std::env::var("BANTER_DATABASE_URL").ok().as_deref())
}

pub fn resolve_database_url(banter_database_url: Option<&str>) -> String {
    present(banter_database_url)
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string())
}

/// `BANTER_DATA` wins when non-empty. Otherwise the platform home plus `Banter`.
pub fn resolve_data_dir(banter_data: Option<&str>, platform_home: &std::path::Path) -> std::path::PathBuf {
    if let Some(path) = banter_data.filter(|text| !text.is_empty()) {
        return std::path::PathBuf::from(path);
    }
    platform_home.join("Banter")
}

fn platform_data_dir() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            if !base.is_empty() {
                return std::path::PathBuf::from(base);
            }
        }
        return std::env::temp_dir();
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join("Library/Application Support");
        }
        return std::env::temp_dir();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            if !xdg.is_empty() {
                return std::path::PathBuf::from(xdg);
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(".local/share");
        }
        std::env::temp_dir()
    }
}
