//! BA-92: voice-bar must not show bare WASAPI E_INVALIDARG (0x80070057).
//! Written before the sanitizer / process-loopback format helpers and kept as written.

use banter::{
    device_error_text, media_wasapi_notice, process_loopback_format, voice_bar_error_lines, ERR_VC_LOOPBACK,
};

#[test]
fn invalid_parameter_hresult_becomes_banter_loopback_notice() {
    let raw = "The parameter is incorrect. (0x80070057)";
    let notice = media_wasapi_notice("Brave Browser", raw);
    assert!(notice.contains(ERR_VC_LOOPBACK), "{notice}");
    assert!(notice.to_lowercase().contains("brave") || notice.to_lowercase().contains("audio"), "{notice}");
    assert!(
        !device_error_text(&notice).contains("0x80070057"),
        "voice-bar text must not be the raw HRESULT: {notice}"
    );
}

#[test]
fn voice_bar_strips_prefix_but_keeps_banter_code() {
    let notice = media_wasapi_notice("Brave Browser", "The parameter is incorrect. (0x80070057)");
    let stored = format!("Brave Browser: {notice}");
    let rows = [stored];
    let lines = voice_bar_error_lines(&rows);
    let shown = lines[0];
    assert!(shown.contains(ERR_VC_LOOPBACK), "{shown}");
    assert!(!shown.contains("0x80070057"), "{shown}");
}

#[test]
fn process_loopback_format_matches_microsoft_sample() {
    let format = process_loopback_format();
    assert_eq!(format.channels, 2);
    assert_eq!(format.sample_rate, 44_100);
    assert_eq!(format.bits_per_sample, 16);
    assert_eq!(format.format_tag, 1, "WAVE_FORMAT_PCM");
    assert_eq!(format.block_align, 4);
    assert_eq!(format.avg_bytes_per_sec, 44_100 * 4);
}
