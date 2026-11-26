//! Serato DJ `.crate` and subcrate binary TLV reader/writer.
//!
//! See `engineering/decisions/dj-software-sync.md` for why Overtone uses a
//! custom zero-dependency TLV writer rather than `triseratops` (whose library
//! module is read-only) and avoids direct `database V2` writes.

use thiserror::Error;

/// Standard `vrsn` payload header written at the start of Serato `.crate` files.
pub const CRATE_VERSION: &str = "1.0/Serato ScratchLive Crate";

/// Subcrate nesting delimiter used in `_Serato_/Subcrates/` filenames.
pub const SUBCRATE_SEPARATOR: &str = "%%";

/// Errors produced when encoding or decoding Serato `.crate` files.
#[derive(Debug, Error)]
pub enum SeratoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid Serato crate binary data: {0}")]
    InvalidFormat(String),
}

/// Formats a hierarchical crate path (such as `["Dubs", "Deep"]`) as a Serato
/// subcrate filename (`Dubs%%Deep.crate`).
pub fn subcrate_filename(segments: &[&str]) -> String {
    format!("{}.crate", segments.join(SUBCRATE_SEPARATOR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_nested_subcrate_filename() {
        assert_eq!(subcrate_filename(&["Dubs", "Deep"]), "Dubs%%Deep.crate");
    }
}
