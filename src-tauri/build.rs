fn main() {
    // Tauri consumes these files while generating the Windows resources, but
    // Cargo does not otherwise know that changing a binary asset must relink
    // the application. Keep icon-only edits from reusing an EXE with stale
    // embedded resources.
    for icon in [
        "icons/32x32.png",
        "icons/128x128.png",
        "icons/128x128@2x.png",
        "icons/icon.ico",
        "icons/icon.icns",
    ] {
        println!("cargo:rerun-if-changed={icon}");
    }
    tauri_build::build()
}
