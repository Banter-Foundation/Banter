//! BA-92: catalog codes for WASAPI / process-loopback device failures.
//! Written before the ERR_VC_LOOPBACK / ERR_VC_DEVICE constants and kept as written.

use banter_server::{BanterIssue, ERR_VC_DEVICE, ERR_VC_LOOPBACK};

#[test]
fn loopback_and_device_codes_are_stable_banter_tokens() {
    for code in [ERR_VC_LOOPBACK, ERR_VC_DEVICE] {
        assert!(code.starts_with("BANTER_"), "{code}");
        assert!(
            code.chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()),
            "{code}"
        );
    }
    assert_eq!(ERR_VC_LOOPBACK, "BANTER_VC_LOOPBACK_FAILED");
    assert_eq!(ERR_VC_DEVICE, "BANTER_VC_DEVICE_FAILED");
}

#[test]
fn loopback_issue_user_text_includes_code_not_raw_hresult() {
    let issue = BanterIssue::new(ERR_VC_LOOPBACK, "couldn't capture audio from Brave Browser")
        .with_detail("The parameter is incorrect. (0x80070057)");
    let text = issue.user_text();
    assert!(text.contains("couldn't capture audio from Brave Browser"), "{text}");
    assert!(text.contains(ERR_VC_LOOPBACK), "{text}");
    assert!(
        !text.contains("0x80070057"),
        "toast must not show the raw HRESULT: {text}"
    );
    assert!(issue.log_text().contains("0x80070057"));
}
