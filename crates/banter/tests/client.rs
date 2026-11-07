//! Client contracts. These were written before the desktop UI and stay as written.

use std::time::{Duration, Instant};

use std::collections::HashSet;

use banter::{
    CameraChoice, CaptureKind, Client, CommentRow, MixSource, MixSourceGroup, MuteMemory, ScreenShare, SettingsSection,
    StreamDefaults, StreamFps, StreamResolution, VoicePrefs, camera_preview_mirrored, clear_token, comment_descendant_count,
    comment_tree_rows, default_stream_fps, default_stream_resolution, discord_shell_columns, dismiss_stream_overlays,
    either_key_held, encode_jpeg_rgba, encode_jpeg_rgba_quality, equal_power_gains, fps_block, grouped_mix_sources,
    hardware_encoder_block, load_json, load_token, mic_opus_frame, mirror_rgba, mix_equal_power, mix_source_group,
    open_dm_peer_after_friend_add, open_mic_should_send, password_blocks_copy, password_is_masked, persist_login,
    push_to_talk_held, rasterize_lucide, remove_message, resample_mono_i16, resolution_block, save_json, save_token,
    settings_sections, speaking_from_level, stamp_cursor, stream_frame_rates, stream_frame_size, stream_interval_ms,
    stream_max_edge, stream_resolutions, toggle_comment_collapsed, upsert_message, websocket_url,
};
use banter_server::{
    ChannelKind, ChatTarget, ClientSignal, LayerId, MediaKind, MediaPlace, Message, ServerEvent, VoiceFlags, sdp_fixture,
    sdp_matches_contract,
};

fn message(id: i64, body: &str, reply_to: Option<i64>) -> Message {
    Message {
        id,
        author_id: 1,
        author_username: "ada".into(),
        author_display_name: "Ada".into(),
        author_nickname: None,
        body: body.into(),
        reply_to,
        created_at: 0,
        edited_at: None,
        attachments: Vec::new(),
        media: Vec::new(),
        author_kind: "user".into(),
        plugin_id: None,
    }
}


fn test_db_url() -> String {
    let base = std::env::var("BANTER_DATABASE_URL")
        .unwrap_or_else(|_| banter_server::DEFAULT_DATABASE_URL.to_string());
    let schema = format!(
        "c{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&base).await.expect("postgres");
        sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .execute(&pool)
            .await
            .expect("schema");
    });
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}options=-csearch_path%3D{schema}")
}

/// Poll `/v1/health` until the spawned test server accepts connections.
/// CI under parallel load often needs more than the old 2.5s (50×50ms) window.
fn wait_until_http_ready(base: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if ureq::get(&format!("{base}/v1/health")).call().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not open {base}");
}

#[test]
fn session_token_round_trip() {
    let dir = std::env::temp_dir().join(format!("banter-client-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(load_token(&dir).is_none());
    save_token(&dir, "abc123").unwrap();
    assert_eq!(load_token(&dir).as_deref(), Some("abc123"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stay_signed_in_writes_the_session_file() {
    let dir = std::env::temp_dir().join(format!("banter-stay-signed-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    persist_login(&dir, "keep-me", true).unwrap();
    assert_eq!(load_token(&dir).as_deref(), Some("keep-me"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_stay_signed_in_does_not_leave_a_session_file() {
    let dir = std::env::temp_dir().join(format!("banter-no-stay-signed-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    save_token(&dir, "old-token").unwrap();
    persist_login(&dir, "session-only", false).unwrap();
    assert!(load_token(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clear_token_removes_a_saved_session() {
    let dir = std::env::temp_dir().join(format!("banter-clear-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    save_token(&dir, "gone").unwrap();
    clear_token(&dir).unwrap();
    assert!(load_token(&dir).is_none());
    clear_token(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deafen_restores_the_mute_from_before_it_was_enabled() {
    let mut memory = MuteMemory::new();
    memory.set_deafened(true);
    assert!(memory.deafened());
    assert!(memory.muted());
    memory.set_muted(false);
    memory.set_deafened(false);
    assert!(!memory.deafened());
    assert!(!memory.muted());

    memory.set_muted(true);
    memory.set_deafened(true);
    memory.set_muted(false);
    memory.set_deafened(false);
    assert!(memory.muted());
}

#[test]
fn voice_flags_never_advertise_talking_while_muted_or_deafened() {
    let mut memory = MuteMemory::new();
    let open = memory.voice_flags(false, true, false, false);
    assert!(open.transmitting);

    memory.set_muted(true);
    let muted = memory.voice_flags(false, true, false, false);
    assert!(muted.muted);
    assert!(!muted.transmitting);

    memory.set_deafened(true);
    let deafened = memory.voice_flags(true, true, false, false);
    assert!(deafened.deafened);
    assert!(deafened.muted);
    assert!(!deafened.transmitting);

    memory.set_deafened(false);
    memory.set_muted(false);
    let unmuted = memory.voice_flags(false, true, false, false);
    assert!(unmuted.transmitting);
}

#[test]
fn password_stays_masked_until_show_password_is_on() {
    assert!(password_is_masked(false));
    assert!(!password_is_masked(true));
}

#[test]
fn hidden_password_still_allows_copy() {
    assert!(password_is_masked(false));
    assert!(!password_blocks_copy(false));
    assert!(!password_blocks_copy(true));
}

#[test]
fn ctrl_shortcut_follows_either_ctrl_key() {
    assert!(!either_key_held(false, false));
    assert!(either_key_held(true, false));
    assert!(either_key_held(false, true));
}

#[test]
fn settings_menu_lists_account_then_voice() {
    let sections = settings_sections();
    assert_eq!(
        sections,
        [
            SettingsSection::Account,
            SettingsSection::Voice,
            SettingsSection::Appearance,
            SettingsSection::Mods,
            SettingsSection::Developer,
        ]
    );
}

#[test]
fn screen_share_waits_for_go_live() {
    let mut share = ScreenShare::new();
    assert!(!share.go_live());
    assert!(!share.is_live());
    assert_eq!(share.live_target(), None);
    share.select(CaptureKind::Screen, 7);
    assert!(!share.is_live());
    assert!(share.go_live());
    assert!(share.is_live());
    assert_eq!(share.selected(), Some((CaptureKind::Screen, 7)));
    assert_eq!(share.live_target(), Some((CaptureKind::Screen, 7)));
    share.select(CaptureKind::Window, 3);
    assert_eq!(share.selected(), Some((CaptureKind::Window, 3)));
    assert_eq!(share.live_target(), Some((CaptureKind::Screen, 7)));
    assert!(share.go_live());
    assert_eq!(share.live_target(), Some((CaptureKind::Window, 3)));
    share.stop();
    assert!(!share.is_live());
    assert_eq!(share.live_target(), None);
    assert_eq!(share.selected(), Some((CaptureKind::Window, 3)));
    share.go_live_portal();
    assert!(share.is_live());
    assert!(share.is_portal());
    assert_eq!(share.live_target(), None);
    share.stop();
    assert!(!share.is_portal());
}

#[test]
fn leave_or_stop_dismisses_stream_overlays() {
    let mut share_open = true;
    let mut stream_menu = true;
    dismiss_stream_overlays(&mut share_open, &mut stream_menu);
    assert!(!share_open);
    assert!(!stream_menu);
    dismiss_stream_overlays(&mut share_open, &mut stream_menu);
    assert!(!share_open);
    assert!(!stream_menu);
}

#[test]
fn discord_stream_quality_defaults_to_720p_30() {
    assert_eq!(
        stream_resolutions(),
        [
            StreamResolution::P480,
            StreamResolution::P720,
            StreamResolution::P1080,
            StreamResolution::P1440,
            StreamResolution::P2160,
            StreamResolution::Source
        ]
    );
    assert_eq!(
        stream_frame_rates(),
        [StreamFps::Fps15, StreamFps::Fps30, StreamFps::Fps60, StreamFps::Fps120, StreamFps::Fps144]
    );
    assert_eq!(default_stream_resolution(), StreamResolution::P720);
    assert_eq!(default_stream_fps(), StreamFps::Fps30);
    assert_eq!(stream_max_edge(StreamResolution::P480), 854);
    assert_eq!(stream_max_edge(StreamResolution::P720), 1280);
    assert_eq!(stream_max_edge(StreamResolution::P1080), 1920);
    assert_eq!(stream_max_edge(StreamResolution::P1440), 2560);
    assert_eq!(stream_max_edge(StreamResolution::P2160), 3840);
    assert_eq!(stream_max_edge(StreamResolution::Source), 0);
    assert_eq!(stream_interval_ms(StreamFps::Fps15), 66);
    assert_eq!(stream_interval_ms(StreamFps::Fps30), 33);
    assert_eq!(stream_interval_ms(StreamFps::Fps60), 16);
    assert_eq!(stream_interval_ms(StreamFps::Fps120), 8);
    assert_eq!(stream_interval_ms(StreamFps::Fps144), 7);
    assert_eq!(stream_frame_size(StreamResolution::P1080, 3840, 2160), (1920, 1080));
    assert!(resolution_block(StreamResolution::P2160, 1080).is_some());
    assert!(resolution_block(StreamResolution::P1080, 1080).is_none());
    assert!(fps_block(StreamFps::Fps144, 60).is_some());
    assert!(fps_block(StreamFps::Fps60, 60).is_none());
}

#[test]
fn camera_preview_is_mirrored_for_the_local_user_only() {
    assert!(camera_preview_mirrored());
    let rgba = vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
    let mirrored = mirror_rgba(2, 2, &rgba).unwrap();
    assert_eq!(mirrored, vec![2, 0, 0, 255, 1, 0, 0, 255, 4, 0, 0, 255, 3, 0, 0, 255]);
    let mut choice = CameraChoice::new();
    assert_eq!(choice.device_index(), 0);
    assert!(choice.needs_reopen(None));
    assert!(!choice.needs_reopen(Some(0)));
    choice.select(2);
    assert!(choice.needs_reopen(Some(0)));
    assert!(!choice.needs_reopen(Some(2)));
}

#[test]
fn lucide_settings_icon_is_a_square_with_ink() {
    let rgba = rasterize_lucide("settings", 24, [255, 255, 255]).expect("settings icon");
    assert_eq!(rgba.len(), 24 * 24 * 4);
    assert!(rgba.chunks(4).any(|pixel| pixel[3] > 0));
    let mic = rasterize_lucide("mic", 24, [255, 255, 255]).unwrap();
    let muted = rasterize_lucide("mic-off", 24, [255, 255, 255]).unwrap();
    assert_ne!(mic, muted);
}

#[test]
fn discord_shell_puts_the_member_list_on_the_right() {
    let columns = discord_shell_columns(1280.0, true);
    assert_eq!(columns.rail_x, 0.0);
    assert_eq!(columns.rail_w, 72.0);
    assert_eq!(columns.sidebar_x, 72.0);
    assert_eq!(columns.sidebar_w, 240.0);
    assert_eq!(columns.stage_x, 312.0);
    assert_eq!(columns.stage_w, 728.0);
    assert_eq!(columns.members_x, 1040.0);
    assert_eq!(columns.members_w, 240.0);
    assert!(columns.members_x > columns.stage_x);
}

#[test]
fn discord_shell_drops_the_member_list_on_home() {
    let columns = discord_shell_columns(1280.0, false);
    assert_eq!(columns.members_w, 0.0);
    assert_eq!(columns.stage_x, 312.0);
    assert_eq!(columns.stage_w, 968.0);
}

#[test]
fn discord_shell_stays_non_negative_in_a_narrow_window() {
    let columns = discord_shell_columns(100.0, true);
    assert!(columns.rail_w >= 0.0);
    assert!(columns.sidebar_w >= 0.0);
    assert!(columns.stage_w >= 0.0);
    assert!(columns.members_w >= 0.0);
    assert!(columns.rail_x <= columns.sidebar_x);
    assert!(columns.sidebar_x <= columns.stage_x);
    assert!(columns.stage_x <= columns.members_x);
    assert!(columns.members_x + columns.members_w <= 100.0 + 0.01);
}

#[test]
fn push_to_talk_ignores_the_key_while_text_input_is_active() {
    assert!(!push_to_talk_held(false, false, false));
    assert!(push_to_talk_held(true, false, false));
    assert!(!push_to_talk_held(true, false, true));
    assert!(push_to_talk_held(true, true, true));
}

#[test]
fn open_mic_drops_silence_and_keeps_a_loud_frame() {
    assert!(!open_mic_should_send(&[0; 960]));
    assert!(open_mic_should_send(&[4_000; 960]));
}

#[test]
fn mic_opus_keeps_quiet_open_mic_frames_for_a_stable_timeline() {
    // Speaking glow still uses open_mic_should_send / speaking_from_level.
    // Feeding Opus must not skip quiet open-mic frames — that chopped words and
    // broke DTX. Mute / deafen / PTT-up still return None so the track stops.
    let quiet = [0i16; 960];
    let loud = [4_000i16; 960];
    let open = VoiceFlags::default();
    assert_eq!(mic_opus_frame(&open, &quiet).as_deref(), Some(quiet.as_slice()));
    assert_eq!(mic_opus_frame(&open, &loud).as_deref(), Some(loud.as_slice()));

    let muted = VoiceFlags {
        muted: true,
        ..VoiceFlags::default()
    };
    assert!(mic_opus_frame(&muted, &loud).is_none());

    let ptt_up = VoiceFlags {
        push_to_talk: true,
        transmitting: false,
        ..VoiceFlags::default()
    };
    assert!(mic_opus_frame(&ptt_up, &loud).is_none());

    let ptt_down = VoiceFlags {
        push_to_talk: true,
        transmitting: true,
        ..VoiceFlags::default()
    };
    assert_eq!(mic_opus_frame(&ptt_down, &quiet).as_deref(), Some(quiet.as_slice()));
}

#[test]
fn resample_keeps_the_same_samples_when_the_rate_matches() {
    let input = [1, 2, 3, 4];
    assert_eq!(resample_mono_i16(&input, 48_000, 48_000), input);
    assert_eq!(resample_mono_i16(&input, 24_000, 48_000).len(), 8);
}

#[test]
fn messages_upsert_in_order_and_delete_clears_replies() {
    let mut messages = vec![message(1, "one", None), message(3, "three", Some(2))];
    upsert_message(&mut messages, message(2, "two", Some(1)));
    assert_eq!(
        messages.iter().map(|message| message.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    upsert_message(&mut messages, message(2, "edited", Some(1)));
    assert_eq!(messages[1].body, "edited");
    remove_message(&mut messages, 1);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].reply_to, None);
}

#[test]
fn comment_tree_nests_replies_depth_first_like_reddit() {
    // Roots in id order; each parent's children immediately after it (Reddit DFS).
    let messages = vec![
        message(1, "root a", None),
        message(2, "root b", None),
        message(3, "reply to a", Some(1)),
        message(4, "reply to reply", Some(3)),
        message(5, "second reply to a", Some(1)),
        message(6, "orphan becomes root", Some(99)),
    ];
    let rows = comment_tree_rows(&messages, &HashSet::new());
    assert_eq!(
        rows
            .iter()
            .map(|row| (row.message_id, row.depth, row.child_count, row.collapsed))
            .collect::<Vec<_>>(),
        vec![
            (1, 0, 3, false),
            (3, 1, 1, false),
            (4, 2, 0, false),
            (5, 1, 0, false),
            (2, 0, 0, false),
            (6, 0, 0, false),
        ]
    );
    // Ancestor chain lets each vertical guide map to the comment it collapses.
    assert_eq!(rows[0].ancestor_ids, Vec::<i64>::new());
    assert_eq!(rows[1].ancestor_ids, vec![1]);
    assert_eq!(rows[2].ancestor_ids, vec![1, 3]);
    assert_eq!(rows[3].ancestor_ids, vec![1]);
    assert!(rows[4].ancestor_ids.is_empty());
    assert!(rows[5].ancestor_ids.is_empty());
}

#[test]
fn comment_tree_ancestor_ids_name_each_thread_guide_owner() {
    // On a depth-2 row, guide 0 collapses the root; guide 1 collapses the mid parent.
    let messages = vec![
        message(10, "root", None),
        message(11, "mid", Some(10)),
        message(12, "leaf", Some(11)),
    ];
    let rows = comment_tree_rows(&messages, &HashSet::new());
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].message_id, 12);
    assert_eq!(rows[2].depth, 2);
    assert_eq!(rows[2].ancestor_ids, vec![10, 11]);
    assert_eq!(rows[2].ancestor_ids[0], 10);
    assert_eq!(rows[2].ancestor_ids[1], 11);
}

#[test]
fn comment_tree_collapse_hides_descendants_and_toggle_round_trips() {
    let messages = vec![
        message(1, "root", None),
        message(2, "child", Some(1)),
        message(3, "grand", Some(2)),
        message(4, "other root", None),
    ];
    let mut collapsed = HashSet::new();
    toggle_comment_collapsed(&mut collapsed, 1);
    assert!(collapsed.contains(&1));
    let rows = comment_tree_rows(&messages, &collapsed);
    assert_eq!(
        rows
            .iter()
            .map(|row| (row.message_id, row.depth, row.child_count, row.collapsed))
            .collect::<Vec<_>>(),
        vec![(1, 0, 2, true), (4, 0, 0, false)]
    );
    assert_eq!(comment_descendant_count(&messages, 1), 2);
    assert_eq!(comment_descendant_count(&messages, 2), 1);
    assert_eq!(comment_descendant_count(&messages, 4), 0);
    toggle_comment_collapsed(&mut collapsed, 1);
    assert!(collapsed.is_empty());
    assert_eq!(comment_tree_rows(&messages, &collapsed).len(), 4);
}

#[test]
fn comment_row_helper_matches_visible_node() {
    let row = CommentRow {
        message_id: 7,
        depth: 3,
        child_count: 2,
        collapsed: true,
        ancestor_ids: vec![1, 2, 4],
    };
    assert_eq!(row.message_id, 7);
    assert_eq!(row.depth, 3);
    assert_eq!(row.child_count, 2);
    assert!(row.collapsed);
    assert_eq!(row.ancestor_ids, vec![1, 2, 4]);
}

#[test]
fn websocket_url_rewrites_the_http_base() {
    assert_eq!(
        websocket_url("http://127.0.0.1:3970", "abc"),
        "ws://127.0.0.1:3970/v1/ws?token=abc"
    );
    assert_eq!(
        websocket_url("https://banter.local/", "abc"),
        "wss://banter.local/v1/ws?token=abc"
    );
}

#[test]
fn jpeg_frame_keeps_small_dimensions() {
    let rgba = [255_u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let jpeg = encode_jpeg_rgba(2, 2, &rgba, 640).unwrap();
    let decoded = image::load_from_memory(&jpeg).unwrap();
    assert_eq!(decoded.width(), 2);
    assert_eq!(decoded.height(), 2);
}

#[test]
fn client_can_register_and_send_a_message() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let dir = std::env::temp_dir().join(format!("banter-client-api-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{port}");
    let serve_addr = addr.clone();
    let serve_dir = dir.clone();
    let db_url = test_db_url();
    let server = std::thread::spawn(move || banter_server::serve(&serve_addr, &serve_dir, &db_url));
    let base = format!("http://{addr}");
    wait_until_http_ready(&base);

    let mut client = Client::new(&base);
    let session = client.register("ada", "Ada", "password1").unwrap();
    assert_eq!(session.account.username, "ada");
    assert_eq!(client.me().unwrap().id, session.account.id);

    let home = client.create_server("Club").unwrap();
    let channel = home
        .categories
        .iter()
        .flat_map(|category| category.channels.iter())
        .find(|channel| channel.kind == ChannelKind::Text)
        .expect("default text channel");
    let sent = client
        .send_message(ChatTarget::Channel(channel.id), "hello", None, &[], &[])
        .unwrap();
    assert_eq!(sent.body, "hello");
    let listed = client
        .messages(ChatTarget::Channel(channel.id), 50, None)
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].body, "hello");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn speaking_ring_follows_real_mic_level() {
    assert!(!speaking_from_level(false, false, 10.0, 180.0));
    assert!(speaking_from_level(false, false, 2_000.0, 180.0));
    assert!(!speaking_from_level(true, false, 2_000.0, 180.0));
    assert!(speaking_from_level(true, true, 0.0, 180.0));
}

#[test]
fn equal_power_mix_and_stream_prefs_round_trip() {
    let (a, b) = equal_power_gains(0.0);
    assert!((a - 1.0).abs() < 0.001 && b.abs() < 0.001);
    let tone = vec![8_000i16; 32];
    let start = mix_equal_power(&tone, &vec![0; 32], 0.0);
    let mid = mix_equal_power(&tone, &tone, 0.5);
    let end = mix_equal_power(&vec![0; 32], &tone, 1.0);
    assert_eq!(start[0], 8_000);
    assert_eq!(end[0], 8_000);
    assert!((mid[0] as i32 - 8_000).abs() < 400);
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        pixel[0] = (index * 13) as u8;
        pixel[1] = (index * 7) as u8;
        pixel[2] = (index * 3) as u8;
        pixel[3] = 255;
    }
    let small = encode_jpeg_rgba_quality(32, 32, &rgba, 0, 15).unwrap();
    let large = encode_jpeg_rgba_quality(32, 32, &rgba, 0, 95).unwrap();
    assert!(large.len() > small.len());
    let mut frame = vec![0u8; 8 * 8 * 4];
    stamp_cursor(&mut frame, 8, 8, 3, 3, true);
    let mut hidden = frame.clone();
    hidden.fill(0);
    stamp_cursor(&mut hidden, 8, 8, 3, 3, false);
    assert_ne!(frame, hidden);
    assert!(hardware_encoder_block(false).is_some());
    let dir = std::env::temp_dir().join(format!("banter-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prefs = VoicePrefs::default();
    save_json(&dir.join("voice.json"), &prefs).unwrap();
    let loaded: VoicePrefs = load_json(&dir.join("voice.json")).unwrap();
    assert!(loaded.noise_suppression);
    let mut stream = StreamDefaults::default();
    stream.resolution = "1080p".into();
    stream.fps = "60".into();
    save_json(&dir.join("stream.json"), &stream).unwrap();
    let loaded: StreamDefaults = load_json(&dir.join("stream.json")).unwrap();
    assert_eq!(loaded.fps, "60");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stream_audio_sources_group_by_channel_and_app() {
    assert_eq!(mix_source_group("mic:Headset"), MixSourceGroup::AudioChannels);
    assert_eq!(mix_source_group("loop:Speakers"), MixSourceGroup::AudioChannels);
    assert_eq!(mix_source_group("app:4242"), MixSourceGroup::Apps);
    assert_eq!(MixSourceGroup::AudioChannels.label(), "Audio channels");
    assert_eq!(MixSourceGroup::Apps.label(), "Apps");

    let sources = vec![
        MixSource {
            id: "mic:Mic".into(),
            name: "Mic".into(),
            enabled: true,
            volume: 1.0,
        },
        MixSource {
            id: "app:100".into(),
            name: "Game".into(),
            enabled: false,
            volume: 0.8,
        },
        MixSource {
            id: "loop:Speakers".into(),
            name: "Speakers (speakers)".into(),
            enabled: true,
            volume: 0.5,
        },
        MixSource {
            id: "app:200".into(),
            name: "Browser".into(),
            enabled: true,
            volume: 1.0,
        },
    ];
    let groups = grouped_mix_sources(&sources);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, MixSourceGroup::AudioChannels);
    assert_eq!(groups[0].1, vec![0, 2]);
    assert_eq!(groups[1].0, MixSourceGroup::Apps);
    assert_eq!(groups[1].1, vec![1, 3]);
    assert!(sources.iter().filter(|source| source.enabled).count() >= 2);

    let apps_only = grouped_mix_sources(&[sources[1].clone()]);
    assert_eq!(apps_only.len(), 1);
    assert_eq!(apps_only[0].0, MixSourceGroup::Apps);
    assert!(grouped_mix_sources(&[]).is_empty());
}

#[test]
fn second_client_receives_voice_and_a_live_quality_switch() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let dir = std::env::temp_dir().join(format!("banter-voice-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{port}");
    let serve_addr = addr.clone();
    let serve_dir = dir.clone();
    let db_url = test_db_url();
    let server = std::thread::spawn(move || banter_server::serve(&serve_addr, &serve_dir, &db_url));
    let base = format!("http://{addr}");
    wait_until_http_ready(&base);

    let mut ada = Client::new(&base);
    ada.register("ada", "Ada", "password1").unwrap();
    let mut bea = Client::new(&base);
    bea.register("bea", "Bea", "password1").unwrap();
    let home = ada.create_server("Club").unwrap();
    let code = ada.create_invite(home.server.id).unwrap();
    bea.join_invite(&code).unwrap();
    let voice = home
        .categories
        .iter()
        .flat_map(|category| category.channels.iter())
        .find(|channel| channel.kind == ChannelKind::Voice)
        .expect("voice channel");
    let ada_token = ada.token().unwrap().to_string();
    let bea_token = bea.token().unwrap().to_string();
    let (mut ada_ws, _) = tungstenite::connect(websocket_url(&base, &ada_token)).unwrap();
    let (mut bea_ws, _) = tungstenite::connect(websocket_url(&base, &bea_token)).unwrap();
    for socket in [&mut ada_ws, &mut bea_ws] {
        if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = socket.get_mut() {
            tcp.set_read_timeout(Some(Duration::from_millis(40))).unwrap();
        }
    }
    ada.join_voice(voice.id).unwrap();
    bea.join_voice(voice.id).unwrap();
    ada.set_flags(VoiceFlags {
        screen: true,
        transmitting: true,
        ..VoiceFlags::default()
    })
    .unwrap();
    bea.set_flags(VoiceFlags::default()).unwrap();

    let ada_id = ada.me().unwrap().id;
    let subscribe = ClientSignal::SubscribeLayer {
        place: MediaPlace::VoiceChannel(voice.id),
        publisher: ada_id,
        track: MediaKind::Screen,
        max_layer: LayerId::Quarter,
    };
    ada_ws.send(tungstenite::Message::Binary(vec![0, 1, 2, 3].into())).unwrap();
    bea_ws.send(tungstenite::Message::Text(serde_json::to_string(&subscribe).unwrap().into())).unwrap();

    let mut wanted = false;
    let mut binary = false;
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(2) && !wanted {
        for socket in [&mut ada_ws, &mut bea_ws] {
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    if let Ok(ServerEvent::WantedLayers { track, layers, .. }) = serde_json::from_str(text.as_ref()) {
                        if track == MediaKind::Screen && layers.contains(&LayerId::Quarter) {
                            wanted = true;
                        }
                    }
                }
                Ok(tungstenite::Message::Binary(_)) => binary = true,
                _ => {}
            }
        }
    }
    assert!(wanted, "the publisher was not told which screen layer to send");
    assert!(!binary, "binary websocket frames are not a media path");
    assert!(sdp_matches_contract(&sdp_fixture()));
    ada_ws.close(None).ok();
    bea_ws.close(None).ok();
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

fn chat_ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis()
}

#[test]
fn chat_rows_group_same_author_and_label_the_local_time() {
    let offset = -6 * 3600;
    let today = chat_ms("2026-11-03T10:17:00-06:00");
    let now = chat_ms("2026-11-03T12:00:00-06:00");
    assert_eq!(banter::chat_time_label(today, now, offset), "Today at 10:17 AM");
    assert_eq!(
        banter::chat_time_label(chat_ms("2026-11-03T09:05:00-06:00"), now, offset),
        "Today at 9:05 AM"
    );
    assert_eq!(
        banter::chat_time_label(chat_ms("2026-11-02T10:17:00-06:00"), now, offset),
        "Yesterday at 10:17 AM"
    );
    assert_eq!(
        banter::chat_time_hover(today, offset),
        "Tuesday, November 3, 2026 at 10:17 AM"
    );
    assert_eq!(banter::chat_date_separator(today, offset), "November 3, 2026");

    let later = today + 7 * 60 * 1000;
    assert!(banter::message_is_followup(4, today, 4, later, offset));
    assert!(!banter::message_is_followup(4, today, 4, later + 1, offset));
    assert!(!banter::message_is_followup(4, today, 9, today + 60_000, offset));
    let before_midnight = chat_ms("2026-11-03T23:59:30-06:00");
    let after_midnight = chat_ms("2026-11-04T00:00:30-06:00");
    assert!(!banter::message_is_followup(4, before_midnight, 4, after_midnight, offset));
    assert!(banter::message_starts_new_day(before_midnight, after_midnight, offset));
    assert!(!banter::message_starts_new_day(today, later, offset));
}

#[test]
fn avatar_crop_covers_the_center_and_zooms_in() {
    let mut rgba = vec![0u8; 4 * 2 * 4];
    let colors = [10u8, 20, 30, 40, 50, 60, 70, 80];
    for (index, color) in colors.iter().enumerate() {
        rgba[index * 4] = *color;
        rgba[index * 4 + 3] = 255;
    }
    let fitted = banter::crop_avatar_rgba(4, 2, &rgba, 1.0, 0.0, 0.0, 2);
    assert_eq!(fitted.len(), 2 * 2 * 4);
    assert_eq!(fitted[0], 20);
    assert_eq!(fitted[4], 30);
    assert_eq!(fitted[8], 60);
    assert_eq!(fitted[12], 70);

    let zoomed = banter::crop_avatar_rgba(4, 2, &rgba, 2.0, 0.0, 0.0, 2);
    assert_eq!(zoomed[0], 20);
    assert_eq!(zoomed[4], 30);

    let panned = banter::crop_avatar_rgba(4, 2, &rgba, 2.0, 1.0, 0.0, 2);
    assert_eq!(panned[0], 40);
}

#[test]
fn adding_a_friend_opens_their_dm_when_they_are_friends() {
    let friends = [(9_i64, "bob"), (4, "cam")];
    assert_eq!(open_dm_peer_after_friend_add(friends, "Bob"), Some(9));
    assert_eq!(open_dm_peer_after_friend_add(friends, "  cam "), Some(4));
    assert_eq!(open_dm_peer_after_friend_add(friends, "nobody"), None);
    assert_eq!(open_dm_peer_after_friend_add(friends, ""), None);
    assert_eq!(open_dm_peer_after_friend_add([], "bob"), None);
}

#[test]
fn accepting_a_friend_then_opening_their_dm_works() {
    // Uses the HTTP client (Store::open is async and needs a DB URL; see foundation tests).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let dir = std::env::temp_dir().join(format!("banter-friend-dm-api-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{port}");
    let serve_addr = addr.clone();
    let serve_dir = dir.clone();
    let db_url = test_db_url();
    let server = std::thread::spawn(move || banter_server::serve(&serve_addr, &serve_dir, &db_url));
    let base = format!("http://{addr}");
    wait_until_http_ready(&base);

    let mut ada = Client::new(&base);
    ada.register("ada", "Ada", "password1").unwrap();
    let mut bob = Client::new(&base);
    bob.register("bob", "Bob", "password1").unwrap();
    let request = ada.send_friend_request("bob").unwrap();
    assert_eq!(
        open_dm_peer_after_friend_add(
            bob.friends().unwrap().iter().map(|friend| (friend.id, friend.username.as_str())),
            &request.from_username,
        ),
        None,
        "pending requests must not open a DM"
    );
    bob.accept_friend_request(request.id).unwrap();
    let friends = bob.friends().unwrap();
    let peer = open_dm_peer_after_friend_add(
        friends.iter().map(|friend| (friend.id, friend.username.as_str())),
        &request.from_username,
    )
    .expect("accepting a friend should yield a DM peer");
    let chat = bob.open_dm(peer).unwrap();
    assert_eq!(chat.peer_id, ada.me().unwrap().id);
    assert_eq!(chat.peer_username, "ada");

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn message_context_menu_lists_reply_copy_and_own_actions() {
    let other = banter::message_context_menu(10, 2, "hello", Some(1));
    assert_eq!(
        banter::context_menu_labels(&other),
        ["Reply", "Copy Text", "Copy Message ID"]
    );
    assert!(banter::context_menu_actions(&other).contains(&banter::ContextMenuAction::Reply { message_id: 10 }));
    assert!(banter::context_menu_actions(&other).contains(&banter::ContextMenuAction::CopyText {
        text: "hello".into()
    }));
    assert!(banter::context_menu_actions(&other).contains(&banter::ContextMenuAction::CopyMessageId {
        message_id: 10
    }));
    assert!(!banter::context_menu_actions(&other).iter().any(|action| matches!(
        action,
        banter::ContextMenuAction::Edit { .. } | banter::ContextMenuAction::Delete { .. }
    )));

    let own = banter::message_context_menu(11, 1, "mine", Some(1));
    assert_eq!(
        banter::context_menu_labels(&own),
        ["Reply", "Edit Message", "Delete Message", "Copy Text", "Copy Message ID"]
    );
    assert!(banter::context_menu_actions(&own).contains(&banter::ContextMenuAction::Edit { message_id: 11 }));
    assert!(banter::context_menu_actions(&own).contains(&banter::ContextMenuAction::Delete { message_id: 11 }));
    assert_eq!(
        banter::context_menu_tone(&own, "Delete Message"),
        Some(banter::ContextMenuTone::Danger)
    );
}

#[test]
fn member_context_menu_skips_self_actions_and_friend_when_already_friends() {
    let stranger = banter::member_context_menu(7, "ada", Some(1), false, false);
    assert_eq!(
        banter::context_menu_labels(&stranger),
        ["Mention", "Message", "Call", "Add Friend", "Copy Username"]
    );
    assert!(banter::context_menu_actions(&stranger).contains(&banter::ContextMenuAction::Mention {
        username: "ada".into()
    }));
    assert!(banter::context_menu_actions(&stranger).contains(&banter::ContextMenuAction::MessageUser {
        user_id: 7
    }));
    assert!(banter::context_menu_actions(&stranger).contains(&banter::ContextMenuAction::CallUser {
        user_id: 7
    }));
    assert!(banter::context_menu_actions(&stranger).contains(&banter::ContextMenuAction::AddFriend {
        username: "ada".into()
    }));
    assert_eq!(
        banter::context_menu_tone(&stranger, "Message"),
        Some(banter::ContextMenuTone::Brand)
    );

    let friend = banter::member_context_menu(7, "ada", Some(1), true, false);
    assert_eq!(
        banter::context_menu_labels(&friend),
        ["Mention", "Message", "Call", "Copy Username"]
    );
    assert!(!banter::context_menu_actions(&friend)
        .iter()
        .any(|action| matches!(action, banter::ContextMenuAction::AddFriend { .. })));

    let self_menu = banter::member_context_menu(1, "owl", Some(1), false, false);
    assert_eq!(banter::context_menu_labels(&self_menu), ["Mention", "Copy Username"]);

    let manager = banter::member_context_menu(7, "ada", Some(1), true, true);
    assert_eq!(
        banter::context_menu_labels(&manager),
        ["Mention", "Message", "Call", "Roles", "Copy Username"]
    );
    assert!(banter::context_menu_actions(&manager).contains(&banter::ContextMenuAction::AssignRoles { user_id: 7 }));
}

#[test]
fn channel_context_menu_offers_invite_and_copy_for_foundation() {
    let menu = banter::channel_context_menu(42, "general", 3, false);
    assert_eq!(
        banter::context_menu_labels(&menu),
        ["Invite People", "Copy Name", "Copy Channel ID"]
    );
    assert!(banter::context_menu_actions(&menu).contains(&banter::ContextMenuAction::InvitePeople {
        server_id: 3
    }));
    assert!(banter::context_menu_actions(&menu).contains(&banter::ContextMenuAction::CopyChannelName {
        name: "general".into()
    }));
    assert!(banter::context_menu_actions(&menu).contains(&banter::ContextMenuAction::CopyChannelId {
        channel_id: 42
    }));
    assert_eq!(
        banter::context_menu_tone(&menu, "Invite People"),
        Some(banter::ContextMenuTone::Brand)
    );

    let manager = banter::channel_context_menu(42, "general", 3, true);
    assert_eq!(
        banter::context_menu_labels(&manager),
        ["Invite People", "Edit Channel", "Copy Name", "Copy Channel ID"]
    );
    assert!(banter::context_menu_actions(&manager).contains(
        &banter::ContextMenuAction::EditChannel { channel_id: 42 }
    ));
}

#[test]
fn channel_display_icon_uses_custom_or_kind_default() {
    use banter_server::ChannelKind;

    assert_eq!(banter::channel_display_icon(&ChannelKind::Text, None), "hash");
    assert_eq!(
        banter::channel_display_icon(&ChannelKind::Voice, None),
        "volume-2"
    );
    assert_eq!(
        banter::channel_display_icon(&ChannelKind::Board, None),
        "pencil"
    );
    assert_eq!(
        banter::channel_display_icon(&ChannelKind::Text, Some("mic")),
        "mic"
    );
    assert_eq!(
        banter::channel_display_icon(&ChannelKind::Text, Some("not-a-real-icon")),
        "hash"
    );
    assert_eq!(
        banter::channel_display_icon(&ChannelKind::Voice, Some("")),
        "volume-2"
    );
}

#[test]
fn insert_mention_pads_the_composer_like_discord() {
    let mut empty = String::new();
    banter::insert_mention(&mut empty, "ada");
    assert_eq!(empty, "@ada ");

    let mut trailing = String::from("hey ");
    banter::insert_mention(&mut trailing, "ada");
    assert_eq!(trailing, "hey @ada ");

    let mut mid = String::from("hey");
    banter::insert_mention(&mut mid, "ada");
    assert_eq!(mid, "hey @ada ");
}

#[test]
fn context_menu_height_counts_items_and_separators() {
    let menu = banter::message_context_menu(1, 1, "x", Some(1));
    let height = banter::context_menu_height(&menu);
    assert!(height > banter::theme::MENU_PAD * 2.0);
    assert_eq!(
        height,
        banter::theme::MENU_PAD * 2.0
            + 5.0 * (banter::theme::MENU_ITEM_H + 2.0)
            + 2.0 * 9.0
    );
}

#[test]
fn clamp_context_menu_keeps_the_panel_on_screen() {
    assert_eq!(
        banter::clamp_context_menu_pos([10.0, 10.0], [220.0, 100.0], [800.0, 600.0]),
        [10.0, 10.0]
    );
    assert_eq!(
        banter::clamp_context_menu_pos([790.0, 580.0], [220.0, 100.0], [800.0, 600.0]),
        [580.0, 500.0]
    );
    assert_eq!(
        banter::clamp_context_menu_pos([-20.0, -5.0], [220.0, 100.0], [800.0, 600.0]),
        [0.0, 0.0]
    );
}

