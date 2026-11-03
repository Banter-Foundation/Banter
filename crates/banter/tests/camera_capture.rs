//! Camera capture resolution contracts. Written before the 720p preference fix and kept as written.
//! Banter must ask the device for ~1280x720 (or the nearest available mode), not the highest
//! resolution then downscale. MJPEG stays preferred when distance ties, so Facecam and Elgato keep working.

use banter::{
    preferred_camera_height, preferred_camera_width, pick_preferred_camera_mode, CameraModeChoice,
};

#[test]
fn preferred_capture_size_is_720p() {
    assert_eq!(preferred_camera_width(), 1280);
    assert_eq!(preferred_camera_height(), 720);
}

#[test]
fn exact_720p_mjpeg_is_chosen_when_present() {
    let modes = [
        CameraModeChoice { width: 1920, height: 1080, fps: 30, mjpeg: true },
        CameraModeChoice { width: 1280, height: 720, fps: 30, mjpeg: true },
        CameraModeChoice { width: 640, height: 480, fps: 30, mjpeg: true },
    ];
    let pick = pick_preferred_camera_mode(&modes).expect("a mode must be chosen");
    assert_eq!((pick.width, pick.height, pick.mjpeg), (1280, 720, true));
}

#[test]
fn nearest_available_mode_is_used_when_720p_is_missing() {
    let modes = [
        CameraModeChoice { width: 1920, height: 1080, fps: 30, mjpeg: true },
        CameraModeChoice { width: 640, height: 480, fps: 30, mjpeg: true },
    ];
    let pick = pick_preferred_camera_mode(&modes).expect("a mode must be chosen");
    assert_eq!((pick.width, pick.height), (1920, 1080), "1080p is nearer to 720p than 480p");
}

#[test]
fn mjpeg_wins_a_tie_at_the_same_resolution() {
    let modes = [
        CameraModeChoice { width: 1280, height: 720, fps: 30, mjpeg: false },
        CameraModeChoice { width: 1280, height: 720, fps: 30, mjpeg: true },
    ];
    let pick = pick_preferred_camera_mode(&modes).expect("a mode must be chosen");
    assert!(pick.mjpeg, "Facecam and Elgato need the MJPEG path");
}

#[test]
fn fps_near_30_breaks_a_resolution_tie() {
    let modes = [
        CameraModeChoice { width: 1280, height: 720, fps: 15, mjpeg: true },
        CameraModeChoice { width: 1280, height: 720, fps: 30, mjpeg: true },
        CameraModeChoice { width: 1280, height: 720, fps: 60, mjpeg: true },
    ];
    let pick = pick_preferred_camera_mode(&modes).expect("a mode must be chosen");
    assert_eq!(pick.fps, 30);
}

#[test]
fn empty_mode_list_returns_none() {
    assert_eq!(pick_preferred_camera_mode(&[]), None);
}
