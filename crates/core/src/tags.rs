//! Mirroring vibes into files' Comment tags, with preview and undo.
//!
//! The opt-in fallback from `engineering/decisions/vibe-tags.md`: each
//! track's vibes become `#slug` hashtags in its Comment. Overtone records
//! the first original Comment before writing so it can be restored, and
//! refreshes the stored track afterwards so the next scan sees the file as
//! unchanged. Writes go through `overtone_audio::tags::write_comment`; see
//! `engineering/decisions/tag-writing.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use overtone_audio::AudioError;
use overtone_audio::tags::{WriteOutcome, apply_vibes_to_comment, read_metadata, write_comment};
use overtone_fs::ProtectedRoots;
use overtone_library::{COMMENT_FIELD, Library, ScannedTrack};
use overtone_model::{Track, TrackId, UnixMillis, VibeId, VibePath};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;

/// One file's Comment before and after mirroring, for the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct CommentChange {
    pub track_id: TrackId,
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A file that couldn't be written. The file is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct MirrorFailure {
    pub track_id: TrackId,
    pub path: String,
    /// Stable category: a `CoreError::code`, or `stale_preview`,
    /// `write_refused`, `verification_failed`, `unsupported_format`.
    pub code: String,
    pub message: String,
}

/// What a batch of Comment writes did. One file failing never stops the
/// others.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct MirrorReport {
    pub written: Vec<TrackId>,
    /// Already had the requested Comment; not rewritten.
    pub unchanged: Vec<TrackId>,
    pub failed: Vec<MirrorFailure>,
}

/// Previews the Comment each track would get: its current Comment with
/// hashtags for its vibes added and hashtags for the library's other vibes
/// removed. Only tracks whose Comment would change are returned.
///
/// The current Comment is read from the file; if the file can't be read
/// the stored value is used, and the write will report the problem.
pub fn plan_vibe_mirroring(
    library: &Library,
    track_ids: &[TrackId],
) -> Result<Vec<CommentChange>, CoreError> {
    let vibes = library.vibes()?;
    let known: HashSet<String> = vibes.iter().map(|vibe| vibe.path.slug()).collect();
    let paths: HashMap<VibeId, VibePath> =
        vibes.into_iter().map(|vibe| (vibe.id, vibe.path)).collect();
    let mut changes = Vec::new();
    for id in track_ids {
        let track = library.track(*id)?;
        let mut desired: Vec<VibePath> = track
            .vibes
            .iter()
            .filter_map(|vibe| paths.get(vibe).cloned())
            .collect();
        desired.sort_by_key(VibePath::slug);
        let before = read_metadata(Path::new(&track.path))
            .map_or_else(|_| track.tags.comment.clone(), |m| m.tags.comment);
        let after = apply_vibes_to_comment(before.as_deref(), &known, &desired);
        if after != before {
            changes.push(CommentChange {
                track_id: track.id,
                path: track.path,
                before,
                after,
            });
        }
    }
    Ok(changes)
}

/// Writes previewed changes. Before each write the file's Comment must still
/// match the preview's `before`, and the first original is recorded so
/// [`restore_original_comments`] can undo it.
pub fn apply_vibe_mirroring(
    library: &mut Library,
    changes: &[CommentChange],
    roots: &ProtectedRoots,
) -> MirrorReport {
    let mut report = MirrorReport::default();
    for change in changes {
        let result = apply_one(library, change, roots);
        record(&mut report, change.track_id, &change.path, result);
    }
    report
}

/// Writes each track's recorded original Comment back and forgets the
/// snapshot. Tracks without a snapshot count as unchanged.
pub fn restore_original_comments(
    library: &mut Library,
    track_ids: &[TrackId],
    roots: &ProtectedRoots,
) -> MirrorReport {
    let mut report = MirrorReport::default();
    for id in track_ids {
        let path = library
            .track(*id)
            .map_or_else(|_| String::new(), |t| t.path);
        let result = restore_one(library, *id, roots);
        record(&mut report, *id, &path, result);
    }
    report
}

pub(crate) enum Done {
    Written,
    Unchanged,
}

pub(crate) fn record(
    report: &mut MirrorReport,
    id: TrackId,
    path: &str,
    result: Result<Done, Failure>,
) {
    match result {
        Ok(Done::Written) => report.written.push(id),
        Ok(Done::Unchanged) => report.unchanged.push(id),
        Err(failure) => report.failed.push(MirrorFailure {
            track_id: id,
            path: path.to_string(),
            code: failure.code,
            message: failure.message,
        }),
    }
}

pub(crate) struct Failure {
    pub code: String,
    pub message: String,
}

impl From<CoreError> for Failure {
    fn from(err: CoreError) -> Self {
        let code = match &err {
            CoreError::Audio(AudioError::WriteRefused { .. }) => "write_refused",
            CoreError::Audio(AudioError::VerificationFailed { .. }) => "verification_failed",
            CoreError::Audio(AudioError::UnsupportedFormat(_)) => "unsupported_format",
            other => other.code(),
        };
        Self {
            code: code.to_string(),
            message: err.to_string(),
        }
    }
}

pub(crate) fn core<E: Into<CoreError>>(err: E) -> Failure {
    match err.into() {
        // Surface guarded-write errors under their own codes.
        CoreError::Audio(AudioError::Fs(fs)) => Failure::from(CoreError::Fs(fs)),
        other => Failure::from(other),
    }
}

fn apply_one(
    library: &mut Library,
    change: &CommentChange,
    roots: &ProtectedRoots,
) -> Result<Done, Failure> {
    let track = library.track(change.track_id).map_err(core)?;
    if track.path != change.path {
        return Err(Failure {
            code: "stale_preview".into(),
            message: format!("{} moved since the preview", change.path),
        });
    }
    let path = Path::new(&track.path);
    let current = read_metadata(path).map_err(core)?.tags.comment;
    if current != change.before {
        return Err(Failure {
            code: "stale_preview".into(),
            message: format!(
                "the Comment in {} changed since the preview; preview again",
                change.path
            ),
        });
    }
    library
        .record_tag_snapshot(track.id, COMMENT_FIELD, current.as_deref())
        .map_err(core)?;
    let outcome = write_comment(roots, path, change.after.as_deref()).map_err(core)?;
    refresh(library, &track)?;
    Ok(match outcome {
        WriteOutcome::Written(_) => Done::Written,
        WriteOutcome::Unchanged => Done::Unchanged,
    })
}

fn restore_one(
    library: &mut Library,
    id: TrackId,
    roots: &ProtectedRoots,
) -> Result<Done, Failure> {
    let track = library.track(id).map_err(core)?;
    let Some(snapshot) = library.tag_snapshot(id, COMMENT_FIELD).map_err(core)? else {
        return Ok(Done::Unchanged);
    };
    let outcome = write_comment(
        roots,
        Path::new(&track.path),
        snapshot.original_value.as_deref(),
    )
    .map_err(core)?;
    refresh(library, &track)?;
    library
        .clear_tag_snapshot(id, COMMENT_FIELD)
        .map_err(core)?;
    Ok(match outcome {
        WriteOutcome::Written(_) => Done::Written,
        WriteOutcome::Unchanged => Done::Unchanged,
    })
}

/// File modification time as stored in `tracks.modified_at`.
pub fn modified_millis(metadata: &std::fs::Metadata) -> UnixMillis {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// Re-reads the written file and stores its new tags, size, and
/// modification time. The audio hash is kept: Comment writes never change
/// the audio, which the write verified byte for byte.
pub(crate) fn refresh(library: &mut Library, track: &Track) -> Result<(), Failure> {
    let path = Path::new(&track.path);
    let metadata = read_metadata(path).map_err(core)?;
    let stat = std::fs::metadata(path).map_err(|err| Failure {
        code: "io".into(),
        message: format!("couldn't stat {}: {err}", track.path),
    })?;
    library
        .upsert_scanned(&ScannedTrack {
            path: track.path.clone(),
            file_size: stat.len(),
            modified_at: modified_millis(&stat),
            audio_hash: track.audio_hash.clone(),
            properties: metadata.properties,
            tags: metadata.tags,
        })
        .map_err(core)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use overtone_library::UpsertOutcome;
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    fn setup() -> (TempLibrary, TrackId, std::path::PathBuf) {
        let mut temp = TempLibrary::new();
        let signal = Signal {
            seconds: 0.2,
            ..Signal::cd(16_000.0)
        };
        let path = temp.add_fixture("tone", &signal, Fixture::Flac { bits: 16 });
        let metadata = read_metadata(&path).expect("read");
        let stat = std::fs::metadata(&path).expect("stat");
        let (id, _) = temp
            .library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: stat.len(),
                modified_at: modified_millis(&stat),
                audio_hash: Some("audio".into()),
                properties: metadata.properties,
                tags: metadata.tags,
            })
            .expect("insert");
        (temp, id, path)
    }

    fn vibe(library: &Library, path: &str) -> VibeId {
        library
            .create_vibe(&path.parse().expect("vibe"))
            .expect("create")
            .id
    }

    fn rescan_outcome(library: &mut Library, path: &Path) -> UpsertOutcome {
        let stat = std::fs::metadata(path).expect("stat");
        let metadata = read_metadata(path).expect("read");
        library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: stat.len(),
                modified_at: modified_millis(&stat),
                audio_hash: Some("audio".into()),
                properties: metadata.properties,
                tags: metadata.tags,
            })
            .expect("rescan")
            .1
    }

    #[test]
    fn previews_writes_refreshes_and_restores() {
        let (mut temp, id, path) = setup();
        let lib = &mut temp.library;
        let warmup = vibe(lib, "Warmup");
        let deep = vibe(lib, "Dubs/Deep");
        vibe(lib, "Peak");
        lib.assign_vibe(&[id], warmup).expect("assign");
        lib.assign_vibe(&[id], deep).expect("assign");
        let roots = ProtectedRoots::none();

        let plan = plan_vibe_mirroring(lib, &[id]).expect("plan");
        assert_eq!(
            plan,
            vec![CommentChange {
                track_id: id,
                path: path.display().to_string(),
                before: None,
                after: Some("#dubs/deep #warmup".into()),
            }]
        );
        let report = apply_vibe_mirroring(lib, &plan, &roots);
        assert_eq!(report.written, vec![id]);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(
            read_metadata(&path).expect("read").tags.comment.as_deref(),
            Some("#dubs/deep #warmup")
        );
        let stored = lib.track(id).expect("track");
        assert_eq!(stored.tags.comment.as_deref(), Some("#dubs/deep #warmup"));
        assert_eq!(stored.audio_hash.as_deref(), Some("audio"));
        assert_eq!(rescan_outcome(lib, &path), UpsertOutcome::Unchanged);
        assert!(plan_vibe_mirroring(lib, &[id]).expect("plan").is_empty());

        lib.unassign_vibe(&[id], warmup).expect("unassign");
        let plan = plan_vibe_mirroring(lib, &[id]).expect("plan");
        assert_eq!(plan[0].after.as_deref(), Some("#dubs/deep"));
        apply_vibe_mirroring(lib, &plan, &roots);
        let snapshot = lib
            .tag_snapshot(id, COMMENT_FIELD)
            .expect("snapshot")
            .expect("exists");
        assert_eq!(snapshot.original_value, None, "first original kept");

        let report = restore_original_comments(lib, &[id], &roots);
        assert_eq!(report.written, vec![id]);
        assert_eq!(read_metadata(&path).expect("read").tags.comment, None);
        assert_eq!(lib.tag_snapshot(id, COMMENT_FIELD).expect("get"), None);
        let report = restore_original_comments(lib, &[id], &roots);
        assert_eq!(report.unchanged, vec![id]);
    }

    #[test]
    fn stale_previews_and_protected_files_fail_without_stopping_the_batch() {
        let (mut temp, id, path) = setup();
        let lib = &mut temp.library;
        let warmup = vibe(lib, "Warmup");
        lib.assign_vibe(&[id], warmup).expect("assign");
        let plan = plan_vibe_mirroring(lib, &[id]).expect("plan");

        let mut stale = plan[0].clone();
        stale.before = Some("something else".into());
        let protected = ProtectedRoots::new([path.parent().expect("dir").to_path_buf()]);
        let report = apply_vibe_mirroring(lib, &[stale, plan[0].clone()], &protected);
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[0].code, "stale_preview");
        assert_eq!(report.failed[1].code, "protected_path");
        assert_eq!(read_metadata(&path).expect("read").tags.comment, None);

        let report = apply_vibe_mirroring(lib, &plan, &ProtectedRoots::none());
        assert_eq!(report.written, vec![id]);
    }
}
