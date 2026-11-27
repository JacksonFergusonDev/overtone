//! Syncing vibes out to DJ software.
//!
//! Every vibe becomes a crate (Serato) or playlist (Rekordbox, Traktor)
//! under a top-level folder Overtone owns, so crates you made yourself are
//! never touched. See `engineering/decisions/vibe-tags.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use overtone_fs::ProtectedRoots;
use overtone_library::{Library, SyncStatus};
use overtone_model::DjApp;
use overtone_sync_serato as serato;
use serde::Serialize;
use ts_rs::TS;

use crate::CoreError;

/// Name of the top-level crate or playlist folder Overtone manages.
pub const ROOT_FOLDER: &str = serato::DEFAULT_ROOT_CRATE;

/// One desired crate per vibe, with the vibe's present (not missing) tracks
/// in library path order.
pub fn desired_crates(library: &Library) -> Result<Vec<serato::DesiredCrate>, CoreError> {
    let tracks = library.tracks()?;
    let mut by_vibe: BTreeMap<i64, Vec<PathBuf>> = BTreeMap::new();
    for track in tracks.iter().filter(|t| t.missing_since.is_none()) {
        for vibe in &track.vibes {
            by_vibe
                .entry(vibe.0)
                .or_default()
                .push(PathBuf::from(&track.path));
        }
    }
    Ok(library
        .vibes()?
        .into_iter()
        .map(|vibe| serato::DesiredCrate {
            segments: vibe.path.segments().to_vec(),
            tracks: by_vibe.remove(&vibe.id.0).unwrap_or_default(),
        })
        .collect())
}

/// Plans a Serato sync without writing anything.
///
/// With `serato_dir`, everything is planned into that one `_Serato_`
/// folder (tracks on other volumes are reported as skipped). Without it,
/// each track goes to the `_Serato_` folder of the volume it lives on:
/// `~/Music/_Serato_` for the startup disk, `/Volumes/<Drive>/_Serato_` for
/// external drives.
pub fn plan_serato(
    library: &Library,
    serato_dir: Option<&Path>,
) -> Result<Vec<serato::SeratoSyncPlan>, CoreError> {
    let home = overtone_fs::paths::home_dir().ok_or(CoreError::NoDataDir)?;
    let desired = desired_crates(library)?;
    if let Some(dir) = serato_dir {
        return Ok(vec![serato::plan_sync(dir, &home, ROOT_FOLDER, &desired)?]);
    }

    let mut dirs: BTreeMap<PathBuf, Vec<serato::DesiredCrate>> = BTreeMap::new();
    let boot_dir = home.join("Music").join(serato::SERATO_DIR_NAME);
    dirs.entry(boot_dir).or_default();
    for desired_crate in &desired {
        for track in &desired_crate.tracks {
            let (dir, _) = serato::serato_dir_for(track, &home);
            dirs.entry(dir).or_default();
        }
    }
    let mut plans = Vec::new();
    for dir in dirs.keys() {
        let per_dir: Vec<serato::DesiredCrate> = desired
            .iter()
            .map(|c| serato::DesiredCrate {
                segments: c.segments.clone(),
                tracks: c
                    .tracks
                    .iter()
                    .filter(|t| &serato::serato_dir_for(t, &home).0 == dir)
                    .cloned()
                    .collect(),
            })
            .collect();
        plans.push(serato::plan_sync(dir, &home, ROOT_FOLDER, &per_dir)?);
    }
    Ok(plans)
}

/// Result of applying Serato plans.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SeratoSyncReport {
    pub folders: Vec<SeratoFolderReport>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SeratoFolderReport {
    pub serato_dir: PathBuf,
    pub report: serato::ApplyReport,
}

/// Applies plans from [`plan_serato`], recording each in the sync history.
/// Refuses while Serato DJ is running. Backups go to the app data folder.
pub fn apply_serato(
    library: &Library,
    plans: &[serato::SeratoSyncPlan],
    roots: &ProtectedRoots,
) -> Result<SeratoSyncReport, CoreError> {
    if serato::serato_running() {
        return Err(serato::SeratoError::SeratoRunning.into());
    }
    let backups = overtone_fs::paths::backups_dir()
        .ok_or(CoreError::NoDataDir)?
        .join("serato");
    let mut folders = Vec::new();
    for plan in plans {
        let target = plan.serato_dir.display().to_string();
        let run = library.start_sync_run(DjApp::Serato, &target, false)?;
        match serato::apply(plan, roots, &backups, serato::RunningCheck::Skip) {
            Ok(report) => {
                let summary = serde_json::to_string(&report).ok();
                let backup = report.backup_dir.as_ref().map(|p| p.display().to_string());
                library.finish_sync_run(
                    run,
                    SyncStatus::Succeeded,
                    backup.as_deref(),
                    summary.as_deref(),
                )?;
                folders.push(SeratoFolderReport {
                    serato_dir: plan.serato_dir.clone(),
                    report,
                });
            }
            Err(err) => {
                library.finish_sync_run(run, SyncStatus::Failed, None, Some(&err.to_string()))?;
                return Err(err.into());
            }
        }
    }
    Ok(SeratoSyncReport { folders })
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn plans_and_applies_vibes_into_a_serato_folder() {
        let dir = TempDir::new().expect("tempdir");
        let mut library = Library::open_in_memory().expect("open");
        let music = dir.path().join("music");
        let (a, _) = library
            .upsert_scanned(&ScannedTrack {
                path: music.join("a.flac").display().to_string(),
                ..ScannedTrack::default()
            })
            .expect("a");
        let (b, _) = library
            .upsert_scanned(&ScannedTrack {
                path: music.join("b.flac").display().to_string(),
                ..ScannedTrack::default()
            })
            .expect("b");
        let deep = library
            .create_vibe(&"Dubs/Deep".parse().expect("vibe"))
            .expect("vibe");
        library.assign_vibe(&[a, b], deep.id).expect("assign");
        library.set_missing(&[b], Some(1)).expect("missing");

        let desired = desired_crates(&library).expect("desired");
        assert_eq!(desired.len(), 2, "parent and child vibes");
        assert_eq!(desired[1].tracks.len(), 1, "missing tracks are left out");

        let serato_dir = dir.path().join("_Serato_");
        let plans = plan_serato(&library, Some(&serato_dir)).expect("plan");
        assert_eq!(plans.len(), 1);
        assert!(
            !serato_dir.join("Subcrates").exists(),
            "planning writes nothing"
        );

        let backups = dir.path().join("backups");
        let report = serato::apply(
            &plans[0],
            &ProtectedRoots::none(),
            &backups,
            serato::RunningCheck::Skip,
        )
        .expect("apply");
        assert!(
            report
                .created
                .contains(&"Overtone%%Dubs%%Deep.crate".to_string())
        );
        let bytes =
            std::fs::read(serato_dir.join("Subcrates/Overtone%%Dubs%%Deep.crate")).expect("read");
        let crate_file = serato::CrateFile::parse(&bytes).expect("parse");
        assert_eq!(crate_file.track_paths().expect("paths").len(), 1);
    }
}
