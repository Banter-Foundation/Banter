//! Contracts for Discord-parity profile customization.
//! Written before the store/API implementation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use banter_server::{
    BanterError, CosmeticKind, ProfileConnectionInput, ProfileWidgetInput, ProfileWidgetKind,
    Store, cosmetics_catalog,
};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("banter-profile-custom-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn base_url() -> String {
    std::env::var("BANTER_DATABASE_URL").unwrap_or_else(|_| banter_server::DEFAULT_DATABASE_URL.to_string())
}

fn schema_url(schema: &str) -> String {
    let base = base_url();
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}options=-csearch_path%3D{schema}")
}

async fn ensure_schema(schema: &str) {
    let pool = sqlx::PgPool::connect(&base_url()).await.expect("connect postgres");
    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .execute(&pool)
        .await
        .expect("create schema");
}

async fn open_store_at(dir: &Path) -> Store {
    let marker = dir.join("pg_schema");
    let schema = if marker.exists() {
        std::fs::read_to_string(&marker).unwrap()
    } else {
        let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
        let schema = format!("t{}{}", std::process::id(), n);
        ensure_schema(&schema).await;
        std::fs::write(&marker, &schema).unwrap();
        schema
    };
    Store::open(dir, &schema_url(&schema))
        .await
        .expect("open store")
}

async fn open_store() -> (Store, PathBuf) {
    let dir = scratch();
    let store = open_store_at(&dir).await;
    (store, dir)
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
fn badges_catalog_covers_discord_badge_row_shapes() {
    let catalog = banter_server::badges_catalog();
    assert!(catalog.len() >= 6);
    for item in &catalog {
        assert!(!item.id.is_empty());
        assert!(!item.label.is_empty());
    }
    assert!(banter_server::badge_exists("banter-leaf"));
    assert!(!banter_server::badge_exists("not-real"));
}

#[tokio::test]
async fn new_profile_defaults_match_discord_empty_customization() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let profile = store.user_profile(ada.id, ada.id, None).await.unwrap();

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
    assert!(profile.badges.is_empty());
    assert!(profile.activity.is_empty());
    assert!(profile.wishlist.is_empty());
    // Default theme is Banter charcoal / accent, not Discord blurple branding.
    assert_ne!(profile.theme_primary, 0);
    assert_ne!(profile.theme_secondary, 0);
}

#[tokio::test]
async fn owner_can_patch_profile_customization_and_friends_see_it() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let req = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, req.id).await.unwrap();

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
                badge_ids: Some(vec!["banter-leaf".into(), "night-eye".into()]),
                activity: Some(vec![banter_server::ProfileActivityInput {
                    kind: "playing".into(),
                    title: "Hytale".into(),
                    detail: "Building".into(),
                }]),
                wishlist: Some(vec![banter_server::ProfileWishlistInput {
                    title: "Minecraft".into(),
                    tag: "Soon".into(),
                }]),
            },
        )
        .await
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
    assert_eq!(updated.badges.len(), 2);
    assert_eq!(updated.badges[0].id, "banter-leaf");
    assert_eq!(updated.activity.len(), 1);
    assert_eq!(updated.activity[0].title, "Hytale");
    assert_eq!(updated.wishlist.len(), 1);
    assert_eq!(updated.wishlist[0].title, "Minecraft");

    let friend_view = store.user_profile(bob.id, ada.id, None).await.unwrap();
    assert_eq!(friend_view.about_me.as_deref(), Some("Creator code OWL"));
    assert_eq!(friend_view.widgets.len(), 2);
    assert_eq!(friend_view.connections.len(), 1);
    assert_eq!(friend_view.display_name_style_id.as_deref(), Some(style.id.as_str()));
    assert_eq!(friend_view.badges.len(), 2);
    assert_eq!(friend_view.activity.len(), 1);
    assert_eq!(friend_view.wishlist.len(), 1);
}

#[tokio::test]
async fn unknown_cosmetic_ids_are_rejected() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
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
                badge_ids: None,
                activity: None,
                wishlist: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, BanterError::InvalidProfile);
}

#[tokio::test]
async fn empty_string_clears_optional_profile_fields() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
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
                badge_ids: None,
                activity: None,
                wishlist: None,
            },
        )
        .await
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
                badge_ids: None,
                activity: None,
                wishlist: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(cleared.about_me, None);
    assert_eq!(cleared.pronouns, None);
    assert_eq!(cleared.custom_status, None);
    assert_eq!(cleared.nameplate_id, None);
    assert!(cleared.widgets.is_empty());
    assert!(cleared.connections.is_empty());
}

#[tokio::test]
async fn banner_upload_increments_rev_and_is_readable() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    assert!(!store.user_profile(ada.id, ada.id, None).await.unwrap().has_banner);

    store.set_banner(ada.id, &tiny_png()).await.unwrap();
    let with_banner = store.user_profile(ada.id, ada.id, None).await.unwrap();
    assert!(with_banner.has_banner);
    assert_eq!(with_banner.banner_rev, 1);
    assert!(!store.banner_bytes(ada.id).await.unwrap().is_empty());

    store.clear_banner(ada.id).await.unwrap();
    let cleared = store.user_profile(ada.id, ada.id, None).await.unwrap();
    assert!(!cleared.has_banner);
    assert_eq!(cleared.banner_rev, 0);
    assert_eq!(store.banner_bytes(ada.id).await.unwrap_err(), BanterError::NotFound);
}

#[tokio::test]
async fn profile_text_limits_match_discord_style_caps() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
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
                badge_ids: None,
                activity: None,
                wishlist: None,
                },
            )
            .await
            .unwrap_err(),
        BanterError::InvalidProfile
    );
}
