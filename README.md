# Overtone

**A desktop DJ library manager for checking audio quality, organizing tracks into overlapping vibes without duplicating files, and syncing crates to Serato, Rekordbox, and Traktor.**

## Overview

Overtone is a local-first library hub built with Rust and Tauri. It automates the tedious, fragile parts of managing a DJ library while keeping musical curation in your hands:

- **Spectral quality forensics:** decodes lossless and lossy audio files (`FLAC`, `WAV`, `AIFF`, `ALAC`, `MP3`, `M4A`) and runs windowed FFT analysis to flag suspected fake lossless transcodes, upscaled lossy files, and fake hi-res containers for review by ear and spectrogram.
- **Multi-vibe organization without duplicate files:** keeps one immutable copy of each audio file on disk, stores your vibe groupings in a local SQLite library and embedded audio tags, and lets one track belong to as many crates as it fits.
- **Cross-platform DJ sync:** exports your groupings to Serato `.crate` subcrates, Rekordbox (`rekordbox.xml` and guarded `master.db`), and Traktor Pro 3/4 (`collection.nml`) with automatic backups before every write.
- **Listening and triage workflow:** audition tracks in a keyboard-fast desktop app with clickable waveforms, an inspectable spectrogram view for flagged tracks, and one-keystroke vibe tagging.

To learn the app, see [Using the desktop app](docs/desktop-app.md). [DJ software support](docs/dj-software-support.md) lists what syncs to each DJ app today, [Your metadata and where it came from](docs/metadata.md) explains how Overtone keeps your own values apart from fetched ones, and [Export copies](docs/export.md) covers CDJ-safe and phone copies.

Licensed under [GPL-3.0-only](LICENSE).

## Development

Requires Rust (stable, pinned via `rust-toolchain.toml`), Node and pnpm, and [just](https://github.com/casey/just).

Run `just sync` to install locked frontend dependencies and `prek install` to enable the Git hooks. Commits run formatting and quick checks on changed files. Pushes run the slower builds, linters, type checks, and tests.

```bash
just          # list available recipes
just sync     # install locked frontend dependencies
just dev      # run the desktop app with hot reload
just ci       # run formatting, lint, type checks, and tests
just build    # build the macOS .app bundle
```

`overtone-cli` is a command-line companion that shares the desktop app's library, audio analysis, and sync crates:

```bash
cargo run -p overtone-cli -- --help
```

See the [engineering index](engineering/README.md) for architecture decisions, crate evaluations, and the implementation roadmap. The [maintainer handoff](PROGRESS.md) tracks current status and next steps.
