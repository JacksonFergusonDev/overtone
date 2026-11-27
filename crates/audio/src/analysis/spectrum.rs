//! Welch-averaged power spectrum with a per-band peak-hold.
//!
//! The mono mix is cut into overlapping frames of `n` samples (75% overlap,
//! hop `n / 4`, which keeps Blackman-Harris frames close to statistically
//! independent), windowed, and transformed with a real FFT. Two summaries
//! accumulate over every audible frame:
//!
//! - **Mean power per bin** (in `f64`, averaged in the power domain, not in
//!   dB). This is the reliable view for stationary program material and the
//!   main input to the cliff detector.
//! - **99th-percentile power per band** of [`BAND_BINS`] bins, from a
//!   0.5 dB histogram per band. Sparse high-frequency content (a hi-hat
//!   every few bars in a dub track) barely moves the mean but shows in the
//!   peak-hold, so a genuine file shows *some* frames above a candidate
//!   cutoff while a transcode shows none. The percentile ignores the odd
//!   click that a plain maximum would catch.
//!
//! Levels are on a tonal dBFS scale: a full-scale sine reads 0 dB in its
//! peak bin. 16-bit quantization noise reads about -131 dB per bin at
//! 5.4 Hz resolution.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "DSP index and level conversions on bounded, non-negative values"
)]

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use super::window::blackman_harris;

/// Bins per peak-hold band (about 86 Hz at 44.1 kHz with `n = 8192`).
pub(crate) const BAND_BINS: usize = 16;
/// Frames quieter than this mean square (-60 dBFS) don't count as audible.
const ACTIVE_GATE_POWER: f32 = 1.0e-6;
/// Lowest level the summaries represent, in dB.
pub(crate) const FLOOR_DB: f32 = -200.0;
const HIST_TOP_DB: f32 = 10.0;
const HIST_STEP_DB: f32 = 0.5;
const HIST_BINS: usize = ((HIST_TOP_DB - FLOOR_DB) / HIST_STEP_DB) as usize + 1;
/// Fraction of audible frames allowed above the peak-hold level.
const HOLD_PERCENTILE_TOP: f64 = 0.01;

/// FFT length for a sample rate, keeping bins between about 2.9 and 6 Hz
/// wide so thresholds in Hz mean the same at 44.1 kHz and 192 kHz: 8192 at
/// 44.1 and 48 kHz, 16384 at 88.2 and 96 kHz, 32768 at 176.4 and 192 kHz.
pub(crate) fn fft_len(sample_rate: u32) -> usize {
    let rate = f64::from(sample_rate.max(1));
    let mut n = 8192_usize;
    while rate / n as f64 > 6.0 {
        n *= 2;
    }
    while n > 1024 && rate / (n as f64) < 2.9 {
        n /= 2;
    }
    n
}

/// Finished spectral summaries for one track.
#[derive(Debug, Clone)]
pub(crate) struct Summary {
    pub sample_rate: u32,
    pub bin_hz: f32,
    /// Mean power per FFT bin, in dB.
    pub mean_db: Vec<f32>,
    pub band_hz: f32,
    /// 99th-percentile power per band, in dB.
    pub hold_db: Vec<f32>,
    /// Seconds of audio above the audibility gate.
    pub active_seconds: f32,
}

pub(crate) struct Welch {
    sample_rate: u32,
    n: usize,
    hop: usize,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    pending: Vec<f32>,
    start: usize,
    power: Vec<f32>,
    power_sum: Vec<f64>,
    active_frames: u64,
    bands: usize,
    hist: Vec<u32>,
    scale: f32,
}

impl Welch {
    pub fn new(sample_rate: u32) -> Self {
        let n = fft_len(sample_rate);
        let window = blackman_harris(n);
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let window_sum: f64 = window.iter().map(|&w| f64::from(w)).sum();
        let bins = n / 2 + 1;
        let bands = bins.div_ceil(BAND_BINS);
        Self {
            sample_rate,
            n,
            hop: n / 4,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            pending: Vec::with_capacity(n * 2),
            start: 0,
            power: vec![0.0; bins],
            power_sum: vec![0.0; bins],
            active_frames: 0,
            bands,
            hist: vec![0; bands * HIST_BINS],
            // One-sided amplitude scaling: a full-scale sine reads 0 dB.
            scale: (4.0 / (window_sum * window_sum)) as f32,
        }
    }

    pub fn bins(&self) -> usize {
        self.power.len()
    }

    pub fn bin_hz(&self) -> f32 {
        self.sample_rate as f32 / self.n as f32
    }

    /// Adds mono samples. `on_frame` sees every frame's power spectrum
    /// (audible or not), for the spectrogram.
    pub fn push(&mut self, mono: &[f32], on_frame: &mut dyn FnMut(&[f32])) {
        self.pending.extend_from_slice(mono);
        while self.pending.len() - self.start >= self.n {
            self.process_frame(on_frame);
            self.start += self.hop;
        }
        if self.start >= self.n * 4 {
            self.pending.drain(..self.start);
            self.start = 0;
        }
    }

    fn process_frame(&mut self, on_frame: &mut dyn FnMut(&[f32])) {
        let frame = &self.pending[self.start..self.start + self.n];
        let mut energy = 0.0_f32;
        for ((slot, &sample), &w) in self.input.iter_mut().zip(frame).zip(&self.window) {
            energy += sample * sample;
            *slot = sample * w;
        }
        let audible = energy / self.n as f32 >= ACTIVE_GATE_POWER;
        // The buffers are sized by the planner, so this can't fail.
        let _ = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch);
        for (power, value) in self.power.iter_mut().zip(&self.output) {
            *power = value.norm_sqr() * self.scale;
        }
        on_frame(&self.power);
        if !audible {
            return;
        }
        self.active_frames += 1;
        for (sum, &power) in self.power_sum.iter_mut().zip(&self.power) {
            *sum += f64::from(power);
        }
        for (band, chunk) in self.power.chunks(BAND_BINS).enumerate() {
            let mean = chunk.iter().sum::<f32>() / chunk.len() as f32;
            let index = ((to_db(mean) - FLOOR_DB) / HIST_STEP_DB) as usize;
            self.hist[band * HIST_BINS + index.min(HIST_BINS - 1)] += 1;
        }
    }

    pub fn finish(self) -> Summary {
        let frames = self.active_frames.max(1) as f64;
        let mean_db = self
            .power_sum
            .iter()
            .map(|&sum| to_db((sum / frames) as f32))
            .collect();
        let allowed_above = (self.active_frames as f64 * HOLD_PERCENTILE_TOP).floor() as u64;
        let hold_db = (0..self.bands)
            .map(|band| {
                let hist = &self.hist[band * HIST_BINS..(band + 1) * HIST_BINS];
                let mut above = 0_u64;
                for (index, &count) in hist.iter().enumerate().rev() {
                    above += u64::from(count);
                    if above > allowed_above {
                        return FLOOR_DB + index as f32 * HIST_STEP_DB;
                    }
                }
                FLOOR_DB
            })
            .collect();
        Summary {
            sample_rate: self.sample_rate,
            bin_hz: self.bin_hz(),
            mean_db,
            band_hz: self.bin_hz() * BAND_BINS as f32,
            hold_db,
            active_seconds: (self.active_frames as f64 * self.hop as f64
                / f64::from(self.sample_rate.max(1))) as f32,
        }
    }
}

/// Power to dB, clamped at [`FLOOR_DB`].
pub(crate) fn to_db(power: f32) -> f32 {
    if power <= 1.0e-20 {
        FLOOR_DB
    } else {
        (10.0 * power.log10()).max(FLOOR_DB)
    }
}

impl Summary {
    /// The mean spectrum reduced to `points` dB values spanning 0 Hz to
    /// Nyquist, averaged in the power domain, for the UI's spectrum view.
    pub fn reduced(&self, points: usize) -> Vec<f32> {
        let bins = self.mean_db.len();
        if bins == 0 || points == 0 {
            return Vec::new();
        }
        (0..points)
            .map(|p| {
                let lo = p * bins / points;
                let hi = ((p + 1) * bins / points).max(lo + 1).min(bins);
                let power: f32 = self.mean_db[lo..hi]
                    .iter()
                    .map(|&db| 10.0_f32.powf(db / 10.0))
                    .sum::<f32>()
                    / (hi - lo) as f32;
                to_db(power)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_length_keeps_bins_near_five_hertz() {
        assert_eq!(fft_len(44_100), 8192);
        assert_eq!(fft_len(48_000), 8192);
        assert_eq!(fft_len(96_000), 16_384);
        assert_eq!(fft_len(192_000), 32_768);
        assert_eq!(fft_len(22_050), 4096);
    }

    #[test]
    fn full_scale_sine_reads_zero_db() {
        let rate = 44_100;
        let freq = 1000.0_f64;
        // Generated in f64: f32 phase error at large arguments would itself
        // raise the floor.
        let samples: Vec<f32> = (0..rate)
            .map(|i| (std::f64::consts::TAU * freq * f64::from(i) / f64::from(rate)).sin() as f32)
            .collect();
        let mut welch = Welch::new(rate);
        welch.push(&samples, &mut |_| {});
        let summary = welch.finish();
        let peak = summary
            .mean_db
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(peak.abs() < 1.0, "peak {peak} dB");
        let far = summary.mean_db[(5000.0 / summary.bin_hz) as usize];
        // The f32 FFT and f32 samples limit the floor, not the window.
        assert!(far < -120.0, "leakage at 5 kHz {far} dB");
    }
}
