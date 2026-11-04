/// Contracts for user profile cards. Written before the profile UI landed.

use banter::{
    ProfileAction, ProfileCard, ProfileKind, display_name_dirty, display_name_error, profile_heading,
    profile_primary_actions,
};

#[test]
fn profile_card_opens_closes_and_toggles_edit() {
    let mut card = ProfileCard::closed();
    assert!(!card.is_open());
    assert!(!card.editing());

    card.open(7, ProfileKind::SelfAccount);
    assert!(card.is_open());
    assert_eq!(card.user_id(), Some(7));
    assert_eq!(card.kind(), Some(ProfileKind::SelfAccount));
    assert!(!card.editing());

    assert!(card.begin_edit("Ada Lovelace"));
    assert!(card.editing());
    assert_eq!(card.draft_display_name(), "Ada Lovelace");
    card.set_draft_display_name("Ada L.");
    assert!(display_name_dirty(card.draft_display_name(), "Ada Lovelace"));
    assert!(display_name_error(card.draft_display_name()).is_none());

    card.cancel_edit();
    assert!(!card.editing());
    assert_eq!(card.draft_display_name(), "");

    card.open(7, ProfileKind::SelfAccount);
    card.begin_edit("Ada");
    card.close();
    assert!(!card.is_open());
    assert!(!card.editing());
    assert_eq!(card.user_id(), None);
}

#[test]
fn profile_card_reopen_replaces_the_target_and_drops_edit() {
    let mut card = ProfileCard::closed();
    card.open(1, ProfileKind::SelfAccount);
    card.begin_edit("Me");
    card.open(2, ProfileKind::Friend);
    assert_eq!(card.user_id(), Some(2));
    assert_eq!(card.kind(), Some(ProfileKind::Friend));
    assert!(!card.editing());
}

#[test]
fn profile_heading_prefers_server_nickname() {
    assert_eq!(profile_heading("Ada", None), "Ada");
    assert_eq!(profile_heading("Ada", Some("")), "Ada");
    assert_eq!(profile_heading("Ada", Some("  Night Owl  ")), "Night Owl");
}

#[test]
fn profile_primary_actions_match_foundation_basics() {
    assert_eq!(
        profile_primary_actions(ProfileKind::SelfAccount),
        &[ProfileAction::EditProfile]
    );
    assert_eq!(
        profile_primary_actions(ProfileKind::Friend),
        &[ProfileAction::Message]
    );
    assert_eq!(
        profile_primary_actions(ProfileKind::ServerMember { is_friend: false }),
        &[ProfileAction::AddFriend]
    );
    assert_eq!(
        profile_primary_actions(ProfileKind::ServerMember { is_friend: true }),
        &[ProfileAction::Message]
    );
}

#[test]
fn profile_edit_only_applies_to_self() {
    let mut card = ProfileCard::closed();
    card.open(9, ProfileKind::Friend);
    assert!(!card.begin_edit("Bob"));
    assert!(!card.editing());

    card.open(9, ProfileKind::ServerMember { is_friend: false });
    assert!(!card.begin_edit("Cam"));
    assert!(!card.editing());
}
