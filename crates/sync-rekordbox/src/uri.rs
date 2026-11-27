//! `Location` attribute encoding.
//!
//! Hand-written instead of the `percent-encoding` crate: the rule is one
//! line (keep `A-Z a-z 0-9 - . _ ~ /`, encode every other byte), and it must
//! work on raw path bytes without any Unicode handling, which is what keeps
//! NFC and NFD file names distinct.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

const PREFIX: &str = "file://localhost";

/// `file://localhost/...` for an absolute path. Every byte except
/// `A-Z a-z 0-9 - . _ ~ /` is percent-encoded from the path's raw bytes, so
/// no Unicode normalization happens and the file is found exactly as stored.
/// Empty and repeated separators are dropped, so the result never contains a
/// double slash.
pub fn location_uri(path: &Path) -> String {
    let mut out = String::from(PREFIX);
    let mut any = false;
    for component in path.components() {
        let bytes: &[u8] = match component {
            Component::RootDir | Component::CurDir | Component::Prefix(_) => continue,
            Component::ParentDir => b"..",
            Component::Normal(name) => name.as_bytes(),
        };
        out.push('/');
        for &byte in bytes {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(byte));
            } else {
                let _ = write!(out, "%{byte:02X}");
            }
        }
        any = true;
    }
    if !any {
        out.push('/');
    }
    out
}

/// Inverse of [`location_uri`]. Also accepts `file:///...`. Returns `None`
/// when the text isn't a `file://` location or has a bad escape.
pub fn location_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri
        .strip_prefix(PREFIX)
        .or_else(|| uri.strip_prefix("file://"))?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(OsStr::from_bytes(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_special_characters() {
        let path = Path::new("/Users/dj/My Music/A & B (live) 100% #1.mp3");
        assert_eq!(
            location_uri(path),
            "file://localhost/Users/dj/My%20Music/A%20%26%20B%20%28live%29%20100%25%20%231.mp3"
        );
    }

    #[test]
    fn keeps_nfc_and_nfd_distinct() {
        let nfc = location_uri(Path::new("/m/\u{e9}.mp3"));
        let nfd = location_uri(Path::new("/m/e\u{301}.mp3"));
        assert_eq!(nfc, "file://localhost/m/%C3%A9.mp3");
        assert_eq!(nfd, "file://localhost/m/e%CC%81.mp3");
    }

    #[test]
    fn never_emits_a_double_slash() {
        let uri = location_uri(Path::new("//a///b//c.mp3"));
        assert_eq!(uri, "file://localhost/a/b/c.mp3");
        assert!(!uri["file://".len()..].contains("//"));
        assert_eq!(location_uri(Path::new("/")), "file://localhost/");
    }

    #[test]
    fn decodes_what_it_encodes() {
        for raw in [
            "/Users/dj/My Music/A & B (live) 100% #1.mp3",
            "/m/\u{e9}.mp3",
            "/m/e\u{301}.mp3",
        ] {
            let path = Path::new(raw);
            assert_eq!(location_from_uri(&location_uri(path)), Some(path.into()));
        }
        assert_eq!(
            location_from_uri("file:///a/b%20c"),
            Some(PathBuf::from("/a/b c"))
        );
        assert_eq!(location_from_uri("http://x"), None);
        assert_eq!(location_from_uri("file://localhost/a%2"), None);
    }
}
