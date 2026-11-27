//! Rekordbox XML export and My Tag sync.
//!
//! Overtone writes a `rekordbox.xml` that Rekordbox imports through
//! Preferences > Advanced > Database > rekordbox xml. The writer and reader
//! are hand-written on `quick-xml` rather than `rbox`, see "Rekordbox XML:
//! written with quick-xml, not rbox" in
//! `engineering/decisions/dj-software-sync.md`.
//!
//! [`master_db`] writes vibes as My Tags into a copy of `master.db`; the
//! guarded swap of the live file lives in `overtone-core`.

use thiserror::Error;

pub mod master_db;
mod model;
mod running;
mod uri;
mod xml_read;
mod xml_write;

pub use model::{PlaylistNode, RekordboxXml, XmlTrack, date_from_unix_secs, kind_for};
pub use running::{
    RunningState, ensure_closed, is_rekordbox_agent_executable, is_rekordbox_executable,
    rekordbox_agent_running, rekordbox_running, running_state,
};
pub use uri::{location_from_uri, location_uri};
pub use xml_read::parse_xml;
pub use xml_write::to_xml;

/// Name shown in the `PRODUCT` element.
pub const PRODUCT_NAME: &str = "Overtone";

/// Errors produced during Rekordbox export and My Tag sync.
#[derive(Debug, Error)]
pub enum RekordboxError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Fs(#[from] overtone_fs::FsError),
    #[error("couldn't read Rekordbox XML: {0}")]
    Xml(String),
    #[error("Rekordbox must be closed before writing to master.db")]
    AppRunning,
    #[error(
        "rekordboxAgent is running; quit it from its menu bar icon (or Activity Monitor) before \
         writing to master.db"
    )]
    AgentRunning,
    #[error("Rekordbox database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("this doesn't look like a Rekordbox master.db: {0}")]
    NotMasterDb(String),
    #[error(
        "master.db has {0} My Tag categories named Overtone; delete or rename all but one in \
         Rekordbox first"
    )]
    AmbiguousCategory(usize),
    #[error("can't sync My Tags: {0}")]
    Unsupported(String),
    #[error("master.db failed verification after the edit, so it was not replaced: {0}")]
    VerifyFailed(String),
    #[error("{0} changed while Overtone was working, so nothing was written; try again")]
    Stale(std::path::PathBuf),
}
