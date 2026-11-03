//! Contract for the Banter foundation. These checks were written before the
//! server implementation and describe the behavior the desktop app relies on.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use banter_server::{
    BanterError, CallPhase, ChannelKind, ChatTarget, Live, MediaKind, MediaPlace, Presence, Store,
    IDLE_AFTER_MS, MAX_ATTACHMENT_BYTES, MAX_MESSAGE_CHARS, decode_client_media,
    decode_media_frame, encode_client_media, encode_media_frame, should_receive_audio,
    should_send_audio, should_send_camera, should_send_screen, VoiceFlags,
};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("banter-foundation-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
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
}

#[test]
fn media_frame_round_trip() {
    let frame = encode_media_frame(MediaKind::Audio, MediaPlace::VoiceChannel(9), 4, b"hi");
    let parsed = decode_media_frame(&frame).unwrap();
    assert_eq!(parsed.kind, MediaKind::Audio);
    assert_eq!(parsed.place, MediaPlace::VoiceChannel(9));
    assert_eq!(parsed.sender_id, 4);
    assert_eq!(parsed.payload, b"hi");

    let client = encode_client_media(MediaKind::Camera, b"jpg");
    let (kind, payload) = decode_client_media(&client).unwrap();
    assert_eq!(kind, MediaKind::Camera);
    assert_eq!(payload, b"jpg");

    let screen = encode_media_frame(MediaKind::Screen, MediaPlace::DirectCall(3), 8, b"");
    let parsed = decode_media_frame(&screen).unwrap();
    assert_eq!(parsed.kind, MediaKind::Screen);
    assert_eq!(parsed.place, MediaPlace::DirectCall(3));
    assert_eq!(parsed.sender_id, 8);
    assert!(parsed.payload.is_empty());

    assert_eq!(decode_media_frame(&[1]).unwrap_err(), BanterError::BadFrame);
    assert_eq!(decode_client_media(&[9]).unwrap_err(), BanterError::BadFrame);
}

#[test]
fn register_login_avatar_display_name_and_persistence() {
    let dir = scratch();
    let avatar = tiny_png();
    let id = {
        let store = Store::open(&dir).unwrap();
        assert_eq!(
            store.register("A", "Ada", "password1").unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("bad name", "Ada", "password1").unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("a..b", "Ada", "password1").unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register(".ab", "Ada", "password1").unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("ab.", "Ada", "password1").unwrap_err(),
            BanterError::InvalidUsername
        );
        assert_eq!(
            store.register("ada", "Ada", "short").unwrap_err(),
            BanterError::InvalidPassword
        );
        assert_eq!(
            store.register("ada", " ", "password1").unwrap_err(),
            BanterError::InvalidDisplayName
        );

        let account = store
            .register("Ada", "  Ada Lovelace  ", "password1")
            .unwrap();
        assert_eq!(account.username, "ada");
        assert_eq!(account.display_name, "Ada Lovelace");
        assert!(!account.has_avatar);

        assert_eq!(
            store.register("ADA", "Other", "password1").unwrap_err(),
            BanterError::UsernameTaken
        );
        assert_eq!(
            store.login("ada", "wrong-password").unwrap_err(),
            BanterError::InvalidCredentials
        );
        assert_eq!(
            store.login("missing", "password1").unwrap_err(),
            BanterError::InvalidCredentials
        );

        let session = store.login("ADA", "password1").unwrap();
        assert_eq!(session.account.id, account.id);
        assert!(!session.token.is_empty());

        let updated = store.set_display_name(account.id, "Ada L.").unwrap();
        assert_eq!(updated.display_name, "Ada L.");
        assert_eq!(store.account(account.id).unwrap().display_name, "Ada L.");

        assert_eq!(
            store.set_avatar(account.id, b"not an image").unwrap_err(),
            BanterError::InvalidAvatar
        );
        store.set_avatar(account.id, &avatar).unwrap();
        assert!(store.account(account.id).unwrap().has_avatar);
        let stored = store.avatar_bytes(account.id).unwrap();
        let decoded = image::load_from_memory(&stored).unwrap();
        assert_eq!(decoded.width(), 1);
        assert_eq!(decoded.height(), 1);
        account.id
    };

    let store = Store::open(&dir).unwrap();
    let session = store.login("ada", "password1").unwrap();
    assert_eq!(session.account.id, id);
    assert_eq!(session.account.display_name, "Ada L.");
    assert!(session.account.has_avatar);
    assert!(!store.avatar_bytes(id).unwrap().is_empty());
}

#[test]
fn friend_request_accept_decline_and_list() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("Ada", "Ada", "password1").unwrap();
    let bob = store.register("Bob", "Bob", "password1").unwrap();
    let cam = store.register("Cam", "Cam", "password1").unwrap();

    assert_eq!(
        store.send_friend_request(ada.id, "ada").unwrap_err(),
        BanterError::CannotFriendSelf
    );
    assert_eq!(
        store.send_friend_request(ada.id, "nobody").unwrap_err(),
        BanterError::NotFound
    );

    let req = store.send_friend_request(ada.id, "Bob").unwrap();
    assert_eq!(req.from_id, ada.id);
    assert_eq!(req.to_id, bob.id);
    assert_eq!(req.from_username, "ada");
    assert_eq!(req.to_username, "bob");
    assert_eq!(store.incoming_requests(bob.id).unwrap().len(), 1);
    assert_eq!(store.outgoing_requests(ada.id).unwrap().len(), 1);
    assert_eq!(
        store.send_friend_request(ada.id, "bob").unwrap_err(),
        BanterError::RequestExists
    );
    assert_eq!(
        store.accept_friend_request(ada.id, req.id).unwrap_err(),
        BanterError::NotPending
    );

    store.decline_friend_request(bob.id, req.id).unwrap();
    assert!(store.incoming_requests(bob.id).unwrap().is_empty());
    assert!(store.friends(ada.id).unwrap().is_empty());

    let req = store.send_friend_request(ada.id, "bob").unwrap();
    store.accept_friend_request(bob.id, req.id).unwrap();
    let friends = store.friends(ada.id).unwrap();
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].id, bob.id);
    assert_eq!(friends[0].username, "bob");
    assert_eq!(store.friends(bob.id).unwrap()[0].username, "ada");
    assert_eq!(
        store.send_friend_request(ada.id, "bob").unwrap_err(),
        BanterError::AlreadyFriends
    );
    assert_eq!(
        store.accept_friend_request(bob.id, req.id).unwrap_err(),
        BanterError::NotPending
    );

    let pending = store.send_friend_request(cam.id, "ada").unwrap();
    store.send_friend_request(ada.id, "cam").unwrap();
    assert!(store
        .friends(ada.id)
        .unwrap()
        .iter()
        .any(|friend| friend.username == "cam"));
    assert!(store
        .friends(cam.id)
        .unwrap()
        .iter()
        .any(|friend| friend.username == "ada"));
    assert!(store.incoming_requests(ada.id).unwrap().is_empty());
    assert!(store
        .outgoing_requests(cam.id)
        .unwrap()
        .iter()
        .all(|request| request.id != pending.id));
}

#[test]
fn server_invite_members_nickname_and_channels() {
    let dir = scratch();
    let server_id;
    let owner_id;
    {
        let store = Store::open(&dir).unwrap();
        let owner = store.register("owner", "Owner", "password1").unwrap();
        let joiner = store.register("joiner", "Joiner", "password1").unwrap();
        owner_id = owner.id;

        assert_eq!(
            store.create_server(owner.id, " ").unwrap_err(),
            BanterError::InvalidName
        );
        let home = store.create_server(owner.id, " Night Owls ").unwrap();
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

        let code = store.create_invite(owner.id, server_id).unwrap();
        assert_eq!(
            store.join_invite(joiner.id, "nope").unwrap_err(),
            BanterError::InvalidInvite
        );
        let joined = store.join_invite(joiner.id, &code).unwrap();
        assert_eq!(joined.server.id, server_id);
        assert_eq!(
            store.join_invite(joiner.id, &code).unwrap_err(),
            BanterError::AlreadyMember
        );

        let members = store.members(owner.id, server_id).unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|member| member.user_id == owner.id && member.is_owner));
        assert!(members
            .iter()
            .any(|member| member.user_id == joiner.id && !member.is_owner));

        store
            .set_nickname(joiner.id, server_id, Some("  JoinerNick "))
            .unwrap();
        let joiner_row = store
            .members(owner.id, server_id)
            .unwrap()
            .into_iter()
            .find(|member| member.user_id == joiner.id)
            .unwrap();
        assert_eq!(joiner_row.nickname.as_deref(), Some("JoinerNick"));
        store.set_nickname(joiner.id, server_id, Some("")).unwrap();
        let joiner_row = store
            .members(owner.id, server_id)
            .unwrap()
            .into_iter()
            .find(|member| member.user_id == joiner.id)
            .unwrap();
        assert_eq!(joiner_row.nickname, None);

        assert_eq!(
            store
                .create_category(joiner.id, server_id, "Notes")
                .unwrap_err(),
            BanterError::NotOwner
        );
        let notes = store.create_category(owner.id, server_id, "Notes").unwrap();
        let announcements = store
            .create_channel(
                owner.id,
                server_id,
                notes.id,
                "announcements",
                ChannelKind::Text,
            )
            .unwrap();
        assert_eq!(announcements.kind, ChannelKind::Text);
        assert_eq!(announcements.category_id, notes.id);
        let lounge = store
            .create_channel(owner.id, server_id, notes.id, "Lounge", ChannelKind::Voice)
            .unwrap();
        assert_eq!(lounge.kind, ChannelKind::Voice);

        let layout = store.layout(joiner.id, server_id).unwrap();
        assert!(layout.iter().any(|category| category.category.name == "Notes"
            && category
                .channels
                .iter()
                .any(|channel| channel.name == "announcements")));
        assert_eq!(
            store.layout(999, server_id).unwrap_err(),
            BanterError::NotMember
        );
    }

    let store = Store::open(&dir).unwrap();
    let servers = store.servers(owner_id).unwrap();
    assert!(servers.iter().any(|server| server.id == server_id && server.name == "Night Owls"));
    assert_eq!(store.layout(owner_id, server_id).unwrap().len(), 3);
}

#[test]
fn messages_reply_edit_delete_attachment_search_mentions() {
    let dir = scratch();
    let store = Store::open(&dir).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let bob = store.register("bob", "Bob", "password1").unwrap();
    let stranger = store.register("stranger", "Stranger", "password1").unwrap();
    let home = store.create_server(ada.id, "Owls").unwrap();
    let server_id = home.server.id;
    let text_id = home.categories[0].channels[0].id;
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, server_id).unwrap();
    store.join_invite(bob.id, &code).unwrap();
    store
        .set_nickname(bob.id, server_id, Some("Bobby"))
        .unwrap();

    assert_eq!(
        store
            .send_message(stranger.id, ChatTarget::Channel(text_id), "nope", None, &[])
            .unwrap_err(),
        BanterError::NotMember
    );
    assert_eq!(
        store
            .send_message(ada.id, ChatTarget::Channel(voice_id), "talk", None, &[])
            .unwrap_err(),
        BanterError::NotTextChannel
    );
    assert_eq!(
        store
            .send_message(ada.id, ChatTarget::Channel(text_id), "   ", None, &[])
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
            )
            .unwrap_err(),
        BanterError::MessageTooLong
    );

    let first = store
        .send_message(ada.id, ChatTarget::Channel(text_id), "one", None, &[])
        .unwrap();
    let second = store
        .send_message(
            bob.id,
            ChatTarget::Channel(text_id),
            "two",
            Some(first.id),
            &[],
        )
        .unwrap();
    assert_eq!(second.reply_to, Some(first.id));
    assert_eq!(second.author_username, "bob");
    assert_eq!(second.author_nickname.as_deref(), Some("Bobby"));
    let third = store
        .send_message(ada.id, ChatTarget::Channel(text_id), "three", None, &[])
        .unwrap();
    let page = store
        .messages(bob.id, ChatTarget::Channel(text_id), 2)
        .unwrap();
    assert_eq!(
        page.iter().map(|message| message.id).collect::<Vec<_>>(),
        vec![second.id, third.id]
    );

    let other = store.create_category(ada.id, server_id, "Other").unwrap();
    let other_channel = store
        .create_channel(ada.id, server_id, other.id, "side", ChannelKind::Text)
        .unwrap();
    let elsewhere = store
        .send_message(
            ada.id,
            ChatTarget::Channel(other_channel.id),
            "elsewhere",
            None,
            &[],
        )
        .unwrap();
    assert_eq!(
        store
            .send_message(
                ada.id,
                ChatTarget::Channel(text_id),
                "bad reply",
                Some(elsewhere.id),
                &[],
            )
            .unwrap_err(),
        BanterError::ReplyTargetMissing
    );

    let edited = store.edit_message(ada.id, first.id, "changed").unwrap();
    assert_eq!(edited.body, "changed");
    assert!(edited.edited_at.is_some());
    assert_eq!(
        store.edit_message(bob.id, first.id, "no").unwrap_err(),
        BanterError::NotAuthor
    );

    let with_file = store
        .send_message(
            ada.id,
            ChatTarget::Channel(text_id),
            "",
            None,
            &[("../secret.txt", b"abc")],
        )
        .unwrap();
    assert_eq!(with_file.body, "");
    assert_eq!(with_file.attachments.len(), 1);
    assert_eq!(with_file.attachments[0].filename, "secret.txt");
    let (name, bytes) = store
        .read_attachment(bob.id, with_file.attachments[0].id)
        .unwrap();
    assert_eq!(name, "secret.txt");
    assert_eq!(bytes, b"abc");
    assert_eq!(
        store
            .read_attachment(stranger.id, with_file.attachments[0].id)
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
            )
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
        )
        .unwrap();
    store
        .send_message(ada.id, ChatTarget::Channel(text_id), "other", None, &[])
        .unwrap();
    let found = store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "hello")
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].body, "Hello there");
    let side = store
        .search_messages(ada.id, ChatTarget::Channel(other_channel.id), "hello")
        .unwrap();
    assert!(side.is_empty());
    let recent = store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "  ")
        .unwrap();
    let listed = store
        .messages(bob.id, ChatTarget::Channel(text_id), 100)
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
        )
        .unwrap();
    let unread = store.unread_mentions(bob.id).unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].message_id, mention.id);
    assert_eq!(unread[0].channel_id, Some(text_id));
    assert_eq!(unread[0].dm_id, None);
    assert_eq!(unread[0].from_username, "ada");
    assert!(store.unread_mentions(ada.id).unwrap().is_empty());
    assert!(store.unread_mentions(stranger.id).unwrap().is_empty());
    store.edit_message(ada.id, mention.id, "hi").unwrap();
    assert!(store.unread_mentions(bob.id).unwrap().is_empty());
    store
        .edit_message(ada.id, mention.id, "hi @bob again")
        .unwrap();
    let unread = store.unread_mentions(bob.id).unwrap();
    assert_eq!(unread.len(), 1);
    store.mark_mention_read(bob.id, unread[0].id).unwrap();
    assert!(store.unread_mentions(bob.id).unwrap().is_empty());

    store.delete_message(ada.id, first.id).unwrap();
    assert_eq!(
        store.delete_message(bob.id, third.id).unwrap_err(),
        BanterError::NotAuthor
    );
    let remaining = store
        .messages(bob.id, ChatTarget::Channel(text_id), 100)
        .unwrap();
    assert!(remaining.iter().all(|message| message.id != first.id));
    let second_now = remaining
        .iter()
        .find(|message| message.id == second.id)
        .unwrap();
    assert_eq!(second_now.reply_to, None);
    assert!(store
        .search_messages(bob.id, ChatTarget::Channel(text_id), "changed")
        .unwrap()
        .is_empty());
    store.delete_message(ada.id, with_file.id).unwrap();
    assert_eq!(
        store
            .read_attachment(bob.id, with_file.attachments[0].id)
            .unwrap_err(),
        BanterError::NotFound
    );

    drop(store);
    let store = Store::open(&dir).unwrap();
    let kept = store
        .messages(ada.id, ChatTarget::Channel(text_id), 100)
        .unwrap();
    assert!(kept.iter().any(|message| message.body == "two"));
    assert!(kept.iter().any(|message| message.body == "three"));
}

#[test]
fn voice_channel_is_drop_in_and_routes_media() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let bob = store.register("bob", "Bob", "password1").unwrap();
    let home = store.create_server(ada.id, "Voice").unwrap();
    let text_id = home.categories[0].channels[0].id;
    let voice_id = home.categories[1].channels[0].id;
    let code = store.create_invite(ada.id, home.server.id).unwrap();
    store.join_invite(bob.id, &code).unwrap();
    let notes = store.create_category(ada.id, home.server.id, "More").unwrap();
    let lounge = store
        .create_channel(ada.id, home.server.id, notes.id, "Lounge", ChannelKind::Voice)
        .unwrap();

    let live = Live::new();
    assert_eq!(
        live.join_server_voice(&store, ada.id, text_id).unwrap_err(),
        BanterError::NotVoiceChannel
    );
    live.join_server_voice(&store, ada.id, voice_id).unwrap();
    live.join_server_voice(&store, bob.id, voice_id).unwrap();
    let members = live.voice_channel_members(voice_id);
    assert_eq!(members.len(), 2);

    let routed = live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Audio, b"pcm")
        .unwrap();
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].user_id, bob.id);
    assert_eq!(routed[0].kind, MediaKind::Audio);
    assert_eq!(routed[0].bytes, b"pcm");

    let mut flags = VoiceFlags::default();
    flags.muted = true;
    live.set_flags(ada.id, flags);
    assert!(live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Audio, b"pcm")
        .unwrap()
        .is_empty());

    flags.muted = false;
    flags.push_to_talk = true;
    live.set_flags(ada.id, flags);
    assert!(live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Audio, b"pcm")
        .unwrap()
        .is_empty());
    flags.transmitting = true;
    live.set_flags(ada.id, flags);
    assert_eq!(
        live.route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Audio, b"pcm")
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
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Audio, b"pcm")
        .unwrap()
        .is_empty());
    let camera = live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Camera, b"jpg")
        .unwrap();
    assert_eq!(camera.len(), 1);
    assert_eq!(camera[0].user_id, bob.id);
    let screen = live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Screen, b"jpg")
        .unwrap();
    assert_eq!(screen.len(), 1);
    flags.camera = false;
    flags.screen = false;
    live.set_flags(ada.id, flags);
    assert!(live
        .route_media(ada.id, MediaPlace::VoiceChannel(voice_id), MediaKind::Camera, b"jpg")
        .unwrap()
        .is_empty());

    live.join_server_voice(&store, ada.id, lounge.id).unwrap();
    assert!(live
        .voice_channel_members(voice_id)
        .iter()
        .all(|member| member.user_id != ada.id));
    assert_eq!(live.voice_channel_members(lounge.id).len(), 1);
    live.leave_voice(ada.id);
    assert!(live.voice_channel_members(lounge.id).is_empty());
}

#[test]
fn direct_message_call() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let bob = store.register("bob", "Bob", "password1").unwrap();
    assert_eq!(
        store.open_dm(ada.id, bob.id).unwrap_err(),
        BanterError::NotFriend
    );
    let request = store.send_friend_request(ada.id, "bob").unwrap();
    store.accept_friend_request(bob.id, request.id).unwrap();
    let chat = store.open_dm(ada.id, bob.id).unwrap();
    let again = store.open_dm(bob.id, ada.id).unwrap();
    assert_eq!(chat.id, again.id);
    assert_eq!(chat.peer_id, bob.id);
    assert_eq!(again.peer_username, "ada");
    assert_eq!(store.direct_messages(ada.id).unwrap().len(), 1);

    let home = store.create_server(ada.id, "Home").unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let text_id = home.categories[0].channels[0].id;
    let sent = store
        .send_message(ada.id, ChatTarget::Direct(chat.id), "ping @bob", None, &[])
        .unwrap();
    assert_eq!(sent.author_nickname, None);
    assert_eq!(
        store
            .messages(bob.id, ChatTarget::Direct(chat.id), 10)
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .search_messages(ada.id, ChatTarget::Channel(text_id), "ping")
        .unwrap()
        .is_empty());
    let mentions = store.unread_mentions(bob.id).unwrap();
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].dm_id, Some(chat.id));
    assert_eq!(mentions[0].channel_id, None);

    let live = Live::new();
    live.join_server_voice(&store, ada.id, voice_id).unwrap();
    let call = live.start_dm_call(&store, ada.id, chat.id).unwrap();
    assert_eq!(call.phase, CallPhase::Ringing);
    assert_eq!(call.caller, ada.id);
    assert_eq!(call.callee, bob.id);
    assert!(live.voice_channel_members(voice_id).is_empty());
    assert!(live
        .route_media(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::Audio, b"pcm")
        .unwrap()
        .is_empty());

    let active = live.accept_dm_call(&store, bob.id, chat.id).unwrap();
    assert_eq!(active.phase, CallPhase::Active);
    let routed = live
        .route_media(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::Audio, b"pcm")
        .unwrap();
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].user_id, bob.id);

    let mut flags = VoiceFlags::default();
    flags.deafened = true;
    live.set_flags(bob.id, flags);
    assert!(live
        .route_media(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::Audio, b"pcm")
        .unwrap()
        .is_empty());
    flags = VoiceFlags {
        camera: true,
        ..VoiceFlags::default()
    };
    live.set_flags(ada.id, flags);
    assert_eq!(
        live.route_media(ada.id, MediaPlace::DirectCall(chat.id), MediaKind::Camera, b"jpg")
            .unwrap()
            .len(),
        1
    );

    live.end_dm_call(ada.id, chat.id).unwrap();
    assert!(live.call(chat.id).is_none());
    let ringing = live.start_dm_call(&store, ada.id, chat.id).unwrap();
    assert_eq!(ringing.phase, CallPhase::Ringing);
    live.decline_dm_call(bob.id, chat.id).unwrap();
    assert!(live.call(chat.id).is_none());
    assert_eq!(
        live.decline_dm_call(bob.id, chat.id).unwrap_err(),
        BanterError::NotInCall
    );
}

#[test]
fn presence_goes_idle_and_disconnect_leaves_voice() {
    let store = Store::open(&scratch()).unwrap();
    let ada = store.register("ada", "Ada", "password1").unwrap();
    let home = store.create_server(ada.id, "Home").unwrap();
    let voice_id = home.categories[1].channels[0].id;
    let live = Live::new();

    assert_eq!(live.presence(ada.id), Presence::Offline);
    live.connect(ada.id, 0);
    assert_eq!(live.presence(ada.id), Presence::Online);
    live.refresh(IDLE_AFTER_MS - 1);
    assert_eq!(live.presence(ada.id), Presence::Online);
    live.refresh(IDLE_AFTER_MS);
    assert_eq!(live.presence(ada.id), Presence::Idle);
    live.note_activity(ada.id, IDLE_AFTER_MS);
    live.refresh(IDLE_AFTER_MS);
    assert_eq!(live.presence(ada.id), Presence::Online);

    live.join_server_voice(&store, ada.id, voice_id).unwrap();
    live.disconnect(ada.id);
    assert_eq!(live.presence(ada.id), Presence::Offline);
    assert!(live.voice_channel_members(voice_id).is_empty());
}

#[test]
fn http_register_and_login_round_trip() {
    let dir = scratch();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let addr = format!("127.0.0.1:{port}");
    let server_addr = addr.clone();
    let server_dir = dir.clone();
    std::thread::spawn(move || {
        let _ = banter_server::serve(&server_addr, &server_dir);
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
