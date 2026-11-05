//! Banter desktop client.
//!
//! The window is GLFW and OpenGL. Dear ImGui is the docking branch (dear-imgui-rs).
//! Startup does not require a local server unless the user is already logged in on
//! Local (or they choose Local in Developer settings). A configured Remote is used
//! as-is. Fresh Local with nothing listening opens the UI and asks for a remote.
//!
//! Deferred, and not implemented: reactions, text chat inside a voice channel,
//! group chats, kick, and ban. Profile About Me / custom status / cosmetics
//! and guild roles are implemented. Chat bold/italic and related styles use
//! BFMD (`bfmd` module). See `docs/foundation.md`.

mod api;
pub mod bfmd;
mod bfmd_paint;
mod icons;
pub mod layout;
mod logic;
mod media;
mod platform;
mod profile;
mod profile_paint;
mod profiler;
mod realtime;
mod session;
mod settings;
mod skin;
pub mod theme;
mod ui;

pub use api::{websocket_url, ApiError, Client};
pub use banter_server::{
    format_banter_notice, ServerFolder, ServerRail, ServerRailServer, ServerSummary, ERR_HTTP_PARSE,
    ERR_HTTP_STATUS, ERR_HTTP_TRANSPORT, ERR_VC_FLAGS, ERR_VC_JOIN, ERR_VC_LEAVE, ERR_VC_PUBLISH, ERR_WS_CONNECT,
    ERR_WS_DROPPED,
};
pub use bfmd::{parse as parse_bfmd, plain_text as bfmd_plain_text, Flag as BfmdFlag};
pub use icons::rasterize_lucide;
pub use layout::{Region, ShellColumns, discord_shell_columns, shell_columns};
pub use logic::*;
pub use media::{
    camera_preview_rgba, current_camera_frame, decode_camera_frame, hardware_encoder_available,
    pick_preferred_camera_mode, preferred_camera_height, preferred_camera_width, CameraModeChoice,
};
pub use profile::*;
pub use session::{clear_token, load_token, persist_login, save_token};
pub use settings::*;

/// This client's Banter release label (same constant as the embedded server).
pub fn client_version() -> &'static str {
    banter_server::BANTER_VERSION
}

/// This client's protocol revision.
pub fn client_protocol() -> u32 {
    banter_server::PROTOCOL
}

/// Oldest server protocol this client will accept.
pub fn min_server_protocol() -> u32 {
    banter_server::MIN_SERVER_PROTOCOL
}

pub fn run() -> Result<(), String> {
    platform::run()
}
