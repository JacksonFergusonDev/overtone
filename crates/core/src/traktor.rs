//! Syncing vibes into Traktor's `collection.nml` as playlists under a
//! top-level `Overtone` folder.
//!
//! The vibe tree is the same one the Rekordbox export uses (see
//! [`crate::sync::rekordbox_document`]): a vibe with child vibes becomes a
//! folder that starts with a same-named playlist of its own tracks (left out
//! when it has none), followed by its children. Tracks are referred to by
//! the key of their entry in Traktor's collection; tracks Traktor doesn't
//! know are skipped and reported, never added. With no vibes at all, an
//! existing Overtone folder is removed.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use overtone_fs::{FsError, ProtectedRoots};
use overtone_library::{Library, SyncStatus};
use overtone_model::{DjApp, TrackId};
use overtone_sync_rekordbox::PlaylistNode as VibeTreeNode;
use overtone_sync_traktor::{self as traktor, CollectionScan, PlaylistNode, TraktorError};
use serde::Serialize;
use ts_rs::TS;

use crate::CoreError;

/// One playlist Overtone keeps in Traktor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct TraktorPlaylistSummary {
    /// Folders down to the playlist, inside the `Overtone` folder. A vibe's
    /// own tracks sit in a playlist named after it, for example
    /// `["Dubs", "Dubs"]` next to `["Dubs", "Deep"]`.
    pub path: Vec<String>,
    pub track_count: usize,
}

/// A track in a vibe that can't go into a Traktor playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct TraktorSkippedTrack {
    pub track_id: TrackId,
    pub path: String,
    pub reason: String,
}

/// What a Traktor sync would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct TraktorSyncPlan {
    pub collection: PathBuf,
    /// The collection's NML version: `19` for Traktor Pro 3, `20` for 4.
    pub traktor_version: Option<String>,
    pub playlists: Vec<TraktorPlaylistSummary>,
    /// Distinct tracks across all playlists.
    pub track_count: usize,
    /// Tracks in a vibe that aren't in Traktor's collection. Add them to
    /// Traktor first, then sync again.
    pub skipped_tracks: Vec<TraktorSkippedTrack>,
    /// Whether the Overtone folder in the collection differs from what a
    /// sync would write.
    pub changed: bool,
    pub has_existing_folder: bool,
}

/// Result of a Traktor sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct TraktorSyncReport {
    pub collection: PathBuf,
    pub playlist_count: usize,
    pub track_count: usize,
    pub skipped_tracks: Vec<TraktorSkippedTrack>,
    /// False when the collection already matched, in which case nothing
    /// was written.
    pub changed: bool,
    /// Copy of the collection made before it was changed.
    pub backup_path: Option<PathBuf>,
}

/// The collection file to sync: `explicit`, or the only
/// `Native Instruments/Traktor */collection.nml` under `documents`.
pub fn resolve_collection(
    explicit: Option<&Path>,
    documents: Option<&Path>,
) -> Result<PathBuf, CoreError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let documents = documents.ok_or(CoreError::NoDataDir)?;
    let mut found = traktor::find_collections(documents);
    match found.len() {
        0 => Err(TraktorError::NoCollection(documents.join("Native Instruments")).into()),
        1 => Ok(found.remove(0)),
        _ => Err(TraktorError::SeveralCollections(found).into()),
    }
}

struct Prepared {
    path: PathBuf,
    bytes: Vec<u8>,
    scan: CollectionScan,
    /// The folder to write, or `None` to have none.
    folder: Option<String>,
    plan: TraktorSyncPlan,
}

fn convert(
    nodes: Vec<VibeTreeNode>,
    keys: &BTreeMap<u32, String>,
    prefix: &mut Vec<String>,
    summaries: &mut Vec<TraktorPlaylistSummary>,
) -> Vec<PlaylistNode> {
    nodes
        .into_iter()
        .map(|node| match node {
            VibeTreeNode::Folder { name, children } => {
                prefix.push(name.clone());
                let children = convert(children, keys, prefix, summaries);
                prefix.pop();
                PlaylistNode::Folder { name, children }
            }
            VibeTreeNode::Playlist { name, track_ids } => {
                let mut seen = HashSet::new();
                let keys: Vec<String> = track_ids
                    .iter()
                    .filter_map(|id| keys.get(id))
                    .filter(|key| seen.insert(key.as_str()))
                    .cloned()
                    .collect();
                let mut path = prefix.clone();
                path.push(name.clone());
                summaries.push(TraktorPlaylistSummary {
                    path,
                    track_count: keys.len(),
                });
                PlaylistNode::Playlist { name, keys }
            }
        })
        .collect()
}

fn prepare(library: &Library, collection: Option<&Path>) -> Result<Prepared, CoreError> {
    let documents = overtone_fs::paths::documents_dir();
    let path = resolve_collection(collection, documents.as_deref())?;
    let bytes = std::fs::read(&path).map_err(|err| FsError::io(&path, err))?;
    let scan = traktor::scan(&bytes)?;

    let doc = crate::sync::rekordbox_document(library)?;
    let mut keys = BTreeMap::new();
    let mut skipped_tracks = Vec::new();
    for track in &doc.tracks {
        let found = traktor::location_for(&track.location, &scan.startup_volume)
            .and_then(|location| scan.lookup(&location.key()).map(str::to_string));
        match found {
            Some(key) => {
                keys.insert(track.track_id, key);
            }
            None => skipped_tracks.push(TraktorSkippedTrack {
                track_id: TrackId(i64::from(track.track_id)),
                path: track.location.display().to_string(),
                reason: "not in Traktor's collection".to_string(),
            }),
        }
    }
    let track_count = keys.values().collect::<HashSet<_>>().len();

    let tree = match doc.playlists.into_iter().next() {
        Some(VibeTreeNode::Folder { children, .. }) => children,
        _ => Vec::new(),
    };
    let has_vibes = !tree.is_empty();
    let mut playlists = Vec::new();
    let nodes = convert(tree, &keys, &mut Vec::new(), &mut playlists);
    let folder = has_vibes.then(|| traktor::render_overtone_folder(&nodes, scan.newline));

    let existing = scan.overtone_bytes(&bytes);
    let changed = existing != folder.as_deref().map(str::as_bytes);
    let plan = TraktorSyncPlan {
        collection: path.clone(),
        traktor_version: scan.version.clone(),
        playlists,
        track_count,
        skipped_tracks,
        changed,
        has_existing_folder: existing.is_some(),
    };
    Ok(Prepared {
        path,
        bytes,
        scan,
        folder,
        plan,
    })
}

/// Plans a Traktor sync without writing anything. Without `collection`,
/// uses the only `collection.nml` under `~/Documents/Native Instruments`.
pub fn plan_traktor_sync(
    library: &Library,
    collection: Option<&Path>,
) -> Result<TraktorSyncPlan, CoreError> {
    Ok(prepare(library, collection)?.plan)
}

/// Writes the Overtone folder into `collection.nml`. Refuses while Traktor
/// is running, backs the file up into the app data folder before changing
/// it, and records the run in the sync history. Writes nothing when the
/// collection already matches.
pub fn apply_traktor_sync(
    library: &Library,
    collection: Option<&Path>,
    roots: &ProtectedRoots,
) -> Result<TraktorSyncReport, CoreError> {
    let backups = overtone_fs::paths::backups_dir()
        .ok_or(CoreError::NoDataDir)?
        .join("traktor");
    apply_traktor_sync_with_backups(library, collection, &backups, roots)
}

/// [`apply_traktor_sync`] with an explicit backup folder.
pub fn apply_traktor_sync_with_backups(
    library: &Library,
    collection: Option<&Path>,
    backups_dir: &Path,
    roots: &ProtectedRoots,
) -> Result<TraktorSyncReport, CoreError> {
    apply_checked(
        library,
        collection,
        backups_dir,
        roots,
        traktor::traktor_running(),
    )
}

/// Refuses to sync while Traktor runs: it rewrites `collection.nml` when it
/// quits, which would undo the sync.
fn ensure_closed(traktor_running: bool) -> Result<(), CoreError> {
    if traktor_running {
        return Err(TraktorError::AppRunning.into());
    }
    Ok(())
}

fn apply_checked(
    library: &Library,
    collection: Option<&Path>,
    backups_dir: &Path,
    roots: &ProtectedRoots,
    traktor_running: bool,
) -> Result<TraktorSyncReport, CoreError> {
    ensure_closed(traktor_running)?;
    let prepared = prepare(library, collection)?;
    let target = prepared.path.display().to_string();
    let run = library.start_sync_run(DjApp::Traktor, &target, false)?;
    match write(&prepared, backups_dir, roots) {
        Ok(backup_path) => {
            let plan = &prepared.plan;
            let report = TraktorSyncReport {
                collection: prepared.path.clone(),
                playlist_count: plan.playlists.len(),
                track_count: plan.track_count,
                skipped_tracks: plan.skipped_tracks.clone(),
                changed: plan.changed,
                backup_path,
            };
            let summary = serde_json::to_string(&report).ok();
            let backup = report.backup_path.as_ref().map(|p| p.display().to_string());
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

/// Patches, verifies, backs up, and replaces the collection unless it
/// already matches. Returns the backup's path when one was made.
fn write(
    prepared: &Prepared,
    backups_dir: &Path,
    roots: &ProtectedRoots,
) -> Result<Option<PathBuf>, CoreError> {
    if !prepared.plan.changed {
        return Ok(None);
    }
    let folder = prepared.folder.as_deref();
    let patched = traktor::patch(&prepared.bytes, folder)?;
    traktor::verify(&prepared.bytes, &patched, folder)?;
    debug_assert_eq!(
        prepared.scan.overtone_span.is_some(),
        prepared.plan.has_existing_folder
    );
    roots.check(&prepared.path)?;
    let current = std::fs::read(&prepared.path).map_err(|err| FsError::io(&prepared.path, err))?;
    if current != prepared.bytes {
        return Err(TraktorError::ChangedDuringSync.into());
    }
    let backup = overtone_fs::backup_file(roots, &prepared.path, backups_dir)?;
    overtone_fs::write_atomic_guarded(roots, &prepared.path, &patched)?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use tempfile::TempDir;

    use super::*;

    /// A Traktor Pro 4 style collection whose entries point at `music`.
    fn collection(music: &Path, files: &[&str]) -> String {
        let location = |file: &str| {
            traktor::location_for(&music.join(file), traktor::DEFAULT_STARTUP_VOLUME)
                .expect("location")
        };
        let mut nml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n<NML VERSION=\"20\"><HEAD COMPANY=\"www.native-instruments.com\" PROGRAM=\"Traktor Pro 4\"></HEAD>\n",
        );
        nml.push_str(&format!("<COLLECTION ENTRIES=\"{}\">", files.len()));
        for file in files {
            let l = location(file);
            nml.push_str(&format!(
                "<ENTRY TITLE=\"{file}\"><LOCATION DIR=\"{}\" FILE=\"{}\" VOLUME=\"{}\" VOLUMEID=\"{}\"></LOCATION>\n<INFO KEY_LYRICS=\"a&#xA;b\"></INFO>\n</ENTRY>\n",
                l.dir, l.file, l.volume, l.volume
            ));
        }
        nml.push_str("</COLLECTION>\n<PLAYLISTS><NODE TYPE=\"FOLDER\" NAME=\"$ROOT\"><SUBNODES COUNT=\"1\"><NODE TYPE=\"PLAYLIST\" NAME=\"Mine\"><PLAYLIST ENTRIES=\"0\" TYPE=\"LIST\" UUID=\"0a1b2c3d4e5f60718293a4b5c6d7e8f9\"></PLAYLIST>\n</NODE>\n</SUBNODES>\n</NODE>\n</PLAYLISTS>\n<INDEXING></INDEXING>\n</NML>\n");
        nml
    }

    struct Setup {
        dir: TempDir,
        library: Library,
        nml: PathBuf,
        ids: Vec<TrackId>,
    }

    /// `a` (in Dubs and Dubs/Deep), `b\u{301}` NFD on disk but NFC in the
    /// library (Dubs/Deep), `c` missing (Warmup), `d` not in Traktor's
    /// collection (Warmup).
    fn setup() -> Setup {
        let dir = TempDir::new().expect("tempdir");
        let music = dir.path().join("music");
        let nml = dir.path().join("Traktor 4.1.0/collection.nml");
        std::fs::create_dir_all(nml.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &nml,
            collection(&music, &["a.mp3", "Cafe\u{301}.flac", "c.wav"]),
        )
        .expect("write");

        let mut library = Library::open_in_memory().expect("open");
        let mut ids = Vec::new();
        for name in ["a.mp3", "Caf\u{e9}.flac", "c.wav", "d.aiff"] {
            let (id, _) = library
                .upsert_scanned(&ScannedTrack {
                    path: music.join(name).display().to_string(),
                    file_size: 100,
                    ..ScannedTrack::default()
                })
                .expect("track");
            ids.push(id);
        }
        let vibe = |path: &str| {
            library
                .create_vibe(&path.parse().expect("vibe"))
                .expect("vibe")
        };
        let dubs = vibe("Dubs");
        let deep = vibe("Dubs/Deep");
        let warm = vibe("Warmup");
        library.assign_vibe(&[ids[0]], dubs.id).expect("assign");
        library
            .assign_vibe(&[ids[0], ids[1]], deep.id)
            .expect("assign");
        library
            .assign_vibe(&[ids[2], ids[3]], warm.id)
            .expect("assign");
        library.set_missing(&[ids[2]], Some(1)).expect("missing");
        Setup {
            dir,
            library,
            nml,
            ids,
        }
    }

    #[test]
    fn plans_nested_vibes_and_reports_skipped_tracks() {
        let s = setup();
        let before = std::fs::read(&s.nml).expect("read");
        let plan = plan_traktor_sync(&s.library, Some(&s.nml)).expect("plan");
        assert_eq!(plan.traktor_version.as_deref(), Some("20"));
        assert!(plan.changed && !plan.has_existing_folder);
        let paths: Vec<_> = plan
            .playlists
            .iter()
            .map(|p| (p.path.join("/"), p.track_count))
            .collect();
        assert_eq!(
            paths,
            [
                ("Dubs/Dubs".to_string(), 1),
                ("Dubs/Deep".to_string(), 2),
                ("Warmup".to_string(), 0)
            ]
        );
        assert_eq!(plan.track_count, 2);
        assert_eq!(
            plan.skipped_tracks.len(),
            1,
            "the missing track isn't reported"
        );
        assert_eq!(plan.skipped_tracks[0].track_id, s.ids[3]);
        assert!(plan.skipped_tracks[0].path.ends_with("d.aiff"));
        assert_eq!(
            std::fs::read(&s.nml).expect("read"),
            before,
            "planning writes nothing"
        );
    }

    #[test]
    fn applies_once_then_writes_nothing() {
        let s = setup();
        let backups = s.dir.path().join("backups");
        let roots = ProtectedRoots::none();
        let original = std::fs::read(&s.nml).expect("read");

        let first =
            apply_checked(&s.library, Some(&s.nml), &backups, &roots, false).expect("apply");
        assert!(first.changed);
        assert_eq!(first.playlist_count, 3);
        let backup = first.backup_path.expect("backup");
        assert!(backup.starts_with(&backups));
        assert_eq!(std::fs::read(&backup).expect("backup"), original);

        let written = std::fs::read_to_string(&s.nml).expect("read");
        assert!(written.contains("<NODE TYPE=\"FOLDER\" NAME=\"Overtone\"><SUBNODES COUNT=\"2\">"));
        assert!(written.contains("<SUBNODES COUNT=\"2\"><NODE TYPE=\"PLAYLIST\" NAME=\"Mine\">"));
        // The collection's NFD spelling is written, not the library's NFC one.
        assert!(written.contains("Cafe\u{301}.flac\"></PRIMARYKEY>"));
        assert!(!written.contains("Caf\u{e9}"));
        let scan = traktor::scan(written.as_bytes()).expect("scan");
        assert_eq!(scan.root_child_count, 2);

        let second =
            apply_checked(&s.library, Some(&s.nml), &backups, &roots, false).expect("apply again");
        assert!(!second.changed);
        assert_eq!(second.backup_path, None);
        assert_eq!(std::fs::read_to_string(&s.nml).expect("read"), written);
        assert!(
            !plan_traktor_sync(&s.library, Some(&s.nml))
                .expect("plan")
                .changed
        );

        let runs = s.library.sync_runs(10).expect("runs");
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|r| r.app == DjApp::Traktor));
    }

    #[test]
    fn removes_the_folder_when_no_vibes_are_left() {
        let s = setup();
        let backups = s.dir.path().join("backups");
        let roots = ProtectedRoots::none();
        let original = std::fs::read(&s.nml).expect("read");
        apply_checked(&s.library, Some(&s.nml), &backups, &roots, false).expect("apply");
        for vibe in s.library.vibes().expect("vibes").into_iter().rev() {
            s.library.delete_vibe(vibe.id).expect("delete");
        }
        let plan = plan_traktor_sync(&s.library, Some(&s.nml)).expect("plan");
        assert!(plan.changed && plan.has_existing_folder && plan.playlists.is_empty());
        apply_checked(&s.library, Some(&s.nml), &backups, &roots, false).expect("remove");
        assert_eq!(std::fs::read(&s.nml).expect("read"), original);
    }

    #[test]
    fn refuses_while_traktor_runs_and_inside_protected_roots() {
        let s = setup();
        let backups = s.dir.path().join("backups");
        let original = std::fs::read(&s.nml).expect("read");
        let err = apply_checked(
            &s.library,
            Some(&s.nml),
            &backups,
            &ProtectedRoots::none(),
            true,
        )
        .expect_err("running");
        assert_eq!(err.code(), "dj_app_running");

        let roots = ProtectedRoots::new([s.dir.path().join("Traktor 4.1.0")]);
        let err = apply_checked(&s.library, Some(&s.nml), &backups, &roots, false)
            .expect_err("protected");
        assert_eq!(err.code(), "protected_path");
        assert_eq!(std::fs::read(&s.nml).expect("read"), original);
        assert!(!backups.exists());
    }

    #[test]
    fn resolves_the_collection_path() {
        let dir = TempDir::new().expect("tempdir");
        let explicit = dir.path().join("x.nml");
        assert_eq!(
            resolve_collection(Some(&explicit), None).expect("explicit"),
            explicit
        );
        let err = resolve_collection(None, Some(dir.path())).expect_err("none");
        assert_eq!(err.code(), "traktor_collection");

        let ni = dir.path().join("Native Instruments");
        for version in ["Traktor 3.11.1", "Traktor 4.1.0"] {
            std::fs::create_dir_all(ni.join(version)).expect("mkdir");
            std::fs::write(ni.join(version).join("collection.nml"), b"").expect("write");
            if version.starts_with("Traktor 3") {
                assert_eq!(
                    resolve_collection(None, Some(dir.path())).expect("one"),
                    ni.join(version).join("collection.nml")
                );
            }
        }
        let err = resolve_collection(None, Some(dir.path())).expect_err("several");
        assert_eq!(err.code(), "traktor_collection");
    }
}
