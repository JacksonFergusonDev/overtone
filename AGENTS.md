## Git workflow

- Commit and push directly to `origin/main` by default for a single, self-contained change, such as a small bug fix or an edit to `AGENTS.md`. Check the pull request criteria below before choosing this path.
- Commit completed, coherent changes autonomously using conventional commit messages.
- Push the working branch autonomously after each coherent milestone and before ending each development session. The pre-push hook runs during `git push`.
- Do not ask for approval before routine commits, pushes, refactoring, dependency changes, or other development activities.
- Never force-push, rewrite published Git history, delete remote branches, or perform destructive repository operations.
- Never bypass Git hooks, disable CI, remove failing tests, or weaken verification to make changes pass.
- When a commit changes something users can see or do in the app or CLI, add a line for it under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) in the same commit. Write it for users, following the writing conventions below. Leave out internal changes such as CI, tooling, refactors, tests and contributor docs.
- Record every check that needs a human (behavior inside Serato, Rekordbox, or Traktor, listening by ear, judgment calls) in [engineering/manual-verification.md](engineering/manual-verification.md) in the same commit as the feature, so the maintainer can verify everything in one pass.
- [PROGRESS.md](PROGRESS.md) is the maintainer handoff, not a log. Update it only when current work, unresolved issues or next steps change, or at a release or milestone. Don't record routine fixes there: Git history and CHANGELOG.md cover them. Keep longer plans in `engineering/` and link to them. Use the [engineering index](engineering/README.md) to find maintained references and design decisions.

### When to use a pull request

- Use a PR when the change needs verification on a GitHub Actions runner beyond what the local hooks cover. Run the relevant workflows against the PR branch, trigger any manual workflows it needs, and confirm the checks passed for the latest revision before merging to `main`.
- Prefer a PR when you expect several rounds of iteration, such as debugging a tricky issue or refining a UI. Keep the intermediate commits on the branch and squash-merge the finished work into `main` as one conventional commit.
- Use a PR when the change needs a lasting explanation beyond a commit message. Record the problem, why you chose the approach, and how you verified it. Document any architectural boundaries or invariants the change affects.
- Use a PR when the diff is large or the work is complex, even if it has a single goal.

## Verification

- Run `prek install` in a new checkout or when hooks are missing. See [development setup](README.md#development) for dependencies. Run `just sync` after dependency changes so the pnpm lockfile hook can check them offline.
- Let normal `git commit` and `git push` run the checks selected by `.pre-commit-config.yaml`. Pre-commit runs formatters and quick checks, commit-msg validates the message, and pre-push runs builds, linters, type checks, and tests.
- Do not run `just ci`, its alias `just check`, or the hook checks manually as a routine step before committing or pushing. Use `just ci` for diagnosing CI discrepancies or when explicitly requested.
- If a hook fails, fix the cause and retry the Git command. If a hook edits files, review and stage the intended edits before retrying the commit. If a push failure requires a code change, commit the fix and retry the push.
- Run focused checks during development or debugging. For example, use `cargo test --locked -p overtone-sync-serato` for Serato changes or `pnpm --dir app test src/App.test.tsx` for frontend changes.

## Autonomous execution

- Continue working through the approved [implementation roadmap](engineering/plans/implementation-roadmap.md) without requesting permission at each milestone.
- Make routine implementation and architectural decisions independently, following the decisions recorded in `engineering/decisions/`.
- Prefer completed, tested features over partially implemented features.
- Use existing libraries and established solutions where appropriate rather than unnecessarily reimplementing functionality, following the crate evaluations in [DJ software sync](engineering/decisions/dj-software-sync.md).
- Escalate only genuinely blocking ambiguities or operations that risk irreversible damage to real user audio files or live DJ software databases.

## Choosing dependencies

- Before implementing a feature that could use an external crate or package, survey the candidates: maintenance, adoption, license (must be compatible with GPL-3.0-only), MSRV, open bugs, and the alternative of writing it yourself.
- When more than one candidate is viable, compare them with spikes and benchmarks on the target machine (Apple Silicon, `aarch64-apple-darwin`) before choosing. Record the choice and the measurements in `engineering/decisions/`.
- Prefer stability and quality over development speed. App bundle size matters less than quality and capability.
- Prefer implementations that use SIMD, NEON, or other hardware acceleration when they measurably improve speed on Apple Silicon. Use `rayon` for parallel work when the batch is large enough to benefit.

## Shared code and tests

- Keep library, audio analysis, tagging, and DJ sync behavior shared by the desktop app and CLI in the Rust crates under `crates/`. Keep Tauri commands and CLI presentation in their respective entry points. When changing a CLI command that supports `--json`, keep stdout limited to the JSON payload and suppress progress output in JSON mode.
- Use fixtures and temporary directories for automated tests that write files, including `tempfile::TempDir` in Rust. Isolate test libraries, audio fixtures, and DJ databases from real user data. Never touch `~/Music`, `_Serato_`, Rekordbox `master.db`, or Traktor `collection.nml` during automated tests.

## Writing conventions

- Follow the Writing section of house-style's guidelines: concrete over abstract, lead with the answer, say "you" in user docs, use bold only for labels, controls, or defined terms, and use no em dashes.
- Write the README for first-time readers, `docs/` for users completing tasks, and `engineering/` for contributors. Link compatibility and performance claims to the relevant test results or measurements.
- Never hard-wrap Markdown. Write each paragraph and list item as a single line and let the editor soft-wrap. rumdl enforces this (`.rumdl.toml`) and its pre-commit hook joins wrapped lines automatically.

## Audio files and DJ database safety

- Never modify the real music library or real DJ software data during development: `~/Music`, `~/Library/Application Support/Serato`, `~/Library/Pioneer`, and `~/Documents/Native Instruments`. Debug builds enforce this through the protected-root check (see [protected paths](engineering/decisions/architecture-and-ui.md#protected-paths)).
- For manual testing against real-world data, use the Git-ignored `local/` testing ground: `local/music/` holds APFS clones (`cp -c`) of a subset of the real library, `local/serato/` holds copies of `_Serato_` and Serato's `Library` folder, and `local/rekordbox/` holds a read-only copy of `~/Library/Pioneer/rekordbox` (including `master.db`). Open Rekordbox databases only from a further `cp -c` copy in a scratch folder, never in place. Copied `.crate` files still reference the original track paths, so only read them; write new crates under `local/`. Recreate clones with `cp -c` rather than plain copies, because the disk has little free space.
- Never auto-delete or silently overwrite user audio files based on automated quality verdicts. Spectral analysis flags suspicious tracks for human review by ear and spectrogram.
- Treat accepted library file paths as immutable after ingest. Moving files breaks cue points, loops, and beatgrids in external DJ software unless those databases are rewritten in lockstep.
- Before writing to any external DJ database or collection file (Serato `.crate`, Rekordbox `master.db`, Traktor `collection.nml`), check that the target DJ application is not running (for live databases such as `master.db` and `collection.nml`), save a timestamped backup first, and write via a temporary file plus atomic rename.
- Never use `serde` full struct roundtripping when editing Traktor `collection.nml`; use a streaming `quick-xml` passthrough patcher so unmodeled Traktor Pro 3 and 4 attributes survive unchanged.
- Do not automate musical vibe or crate assignment. Overtone automates technical checks (spectral quality, metadata hygiene, and multi-software crate sync) while leaving creative curation to the DJ.

## UI design

- Keep the desktop interface dark-first, legible, and keyboard-fast for listening and tagging workflows.
- Verify presentation at default and minimum window sizes; adapt layout instead of shrinking text.
- Keep views mounted once shown and draw cached data on first render: hide inactive tabs and sub-views instead of unmounting them, and avoid flashing loading states for local library data already in memory.
