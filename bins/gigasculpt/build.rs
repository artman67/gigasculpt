//! Puts the app's icon into the Windows program, where Explorer and the taskbar read it
//! (tests/icon.rs). The other systems take theirs from the package: the macOS app's
//! Resources, Linux's desktop file.

use std::path::PathBuf;

fn main() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let icon: PathBuf = [dir, "..", "..", "assets", "brand", "icon.ico"]
        .iter()
        .collect();
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let path = icon
            .to_str()
            .unwrap_or_else(|| panic!("{} is not UTF-8", icon.display()));
        winresource::WindowsResource::new()
            .set_icon(path)
            .compile()
            .unwrap_or_else(|e| panic!("cannot put {path} into the program: {e}"));
    }
}
