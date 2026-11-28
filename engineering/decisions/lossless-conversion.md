# Lossless conversion to FLAC

Decided November 27, 2026. Phase 6 step 8a ([plan](../plans/metadata-and-conversion.md#step-8a-lossless-conversion-wav-and-aiff-to-flac)). Implemented in `overtone_audio::convert` (tag translation, assembly, verification) and `overtone_core::convert` (orchestration), exposed as `overtone-cli convert`.

## Decision

- WAV and AIFF tracks holding integer PCM up to 24 bits are converted to FLAC beside the original (`Track.wav` becomes `Track.flac` in the same folder). 32-bit float WAV is refused, because FLAC stores integers and the conversion would round. Files with two ID3 chunks are refused, as for tag writes.
- The original is never touched. It stays the track in Overtone and in the DJ apps, and the FLAC is recorded in `conversions` as waiting for relocation. Scans skip waiting FLACs so they don't appear as duplicate tracks. Relocation (step 8b) will point Overtone and each DJ app at the FLAC and only then offer to archive the original.
- `convert` previews by default and lists, per track, the Serato data carried over and every frame that has no FLAC equivalent. `--write` converts; `--discard` deletes the FLACs Overtone wrote (never an original).

## The pipeline

1. **Check** the source: container, PCM format (`fmt` tag 1, or extensible with PCM subformat; AIFF-C only `NONE`, `sowt`, or `twos`), bit depth, and the same structural walk tag writes use.
2. **Plan the metadata.** The 12 fields the scanner reads (title, artist, album, album artist, remixer, label, genre, date, BPM, key, Comment, Grouping) are written from the scanner's own reading as `TITLE`, `ARTIST`, `ALBUM`, `ALBUMARTIST`, `REMIXER`, `LABEL`, `GENRE`, `DATE`, `BPM`, `INITIALKEY`, `COMMENT`, and `GROUPING`, so they read back identically; a full date and the BPM text are kept as written when they agree with the scanner. Other text frames map to their usual Vorbis keys (`TRCK` to `TRACKNUMBER`, `TSRC` to `ISRC`, `TSOP` to `ARTISTSORT`, and so on), `TXXX` to its description, `USLT` to `LYRICS`, and `APIC` to PICTURE blocks.
3. **Serato data.** Each Serato GEOB frame becomes the Vorbis field Serato itself writes in FLAC, holding base64 of the GEOB body without its encoding byte (`application/octet-stream\0\0Serato Markers2\0` and the payload). Measured on 300 Serato-analysed FLAC files on `local/`, Serato's own base64 differs by field, and Overtone writes each the same way: wrapped at 72 characters; `SERATO_MARKERS_V2`, `SERATO_BEATGRID`, `SERATO_ANALYSIS`, `SERATO_RELVOL`, and `SERATO_VIDEO_ASSOC` without padding or a trailing newline; `SERATO_AUTOGAIN` (from `Serato Autotags`), `SERATO_OVERVIEW`, and `SERATO_TEMPO_MAP` with `=` padding and a trailing newline. `Serato Markers_` and `Serato Offsets_` have no FLAC field (FLAC keeps cues in Markers2) and are dropped without a warning. Serato often leaves one stray character at the end of `SERATO_BEATGRID` (299 of 301 files); Overtone writes it clean.
4. **Encode** with ffmpeg (`-map 0:a:0 -map_metadata -1 -fflags +bitexact -c:a flac -compression_level 8`) to a temporary file, then **assemble** the final FLAC: ffmpeg's STREAMINFO (with its MD5 of the audio) and SEEKTABLE, Overtone's Vorbis comment and PICTURE blocks, 8 KiB of padding for later tag edits, and the encoded frames.
5. **Verify** before the FLAC is moved into place (`create_new_via_temp`, which never replaces a file): Symphonia decodes source and FLAC and every sample must have identical bits; sample rate, channels, bit depth, and length match, and STREAMINFO agrees and has an MD5; the Vorbis comment and pictures are exactly the plan; every Serato field decodes with the right header; and the scanner reads the same tags from the FLAC as from the source.

## Results on `local/`

64 WAV and AIFF files cloned from `local/music` (all 61 WAV and 3 AIFF): 52 converted and verified in 18.5 s on 4 threads; 7 refused for two ID3 chunks, 4 for 32-bit float audio, and 1 that lofty can't read. An independent Python check found all 265 Serato fields in the FLACs byte-identical to the source GEOB data after decoding. Frames not carried over: `RVAD` (37 files) and `RVA2` (4), volume adjustments with no Vorbis equivalent; DJUCED's own GEOB data (1 file); and one each of `POPM` (rating), `WCOP`, `UFID`, and `TIME`.

## What still needs a person

Whether Serato DJ shows the cues, loops, beatgrid, and overview from a converted FLAC exactly as from the original WAV is on the [manual verification checklist](../manual-verification.md). Rekordbox and Traktor keep their cues in their own databases, keyed by path, which is step 8b.
