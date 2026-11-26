//! Guarded file writes shared by every Overtone crate that touches disk.
//!
//! Every write to a user file, DJ database, or crate file goes through this
//! crate so the safety rules in `AGENTS.md` are enforced in one place:
//! protected roots in development builds, timestamped backups, and atomic
//! replacement. Checking that a DJ app is closed lands with the sync crates
//! that need it.

mod error;
pub mod paths;
mod protect;
mod write;

pub use error::FsError;
pub use protect::{PROTECTED_ROOTS_ENV, ProtectedRoots};
pub use write::{backup_file, edit_via_copy, replace_guarded, write_atomic, write_atomic_guarded};
