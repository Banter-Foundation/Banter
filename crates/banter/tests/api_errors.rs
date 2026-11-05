//! Client API error surfacing (BA-87 / BA-88). Written before the fix and kept as written.
//! Opaque "request failed" must not be the toast. Voice flag sync failures get a catalog code.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use banter::{
    Client, format_banter_notice, voice_flags_retry_ready, ERR_HTTP_STATUS, ERR_VC_FLAGS, ERR_VC_JOIN,
};
use banter_server::VoiceFlags;

fn serve_once(status_line: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });
    format!("http://{addr}")
}

#[test]
fn empty_http_error_body_is_not_opaque_request_failed() {
    let base = serve_once("HTTP/1.1 502 Bad Gateway", "");
    let client = Client::new(base);
    let err = client.set_flags(VoiceFlags::default()).unwrap_err();
    let text = err.to_string();
    assert!(
        !text.eq_ignore_ascii_case("request failed"),
        "opaque toast: {text}"
    );
    assert!(text.contains(ERR_VC_FLAGS) || text.contains(ERR_HTTP_STATUS), "{text}");
    assert!(text.to_lowercase().contains("502") || text.to_lowercase().contains("voice"), "{text}");
    assert_eq!(err.code(), ERR_VC_FLAGS);
}

#[test]
fn json_error_body_keeps_server_message_and_code() {
    let body = r#"{"error":"that channel is not a voice channel","code":"BANTER_NOT_VOICE_CHANNEL"}"#;
    let base = serve_once("HTTP/1.1 400 Bad Request", body);
    let client = Client::new(base);
    let err = client.join_voice(9).unwrap_err();
    assert_eq!(err.code(), ERR_VC_JOIN);
    assert!(err.message().contains("that channel is not a voice channel"), "{}", err.message());
    assert!(err.to_string().contains("BANTER_NOT_VOICE_CHANNEL") || err.to_string().contains(ERR_VC_JOIN));
    assert_eq!(err.server_code(), Some("BANTER_NOT_VOICE_CHANNEL"));
}

#[test]
fn notice_format_includes_message_and_code() {
    let text = format_banter_notice(ERR_VC_FLAGS, "couldn't update voice state");
    assert!(text.contains("couldn't update voice state"), "{text}");
    assert!(text.contains(ERR_VC_FLAGS), "{text}");
    assert!(!text.eq_ignore_ascii_case("request failed"), "{text}");
}

#[test]
fn voice_flags_retry_backs_off_after_a_failure() {
    let now = Instant::now();
    assert!(voice_flags_retry_ready(None, now, Duration::from_secs(2)));
    assert!(!voice_flags_retry_ready(Some(now), now + Duration::from_millis(200), Duration::from_secs(2)));
    assert!(voice_flags_retry_ready(Some(now), now + Duration::from_secs(2), Duration::from_secs(2)));
}
