//! Local plugin packages live in `<data>/plugins`, beside `<data>/mods`.

use std::fs;

use banter::{ensure_plugin_samples, local_plugins, plugin_file_lines, plugin_server_lines};
use banter_server::zip_files;

#[test]
fn local_plugins_reads_folders_and_zips_next_to_mods() {
    let root = std::env::temp_dir().join(format!("banter-local-plugins-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let plugins = root.join("plugins");
    fs::create_dir_all(plugins.join("hello").join("server")).unwrap();
    fs::create_dir_all(root.join("mods")).unwrap();
    fs::write(
        plugins.join("hello").join("banter.plugin.json"),
        r#"{"id":"hello","name":"Hello","version":"1.0.0","banter":"1","server_entry":"server/main.luau","client":"none","permissions":["storage"]}"#,
    )
    .unwrap();
    fs::write(plugins.join("hello").join("server").join("main.luau"), "banter.storage.set(\"hello\", \"ready\")\n").unwrap();
    fs::write(plugins.join("readme.txt"), "not a plugin").unwrap();
    fs::write(plugins.join("broken.zip"), b"not-a-zip").unwrap();
    let packed = zip_files(&[
        (
            "banter.plugin.json",
            br#"{"id":"packed","name":"Packed","version":"1.0.0","banter":"1","server_entry":"server/main.luau","client":"none","permissions":[]}"#,
        ),
        ("server/main.luau", b"banter.events.on(\"load\", function() end)\n"),
    ])
    .unwrap();
    fs::write(plugins.join("packed.zip"), packed).unwrap();

    let found = local_plugins(&plugins);
    let ids: Vec<&str> = found.iter().map(|plugin| plugin.id.as_str()).collect();
    assert_eq!(ids, ["broken", "hello", "packed"]);
    let hello = found.iter().find(|plugin| plugin.id == "hello").unwrap();
    assert!(hello.failed.is_none());
    assert!(!hello.zip.is_empty());
    assert_eq!(hello.client, "none");
    assert!(found.iter().find(|plugin| plugin.id == "broken").unwrap().failed.is_some());
    assert!(found.iter().find(|plugin| plugin.id == "packed").unwrap().failed.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn plugin_allow_lines_name_server_powers_and_file_access() {
    let lines = plugin_server_lines(&["messages".into(), "events".into(), "io".into()]);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("change messages"));
    assert!(lines.iter().all(|line| !line.contains("files")));
    let files = plugin_file_lines(&["io".into()]);
    assert!(files[0].contains("not responsible"));
    assert!(files[1].contains("this server"));
    assert!(plugin_file_lines(&["messages".into()]).is_empty());
}

#[test]
fn an_empty_data_dir_receives_the_sample_plugins() {
    let root = std::env::temp_dir().join(format!("banter-seed-plugins-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    ensure_plugin_samples(&root);
    let mut ids: Vec<String> = local_plugins(&root.join("plugins")).into_iter().map(|plugin| plugin.id).collect();
    ids.sort();
    assert_eq!(ids, ["notes", "roll", "welcome"]);
    assert!(local_plugins(&root.join("plugins")).iter().all(|plugin| plugin.failed.is_none() && !plugin.zip.is_empty()));
    let _ = fs::remove_dir_all(&root);
}
