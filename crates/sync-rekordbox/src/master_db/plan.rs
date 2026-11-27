//! Planning a My Tag sync: comparing the vibes Overtone wants with the
//! `Overtone` category in a database, without writing anything.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use super::{ATTRIBUTE_TAG, MasterDb, normalize_path};
use crate::RekordboxError;

/// One tag Overtone wants: a vibe's full path and its tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredTag {
    pub name: String,
    pub tracks: Vec<PathBuf>,
}

/// An existing tag in the `Overtone` category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingTag {
    pub id: String,
    pub name: String,
    /// Song-tag rows it holds, which are deleted with it.
    pub track_count: usize,
}

/// A track to add to a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagAssignment {
    pub tag: String,
    pub content_id: String,
}

/// A song-tag row to remove from a tag that stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongTagRemoval {
    pub tag: String,
    /// `djmdSongMyTag.ID`.
    pub row_id: String,
    pub content_id: String,
}

/// The state a tag must have after applying, for verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredTagState {
    pub name: String,
    pub content_ids: BTreeSet<String>,
}

/// What a My Tag sync would change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MyTagPlan {
    /// `djmdMyTag.ID` of the existing `Overtone` category.
    pub category_id: Option<String>,
    pub create_category: bool,
    /// New tags, in the order they are appended.
    pub create_tags: Vec<String>,
    /// Tags whose vibe no longer exists, plus duplicates of a tag name.
    pub delete_tags: Vec<ExistingTag>,
    pub add: Vec<TagAssignment>,
    pub remove: Vec<SongTagRemoval>,
    /// Tracks Rekordbox doesn't have. They are reported, never added.
    pub skipped_tracks: Vec<PathBuf>,
    /// Tags that already match.
    pub unchanged_tags: Vec<String>,
    /// Every tag the category must hold afterwards, in desired order.
    pub desired: Vec<DesiredTagState>,
}

impl MyTagPlan {
    /// Whether applying would write nothing.
    pub fn is_empty(&self) -> bool {
        !self.create_category
            && self.create_tags.is_empty()
            && self.delete_tags.is_empty()
            && self.add.is_empty()
            && self.remove.is_empty()
    }

    /// The local USN increments applying makes, following rbox: one per
    /// created category or tag, one per deleted tag row, one per removed
    /// song-tag row, and two per added song-tag row.
    pub fn expected_usn_delta(&self) -> i64 {
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        count(usize::from(self.create_category))
            + count(self.create_tags.len())
            + count(self.delete_tags.len())
            + count(self.remove.len())
            + 2 * count(self.add.len())
    }
}

/// Plans the sync of `desired` tags into the `Overtone` category of `db`.
///
/// Only rows in that category are considered. A tag whose name is not
/// desired is deleted with its song-tag rows; a name that appears more than
/// once keeps its first tag (lowest `Seq`) and loses the others. Tracks are
/// matched by NFC-normalized absolute path, and a track Rekordbox doesn't
/// have is skipped. The category is created only when there is something to
/// put in it.
pub fn plan(db: &MasterDb, desired: &[DesiredTag]) -> Result<MyTagPlan, RekordboxError> {
    let contents = db.contents_by_path()?;
    let category = db.overtone_category()?;
    let existing = db.overtone_tags()?;
    if let Some(folder) = existing.iter().find(|tag| tag.attribute != ATTRIBUTE_TAG) {
        return Err(RekordboxError::Unsupported(format!(
            "the {} My Tag category holds a sub-category, {:?}; Overtone only writes tags there",
            super::CATEGORY_NAME,
            folder.name
        )));
    }
    let tag_ids: Vec<String> = existing.iter().map(|tag| tag.id.clone()).collect();
    let assignments = db.tag_assignments(&tag_ids)?;

    let mut out = MyTagPlan {
        category_id: category.as_ref().map(|c| c.id.clone()),
        create_category: category.is_none() && !desired.is_empty(),
        ..MyTagPlan::default()
    };

    let mut skipped = BTreeSet::new();
    let mut seen_names = HashSet::new();
    for tag in desired {
        if !seen_names.insert(tag.name.as_str()) {
            continue;
        }
        let mut wanted: Vec<String> = Vec::new();
        let mut wanted_set = BTreeSet::new();
        for track in &tag.tracks {
            let found = track
                .to_str()
                .and_then(|path| contents.get(&normalize_path(path)));
            match found {
                Some(ids) => {
                    for id in ids {
                        if wanted_set.insert(id.clone()) {
                            wanted.push(id.clone());
                        }
                    }
                }
                None => {
                    skipped.insert(track.clone());
                }
            }
        }

        if let Some(row) = existing.iter().find(|row| row.name == tag.name) {
            let rows = assignments.get(&row.id).map_or(&[][..], Vec::as_slice);
            let mut kept = HashSet::new();
            let mut changed = false;
            for song in rows {
                // Removes tracks no longer wanted and repeated rows.
                if wanted_set.contains(&song.content_id) && kept.insert(&song.content_id) {
                    continue;
                }
                changed = true;
                out.remove.push(SongTagRemoval {
                    tag: tag.name.clone(),
                    row_id: song.id.clone(),
                    content_id: song.content_id.clone(),
                });
            }
            for id in &wanted {
                if !kept.contains(id) {
                    changed = true;
                    out.add.push(TagAssignment {
                        tag: tag.name.clone(),
                        content_id: id.clone(),
                    });
                }
            }
            if !changed {
                out.unchanged_tags.push(tag.name.clone());
            }
        } else {
            out.create_tags.push(tag.name.clone());
            out.add.extend(wanted.iter().map(|id| TagAssignment {
                tag: tag.name.clone(),
                content_id: id.clone(),
            }));
        }
        out.desired.push(DesiredTagState {
            name: tag.name.clone(),
            content_ids: wanted_set,
        });
    }

    let mut kept_names = HashSet::new();
    for row in &existing {
        let wanted = seen_names.contains(row.name.as_str());
        if wanted && kept_names.insert(row.name.as_str()) {
            continue;
        }
        out.delete_tags.push(ExistingTag {
            id: row.id.clone(),
            name: row.name.clone(),
            track_count: assignments.get(&row.id).map_or(0, Vec::len),
        });
    }
    out.skipped_tracks = skipped.into_iter().collect();
    Ok(out)
}
