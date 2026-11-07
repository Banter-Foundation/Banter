//! Vendored UI (Dongle) + chat (Shantell Sans) font assets.

fn assert_ttf(path: &str, bytes: &[u8]) {
    assert!(
        bytes.len() > 1024,
        "{path} is too small ({})",
        bytes.len()
    );
    let magic = &bytes[..4];
    assert!(
        magic == b"\x00\x01\x00\x00" || magic == b"OTTO" || magic == b"true",
        "{path} is not a TrueType/OpenType face (magic {magic:?})"
    );
}

#[test]
fn dongle_ui_faces_are_vendored() {
    assert_ttf(
        "Dongle-Regular",
        include_bytes!("../assets/fonts/dongle/Dongle-Regular.ttf"),
    );
    assert_ttf(
        "Dongle-Bold",
        include_bytes!("../assets/fonts/dongle/Dongle-Bold.ttf"),
    );
    assert_ttf(
        "Dongle-Light",
        include_bytes!("../assets/fonts/dongle/Dongle-Light.ttf"),
    );
    let ofl = include_str!("../assets/fonts/dongle/OFL.txt");
    assert!(ofl.contains("SIL OPEN FONT LICENSE"));
}

#[test]
fn shantell_chat_faces_are_vendored() {
    assert_ttf(
        "ShantellSans-Regular",
        include_bytes!("../assets/fonts/shantell-sans/ShantellSans-Regular.ttf"),
    );
    assert_ttf(
        "ShantellSans-Bold",
        include_bytes!("../assets/fonts/shantell-sans/ShantellSans-Bold.ttf"),
    );
    assert_ttf(
        "ShantellSans-Italic",
        include_bytes!("../assets/fonts/shantell-sans/ShantellSans-Italic.ttf"),
    );
    assert_ttf(
        "ShantellSans-BoldItalic",
        include_bytes!("../assets/fonts/shantell-sans/ShantellSans-BoldItalic.ttf"),
    );
    let ofl = include_str!("../assets/fonts/shantell-sans/OFL.txt");
    assert!(ofl.contains("SIL OPEN FONT LICENSE"));
}
