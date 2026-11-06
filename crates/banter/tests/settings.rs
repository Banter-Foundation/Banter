//! User Settings contracts. Written before the settings fixes and kept as written.

use banter::{
    SettingsSection, SettingsState, StreamDefaults, VideoPrefs, VoicePrefs, account_avatar_actions_origin_x,
    account_avatar_actions_place, clamp_window_size, db_to_rms, display_name_dirty, display_name_error, load_json,
    meter_fraction, normalize_refresh_hz, percent_to_volume, pick_camera, pick_display_refresh_hz, ptt_key,
    ptt_key_names, rms_to_db, save_json, settings_layout, settings_nav_list_w, settings_nav_list_x,
    settings_window_default_size, ui_swap_needs_update, volume_to_percent, wrap_rows, DisplayRefresh,
    DEFAULT_UI_REFRESH_HZ, OPEN_MIC_RMS, SETTINGS_NAV_LIST_INSET, SETTINGS_NAV_LIST_W, SETTINGS_NAV_SECTION_GAP,
    SETTINGS_WINDOW_TITLE,
};
use dear_imgui_rs::Key;

const WIDTHS: [f32; 12] = [320.0, 480.0, 640.0, 720.0, 800.0, 960.0, 1024.0, 1110.0, 1280.0, 1600.0, 1920.0, 3840.0];

#[test]
fn settings_layout_fits_every_window_width_without_overlap() {
    for width in WIDTHS {
        let layout = settings_layout(width, 800.0);
        assert_eq!(layout.nav_x, 0.0, "nav starts at the left edge at {width}");
        assert!(layout.nav_w > 0.0, "nav is visible at {width}");
        assert!((layout.body_x - layout.nav_w).abs() < 0.01, "body starts where nav ends at {width}");
        assert!((layout.body_x + layout.body_w - width).abs() < 0.01, "body reaches the right edge at {width}");
        assert!(layout.content_x >= 0.0, "content starts inside the body at {width}");
        assert!(layout.content_w > 0.0, "content has room at {width}");
        assert!(layout.content_w <= 740.0, "content is capped like Discord at {width}");
        let content_right = layout.body_x + layout.content_x + layout.content_w;
        assert!(layout.close_x >= content_right, "close button sits right of the content at {width}");
        assert!(layout.close_x + layout.close_size <= width, "close button is inside the window at {width}");
    }
}

#[test]
fn settings_layout_keeps_discord_widths_and_centers_on_wide_windows() {
    let layout = settings_layout(1280.0, 800.0);
    assert!(layout.nav_w >= 218.0);
    assert_eq!(layout.content_w, 740.0);
    for width in [1600.0, 1920.0, 3840.0] {
        let layout = settings_layout(width, 1000.0);
        let left_gutter = layout.nav_w - 218.0;
        let right_gutter = width - (layout.close_x + layout.close_size);
        assert!(left_gutter > 0.0, "the nav list is pushed right at {width}");
        assert!(right_gutter > 0.0, "there is space right of the close button at {width}");
        assert!((left_gutter - right_gutter).abs() <= 80.0, "the group is centered at {width}");
    }
}

#[test]
fn settings_layout_close_button_is_inside_short_windows() {
    for height in [200.0, 400.0, 800.0] {
        let layout = settings_layout(1280.0, height);
        assert!(layout.close_y >= 0.0);
        assert!(layout.close_y + layout.close_size <= height);
    }
}

#[test]
fn settings_open_close_and_navigation() {
    let mut state = SettingsState::new();
    assert!(!state.is_open());
    state.open();
    assert!(state.is_open());
    assert_eq!(state.section(), SettingsSection::Account);
    state.select(SettingsSection::Voice);
    assert_eq!(state.section(), SettingsSection::Voice);
    state.close();
    assert!(!state.is_open());
}

#[test]
fn mic_test_stops_when_settings_close_or_the_page_changes() {
    let mut state = SettingsState::new();
    state.open();
    state.select(SettingsSection::Voice);
    state.toggle_mic_test();
    assert!(state.mic_test());
    state.select(SettingsSection::Account);
    assert!(!state.mic_test(), "leaving Voice & Video stops the mic test");

    state.select(SettingsSection::Voice);
    state.toggle_mic_test();
    assert!(state.mic_test());
    state.close();
    assert!(!state.mic_test(), "closing settings stops the mic test");

    state.toggle_mic_test();
    assert!(!state.mic_test(), "the mic test cannot start while settings is closed");
}

#[test]
fn escape_cancels_key_recording_before_closing_settings() {
    let mut state = SettingsState::new();
    state.open();
    state.select(SettingsSection::Voice);
    state.start_recording();
    assert!(state.recording());
    state.escape(false);
    assert!(!state.recording(), "escape stops recording the keybind");
    assert!(state.is_open(), "escape while recording keeps settings open");
    state.escape(false);
    assert!(!state.is_open(), "a second escape closes settings");
}

#[test]
fn escape_closes_an_open_dropdown_first() {
    let mut state = SettingsState::new();
    state.open();
    state.escape(true);
    assert!(state.is_open(), "escape with a dropdown open only closes the dropdown");
    state.escape(false);
    assert!(!state.is_open());
}

#[test]
fn close_and_page_change_stop_key_recording() {
    let mut state = SettingsState::new();
    state.open();
    state.select(SettingsSection::Voice);
    state.start_recording();
    state.select(SettingsSection::Account);
    assert!(!state.recording());
    state.select(SettingsSection::Voice);
    state.start_recording();
    state.close();
    assert!(!state.recording());
}

#[test]
fn volume_percent_round_trips_and_clamps() {
    assert_eq!(volume_to_percent(1.0), 100);
    assert_eq!(volume_to_percent(0.0), 0);
    assert_eq!(volume_to_percent(2.0), 200);
    assert_eq!(volume_to_percent(3.5), 200);
    assert_eq!(volume_to_percent(-1.0), 0);
    assert_eq!(volume_to_percent(0.555), 56);
    assert!((percent_to_volume(100) - 1.0).abs() < 1e-6);
    assert!((percent_to_volume(250) - 2.0).abs() < 1e-6);
    assert!((percent_to_volume(-5) - 0.0).abs() < 1e-6);
    for percent in 0..=200 {
        assert_eq!(volume_to_percent(percent_to_volume(percent)), percent);
    }
}

#[test]
fn sensitivity_decibels_round_trip_and_clamp() {
    assert!((rms_to_db(32768.0) - 0.0).abs() < 0.01);
    assert_eq!(rms_to_db(0.0), -100.0);
    assert_eq!(rms_to_db(-4.0), -100.0);
    assert!(rms_to_db(1.0e9) <= 0.0);
    let open_mic = rms_to_db(OPEN_MIC_RMS);
    assert!(open_mic > -50.0 && open_mic < -40.0, "the open-mic gate is about -45 dB, got {open_mic}");
    for db in [-100.0_f32, -80.0, -60.0, -45.0, -20.0, -3.0, 0.0] {
        let back = rms_to_db(db_to_rms(db));
        assert!((back - db).abs() < 0.05, "{db} dB came back as {back}");
    }
}

#[test]
fn meter_fraction_is_monotonic_and_matches_the_threshold_mark() {
    let mut last = -1.0;
    for rms in [0.0, 1.0, 10.0, 100.0, 1000.0, 10000.0, 32768.0] {
        let fraction = meter_fraction(rms);
        assert!((0.0..=1.0).contains(&fraction));
        assert!(fraction >= last);
        last = fraction;
    }
    assert_eq!(meter_fraction(0.0), 0.0);
    assert!((meter_fraction(32768.0) - 1.0).abs() < 1e-4);
    let threshold = db_to_rms(-50.0);
    assert!((meter_fraction(threshold) - 0.5).abs() < 0.01);
}

#[test]
fn push_to_talk_keys_cover_letters_digits_and_function_keys() {
    let names = ptt_key_names();
    for letter in 'A'..='Z' {
        assert!(names.contains(&letter.to_string().as_str()), "{letter} can be bound");
    }
    for digit in 0..=9 {
        assert!(names.contains(&digit.to_string().as_str()), "{digit} can be bound");
    }
    for function in 1..=12 {
        assert!(names.contains(&format!("F{function}").as_str()), "F{function} can be bound");
    }
    assert!(!names.contains(&"Escape"), "escape cancels recording and is never bound");
    let mut unique = names.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "key names are unique");
    for name in names {
        assert!(ptt_key(name).is_some(), "{name} maps to a key");
    }
    assert_eq!(ptt_key("V"), Some(Key::V));
    assert_eq!(ptt_key("F5"), Some(Key::F5));
    assert_eq!(ptt_key("7"), Some(Key::Key7));
    assert_eq!(ptt_key("Escape"), None);
    assert_eq!(ptt_key(""), None);
}

#[test]
fn voice_prefs_keep_saved_values_when_a_file_has_missing_fields() {
    let partial: VoicePrefs = serde_json::from_str(r#"{ "input_volume": 0.5, "push_to_talk": true }"#)
        .expect("an older or partial voice.json still loads");
    assert_eq!(partial.input_volume, 0.5);
    assert!(partial.push_to_talk);
    let defaults = VoicePrefs::default();
    assert_eq!(partial.output_volume, defaults.output_volume);
    assert_eq!(partial.ptt_key, "V");
    assert_eq!(partial.camera, "");
}

#[test]
fn stream_defaults_keep_saved_values_when_a_file_has_missing_fields() {
    let partial: StreamDefaults = serde_json::from_str(r#"{ "resolution": "1080p" }"#)
        .expect("an older or partial stream.json still loads");
    assert_eq!(partial.resolution, "1080p");
    assert_eq!(partial.fps, StreamDefaults::default().fps);
    assert!(partial.show_cursor);
}

#[test]
fn video_prefs_keep_saved_values_when_a_file_has_missing_fields() {
    let partial: VideoPrefs = serde_json::from_str(r#"{ "vsync": false, "width": 1920 }"#)
        .expect("an older or partial video.json still loads");
    assert!(!partial.vsync);
    assert_eq!(partial.width, 1920);
    let defaults = VideoPrefs::default();
    assert_eq!(partial.height, defaults.height);
    assert_eq!(partial.fullscreen, defaults.fullscreen);
}

#[test]
fn every_video_setting_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("banter-settings-video-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("video.json");
    let saved = VideoPrefs {
        vsync: false,
        fullscreen: true,
        width: 1920,
        height: 1080,
    };
    save_json(&path, &saved).unwrap();
    let loaded: VideoPrefs = load_json(&path).expect("video.json loads");
    assert_eq!(loaded, saved);
    assert_eq!(clamp_window_size(100, 50), (640, 480));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_refresh_follows_the_display_under_the_window() {
    let left = DisplayRefresh {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        refresh_hz: 60,
    };
    let right = DisplayRefresh {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1440,
        refresh_hz: 144,
    };
    let displays = [left, right];
    assert_eq!(pick_display_refresh_hz(100, 100, None, &displays), 60);
    assert_eq!(pick_display_refresh_hz(2000, 100, None, &displays), 144);
    assert_eq!(
        pick_display_refresh_hz(100, 100, Some(165), &displays),
        165,
        "fullscreen uses the active monitor mode, not the window center"
    );
    assert_eq!(normalize_refresh_hz(0), DEFAULT_UI_REFRESH_HZ);
    assert_eq!(pick_display_refresh_hz(0, 0, None, &[]), DEFAULT_UI_REFRESH_HZ);
    assert_eq!(
        pick_display_refresh_hz(-50, -50, None, &displays),
        60,
        "off-screen falls back to the first connected display"
    );
}

#[test]
fn ui_swap_interval_reapplies_when_monitor_or_vsync_changes() {
    assert!(ui_swap_needs_update(None, true, 60));
    assert!(!ui_swap_needs_update(Some((true, 60)), true, 60));
    assert!(ui_swap_needs_update(Some((true, 60)), true, 144));
    assert!(ui_swap_needs_update(Some((true, 144)), false, 144));
    assert!(!ui_swap_needs_update(Some((false, 144)), false, 144));
}

#[test]
fn every_voice_setting_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("banter-settings-voice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("voice.json");
    let saved = VoicePrefs {
        input_device: "USB Mic".into(),
        output_device: "Headphones".into(),
        input_volume: 1.4,
        output_volume: 0.6,
        push_to_talk: true,
        sensitivity: 900.0,
        auto_sensitivity: false,
        release_ms: 640,
        noise_suppression: false,
        echo_cancellation: false,
        auto_gain: true,
        ptt_key: "F5".into(),
        camera: "Front Camera".into(),
        guild_p2p_video: true,
    };
    save_json(&path, &saved).unwrap();
    let loaded: VoicePrefs = load_json(&path).expect("voice.json loads");
    assert_eq!(loaded, saved);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn camera_choice_prefers_the_saved_name() {
    let cameras = vec![(0, "Built-in".to_string()), (2, "Front Camera".to_string()), (5, "Capture Card".to_string())];
    assert_eq!(pick_camera(&cameras, "Front Camera", 0), Some(2));
    assert_eq!(pick_camera(&cameras, "Unplugged", 5), Some(5), "a missing saved camera keeps the current one");
    assert_eq!(pick_camera(&cameras, "", 9), Some(0), "a gone current camera falls back to the first");
    assert_eq!(pick_camera(&[], "Front Camera", 0), None);
}

#[test]
fn chips_wrap_onto_rows_that_fit() {
    let rows = wrap_rows(&[60.0, 60.0, 60.0, 60.0], 200.0, 8.0);
    assert_eq!(rows, vec![0, 0, 0, 1]);
    let rows = wrap_rows(&[300.0, 50.0], 200.0, 8.0);
    assert_eq!(rows, vec![0, 1], "an item wider than the row gets its own row");
    let rows = wrap_rows(&[], 200.0, 8.0);
    assert!(rows.is_empty());
    let widths = [70.0, 55.0, 90.0, 40.0, 120.0, 65.0, 30.0];
    let rows = wrap_rows(&widths, 180.0, 8.0);
    let last = *rows.last().unwrap();
    for row in 0..=last {
        let items: Vec<f32> = widths.iter().zip(&rows).filter(|(_, r)| **r == row).map(|(w, _)| *w).collect();
        let used: f32 = items.iter().sum::<f32>() + 8.0 * (items.len() as f32 - 1.0);
        assert!(items.len() == 1 || used <= 180.0, "row {row} fits");
    }
    assert!(rows.windows(2).all(|pair| pair[1] >= pair[0] && pair[1] - pair[0] <= 1));
}

#[test]
fn display_name_rules_match_the_server() {
    assert_eq!(display_name_error("Owl"), None);
    assert_eq!(display_name_error("  Owl  "), None);
    assert!(display_name_error("").is_some());
    assert!(display_name_error("   ").is_some());
    assert!(display_name_error(&"x".repeat(33)).is_some());
    assert_eq!(display_name_error(&"x".repeat(32)), None);
    assert!(display_name_error("bad\nname").is_some());
    assert!(!display_name_dirty("Owl", "Owl"));
    assert!(!display_name_dirty(" Owl ", "Owl"));
    assert!(display_name_dirty("Snow Owl", "Owl"));
}

#[test]
fn account_avatar_actions_stay_inside_the_card() {
    // Change Avatar + Remove Avatar must right-align as one group inside the card,
    // like Discord. Positioning only Change left Remove clipped past the right edge.
    let card_x = 100.0_f32;
    let card_w = 500.0_f32;
    let pad = 16.0_f32;
    let change_w = 130.0_f32;
    let remove_w = 130.0_f32;
    let gap = 8.0_f32;
    let old_x = card_x + card_w - pad - change_w;
    assert!(
        old_x + change_w + gap + remove_w > card_x + card_w - pad,
        "the old single-button origin overflows Remove Avatar"
    );
    let x = account_avatar_actions_origin_x(card_x, card_w, pad, change_w, remove_w, gap);
    let remove_right = x + change_w + gap + remove_w;
    assert!(
        (remove_right - (card_x + card_w - pad)).abs() < 0.01,
        "action group is right-aligned with card padding"
    );
    assert!(x >= card_x + pad - 0.01, "actions do not start left of card padding");
    assert!(
        remove_right <= card_x + card_w - pad + 0.01,
        "Remove Avatar stays inside the card"
    );
}

#[test]
fn settings_opens_in_its_own_os_window_and_clears_cleanly() {
    let mut state = SettingsState::new();
    assert!(!state.os_window_desired());
    assert!(!state.take_needs_focus());

    state.open();
    assert!(state.is_open());
    assert!(state.os_window_desired(), "open User Settings means a Banter Settings OS window");
    assert!(state.take_needs_focus(), "first open asks the platform to focus the settings window");
    assert!(!state.take_needs_focus(), "focus request is one-shot");

    state.select(SettingsSection::Voice);
    state.start_recording();
    state.toggle_mic_test();
    assert!(state.recording());
    assert!(state.mic_test());

    state.window_closed();
    assert!(!state.is_open());
    assert!(!state.os_window_desired(), "closing the OS window clears settings host desire");
    assert!(!state.recording(), "window close must not leave keybind recording stuck");
    assert!(!state.mic_test(), "window close must not leave mic test stuck");
    assert!(!state.take_needs_focus());
}

#[test]
fn settings_ui_close_and_escape_also_drop_os_window_desire() {
    // Platform destroys the shared GLFW window when desire goes false. If desire sticks
    // true, Banter keeps a host that may already be a glfw-rs zombie (is_shared Drop leak).
    let mut state = SettingsState::new();
    state.open_section(SettingsSection::Account);
    assert!(state.os_window_desired());

    state.close();
    assert!(!state.os_window_desired(), "in-UI X must drop Banter Settings OS window desire");

    state.open();
    state.escape(false);
    assert!(!state.os_window_desired(), "Escape must drop Banter Settings OS window desire");

    state.open();
    state.escape(true);
    assert!(
        state.os_window_desired(),
        "Escape with a popup open must not close Settings (dropdown closes first)"
    );
}

#[test]
fn settings_open_section_targets_my_account_voice_or_developer() {
    let mut state = SettingsState::new();
    state.open_section(SettingsSection::Developer);
    assert!(state.os_window_desired());
    assert_eq!(state.section(), SettingsSection::Developer);
    assert!(state.take_needs_focus());

    state.close();
    state.open_section(SettingsSection::Account);
    assert_eq!(state.section(), SettingsSection::Account);
    assert!(state.os_window_desired());
}

#[test]
fn settings_window_title_and_default_size_fit_the_discord_like_layout() {
    assert_eq!(SETTINGS_WINDOW_TITLE, "Banter Settings");
    let (width, height) = settings_window_default_size();
    assert!(width >= 960, "wide enough for nav + content + close");
    assert!(height >= 640);
    let layout = settings_layout(width as f32, height as f32);
    assert!(layout.nav_w >= SETTINGS_NAV_LIST_W);
    assert_eq!(layout.content_w, 740.0);
    assert!(layout.close_x + layout.close_size <= width as f32);
}

#[test]
fn settings_nav_list_sits_inside_the_nav_column_with_inset() {
    for nav_w in [180.0_f32, 218.0, 300.0, 400.0] {
        let list_x = settings_nav_list_x(nav_w);
        let list_w = settings_nav_list_w(nav_w);
        assert!(list_x >= SETTINGS_NAV_LIST_INSET - 0.01);
        assert!(list_w >= 32.0);
        assert!(list_x + list_w <= nav_w + 0.01, "nav list stays inside the column at {nav_w}");
    }
    assert!(SETTINGS_NAV_SECTION_GAP >= 8.0, "section blocks need clear separation");
}

#[test]
fn account_avatar_actions_stack_when_the_card_is_too_narrow_for_the_name() {
    let card_x = 40.0_f32;
    let card_y = 100.0_f32;
    let card_w = 320.0_f32;
    let banner_h = 100.0_f32;
    let pad = 16.0_f32;
    let name_left = card_x + 124.0;
    let name_reserve = 160.0_f32;
    let change_w = 130.0_f32;
    let remove_w = 130.0_f32;
    let gap = 8.0_f32;
    let place = account_avatar_actions_place(
        card_x,
        card_y,
        card_w,
        banner_h,
        pad,
        name_left,
        name_reserve,
        change_w,
        remove_w,
        gap,
    );
    assert!(place.stacked, "narrow cards stack avatar actions under the name");
    assert!(place.x >= card_x + pad - 0.01);
    assert!(place.y > card_y + banner_h + 14.0);
    let remove_right = place.x + change_w + gap + remove_w;
    assert!(
        remove_right <= card_x + card_w - pad + 0.01 || place.stacked,
        "stacked actions still prefer staying in the card"
    );

    let wide = account_avatar_actions_place(
        card_x,
        card_y,
        740.0,
        banner_h,
        pad,
        name_left,
        80.0,
        change_w,
        remove_w,
        gap,
    );
    assert!(!wide.stacked, "wide cards keep Change/Remove on the name row");
    assert!((wide.y - (card_y + banner_h + 14.0)).abs() < 0.01);
}
