//! macOS TCC gates for microphone, camera, and screen capture.
//!
//! A bare `banter` binary without usage-description strings (or without a real
//! `.app` bundle) never appears in System Settings → Privacy & Security, so
//! Screen Recording / Mic / Camera stay denied with no prompt. The release
//! package wraps the binary as `Banter.app` and embeds `macos/Info.plist`.

use std::sync::Once;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Ask for Screen Recording if it is not already granted.
///
/// Returns `Ok(())` when capture is allowed. On denial, returns a short message
/// that points at System Settings (macOS will not keep retrying the prompt).
pub fn ensure_screen_recording() -> Result<(), String> {
    static ASKED: Once = Once::new();
    // SAFETY: Apple documents these as process-wide TCC queries.
    let granted = unsafe { CGPreflightScreenCaptureAccess() };
    if granted {
        return Ok(());
    }
    ASKED.call_once(|| {
        let _ = unsafe { CGRequestScreenCaptureAccess() };
    });
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    Err(
        "Screen Recording is off for Banter. Open System Settings → Privacy & Security → Screen Recording, enable Banter, then quit and reopen Banter."
            .into(),
    )
}

/// Soft mic/camera hint used when opening the device fails on macOS.
pub fn privacy_hint(kind: &str) -> String {
    match kind {
        "microphone" => {
            "Microphone access may be off. Open System Settings → Privacy & Security → Microphone and enable Banter, then quit and reopen Banter.".into()
        }
        "camera" => {
            "Camera access may be off. Open System Settings → Privacy & Security → Camera and enable Banter, then quit and reopen Banter.".into()
        }
        _ => format!(
            "{kind} may need a Privacy & Security grant for Banter. Quit and reopen Banter after changing it."
        ),
    }
}
