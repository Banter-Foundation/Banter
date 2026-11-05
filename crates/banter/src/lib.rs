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
mod logic;
mod mods;
mod media;
mod platform;
mod plugins;
mod profile;
mod profile_paint;
mod profiler;
mod realtime;
mod session;
mod settings;
mod skin;
pub mod theme;
mod ui;

pub use api::{Client, websocket_url};
pub use banter_server::{ServerFolder, ServerRail, ServerRailServer, ServerSummary};
pub use bfmd::{parse as parse_bfmd, plain_text as bfmd_plain_text, Flag as BfmdFlag};
pub use icons::rasterize_lucide;
pub use logic::*;
pub use mods::{default_mod_order, disable_mod, enable_mod, memory_warning, move_mod, ModHost};
pub use plugins::{
    ensure_plugin_samples, local_plugins, plugin_file_lines, plugin_server_lines, plugin_usage, plugin_version_is_newer,
    LocalPlugin,
};
pub use media::{
    camera_preview_rgba, current_camera_frame, decode_camera_frame, hardware_encoder_available,
    pick_preferred_camera_mode, preferred_camera_height, preferred_camera_width, CameraModeChoice,
};
pub use profile::*;
pub use session::{clear_token, load_token, persist_login, save_token};
pub use settings::*;

pub fn run() -> Result<(), String> {
    platform::run()
}
