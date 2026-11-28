//! Library scanning: find audio files under the library's folders, read
//! and hash the new or changed ones, and record what disappeared.
//!
//! Scans are incremental. A file whose size and modification time match
//! its stored record is not opened at all, so rescanning a large, mostly
//! unchanged library costs one directory walk. New and changed files are
//! read in parallel, then saved in batches.
//!
//! The scanner opens audio files read-only and writes nothing but the
//! library database. Paths are stored exactly as the filesystem returns
//! them under the canonical root, and never change afterwards (see
//! `engineering/decisions/architecture-and-ui.md#library-model`).

#[cfg(test)]
mod tests;
mod walk;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use overtone_audio::AudioError;
use overtone_audio::hash::hash_payload;
use overtone_audio::tags::{FileMetadata, read_metadata};
use overtone_library::{Library, LibraryRoot, ScannedTrack, StoredFileState, UpsertOutcome};
use overtone_model::{AudioFormat, TrackId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;
use walk::{FoundFile, RootWalk, is_at_or_under, unix_millis, walk_root};

/// Files read and hashed between database saves. Large enough that the
/// per-transaction cost vanishes, small enough that an interrupted scan
/// keeps most of its work and progress stays smooth.
const SAVE_BATCH: usize = 500;

/// What a scan did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ScanReport {
    /// Files seen for the first time.
    pub added: u64,
    /// Known files whose size or modification time changed, re-read.
    pub updated: u64,
    /// Known files that matched their record and weren't opened.
    pub unchanged: u64,
    /// Tracks under a scanned folder whose file wasn't found. Their records,
    /// vibes, and reports are kept and flagged as missing.
    pub missing: u64,
    /// Files that couldn't be fully read. Tracks are still stored when the
    /// file exists; a failed hash just leaves `audio_hash` empty.
    pub failed: Vec<ScanFailure>,
    /// Library folders that couldn't be reached, such as an unplugged
    /// drive. Their tracks were left as they were, not marked missing.
    pub unavailable_roots: Vec<String>,
    pub elapsed_ms: u64,
}

/// One file the scan couldn't fully read, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ScanFailure {
    pub path: String,
    pub reason: String,
}

impl ScanFailure {
    fn new(path: &Path, reason: &str) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            reason: reason.to_owned(),
        }
    }
}

/// Progress updates, in phase order. Counts are cumulative within a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ScanProgress {
    /// Audio files found so far.
    Walking { found: u64 },
    /// New or changed files read and hashed, out of `total`.
    Processing { done: u64, total: u64 },
    /// Read files saved to the library, out of `total`.
    Saving { done: u64, total: u64 },
}

/// Adds `path` as a library folder, stored as its canonical absolute path
/// without a trailing slash, so track paths under it compare literally.
pub fn add_root(library: &Library, path: &Path) -> Result<LibraryRoot, CoreError> {
    let invalid = |reason: String| CoreError::InvalidRoot {
        path: path.to_path_buf(),
        reason,
    };
    let canonical = walk::canonical_dir(path).map_err(invalid)?;
    let text = canonical
        .to_str()
        .ok_or_else(|| invalid("path is not valid UTF-8".to_owned()))?;
    Ok(library.add_root(text)?)
}

/// Library setting recording which metadata reader filled the library.
const READER_VERSION_SETTING: &str = "scan.reader_version";

/// Options for a scan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// Reread every file, even unchanged ones. Happens automatically when
    /// the metadata reader is newer than the one that filled the library.
    pub refresh: bool,
}

/// Scans every folder in the library.
pub fn scan_library(
    library: &mut Library,
    on_progress: &(dyn Fn(ScanProgress) + Sync),
) -> Result<ScanReport, CoreError> {
    scan_library_with(library, ScanOptions::default(), on_progress)
}

/// [`scan_library`] with options.
pub fn scan_library_with(
    library: &mut Library,
    options: ScanOptions,
    on_progress: &(dyn Fn(ScanProgress) + Sync),
) -> Result<ScanReport, CoreError> {
    let roots: Vec<PathBuf> = library
        .roots()?
        .into_iter()
        .map(|root| PathBuf::from(root.path))
        .collect();
    scan_roots_with(library, &roots, options, on_progress)
}

/// Scans `roots`, adding new files, refreshing changed ones, and marking
/// tracks whose files are gone as missing. `on_progress` may be called from
/// several threads at once.
pub fn scan_roots(
    library: &mut Library,
    roots: &[PathBuf],
    on_progress: &(dyn Fn(ScanProgress) + Sync),
) -> Result<ScanReport, CoreError> {
    scan_roots_with(library, roots, ScanOptions::default(), on_progress)
}

/// [`scan_roots`] with options.
pub fn scan_roots_with(
    library: &mut Library,
    roots: &[PathBuf],
    options: ScanOptions,
    on_progress: &(dyn Fn(ScanProgress) + Sync),
) -> Result<ScanReport, CoreError> {
    let reader_version = overtone_audio::tags::READER_VERSION;
    let stored_version: u32 = library.setting(READER_VERSION_SETTING)?.unwrap_or(0);
    let refresh = options.refresh || stored_version < reader_version;
    let started = Instant::now();
    let mut report = ScanReport::default();

    let walks = walk_all(roots, on_progress, &mut report);
    let stored = stored_states(library, &walks)?;
    // FLACs from conversions waiting for relocation aren't tracks yet.
    let waiting = library.pending_conversion_outputs()?;
    let found: Vec<&FoundFile> = unique_files(&walks)
        .into_iter()
        .filter(|file| !waiting.contains(&file.path))
        .collect();

    let mut reappeared = Vec::new();
    let mut to_read = Vec::new();
    for file in &found {
        match stored.get(file.path.as_str()) {
            Some(state)
                if !refresh
                    && state.file_size == file.file_size
                    && state.modified_at == file.modified_at =>
            {
                report.unchanged += 1;
                if state.missing_since.is_some() {
                    reappeared.push(state.id);
                }
            }
            _ => to_read.push(*file),
        }
    }
    library.set_missing(&reappeared, None)?;

    read_and_save(library, &to_read, refresh, on_progress, &mut report)?;
    mark_missing(library, &walks, &found, &stored, &mut report)?;
    if stored_version < reader_version {
        library.set_setting(READER_VERSION_SETTING, &reader_version)?;
    }

    report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

fn walk_all(
    roots: &[PathBuf],
    on_progress: &(dyn Fn(ScanProgress) + Sync),
    report: &mut ScanReport,
) -> Vec<RootWalk> {
    let mut walks = Vec::new();
    let mut found_before = 0;
    for root in roots {
        let mut on_found = |n: usize| {
            on_progress(ScanProgress::Walking {
                found: (found_before + n) as u64,
            });
        };
        match walk_root(root, &mut on_found) {
            Some(mut walk) => {
                found_before += walk.files.len();
                report.failed.append(&mut walk.failures);
                walks.push(walk);
            }
            None => report
                .unavailable_roots
                .push(root.to_string_lossy().into_owned()),
        }
    }
    walks
}

/// Stored records under every walked root, by path.
fn stored_states(
    library: &Library,
    walks: &[RootWalk],
) -> Result<HashMap<String, StoredFileState>, CoreError> {
    let mut stored = HashMap::new();
    for walk in walks {
        for state in library.file_states_under(&walk.root)? {
            stored.insert(state.path.clone(), state);
        }
    }
    Ok(stored)
}

/// Found files across all roots, once each even when roots nest.
fn unique_files(walks: &[RootWalk]) -> Vec<&FoundFile> {
    let mut seen = HashSet::new();
    walks
        .iter()
        .flat_map(|walk| &walk.files)
        .filter(|file| seen.insert(file.path.as_str()))
        .collect()
}

fn read_and_save(
    library: &mut Library,
    files: &[&FoundFile],
    refresh: bool,
    on_progress: &(dyn Fn(ScanProgress) + Sync),
    report: &mut ScanReport,
) -> Result<(), CoreError> {
    let total = files.len() as u64;
    let read = AtomicU64::new(0);
    let mut saved = 0;
    for batch in files.chunks(SAVE_BATCH) {
        let results: Vec<(ScannedTrack, Option<ScanFailure>)> = batch
            .par_iter()
            .map(|file| {
                let result = read_file(file);
                let done = read.fetch_add(1, Ordering::Relaxed) + 1;
                on_progress(ScanProgress::Processing { done, total });
                result
            })
            .collect();
        let (tracks, failures): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        report.failed.extend(failures.into_iter().flatten());
        for (_, outcome) in library.upsert_scanned_batch_with(&tracks, refresh)? {
            match outcome {
                UpsertOutcome::Inserted => report.added += 1,
                UpsertOutcome::Updated => report.updated += 1,
                UpsertOutcome::Unchanged => report.unchanged += 1,
            }
        }
        saved += tracks.len() as u64;
        on_progress(ScanProgress::Saving { done: saved, total });
    }
    Ok(())
}

/// Reads tags and hashes the audio of one file. Problems are reported, not
/// fatal: the track is stored with whatever could be read, so it shows up in
/// the library and can be reviewed.
fn read_file(file: &FoundFile) -> (ScannedTrack, Option<ScanFailure>) {
    let path = Path::new(&file.path);
    let mut problems = Vec::new();
    let FileMetadata {
        mut properties,
        tags,
    } = match read_metadata(path) {
        Ok(metadata) => metadata,
        // Tag reading lands in a separate work package; until then, a
        // stub is not a problem with the file.
        Err(AudioError::NotImplemented(_)) => FileMetadata::default(),
        Err(err) => {
            problems.push(format!("tags: {err}"));
            FileMetadata::default()
        }
    };
    let (audio_hash, container_format) = match hash_payload(path) {
        Ok(payload) => (Some(payload.hash), payload.format),
        Err(err) => {
            problems.push(format!("audio hash: {err}"));
            (None, None)
        }
    };
    properties.format = properties.format.or(container_format).or_else(|| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(AudioFormat::from_extension)
    });
    let failure = (!problems.is_empty()).then(|| ScanFailure {
        path: file.path.clone(),
        reason: problems.join("; "),
    });
    let track = ScannedTrack {
        path: file.path.clone(),
        file_size: file.file_size,
        modified_at: file.modified_at,
        audio_hash,
        properties,
        tags,
    };
    (track, failure)
}

/// Flags tracks under each walked root whose file wasn't found, keeping the
/// original timestamp for tracks already missing. Paths the walk couldn't
/// read are left alone: their files may well still be there.
fn mark_missing(
    library: &mut Library,
    walks: &[RootWalk],
    found: &[&FoundFile],
    stored: &HashMap<String, StoredFileState>,
    report: &mut ScanReport,
) -> Result<(), CoreError> {
    let found: HashSet<&str> = found.iter().map(|file| file.path.as_str()).collect();
    let unreadable: Vec<&str> = walks
        .iter()
        .flat_map(|walk| walk.unreadable.iter().map(String::as_str))
        .collect();
    let mut missing: Vec<&StoredFileState> = stored
        .values()
        .filter(|state| !found.contains(state.path.as_str()))
        .filter(|state| {
            !unreadable
                .iter()
                .any(|dir| is_at_or_under(&state.path, dir))
        })
        .collect();
    missing.sort_by_key(|state| state.id);
    report.missing = missing.len() as u64;
    let newly_missing: Vec<TrackId> = missing
        .iter()
        .filter(|state| state.missing_since.is_none())
        .map(|state| state.id)
        .collect();
    library.set_missing(&newly_missing, Some(unix_millis(SystemTime::now())))?;
    Ok(())
}
