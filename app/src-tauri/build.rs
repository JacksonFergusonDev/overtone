//! The app's commands are listed here so Tauri enforces its ACL on them.
//! Each one also needs granting in `capabilities/`; a window with no grant
//! can't call it.

const COMMANDS: &[&str] = &["app_status"];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));

    tauri_build::try_build(attributes).expect("failed to run the Tauri build script");
}
