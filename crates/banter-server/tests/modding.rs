//! Bots, plugin install, and intent checks.

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use banter_server::{BanterError, ChatTarget, Store, call_is_null, inspect_plugin_zip, parse_manifest, zip_files};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("banter-modding-{}-{n}", std::process::id()));
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

async fn open_store() -> (Store, PathBuf) {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let schema = format!("d{}{n}", std::process::id());
    let pool = sqlx::PgPool::connect(&base_url()).await.expect("connect postgres");
    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .execute(&pool)
        .await
        .expect("create schema");
    let dir = scratch();
    let store = Store::open(&dir, &schema_url(&schema)).await.expect("open store");
    (store, dir)
}

fn package(files: &[(&str, &str)]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let options = SimpleFileOptions::default();
    for (name, body) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    cursor.into_inner()
}

#[test]
fn a_missing_intent_is_null_and_an_accepted_one_is_not() {
    assert!(call_is_null(true, &[], None, "send_messages"));
    assert!(call_is_null(true, &["send_messages".into()], Some(&[]), "send_messages"));
    assert!(!call_is_null(true, &["send_messages".into()], Some(&["send_messages".into()]), "send_messages"));
    assert!(!call_is_null(false, &[], None, "send_messages"));
}

#[test]
fn server_only_plugins_cannot_carry_a_client_entry() {
    let manifest = r#"{"id":"chess","name":"Chess","version":"1.0.0","banter":"1","client":"none","server_entry":"server/main.luau","permissions":["storage"]}"#;
    let parsed = parse_manifest(manifest).unwrap();
    assert_eq!(parsed.client, "none");
    assert!(parsed.client_entry.is_none());
    let refused = r#"{"id":"chess","name":"Chess","version":"1.0.0","banter":"1","client":"none","server_entry":"server/main.luau","client_entry":"client/main.luau"}"#;
    assert!(matches!(parse_manifest(refused), Err(BanterError::Invalid)));
    let optional = r#"{"id":"chess","name":"Chess","version":"1.0.0","banter":"1","client":"optional","permissions":[]}"#;
    assert!(matches!(parse_manifest(optional), Err(BanterError::Invalid)));
}

#[test]
fn zip_files_round_trip_a_plugin_manifest() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/welcome");
    let manifest = std::fs::read(root.join("banter.plugin.json")).unwrap();
    let source = std::fs::read(root.join("server/main.luau")).unwrap();
    let bytes = zip_files(&[("banter.plugin.json", &manifest), ("server/main.luau", &source)]).unwrap();
    let parsed = inspect_plugin_zip(&bytes).unwrap();
    assert_eq!(parsed.id, "welcome");
    assert_eq!(parsed.client, "none");
}

#[test]
fn sample_plugins_parse() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
    let welcome = parse_manifest(&std::fs::read_to_string(root.join("welcome/banter.plugin.json")).unwrap()).unwrap();
    assert_eq!(welcome.client, "none");
    assert!(welcome.client_entry.is_none());
    let roll = parse_manifest(&std::fs::read_to_string(root.join("roll/banter.plugin.json")).unwrap()).unwrap();
    assert_eq!(roll.id, "roll");
    let notes = parse_manifest(&std::fs::read_to_string(root.join("notes/banter.plugin.json")).unwrap()).unwrap();
    assert_eq!(notes.client, "optional");
    assert_eq!(notes.client_entry.as_deref(), Some("client/main.luau"));
    assert_eq!(notes.channel_kinds, ["notes.note"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bots_have_no_password_and_missing_send_intent_sends_nothing() {
    let (store, dir) = open_store().await;
    let owner = store.register("ada", "Ada", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Home").await.unwrap();
    let channel = home.categories[0].channels[0].id;
    let session = store
        .create_bot(owner.id, "chessbot", "Chess", &["view_channels".into()])
        .await
        .unwrap();
    assert!(session.account.bot);
    assert!(store.login("chessbot", "password1").await.is_err());
    assert!(store.logout(&session.token).await.is_err());
    let me = store.session_account(&session.token).await.unwrap();
    assert!(store.bot_call_is_null(&me, "send_messages", Some(home.server.id)).await.unwrap());
    store.install_bot(owner.id, home.server.id, me.id).await.unwrap();
    assert!(store.bot_call_is_null(&me, "send_messages", Some(home.server.id)).await.unwrap());
    let _ = channel;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_send_intent_lets_the_bot_post() {
    let (store, dir) = open_store().await;
    let owner = store.register("bea", "Bea", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Home").await.unwrap();
    let channel = home.categories[0].channels[0].id;
    let session = store
        .create_bot(owner.id, "poster", "Poster", &["send_messages".into()])
        .await
        .unwrap();
    store.install_bot(owner.id, home.server.id, session.account.id).await.unwrap();
    assert!(!store.bot_call_is_null(&session.account, "send_messages", Some(home.server.id)).await.unwrap());
    let sent = store
        .send_message(session.account.id, ChatTarget::Channel(channel), "from the bot", None, &[], &[])
        .await
        .unwrap();
    assert_eq!(sent.author_kind, "user");
    assert!(sent.author_id > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_rewrites_a_new_channel_message_to_test() {
    let (store, dir) = open_store().await;
    let owner = store.register("dee", "Dee", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Home").await.unwrap();
    let channel = home.categories[0].channels[0].id;
    let zip = package(&[
        (
            "banter.plugin.json",
            r#"{"id":"welcome","name":"Welcome","version":"1.1.0","banter":"1","client":"none","server_entry":"server/main.luau","permissions":["messages","events"]}"#,
        ),
        (
            "server/main.luau",
            r#"banter.events.on("message_created", function(event)
                banter.messages.edit(event.message.id, "test")
            end)"#,
        ),
    ]);
    store.install_plugin(owner.id, home.server.id, &zip).await.unwrap();
    let sent = store
        .send_message(owner.id, ChatTarget::Channel(channel), "hello", None, &[], &[])
        .await
        .unwrap();
    assert_eq!(sent.body, "hello");
    store.notify_plugin_message(home.server.id, channel, sent.id, &sent.body);
    let page = store.messages(owner.id, ChatTarget::Channel(channel), 10).await.unwrap();
    let rewritten = page.iter().find(|message| message.id == sent.id).unwrap();
    assert_eq!(rewritten.body, "test");
    store.remove_plugin(owner.id, home.server.id, "welcome").await.unwrap();
    assert!(store.plugins(owner.id, home.server.id).await.unwrap().is_empty());
    let next = store
        .send_message(owner.id, ChatTarget::Channel(channel), "still here", None, &[], &[])
        .await
        .unwrap();
    store.notify_plugin_message(home.server.id, channel, next.id, &next.body);
    let page = store.messages(owner.id, ChatTarget::Channel(channel), 10).await.unwrap();
    let kept = page.iter().find(|message| message.id == next.id).unwrap();
    assert_eq!(kept.body, "still here");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_only_plugin_stores_a_value_and_skips_opt_in() {
    let (store, dir) = open_store().await;
    let owner = store.register("cy", "Cy", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Home").await.unwrap();
    let zip = package(&[
        (
            "banter.plugin.json",
            r#"{"id":"notes","name":"Notes","version":"1.0.0","banter":"1","client":"none","server_entry":"server/main.luau","permissions":["storage"]}"#,
        ),
        ("server/main.luau", r#"banter.storage.set("k", "value")"#),
    ]);
    let info = store.install_plugin(owner.id, home.server.id, &zip).await.unwrap();
    assert_eq!(info.client, "none");
    assert!(!info.has_client);
    assert!(info.has_server);
    assert_eq!(store.plugin_value(home.server.id, "notes", "k").await.unwrap().as_deref(), Some("value"));
    assert!(matches!(
        store.opt_in_plugin(owner.id, home.server.id, "notes", "1.0.0").await,
        Err(BanterError::NotFound)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_without_permission_does_not_write() {
    let (store, dir) = open_store().await;
    let owner = store.register("dee", "Dee", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Home").await.unwrap();
    let zip = package(&[
        (
            "banter.plugin.json",
            r#"{"id":"quiet","name":"Quiet","version":"1.0.0","banter":"1","client":"none","server_entry":"server/main.luau","permissions":[]}"#,
        ),
        ("server/main.luau", r#"banter.storage.set("k", "value")"#),
    ]);
    store.install_plugin(owner.id, home.server.id, &zip).await.unwrap();
    assert_eq!(store.plugin_value(home.server.id, "quiet", "k").await.unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}
