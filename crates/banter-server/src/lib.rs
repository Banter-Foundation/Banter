//! Local Banter server.
//!
//! Accounts, servers, channels, messages, and files are stored in SQLite and
//! survive after the process exits. Voice membership, calls, and presence live
//! in memory and reset when the server stops.
//!
//! Deferred, and not implemented here: About Me, custom status, reactions,
//! bold and italic, text chat inside a voice channel, group chats, roles,
//! kick, and ban. See `docs/foundation.md`.

mod codec;
mod error;
mod http;
mod live;
mod model;
mod store;

pub use codec::{decode_client_media, decode_media_frame, encode_client_media, encode_media_frame};
pub use error::BanterError;
pub use live::Live;
pub use model::*;
pub use store::Store;

pub const DEFAULT_ADDR: &str = "127.0.0.1:3970";

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

pub fn serve(addr: &str, data_dir: &std::path::Path) -> Result<(), BanterError> {
    http::serve(addr, data_dir)
}

pub fn default_data_dir() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("BANTER_DATA") {
        if !path.is_empty() {
            return std::path::PathBuf::from(path);
        }
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
        std::env::temp_dir().to_string_lossy().into_owned()
    });
    std::path::PathBuf::from(base).join("Banter")
}
