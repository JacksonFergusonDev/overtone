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
