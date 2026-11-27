//! Applies a [`SeratoSyncPlan`] to disk.
//!
//! Order of operations, so a refusal never leaves a half-changed library:
//!
//! 1. Refuse if Serato DJ is running (unless skipped for tests).
//! 2. Check every target and the backup folder against the protected roots.
//! 3. Check every file still matches what the plan read, so the plan never
//!    overwrites changes made since.
//! 4. Back up everything that will be overwritten or deleted, plus
//!    `neworder.pref`, into one timestamped folder.
//! 5. Write crates and `neworder.pref` through atomic temporary-file renames,
//!    then delete crates that are no longer wanted.

use std::path::{Path, PathBuf};

use overtone_fs::{FsError, ProtectedRoots, backup_file, write_atomic_guarded};
use serde::Serialize;
use ts_rs::TS;

use crate::SeratoError;
use crate::paths::subcrates_dir;
use crate::plan::{FileAction, NEWORDER_FILE, SeratoSyncPlan};
use crate::running::serato_running;

/// Whether [`apply`] looks for a running Serato DJ first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningCheck {
    /// Refuse to write while Serato DJ is running.
    Enforce,
    /// Don't look. For tests and for callers that already checked.
    Skip,
}

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS)]
#[ts(export, rename = "SeratoApplyReport")]
pub struct ApplyReport {
    /// Folder holding the backups, when anything needed backing up.
    pub backup_dir: Option<PathBuf>,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub deleted: Vec<String>,
    pub unchanged: usize,
    pub neworder_written: bool,
}

/// Writes `plan` to disk. See the module docs for the order of steps.
pub fn apply(
    plan: &SeratoSyncPlan,
    roots: &ProtectedRoots,
    backups_dir: &Path,
    running: RunningCheck,
) -> Result<ApplyReport, SeratoError> {
    if running == RunningCheck::Enforce && serato_running() {
        return Err(SeratoError::SeratoRunning);
    }
    let crates_dir = subcrates_dir(&plan.serato_dir);
    let neworder_path = plan.serato_dir.join(NEWORDER_FILE);

    // Everything the plan touches, with the contents it expects to find.
    let mut touched: Vec<(PathBuf, Option<&Vec<u8>>)> = Vec::new();
    for file in &plan.files {
        match file.action {
            FileAction::Unchanged => {}
            FileAction::Create { .. } | FileAction::Update { .. } => {
                if file.new_bytes.is_none() {
                    return Err(SeratoError::StalePlan(format!(
                        "plan for {} has no content; plan again before applying",
                        file.filename
                    )));
                }
                touched.push((crates_dir.join(&file.filename), file.original.as_ref()));
            }
            FileAction::Delete { .. } => {
                touched.push((crates_dir.join(&file.filename), file.original.as_ref()));
            }
        }
    }
    let neworder_changes = plan.neworder.new_bytes.is_some();
    if neworder_changes {
        touched.push((neworder_path.clone(), plan.neworder.original.as_ref()));
    }

    roots.check(backups_dir)?;
    for (path, _) in &touched {
        roots.check(path)?;
    }
    for (path, expected) in &touched {
        let current = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(FsError::io(path, err).into()),
        };
        if current.as_ref() != *expected {
            return Err(SeratoError::StalePlan(format!(
                "{} changed since the plan was made; plan again before applying",
                path.display()
            )));
        }
    }

    let backup_dir = back_up(roots, backups_dir, &touched)?;

    let mut report = ApplyReport {
        backup_dir,
        ..ApplyReport::default()
    };
    if plan
        .files
        .iter()
        .any(|f| matches!(f.action, FileAction::Create { .. }))
    {
        roots.check(&crates_dir)?;
        std::fs::create_dir_all(&crates_dir).map_err(|err| FsError::io(&crates_dir, err))?;
    }
    for file in &plan.files {
        let path = crates_dir.join(&file.filename);
        match (&file.action, &file.new_bytes) {
            (FileAction::Create { .. }, Some(bytes)) => {
                write_atomic_guarded(roots, &path, bytes)?;
                report.created.push(file.filename.clone());
            }
            (FileAction::Update { .. }, Some(bytes)) => {
                write_atomic_guarded(roots, &path, bytes)?;
                report.updated.push(file.filename.clone());
            }
            (FileAction::Unchanged, _) => report.unchanged += 1,
            _ => {}
        }
    }
    if let Some(bytes) = &plan.neworder.new_bytes {
        write_atomic_guarded(roots, &neworder_path, bytes)?;
        report.neworder_written = true;
    }
    for file in &plan.files {
        if matches!(file.action, FileAction::Delete { .. }) {
            let path = crates_dir.join(&file.filename);
            roots.check(&path)?;
            std::fs::remove_file(&path).map_err(|err| FsError::io(&path, err))?;
            report.deleted.push(file.filename.clone());
        }
    }
    Ok(report)
}

/// Copies every existing file in `touched` into one timestamped folder.
/// `overtone_fs::backup_file` makes the folder with the first file; the rest
/// join it so one sync leaves one backup.
fn back_up(
    roots: &ProtectedRoots,
    backups_dir: &Path,
    touched: &[(PathBuf, Option<&Vec<u8>>)],
) -> Result<Option<PathBuf>, SeratoError> {
    let mut folder: Option<PathBuf> = None;
    for (path, _) in touched.iter().filter(|(path, _)| path.exists()) {
        match &folder {
            None => {
                let first = backup_file(roots, path, backups_dir)?;
                folder = first.parent().map(Path::to_path_buf);
            }
            Some(folder) => {
                let name = path.file_name().ok_or_else(|| {
                    FsError::io(path, std::io::Error::other("path has no file name"))
                })?;
                let dest = folder.join(name);
                std::fs::copy(path, &dest).map_err(|err| FsError::io(path, err))?;
            }
        }
    }
    Ok(folder)
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
