//! Banter desktop client.
//!
//! The window is GLFW and OpenGL. Dear ImGui is the docking branch (dear-imgui-rs).
//! Startup does not require a local server unless the user is already logged in on
//! Local (or they choose Local in Developer settings). A configured Remote is used
//! as-is. Fresh Local with nothing listening opens the UI and asks for a remote.
//!
//! Deferred, and not implemented: reactions, text chat inside a voice channel,
//! group chats, roles, kick, and ban. Profile About Me / custom status /
//! cosmetics are implemented for Discord profile parity. Chat bold/italic and
//! related styles use BFMD (`bfmd` module). See `docs/foundation.md`.

mod api;
pub mod bfmd;
mod bfmd_paint;
mod icons;
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

pub use api::{Client, websocket_url};
pub use bfmd::{parse as parse_bfmd, plain_text as bfmd_plain_text, Flag as BfmdFlag};
pub use icons::rasterize_lucide;
pub use logic::*;
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
