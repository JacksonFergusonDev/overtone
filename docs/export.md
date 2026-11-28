# Export copies for USB sticks, CDJs, and phones

`overtone-cli export` writes copies of tracks in another format into a folder. Your originals are only read, and an existing file is never replaced: a second export of the same track is named `Track (2).aiff`.

```bash
overtone-cli export --vibe "Dubs/Deep" --to /Volumes/USB/Dubs
```

Name tracks by id or path, or pass `--vibe` to export every track in a vibe. Exporting needs ffmpeg (`brew install ffmpeg`).

## Formats

| `--format` | What you get | Use it for |
|---|---|---|
| `cdj` (default) | AIFF, 16-bit, 44.1 kHz | Any Pioneer CDJ or XDJ, including players without FLAC or hi-res support |
| `aiff` | AIFF at the source's rate, 16 or 24-bit | Newer players |
| `flac` | FLAC, lossless | Players and apps that read FLAC |
| `wav` | WAV | Software that wants plain PCM |
| `aac` | AAC in `.m4a`, 256 kbps, Apple's encoder | Phones and music apps |
| `mp3` | MP3, 320 kbps | Anything else |

`--sample-rate 44100`, `--bit-depth 16`, and `--bitrate 192` override the defaults. Lowering the bit depth adds dither, and changing the sample rate uses a high-quality resampler.

## What the copies carry

- Tags and artwork from the source, with the values you set or accepted in Overtone (see [Your metadata and where it came from](metadata.md)) in place of the file's.
- Not Serato, Rekordbox, or Traktor cue points, loops, or beatgrids. Analyze the copies in your DJ software, or use your DJ software's own export for a USB stick that keeps your cues.

Each copy is decoded again before it's saved, to check its format, sample rate, and length, so a failed encode never leaves a broken file in the folder.
