//! Per-user server-rail folders: nesting limits, cycle checks, and colors.
//!
//! Folders are private to each account. A folder may contain servers and other
//! folders. Colors use the same 24-bit RGB range as guild roles (`0` = default).

use std::collections::{HashMap, HashSet};

use crate::error::BanterError;
use crate::roles::role_color_ok;

/// Deepest allowed folder nesting from a top-level root (root depth = 1).
pub const MAX_SERVER_FOLDER_DEPTH: i64 = 8;

/// Soft cap so a broken client cannot flood the rail with empty folders.
pub const MAX_SERVER_FOLDERS: i64 = 100;

pub fn folder_color_ok(color: i64) -> bool {
    role_color_ok(color)
}

/// Depth of `folder_id` in `parent_of` (root with no parent = 1).
pub fn folder_depth(folder_id: i64, parent_of: &HashMap<i64, Option<i64>>) -> Result<i64, BanterError> {
    let mut depth = 1i64;
    let mut current = folder_id;
    let mut seen = HashSet::new();
    while let Some(parent) = parent_of.get(&current).copied().flatten() {
        if !seen.insert(current) {
            return Err(BanterError::InvalidName);
        }
        depth += 1;
        if depth > MAX_SERVER_FOLDER_DEPTH {
            return Err(BanterError::InvalidName);
        }
        current = parent;
    }
    Ok(depth)
}

/// True when moving `folder_id` under `new_parent` would create a cycle.
pub fn would_cycle(folder_id: i64, new_parent: Option<i64>, parent_of: &HashMap<i64, Option<i64>>) -> bool {
    let Some(mut current) = new_parent else {
        return false;
    };
    if current == folder_id {
        return true;
    }
    let mut seen = HashSet::from([folder_id]);
    while seen.insert(current) {
        match parent_of.get(&current).copied().flatten() {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    true
}

/// Depth a new child under `parent_id` would have. `None` parent → depth 1.
pub fn child_depth(parent_id: Option<i64>, parent_of: &HashMap<i64, Option<i64>>) -> Result<i64, BanterError> {
    match parent_id {
        None => Ok(1),
        Some(parent) => {
            if !parent_of.contains_key(&parent) {
                return Err(BanterError::NotFound);
            }
            Ok(folder_depth(parent, parent_of)? + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(i64, Option<i64>)]) -> HashMap<i64, Option<i64>> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn color_matches_role_rgb_range() {
        assert!(folder_color_ok(0));
        assert!(folder_color_ok(0xFFFFFF));
        assert!(!folder_color_ok(-1));
        assert!(!folder_color_ok(0x0100_0000));
    }

    #[test]
    fn depth_and_cycle_rules() {
        let tree = map(&[(1, None), (2, Some(1)), (3, Some(2))]);
        assert_eq!(folder_depth(1, &tree).unwrap(), 1);
        assert_eq!(folder_depth(3, &tree).unwrap(), 3);
        assert_eq!(child_depth(Some(3), &tree).unwrap(), 4);
        assert_eq!(child_depth(None, &tree).unwrap(), 1);
        assert!(would_cycle(1, Some(2), &tree));
        assert!(would_cycle(1, Some(3), &tree));
        assert!(would_cycle(2, Some(2), &tree));
        assert!(!would_cycle(3, Some(1), &tree));
        assert!(!would_cycle(2, None, &tree));
    }
}
