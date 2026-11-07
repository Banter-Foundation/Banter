//! Encoder quality-per-bitrate / CPU contracts for VC + screen share.
//!
//! Written before the encoder improvements so the splits, QP floors, probe cache,
//! and HW-first open stay intentional. Must not undo BA-97 interactive latency
//! (bitrate RC, Low complexity, single thread, 1 s keyframes, screen AQ/BG off).

use banter::{
    hardware_encoder_available, hardware_probe, layer_target_bps, openh264_qp_bounds,
    stream_defaults_keyframe_secs,
};
use banter_server::LayerId;

#[test]
fn default_keyframe_stays_interactive_one_second() {
    // BA-97 / PR #85 — do not stretch GOP for archive quality.
    assert_eq!(stream_defaults_keyframe_secs(), 1);
}

#[test]
fn lower_spatial_layers_get_more_than_area_share() {
    // Full keeps the configured target. Half/Quarter used to track pixel area
    // (1/4 and 1/16), which left mid/thumbnail layers soft under dynacast.
    let full = 2_500_000;
    assert_eq!(layer_target_bps(full, LayerId::Full), full);
    assert_eq!(layer_target_bps(full, LayerId::Half), (full / 3).max(100_000));
    assert_eq!(layer_target_bps(full, LayerId::Quarter), (full / 10).max(50_000));
    assert_eq!(layer_target_bps(full, LayerId::Off), 0);
    assert!(layer_target_bps(full, LayerId::Half) > full / 4);
    assert!(layer_target_bps(full, LayerId::Quarter) > full / 16);
}

#[test]
fn layer_floors_keep_tiny_manual_bitrates_usable() {
    assert_eq!(layer_target_bps(200_000, LayerId::Half), 100_000);
    assert_eq!(layer_target_bps(200_000, LayerId::Quarter), 50_000);
}

#[test]
fn screen_qp_is_tighter_than_camera_for_text() {
    let (cam_min, cam_max) = openh264_qp_bounds(false);
    let (scr_min, scr_max) = openh264_qp_bounds(true);
    assert!(cam_min <= cam_max && cam_max <= 51);
    assert!(scr_min <= scr_max && scr_max <= 51);
    // Screen max QP must stay below camera so UI text does not mush under RC stress.
    assert!(scr_max < cam_max);
    // Both raise min QP above 0 so bits are not wasted on imperceptible detail.
    assert!(cam_min >= 12);
    assert!(scr_min >= 10);
}

#[test]
fn hardware_probe_is_stable_across_calls() {
    let first = hardware_probe();
    let second = hardware_probe();
    assert_eq!(first, second);
    assert_eq!(hardware_encoder_available(), !first.is_empty());
}
