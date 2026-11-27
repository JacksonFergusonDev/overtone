# Phase 6 plan: metadata provenance, lookup, ffmpeg, and format conversion

Agreed November 27, 2026. This is Phase 6 of the [implementation roadmap](implementation-roadmap.md). It records the build order, what each step delivers, the research behind it, and the questions still open, so work can resume from any step in a new session. Mark each step done here (and in [PROGRESS.md](../../PROGRESS.md)) as it lands.

## Build order

The order matters. Provenance comes before any lookup so fetched values never mix with the DJ's own. ffmpeg comes before fingerprinting so every decodable format can be identified. Conversion comes last because its relocation step writes to the DJ's own crates and databases.

| Step | Work | Depends on | Status |
|---|---|---|---|
| 1 | Surgical ID3v2 COMM patcher | nothing | Done Nov 27: byte-level splicer for ID3v2 and FLAC, 1,067 of 1,082 files writable |
| 2 | Per-field metadata provenance | nothing | Not started |
| 3 | ffmpeg foundation (decision, build, wrapper crate) | nothing | Not started |
| 4 | ffmpeg fallback decoder and integrity check in the audit | 3 | Not started |
| 5 | Metadata lookup (MusicBrainz, Discogs, then AcoustID) | 2, and 4 for AcoustID | Not started |
| 6 | Loudness, true peak, and leading-silence reports | 4 optional | Not started |
| 7 | Export copies (downsampled and lossy) | 3 | Not started |
| 8a | Lossless conversion: audio, verification, tags, Serato tags | 1, 3 | Not started |
| 8b | Conversion relocation in DJ apps, one app at a time | 8a, the manual verification pass | Not started |

Each step follows the usual process from [AGENTS.md](../../AGENTS.md): survey crates first, spike and benchmark contested choices on the M3, record decisions in `engineering/decisions/`, add a CHANGELOG line for user-visible changes, and add human-only checks to the [manual verification checklist](../manual-verification.md).

## Step 1: surgical ID3v2 COMM patcher

About 6% of MP3s and some FLAC and WAV files refuse Comment writes today because lofty would normalize or drop frames ([tag writing](../decisions/tag-writing.md#files-a-write-refuses)). Patch only the COMM frame using the `id3` crate (or a hand-written frame splice), leaving every other frame byte-identical. This is the same reason OneTagger's `onetagger-tag` uses `id3` and `metaflac` directly instead of lofty. Done when most currently refused files become writable and the existing regression gate (GEOB, PRIV, and artwork byte-identical, ID3 version preserved) still passes. Step 8a reuses this tag-level control.

## Step 2: per-field metadata provenance

Goal: values the DJ entered by hand must never get lost among values Overtone fetched.

- Store a source for each field of each track in the library database: `file` (present on the file at first scan), `user` (entered in Overtone), or `lookup:<provider>`, with confidence, fetch time, and the provider's record ID.
- Fetched values arrive as suggestions in a pending state. Nothing is written to the file or shown as the track's value until the DJ accepts it. Accepted values keep their `lookup` source plus an accepted flag.
- Precedence: a `user` value always wins. A lookup never overwrites a `user` or `file` value. By default lookups only fill empty fields; overwriting is an explicit per-suggestion choice.
- Lookups never touch the DJ's personal fields: Comment, Grouping, vibes, rating, and color. They only propose factual fields: label, catalog number, release date, ISRC, artwork, album, and track and disc numbers.
- Genre is kept apart: the store's genre is saved as a reference value (for example "Beatport genre") and only copied into Genre when the DJ chooses.
- The UI shows each value's source at a glance and can filter by it (for example "fetched but not reviewed").
- Files carry only final values. Provenance lives in Overtone's database, not in a private tag.

## Step 3: ffmpeg foundation

Decision so far: bundle ffmpeg, but use it only where Rust has no good answer.

- Use ffmpeg for lossy encoding (Apple's AAC encoder through `aac_at` on macOS, `libmp3lame` for MP3), decoding formats Symphonia can't (HE-AAC, Opus, WMA, damaged containers), and the encode side of conversion.
- Keep Symphonia for analysis and for verifying ffmpeg's output, so the check is independent of the encoder. Prefer `ebur128` for loudness and `rubato` for resampling inside the analysis pass.
- Integration: run a bundled ffmpeg binary as a child process (a Tauri sidecar for the app, a bundled or system ffmpeg for the CLI) rather than linking `ffmpeg-next` (maintenance mode) or `rsmpeg`. A crash on one file only kills that process, matching the per-track `catch_unwind` isolation, it avoids C linking, and ffmpeg can be updated without a rebuild. Raw PCM can be piped back (`-f f32le -`) when analysis needs it. Confirm with a spike against the bindings and record the result in a new decision doc.
- Build: a static, audio-only ffmpeg with only redistributable codecs (no `fdk-aac`). Record the exact configure flags, version, and license in the decision doc.
- Code: a small shared wrapper crate (working name `overtone-ffmpeg`) for locating the binary, building argument lists, progress, and error mapping, used by both the app and the CLI.
- Release packaging must sign and notarize the bundled binary.

## Step 4: fallback decoder and integrity check

- When Symphonia can't decode a file, decode through ffmpeg and run the normal spectral analysis on the piped PCM. This replaces today's Unchecked result for HE-AAC and adds Opus, WMA, and damaged files.
- Integrity check: a full ffmpeg decode with errors reported (`-v error -f null -`) as a second opinion on corrupt or truncated files, shown in the audit alongside the spectral verdict.

## Step 5: metadata lookup

OneTagger ([Marekkon5/onetagger](https://github.com/Marekkon5/onetagger)) is the model, but its crates are not taken as dependencies:

- None are published on crates.io, the last release is 1.7.0 from August 3, 2023 with 194 commits on master since, so it would mean pinning an unreleased git revision.
- `onetagger-tag` uses `lofty` 0.22, while Overtone pins `=0.25.5` after verifying round-trips on that version.
- `onetagger-autotag` depends on an unpinned git fork of SongRec (Shazam's unofficial API), and `onetagger-platforms` brings in tokio, reqwest, and rspotify. Several of its platforms are HTML scrapers that break when stores change.
- Spotify shut down its audio-features API in late 2024.

OneTagger is GPL-3.0, compatible with Overtone's GPL-3.0-only, so its matching logic (fuzzy artist and title matching, duration tolerance, candidate selection) can be ported with attribution.

Plan:

1. A new crate (working name `overtone-metadata`) with clients for the official APIs: MusicBrainz first (free, 1 request per second), then Discogs (token). Beatport and others later, if at all.
2. AcoustID identification with `rusty-chromaprint` (pure Rust) rather than Shazam.
3. All results go through step 2's suggestion flow. Lookups are for metadata hygiene only and never assign vibes or crates.

## Step 6: loudness, true peak, and leading silence

Reports only, never changes to audio: integrated loudness (EBU R128), true peak, and clipping (already listed as Phase 2+ signals in [audio quality analysis](../decisions/audio-quality-analysis.md)), plus leading-silence length. Trimming silence is out of scope because it would shift every cue point.

## Step 7: export copies

Write new files and never replace originals: downsample hi-res to 16-bit/44.1 kHz with dither (for older CDJs that can't play above 48 kHz) and make AAC or MP3 copies for USB sticks or phones. Carry tags over and verify them like step 8a does. Export copies are new files, so they need no relocation.

## Step 8a: lossless conversion (WAV and AIFF to FLAC)

| Layer | Difficulty | Approach |
|---|---|---|
| Audio | Easy | Encode with ffmpeg. Verify by decoding source and output with Symphonia and comparing every sample, plus the FLAC STREAMINFO MD5 of the decoded audio. The content hash covers encoded bytes, so it changes by design and can't be the check. The 26 ms cue shift DJs report comes from MP3 encoder delay and does not apply to lossless conversion. |
| Regular tags | Moderate | Map WAV and AIFF ID3 frames (and WAV `LIST`/`INFO`) to Vorbis comments, then read back and compare. |
| Serato tags | Moderate | Translate GEOB frames to base64 Vorbis comments following [Holzhaus/serato-tags](https://github.com/Holzhaus/serato-tags) `docs/fileformats.md`: for example `Serato Markers2` becomes `SERATO_MARKERS_V2`, and so on for Analysis, Autotags, BeatGrid, and Overview. Base64 without padding, a newline every 72 characters, and decoded content of `application/octet-stream`, a null byte, the field name, then the payload. The last base64 byte varies when Serato writes, so compare after decoding, not byte for byte. Needs a check inside Serato. |

Safety: dry run first, write the FLAC next to the original, keep originals (move them to an archive folder only after the DJ confirms), and warn that pre-NXS2 CDJs don't play FLAC (offer AIFF as the target for those players).

## Step 8b: relocation in DJ apps

Converting changes the file extension, and accepted library paths are treated as immutable because the DJ apps key cues, loops, and beatgrids to the path. Relocation rewrites each app's references in lockstep, using the existing guarded write paths (app-not-running check, timestamped backup, atomic write or verified copy). Do one app at a time, starting with the one the maintainer uses most (not yet chosen).

| App | Difficulty | Approach |
|---|---|---|
| Rekordbox | Moderate | Update the existing `djmdContent` row in place rather than creating a new one, so ANLZ analysis and cues stay linked. Reuse the verified-copy `master.db` writer. [rekordbox-edit](https://github.com/jviall/rekordbox-edit) (MIT, Python, 0.13.3 from October 2026) already converts with ffmpeg and repoints `master.db`; port its field list. Its docs list no post-conversion verification and don't name the columns it changes, so read its source. |
| Serato | Hardest | Cues travel in the file tags (8a), but the DJ's own crates and Serato's `database V2` still reference the old path. Today Overtone only writes crates under its own `Overtone` root, so rewriting user crates and `database V2` is new territory. [serato-tools](https://pypi.org/project/serato-tools/) (MIT, Python) only writes Serato tags to ID3 files and doesn't convert formats. |
| Traktor | Moderate, risky | Rewrite `LOCATION` with the existing splice patcher. There are [reports of Traktor beatgrids drifting on FLAC](https://community.native-instruments.com/discussion/comment/8256), which needs checking by ear. |

Run the manual verification pass before this step: relocation builds on sync assumptions not yet checked inside the real DJ apps.

## Research: existing tools

No open-source tool converts formats while keeping DJ data across Serato, Rekordbox, and Traktor (search on November 27, 2026). The pieces that exist:

- [rekordbox-edit](https://rekordbox-edit.readthedocs.io/en/latest/commands/convert/): Rekordbox only, described above.
- [Holzhaus/serato-tags](https://github.com/Holzhaus/serato-tags): the Serato format spec per container (docs CC BY-SA 4.0, scripts MIT). Appears unmaintained, but the spec is what matters.
- [serato-tools](https://pypi.org/project/serato-tools/): Serato tag parsing in Python, ID3-only writes.
- [triseratops](https://docs.rs/triseratops): a Rust Serato parser, stuck at 0.0.3 and licensed AGPL-3.0-or-later. Use for reference, not as a dependency.
- [dj-data-converter](https://github.com/digital-dj-tools/dj-data-converter) and Lexicon (commercial): convert libraries between DJ apps, not file formats.
- Forum threads ([NI community](https://community.native-instruments.com/discussion/comment/162726), [DJTechTools](https://forum.djtechtools.com/t/converting-audio-formats-and-keeping-cue-grid-in-traktor/13741)) ask for this and get no working answer.

## Deferred

- Remux repair (fixing broken WAV headers or MP4 atom layouts). It rewrites files in place, against the rule of never silently overwriting audio. If added, it writes a repaired copy for review.

## Open questions

- Which DJ app step 8b supports first.
- Whether ffmpeg is bundled with the CLI or the CLI uses a system ffmpeg (decide in step 3).
