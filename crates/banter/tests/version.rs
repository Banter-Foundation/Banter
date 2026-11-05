//! Client version-checker contracts. Written before the checker and kept as written.
//! Prefer protocol/min-protocol over equal version strings (stale hosts like pre–PR #19).

use banter_server::{
    check_remote_version, format_version_notice, unreachable_version_check, BanterVersionReport,
    VersionCheck, BANTER_VERSION, ERR_SERVER_TOO_OLD, ERR_VERSION_UNREACHABLE, MIN_SERVER_PROTOCOL,
    PROTOCOL,
};

#[test]
fn client_and_server_share_the_same_build_label_and_protocol() {
    assert_eq!(banter::client_version(), BANTER_VERSION);
    assert_eq!(banter::client_protocol(), PROTOCOL);
    assert_eq!(banter::min_server_protocol(), MIN_SERVER_PROTOCOL);
}

#[test]
fn stale_pre_version_health_json_is_blocked() {
    // Live fingerprint before the version system: {"ok":true} only.
    let report: BanterVersionReport = serde_json::from_str(r#"{"ok":true}"#).expect("parse");
    assert!(report.ok);
    assert!(report.protocol.is_none());
    match check_remote_version(&report) {
        VersionCheck::Block { code, message } => {
            assert_eq!(code, ERR_SERVER_TOO_OLD);
            let notice = format_version_notice(code, &message);
            assert!(notice.contains("BANTER_SERVER_TOO_OLD"), "{notice}");
        }
        other => panic!("expected block for missing protocol, got {other:?}"),
    }
}

#[test]
fn current_health_json_is_compatible() {
    let body = format!(
        r#"{{"ok":true,"version":"{BANTER_VERSION}","protocol":{PROTOCOL},"min_client_protocol":1}}"#
    );
    let report: BanterVersionReport = serde_json::from_str(&body).expect("parse");
    match check_remote_version(&report) {
        VersionCheck::Compatible { warn } => assert!(warn.is_none()),
        other => panic!("expected compatible, got {other:?}"),
    }
}

#[test]
fn unreachable_health_uses_version_unreachable_code() {
    match unreachable_version_check("connection refused") {
        VersionCheck::Block { code, message } => {
            assert_eq!(code, ERR_VERSION_UNREACHABLE);
            let notice = format_version_notice(code, &message);
            assert!(notice.contains(ERR_VERSION_UNREACHABLE), "{notice}");
        }
        other => panic!("expected block, got {other:?}"),
    }
}
