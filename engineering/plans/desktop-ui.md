# Desktop UI plan (Phase 4)

How the Phase 4 desktop app is split into packages, and the IPC contract between the Tauri backend and the React frontend. The roadmap's [Phase 4](implementation-roadmap.md#phase-4-tauri-desktop-listening-and-triage-app) lists the goals; [architecture and UI](../decisions/architecture-and-ui.md) records the stack decisions.

## Packages

1. **Tauri commands** (`app/src-tauri/`, plus small additions to `crates/core` where a workflow is missing): thin commands over `overtone-core`, progress channels, the runtime asset-protocol scope, and the spectrogram cache.
2. **Frontend** (`app/src/`): the library view, player and waveform scrubber, quality inspector, vibe tagging, sync panel, and settings. Built against the mock backend first, so it runs in a browser with `pnpm dev` and in vitest.
3. **Rekordbox XML export** (Phase 3, `crates/sync-rekordbox` and `crates/core/src/sync.rs`). Its Tauri commands are added after it merges.

## IPC contract

Every command returns `Result<T, CommandError>`. Types in `app/src/bindings/` come from ts-rs. Long-running work runs on a blocking thread with its own library connection (SQLite WAL lets the UI keep reading), and streams progress over a `tauri::ipc::Channel`.

| Command | Arguments | Returns |
| --- | --- | --- |
| `app_status` | | `AppStatus` |
| `list_tracks` | | `Track[]` (every track, including missing ones) |
| `list_vibes` | | `Vibe[]` |
| `list_roots` | | `LibraryRoot[]` |
| `add_root` | `path: string` | `LibraryRoot` (also widens the asset scope) |
| `remove_root` | `id: number` | `null` |
| `scan_library` | `refresh: boolean`, `onProgress: Channel<ScanProgress>` | `ScanReport` |
| `audit_tracks` | `trackIds: TrackId[] \| null` (null = pending only), `onProgress: Channel<AuditProgress>` | `AuditReport` |
| `cancel_audit` | | `null` (the running audit stops after the tracks in flight) |
| `set_review` | `trackId`, `review: ReviewStatus` | `Track` |
| `track_waveform` | `trackId` | `WaveformPeaks \| null` (`{ buckets: [min, max][] }`, -1..1) |
| `render_spectrogram` | `trackId`, `log: boolean` | `SpectrogramImage` (`{ path, report }`, PNG in the app cache, reused while the file is unchanged) |
| `create_vibe` | `path: string` | `Vibe` |
| `rename_vibe` | `vibeId`, `newPath: string` | `Vibe` |
| `delete_vibe` | `vibeId` | `null` |
| `assign_vibe` | `trackIds`, `vibeId` | `null` |
| `unassign_vibe` | `trackIds`, `vibeId` | `null` |
| `dj_apps_running` | | `RunningApps` (`{ serato, rekordbox, traktor }`) |
| `plan_serato_sync` | `seratoDir: string \| null` | `SeratoSyncPlan[]` |
| `apply_serato_sync` | `seratoDir: string \| null` | `SeratoSyncReport` (re-plans, then applies) |
| `plan_tag_mirror` | `vibe: string \| null` | `CommentChange[]` |
| `apply_tag_mirror` | `vibe: string \| null` | `MirrorReport` |
| `restore_comments` | | `MirrorReport` |
| `sync_history` | `limit: number` | `SyncRun[]` |
| `plan_rekordbox_export` / `export_rekordbox_xml` | added with Phase 3 | |

Argument names are camelCase on the frontend; Tauri maps them to the Rust snake_case parameters.

Playback uses `convertFileSrc(track.path)` with an `<audio>` element. At startup and in `add_root`, the backend allows each library folder in the asset-protocol scope, so the webview can only read audio inside library folders and the spectrogram cache.
