//! Client mod order, memory warning, and header widgets.

use std::fs;

use banter::{default_mod_order, disable_mod, enable_mod, memory_warning, move_mod, ModHost};

#[test]
fn default_order_is_byte_sorted_and_enable_appends() {
    assert_eq!(default_mod_order(&["clock_bar", "alpha", "beta"]), ["alpha", "beta", "clock_bar"]);
    let mut order = default_mod_order(&["b", "a"]);
    enable_mod(&mut order, "c");
    assert_eq!(order, ["a", "b", "c"]);
    disable_mod(&mut order, "a");
    enable_mod(&mut order, "a");
    assert_eq!(order, ["b", "c", "a"]);
    move_mod(&mut order, "a", true);
    assert_eq!(order, ["b", "a", "c"]);
}

#[test]
fn memory_warning_is_only_above_150() {
    assert!(memory_warning(8).is_none());
    assert!(memory_warning(150).is_none());
    let warning = memory_warning(200).expect("warning");
    assert!(warning.contains("200"));
    assert!(warning.contains("150"));
}

#[test]
fn enabled_mods_paint_the_header_in_order() {
    let root = std::env::temp_dir().join(format!("banter-mods-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("mods").join("zeta")).unwrap();
    fs::create_dir_all(root.join("mods").join("alpha")).unwrap();
    write_mod(&root.join("mods").join("zeta"), "zeta", "Zeta");
    write_mod(&root.join("mods").join("alpha"), "alpha", "Alpha");
    let mut host = ModHost::open(&root);
    host.set_enabled("zeta", true);
    host.set_enabled("alpha", true);
    host.tick();
    let labels: Vec<String> = host.header_widgets().into_iter().map(|widget| widget.label).collect();
    assert_eq!(labels, ["Zeta", "Alpha"]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_empty_data_dir_receives_the_sample_mods() {
    let root = std::env::temp_dir().join(format!("banter-seed-mods-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let host = ModHost::open(&root);
    let mut ids: Vec<String> = host.rows().into_iter().map(|row| row.id).collect();
    ids.sort();
    assert_eq!(ids, ["ping", "purple_glow", "status_chip", "warm_shell"]);
    assert!(host.rows().iter().all(|row| !row.enabled && row.failed.is_none()));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sample_mods_paint_and_tint() {
    let samples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods");
    let root = std::env::temp_dir().join(format!("banter-sample-mods-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for name in ["status_chip", "ping", "warm_shell", "purple_glow"] {
        copy_dir(&samples.join(name), &root.join("mods").join(name));
    }
    let mut host = ModHost::open(&root);
    host.set_enabled("status_chip", true);
    host.set_enabled("ping", true);
    host.set_enabled("warm_shell", true);
    host.tick();
    let labels: Vec<String> = host.header_widgets().into_iter().map(|widget| widget.label).collect();
    assert_eq!(labels, ["Local", "0", "Ping"]);
    assert_eq!(host.shell_color("sidebar_bg"), Some(0x4A3C3C));
    host.set_enabled("purple_glow", true);
    host.tick();
    assert_eq!(host.shell_color("sidebar_bg"), Some(0x1C0A2E));
    assert_eq!(host.shell_color("row_selected"), Some(0x3A2A56));
    assert_eq!(host.shell_color("accent"), Some(0xB24BFF));
    let rows = host.rows();
    assert!(rows.iter().all(|row| row.hard.is_none() && row.failed.is_none() && row.soft.is_empty()));
    let _ = fs::remove_dir_all(&root);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn write_mod(folder: &std::path::Path, id: &str, label: &str) {
    fs::write(
        folder.join("banter.mod.json"),
        format!(
            r#"{{"id":"{id}","name":"{label}","version":"1.0.0","banter":"1","entry":"main.luau","permissions":["ui"]}}"#
        ),
    )
    .unwrap();
    fs::write(
        folder.join("main.luau"),
        format!(
            r#"banter.render(function()
                if banter.ui.begin("stage.header.actions") then
                    banter.ui.text("{label}")
                    banter.ui.close()
                end
            end)"#
        ),
    )
    .unwrap();
}
