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
