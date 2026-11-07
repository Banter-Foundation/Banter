//! Discord-like chat commands: registry, prefix, aliases, and parse (BA-104).
//!
//! Built-ins live in the client. Guild prefix and aliases are fetched from the
//! server. [`CommandRegistry`] is the extension point for mods/plugins later.

use std::collections::HashMap;

/// Default composer command prefix (overridden per guild when configured).
pub const DEFAULT_COMMAND_PREFIX: &str = "/";

/// Face appended by `/shrug`.
pub const SHRUG_FACE: &str = r"¯\_(ツ)_/¯";

/// Face sent by `/tableflip`.
pub const TABLEFLIP_FACE: &str = "(╯°□°）╯︵ ┻━┻";

const MAX_PREFIX_CHARS: usize = 8;
const MAX_NAME_CHARS: usize = 32;

/// What running a resolved command should do in the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAction {
    Me,
    Shrug,
    Tableflip,
    Mute,
    Deafen,
    Join,
    DebugNow,
    /// Listed stub — not wired yet. `note` explains why.
    Stub { note: &'static str },
}

/// One registered command (primary name plus optional built-in aliases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub args_hint: String,
    pub hidden: bool,
    pub action: CommandAction,
}

/// Parsed `{prefix}{name}[ args…]` from the composer plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,
    pub args: String,
}

/// Extensible command table. Built-ins ship in [`default_command_registry`];
/// plugins/mods can [`CommandRegistry::register`] later when the client plugin
/// API runs composer commands.
pub trait CommandRegistry {
    fn prefix(&self) -> &str;
    fn set_prefix(&mut self, prefix: String) -> Result<(), String>;
    fn register(&mut self, spec: CommandSpec) -> Result<(), String>;
    fn set_server_aliases(&mut self, aliases: Vec<(String, String)>) -> Result<(), String>;
    fn server_aliases(&self) -> &[(String, String)];
    fn resolve(&self, name: &str) -> Option<&CommandSpec>;
    fn suggestions(&self, partial: &str) -> Vec<&CommandSpec>;
}

/// In-memory registry used by the desktop composer.
#[derive(Clone, Debug)]
pub struct DefaultCommandRegistry {
    prefix: String,
    commands: Vec<CommandSpec>,
    /// Lowercased lookup → index into `commands`.
    index: HashMap<String, usize>,
    server_aliases: Vec<(String, String)>,
}

impl DefaultCommandRegistry {
    fn rebuild_index(&mut self) {
        self.index.clear();
        for (idx, spec) in self.commands.iter().enumerate() {
            self.index.insert(spec.name.to_ascii_lowercase(), idx);
            for alias in &spec.aliases {
                self.index.insert(alias.to_ascii_lowercase(), idx);
            }
        }
        for (alias, target) in &self.server_aliases {
            if let Some(idx) = self.index.get(&target.to_ascii_lowercase()).copied() {
                self.index.insert(alias.to_ascii_lowercase(), idx);
            }
        }
    }
}

impl CommandRegistry for DefaultCommandRegistry {
    fn prefix(&self) -> &str {
        &self.prefix
    }

    fn set_prefix(&mut self, prefix: String) -> Result<(), String> {
        validate_command_prefix(&prefix)?;
        self.prefix = prefix;
        Ok(())
    }

    fn register(&mut self, spec: CommandSpec) -> Result<(), String> {
        validate_command_name(&spec.name)?;
        for alias in &spec.aliases {
            validate_command_name(alias)?;
        }
        let key = spec.name.to_ascii_lowercase();
        if let Some(idx) = self.index.get(&key).copied() {
            if self.commands[idx].name.eq_ignore_ascii_case(&spec.name) {
                self.commands[idx] = spec;
                self.rebuild_index();
                return Ok(());
            }
        }
        self.commands.push(spec);
        self.rebuild_index();
        Ok(())
    }

    fn set_server_aliases(&mut self, aliases: Vec<(String, String)>) -> Result<(), String> {
        let mut normalized = Vec::with_capacity(aliases.len());
        let mut seen = std::collections::HashSet::new();
        for (alias, target) in aliases {
            let alias = alias.trim().to_ascii_lowercase();
            let target = target.trim().to_ascii_lowercase();
            validate_command_name(&alias)?;
            validate_command_name(&target)?;
            if alias == target {
                return Err("alias cannot equal its target".into());
            }
            if !seen.insert(alias.clone()) {
                return Err("duplicate alias".into());
            }
            normalized.push((alias, target));
        }
        // Preserve caller order (tests and modal drafts); the server API sorts for storage.
        self.server_aliases = normalized;
        self.rebuild_index();
        Ok(())
    }

    fn server_aliases(&self) -> &[(String, String)] {
        &self.server_aliases
    }

    fn resolve(&self, name: &str) -> Option<&CommandSpec> {
        let key = name.trim().to_ascii_lowercase();
        self.index.get(&key).map(|&idx| &self.commands[idx])
    }

    fn suggestions(&self, partial: &str) -> Vec<&CommandSpec> {
        let partial = partial.trim().to_ascii_lowercase();
        let mut out: Vec<&CommandSpec> = self
            .commands
            .iter()
            .filter(|spec| {
                if spec.hidden {
                    return false;
                }
                if partial.is_empty() {
                    return true;
                }
                if spec.name.starts_with(&partial) {
                    return true;
                }
                spec.aliases
                    .iter()
                    .any(|alias| alias.to_ascii_lowercase().starts_with(&partial))
                    || self.server_aliases.iter().any(|(alias, target)| {
                        target == &spec.name && alias.starts_with(&partial)
                    })
            })
            .collect();
        out.sort_by(|left, right| left.name.cmp(&right.name));
        out
    }
}

/// Built-in Banter commands shipped with the client.
pub fn default_command_registry() -> DefaultCommandRegistry {
    let mut registry = DefaultCommandRegistry {
        prefix: DEFAULT_COMMAND_PREFIX.to_string(),
        commands: Vec::new(),
        index: HashMap::new(),
        server_aliases: Vec::new(),
    };
    let builtins = [
        CommandSpec {
            name: "me".into(),
            aliases: vec![],
            description: "Send an action in italics".into(),
            args_hint: "action".into(),
            hidden: false,
            action: CommandAction::Me,
        },
        CommandSpec {
            name: "shrug".into(),
            aliases: vec!["shruggie".into()],
            description: "Append a shrug face".into(),
            args_hint: "message".into(),
            hidden: false,
            action: CommandAction::Shrug,
        },
        CommandSpec {
            name: "tableflip".into(),
            aliases: vec![],
            description: "Flip a table".into(),
            args_hint: "message".into(),
            hidden: false,
            action: CommandAction::Tableflip,
        },
        CommandSpec {
            name: "mute".into(),
            aliases: vec![],
            description: "Toggle mute".into(),
            args_hint: String::new(),
            hidden: false,
            action: CommandAction::Mute,
        },
        CommandSpec {
            name: "deafen".into(),
            aliases: vec![],
            description: "Toggle deafen".into(),
            args_hint: String::new(),
            hidden: false,
            action: CommandAction::Deafen,
        },
        CommandSpec {
            name: "join".into(),
            aliases: vec![],
            description: "Switch to a channel by name".into(),
            args_hint: "channel".into(),
            hidden: false,
            action: CommandAction::Join,
        },
        CommandSpec {
            name: "nick".into(),
            aliases: vec![],
            description: "Change nickname".into(),
            args_hint: "name".into(),
            hidden: false,
            action: CommandAction::Stub {
                note: "stub — use Server menu → Change Nickname",
            },
        },
        CommandSpec {
            name: "debugnow".into(),
            aliases: vec![],
            description: "Open server debug".into(),
            args_hint: String::new(),
            hidden: true,
            action: CommandAction::DebugNow,
        },
    ];
    for spec in builtins {
        let _ = registry.register(spec);
    }
    registry
}

pub fn validate_command_prefix(prefix: &str) -> Result<(), String> {
    if prefix.is_empty() || prefix.len() > MAX_PREFIX_CHARS {
        return Err("prefix must be 1-8 characters".into());
    }
    if prefix.chars().any(|ch| ch.is_whitespace()) {
        return Err("prefix cannot contain whitespace".into());
    }
    Ok(())
}

pub fn validate_command_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_CHARS {
        return Err("name must be 1-32 characters".into());
    }
    if !name
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_' | '-'))
    {
        return Err("name must be lowercase letters, digits, underscore, or hyphen".into());
    }
    Ok(())
}

/// Parse a composer line when it starts with `prefix`.
pub fn parse_command_line(plain: &str, prefix: &str) -> Option<ParsedCommand> {
    if prefix.is_empty() {
        return None;
    }
    let plain = plain.trim_start();
    if !plain.starts_with(prefix) {
        return None;
    }
    let rest = &plain[prefix.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let args = parts.next().unwrap_or("").trim().to_string();
    Some(ParsedCommand {
        name: name.to_ascii_lowercase(),
        args,
    })
}

/// BFMD italic action body for `/me`.
pub fn format_me_body(display_name: &str, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        format!("<{{i:t}}>{display_name}")
    } else {
        format!("<{{i:t}}>{display_name} {args}")
    }
}

/// Text body for `/shrug` (optional message + face).
pub fn format_shrug_body(args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        SHRUG_FACE.to_string()
    } else {
        format!("{args} {SHRUG_FACE}")
    }
}

/// Text body for `/tableflip`.
pub fn format_tableflip_body(args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        TABLEFLIP_FACE.to_string()
    } else {
        format!("{args} {TABLEFLIP_FACE}")
    }
}

/// True when the composer should show the command popup (starts with prefix, no
/// completed args yet — still editing the command name, or empty after prefix).
pub fn command_popup_partial(plain: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    let plain = plain.trim_start();
    if !plain.starts_with(prefix) {
        return None;
    }
    let rest = &plain[prefix.len()..];
    if rest.contains(char::is_whitespace) {
        return None;
    }
    Some(rest.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_partial_stops_after_space() {
        assert_eq!(command_popup_partial("/me", "/").as_deref(), Some("me"));
        assert_eq!(command_popup_partial("/m", "/").as_deref(), Some("m"));
        assert_eq!(command_popup_partial("/", "/").as_deref(), Some(""));
        assert!(command_popup_partial("/me waves", "/").is_none());
        assert!(command_popup_partial("!me", "/").is_none());
    }
}
