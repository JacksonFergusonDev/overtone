//! Audio decoding, spectral quality forensics, visual asset generation, and tag I/O.
//!
//! See `engineering/decisions/audio-quality-analysis.md` for the planned
//! `symphonia` + `rustfft` + `lofty` implementation. Results use the shared
//! types in `overtone-model`.

pub use overtone_model::{QualityReport, QualityVerdict};
use thiserror::Error;

/// Errors produced during audio inspection, analysis, or tag operations.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),
}
