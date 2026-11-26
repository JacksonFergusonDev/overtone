# Implementation roadmap

Phased delivery plan for Overtone. Complete and test each phase with fixtures and `tempfile::TempDir` before moving to the next. Record user-visible changes under **Unreleased** in [CHANGELOG.md](../../CHANGELOG.md) and update [PROGRESS.md](../../PROGRESS.md) at each milestone.

## Phase 1: Core library, metadata tagging, and CLI foundation

Goal: scan audio folders, store tracks and overlapping vibe tags in SQLite, optionally mirror tags into audio files, and drive everything from `overtone-cli`.

- **`crates/library`:** design SQLite schema (`tracks`, `vibes`, `track_vibes`, `quality_reports`, `sync_history`, `settings`) with explicit schema migrations, literal path comparisons (never SQL `LIKE` on filenames with underscores), and snapshot backups before bulk edits.
- **`crates/audio`:** add `lofty` metadata extraction (artist, title, album, BPM, key, genre, comment, grouping, duration, declared format/sample rate/bitrate) and safe tag writing that preserves opaque vendor frames (`GEOB`, `PRIV`/`TRAKTOR4`).
- **`crates/core`:** implement folder scanning, content hashing/duplicate detection, inbox-to-library immutable ingest, and vibe CRUD (assigning/removing one or more vibes per track, hierarchical crate paths such as `Dubs/Deep`).
- **`crates/cli`:** implement `scan`, `ls`, `show`, `vibe add|rm|ls`, and `tag` with `--json` output (clean JSON on stdout, progress suppressed in JSON mode) and `comfy-table` / `console` formatting for interactive terminals.

## Phase 2: Spectral quality analysis, waveforms, and spectrograms

Goal: catch fake lossless files, upscaled lossy files, and fake hi-res containers automatically, and generate visual assets for review.

- **`crates/audio`:** implement `symphonia` decoding + Blackman-Harris windowed FFT (`rustfft`) spectral cutoff analysis, fake hi-res detection, and traffic-light verdicts (`Pass`, `Warning`, `Suspect`) with measured cutoff Hz and human-readable explanation.
- **`crates/audio`:** implement downsampled waveform peak extraction and spectrogram PNG rendering for flagged tracks.
- **`crates/core` & `crates/cli`:** add parallel batch auditing (`rayon`), `overtone-cli audit` / `overtone-cli audit --suspects`, and verdict override (`verify` to dismiss false positives without deleting anything).
- **Testing:** generate synthetic test WAV/FLAC fixtures in unit tests (pure sine sweeps and low-pass filtered signals at 15.5 kHz, 19 kHz, and 21.5 kHz) to verify cutoff detection deterministically without committing large audio files.

## Phase 3: Serato `.crate` and Rekordbox XML sync

Goal: compile Overtone's vibe groupings out to Serato and Rekordbox safely without duplicating audio files.

- **`crates/sync-serato`:** implement full big-endian TLV reader and writer for `_Serato_/Subcrates/*.crate` (`vrsn`, `otrk`, `ptrk`, `%%` subcrate hierarchy, macOS root-volume and external-volume relative path conversion, preserving existing `osrt`/`ovct` sort columns when updating).
- **`crates/sync-rekordbox`:** integrate `rbox` (`default-features = false, features = ["xml"]`) to generate `rekordbox.xml` with full track metadata and nested playlist folders.
- **`crates/core` & `crates/cli`:** implement dry-run diff previews (`overtone-cli sync serato --dry-run`), timestamped backups before every overwrite, and atomic temp-file writes.

## Phase 4: Tauri desktop listening and triage app

Goal: build a fast, dark-first desktop UI for auditioning tracks, verifying flagged quality suspects, and assigning vibes at keyboard speed.

- **Audio playback (`app/`):** webview `<audio>` playback through Tauri's runtime-scoped asset protocol (play, pause, seek, volume, skip). See [Playback](../decisions/architecture-and-ui.md#playback).
- **Main library & inbox triage views (`app/src/`):**
  - Filterable track table with quality verdict badges, BPM, key, format, and multi-vibe tag pills.
  - Interactive clickable canvas waveform scrubber and keyboard shortcuts (`Space` play/pause, `J`/`K` next/prev track, arrow/number jump-to-section, quick-tag palette for assigning multiple vibes).
  - Expandable spectrogram inspector for flagged tracks with one-click **Verify OK** or **Flag for replacement** actions.
  - Sync panel showing pending crate/playlist diffs for Serato and Rekordbox before applying.
- **UI architecture:** keep tabs mounted once opened (`hidden` attribute, never unmounting active state), maintain a browser mock backend (`import.meta.env.DEV && !isTauri()`) for rapid UI iteration and Vitest coverage, and enforce Tauri command ACLs in `app/src-tauri/build.rs` and `capabilities/default.json`.

## Phase 5: Guarded Rekordbox `master.db` and Traktor `collection.nml` sync

Goal: add seamless native database syncing for Rekordbox 6/7 and Traktor Pro 3/4.

- **`crates/sync-rekordbox`:** enable `rbox`'s `master-db` feature behind process-closed detection (`rekordbox` not running), automatic `master.db` + WAL/SHM backups, and verified macOS SQLCipher build configuration.
- **`crates/sync-traktor`:** implement the `quick-xml` streaming `Reader` -> `Writer` passthrough patcher for `collection.nml` (Traktor Pro 3 and 4, `/:` path encoding, nested `<NODE>` playlist folders, preserving all unmodeled TP3/TP4 attributes, `<GRID>`, `<STEMS>`, and multiline `KEY_LYRICS`), with Traktor-closed checks, backups, and atomic writes.
- **Optional follow-ups:** read-only Serato DJ 4.x `master.sqlite` import, embedded Serato/Rekordbox cue/beatgrid inspection, and Homebrew/GitHub release automation.
