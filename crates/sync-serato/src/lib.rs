//! Serato DJ `.crate` reader, writer, and sync planner.
//!
//! See `engineering/decisions/dj-software-sync.md` for why Overtone uses a
//! custom zero-dependency TLV codec rather than `triseratops` (whose library
//! module is read-only) and avoids writing `database V2`.
//!
//! The pieces, one module each:
//!
//! - [`codec`] parses and serializes `.crate` files losslessly.
//! - [`paths`] maps absolute paths to `_Serato_` folders and `ptrk` strings.
//! - [`names`] builds and parses `A%%B.crate` file names.
//! - [`neworder`] reads and edits `neworder.pref`.
//! - [`plan`] makes a dry-run [`SeratoSyncPlan`] for the crates Overtone owns.
//! - [`apply`] backs up, then writes the plan atomically.
//! - [`running`] detects a running Serato DJ.

use thiserror::Error;

pub mod apply;
pub mod codec;
pub mod names;
pub mod neworder;
pub mod paths;
pub mod plan;
pub mod running;
#[cfg(test)]
mod testutil;

pub use apply::{ApplyReport, RunningCheck, apply};
pub use codec::{
    CRATE_VERSION, Column, Content, CrateFile, DEFAULT_COLUMNS, Record, SortOrder, Tag,
};
pub use names::{
    SUBCRATE_SEPARATOR, crate_filename, crate_name, name_key, parse_crate_filename,
    parse_crate_name, sanitize_segment,
};
pub use neworder::NewOrder;
pub use paths::{SERATO_DIR_NAME, absolute_path_for, path_key, same_path, serato_dir_for};
pub use plan::{
    DEFAULT_ROOT_CRATE, DesiredCrate, FileAction, NewOrderChange, SeratoFilePlan, SeratoSyncPlan,
    SkippedTrack, plan_sync,
};
pub use running::serato_running;

/// Errors produced when encoding, decoding, planning, or applying Serato
/// crate changes.
#[derive(Debug, Error)]
pub enum SeratoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid Serato crate binary data: {0}")]
    InvalidFormat(String),
    #[error("{0}")]
    InvalidName(String),
    #[error(transparent)]
    Fs(#[from] overtone_fs::FsError),
    #[error("Serato DJ is running; close it before syncing so it doesn't overwrite the changes")]
    SeratoRunning,
    #[error("{0}")]
    StalePlan(String),
}
