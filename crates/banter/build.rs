use std::env;
use std::path::PathBuf;

fn main() {
    // Embed usage-description strings so macOS TCC can attribute Mic / Camera /
    // Screen Recording to this binary (bare CLI builds otherwise get silent denies).
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let plist = manifest_dir.join("macos/Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    println!(
        "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        plist.display()
    );
}
