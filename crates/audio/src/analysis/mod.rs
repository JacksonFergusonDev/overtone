//! Decoding, spectral quality analysis, waveform peaks, and spectrograms.
//! Owned by the spectral analysis work package.
//!
//! [`analyze`] decodes a file once, read-only, and accumulates everything in
//! the same pass: a Welch-averaged spectrum with a peak-hold for the
//! verdict, waveform peaks for the scrub bar, sample peaks and clipping
//! runs, and optionally a spectrogram PNG. See [`verdict`] for how the
//! spectrum becomes a [`QualityVerdict`] and
//! `engineering/decisions/audio-quality-analysis.md` for the measurements
//! behind the thresholds.
//!
//! Verdicts only flag tracks for a human to check by ear and spectrogram.
//! Nothing here deletes, moves, or rewrites audio.

pub mod decode;
pub mod fingerprint;
mod source;
mod spectrogram;
mod spectrum;
pub mod verdict;
mod waveform;
mod window;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use overtone_ffmpeg::Ffmpeg;
use overtone_model::{LoudnessReport, QualityReport, QualityVerdict, ReviewStatus, UnixMillis};

use source::Source;

pub use decode::{Decoder, StreamInfo};
pub use spectrogram::{SPECTROGRAM_BOTTOM_DB, SPECTROGRAM_TOP_DB, SpectrogramOptions};
pub use verdict::{SpectralFeatures, Wall, WallKind};
pub use waveform::{WAVEFORM_BUCKETS, WAVEFORM_ENCODING_VERSION, Waveform};

use crate::AudioError;

/// Version of the analyzer's heuristics. Bump it whenever thresholds or
/// measurements change, so stored reports from older versions are re-run.
pub const ANALYZER_VERSION: u32 = 2;

/// Points in [`Analysis::spectrum_summary`].
pub const SPECTRUM_SUMMARY_POINTS: usize = 512;

/// Samples at or above this magnitude count as full scale for clipping.
/// 16-bit full scale is 32767/32768 = 0.99997.
const CLIP_LEVEL: f32 = 0.9999;
/// Consecutive full-scale samples in one channel that count as a clip.
const CLIP_RUN: u32 = 3;

/// Options for [`analyze`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// Also render a spectrogram PNG from the same pass.
    pub spectrogram: Option<SpectrogramOptions>,
    /// ffmpeg to decode with when Symphonia can't (HE-AAC, Opus, WMA,
    /// damaged files). `None` records such files as Unchecked.
    pub fallback: Option<Ffmpeg>,
}

/// Everything one analysis pass produces.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// Verdict for the library, with `review` set to Unreviewed.
    pub report: QualityReport,
    /// Measurements behind the verdict.
    pub features: SpectralFeatures,
    pub stream: StreamInfo,
    pub waveform: Waveform,
    /// Mean spectrum in dB, [`SPECTRUM_SUMMARY_POINTS`] values from 0 Hz to
    /// Nyquist, for the UI's spectrum view.
    pub spectrum_summary: Vec<f32>,
    pub duration_ms: u64,
    pub sample_rate_hz: u32,
    /// Sample peak per channel (linear, 1.0 = full scale).
    pub channel_peaks: Vec<f32>,
    /// Runs of at least three consecutive full-scale samples in one channel.
    pub clipped_runs: u64,
    /// Damaged packets the decoder skipped.
    pub skipped_packets: u64,
    /// Whether the audio was decoded by ffmpeg because Symphonia couldn't.
    pub decoded_by_ffmpeg: bool,
    /// Loudness, true peak, clipping, and leading silence.
    pub loudness: LoudnessReport,
}

/// Samples below this magnitude (-60 dBFS) count as silence.
const SILENCE_LEVEL: f32 = 0.001;

/// Decodes `path` read-only and analyzes it in one pass.
///
/// A panic anywhere in decoding or analysis is caught and returned as
/// [`AudioError::Panicked`], so one hostile file can't take down a batch.
/// (This relies on unwinding; a `panic = "abort"` profile defeats it.)
pub fn analyze(path: &Path, options: &AnalysisOptions) -> Result<Analysis, AudioError> {
    let first = guarded(path, || {
        analyze_source(path, options, Source::Symphonia(Decoder::open(path)?))
    });
    let (Err(symphonia_error), Some(ffmpeg)) = (&first, &options.fallback) else {
        return first;
    };
    guarded(path, || {
        let mut analysis = analyze_source(path, options, Source::ffmpeg(ffmpeg, path)?)?;
        if let Some(reason) = analysis.report.reason.as_mut() {
            reason.push_str(&format!(
                " Decoded with ffmpeg because Symphonia couldn't: {symphonia_error}."
            ));
        }
        Ok(analysis)
    })
    .map_err(|_| first.expect_err("checked above"))
}

fn guarded(
    path: &Path,
    work: impl FnOnce() -> Result<Analysis, AudioError>,
) -> Result<Analysis, AudioError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(AudioError::Panicked {
            path: path.to_path_buf(),
            message,
        })
    })
}

/// A report recording that analysis failed, so the batch moves on and the
/// track isn't retried until the analyzer changes.
pub fn unchecked_report(reason: impl Into<String>) -> QualityReport {
    QualityReport {
        verdict: QualityVerdict::Unchecked,
        cutoff_hz: None,
        confidence: None,
        reason: Some(reason.into()),
        analyzer_version: ANALYZER_VERSION,
        analyzed_at: now_millis(),
        review: ReviewStatus::Unreviewed,
        reviewed_at: None,
    }
}

fn analyze_source(
    path: &Path,
    options: &AnalysisOptions,
    mut decoder: Source,
) -> Result<Analysis, AudioError> {
    let mut sample_rate = decoder.info().sample_rate_hz;

    let mut welch = None;
    let mut image = None;
    let mut waveform = waveform::WaveformBuilder::new();
    let mut peaks: Vec<f32> = Vec::new();
    let mut runs: Vec<u32> = Vec::new();
    let mut clipped_runs = 0_u64;
    let mut mono = Vec::new();
    let mut meter: Option<ebur128::EbuR128> = None;
    let mut frames_seen = 0_u64;
    let mut first_sound: Option<u64> = None;

    while let Some(block) = decoder.next_block()? {
        let channels = block.channels;
        if peaks.len() < channels {
            peaks.resize(channels, 0.0);
            runs.resize(channels, 0);
        }
        if welch.is_none() {
            if sample_rate == 0 {
                sample_rate = block.sample_rate;
            }
            if sample_rate == 0 {
                return Err(AudioError::Decode {
                    path: path.to_path_buf(),
                    message: "unknown sample rate".into(),
                });
            }
            let w = spectrum::Welch::new(sample_rate);
            image = options
                .spectrogram
                .clone()
                .map(|o| spectrogram::SpectrogramBuilder::new(o, w.bins(), w.bin_hz()));
            welch = Some(w);
            meter = ebur128::EbuR128::new(
                u32::try_from(channels).unwrap_or(2),
                sample_rate,
                ebur128::Mode::I
                    | ebur128::Mode::LRA
                    | ebur128::Mode::TRUE_PEAK
                    | ebur128::Mode::SAMPLE_PEAK,
            )
            .ok();
        }
        if let Some(m) = meter.as_mut() {
            // A channel-count change mid-stream (rare, broken files) stops
            // the meter rather than mixing layouts.
            if m.channels() as usize == channels {
                let _ = m.add_frames_f32(block.samples);
            } else {
                meter = None;
            }
        }
        if first_sound.is_none()
            && let Some(at) = block.samples.iter().position(|s| s.abs() > SILENCE_LEVEL)
        {
            first_sound = Some(frames_seen + (at / channels) as u64);
        }
        frames_seen += (block.samples.len() / channels) as u64;

        mono.clear();
        let samples = block.samples;
        let scale = 1.0 / channels as f32;
        for frame in samples.chunks_exact(channels) {
            let mut sum = 0.0;
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for (ch, &sample) in frame.iter().enumerate() {
                sum += sample;
                min = min.min(sample);
                max = max.max(sample);
                let magnitude = sample.abs();
                if magnitude > peaks[ch] {
                    peaks[ch] = magnitude;
                }
                if magnitude >= CLIP_LEVEL {
                    runs[ch] += 1;
                    if runs[ch] == CLIP_RUN {
                        clipped_runs += 1;
                    }
                } else {
                    runs[ch] = 0;
                }
            }
            waveform.push(min, max);
            mono.push(sum * scale);
        }
        if let Some(w) = welch.as_mut() {
            match image.as_mut() {
                Some(img) => w.push(&mono, &mut |power| img.push(power)),
                None => w.push(&mono, &mut |_| {}),
            }
        }
    }

    let frames = decoder.decoded_frames();
    if !decoder.decoded_by_ffmpeg() {
        check_full_length(path, decoder.info(), frames)?;
    }
    if frames == 0 {
        return Err(AudioError::Decode {
            path: path.to_path_buf(),
            message: "no audio frames decoded".into(),
        });
    }
    let welch = welch.unwrap_or_else(|| spectrum::Welch::new(sample_rate.max(1)));
    let summary = welch.finish();
    if let Some(img) = image {
        img.finish()?;
    }

    let duration_secs = frames as f64 / f64::from(sample_rate);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "duration is a small positive value"
    )]
    let (bitrate_kbps, duration_ms) = (
        decoder.bitrate_kbps(duration_secs),
        (duration_secs * 1000.0).round() as u64,
    );
    let info = decoder.info().clone();
    let judgement = verdict::judge(
        &summary,
        verdict::Context {
            format: info.format,
            sample_rate,
            bitrate_kbps,
        },
    );

    Ok(Analysis {
        report: QualityReport {
            verdict: judgement.verdict,
            cutoff_hz: judgement.cutoff_hz,
            confidence: judgement.confidence,
            reason: Some(judgement.reason),
            analyzer_version: ANALYZER_VERSION,
            analyzed_at: now_millis(),
            review: ReviewStatus::Unreviewed,
            reviewed_at: None,
        },
        features: judgement.features,
        spectrum_summary: summary.reduced(SPECTRUM_SUMMARY_POINTS),
        stream: info,
        waveform: waveform.finish(),
        duration_ms,
        sample_rate_hz: sample_rate,
        channel_peaks: peaks,
        clipped_runs,
        skipped_packets: decoder.skipped_packets(),
        decoded_by_ffmpeg: decoder.decoded_by_ffmpeg(),
        loudness: loudness_report(
            meter.as_ref(),
            clipped_runs,
            first_sound,
            frames,
            sample_rate,
        ),
    })
}

fn to_db(linear: f64) -> Option<f64> {
    (linear > 0.0).then(|| 20.0 * linear.log10())
}

fn loudness_report(
    meter: Option<&ebur128::EbuR128>,
    clipped_runs: u64,
    first_sound: Option<u64>,
    frames: u64,
    sample_rate: u32,
) -> LoudnessReport {
    let finite = |v: f64| v.is_finite().then_some(v);
    let peak = |f: &dyn Fn(&ebur128::EbuR128, u32) -> Result<f64, ebur128::Error>| {
        meter.and_then(|m| {
            (0..m.channels())
                .filter_map(|ch| f(m, ch).ok())
                .fold(None, |acc: Option<f64>, v| {
                    Some(acc.map_or(v, |a| a.max(v)))
                })
                .and_then(to_db)
        })
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a frame count in milliseconds"
    )]
    let leading_silence_ms = (first_sound.unwrap_or(frames) as f64 * 1000.0
        / f64::from(sample_rate.max(1)))
    .round() as u64;
    LoudnessReport {
        integrated_lufs: meter
            .and_then(|m| m.loudness_global().ok())
            .and_then(finite),
        loudness_range_lu: meter.and_then(|m| m.loudness_range().ok()).and_then(finite),
        true_peak_dbtp: peak(&|m, ch| m.true_peak(ch)),
        sample_peak_dbfs: peak(&|m, ch| m.sample_peak(ch)),
        clipped_runs,
        leading_silence_ms,
    }
}

/// HE-AAC with backward-compatible signalling (Apple's encoder writes it)
/// declares the full rate, so Symphonia decodes only the half-rate core:
/// half the frames, and a spectrum that stops near a quarter of the sample
/// rate. Catch it from the frame count so it falls back to ffmpeg instead
/// of producing a false verdict.
fn check_full_length(path: &Path, info: &StreamInfo, frames: u64) -> Result<(), AudioError> {
    let Some(hint) = info.frames_hint.filter(|h| *h > 0) else {
        return Ok(());
    };
    let ratio = frames as f64 / hint as f64;
    if info.format == overtone_model::AudioFormat::Aac && (0.4..0.6).contains(&ratio) {
        return Err(AudioError::UnsupportedCodec {
            path: path.to_path_buf(),
            reason: "the AAC stream decoded to half its declared length, so it is HE-AAC, \
                     which the decoder can't reproduce yet"
                .into(),
        });
    }
    Ok(())
}

fn now_millis() -> UnixMillis {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX)
}
