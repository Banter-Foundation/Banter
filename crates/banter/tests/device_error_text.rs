use banter::{device_error_text, friendly_device_error, remember_device_error};

#[test]
fn stored_device_errors_display_without_the_device_prefix() {
    let mut list = Vec::new();
    let message = friendly_device_error("camera").expect("camera message");
    remember_device_error(&mut list, "camera", message);
    assert_eq!(device_error_text(&list[0]), message);
}

#[test]
fn unprefixed_lines_display_unchanged() {
    assert_eq!(device_error_text("Something went wrong."), "Something went wrong.");
}
