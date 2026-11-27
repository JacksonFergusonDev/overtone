# Changelog

What changed in each release of Overtone, for people using it. Add entries under Unreleased as user-visible changes land.

## Unreleased

- Initial repository scaffold for the Overtone desktop app and CLI.
- The desktop app and `overtone-cli` share one library database in `~/Library/Application Support/com.jacksonferguson.overtone/`. Pass `--library <path>` (or set `OVERTONE_LIBRARY`) to use a different one.
- `overtone-cli status` shows track, missing track, vibe, and needs-review counts. With `--json`, errors are also printed as JSON.
- `overtone-cli scan [folders…]` indexes your audio files in place, reading them without ever modifying them. Rescans only look at new or changed files, tracks whose files disappear are kept and marked missing, and folders on unplugged drives are left alone. Duplicate detection compares audio content only, so the same recording with different tags is still recognized.
- New `overtone-cli` commands: `roots add|ls|rm` to choose folders to index, `ls` (with `--vibe`, `--missing`, and `--needs-review` filters), `show <id or path>`, and `vibe ls|create|rename|delete|add|rm` for overlapping, nested vibes.
- `overtone-cli sync serato` writes each vibe as a Serato crate under a top-level `Overtone` crate, leaving crates you made yourself untouched. Add `--dry-run` to preview the changes. Overtone refuses to sync while Serato DJ is running and backs up every crate file it replaces.
