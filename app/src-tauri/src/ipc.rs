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

/// Everything the inspector shows about one track beyond its row: each
/// field's value and source, pending lookup suggestions, loudness, the
/// integrity check, and a conversion waiting for relocation.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct TrackDetails {
    pub track_id: overtone_core::model::TrackId,
    pub fields: Vec<overtone_core::model::FieldValue>,
    pub suggestions: Vec<overtone_core::model::MetadataSuggestion>,
    pub loudness: Option<overtone_core::model::LoudnessReport>,
    pub integrity: Option<overtone_core::model::IntegrityCheck>,
    pub conversion: Option<overtone_core::model::Conversion>,
    /// Your or accepted values the file doesn't hold yet.
    pub unwritten: Vec<overtone_core::model::MetadataField>,
    /// Whether Overtone wrote field values into this file and kept the
    /// originals.
    pub restorable: bool,
}

/// Which lookup credentials are set. The secrets themselves never go back
/// to the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LookupSettings {
    pub acoustid_key_set: bool,
    pub discogs_token_set: bool,
    pub contact: Option<String>,
    /// Providers a lookup would use now.
    pub providers: Vec<overtone_core::lookup::Provider>,
}
