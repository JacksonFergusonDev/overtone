//! Syncing vibes out to DJ software.
//!
//! Every vibe becomes a crate (Serato) or playlist (Rekordbox, Traktor)
//! under a top-level folder Overtone owns, so crates you made yourself are
//! never touched. See `engineering/decisions/vibe-tags.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use overtone_fs::{FsError, ProtectedRoots};
use overtone_library::{Library, SyncStatus};
use overtone_model::{DjApp, Track};
use overtone_sync_rekordbox::{self as rekordbox, PlaylistNode, RekordboxXml, XmlTrack};
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

/// One vibe in the tree under construction.
struct VibeNode {
    name: String,
    /// Tracks assigned directly to this vibe.
    track_ids: Vec<u32>,
    children: Vec<VibeNode>,
}

impl VibeNode {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            track_ids: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The node at `segments` below this one, created as needed.
    fn descend(&mut self, segments: &[String]) -> &mut VibeNode {
        let Some((first, rest)) = segments.split_first() else {
            return self;
        };
        let existing = self.children.iter().position(|c| &c.name == first);
        let index = existing.unwrap_or(self.children.len());
        if existing.is_none() {
            self.children.push(VibeNode::new(first));
        }
        self.children[index].descend(rest)
    }

    /// A vibe with child vibes becomes a folder holding a playlist of its
    /// own direct tracks (only if it has any) followed by its children, so
    /// those tracks stay reachable. A vibe without children is a playlist.
    fn into_playlist_node(self) -> PlaylistNode {
        if self.children.is_empty() {
            return PlaylistNode::Playlist {
                name: self.name,
                track_ids: self.track_ids,
            };
        }
        let mut children = Vec::with_capacity(self.children.len() + 1);
        if !self.track_ids.is_empty() {
            children.push(PlaylistNode::Playlist {
                name: self.name.clone(),
                track_ids: self.track_ids,
            });
        }
        children.extend(self.children.into_iter().map(Self::into_playlist_node));
        PlaylistNode::Folder {
            name: self.name,
            children,
        }
    }
}

fn xml_track(track: &Track, id: u32) -> XmlTrack {
    let path = PathBuf::from(&track.path);
    let name = track.tags.title.clone().or_else(|| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    });
    XmlTrack {
        track_id: id,
        location: path,
        name,
        artist: track.tags.artist.clone(),
        album: track.tags.album.clone(),
        genre: track.tags.genre.clone(),
        kind: track
            .properties
            .format
            .and_then(rekordbox::kind_for)
            .map(str::to_string),
        size_bytes: (track.file_size > 0).then_some(track.file_size),
        total_time_s: track
            .properties
            .duration_ms
            .and_then(|ms| u32::try_from((ms + 500) / 1000).ok()),
        average_bpm: track.tags.bpm,
        tonality: track.tags.key.clone(),
        bit_rate_kbps: track.properties.bitrate_kbps,
        sample_rate_hz: track.properties.sample_rate_hz,
        comments: track.tags.comment.clone(),
        remixer: track.tags.remixer.clone(),
        label: track.tags.label.clone(),
        year: track.tags.year,
        date_added: Some(rekordbox::date_from_unix_secs(
            track.added_at.div_euclid(1000),
        )),
    }
}

/// Builds the Rekordbox document: every present track that is in at least
/// one vibe, and one top-level `Overtone` folder holding the vibe tree.
///
/// TrackIDs are Overtone's own track ids, so they stay the same between
/// exports (an id beyond `u32`, which never happens in practice, leaves the
/// track out). A vibe with child vibes becomes a folder that starts with a
/// playlist of the same name holding its direct tracks (left out when it has
/// none), followed by its child vibes.
pub fn rekordbox_document(library: &Library) -> Result<RekordboxXml, CoreError> {
    let mut root = VibeNode::new(ROOT_FOLDER);
    let mut segments_by_vibe = BTreeMap::new();
    for vibe in library.vibes()? {
        root.descend(vibe.path.segments());
        segments_by_vibe.insert(vibe.id.0, vibe.path.segments().to_vec());
    }

    let mut tracks = Vec::new();
    for track in library
        .tracks()?
        .iter()
        .filter(|t| t.missing_since.is_none())
    {
        let Ok(id) = u32::try_from(track.id.0) else {
            continue;
        };
        let mut in_vibe = false;
        for vibe in &track.vibes {
            if let Some(segments) = segments_by_vibe.get(&vibe.0) {
                root.descend(segments).track_ids.push(id);
                in_vibe = true;
            }
        }
        if in_vibe {
            tracks.push(xml_track(track, id));
        }
    }

    let children = root
        .children
        .into_iter()
        .map(VibeNode::into_playlist_node)
        .collect();
    Ok(RekordboxXml {
        product_version: env!("CARGO_PKG_VERSION").to_string(),
        tracks,
        playlists: vec![PlaylistNode::Folder {
            name: ROOT_FOLDER.to_string(),
            children,
        }],
    })
}

/// One playlist in a Rekordbox export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxPlaylistSummary {
    /// Folders down to the playlist, inside the `Overtone` folder. A vibe's
    /// own tracks sit in a playlist named after it, for example
    /// `["Dubs", "Dubs"]` next to `["Dubs", "Deep"]`.
    pub path: Vec<String>,
    pub track_count: usize,
}

/// What a Rekordbox XML export would write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxExportPlan {
    pub path: PathBuf,
    pub track_count: usize,
    pub playlist_count: usize,
    pub playlists: Vec<RekordboxPlaylistSummary>,
    /// Whether the file at `path` differs from what an export would write.
    /// Always true when it doesn't exist.
    pub changed: bool,
    pub exists: bool,
}

/// Result of writing a Rekordbox XML export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RekordboxExportReport {
    pub path: PathBuf,
    pub track_count: usize,
    pub playlist_count: usize,
    /// False when the file already matched, in which case nothing was written.
    pub changed: bool,
    /// Copy of the file this export replaced.
    pub backup_path: Option<PathBuf>,
    /// Rekordbox doesn't need to be closed for an export, but a running
    /// instance has to import the file again to see it.
    pub rekordbox_running: bool,
}

fn collect_playlists(
    nodes: &[PlaylistNode],
    prefix: &mut Vec<String>,
    out: &mut Vec<RekordboxPlaylistSummary>,
) {
    for node in nodes {
        prefix.push(node.name().to_string());
        match node {
            PlaylistNode::Folder { children, .. } => collect_playlists(children, prefix, out),
            PlaylistNode::Playlist { track_ids, .. } => out.push(RekordboxPlaylistSummary {
                path: prefix.clone(),
                track_count: track_ids.len(),
            }),
        }
        prefix.pop();
    }
}

fn rekordbox_path(path: Option<&Path>) -> Result<PathBuf, CoreError> {
    match path {
        Some(path) => Ok(path.to_path_buf()),
        None => overtone_fs::paths::rekordbox_xml_path().ok_or(CoreError::NoDataDir),
    }
}

/// Reads the file at `path`, if any.
fn read_existing(path: &Path) -> Result<Option<String>, CoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(FsError::io(path, err).into()),
    }
}

struct Prepared {
    path: PathBuf,
    xml: String,
    existing: Option<String>,
    track_count: usize,
    playlists: Vec<RekordboxPlaylistSummary>,
}

impl Prepared {
    fn changed(&self) -> bool {
        self.existing.as_deref() != Some(self.xml.as_str())
    }
}

fn prepare_rekordbox(library: &Library, path: Option<&Path>) -> Result<Prepared, CoreError> {
    let path = rekordbox_path(path)?;
    let doc = rekordbox_document(library)?;
    let mut playlists = Vec::new();
    if let Some(PlaylistNode::Folder { children, .. }) = doc.playlists.first() {
        collect_playlists(children, &mut Vec::new(), &mut playlists);
    }
    Ok(Prepared {
        existing: read_existing(&path)?,
        xml: rekordbox::to_xml(&doc),
        track_count: doc.tracks.len(),
        playlists,
        path,
    })
}

/// Plans a Rekordbox XML export without writing anything. `path` defaults
/// to `rekordbox.xml` in the app data folder.
pub fn plan_rekordbox_export(
    library: &Library,
    path: Option<&Path>,
) -> Result<RekordboxExportPlan, CoreError> {
    let prepared = prepare_rekordbox(library, path)?;
    Ok(RekordboxExportPlan {
        changed: prepared.changed(),
        exists: prepared.existing.is_some(),
        track_count: prepared.track_count,
        playlist_count: prepared.playlists.len(),
        playlists: prepared.playlists,
        path: prepared.path,
    })
}

/// Writes the Rekordbox XML export, backing up a file it replaces into the
/// app data folder and recording the run in the sync history. Rekordbox may
/// be running: the file is only read when you import it.
pub fn export_rekordbox_xml(
    library: &Library,
    path: Option<&Path>,
    roots: &ProtectedRoots,
) -> Result<RekordboxExportReport, CoreError> {
    let backups = overtone_fs::paths::backups_dir()
        .ok_or(CoreError::NoDataDir)?
        .join("rekordbox");
    export_rekordbox_xml_with_backups(library, path, &backups, roots)
}

/// [`export_rekordbox_xml`] with an explicit backup folder.
pub fn export_rekordbox_xml_with_backups(
    library: &Library,
    path: Option<&Path>,
    backups_dir: &Path,
    roots: &ProtectedRoots,
) -> Result<RekordboxExportReport, CoreError> {
    let prepared = prepare_rekordbox(library, path)?;
    let target = prepared.path.display().to_string();
    let run = library.start_sync_run(DjApp::Rekordbox, &target, false)?;
    match write_rekordbox(&prepared, backups_dir, roots) {
        Ok(backup_path) => {
            let report = RekordboxExportReport {
                path: prepared.path.clone(),
                track_count: prepared.track_count,
                playlist_count: prepared.playlists.len(),
                changed: prepared.changed(),
                backup_path,
                rekordbox_running: rekordbox::rekordbox_running(),
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

/// Backs up and replaces the file unless it already matches. Returns the
/// backup's path when one was made.
fn write_rekordbox(
    prepared: &Prepared,
    backups_dir: &Path,
    roots: &ProtectedRoots,
) -> Result<Option<PathBuf>, CoreError> {
    if !prepared.changed() {
        return Ok(None);
    }
    roots.check(&prepared.path)?;
    if let Some(parent) = prepared.path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|err| FsError::io(parent, err))?;
    }
    let backup = if prepared.existing.is_some() {
        Some(overtone_fs::backup_file(
            roots,
            &prepared.path,
            backups_dir,
        )?)
    } else {
        None
    };
    overtone_fs::write_atomic_guarded(roots, &prepared.path, prepared.xml.as_bytes())?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_model::{AudioFormat, AudioProperties, TrackTags};
    use tempfile::TempDir;

    use super::*;

    fn rekordbox_library(dir: &TempDir) -> Library {
        let mut library = Library::open_in_memory().expect("open");
        let music = dir.path().join("music");
        let mut ids = Vec::new();
        for (name, title) in [
            ("a.flac", Some("Alpha")),
            ("b.flac", None),
            ("c.flac", None),
        ] {
            let (id, _) = library
                .upsert_scanned(&ScannedTrack {
                    path: music.join(name).display().to_string(),
                    file_size: 100,
                    properties: AudioProperties {
                        format: Some(AudioFormat::Flac),
                        duration_ms: Some(61_400),
                        ..AudioProperties::default()
                    },
                    tags: TrackTags {
                        title: title.map(str::to_string),
                        bpm: Some(140.0),
                        ..TrackTags::default()
                    },
                    ..ScannedTrack::default()
                })
                .expect("track");
            ids.push(id);
        }
        let dubs = library
            .create_vibe(&"Dubs".parse().expect("vibe"))
            .expect("dubs");
        let deep = library
            .create_vibe(&"Dubs/Deep".parse().expect("vibe"))
            .expect("deep");
        let warm = library
            .create_vibe(&"Warmup".parse().expect("vibe"))
            .expect("warm");
        library.assign_vibe(&[ids[0]], dubs.id).expect("assign");
        library
            .assign_vibe(&[ids[0], ids[1]], deep.id)
            .expect("assign");
        library.assign_vibe(&[ids[1]], warm.id).expect("assign");
        // `c` is in no vibe and `b` goes missing after being assigned.
        library.set_missing(&[ids[1]], Some(1)).expect("missing");
        library
    }

    #[test]
    fn builds_the_rekordbox_tree_without_missing_tracks() {
        let dir = TempDir::new().expect("tempdir");
        let library = rekordbox_library(&dir);
        let doc = rekordbox_document(&library).expect("doc");
        assert_eq!(doc.tracks.len(), 1, "only a is present and in a vibe");
        let track = &doc.tracks[0];
        assert_eq!(track.name.as_deref(), Some("Alpha"));
        assert_eq!(track.kind.as_deref(), Some("FLAC File"));
        assert_eq!(track.total_time_s, Some(61));
        assert_eq!(track.average_bpm, Some(140.0));
        let id = track.track_id;
        let playlist = |name: &str, track_ids: Vec<u32>| PlaylistNode::Playlist {
            name: name.to_string(),
            track_ids,
        };
        assert_eq!(
            doc.playlists,
            vec![PlaylistNode::Folder {
                name: "Overtone".into(),
                children: vec![
                    PlaylistNode::Folder {
                        name: "Dubs".into(),
                        children: vec![playlist("Dubs", vec![id]), playlist("Deep", vec![id])],
                    },
                    playlist("Warmup", vec![]),
                ],
            }]
        );
    }

    #[test]
    fn plans_and_exports_rekordbox_xml_idempotently() {
        let dir = TempDir::new().expect("tempdir");
        let library = rekordbox_library(&dir);
        let out = dir.path().join("out/rekordbox.xml");
        let backups = dir.path().join("backups");
        let roots = ProtectedRoots::none();

        let plan = plan_rekordbox_export(&library, Some(&out)).expect("plan");
        assert!(plan.changed && !plan.exists);
        assert_eq!((plan.track_count, plan.playlist_count), (1, 3));
        assert_eq!(plan.playlists[0].path, ["Dubs", "Dubs"]);
        assert!(!out.exists(), "planning writes nothing");

        let first = export_rekordbox_xml_with_backups(&library, Some(&out), &backups, &roots)
            .expect("export");
        assert!(first.changed);
        assert_eq!(first.backup_path, None);
        let xml = std::fs::read_to_string(&out).expect("read");
        let parsed = rekordbox::parse_xml(&xml).expect("parse");
        assert_eq!(parsed, rekordbox_document(&library).expect("doc"));

        let second = export_rekordbox_xml_with_backups(&library, Some(&out), &backups, &roots)
            .expect("export again");
        assert!(!second.changed);
        assert_eq!(second.backup_path, None);
        assert!(
            !plan_rekordbox_export(&library, Some(&out))
                .expect("plan")
                .changed
        );

        // A new vibe changes the export and backs up the old file.
        library
            .create_vibe(&"Peak".parse().expect("vibe"))
            .expect("vibe");
        let third = export_rekordbox_xml_with_backups(&library, Some(&out), &backups, &roots)
            .expect("export changed");
        assert!(third.changed);
        let backup = third.backup_path.expect("backup of the old file");
        assert!(backup.starts_with(&backups));
        assert_eq!(std::fs::read_to_string(backup).expect("backup"), xml);
        assert_ne!(std::fs::read_to_string(&out).expect("read"), xml);
    }

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
