//! Audio decoding, spectral quality forensics, visual asset generation, and tag I/O.
//!
//! See `engineering/decisions/audio-quality-analysis.md` for the planned
//! `symphonia` + `rustfft` + `lofty` implementation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced during audio inspection, analysis, or tag operations.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),
}

/// Result of spectral quality analysis for a single audio track.
///
/// Automated verdicts only flag tracks for human review by ear and spectrogram;
/// Overtone never auto-deletes files based on a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityVerdict {
    Unchecked,
    Pass,
    Warning,
    Suspect,
    VerifiedOk,
}

/// Summary of spectral measurements recorded for a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
    pub verdict: QualityVerdict,
    pub sample_rate_hz: u32,
    pub estimated_cutoff_hz: Option<f32>,
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_verdict_serializes_as_snake_case() {
        let json = serde_json::to_string(&QualityVerdict::VerifiedOk).expect("serialize verdict");
        assert_eq!(json, "\"verified_ok\"");
    }
}
