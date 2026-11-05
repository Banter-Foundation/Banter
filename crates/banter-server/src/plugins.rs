//! Plugin packages: manifest rules, zip limits, and install on one server.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use sqlx::Row;

use crate::error::BanterError;
use crate::roles::PERM_MANAGE_PLUGINS;
use crate::script::{Addon, Limits, PluginLink};
use crate::store::Store;

const MAX_PACKAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 64;
const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub banter: String,
    pub server_entry: Option<String>,
    pub client: String,
    pub client_entry: Option<String>,
    pub permissions: Vec<String>,
    pub client_permissions: Vec<String>,
    pub channel_kinds: Vec<String>,
    pub raw: String,
}

pub fn ident_ok(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    (1..=31).contains(&rest.len()) && rest.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
}

pub fn plugin_kind_ok(value: &str) -> bool {
    let Some((id, name)) = value.split_once('.') else {
        return false;
    };
    ident_ok(id) && ident_ok(name)
}

pub fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

const SERVER_PERMISSIONS: &[&str] = &[
    "storage", "messages", "channels", "members_read", "events", "sync", "commands", "io", "os",
];
const CLIENT_PERMISSIONS: &[&str] = &["log", "ui", "read_screen", "sync", "io", "os"];

pub fn parse_manifest(raw: &str) -> Result<PluginManifest, BanterError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| BanterError::Invalid)?;
    let id = string_field(&value, "id")?;
    let name = string_field(&value, "name")?;
    let version = string_field(&value, "version")?;
    let banter = string_field(&value, "banter")?;
    let client = string_field(&value, "client")?;
    if !ident_ok(&id) || !(1..=32).contains(&name.chars().count()) || parse_version(&version).is_none() || banter != "1" {
        return Err(BanterError::Invalid);
    }
    if !matches!(client.as_str(), "none" | "optional" | "required") {
        return Err(BanterError::Invalid);
    }
    let server_entry = optional_string(&value, "server_entry")?;
    let client_entry = optional_string(&value, "client_entry")?;
    let permissions = string_list(&value, "permissions")?;
    let client_permissions = string_list(&value, "client_permissions")?;
    match client.as_str() {
        "none" => {
            if client_entry.is_some() || !client_permissions.is_empty() || server_entry.is_none() {
                return Err(BanterError::Invalid);
            }
        }
        "optional" | "required" => {
            if client_entry.is_none() {
                return Err(BanterError::Invalid);
            }
        }
        _ => return Err(BanterError::Invalid),
    }
    if permissions.iter().any(|item| !SERVER_PERMISSIONS.contains(&item.as_str())) {
        return Err(BanterError::Invalid);
    }
    if client_permissions.iter().any(|item| !CLIENT_PERMISSIONS.contains(&item.as_str())) {
        return Err(BanterError::Invalid);
    }
    let channel_kinds = channel_kinds(&value, &id)?;
    Ok(PluginManifest {
        id,
        name,
        version,
        banter,
        server_entry,
        client,
        client_entry,
        permissions,
        client_permissions,
        channel_kinds,
        raw: raw.to_string(),
    })
}

fn string_field(value: &Value, key: &str) -> Result<String, BanterError> {
    value.get(key).and_then(Value::as_str).map(str::to_string).ok_or(BanterError::Invalid)
}

fn optional_string(value: &Value, key: &str) -> Result<Option<String>, BanterError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        _ => Err(BanterError::Invalid),
    }
}

fn string_list(value: &Value, key: &str) -> Result<Vec<String>, BanterError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or(BanterError::Invalid))
            .collect(),
        _ => Err(BanterError::Invalid),
    }
}

fn channel_kinds(value: &Value, plugin_id: &str) -> Result<Vec<String>, BanterError> {
    let Some(items) = value.get("channel_kinds") else {
        return Ok(Vec::new());
    };
    let Some(items) = items.as_array() else {
        return Err(BanterError::Invalid);
    };
    let mut kinds = Vec::new();
    for item in items {
        let name = item.get("name").and_then(Value::as_str).ok_or(BanterError::Invalid)?;
        let label = item.get("label").and_then(Value::as_str).ok_or(BanterError::Invalid)?;
        if !ident_ok(name) || !(1..=32).contains(&label.chars().count()) {
            return Err(BanterError::Invalid);
        }
        kinds.push(format!("{plugin_id}.{name}"));
    }
    Ok(kinds)
}

#[derive(Clone, Debug)]
pub struct PackageFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn read_zip(bytes: &[u8]) -> Result<Vec<PackageFile>, BanterError> {
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(BanterError::Invalid);
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| BanterError::Invalid)?;
    if archive.len() > MAX_FILES {
        return Err(BanterError::Invalid);
    }
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| BanterError::Invalid)?;
        if file.is_dir() {
            continue;
        }
        let Some(path) = file.enclosed_name() else {
            return Err(BanterError::Invalid);
        };
        let path = path.to_string_lossy().replace('\\', "/");
        let suffix_ok = path.ends_with(".luau") || path.ends_with(".json") || path.ends_with(".png") || path.ends_with(".webp");
        if !suffix_ok {
            return Err(BanterError::Invalid);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| BanterError::Invalid)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(BanterError::Invalid);
        }
        files.push(PackageFile { path, bytes });
    }
    if files.len() > MAX_FILES {
        return Err(BanterError::Invalid);
    }
    Ok(files)
}

/// Pack plugin files into a zip the install route accepts. Paths are relative, with `/` separators.
pub fn zip_files(files: &[(&str, &[u8])]) -> Result<Vec<u8>, BanterError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let options = SimpleFileOptions::default();
    for (name, body) in files {
        if name.is_empty() || name.starts_with('/') || name.contains("..") {
            return Err(BanterError::Invalid);
        }
        zip.start_file(*name, options).map_err(|_| BanterError::Invalid)?;
        zip.write_all(body).map_err(|_| BanterError::Invalid)?;
    }
    zip.finish().map_err(|_| BanterError::Invalid)?;
    Ok(cursor.into_inner())
}

/// Read `banter.plugin.json` out of a plugin zip.
pub fn inspect_plugin_zip(bytes: &[u8]) -> Result<PluginManifest, BanterError> {
    let files = read_zip(bytes)?;
    let manifest = files
        .iter()
        .find(|file| file.path == "banter.plugin.json")
        .ok_or(BanterError::Invalid)?;
    parse_manifest(std::str::from_utf8(&manifest.bytes).map_err(|_| BanterError::Invalid)?)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub client: String,
    pub has_server: bool,
    pub has_client: bool,
    pub channel_kinds: Vec<String>,
}

impl Store {
    pub async fn install_plugin(&self, user_id: i64, server_id: i64, zip_bytes: &[u8]) -> Result<PluginInfo, BanterError> {
        self.require_flag(user_id, server_id, PERM_MANAGE_PLUGINS).await?;
        let files = read_zip(zip_bytes)?;
        let manifest_bytes = files
            .iter()
            .find(|file| file.path == "banter.plugin.json")
            .ok_or(BanterError::Invalid)?;
        let manifest = parse_manifest(std::str::from_utf8(&manifest_bytes.bytes).map_err(|_| BanterError::Invalid)?)?;
        if manifest.client == "none" && files.iter().any(|file| file.path.starts_with("client/")) {
            return Err(BanterError::Invalid);
        }
        for entry in [manifest.server_entry.as_deref(), manifest.client_entry.as_deref()].into_iter().flatten() {
            if !files.iter().any(|file| file.path == entry) {
                return Err(BanterError::Invalid);
            }
        }
        let enabled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugins WHERE server_id = $1 AND enabled AND plugin_id <> $2",
        )
        .bind(server_id)
        .bind(&manifest.id)
        .fetch_one(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        if enabled >= 16 {
            return Err(BanterError::Invalid);
        }
        if let Some((existing,)) = self.plugin_version(server_id, &manifest.id).await? {
            if parse_version(&manifest.version) <= parse_version(&existing) {
                return Err(BanterError::Conflict);
            }
        }
        let dir = self.root.join("plugins").join(server_id.to_string()).join(&manifest.id);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        write_tree(&dir, &files)?;
        let server_name = self.server_name(server_id).await?;
        let mut hard_error = None;
        if let Some(entry) = &manifest.server_entry {
            let source = fs::read_to_string(dir.join(entry)).map_err(|err| BanterError::Storage(err.to_string()))?;
            let link = PluginLink {
                pool: self.pool.clone(),
                server_id,
                plugin_id: manifest.id.clone(),
                server_name,
                version: manifest.version.clone(),
                client: manifest.client.clone(),
            };
            let mut addon = Addon::open(
                &manifest.id,
                &manifest.permissions,
                Limits { memory_mb: 8, unlocked: false, allow_shell: false },
                Some(link),
            )
            .map_err(|_| BanterError::Invalid)?;
            if addon.exec(&source).is_err() {
                hard_error = addon.report_hard();
            } else {
                self.plugins.lock().expect("plugins").insert((server_id, manifest.id.clone()), addon);
            }
        }
        sqlx::query(
            "INSERT INTO plugins (server_id, plugin_id, name, version, enabled, client, manifest, package_dir, hard_error)
             VALUES ($1, $2, $3, $4, TRUE, $5, $6, $7, $8)
             ON CONFLICT (server_id, plugin_id) DO UPDATE
             SET name = EXCLUDED.name, version = EXCLUDED.version, enabled = TRUE, client = EXCLUDED.client,
                 manifest = EXCLUDED.manifest, package_dir = EXCLUDED.package_dir, hard_error = EXCLUDED.hard_error",
        )
        .bind(server_id)
        .bind(&manifest.id)
        .bind(&manifest.name)
        .bind(&manifest.version)
        .bind(&manifest.client)
        .bind(&manifest.raw)
        .bind(dir.to_string_lossy().as_ref())
        .bind(&hard_error)
        .execute(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        Ok(info_from(&manifest, true, hard_error.is_none()))
    }

    /// Unload the script, delete the package, and drop its stored rows.
    pub async fn remove_plugin(&self, user_id: i64, server_id: i64, plugin_id: &str) -> Result<(), BanterError> {
        self.require_flag(user_id, server_id, PERM_MANAGE_PLUGINS).await?;
        let dir: Option<String> = sqlx::query_scalar(
            "SELECT package_dir FROM plugins WHERE server_id = $1 AND plugin_id = $2",
        )
        .bind(server_id)
        .bind(plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        let Some(dir) = dir else {
            return Err(BanterError::NotFound);
        };
        self.plugins.lock().expect("plugins").remove(&(server_id, plugin_id.to_string()));
        for statement in [
            "DELETE FROM plugin_storage WHERE server_id = $1 AND plugin_id = $2",
            "DELETE FROM plugin_opt_in WHERE server_id = $1 AND plugin_id = $2",
            "DELETE FROM plugin_sync WHERE server_id = $1 AND plugin_id = $2",
            "DELETE FROM plugins WHERE server_id = $1 AND plugin_id = $2",
        ] {
            sqlx::query(statement)
                .bind(server_id)
                .bind(plugin_id)
                .execute(&self.pool)
                .await
                .map_err(crate::store::db_err)?;
        }
        if Path::new(&dir).exists() {
            fs::remove_dir_all(&dir).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        Ok(())
    }

    pub async fn plugins(&self, user_id: i64, server_id: i64) -> Result<Vec<PluginInfo>, BanterError> {
        if !crate::store::is_member(&self.pool, user_id, server_id).await? {
            return Err(BanterError::NotMember);
        }
        let rows = sqlx::query(
            "SELECT plugin_id, name, version, enabled, client, manifest FROM plugins WHERE server_id = $1 ORDER BY plugin_id",
        )
        .bind(server_id)
        .fetch_all(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        let mut infos = Vec::new();
        for row in rows {
            let raw: String = row.get("manifest");
            let manifest = parse_manifest(&raw)?;
            let enabled: bool = row.get("enabled");
            infos.push(info_from(&manifest, enabled, true));
        }
        Ok(infos)
    }

    pub async fn opt_in_plugin(&self, user_id: i64, server_id: i64, plugin_id: &str, version: &str) -> Result<(), BanterError> {
        if !crate::store::is_member(&self.pool, user_id, server_id).await? {
            return Err(BanterError::NotMember);
        }
        let client: Option<String> = sqlx::query_scalar(
            "SELECT client FROM plugins WHERE server_id = $1 AND plugin_id = $2 AND version = $3 AND enabled",
        )
        .bind(server_id)
        .bind(plugin_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        match client.as_deref() {
            None => return Err(BanterError::NotFound),
            Some("none") => return Err(BanterError::NotFound),
            Some(_) => {}
        }
        sqlx::query(
            "INSERT INTO plugin_opt_in (server_id, plugin_id, user_id, version) VALUES ($1, $2, $3, $4)
             ON CONFLICT (server_id, plugin_id, user_id) DO UPDATE SET version = EXCLUDED.version",
        )
        .bind(server_id)
        .bind(plugin_id)
        .bind(user_id)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        Ok(())
    }

    pub async fn plugin_value(&self, server_id: i64, plugin_id: &str, key: &str) -> Result<Option<String>, BanterError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT value FROM plugin_storage WHERE server_id = $1 AND plugin_id = $2 AND key = $3",
        )
        .bind(server_id)
        .bind(plugin_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        Ok(value)
    }

    async fn plugin_version(&self, server_id: i64, plugin_id: &str) -> Result<Option<(String,)>, BanterError> {
        let version: Option<String> = sqlx::query_scalar(
            "SELECT version FROM plugins WHERE server_id = $1 AND plugin_id = $2",
        )
        .bind(server_id)
        .bind(plugin_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(crate::store::db_err)?;
        Ok(version.map(|version| (version,)))
    }

    async fn server_name(&self, server_id: i64) -> Result<String, BanterError> {
        sqlx::query_scalar("SELECT name FROM servers WHERE id = $1")
            .bind(server_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(crate::store::db_err)?
            .ok_or(BanterError::NotFound)
    }

    /// Deliver `message_created` to every running plugin on this server.
    pub fn notify_plugin_message(&self, server_id: i64, channel_id: i64, message_id: i64, body: &str) {
        let mut plugins = self.plugins.lock().expect("plugins");
        let ids: Vec<String> = plugins
            .keys()
            .filter(|(id, _)| *id == server_id)
            .map(|(_, plugin_id)| plugin_id.clone())
            .collect();
        for plugin_id in ids {
            if let Some(addon) = plugins.get_mut(&(server_id, plugin_id)) {
                let _ = addon.emit("message_created", channel_id, message_id, body);
            }
        }
    }
}

fn info_from(manifest: &PluginManifest, enabled: bool, _running: bool) -> PluginInfo {
    PluginInfo {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        enabled,
        client: manifest.client.clone(),
        has_server: manifest.server_entry.is_some(),
        has_client: manifest.client != "none",
        channel_kinds: manifest.channel_kinds.clone(),
    }
}

fn write_tree(dir: &Path, files: &[PackageFile]) -> Result<(), BanterError> {
    for file in files {
        let path = dir.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        let mut out = fs::File::create(path).map_err(|err| BanterError::Storage(err.to_string()))?;
        out.write_all(&file.bytes).map_err(|err| BanterError::Storage(err.to_string()))?;
    }
    Ok(())
}
