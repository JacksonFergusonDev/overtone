//! Shared fixtures for unit tests. Everything lives in a `TempDir`.

use std::path::PathBuf;

use crate::codec::{CrateFile, encode_utf16be};

/// A throwaway home with `Music/_Serato_/{Subcrates,neworder.pref}`.
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub home: PathBuf,
    pub serato: PathBuf,
}

impl Fixture {
    pub fn new(order: &[&str]) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let home = dir.path().join("home");
        let serato = home.join("Music/_Serato_");
        std::fs::create_dir_all(serato.join("Subcrates")).unwrap();
        let mut text = String::from("[begin record]\n");
        for name in order {
            text.push_str(&format!("[crate]{name}\n"));
        }
        text.push_str("[end record]\n");
        std::fs::write(serato.join("neworder.pref"), encode_utf16be(&text)).unwrap();
        Self { dir, home, serato }
    }

    pub fn crate_path(&self, filename: &str) -> PathBuf {
        self.serato.join("Subcrates").join(filename)
    }

    pub fn write_crate(&self, filename: &str, ptrks: &[&str]) {
        let mut file = CrateFile::new();
        file.set_track_paths(ptrks);
        std::fs::write(self.crate_path(filename), file.to_bytes().unwrap()).unwrap();
    }

    pub fn read_tracks(&self, filename: &str) -> Vec<String> {
        let bytes = std::fs::read(self.crate_path(filename)).unwrap();
        CrateFile::parse(&bytes).unwrap().track_paths().unwrap()
    }

    /// Absolute path of a track under the fixture's `~/Music`.
    pub fn track(&self, rel: &str) -> PathBuf {
        self.home.join("Music").join(rel)
    }

    /// The `ptrk` Serato stores for [`Fixture::track`].
    pub fn ptrk(&self, rel: &str) -> String {
        self.track(rel)
            .to_string_lossy()
            .trim_start_matches('/')
            .to_owned()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.serato.join("Subcrates"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}
