//! Banter release label and protocol revision.
//!
//! Display strings follow the GitHub release scheme (`Version-1-test-N`). Compatibility
//! is decided by numeric `protocol` / `min_*_protocol`, not equal labels — so a client
//! can refuse a stale host (for example pre–PR #19 SFU) even when health still returns
//! `{"ok":true}`.
//!
//! Version mismatch codes live in [`crate::errcode`] with the rest of the BA-88 catalog.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errcode::format_banter_notice;
pub use crate::errcode::{ERR_CLIENT_TOO_OLD, ERR_SERVER_TOO_OLD, ERR_VERSION_UNREACHABLE};

/// Human-facing Banter release label (bump when cutting a Version-1-test-N tag).
pub const BANTER_VERSION: &str = "Version-1-test-13.1";

/// Wire / feature protocol for this build.
///
/// Bump when the client requires server behavior older hosts lack (SFU RTP rewrite,
/// badges API, ICE advertise, debug endpoint era, …). Missing `protocol` on health
/// is treated as `0`.
pub const PROTOCOL: u32 = 2;

/// Oldest client protocol this server accepts.
pub const MIN_CLIENT_PROTOCOL: u32 = 1;

/// Oldest server protocol this client accepts.
pub const MIN_SERVER_PROTOCOL: u32 = 1;

/// Parsed `/v1/health` body. Older hosts may only send `ok`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanterVersionReport {
    #[serde(default = "default_ok")]
    pub ok: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub min_client_protocol: Option<u32>,
}

fn default_ok() -> bool {
    true
}

/// Result of comparing local build vs a remote health report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionCheck {
    /// Safe to use the host. Optional warn when labels differ but protocol matches.
    Compatible { warn: Option<String> },
    /// Do not stay connected / do not sign in. `message` is short user text (no code).
    Block { code: &'static str, message: String },
}

/// JSON body for `GET /v1/health` (and `/v1/version`).
pub fn health_payload() -> Value {
    json!({
        "ok": true,
        "version": BANTER_VERSION,
        "protocol": PROTOCOL,
        "min_client_protocol": MIN_CLIENT_PROTOCOL,
    })
}

/// Notice strip text: same as [`format_banter_notice`] (`{message} ({CODE})`).
pub fn format_version_notice(code: &str, message: &str) -> String {
    format_banter_notice(code, message)
}

/// Compare a remote health/version report to this build.
///
/// Prefers protocol floors over equal version strings. A host with no `protocol`
/// field is treated as revision `0` and fails [`MIN_SERVER_PROTOCOL`].
pub fn check_remote_version(report: &BanterVersionReport) -> VersionCheck {
    let server_protocol = report.protocol.unwrap_or(0);
    if server_protocol < MIN_SERVER_PROTOCOL {
        let host = report
            .version
            .as_deref()
            .unwrap_or("an unknown older build");
        return VersionCheck::Block {
            code: ERR_SERVER_TOO_OLD,
            message: format!(
                "this Banter server is too old for this client (server {host}, protocol {server_protocol}; need protocol {MIN_SERVER_PROTOCOL}+). Upgrade the server"
            ),
        };
    }

    let min_client = report.min_client_protocol.unwrap_or(0);
    if PROTOCOL < min_client {
        let host = report.version.as_deref().unwrap_or("the server");
        return VersionCheck::Block {
            code: ERR_CLIENT_TOO_OLD,
            message: format!(
                "this Banter client is too old for {host} (client protocol {PROTOCOL}; server needs {min_client}+). Update the client"
            ),
        };
    }

    let warn = match report.version.as_deref() {
        Some(remote) if remote != BANTER_VERSION => Some(format!(
            "client {BANTER_VERSION} talking to server {remote} (protocol {server_protocol} ok)"
        )),
        _ => None,
    };
    VersionCheck::Compatible { warn }
}

/// Health probe failed (transport / timeout). Used when the client expected a reachable host.
pub fn unreachable_version_check(detail: impl Into<String>) -> VersionCheck {
    let detail = detail.into();
    VersionCheck::Block {
        code: ERR_VERSION_UNREACHABLE,
        message: if detail.is_empty() {
            "couldn't reach the Banter server to check its version".into()
        } else {
            format!("couldn't reach the Banter server to check its version: {detail}")
        },
    }
}
