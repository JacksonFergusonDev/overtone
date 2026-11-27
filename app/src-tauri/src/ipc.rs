//! Types that exist only for the desktop app's IPC. Shared workflow types
//! (tracks, vibes, plans, reports) come from `overtone-core` and its crates.

use overtone_core::model::QualityReport;
use serde::Serialize;
use ts_rs::TS;

/// Waveform peaks for the scrub bar: `[min, max]` per bucket in time order,
/// each in -1.0..=1.0.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct WaveformPeaks {
    pub buckets: Vec<[f32; 2]>,
}

/// A rendered spectrogram PNG in the app cache, served to the webview over
/// the asset protocol.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SpectrogramImage {
    /// Absolute path to the PNG; pass it to `convertFileSrc`.
    pub path: String,
    /// The analysis that produced it.
    pub report: QualityReport,
    /// Whether the frequency axis is logarithmic.
    pub log_frequency: bool,
}

/// Which DJ apps are running now. Sync to an app is refused while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, TS)]
#[ts(export)]
pub struct RunningApps {
    pub serato: bool,
    pub rekordbox: bool,
    pub traktor: bool,
}
