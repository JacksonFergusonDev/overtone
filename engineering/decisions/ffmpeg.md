# ffmpeg as a child process

Decided November 27, 2026. Phase 6 step 3 ([plan](../plans/metadata-and-conversion.md#step-3-ffmpeg-foundation)). Implemented in `crates/ffmpeg` (`overtone-ffmpeg`).

## Decision

- Overtone uses ffmpeg only where Rust has no good answer: lossy encoding (Apple's AAC encoder through `aac_at`, MP3 through `libmp3lame`), decoding formats Symphonia can't (HE-AAC, Opus, WMA, damaged containers), integrity checks, and the encode side of format conversion.
- Symphonia stays the decoder for analysis and is the independent check on everything ffmpeg produces. A conversion or export is accepted only when Symphonia's decode of the result matches what Overtone expects.
- ffmpeg runs as a child process, never linked. `overtone-ffmpeg` locates it, checks its version, streams decoded PCM over a pipe, runs integrity checks, and runs encode jobs with progress and cancellation.
- The desktop app will bundle a static, audio-only, LGPL ffmpeg as a Tauri sidecar (release packaging). The CLI and development builds use the same lookup order and fall back to a system ffmpeg.

## Why a child process, not bindings

| | Child process | `ffmpeg-next` 9.1 / `rsmpeg` 0.18 |
|---|---|---|
| Crash on a bad file | Kills one ffmpeg process; the scan or batch continues, like the per-track `catch_unwind` around Symphonia | Takes down the app or CLI |
| Build | No C toolchain, headers, or `pkg-config` | Links FFmpeg libraries; `rsmpeg` 0.18 targets FFmpeg 8.0, one major version behind the local 9.1 |
| Updates | Swap the binary | Rebuild against new headers; bindings lag FFmpeg releases |
| License boundary | Separate program, so the LGPL build terms stay simple | Linking LGPL static libraries adds relinking obligations |
| Cost | About 25 ms to start a process | None |
| Data path | PCM over a pipe | In-process buffers |

The 25 ms start cost is small next to decoding a track (tens to hundreds of milliseconds), and batch work runs one file per core either way, so the bindings' only advantage doesn't matter here.

## Measurements

MacBook Air M3, ffmpeg 9.1 (Homebrew), `cargo run --release -p overtone-audio --example decode_bench -- local/music 15`. Single-threaded full decodes of the first 15 files of each format on the `local/` testing ground (fewer for AIFF). Process start was measured as `ffmpeg -version` 20 times: 24.6 ms each.

| Format | Files | Audio (min) | Symphonia (s) | ffmpeg pipe (s) | ffmpeg / Symphonia | Length differs | Max sample difference |
|---|---|---|---|---|---|---|---|
| AIFF | 3 | 16.3 | 0.15 | 0.35 | 2.34x | 0 | 0 |
| FLAC | 15 | 59.1 | 2.51 | 1.42 | 0.57x | 0 | 0 |
| M4A | 15 | 62.9 | 2.61 | 2.88 | 1.10x | 15 | 0 |
| MP3 | 15 | 57.7 | 2.18 | 3.40 | 1.56x | 1 | 5.4e-6 |
| WAV | 15 | 53.3 | 0.53 | 0.94 | 1.79x | 0 | 0 |

- **Lossless decodes agree bit for bit.** Every FLAC, WAV, and AIFF sample from ffmpeg equals Symphonia's. That makes Symphonia a sound independent check on ffmpeg's output.
- **Lossy decodes differ in length, not content.** All 15 M4A files and one MP3 have different sample counts because the two decoders trim encoder delay and padding differently; where lengths match, samples agree within 5.4e-6. Checks on lossy output must allow for this.
- **Speed is close enough to not matter.** ffmpeg is faster on FLAC and slower on PCM and MP3, mostly from the pipe and process start. Analysis keeps Symphonia, so this only affects the fallback and conversion paths.

## How `overtone-ffmpeg` finds ffmpeg

1. `OVERTONE_FFMPEG`, if set.
2. `ffmpeg` next to the running executable, where Tauri puts an `externalBin` sidecar.
3. `PATH`, then `/opt/homebrew/bin` and `/usr/local/bin`, because apps opened from Finder don't inherit the shell's `PATH`.

Versions older than 6.0 are refused. Tests that need ffmpeg skip with a note when none is installed, so CI runners without it still pass; the pre-push hooks on the development machine run them.

## The bundled build (for release packaging)

Not built yet; release packaging builds, signs, and notarizes it. The intended configuration is audio-only, static, and LGPL (no `--enable-gpl` or `--enable-nonfree`, so no `fdk-aac`):

```text
--disable-everything --disable-autodetect --disable-network --disable-doc
--disable-ffplay --disable-ffprobe --enable-ffmpeg --enable-static --disable-shared
--enable-audiotoolbox --enable-libmp3lame --enable-libsoxr
--enable-protocol=file,pipe
--enable-demuxer=aac,aiff,asf,caf,flac,mov,mp3,ogg,w64,wav
--enable-muxer=adts,aiff,flac,ipod,mp3,mp4,null,wav
--enable-parser=aac,flac,mpegaudio,opus,vorbis
--enable-decoder=aac,aac_fixed,alac,flac,mp3,mp3float,opus,vorbis,wmalossless,wmapro,wmav1,wmav2,pcm_f32le,pcm_s16be,pcm_s16le,pcm_s24be,pcm_s24le,pcm_s32le
--enable-encoder=aac_at,alac,flac,libmp3lame,pcm_f32le,pcm_s16be,pcm_s16le,pcm_s24be,pcm_s24le
--enable-filter=aformat,anull,aresample,astats,ebur128,silencedetect,volumedetect
```

`libmp3lame` and `libsoxr` are LGPL and built from source as static libraries. Record the exact FFmpeg, LAME, and soxr versions and the build's `-buildconf` output here when the first release bundles it.
