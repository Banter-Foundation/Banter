//! Developer server target contracts. Written before the Dev Settings UI and kept as written.

use banter::{
    DevServerPrefs, ServerTarget, StartupLocalPolicy, api_base_for, load_json, local_api_base,
    normalize_remote_base, save_json, settings_sections, startup_api_base, startup_local_policy,
    startup_should_prompt_remote, SettingsSection,
};
use banter_server::DEFAULT_ADDR;

#[test]
fn settings_menu_lists_account_voice_then_developer() {
    assert_eq!(
        settings_sections(),
        [
            SettingsSection::Account,
            SettingsSection::Voice,
            SettingsSection::Mods,
            SettingsSection::Developer,
        ]
    );
}

#[test]
fn local_api_base_matches_default_addr() {
    assert_eq!(local_api_base(), format!("http://{DEFAULT_ADDR}"));
    assert_eq!(local_api_base(), "http://127.0.0.1:3970");
}

#[test]
fn api_base_for_local_ignores_remote_host_text() {
    let prefs = DevServerPrefs {
        target: ServerTarget::Local,
        remote_host: "example.com:9999".into(),
    };
    assert_eq!(api_base_for(&prefs).unwrap(), local_api_base());
}

#[test]
fn api_base_for_remote_requires_a_host() {
    let prefs = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: "   ".into(),
    };
    let err = api_base_for(&prefs).unwrap_err();
    assert!(err.to_lowercase().contains("remote"), "{err}");
}

#[test]
fn normalize_remote_base_accepts_host_port_and_urls() {
    assert_eq!(
        normalize_remote_base("example.com").unwrap(),
        "http://example.com"
    );
    assert_eq!(
        normalize_remote_base("example.com:3970").unwrap(),
        "http://example.com:3970"
    );
    assert_eq!(
        normalize_remote_base("http://example.com/").unwrap(),
        "http://example.com"
    );
    assert_eq!(
        normalize_remote_base("https://chat.example.com:8443/v1").unwrap(),
        "https://chat.example.com:8443"
    );
    assert_eq!(
        normalize_remote_base("127.0.0.1:3971").unwrap(),
        "http://127.0.0.1:3971"
    );
}

#[test]
fn normalize_remote_base_rejects_invalid_input() {
    assert!(normalize_remote_base("").is_err());
    assert!(normalize_remote_base("   ").is_err());
    assert!(normalize_remote_base("http://").is_err());
    assert!(normalize_remote_base("ftp://example.com").is_err());
    assert!(normalize_remote_base("host with spaces").is_err());
    assert!(normalize_remote_base("://missing-scheme").is_err());
}

#[test]
fn dev_server_prefs_round_trip_through_json() {
    let dir = std::env::temp_dir().join(format!(
        "banter-dev-server-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dev-server.json");
    let saved = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: "banter.example:3970".into(),
    };
    save_json(&path, &saved).unwrap();
    let loaded: DevServerPrefs = load_json(&path).expect("dev-server.json loads");
    assert_eq!(loaded.target, ServerTarget::Remote);
    assert_eq!(loaded.remote_host, "banter.example:3970");
    assert_eq!(api_base_for(&loaded).unwrap(), "http://banter.example:3970");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partial_dev_server_json_falls_back_to_local() {
    let partial: DevServerPrefs = serde_json::from_str(r#"{ "remote_host": "kept.example" }"#)
        .expect("an older or partial dev-server.json still loads");
    assert_eq!(partial.target, ServerTarget::Local);
    assert_eq!(partial.remote_host, "kept.example");
    assert_eq!(api_base_for(&partial).unwrap(), local_api_base());
}

#[test]
fn startup_policy_remote_skips_local_even_when_logged_in() {
    let prefs = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: "banter.example:3970".into(),
    };
    assert_eq!(
        startup_local_policy(&prefs, true),
        StartupLocalPolicy::SkipLocalUseRemote
    );
    assert_eq!(
        startup_local_policy(&prefs, false),
        StartupLocalPolicy::SkipLocalUseRemote
    );
}

#[test]
fn startup_policy_logged_in_local_starts_embedded_server() {
    let prefs = DevServerPrefs {
        target: ServerTarget::Local,
        remote_host: String::new(),
    };
    assert_eq!(
        startup_local_policy(&prefs, true),
        StartupLocalPolicy::StartLocal
    );
}

#[test]
fn startup_policy_fresh_local_prefers_running_else_asks_remote() {
    let prefs = DevServerPrefs::default();
    assert_eq!(prefs.target, ServerTarget::Local);
    assert_eq!(
        startup_local_policy(&prefs, false),
        StartupLocalPolicy::PreferRunningLocalElseAskRemote
    );
}

#[test]
fn startup_prompt_remote_when_local_missing_or_remote_invalid() {
    let local = DevServerPrefs::default();
    assert!(
        !startup_should_prompt_remote(&local, true, true),
        "usable local while logged in must not force the remote prompt"
    );
    assert!(
        startup_should_prompt_remote(&local, false, true),
        "logged-in local that failed to start must ask for remote instead of hard-failing"
    );
    assert!(
        startup_should_prompt_remote(&local, false, false),
        "fresh Local with nothing listening must ask for remote"
    );
    assert!(
        !startup_should_prompt_remote(&local, true, false),
        "fresh Local with an already-running loopback server stays on local"
    );

    let remote_ok = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: "174.103.73.97:3970".into(),
    };
    assert!(!startup_should_prompt_remote(&remote_ok, false, true));

    let remote_bad = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: String::new(),
    };
    assert!(startup_should_prompt_remote(&remote_bad, false, false));
}

#[test]
fn startup_api_base_resolves_remote_or_falls_back_to_local_placeholder() {
    let remote = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: "chat.example:3970".into(),
    };
    assert_eq!(
        startup_api_base(&remote),
        "http://chat.example:3970"
    );

    let remote_empty = DevServerPrefs {
        target: ServerTarget::Remote,
        remote_host: String::new(),
    };
    assert_eq!(startup_api_base(&remote_empty), local_api_base());

    let local = DevServerPrefs::default();
    assert_eq!(startup_api_base(&local), local_api_base());
}
