//! Per-guild chat command settings: prefix and aliases.

use crate::error::BanterError;
use crate::model::{CommandAlias, ServerCommandSettings};

/// Default composer command prefix when a server has not customized it.
pub const DEFAULT_COMMAND_PREFIX: &str = "/";

const MAX_PREFIX_CHARS: usize = 8;
const MAX_ALIAS_CHARS: usize = 32;
const MAX_ALIASES: usize = 64;

/// Reject empty, whitespace, or overly long prefixes.
pub fn validate_command_prefix(prefix: &str) -> Result<(), BanterError> {
    if prefix.is_empty() || prefix.len() > MAX_PREFIX_CHARS {
        return Err(BanterError::Invalid);
    }
    if prefix.chars().any(|ch| ch.is_whitespace()) {
        return Err(BanterError::Invalid);
    }
    Ok(())
}

/// Command / alias names: lowercase letters, digits, underscore, hyphen.
pub fn validate_command_name(name: &str) -> Result<(), BanterError> {
    if name.is_empty() || name.len() > MAX_ALIAS_CHARS {
        return Err(BanterError::Invalid);
    }
    if !name
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_' | '-'))
    {
        return Err(BanterError::Invalid);
    }
    Ok(())
}

/// Normalize and validate a patch of aliases. Sorted by alias for stable responses.
/// Names must already be lowercase (uppercase is rejected before normalize).
pub fn normalize_command_aliases(
    aliases: Vec<CommandAlias>,
) -> Result<Vec<CommandAlias>, BanterError> {
    if aliases.len() > MAX_ALIASES {
        return Err(BanterError::Invalid);
    }
    let mut out = Vec::with_capacity(aliases.len());
    let mut seen = std::collections::HashSet::new();
    for entry in aliases {
        let alias = entry.alias.trim().to_string();
        let target = entry.target.trim().to_string();
        validate_command_name(&alias)?;
        validate_command_name(&target)?;
        if alias == target {
            return Err(BanterError::Invalid);
        }
        if !seen.insert(alias.clone()) {
            return Err(BanterError::Invalid);
        }
        out.push(CommandAlias { alias, target });
    }
    out.sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(out)
}

pub fn default_command_settings() -> ServerCommandSettings {
    ServerCommandSettings {
        prefix: DEFAULT_COMMAND_PREFIX.to_string(),
        aliases: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_uppercase_alias_names() {
        assert_eq!(
            validate_command_name("BAD"),
            Err(BanterError::Invalid)
        );
    }
}
