# Progress

The maintainer handoff: where things stand, what's open, and what's next. Update it when those change or at a release, not for routine fixes. [CHANGELOG.md](CHANGELOG.md) records user-visible changes and Git history records the rest.

[Engineering index](engineering/README.md) · [Implementation roadmap](engineering/plans/implementation-roadmap.md)

## State of the project

- Version `0.1.0` (unreleased). Phase 1 and 2 are complete in the CLI, and Phase 3 is done for Serato. The desktop UI (Phase 4) is still the scaffold.
- **Library and scanning:** in-place indexing of library folders, incremental rescans, missing-file tracking, and audio-only BLAKE3 hashes for duplicate detection ([content hashing](engineering/decisions/content-hashing.md)). Tag and stream property reading via `lofty` 0.25.5. On the `local/` testing ground, 1,151 files scan in about 9 s cold and rescan in milliseconds.
- **Quality analysis:** single-pass `symphonia` decode with Welch-averaged spectra, cutoff wall detection, verdicts, waveform peaks, and spectrogram PNGs ([audio quality analysis](engineering/decisions/audio-quality-analysis.md)). On `local/`: 1,092 pass, 39 warning, 15 suspect, 5 inconclusive, in 100 s on 4 threads.
- **Tag writing:** verified Comment writes for MP3, WAV, AIFF, and FLAC that keep Serato GEOB, Traktor PRIV, and artwork byte-identical and preserve the ID3 version; refuses files lofty can't round-trip ([tag writing](engineering/decisions/tag-writing.md)). M4A writes are deferred.
- **Serato sync:** lossless `.crate` codec (all 140 sample crates round-trip byte-identically), dry-run plans, backups, and atomic writes under an `Overtone` root crate.
- **CLI:** `status`, `roots`, `scan`, `ls`, `show`, `vibe`, `audit`, `review`, `spectrogram`, `tag mirror|restore`, and `sync serato`, all with `--json`.

## Unresolved

- **Needs testing in real DJ apps (manual, by the maintainer):**
  - Serato DJ 4 picks up Overtone-written crates and keeps them across launches. Serato 4's `master.sqlite` tracks `.crate` size and MD5, so test on a throwaway library first.
  - After `tag mirror --write`, cues, loops, beatgrids, and overviews are intact in Serato, Rekordbox (after Reload Tag), and Traktor (after Import Metadata), for ID3v2.3 and v2.4 MP3, FLAC, WAV, and AIFF.
  - Listen to the 15 suspects found in `local/` (13 "320 kbps" MP3s that look like 128k sources, plus one FLAC and one WAV with a 15.8 kHz ceiling).
- **Detection limits:** LAME V0 and 256k AAC transcodes can't be detected from the spectrum alone; an MDCT/frame-alignment detector would be needed. HE-AAC files can't be decoded by symphonia and are recorded as Unchecked.
- **Tag write refusals:** about 6% of MP3 and some FLAC and WAV files are refused because lofty would normalize or drop frames. A surgical ID3v2 COMM patcher would make most of them writable.
- DJ app running detection uses `sysinfo` path matching for Serato. Rekordbox and Traktor bundle IDs still need confirming against installed apps.
- `rbox` (`0.3.1`) macOS SQLCipher build flags need verification when Phase 5 (`master.db` write support) begins.

## Next

1. Phase 3 rest: Rekordbox `rekordbox.xml` export via `rbox-xml`, sharing the core `desired_crates` mapping.
2. Phase 4: the desktop UI (library table with virtualization, waveform scrubber and webview playback, spectrogram inspector with review actions, keyboard-first vibe tagging, sync panel with dry-run diffs). Tauri commands wrap the existing core functions; the ts-rs bindings already cover the IPC types.
3. Phase 5: guarded Rekordbox `master.db` sync (My Tags) and the Traktor `collection.nml` streaming patcher.
