//! Syncing vibes into Rekordbox's `master.db` as My Tags.
//!
//! Overtone never opens the live database: opening a WAL database creates
//! `-wal` and `-shm` files beside it, and a bug could corrupt it in place.
//! Planning reads a copy in a temporary folder. Applying edits a copy too,
//! and the edited copy replaces `master.db` only after it passes
//! verification, with Rekordbox and `rekordboxAgent` closed, the database and
//! its sidecar files backed up, and the live files unchanged since the copy
//! was taken. See "Rekordbox master.db: own My Tag writer over rusqlite with
//! SQLCipher, not rbox" in `engineering/decisions/dj-software-sync.md`.
//!
//! The swap, in order:
//!
//! 1. Copy `master.db` and `master.db-wal` into a temporary folder. The copy
//!    replays the WAL when opened, so changes Rekordbox left there are kept.
//!    `-shm` is not copied: it is an index SQLite rebuilds from the WAL.
//! 2. Plan on the copy. If nothing changes, stop: nothing is written.
//! 3. Back up `master.db`, `-wal`, `-shm`, and `masterPlaylists6.xml`.
//! 4. Apply and verify on the copy, checkpoint its WAL into the main file
//!    with `wal_checkpoint(TRUNCATE)`, close it, and check no `-wal` is left,
//!    so the copy is one self-contained file. Reopen it and check again.
//! 5. Check Rekordbox is still closed and the live files' sizes and
//!    modification times match step 1, else abort as stale.
//! 6. Move the live `master.db-wal` aside and delete `master.db-shm`. They
//!    describe the old file, and SQLite would replay the WAL's pages onto
//!    the new one. Then replace `master.db` atomically (a temporary sibling
//!    renamed over it) and delete the moved-aside WAL. If the replacement
//!    fails, the WAL is moved back.
//!
//! Anything that fails before step 6 leaves the live files untouched.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use overtone_fs::{FsError, ProtectedRoots};
use overtone_library::{Library, SyncStatus};
use overtone_model::DjApp;
use overtone_sync_rekordbox::master_db::{self, DesiredTag, MasterDb, MyTagPlan};
use overtone_sync_rekordbox::{RekordboxError, RunningState};
use serde::Serialize;
use ts_rs::TS;

use crate::CoreError;

/// Where Rekordbox 6 and 7 keep `master.db`, relative to the home folder.
pub const MASTER_DB_RELATIVE: &str = "Library/Pioneer/rekordbox/master.db";

/// Rekordbox's playlist index beside `master.db`. Overtone doesn't change
/// it, but backs it up with the database.
const PLAYLISTS_XML: &str = "masterPlaylists6.xml";

/// The default `master.db`: `~/Library/Pioneer/rekordbox/master.db`.
pub fn default_master_db_path() -> Option<PathBuf> {
    overtone_fs::paths::home_dir().map(|home| home.join(MASTER_DB_RELATIVE))
}

fn master_db_path(explicit: Option<&Path>) -> Result<PathBuf, CoreError> {
    match explicit {
        Some(path) => Ok(path.to_path_buf()),
        None => default_master_db_path().ok_or(CoreError::NoDataDir),
    }
}

/// One tag per vibe, named by its full path such as `Dubs/Deep`, holding
/// the tracks assigned directly to that vibe that aren't missing.
pub fn desired_tags(library: &Library) -> Result<Vec<DesiredTag>, CoreError> {
    Ok(crate::sync::desired_crates(library)?
        .into_iter()
        .map(|desired| DesiredTag {
            name: desired.segments.join("/"),
            tracks: desired.tracks,
        })
        .collect())
}

/// What happens to one My Tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RekordboxTagAction {
    Create,
    Update,
    Unchanged,
    Delete,
}

/// One tag in the `Overtone` My Tag category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxTagChange {
    pub name: String,
    pub action: RekordboxTagAction,
    /// Tracks in the tag afterwards (before, for a deleted tag).
    pub track_count: usize,
    pub added: usize,
    pub removed: usize,
}

/// What a My Tag sync would change in `master.db`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxTagPlan {
    pub db_path: PathBuf,
    /// Whether the `Overtone` category would be created.
    pub create_category: bool,
    pub tags: Vec<RekordboxTagChange>,
    pub added: usize,
    pub removed: usize,
    /// Tracks Rekordbox doesn't have. Import them in Rekordbox to tag them.
    pub skipped_tracks: Vec<PathBuf>,
    /// Whether applying would write anything.
    pub changed: bool,
    /// Applying refuses while either runs.
    pub rekordbox_running: bool,
    pub agent_running: bool,
}

/// Result of a My Tag sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxTagReport {
    pub db_path: PathBuf,
    /// False when the tags already matched, in which case nothing was written.
    pub changed: bool,
    pub created_category: bool,
    pub created_tags: Vec<String>,
    pub deleted_tags: Vec<String>,
    pub added: usize,
    pub removed: usize,
    pub skipped_tracks: Vec<PathBuf>,
    /// Folder holding the replaced `master.db` and its sidecar files.
    pub backup_dir: Option<PathBuf>,
}

fn summarize(path: &Path, plan: &MyTagPlan, running: RunningState) -> RekordboxTagPlan {
    let mut tags: Vec<RekordboxTagChange> = plan
        .desired
        .iter()
        .map(|tag| {
            let added = plan.add.iter().filter(|a| a.tag == tag.name).count();
            let removed = plan.remove.iter().filter(|r| r.tag == tag.name).count();
            let action = if plan.create_tags.contains(&tag.name) {
                RekordboxTagAction::Create
            } else if added + removed > 0 {
                RekordboxTagAction::Update
            } else {
                RekordboxTagAction::Unchanged
            };
            RekordboxTagChange {
                name: tag.name.clone(),
                action,
                track_count: tag.content_ids.len(),
                added,
                removed,
            }
        })
        .collect();
    tags.extend(plan.delete_tags.iter().map(|tag| RekordboxTagChange {
        name: tag.name.clone(),
        action: RekordboxTagAction::Delete,
        track_count: tag.track_count,
        added: 0,
        removed: tag.track_count,
    }));
    RekordboxTagPlan {
        db_path: path.to_path_buf(),
        create_category: plan.create_category,
        tags,
        added: plan.add.len(),
        removed: plan.remove.len(),
        skipped_tracks: plan.skipped_tracks.clone(),
        changed: !plan.is_empty(),
        rekordbox_running: running.app,
        agent_running: running.agent,
    }
}

/// `master.db` plus `suffix`, such as `master.db-wal`.
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Size and modification time of the database and its sidecars, to detect
/// a write that happened while Overtone worked on its copy.
#[derive(Debug, PartialEq, Eq)]
struct Fingerprint(Vec<Option<(u64, SystemTime)>>);

impl Fingerprint {
    fn of(path: &Path) -> Result<Self, CoreError> {
        let mut out = Vec::new();
        for file in [
            path.to_path_buf(),
            sidecar(path, "-wal"),
            sidecar(path, "-shm"),
        ] {
            out.push(match std::fs::metadata(&file) {
                Ok(meta) => Some((
                    meta.len(),
                    meta.modified().map_err(|err| FsError::io(&file, err))?,
                )),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(err) => return Err(FsError::io(&file, err).into()),
            });
        }
        Ok(Self(out))
    }
}

/// Copies `master.db` and its `-wal` (if any) into a new temporary folder,
/// only reading the originals. Returns the folder and the copy's path.
fn working_copy(live: &Path) -> Result<(tempfile::TempDir, PathBuf), CoreError> {
    let dir = tempfile::Builder::new()
        .prefix("overtone-rekordbox-")
        .tempdir()
        .map_err(|err| FsError::io(std::env::temp_dir(), err))?;
    let copy = dir.path().join("master.db");
    copy_writable(live, &copy)?;
    let wal = sidecar(live, "-wal");
    if wal.exists() {
        copy_writable(&wal, &sidecar(&copy, "-wal"))?;
    }
    Ok((dir, copy))
}

fn copy_writable(from: &Path, to: &Path) -> Result<(), CoreError> {
    std::fs::copy(from, to).map_err(|err| FsError::io(from, err))?;
    let mut permissions = std::fs::metadata(to)
        .map_err(|err| FsError::io(to, err))?
        .permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(to, permissions).map_err(|err| FsError::io(to, err))?;
    Ok(())
}

fn require_file(path: &Path) -> Result<(), CoreError> {
    if path.is_file() {
        return Ok(());
    }
    Err(FsError::io(
        path,
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no Rekordbox master.db here; open Rekordbox once to create it",
        ),
    )
    .into())
}

/// Plans a My Tag sync without writing anything. `db` defaults to
/// `~/Library/Pioneer/rekordbox/master.db`, which is only read (copied).
pub fn plan_rekordbox_tags(
    library: &Library,
    db: Option<&Path>,
) -> Result<RekordboxTagPlan, CoreError> {
    let path = master_db_path(db)?;
    require_file(&path)?;
    let desired = desired_tags(library)?;
    let (_dir, copy) = working_copy(&path)?;
    let copy_db = MasterDb::open(&copy)?;
    let plan = master_db::plan(&copy_db, &desired)?;
    copy_db.close()?;
    Ok(summarize(
        &path,
        &plan,
        overtone_sync_rekordbox::running_state(),
    ))
}

/// Writes vibes into `master.db` as My Tags, recording the run in the sync
/// history. Refuses while Rekordbox or `rekordboxAgent` runs. Backups go to
/// `rekordbox-db/` in the app data folder's backups.
pub fn apply_rekordbox_tags(
    library: &Library,
    db: Option<&Path>,
    roots: &ProtectedRoots,
) -> Result<RekordboxTagReport, CoreError> {
    let backups = overtone_fs::paths::backups_dir()
        .ok_or(CoreError::NoDataDir)?
        .join("rekordbox-db");
    apply_rekordbox_tags_with_backups(
        library,
        db,
        &backups,
        roots,
        overtone_sync_rekordbox::running_state,
    )
}

/// [`apply_rekordbox_tags`] with an explicit backup folder and process
/// check, for tests.
pub fn apply_rekordbox_tags_with_backups(
    library: &Library,
    db: Option<&Path>,
    backups_dir: &Path,
    roots: &ProtectedRoots,
    running: impl Fn() -> RunningState,
) -> Result<RekordboxTagReport, CoreError> {
    let path = master_db_path(db)?;
    let run = library.start_sync_run(DjApp::Rekordbox, &path.display().to_string(), false)?;
    match write_tags(library, &path, backups_dir, roots, &running) {
        Ok(report) => {
            let summary = serde_json::to_string(&report).ok();
            let backup = report.backup_dir.as_ref().map(|p| p.display().to_string());
            library.finish_sync_run(
                run,
                SyncStatus::Succeeded,
                backup.as_deref(),
                summary.as_deref(),
            )?;
            Ok(report)
        }
        Err(err) => {
            library.finish_sync_run(run, SyncStatus::Failed, None, Some(&err.to_string()))?;
            Err(err)
        }
    }
}

fn write_tags(
    library: &Library,
    path: &Path,
    backups_dir: &Path,
    roots: &ProtectedRoots,
    running: &dyn Fn() -> RunningState,
) -> Result<RekordboxTagReport, CoreError> {
    let wal = sidecar(path, "-wal");
    let shm = sidecar(path, "-shm");
    for target in [path, &wal, &shm] {
        roots.check(target)?;
    }
    overtone_sync_rekordbox::ensure_closed(running())?;
    require_file(path)?;
    if sidecar(path, "-journal").exists() {
        return Err(RekordboxError::Unsupported(
            "master.db has a rollback journal beside it; open and close Rekordbox, then try again"
                .into(),
        )
        .into());
    }
    let desired = desired_tags(library)?;

    // 1. Copy, and make sure nothing wrote while copying.
    let fingerprint = Fingerprint::of(path)?;
    let (_work, copy) = working_copy(path)?;
    if Fingerprint::of(path)? != fingerprint {
        return Err(RekordboxError::Stale(path.to_path_buf()).into());
    }

    // 2. Plan on the copy.
    let mut db = MasterDb::open(&copy)?;
    let plan = master_db::plan(&db, &desired)?;
    let mut report = RekordboxTagReport {
        db_path: path.to_path_buf(),
        changed: false,
        created_category: false,
        created_tags: Vec::new(),
        deleted_tags: Vec::new(),
        added: 0,
        removed: 0,
        skipped_tracks: plan.skipped_tracks.clone(),
        backup_dir: None,
    };
    if plan.is_empty() {
        db.close()?;
        return Ok(report);
    }

    // 3. Back up everything the swap replaces or deletes.
    let mut sources: Vec<PathBuf> = vec![path.to_path_buf()];
    sources.extend(
        [wal.clone(), shm.clone(), path.with_file_name(PLAYLISTS_XML)]
            .into_iter()
            .filter(|p| p.exists()),
    );
    let refs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let backup_dir = overtone_fs::backup_files(roots, &refs, backups_dir)?;
    if Fingerprint::of(path)? != fingerprint {
        return Err(RekordboxError::Stale(path.to_path_buf()).into());
    }

    // 4. Apply and verify, then fold the WAL into one self-contained file.
    let outcome = master_db::apply(&mut db, &plan)?;
    finish_single_file(db, &copy)?;
    let reopened = MasterDb::open(&copy)?;
    let replan = master_db::plan(&reopened, &desired)?;
    if !replan.is_empty() {
        return Err(RekordboxError::VerifyFailed(
            "the edited copy doesn't match the vibes after reopening".into(),
        )
        .into());
    }
    finish_single_file(reopened, &copy)?;

    // 5. Re-check right before the swap.
    overtone_sync_rekordbox::ensure_closed(running())?;
    if Fingerprint::of(path)? != fingerprint {
        return Err(RekordboxError::Stale(path.to_path_buf()).into());
    }

    // 6. Swap.
    swap(roots, path, &copy)?;

    report.changed = true;
    report.created_category = outcome.created_category;
    report.created_tags = outcome.created_tags;
    report.deleted_tags = outcome.deleted_tags;
    report.added = outcome.added;
    report.removed = outcome.removed;
    report.backup_dir = Some(backup_dir);
    Ok(report)
}

/// Checkpoints and closes `db`, then checks its WAL is gone (or empty), so
/// the main file holds every change.
fn finish_single_file(db: MasterDb, copy: &Path) -> Result<(), CoreError> {
    let journal = db.journal_mode()?;
    if journal == "wal" {
        db.checkpoint()?;
    }
    db.close()?;
    let wal = sidecar(copy, "-wal");
    if std::fs::metadata(&wal).is_ok_and(|meta| meta.len() > 0) {
        return Err(RekordboxError::VerifyFailed(
            "the edited copy still has WAL frames after closing".into(),
        )
        .into());
    }
    Ok(())
}

/// Replaces the live database with `copy` and removes its stale sidecars.
fn swap(roots: &ProtectedRoots, path: &Path, copy: &Path) -> Result<(), CoreError> {
    let wal = sidecar(path, "-wal");
    let shm = sidecar(path, "-shm");
    let parked = sidecar(path, "-wal.overtone-stale");
    for target in [path, &wal, &shm, &parked] {
        roots.check(target)?;
    }
    let had_wal = wal.exists();
    if had_wal {
        std::fs::rename(&wal, &parked).map_err(|err| FsError::io(&wal, err))?;
    }
    let restore = || {
        if had_wal {
            let _ = std::fs::rename(&parked, &wal);
        }
    };
    if shm.exists()
        && let Err(err) = std::fs::remove_file(&shm)
    {
        restore();
        return Err(FsError::io(&shm, err).into());
    }
    let replaced = overtone_fs::replace_guarded(roots, path, |file| {
        let mut source = std::fs::File::open(copy).map_err(|err| FsError::io(copy, err))?;
        std::io::copy(&mut source, file).map_err(|err| FsError::io(copy, err))?;
        Ok(())
    });
    if let Err(err) = replaced {
        restore();
        return Err(err.into());
    }
    if had_wal {
        std::fs::remove_file(&parked).map_err(|err| FsError::io(&parked, err))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_sync_rekordbox::master_db::fixture;
    use tempfile::TempDir;

    use super::*;

    struct Setup {
        dir: TempDir,
        library: Library,
        db: PathBuf,
        tracks: Vec<PathBuf>,
    }

    /// A library with two tracks in `Dubs/Deep` (one of them unknown to
    /// Rekordbox), and a live-looking master.db with WAL frames in a
    /// `Pioneer/rekordbox` folder.
    fn setup() -> Setup {
        let dir = TempDir::new().expect("tempdir");
        let music = dir.path().join("music");
        let a = music.join("a.flac");
        let b = music.join("b.flac");
        let unknown = music.join("unknown.flac");
        let db = fixture::create_live(
            &dir.path().join("Pioneer/rekordbox"),
            &[
                ("11", a.to_str().expect("utf8")),
                ("12", b.to_str().expect("utf8")),
            ],
        )
        .expect("fixture");
        std::fs::write(db.with_file_name(PLAYLISTS_XML), b"<playlists/>").expect("xml");
        let mut library = Library::open_in_memory().expect("library");
        let mut ids = Vec::new();
        for track in [&a, &unknown] {
            let (id, _) = library
                .upsert_scanned(&ScannedTrack {
                    path: track.display().to_string(),
                    ..ScannedTrack::default()
                })
                .expect("track");
            ids.push(id);
        }
        let vibe = library
            .create_vibe(&"Dubs/Deep".parse().expect("vibe"))
            .expect("vibe");
        library.assign_vibe(&ids, vibe.id).expect("assign");
        Setup {
            dir,
            library,
            db,
            tracks: vec![a, b, unknown],
        }
    }

    fn closed() -> RunningState {
        RunningState::default()
    }

    fn apply(setup: &Setup, backups: &Path) -> Result<RekordboxTagReport, CoreError> {
        apply_rekordbox_tags_with_backups(
            &setup.library,
            Some(&setup.db),
            backups,
            &ProtectedRoots::none(),
            closed,
        )
    }

    fn tag_names(db: &Path) -> Vec<String> {
        let dir = TempDir::new().expect("tempdir");
        let copy = dir.path().join("master.db");
        std::fs::copy(db, &copy).expect("copy");
        let db = MasterDb::open(&copy).expect("open");
        db.overtone_tags()
            .expect("tags")
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    #[test]
    fn plans_on_a_copy_without_touching_the_live_files() {
        let setup = setup();
        let folder = setup.db.parent().expect("parent").to_path_buf();
        let before: Vec<_> = std::fs::read_dir(&folder)
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        let fingerprint = Fingerprint::of(&setup.db).expect("fingerprint");
        let plan = plan_rekordbox_tags(&setup.library, Some(&setup.db)).expect("plan");
        assert!(plan.changed && plan.create_category);
        assert_eq!(plan.added, 1);
        assert_eq!(plan.skipped_tracks, [setup.tracks[2].clone()]);
        assert_eq!(plan.tags.len(), 2, "Dubs and Dubs/Deep");
        assert_eq!(
            Fingerprint::of(&setup.db).expect("fingerprint"),
            fingerprint
        );
        let after: Vec<_> = std::fs::read_dir(&folder)
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn swaps_in_the_edited_copy_and_removes_stale_sidecars() {
        let setup = setup();
        let backups = setup.dir.path().join("backups");
        let wal = sidecar(&setup.db, "-wal");
        let shm = sidecar(&setup.db, "-shm");
        let original = std::fs::read(&setup.db).expect("read");
        let original_wal = std::fs::read(&wal).expect("wal");
        assert!(!original_wal.is_empty());

        let report = apply(&setup, &backups).expect("apply");
        assert!(report.changed && report.created_category);
        assert_eq!(report.created_tags, ["Dubs", "Dubs/Deep"]);
        assert_eq!(report.added, 1);
        assert_eq!(report.skipped_tracks.len(), 1);

        assert!(!wal.exists(), "stale WAL removed");
        assert!(!shm.exists(), "stale shm removed");
        assert!(!sidecar(&setup.db, "-wal.overtone-stale").exists());
        let leftovers: Vec<_> = std::fs::read_dir(setup.db.parent().expect("parent"))
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        let mut leftovers = leftovers;
        leftovers.sort();
        assert_eq!(leftovers, ["master.db", PLAYLISTS_XML]);

        // The WAL's committed changes made it into the new file.
        let dir = TempDir::new().expect("tempdir");
        let copy = dir.path().join("master.db");
        std::fs::copy(&setup.db, &copy).expect("copy");
        let db = MasterDb::open(&copy).expect("open new");
        assert_eq!(
            db.my_tags_named(fixture::WAL_ONLY_TAG)
                .expect("query")
                .len(),
            1,
            "the WAL's committed changes made it in"
        );
        assert_eq!(
            db.local_usn().expect("usn"),
            fixture::INITIAL_USN + 1 + 2 + 2
        );
        db.close().expect("close");

        // Backups hold all four files, byte for byte.
        let folder = report.backup_dir.expect("backup");
        assert!(folder.starts_with(&backups));
        assert_eq!(
            std::fs::read(folder.join("master.db")).expect("db"),
            original
        );
        assert_eq!(
            std::fs::read(folder.join("master.db-wal")).expect("wal"),
            original_wal
        );
        assert!(folder.join("master.db-shm").exists());
        assert!(folder.join(PLAYLISTS_XML).exists());
        assert_eq!(
            fixture::credential_row(&copy).expect("secret").as_deref(),
            Some(fixture::CREDENTIAL_SECRET)
        );

        // A second run writes nothing.
        let modified = std::fs::metadata(&setup.db).expect("stat").modified().ok();
        let again = apply(&setup, &backups).expect("again");
        assert!(!again.changed && again.backup_dir.is_none());
        assert_eq!(
            std::fs::metadata(&setup.db).expect("stat").modified().ok(),
            modified
        );
        assert_eq!(tag_names(&setup.db), ["Dubs", "Dubs/Deep"]);
        let runs = setup.library.sync_runs(10).expect("runs");
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn refuses_while_rekordbox_or_its_agent_runs() {
        let setup = setup();
        let backups = setup.dir.path().join("backups");
        let original = std::fs::read(&setup.db).expect("read");
        for (state, code) in [
            (
                RunningState {
                    app: true,
                    agent: false,
                },
                "dj_app_running",
            ),
            (
                RunningState {
                    app: false,
                    agent: true,
                },
                "dj_app_running",
            ),
        ] {
            let err = apply_rekordbox_tags_with_backups(
                &setup.library,
                Some(&setup.db),
                &backups,
                &ProtectedRoots::none(),
                move || state,
            )
            .expect_err("refused");
            assert_eq!(err.code(), code);
            assert_eq!(
                err.to_string().contains("rekordboxAgent"),
                state.agent && !state.app
            );
        }
        assert_eq!(std::fs::read(&setup.db).expect("read"), original);
        assert!(!backups.exists());
        assert!(sidecar(&setup.db, "-wal").exists());
    }

    #[test]
    fn aborts_when_the_live_file_changes_before_the_swap() {
        let setup = setup();
        let backups = setup.dir.path().join("backups");
        let db = setup.db.clone();
        let calls = std::cell::Cell::new(0);
        // The second running check happens right before the swap; touch the
        // live WAL then, as Rekordbox would if it started in between.
        let err = apply_rekordbox_tags_with_backups(
            &setup.library,
            Some(&setup.db),
            &backups,
            &ProtectedRoots::none(),
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    let wal = sidecar(&db, "-wal");
                    let mut bytes = std::fs::read(&wal).expect("wal");
                    bytes.extend_from_slice(&[0; 16]);
                    std::fs::write(&wal, bytes).expect("touch");
                }
                RunningState::default()
            },
        )
        .expect_err("stale");
        assert_eq!(err.code(), "rekordbox_db_changed");
        assert!(sidecar(&setup.db, "-wal").exists(), "live WAL untouched");
        assert!(sidecar(&setup.db, "-shm").exists());
        assert!(tag_names(&setup.db).is_empty(), "live db unchanged");
        let run = &setup.library.sync_runs(1).expect("runs")[0];
        assert_eq!(run.status, SyncStatus::Failed);
    }

    #[test]
    fn refuses_protected_targets_before_reading() {
        let setup = setup();
        let backups = setup.dir.path().join("backups");
        let roots = ProtectedRoots::new([setup.dir.path().join("Pioneer")]);
        let err = apply_rekordbox_tags_with_backups(
            &setup.library,
            Some(&setup.db),
            &backups,
            &roots,
            closed,
        )
        .expect_err("protected");
        assert_eq!(err.code(), "protected_path");
        assert!(!backups.exists());
    }

    #[test]
    fn default_path_is_under_pioneer() {
        if let Some(path) = default_master_db_path() {
            assert!(path.ends_with("Library/Pioneer/rekordbox/master.db"));
        }
    }
}
