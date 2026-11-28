# Writing tags safely

Decided November 26, 2026; the writer was replaced by the byte-level splicer on November 27, 2026, and extended to other text fields (see [Field writes](#field-writes)) the same day. This records how Overtone reads tags and writes the Comment field that mirrors vibes (see [Vibe tags in audio files and DJ software](vibe-tags.md)), and what a spike on 1,151 real Serato-analysed files showed.

## Decision

- **Read** every format with `lofty` pinned to `=0.25.5`, through its concrete file types (`MpegFile`, `FlacFile`, `WavFile`, `AiffFile`, `Mp4File`), never the generic `Tag`. Scans skip artwork and parse in best-attempt mode, falling back to relaxed parsing, then to the MP3's ID3v2 tag alone, then to stream properties only, so one broken frame never hides a track.
- **Write** only the Comment and the [field writes](#field-writes), only for MP3, WAV, AIFF (ID3v2) and FLAC (Vorbis comments), only through `overtone_audio::tags::write_comment` and `write_fields`. M4A returns `UnsupportedFormat("writing M4A tags is not supported yet")` until a spike proves lofty's `ilst` rewrite safe.
- **Write by splicing, not through a tag library.** `tags::splice` copies the original into a copy made by `overtone_fs::edit_via_copy`, rebuilding only the target `COMM` frame (ID3v2) or `COMMENT` field (FLAC) and copying every other frame, chunk, block, and audio byte verbatim. lofty is no longer involved in writes, so frames it would drop, merge, or normalise survive unchanged.
- **Verify independently.** Before the copy may replace the original, Overtone's own byte-level walkers (`tags::raw`) must account for every byte, and any mismatch discards the copy and names the failed check. The original is never opened for writing.

## Which frame is "Comment"

- ID3v2: the first `COMM` frame with language `eng` and an empty description, or failing that the first `COMM` with an empty description in any language. It is edited in place, keeping its language, description, position and (when the text fits) its encoding. A new frame is `eng`, empty description, Latin-1 for ASCII text and UTF-16 otherwise. Other `COMM` frames (`iTunNORM`, other languages) are untouched.
- FLAC: the first `COMMENT` field (key matched case-insensitively). Its key spelling and position are kept; later `COMMENT` fields are untouched.
- MP4 (read only): `©cmt`.
- Grouping is read from ID3v2 `GRP1` when present, else `TIT1` (iTunes 12.5+ moved Grouping to `GRP1`), from Vorbis `GROUPING`, and from MP4 `©grp`. Mirroring never writes it; a field write does (below).
- BPM keeps decimals: ID3 `TBPM` and Vorbis `BPM` are parsed from their raw text (`127,99` reads as 127.99). MP4 uses freeform `BPM`, else `tmpo`.

## The write pipeline

1. Refuse paths under `ProtectedRoots`, then detect the container from file content.
2. Walk the original with the independent walker. Refuse anything it can't fully account for: ID3v2.2, unsynchronisation, extended headers, experimental flags, footers, compressed, encrypted, grouped or length-prefixed frames, non-synchsafe v2.4 sizes, empty frames, data inside padding, an ID3v2 tag that doesn't start at byte 0, chunk sizes that don't add up to the RIFF/FORM size, more than one ID3 chunk, an ID3v2 tag in front of a FLAC stream, more than one `VORBIS_COMMENT` block, or several Comment frames with the same language and description.
3. Read the tags the way the scanner does (lofty, tolerant; for MP3 only the leading ID3v2 tag) and cross-check that it and the walker agree on the current Comment. A file lofty can't read even tolerantly is refused, since Overtone couldn't show what it wrote. If the Comment already has the requested value, return `Unchanged` without touching the file.
4. Splice the copy:
   - **ID3v2 (MP3, and the `ID3` chunk of WAV and AIFF):** re-serialise the tag from the walker's frames, each written from its stored ID, flags, and body. The target `COMM` keeps its position, language, encoding (Latin-1 switches to UTF-16 only when the text needs it), and stored description bytes. A new Comment is appended as `eng`, empty description. The tag keeps its exact size when its padding absorbs the change, so nothing after it moves; otherwise it grows with 2 KiB of fresh padding. In WAV and AIFF the chunk and RIFF/FORM sizes are updated to match. A file without a tag gets an ID3v2.4 tag (MP3) or an `ID3` chunk appended to the RIFF/FORM chunk.
   - **FLAC:** rebuild the `VORBIS_COMMENT` body from the walker's raw entries with only the first `COMMENT` value replaced (its key spelling kept), removed, or appended. The first `PADDING` block grows or shrinks so the metadata keeps its size and the audio doesn't move; if it can't, it is reset to 2 KiB. Block order is unchanged.
5. Copy extended attributes (Finder tags in `com.apple.metadata:_kMDItemUserTags`, Spotlight comments) and the creation date onto the copy.
6. Verify the copy, and discard it on any failure:
   - **Structure:** the copy walks cleanly.
   - **ID3 version:** unchanged.
   - **Frames:** every non-Comment frame survives with the same ID, flags and count, in the same order within each ID. Binary frames (`GEOB`, `PRIV`, `APIC`, `UFID`, `POPM`, anything not text) are byte-identical. Text frames must decode to identical strings (with the splicer they are byte-identical too).
   - **Comment value:** exactly one target frame holding the requested text, or none for a removal.
   - **Chunks (WAV/AIFF):** every chunk except the ID3 chunk and JUNK padding directly after it is byte-identical and in order; bytes after the RIFF/FORM chunk are unchanged.
   - **FLAC blocks:** `STREAMINFO` identical; `APPLICATION`, `SEEKTABLE`, `CUESHEET` and `PICTURE` blocks byte-identical as a multiset; the vendor string and every Vorbis field other than `COMMENT` identical bytes in the same order.
   - **Audio payload:** byte-identical, streamed in 1 MiB blocks: MPEG frames between the ID3v2 tag and any ID3v1/APE trailer, FLAC frames after the last metadata block, or the `data` / `SSND` chunk body. MP3 trailers are compared separately.
   - **Re-read:** the scanner's read of the copy equals the original's with only the Comment changed, and stream properties are unchanged.
7. `fsync` the copy and atomically rename it over the original, then `fsync` the folder.

`overtone_core::tags` builds on this: `plan_vibe_mirroring` previews `CommentChange { track_id, path, before, after }` using `tags::apply_vibes_to_comment`; `apply_vibe_mirroring` refuses a change whose file Comment no longer matches the preview (`stale_preview`), records the first original Comment in `tag_snapshots`, writes, and refreshes the stored track through `upsert_scanned` with the new size and modification time and the unchanged audio hash, so the next scan reports it unchanged; `restore_original_comments` writes snapshots back and clears them. Failures are collected per file with stable codes (`stale_preview`, `write_refused`, `verification_failed`, `unsupported_format`, `protected_path`, `io`).

## Field writes

`tags::write_fields` writes the other fields the scanner reads, for the DJ's own values and accepted lookup values (`overtone_core::writeback`, `overtone-cli meta apply`): title, artist, album, album artist, remixer, label, genre, year, BPM, key, grouping, catalog number, ISRC, release date, and track and disc numbers. Comment stays with `write_comment`, and store genre stays in Overtone, because no tag the scanner reads holds it.

- **Same pipeline.** The splicer takes an `Editor` (the frames of the new ID3 tag, the entries of the new Vorbis comment); `CommentEdit` and the field editor share it, `prepare`, `edit_via_copy`, the chunk, block, trailer, and payload checks, and the re-read.
- **Target the frame the scanner reads.** Each field has a priority list of ID3 frames and Vorbis keys, mirroring the reader: year is `TDRC`, `TYER`, `TDOR`, `TORY` and `DATE`, `YEAR`, `ORIGINALDATE`; grouping is `GRP1`, `TIT1`; label is `TPUB` and `LABEL`, `ORGANIZATION`, `PUBLISHER`; and so on. A value replaces the first frame or key that holds a usable value (for Vorbis, the first entry of that key), else the first one present, else a new standard one (`TDRC` in v2.4, `TYER` in v2.3, the first Vorbis key). Aliases and later duplicates are left alone, so other software keeps reading what it read before. Clearing a field removes every frame or key it could be read from.
- **Values.** Trimmed; blank clears. A year is stored as its four digits, and a BPM must parse. A replaced frame keeps its flags and, when the text fits, its encoding; otherwise Latin-1 becomes UTF-16 (or stays UTF-8/UTF-16BE in v2.4).
- **Refusals.** Everything `write_comment` refuses, plus more than one frame with an ID the edit touches (two `TPUB` frames, say).
- **Verification.** ID3: frames the edits own hold exactly the planned value (or are gone), and every other frame passes the same comparison as a Comment write. Vorbis: the entries equal the planned list, entries under keys no edit touches are identical and in order, the vendor is unchanged, and a reader independent of the planner finds each new value. Then the scanner's re-read must equal the original's tags with only the edited fields changed.
- **Undo.** `overtone_core::writeback` records each field's first original in `tag_snapshots` (field names as in `MetadataField`) and drops snapshots again when a write is refused or unchanged; `restore_original_tags` writes them back, skipping files that already hold them.

The raw walker's text decoder treats iTunes' `GRP1`, `MVNM`, and `MVIN` as text frames, as lofty does; before, a `GRP1` grouping looked empty to the planner, which picked `TIT1`, and the re-read check refused the write.

On November 27, 2026, 223 `cp -c` clones from `local/music` (120 MP3, 60 FLAC, 40 WAV, 3 AIFF) got a new label with non-Latin-1 text, year, and grouping, plus a cleared key on every fourth and a changed title on every fifth file. `meta apply --write` wrote 217 in 11.7 s and refused 6 (5 WAVs with two ID3 chunks, 1 MP3 whose `UFID` frame lofty can't parse). A Python check sharing no code with Overtone (its own ID3 and FLAC walkers, plus an ffmpeg MD5 of the decoded audio) found decoded audio identical in all 223 files, all 1,112 `GEOB` frames and 473 `SERATO_*` fields unchanged, every other frame and Vorbis entry unchanged, the refused files untouched, and the new values where the scanner reads them. A rescan reported every file unchanged, and `meta restore --write` returned the tags of all 217 to exactly what the first scan read.

### Release details

Reader version 2 (migration `0008_more_tags.sql`) reads catalog number, ISRC, release date, and track and disc numbers, so field writes can be re-read and lookups can tell when they would replace a value.

- **ID3:** catalog number is `TXXX:CATALOGNUMBER` (the description matched case-insensitively and kept as stored when replaced), ISRC `TSRC`, track `TRCK`, disc `TPOS`, release date `TDRL`, else `TXXX:RELEASEDATE`. New release dates go to `TDRL` in v2.4 and `TXXX:RELEASEDATE` in v2.3, which has no `TDRL`. The planner keys `TXXX` frames by description, so other `TXXX` frames (Serato's among them) pass the ordinary frame comparison. A release date must be `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`, the forms an ID3v2.4 timestamp holds.
- **Vorbis:** `CATALOGNUMBER`, `ISRC`, `RELEASEDATE`, `TRACKNUMBER`, `DISCNUMBER`. lofty splits `TRACKNUMBER=3/12` into a number and `TRACKTOTAL` on read, so the reader joins the number with `TRACKTOTAL` or `TOTALTRACKS` (`DISCTOTAL` or `TOTALDISCS`), and the writer stores `3/12` as `TRACKNUMBER=3` plus `TRACKTOTAL=12`, removing the total when the new value has none. The first run, before this, wrote `TRACKNUMBER=12/12` and the re-read check refused all 40 FLACs untouched.
- **MP4 (read only):** freeform `CATALOGNUMBER`, `ISRC`, `RELEASEDATE`, and `trkn`/`disk` as `3/12`.
- **Restore** rewrites only fields that no longer read as their originals. lofty reads track numbers as integers, so a restored `TRACKNUMBER=01` comes back as `1`; Overtone reads both the same.

On November 27, 2026, 147 `cp -c` clones (80 MP3, 40 FLAC, 25 WAV, 2 AIFF) got all five fields (a non-Latin-1 catalog number, ISRC cleared on every sixth). 144 were written; the same 3 structural refusals as before were left untouched. The independent Python check found decoded audio identical in all 147, all 774 `GEOB` frames and 299 `SERATO_*` fields unchanged, every frame and Vorbis entry outside the five fields unchanged, and the new values in `TXXX:CATALOGNUMBER`, `TPOS`, `TDRL` (v2.4) or `TXXX:RELEASEDATE` (v2.3), and the split Vorbis keys. A rescan reported every file unchanged, and `meta restore --write` put back every original value, apart from the zero-padding noted above in 14 FLACs.

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

The tables above are from the November 26 run with the lofty writer, which wrote 1,024 of 1,082 writable files. The splicer re-run on November 27 (same corpus, same spike) wrote and verified 1,067: all 366 FLAC, all 3 AIFF, 645 of 652 MP3 and 53 of 61 WAV, with no text frame re-encoded and no frame, chunk, or block reordered. What it still refuses:

| Format | Files | Reason |
| --- | --- | --- |
| M4A | 69 | Writing M4A is deferred. |
| MP3 | 3 | Unsynchronised tag. |
| MP3 | 1 | Experimental tag flag. |
| MP3 | 3 | lofty can't read the tag even tolerantly (`UFID` with an empty owner), so the scanner couldn't show the result. |
| WAV | 7 | Two ID3 chunks (`ID3` and `id3 `); which one each DJ app reads is unknown. |
| WAV | 1 | lofty can't read the file's chunk sizes. |

Refusals are reported to the DJ; the file is never touched. Everything the lofty writer refused for normalising data (`TSOP` in v2.3, duplicate frames, `POPM`, `TDRC` dates, UTF-16 `TXXX`, invalid languages, junk before the first MPEG frame, FLAC key case and dropped fields, the AIFF chunk sizes) is now written with those bytes untouched.

## Read throughput

`cargo run --release -p overtone-audio --example scan_bench -- local/music 3` on a MacBook Air M3 (`Mac15,12`, 8 threads, rayon) over 1,151 files: 4,587 to 5,047 files/s on the first pass and 28,617 to 36,205 files/s warm. The first pass was not cold: the page cache already held these files from the spike, and a truly cold read needs root to purge. One WAV fails to read at all because lofty rejects its chunk sizes.

## What the DJ must verify in the DJ apps

Overtone proves the bytes Serato, Traktor and Rekordbox own are unchanged, but not how each app reacts to a changed file. Before enabling mirroring on a real library, take copies of a few analysed MP3 (ID3v2.3 and ID3v2.4), FLAC, WAV and AIFF files, write a Comment with Overtone, and check:

- **Serato DJ:** cue points, loops, beatgrid, track colour and overview are intact; the Comment shows; a smart crate "Comment contains `#vibe`" matches.
- **Rekordbox:** after "Reload Tag", hot cues, memory cues and the beatgrid are intact, and the Comment shows. Note whether the changed size and modification time alone trigger re-analysis.
- **Traktor:** after "Import Metadata from Files", cue points and the grid are intact and the Comment shows. Traktor stores cues in `collection.nml` and in the `TRAKTOR4` frame, which is byte-identical; check it doesn't re-analyse.

## Regression gate

lofty is pinned to `=0.25.5`. It no longer writes, but it still decides what the scanner reads and so which files pass the cross-check and re-read. Re-run the spike and `cargo test -p overtone-audio` before any bump.
