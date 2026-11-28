//! Chromaprint fingerprints for AcoustID, computed like `fpcalc`: the first
//! two minutes, algorithm 2 (Chromaprint's default, `preset_test2`).

use std::path::Path;

use overtone_ffmpeg::{DecodeOptions, Ffmpeg};
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};

use super::decode::Decoder;
use crate::AudioError;

/// Seconds of audio fingerprinted, as `fpcalc` does by default.
pub const FINGERPRINT_SECONDS: u64 = 120;

/// A compressed Chromaprint fingerprint and the file's full duration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFingerprint {
    /// Compressed with Chromaprint's format; AcoustID wants it as
    /// URL-safe base64.
    pub compressed: Vec<u8>,
    /// The raw fingerprint, for comparing two files.
    pub raw: Vec<u32>,
    pub duration_secs: u64,
}

fn to_i16(samples: &[f32], out: &mut Vec<i16>) {
    out.clear();
    #[allow(clippy::cast_possible_truncation, reason = "clamped to the i16 range")]
    out.extend(
        samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16),
    );
}

/// Fingerprints `path` with Symphonia, or with ffmpeg when Symphonia can't
/// decode it and `fallback` is given.
pub fn fingerprint(path: &Path, fallback: Option<&Ffmpeg>) -> Result<AudioFingerprint, AudioError> {
    match fingerprint_symphonia(path) {
        Ok(found) => Ok(found),
        Err(err) => match fallback {
            Some(ffmpeg) => fingerprint_ffmpeg(path, ffmpeg).map_err(|_| err),
            None => Err(err),
        },
    }
}

fn config() -> Configuration {
    Configuration::preset_test2()
}

fn finish(
    printer: &mut Fingerprinter,
    config: &Configuration,
    duration_secs: u64,
) -> AudioFingerprint {
    printer.finish();
    let raw = printer.fingerprint().to_vec();
    AudioFingerprint {
        compressed: FingerprintCompressor::from(config).compress(&raw),
        raw,
        duration_secs,
    }
}

fn start_error(path: &Path, err: impl std::fmt::Debug) -> AudioError {
    AudioError::Decode {
        path: path.to_path_buf(),
        message: format!("couldn't start fingerprinting: {err:?}"),
    }
}

fn fingerprint_symphonia(path: &Path) -> Result<AudioFingerprint, AudioError> {
    let mut decoder = Decoder::open(path)?;
    let info = decoder.info().clone();
    let config = config();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(info.sample_rate_hz, u32::from(info.channels))
        .map_err(|e| start_error(path, e))?;
    let limit = FINGERPRINT_SECONDS * u64::from(info.sample_rate_hz);
    let mut fed = 0u64;
    let mut buf = Vec::new();
    while let Some(block) = decoder.next_block()? {
        if fed < limit {
            let frames = (block.samples.len() / block.channels) as u64;
            let take =
                usize::try_from(frames.min(limit - fed)).unwrap_or(usize::MAX) * block.channels;
            to_i16(&block.samples[..take], &mut buf);
            printer.consume(&buf);
            fed += frames;
        }
    }
    let duration = decoder.decoded_frames() / u64::from(info.sample_rate_hz.max(1));
    if decoder.decoded_frames() == 0 {
        return Err(AudioError::Decode {
            path: path.to_path_buf(),
            message: "no audio frames decoded".into(),
        });
    }
    Ok(finish(&mut printer, &config, duration))
}

fn fingerprint_ffmpeg(path: &Path, ffmpeg: &Ffmpeg) -> Result<AudioFingerprint, AudioError> {
    let decode_error = |err: overtone_ffmpeg::FfmpegError| AudioError::Decode {
        path: path.to_path_buf(),
        message: err.to_string(),
    };
    let mut pcm = ffmpeg
        .decode(path, &DecodeOptions::default())
        .map_err(decode_error)?;
    let rate = pcm.sample_rate();
    let channels = usize::from(pcm.channels());
    let config = config();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(rate, u32::from(pcm.channels()))
        .map_err(|e| start_error(path, e))?;
    let limit = FINGERPRINT_SECONDS * u64::from(rate) * channels as u64;
    let mut samples = 0u64;
    let mut block = vec![0f32; 8192 * channels];
    let mut buf = Vec::new();
    loop {
        let n = pcm.read(&mut block).map_err(decode_error)?;
        if n == 0 {
            break;
        }
        if samples < limit {
            let take = n.min(usize::try_from(limit - samples).unwrap_or(usize::MAX));
            to_i16(&block[..take - take % channels], &mut buf);
            printer.consume(&buf);
        }
        samples += n as u64;
    }
    pcm.finish().map_err(decode_error)?;
    let duration = samples / channels.max(1) as u64 / u64::from(rate.max(1));
    Ok(finish(&mut printer, &config, duration))
}

/// The fraction of matching bits between two raw fingerprints, aligned at
/// their start (1.0 identical, about 0.5 unrelated).
pub fn similarity(a: &[u32], b: &[u32]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let differing: u32 = a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum();
    1.0 - f64::from(differing) / (n as f64 * 32.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_chromaprint_algorithm_two() {
        // fpcalc's default; the compressed header names it, and AcoustID
        // rejects fingerprints from other algorithms.
        assert_eq!(config().id(), 1);
    }
}
