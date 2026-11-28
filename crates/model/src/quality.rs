use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::UnixMillis;

/// Automated spectral quality verdict for a track.
///
/// Verdicts only flag tracks for human review by ear and spectrogram.
/// Overtone never deletes or moves files based on a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum QualityVerdict {
    /// Not analyzed yet.
    Unchecked,
    /// Spectrum is consistent with the container's claimed quality.
    Pass,
    /// Something looks off, but it is often a legitimate master (vinyl rip, lo-fi, heavy low-pass).
    Warning,
    /// Strong evidence of a lossy transcode or fake hi-res upsample.
    Suspect,
    /// Analysis ran but the audio carries too little high-frequency content
    /// to judge (common for dub, ambient, and older masters). Not flagged.
    Inconclusive,
}

impl QualityVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchecked => "unchecked",
            Self::Pass => "pass",
            Self::Warning => "warning",
            Self::Suspect => "suspect",
            Self::Inconclusive => "inconclusive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unchecked" => Self::Unchecked,
            "pass" => Self::Pass,
            "warning" => Self::Warning,
            "suspect" => Self::Suspect,
            "inconclusive" => Self::Inconclusive,
            _ => return None,
        })
    }

    /// Whether the verdict asks a human to listen and look.
    pub fn needs_review(self) -> bool {
        matches!(self, Self::Warning | Self::Suspect)
    }
}

/// The DJ's own decision about a flagged track, kept separate from the
/// automated verdict so re-running analysis never erases a human judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Unreviewed,
    /// Listened and looked: the flag was a false positive.
    VerifiedOk,
    /// Confirmed bad; the DJ wants to find a better copy. The file is left in place.
    ReplaceWanted,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::VerifiedOk => "verified_ok",
            Self::ReplaceWanted => "replace_wanted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unreviewed" => Self::Unreviewed,
            "verified_ok" => Self::VerifiedOk,
            "replace_wanted" => Self::ReplaceWanted,
            _ => return None,
        })
    }
}

/// Spectral measurements and verdict recorded for one track.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct QualityReport {
    pub verdict: QualityVerdict,
    /// Estimated frequency where the spectrum's energy falls off, in Hz.
    pub cutoff_hz: Option<f32>,
    /// Confidence in the verdict, from 0.0 to 1.0.
    pub confidence: Option<f32>,
    /// Human-readable explanation of the verdict.
    pub reason: Option<String>,
    /// Version of the analyzer that produced this report, so reports from
    /// older heuristics can be re-run.
    pub analyzer_version: u32,
    pub analyzed_at: UnixMillis,
    pub review: ReviewStatus,
    pub reviewed_at: Option<UnixMillis>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_strings_roundtrip() {
        for verdict in [
            QualityVerdict::Unchecked,
            QualityVerdict::Pass,
            QualityVerdict::Warning,
            QualityVerdict::Suspect,
            QualityVerdict::Inconclusive,
        ] {
            assert_eq!(QualityVerdict::parse(verdict.as_str()), Some(verdict));
            let json = serde_json::to_string(&verdict).expect("serialize");
            assert_eq!(json, format!("\"{}\"", verdict.as_str()));
        }
    }

    #[test]
    fn review_strings_roundtrip() {
        for status in [
            ReviewStatus::Unreviewed,
            ReviewStatus::VerifiedOk,
            ReviewStatus::ReplaceWanted,
        ] {
            assert_eq!(ReviewStatus::parse(status.as_str()), Some(status));
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
    }
}

/// Loudness and level measurements from the analysis pass. Reports only:
/// Overtone never changes gain, trims silence, or rewrites audio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct LoudnessReport {
    /// Integrated loudness (EBU R128), in LUFS. `None` for silence.
    pub integrated_lufs: Option<f64>,
    /// Loudness range (EBU Tech 3342), in LU.
    pub loudness_range_lu: Option<f64>,
    /// Highest true peak over all channels (4x oversampled), in dBTP.
    pub true_peak_dbtp: Option<f64>,
    /// Highest sample peak over all channels, in dBFS.
    pub sample_peak_dbfs: Option<f64>,
    /// Runs of at least three consecutive full-scale samples in a channel.
    pub clipped_runs: u64,
    /// Silence before the first sound above -60 dBFS, in milliseconds.
    pub leading_silence_ms: u64,
}
