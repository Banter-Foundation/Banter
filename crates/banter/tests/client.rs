//! Client contracts. These were written before the desktop UI and stay as written.

use std::time::Duration;

use banter::{
    CameraChoice, CaptureKind, Client, FrameReport, MuteMemory, PrewarmClock, ScreenShare, SettingsSection, StreamDefaults,
    StreamFps, StreamJob, StreamResolution, VoicePrefs, camera_preview_mirrored, default_stream_fps,
    default_stream_resolution, discord_shell_columns, either_key_held, encode_jpeg_rgba, encode_jpeg_rgba_quality,
    equal_power_gains, fps_block, hardware_encoder_block, jpeg_quality_for_bitrate, load_json, load_token,
    measure_frame_gaps, mirror_rgba, mix_equal_power, open_mic_should_send, pack_picture, password_blocks_copy,
    password_is_masked, push_to_talk_held, rasterize_lucide, remove_message, resample_mono_i16, resolution_block,
    save_json, save_token, settings_sections, speaking_from_level, stamp_cursor, stream_frame_rates, stream_frame_size,
    stream_interval_ms, stream_max_edge, stream_resolutions, unpack_picture, upsert_message, websocket_url,
};
use banter_server::{
    ChannelKind, ChatTarget, MediaKind, Message, VoiceFlags, decode_media_frame, encode_client_media,
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
    }
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
    assert_eq!(sections, [SettingsSection::Account, SettingsSection::Voice]);
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
    let server = std::thread::spawn(move || banter_server::serve(&serve_addr, &serve_dir));
    let base = format!("http://{addr}");
    let mut ready = false;
    for _ in 0..50 {
        if ureq::get(&format!("{base}/v1/health")).call().is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "server did not open {base}");

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
        .send_message(ChatTarget::Channel(channel.id), "hello", None, &[])
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
fn prewarm_cuts_over_on_a_frame_boundary_without_a_gap() {
    let mut clock = PrewarmClock::new(StreamJob { source: 1, edge: 1280, interval_ms: 33, quality: 75 });
    let mut stamps = Vec::new();
    let mut saw_keyframe = false;
    for now in 0..700 {
        if now == 200 {
            clock.request(StreamJob { source: 2, edge: 1920, interval_ms: 16, quality: 40 });
            clock.prewarm_ready();
        }
        if let Some(frame) = clock.poll(now) {
            if frame.keyframe && frame.job.edge == 1920 {
                saw_keyframe = true;
                assert_eq!(frame.at_ms % 33, 0);
            }
            stamps.push(frame.at_ms);
        }
    }
    assert!(saw_keyframe);
    let report = measure_frame_gaps(&stamps);
    assert_eq!(report.dropped, 0, "gaps {stamps:?}");
    assert!(report.worst_gap_ms <= 33, "worst {}", report.worst_gap_ms);
}

#[test]
fn equal_power_mix_holds_level_and_bitrate_changes_the_jpeg() {
    let (a, b) = equal_power_gains(0.0);
    assert!((a - 1.0).abs() < 0.001 && b.abs() < 0.001);
    let tone = vec![8_000i16; 32];
    let start = mix_equal_power(&tone, &vec![0; 32], 0.0);
    let mid = mix_equal_power(&tone, &tone, 0.5);
    let end = mix_equal_power(&vec![0; 32], &tone, 1.0);
    assert_eq!(start[0], 8_000);
    assert_eq!(end[0], 8_000);
    assert!((mid[0] as i32 - 8_000).abs() < 400);
    assert_eq!(jpeg_quality_for_bitrate(false, 800), 10.max(15));
    assert!(jpeg_quality_for_bitrate(false, 8_000) > jpeg_quality_for_bitrate(false, 1_200));
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
    let wide = encode_jpeg_rgba_quality(64, 36, &{
        let mut big = vec![0u8; 64 * 36 * 4];
        for pixel in big.chunks_exact_mut(4) {
            pixel[0] = 20;
            pixel[3] = 255;
        }
        big
    }, 1920, 80).unwrap();
    let decoded = image::load_from_memory(&wide).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 36));
    let full = vec![12u8; 1920 * 1080 * 4];
    let hd = encode_jpeg_rgba_quality(1920, 1080, &full, 1920, 60).unwrap();
    let hd_image = image::load_from_memory(&hd).unwrap();
    assert_eq!((hd_image.width(), hd_image.height()), (1920, 1080));
    let packet = pack_picture(true, 1920, 1080, &wide);
    let (key, width, height, jpeg) = unpack_picture(&packet);
    assert!(key);
    assert_eq!((width, height), (1920, 1080));
    assert!(jpeg.starts_with(&[0xFF, 0xD8]));
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
    let _ = FrameReport { dropped: 0, worst_gap_ms: 0 };
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
    let server = std::thread::spawn(move || banter_server::serve(&serve_addr, &serve_dir));
    let base = format!("http://{addr}");
    for _ in 0..50 {
        if ureq::get(&format!("{base}/v1/health")).call().is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

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
    ada.join_voice(voice.id).unwrap();
    bea.join_voice(voice.id).unwrap();
    ada.set_flags(VoiceFlags {
        screen: true,
        transmitting: true,
        ..VoiceFlags::default()
    })
    .unwrap();
    bea.set_flags(VoiceFlags::default()).unwrap();

    let mut tone = Vec::new();
    for sample in [4_000i16; 960] {
        tone.extend_from_slice(&sample.to_le_bytes());
    }
    ada_ws.send(tungstenite::Message::Binary(encode_client_media(MediaKind::Audio, &tone).into())).unwrap();

    let mut heard = false;
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if let Ok(tungstenite::Message::Binary(bytes)) = bea_ws.read() {
            if let Ok(frame) = decode_media_frame(&bytes) {
                if frame.kind == MediaKind::Audio && frame.payload.windows(2).any(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) != 0) {
                    heard = true;
                    break;
                }
            }
        }
    }
    assert!(heard, "the second client did not receive microphone audio");

    let mut clock = PrewarmClock::new(StreamJob { source: 7, edge: 1280, interval_ms: 16, quality: 70 });
    let mut arrival = Vec::new();
    let mut walls = Vec::new();
    let wall = std::time::Instant::now();
    for now in (0..400).step_by(16) {
        if now == 160 {
            clock.request(StreamJob { source: 9, edge: 1920, interval_ms: 16, quality: 30 });
            clock.prewarm_ready();
        }
        let Some(frame) = clock.poll(now) else { continue };
        let jpeg = encode_jpeg_rgba_quality(8, 8, &vec![frame.job.quality, 0, 0, 255].repeat(64), frame.job.edge, frame.job.quality).unwrap();
        let packet = pack_picture(frame.keyframe, frame.job.edge, 1, &jpeg);
        ada_ws.send(tungstenite::Message::Binary(encode_client_media(MediaKind::Screen, &packet).into())).unwrap();
        let deadline = wall + Duration::from_millis(now + 80);
        while std::time::Instant::now() < deadline {
            if let Ok(tungstenite::Message::Binary(bytes)) = bea_ws.read() {
                if let Ok(decoded) = decode_media_frame(&bytes) {
                    if decoded.kind == MediaKind::Screen {
                        let (key, edge, _, _) = unpack_picture(&decoded.payload);
                        arrival.push((now, key, edge));
                        // Microseconds so fast machines do not coalesce many frames into one ms stamp.
                        walls.push(wall.elapsed().as_micros() as u64);
                        break;
                    }
                }
            }
        }
    }
    assert!(arrival.len() >= 10, "viewer missed the stream: {arrival:?}");
    assert!(arrival.iter().any(|(_, key, edge)| *key && *edge == 1920));
    let stamps: Vec<u64> = arrival.iter().map(|(at, _, _)| *at).collect();
    let report = measure_frame_gaps(&stamps);
    assert_eq!(report.dropped, 0, "dropped {} worst {} {arrival:?}", report.dropped, report.worst_gap_ms);
    assert!(report.worst_gap_ms <= 16, "worst gap {}", report.worst_gap_ms);
    // Wall time only proves delivery kept up overall. OS scheduling can insert a
    // short hitch that measure_frame_gaps would count as hundreds of drops.
    let span_us = walls.last().copied().unwrap_or(0).saturating_sub(walls.first().copied().unwrap_or(0));
    assert!(
        span_us <= 800_000,
        "viewer wall span {span_us}µs for {} frames {walls:?}",
        walls.len()
    );
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

