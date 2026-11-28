//! Tauri desktop shell for Overtone.
//!
//! Keep library, audio analysis, tagging, and DJ sync logic in `crates/`;
//! this crate exposes thin Tauri commands and window management.

// Tauri passes command arguments such as `State` by value.
#![allow(clippy::needless_pass_by_value)]

mod commands;
mod error;
pub mod ipc;
mod state;

use tauri::Manager;

use commands::{files, library, metadata, quality, sync, tags, vibes};
pub use error::CommandError;
pub use state::AppState;

/// A builder with every command registered. Each command also needs an
/// entry in `build.rs` and a grant in `capabilities/default.json`.
fn builder<R: tauri::Runtime>() -> tauri::Builder<R> {
    tauri::Builder::<R>::new().invoke_handler(tauri::generate_handler![
        library::app_status,
        library::list_tracks,
        library::list_roots,
        library::add_root,
        library::remove_root,
        library::scan_library,
        quality::audit_tracks,
        quality::cancel_audit,
        quality::set_review,
        quality::track_waveform,
        quality::render_spectrogram,
        vibes::list_vibes,
        vibes::create_vibe,
        vibes::rename_vibe,
        vibes::delete_vibe,
        vibes::assign_vibe,
        vibes::unassign_vibe,
        sync::dj_apps_running,
        sync::plan_serato_sync,
        sync::apply_serato_sync,
        sync::plan_rekordbox_export,
        sync::export_rekordbox_xml,
        sync::plan_rekordbox_tags,
        sync::apply_rekordbox_tags,
        sync::plan_traktor_sync,
        sync::apply_traktor_sync,
        sync::sync_history,
        tags::plan_tag_mirror,
        tags::apply_tag_mirror,
        tags::restore_comments,
        metadata::track_details,
        metadata::set_track_field,
        metadata::reset_track_field,
        metadata::list_suggestions,
        metadata::accept_suggestion,
        metadata::reject_suggestion,
        metadata::lookup_settings,
        metadata::set_lookup_settings,
        metadata::lookup_tracks,
        files::check_integrity,
        files::export_tracks,
        files::plan_conversion,
        files::convert_tracks,
        files::discard_conversion,
        metadata::plan_tag_writes,
        metadata::write_tags,
        metadata::restore_original_tags,
    ])
}

/// The generated app context. Expanded once, because on macOS it embeds the
/// app's `Info.plist` as a symbol.
fn context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

/// Starts the Tauri application.
///
/// # Panics
///
/// Panics if Tauri fails to build or run the desktop application.
pub fn run() {
    builder::<tauri::Wry>()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cache = app
                .path()
                .app_cache_dir()
                .map_err(|err| err.to_string())?
                .join("spectrograms");
            let state = AppState::open(None, cache);
            library::allow_startup_directories(app.handle(), &state);
            app.manage(state);
            Ok(())
        })
        .run(context())
        .expect("error while running Overtone");
}

#[cfg(test)]
mod tests {
    use tauri::WebviewWindowBuilder;
    use tauri::test::{MockRuntime, get_ipc_response};
    use tauri::webview::InvokeRequest;

    use super::*;

    fn request(key: &str, cmd: &str, body: serde_json::Value) -> InvokeRequest {
        InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().expect("url"),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: tauri::http::HeaderMap::default(),
            invoke_key: key.to_string(),
        }
    }

    /// Commands go through Tauri's IPC and ACL as the webview calls them,
    /// so a command missing from `build.rs` or the capability is caught.
    #[test]
    fn commands_are_reachable_through_ipc_and_the_acl() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let state = AppState::open(
            Some(&dir.path().join("lib.sqlite")),
            dir.path().join("cache"),
        );
        // The real context, so the ACL comes from `capabilities/`.
        let context = context::<MockRuntime>();
        let app = builder::<MockRuntime>()
            // The menu needs the main thread, which a test doesn't have.
            .enable_macos_default_menu(false)
            .manage(state)
            .build(context)
            .expect("app");
        let key = app.invoke_key().to_string();
        let window = WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
            .build()
            .expect("window");

        for (cmd, body) in [
            ("app_status", serde_json::json!({})),
            ("list_tracks", serde_json::json!({})),
            ("list_vibes", serde_json::json!({})),
            ("list_roots", serde_json::json!({})),
            ("create_vibe", serde_json::json!({ "path": "Dubs/Deep" })),
            ("sync_history", serde_json::json!({ "limit": 5 })),
            ("dj_apps_running", serde_json::json!({})),
            ("lookup_settings", serde_json::json!({})),
            ("list_suggestions", serde_json::json!({})),
            ("plan_conversion", serde_json::json!({ "trackIds": [] })),
            ("plan_tag_writes", serde_json::json!({ "trackIds": [] })),
        ] {
            let response = get_ipc_response(&window, request(&key, cmd, body));
            assert!(response.is_ok(), "{cmd} failed: {:?}", response.err());
        }
        let vibes = get_ipc_response(&window, request(&key, "list_vibes", serde_json::json!({})))
            .expect("vibes")
            .deserialize::<Vec<serde_json::Value>>()
            .expect("json");
        assert_eq!(vibes.len(), 2, "parent and child vibe");

        let unknown = get_ipc_response(
            &window,
            request(&key, "not_a_command", serde_json::json!({})),
        );
        assert!(unknown.is_err());
    }

    /// Every command listed in `build.rs` is granted to the main window.
    #[test]
    fn every_command_is_granted_in_the_capability() {
        let build = include_str!("../build.rs");
        let list = build
            .split_once("COMMANDS: &[&str] = &[")
            .and_then(|(_, rest)| rest.split_once("];"))
            .expect("COMMANDS list")
            .0;
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).expect("json");
        let granted: Vec<&str> = capability["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        let commands: Vec<&str> = list
            .split(',')
            .map(|item| item.trim().trim_matches('"'))
            .filter(|item| !item.is_empty())
            .collect();
        assert!(commands.len() > 20, "parsed {commands:?}");
        for command in commands {
            let permission = format!("allow-{}", command.replace('_', "-"));
            assert!(
                granted.contains(&permission.as_str()),
                "{permission} is missing from capabilities/default.json"
            );
        }
    }
}
