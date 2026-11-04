//! Guild roles: permission bits, hierarchy, and the default @everyone role.
//!
//! Higher `position` outranks a lower one. The server owner bypasses the
//! hierarchy. @everyone is position 0 and is implicit for every member.

use crate::model::GuildRole;

pub const PERM_ADMINISTRATOR: i64 = 1 << 0;
pub const PERM_MANAGE_CHANNELS: i64 = 1 << 1;
pub const PERM_MANAGE_ROLES: i64 = 1 << 2;
pub const PERM_CREATE_INVITE: i64 = 1 << 3;

/// Bits a role editor is allowed to store.
pub const ROLE_PERMISSION_MASK: i64 =
    PERM_ADMINISTRATOR | PERM_MANAGE_CHANNELS | PERM_MANAGE_ROLES | PERM_CREATE_INVITE;

/// New servers grant invite on @everyone so joining still matches the old member invite.
pub const EVERYONE_PERMISSIONS: i64 = PERM_CREATE_INVITE;

pub const EVERYONE_NAME: &str = "@everyone";

pub const MAX_GUILD_ROLES: i64 = 50;

pub fn has_permission(bits: i64, flag: i64) -> bool {
    bits & PERM_ADMINISTRATOR != 0 || bits & flag != 0
}

pub fn combine_permissions(bits: impl IntoIterator<Item = i64>) -> i64 {
    bits.into_iter().fold(0, |acc, bit| acc | bit)
}

/// `requested` may only include bits the actor already holds. Administrator holds every bit.
pub fn grants_only_held(actor_bits: i64, requested: i64) -> bool {
    if has_permission(actor_bits, PERM_ADMINISTRATOR) {
        return true;
    }
    requested & !actor_bits == 0
}

/// Next position strictly below `ceiling`. `None` when there is no free slot under the actor.
pub fn next_role_position(positions: &[i64], ceiling: i64) -> Option<i64> {
    if ceiling <= 0 {
        return None;
    }
    let below = positions.iter().copied().filter(|position| *position < ceiling).max().unwrap_or(0);
    let next = below.saturating_add(1);
    if next < ceiling { Some(next) } else { None }
}

pub fn role_color_ok(color: i64) -> bool {
    (0..=0x00FF_FFFF).contains(&color)
}

/// Highest colored role wins the member-list name color. `0` means no color.
pub fn top_role_color(roles: &[GuildRole]) -> Option<i64> {
    roles
        .iter()
        .filter(|role| role.color > 0)
        .max_by_key(|role| role.position)
        .map(|role| role.color)
}

/// Highest hoisted role, ignoring @everyone.
pub fn top_hoisted_role(roles: &[GuildRole]) -> Option<&GuildRole> {
    roles
        .iter()
        .filter(|role| role.hoist && !role.everyone)
        .max_by_key(|role| role.position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(position: i64, color: i64, hoist: bool, everyone: bool) -> GuildRole {
        GuildRole {
            id: position,
            server_id: 1,
            name: format!("r{position}"),
            color,
            position,
            hoist,
            permissions: 0,
            everyone,
        }
    }

    #[test]
    fn administrator_covers_every_flag_and_grants() {
        assert!(has_permission(PERM_ADMINISTRATOR, PERM_MANAGE_CHANNELS));
        assert!(has_permission(PERM_MANAGE_ROLES, PERM_MANAGE_ROLES));
        assert!(!has_permission(PERM_CREATE_INVITE, PERM_MANAGE_CHANNELS));
        assert!(grants_only_held(PERM_ADMINISTRATOR, PERM_MANAGE_CHANNELS | PERM_MANAGE_ROLES));
        assert!(grants_only_held(PERM_MANAGE_ROLES | PERM_CREATE_INVITE, PERM_CREATE_INVITE));
        assert!(!grants_only_held(PERM_MANAGE_ROLES, PERM_MANAGE_CHANNELS));
        assert!(grants_only_held(PERM_MANAGE_ROLES, 0));
    }

    #[test]
    fn new_role_sits_under_the_actor() {
        assert_eq!(next_role_position(&[0, 5], i64::MAX), Some(6));
        assert_eq!(next_role_position(&[0, 5], 5), Some(1));
        assert_eq!(next_role_position(&[0], 1), None);
        assert_eq!(next_role_position(&[], i64::MAX), Some(1));
    }

    #[test]
    fn color_and_hoist_follow_the_highest_role() {
        let roles = vec![role(0, 0, false, true), role(2, 0x5865F2, true, false), role(4, 0, true, false)];
        assert_eq!(top_role_color(&roles), Some(0x5865F2));
        assert_eq!(top_hoisted_role(&roles).map(|role| role.position), Some(4));
        assert!(role_color_ok(0));
        assert!(role_color_ok(0xFFFFFF));
        assert!(!role_color_ok(-1));
        assert!(!role_color_ok(0x0100_0000));
    }
}
