//! Banter error catalog contract. Written before the catalog module and kept as written.
//! Stable codes + short user text; optional detail is for logs, not the toast alone.

use banter_server::{
    BanterError, BanterIssue, ERR_HTTP_STATUS, ERR_HTTP_TRANSPORT, ERR_VC_FLAGS, ERR_VC_JOIN, ERR_VC_LEAVE,
    ERR_VC_PUBLISH, ERR_WS_CONNECT, ERR_WS_DROPPED,
};

#[test]
fn catalog_codes_are_stable_banter_prefixed_tokens() {
    for code in [
        ERR_HTTP_STATUS,
        ERR_HTTP_TRANSPORT,
        ERR_VC_JOIN,
        ERR_VC_LEAVE,
        ERR_VC_FLAGS,
        ERR_VC_PUBLISH,
        ERR_WS_CONNECT,
        ERR_WS_DROPPED,
    ] {
        assert!(code.starts_with("BANTER_"), "{code}");
        assert!(code.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()), "{code}");
    }
}

#[test]
fn issue_user_text_keeps_message_and_code() {
    let issue = BanterIssue::new(ERR_VC_FLAGS, "couldn't update voice state");
    assert_eq!(issue.code(), ERR_VC_FLAGS);
    assert_eq!(issue.message(), "couldn't update voice state");
    assert!(issue.detail().is_none());
    let text = issue.user_text();
    assert!(text.contains("couldn't update voice state"), "{text}");
    assert!(text.contains(ERR_VC_FLAGS), "{text}");
}

#[test]
fn issue_detail_is_optional_and_logged_separately() {
    let issue = BanterIssue::new(ERR_HTTP_STATUS, "voice request failed")
        .with_detail("POST /v1/voice/flags → HTTP 502 empty body");
    assert_eq!(
        issue.detail(),
        Some("POST /v1/voice/flags → HTTP 502 empty body")
    );
    let text = issue.user_text();
    assert!(!text.contains("HTTP 502"), "toast stays short: {text}");
    assert!(issue.log_text().contains("HTTP 502"));
    assert!(issue.log_text().contains(ERR_HTTP_STATUS));
}

#[test]
fn banter_error_codes_match_catalog_style() {
    assert_eq!(BanterError::NotVoiceChannel.code(), "BANTER_NOT_VOICE_CHANNEL");
    assert_eq!(BanterError::NotFound.code(), "BANTER_NOT_FOUND");
    assert_eq!(BanterError::InvalidCredentials.code(), "BANTER_INVALID_CREDENTIALS");
    assert_eq!(BanterError::Storage("disk".into()).code(), "BANTER_STORAGE");
    assert_eq!(BanterError::NotInCall.code(), "BANTER_NOT_IN_CALL");
}

#[test]
fn banter_error_maps_to_an_issue() {
    let issue = BanterIssue::from_server(BanterError::NotVoiceChannel);
    assert_eq!(issue.code(), "BANTER_NOT_VOICE_CHANNEL");
    assert_eq!(issue.message(), "that channel is not a voice channel");
}
