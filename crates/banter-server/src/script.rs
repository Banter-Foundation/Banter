//! Luau host for client mods and the server half of a plugin.
//!
//! A permission that is not on the manifest makes the call return nil. `io` and `os`
//! are real only when those permissions are checked. Each callback is capped at one
//! million interrupt steps and 50 ms unless the user unlocked that mod.

use std::collections::{BTreeMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlua::{Error, Function, IntoLua, Lua, StdLib, Table, UserData, UserDataMethods, Value, VmState};
use sqlx::PgPool;

use crate::error::BanterError;

const REGIONS: &[&str] = &[
    "rail.bottom",
    "sidebar.footer",
    "stage.header.actions",
    "composer.toolbar",
    "members.above",
    "settings.page",
    "home.section",
    "menu.message",
    "menu.channel",
    "menu.member",
    "stage",
];

const THEME_TOKENS: &[(&str, u32)] = &[
    ("text", 0xF2F3F5),
    ("muted", 0x949BA4),
    ("heading", 0xF2F3F5),
    ("danger", 0xDA373C),
    ("link", 0x00A8FC),
    ("accent", 0x5865F2),
    ("background", 0x313338),
    ("input_bg", 0x1E1F22),
    ("button", 0x5865F2),
    ("button_hover", 0x4752C4),
    ("button_text", 0xFFFFFF),
];

const SHELL_TOKENS: &[&str] = &[
    "rail_bg",
    "sidebar_bg",
    "stage_bg",
    "panel_bg",
    "composer_bg",
    "input_bg",
    "text",
    "muted",
    "accent",
    "danger",
    "row_hover",
    "row_selected",
];

const WIDGET_THEME: &[&str] = &["color", "text_color", "background", "hover"];

static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

const STEP_LIMIT: u64 = 1_000_000;
const TIME_LIMIT: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct Limits {
    pub memory_mb: u32,
    pub unlocked: bool,
    pub allow_shell: bool,
}

#[derive(Clone, Debug)]
pub struct PluginLink {
    pub pool: PgPool,
    pub server_id: i64,
    pub plugin_id: String,
    pub server_name: String,
    pub version: String,
    pub client: String,
}

#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub hard: Option<String>,
    pub soft: Vec<String>,
    pub warnings: Vec<String>,
    pub logs: Vec<String>,
    pub widgets: Vec<Widget>,
    pub theme: BTreeMap<String, u32>,
    pub shell: BTreeMap<String, u32>,
}

#[derive(Clone, Debug)]
pub struct Widget {
    pub id: String,
    pub slot: String,
    pub kind: String,
    pub label: String,
    pub theme: BTreeMap<String, u32>,
}

pub fn export_allowed(report: &RunReport) -> bool {
    report.hard.is_none() && report.soft.is_empty()
}

pub fn memory_warning(mb: u32) -> Option<String> {
    if mb > 150 {
        Some(format!("This mod requires {mb} MB of memory. That is more than 150 MB."))
    } else {
        None
    }
}

/// Run a mod entry and one render. Used by tests and by export's one-shot check.
pub fn run_client_script(id: &str, source: &str, permissions: &[&str], memory_mb: u32, unlocked: bool) -> RunReport {
    let perms: Vec<String> = permissions.iter().map(|item| (*item).to_string()).collect();
    let mut addon = match Addon::open(
        id,
        &perms,
        Limits { memory_mb, unlocked, allow_shell: true },
        None,
    ) {
        Ok(addon) => addon,
        Err(message) => {
            return RunReport { hard: Some(message), ..RunReport::default() };
        }
    };
    if addon.exec(source).is_err() {
        return addon.report();
    }
    addon.render(&[])
}

struct Budget {
    started: Instant,
    steps: u64,
    unlocked: bool,
}

struct Frame {
    id: String,
    permissions: HashSet<String>,
    logs: Vec<String>,
    soft: Vec<String>,
    warnings: Vec<String>,
    warned: HashSet<String>,
    widgets: Vec<Widget>,
    stack: Vec<String>,
    theme: BTreeMap<String, u32>,
    shell: BTreeMap<String, u32>,
    storage: BTreeMap<String, String>,
    clicks: HashSet<String>,
    plugin: Option<PluginLink>,
}

pub struct Addon {
    lua: Lua,
    frame: Arc<Mutex<Frame>>,
    budget: Arc<Mutex<Budget>>,
    stopped: bool,
    report: RunReport,
}

impl Addon {
    pub fn open(id: &str, permissions: &[String], limits: Limits, plugin: Option<PluginLink>) -> Result<Self, String> {
        let libs = StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::BIT;
        let lua = Lua::new_with(libs, mlua::LuaOptions::default()).map_err(|err| err.to_string())?;
        let _ = lua.sandbox(true);
        let bytes = (limits.memory_mb.max(1) as usize).saturating_mul(1024 * 1024);
        lua.set_memory_limit(bytes).map_err(|err| err.to_string())?;
        let budget = Arc::new(Mutex::new(Budget {
            started: Instant::now(),
            steps: 0,
            unlocked: limits.unlocked,
        }));
        let hook = Arc::clone(&budget);
        lua.set_interrupt(move |_| {
            let mut budget = hook.lock().expect("budget");
            budget.steps += 1;
            if budget.unlocked {
                return Ok(VmState::Continue);
            }
            if budget.started.elapsed() >= TIME_LIMIT {
                return Err(Error::RuntimeError("time_limit".into()));
            }
            if budget.steps > STEP_LIMIT {
                return Err(Error::RuntimeError("instruction_limit".into()));
            }
            Ok(VmState::Continue)
        });
        let frame = Arc::new(Mutex::new(Frame {
            id: id.to_string(),
            permissions: permissions.iter().cloned().collect(),
            logs: Vec::new(),
            soft: Vec::new(),
            warnings: Vec::new(),
            warned: HashSet::new(),
            widgets: Vec::new(),
            stack: Vec::new(),
            theme: THEME_TOKENS.iter().map(|(key, value)| ((*key).to_string(), *value)).collect(),
            shell: BTreeMap::new(),
            storage: BTreeMap::new(),
            clicks: HashSet::new(),
            plugin,
        }));
        install_api(&lua, &frame, limits.allow_shell).map_err(|err| err.to_string())?;
        Ok(Self { lua, frame, budget, stopped: false, report: RunReport::default() })
    }

    pub fn set_storage(&self, storage: BTreeMap<String, String>) {
        self.frame.lock().expect("frame").storage = storage;
    }

    pub fn storage(&self) -> BTreeMap<String, String> {
        self.frame.lock().expect("frame").storage.clone()
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn report_hard(&self) -> Option<String> {
        self.report.hard.clone()
    }

    pub fn exec(&mut self, source: &str) -> std::result::Result<(), ()> {
        self.arm();
        match self.lua.load(source).exec() {
            Ok(()) => {
                self.collect(false);
                Ok(())
            }
            Err(err) => {
                self.note_hard(&err);
                Err(())
            }
        }
    }

    /// Run one registered `banter.events.on` handler. Missing handlers do nothing.
    pub fn emit(&mut self, name: &str, channel_id: i64, message_id: i64, body: &str) -> std::result::Result<(), ()> {
        if self.stopped {
            return Ok(());
        }
        let handlers: mlua::Result<Table> = self.lua.named_registry_value("banter_handlers");
        let Ok(handlers) = handlers else {
            return Ok(());
        };
        let function: mlua::Result<Function> = handlers.get(name);
        let Ok(function) = function else {
            return Ok(());
        };
        let event = match self.lua.create_table() {
            Ok(event) => event,
            Err(_) => return Err(()),
        };
        let message = match self.lua.create_table() {
            Ok(message) => message,
            Err(_) => return Err(()),
        };
        if message.set("id", message_id).is_err() || message.set("body", body).is_err() {
            return Err(());
        }
        if event.set("channel_id", channel_id).is_err() || event.set("message", message).is_err() {
            return Err(());
        }
        self.arm();
        if let Err(err) = function.call::<()>(event) {
            self.note_hard(&err);
            return Err(());
        }
        Ok(())
    }

    pub fn render(&mut self, clicks: &[String]) -> RunReport {
        if self.stopped {
            return self.report.clone();
        }
        {
            let mut frame = self.frame.lock().expect("frame");
            frame.widgets.clear();
            frame.stack.clear();
            frame.soft.clear();
            frame.clicks = clicks.iter().cloned().collect();
        }
        self.arm();
        let function: mlua::Result<Function> = self.lua.named_registry_value("banter_render");
        if let Ok(function) = function {
            if let Err(err) = function.call::<()>(()) {
                self.note_hard(&err);
                return self.report();
            }
        }
        self.close_open_regions();
        self.collect(false);
        self.report()
    }

    fn report(&mut self) -> RunReport {
        self.collect(self.stopped);
        self.report.clone()
    }

    fn arm(&self) {
        let mut budget = self.budget.lock().expect("budget");
        budget.started = Instant::now();
        budget.steps = 0;
    }

    fn note_hard(&mut self, err: &Error) {
        self.stopped = true;
        self.report.hard = Some(hard_code(err));
        self.collect(true);
    }

    fn close_open_regions(&self) {
        let mut frame = self.frame.lock().expect("frame");
        while let Some(slot) = frame.stack.pop() {
            let id = frame.id.clone();
            frame.soft.push(format!("{id} left '{slot}' open"));
        }
    }

    fn collect(&mut self, clear_draw: bool) {
        let frame = self.frame.lock().expect("frame");
        self.report.logs = frame.logs.clone();
        self.report.soft = frame.soft.clone();
        self.report.warnings = frame.warnings.clone();
        self.report.theme = frame.theme.clone();
        self.report.shell = frame.shell.clone();
        self.report.widgets = if clear_draw { Vec::new() } else { frame.widgets.clone() };
        if clear_draw {
            self.report.shell.clear();
        }
    }
}

fn hard_code(err: &Error) -> String {
    if matches!(err, Error::MemoryError(_)) {
        return "memory_limit".into();
    }
    let text = err.to_string();
    for code in ["time_limit", "instruction_limit", "memory_limit"] {
        if text.contains(code) {
            return code.into();
        }
    }
    "error".into()
}

fn install_api(lua: &Lua, frame: &Arc<Mutex<Frame>>, allow_shell: bool) -> mlua::Result<()> {
    let banter = lua.create_table()?;
    banter.set("log", log_fn(lua, frame)?)?;
    banter.set("render", render_fn(lua, frame)?)?;
    banter.set("storage", storage_table(lua, frame)?)?;
    banter.set("ui", ui_table(lua, frame)?)?;
    banter.set("theme", theme_table(lua, frame, allow_shell)?)?;
    if frame.lock().expect("frame").plugin.is_some() {
        banter.set("server", server_table(lua, frame)?)?;
        banter.set("plugin", plugin_table(lua, frame)?)?;
        banter.set("messages", messages_table(lua, frame)?)?;
        banter.set("channels", channels_table(lua, frame)?)?;
        banter.set("events", events_table(lua, frame)?)?;
    }
    lua.globals().set("banter", banter)?;
    lua.globals().set("io", io_table(lua, frame)?)?;
    lua.globals().set("os", os_table(lua, frame)?)?;
    Ok(())
}

fn allows(frame: &Arc<Mutex<Frame>>, permission: &str, function: &str) -> bool {
    let mut frame = frame.lock().expect("frame");
    if frame.permissions.contains(permission) {
        return true;
    }
    if frame.warned.insert(function.to_string()) {
        frame.warnings.push(format!("{function} returned nil"));
    }
    false
}

fn log_fn(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Function> {
    let frame = Arc::clone(frame);
    lua.create_function(move |_, (level, message): (String, String)| {
        if !allows(&frame, "log", "banter.log") {
            return Ok(Value::Nil);
        }
        let level = match level.as_str() {
            "debug" | "info" | "warn" | "error" => level,
            _ => return Err(Error::RuntimeError("invalid".into())),
        };
        let message: String = message.chars().take(2000).collect();
        frame.lock().expect("frame").logs.push(format!("{level} {message}"));
        Ok(Value::Boolean(true))
    })
}

fn render_fn(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Function> {
    let frame = Arc::clone(frame);
    lua.create_function(move |lua, function: Function| {
        if !allows(&frame, "ui", "banter.render") {
            return Ok(Value::Nil);
        }
        lua.set_named_registry_value("banter_render", function)?;
        Ok(Value::Boolean(true))
    })
}

fn storage_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let get_frame = Arc::clone(frame);
    table.set(
        "get",
        lua.create_function(move |_, key: String| {
            if !allows(&get_frame, "storage", "banter.storage.get") {
                return Ok(None);
            }
            if storage_key(&key).is_err() {
                return Err(Error::RuntimeError("invalid".into()));
            }
            let frame = get_frame.lock().expect("frame");
            if let Some(plugin) = &frame.plugin {
                let pool = plugin.pool.clone();
                let server_id = plugin.server_id;
                let plugin_id = plugin.plugin_id.clone();
                drop(frame);
                let value = block(storage_get_db(&pool, server_id, &plugin_id, &key))?;
                return Ok(value);
            }
            Ok(frame.storage.get(&key).cloned())
        })?,
    )?;
    let set_frame = Arc::clone(frame);
    table.set(
        "set",
        lua.create_function(move |_, (key, value): (String, String)| {
            if !allows(&set_frame, "storage", "banter.storage.set") {
                return Ok(Value::Nil);
            }
            if storage_key(&key).is_err() || value.len() > 64 * 1024 {
                return Err(Error::RuntimeError("invalid".into()));
            }
            let mut frame = set_frame.lock().expect("frame");
            if let Some(plugin) = frame.plugin.clone() {
                drop(frame);
                block(storage_set_db(&plugin.pool, plugin.server_id, &plugin.plugin_id, &key, &value))?;
                return Ok(Value::Nil);
            }
            let total: usize = frame.storage.values().map(String::len).sum();
            let previous = frame.storage.get(&key).map(String::len).unwrap_or(0);
            if total - previous + value.len() > 1024 * 1024 {
                return Err(Error::RuntimeError("limit".into()));
            }
            frame.storage.insert(key, value);
            Ok(Value::Nil)
        })?,
    )?;
    let del_frame = Arc::clone(frame);
    table.set(
        "delete",
        lua.create_function(move |_, key: String| {
            if !allows(&del_frame, "storage", "banter.storage.delete") {
                return Ok(Value::Nil);
            }
            let mut frame = del_frame.lock().expect("frame");
            if let Some(plugin) = frame.plugin.clone() {
                drop(frame);
                block(storage_delete_db(&plugin.pool, plugin.server_id, &plugin.plugin_id, &key))?;
                return Ok(Value::Nil);
            }
            frame.storage.remove(&key);
            Ok(Value::Nil)
        })?,
    )?;
    Ok(table)
}

fn storage_key(key: &str) -> mlua::Result<()> {
    let ok = (1..=64).contains(&key.chars().count())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    if ok { Ok(()) } else { Err(Error::RuntimeError("invalid".into())) }
}

fn block<T>(future: impl std::future::Future<Output = std::result::Result<T, BanterError>>) -> mlua::Result<T> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| handle.block_on(future)).map_err(|err| Error::RuntimeError(err.to_string()))
}

async fn storage_get_db(pool: &PgPool, server_id: i64, plugin_id: &str, key: &str) -> Result<Option<String>, BanterError> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM plugin_storage WHERE server_id = $1 AND plugin_id = $2 AND key = $3",
    )
    .bind(server_id)
    .bind(plugin_id)
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    Ok(value)
}

async fn storage_set_db(pool: &PgPool, server_id: i64, plugin_id: &str, key: &str, value: &str) -> Result<(), BanterError> {
    if value.len() > 256 * 1024 {
        return Err(BanterError::Invalid);
    }
    sqlx::query(
        "INSERT INTO plugin_storage (server_id, plugin_id, key, value) VALUES ($1, $2, $3, $4)
         ON CONFLICT (server_id, plugin_id, key) DO UPDATE SET value = EXCLUDED.value",
    )
    .bind(server_id)
    .bind(plugin_id)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    Ok(())
}

async fn storage_delete_db(pool: &PgPool, server_id: i64, plugin_id: &str, key: &str) -> Result<(), BanterError> {
    sqlx::query("DELETE FROM plugin_storage WHERE server_id = $1 AND plugin_id = $2 AND key = $3")
        .bind(server_id)
        .bind(plugin_id)
        .bind(key)
        .execute(pool)
        .await
        .map_err(|err| BanterError::Storage(err.to_string()))?;
    Ok(())
}

fn ui_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let begin_frame = Arc::clone(frame);
    table.set(
        "begin",
        lua.create_function(move |_, (name, kind): (String, Option<String>)| {
            if !allows(&begin_frame, "ui", "banter.ui.begin") {
                return Ok(Value::Nil);
            }
            if !REGIONS.contains(&name.as_str()) {
                return Ok(Value::Boolean(false));
            }
            let slot = match kind {
                Some(kind) => format!("{name}:{kind}"),
                None => name,
            };
            begin_frame.lock().expect("frame").stack.push(slot);
            Ok(Value::Boolean(true))
        })?,
    )?;
    let close_frame = Arc::clone(frame);
    table.set(
        "close",
        lua.create_function(move |_, (): ()| {
            if !allows(&close_frame, "ui", "banter.ui.close") {
                return Ok(Value::Nil);
            }
            let mut frame = close_frame.lock().expect("frame");
            if frame.stack.pop().is_none() {
                let id = frame.id.clone();
                frame.soft.push(format!("{id} closed with nothing open"));
            }
            Ok(Value::Nil)
        })?,
    )?;
    let text_frame = Arc::clone(frame);
    table.set(
        "text",
        lua.create_function(move |lua, (value, style, theme): (String, Option<Value>, Option<Value>)| {
            if !allows(&text_frame, "ui", "banter.ui.text") {
                return Ok(Value::Nil);
            }
            let theme = widget_theme(style.as_ref(), theme.as_ref())?;
            let _ = style;
            push_widget(&text_frame, "text", &value, theme);
            let _ = lua;
            Ok(Value::Nil)
        })?,
    )?;
    let button_frame = Arc::clone(frame);
    table.set(
        "button",
        lua.create_function(move |_, (label, style, theme): (String, Option<Value>, Option<Value>)| {
            if !allows(&button_frame, "ui", "banter.ui.button") {
                return Ok(Value::Nil);
            }
            let theme = widget_theme(style.as_ref(), theme.as_ref())?;
            let id = push_widget(&button_frame, "button", &label, theme);
            let clicked = button_frame.lock().expect("frame").clicks.contains(&id);
            Ok(Value::Boolean(clicked))
        })?,
    )?;
    Ok(table)
}

fn push_widget(frame: &Arc<Mutex<Frame>>, kind: &str, label: &str, theme: BTreeMap<String, u32>) -> String {
    let mut frame = frame.lock().expect("frame");
    let slot = frame.stack.last().cloned().unwrap_or_default();
    if slot.is_empty() {
        let id = frame.id.clone();
        frame.soft.push(format!("{id} drew outside a region"));
    }
    let id = format!("{slot}/{label}");
    frame.widgets.push(Widget { id: id.clone(), slot, kind: kind.to_string(), label: label.to_string(), theme });
    id
}

fn widget_theme(style: Option<&Value>, theme: Option<&Value>) -> mlua::Result<BTreeMap<String, u32>> {
    let table = match (style, theme) {
        (_, Some(Value::Table(table))) => Some(table),
        (Some(Value::Table(table)), _) => Some(table),
        _ => None,
    };
    let Some(table) = table else {
        return Ok(BTreeMap::new());
    };
    read_colors(table, WIDGET_THEME)
}

fn theme_table(lua: &Lua, frame: &Arc<Mutex<Frame>>, allow_shell: bool) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let get_frame = Arc::clone(frame);
    table.set(
        "get",
        lua.create_function(move |lua, (): ()| {
            if !allows(&get_frame, "ui", "banter.theme.get") {
                return Ok(Value::Nil);
            }
            let theme = get_frame.lock().expect("frame").theme.clone();
            colors_to_table(lua, &theme)
        })?,
    )?;
    let set_frame = Arc::clone(frame);
    table.set(
        "set",
        lua.create_function(move |_, patch: Table| {
            if !allows(&set_frame, "ui", "banter.theme.set") {
                return Ok(Value::Nil);
            }
            let names: Vec<&str> = THEME_TOKENS.iter().map(|(name, _)| *name).collect();
            let patch = read_colors(&patch, &names)?;
            set_frame.lock().expect("frame").theme.extend(patch);
            Ok(Value::Nil)
        })?,
    )?;
    let reset_frame = Arc::clone(frame);
    table.set(
        "reset",
        lua.create_function(move |_, (): ()| {
            if !allows(&reset_frame, "ui", "banter.theme.reset") {
                return Ok(Value::Nil);
            }
            reset_frame.lock().expect("frame").theme =
                THEME_TOKENS.iter().map(|(key, value)| ((*key).to_string(), *value)).collect();
            Ok(Value::Nil)
        })?,
    )?;
    if allow_shell {
        let shell_frame = Arc::clone(frame);
        table.set(
            "shell",
            lua.create_function(move |_, patch: Table| {
                if !allows(&shell_frame, "ui", "banter.theme.shell") {
                    return Ok(Value::Nil);
                }
                let patch = read_colors(&patch, SHELL_TOKENS)?;
                shell_frame.lock().expect("frame").shell.extend(patch);
                Ok(Value::Nil)
            })?,
        )?;
        let clear_frame = Arc::clone(frame);
        table.set(
            "clear_shell",
            lua.create_function(move |_, (): ()| {
                if !allows(&clear_frame, "ui", "banter.theme.clear_shell") {
                    return Ok(Value::Nil);
                }
                clear_frame.lock().expect("frame").shell.clear();
                Ok(Value::Nil)
            })?,
        )?;
    } else {
        table.set("shell", Value::Nil)?;
    }
    Ok(table)
}

fn read_colors(table: &Table, allowed: &[&str]) -> mlua::Result<BTreeMap<String, u32>> {
    let mut out = BTreeMap::new();
    for pair in table.pairs::<String, Value>() {
        let (key, value) = pair?;
        if !allowed.contains(&key.as_str()) {
            return Err(Error::RuntimeError("invalid".into()));
        }
        let Some(color) = color_of(&value) else {
            return Err(Error::RuntimeError("invalid".into()));
        };
        out.insert(key, color);
    }
    Ok(out)
}

fn color_of(value: &Value) -> Option<u32> {
    match value {
        Value::Integer(number) if *number >= 0 => Some(*number as u32),
        Value::Number(number) if *number >= 0.0 => Some(*number as u32),
        _ => None,
    }
}

fn colors_to_table(lua: &Lua, colors: &BTreeMap<String, u32>) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    for (key, value) in colors {
        table.set(key.as_str(), *value)?;
    }
    Ok(table.into_lua(lua)?)
}

fn server_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let frame = frame.lock().expect("frame");
    let plugin = frame.plugin.as_ref().expect("plugin");
    let table = lua.create_table()?;
    table.set("id", plugin.server_id)?;
    table.set("name", plugin.server_name.as_str())?;
    Ok(table)
}

fn plugin_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let frame = frame.lock().expect("frame");
    let plugin = frame.plugin.as_ref().expect("plugin");
    let table = lua.create_table()?;
    table.set("id", plugin.plugin_id.as_str())?;
    table.set("name", plugin.plugin_id.as_str())?;
    table.set("version", plugin.version.as_str())?;
    table.set("client", plugin.client.as_str())?;
    Ok(table)
}

fn messages_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let send_frame = Arc::clone(frame);
    table.set(
        "send",
        lua.create_function(move |_, args: Table| {
            if !allows(&send_frame, "messages", "banter.messages.send") {
                return Ok(Value::Nil);
            }
            let channel_id: i64 = args.get("channel_id")?;
            let body: String = args.get("body")?;
            let plugin = send_frame.lock().expect("frame").plugin.clone().expect("plugin");
            block(plugin_send(&plugin, channel_id, &body))?;
            Ok(Value::Boolean(true))
        })?,
    )?;
    let edit_frame = Arc::clone(frame);
    table.set(
        "edit",
        lua.create_function(move |_, (id, body): (i64, String)| {
            if !allows(&edit_frame, "messages", "banter.messages.edit") {
                return Ok(Value::Nil);
            }
            let plugin = edit_frame.lock().expect("frame").plugin.clone().expect("plugin");
            block(plugin_edit(&plugin, id, &body))?;
            Ok(Value::Boolean(true))
        })?,
    )?;
    Ok(table)
}

fn channels_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let create_frame = Arc::clone(frame);
    table.set(
        "create",
        lua.create_function(move |_, args: Table| {
            if !allows(&create_frame, "channels", "banter.channels.create") {
                return Ok(Value::Nil);
            }
            let category_id: i64 = args.get("category_id")?;
            let name: String = args.get("name")?;
            let kind: String = args.get("kind")?;
            let plugin = create_frame.lock().expect("frame").plugin.clone().expect("plugin");
            let id = block(plugin_create_channel(&plugin, category_id, &name, &kind))?;
            Ok(Value::Integer(id))
        })?,
    )?;
    Ok(table)
}

fn events_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let on_frame = Arc::clone(frame);
    table.set(
        "on",
        lua.create_function(move |lua, (name, function): (String, Function)| {
            let permission = if name == "command" { "commands" } else { "events" };
            if !allows(&on_frame, permission, "banter.events.on") {
                return Ok(Value::Nil);
            }
            let handlers: Table = match lua.named_registry_value("banter_handlers") {
                Ok(table) => table,
                Err(_) => lua.create_table()?,
            };
            handlers.set(name, function)?;
            lua.set_named_registry_value("banter_handlers", handlers)?;
            Ok(Value::Boolean(true))
        })?,
    )?;
    Ok(table)
}

async fn plugin_send(plugin: &PluginLink, channel_id: i64, body: &str) -> Result<(), BanterError> {
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT kind FROM channels WHERE id = $1 AND server_id = $2",
    )
    .bind(channel_id)
    .bind(plugin.server_id)
    .fetch_optional(&plugin.pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    let Some(kind) = kind else {
        return Err(BanterError::NotFound);
    };
    let owned = kind == "text" || kind.starts_with(&format!("{}.", plugin.plugin_id));
    if !owned {
        return Err(BanterError::NotTextChannel);
    }
    let body = body.trim();
    if body.is_empty() {
        return Err(BanterError::EmptyMessage);
    }
    sqlx::query(
        "INSERT INTO messages (channel_id, dm_id, author_id, author_kind, plugin_id, body, reply_to, created_at, edited_at)
         VALUES ($1, NULL, NULL, 'plugin', $2, $3, NULL, $4, NULL)",
    )
    .bind(channel_id)
    .bind(&plugin.plugin_id)
    .bind(body)
    .bind(now_ms())
    .execute(&plugin.pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    Ok(())
}

async fn plugin_edit(plugin: &PluginLink, message_id: i64, body: &str) -> Result<(), BanterError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(BanterError::EmptyMessage);
    }
    if body.chars().count() > 2000 {
        return Err(BanterError::MessageTooLong);
    }
    let updated = sqlx::query(
        "UPDATE messages SET body = $1, edited_at = $2
         WHERE id = $3 AND channel_id IN (SELECT id FROM channels WHERE server_id = $4)",
    )
    .bind(body)
    .bind(now_ms())
    .bind(message_id)
    .bind(plugin.server_id)
    .execute(&plugin.pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    if updated.rows_affected() == 0 {
        return Err(BanterError::NotFound);
    }
    Ok(())
}

async fn plugin_create_channel(plugin: &PluginLink, category_id: i64, name: &str, kind: &str) -> Result<i64, BanterError> {
    let allowed = matches!(kind, "text" | "voice" | "board") || kind.starts_with(&format!("{}.", plugin.plugin_id));
    if !allowed || !crate::plugins::plugin_kind_ok(kind) && !matches!(kind, "text" | "voice" | "board") {
        return Err(BanterError::InvalidName);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO channels (server_id, category_id, name, kind, position, icon, created_by_plugin)
         VALUES ($1, $2, $3, $4, 0, NULL, $5) RETURNING id",
    )
    .bind(plugin.server_id)
    .bind(category_id)
    .bind(name)
    .bind(kind)
    .bind(&plugin.plugin_id)
    .fetch_one(&plugin.pool)
    .await
    .map_err(|err| BanterError::Storage(err.to_string()))?;
    Ok(id)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct HostFile {
    file: Option<std::fs::File>,
    read: bool,
}

impl UserData for HostFile {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("read", |_, this, (): ()| {
            let Some(file) = this.file.as_mut() else {
                return Ok(None);
            };
            if !this.read {
                return Ok(None);
            }
            let mut text = String::new();
            file.read_to_string(&mut text).map_err(|err| Error::external(err.to_string()))?;
            Ok(Some(text))
        });
        methods.add_method_mut("write", |_, this, text: String| {
            let Some(file) = this.file.as_mut() else {
                return Ok(());
            };
            file.write_all(text.as_bytes()).map_err(|err| Error::external(err.to_string()))?;
            Ok(())
        });
        methods.add_method_mut("close", |_, this, (): ()| {
            this.file.take();
            Ok(())
        });
    }
}

fn io_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let open_frame = Arc::clone(frame);
    table.set(
        "open",
        lua.create_function(move |lua, (path, mode): (String, Option<String>)| {
            if !allows(&open_frame, "io", "io.open") {
                return Ok(Value::Nil);
            }
            let mode = mode.unwrap_or_else(|| "r".into());
            let mut options = OpenOptions::new();
            if mode.contains('a') {
                options.create(true).append(true);
            } else if mode.contains('w') {
                options.create(true).write(true).truncate(true);
            } else {
                options.read(true);
            }
            match options.open(path) {
                Ok(file) => {
                    let host = HostFile { read: !mode.contains('w') || mode.contains('r'), file: Some(file) };
                    Ok(host.into_lua(lua)?)
                }
                Err(_) => Ok(Value::Nil),
            }
        })?,
    )?;
    Ok(table)
}

fn os_table(lua: &Lua, frame: &Arc<Mutex<Frame>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let env_frame = Arc::clone(frame);
    table.set(
        "getenv",
        lua.create_function(move |_, name: String| {
            if !allows(&env_frame, "os", "os.getenv") {
                return Ok(None);
            }
            Ok(std::env::var(name).ok())
        })?,
    )?;
    let exec_frame = Arc::clone(frame);
    table.set(
        "execute",
        lua.create_function(move |_, command: String| {
            if !allows(&exec_frame, "os", "os.execute") {
                return Ok(Value::Nil);
            }
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .status()
                .map_err(|err| Error::external(err.to_string()))?;
            Ok(Value::Integer(status.code().unwrap_or(1) as i64))
        })?,
    )?;
    let clock_frame = Arc::clone(frame);
    table.set(
        "clock",
        lua.create_function(move |_, (): ()| {
            if !allows(&clock_frame, "os", "os.clock") {
                return Ok(Value::Nil);
            }
            let elapsed = START.elapsed().as_secs_f64();
            Ok(Value::Number(elapsed))
        })?,
    )?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_permission_returns_nil() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function() banter.log("info", "nope") end)"#,
            &["ui"],
            8,
            true,
        );
        assert!(report.logs.is_empty());
        assert!(report.hard.is_none());
        assert!(report.warnings.iter().any(|line| line.contains("banter.log")));
    }

    #[test]
    fn log_permission_records_the_line() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function() banter.log("info", "hello") end)"#,
            &["ui", "log"],
            8,
            true,
        );
        assert_eq!(report.logs, ["info hello"]);
        assert!(report.hard.is_none());
    }

    #[test]
    fn unclosed_begin_is_a_soft_error_and_blocks_export() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function()
                banter.ui.begin("stage.header.actions")
                banter.ui.text("Hello")
            end)"#,
            &["ui"],
            8,
            true,
        );
        assert!(report.hard.is_none());
        assert!(report.soft.iter().any(|line| line.contains("stage.header.actions")));
        assert!(!export_allowed(&report));
        assert!(report.widgets.iter().any(|widget| widget.label == "Hello"));
    }

    #[test]
    fn a_balanced_render_can_be_exported() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function()
                if banter.ui.begin("stage.header.actions") then
                    banter.ui.text("Hello")
                    banter.ui.close()
                end
            end)"#,
            &["ui"],
            8,
            true,
        );
        assert!(export_allowed(&report), "{report:?}");
    }

    #[test]
    fn locked_infinite_loop_is_a_hard_error() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function() while true do end end)"#,
            &["ui"],
            8,
            false,
        );
        assert!(matches!(report.hard.as_deref(), Some("time_limit") | Some("instruction_limit")));
        assert!(!export_allowed(&report));
    }

    #[test]
    fn unlocked_finite_loop_finishes() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function()
                local n = 0
                for i = 1, 1000 do n = n + 1 end
                banter.log("info", tostring(n))
            end)"#,
            &["ui", "log"],
            8,
            true,
        );
        assert_eq!(report.hard, None, "{report:?}");
        assert_eq!(report.logs, ["info 1000"]);
    }

    #[test]
    fn memory_over_the_cap_is_memory_limit() {
        let report = run_client_script(
            "clock_bar",
            r#"banter.render(function() local s = string.rep("x", 3 * 1024 * 1024) banter.log("info", tostring(#s)) end)"#,
            &["ui", "log"],
            1,
            true,
        );
        assert_eq!(report.hard.as_deref(), Some("memory_limit"));
    }

    #[test]
    fn theme_set_and_shell_and_widget_theme() {
        let report = run_client_script(
            "clock_bar",
            r#"
            banter.theme.set({ accent = 0x23A55A })
            banter.theme.shell({ rail_bg = 0x111111 })
            banter.render(function()
                if banter.ui.begin("stage.header.actions") then
                    banter.ui.text("Hi", "body", { background = 0xFF0000 })
                    banter.ui.close()
                end
            end)
            "#,
            &["ui"],
            8,
            true,
        );
        assert_eq!(report.hard, None, "{report:?}");
        assert_eq!(report.theme.get("accent"), Some(&0x23A55A));
        assert_eq!(report.shell.get("rail_bg"), Some(&0x111111));
        assert_eq!(report.widgets[0].theme.get("background"), Some(&0xFF0000));
    }

    #[test]
    fn memory_warning_starts_above_150() {
        assert!(memory_warning(150).is_none());
        assert_eq!(
            memory_warning(151).as_deref(),
            Some("This mod requires 151 MB of memory. That is more than 150 MB.")
        );
    }
}
