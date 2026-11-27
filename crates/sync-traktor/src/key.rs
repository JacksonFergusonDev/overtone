//! Traktor's track keys.
//!
//! A collection entry's `LOCATION` splits a file's path into `VOLUME`,
//! `DIR`, and `FILE`. Playlists refer to tracks by `VOLUME` + `DIR` + `FILE`,
//! for example `Macintosh HD/:Users/:me/:Music/:a.mp3`. In `DIR`, every `/`
//! becomes `/:`, and a literal `:` in a folder or file name becomes `//`.
//!
//! Files on the startup disk use the startup volume's name. Files under
//! `/Volumes/<Drive>/` are assumed to use `<Drive>` as the volume, with
//! `DIR` relative to the drive. That external-drive mapping follows the
//! format survey and is unverified against a real Traktor install, see
//! `engineering/manual-verification.md`.

use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// Directory separator used inside `LOCATION/@DIR` and keys.
pub const NML_DIR_SEPARATOR: &str = "/:";

/// What a literal `:` in a path component becomes.
const ESCAPED_COLON: &str = "//";

/// Startup volume name used when the collection gives no better guess.
pub const DEFAULT_STARTUP_VOLUME: &str = "Macintosh HD";

/// A file's `LOCATION` in Traktor's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub volume: String,
    /// Encoded, for example `/:Users/:me/:Music/:`.
    pub dir: String,
    /// Encoded file name.
    pub file: String,
}

impl Location {
    /// `VOLUME` + `DIR` + `FILE`, the key playlists refer to.
    pub fn key(&self) -> String {
        format!("{}{}{}", self.volume, self.dir, self.file)
    }
}

/// Encodes one folder or file name: a literal `:` becomes `//`.
pub fn encode_name(name: &str) -> String {
    name.replace(':', ESCAPED_COLON)
}

/// Reverses [`encode_name`].
pub fn decode_name(name: &str) -> String {
    name.replace(ESCAPED_COLON, ":")
}

/// Encodes folder names (such as `["Users", "dj", "Music"]`) into the
/// `DIR` format (`/:Users/:dj/:Music/:`).
pub fn encode_nml_dir(components: &[&str]) -> String {
    let mut dir = NML_DIR_SEPARATOR.to_string();
    for component in components {
        dir.push_str(&encode_name(component));
        dir.push_str(NML_DIR_SEPARATOR);
    }
    dir
}

/// The volume a file lives on and its path relative to that volume's root
/// (still starting with `/`). `/Volumes/<X>/rest` is on volume `X`;
/// anything else is on `startup_volume`.
pub fn volume_for(path: &Path, startup_volume: &str) -> (String, PathBuf) {
    if let Ok(rest) = path.strip_prefix("/Volumes")
        && let Some(Component::Normal(volume)) = rest.components().next()
    {
        let inner = rest.strip_prefix(volume).unwrap_or(rest);
        return (
            volume.to_string_lossy().into_owned(),
            Path::new("/").join(inner),
        );
    }
    (startup_volume.to_string(), path.to_path_buf())
}

/// The key for a file at `path`, relative to the root of `volume` (as
/// [`volume_for`] returns it). `None` when the path has no file name or
/// isn't valid UTF-8.
pub fn traktor_key(volume: &str, path: &Path) -> Option<String> {
    location_in(volume, path).map(|location| location.key())
}

fn location_in(volume: &str, path: &Path) -> Option<Location> {
    let file = path.file_name()?.to_str()?;
    let mut components = Vec::new();
    for component in path.parent()?.components() {
        match component {
            Component::Normal(name) => components.push(name.to_str()?),
            Component::RootDir => {}
            _ => return None,
        }
    }
    Some(Location {
        volume: volume.to_string(),
        dir: encode_nml_dir(&components),
        file: encode_name(file),
    })
}

/// The `LOCATION` an absolute file path would have in a collection whose
/// startup volume is `startup_volume`.
pub fn location_for(path: &Path, startup_volume: &str) -> Option<Location> {
    let (volume, relative) = volume_for(path, startup_volume);
    location_in(&volume, &relative)
}

/// Reverses [`location_for`]: the absolute path a key points at.
pub fn path_from_key(key: &str, startup_volume: &str) -> Option<PathBuf> {
    let split = key.find(NML_DIR_SEPARATOR)?;
    let (volume, rest) = key.split_at(split);
    let mut path = if volume == startup_volume {
        PathBuf::from("/")
    } else {
        Path::new("/Volumes").join(volume)
    };
    for component in rest[NML_DIR_SEPARATOR.len()..].split(NML_DIR_SEPARATOR) {
        if component.is_empty() {
            return None;
        }
        path.push(decode_name(component));
    }
    Some(path)
}

/// Unicode NFC form, used to match keys regardless of normalization.
pub fn nfc(text: &str) -> String {
    text.nfc().collect()
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
        assert_eq!(encode_nml_dir(&[]), "/:");
    }

    #[test]
    fn builds_startup_disk_keys() {
        let location =
            location_for(Path::new("/Users/me/Music/a.mp3"), "Macintosh HD").expect("location");
        assert_eq!(location.dir, "/:Users/:me/:Music/:");
        assert_eq!(location.file, "a.mp3");
        assert_eq!(location.key(), "Macintosh HD/:Users/:me/:Music/:a.mp3");
    }

    #[test]
    fn maps_external_volumes() {
        let (volume, rest) = volume_for(Path::new("/Volumes/Gig/Sets/a.flac"), "Macintosh HD");
        assert_eq!(
            (volume.as_str(), rest.as_path()),
            ("Gig", Path::new("/Sets/a.flac"))
        );
        assert_eq!(
            location_for(Path::new("/Volumes/Gig/a.flac"), "SSD")
                .expect("location")
                .key(),
            "Gig/:a.flac"
        );
        assert_eq!(
            traktor_key("Gig", Path::new("/Sets/a.flac")).as_deref(),
            Some("Gig/:Sets/:a.flac")
        );
    }

    #[test]
    fn round_trips_colons_and_nfd_names() {
        for path in [
            "/Users/me/Music/a.mp3",
            "/Users/me/Music/Live: 2024/b:c.flac",
            "/Users/me/Music/x:/:y/d.wav",
            "/Users/me/Music/Cafe\u{301} del Mar.mp3",
            "/Volumes/Gig/Sets/e.aiff",
            "/Volumes/Gig/f.mp3",
        ] {
            let key = location_for(Path::new(path), "Macintosh HD")
                .expect("location")
                .key();
            assert_eq!(
                path_from_key(&key, "Macintosh HD"),
                Some(PathBuf::from(path)),
                "{key}"
            );
        }
        let key = location_for(Path::new("/Users/me/Live: 2024/b:c.flac"), "Macintosh HD")
            .expect("location")
            .key();
        assert_eq!(key, "Macintosh HD/:Users/:me/:Live// 2024/:b//c.flac");
    }

    #[test]
    fn normalizes_to_nfc() {
        assert_eq!(nfc("Cafe\u{301}"), "Caf\u{e9}");
        assert_eq!(nfc("Caf\u{e9}"), "Caf\u{e9}");
    }

    #[test]
    fn rejects_paths_without_a_file_name() {
        assert_eq!(location_for(Path::new("/"), "Macintosh HD"), None);
        assert_eq!(path_from_key("no separator", "Macintosh HD"), None);
    }
}
