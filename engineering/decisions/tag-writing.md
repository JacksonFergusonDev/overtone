# Writing Comment tags safely

Decided November 26, 2026. This records how Overtone reads tags and writes the Comment field that mirrors vibes (see [Vibe tags in audio files and DJ software](vibe-tags.md)), and what a spike on 1,151 real Serato-analysed files showed.

## Decision

- **Read** every format with `lofty` pinned to `=0.25.5`, through its concrete file types (`MpegFile`, `FlacFile`, `WavFile`, `AiffFile`, `Mp4File`), never the generic `Tag`. Scans skip artwork and parse in best-attempt mode, falling back to relaxed parsing, then to the MP3's ID3v2 tag alone, then to stream properties only, so one broken frame never hides a track.
- **Write** only the Comment, only for MP3, WAV, AIFF (ID3v2) and FLAC (Vorbis `COMMENT`), only through `overtone_audio::tags::write_comment`. M4A returns `UnsupportedFormat("writing M4A tags is not supported yet")` until a spike proves lofty's `ilst` rewrite safe.
- **Verify independently.** lofty edits a copy made by `overtone_fs::edit_via_copy`. Before the copy may replace the original, Overtone's own byte-level walkers (`tags::raw`, no lofty code) must account for every byte, and any mismatch discards the copy and names the failed check. The original is never opened for writing.

## Which frame is "Comment"

- ID3v2: the first `COMM` frame with language `eng` and an empty description, or failing that the first `COMM` with an empty description in any language. It is edited in place, keeping its language, description, position and (when the text fits) its encoding. A new frame is `eng`, empty description, Latin-1 for ASCII text and UTF-16 otherwise. Other `COMM` frames (`iTunNORM`, other languages) are untouched.
- FLAC: the first `COMMENT` field (key matched case-insensitively). Its key spelling and position are kept; later `COMMENT` fields are untouched.
- MP4 (read only): `©cmt`.
- Grouping is read from ID3v2 `GRP1` when present, else `TIT1` (iTunes 12.5+ moved Grouping to `GRP1`), from Vorbis `GROUPING`, and from MP4 `©grp`. It is never written.
- BPM keeps decimals: ID3 `TBPM` and Vorbis `BPM` are parsed from their raw text (`127,99` reads as 127.99). MP4 uses freeform `BPM`, else `tmpo`.

## The write pipeline

1. Refuse paths under `ProtectedRoots`, then detect the container from file content.
2. Walk the original with the independent walker. Refuse anything it can't fully account for: ID3v2.2, unsynchronisation, extended headers, experimental flags, footers, compressed, encrypted, grouped or length-prefixed frames, non-synchsafe v2.4 sizes, empty frames, data inside padding, an ID3v2 tag that doesn't start at byte 0, chunk sizes that don't add up to the RIFF/FORM size, more than one ID3 chunk, an ID3v2 tag in front of a FLAC stream, or more than one `VORBIS_COMMENT` block.
3. Refuse tags lofty 0.25.5 is known to alter: ID3v2.4-only frames (`TSOP`, `TSOA`, `TSOT` and others) inside an ID3v2.3 tag, which lofty drops on a v2.3 write; exact duplicate frames, which lofty merges; and several Comment frames with the same language and description.
4. Parse strictly with lofty (artwork on, implicit ID3v2.3 conversions off, `max_junk_bytes` 1024 on read and write). For MP3 only the ID3v2 tag region is parsed strictly, because trailing ID3v1 and APE tags are never written and are compared byte for byte instead. Cross-check that lofty and the walker agree on the current Comment. If the Comment already has the requested value, return `Unchanged` without touching the file.
5. On the copy, edit the concrete `Id3v2Tag` or `VorbisComments` and save with `use_id3v23` set from the original version, `lossy_text_encoding(false)`, and the original `ID3` / `id3` chunk case. FLAC saves through `FlacFile::save_to`, because `VorbisComments::save_to` drops `PICTURE` blocks. Two lofty normalisations are undone first: `APIC` keeps its original MIME string (lofty rewrites `image/jpg` as `image/jpeg`), and `WXXX` frames with a Latin-1 description are written as Latin-1 (lofty otherwise encodes the URL in UTF-16, which the spec forbids).
6. Copy extended attributes (Finder tags in `com.apple.metadata:_kMDItemUserTags`, Spotlight comments) and the creation date onto the copy.
7. Verify the copy, and discard it on any failure:
   - **Structure:** the copy walks cleanly.
   - **ID3 version:** unchanged.
   - **Frames:** every non-Comment frame survives with the same ID, flags and count, in the same order within each ID. Binary frames (`GEOB`, `PRIV`, `APIC`, `UFID`, `POPM`, anything not text) are byte-identical. Text frames may be re-encoded but must decode to identical strings.
   - **Comment value:** exactly one target frame holding the requested text, or none for a removal.
   - **Chunks (WAV/AIFF):** every chunk except the ID3 chunk and JUNK padding directly after it is byte-identical and in order; bytes after the RIFF/FORM chunk are unchanged.
   - **FLAC blocks:** `STREAMINFO` identical; `APPLICATION`, `SEEKTABLE`, `CUESHEET` and `PICTURE` blocks byte-identical as a multiset; the vendor string and every Vorbis field other than `COMMENT` identical bytes in the same order.
   - **Audio payload:** byte-identical, streamed in 1 MiB blocks: MPEG frames between the ID3v2 tag and any ID3v1/APE trailer, FLAC frames after the last metadata block, or the `data` / `SSND` chunk body. MP3 trailers are compared separately.
   - **Strict re-read:** lofty re-reads the copy strictly; the Comment equals the request, every other `TrackTags` field is unchanged, and stream properties are unchanged.
8. `fsync` the copy and atomically rename it over the original, then `fsync` the folder.

`overtone_core::tags` builds on this: `plan_vibe_mirroring` previews `CommentChange { track_id, path, before, after }` using `tags::apply_vibes_to_comment`; `apply_vibe_mirroring` refuses a change whose file Comment no longer matches the preview (`stale_preview`), records the first original Comment in `tag_snapshots`, writes, and refreshes the stored track through `upsert_scanned` with the new size and modification time and the unchanged audio hash, so the next scan reports it unchanged; `restore_original_comments` writes snapshots back and clears them. Failures are collected per file with stable codes (`stale_preview`, `write_refused`, `verification_failed`, `unsupported_format`, `protected_path`, `io`).

## Spike results

Run with `cargo run --release -p overtone-audio --example tag_spike -- <folder>`. Each file was cloned with `cp -c` into a temporary folder; only clones were written. Each clone got a Comment with `#overtone-spike` appended, a second identical write, then a write restoring the original Comment.

### Corpus

| Format | Files | ID3v2.3 | ID3v2.4 | Written and verified | Refused |
| --- | --- | --- | --- | --- | --- |
| MP3 | 652 | 473 | 174 | 615 | 37 |
| FLAC | 366 | | | 360 | 6 |
| WAV | 61 | 48 | 3 | 49 | 12 |
| AIFF | 3 | | 3 | 0 | 3 |
| M4A | 69 | | | 0 (not supported yet) | 69 |

Vendor data present in the originals (files): `GEOB` "Serato Markers2" (612 MP3, 43 WAV, 3 AIFF), "Serato Markers_" (613, 42, 3), "Serato BeatGrid" (600, 43, 3), "Serato Overview" (623, 49, 3), "Serato Autotags" (623, 49, 3), "Serato Analysis", "Serato Tempo Map", "Serato Offsets_" (377 MP3); UTF-16 twins of the Serato GEOBs (179 MP3); DJUCED and Sony Acid GEOBs; `PRIV` "TRAKTOR4" (39 MP3); FLAC Vorbis `SERATO_MARKERS_V2` (302), `SERATO_BEATGRID` (301), `SERATO_OVERVIEW` (309), `SERATO_AUTOGAIN`, `SERATO_ANALYSIS`, `SERATO_TEMPO_MAP`, `SERATO_PLAYCOUNT`, `TRAKTOR4` (6); FLAC `APPLICATION` "riff" (48) and "SONY" (3); artwork in 484 MP3 (`APIC`) and 302 FLAC (`PICTURE`).

### What survived a Comment write

| Spike | Result |
| --- | --- |
| Per-frame byte diff, every non-Comment ID3v2 frame (MP3 v2.3, MP3 v2.4, WAV, AIFF) | All `GEOB`, `PRIV`, `APIC`, `UFID` frames byte-identical in every written file. Text frames re-encoded with identical text in a minority of files (for example `TXXX` in 52 MP3 and 12 WAV, `TCON` in 16 MP3, `TPE2` in 12, `WXXX` in 124 after the Latin-1 fix). No MP3 changed frame order; 17 WAV files had their ID3 chunk moved (lofty rewrites a grown chunk at the end) and 28 FLAC files had blocks reordered, with every chunk and block still byte-identical. |
| `geob_serato.id3` fixture (14 GEOBs, Latin-1 and UTF-16 twins) | Byte-identical after writes (`serato_mp3_keeps_every_geob_frame_byte_for_byte`). |
| FLAC blocks and Vorbis fields | `STREAMINFO`, `APPLICATION`, `PICTURE`, vendor string and all `SERATO_*` fields byte-identical in all 360 written files; field order and key case kept. |
| Version preservation | ID3v2.3 stayed v2.3 and v2.4 stayed v2.4 in every write (checked on every write). v2.2 is refused. |
| Unsynchronisation and compression | Refused before writing: 3 MP3 with an unsynchronised tag, none with compressed or encrypted frames. lofty decodes these on read but doesn't re-encode them. |
| Malformed or dropped frames | Caught. Without step 3, lofty silently dropped `TSOP`/`TSOA`/`TSOT` from v2.3 tags and merged duplicate `TXXX`, `TKEY`, `POPM` frames; the frame check refused all of them. |
| Idempotent double write | The second identical write returned `Unchanged` and left the file byte-identical, including its modification time, in all 1,024 written files. |
| Audio payload | Byte-identical in every written file (checked on every write). |
| Artwork | `APIC` byte-identical once the MIME string is restored (37 files would otherwise have changed `image/jpg` to `image/jpeg`); FLAC `PICTURE` byte-identical. |
| File metadata through `edit_via_copy` | On APFS, `clonefile` already keeps extended attributes, creation date, permissions and modification time on the copy, and lofty edits the copy in place, so all survive. Overtone also copies extended attributes and the creation date explicitly for non-APFS volumes. Finder tags, a custom xattr, mode 0640 and creation date verified on all written clones and in `finder_tags_creation_date_and_permissions_survive`. The modification time becomes the write time, as it should, so DJ apps can notice the change. |
| Crash safety | `crash_mid_write_leaves_a_valid_file` kills a child process 8 times during back-to-back writes of a 1.7 MB Serato MP3. The original always parsed, its audio and GEOB frames were intact, and the Comment was either the old or a new value. A kill can leave a hidden `.overtone-*.tmp` copy next to the file; it never replaces the original and isn't scanned as audio. |

### Files a write refuses

| Format | Files | Reason |
| --- | --- | --- |
| M4A | 69 | Writing M4A is deferred. |
| MP3 | 12 | ID3v2.3 tag contains v2.4-only frames (`TSOP`, `TSOA`, `TSOT`) lofty would drop. |
| MP3 | 5 | lofty merges frames: 2 tags with two `TXXX` frames sharing a description, 3 with exact duplicates (`TKEY`, `TXXX`, `UFID`). |
| MP3 | 5 | lofty normalises a frame: `POPM` gains a play counter, `TDRC` `20220310` becomes `2022-03-10`, multi-value UTF-16 `TXXX` loses its byte-order marks (3 files). |
| MP3 | 3 | Unsynchronised tag. |
| MP3 | 1 | Experimental tag flag. |
| MP3 | 3 | Strict parse fails: malformed `TDEN`, `UFID` with an empty owner (2). |
| MP3 | 5 | lofty refuses to write: `COMM`/`USLT` with an invalid language (4), `TDRC` with month 13. |
| MP3 | 3 | lofty can't recognise the stream when saving (junk between the tag and the first MPEG frame). |
| FLAC | 6 | lofty uppercases `tracknumber`/`discnumber` keys (3) and drops some fields such as `ACOUSTID_ID`, `ENCODER` and `LYRICS` (3). |
| WAV | 7 | Two ID3 chunks (`ID3` and `id3 `); which one each DJ app reads is unknown. |
| WAV | 5 | Strict parse or write fails: `TSOP` in v2.3, a malformed `TDRC`, a non-UTF-8 RIFF `INFO` chunk, a `COMM` with an invalid language, and one file lofty can't parse at all. |
| AIFF | 3 | lofty's strict parse rejects a chunk size in the Interplanetary Criminal "Dubs Vol.2" files. |

Refusals are reported to the DJ; the file is never touched. Most come from lofty normalising data it doesn't need to touch, which is the case for the surgical ID3v2 `COMM` patcher described in the tag research (Phase B): it would make the 12 `TSOP` files, the duplicate-frame files and the normalisation cases writable.

## Read throughput

`cargo run --release -p overtone-audio --example scan_bench -- local/music 3` on a MacBook Air M3 (`Mac15,12`, 8 threads, rayon) over 1,151 files: 4,587 to 5,047 files/s on the first pass and 28,617 to 36,205 files/s warm. The first pass was not cold: the page cache already held these files from the spike, and a truly cold read needs root to purge. One WAV fails to read at all because lofty rejects its chunk sizes.

## What the DJ must verify in the DJ apps

Overtone proves the bytes Serato, Traktor and Rekordbox own are unchanged, but not how each app reacts to a changed file. Before enabling mirroring on a real library, take copies of a few analysed MP3 (ID3v2.3 and ID3v2.4), FLAC, WAV and AIFF files, write a Comment with Overtone, and check:

- **Serato DJ:** cue points, loops, beatgrid, track colour and overview are intact; the Comment shows; a smart crate "Comment contains `#vibe`" matches.
- **Rekordbox:** after "Reload Tag", hot cues, memory cues and the beatgrid are intact, and the Comment shows. Note whether the changed size and modification time alone trigger re-analysis.
- **Traktor:** after "Import Metadata from Files", cue points and the grid are intact and the Comment shows. Traktor stores cues in `collection.nml` and in the `TRAKTOR4` frame, which is byte-identical; check it doesn't re-analyse.

## Regression gate

lofty is pinned to `=0.25.5`. Re-run the spike and `cargo test -p overtone-audio` before any bump; several fixes on lofty's main branch (Vorbis custom keys, ID3v2.3 dates, EOF splice truncation) may change which files are refused.
