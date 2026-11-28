//! Writing the DJ's own and accepted lookup values into audio files, with
//! preview and undo.
//!
//! Overtone keeps values you set or accept in its library first (see
//! `engineering/decisions/metadata-lookup.md`). This module copies them into
//! the files' tags through `overtone_audio::tags::write_fields`, which
//! verifies every write the same way Comment mirroring does. The first
//! original value of each field is recorded in `tag_snapshots` before the
//! first write, so [`restore_original_tags`] can put the file back.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overtone_audio::tags::{
    FieldEdit, WRITABLE_FIELDS, WriteOutcome, read_metadata, reads_as, write_fields,
};
use overtone_fs::ProtectedRoots;
use overtone_library::Library;
use overtone_model::{FieldSource, MetadataField, TrackId, TrackTags};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;
use crate::tags::{Done, Failure, MirrorReport, core, record, refresh};

/// One field a write would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct FieldWrite {
    pub field: MetadataField,
    /// What the file holds now.
    pub before: Option<String>,
    /// What Overtone would write: your value or an accepted lookup value.
    pub after: Option<String>,
}

/// The fields one file would get, for the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct TagWritePlan {
    pub track_id: TrackId,
    pub path: String,
    pub changes: Vec<FieldWrite>,
}

/// Progress after each file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct TagWriteProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
}

fn file_tags(path: &Path, stored: &TrackTags) -> TrackTags {
    read_metadata(path).map_or_else(|_| stored.clone(), |m| m.tags)
}

/// Tracks with at least one value you set or accepted.
pub fn tracks_with_own_values(library: &Library) -> Result<Vec<TrackId>, CoreError> {
    let mut ids = library.tracks_with_field_source(FieldSource::User)?;
    ids.extend(library.tracks_with_field_source(FieldSource::Lookup)?);
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Previews which of your and accepted lookup values differ from what each
/// file holds. Only tracks with something to write are returned; missing
/// files are skipped. Values are compared the way the file would read them,
/// so `2015` and `2015-06-04` count as the same year.
pub fn plan_tag_writes(
    library: &Library,
    track_ids: &[TrackId],
) -> Result<Vec<TagWritePlan>, CoreError> {
    let mut plans = Vec::new();
    for id in track_ids {
        let track = library.track(*id)?;
        if track.missing_since.is_some() {
            continue;
        }
        let tags = file_tags(Path::new(&track.path), &track.tags);
        let changes: Vec<FieldWrite> = library
            .field_values(track.id)?
            .into_iter()
            .filter(|value| {
                value.source != FieldSource::File
                    && WRITABLE_FIELDS.contains(&value.field)
                    && !reads_as(value.field, &tags, value.value.as_deref())
            })
            .map(|value| FieldWrite {
                field: value.field,
                before: value.field.file_value(&tags),
                after: value.value,
            })
            .collect();
        if !changes.is_empty() {
            plans.push(TagWritePlan {
                track_id: track.id,
                path: track.path,
                changes,
            });
        }
    }
    Ok(plans)
}

/// Writes previewed plans. Before each file is written, every field it
/// changes must still read as the preview's `before`; the first original
/// of each field is recorded so [`restore_original_tags`] can undo it. One
/// file failing never stops the others. With `cancel` set, files not yet
/// started are left alone.
pub fn apply_tag_writes(
    library: &mut Library,
    plans: &[TagWritePlan],
    roots: &ProtectedRoots,
    cancel: Option<&Arc<AtomicBool>>,
    on_progress: &dyn Fn(TagWriteProgress),
) -> MirrorReport {
    let mut report = MirrorReport::default();
    let total = u32::try_from(plans.len()).unwrap_or(u32::MAX);
    for (done, plan) in plans.iter().enumerate() {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            break;
        }
        let result = apply_one(library, plan, roots);
        record(&mut report, plan.track_id, &plan.path, result);
        on_progress(TagWriteProgress {
            done: u32::try_from(done + 1).unwrap_or(u32::MAX),
            total,
            track_id: plan.track_id,
        });
    }
    report
}

fn stale(path: &str, what: &str) -> Failure {
    Failure {
        code: "stale_preview".into(),
        message: format!("{what} in {path} changed since the preview; preview again"),
    }
}

fn apply_one(
    library: &mut Library,
    plan: &TagWritePlan,
    roots: &ProtectedRoots,
) -> Result<Done, Failure> {
    let track = library.track(plan.track_id).map_err(core)?;
    if track.path != plan.path {
        return Err(Failure {
            code: "stale_preview".into(),
            message: format!("{} moved since the preview", plan.path),
        });
    }
    let path = Path::new(&track.path);
    let tags = read_metadata(path).map_err(core)?.tags;
    for change in &plan.changes {
        if change.field.file_value(&tags) != change.before {
            return Err(stale(&plan.path, change.field.as_str()));
        }
    }
    // Snapshots this write adds; dropped again if nothing gets written, so
    // only files Overtone changed are offered for restoring.
    let mut recorded = Vec::new();
    for change in &plan.changes {
        if library
            .record_tag_snapshot(track.id, change.field.as_str(), change.before.as_deref())
            .map_err(core)?
        {
            recorded.push(change.field);
        }
    }
    let edits: Vec<FieldEdit> = plan
        .changes
        .iter()
        .map(|change| FieldEdit {
            field: change.field,
            value: change.after.clone(),
        })
        .collect();
    let outcome = write_fields(roots, path, &edits);
    if !matches!(outcome, Ok(WriteOutcome::Written(_))) {
        for field in &recorded {
            library
                .clear_tag_snapshot(track.id, field.as_str())
                .map_err(core)?;
        }
    }
    let outcome = outcome.map_err(core)?;
    refresh(library, &track)?;
    Ok(match outcome {
        WriteOutcome::Written(_) => Done::Written,
        WriteOutcome::Unchanged => Done::Unchanged,
    })
}

/// Tracks whose files hold field values Overtone wrote, with originals
/// recorded.
pub fn tracks_with_original_tags(library: &Library) -> Result<Vec<TrackId>, CoreError> {
    let mut ids = Vec::new();
    for field in WRITABLE_FIELDS {
        ids.extend(library.tracks_with_tag_snapshot(field.as_str())?);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Writes each track's recorded original field values back and forgets the
/// snapshots. Your values stay in Overtone's library. Tracks without
/// snapshots count as unchanged.
pub fn restore_original_tags(
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

fn restore_one(
    library: &mut Library,
    id: TrackId,
    roots: &ProtectedRoots,
) -> Result<Done, Failure> {
    let track = library.track(id).map_err(core)?;
    let mut edits = Vec::new();
    for field in WRITABLE_FIELDS {
        if let Some(snapshot) = library.tag_snapshot(id, field.as_str()).map_err(core)? {
            edits.push(FieldEdit {
                field,
                value: snapshot.original_value,
            });
        }
    }
    if edits.is_empty() {
        return Ok(Done::Unchanged);
    }
    let path = Path::new(&track.path);
    let tags = read_metadata(path).map_err(core)?.tags;
    let outcome = if edits
        .iter()
        .all(|e| reads_as(e.field, &tags, e.value.as_deref()))
    {
        // Already back to the originals (or never changed): nothing to write.
        WriteOutcome::Unchanged
    } else {
        write_fields(roots, path, &edits).map_err(core)?
    };
    refresh(library, &track)?;
    for edit in &edits {
        library
            .clear_tag_snapshot(id, edit.field.as_str())
            .map_err(core)?;
    }
    Ok(match outcome {
        WriteOutcome::Written(_) => Done::Written,
        WriteOutcome::Unchanged => Done::Unchanged,
    })
}

/// Whether `field` is one Overtone can write into files.
pub fn is_writable(field: MetadataField) -> bool {
    WRITABLE_FIELDS.contains(&field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::modified_millis;
    use overtone_library::{ScannedTrack, UpsertOutcome};
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    fn setup() -> (TempLibrary, TrackId, std::path::PathBuf) {
        let mut temp = TempLibrary::new();
        let signal = Signal {
            seconds: 0.2,
            ..Signal::cd(16_000.0)
        };
        let path = temp.add_fixture("tone", &signal, Fixture::Flac { bits: 16 });
        let (id, _) = rescan(&mut temp.library, &path);
        (temp, id, path)
    }

    fn rescan(library: &mut Library, path: &Path) -> (TrackId, UpsertOutcome) {
        let metadata = read_metadata(path).expect("read");
        let stat = std::fs::metadata(path).expect("stat");
        library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: stat.len(),
                modified_at: modified_millis(&stat),
                audio_hash: Some("audio".into()),
                properties: metadata.properties,
                tags: metadata.tags,
            })
            .expect("upsert")
    }

    #[test]
    fn previews_writes_refreshes_and_restores() {
        let (mut temp, id, path) = setup();
        let lib = &mut temp.library;
        let roots = ProtectedRoots::none();
        lib.set_user_field(id, MetadataField::Label, Some("Hospital"))
            .expect("set");
        lib.set_user_field(id, MetadataField::Year, Some("2015"))
            .expect("set");

        let plan = plan_tag_writes(lib, &[id]).expect("plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].changes,
            vec![
                FieldWrite {
                    field: MetadataField::Label,
                    before: None,
                    after: Some("Hospital".into()),
                },
                FieldWrite {
                    field: MetadataField::Year,
                    before: None,
                    after: Some("2015".into()),
                },
            ]
        );
        let report = apply_tag_writes(lib, &plan, &roots, None, &|_| {});
        assert_eq!(report.written, vec![id], "{:?}", report.failed);
        let tags = read_metadata(&path).expect("read").tags;
        assert_eq!(tags.label.as_deref(), Some("Hospital"));
        assert_eq!(tags.year, Some(2015));
        let stored = lib.track(id).expect("track");
        assert_eq!(stored.tags.label.as_deref(), Some("Hospital"));
        assert_eq!(rescan(lib, &path).1, UpsertOutcome::Unchanged);
        assert!(plan_tag_writes(lib, &[id]).expect("plan").is_empty());
        assert_eq!(tracks_with_original_tags(lib).expect("list"), vec![id]);

        // Your value stays yours after it's written.
        let label = lib.field_value(id, MetadataField::Label).expect("value");
        assert_eq!(label.source, FieldSource::User);

        let report = restore_original_tags(lib, &[id], &roots);
        assert_eq!(report.written, vec![id], "{:?}", report.failed);
        let tags = read_metadata(&path).expect("read").tags;
        assert_eq!(tags.label, None);
        assert_eq!(tags.year, None);
        assert!(tracks_with_original_tags(lib).expect("list").is_empty());
        assert_eq!(plan_tag_writes(lib, &[id]).expect("plan").len(), 1);
    }

    #[test]
    fn stale_previews_and_protected_files_fail_without_stopping_the_batch() {
        let (mut temp, id, path) = setup();
        let lib = &mut temp.library;
        lib.set_user_field(id, MetadataField::Title, Some("Mine"))
            .expect("set");
        let plan = plan_tag_writes(lib, &[id]).expect("plan");
        let mut stale = plan[0].clone();
        stale.changes[0].before = Some("something else".into());
        let protected = ProtectedRoots::new([path.parent().expect("dir").to_path_buf()]);
        let report = apply_tag_writes(lib, &[stale, plan[0].clone()], &protected, None, &|_| {});
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[0].code, "stale_preview");
        assert_eq!(report.failed[1].code, "protected_path");
        assert!(
            tracks_with_original_tags(lib).expect("list").is_empty(),
            "a refused write leaves nothing to restore"
        );

        let cancel = Arc::new(AtomicBool::new(true));
        let report = apply_tag_writes(lib, &plan, &ProtectedRoots::none(), Some(&cancel), &|_| {});
        assert!(report.written.is_empty() && report.failed.is_empty());

        let report = apply_tag_writes(lib, &plan, &ProtectedRoots::none(), None, &|_| {});
        assert_eq!(report.written, vec![id]);
    }
}
