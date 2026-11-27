//! Spectrogram image rendering.
//!
//! The image reuses the analysis pass's FFT frames, so it costs no second
//! decode. Each frame's power spectrum is folded into `height` rows
//! (linear 0 Hz to Nyquist like Spek, or logarithmic from 20 Hz), and
//! frames are averaged in the power domain into time slots. When the slot
//! list reaches twice the image width, neighbouring slots merge, so memory
//! is bounded without knowing the track length in advance.
//!
//! Levels map 0 to -120 dBFS onto a 256-entry magma palette (the range Spek
//! users are used to) and the file is an 8-bit indexed PNG: a third the size
//! of RGB and exact, because the palette is the colormap. Rows run from the
//! highest frequency at the top. Rulers and cutoff markers are left to the
//! UI so the PNG stays pure data.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel and bin index conversions on bounded, non-negative values"
)]

use std::path::{Path, PathBuf};

use super::spectrum::to_db;
use crate::AudioError;

/// Top of the colour scale, in dBFS.
pub const SPECTROGRAM_TOP_DB: f32 = 0.0;
/// Bottom of the colour scale, in dBFS.
pub const SPECTROGRAM_BOTTOM_DB: f32 = -120.0;
const LOG_LOW_HZ: f32 = 20.0;

/// Asks [`super::analyze`] to also write a spectrogram PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpectrogramOptions {
    /// Where to write the PNG. Written atomically through `overtone-fs`, so
    /// protected folders are refused.
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Logarithmic frequency axis from 20 Hz instead of linear from 0 Hz.
    pub log_frequency: bool,
}

impl SpectrogramOptions {
    /// A 1600 x 512 linear-frequency image at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            width: 1600,
            height: 512,
            log_frequency: false,
        }
    }
}

pub(crate) struct SpectrogramBuilder {
    options: SpectrogramOptions,
    /// Bin range `[lo, hi)` for each row, top row first.
    rows: Vec<(usize, usize)>,
    slots: Vec<f32>,
    slot_counts: Vec<u32>,
    current: Vec<f32>,
    current_frames: u32,
    frames_per_slot: u32,
}

impl SpectrogramBuilder {
    pub fn new(options: SpectrogramOptions, bins: usize, bin_hz: f32) -> Self {
        let height = options.height.max(1) as usize;
        let nyquist = bin_hz * (bins - 1) as f32;
        let rows = (0..height)
            .map(|r| {
                let j = (height - 1 - r) as f32;
                let (f_lo, f_hi) = if options.log_frequency {
                    let ratio = (nyquist / LOG_LOW_HZ).max(1.0);
                    (
                        LOG_LOW_HZ * ratio.powf(j / height as f32),
                        LOG_LOW_HZ * ratio.powf((j + 1.0) / height as f32),
                    )
                } else {
                    (
                        nyquist * j / height as f32,
                        nyquist * (j + 1.0) / height as f32,
                    )
                };
                let lo = ((f_lo / bin_hz).floor() as usize).min(bins - 1);
                let hi = ((f_hi / bin_hz).ceil() as usize).clamp(lo + 1, bins);
                (lo, hi)
            })
            .collect();
        Self {
            rows,
            slots: Vec::new(),
            slot_counts: Vec::new(),
            current: vec![0.0; height],
            current_frames: 0,
            frames_per_slot: 1,
            options,
        }
    }

    pub fn push(&mut self, power: &[f32]) {
        for (value, &(lo, hi)) in self.current.iter_mut().zip(&self.rows) {
            let band = &power[lo.min(power.len())..hi.min(power.len())];
            if !band.is_empty() {
                *value += band.iter().sum::<f32>() / band.len() as f32;
            }
        }
        self.current_frames += 1;
        if self.current_frames == self.frames_per_slot {
            self.flush_current();
        }
    }

    fn flush_current(&mut self) {
        self.slots.extend_from_slice(&self.current);
        self.slot_counts.push(self.current_frames);
        self.current.iter_mut().for_each(|v| *v = 0.0);
        self.current_frames = 0;
        let width = self.options.width.max(1) as usize;
        if self.slot_counts.len() >= width * 2 {
            let height = self.rows.len();
            let pairs = self.slot_counts.len() / 2;
            let mut slots = Vec::with_capacity(self.slots.len() / 2 + height);
            let mut counts = Vec::with_capacity(pairs + 1);
            for pair in 0..pairs {
                let a = &self.slots[2 * pair * height..(2 * pair + 1) * height];
                let b = &self.slots[(2 * pair + 1) * height..(2 * pair + 2) * height];
                slots.extend(a.iter().zip(b).map(|(x, y)| x + y));
                counts.push(self.slot_counts[2 * pair] + self.slot_counts[2 * pair + 1]);
            }
            if self.slot_counts.len() % 2 == 1 {
                slots.extend_from_slice(&self.slots[2 * pairs * height..]);
                counts.push(self.slot_counts[2 * pairs]);
            }
            self.slots = slots;
            self.slot_counts = counts;
            self.frames_per_slot *= 2;
        }
    }

    /// Renders and writes the PNG.
    pub fn finish(mut self) -> Result<(), AudioError> {
        if self.current_frames > 0 {
            self.flush_current();
        }
        let width = self.options.width.max(1) as usize;
        let height = self.rows.len();
        let slots = self.slot_counts.len();
        let mut pixels = vec![0_u8; width * height];
        if slots > 0 {
            for x in 0..width {
                let (lo, hi) = if slots >= width {
                    let lo = x * slots / width;
                    (lo, ((x + 1) * slots / width).max(lo + 1))
                } else {
                    let s = x * slots / width;
                    (s, s + 1)
                };
                let frames: u32 = self.slot_counts[lo..hi].iter().sum();
                for row in 0..height {
                    let power: f32 = (lo..hi).map(|s| self.slots[s * height + row]).sum();
                    pixels[row * width + x] = level_index(to_db(power / frames.max(1) as f32));
                }
            }
        }
        let bytes = encode_png(&self.options.path, &pixels, width, height)?;
        overtone_fs::write_atomic(&self.options.path, &bytes)?;
        Ok(())
    }
}

fn level_index(db: f32) -> u8 {
    let t = (db - SPECTROGRAM_BOTTOM_DB) / (SPECTROGRAM_TOP_DB - SPECTROGRAM_BOTTOM_DB);
    (t.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn palette() -> Vec<u8> {
    (0..256)
        .flat_map(|i| {
            let colour = colorous::MAGMA.eval_rational(i, 256);
            [colour.r, colour.g, colour.b]
        })
        .collect()
}

fn encode_png(
    path: &Path,
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, AudioError> {
    let image_error = |err: png::EncodingError| AudioError::Image {
        path: path.to_path_buf(),
        message: err.to_string(),
    };
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(
        &mut bytes,
        u32::try_from(width).unwrap_or(u32::MAX),
        u32::try_from(height).unwrap_or(u32::MAX),
    );
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette());
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(image_error)?;
    writer.write_image_data(pixels).map_err(image_error)?;
    writer.finish().map_err(image_error)?;
    Ok(bytes)
}
