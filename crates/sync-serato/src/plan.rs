//! Dry-run planning for the crates Overtone owns.
//!
//! Overtone manages crates only under one top-level parent crate (default
//! `Overtone`). A vibe `Dubs/Deep` becomes `Overtone%%Dubs%%Deep.crate`, with
//! `Overtone.crate` and `Overtone%%Dubs.crate` ensured as its parents. Crates
//! outside that namespace are never read for changes, modified, or deleted.
//! Everything inside it that no vibe asks for is planned for deletion, so the
//! namespace belongs to Overtone: don't keep your own crates there.
//!
//! Planning only reads. [`crate::apply`] performs the writes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::SeratoError;
use crate::codec::{CrateFile, Record};
use crate::names::{
    CRATE_EXTENSION, crate_filename, crate_name, name_key, parse_crate_filename, parse_crate_name,
};
use crate::neworder::NewOrder;
use crate::paths::{path_key, serato_dir_for, subcrates_dir};

/// Name of the top-level crate Overtone manages by default.
pub const DEFAULT_ROOT_CRATE: &str = "Overtone";

/// File inside `_Serato_` that lists crate order.
pub const NEWORDER_FILE: &str = "neworder.pref";

/// A crate Overtone wants to exist, for example one vibe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, rename = "SeratoDesiredCrate")]
pub struct DesiredCrate {
    /// Vibe path below the root crate, such as `["Dubs", "Deep"]`.
    pub segments: Vec<String>,
    /// Absolute paths of the tracks, in the order they should be added.
    pub tracks: Vec<PathBuf>,
}

/// What a plan does to one crate file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "SeratoFileAction")]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FileAction {
    /// The file doesn't exist yet.
    Create { tracks: usize },
    /// Existing file whose track list changes.
    Update { added: usize, removed: usize },
    /// Nothing to do.
    Unchanged,
    /// A managed crate that no vibe asks for any more.
    Delete { tracks: usize },
}

/// Planned change to one `Subcrates/*.crate` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "SeratoFilePlan")]
pub struct SeratoFilePlan {
    /// File name inside `Subcrates`, such as `Overtone%%Dubs.crate`.
    pub filename: String,
    pub action: FileAction,
    /// Tracks in the file after the plan is applied.
    pub track_count: usize,
    /// File contents when planned, to detect changes before applying.
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) original: Option<Vec<u8>>,
    /// File contents to write for Create and Update.
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) new_bytes: Option<Vec<u8>>,
}

/// Planned change to `neworder.pref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "SeratoNewOrderChange")]
pub struct NewOrderChange {
    /// Whether `neworder.pref` exists. Overtone doesn't create it: without
    /// one Serato sorts crates itself.
    pub exists: bool,
    /// Crate names to list.
    pub added: Vec<String>,
    /// Crate names to drop.
    pub removed: Vec<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) original: Option<Vec<u8>>,
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) new_bytes: Option<Vec<u8>>,
}

/// A track left out because it lives on a different volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "SeratoSkippedTrack")]
pub struct SkippedTrack {
    /// Crate name the track was meant for.
    pub crate_name: String,
    pub path: PathBuf,
    /// The `_Serato_` folder this track belongs to instead.
    pub serato_dir: PathBuf,
}

/// Everything a sync would do to one `_Serato_` folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "SeratoSyncPlan")]
pub struct SeratoSyncPlan {
    pub serato_dir: PathBuf,
    /// Top-level crate Overtone owns.
    pub root_crate: String,
    /// Creates, updates, and unchanged files in crate order, then deletes
    /// with children before parents.
    pub files: Vec<SeratoFilePlan>,
    pub neworder: NewOrderChange,
    /// Tracks on other volumes, to sync into that volume's own `_Serato_`.
    pub skipped_tracks: Vec<SkippedTrack>,
    pub warnings: Vec<String>,
}

impl SeratoSyncPlan {
    /// Whether applying would change anything on disk.
    pub fn has_changes(&self) -> bool {
        self.neworder.new_bytes.is_some()
            || self
                .files
                .iter()
                .any(|file| file.action != FileAction::Unchanged)
    }
}

struct Wanted {
    segments: Vec<String>,
    /// `None` for a parent that only has to exist.
    tracks: Option<Vec<String>>,
}

struct Existing {
    filename: String,
    segments: Vec<String>,
}

/// Plans the sync of `desired` crates into `serato_dir`. Nothing is written.
///
/// `home` decides which tracks belong to this folder: those on the boot
/// volume belong to `<home>/Music/_Serato_`, those under `/Volumes/<Name>`
/// to that volume's own folder. Tracks that belong elsewhere are listed in
/// [`SeratoSyncPlan::skipped_tracks`].
/// Whether `dir` is the startup disk's default `_Serato_` folder.
fn is_startup_disk_dir(dir: &Path, home: &Path) -> bool {
    dir == home.join("Music").join(crate::paths::SERATO_DIR_NAME)
}

/// Whether `dir` is the `_Serato_` folder at the root of an external volume
/// (`/Volumes/<Drive>/_Serato_`). Any other target folder, such as a copy
/// used for testing, is treated as the startup disk's: its tracks use
/// startup-disk paths, exactly as `~/Music/_Serato_` would store them.
fn is_volume_dir(dir: &Path) -> bool {
    dir.strip_prefix("/Volumes")
        .is_ok_and(|rest| rest.components().count() == 2)
}

pub fn plan_sync(
    serato_dir: &Path,
    home: &Path,
    root_name: &str,
    desired: &[DesiredCrate],
) -> Result<SeratoSyncPlan, SeratoError> {
    crate_filename(&[root_name])?;
    let mut skipped = Vec::new();
    let mut wanted: Vec<Wanted> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for crate_spec in desired {
        if crate_spec.segments.is_empty() {
            return Err(SeratoError::InvalidName(
                "a vibe needs at least one name segment".to_owned(),
            ));
        }
        let mut full = vec![root_name.to_owned()];
        full.extend(crate_spec.segments.iter().cloned());
        let name = crate_name(&full)?;
        let mut ptrks = Vec::new();
        for track in &crate_spec.tracks {
            let (dir, ptrk) = serato_dir_for(track, home);
            if dir == serato_dir || (is_startup_disk_dir(&dir, home) && !is_volume_dir(serato_dir))
            {
                ptrks.push(ptrk);
            } else {
                skipped.push(SkippedTrack {
                    crate_name: name.clone(),
                    path: track.clone(),
                    serato_dir: dir,
                });
            }
        }
        for depth in 1..full.len() {
            let parent = crate_name(&full[..depth])?;
            index.entry(name_key(&parent)).or_insert_with(|| {
                wanted.push(Wanted {
                    segments: full[..depth].to_vec(),
                    tracks: None,
                });
                wanted.len() - 1
            });
        }
        if let Some(at) = index.get(&name_key(&name)).copied() {
            wanted[at].tracks.get_or_insert_with(Vec::new).extend(ptrks);
        } else {
            index.insert(name_key(&name), wanted.len());
            wanted.push(Wanted {
                segments: full,
                tracks: Some(ptrks),
            });
        }
    }
    // Drop repeats within each crate, keeping first occurrences.
    for slot in &mut wanted {
        if let Some(tracks) = &mut slot.tracks {
            let mut seen = HashSet::new();
            tracks.retain(|ptrk| seen.insert(path_key(ptrk)));
        }
    }

    let dir = subcrates_dir(serato_dir);
    let existing = list_existing(&dir)?;
    let root_key = name_key(root_name);
    let mut files = Vec::new();
    let mut plan_names: Vec<String> = Vec::new();
    let mut wanted_keys = HashSet::new();

    for slot in &wanted {
        let name = crate_name(&slot.segments)?;
        let key = name_key(&name);
        wanted_keys.insert(key.clone());
        let found = existing.get(&key);
        let filename = found.map_or_else(
            || crate_filename(&slot.segments),
            |entry| Ok(entry.filename.clone()),
        )?;
        let on_disk = match found {
            Some(entry) => Some(std::fs::read(dir.join(&entry.filename))?),
            None => None,
        };
        plan_names.push(
            filename
                .strip_suffix(CRATE_EXTENSION)
                .unwrap_or(&filename)
                .to_owned(),
        );
        files.push(plan_file(filename, slot.tracks.as_deref(), on_disk)?);
    }

    let mut deletes: Vec<(&String, &Existing)> = existing
        .iter()
        .filter(|(key, entry)| {
            entry
                .segments
                .first()
                .is_some_and(|first| name_key(first) == root_key)
                && !wanted_keys.contains(*key)
        })
        .collect();
    deletes.sort_by(|a, b| {
        b.1.segments
            .len()
            .cmp(&a.1.segments.len())
            .then_with(|| a.0.cmp(b.0))
    });
    let mut delete_names = Vec::new();
    for (_, entry) in deletes {
        let bytes = std::fs::read(dir.join(&entry.filename))?;
        let tracks = CrateFile::parse(&bytes)
            .and_then(|file| file.track_paths())
            .map_or(0, |paths| paths.len());
        delete_names.push(
            entry
                .filename
                .strip_suffix(CRATE_EXTENSION)
                .unwrap_or(&entry.filename)
                .to_owned(),
        );
        files.push(SeratoFilePlan {
            filename: entry.filename.clone(),
            action: FileAction::Delete { tracks },
            track_count: 0,
            original: Some(bytes),
            new_bytes: None,
        });
    }

    let mut warnings = Vec::new();
    let neworder = plan_neworder(serato_dir, &plan_names, &delete_names, &mut warnings)?;
    if !skipped.is_empty() {
        warnings.push(format!(
            "{} track(s) live on another volume and were left out; sync that volume's _Serato_ folder separately",
            skipped.len()
        ));
    }

    Ok(SeratoSyncPlan {
        serato_dir: serato_dir.to_path_buf(),
        root_crate: root_name.to_owned(),
        files,
        neworder,
        skipped_tracks: skipped,
        warnings,
    })
}

/// Crate files already in `Subcrates`, keyed by [`name_key`].
fn list_existing(dir: &Path) -> Result<HashMap<String, Existing>, SeratoError> {
    let mut found = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(segments) = parse_crate_filename(&filename) {
            let key = name_key(&segments.join(crate::names::SUBCRATE_SEPARATOR));
            found.insert(key, Existing { filename, segments });
        }
    }
    Ok(found)
}

/// Plans one wanted crate against what is on disk.
fn plan_file(
    filename: String,
    tracks: Option<&[String]>,
    on_disk: Option<Vec<u8>>,
) -> Result<SeratoFilePlan, SeratoError> {
    let wrap = |err: SeratoError| match err {
        SeratoError::InvalidFormat(why) => SeratoError::InvalidFormat(format!("{filename}: {why}")),
        other => other,
    };
    let Some(original) = on_disk else {
        let mut file = CrateFile::new();
        let paths = tracks.unwrap_or_default();
        file.set_track_paths(paths);
        return Ok(SeratoFilePlan {
            filename,
            action: FileAction::Create {
                tracks: paths.len(),
            },
            track_count: paths.len(),
            original: None,
            new_bytes: Some(file.to_bytes()?),
        });
    };
    let mut file = CrateFile::parse(&original).map_err(wrap)?;
    let existing_paths = file.track_paths().map_err(wrap)?;
    let Some(tracks) = tracks else {
        // A parent that only has to exist: leave its contents alone.
        return Ok(SeratoFilePlan {
            filename,
            action: FileAction::Unchanged,
            track_count: existing_paths.len(),
            original: Some(original),
            new_bytes: None,
        });
    };
    let existing_records = file.track_records().count();
    let wanted: HashSet<String> = tracks.iter().map(|ptrk| path_key(ptrk)).collect();
    let mut present = HashSet::new();
    let mut kept: Vec<Record> = Vec::new();
    for record in file.track_records() {
        let keep = match record.track_path().map_err(wrap)? {
            Some(path) => {
                let key = path_key(&path);
                wanted.contains(&key) && present.insert(key)
            }
            None => true,
        };
        if keep {
            kept.push(record.clone());
        }
    }
    let kept_count = kept.len();
    let mut added = 0;
    for ptrk in tracks {
        if present.insert(path_key(ptrk)) {
            kept.push(Record::track(ptrk));
            added += 1;
        }
    }
    let removed = existing_records - kept_count;
    let total = kept.len();
    file.set_track_records(kept);
    let bytes = file.to_bytes()?;
    if bytes == original {
        return Ok(SeratoFilePlan {
            filename,
            action: FileAction::Unchanged,
            track_count: total,
            original: Some(original),
            new_bytes: None,
        });
    }
    Ok(SeratoFilePlan {
        filename,
        action: FileAction::Update { added, removed },
        track_count: total,
        original: Some(original),
        new_bytes: Some(bytes),
    })
}

fn plan_neworder(
    serato_dir: &Path,
    names: &[String],
    deleted: &[String],
    warnings: &mut Vec<String>,
) -> Result<NewOrderChange, SeratoError> {
    let path = serato_dir.join(NEWORDER_FILE);
    let none = |exists| NewOrderChange {
        exists,
        added: Vec::new(),
        removed: Vec::new(),
        original: None,
        new_bytes: None,
    };
    let original = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !names.is_empty() {
                warnings.push(
                    "neworder.pref doesn't exist, so Serato will sort the new crates itself"
                        .to_owned(),
                );
            }
            return Ok(none(false));
        }
        Err(err) => return Err(err.into()),
    };
    let mut order = match NewOrder::parse(&original) {
        Ok(order) => order,
        Err(err) => {
            warnings.push(format!("neworder.pref was left alone: {err}"));
            return Ok(NewOrderChange {
                original: Some(original),
                ..none(true)
            });
        }
    };
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for name in names {
        if parse_crate_name(name).is_some() && order.insert(name) {
            added.push(name.clone());
        }
    }
    for name in deleted {
        if order.remove(name) {
            removed.push(name.clone());
        }
    }
    let new_bytes = (!added.is_empty() || !removed.is_empty()).then(|| order.to_bytes());
    Ok(NewOrderChange {
        exists: true,
        added,
        removed,
        original: Some(original),
        new_bytes,
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
