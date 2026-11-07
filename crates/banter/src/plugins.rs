//! Plugin packages sitting in `<data>/plugins`, next to `<data>/mods`.

use std::fs;
use std::path::Path;

use banter_server::{inspect_plugin_zip, parse_manifest, parse_version, zip_files};

#[derive(Clone, Debug)]
pub struct LocalPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub client: String,
    pub permissions: Vec<String>,
    pub failed: Option<String>,
    pub zip: Vec<u8>,
}

/// Plain lines for the admin allow dialog. `io` and `os` are omitted; they use the second dialog.
pub fn plugin_server_lines(permissions: &[String]) -> Vec<String> {
    permissions
        .iter()
        .filter_map(|name| server_line(name))
        .collect()
}

fn server_line(name: &str) -> Option<String> {
    let line = match name {
        "storage" => "Save data for this plugin on this server.",
        "messages" => {
            "Send messages as this plugin, and change messages people post on this server."
        }
        "channels" => "Create and delete channels on this server.",
        "members_read" => "Read the member list of this server.",
        "events" => "Run when someone posts, joins, or leaves this server.",
        "sync" => "Keep channel state for this plugin on this server.",
        "commands" => "Run this plugin's commands in this server.",
        _ => return None,
    };
    Some(line.to_string())
}

/// Second warning, shown only when the server list checks `io` or `os`.
pub fn plugin_file_lines(permissions: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    if permissions.iter().any(|name| name == "io" || name == "os") {
        lines.push("Banter is not responsible for addons that modify your files.".to_string());
    }
    if permissions.iter().any(|name| name == "io") {
        lines.push("This mod can read and write files on this server.".to_string());
    }
    if permissions.iter().any(|name| name == "os") {
        lines.push("This mod can run programs, read environment variables, and use other operating system functions.".to_string());
    }
    lines
}

/// What the person should do after the plugin is allowed, so the result is visible.
pub fn plugin_usage(id: &str) -> &'static str {
    match id {
        "welcome" => "New channel messages become test.",
        "roll" => {
            "Declares a roll command. Register it on CommandRegistry when client plugin commands ship; the composer registry is ready (BA-104)."
        }
        "notes" => "Stores a note on this server. The client half is not shown in the window yet.",
        _ => "The server script runs on this server after you allow it.",
    }
}

pub fn plugin_version_is_newer(local: &str, installed: &str) -> bool {
    match (parse_version(local), parse_version(installed)) {
        (Some(local), Some(installed)) => local > installed,
        _ => false,
    }
}

/// Folders with `banter.plugin.json`, and `.zip` files, in this directory.
pub fn local_plugins(folder: &Path) -> Vec<LocalPlugin> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            plugins.push(from_dir(&path));
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("zip") {
            plugins.push(from_zip(&path));
        }
    }
    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    plugins
}

/// First launch has no plugins folder. Drop the sample packages in beside mods.
pub fn ensure_plugin_samples(data_dir: &Path) {
    let root = data_dir.join("plugins");
    if root.is_dir() {
        return;
    }
    let files = [
        (
            "welcome/banter.plugin.json",
            include_str!("../../../examples/plugins/welcome/banter.plugin.json"),
        ),
        (
            "welcome/server/main.luau",
            include_str!("../../../examples/plugins/welcome/server/main.luau"),
        ),
        (
            "roll/banter.plugin.json",
            include_str!("../../../examples/plugins/roll/banter.plugin.json"),
        ),
        (
            "roll/server/main.luau",
            include_str!("../../../examples/plugins/roll/server/main.luau"),
        ),
        (
            "notes/banter.plugin.json",
            include_str!("../../../examples/plugins/notes/banter.plugin.json"),
        ),
        (
            "notes/server/main.luau",
            include_str!("../../../examples/plugins/notes/server/main.luau"),
        ),
        (
            "notes/client/main.luau",
            include_str!("../../../examples/plugins/notes/client/main.luau"),
        ),
    ];
    for (path, body) in files {
        let dest = root.join(path);
        if let Some(parent) = dest.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if fs::write(dest, body).is_err() {
            return;
        }
    }
}

fn from_dir(dir: &Path) -> LocalPlugin {
    let fallback = file_label(dir);
    let manifest_path = dir.join("banter.plugin.json");
    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(_) => return failed(fallback, "missing banter.plugin.json"),
    };
    let parsed = parse_manifest(&raw).ok();
    let mut files = Vec::new();
    if let Err(reason) = collect_files(dir, dir, &mut files) {
        return named_failure(parsed.as_ref(), fallback, &reason);
    }
    let borrowed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    let zip = match zip_files(&borrowed) {
        Ok(zip) => zip,
        Err(_) => return named_failure(parsed.as_ref(), fallback, "could not pack this plugin"),
    };
    match inspect_plugin_zip(&zip) {
        Ok(manifest) => LocalPlugin {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            client: manifest.client,
            permissions: manifest.permissions,
            failed: None,
            zip,
        },
        Err(_) => named_failure(parsed.as_ref(), fallback, "package is invalid"),
    }
}

fn from_zip(path: &Path) -> LocalPlugin {
    let fallback = file_label(path);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return failed(fallback, &err.to_string()),
    };
    match inspect_plugin_zip(&bytes) {
        Ok(manifest) => LocalPlugin {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            client: manifest.client,
            permissions: manifest.permissions,
            failed: None,
            zip: bytes,
        },
        Err(_) => failed(fallback, "package is invalid"),
    }
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, root, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|err| err.to_string())?;
        let name = rel.to_string_lossy().replace('\\', "/");
        if name.is_empty() || name.starts_with('/') || name.contains("..") {
            return Err("invalid path".into());
        }
        let allowed = name.ends_with(".luau")
            || name.ends_with(".json")
            || name.ends_with(".png")
            || name.ends_with(".webp");
        if !allowed {
            continue;
        }
        let bytes = fs::read(&path).map_err(|err| err.to_string())?;
        out.push((name, bytes));
    }
    Ok(())
}

fn named_failure(
    parsed: Option<&banter_server::PluginManifest>,
    fallback: String,
    reason: &str,
) -> LocalPlugin {
    match parsed {
        Some(manifest) => LocalPlugin {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            client: manifest.client.clone(),
            permissions: manifest.permissions.clone(),
            failed: Some(reason.to_string()),
            zip: Vec::new(),
        },
        None => failed(fallback, reason),
    }
}

fn failed(label: String, reason: &str) -> LocalPlugin {
    LocalPlugin {
        id: label.clone(),
        name: label,
        version: String::new(),
        client: String::new(),
        permissions: Vec::new(),
        failed: Some(reason.to_string()),
        zip: Vec::new(),
    }
}

fn file_label(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin")
        .to_string()
}
