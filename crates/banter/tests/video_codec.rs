//! Outbound camera and screen video is I420 then H.264 over WebRTC — not a JPEG frame spray.
//! Local camera preview matches screen preview: RGBA. Device MJPEG capture decode is separate.

use banter::{
    camera_preview_rgba, current_camera_frame, hardware_encoder_available, hardware_encoder_block,
};

#[test]
fn camera_preview_stays_rgba_not_jpeg() {
    let rgba = [255_u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let (width, height, preview) = camera_preview_rgba(2, 2, &rgba).expect("preview");
    assert_eq!((width, height), (2, 2));
    assert_eq!(preview.len(), 2 * 2 * 4);
    assert_ne!(&preview[..2], &[0xFF, 0xD8], "preview must not be a JPEG bitstream");
    assert_eq!(&preview[..4], &[255, 0, 0, 255]);
}

#[test]
fn tagged_camera_preview_ignores_stale_device() {
    let rgba = vec![10_u8; 4 * 4 * 4];
    let kept = current_camera_frame(Some((1, 4, 4, rgba.clone())), 1).expect("matching device");
    assert_eq!(kept, (4, 4, rgba));
    assert!(current_camera_frame(Some((1, 4, 4, vec![1; 64])), 2).is_none());
}

#[test]
fn encoder_block_names_h264_never_jpeg() {
    let available = hardware_encoder_available();
    assert_eq!(hardware_encoder_block(available).is_none(), available);
    let message = hardware_encoder_block(false).expect("software path explains missing hardware");
    assert!(message.contains("H.264"));
    assert!(!message.to_ascii_lowercase().contains("jpeg"));
}
