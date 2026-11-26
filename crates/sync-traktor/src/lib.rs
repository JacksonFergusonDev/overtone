//! Lossless `collection.nml` streaming patcher for Traktor Pro 3 and 4.
//!
//! See `engineering/decisions/dj-software-sync.md` for why Overtone patches
//! `collection.nml` via `quick-xml` event passthrough instead of `serde`
//! struct roundtripping.

use thiserror::Error;

/// Directory separator used inside Traktor NML `LOCATION/@DIR` attributes.
pub const NML_DIR_SEPARATOR: &str = "/:";

/// Errors produced during Traktor `collection.nml` reading or patching.
#[derive(Debug, Error)]
pub enum TraktorError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Traktor must be closed before patching collection.nml")]
    AppRunning,
}

/// Encodes path components (such as `["Users", "dj", "Music"]`) into Traktor's
/// `/:`-delimited `DIR` attribute format (`/:Users/:dj/:Music/:`).
pub fn encode_nml_dir(components: &[&str]) -> String {
    if components.is_empty() {
        return NML_DIR_SEPARATOR.to_string();
    }
    format!(
        "{NML_DIR_SEPARATOR}{}{NML_DIR_SEPARATOR}",
        components.join(NML_DIR_SEPARATOR)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_traktor_dir_path() {
        assert_eq!(
            encode_nml_dir(&["Users", "dj", "Music"]),
            "/:Users/:dj/:Music/:"
        );
    }
}
