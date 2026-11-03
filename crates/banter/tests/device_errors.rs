//! Device error contracts for the Voice & Video page. Written before the fix and kept as written.

use banter::{forget_device_errors, friendly_device_error, remember_device_error};

#[test]
fn device_errors_are_listed_once() {
    let mut list = Vec::new();
    remember_device_error(&mut list, "camera", "Banter could not use this camera.");
    remember_device_error(&mut list, "camera", "Banter could not use this camera.");
    assert_eq!(list.len(), 1);
}

#[test]
fn a_working_device_clears_only_its_own_errors() {
    let mut list = Vec::new();
    remember_device_error(&mut list, "camera", "Banter could not use this camera.");
    remember_device_error(&mut list, "microphone", "Banter could not open this microphone.");
    forget_device_errors(&mut list, "camera");
    assert_eq!(list.len(), 1);
    assert!(list[0].contains("microphone"));
    forget_device_errors(&mut list, "microphone");
    assert!(list.is_empty());
}

#[test]
fn forgetting_a_device_does_not_match_a_longer_name() {
    let mut list = Vec::new();
    remember_device_error(&mut list, "camera2", "other");
    forget_device_errors(&mut list, "camera");
    assert_eq!(list.len(), 1);
}

#[test]
fn settings_devices_get_plain_messages_instead_of_library_text() {
    for device in ["camera", "microphone", "speakers"] {
        let message = friendly_device_error(device).expect("settings devices have a message");
        assert!(!message.is_empty());
        assert!(!message.contains("WASM"));
        assert!(message.ends_with('.'));
    }
    assert!(friendly_device_error("camera").unwrap().contains("camera"));
    assert!(friendly_device_error("microphone").unwrap().contains("microphone"));
    assert!(friendly_device_error("speakers").unwrap().contains("speakers"));
    assert_eq!(friendly_device_error("screen"), None);
}
