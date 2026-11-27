# Changelog

What changed in each release of Overtone, for people using it. Add entries under Unreleased as user-visible changes land.

## Unreleased

- Initial repository scaffold for the Overtone desktop app and CLI.
- The desktop app and `overtone-cli` share one library database in `~/Library/Application Support/com.jacksonferguson.overtone/`. Pass `--library <path>` (or set `OVERTONE_LIBRARY`) to use a different one.
- `overtone-cli status` shows track, missing track, vibe, and needs-review counts. With `--json`, errors are also printed as JSON.
- `overtone-cli scan [folders…]` indexes your audio files in place, reading them without ever modifying them. Rescans only look at new or changed files, tracks whose files disappear are kept and marked missing, and folders on unplugged drives are left alone. Duplicate detection compares audio content only, so the same recording with different tags is still recognized. `scan --refresh` rereads every file's tags, and scans do this automatically after an Overtone update improves tag reading.
- Scans read title, artist, album, BPM (with decimals), key, genre, comment, grouping, and stream details such as sample rate, bit depth, and whether an `.m4a` is ALAC or AAC.
- `overtone-cli audit` checks audio quality and flags likely fake lossless files (MP3s wrapped as FLAC or WAV), upscaled MP3s (a 128 kbps file re-encoded at 320), and fake hi-res files, with the measured cutoff and a plain-language reason. Nothing is ever deleted or moved. Review flagged tracks with `ls --needs-review`, render a spectrogram with `spectrogram <track>`, and record your verdict with `review <track> ok|replace|clear`. Your reviews survive reanalysis.
- `overtone-cli tag mirror` writes each track's vibes as hashtags into its Comment tag for Serato smart crates, Rekordbox intelligent playlists, and Traktor smartlists, keeping all other Comment text. It previews by default; `--write` applies. Every write is verified against the original before it replaces the file (Serato and Traktor cue data, artwork, and audio must be byte-identical, and the ID3 version is kept), and files Overtone can't edit safely are skipped with a reason. M4A files aren't written yet. `tag restore --write` puts back the original Comments.
- New `overtone-cli` commands: `roots add|ls|rm` to choose folders to index, `ls` (with `--vibe`, `--missing`, and `--needs-review` filters), `show <id or path>`, and `vibe ls|create|rename|delete|add|rm` for overlapping, nested vibes.
- `overtone-cli sync serato` writes each vibe as a Serato crate under a top-level `Overtone` crate, leaving crates you made yourself untouched. Add `--dry-run` to preview the changes. Overtone refuses to sync while Serato DJ is running and backs up every crate file it replaces.
