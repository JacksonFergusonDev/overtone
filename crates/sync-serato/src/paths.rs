//! Mapping between absolute file paths and Serato's per-volume library
//! folders and `ptrk` strings.
//!
//! Serato keeps one `_Serato_` folder per volume. Files on the boot volume
//! belong to `~/Music/_Serato_` and their `ptrk` is the absolute path without
//! the leading slash. Files on an external drive belong to
//! `/Volumes/<Name>/_Serato_` and their `ptrk` is relative to the drive root.

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// Name of the library folder Serato creates on each volume.
pub const SERATO_DIR_NAME: &str = "_Serato_";

/// Folder inside `_Serato_` that holds `.crate` files.
pub const SUBCRATES_DIR: &str = "Subcrates";

const VOLUMES: &str = "/Volumes";

/// The `_Serato_` folder that owns `path`, and the `ptrk` string Serato
/// stores for it there.
///
/// `home` is the user's home directory, passed in so callers and tests
/// don't depend on the real one. The `ptrk` keeps the path's Unicode form
/// exactly: macOS file names may be NFC or NFD and Serato stores whichever
/// the file system has. Non-UTF-8 bytes in a path are replaced, which
/// Serato's UTF-16 storage can't represent anyway.
pub fn serato_dir_for(path: &Path, home: &Path) -> (PathBuf, String) {
    let volumes = Path::new(VOLUMES);
    if let Ok(rest) = path.strip_prefix(volumes) {
        let mut parts = rest.components();
        if let Some(name) = parts.next() {
            let relative = parts.as_path();
            if !relative.as_os_str().is_empty() {
                let dir = volumes.join(name.as_os_str()).join(SERATO_DIR_NAME);
                return (dir, relative.to_string_lossy().into_owned());
            }
        }
    }
    let text = path.to_string_lossy();
    (
        home.join("Music").join(SERATO_DIR_NAME),
        text.trim_start_matches('/').to_owned(),
    )
}

/// Inverse of [`serato_dir_for`]: the absolute path a `ptrk` refers to when
/// read from `serato_dir`.
pub fn absolute_path_for(serato_dir: &Path, ptrk: &str, home: &Path) -> PathBuf {
    let on_volume = serato_dir
        .strip_prefix(VOLUMES)
        .is_ok_and(|rest| rest.components().count() == 2);
    if serato_dir == home.join("Music").join(SERATO_DIR_NAME) || !on_volume {
        return Path::new("/").join(ptrk);
    }
    serato_dir
        .parent()
        .map_or_else(|| Path::new("/").join(ptrk), |volume| volume.join(ptrk))
}

/// Folder holding the `.crate` files for a `_Serato_` folder.
pub fn subcrates_dir(serato_dir: &Path) -> PathBuf {
    serato_dir.join(SUBCRATES_DIR)
}

/// Normalization-insensitive path equality: `é` as one code point equals
/// `e` plus a combining accent. Use for comparisons only; never rewrite a
/// stored `ptrk` into a different form.
pub fn same_path(a: &str, b: &str) -> bool {
    a == b || a.nfc().eq(b.nfc())
}

/// Key under which two paths that [`same_path`] considers equal collide.
pub fn path_key(path: &str) -> String {
    path.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/dj";

    #[test]
    fn boot_volume_paths_use_music_serato_without_leading_slash() {
        let (dir, ptrk) = serato_dir_for(
            Path::new("/Users/dj/Music/High Quality/Bass/a.flac"),
            Path::new(HOME),
        );
        assert_eq!(dir, Path::new("/Users/dj/Music/_Serato_"));
        assert_eq!(ptrk, "Users/dj/Music/High Quality/Bass/a.flac");
    }

    #[test]
    fn external_volume_paths_are_relative_to_the_volume_root() {
        let (dir, ptrk) = serato_dir_for(
            Path::new("/Volumes/Big Disk/Music/Dubs/a.mp3"),
            Path::new(HOME),
        );
        assert_eq!(dir, Path::new("/Volumes/Big Disk/_Serato_"));
        assert_eq!(ptrk, "Music/Dubs/a.mp3");
    }

    #[test]
    fn a_bare_volume_root_is_not_a_track_on_that_volume() {
        let (dir, _) = serato_dir_for(Path::new("/Volumes/Big Disk"), Path::new(HOME));
        assert_eq!(dir, Path::new("/Users/dj/Music/_Serato_"));
    }

    #[test]
    fn unicode_form_is_preserved() {
        let nfd = "/Users/dj/Music/Beyonce\u{301}/Tre\u{300}s.flac";
        let (_, ptrk) = serato_dir_for(Path::new(nfd), Path::new(HOME));
        assert_eq!(ptrk, nfd.trim_start_matches('/'));
        assert!(ptrk.contains("e\u{301}"));
        assert!(!ptrk.contains('\u{e9}'));
    }

    #[test]
    fn comparison_ignores_normalization_but_not_content() {
        let nfc = "Users/dj/Music/Beyonc\u{e9}/a.flac";
        let nfd = "Users/dj/Music/Beyonce\u{301}/a.flac";
        assert_ne!(nfc, nfd);
        assert!(same_path(nfc, nfd));
        assert_eq!(path_key(nfc), path_key(nfd));
        assert!(!same_path(nfc, "Users/dj/Music/Beyonce/a.flac"));
    }

    #[test]
    fn absolute_path_round_trips() {
        let home = Path::new(HOME);
        for path in [
            "/Users/dj/Music/a.flac",
            "/Volumes/X/Music/a.flac",
            "/opt/audio/a.flac",
        ] {
            let (dir, ptrk) = serato_dir_for(Path::new(path), home);
            assert_eq!(absolute_path_for(&dir, &ptrk, home), Path::new(path));
        }
    }
}
