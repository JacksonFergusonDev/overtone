//! Carrying file metadata (extended attributes such as Finder tags, and the
//! creation date) from the original onto the edited copy before it
//! replaces the original.

use std::path::Path;

use crate::AudioError;

/// Extended attributes macOS manages itself and refuses to set from user
/// code; losing them on the edited copy is expected.
const SYSTEM_MANAGED: &[&str] = &["com.apple.provenance", "com.apple.rootless"];

/// Copies every extended attribute of `original` onto `copy` and restores
/// the creation date. Fails if a user-visible attribute (Finder tags,
/// comments, Spotlight metadata) can't be carried over.
pub(crate) fn carry_over(original: &Path, copy: &Path) -> Result<(), AudioError> {
    if xattr::SUPPORTED_PLATFORM {
        let names = xattr::list(original).map_err(|err| AudioError::io(original, err))?;
        for name in names {
            let label = name.to_string_lossy().into_owned();
            let Some(value) =
                xattr::get(original, &name).map_err(|err| AudioError::io(original, err))?
            else {
                continue;
            };
            let current = xattr::get(copy, &name).ok().flatten();
            if current.as_deref() == Some(value.as_slice()) {
                continue;
            }
            if let Err(err) = xattr::set(copy, &name, &value)
                && !SYSTEM_MANAGED.contains(&label.as_str())
            {
                return Err(AudioError::VerificationFailed {
                    path: original.to_path_buf(),
                    check: "file metadata",
                    detail: format!("couldn't keep extended attribute {label}: {err}"),
                });
            }
        }
    }
    restore_created(original, copy)
}

#[cfg(target_os = "macos")]
fn restore_created(original: &Path, copy: &Path) -> Result<(), AudioError> {
    use std::fs::{FileTimes, OpenOptions};
    use std::os::macos::fs::FileTimesExt;

    let Ok(created) = std::fs::metadata(original).and_then(|m| m.created()) else {
        return Ok(());
    };
    let file = OpenOptions::new()
        .write(true)
        .open(copy)
        .map_err(|err| AudioError::io(copy, err))?;
    file.set_times(FileTimes::new().set_created(created))
        .map_err(|err| AudioError::io(copy, err))
}

#[cfg(not(target_os = "macos"))]
fn restore_created(_original: &Path, _copy: &Path) -> Result<(), AudioError> {
    Ok(())
}
