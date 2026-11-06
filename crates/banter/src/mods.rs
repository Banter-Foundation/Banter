//! Client mods. Each enabled mod is one Luau state, run in the user's order.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use banter_server::{Addon, Limits, Widget};
use serde::{Deserialize, Serialize};

pub fn memory_warning(mb: u32) -> Option<String> {
    banter_server::memory_warning(mb)
}

/// Byte-wise ascending order. This is the order before the user moves anything.
pub fn default_mod_order(ids: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| (*id).to_string()).collect();
    ids.sort();
    ids
}

/// Enabling places the mod at the end, including a mod that was enabled before.
pub fn enable_mod(order: &mut Vec<String>, id: &str) {
    order.retain(|item| item != id);
    order.push(id.to_string());
}

pub fn disable_mod(order: &mut Vec<String>, id: &str) {
    order.retain(|item| item != id);
}

pub fn move_mod(order: &mut Vec<String>, id: &str, up: bool) {
    let Some(index) = order.iter().position(|item| item == id) else {
        return;
    };
    if up && index > 0 {
        order.swap(index, index - 1);
    } else if !up && index + 1 < order.len() {
        order.swap(index, index + 1);
    }
}

#[derive(Clone, Debug)]
pub struct ModRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub unlocked: bool,
    pub memory_mb: u32,
    pub warning: Option<String>,
    pub hard: Option<String>,
    pub soft: Vec<String>,
    pub failed: Option<String>,
}

struct Installed {
    id: String,
    name: String,
    source: String,
    permissions: Vec<String>,
    memory_mb: u32,
    failed: Option<String>,
}

struct Running {
    addon: Addon,
    hard: Option<String>,
    soft: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct OrderFile {
    order: Vec<String>,
    enabled: Vec<String>,
    unlocked: Vec<String>,
}

pub struct ModHost {
    root: PathBuf,
    installed: Vec<Installed>,
    order: Vec<String>,
    enabled: Vec<String>,
    unlocked: Vec<String>,
    running: BTreeMap<String, Running>,
    clicks: Vec<String>,
    widgets: Vec<Widget>,
    shell: BTreeMap<String, u32>,
}

impl ModHost {
    pub fn open(data_dir: &Path) -> Self {
        let root = data_dir.to_path_buf();
        let saved = load_order(&root);
        let mut host = Self {
            root,
            installed: Vec::new(),
            order: saved.order,
            enabled: saved.enabled,
            unlocked: saved.unlocked,
            running: BTreeMap::new(),
            clicks: Vec::new(),
            widgets: Vec::new(),
            shell: BTreeMap::new(),
        };
        host.ensure_samples();
        host.reload();
        host
    }

    /// First launch has no mods folder. Drop the repo samples in so Settings → Mods is not empty.
    fn ensure_samples(&self) {
        let mods = self.root.join("mods");
        if mods.is_dir() {
            return;
        }
        let samples = [
            (
                "ping",
                include_str!("../../../examples/mods/ping/banter.mod.json"),
                include_str!("../../../examples/mods/ping/main.luau"),
            ),
            (
                "status_chip",
                include_str!("../../../examples/mods/status_chip/banter.mod.json"),
                include_str!("../../../examples/mods/status_chip/main.luau"),
            ),
            (
                "warm_shell",
                include_str!("../../../examples/mods/warm_shell/banter.mod.json"),
                include_str!("../../../examples/mods/warm_shell/main.luau"),
            ),
            (
                "purple_glow",
                include_str!("../../../examples/mods/purple_glow/banter.mod.json"),
                include_str!("../../../examples/mods/purple_glow/main.luau"),
            ),
        ];
        for (id, manifest, source) in samples {
            let folder = mods.join(id);
            if fs::create_dir_all(&folder).is_err() {
                continue;
            }
            let _ = fs::write(folder.join("banter.mod.json"), manifest);
            let _ = fs::write(folder.join("main.luau"), source);
        }
    }

    pub fn reload(&mut self) {
        self.installed = scan_mods(&self.root.join("mods"));
        let known: Vec<String> = self.installed.iter().map(|item| item.id.clone()).collect();
        self.enabled
            .retain(|id| known.iter().any(|item| item == id));
        self.order
            .retain(|id| self.enabled.iter().any(|item| item == id));
        for id in &self.enabled {
            if !self.order.iter().any(|item| item == id) {
                self.order.push(id.clone());
            }
        }
        self.sync_running();
    }

    pub fn rows(&self) -> Vec<ModRow> {
        let mut rows = Vec::new();
        for id in &self.order {
            if let Some(row) = self.row(id) {
                rows.push(row);
            }
        }
        let mut rest: Vec<&Installed> = self
            .installed
            .iter()
            .filter(|item| !self.enabled.iter().any(|id| id == &item.id))
            .collect();
        rest.sort_by(|a, b| a.id.cmp(&b.id));
        for item in rest {
            rows.push(self.row_for(item));
        }
        rows
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            if self.enabled.len() >= 32
                || !self
                    .installed
                    .iter()
                    .any(|item| item.id == id && item.failed.is_none())
            {
                return;
            }
            enable_mod(&mut self.order, id);
            if !self.enabled.iter().any(|item| item == id) {
                self.enabled.push(id.to_string());
            }
        } else {
            disable_mod(&mut self.order, id);
            self.enabled.retain(|item| item != id);
            self.running.remove(id);
        }
        self.save();
        self.sync_running();
    }

    pub fn set_unlocked(&mut self, id: &str, unlocked: bool) {
        self.unlocked.retain(|item| item != id);
        if unlocked {
            self.unlocked.push(id.to_string());
        }
        self.running.remove(id);
        self.save();
        self.sync_running();
    }

    pub fn move_by(&mut self, id: &str, up: bool) {
        move_mod(&mut self.order, id, up);
        self.save();
    }

    pub fn click(&mut self, id: &str) {
        self.clicks.push(id.to_string());
    }

    pub fn tick(&mut self) {
        let clicks = std::mem::take(&mut self.clicks);
        self.widgets.clear();
        self.shell.clear();
        for id in self.order.clone() {
            let Some(running) = self.running.get_mut(&id) else {
                continue;
            };
            if running.addon.stopped() {
                continue;
            }
            let report = running.addon.render(&clicks);
            running.hard = report.hard.clone();
            running.soft = report.soft.clone();
            if report.hard.is_none() {
                self.widgets.extend(report.widgets);
                for (token, color) in report.shell {
                    self.shell.insert(token, color);
                }
            }
        }
    }

    pub fn header_widgets(&self) -> Vec<Widget> {
        self.widgets
            .iter()
            .filter(|widget| widget.slot == "stage.header.actions")
            .cloned()
            .collect()
    }

    pub fn shell_color(&self, token: &str) -> Option<u32> {
        self.shell.get(token).copied()
    }

    fn row(&self, id: &str) -> Option<ModRow> {
        self.installed
            .iter()
            .find(|item| item.id == id)
            .map(|item| self.row_for(item))
    }

    fn row_for(&self, item: &Installed) -> ModRow {
        let running = self.running.get(&item.id);
        ModRow {
            id: item.id.clone(),
            name: item.name.clone(),
            enabled: self.enabled.iter().any(|id| id == &item.id),
            unlocked: self.unlocked.iter().any(|id| id == &item.id),
            memory_mb: item.memory_mb,
            warning: memory_warning(item.memory_mb),
            hard: running.and_then(|running| running.hard.clone()),
            soft: running
                .map(|running| running.soft.clone())
                .unwrap_or_default(),
            failed: item.failed.clone(),
        }
    }

    fn sync_running(&mut self) {
        let order = self.order.clone();
        self.running
            .retain(|id, _| order.iter().any(|item| item == id));
        for id in order {
            if self.running.contains_key(&id) {
                continue;
            }
            let Some(installed) = self
                .installed
                .iter()
                .find(|item| item.id == id && item.failed.is_none())
            else {
                continue;
            };
            let unlocked = self.unlocked.iter().any(|item| item == &id);
            let mut addon = match Addon::open(
                &installed.id,
                &installed.permissions,
                Limits {
                    memory_mb: installed.memory_mb,
                    unlocked,
                    allow_shell: true,
                },
                None,
            ) {
                Ok(addon) => addon,
                Err(message) => {
                    self.running.insert(
                        id,
                        Running {
                            addon: broken_addon(),
                            hard: Some(message),
                            soft: Vec::new(),
                        },
                    );
                    continue;
                }
            };
            let hard = if addon.exec(&installed.source).is_err() {
                addon.report_hard()
            } else {
                None
            };
            self.running.insert(
                id.clone(),
                Running {
                    addon,
                    hard,
                    soft: Vec::new(),
                },
            );
        }
    }

    fn save(&self) {
        let file = OrderFile {
            order: self.order.clone(),
            enabled: self.enabled.clone(),
            unlocked: self.unlocked.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&file) {
            let _ = fs::write(self.root.join("mods-order.json"), text);
        }
    }
}

fn broken_addon() -> Addon {
    Addon::open(
        "broken",
        &[],
        Limits {
            memory_mb: 8,
            unlocked: true,
            allow_shell: true,
        },
        None,
    )
    .expect("empty addon")
}

fn load_order(root: &Path) -> OrderFile {
    let Ok(text) = fs::read_to_string(root.join("mods-order.json")) else {
        return OrderFile::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn scan_mods(root: &Path) -> Vec<Installed> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut mods = Vec::new();
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        mods.push(read_mod(&folder));
    }
    mods
}

fn read_mod(folder: &Path) -> Installed {
    let manifest_path = folder.join("banter.mod.json");
    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(err) => return failed_mod(folder, err.to_string()),
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return failed_mod(folder, "manifest is not json".into()),
    };
    let id = value
        .get("id")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_string();
    let name = value
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_string();
    let banter = value
        .get("banter")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let entry = value
        .get("entry")
        .and_then(|item| item.as_str())
        .unwrap_or("main.luau");
    let memory_mb = value
        .get("memory_mb")
        .and_then(|item| item.as_u64())
        .unwrap_or(8) as u32;
    let permissions = value
        .get("permissions")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(Vec::new);
    let known = [
        "log",
        "storage",
        "ui",
        "read_screen",
        "messages",
        "voice",
        "presence",
        "io",
        "os",
    ];
    if !banter_server::ident_ok(&id)
        || !(1..=32).contains(&name.chars().count())
        || banter != "1"
        || memory_mb < 1
    {
        return failed_mod(folder, "manifest is invalid".into());
    }
    if permissions
        .iter()
        .any(|item| !known.contains(&item.as_str()))
    {
        return failed_mod(folder, "unknown permission".into());
    }
    let source = match fs::read_to_string(folder.join(entry)) {
        Ok(source) => source,
        Err(err) => return failed_mod(folder, err.to_string()),
    };
    Installed {
        id,
        name,
        source,
        permissions,
        memory_mb,
        failed: None,
    }
}

fn failed_mod(folder: &Path, reason: String) -> Installed {
    Installed {
        id: folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mod")
            .to_string(),
        name: folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Mod")
            .to_string(),
        source: String::new(),
        permissions: Vec::new(),
        memory_mb: 8,
        failed: Some(reason),
    }
}
