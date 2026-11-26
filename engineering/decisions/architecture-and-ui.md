# Architecture, data model, and UI stack

Decided November 26, 2026. This records why Overtone is built as a Tauri desktop app backed by a modular Rust workspace, why musical curation stays manual, and how files, tags, and DJ crates relate.

## Core principles

1. **Human curation stays human.** Sorting a track into the right vibe by listening to it is the core of DJing. Overtone never automates mood, vibe, or crate assignment. It automates the surrounding mechanical work: spectral quality forensics, metadata hygiene, non-duplicating multi-grouping, and multi-platform DJ crate export.
2. **One immutable file per track on disk.** Tracks live in a permanent library layout (for example, flat or date-bucketed folders after triage from an inbox) and are never copied into multiple folders or moved once accepted. Moving or duplicating files bloats storage, splits cue points and play history, and breaks paths inside Serato, Rekordbox, and Traktor.
3. **Symlinks are avoided.** Symbolic links let one file appear in multiple folders, but they break easily across external drives, FAT32/exFAT CDJ USBs, cloud backups, and DJ software path tables.
4. **Portable single source of truth.** Overtone stores track records, quality verdicts, and overlapping vibe groupings in its own local SQLite database (`overtone-library`) and optionally mirrors vibe tags into standard embedded audio tags (`Comments` or `Grouping`, for example `#dub #warmup #hypnotic`) via `lofty`. That makes organization portable even without Overtone, and enables Smart Crates (Serato) and Intelligent Playlists (Rekordbox) as an additional fallback.

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
