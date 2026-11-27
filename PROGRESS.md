# Progress

The maintainer handoff: where things stand, what's open, and what's next. Update it when those change or at a release, not for routine fixes. [CHANGELOG.md](CHANGELOG.md) records user-visible changes and Git history records the rest.

[Engineering index](engineering/README.md) · [Implementation roadmap](engineering/plans/implementation-roadmap.md)

## State of the project

- Version `0.1.0` (unreleased). Phases 1 to 4 are complete: the CLI, quality analysis, Serato and Rekordbox XML sync, and the desktop app. None of it has been checked inside real DJ apps or in a real app window yet (see Unresolved).
- **Library and scanning:** in-place indexing of library folders, incremental rescans, missing-file tracking, and audio-only BLAKE3 hashes for duplicate detection ([content hashing](engineering/decisions/content-hashing.md)). Tag and stream property reading via `lofty` 0.25.5. On the `local/` testing ground, 1,151 files scan in about 9 s cold and rescan in milliseconds.
- **Quality analysis:** single-pass `symphonia` decode with Welch-averaged spectra, cutoff wall detection, verdicts, waveform peaks, and spectrogram PNGs ([audio quality analysis](engineering/decisions/audio-quality-analysis.md)). On `local/`: 1,092 pass, 39 warning, 15 suspect, 5 inconclusive, in 100 s on 4 threads.
- **Tag writing:** verified Comment writes for MP3, WAV, AIFF, and FLAC that keep Serato GEOB, Traktor PRIV, and artwork byte-identical and preserve the ID3 version; refuses files lofty can't round-trip ([tag writing](engineering/decisions/tag-writing.md)). M4A writes are deferred.
- **Serato sync:** lossless `.crate` codec (all 140 sample crates round-trip byte-identically), dry-run plans, backups, and atomic writes under an `Overtone` root crate.
- **Rekordbox XML:** `rekordbox.xml` written with `quick-xml` (not `rbox`, see [DJ software sync](engineering/decisions/dj-software-sync.md#rekordbox-xml-written-with-quick-xml-not-rbox)): an `Overtone` playlist folder mirroring the vibe tree, stable TrackIDs, percent-encoded `Location` URIs from raw path bytes, no cue or beatgrid data, backups, and unchanged-file detection.
- **Desktop app:** 26 Tauri commands over core ([desktop UI plan](engineering/plans/desktop-ui.md)) with progress channels, a runtime asset scope for library folders, and a spectrogram cache. The React UI ([frontend stack](engineering/decisions/architecture-and-ui.md#frontend-stack)) has a virtualized library table, a player with a canvas waveform, a quick-tag palette, a spectrogram inspector with review actions, a needs-review flow, a Sync view for all three sync paths, and Settings for folders, scans, and audits. `pnpm --dir app dev` runs it in a browser against a seeded mock backend. An IPC test runs commands through Tauri's real ACL on the mock runtime.
- **CLI:** `status`, `roots`, `scan`, `ls`, `show`, `vibe`, `audit`, `review`, `spectrogram`, `tag mirror|restore`, `sync serato`, and `sync rekordbox`, all with `--json`.

## Unresolved

- **Manual checks pending:** real DJ app behavior (Serato DJ 4 accepting written crates, Rekordbox importing the XML, cues surviving Comment writes), listening to the 15 flagged suspects, and the desktop app in a real window (WKWebView playback of FLAC and AIFF, layout at 840x560, keyboard flow). The UI has only been exercised in a browser against the mock backend. All of them are in the [manual verification checklist](engineering/manual-verification.md).
- **Detection limits:** LAME V0 and 256k AAC transcodes can't be detected from the spectrum alone; an MDCT/frame-alignment detector would be needed. HE-AAC files can't be decoded by symphonia and are recorded as Unchecked.
- **Tag write refusals:** about 6% of MP3 and some FLAC and WAV files are refused because lofty would normalize or drop frames. A surgical ID3v2 COMM patcher would make most of them writable.
- DJ app running detection uses `sysinfo` process matching for Serato and Rekordbox; the Rekordbox process name is unverified and Traktor isn't detected yet.
- **Desktop app rough edges:** playback stops at the end of a track instead of continuing; sidebar vibe counts are direct only; Serato sync and Comment mirroring always use the default target (no folder or vibe picker); a removed library folder stays playable until restart.
- `rbox` (`0.3.1`) macOS SQLCipher build flags need verification when Phase 5 (`master.db` write support) begins.

## Next

How the work runs: before each feature, a research subagent surveys crates; contested choices get spikes and benchmarks on the M3 (recorded in `engineering/decisions/`). Implementation packages run in parallel git worktrees under `.claude/worktrees/` with written contracts (owned files, API, tests, spikes), Opus for risky or subtle work and Sonnet for well-specified work. The main session reviews each diff, merges with `--no-ff`, integrates into core and the CLI, verifies against `local/`, and adds manual checks to the checklist.

1. Phase 5: guarded Rekordbox `master.db` sync (My Tags) via `rbox`, and the Traktor `collection.nml` streaming patcher, each with a Sync view card.
2. Desktop polish from the manual pass: continuous playback, a target picker for Serato and Comment mirroring, and anything the real-window checks turn up.
