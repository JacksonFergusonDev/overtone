//! Rekordbox XML export.
//!
//! Overtone writes a `rekordbox.xml` that Rekordbox imports through
//! Preferences > Advanced > Database > rekordbox xml. It never touches
//! `master.db`. The writer and reader are hand-written on `quick-xml`
//! rather than `rbox`, see "Rekordbox XML: written with quick-xml, not rbox"
//! in `engineering/decisions/dj-software-sync.md`.

use thiserror::Error;

mod model;
mod running;
mod uri;
mod xml_read;
mod xml_write;

pub use model::{PlaylistNode, RekordboxXml, XmlTrack, date_from_unix_secs, kind_for};
pub use running::{is_rekordbox_executable, rekordbox_running};
pub use uri::{location_from_uri, location_uri};
pub use xml_read::parse_xml;
pub use xml_write::to_xml;

/// Name shown in the `PRODUCT` element.
pub const PRODUCT_NAME: &str = "Overtone";

/// Errors produced during Rekordbox export.
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
}
