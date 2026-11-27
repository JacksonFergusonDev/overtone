//! Walking a library root for audio files.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use overtone_model::{UnixMillis, is_audio_extension};
use walkdir::{DirEntry, WalkDir};

use super::ScanFailure;

/// How often, in files found, the walk reports progress.
const PROGRESS_EVERY: usize = 256;

/// Folder and file names the walk never enters or reports, beyond anything
/// starting with a dot (hidden files, AppleDouble `._*` files, `.Trashes`,
/// `.Spotlight-V100`, `.fseventsd`). These are Windows equivalents found on
/// FAT and exFAT drives.
const SKIPPED_NAMES: &[&str] = &["$RECYCLE.BIN", "System Volume Information"];

/// An audio file found on disk, with the facts that decide whether it
/// needs reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FoundFile {
    pub path: String,
    pub file_size: u64,
    pub modified_at: UnixMillis,
}

/// Everything learned from walking one root.
#[derive(Debug, Default)]
pub(super) struct RootWalk {
    /// The canonical root path, as stored for tracks under it.
    pub root: String,
    pub files: Vec<FoundFile>,
    /// Paths the walk couldn't read. Tracks at or under them may still
    /// exist, so they must not be marked missing.
    pub unreadable: Vec<String>,
    pub failures: Vec<ScanFailure>,
}

/// Walks `root` without following symlinks. Returns `None` when the root
/// itself is unreachable, such as an unplugged drive.
pub(super) fn walk_root(root: &Path, on_found: &mut dyn FnMut(usize)) -> Option<RootWalk> {
    let canonical = std::fs::canonicalize(root).ok().filter(|p| p.is_dir())?;
    let Some(root_str) = canonical.to_str() else {
        return Some(RootWalk {
            root: root.to_string_lossy().into_owned(),
            failures: vec![ScanFailure::new(
                &canonical,
                "folder path is not valid UTF-8",
            )],
            unreadable: vec![root.to_string_lossy().into_owned()],
            ..RootWalk::default()
        });
    };
    let mut walk = RootWalk {
        root: root_str.trim_end_matches('/').to_owned(),
        ..RootWalk::default()
    };
    let entries = WalkDir::new(&canonical)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped(entry));
    let mut reported = 0;
    for entry in entries {
        match entry {
            Ok(entry) => record_entry(&entry, &mut walk),
            Err(err) => {
                let path = err
                    .path()
                    .map_or_else(|| canonical.clone(), Path::to_path_buf);
                walk.unreadable.push(path.to_string_lossy().into_owned());
                walk.failures
                    .push(ScanFailure::new(&path, &err.to_string()));
            }
        }
        if walk.files.len() >= reported + PROGRESS_EVERY {
            reported = walk.files.len();
            on_found(reported);
        }
    }
    on_found(walk.files.len());
    Some(walk)
}

fn is_skipped(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    name.starts_with('.') || SKIPPED_NAMES.contains(&name.as_ref())
}

fn record_entry(entry: &DirEntry, walk: &mut RootWalk) {
    // Symlinks report their own type here, so linked files are skipped.
    if !entry.file_type().is_file() || !has_audio_extension(entry.path()) {
        return;
    }
    let Some(path) = entry.path().to_str() else {
        walk.failures.push(ScanFailure::new(
            entry.path(),
            "file path is not valid UTF-8, so the library can't store it",
        ));
        return;
    };
    match entry.metadata() {
        Ok(metadata) => walk.files.push(FoundFile {
            path: path.to_owned(),
            file_size: metadata.len(),
            modified_at: metadata.modified().map_or(0, unix_millis),
        }),
        Err(err) => {
            walk.unreadable.push(path.to_owned());
            walk.failures
                .push(ScanFailure::new(entry.path(), &err.to_string()));
        }
    }
}

fn has_audio_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(is_audio_extension)
}

/// Milliseconds since the Unix epoch, negative for earlier times.
pub(super) fn unix_millis(time: SystemTime) -> UnixMillis {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// Whether `path` is `prefix` itself or lies under it.
pub(super) fn is_at_or_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Canonical form of a folder for storing as a library root.
pub(super) fn canonical_dir(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path).map_err(|err| err.to_string())?;
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err("not a folder".to_owned())
    }
}
