//! Interactive VC latency contracts. Written before the low-latency wiring so the
//! caps and RTP durations stay intentional (favor live call delay over archive quality).

use banter::{
    pcm_backlog_samples, playback_cap_samples, stream_defaults_keyframe_secs, video_rtp_duration_ms,
};

#[test]
fn playback_cap_targets_about_sixty_ms() {
    assert_eq!(playback_cap_samples(48_000), 2_880);
    assert_eq!(playback_cap_samples(44_100), 2_646);
    // Hard floor keeps a 20 ms Opus frame available at low device rates.
    assert_eq!(playback_cap_samples(8_000), 960);
}

#[test]
fn video_rtp_duration_tracks_encode_fps() {
    assert_eq!(video_rtp_duration_ms(30), 33);
    assert_eq!(video_rtp_duration_ms(60), 16);
    assert_eq!(video_rtp_duration_ms(15), 66);
    assert_eq!(video_rtp_duration_ms(120), 8);
    assert_eq!(video_rtp_duration_ms(144), 6);
    assert_eq!(video_rtp_duration_ms(0), 1000);
}

#[test]
fn pcm_backlog_is_two_hundred_ms_not_a_full_second() {
    assert_eq!(pcm_backlog_samples(), 9_600);
}

#[test]
fn default_keyframe_interval_is_two_seconds() {
    assert_eq!(stream_defaults_keyframe_secs(), 2);
}
