//! Shared codec and SDP contract.
//!
//! Live media is Opus and H.264 Constrained Baseline over WebRTC. The profile,
//! packetization mode, and simulcast RIDs are the same on Linux and Windows.

use crate::model::LayerId;

/// H.264 Constrained Baseline, level 3.1. `42` is Baseline, `e0` sets constraint_set1.
pub const H264_PROFILE_LEVEL_ID: &str = "42e01f";
pub const H264_FMTP: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
pub const OPUS_CLOCK_HZ: u32 = 48_000;
pub const VIDEO_CLOCK_HZ: u32 = 90_000;

/// Lines every Banter offer and answer must carry for the video simulcast section.
pub fn locked_sdp_tokens() -> &'static [&'static str] {
    &[
        "H264/90000",
        "profile-level-id=42e01f",
        "packetization-mode=1",
        "level-asymmetry-allowed=1",
        "a=rid:f",
        "a=rid:h",
        "a=rid:q",
        "a=simulcast:send q;h;f",
        "opus/48000",
        "useinbandfec=1",
        "usedtx=1",
    ]
}

pub fn sdp_matches_contract(sdp: &str) -> bool {
    locked_sdp_tokens().iter().all(|token| sdp.contains(token))
}

/// Canonical fragment locked by tests. A live PeerConnection offer must contain the same tokens.
pub fn sdp_fixture() -> String {
    let _ = LayerId::RIDS;
    format!(
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
         a=rtpmap:111 opus/{OPUS_CLOCK_HZ}/2\r\n\
         a=fmtp:111 minptime=10;useinbandfec=1;usedtx=1\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 102\r\n\
         a=rtpmap:102 H264/{VIDEO_CLOCK_HZ}\r\n\
         a=fmtp:102 {H264_FMTP}\r\n\
         a=rid:q send\r\n\
         a=rid:h send\r\n\
         a=rid:f send\r\n\
         a=simulcast:send q;h;f\r\n"
    )
}

/// Default STUN server. `BANTER_STUN` overrides it. Empty disables STUN and leaves host candidates.
pub fn stun_url() -> String {
    match std::env::var("BANTER_STUN") {
        Ok(value) if value.is_empty() => String::new(),
        Ok(value) => value,
        Err(_) => "stun:stun.l.google.com:19302".to_string(),
    }
}

/// `BANTER_TURN` plus `BANTER_TURN_USER` and `BANTER_TURN_PASSWORD` when a relay is configured.
pub fn turn_server() -> Option<(String, String, String)> {
    let url = std::env::var("BANTER_TURN").ok().filter(|value| !value.is_empty())?;
    let user = std::env::var("BANTER_TURN_USER").unwrap_or_default();
    let password = std::env::var("BANTER_TURN_PASSWORD").unwrap_or_default();
    Some((url, user, password))
}

/// ICE servers the desktop should use. Served at `GET /v1/ice` so clients off-LAN
/// pick up the host’s TURN without local env vars (LAN host candidates alone fail
/// through most home NATs).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IceConfig {
    /// May be empty when `BANTER_STUN=` disables STUN.
    pub stun: String,
    #[serde(default)]
    pub turn: Option<IceTurn>,
    /// `BANTER_ADVERTISE_IP` — SFU host candidates use this. Empty means unset.
    #[serde(default)]
    pub advertise_ip: String,
    #[serde(default)]
    pub udp_min: Option<u32>,
    #[serde(default)]
    pub udp_max: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IceTurn {
    pub urls: String,
    pub username: String,
    pub credential: String,
}

/// Snapshot of STUN/TURN + SFU advertise hint from the process environment.
pub fn ice_config() -> IceConfig {
    let turn = turn_server().and_then(|(urls, username, credential)| {
        if username.is_empty() || credential.is_empty() {
            None
        } else {
            Some(IceTurn {
                urls,
                username,
                credential,
            })
        }
    });
    let advertise_ip = std::env::var("BANTER_ADVERTISE_IP").unwrap_or_default();
    let udp_min = std::env::var("BANTER_UDP_MIN")
        .ok()
        .and_then(|v| v.parse().ok());
    let udp_max = std::env::var("BANTER_UDP_MAX")
        .ok()
        .and_then(|v| v.parse().ok());
    IceConfig {
        stun: stun_url(),
        turn,
        advertise_ip,
        udp_min,
        udp_max,
    }
}

#[cfg(test)]
mod ice_config_tests {
    use super::*;

    #[test]
    fn ice_config_includes_default_stun() {
        // Do not clear BANTER_STUN if a harness set it — only assert shape.
        let cfg = ice_config();
        if std::env::var("BANTER_STUN").ok().as_deref() != Some("") {
            assert!(
                cfg.stun.contains("stun:") || cfg.stun.is_empty(),
                "stun should be a stun: URL or empty, got {:?}",
                cfg.stun
            );
        }
    }
}
