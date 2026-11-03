use banter::{current_camera_frame, decode_camera_frame};
use image::codecs::jpeg::JpegEncoder;
use image::ExtendedColorType;
use nokhwa::utils::{FrameFormat, Resolution};
use nokhwa::Buffer;

fn jpeg_bytes(width: u32, height: u32) -> Vec<u8> {
    let rgb: Vec<u8> = (0..width * height).flat_map(|index| [(index * 20) as u8, 128, 255 - (index * 20) as u8]).collect();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .encode(&rgb, width, height, ExtendedColorType::Rgb8)
        .expect("encode jpeg");
    out
}

#[test]
fn mjpeg_frames_decode_without_nokhwas_mjpeg_feature() {
    let buffer = Buffer::new(Resolution::new(8, 6), &jpeg_bytes(8, 6), FrameFormat::MJPEG);
    let (width, height, rgb) = decode_camera_frame(&buffer).expect("MJPEG should decode");
    assert_eq!((width, height), (8, 6));
    assert_eq!(rgb.len(), 8 * 6 * 3);
}

#[test]
fn yuyv_frames_still_decode() {
    let buffer = Buffer::new(Resolution::new(2, 1), &[128, 128, 128, 128], FrameFormat::YUYV);
    let (width, height, rgb) = decode_camera_frame(&buffer).expect("YUYV should decode");
    assert_eq!((width, height), (2, 1));
    assert_eq!(rgb.len(), 2 * 3);
}

#[test]
fn corrupt_mjpeg_is_an_error_not_a_wasm_message() {
    let buffer = Buffer::new(Resolution::new(8, 6), &[0xFF, 0xD8, 0x00, 0x01], FrameFormat::MJPEG);
    let error = decode_camera_frame(&buffer).expect_err("corrupt data must fail");
    assert!(!error.contains("WASM"), "{error}");
}

#[test]
fn frames_from_a_camera_that_is_no_longer_chosen_are_dropped() {
    assert_eq!(current_camera_frame(Some((1, vec![7])), 1), Some(vec![7]));
    assert_eq!(current_camera_frame(Some((0, vec![7])), 1), None);
    assert_eq!(current_camera_frame(None, 1), None);
}
