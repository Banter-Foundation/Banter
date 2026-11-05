//! Contract for the Banter foundation. These checks were written before the
//! server implementation and describe the behavior the desktop app relies on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use banter_server::{
    BanterError, BoardElement, BoardElementKind, BoardOp, CallPhase, ChannelKind, ChatTarget, ClientSignal, LayerId,
    Live, MediaKind, MediaPlace, Presence, ServerEvent, Store, IDLE_AFTER_MS, MAX_ATTACHMENT_BYTES, MAX_MESSAGE_CHARS,
    sanitize_voice_flags, should_receive_audio, should_send_audio, should_send_camera, should_send_screen,
    should_send_stream_audio, sdp_fixture, sdp_matches_contract, VoiceFlags,
};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("banter-foundation-{}-{n}", std::process::id()));
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
    let image = image::RgbImage::from_pixel(1, 1, image::Rgb([20, 40, 60]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

#[test]
fn voice_flag_rules() {
    let mut flags = VoiceFlags::default();
    assert!(should_send_audio(&flags));
    assert!(should_receive_audio(&flags));
    assert!(!should_send_camera(&flags));
    assert!(!should_send_screen(&flags));

    flags.muted = true;
    assert!(!should_send_audio(&flags));
    flags.muted = false;
    flags.deafened = true;
    assert!(!should_send_audio(&flags));
    assert!(!should_receive_audio(&flags));

    flags.deafened = false;
    flags.push_to_talk = true;
    assert!(!should_send_audio(&flags));
    flags.transmitting = true;
    assert!(should_send_audio(&flags));

    flags.camera = true;
    flags.screen = true;
    assert!(should_send_camera(&flags));
    assert!(should_send_screen(&flags));
    assert!(should_send_stream_audio(&flags));
    flags.screen = false;
    assert!(!should_send_stream_audio(&flags));
    flags.muted = true;
    flags.screen = true;
    assert!(!should_send_audio(&flags));
    assert!(should_send_stream_audio(&flags));
}

#[test]
fn muted_or_deafened_voice_flags_drop_transmitting() {
    let muted = sanitize_voice_flags(VoiceFlags {
        muted: true,
        transmitting: true,
        ..VoiceFlags::default()
    });
    assert!(!muted.transmitting);
    assert!(muted.muted);

    let deafened = sanitize_voice_flags(VoiceFlags {
        muted: true,
        deafened: true,
        transmitting: true,
        ..VoiceFlags::default()
    });
    assert!(!deafened.transmitting);
    assert!(deafened.deafened);

    let open = sanitize_voice_flags(VoiceFlags {
        transmitting: true,
        ..VoiceFlags::default()
    });
    assert!(open.transmitting);
}

#[test]
fn signaling_round_trip_and_sdp_contract() {
    let signal = ClientSignal::SubscribeLayer {
        place: MediaPlace::VoiceChannel(9),
        publisher: 4,
        track: MediaKind::Screen,
        max_layer: LayerId::Quarter,
    };
    let text = serde_json::to_string(&signal).unwrap();
    let parsed: ClientSignal = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, signal);
    assert!(text.contains("\"track\":\"screen\""));
    assert!(text.contains("\"max_layer\":\"quarter\""));

    let offer = ClientSignal::RtcOffer {
        place: MediaPlace::DirectCall(3),
        sdp: sdp_fixture(),
    };
    let text = serde_json::to_string(&offer).unwrap();
    let parsed: ClientSignal = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, offer);
    assert!(sdp_matches_contract(&sdp_fixture()));
    assert_eq!(MediaKind::UserAudio.code(), 1);
    assert_eq!(MediaKind::from_code(4), Some(MediaKind::StreamAudio));
    assert!(serde_json::from_str::<ClientSignal>("nope").is_err());
    let _ = ServerEvent::CallEnded { dm_id: 1 };
}

#[tokio::test]
async fn register_login_avatar_display_name_and_persistence() {
    let dir = scratch();
    let avatar = tiny_png();
    let id = {
        let store = open_store_at(&dir).await;
        assert_eq!(
            store.register("A", "Ada", "password1").await.unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("bad name", "Ada", "password1").await.unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("a..b", "Ada", "password1").await.unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register(".ab", "Ada", "password1").await.unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("ab.", "Ada", "password1").await.unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("ada", "Ada", "short").await.unwrap_err(),
            BanterError::InvalidPassword
        );
        assert_eq!(
            store.register("ada", " ", "password1").await.unwrap_err(),
            BanterError::InvalidDisplayName
        );

        let account = store
            .register("Ada", "  Ada Lovelace  ", "password1").await
            .unwrap();
        assert_eq!(account.username, "ada");
        assert_eq!(account.display_name, "Ada Lovelace");
        assert!(!account.has_avatar);

        assert_eq!(
            store.register("ADA", "Other", "password1").await.unwrap_err(),
            BanterError::UsernameTaken
        );
        assert_eq!(
            store.login("ada", "wrong-password").await.unwrap_err(),
            BanterError::InvalidCredentials
        );
        assert_eq!(
            store.login("missing", "password1").await.unwrap_err(),
            BanterError::InvalidCredentials
        );

        let session = store.login("ADA", "password1").await.unwrap();
        assert_eq!(session.account.id, account.id);
        assert!(!session.token.is_empty());

        let updated = store.set_display_name(account.id, "Ada L.").await.unwrap();
        assert_eq!(updated.display_name, "Ada L.");
        assert_eq!(store.account(account.id).await.unwrap().display_name, "Ada L.");

        assert_eq!(
            store.set_avatar(account.id, b"not an image").await.unwrap_err(),
            BanterError::InvalidAvatar
        );
        store.set_avatar(account.id, &avatar).await.unwrap();
        assert!(store.account(account.id).await.unwrap().has_avatar);
        let stored = store.avatar_bytes(account.id).await.unwrap();
        let decoded = image::load_from_memory(&stored).unwrap();
        assert_eq!(decoded.width(), 1);
        assert_eq!(decoded.height(), 1);
        account.id
    };

    let store = open_store_at(&dir).await;
    let session = store.login("ada", "password1").await.unwrap();
    assert_eq!(session.account.id, id);
    assert_eq!(session.account.display_name, "Ada L.");
    assert!(session.account.has_avatar);
    assert!(!store.avatar_bytes(id).await.unwrap().is_empty());
}

#[tokio::test]
async fn friend_request_accept_decline_and_list() {
    let store = open_store().await.0;
    let ada = store.register("Ada", "Ada", "password1").await.unwrap();
    let bob = store.register("Bob", "Bob", "password1").await.unwrap();
    let cam = store.register("Cam", "Cam", "password1").await.unwrap();

    assert_eq!(
        store.send_friend_request(ada.id, "ada").await.unwrap_err(),
        BanterError::CannotFriendSelf
    );
    assert_eq!(
        store.send_friend_request(ada.id, "nobody").await.unwrap_err(),
        BanterError::NotFound
    );

    let req = store.send_friend_request(ada.id, "Bob").await.unwrap();
    assert_eq!(req.from_id, ada.id);
    assert_eq!(req.to_id, bob.id);
    assert_eq!(req.from_username, "ada");
    assert_eq!(req.to_username, "bob");
    assert_eq!(store.incoming_requests(bob.id).await.unwrap().len(), 1);
    assert_eq!(store.outgoing_requests(ada.id).await.unwrap().len(), 1);
    assert_eq!(
        store.send_friend_request(ada.id, "bob").await.unwrap_err(),
        BanterError::RequestExists
    );
    assert_eq!(
        store.accept_friend_request(ada.id, req.id).await.unwrap_err(),
        BanterError::NotPending
    );

    store.decline_friend_request(bob.id, req.id).await.unwrap();
    assert!(store.incoming_requests(bob.id).await.unwrap().is_empty());
    assert!(store.friends(ada.id).await.unwrap().is_empty());

    let req = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, req.id).await.unwrap();
    let friends = store.friends(ada.id).await.unwrap();
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].id, bob.id);
    assert_eq!(friends[0].username, "bob");
    assert_eq!(store.friends(bob.id).await.unwrap()[0].username, "ada");
    assert_eq!(
        store.send_friend_request(ada.id, "bob").await.unwrap_err(),
        BanterError::AlreadyFriends
    );
    assert_eq!(
        store.accept_friend_request(bob.id, req.id).await.unwrap_err(),
        BanterError::NotPending
    );

    let pending = store.send_friend_request(cam.id, "ada").await.unwrap();
    store.send_friend_request(ada.id, "cam").await.unwrap();
    assert!(store
        .friends(ada.id).await
        .unwrap()
        .iter()
        .any(|friend| friend.username == "cam"));
    assert!(store
        .friends(cam.id).await
        .unwrap()
        .iter()
        .any(|friend| friend.username == "ada"));
    assert!(store.incoming_requests(ada.id).await.unwrap().is_empty());
    assert!(store
        .outgoing_requests(cam.id).await
        .unwrap()
        .iter()
        .all(|request| request.id != pending.id));
}

#[tokio::test]
async fn server_invite_members_nickname_and_channels() {
    let dir = scratch();
    let server_id;
    let owner_id;
    {
        let store = open_store_at(&dir).await;
        let owner = store.register("owner", "Owner", "password1").await.unwrap();
        let joiner = store.register("joiner", "Joiner", "password1").await.unwrap();
        owner_id = owner.id;

        assert_eq!(
            store.create_server(owner.id, " ").await.unwrap_err(),
            BanterError::InvalidName
        );
        let home = store.create_server(owner.id, " Night Owls ").await.unwrap();
        server_id = home.server.id;
        assert_eq!(home.server.name, "Night Owls");
        assert_eq!(home.server.owner_id, owner.id);
        assert_eq!(home.categories.len(), 2);
        assert_eq!(home.categories[0].category.name, "Text Channels");
        assert_eq!(home.categories[0].channels.len(), 1);
        assert_eq!(home.categories[0].channels[0].name, "general");
        assert_eq!(home.categories[0].channels[0].kind, ChannelKind::Text);
        assert_eq!(home.categories[1].category.name, "Voice Channels");
        assert_eq!(home.categories[1].channels[0].name, "General");
        assert_eq!(home.categories[1].channels[0].kind, ChannelKind::Voice);

        let code = store.create_invite(owner.id, server_id).await.unwrap();
        assert_eq!(
            store.join_invite(joiner.id, "nope").await.unwrap_err(),
            BanterError::InvalidInvite
        );
        let joined = store.join_invite(joiner.id, &code).await.unwrap();
        assert_eq!(joined.server.id, server_id);
        assert_eq!(
            store.join_invite(joiner.id, &code).await.unwrap_err(),
            BanterError::AlreadyMember
        );

        let members = store.members(owner.id, server_id).await.unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|member| member.user_id == owner.id && member.is_owner));
        assert!(members
            .iter()
            .any(|member| member.user_id == joiner.id && !member.is_owner));

        store
            .set_nickname(joiner.id, server_id, Some("  JoinerNick ")).await
            .unwrap();
        let joiner_row = store
            .members(owner.id, server_id).await
            .unwrap()
            .into_iter()
            .find(|member| member.user_id == joiner.id)
            .unwrap();
        assert_eq!(joiner_row.nickname.as_deref(), Some("JoinerNick"));
        store.set_nickname(joiner.id, server_id, Some("")).await.unwrap();
        let joiner_row = store
            .members(owner.id, server_id).await
            .unwrap()
            .into_iter()
            .find(|member| member.user_id == joiner.id)
            .unwrap();
        assert_eq!(joiner_row.nickname, None);

        assert_eq!(
            store
                .create_category(joiner.id, server_id, "Notes").await
                .unwrap_err(),
            BanterError::NotOwner
        );
        let notes = store.create_category(owner.id, server_id, "Notes").await.unwrap();
        let announcements = store
            .create_channel(
                owner.id,
                server_id,
                notes.id,
                "announcements",
                ChannelKind::Text,
                None,
            ).await
            .unwrap();
        assert_eq!(announcements.kind, ChannelKind::Text);
        assert_eq!(announcements.category_id, notes.id);
        let lounge = store
            .create_channel(owner.id, server_id, notes.id, "Lounge", ChannelKind::Voice, None).await
            .unwrap();
        assert_eq!(lounge.kind, ChannelKind::Voice);
        let sketch = store
            .create_channel(owner.id, server_id, notes.id, "Sketch", ChannelKind::Board, None).await
            .unwrap();
        assert_eq!(sketch.kind, ChannelKind::Board);
        let scene = store.board_scene(joiner.id, sketch.id).await.unwrap();
        assert_eq!(scene.revision, 0);
        assert!(scene.elements.is_empty());

        let layout = store.layout(joiner.id, server_id).await.unwrap();
        assert!(layout.iter().any(|category| category.category.name == "Notes"
            && category
                .channels
                .iter()
                .any(|channel| channel.name == "announcements")));
        assert_eq!(
            store.layout(999, server_id).await.unwrap_err(),
            BanterError::NotMember
        );
    }

    let store = open_store_at(&dir).await;
    let servers = store.servers(owner_id).await.unwrap();
    assert!(servers.iter().any(|server| server.id == server_id && server.name == "Night Owls"));
    assert_eq!(store.layout(owner_id, server_id).await.unwrap().len(), 3);
}

#[tokio::test]
async fn messages_reply_edit_delete_attachment_search_mentions() {
    let dir = scratch();
    let store = open_store_at(&dir).await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let stranger = store.register("stranger", "Stranger", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Owls").await.unwrap();
    let server_id = home.server.id;
    let text_id = home.categories[0].channels[0].id;
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, server_id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();
    store
        .set_nickname(bob.id, server_id, Some("Bobby")).await
        .unwrap();

    assert_eq!(
        store
            .send_message(stranger.id, ChatTarget::Channel(text_id), "nope", None, &[]).await
            .unwrap_err(),
        BanterError::NotMember
    );
    assert_eq!(
        store
            .send_message(ada.id, ChatTarget::Channel(voice_id), "talk", None, &[]).await
            .unwrap_err(),
        BanterError::NotTextChannel
    );
    assert_eq!(
        store
            .send_message(ada.id, ChatTarget::Channel(text_id), "   ", None, &[]).await
            .unwrap_err(),
        BanterError::EmptyMessage
    );
    assert_eq!(
        store
            .send_message(
                ada.id,
                ChatTarget::Channel(text_id),
                &"a".repeat(MAX_MESSAGE_CHARS + 1),
                None,
                &[],
            ).await
            .unwrap_err(),
        BanterError::MessageTooLong
    );

    let first = store
        .send_message(ada.id, ChatTarget::Channel(text_id), "one", None, &[]).await
        .unwrap();
    let second = store
        .send_message(
            bob.id,
            ChatTarget::Channel(text_id),
            "two",
            Some(first.id),
            &[],
        ).await
        .unwrap();
    assert_eq!(second.reply_to, Some(first.id));
    assert_eq!(second.author_username, "bob");
    assert_eq!(second.author_nickname.as_deref(), Some("Bobby"));
    let third = store
        .send_message(ada.id, ChatTarget::Channel(text_id), "three", None, &[]).await
        .unwrap();
    let page = store
        .messages(bob.id, ChatTarget::Channel(text_id), 2).await
        .unwrap();
    assert_eq!(
        page.iter().map(|message| message.id).collect::<Vec<_>>(),
        vec![second.id, third.id]
    );

    let other = store.create_category(ada.id, server_id, "Other").await.unwrap();
    let other_channel = store
        .create_channel(ada.id, server_id, other.id, "side", ChannelKind::Text, None).await
        .unwrap();
    let elsewhere = store
        .send_message(
            ada.id,
            ChatTarget::Channel(other_channel.id),
            "elsewhere",
            None,
            &[],
        ).await
        .unwrap();
    assert_eq!(
        store
            .send_message(
                ada.id,
                ChatTarget::Channel(text_id),
                "bad reply",
                Some(elsewhere.id),
                &[],
            ).await
            .unwrap_err(),
        BanterError::ReplyTargetMissing
    );

    let edited = store.edit_message(ada.id, first.id, "changed").await.unwrap();
    assert_eq!(edited.body, "changed");
    assert!(edited.edited_at.is_some());
    assert_eq!(
        store.edit_message(bob.id, first.id, "no").await.unwrap_err(),
        BanterError::NotAuthor
    );

    let with_file = store
        .send_message(
            ada.id,
            ChatTarget::Channel(text_id),
            "",
            None,
            &[("../secret.txt", b"abc")],
        ).await
        .unwrap();
    assert_eq!(with_file.body, "");
    assert_eq!(with_file.attachments.len(), 1);
    assert_eq!(with_file.attachments[0].filename, "secret.txt");
    let (name, bytes) = store
        .read_attachment(bob.id, with_file.attachments[0].id).await
        .unwrap();
    assert_eq!(name, "secret.txt");
    assert_eq!(bytes, b"abc");
    assert_eq!(
        store
            .read_attachment(stranger.id, with_file.attachments[0].id).await
            .unwrap_err(),
        BanterError::NotMember
    );
    let big = vec![0_u8; MAX_ATTACHMENT_BYTES + 1];
    assert_eq!(
        store
            .send_message(
                ada.id,
                ChatTarget::Channel(text_id),
                "big",
                None,
                &[("a.bin", big.as_slice())],
            ).await
            .unwrap_err(),
        BanterError::FileTooLarge
    );

    store
        .send_message(
            ada.id,
            ChatTarget::Channel(text_id),
            "Hello there",
            None,
            &[],
        ).await
        .unwrap();
    store
        .send_message(ada.id, ChatTarget::Channel(text_id), "other", None, &[]).await
        .unwrap();
    let found = store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "hello").await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].body, "Hello there");
    let side = store
        .search_messages(ada.id, ChatTarget::Channel(other_channel.id), "hello").await
        .unwrap();
    assert!(side.is_empty());
    let recent = store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "  ").await
        .unwrap();
    let listed = store
        .messages(bob.id, ChatTarget::Channel(text_id), 100).await
        .unwrap();
    assert_eq!(
        recent.iter().map(|message| message.id).collect::<Vec<_>>(),
        listed.iter().map(|message| message.id).collect::<Vec<_>>()
    );

    let mention = store
        .send_message(
            ada.id,
            ChatTarget::Channel(text_id),
            "hi @bob and @bob @stranger @ada",
            None,
            &[],
        ).await
        .unwrap();
    let unread = store.unread_mentions(bob.id).await.unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].message_id, mention.id);
    assert_eq!(unread[0].channel_id, Some(text_id));
    assert_eq!(unread[0].dm_id, None);
    assert_eq!(unread[0].from_username, "ada");
    assert!(store.unread_mentions(ada.id).await.unwrap().is_empty());
    assert!(store.unread_mentions(stranger.id).await.unwrap().is_empty());
    store.edit_message(ada.id, mention.id, "hi").await.unwrap();
    assert!(store.unread_mentions(bob.id).await.unwrap().is_empty());
    store
        .edit_message(ada.id, mention.id, "hi @bob again").await
        .unwrap();
    let unread = store.unread_mentions(bob.id).await.unwrap();
    assert_eq!(unread.len(), 1);
    store.mark_mention_read(bob.id, unread[0].id).await.unwrap();
    assert!(store.unread_mentions(bob.id).await.unwrap().is_empty());

    store.delete_message(ada.id, first.id).await.unwrap();
    assert_eq!(
        store.delete_message(bob.id, third.id).await.unwrap_err(),
        BanterError::NotAuthor
    );
    let remaining = store
        .messages(bob.id, ChatTarget::Channel(text_id), 100).await
        .unwrap();
    assert!(remaining.iter().all(|message| message.id != first.id));
    let second_now = remaining
        .iter()
        .find(|message| message.id == second.id)
        .unwrap();
    assert_eq!(second_now.reply_to, None);
    assert!(store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "changed").await
        .unwrap()
        .is_empty());
    store.delete_message(ada.id, with_file.id).await.unwrap();
    assert_eq!(
        store
            .read_attachment(bob.id, with_file.attachments[0].id).await
            .unwrap_err(),
        BanterError::NotFound
    );

    drop(store);
    let store = open_store_at(&dir).await;
    let kept = store
        .messages(ada.id, ChatTarget::Channel(text_id), 100).await
        .unwrap();
    assert!(kept.iter().any(|message| message.body == "two"));
    assert!(kept.iter().any(|message| message.body == "three"));
}

#[tokio::test]
async fn messages_persist_nested_reply_threads() {
    let dir = scratch();
    let store = open_store_at(&dir).await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Threads").await.unwrap();
    let text_id = home.categories[0].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();

    let root = store
        .send_message(ada.id, ChatTarget::Channel(text_id), "root", None, &[])
        .await
        .unwrap();
    let child = store
        .send_message(
            bob.id,
            ChatTarget::Channel(text_id),
            "child",
            Some(root.id),
            &[],
        )
        .await
        .unwrap();
    let grand = store
        .send_message(
            ada.id,
            ChatTarget::Channel(text_id),
            "grand",
            Some(child.id),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(child.reply_to, Some(root.id));
    assert_eq!(grand.reply_to, Some(child.id));

    let request = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, request.id).await.unwrap();
    let dm = store.open_dm(ada.id, bob.id).await.unwrap();
    let dm_root = store
        .send_message(ada.id, ChatTarget::Direct(dm.id), "dm root", None, &[])
        .await
        .unwrap();
    let dm_reply = store
        .send_message(
            bob.id,
            ChatTarget::Direct(dm.id),
            "dm nested",
            Some(dm_root.id),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(dm_reply.reply_to, Some(dm_root.id));

    drop(store);
    let store = open_store_at(&dir).await;
    let channel = store
        .messages(bob.id, ChatTarget::Channel(text_id), 100)
        .await
        .unwrap();
    let by_id = |id: i64| channel.iter().find(|m| m.id == id).unwrap();
    assert_eq!(by_id(root.id).reply_to, None);
    assert_eq!(by_id(child.id).reply_to, Some(root.id));
    assert_eq!(by_id(grand.id).reply_to, Some(child.id));
    let direct = store
        .messages(ada.id, ChatTarget::Direct(dm.id), 100)
        .await
        .unwrap();
    assert_eq!(
        direct.iter().find(|m| m.id == dm_reply.id).unwrap().reply_to,
        Some(dm_root.id)
    );
}

#[tokio::test]
async fn voice_channel_is_drop_in_and_routes_media() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Voice").await.unwrap();
    let text_id = home.categories[0].channels[0].id;
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();
    let notes = store.create_category(ada.id, home.server.id, "More").await.unwrap();
    let lounge = store
        .create_channel(ada.id, home.server.id, notes.id, "Lounge", ChannelKind::Voice, None).await
        .unwrap();

    let live = Live::new();
    assert_eq!(
        live.join_server_voice(&store, ada.id, text_id).await.unwrap_err(),
        BanterError::NotVoiceChannel
    );
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    live.join_server_voice(&store, bob.id, voice_id).await.unwrap();
    let members = live.voice_channel_members(voice_id);
    assert_eq!(members.len(), 2);

    let routed = live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::UserAudio)
        .unwrap();
    assert_eq!(routed, vec![bob.id]);

    let mut flags = VoiceFlags::default();
    flags.muted = true;
    flags.transmitting = true;
    live.set_flags(ada.id, flags);
    let ada_flags = live
        .voice_channel_members(voice_id)
        .into_iter()
        .find(|member| member.user_id == ada.id)
        .unwrap()
        .flags;
    assert!(ada_flags.muted);
    assert!(!ada_flags.transmitting, "muted users must not publish a talking flag");
    assert!(live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::UserAudio)
        .unwrap()
        .is_empty());

    flags.muted = false;
    flags.transmitting = false;
    flags.push_to_talk = true;
    live.set_flags(ada.id, flags);
    assert!(live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::UserAudio)
        .unwrap()
        .is_empty());
    flags.transmitting = true;
    live.set_flags(ada.id, flags);
    assert_eq!(
        live.media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::UserAudio)
            .unwrap()
            .len(),
        1
    );

    flags = VoiceFlags::default();
    flags.deafened = true;
    live.set_flags(bob.id, flags);
    flags = VoiceFlags {
        camera: true,
        screen: true,
        ..VoiceFlags::default()
    };
    live.set_flags(ada.id, flags);
    assert!(live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::UserAudio)
        .unwrap()
        .is_empty());
    assert!(live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::StreamAudio)
        .unwrap()
        .is_empty());
    let camera = live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Camera)
        .unwrap();
    assert_eq!(camera, vec![bob.id]);
    let screen = live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Screen)
        .unwrap();
    assert_eq!(screen, vec![bob.id]);
    flags.camera = false;
    flags.screen = false;
    live.set_flags(ada.id, flags);
    assert!(live
        .media_subscribers(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Camera)
        .unwrap()
        .is_empty());

    live.join_server_voice(&store, ada.id, lounge.id).await.unwrap();
    assert!(live
        .voice_channel_members(voice_id)
        .iter()
        .all(|member| member.user_id != ada.id));
    assert_eq!(live.voice_channel_members(lounge.id).len(), 1);
    live.leave_voice(ada.id);
    assert!(live.voice_channel_members(lounge.id).is_empty());
}

#[tokio::test]
async fn voice_roster_reaches_members_who_are_not_in_the_channel() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let cam = store.register("cam", "Cam", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Voice").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    let watchers = live.voice_watcher_ids(&store, ada.id, voice_id).await;
    assert!(watchers.contains(&ada.id));
    assert!(watchers.contains(&bob.id));

    let channels = store.voice_channel_ids(bob.id, home.server.id).await.unwrap();
    assert!(channels.contains(&voice_id));
    let members = live.voice_channel_members(voice_id);
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].user_id, ada.id);
    assert!(store.voice_channel_ids(cam.id, home.server.id).await.is_err());

    live.leave_voice(ada.id);
    assert!(live.voice_channel_members(voice_id).is_empty());
    let still = live.voice_watcher_ids(&store, ada.id, voice_id).await;
    assert!(still.contains(&bob.id));
    assert!(still.contains(&ada.id));
}

#[test]
fn sfu_forward_rtp_rewrites_publisher_ssrc_for_write_rtp() {
    // Forward tracks negotiate their own SSRCs. Cloning the publisher packet into
    // write_rtp left the publisher SSRC in place; the sender rejected every packet
    // (ErrSenderWithNoSSRCs / extension mismatch), so channel peers heard silence
    // and never got camera or screen tiles. rewrite_forward_rtp stamps the outbound
    // SSRC/PT and strips inbound header extensions before write_rtp.
    let mut packet = rtc::rtp::Packet::default();
    packet.header.ssrc = banter_server::USER_AUDIO_SSRC;
    packet.header.payload_type = 42;
    packet.header.extension = true;
    packet.header.extensions = vec![rtc::rtp::header::Extension {
        id: 1,
        payload: bytes::Bytes::from_static(&[9]),
    }];
    packet.payload = bytes::Bytes::from_static(&[0xaa]);

    let out = banter_server::rewrite_forward_rtp(packet, 0x8100_0100, banter_server::OPUS_PT);
    assert_eq!(out.header.ssrc, 0x8100_0100);
    assert_eq!(out.header.payload_type, banter_server::OPUS_PT);
    assert!(out.header.extensions.is_empty());
    assert!(!out.header.extension);

    let mut video = rtc::rtp::Packet::default();
    video.header.ssrc = banter_server::CAMERA_SSRC_Q + 2;
    video.header.payload_type = banter_server::H264_PT;
    let out_v = banter_server::rewrite_forward_rtp(video, 0x8100_0200, banter_server::H264_PT);
    assert_eq!(out_v.header.ssrc, 0x8100_0200);
    assert_ne!(out_v.header.ssrc, banter_server::CAMERA_SSRC_Q + 2);
}

#[tokio::test]
async fn voice_channel_audio_forwards_after_pair_subscribe_without_prior_layer_note() {
    // Voice-channel SFU used to require note_layer before admit_rtp; the first
    // (and often every) Opus packet then had an empty publish set and was dropped.
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "VoiceFwd").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    live.join_server_voice(&store, bob.id, voice_id).await.unwrap();
    live.set_flags(
        ada.id,
        VoiceFlags {
            transmitting: true,
            ..VoiceFlags::default()
        },
    );
    live.subscribe_voice_pair(ada.id, bob.id);

    let got = live.admit_rtp(voice_id, ada.id, MediaKind::UserAudio, LayerId::Full, 96, false);
    assert!(
        got.contains(&bob.id),
        "Opus on a voice channel must reach the other member without a prior note_layer, got {got:?}"
    );

    assert_eq!(
        banter_server::inbound_rtp_layer(MediaKind::UserAudio, None),
        Some(LayerId::Full),
        "audio RTP must not wait on an OnOpen SSRC→RID map"
    );
    assert_eq!(banter_server::inbound_rtp_layer(MediaKind::StreamAudio, None), Some(LayerId::Full));
    assert_eq!(
        banter_server::inbound_rtp_layer(MediaKind::Camera, Some("h")),
        Some(LayerId::Half)
    );
    assert_eq!(
        banter_server::inbound_rtp_layer(MediaKind::Screen, Some("q")),
        Some(LayerId::Quarter)
    );
}

#[tokio::test]
async fn voice_channel_screen_forwards_without_onopen_rid_map() {
    // webrtc bind_declared_ssrc fires OnOpen with rid=None for Banter's fixed
    // screen SSRCs. The SFU used to drop every screen packet, so remotes never
    // saw Go Live. Prefer RID, then SCREEN_SSRC_Q offsets, then Full.
    let store = open_store().await.0;
    let ada = store.register("screenada", "ScreenAda", "password1").await.unwrap();
    let bob = store.register("screenbob", "ScreenBob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "ScreenFwd").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    live.join_server_voice(&store, bob.id, voice_id).await.unwrap();
    live.set_flags(
        ada.id,
        VoiceFlags {
            screen: true,
            transmitting: true,
            ..VoiceFlags::default()
        },
    );
    live.subscribe_voice_pair(ada.id, bob.id);

    let got = live.admit_rtp(voice_id, ada.id, MediaKind::Screen, LayerId::Full, 1200, true);
    assert!(
        got.contains(&bob.id),
        "screen RTP on a voice channel must reach the other member after pair subscribe, got {got:?}"
    );

    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Screen, None, 0),
        Some(LayerId::Full),
        "screen must not wait on an OnOpen SSRC→RID map"
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Screen, Some(""), banter_server::SCREEN_SSRC_Q),
        Some(LayerId::Quarter),
        "empty RID still resolves from the Banter screen quarter SSRC"
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(
            MediaKind::Screen,
            None,
            banter_server::SCREEN_SSRC_Q + 1
        ),
        Some(LayerId::Half)
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(
            MediaKind::Screen,
            None,
            banter_server::SCREEN_SSRC_Q + 2
        ),
        Some(LayerId::Full)
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Screen, Some("h"), 0),
        Some(LayerId::Half),
        "a known RID still wins over SSRC guesses"
    );
    assert_eq!(banter_server::inbound_rtp_layer(MediaKind::Screen, None), Some(LayerId::Full));
}

#[tokio::test]
async fn voice_channel_camera_forwards_without_onopen_rid_map() {
    // Same class of bug as Opus/screen: OnOpen often never maps SSRC→RID for camera, so the
    // SFU dropped every H.264 packet and remotes never saw the tile. Prefer RID, then Banter's
    // fixed publish SSRCs, then Full — never wait forever on OnOpen.
    let store = open_store().await.0;
    let ada = store.register("camada", "CamAda", "password1").await.unwrap();
    let bob = store.register("cambob", "CamBob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "CamFwd").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    live.join_server_voice(&store, bob.id, voice_id).await.unwrap();
    live.set_flags(
        ada.id,
        VoiceFlags {
            camera: true,
            ..VoiceFlags::default()
        },
    );
    live.subscribe_voice_pair(ada.id, bob.id);

    let got = live.admit_rtp(voice_id, ada.id, MediaKind::Camera, LayerId::Full, 1200, true);
    assert!(
        got.contains(&bob.id),
        "camera RTP on a voice channel must reach the other member after pair subscribe, got {got:?}"
    );

    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Camera, None, 0),
        Some(LayerId::Full),
        "camera must not wait on an OnOpen SSRC→RID map"
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Camera, Some(""), banter_server::CAMERA_SSRC_Q),
        Some(LayerId::Quarter),
        "empty RID still resolves from the Banter camera quarter SSRC"
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(
            MediaKind::Camera,
            None,
            banter_server::CAMERA_SSRC_Q + 2
        ),
        Some(LayerId::Full)
    );
    assert_eq!(
        banter_server::inbound_rtp_layer_for_packet(MediaKind::Camera, Some("h"), 0),
        Some(LayerId::Half),
        "a known RID still wins over SSRC guesses"
    );
    assert_eq!(banter_server::inbound_rtp_layer(MediaKind::Camera, None), Some(LayerId::Full));
}

#[tokio::test]
async fn direct_message_call() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    assert_eq!(
        store.open_dm(ada.id, bob.id).await.unwrap_err(),
        BanterError::NotFriend
    );
    let request = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, request.id).await.unwrap();
    let chat = store.open_dm(ada.id, bob.id).await.unwrap();
    let again = store.open_dm(bob.id, ada.id).await.unwrap();
    assert_eq!(chat.id, again.id);
    assert_eq!(chat.peer_id, bob.id);
    assert_eq!(again.peer_username, "ada");
    assert_eq!(store.direct_messages(ada.id).await.unwrap().len(), 1);

    let home = store.create_server(ada.id, "Home").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let text_id = home.categories[0].channels[0].id;
    let sent = store
        .send_message(ada.id, ChatTarget::Direct(chat.id), "ping @bob", None, &[]).await
        .unwrap();
    assert_eq!(sent.author_nickname, None);
    assert_eq!(
        store
            .messages(bob.id, ChatTarget::Direct(chat.id), 10).await
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .search_messages(ada.id, ChatTarget::Channel(text_id), "ping").await
        .unwrap()
        .is_empty());
    let mentions = store.unread_mentions(bob.id).await.unwrap();
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].dm_id, Some(chat.id));
    assert_eq!(mentions[0].channel_id, None);

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    let call = live.start_dm_call(&store, ada.id, chat.id).await.unwrap();
    assert_eq!(call.phase, CallPhase::Ringing);
    assert_eq!(call.caller, ada.id);
    assert_eq!(call.callee, bob.id);
    assert!(live.voice_channel_members(voice_id).is_empty());
    assert!(live
        .media_subscribers(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::UserAudio)
        .unwrap()
        .is_empty());

    let active = live.accept_dm_call(&store, bob.id, chat.id).await.unwrap();
    assert_eq!(active.phase, CallPhase::Active);
    let routed = live
        .media_subscribers(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::UserAudio)
        .unwrap();
    assert_eq!(routed, vec![bob.id]);

    let mut flags = VoiceFlags::default();
    flags.deafened = true;
    live.set_flags(bob.id, flags);
    assert!(live
        .media_subscribers(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::UserAudio)
        .unwrap()
        .is_empty());
    flags = VoiceFlags {
        camera: true,
        ..VoiceFlags::default()
    };
    live.set_flags(ada.id, flags);
    assert_eq!(
        live.media_subscribers(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::Camera)
            .unwrap(),
        vec![bob.id]
    );
    live.handle_signal(
        bob.id,
        ClientSignal::SubscribeLayer {
            place: MediaPlace::DirectCall(chat.id),
            publisher: ada.id,
            track: MediaKind::Camera,
            max_layer: LayerId::Half,
        },
    )
    .unwrap();

    live.end_dm_call(ada.id, chat.id).unwrap();
    assert!(live.call(chat.id).is_none());
    let ringing = live.start_dm_call(&store, ada.id, chat.id).await.unwrap();
    assert_eq!(ringing.phase, CallPhase::Ringing);
    live.decline_dm_call(bob.id, chat.id).unwrap();
    assert!(live.call(chat.id).is_none());
    assert_eq!(
        live.decline_dm_call(bob.id, chat.id).unwrap_err(),
        BanterError::NotInCall
    );
}

#[tokio::test]
async fn presence_goes_idle_and_disconnect_leaves_voice() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Home").await.unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let live = Live::new();

    assert_eq!(live.presence(ada.id), Presence::Offline);
    live.connect(ada.id, 0, None);
    assert_eq!(live.presence(ada.id), Presence::Online);
    live.refresh(IDLE_AFTER_MS - 1);
    assert_eq!(live.presence(ada.id), Presence::Online);
    live.refresh(IDLE_AFTER_MS);
    assert_eq!(live.presence(ada.id), Presence::Idle);
    live.note_activity(ada.id, IDLE_AFTER_MS);
    live.refresh(IDLE_AFTER_MS);
    assert_eq!(live.presence(ada.id), Presence::Online);

    live.join_server_voice(&store, ada.id, voice_id).await.unwrap();
    live.disconnect(ada.id);
    assert_eq!(live.presence(ada.id), Presence::Offline);
    assert!(live.voice_channel_members(voice_id).is_empty());
}

#[tokio::test]
async fn drawing_board_ops_assets_and_conflict() {
    let dir = scratch();
    let store = open_store_at(&dir).await;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let home = store.create_server(ada.id, "Owls").await.unwrap();
    let server_id = home.server.id;
    let text_id = home.categories[0].channels[0].id;
    let code = store.create_invite(ada.id, server_id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();
    let category_id = home.categories[0].category.id;
    let board = store
        .create_channel(ada.id, server_id, category_id, "whiteboard", ChannelKind::Board, None).await
        .unwrap();

    assert_eq!(
        store.board_scene(ada.id, text_id).await.unwrap_err(),
        BanterError::NotBoardChannel
    );
    let empty = store.board_scene(bob.id, board.id).await.unwrap();
    assert_eq!(empty.revision, 0);

    let element = BoardElement {
        id: "rect-1".into(),
        kind: BoardElementKind::Rect,
        x: 10.0,
        y: 20.0,
        w: 100.0,
        h: 80.0,
        points: Vec::new(),
        stroke: "#FFFFFF".into(),
        fill: "#5865F255".into(),
        stroke_width: 2.0,
        stroke_style: Default::default(),
        rotation: 0.0,
        z: 0,
        locked: false,
        group_id: None,
        text: None,
        asset_id: None,
        filename: None,
        updated_by: ada.id,
    };
    let op = BoardOp::Upsert {
        base_revision: 0,
        elements: vec![element.clone()],
    };
    let scene = store.apply_board_op(ada.id, board.id, op).await.unwrap();
    assert_eq!(scene.revision, 1);
    assert_eq!(scene.elements.len(), 1);
    assert_eq!(scene.elements[0].id, "rect-1");

    let stale = BoardOp::Upsert {
        base_revision: 0,
        elements: vec![element],
    };
    assert_eq!(
        store.apply_board_op(bob.id, board.id, stale).await.unwrap_err(),
        BanterError::BoardConflict
    );

    let asset = store
        .upload_board_asset(ada.id, board.id, "note.txt", "text/plain", b"hello board")
        .await
        .unwrap();
    assert_eq!(asset.filename, "note.txt");
    let (loaded, bytes) = store.read_board_asset(bob.id, asset.id).await.unwrap();
    assert_eq!(loaded.id, asset.id);
    assert_eq!(bytes, b"hello board");

    let png = tiny_png();
    let image = store
        .upload_board_asset(bob.id, board.id, "dot.png", "image/png", &png)
        .await
        .unwrap();
    let (_, image_bytes) = store.read_board_asset(ada.id, image.id).await.unwrap();
    assert_eq!(image_bytes, png);
}

#[tokio::test]
async fn http_register_and_login_round_trip() {
    let dir = scratch();
    let _ = open_store_at(&dir).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let addr = format!("127.0.0.1:{port}");
    let server_addr = addr.clone();
    let server_dir = dir.clone();
    std::thread::spawn(move || {
        let url = {
            let marker = server_dir.join("pg_schema");
            let schema = std::fs::read_to_string(&marker).unwrap();
            schema_url(&schema)
        };
        let _ = banter_server::serve(&server_addr, &server_dir, &url);
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let health = format!("http://{addr}/v1/health");
    let mut up = false;
    for _ in 0..50 {
        if agent.get(&health).call().is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(up, "server did not open {health}");

    let register = agent
        .post(&format!("http://{addr}/v1/register"))
        .send_json(serde_json::json!({
            "username": "ada",
            "display_name": "Ada",
            "password": "password1"
        }))
        .unwrap();
    assert_eq!(register.status(), 200);
    let body: serde_json::Value = register.into_json().unwrap();
    assert_eq!(body["account"]["username"], "ada");
    assert_eq!(body["account"]["display_name"], "Ada");
    assert_eq!(body["account"]["has_avatar"], false);
    let token = body["token"].as_str().unwrap().to_string();
    assert!(!token.is_empty());

    let duplicate = agent
        .post(&format!("http://{addr}/v1/register"))
        .send_json(serde_json::json!({
            "username": "ada",
            "display_name": "Ada",
            "password": "password1"
        }));
    match duplicate {
        Err(ureq::Error::Status(409, _)) => {}
        other => panic!("expected 409, got {other:?}"),
    }

    let bad = agent.post(&format!("http://{addr}/v1/login")).send_json(serde_json::json!({
        "username": "ada",
        "password": "wrong-password"
    }));
    match bad {
        Err(ureq::Error::Status(401, _)) => {}
        other => panic!("expected 401, got {other:?}"),
    }

    let login = agent
        .post(&format!("http://{addr}/v1/login"))
        .send_json(serde_json::json!({
            "username": "Ada",
            "password": "password1"
        }))
        .unwrap();
    assert_eq!(login.status(), 200);
    let me = agent
        .get(&format!("http://{addr}/v1/me"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .unwrap();
    let me: serde_json::Value = me.into_json().unwrap();
    assert_eq!(me["username"], "ada");
    assert_eq!(me["display_name"], "Ada");
}

fn encode_image(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([20, 40, 60]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image.write_to(&mut cursor, format).unwrap();
    cursor.into_inner()
}

#[tokio::test]
async fn avatar_formats_revision_and_remove() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    assert_eq!(ada.avatar_rev, 0);
    assert!(!ada.has_avatar);

    store.set_avatar(ada.id, &encode_image(2, 2, image::ImageFormat::Jpeg)).await.unwrap();
    let jpeg = store.account(ada.id).await.unwrap();
    assert!(jpeg.has_avatar);
    assert!(jpeg.avatar_rev > 0);
    let stored = store.avatar_bytes(ada.id).await.unwrap();
    let decoded = image::load_from_memory(&stored).unwrap();
    assert_eq!(decoded.width(), 2);
    assert_eq!(decoded.height(), 2);

    store.set_avatar(ada.id, &encode_image(2, 2, image::ImageFormat::Gif)).await.unwrap();
    let gif = store.account(ada.id).await.unwrap();
    assert!(gif.avatar_rev > jpeg.avatar_rev);

    store.set_avatar(ada.id, &encode_image(2, 2, image::ImageFormat::WebP)).await.unwrap();
    let webp = store.account(ada.id).await.unwrap();
    assert!(webp.avatar_rev > gif.avatar_rev);

    let home = store.create_server(ada.id, "Home").await.unwrap();
    let member = store.members(ada.id, home.server.id).await.unwrap().into_iter().find(|member| member.user_id == ada.id).unwrap();
    assert!(member.has_avatar);
    assert_eq!(member.avatar_rev, webp.avatar_rev);

    let req = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, req.id).await.unwrap();
    let friend = store.friends(bob.id).await.unwrap().into_iter().find(|friend| friend.id == ada.id).unwrap();
    assert!(friend.has_avatar);
    assert_eq!(friend.avatar_rev, webp.avatar_rev);
    let chat = store.open_dm(bob.id, ada.id).await.unwrap();
    assert!(chat.peer_has_avatar);
    assert_eq!(chat.peer_avatar_rev, webp.avatar_rev);

    store.clear_avatar(ada.id).await.unwrap();
    let cleared = store.account(ada.id).await.unwrap();
    assert!(!cleared.has_avatar);
    assert_eq!(cleared.avatar_rev, 0);
    assert_eq!(store.avatar_bytes(ada.id).await.unwrap_err(), BanterError::NotFound);
    let member = store.members(ada.id, home.server.id).await.unwrap().into_iter().find(|member| member.user_id == ada.id).unwrap();
    assert!(!member.has_avatar);
    let chat = store.open_dm(bob.id, ada.id).await.unwrap();
    assert!(!chat.peer_has_avatar);
    assert_eq!(chat.peer_avatar_rev, 0);
}

#[tokio::test]
async fn listen_addr_prefers_the_argument_then_env_then_loopback() {
    assert_eq!(
        banter_server::listen_addr(None, None, None),
        "127.0.0.1:3970"
    );
    assert_eq!(
        banter_server::listen_addr(None, Some(""), Some("")),
        "127.0.0.1:3970"
    );
    assert_eq!(
        banter_server::listen_addr(Some(""), None, None),
        "127.0.0.1:3970"
    );
    assert_eq!(
        banter_server::listen_addr(Some("10.1.2.3:9"), Some("0.0.0.0"), Some("4000")),
        "10.1.2.3:9"
    );
    assert_eq!(
        banter_server::listen_addr(None, Some("0.0.0.0"), None),
        "0.0.0.0:3970"
    );
    assert_eq!(
        banter_server::listen_addr(None, None, Some("4000")),
        "0.0.0.0:4000"
    );
    assert_eq!(
        banter_server::listen_addr(None, Some("10.0.0.8"), Some("4000")),
        "10.0.0.8:4000"
    );
}

#[tokio::test]
async fn data_dir_prefers_banter_data_then_platform_home() {
    assert_eq!(
        banter_server::resolve_data_dir(Some("/tmp/banter-data-test"), Path::new("/home/owl/.local/share")),
        PathBuf::from("/tmp/banter-data-test")
    );
    assert_eq!(
        banter_server::resolve_data_dir(Some(""), Path::new("/home/owl/.local/share")),
        PathBuf::from("/home/owl/.local/share").join("Banter")
    );
    assert_eq!(
        banter_server::resolve_data_dir(None, Path::new("AppData").join("Local").as_path()),
        PathBuf::from("AppData").join("Local").join("Banter")
    );
}

#[tokio::test]
async fn user_profile_visible_to_self_friends_and_shared_server() {
    let store = open_store().await.0;
    let ada = store.register("ada", "Ada Lovelace", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let cam = store.register("cam", "Cam", "password1").await.unwrap();
    let stranger = store.register("stranger", "Stranger", "password1").await.unwrap();

    let self_profile = store.user_profile(ada.id, ada.id, None).await.unwrap();
    assert_eq!(self_profile.id, ada.id);
    assert_eq!(self_profile.username, "ada");
    assert_eq!(self_profile.display_name, "Ada Lovelace");
    assert!(!self_profile.has_avatar);
    assert_eq!(self_profile.avatar_rev, 0);
    assert!(self_profile.is_self);
    assert!(!self_profile.is_friend);
    assert_eq!(self_profile.nickname, None);

    assert_eq!(
        store.user_profile(ada.id, bob.id, None).await.unwrap_err(),
        BanterError::NotFound
    );
    assert_eq!(
        store.user_profile(ada.id, stranger.id, None).await.unwrap_err(),
        BanterError::NotFound
    );

    let req = store.send_friend_request(ada.id, "bob").await.unwrap();
    store.accept_friend_request(bob.id, req.id).await.unwrap();
    let friend = store.user_profile(ada.id, bob.id, None).await.unwrap();
    assert_eq!(friend.id, bob.id);
    assert_eq!(friend.username, "bob");
    assert_eq!(friend.display_name, "Bob");
    assert!(!friend.is_self);
    assert!(friend.is_friend);
    assert_eq!(friend.nickname, None);
    assert!(store.user_profile(bob.id, ada.id, None).await.unwrap().is_friend);

    let home = store.create_server(ada.id, "Owls").await.unwrap();
    let server_id = home.server.id;
    let code = store.create_invite(ada.id, server_id).await.unwrap();
    store.join_invite(cam.id, &code).await.unwrap();
    store.set_nickname(cam.id, server_id, Some("Night Owl")).await.unwrap();

    let member = store.user_profile(ada.id, cam.id, None).await.unwrap();
    assert_eq!(member.id, cam.id);
    assert!(!member.is_friend);
    assert!(!member.is_self);
    assert_eq!(member.nickname, None);

    let nicknamed = store.user_profile(ada.id, cam.id, Some(server_id)).await.unwrap();
    assert_eq!(nicknamed.nickname.as_deref(), Some("Night Owl"));
    assert_eq!(nicknamed.display_name, "Cam");

    assert_eq!(
        store.user_profile(stranger.id, cam.id, Some(server_id)).await.unwrap_err(),
        BanterError::NotMember
    );
    assert_eq!(
        store.user_profile(ada.id, stranger.id, Some(server_id)).await.unwrap_err(),
        BanterError::NotFound
    );
    assert_eq!(
        store.user_profile(ada.id, cam.id, Some(server_id + 99)).await.unwrap_err(),
        BanterError::NotMember
    );
}

#[tokio::test]
async fn debug_table_counts_include_new_rows() {
    let (store, _dir) = open_store().await;
    let before = store.table_counts().await.unwrap();
    let ada = store.register("dbgada", "Ada", "password1").await.unwrap();
    store.create_server(ada.id, "Debug").await.unwrap();
    let (users, servers, channels, messages) = store.table_counts().await.unwrap();
    assert_eq!(users, before.0 + 1);
    assert_eq!(servers, before.1 + 1);
    assert!(channels >= before.2 + 2);
    assert_eq!(messages, before.3);
    let (size, idle) = store.pool_usage();
    assert!(size >= 1);
    assert!(idle <= size);
}

#[tokio::test]
async fn guild_roles_gate_channels_and_stay_below_the_actor() {
    use banter_server::{
        PERM_ADMINISTRATOR, PERM_CREATE_INVITE, PERM_MANAGE_CHANNELS, PERM_MANAGE_ROLES,
    };

    let (store, _dir) = open_store().await;
    let owner = store.register("roleowner", "Owner", "password1").await.unwrap();
    let joiner = store.register("rolejoiner", "Joiner", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Roles").await.unwrap();
    let server_id = home.server.id;
    let category_id = home.categories[0].category.id;

    let roles = store.roles(owner.id, server_id).await.unwrap();
    assert_eq!(roles.len(), 1);
    assert!(roles[0].everyone);
    assert_eq!(roles[0].name, "@everyone");
    assert_eq!(roles[0].permissions, PERM_CREATE_INVITE);
    assert_eq!(roles[0].position, 0);

    let code = store.create_invite(owner.id, server_id).await.unwrap();
    store.join_invite(joiner.id, &code).await.unwrap();
    assert_eq!(
        store
            .create_channel(joiner.id, server_id, category_id, "secret", ChannelKind::Text, None)
            .await
            .unwrap_err(),
        BanterError::NotOwner
    );
    assert_eq!(
        store.delete_role(owner.id, server_id, roles[0].id).await.unwrap_err(),
        BanterError::InvalidName
    );

    let mod_role = store
        .create_role(owner.id, server_id, "Mod", 0x5865F2, true, PERM_MANAGE_CHANNELS)
        .await
        .unwrap();
    assert!(mod_role.position > 0);
    store
        .set_member_roles(owner.id, server_id, joiner.id, &[mod_role.id])
        .await
        .unwrap();
    let created = store
        .create_channel(joiner.id, server_id, category_id, "mod-chat", ChannelKind::Text, None)
        .await
        .unwrap();
    assert_eq!(created.name, "mod-chat");

    assert_eq!(
        store
            .update_role(joiner.id, server_id, mod_role.id, "Nope", 0, false, 0)
            .await
            .unwrap_err(),
        BanterError::NotOwner
    );
    store
        .update_role(
            owner.id,
            server_id,
            mod_role.id,
            "Mod",
            0x5865F2,
            true,
            PERM_MANAGE_CHANNELS | PERM_MANAGE_ROLES,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .update_role(joiner.id, server_id, mod_role.id, "Nope", 0, false, 0)
            .await
            .unwrap_err(),
        BanterError::RoleHierarchy
    );
    assert_eq!(
        store
            .create_role(joiner.id, server_id, "Admin", 0, false, PERM_ADMINISTRATOR)
            .await
            .unwrap_err(),
        BanterError::RoleHierarchy
    );

    let everyone = store
        .update_role(joiner.id, server_id, roles[0].id, "renamed", 1, true, PERM_CREATE_INVITE)
        .await
        .unwrap();
    assert_eq!(everyone.name, "@everyone");
    assert!(!everyone.hoist);
    assert_eq!(everyone.color, 1);

    let members = store.members(owner.id, server_id).await.unwrap();
    let joiner_row = members.iter().find(|member| member.user_id == joiner.id).unwrap();
    assert!(joiner_row.roles.iter().any(|role| role.id == mod_role.id));
    assert!(joiner_row.roles.iter().any(|role| role.everyone));

    let profile = store.user_profile(owner.id, joiner.id, Some(server_id)).await.unwrap();
    assert!(profile.roles.iter().any(|role| role.name == "Mod"));
}

#[tokio::test]
async fn server_folders_nest_hex_color_collapse_and_delete() {
    use banter_server::{MAX_SERVER_FOLDER_DEPTH, ServerFolderPatch};

    let store = open_store().await.0;
    let ada = store.register("ada", "Ada", "password1").await.unwrap();
    let bob = store.register("bob", "Bob", "password1").await.unwrap();
    let alpha = store.create_server(ada.id, "Alpha").await.unwrap().server;
    let beta = store.create_server(ada.id, "Beta").await.unwrap().server;
    let gamma = store.create_server(ada.id, "Gamma").await.unwrap().server;

    let rail = store.server_rail(ada.id).await.unwrap();
    assert!(rail.folders.is_empty());
    assert_eq!(rail.servers.len(), 3);
    assert!(rail.servers.iter().all(|row| row.folder_id.is_none()));
    assert!(rail.servers.windows(2).all(|pair| pair[0].position <= pair[1].position));

    assert_eq!(
        store
            .create_server_folder(ada.id, " ", None, Some(0x5865F2))
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );
    assert_eq!(
        store
            .create_server_folder(ada.id, "Games", None, Some(0x0100_0000))
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );

    let games = store
        .create_server_folder(ada.id, " Games ", None, Some(0x5865F2))
        .await
        .unwrap();
    assert_eq!(games.name, "Games");
    assert_eq!(games.color, 0x5865F2);
    assert_eq!(games.parent_id, None);
    assert!(!games.collapsed);

    store
        .move_server_to_folder(ada.id, alpha.id, Some(games.id), 0)
        .await
        .unwrap();
    store
        .move_server_to_folder(ada.id, beta.id, Some(games.id), 1)
        .await
        .unwrap();

    let fps = store
        .create_server_folder(ada.id, "FPS", Some(games.id), Some(0xED4245))
        .await
        .unwrap();
    assert_eq!(fps.parent_id, Some(games.id));
    assert_eq!(fps.color, 0xED4245);
    store
        .move_server_to_folder(ada.id, gamma.id, Some(fps.id), 0)
        .await
        .unwrap();

    // Cannot parent a folder under itself or a descendant.
    assert_eq!(
        store
            .update_server_folder(
                ada.id,
                games.id,
                ServerFolderPatch {
                    parent_id: Some(Some(fps.id)),
                    ..ServerFolderPatch::default()
                },
            )
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );
    assert_eq!(
        store
            .update_server_folder(
                ada.id,
                games.id,
                ServerFolderPatch {
                    parent_id: Some(Some(games.id)),
                    ..ServerFolderPatch::default()
                },
            )
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );

    // Depth limit: a fresh root nested to MAX_SERVER_FOLDER_DEPTH, then one more fails.
    let deep_root = store
        .create_server_folder(ada.id, "DeepRoot", None, Some(0))
        .await
        .unwrap();
    let mut parent = deep_root.id;
    let mut deepest = deep_root.id;
    for level in 2..=MAX_SERVER_FOLDER_DEPTH {
        let child = store
            .create_server_folder(ada.id, &format!("L{level}"), Some(parent), Some(0))
            .await
            .unwrap();
        deepest = child.id;
        parent = child.id;
    }
    assert_eq!(
        store
            .create_server_folder(ada.id, "TooDeep", Some(deepest), Some(0))
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );

    let collapsed = store
        .update_server_folder(
            ada.id,
            games.id,
            ServerFolderPatch {
                collapsed: Some(true),
                name: Some(" Play ".into()),
                color: Some(0x57F287),
                ..ServerFolderPatch::default()
            },
        )
        .await
        .unwrap();
    assert!(collapsed.collapsed);
    assert_eq!(collapsed.name, "Play");
    assert_eq!(collapsed.color, 0x57F287);

    // Clear color back to default (0).
    let cleared = store
        .update_server_folder(
            ada.id,
            games.id,
            ServerFolderPatch {
                color: Some(0),
                ..ServerFolderPatch::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.color, 0);

    let rail = store.server_rail(ada.id).await.unwrap();
    let games_row = rail.folders.iter().find(|folder| folder.id == games.id).unwrap();
    assert!(games_row.collapsed);
    assert_eq!(
        rail.servers
            .iter()
            .find(|row| row.server.id == alpha.id)
            .unwrap()
            .folder_id,
        Some(games.id)
    );
    assert_eq!(
        rail.servers
            .iter()
            .find(|row| row.server.id == gamma.id)
            .unwrap()
            .folder_id,
        Some(fps.id)
    );

    // Delete games: direct child folders/servers promote to top-level; deeper nesting stays.
    store.delete_server_folder(ada.id, games.id).await.unwrap();
    let rail = store.server_rail(ada.id).await.unwrap();
    assert!(!rail.folders.iter().any(|folder| folder.id == games.id));
    let fps_row = rail.folders.iter().find(|folder| folder.id == fps.id).unwrap();
    assert_eq!(fps_row.parent_id, None);
    assert_eq!(
        rail.servers
            .iter()
            .find(|row| row.server.id == alpha.id)
            .unwrap()
            .folder_id,
        None
    );
    assert_eq!(
        rail.servers
            .iter()
            .find(|row| row.server.id == beta.id)
            .unwrap()
            .folder_id,
        None
    );
    assert_eq!(
        rail.servers
            .iter()
            .find(|row| row.server.id == gamma.id)
            .unwrap()
            .folder_id,
        Some(fps.id)
    );

    // Folders are private to the signed-in user.
    assert!(store.server_rail(bob.id).await.unwrap().folders.is_empty());
    assert_eq!(
        store
            .update_server_folder(bob.id, fps.id, ServerFolderPatch::default())
            .await
            .unwrap_err(),
        BanterError::NotFound
    );
    assert_eq!(
        store.delete_server_folder(bob.id, fps.id).await.unwrap_err(),
        BanterError::NotFound
    );

    // Joining a server places it on that user's top-level rail.
    let code = store.create_invite(ada.id, alpha.id).await.unwrap();
    store.join_invite(bob.id, &code).await.unwrap();
    let bob_rail = store.server_rail(bob.id).await.unwrap();
    assert_eq!(bob_rail.servers.len(), 1);
    assert_eq!(bob_rail.servers[0].server.id, alpha.id);
    assert_eq!(bob_rail.servers[0].folder_id, None);
}

#[tokio::test]
async fn channels_can_set_and_clear_an_optional_lucide_icon() {
    let (store, _dir) = open_store().await;
    let owner = store.register("iconowner", "Owner", "password1").await.unwrap();
    let home = store.create_server(owner.id, "Icons").await.unwrap();
    let server_id = home.server.id;
    let category_id = home.categories[0].category.id;

    let plain = store
        .create_channel(
            owner.id,
            server_id,
            category_id,
            "plain",
            ChannelKind::Text,
            None,
        )
        .await
        .unwrap();
    assert_eq!(plain.icon, None);

    let themed = store
        .create_channel(
            owner.id,
            server_id,
            category_id,
            "games",
            ChannelKind::Voice,
            Some("mic"),
        )
        .await
        .unwrap();
    assert_eq!(themed.icon.as_deref(), Some("mic"));

    assert_eq!(
        store
            .create_channel(
                owner.id,
                server_id,
                category_id,
                "bad",
                ChannelKind::Text,
                Some("not-a-real-icon"),
            )
            .await
            .unwrap_err(),
        BanterError::InvalidName
    );

    let cleared = store
        .update_channel(owner.id, server_id, themed.id, "team", None)
        .await
        .unwrap();
    assert_eq!(cleared.name, "team");
    assert_eq!(cleared.icon, None);

    let renamed = store
        .update_channel(owner.id, server_id, themed.id, "squad", Some("users"))
        .await
        .unwrap();
    assert_eq!(renamed.name, "squad");
    assert_eq!(renamed.icon.as_deref(), Some("users"));

    let layout = store.layout(owner.id, server_id).await.unwrap();
    let found = layout
        .iter()
        .flat_map(|category| category.channels.iter())
        .find(|channel| channel.id == themed.id)
        .expect("updated channel in layout");
    assert_eq!(found.icon.as_deref(), Some("users"));
    assert_eq!(found.name, "squad");

    let stranger = store.register("iconstranger", "Stranger", "password1").await.unwrap();
    assert_eq!(
        store
            .update_channel(stranger.id, server_id, themed.id, "nope", Some("mic"))
            .await
            .unwrap_err(),
        BanterError::NotMember
    );
}

#[test]
fn channel_icon_choices_are_known_lucide_names() {
    assert!(banter_server::CHANNEL_ICON_CHOICES.contains(&"mic"));
    assert!(banter_server::CHANNEL_ICON_CHOICES.contains(&"users"));
    assert!(banter_server::is_channel_icon("hash"));
    assert!(!banter_server::is_channel_icon("not-a-real-icon"));
    assert_eq!(
        banter_server::normalize_channel_icon(Some("  mic  ")).unwrap().as_deref(),
        Some("mic")
    );
    assert_eq!(banter_server::normalize_channel_icon(Some("")).unwrap(), None);
    assert_eq!(banter_server::normalize_channel_icon(None).unwrap(), None);
    assert_eq!(
        banter_server::normalize_channel_icon(Some("nope")).unwrap_err(),
        BanterError::InvalidName
    );
}
