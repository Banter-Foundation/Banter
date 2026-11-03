//! Remembered login. Two clients share this file, so the last login wins the next launch.

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
