# Architecture, data model, and UI stack

Decided November 26, 2026. This records why Overtone is built as a Tauri desktop app backed by a modular Rust workspace, why musical curation stays manual, and how files, tags, and DJ crates relate.

## Core principles

1. **Human curation stays human.** Sorting a track into the right vibe by listening to it is the core of DJing. Overtone never automates mood, vibe, or crate assignment. It automates the surrounding mechanical work: spectral quality forensics, metadata hygiene, non-duplicating multi-grouping, and multi-platform DJ crate export.
2. **One immutable file per track on disk.** Tracks live in a permanent library layout (for example, flat or date-bucketed folders after triage from an inbox) and are never copied into multiple folders or moved once accepted. Moving or duplicating files bloats storage, splits cue points and play history, and breaks paths inside Serato, Rekordbox, and Traktor.
3. **Symlinks are avoided.** Symbolic links let one file appear in multiple folders, but they break easily across external drives, FAT32/exFAT CDJ USBs, cloud backups, and DJ software path tables.
4. **Portable single source of truth.** Overtone stores track records, quality verdicts, and overlapping vibe groupings in its own local SQLite database (`overtone-library`) and optionally mirrors vibe tags into standard embedded audio tags (`Comments` or `Grouping`, for example `#dub #warmup #hypnotic`) via `lofty`. That makes organization portable even without Overtone, and enables Smart Crates (Serato) and Intelligent Playlists (Rekordbox) as an additional fallback.

## Library model

Decided November 26, 2026.

- **Index in place.** Overtone indexes the folders you point it at and never reorganizes them. An optional inbox step can move a new file into a library folder once, at ingest. After that its path is immutable, as principle 2 requires.
- **One database shared by the app and CLI.** The library lives at `library.sqlite` in the app data directory for bundle identifier `com.jacksonferguson.overtone` (`~/Library/Application Support/com.jacksonferguson.overtone/` on macOS, matching Tauri's `app_data_dir()`). The CLI opens the same file and accepts `--library <path>` to use another one.
- **Missing files.** Rescans mark tracks whose files disappeared as missing instead of deleting their records, so vibes and quality reports survive until you relink or remove them.
- **Audio-only content hashes.** Duplicate detection hashes audio content, not whole files, so tag edits do not make a track look new. See [Vibe tags](vibe-tags.md).

## Playback

Decided November 26, 2026. Playback runs in the webview with an HTML `<audio>` element fed through Tauri's asset protocol, scoped at runtime to the library folders. WKWebView plays FLAC, ALAC, AIFF, WAV, MP3, and AAC natively, which avoids a native audio thread and position events over IPC. This replaces the roadmap's original `rodio` / `cpal` plan. Revisit it only if measured seek latency or format coverage falls short.

## TypeScript bindings

Decided November 26, 2026. Rust types that cross the IPC boundary derive `ts_rs::TS` with `#[ts(export)]`, and `cargo test` writes them to `app/src/bindings/` (configured in `.cargo/config.toml`). `TS_RS_LARGE_INT = "number"` maps `i64` and `u64` to `number`, which is exact for SQLite row ids, byte sizes, and millisecond timestamps below 2^53. Commands get thin hand-written wrappers in `app/src/api.ts`. A pre-push hook and CI fail when committed bindings differ from what `cargo test` generates.

`tauri-specta` was spiked and rejected for now: its newest release is `2.0.0-rc.29`, the TypeScript exporter refuses `i64`/`u64` unless every field is annotated or a still-changing runtime BigInt layer is enabled, and full command discovery needs an unreleased Tauri version. Revisit when it ships a stable 2.0.

## Protected paths

Debug builds refuse to write under protected roots, so a development bug cannot touch a real DJ library: `~/Music`, `~/Library/Application Support/Serato`, `~/Library/Pioneer`, and `~/Documents/Native Instruments`. `OVERTONE_PROTECTED_ROOTS` (colon-separated) adds more roots. Every file write in the workspace goes through this check.

## Workspace layout

Shared domain logic lives in Rust library crates under `crates/`, kept independent of both the desktop shell and the CLI:

- `crates/audio` (`overtone-audio`): audio decoding (`symphonia`), windowed FFT spectral quality checks (`rustfft`), waveform peak generation, spectrogram rendering, and embedded metadata read/write (`lofty`). Compiled at `opt-level = 3` even in dev builds so decoding and FFT stay fast during development.
- `crates/library` (`overtone-library`): SQLite database (`rusqlite`), schema migrations, tracks, vibes/crates, quality audit records, and library snapshots.
- `crates/sync-serato` (`overtone-sync-serato`): custom zero-dependency binary TLV reader/writer for Serato `.crate` and subcrate files.
- `crates/sync-rekordbox` (`overtone-sync-rekordbox`): Rekordbox exporter starting with `rekordbox.xml` and extending to guarded `master.db` writes via `rbox`.
- `crates/sync-traktor` (`overtone-sync-traktor`): lossless `quick-xml` streaming patcher for Traktor Pro 3 and 4 `collection.nml`.
- `crates/core` (`overtone-core`): high-level workflows combining ingest, scanning, quality auditing, tag writing, backup creation, and DJ sync.
- `crates/cli` (`overtone-cli`): scriptable CLI companion (`clap`, `comfy-table`, `console`, `indicatif`, `--json` mode) sharing `overtone-core`.
- `app/` (`overtone-app`): Tauri v2 desktop app with React 19 + TypeScript + Vite frontend and thin command wrappers over `overtone-core`.

## Why Tauri + Rust backend

Three UI approaches were evaluated against the core DJ workflow (auditioning tracks, scrubbing waveforms quickly, inspecting spectrograms on flagged files, and assigning overlapping vibe tags at keyboard speed):

- **Terminal app (CLI + `ratatui` TUI):** fastest to build and great for batch automation (`scan`, `check`, `sync`), but poor for clickable waveform scrubbing, crisp spectrogram inspection of flagged tracks, and drag-and-drop. We keep the CLI (`overtone-cli`) for scripting and batch tasks rather than forcing visual audio workflows into a terminal.
- **Pure Rust desktop (`egui` or `iced`):** single-language stack with direct access to audio buffers for canvas rendering, but multi-select tag pills, searchable filterable tables, keyboard focus polish, and macOS native ergonomics require significantly more UI boilerplate than web tech.
- **Tauri v2 + Rust backend (chosen):** best UX-to-effort ratio for a daily-driver music app. HTML/CSS/TypeScript makes dense tables, keyboard-first multi-tag pickers, clickable canvas waveforms, and zoomable spectrogram panels fast to build, while using macOS's system `WKWebView` keeps the bundle small (~10 to 15 MB). Rust handles all decoding, FFT analysis, audio playback, SQLite queries, tagging, and DJ file sync; IPC only passes compact track JSON, playback commands, downsampled waveform peak arrays, and pre-rendered spectrogram images.

## Licensing

The workspace uses `GPL-3.0-only`, matching Parallax and remaining directly compatible with `rbox` (`GPL-3.0-only`), which provides Rekordbox database and XML support.

## Frontend stack

Decided November 27, 2026, after a survey of current packages (versions as of that date):

- **Track table:** `@tanstack/react-virtual` (3.14) for row virtualization, with hand-written sort and selection. The columns are fixed, so `@tanstack/react-table` would add a model layer without benefit. Tests pass `initialRect` and stub `ResizeObserver`, because jsdom has no layout and a virtualizer would otherwise render no rows.
- **Waveform:** a canvas component drawing the 1,200 stored min/max peaks. `wavesurfer.js` wants to own decoding and rendering, which Overtone already does in Rust.
- **Keyboard:** `react-hotkeys-hook` (5.4), which ignores keys typed into form fields by default and has scopes. The command palette and quick-tag picker are hand-written comboboxes; `cmdk` hasn't had a release since August 2025 and the tag picker needs its own ranking and create-new behavior.
- **State:** `zustand` (5.1) stores loaded once at startup and updated by commands and progress channels. Library data is local, so a remote-cache layer such as `@tanstack/react-query` would only add loading states that AGENTS.md rules out.
- **IPC:** long-running commands are `async` and run their blocking work through `tauri::async_runtime::spawn_blocking`, streaming progress over `tauri::ipc::Channel`. Spectrogram PNGs are written to the app cache and served over the asset protocol, so images stay out of the IPC path. Tauri's asset protocol answers `Range` requests, so seeking in large files works; FLAC and AIFF playback in WKWebView is on the [manual verification checklist](../manual-verification.md).

The IPC contract and package split are in the [desktop UI plan](../plans/desktop-ui.md).
