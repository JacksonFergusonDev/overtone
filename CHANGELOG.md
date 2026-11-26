# Changelog

What changed in each release of Overtone, for people using it. Add entries under Unreleased as user-visible changes land.

## Unreleased

- Initial repository scaffold for the Overtone desktop app and CLI.
- The desktop app and `overtone-cli` share one library database in `~/Library/Application Support/com.jacksonferguson.overtone/`. Pass `--library <path>` (or set `OVERTONE_LIBRARY`) to use a different one.
- `overtone-cli status` shows track, missing track, vibe, and needs-review counts. With `--json`, errors are also printed as JSON.
