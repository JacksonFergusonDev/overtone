//! Audio decoding, spectral quality forensics, visual asset generation, and tag I/O.
//!
//! Module ownership (one feature per module, so parallel work doesn't collide):
//! - [`hash`]: audio-only content hashing for duplicate detection.
//! - [`tags`]: reading tags and stream properties, and safe Comment writes.
//! - [`analysis`]: decoding, spectral quality verdicts, waveform peaks, spectrograms.
//!
//! See `engineering/decisions/audio-quality-analysis.md` and
//! `engineering/decisions/vibe-tags.md`. Results use the shared types in
//! `overtone-model`.

pub mod analysis;
mod error;
pub mod hash;
pub mod tags;

pub use error::AudioError;
pub use overtone_model::{QualityReport, QualityVerdict};
