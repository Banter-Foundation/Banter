//! Banter desktop client.
//!
//! The window is GLFW and OpenGL. Dear ImGui is the docking branch (dear-imgui-rs).
//! If nothing is listening on 127.0.0.1:3970, the client starts the local server.
//!
//! Deferred, and not implemented: About Me, custom status, reactions, bold and italic,
//! text chat inside a voice channel, group chats, roles, kick, and ban.
//! See `docs/foundation.md`.

mod api;
mod icons;
mod logic;
mod media;
mod platform;
mod profiler;
mod realtime;
mod session;
mod settings;
mod skin;
pub mod theme;
mod ui;

pub use api::{Client, websocket_url};
pub use icons::rasterize_lucide;
pub use logic::*;
pub use media::{current_camera_frame, decode_camera_frame};
pub use session::{load_token, save_token};
pub use settings::*;

pub fn run() -> Result<(), String> {
    platform::run()
}
