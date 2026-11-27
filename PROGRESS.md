# Progress

The maintainer handoff: where things stand, what's open, and what's next. Update it when those change or at a release, not for routine fixes. [CHANGELOG.md](CHANGELOG.md) records user-visible changes and Git history records the rest.

[Engineering index](engineering/README.md) · [Implementation roadmap](engineering/plans/implementation-roadmap.md)

## State of the project

- Version `0.1.0` (unreleased). All five roadmap phases are implemented: the CLI, quality analysis, sync to Serato, Rekordbox (XML and My Tags in `master.db`), and Traktor, and the desktop app. Phase 6 (metadata provenance and lookup, ffmpeg, and format conversion) is planned and not started ([Phase 6 plan](engineering/plans/metadata-and-conversion.md)). None of it has been checked inside real DJ apps or in a real app window yet (see Unresolved).
- **Library and scanning:** in-place indexing of library folders, incremental rescans, missing-file tracking, and audio-only BLAKE3 hashes for duplicate detection ([content hashing](engineering/decisions/content-hashing.md)). Tag and stream property reading via `lofty` 0.25.5. On the `local/` testing ground, 1,151 files scan in about 9 s cold and rescan in milliseconds.
- **Quality analysis:** single-pass `symphonia` decode with Welch-averaged spectra, cutoff wall detection, verdicts, waveform peaks, and spectrogram PNGs ([audio quality analysis](engineering/decisions/audio-quality-analysis.md)). On `local/`: 1,092 pass, 39 warning, 15 suspect, 5 inconclusive, in 100 s on 4 threads.
- **Tag writing:** verified Comment writes for MP3, WAV, AIFF, and FLAC that keep Serato GEOB, Traktor PRIV, and artwork byte-identical and preserve the ID3 version; refuses files lofty can't round-trip ([tag writing](engineering/decisions/tag-writing.md)). M4A writes are deferred.
- **Serato sync:** lossless `.crate` codec (all 140 sample crates round-trip byte-identically), dry-run plans, backups, and atomic writes under an `Overtone` root crate.
- **Rekordbox XML:** `rekordbox.xml` written with `quick-xml` (not `rbox`, see [DJ software sync](engineering/decisions/dj-software-sync.md#rekordbox-xml-written-with-quick-xml-not-rbox)): an `Overtone` playlist folder mirroring the vibe tree, stable TrackIDs, percent-encoded `Location` URIs from raw path bytes, no cue or beatgrid data, backups, and unchanged-file detection.
- **Rekordbox My Tags:** each vibe becomes a My Tag in an `Overtone` category of `master.db`, written by Overtone's own SQLCipher code rather than `rbox` (it can't share a build with `rusqlite` 0.43, see [DJ software sync](engineering/decisions/dj-software-sync.md#rekordbox-masterdb-own-my-tag-writer-over-rusqlite-with-sqlcipher-not-rbox)). The live database is never opened: a copy is edited, verified (integrity, foreign keys, every row outside the category unchanged, update counter), and swapped in with the stale WAL parked first. Tested on a copy of the real 7.0.8 database (`OVERTONE_RB_FIXTURE`, opt-in).
- **Traktor:** byte-preserving splice of an `Overtone` folder into `collection.nml` ([design](engineering/decisions/dj-software-sync.md#traktor-splice-the-overtone-playlist-folder-into-collectionnml)), tested on synthetic TP3 and TP4 fixtures; there is no Traktor install or real collection here.
- **Desktop app:** 30 Tauri commands over core ([desktop UI plan](engineering/plans/desktop-ui.md)) with progress channels, a runtime asset scope for library folders, and a spectrogram cache. The React UI ([frontend stack](engineering/decisions/architecture-and-ui.md#frontend-stack)) has a virtualized library table, a player with a canvas waveform, a quick-tag palette, a spectrogram inspector with review actions, a needs-review flow, a Sync view for every sync target, and Settings for folders, scans, and audits. `pnpm --dir app dev` runs it in a browser against a seeded mock backend. An IPC test runs commands through Tauri's real ACL on the mock runtime.
- **CLI:** `status`, `roots`, `scan`, `ls`, `show`, `vibe`, `audit`, `review`, `spectrogram`, `tag mirror|restore`, and `sync serato|rekordbox|rekordbox-tags|traktor`, all with `--json`.

## Unresolved

- **Manual checks pending:** real DJ app behavior (Serato DJ 4 accepting written crates, Rekordbox importing the XML and opening a database with Overtone's My Tags, Traktor loading the patched collection, cues surviving Comment writes), listening to the 15 flagged suspects, and the desktop app in a real window (WKWebView playback of FLAC and AIFF, layout at 840x560, keyboard flow). The UI has only been exercised in a browser against the mock backend. All of them are in the [manual verification checklist](engineering/manual-verification.md).
- **Detection limits:** LAME V0 and 256k AAC transcodes can't be detected from the spectrum alone; an MDCT/frame-alignment detector would be needed. HE-AAC files can't be decoded by symphonia and are recorded as Unchecked. Phase 6 step 4 adds an ffmpeg fallback decoder for them.
- **Tag write refusals:** about 6% of MP3 and some FLAC and WAV files are refused because lofty would normalize or drop frames. A surgical ID3v2 COMM patcher would make most of them writable; it is Phase 6 step 1.
- DJ app running detection uses `sysinfo` process matching. The Rekordbox, `rekordboxAgent`, and Traktor Pro 3 process names are unverified against real installs.
- **Unverified sync assumptions:** how Rekordbox Cloud Library Sync treats rows Overtone adds, and Traktor's mapping of `/Volumes/<Drive>` paths to `VOLUME`. Both are on the checklist.
- **Desktop app rough edges:** playback stops at the end of a track instead of continuing; sidebar vibe counts are direct only; Serato sync and Comment mirroring always use the default target (no folder or vibe picker); a removed library folder stays playable until restart.
- `rbox` (`0.3.1`) macOS SQLCipher build flags need verification when Phase 5 (`master.db` write support) begins.

## Next

How the work runs: before each feature, a research subagent surveys crates; contested choices get spikes and benchmarks on the M3 (recorded in `engineering/decisions/`). Implementation packages run in parallel git worktrees under `.claude/worktrees/` with written contracts (owned files, API, tests, spikes), Opus for risky or subtle work and Sonnet for well-specified work. The main session reviews each diff, merges with `--no-ff`, integrates into core and the CLI, verifies against `local/`, and adds manual checks to the checklist.

Phase 6 is the current work. Follow the [Phase 6 plan](engineering/plans/metadata-and-conversion.md) in order and mark each step done there and here as it lands. Its key rules: provenance (step 2) lands before any lookup (step 5) so fetched values never mix with the DJ's own; OneTagger is a source of ported ideas and code, not a dependency; ffmpeg runs as a bundled child process and Symphonia verifies its output.

1. Phase 6 step 1: surgical ID3v2 COMM patcher.
2. Step 2: per-field metadata provenance (`file`, `user`, `lookup:<provider>`) and the suggestion flow.
3. Step 3: ffmpeg foundation (spike against `rsmpeg`/`ffmpeg-next`, audio-only static build, `overtone-ffmpeg` wrapper crate, decision doc).
4. Step 4: ffmpeg fallback decoder and integrity check in the audit.
5. Step 5: metadata lookup (MusicBrainz, Discogs, then AcoustID).
6. Step 6: loudness, true peak, and leading-silence reports.
7. Step 7: export copies (downsampled and lossy).
8. Step 8a: lossless WAV/AIFF to FLAC conversion with sample-level verification, tags, and Serato tag translation.
9. The manual verification pass ([checklist](engineering/manual-verification.md)), on throwaway DJ libraries, then fix what it turns up. It must happen before step 8b.
10. Step 8b: conversion relocation in DJ apps, one app at a time. Which app goes first is undecided.
11. Desktop polish: continuous playback, a target picker for Serato, Comment mirroring, and Traktor collections, and Traktor `INFO/@COMMENT` hashtag mirroring (noted in [vibe tags](engineering/decisions/vibe-tags.md)).
12. Release packaging: signing, notarization (including the bundled ffmpeg binary), and Homebrew/GitHub release automation.
