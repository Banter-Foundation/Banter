//! Process-wide ICE config from `GET /v1/ice`.
//!
//! Desktop clients rarely set `BANTER_TURN*` themselves. After sign-in we pull
//! the host’s STUN/TURN so PeerConnections off-LAN can relay instead of dying
//! on host-only candidates.

use std::sync::Mutex;

use banter_server::IceConfig;

static ICE: Mutex<Option<IceConfig>> = Mutex::new(None);

pub fn store(config: IceConfig) {
    *ICE.lock().expect("ice cache") = Some(config);
}

pub fn clear() {
    *ICE.lock().expect("ice cache") = None;
}

pub fn get() -> Option<IceConfig> {
    ICE.lock().expect("ice cache").clone()
}
