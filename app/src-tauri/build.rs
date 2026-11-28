//! The app's commands are listed here so Tauri enforces its ACL on them.
//! Each one also needs granting in `capabilities/`; a window with no grant
//! can't call it.

const COMMANDS: &[&str] = &[
    "app_status",
    "list_tracks",
    "list_vibes",
    "list_roots",
    "add_root",
    "remove_root",
    "scan_library",
    "audit_tracks",
    "cancel_audit",
    "set_review",
    "track_waveform",
    "render_spectrogram",
    "create_vibe",
    "rename_vibe",
    "delete_vibe",
    "assign_vibe",
    "unassign_vibe",
    "dj_apps_running",
    "plan_serato_sync",
    "apply_serato_sync",
    "plan_rekordbox_export",
    "export_rekordbox_xml",
    "plan_rekordbox_tags",
    "apply_rekordbox_tags",
    "plan_traktor_sync",
    "apply_traktor_sync",
    "plan_tag_mirror",
    "apply_tag_mirror",
    "restore_comments",
    "sync_history",
    "track_details",
    "set_track_field",
    "reset_track_field",
    "list_suggestions",
    "accept_suggestion",
    "reject_suggestion",
    "lookup_settings",
    "set_lookup_settings",
    "lookup_tracks",
    "check_integrity",
    "export_tracks",
    "plan_conversion",
    "convert_tracks",
    "discard_conversion",
    "plan_tag_writes",
    "write_tags",
    "restore_original_tags",
    "damaged_tracks",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));

    tauri_build::try_build(attributes).expect("failed to run the Tauri build script");
}
