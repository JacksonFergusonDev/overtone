# Progress

The maintainer handoff: where things stand, what's open, and what's next. Update it when those change or at a release, not for routine fixes. [CHANGELOG.md](CHANGELOG.md) records user-visible changes and Git history records the rest.

[Engineering index](engineering/README.md) · [Implementation roadmap](engineering/plans/implementation-roadmap.md)

## State of the project

- Repository scaffolded with a Cargo workspace (`crates/audio`, `crates/library`, `crates/sync-serato`, `crates/sync-rekordbox`, `crates/sync-traktor`, `crates/core`, `crates/cli`) and a Tauri v2 + React 19 + TypeScript frontend in `app/`.
- Engineering decisions from initial architecture and Rust ecosystem research are recorded under `engineering/decisions/`.
- CI, `justfile`, `.rumdl.toml`, and `.pre-commit-config.yaml` are configured following the Parallax repository layout.

## Current

- Version `0.1.0` (unreleased). Ready for Phase 1 implementation.

## Unresolved

- Real-world spectral cutoff thresholds need calibration against genuine warm/lo-fi/vinyl-ripped dubs and known lossy transcodes during Phase 2.
- `rbox` (`0.3.1`, GPL-3.0-only) macOS SQLCipher build flags (`bundled-vendored-openssl` vs. Homebrew `sqlcipher`) need local verification when Phase 5 (`master.db` write support) begins.
- Serato DJ 4.x `master.sqlite` schema reading is deferred until core `.crate` writing is complete and tested against Serato DJ Pro.

## Next

- Follow the phased plan in [implementation roadmap](engineering/plans/implementation-roadmap.md):
  1. Phase 1: SQLite schema, library scanner, `lofty` tag read/write, and `overtone-cli` commands (`scan`, `ls`, `tag`).
  2. Phase 2: `symphonia` + `rustfft` spectral quality checker, waveform peak extraction, and spectrogram rendering.
  3. Phase 3: Serato `.crate` TLV reader/writer and Rekordbox XML exporter (`rbox_xml`), with automatic backup and dry-run previews.
  4. Phase 4: Tauri desktop UI with audio playback, clickable waveform scrubber, spectrogram inspection panel, and keyboard-first multi-vibe triage.
  5. Phase 5: Guarded Rekordbox `master.db` sync (`rbox-masterdb`) and lossless Traktor `collection.nml` streaming patcher (`quick-xml`).
