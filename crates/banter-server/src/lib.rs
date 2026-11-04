//! Local Banter server.
//!
//! Accounts, servers, channels, and messages live in PostgreSQL
//! (`BANTER_DATABASE_URL`). Uploaded files stay under `BANTER_DATA` and survive
//! after the process exits. Voice membership, calls, and presence live in
//! memory and reset when the server stops.
//!
//! Deferred, and not implemented here: reactions, bold and italic, text chat
//! inside a voice channel, group chats, roles, kick, and ban.
//! Profile About Me / custom status / cosmetics are implemented for Discord
//! profile parity. See `docs/foundation.md`.

mod codec;
mod cosmetics;
mod error;
mod http;
mod live;
mod model;
mod room;
mod sfu;
mod store;

pub use codec::{sdp_fixture, sdp_matches_contract, stun_url, turn_server, H264_FMTP, H264_PROFILE_LEVEL_ID};
pub use cosmetics::{
    cosmetics_catalog, display_name_style_color, CosmeticItem, CosmeticKind, DEFAULT_THEME_PRIMARY,
    DEFAULT_THEME_SECONDARY,
};
pub use error::BanterError;
pub use live::Live;
pub use model::*;
pub use room::{
    attach_publish_tracks, inbound_rtp_layer, inbound_rtp_layer_for_packet, publish_offer_sdp, rewrite_forward_rtp,
    sample_bytes, PublishTracks, CAMERA_SSRC_Q, H264_PT, OPUS_PT, SCREEN_SSRC_Q, STREAM_AUDIO_SSRC, USER_AUDIO_SSRC,
};
pub use sfu::{bench_eight_publishers, BenchReport, ForwardTable};
pub use store::Store;

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
