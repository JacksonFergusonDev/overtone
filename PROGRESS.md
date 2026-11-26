# Progress

The maintainer handoff: where things stand, what's open, and what's next. Update it when those change or at a release, not for routine fixes. [CHANGELOG.md](CHANGELOG.md) records user-visible changes and Git history records the rest.

[Engineering index](engineering/README.md) · [Implementation roadmap](engineering/plans/implementation-roadmap.md)

## State of the project

- Version `0.1.0` (unreleased). The shared foundation is in place; Phase 1 feature work comes next.
- `overtone-model` holds the domain types (tracks, nested vibe paths, quality verdicts and reviews). `overtone-library` has the version 1 SQLite schema with track, vibe, quality report, tag snapshot, sync run, root, and settings storage.
- `overtone-fs` guards every write: protected roots in debug builds, atomic replacement, edit-a-clone-then-swap for audio files, APFS clone backups, and the shared app data path.
- The CLI (`status`, `--library`, `--json`) and desktop app open the same `library.sqlite`. Commands return `{ code, message }` errors.
- TypeScript types are generated from Rust by ts-rs into `app/src/bindings/`, with freshness checks in pre-push and CI. The frontend uses Testing Library.
- `overtone-testkit` generates deterministic band-limited WAV, FLAC, and AIFF fixtures and temporary libraries.
- The Git-ignored `local/` testing ground holds APFS clones of the DnB, Garage, and House `My Favourite` folders and copies of Serato's data (see [AGENTS.md](AGENTS.md#audio-files-and-dj-database-safety)).

## Unresolved

- Real-world spectral cutoff thresholds need calibration against genuine warm/lo-fi/vinyl-ripped dubs and known lossy transcodes during Phase 2.
- `rbox` (`0.3.1`, GPL-3.0-only) macOS SQLCipher build flags (`bundled-vendored-openssl` vs. Homebrew `sqlcipher`) need local verification when Phase 5 (`master.db` write support) begins.
- Serato DJ 4.x `master.sqlite` schema reading is deferred until core `.crate` writing is complete and tested against Serato DJ Pro.
- DJ app running detection (Phase 3 and 5) needs a spike: `objc2-app-kit` `NSWorkspace` bundle-ID matching requires `unsafe` (the workspace forbids it) versus a safe `sysinfo` path match. Serato's bundle ID is `com.serato.seratodj`; Rekordbox (`com.pioneerdj.rekordboxdj`) and Traktor IDs need confirming.
- Whether `edit_via_copy` keeps creation dates, xattrs, and Finder tags on internal APFS and external exFAT drives needs a metadata survival spike before tag writing ships.
- On this Mac the Xcode license needs re-accepting (`sudo xcodebuild -license accept`). Until then, build with `DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

## Next

Build order and agent plan (see the [implementation roadmap](engineering/plans/implementation-roadmap.md)). Each piece gets a written contract (API, fixtures, tests that must pass) and runs in its own worktree; the main session reviews and merges.

1. Scanner and audio-only content hashing (`core` + `audio`): `walkdir` scan of library roots, `blake3` over per-format audio payload ranges, batched upserts, missing-file detection. Spikes: hash throughput and payload stability across tag edits.
2. Tag reading and safe writing (`audio`): `lofty` read into `TrackTags` and `AudioProperties`; Comment hashtag mirroring per [vibe tags](engineering/decisions/vibe-tags.md) through `overtone_fs::edit_via_copy`, preserving `GEOB`/`PRIV` frames.
3. CLI commands (`cli`): `scan`, `ls`, `show`, `vibe add|rm|ls|rename`, `tag`, with `--json`.
4. Spectral analysis (`audio`, Phase 2): runs in parallel with 1 to 3.
5. Serato `.crate` TLV reader/writer (`sync-serato`, Phase 3): runs in parallel, reading copies in `local/serato/` as real-world fixtures.
