//! Contract for Emote Library (BA-83). Written before the implementation.
//!
//! Covers packs, asset upload (emote/sticker/GIF), 15s duration gate, subscribe,
//! server packs, Model A sticker send, and GIF media on messages. No seed packs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use banter_server::{
    BanterError, ChatTarget, EmoteAssetKind, EmoteGifEncoding, EmotePackOwner, EmotePackVisibility,
    EmotePickerSource, Store, MAX_EMOTE_BYTES, MAX_GIF_BYTES, MAX_STICKER_BYTES,
    classic_gif_duration_ms, video_container_duration_ms, GIF_MAX_DURATION_MS,
};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("banter-emote-{}-{n}", std::process::id()));
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
        let schema = format!("e{}{}", std::process::id(), n);
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
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb([20, 40, 60]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

/// Minimal animated GIF: two frames, each with 200cs (2s) delay → 4s total.
fn short_animated_gif() -> Vec<u8> {
    // Hand-built GIF89a: 1x1, two graphic frames with GCE delay 200 (2.00s each).
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&[1, 0, 1, 0, 0x00, 0x00, 0x00]); // logical screen, no GCT
    // Netscape loop (ignored for duration)
    out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[0x03, 0x01, 0x00, 0x00, 0x00]);
    for _ in 0..2 {
        // GCE: delay 200 cs
        out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0xC8, 0x00, 0x00, 0x00]);
        // Image descriptor 1x1 + minimal LZW
        out.extend_from_slice(&[0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        out.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    }
    out.push(0x3B);
    out
}

/// Same structure as short_animated_gif but delays of 800 cs × 2 = 16s (>15s).
fn long_animated_gif() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&[1, 0, 1, 0, 0x00, 0x00, 0x00]);
    for _ in 0..2 {
        out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0x20, 0x03, 0x00, 0x00]); // 800 cs
        out.extend_from_slice(&[0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        out.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    }
    out.push(0x3B);
    out
}

#[test]
fn classic_gif_duration_sums_frame_delays() {
    let short = short_animated_gif();
    let ms = classic_gif_duration_ms(&short).expect("parse short gif");
    assert_eq!(ms, 4000);
    assert!(ms <= GIF_MAX_DURATION_MS);

    let long = long_animated_gif();
    let ms = classic_gif_duration_ms(&long).expect("parse long gif");
    assert_eq!(ms, 16000);
    assert!(ms > GIF_MAX_DURATION_MS);
}

#[test]
fn zero_delay_gif_frame_counts_as_100ms() {
    // One frame, delay 0 → treated as 100 ms.
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&[1, 0, 1, 0, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    out.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    out.push(0x3B);
    assert_eq!(classic_gif_duration_ms(&out).unwrap(), 100);
}

#[test]
fn video_mp4_duration_from_mvhd() {
    // Build a tiny fake MP4 with an `mvhd` atom (version 0) duration 10s @ timescale 1000.
    let mut mvhd_body = Vec::new();
    mvhd_body.push(0); // version
    mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
    mvhd_body.extend_from_slice(&0u32.to_be_bytes()); // creation
    mvhd_body.extend_from_slice(&0u32.to_be_bytes()); // modification
    mvhd_body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
    mvhd_body.extend_from_slice(&10_000u32.to_be_bytes()); // duration
    mvhd_body.extend_from_slice(&[0u8; 80]); // remainder ignored

    let mut moov = Vec::new();
    let mvhd_size = (8 + mvhd_body.len()) as u32;
    moov.extend_from_slice(&mvhd_size.to_be_bytes());
    moov.extend_from_slice(b"mvhd");
    moov.extend_from_slice(&mvhd_body);

    let mut mp4 = Vec::new();
    let moov_size = (8 + moov.len()) as u32;
    mp4.extend_from_slice(&moov_size.to_be_bytes());
    mp4.extend_from_slice(b"moov");
    mp4.extend_from_slice(&moov);

    let ms = video_container_duration_ms(&mp4).expect("mvhd duration");
    assert_eq!(ms, 10_000);
    assert!(ms <= GIF_MAX_DURATION_MS);
}

#[tokio::test]
async fn empty_library_until_user_creates_pack() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Lab").await.unwrap();
    let packs = store
        .list_emote_assets_for_picker(
            ada.id,
            Some(home.server.id),
            EmoteAssetKind::Emote,
            EmotePickerSource::Server,
        )
        .await
        .unwrap();
    assert!(packs.is_empty(), "no mock/seed packs");
    let subscribed = store
        .list_emote_assets_for_picker(
            ada.id,
            Some(home.server.id),
            EmoteAssetKind::Emote,
            EmotePickerSource::Subscribed,
        )
        .await
        .unwrap();
    assert!(subscribed.is_empty());
}

#[tokio::test]
async fn create_user_pack_upload_emote_and_insert_token() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Party",
            "party",
            "fun",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    assert_eq!(pack.name, "Party");
    assert_eq!(pack.slug, "party");
    assert_eq!(pack.owner_kind, "user");
    assert_eq!(pack.owner_id, ada.id);

    let asset = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "party_parrot",
            EmoteAssetKind::Emote,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();
    assert_eq!(asset.name, "party_parrot");
    assert_eq!(asset.kind, EmoteAssetKind::Emote);
    assert!(asset.byte_size > 0);
    assert!(asset.byte_size as usize <= MAX_EMOTE_BYTES);

    store.subscribe_emote_pack(ada.id, pack.id).await.unwrap();
    let home = store.create_server(ada.id, "Lab").await.unwrap();
    let channel = home
        .categories
        .iter()
        .flat_map(|c| c.channels.iter())
        .find(|c| c.kind == banter_server::ChannelKind::Text)
        .unwrap()
        .id;
    let body = format!("hi :{{e:{}}}:", asset.id);
    let message = store
        .send_message(ada.id, ChatTarget::Channel(channel), &body, None, &[], &[])
        .await
        .unwrap();
    assert!(message.body.contains(&format!(":{{e:{}}}:", asset.id)));
    assert!(message.media.is_empty());
}

#[tokio::test]
async fn emote_upload_rejects_oversized_file() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Big",
            "big",
            "",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    let too_big = vec![0u8; MAX_EMOTE_BYTES + 1];
    // Pretend PNG magic so we hit the size gate before decode.
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&too_big);
    assert_eq!(
        store
            .upload_emote_asset(ada.id, pack.id, "huge", EmoteAssetKind::Emote, "image/png", &bytes)
            .await
            .unwrap_err(),
        BanterError::FileTooLarge
    );
}

#[tokio::test]
async fn sticker_model_a_primary_send() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Stickers",
            "stickers",
            "",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    let sticker = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "wave",
            EmoteAssetKind::Sticker,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();
    assert!(sticker.byte_size as usize <= MAX_STICKER_BYTES);
    store.subscribe_emote_pack(ada.id, pack.id).await.unwrap();

    let home = store.create_server(ada.id, "Lab").await.unwrap();
    let channel = home
        .categories
        .iter()
        .flat_map(|c| c.channels.iter())
        .find(|c| c.kind == banter_server::ChannelKind::Text)
        .unwrap()
        .id;

    // Empty body + one sticker is valid (Model A).
    let message = store
        .send_message(
            ada.id,
            ChatTarget::Channel(channel),
            "",
            None,
            &[],
            &[sticker.id],
        )
        .await
        .unwrap();
    assert!(message.body.is_empty());
    assert_eq!(message.media.len(), 1);
    assert_eq!(message.media[0].asset_id, sticker.id);
    assert_eq!(message.media[0].kind, EmoteAssetKind::Sticker);
    assert_eq!(message.media[0].name, "wave");

    // Two stickers in one message rejected.
    let sticker2 = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "wave2",
            EmoteAssetKind::Sticker,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .send_message(
                ada.id,
                ChatTarget::Channel(channel),
                "",
                None,
                &[],
                &[sticker.id, sticker2.id],
            )
            .await
            .unwrap_err(),
        BanterError::Invalid
    );
}

#[tokio::test]
async fn gif_upload_rejects_over_15_seconds() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Gifs",
            "gifs",
            "",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .upload_emote_asset(
                ada.id,
                pack.id,
                "too_long",
                EmoteAssetKind::Gif,
                "image/gif",
                &long_animated_gif(),
            )
            .await
            .unwrap_err(),
        BanterError::EmoteDurationTooLong
    );

    let ok = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "short_clip",
            EmoteAssetKind::Gif,
            "image/gif",
            &short_animated_gif(),
        )
        .await
        .unwrap();
    assert_eq!(ok.encoding, Some(EmoteGifEncoding::ClassicGif));
    assert!(ok.duration_ms.unwrap() <= GIF_MAX_DURATION_MS);
    assert!(ok.byte_size as usize <= MAX_GIF_BYTES);
}

#[tokio::test]
async fn gif_send_as_message_media() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Gifs",
            "gifs",
            "",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    let gif = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "dance",
            EmoteAssetKind::Gif,
            "image/gif",
            &short_animated_gif(),
        )
        .await
        .unwrap();
    store.subscribe_emote_pack(ada.id, pack.id).await.unwrap();
    let home = store.create_server(ada.id, "Lab").await.unwrap();
    let channel = home
        .categories
        .iter()
        .flat_map(|c| c.channels.iter())
        .find(|c| c.kind == banter_server::ChannelKind::Text)
        .unwrap()
        .id;
    let message = store
        .send_message(
            ada.id,
            ChatTarget::Channel(channel),
            "look",
            None,
            &[],
            &[gif.id],
        )
        .await
        .unwrap();
    assert_eq!(message.body, "look");
    assert_eq!(message.media.len(), 1);
    assert_eq!(message.media[0].kind, EmoteAssetKind::Gif);
}

#[tokio::test]
async fn server_pack_auto_available_and_subscribe_to_keep() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Lab").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Server Faces",
            "server-faces",
            "",
            EmotePackVisibility::Unlisted,
            EmotePackOwner::Server(home.server.id),
        )
        .await
        .unwrap();
    let asset = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "lab",
            EmoteAssetKind::Emote,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();

    // Member sees server pack without personal subscribe.
    let invite = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &invite).await.unwrap();
    let server_assets = store
        .list_emote_assets_for_picker(
            bob.id,
            Some(home.server.id),
            EmoteAssetKind::Emote,
            EmotePickerSource::Server,
        )
        .await
        .unwrap();
    assert!(server_assets.iter().any(|a| a.id == asset.id));

    // Subscribe-to-keep unlocks pack under Subscribed (cross-server).
    store.subscribe_emote_pack(bob.id, pack.id).await.unwrap();
    let kept = store
        .list_emote_assets_for_picker(
            bob.id,
            None,
            EmoteAssetKind::Emote,
            EmotePickerSource::Subscribed,
        )
        .await
        .unwrap();
    assert!(kept.iter().any(|a| a.id == asset.id));

    store.unsubscribe_emote_pack(bob.id, pack.id).await.unwrap();
    let after = store
        .list_emote_assets_for_picker(
            bob.id,
            None,
            EmoteAssetKind::Emote,
            EmotePickerSource::Subscribed,
        )
        .await
        .unwrap();
    assert!(!after.iter().any(|a| a.id == asset.id));
}

#[tokio::test]
async fn mixed_kind_pack_ok_and_read_bytes() {
    let (store, dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Mixed",
            "mixed",
            "",
            EmotePackVisibility::Private,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    let emote = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "e1",
            EmoteAssetKind::Emote,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();
    let sticker = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "s1",
            EmoteAssetKind::Sticker,
            "image/png",
            &tiny_png(),
        )
        .await
        .unwrap();
    let gif = store
        .upload_emote_asset(
            ada.id,
            pack.id,
            "g1",
            EmoteAssetKind::Gif,
            "image/gif",
            &short_animated_gif(),
        )
        .await
        .unwrap();
    assert_ne!(emote.id, sticker.id);
    let (_meta, bytes) = store.read_emote_asset(ada.id, emote.id).await.unwrap();
    assert!(!bytes.is_empty());
    // Bytes land under BANTER_DATA/emotes/
    assert!(dir.join("emotes").exists());
    let _ = gif;
}

#[tokio::test]
async fn non_owner_cannot_upload_to_user_pack() {
    let (store, _dir) = open_store().await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let pack = store
        .create_emote_pack(
            ada.id,
            "Ada Only",
            "ada-only",
            "",
            EmotePackVisibility::Public,
            EmotePackOwner::User,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .upload_emote_asset(
                bob.id,
                pack.id,
                "nope",
                EmoteAssetKind::Emote,
                "image/png",
                &tiny_png(),
            )
            .await
            .unwrap_err(),
        BanterError::NotOwner
    );
}
