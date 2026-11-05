//! Banter version / protocol contract. Written before the version module and kept as written.
//!
//! Health must expose a Banter release label plus a numeric protocol so clients can
//! detect a host that is too old (e.g. pre–PR #19 SFU) without relying on equal strings.

use banter_server::{
    check_remote_version, health_payload, BanterVersionReport, ERR_CLIENT_TOO_OLD, ERR_SERVER_TOO_OLD,
    ERR_VERSION_UNREACHABLE, BANTER_VERSION, MIN_CLIENT_PROTOCOL, MIN_SERVER_PROTOCOL, PROTOCOL,
    VersionCheck,
};

#[test]
fn banter_version_matches_release_label_shape() {
    assert!(
        BANTER_VERSION.starts_with("Version-"),
        "expected Version-* label, got {BANTER_VERSION}"
    );
    assert!(!BANTER_VERSION.is_empty());
}

#[test]
fn protocol_constants_are_positive_and_ordered() {
    assert!(PROTOCOL >= 1, "current protocol must be at least 1 once the version system ships");
    assert!(MIN_SERVER_PROTOCOL >= 1);
    assert!(MIN_CLIENT_PROTOCOL >= 1);
    assert!(
        PROTOCOL >= MIN_CLIENT_PROTOCOL,
        "this build cannot require a client newer than itself"
    );
    assert!(
        PROTOCOL >= MIN_SERVER_PROTOCOL,
        "this build cannot require a server newer than itself"
    );
}

#[test]
fn catalog_codes_are_stable_banter_prefixed_tokens() {
    for code in [ERR_SERVER_TOO_OLD, ERR_CLIENT_TOO_OLD, ERR_VERSION_UNREACHABLE] {
        assert!(code.starts_with("BANTER_"), "{code}");
        assert!(
            code.chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()),
            "{code}"
        );
    }
}

#[test]
fn health_payload_includes_ok_version_and_protocol() {
    let value = health_payload();
    assert_eq!(value["ok"], true);
    assert_eq!(value["version"], BANTER_VERSION);
    assert_eq!(value["protocol"], PROTOCOL);
    assert_eq!(value["min_client_protocol"], MIN_CLIENT_PROTOCOL);
}

#[test]
fn missing_protocol_means_server_too_old() {
    let report = BanterVersionReport {
        ok: true,
        version: None,
        protocol: None,
        min_client_protocol: None,
    };
    match check_remote_version(&report) {
        VersionCheck::Block { code, message } => {
            assert_eq!(code, ERR_SERVER_TOO_OLD);
            assert!(
                message.to_lowercase().contains("old") || message.to_lowercase().contains("upgrade"),
                "{message}"
            );
        }
        other => panic!("expected block, got {other:?}"),
    }
}

#[test]
fn protocol_below_min_server_blocks() {
    let report = BanterVersionReport {
        ok: true,
        version: Some("Version-1-test-4".into()),
        protocol: Some(MIN_SERVER_PROTOCOL.saturating_sub(1)),
        min_client_protocol: Some(1),
    };
    match check_remote_version(&report) {
        VersionCheck::Block { code, .. } => assert_eq!(code, ERR_SERVER_TOO_OLD),
        other => panic!("expected block, got {other:?}"),
    }
}

#[test]
fn server_min_client_above_local_blocks() {
    let report = BanterVersionReport {
        ok: true,
        version: Some(BANTER_VERSION.into()),
        protocol: Some(PROTOCOL),
        min_client_protocol: Some(PROTOCOL + 10),
    };
    match check_remote_version(&report) {
        VersionCheck::Block { code, .. } => assert_eq!(code, ERR_CLIENT_TOO_OLD),
        other => panic!("expected block, got {other:?}"),
    }
}

#[test]
fn matching_protocol_is_compatible_even_when_labels_differ() {
    let report = BanterVersionReport {
        ok: true,
        version: Some("Version-1-test-99".into()),
        protocol: Some(PROTOCOL),
        min_client_protocol: Some(MIN_CLIENT_PROTOCOL),
    };
    match check_remote_version(&report) {
        VersionCheck::Compatible { warn } => {
            assert!(warn.is_some(), "different labels should warn");
            let text = warn.unwrap();
            assert!(text.contains("Version-1-test-99"), "{text}");
            assert!(text.contains(BANTER_VERSION), "{text}");
        }
        other => panic!("expected compatible, got {other:?}"),
    }
}

#[test]
fn identical_build_is_compatible_without_warn() {
    let report = BanterVersionReport {
        ok: true,
        version: Some(BANTER_VERSION.into()),
        protocol: Some(PROTOCOL),
        min_client_protocol: Some(MIN_CLIENT_PROTOCOL),
    };
    match check_remote_version(&report) {
        VersionCheck::Compatible { warn } => assert!(warn.is_none()),
        other => panic!("expected compatible, got {other:?}"),
    }
}

#[test]
fn user_text_includes_banter_code() {
    let report = BanterVersionReport {
        ok: true,
        version: None,
        protocol: None,
        min_client_protocol: None,
    };
    let VersionCheck::Block { code, message } = check_remote_version(&report) else {
        panic!("expected block");
    };
    let notice = banter_server::format_version_notice(code, &message);
    assert!(notice.contains(code), "{notice}");
    assert!(notice.contains(&message), "{notice}");
    assert_eq!(notice, format!("{message} ({code})"));
}
