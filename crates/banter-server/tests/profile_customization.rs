//! Contracts for Discord-parity profile customization.
//! Written before the store/API implementation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use banter_server::{
    BanterError, CosmeticKind, ProfileConnectionInput, ProfileWidgetInput, ProfileWidgetKind,
    Store, cosmetics_catalog,
};

fn scratch() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("banter-profile-custom-{nanos}"));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn tiny_png() -> Vec<u8> {
    // 2x2 solid PNG
    let mut rgba = image::RgbaImage::new(2, 2);
    for pixel in rgba.pixels_mut() {
        *pixel = image::Rgba([40, 120, 80, 255]);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn cosmetics_catalog_covers_discord_editor_slots() {
    let catalog = cosmetics_catalog();
    assert!(catalog.iter().any(|item| item.kind == CosmeticKind::Nameplate));
    assert!(catalog.iter().any(|item| item.kind == CosmeticKind::AvatarDecoration));
    assert!(catalog.iter().any(|item| item.kind == CosmeticKind::DisplayNameStyle));
    assert!(catalog.iter().any(|item| item.kind == CosmeticKind::ProfileEffect));
    assert!(catalog.iter().any(|item| item.kind == CosmeticKind::ProfileFrame));
    for item in &catalog {
        assert!(!item.id.is_empty());
        assert!(!item.label.is_empty());
    }
}

#[test]
fn new_profile_defaults_match_discord_empty_customization() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let profile = store.user_profile(ada.id, ada.id, None).unwrap();

    assert_eq!(profile.about_me.as_deref(), None);
    assert_eq!(profile.pronouns.as_deref(), None);
    assert_eq!(profile.custom_status.as_deref(), None);
    assert!(!profile.has_banner);
    assert_eq!(profile.banner_rev, 0);
    assert!(profile.member_since > 0);
    assert_eq!(profile.nameplate_id.as_deref(), None);
    assert_eq!(profile.avatar_decoration_id.as_deref(), None);
    assert_eq!(profile.display_name_style_id.as_deref(), None);
    assert_eq!(profile.profile_effect_id.as_deref(), None);
    assert_eq!(profile.profile_frame_id.as_deref(), None);
    assert!(profile.widgets.is_empty());
    assert!(profile.connections.is_empty());
    // Default theme is Banter charcoal / accent, not Discord blurple branding.
    assert_ne!(profile.theme_primary, 0);
    assert_ne!(profile.theme_secondary, 0);
}

#[test]
fn owner_can_patch_profile_customization_and_friends_see_it() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let bob = store.register("bob", "Bob", "password1").unwrap();
    let req = store.send_friend_request(ada.id, "bob").unwrap();
    store.accept_friend_request(bob.id, req.id).unwrap();

    let style = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::DisplayNameStyle)
        .expect("display name style");
    let deco = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::AvatarDecoration)
        .expect("decoration");
    let plate = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::Nameplate)
        .expect("nameplate");
    let effect = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::ProfileEffect)
        .expect("effect");
    let frame = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::ProfileFrame)
        .expect("frame");

    let updated = store
        .update_profile(
            ada.id,
            banter_server::ProfilePatch {
                display_name: Some("Ada L.".into()),
                about_me: Some("Creator code OWL".into()),
                pronouns: Some("she/her".into()),
                custom_status: Some("Favorite NPC companion?".into()),
                theme_primary: Some(0x2d4a3e),
                theme_secondary: Some(0x0f1a14),
                nameplate_id: Some(plate.id.clone()),
                avatar_decoration_id: Some(deco.id.clone()),
                display_name_style_id: Some(style.id.clone()),
                profile_effect_id: Some(effect.id.clone()),
                profile_frame_id: Some(frame.id.clone()),
                widgets: Some(vec![
                    ProfileWidgetInput {
                        kind: ProfileWidgetKind::FavoriteGame,
                        title: "Favorite Game".into(),
                        payload: serde_json::json!({
                            "title": "Hytale",
                            "quote": "Let everyone know why this is your favorite"
                        }),
                    },
                    ProfileWidgetInput {
                        kind: ProfileWidgetKind::GamesInRotation,
                        title: "Games in Rotation".into(),
                        payload: serde_json::json!({
                            "games": [
                                {"title": "Hytale", "tag": "Obsessed"},
                                {"title": "Minecraft", "tag": "Expert"}
                            ]
                        }),
                    },
                ]),
                connections: Some(vec![ProfileConnectionInput {
                    kind: "website".into(),
                    name: "owl.site".into(),
                    url: "https://example.com/owl".into(),
                }]),
            },
        )
        .unwrap();

    assert_eq!(updated.display_name, "Ada L.");
    assert_eq!(updated.about_me.as_deref(), Some("Creator code OWL"));
    assert_eq!(updated.pronouns.as_deref(), Some("she/her"));
    assert_eq!(updated.custom_status.as_deref(), Some("Favorite NPC companion?"));
    assert_eq!(updated.theme_primary, 0x2d4a3e);
    assert_eq!(updated.theme_secondary, 0x0f1a14);
    assert_eq!(updated.nameplate_id.as_deref(), Some(plate.id.as_str()));
    assert_eq!(updated.avatar_decoration_id.as_deref(), Some(deco.id.as_str()));
    assert_eq!(updated.display_name_style_id.as_deref(), Some(style.id.as_str()));
    assert_eq!(updated.profile_effect_id.as_deref(), Some(effect.id.as_str()));
    assert_eq!(updated.profile_frame_id.as_deref(), Some(frame.id.as_str()));
    assert_eq!(updated.widgets.len(), 2);
    assert_eq!(updated.widgets[0].kind, ProfileWidgetKind::FavoriteGame);
    assert_eq!(updated.connections.len(), 1);
    assert_eq!(updated.connections[0].name, "owl.site");

    let friend_view = store.user_profile(bob.id, ada.id, None).unwrap();
    assert_eq!(friend_view.about_me.as_deref(), Some("Creator code OWL"));
    assert_eq!(friend_view.widgets.len(), 2);
    assert_eq!(friend_view.connections.len(), 1);
    assert_eq!(friend_view.display_name_style_id.as_deref(), Some(style.id.as_str()));
}

#[test]
fn unknown_cosmetic_ids_are_rejected() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let err = store
        .update_profile(
            ada.id,
            banter_server::ProfilePatch {
                display_name: None,
                about_me: None,
                pronouns: None,
                custom_status: None,
                theme_primary: None,
                theme_secondary: None,
                nameplate_id: Some("not-a-real-plate".into()),
                avatar_decoration_id: None,
                display_name_style_id: None,
                profile_effect_id: None,
                profile_frame_id: None,
                widgets: None,
                connections: None,
            },
        )
        .unwrap_err();
    assert_eq!(err, BanterError::InvalidProfile);
}

#[test]
fn empty_string_clears_optional_profile_fields() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let plate = cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::Nameplate)
        .unwrap();
    store
        .update_profile(
            ada.id,
            banter_server::ProfilePatch {
                display_name: None,
                about_me: Some("bio".into()),
                pronouns: Some("they/them".into()),
                custom_status: Some("hi".into()),
                theme_primary: None,
                theme_secondary: None,
                nameplate_id: Some(plate.id.clone()),
                avatar_decoration_id: None,
                display_name_style_id: None,
                profile_effect_id: None,
                profile_frame_id: None,
                widgets: None,
                connections: None,
            },
        )
        .unwrap();

    let cleared = store
        .update_profile(
            ada.id,
            banter_server::ProfilePatch {
                display_name: None,
                about_me: Some(String::new()),
                pronouns: Some(String::new()),
                custom_status: Some(String::new()),
                theme_primary: None,
                theme_secondary: None,
                nameplate_id: Some(String::new()),
                avatar_decoration_id: None,
                display_name_style_id: None,
                profile_effect_id: None,
                profile_frame_id: None,
                widgets: Some(vec![]),
                connections: Some(vec![]),
            },
        )
        .unwrap();

    assert_eq!(cleared.about_me, None);
    assert_eq!(cleared.pronouns, None);
    assert_eq!(cleared.custom_status, None);
    assert_eq!(cleared.nameplate_id, None);
    assert!(cleared.widgets.is_empty());
    assert!(cleared.connections.is_empty());
}

#[test]
fn banner_upload_increments_rev_and_is_readable() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    assert!(!store.user_profile(ada.id, ada.id, None).unwrap().has_banner);

    store.set_banner(ada.id, &tiny_png()).unwrap();
    let with_banner = store.user_profile(ada.id, ada.id, None).unwrap();
    assert!(with_banner.has_banner);
    assert_eq!(with_banner.banner_rev, 1);
    assert!(!store.banner_bytes(ada.id).unwrap().is_empty());

    store.clear_banner(ada.id).unwrap();
    let cleared = store.user_profile(ada.id, ada.id, None).unwrap();
    assert!(!cleared.has_banner);
    assert_eq!(cleared.banner_rev, 0);
    assert_eq!(store.banner_bytes(ada.id).unwrap_err(), BanterError::NotFound);
}

#[test]
fn profile_text_limits_match_discord_style_caps() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let too_long_bio = "x".repeat(191);
    assert_eq!(
        store
            .update_profile(
                ada.id,
                banter_server::ProfilePatch {
                    display_name: None,
                    about_me: Some(too_long_bio),
                    pronouns: None,
                    custom_status: None,
                    theme_primary: None,
                    theme_secondary: None,
                    nameplate_id: None,
                    avatar_decoration_id: None,
                    display_name_style_id: None,
                    profile_effect_id: None,
                    profile_frame_id: None,
                    widgets: None,
                    connections: None,
                },
            )
            .unwrap_err(),
        BanterError::InvalidProfile
    );
}
