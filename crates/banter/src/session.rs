//! Remembered login. Two clients share this file, so the last login wins the next launch.
//!
//! Sign-in can opt out with **Stay signed in** unchecked: the token stays in memory for
//! this run only, and any previous `client-session.json` is cleared so relaunch asks again.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SessionFile {
    token: String,
}

pub fn session_path(data_dir: &Path) -> PathBuf {
    data_dir.join("client-session.json")
}

pub fn save_token(data_dir: &Path, token: &str) -> std::io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let body = serde_json::to_vec_pretty(&SessionFile { token: token.to_string() })
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    fs::write(session_path(data_dir), body)
}

pub fn load_token(data_dir: &Path) -> Option<String> {
    let bytes = fs::read(session_path(data_dir)).ok()?;
    let file: SessionFile = serde_json::from_slice(&bytes).ok()?;
    if file.token.is_empty() {
        None
    } else {
        Some(file.token)
    }
}

/// Removes `client-session.json` when present. Missing file is success (Log Out / no persist).
pub fn clear_token(data_dir: &Path) -> std::io::Result<()> {
    match fs::remove_file(session_path(data_dir)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// After a successful login or register: persist the token only when Stay signed in is on.
/// When off, clear any prior saved session so the next cold start shows sign-in.
pub fn persist_login(data_dir: &Path, token: &str, stay_signed_in: bool) -> std::io::Result<()> {
    if stay_signed_in {
        save_token(data_dir, token)
    } else {
        clear_token(data_dir)
    }
}
