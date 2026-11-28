# Audio quality analysis and spectral forensics

Decided November 26, 2026. This records how Overtone detects suspected fake lossless files, upscaled lossy files, and fake hi-res containers, why verdicts only flag tracks for human review, and which Rust crates power the pipeline.

## Problem

DJ libraries commonly pick up files whose container claims higher fidelity than the underlying audio:

- **Fake lossless (lossy-to-lossless transcode):** an MP3, AAC, Opus, or Vorbis file decoded and re-wrapped into `FLAC`, `WAV`, `AIFF`, or `ALAC`.
- **Upscaled lossy (lossy-to-lossy transcode):** a 128 kbps or 192 kbps MP3 re-encoded at 320 kbps.
- **Fake hi-res:** 44.1 kHz or 48 kHz CD/lossy material upsampled into a 96 kHz or 192 kHz container with empty upper spectrum.

## Detection approach

1. **Decode actual PCM audio, ignoring container claims.** Decode across `FLAC`, `WAV`, `AIFF`, `ALAC`, `MP3`, and `M4A/AAC` using pure-Rust `symphonia`. Record declared container format, sample rate, bit depth, and reported bitrate alongside the decoded PCM stream.
2. **Windowed FFT spectral cutoff estimation.** Sample multiple windows across the active body of the track (skipping leading/trailing silence) using a Blackman-Harris window via `rustfft` to minimize spectral leakage from loud low-end sub-bass (critical for electronic and dub music). Compute the average and peak-relative power spectral density and locate the effective high-frequency shelf/cliff:
   - ~16 kHz hard shelf inside a 320 kbps or lossless container strongly indicates a ~128 kbps lossy origin.
   - ~18 to 19 kHz shelf indicates ~192 kbps origin.
   - ~20 to 20.5 kHz brick-wall shelf inside a lossless container indicates a 320 kbps MP3/AAC transcode (true 44.1 kHz lossless typically carries energy up to ~21 to 22.05 kHz).
   - Empty spectrum above 22.05 kHz inside a 96 kHz or 192 kHz container indicates fake hi-res upsampling.
3. **Additional integrity signals (Phase 2+):** peak clipping / true-peak ceiling checks, dynamic range / LUFS loudness measurement, and optional brick-wall steepness scoring (distinguishing an encoder low-pass cliff from a gentle analog roll-off).
4. **Parallel batch scanning.** Use `rayon` across tracks; compile `overtone-audio` at `opt-level = 3` in dev builds so library scans stay fast during development.

## Prior art surveyed

- **`lossless-checker`** (`NaughtyDogOfSchrodinger/lossless-checker`, Rust): pure-Rust reference architecture using `symphonia` + `rustfft` with Blackman-Harris windowed FFTs and peak-relative cutoff cliffs for fake lossless and fake hi-res detection.
- **`flaccheck`** (`dasunNimantha/flaccheck`, Rust): multi-tier forensic detectors (spectral cutoffs, brick-wall fingerprints, MDCT quantization residuals, fake hi-res padding).
- **`flac-detective`** (Python): traffic-light verdict model (`AUTHENTIC`, `WARNING`, `SUSPICIOUS`, `FAKE_CERTAIN`).
- **Spek** and **Fakin' The Funk**: manual visual spectrogram verification (Spek) and proprietary Windows-only GUI batch checking (Fakin' The Funk).

## Safety invariant: flag, never auto-delete

Spectral cutoff heuristics have known false positives on genuinely warm or lo-fi recordings, vinyl rips, older masters, and heavily low-passed dubs or electronic productions. Therefore:

- Overtone assigns a structured verdict (`Pass`, `Warning`, `Suspect`, or `Unchecked`) with measured cutoff frequency, confidence, and reason.
- Flagged tracks are never deleted or quarantined automatically. Instead, the desktop app pairs each flagged track with an **in-app spectrogram view** (rendered in `overtone-audio` and displayed in the UI) and instant playback so you can verify by eye and ear, then mark a track as **Verified OK** (dismissing the false positive) or **Replace / Remove**.

## Waveform peaks and spectrogram rendering

Because `overtone-audio` already decodes every track during ingest and quality scanning, it computes and caches two compact visual assets in the same pass:

- **Downsampled waveform peak array** (for example, 800 to 1,200 min/max amplitude buckets per track) stored in SQLite or app cache for instant clickable scrubbing in the Tauri frontend.
- **Spectrogram image** (PNG, generated on demand or cached for flagged tracks) so you never need to leave Overtone for Spek when checking a suspicious file.

## Analyzer version 1

`overtone_audio::analysis::analyze` decodes each file once, read-only, with `symphonia` 0.6.1 (pinned) and accumulates everything in that pass: a Welch-averaged spectrum of the mono mix (4-term Blackman-Harris, 75% overlap, FFT size chosen for about 5.4 Hz bins at any sample rate: 8192 at 44.1/48 kHz, 16384 at 88.2/96 kHz), a 99th-percentile peak-hold per 16-bin band, 1,200 min/max waveform buckets, sample peaks, clipping runs, and optionally a spectrogram. Thresholds and the reasoning behind each rule are in `crates/audio/src/analysis/verdict.rs`.

- **Wall detection.** A wall is the lowest frequency from 11 kHz up where the median-smoothed mean spectrum falls at least 20 dB between 250-1000 Hz below and above it (12 dB onto a floor flat to within 1 dB over 2 kHz or more, which is what clipping splatter or dither looks like), the mean never rises more than 10 dB past it, and the peak-hold above it stays at least 20 dB under the program just below it. Its local slope over 500 Hz is reported in dB/octave; 250 dB/oct or more with a flat floor is a hard (encoder-like) wall.
- **Sporadic content.** If the peak-hold rises 12 dB over the floor somewhere more than 1.5 kHz above a wall, some passages carry treble a lower-bitrate source could not have kept. In a lossy file this is usually an encoder dropping its top band in busy passages, so the verdict is at most Warning. In a lossless file a hard wall at 15-17.2 kHz stays Suspect with lower confidence, because decoding a loud MP3 to 16-bit clips and splatters into the empty band.
- **Waveform encoding.** The library stores waveform peaks as an opaque blob from `Waveform::encode`: one version byte (1), then a signed 8-bit min and max per bucket (`round(x * 127)`), about 2.4 KB per track.
- **Spectrogram.** 8-bit indexed PNG, magma palette, 0 to -120 dBFS, linear or log frequency, rendered from the same FFT frames (no second decode), written atomically through `overtone-fs`.
- **Failures.** HE-AAC (declared, or implied by an AAC stream at 24 kHz or less) and 32-bit FLAC are refused before decoding. Since analyzer version 2, HE-AAC is also caught after decoding when an AAC stream yields about half its declared frames (Apple's encoder signals SBR in a way Symphonia misses, so it silently decodes only the half-rate core), and every file Symphonia refuses or fails on is decoded through ffmpeg instead when ffmpeg is installed ([ffmpeg](ffmpeg.md)); its report says so. Damaged packets are skipped. A truncated file is analyzed on what it holds. Panics are caught per track with `catch_unwind`, which needs unwinding: the workspace release profile currently sets `panic = "abort"`, which defeats it.

### Thresholds

| Rule | Lossless 44.1/48 kHz | Hi-res (88.2 kHz and up) | MP3 / AAC |
|---|---|---|---|
| Hard wall 15-17.2 kHz, nothing above | Suspect (128 kbps origin) | Suspect (fake hi-res) | Suspect if 1.5 kHz or more under the bitrate's expected cutoff, Warning if 0.7 kHz under, else Pass |
| Hard wall 17.2-19.6 kHz | Suspect if at least 600 dB/oct, 30 dB drop and 30 dB peak-hold gap, else Warning | Suspect | as above |
| Hard wall 19.6-20.8 kHz | Warning (a 320 kbps transcode or a genuine master) | Suspect | as above |
| Hard wall under 15 kHz | Warning (low bitrate or deliberate filter) | Suspect | as above |
| Soft shelf (80-250 dB/oct) under 20.8 kHz | Warning | not flagged on its own | as above |
| Wall with sporadic content above | Warning (Suspect kept at 15-17.2 kHz) | not flagged | Warning if under the Suspect margin, else Pass |
| No wall, content reaches 20.8 kHz | Pass | Pass if content above 24.5 kHz, else Warning | Pass if within 0.7 kHz of the expected cutoff |
| No wall, content stops earlier | Inconclusive | Warning | Warning if 1.5 kHz under the expected cutoff and the program is louder than -75 dB, else Inconclusive |
| Under 2 s of audible audio | Inconclusive | Inconclusive | Inconclusive |

Expected lossy cutoffs come from LAME 3.100's lowpass table less tolerance for other encoders: 112-143 kbps 15.5 kHz, 144-175 kbps 16.8 kHz, 176-223 kbps 17.8 kHz, 224-287 kbps 18.8 kHz, 288 kbps and up 19.5 kHz; AAC 0.5 kHz lower. The bitrate is measured from compressed packet sizes, not read from tags, so embedded artwork doesn't inflate it.

## Analyzer version 2: ffmpeg fallback and integrity checks

Decided November 27, 2026 (Phase 6 step 4). The spectral heuristics are unchanged; version 2 only means more files can be analyzed, so an audit after upgrading reanalyzes everything once. On `local/` the verdicts are identical to version 1 (1,092 pass, 39 warning, 15 suspect, 5 inconclusive, in 105 s on 4 threads). None of the 69 M4A files there are HE-AAC: all decode within 0.06% of ffmpeg's length.

`overtone-cli audit --integrity` decodes each file in full with ffmpeg (`-err_detect crccheck+bitstream+buffer`) and stores every error line beside the verdict. It is a separate, opt-in pass because it costs as much as the audit. Two refinements came from running it on `local/` (1,151 files, 106 s):

- **Artwork noise.** ffmpeg probes embedded artwork even when only audio is mapped, so a mislabelled cover ("Invalid PNG signature" on a JPEG) looked like damage. Lines from image decoders and "Last message repeated" are dropped.
- **Junk at the edges.** 36 files, mostly MP3s, report "Header missing" only in their last seconds, where zero padding or leftover bytes sit before an ID3v1 tag. A file with errors is checked again without its first 0.5 s and last 2.5 s; if that decodes cleanly it is recorded as edge-only, which `ls --damaged` leaves out and `show` explains.

That leaves 16 files with errors inside the music: 14 MP3s on which ffmpeg reports `bits_left` warnings in most frames (from 2 to 47,011 lines), one MP3 with corrupt frames, and one WAV ffmpeg can't open at all (lofty can't read its chunk sizes either). Whether the `bits_left` files sound damaged is a listening check in the [manual verification checklist](../manual-verification.md).

## Measurements

Measured November 26, 2026 on an Apple M3 MacBook Air (fanless, 4 performance and 4 efficiency cores, macOS 27.2.1, rustc 1.100.0, release build) against `local/music`: 1,151 audio files (366 FLAC, 652 MP3, 69 M4A, 61 WAV, 3 AIFF), 80 hours of audio, all at 44.1 or 48 kHz. Files were only read. The runner lives outside the repository; the numbers come from its per-file timings.

### S1: decode throughput and failures

Single thread, a 191-file sample (every sixth file, 833 minutes of audio), files in the page cache:

| Codec | Files | Decode only | Full analysis | Decode share |
|---|---|---|---|---|
| FLAC 16-bit | 64 | 1,930x realtime | 900x | 46% |
| MP3 | 105 | 1,580x | 850x | 54% |
| AAC (M4A) | 12 | 1,240x | 720x | 59% |
| WAV | 9 | 7,700x | 1,500x | 19% |
| AIFF | 1 | 8,300x | 1,560x | 19% |
| All | 191 | 1,720x | 875x | 51% |

A five-minute track takes about 0.35 s on one core. Over the whole corpus no file fails to decode. Two MP3s with large embedded PNG artwork first failed with "too many undecodable packets", because decoding stopped after 64 consecutive bad packets and resynchronising past the artwork needs more; the limit is now 100,000, and a file with no audio still fails cleanly. The corpus has no HE-AAC files (all 69 M4A files are AAC-LC), and no panics occurred.

### S2 and S3: where the time goes

From the S1 numbers, per second of 44.1 kHz audio on one core: decoding takes about 0.58 ms (51%), FFTs about 0.19 ms (17%: 21.5 real 8192-point FFTs at 8.9 us each with `realfft`), and the rest (window multiply, power and peak-hold accumulation, mono mix, waveform and clip tracking) about 0.37 ms (32%).

FFT is just over the 15% threshold, so S2 compared `phastft` 0.4.1: a complex FFT of N/2 points, the core of a packed real FFT of N, takes 5.4 us for N = 8192 against 8.9 us for `realfft`'s whole real transform. Even ignoring the real-FFT post-processing, switching would save at most 7% of total time, so `realfft` stays. Halving the overlap (hop N/2) is the cheaper lever if analysis ever needs to be faster.

### S5: detection on real transcodes

24 genuine CD-quality FLACs from the corpus (90 s of each, from 60 s in), encoded with LAME 4.0 and macOS AAC, decoded to 16-bit, and rewrapped as FLAC:

| Source | Pass | Warning | Suspect | Median cutoff |
|---|---|---|---|---|
| Genuine | 24 | 0 | 0 | full band |
| MP3 128 kbps | 0 | 0 | 24 | 16.6 kHz |
| MP3 192 kbps | 2 | 19 | 3 | 18.7 kHz |
| MP3 256 kbps | 2 | 19 | 3 | 19.4 kHz |
| MP3 320 kbps | 5 | 19 | 0 | 20.1 kHz |
| MP3 V2 | 2 | 20 | 2 | 18.7 kHz |
| MP3 V0 | 23 | 1 | 0 | full band |
| AAC 128 kbps | 1 | 22 | 1 | |
| AAC 256 kbps | 24 | 0 | 0 | full band |

128 kbps sources are caught every time, and 192-320 kbps sources mostly land on Warning, as intended. LAME V0 and 256 kbps AAC keep content to Nyquist and can't be detected from the spectrum; that needs the MDCT or frame-alignment detector (S9). Decoding these loud masters clipped, which put a flat floor 15-20 dB under the program above the cutoff; the flat-floor rule exists because of this.

The synthetic fixtures behave as designed: brick walls at 16 and 19 kHz are Suspect, 20.5 kHz is Warning, 12 and 60 dB/oct roll-offs pass, a 96 kHz file with content to 20 kHz is Suspect, and a full-band 96 kHz file passes.

### S5: verdicts over the DJ's library

| Format | Pass | Warning | Suspect | Inconclusive |
|---|---|---|---|---|
| FLAC (366) | 350 | 15 | 1 | 0 |
| MP3 (652) | 614 | 20 | 13 | 5 |
| M4A (69) | 69 | 0 | 0 | 0 |
| WAV (61) | 56 | 4 | 1 | 0 |
| AIFF (3) | 3 | 0 | 0 | 0 |

The 15 Suspects, checked against their spectrograms:

- 13 "320 kbps" MP3s with a flat 15.4-16.8 kHz ceiling and nothing above it for the whole track, the signature of a 128 kbps source re-encoded at 320 kbps: Circadian "Energy In Motion", Junk Mail "Lock Stock", Dimension "UK (Bou Remix)", Traumatize & Rodeo "Badge Flasher (VIP)", Konflict "Messiah", La Roux "In For The Kill (High Contrast Remix)", Olivia Rodrigo "Deja Vu (Jakar Bootleg)", Oppidan & Cutty Ranks "Armed & Dangerous", Peggy Gou "It Goes Like (Nanana) (Sammy Virji Edit)", Badflite & RYVM "Where the Sun Goes Down", "April 14th 2020", Romy & Fred Again "Strong", and A$AP Rocky "Lord Pretty Flacko Jodye 2 (Hntr Remix)". Several are free-download bootlegs, where this is common.
- "Rolling In The Deep (Dimension DNB Remix)" (FLAC) and "Work (Proppa x Rich DietZ x Smith & Sorren Tech House Edit)" (WAV): a hard 15.8 kHz ceiling through most of the track, with brief content above it only in the loudest passages. Most likely lossy sources wrapped as lossless.

Of the Warnings, 13 lossless files have a steep wall at 20.1-20.8 kHz, which a 320 kbps transcode and some genuine masters share. The rest are MP3s with a 15-18 kHz edge and sporadic content above it, a few quiet ambient interludes whose content stops near 16 kHz, and two 192 kbps MP3s with a 16.7 kHz wall. None of these can be settled from the spectrum alone.

### S8: threads and thermals

Whole corpus, files in the page cache:

| Threads | Wall time | Throughput | First vs last quarter |
|---|---|---|---|
| 4 (cool start) | 96 s | about 3,000x realtime | |
| 8 | 79 s | 3,740x | 3,744x to 3,553x (-5%) |
| 4 (straight after the 8-thread run) | 124 s | 2,380x | 2,574x to 2,399x (-7%) |

Throughput scales almost linearly to 4 threads, the performance cores. Eight threads add 25-50% by also using the efficiency cores but keep the whole machine busy. Sustained load throttles the fanless M3: the same 4-thread scan ran 22% slower straight after the 8-thread one, and throughput sags 5-7% within a run. `pmset -g therm` recorded no thermal warning levels; `powermetrics` needs root and was not run. `DEFAULT_AUDIT_THREADS` is 4, which scans this library in under two minutes while leaving headroom for playback and the UI. Results are identical across thread counts.
