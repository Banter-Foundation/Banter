//! Optional Lucide icon ids for channels.
//!
//! Values are Lucide names that Banter already vendors for the desktop client.
//! Uploaded/custom image icons are out of scope (chat uploads are disabled).

use crate::error::BanterError;

/// Curated Lucide names a server manager may assign to a channel.
pub const CHANNEL_ICON_CHOICES: &[&str] = &[
    "hash",
    "volume-2",
    "pencil",
    "message-circle",
    "users",
    "mic",
    "mic-off",
    "headphones",
    "video",
    "monitor",
    "phone",
    "image",
    "file-text",
    "sticky-note",
    "crown",
    "settings",
    "search",
    "signal",
    "circle",
    "square",
    "type",
    "mouse-pointer-2",
    "check",
    "plus",
    "paperclip",
    "reply",
];

/// True when `name` is an allowed channel Lucide id.
pub fn is_channel_icon(name: &str) -> bool {
    CHANNEL_ICON_CHOICES.iter().any(|choice| *choice == name)
}

/// Normalize an optional icon from create/update bodies.
///
/// Empty or whitespace becomes `None` (no custom icon). Unknown names are
/// `InvalidName`.
pub fn normalize_channel_icon(raw: Option<&str>) -> Result<Option<String>, BanterError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if is_channel_icon(name) {
        Ok(Some(name.to_string()))
    } else {
        Err(BanterError::InvalidName)
    }
}
