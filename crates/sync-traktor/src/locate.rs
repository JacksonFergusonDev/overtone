//! Finds Traktor's collection files.
//!
//! Traktor keeps one folder per installed version, for example
//! `~/Documents/Native Instruments/Traktor 4.1.0/collection.nml`.

use std::path::{Path, PathBuf};

/// File name of Traktor's collection.
pub const COLLECTION_FILE_NAME: &str = "collection.nml";

/// Every `Native Instruments/Traktor */collection.nml` under `documents`,
/// sorted by path. Only reads directory listings.
pub fn find_collections(documents: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(documents.join("Native Instruments")) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("Traktor "))
        })
        .map(|entry| entry.path().join(COLLECTION_FILE_NAME))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_collections_per_version() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let ni = dir.path().join("Native Instruments");
        for version in [
            "Traktor 4.1.0",
            "Traktor 3.11.1",
            "Traktor Kontrol",
            "Other",
        ] {
            std::fs::create_dir_all(ni.join(version)).expect("mkdir");
        }
        for version in ["Traktor 4.1.0", "Traktor 3.11.1", "Other"] {
            std::fs::write(ni.join(version).join(COLLECTION_FILE_NAME), b"").expect("write");
        }
        assert_eq!(
            find_collections(dir.path()),
            vec![
                ni.join("Traktor 3.11.1/collection.nml"),
                ni.join("Traktor 4.1.0/collection.nml"),
            ]
        );
        assert!(find_collections(&dir.path().join("missing")).is_empty());
    }
}
