//! Rekordbox XML and guarded `master.db` integration.
//!
//! See `engineering/decisions/dj-software-sync.md` for the phased `rbox`
//! integration plan (`rbox_xml` in Phase 3, guarded `rbox-masterdb` in Phase 5).

use thiserror::Error;

/// Supported Rekordbox sync targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RekordboxTarget {
    /// Safe text interchange file (`rekordbox.xml`).
    Xml,
    /// Guarded direct SQLCipher database (`master.db`); requires Rekordbox closed
    /// and an automatic timestamped backup before every write.
    MasterDb,
}

/// Errors produced during Rekordbox export or sync.
#[derive(Debug, Error)]
pub enum RekordboxError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Rekordbox must be closed before writing to master.db")]
    AppRunning,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_xml_and_master_db_targets() {
        assert_ne!(RekordboxTarget::Xml, RekordboxTarget::MasterDb);
    }
}
