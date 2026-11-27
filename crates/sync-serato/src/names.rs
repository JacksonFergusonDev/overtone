//! Crate file names: segments joined with `%%`, plus `.crate`.

use crate::SeratoError;
use crate::paths::path_key;

/// Subcrate nesting delimiter used in `_Serato_/Subcrates/` file names.
pub const SUBCRATE_SEPARATOR: &str = "%%";

/// Extension of crate files.
pub const CRATE_EXTENSION: &str = ".crate";

/// Longest file name macOS accepts, in bytes.
const MAX_FILENAME_BYTES: usize = 255;

/// Why a crate name can't be used.
fn problem(segment: &str) -> Option<&'static str> {
    if segment.trim().is_empty() {
        Some("is empty")
    } else if segment != segment.trim() {
        Some("has leading or trailing spaces")
    } else if segment.contains('/') {
        Some("contains '/'")
    } else if segment.contains(':') {
        Some("contains ':'")
    } else if segment.contains(SUBCRATE_SEPARATOR) {
        Some("contains '%%'")
    } else if segment.starts_with('%') || segment.ends_with('%') {
        // `a%` + `%%` + `b` would read back as `a` + `%%` + `%b`.
        Some("starts or ends with '%'")
    } else if segment.chars().any(char::is_control) {
        Some("contains a control character")
    } else {
        None
    }
}

/// Replaces what Serato can't hold in a crate name with `-`, and trims it.
/// Returns `None` if nothing usable is left.
pub fn sanitize_segment(segment: &str) -> Option<String> {
    let mut text = segment.replace(SUBCRATE_SEPARATOR, "-");
    text = text
        .chars()
        .map(|c| {
            if matches!(c, '/' | ':' | '%' | '\\') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Builds `A%%B%%C.crate` from segments, or says which segment is unusable.
pub fn crate_filename<S: AsRef<str>>(segments: &[S]) -> Result<String, SeratoError> {
    if segments.is_empty() {
        return Err(SeratoError::InvalidName("crate path is empty".to_owned()));
    }
    for segment in segments {
        if let Some(why) = problem(segment.as_ref()) {
            return Err(SeratoError::InvalidName(format!(
                "crate name {:?} {why}",
                segment.as_ref()
            )));
        }
    }
    let joined = segments
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(SUBCRATE_SEPARATOR);
    let name = format!("{joined}{CRATE_EXTENSION}");
    if name.len() > MAX_FILENAME_BYTES {
        return Err(SeratoError::InvalidName(format!(
            "crate file name is {} bytes, over the {MAX_FILENAME_BYTES} byte limit",
            name.len()
        )));
    }
    Ok(name)
}

/// The crate name without extension, as used in `neworder.pref`.
pub fn crate_name<S: AsRef<str>>(segments: &[S]) -> Result<String, SeratoError> {
    crate_filename(segments).map(|name| {
        name.strip_suffix(CRATE_EXTENSION)
            .map_or(name.clone(), str::to_owned)
    })
}

/// Splits a crate file name (or a bare crate name) into its segments.
/// Returns `None` for names that aren't crate files.
pub fn parse_crate_filename(filename: &str) -> Option<Vec<String>> {
    let name = filename.strip_suffix(CRATE_EXTENSION)?;
    parse_crate_name(name)
}

/// Splits a crate name without extension into segments.
pub fn parse_crate_name(name: &str) -> Option<Vec<String>> {
    if name.is_empty() {
        return None;
    }
    let segments: Vec<String> = name.split(SUBCRATE_SEPARATOR).map(str::to_owned).collect();
    segments
        .iter()
        .all(|segment| !segment.is_empty())
        .then_some(segments)
}

/// Case- and normalization-insensitive key for a crate name. Serato treats
/// crate names as case-insensitive and APFS is case-insensitive by default,
/// so `Dubs` and `dubs` are the same file.
pub fn name_key(name: &str) -> String {
    path_key(name).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_parses_segments() {
        let name = crate_filename(&["Overtone", "Dubs", "Deep"]).unwrap();
        assert_eq!(name, "Overtone%%Dubs%%Deep.crate");
        assert_eq!(
            parse_crate_filename(&name).unwrap(),
            ["Overtone", "Dubs", "Deep"]
        );
        assert_eq!(crate_name(&["Overtone", "Dubs"]).unwrap(), "Overtone%%Dubs");
    }

    #[test]
    fn keeps_characters_seen_in_real_crates() {
        for text in [
            "Fred Again...",
            "Atmospheric & Progressive",
            "Rock ’n’ Roll",
        ] {
            assert_eq!(
                parse_crate_filename(&crate_filename(&[text]).unwrap()).unwrap(),
                [text]
            );
        }
    }

    #[test]
    fn rejects_unrepresentable_segments() {
        for bad in ["a/b", "a:b", "a%%b", "a%", "%a", "", "  ", " a", "a\nb"] {
            assert!(crate_filename(&[bad]).is_err(), "{bad:?}");
        }
        assert!(crate_filename::<&str>(&[]).is_err());
        assert!(crate_filename(&["x".repeat(300)]).is_err());
    }

    #[test]
    fn sanitizes_to_acceptable_names() {
        assert_eq!(sanitize_segment("a/b: c%%d").as_deref(), Some("a-b- c-d"));
        assert_eq!(sanitize_segment("100%").as_deref(), Some("100-"));
        assert_eq!(sanitize_segment("  / ").as_deref(), Some("-"));
        assert_eq!(sanitize_segment("   "), None);
        for text in ["a/b: c%%d", "100%", "%x%"] {
            let clean = sanitize_segment(text).unwrap();
            assert!(crate_filename(&[clean]).is_ok());
        }
    }

    #[test]
    fn rejects_non_crate_file_names() {
        assert_eq!(parse_crate_filename("notes.txt"), None);
        assert_eq!(parse_crate_filename(".crate"), None);
        assert_eq!(parse_crate_filename("a%%%%b.crate"), None);
    }

    #[test]
    fn keys_ignore_case_and_normalization() {
        assert_eq!(name_key("Caf\u{e9}"), name_key("CAFE\u{301}"));
    }
}
