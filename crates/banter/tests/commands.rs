//! Chat command registry contract (BA-104). Written before the implementation.

use banter::{
    format_me_body, parse_command_line, validate_command_name, validate_command_prefix,
    CommandAction, CommandRegistry, CommandSpec, DEFAULT_COMMAND_PREFIX, SHRUG_FACE,
};

#[test]
fn default_prefix_is_slash() {
    assert_eq!(DEFAULT_COMMAND_PREFIX, "/");
    let registry = banter::default_command_registry();
    assert_eq!(registry.prefix(), "/");
}

#[test]
fn prefix_can_be_changed_and_validated() {
    let mut registry = banter::default_command_registry();
    assert!(registry.set_prefix("!".into()).is_ok());
    assert_eq!(registry.prefix(), "!");
    assert!(validate_command_prefix("").is_err());
    assert!(validate_command_prefix(" ").is_err());
    assert!(validate_command_prefix("hello world").is_err());
    assert!(validate_command_prefix(&"x".repeat(9)).is_err());
    assert!(validate_command_prefix("!!").is_ok());
    assert!(registry.set_prefix(" ".into()).is_err());
}

#[test]
fn parse_uses_configured_prefix_not_hardcoded_slash() {
    assert!(parse_command_line("/me waves", "!").is_none());
    let parsed = parse_command_line("!me waves hello", "!").expect("bang me");
    assert_eq!(parsed.name, "me");
    assert_eq!(parsed.args, "waves hello");
    let shrug = parse_command_line("/shrug", "/").expect("slash shrug");
    assert_eq!(shrug.name, "shrug");
    assert!(shrug.args.is_empty());
    assert!(parse_command_line("me waves", "/").is_none());
    assert!(parse_command_line("/", "/").is_none());
}

#[test]
fn builtins_include_fun_voice_and_join() {
    let registry = banter::default_command_registry();
    for name in ["me", "shrug", "tableflip", "mute", "deafen", "join"] {
        let spec = registry.resolve(name).unwrap_or_else(|| panic!("missing {name}"));
        assert!(!spec.hidden, "{name} should be listed");
    }
    let debug = registry.resolve("debugnow").expect("hidden debug");
    assert!(debug.hidden);
    assert!(matches!(
        registry.resolve("me").map(|s| &s.action),
        Some(CommandAction::Me)
    ));
    assert!(matches!(
        registry.resolve("mute").map(|s| &s.action),
        Some(CommandAction::Mute)
    ));
    assert!(matches!(
        registry.resolve("deafen").map(|s| &s.action),
        Some(CommandAction::Deafen)
    ));
    assert!(matches!(
        registry.resolve("join").map(|s| &s.action),
        Some(CommandAction::Join)
    ));
}

#[test]
fn builtin_and_server_aliases_resolve_to_same_handler() {
    let mut registry = banter::default_command_registry();
    let via_builtin = registry
        .resolve("shruggie")
        .expect("builtin shrug alias")
        .name
        .clone();
    assert_eq!(via_builtin, "shrug");

    registry
        .set_server_aliases(vec![("s".into(), "shrug".into()), ("goto".into(), "join".into())])
        .expect("server aliases");
    assert_eq!(registry.resolve("s").map(|s| s.name.as_str()), Some("shrug"));
    assert_eq!(registry.resolve("goto").map(|s| s.name.as_str()), Some("join"));
    assert_eq!(
        registry.server_aliases(),
        &[("s".into(), "shrug".into()), ("goto".into(), "join".into())]
    );
}

#[test]
fn suggestions_filter_by_partial_and_skip_hidden() {
    let registry = banter::default_command_registry();
    let names: Vec<_> = registry
        .suggestions("m")
        .into_iter()
        .map(|spec| spec.name.as_str())
        .collect();
    assert!(names.contains(&"me"));
    assert!(names.contains(&"mute"));
    assert!(!names.iter().any(|name| *name == "debugnow"));
    assert!(registry.suggestions("zzz").is_empty());
}

#[test]
fn registry_accepts_plugin_style_registration() {
    let mut registry = banter::default_command_registry();
    registry
        .register(CommandSpec {
            name: "roll".into(),
            aliases: vec!["dice".into()],
            description: "Roll a die (plugin stub)".into(),
            args_hint: String::new(),
            hidden: false,
            action: CommandAction::Stub {
                note: "plugin commands run on the server later",
            },
        })
        .expect("register roll");
    assert_eq!(registry.resolve("dice").map(|s| s.name.as_str()), Some("roll"));
    assert!(matches!(
        registry.resolve("roll").map(|s| &s.action),
        Some(CommandAction::Stub { .. })
    ));
    assert!(validate_command_name("Bad Name").is_err());
    assert!(validate_command_name("roll").is_ok());
}

#[test]
fn me_and_shrug_formatters() {
    let body = format_me_body("Owl", "waves");
    assert!(body.contains("Owl"));
    assert!(body.contains("waves"));
    assert!(body.contains("{i"));
    assert!(!SHRUG_FACE.is_empty());
}
