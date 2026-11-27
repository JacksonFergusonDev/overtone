//! From spectral summaries to a quality verdict.
//!
//! The guiding rule is "protect authentic files first": a verdict only asks
//! the DJ to listen and look, but a false Suspect on a favourite track
//! erodes trust in every other flag. So the detector looks for the specific
//! fingerprint of an encoder low-pass, not for "not much treble":
//!
//! 1. **A wall.** Within about 1.25 kHz the smoothed mean spectrum falls by
//!    at least [`WALL_MIN_DROP_DB`]. Lossy encoders cut with a transition
//!    a few hundred hertz wide; natural roll-off (tape, vinyl, a warm
//!    master, a low-pass in the mix) falls a few dB per 500 Hz.
//! 2. **Nothing comes back.** Above the wall, the mean stays flat (no band
//!    rises more than [`RETURN_MAX_DB`] over the floor) and the 99th
//!    percentile peak-hold stays at least [`HOLD_GAP_MIN_DB`] below the
//!    program just under the wall. Sparse genuine treble (hats every few
//!    bars) fails this test even when the mean looks like a wall.
//! 3. **Steepness.** The local slope across the cutoff, in dB per octave
//!    over a 500 Hz span, separates a hard encoder wall
//!    ([`HARD_STEEPNESS_DB_PER_OCT`]) from a soft shelf.
//! 4. **Context.** The same wall means different things in different
//!    containers: a 16 kHz wall is normal for a 128 kbps MP3, suspicious in
//!    a FLAC or in a "320 kbps" MP3, and a wall at 22 kHz in a 96 kHz file
//!    means the hi-res container holds CD-rate audio.
//!
//! Without a wall, the verdict rests on how far content reaches. When it
//! fades out gradually or there is too little treble to see a wall at all,
//! the answer is [`QualityVerdict::Inconclusive`], never Suspect.
//!
//! All thresholds are in this file. See
//! `engineering/decisions/audio-quality-analysis.md` for the calibration
//! measurements behind them.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "frequency-to-index conversions on bounded, non-negative values"
)]

use overtone_model::{AudioFormat, QualityVerdict};

use super::spectrum::{FLOOR_DB, Summary};

// --- Thresholds -------------------------------------------------------------
//
// | Constant                    | Value      | Meaning                                                     |
// |-----------------------------|------------|-------------------------------------------------------------|
// | GRID_HZ                     | 50 Hz      | Resolution of the smoothed spectrum                         |
// | SMOOTH_HZ                   | 150 Hz     | Median filter width (removes tones, keeps a step sharp)     |
// | MIN_ACTIVE_SECONDS          | 2 s        | Less audible audio than this is Inconclusive                |
// | SCAN_MIN_HZ                 | 11 kHz     | Lowest wall considered (bass-heavy material is protected)   |
// | WALL_MIN_DROP_DB            | 20 dB      | Fall between 250-1000 Hz below and above a wall             |
// | FLAT_FLOOR_MIN_DROP_DB      | 12 dB      | ...or this onto a floor flat to within 1 dB over 2 kHz+     |
// | RETURN_MAX_DB               | 10 dB      | Max rise of the mean above the floor past a wall            |
// | HOLD_GAP_MIN_DB             | 20 dB      | Peak-hold gap between program under the wall and above it   |
// | HARD_STEEPNESS_DB_PER_OCT   | 250 dB/oct | Local slope of an encoder wall                              |
// | VERY_STEEP_DB_PER_OCT       | 600 dB/oct | Slope needed to call a 17.2-19.6 kHz wall Suspect           |
// | SOFT_STEEPNESS_DB_PER_OCT   | 80 dB/oct  | Below this a drop is a roll-off, not a wall                 |
// | FLOOR_MAX_SPREAD_DB         | 8 dB       | Std dev of the floor 0.5-1.5 kHz above a hard wall          |
// | SPORADIC_CONTENT_MARGIN_HZ  | 1500 Hz    | Content edge this far above a wall: band isn't empty        |
// | SPORADIC_OVER_FLOOR_DB      | 12 dB      | ...and its peak-hold must clear the floor's mean by this    |
// | MIN_PROGRAM_DB              | -75 dB     | Quieter program can't support a judgement without a wall    |
// | CONTENT_MIN_DB              | -115 dB    | Peak-hold level that counts as content (16-bit floor -131)  |
// | SUSPECT_MIN_HZ..SUSPECT_MAX | 15-17.2 k  | Hard wall in a lossless file: typical 128 kbps origin       |
// | SUSPECT_STEEP_MAX_HZ        | 19.6 kHz   | Upper end of the very-steep Suspect band (160-256 kbps)     |
// | PASS_MIN_HZ                 | 20.8 kHz   | Content or wall at or above this is a full-band CD master   |
// | HIRES_MIN_RATE_HZ           | 88.2 kHz   | Sample rates judged as hi-res                               |
// | HIRES_CONTENT_MIN_HZ        | 24.5 kHz   | Hi-res must carry content above this                        |
// | LOSSY_SUSPECT_MARGIN_HZ     | 1500 Hz    | Wall this far under the bitrate's expected cutoff: upscaled |
// | LOSSY_WARNING_MARGIN_HZ     | 700 Hz     | Wall this far under it: Warning                             |

/// Resolution of the smoothed spectrum the detector works on.
pub const GRID_HZ: f32 = 50.0;
/// Width of the median filter that builds the smoothed spectrum. A median
/// removes isolated tones (a 15.7 kHz CRT whine on a vinyl rip) that a mean
/// would smear, and keeps a step sharp.
pub const SMOOTH_HZ: f32 = 150.0;
/// Tracks with less audible audio than this are Inconclusive.
pub const MIN_ACTIVE_SECONDS: f32 = 2.0;
/// Walls below this frequency are not considered. Bass-heavy material
/// (dub, ambient, older masters) often carries nothing up here at all.
pub const SCAN_MIN_HZ: f32 = 11_000.0;
/// Minimum fall between the mean levels 250-1000 Hz below and above a wall.
pub const WALL_MIN_DROP_DB: f32 = 20.0;
/// A step onto a flat floor needs only this much drop (and a peak-hold gap
/// 2 dB smaller), when the floor spans [`FLAT_FLOOR_MIN_SPAN_HZ`] with a
/// standard deviation under [`FLAT_FLOOR_MAX_SPREAD_DB`].
pub const FLAT_FLOOR_MIN_DROP_DB: f32 = 12.0;
pub const FLAT_FLOOR_MAX_SPREAD_DB: f32 = 1.0;
pub const FLAT_FLOOR_MIN_SPAN_HZ: f32 = 2000.0;
/// How far the smoothed mean may rise above the floor anywhere past a wall.
pub const RETURN_MAX_DB: f32 = 10.0;
/// Minimum gap between the peak-hold level just under a wall and the
/// loudest peak-hold band above it.
pub const HOLD_GAP_MIN_DB: f32 = 20.0;
/// Local slope across the cutoff that marks a hard encoder wall.
pub const HARD_STEEPNESS_DB_PER_OCT: f32 = 250.0;
/// Slope needed to call a wall between 17.2 and 19.6 kHz Suspect rather
/// than Warning.
pub const VERY_STEEP_DB_PER_OCT: f32 = 600.0;
/// Below this slope a drop is a roll-off, not a wall.
pub const SOFT_STEEPNESS_DB_PER_OCT: f32 = 80.0;
/// Largest standard deviation of a flat floor above a hard wall.
pub const FLOOR_MAX_SPREAD_DB: f32 = 8.0;
/// Peak-hold level that counts as content. 16-bit dither reads about
/// -131 dB per bin, so this leaves margin above any noise floor.
pub const CONTENT_MIN_DB: f32 = -115.0;
/// Hard walls from here to [`SUSPECT_MAX_HZ`] in a lossless container are
/// Suspect (typical of 128 kbps MP3, whose sfb21 cut sits near 16 kHz).
pub const SUSPECT_MIN_HZ: f32 = 15_000.0;
pub const SUSPECT_MAX_HZ: f32 = 17_200.0;
/// Very steep walls up to here are Suspect; less steep ones are Warning.
pub const SUSPECT_STEEP_MAX_HZ: f32 = 19_600.0;
/// Content (or an anti-alias wall) at or above this is a full-band master.
pub const PASS_MIN_HZ: f32 = 20_800.0;
/// Sample rates at or above this are judged as hi-res.
pub const HIRES_MIN_RATE_HZ: u32 = 88_200;
/// A hi-res file must carry content above this to be more than CD audio.
pub const HIRES_CONTENT_MIN_HZ: f32 = 24_500.0;
/// A lossy file whose wall sits this far under its bitrate's expected
/// cutoff is Suspect of being re-encoded from a lower bitrate.
pub const LOSSY_SUSPECT_MARGIN_HZ: f32 = 1500.0;
pub const LOSSY_WARNING_MARGIN_HZ: f32 = 700.0;

/// Without a wall, a judgement from how far content reaches needs the
/// program (1-8 kHz) at least this loud; near-silent interludes are
/// Inconclusive.
pub const MIN_PROGRAM_DB: f32 = -75.0;

/// Content edge this far above a wall means the band above isn't empty.
pub const SPORADIC_CONTENT_MARGIN_HZ: f32 = 1500.0;

/// How far the peak-hold must rise over the floor above a wall to count as
/// real content rather than a stationary noise or dither floor.
pub const SPORADIC_OVER_FLOOR_DB: f32 = 12.0;

/// Kept clear of Nyquist, where resampler roll-off and FFT edge effects live.
const TOP_GUARD_HZ: f32 = 150.0;

/// The lowest cutoff a lossy encoder normally leaves at a bitrate, from
/// LAME 3.100's lowpass table and common AAC encoders, less tolerance for
/// other encoders. `None` below 112 kbps, where cutoffs vary too widely.
pub fn expected_lossy_cutoff_hz(format: AudioFormat, bitrate_kbps: u32) -> Option<f32> {
    let mp3 = match bitrate_kbps {
        288.. => 19_500.0,
        224..=287 => 18_800.0,
        176..=223 => 17_800.0,
        144..=175 => 16_800.0,
        112..=143 => 15_500.0,
        _ => return None,
    };
    // AAC encoders low-pass a little lower than LAME at the same bitrate.
    Some(if format == AudioFormat::Aac {
        mp3 - 500.0
    } else {
        mp3
    })
}

/// How a detected wall looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallKind {
    /// Steep with a flat floor: the shape of an encoder low-pass.
    Hard,
    /// A clear drop, but a shelf rather than a cliff (AAC-style, or an EQ).
    Soft,
}

/// A detected high-frequency wall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wall {
    pub kind: WallKind,
    pub cutoff_hz: f32,
    /// Mean level 250-1000 Hz below the wall minus the level above it.
    pub drop_db: f32,
    /// Local fall across the cutoff over a 500 Hz span, as a positive number.
    pub steepness_db_per_oct: f32,
    /// Standard deviation of the floor above the wall.
    pub floor_spread_db: f32,
    /// Peak-hold gap between program under the wall and anything above it.
    pub hold_gap_db: f32,
    /// Set when some passages still carry content well above the wall
    /// (more than [`SPORADIC_CONTENT_MARGIN_HZ`]), up to this frequency.
    /// A lower-bitrate source can't produce that, so the wall is weaker
    /// evidence: often an encoder dropping its top band in busy passages.
    pub sporadic_content_to_hz: Option<f32>,
}

/// Measurements behind a verdict, kept for calibration and the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectralFeatures {
    /// Median smoothed level in 1-8 kHz, in dB.
    pub reference_db: f32,
    /// Median smoothed level in 11-15 kHz, in dB.
    pub treble_db: f32,
    pub wall: Option<Wall>,
    /// Highest frequency whose peak-hold level reaches [`CONTENT_MIN_DB`].
    pub content_edge_hz: Option<f32>,
    pub active_seconds: f32,
    /// True average bitrate from the compressed packet sizes.
    pub bitrate_kbps: Option<u32>,
}

/// What the container declares, for context.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context {
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub bitrate_kbps: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct Judgement {
    pub verdict: QualityVerdict,
    pub confidence: Option<f32>,
    pub cutoff_hz: Option<f32>,
    pub reason: String,
    pub features: SpectralFeatures,
}

/// The smoothed spectrum and peak-hold on a common [`GRID_HZ`] grid.
struct Grid {
    smooth: Vec<f32>,
    hold: Vec<f32>,
    top_hz: f32,
}

impl Grid {
    fn new(summary: &Summary) -> Self {
        let bins = summary.mean_db.len();
        let nyquist = summary.sample_rate as f32 / 2.0;
        let points = (nyquist / GRID_HZ) as usize + 1;
        let half = SMOOTH_HZ / 2.0;
        let mut window = Vec::new();
        let smooth = (0..points)
            .map(|g| {
                let f = g as f32 * GRID_HZ;
                let lo = (((f - half) / summary.bin_hz).round().max(0.0) as usize).min(bins - 1);
                let hi = (((f + half) / summary.bin_hz).round() as usize).min(bins - 1);
                window.clear();
                window.extend_from_slice(&summary.mean_db[lo..=hi]);
                median(&mut window)
            })
            .collect();
        let hold = (0..points)
            .map(|g| {
                let band = (g as f32 * GRID_HZ / summary.band_hz) as usize;
                summary.hold_db[band.min(summary.hold_db.len() - 1)]
            })
            .collect();
        Self {
            smooth,
            hold,
            top_hz: nyquist - TOP_GUARD_HZ,
        }
    }

    fn range(&self, lo_hz: f32, hi_hz: f32) -> std::ops::RangeInclusive<usize> {
        let lo = (lo_hz.max(0.0) / GRID_HZ).ceil() as usize;
        let hi = ((hi_hz.min(self.top_hz) / GRID_HZ).floor() as usize).min(self.smooth.len() - 1);
        lo..=hi
    }

    fn mean(&self, lo_hz: f32, hi_hz: f32) -> Option<f32> {
        let values = self.smooth.get(self.range(lo_hz, hi_hz))?;
        (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
    }

    fn max(values: Option<&[f32]>) -> Option<f32> {
        values
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().copied().fold(f32::NEG_INFINITY, f32::max))
    }

    fn median_of(values: Option<&[f32]>) -> Option<f32> {
        let values = values.filter(|v| !v.is_empty())?;
        Some(median(&mut values.to_vec()))
    }

    fn at(&self, hz: f32) -> f32 {
        let index = ((hz / GRID_HZ).round().max(0.0) as usize).min(self.smooth.len() - 1);
        self.smooth[index]
    }

    /// Scans for the lowest wall at or above `scan_lo_hz` that passes the
    /// "nothing comes back" tests.
    fn find_wall(&self, scan_lo_hz: f32) -> Option<Wall> {
        let last = self.top_hz - 550.0;
        let mut g = (scan_lo_hz / GRID_HZ).ceil() as usize;
        while (g as f32) * GRID_HZ <= last {
            let drop = self.drop_at(g as f32 * GRID_HZ);
            if drop.is_none_or(|d| d < FLAT_FLOOR_MIN_DROP_DB) {
                g += 1;
                continue;
            }
            // Walk the cluster of candidates and take its strongest point.
            let mut best = (g, drop.unwrap_or(0.0));
            let mut next = g + 1;
            while (next as f32) * GRID_HZ <= last {
                match self.drop_at(next as f32 * GRID_HZ) {
                    Some(d) if d >= FLAT_FLOOR_MIN_DROP_DB => {
                        if d > best.1 {
                            best = (next, d);
                        }
                        next += 1;
                    }
                    _ => break,
                }
            }
            if let Some(wall) = self.measure_wall(best.0 as f32 * GRID_HZ) {
                return Some(wall);
            }
            g = next;
        }
        None
    }

    fn drop_at(&self, f: f32) -> Option<f32> {
        let below = self.mean(f - 1000.0, f - 250.0)?;
        let above = self.mean(f + 250.0, (f + 1000.0).min(self.top_hz))?;
        Some(below - above)
    }

    fn measure_wall(&self, peak_hz: f32) -> Option<Wall> {
        let below = self.mean(peak_hz - 1000.0, peak_hz - 250.0)?;
        let above = self.mean(peak_hz + 250.0, (peak_hz + 1000.0).min(self.top_hz))?;
        let drop = below - above;
        // A perfectly flat floor all the way to Nyquist is white noise, not
        // music: clipping splatter from decoding a loud MP3 to 16-bit, or
        // dither. A smaller step onto such a floor still counts as a wall.
        let wide_floor = self.smooth.get(self.range(peak_hz + 500.0, self.top_hz))?;
        let flat_floor = wide_floor.len() as f32 * GRID_HZ >= FLAT_FLOOR_MIN_SPAN_HZ
            && std_dev(wide_floor) <= FLAT_FLOOR_MAX_SPREAD_DB;
        if drop < WALL_MIN_DROP_DB && !flat_floor {
            return None;
        }

        // Nothing comes back in the mean (past the transition band, so a
        // LAME-style two-step edge, a 16 kHz step and then a sparse sfb21
        // shelf to 17 kHz, still counts as one wall)...
        let rise = Self::max(self.smooth.get(self.range(peak_hz + 500.0, self.top_hz)))? - above;
        if rise > RETURN_MAX_DB {
            return None;
        }
        // ...or in the peak-hold.
        let hold_below =
            Self::median_of(self.hold.get(self.range(peak_hz - 1500.0, peak_hz - 250.0)))?;
        let hold_above =
            Self::max(self.hold.get(self.range(peak_hz + 500.0, self.top_hz))).unwrap_or(FLOOR_DB);
        let hold_gap = hold_below - hold_above;
        let min_gap = if flat_floor {
            FLAT_FLOOR_MIN_DROP_DB - 2.0
        } else {
            HOLD_GAP_MIN_DB
        };
        if hold_gap < min_gap {
            return None;
        }

        // Place the cutoff where the curve crosses halfway down the step.
        let mid = f32::midpoint(below, above);
        let mut cutoff = peak_hz;
        if self.at(cutoff) >= mid {
            while cutoff + GRID_HZ <= peak_hz + 1000.0 && self.at(cutoff + GRID_HZ) >= mid {
                cutoff += GRID_HZ;
            }
        } else {
            while cutoff - GRID_HZ >= peak_hz - 1000.0 && self.at(cutoff) < mid {
                cutoff -= GRID_HZ;
            }
        }

        let upper = self.mean(cutoff - 350.0, cutoff - 150.0)?;
        let lower = self.mean(cutoff + 150.0, cutoff + 350.0)?;
        let octaves = ((cutoff + 250.0) / (cutoff - 250.0)).log2();
        let steepness = (upper - lower) / octaves;

        let floor = self
            .smooth
            .get(self.range(cutoff + 500.0, cutoff + 1500.0))?;
        let floor_spread = std_dev(floor);

        let kind = if steepness >= HARD_STEEPNESS_DB_PER_OCT && floor_spread <= FLOOR_MAX_SPREAD_DB
        {
            WallKind::Hard
        } else if steepness >= SOFT_STEEPNESS_DB_PER_OCT {
            WallKind::Soft
        } else {
            return None;
        };
        Some(Wall {
            kind,
            cutoff_hz: cutoff,
            drop_db: drop,
            steepness_db_per_oct: steepness,
            floor_spread_db: floor_spread,
            hold_gap_db: hold_gap,
            sporadic_content_to_hz: None,
        })
    }

    fn content_edge(&self) -> Option<f32> {
        let top = self.range(0.0, self.top_hz);
        let top_index = *top.end();
        (0..=top_index)
            .rev()
            .find(|&g| self.hold[g] >= CONTENT_MIN_DB)
            .map(|g| g as f32 * GRID_HZ)
    }

    /// Highest frequency above a wall at `cutoff` where the peak-hold rises
    /// clearly over the floor's own level. A stationary floor, such as the
    /// noise-shaped dither a 16-bit conversion adds (often -80 dB per bin
    /// near 20 kHz), has a peak-hold barely above its mean; passages of
    /// real treble stand [`SPORADIC_OVER_FLOOR_DB`] or more above it.
    fn sporadic_edge(&self, cutoff: f32) -> Option<f32> {
        let floor = self.mean(cutoff + 500.0, self.top_hz)?;
        let threshold = CONTENT_MIN_DB.max(floor + SPORADIC_OVER_FLOOR_DB);
        let range = self.range(cutoff + 500.0, self.top_hz);
        range
            .rev()
            .find(|&g| self.hold[g] >= threshold)
            .map(|g| g as f32 * GRID_HZ)
    }

    fn has_content_above(&self, hz: f32) -> bool {
        Self::max(self.hold.get(self.range(hz, self.top_hz))).is_some_and(|v| v >= CONTENT_MIN_DB)
    }
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return FLOOR_DB;
    }
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, f32::total_cmp).1
}

fn std_dev(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let n = values.len() as f32;
    let mean = values.iter().sum::<f32>() / n;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n).sqrt()
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// How strongly a wall looks like an encoder low-pass, from 0 to 1.
fn wall_strength(wall: &Wall) -> f32 {
    let steep = clamp01(
        (wall.steepness_db_per_oct - HARD_STEEPNESS_DB_PER_OCT)
            / (2.0 * VERY_STEEP_DB_PER_OCT - HARD_STEEPNESS_DB_PER_OCT),
    );
    let drop = clamp01((wall.drop_db - WALL_MIN_DROP_DB) / 30.0);
    let gap = clamp01((wall.hold_gap_db - HOLD_GAP_MIN_DB) / 40.0);
    (steep + drop + gap) / 3.0
}

fn khz(hz: f32) -> String {
    format!("{:.1} kHz", hz / 1000.0)
}

fn container_name(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Flac => "FLAC",
        AudioFormat::Wav => "WAV",
        AudioFormat::Aiff => "AIFF",
        AudioFormat::Alac => "ALAC",
        AudioFormat::Mp3 => "MP3",
        AudioFormat::Aac => "AAC",
        AudioFormat::Other => "lossless",
    }
}

/// The lossy source a lossless file with a wall at `cutoff` probably came from.
fn likely_origin(cutoff: f32) -> &'static str {
    match cutoff {
        c if c < SUSPECT_MIN_HZ => "a low-bitrate (96 kbps or less) lossy source",
        c if c < SUSPECT_MAX_HZ => "a 128 kbps MP3 source",
        c if c < 18_300.0 => "a 160 kbps lossy source",
        c if c < 19_200.0 => "a 192 kbps lossy source",
        c if c < 19_900.0 => "a 224 to 256 kbps lossy source",
        _ => "a 256 to 320 kbps lossy source",
    }
}

fn describe_wall(wall: &Wall) -> String {
    let shape = if wall.steepness_db_per_oct >= HARD_STEEPNESS_DB_PER_OCT {
        "Steep cutoff"
    } else {
        "Soft shelf"
    };
    format!(
        "{shape} at {} (\u{2212}{:.0} dB/oct, {:.0} dB drop)",
        khz(wall.cutoff_hz),
        wall.steepness_db_per_oct,
        wall.drop_db
    )
}

pub(crate) fn judge(summary: &Summary, context: Context) -> Judgement {
    let grid = Grid::new(summary);
    let mut features = SpectralFeatures {
        reference_db: Grid::median_of(grid.smooth.get(grid.range(1000.0, 8000.0)))
            .unwrap_or(FLOOR_DB),
        treble_db: Grid::median_of(grid.smooth.get(grid.range(11_000.0, 15_000.0)))
            .unwrap_or(FLOOR_DB),
        wall: grid.find_wall(SCAN_MIN_HZ),
        content_edge_hz: grid.content_edge(),
        active_seconds: summary.active_seconds,
        bitrate_kbps: context.bitrate_kbps,
    };
    if let Some(wall) = features.wall.as_mut() {
        wall.sporadic_content_to_hz = grid
            .sporadic_edge(wall.cutoff_hz)
            .filter(|&edge| edge > wall.cutoff_hz + SPORADIC_CONTENT_MARGIN_HZ);
    }
    let outcome = if features.active_seconds < MIN_ACTIVE_SECONDS {
        Outcome::inconclusive(format!(
            "Only {:.1} s of audible audio, too little to judge",
            features.active_seconds
        ))
    } else if matches!(context.format, AudioFormat::Mp3 | AudioFormat::Aac) {
        judge_lossy(&features, context)
    } else if context.sample_rate >= HIRES_MIN_RATE_HZ {
        judge_hires(&grid, &features, context)
    } else {
        judge_lossless(&features, context)
    };
    Judgement {
        verdict: outcome.verdict,
        confidence: outcome.confidence,
        cutoff_hz: outcome
            .cutoff_hz
            .or(features.wall.map(|w| w.cutoff_hz))
            .or(features.content_edge_hz),
        reason: outcome.reason,
        features,
    }
}

struct Outcome {
    verdict: QualityVerdict,
    confidence: Option<f32>,
    cutoff_hz: Option<f32>,
    reason: String,
}

impl Outcome {
    fn new(verdict: QualityVerdict, confidence: f32, reason: String) -> Self {
        Self {
            verdict,
            confidence: Some(clamp01(confidence)),
            cutoff_hz: None,
            reason,
        }
    }

    fn inconclusive(reason: String) -> Self {
        Self {
            verdict: QualityVerdict::Inconclusive,
            confidence: None,
            cutoff_hz: None,
            reason,
        }
    }
}

fn full_band_pass(edge: f32, container: &str) -> Outcome {
    Outcome::new(
        QualityVerdict::Pass,
        0.6 + 0.35 * clamp01((edge - PASS_MIN_HZ) / 1000.0),
        format!(
            "Content reaches {} with no encoder cutoff: consistent with a genuine {container}",
            khz(edge)
        ),
    )
}

fn no_wall_inconclusive(features: &SpectralFeatures) -> Outcome {
    if features.reference_db < MIN_PROGRAM_DB {
        return Outcome::inconclusive(format!(
            "The music is too quiet ({:.0} dB program level) to judge without an encoder-style \
             cutoff",
            features.reference_db
        ));
    }
    match features.content_edge_hz {
        Some(edge) if edge >= SCAN_MIN_HZ => Outcome::inconclusive(format!(
            "High frequencies fade out gradually by {} with no encoder-style cutoff: \
             typical of a warm master, vinyl rip, or low-passed mix",
            khz(edge)
        )),
        _ => Outcome::inconclusive(
            "Too little high-frequency energy to judge (bass-heavy or quiet material)".into(),
        ),
    }
}

fn judge_lossless(features: &SpectralFeatures, context: Context) -> Outcome {
    let container = container_name(context.format);
    if let Some(wall) = features.wall.filter(|w| w.cutoff_hz < PASS_MIN_HZ) {
        let strength = wall_strength(&wall);
        let c = wall.cutoff_hz;
        let described = describe_wall(&wall);
        let origin = likely_origin(c);
        let core_band = (SUSPECT_MIN_HZ..SUSPECT_MAX_HZ).contains(&c);
        if let Some(reach) = wall.sporadic_content_to_hz
            && !(wall.kind == WallKind::Hard && core_band)
        {
            return Outcome::new(
                QualityVerdict::Warning,
                0.35 + 0.25 * strength,
                format!(
                    "{described} inside a {container} container, though some passages reach {}: \
                     possibly a high-bitrate lossy source that drops its top band in busy \
                     passages, or the master itself",
                    khz(reach)
                ),
            );
        }
        // A hard wall near 16 kHz in a lossless file stays Suspect even with
        // sporadic energy above it: decoding a loud MP3 to 16-bit clips, and
        // the clipping splatters across the empty band. Confidence drops.
        let (penalty, note) = match wall.sporadic_content_to_hz {
            Some(reach) => (
                0.15,
                format!(
                    " (some passages reach {}, possibly clipping from the decode)",
                    khz(reach)
                ),
            ),
            None => (0.0, String::new()),
        };
        return match wall.kind {
            WallKind::Hard if core_band => Outcome::new(
                QualityVerdict::Suspect,
                0.6 + 0.35 * strength - penalty,
                format!("{described} inside a {container} container: typical of {origin}{note}"),
            ),
            WallKind::Hard
                if (SUSPECT_MAX_HZ..SUSPECT_STEEP_MAX_HZ).contains(&c)
                    && wall.steepness_db_per_oct >= VERY_STEEP_DB_PER_OCT
                    && wall.drop_db >= 30.0
                    && wall.hold_gap_db >= 30.0 =>
            {
                Outcome::new(
                    QualityVerdict::Suspect,
                    0.55 + 0.35 * strength,
                    format!("{described} inside a {container} container: typical of {origin}"),
                )
            }
            WallKind::Hard if c < SUSPECT_MIN_HZ => Outcome::new(
                QualityVerdict::Warning,
                0.4 + 0.3 * strength,
                format!(
                    "{described} inside a {container} container: {origin}, or a deliberately \
                     filtered master"
                ),
            ),
            WallKind::Hard if c >= SUSPECT_STEEP_MAX_HZ => Outcome::new(
                QualityVerdict::Warning,
                0.35 + 0.25 * strength,
                format!(
                    "{described} inside a {container} container: typical of {origin}, though some \
                     genuine masters roll off here too"
                ),
            ),
            WallKind::Hard => Outcome::new(
                QualityVerdict::Warning,
                0.45 + 0.3 * strength,
                format!("{described} inside a {container} container: possibly {origin}"),
            ),
            WallKind::Soft => Outcome::new(
                QualityVerdict::Warning,
                0.3 + 0.2 * strength,
                format!(
                    "{described} inside a {container} container: an AAC-style lossy source or a \
                     steep master EQ"
                ),
            ),
        };
    }
    let reach = features
        .content_edge_hz
        .into_iter()
        .chain(features.wall.map(|w| w.cutoff_hz))
        .fold(0.0_f32, f32::max);
    if reach >= PASS_MIN_HZ {
        return full_band_pass(reach, container);
    }
    no_wall_inconclusive(features)
}

fn judge_hires(grid: &Grid, features: &SpectralFeatures, context: Context) -> Outcome {
    let container = format!(
        "{:.1} kHz {}",
        context.sample_rate as f32 / 1000.0,
        container_name(context.format)
    );
    if let Some(wall) = features.wall.filter(|w| {
        w.kind == WallKind::Hard
            && w.cutoff_hz <= HIRES_CONTENT_MIN_HZ
            && w.sporadic_content_to_hz.is_none()
    }) {
        let mut reason = format!(
            "{} and nothing above it inside a {container}: upsampled from CD-rate audio",
            describe_wall(&wall)
        );
        if wall.cutoff_hz < PASS_MIN_HZ {
            reason.push_str(&format!(
                ", and the cutoff is typical of {}",
                likely_origin(wall.cutoff_hz)
            ));
        }
        return Outcome::new(
            QualityVerdict::Suspect,
            0.65 + 0.3 * wall_strength(&wall),
            reason,
        );
    }
    if grid.has_content_above(HIRES_CONTENT_MIN_HZ) {
        let edge = features.content_edge_hz.unwrap_or(HIRES_CONTENT_MIN_HZ);
        return Outcome::new(
            QualityVerdict::Pass,
            0.7 + 0.25 * clamp01((edge - HIRES_CONTENT_MIN_HZ) / 10_000.0),
            format!(
                "Content reaches {} inside a {container}: genuine hi-res",
                khz(edge)
            ),
        );
    }
    let edge = features.content_edge_hz.unwrap_or(0.0);
    Outcome::new(
        QualityVerdict::Warning,
        0.4,
        format!(
            "Very little content above {} inside a {container}: possibly upsampled, or a \
             recording that rolls off early",
            khz(edge.max(SCAN_MIN_HZ))
        ),
    )
}

fn judge_lossy(features: &SpectralFeatures, context: Context) -> Outcome {
    let codec = container_name(context.format);
    let Some(kbps) = context.bitrate_kbps else {
        return Outcome::inconclusive(format!(
            "{codec} bitrate unknown, so there's nothing to compare the cutoff with"
        ));
    };
    let label = format!("{kbps} kbps {codec}");
    let Some(expected) = expected_lossy_cutoff_hz(context.format, kbps) else {
        let cutoff = features
            .wall
            .map(|w| w.cutoff_hz)
            .or(features.content_edge_hz);
        return Outcome::new(
            QualityVerdict::Pass,
            0.5,
            match cutoff {
                Some(c) => format!("Cutoff at {} is normal for a {label}", khz(c)),
                None => format!("Nothing unusual for a {label}"),
            },
        );
    };
    if let Some(wall) = features.wall {
        let c = wall.cutoff_hz;
        let strength = wall_strength(&wall);
        let described = describe_wall(&wall);
        if let Some(reach) = wall.sporadic_content_to_hz {
            if c < expected - LOSSY_SUSPECT_MARGIN_HZ {
                return Outcome::new(
                    QualityVerdict::Warning,
                    0.3 + 0.2 * strength,
                    format!(
                        "{described} inside a {label}, though some passages reach {}: the \
                         encoder may drop its top band in busy passages, or it was re-encoded \
                         from a lower bitrate",
                        khz(reach)
                    ),
                );
            }
            return Outcome::new(
                QualityVerdict::Pass,
                0.6,
                format!(
                    "Content reaches {} with a band edge at {}: consistent with a {label}",
                    khz(reach),
                    khz(c)
                ),
            );
        }
        if c < expected - LOSSY_SUSPECT_MARGIN_HZ {
            return Outcome::new(
                QualityVerdict::Suspect,
                0.6 + 0.35 * strength,
                format!(
                    "{described} inside a {label}, which normally reaches {}: typical of {} \
                     re-encoded at a higher bitrate",
                    khz(expected),
                    likely_origin(c)
                ),
            );
        }
        if c < expected - LOSSY_WARNING_MARGIN_HZ {
            return Outcome::new(
                QualityVerdict::Warning,
                0.4 + 0.3 * strength,
                format!(
                    "{described} inside a {label}, which normally reaches {}: possibly \
                     re-encoded from a lower bitrate",
                    khz(expected)
                ),
            );
        }
        return Outcome::new(
            QualityVerdict::Pass,
            0.75,
            format!("Cutoff at {} is consistent with a {label}", khz(c)),
        );
    }
    match features.content_edge_hz {
        Some(edge) if edge >= expected - LOSSY_WARNING_MARGIN_HZ => Outcome::new(
            QualityVerdict::Pass,
            0.7,
            format!("Content reaches {}, consistent with a {label}", khz(edge)),
        ),
        Some(edge)
            if edge >= SCAN_MIN_HZ
                && edge < expected - LOSSY_SUSPECT_MARGIN_HZ
                && features.reference_db >= MIN_PROGRAM_DB =>
        {
            Outcome::new(
                QualityVerdict::Warning,
                0.35,
                format!(
                    "Content stops at {} inside a {label}, which normally reaches {}: possibly \
                     re-encoded from a lower bitrate, or a dull master",
                    khz(edge),
                    khz(expected)
                ),
            )
        }
        _ => no_wall_inconclusive(features),
    }
}
