# Convert WAV and AIFF to FLAC

`overtone-cli convert` turns WAV and AIFF tracks into FLAC, which is lossless but smaller and holds tags reliably. The audio is identical, sample for sample, and Overtone checks every sample before it keeps a FLAC.

```bash
overtone-cli convert --all
overtone-cli convert --all --write
```

The first command previews: for each track it shows the FLAC it would write, the Serato data that moves with it (cues, loops, beatgrid, overview), and anything with no FLAC equivalent, such as volume-adjust frames. Add `--write` to convert. Pass track ids or paths, or `--vibe <vibe>`, instead of `--all`. Converting needs ffmpeg (`brew install ffmpeg`).

## Your originals stay put

The FLAC is written next to the original, which isn't touched. Overtone and your DJ software keep using the original until relocation, a later step that moves them to the FLAC together so cues and beatgrids stay attached. Until then, `overtone-cli convert --pending` lists the waiting FLACs, scans ignore them, and `overtone-cli convert <tracks> --discard` deletes the FLACs Overtone wrote (never the originals).

## What's converted

- 16 and 24-bit WAV and AIFF. 32-bit floating-point WAV isn't, because FLAC can only store it by rounding.
- Title, artist, album, BPM, key, Comment, Grouping, track numbers, ISRC, lyrics, other text tags, and artwork.
- Serato's cues, loops, beatgrid, overview, and autogain, stored the way Serato stores them in FLAC.
- Not: ratings, volume-adjust frames, and other DJ software's private data. The preview lists them for each track.

Pioneer CDJs older than the NXS2 can't play FLAC. If you play on those, keep AIFF, or export CDJ-safe copies (see [Export copies](export.md)).
