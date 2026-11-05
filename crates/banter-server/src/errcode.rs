//! Shared Banter error catalog: stable codes, short user text, optional detail.
//!
//! Pattern for call sites (client or server):
//! 1. Pick a `BANTER_*` code from this module (or `BanterError::code()`).
//! 2. Build a `BanterIssue` with a short user-facing message.
//! 3. Attach technical detail with `with_detail` for logs; keep the toast on `user_text()`.
//! 4. Voice/media paths use `ERR_VC_*` / `ERR_WS_*` first; migrate other call sites as you touch them.

use crate::error::BanterError;

/// HTTP status with empty or unreadable body (never show bare "request failed").
pub const ERR_HTTP_STATUS: &str = "BANTER_HTTP_STATUS";
/// Transport/timeout/DNS failure talking to the Banter API.
pub const ERR_HTTP_TRANSPORT: &str = "BANTER_HTTP_TRANSPORT";
/// Response body was not usable JSON for the expected type.
pub const ERR_HTTP_PARSE: &str = "BANTER_HTTP_PARSE";

/// Could not join a voice channel.
pub const ERR_VC_JOIN: &str = "BANTER_VC_JOIN_FAILED";
/// Could not leave a voice channel.
pub const ERR_VC_LEAVE: &str = "BANTER_VC_LEAVE_FAILED";
/// Could not publish mute/deafen/camera/screen flags while in voice.
pub const ERR_VC_FLAGS: &str = "BANTER_VC_FLAGS_FAILED";
/// Could not publish local media (mic/camera/screen) onto the SFU/P2P path.
pub const ERR_VC_PUBLISH: &str = "BANTER_VC_PUBLISH_FAILED";
/// Could not open microphone, speakers, or camera.
pub const ERR_VC_DEVICE: &str = "BANTER_VC_DEVICE_FAILED";
/// Could not open WASAPI speaker/app loopback (includes process loopback).
pub const ERR_VC_LOOPBACK: &str = "BANTER_VC_LOOPBACK_FAILED";

/// Websocket to `/v1/ws` failed to connect.
pub const ERR_WS_CONNECT: &str = "BANTER_WS_CONNECT_FAILED";
/// Websocket dropped after connect.
pub const ERR_WS_DROPPED: &str = "BANTER_WS_DROPPED";

/// Remote Banter host is older than this client needs (BA-91).
pub const ERR_SERVER_TOO_OLD: &str = "BANTER_SERVER_TOO_OLD";
/// This client is older than the remote host requires (BA-91).
pub const ERR_CLIENT_TOO_OLD: &str = "BANTER_CLIENT_TOO_OLD";
/// Health/version probe could not reach the server (BA-91).
pub const ERR_VERSION_UNREACHABLE: &str = "BANTER_VERSION_UNREACHABLE";

/// One Banter failure: stable code, short user string, optional technical detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BanterIssue {
    code: String,
    message: String,
    detail: Option<String>,
}

impl BanterIssue {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn from_server(error: BanterError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Toast / notice strip: short message plus the stable code.
    pub fn user_text(&self) -> String {
        format_banter_notice(&self.code, &self.message)
    }

    /// Log line: user text plus optional detail.
    pub fn log_text(&self) -> String {
        match &self.detail {
            Some(detail) => format!("{} | {detail}", self.user_text()),
            None => self.user_text(),
        }
    }
}

impl std::fmt::Display for BanterIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.user_text())
    }
}

/// Notice strip text: `{message} ({CODE})`.
pub fn format_banter_notice(code: &str, message: &str) -> String {
    format!("{message} ({code})")
}
