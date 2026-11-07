//! BA-100: Windows Go Live must not surface bare DXGI/WASAPI E_INVALIDARG (0x80070057).
//! Written before the Windows screen-recorder fallback / screen-notice helpers and kept as written.

use banter::{
    device_error_text, media_screen_notice, prefer_still_screen_after_recorder_error,
    process_loopback_format, voice_bar_error_lines, ERR_VC_PUBLISH,
};

#[test]
fn windows_prefers_still_screen_when_recorder_fails() {
    // Linux keeps PipeWire / X11 paths; only Windows falls back after DXGI video_recorder fails.
    assert_eq!(
        prefer_still_screen_after_recorder_error(),
        cfg!(windows),
        "Still/GDI fallback is Windows-only so Linux Cast/X11 stay untouched"
    );
}

#[test]
fn screen_invalid_parameter_becomes_banter_publish_notice() {
    let raw = "The parameter is incorrect. (0x80070057)";
    let notice = media_screen_notice(raw);
    assert!(notice.contains(ERR_VC_PUBLISH), "{notice}");
    assert!(
        notice.to_lowercase().contains("screen") || notice.to_lowercase().contains("capture"),
        "{notice}"
    );
    assert!(
        !device_error_text(&notice).contains("0x80070057"),
        "voice-bar text must not be the raw HRESULT: {notice}"
    );
}

#[test]
fn voice_bar_screen_prefix_strips_but_hides_hresult() {
    let notice = media_screen_notice("The parameter is incorrect. (0x80070057)");
    let stored = format!("screen: {notice}");
    let rows = [stored];
    let lines = voice_bar_error_lines(&rows);
    let shown = lines[0];
    assert!(shown.contains(ERR_VC_PUBLISH), "{shown}");
    assert!(!shown.contains("0x80070057"), "{shown}");
}

#[test]
fn process_loopback_still_matches_microsoft_application_loopback() {
    // BA-92 must stay on main: process loopback never calls GetMixFormat.
    let format = process_loopback_format();
    assert_eq!(format.format_tag, 1, "WAVE_FORMAT_PCM");
    assert_eq!(format.channels, 2);
    assert_eq!(format.sample_rate, 44_100);
    assert_eq!(format.bits_per_sample, 16);
}
