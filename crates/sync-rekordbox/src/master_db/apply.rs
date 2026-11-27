//! Applying a [`MyTagPlan`] in one transaction, with the bookkeeping ported
//! from `rbox-masterdb` 0.3.1, and verifying the result.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::SystemTime;

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use super::{
    ATTRIBUTE_CATEGORY, ATTRIBUTE_TAG, CATEGORY_NAME, LOCAL_USN_ROW, MasterDb, MyTagPlan,
    ROOT_PARENT,
};
use crate::RekordboxError;

/// What [`apply`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub category_id: Option<String>,
    pub created_category: bool,
    pub created_tags: Vec<String>,
    pub deleted_tags: Vec<String>,
    pub added: usize,
    pub removed: usize,
    /// `localUpdateCount` before and after.
    pub usn_before: i64,
    pub usn_after: i64,
}

/// Rows outside the `Overtone` category, captured before a write so
/// [`verify`] can prove they are unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct OutsideRows {
    my_tags: Vec<Vec<Value>>,
    song_tags: Vec<Vec<Value>>,
    content_count: i64,
}

impl OutsideRows {
    pub fn capture(db: &MasterDb) -> Result<Self, RekordboxError> {
        let (category, tags) = overtone_ids(db)?;
        let mut owned: BTreeSet<String> = tags.iter().cloned().collect();
        owned.extend(category);
        let my_tags = all_rows(db, "djmdMyTag")?
            .into_iter()
            .filter(|row| !matches!(&row[0], Value::Text(id) if owned.contains(id)))
            .collect();
        let tag_set: BTreeSet<&String> = tags.iter().collect();
        let column = song_my_tag_column(db)?;
        let song_tags = all_rows(db, "djmdSongMyTag")?
            .into_iter()
            .filter(|row| !matches!(&row[column], Value::Text(id) if tag_set.contains(id)))
            .collect();
        Ok(Self {
            my_tags,
            song_tags,
            content_count: db.content_count()?,
        })
    }
}

fn overtone_ids(db: &MasterDb) -> Result<(Option<String>, Vec<String>), RekordboxError> {
    let category = db.overtone_category()?.map(|c| c.id);
    let tags = match &category {
        Some(id) => db.children(id)?.into_iter().map(|t| t.id).collect(),
        None => Vec::new(),
    };
    Ok((category, tags))
}

fn all_rows(db: &MasterDb, table: &str) -> Result<Vec<Vec<Value>>, RekordboxError> {
    let mut stmt = db
        .conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY ID"))?;
    let columns = stmt.column_count();
    let rows = stmt.query_map([], |row| {
        (0..columns).map(|i| row.get::<_, Value>(i)).collect()
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn song_my_tag_column(db: &MasterDb) -> Result<usize, RekordboxError> {
    let stmt = db.conn.prepare("SELECT * FROM djmdSongMyTag LIMIT 0")?;
    stmt.column_index("MyTagID").map_err(Into::into)
}

/// Applies `plan` to `db` in one transaction, then [`verify`]s the result.
///
/// The plan must come from [`super::plan`] on this same database with no
/// writes in between; a changed `Overtone` category is refused. If anything
/// fails before the commit, nothing is written.
pub fn apply(db: &mut MasterDb, plan: &MyTagPlan) -> Result<ApplyOutcome, RekordboxError> {
    let current = db.overtone_category()?.map(|c| c.id);
    if current != plan.category_id {
        return Err(RekordboxError::Unsupported(
            "the plan is out of date: the Overtone My Tag category changed".into(),
        ));
    }
    let before = OutsideRows::capture(db)?;
    let usn_before = db.local_usn()?;
    let now = rekordbox_timestamp(SystemTime::now());

    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let category_id = match &plan.category_id {
        Some(id) => Some(id.clone()),
        None if plan.create_category => Some(insert_my_tag(
            &tx,
            CATEGORY_NAME,
            ATTRIBUTE_CATEGORY,
            ROOT_PARENT,
            &now,
        )?),
        None => None,
    };
    let mut tag_ids: HashMap<String, String> = HashMap::new();
    if let Some(category) = &category_id {
        let mut stmt = tx.prepare("SELECT ID, Name FROM djmdMyTag WHERE ParentID = ?1")?;
        let rows = stmt.query_map([category], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let deleted: BTreeSet<&str> = plan.delete_tags.iter().map(|t| t.id.as_str()).collect();
        for row in rows {
            let (id, name) = row?;
            if !deleted.contains(id.as_str()) {
                tag_ids.entry(name.unwrap_or_default()).or_insert(id);
            }
        }
    }
    for tag in &plan.delete_tags {
        delete_my_tag(&tx, &tag.id, &now)?;
    }
    for name in &plan.create_tags {
        let parent = category_id.as_deref().ok_or_else(|| {
            RekordboxError::Unsupported("the plan creates tags without a category".into())
        })?;
        let id = insert_my_tag(&tx, name, ATTRIBUTE_TAG, parent, &now)?;
        tag_ids.insert(name.clone(), id);
    }
    for removal in &plan.remove {
        delete_song_my_tag(&tx, &removal.row_id, &now)?;
    }
    for assignment in &plan.add {
        let tag_id = tag_ids.get(&assignment.tag).ok_or_else(|| {
            RekordboxError::Unsupported(format!("no tag {:?} to add tracks to", assignment.tag))
        })?;
        insert_song_my_tag(&tx, tag_id, &assignment.content_id, &now)?;
    }
    tx.commit()?;

    let outcome = ApplyOutcome {
        category_id,
        created_category: plan.create_category,
        created_tags: plan.create_tags.clone(),
        deleted_tags: plan.delete_tags.iter().map(|t| t.name.clone()).collect(),
        added: plan.add.len(),
        removed: plan.remove.len(),
        usn_before,
        usn_after: db.local_usn()?,
    };
    verify(db, plan, &before, &outcome)?;
    Ok(outcome)
}

/// Checks a database after [`apply`]: the `Overtone` category holds exactly
/// the desired tags and tracks, every row outside it is unchanged, the USN
/// counter moved by exactly what rbox's rules give, and SQLite's
/// `integrity_check` and `foreign_key_check` pass.
pub fn verify(
    db: &MasterDb,
    plan: &MyTagPlan,
    before: &OutsideRows,
    outcome: &ApplyOutcome,
) -> Result<(), RekordboxError> {
    let fail = |message: String| Err(RekordboxError::VerifyFailed(message));

    let tags = db.overtone_tags()?;
    if plan.desired.is_empty() && tags.is_empty() {
        // Nothing wanted and nothing there.
    } else if db.overtone_category()?.is_none() {
        return fail("the Overtone category is missing".into());
    }
    let mut seqs = BTreeSet::new();
    let mut by_name: BTreeMap<&str, &str> = BTreeMap::new();
    for tag in &tags {
        if tag.attribute != ATTRIBUTE_TAG {
            return fail(format!("{:?} is not a tag", tag.name));
        }
        if !seqs.insert(tag.seq) {
            return fail(format!("two tags share Seq {}", tag.seq));
        }
        if by_name.insert(&tag.name, &tag.id).is_some() {
            return fail(format!("tag {:?} appears twice", tag.name));
        }
    }
    let wanted: BTreeSet<&str> = plan.desired.iter().map(|t| t.name.as_str()).collect();
    let present: BTreeSet<&str> = by_name.keys().copied().collect();
    if wanted != present {
        return fail(format!("tags are {present:?}, expected {wanted:?}"));
    }
    let ids: Vec<String> = tags.iter().map(|t| t.id.clone()).collect();
    let assignments = db.tag_assignments(&ids)?;
    for desired in &plan.desired {
        let id = by_name[desired.name.as_str()];
        let rows = &assignments[id];
        let contents: BTreeSet<String> = rows.iter().map(|r| r.content_id.clone()).collect();
        if contents.len() != rows.len() {
            return fail(format!("tag {:?} lists a track twice", desired.name));
        }
        if contents != desired.content_ids {
            return fail(format!("tag {:?} holds the wrong tracks", desired.name));
        }
        let track_nos: BTreeSet<i64> = rows.iter().map(|r| r.track_no).collect();
        if track_nos.len() != rows.len() {
            return fail(format!("tag {:?} has a repeated TrackNo", desired.name));
        }
    }

    if OutsideRows::capture(db)? != *before {
        return fail("rows outside the Overtone category changed".into());
    }
    let expected = outcome.usn_before + plan.expected_usn_delta();
    if outcome.usn_after != expected || db.local_usn()? != expected {
        return fail(format!(
            "localUpdateCount is {}, expected {expected}",
            outcome.usn_after
        ));
    }
    let max_usn: Option<i64> = db.conn.query_row(
        "SELECT max(rb_local_usn) FROM djmdMyTag WHERE ID = ?1 OR ParentID = ?1",
        [outcome.category_id.as_deref().unwrap_or("")],
        |row| row.get(0),
    )?;
    if max_usn.is_some_and(|usn| usn > expected) {
        return fail("a My Tag row is stamped past localUpdateCount".into());
    }

    let integrity: Vec<String> = {
        let mut stmt = db.conn.prepare("PRAGMA integrity_check")?;
        stmt.query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    if integrity != ["ok"] {
        return fail(format!("integrity_check: {}", integrity.join("; ")));
    }
    let violations: i64 = {
        let mut stmt = db.conn.prepare("PRAGMA foreign_key_check")?;
        let mut rows = stmt.query([])?;
        let mut count = 0;
        while rows.next()?.is_some() {
            count += 1;
        }
        count
    };
    if violations != 0 {
        return fail(format!("foreign_key_check found {violations} problem(s)"));
    }
    Ok(())
}

/// Adds `delta` to the local USN counter and returns the new value (rbox
/// `models/agent_registry.rs`, `update_local_usn`).
fn increment_usn(tx: &Transaction<'_>, delta: i64) -> Result<i64, RekordboxError> {
    Ok(tx.query_row(
        "UPDATE agentRegistry SET int_1 = int_1 + ?1 WHERE registry_id = ?2 RETURNING int_1",
        params![delta, LOCAL_USN_ROW],
        |row| row.get(0),
    )?)
}

fn current_usn(tx: &Transaction<'_>) -> Result<i64, RekordboxError> {
    Ok(tx.query_row(
        "SELECT int_1 FROM agentRegistry WHERE registry_id = ?1",
        [LOCAL_USN_ROW],
        |row| row.get(0),
    )?)
}

/// A random unused ID for `table`, as rbox `models/mod.rs`
/// `RandomIdGenerator::new(true)` makes them: 32 random bits shifted right
/// by 4 (28-bit IDs), rejecting values below 100, written as decimal text,
/// and retried until no row has it.
fn random_id(tx: &Transaction<'_>, table: &str) -> Result<String, RekordboxError> {
    let mut stmt = tx.prepare(&format!("SELECT 1 FROM {table} WHERE ID = ?1"))?;
    for _ in 0..1_000_000 {
        let bytes = Uuid::new_v4().into_bytes();
        // The first four bytes of a version 4 UUID are all random.
        let id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 4;
        if id < 100 {
            continue;
        }
        let id = id.to_string();
        if stmt.query_row([&id], |_| Ok(())).optional()?.is_none() {
            return Ok(id);
        }
    }
    Err(RekordboxError::Unsupported(format!(
        "couldn't find a free ID in {table}"
    )))
}

/// Inserts a category or tag after its existing siblings (rbox
/// `models/djmd_my_tag.rs`, `NewDjmdMyTag::insert` with no `seq`): a fresh
/// ID and UUIDv4, the bookkeeping columns at their defaults, `usn` NULL, and
/// `rb_local_usn` from a counter incremented by one. Appending shifts no
/// siblings, so rbox's extra increment for shifted rows never applies.
fn insert_my_tag(
    tx: &Transaction<'_>,
    name: &str,
    attribute: i64,
    parent_id: &str,
    now: &str,
) -> Result<String, RekordboxError> {
    let id = random_id(tx, "djmdMyTag")?;
    let seq: i64 = tx.query_row(
        "SELECT COALESCE(max(Seq), 0) + 1 FROM djmdMyTag WHERE ParentID = ?1",
        [parent_id],
        |row| row.get(0),
    )?;
    let usn = increment_usn(tx, 1)?;
    tx.execute(
        "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, UUID, rb_data_status, \
         rb_local_data_status, rb_local_deleted, rb_local_synced, usn, rb_local_usn, \
         created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0, 0, NULL, ?7, ?8, ?8)",
        params![
            id,
            seq,
            name,
            attribute,
            parent_id,
            Uuid::new_v4().to_string(),
            usn,
            now
        ],
    )?;
    Ok(id)
}

/// Deletes a tag as rbox `models/djmd_my_tag.rs` `DjmdMyTag::delete` does:
/// a hard delete (not `rb_local_deleted`), later siblings moved up one
/// `Seq` and stamped with the next USN without incrementing it
/// (`shift_seq`), descendants deleted, every song-tag row of the deleted
/// tags deleted without USN changes, then the counter incremented by the
/// number of deleted `djmdMyTag` rows.
fn delete_my_tag(tx: &Transaction<'_>, id: &str, now: &str) -> Result<(), RekordboxError> {
    let (parent, seq): (Option<String>, Option<i64>) = tx.query_row(
        "DELETE FROM djmdMyTag WHERE ID = ?1 RETURNING ParentID, Seq",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let next_usn = current_usn(tx)? + 1;
    tx.execute(
        "UPDATE djmdMyTag SET Seq = Seq - 1, updated_at = ?1, rb_local_usn = ?2 \
         WHERE ParentID = ?3 AND Seq > ?4",
        params![now, next_usn, parent, seq.unwrap_or_default()],
    )?;
    let mut deleted = vec![id.to_string()];
    let mut queue = vec![id.to_string()];
    while let Some(parent) = queue.pop() {
        let mut stmt = tx.prepare("DELETE FROM djmdMyTag WHERE ParentID = ?1 RETURNING ID")?;
        let children = stmt
            .query_map([&parent], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        deleted.extend(children.iter().cloned());
        queue.extend(children);
    }
    for tag in &deleted {
        tx.execute("DELETE FROM djmdSongMyTag WHERE MyTagID = ?1", [tag])?;
    }
    increment_usn(tx, i64::try_from(deleted.len()).unwrap_or(i64::MAX))?;
    Ok(())
}

/// Removes one track from a tag as rbox `models/djmd_my_tag.rs`
/// `DjmdSongMyTag::delete` does: a hard delete, later rows moved up one
/// `TrackNo` and stamped with the next USN, then the counter incremented
/// by one.
fn delete_song_my_tag(tx: &Transaction<'_>, row_id: &str, now: &str) -> Result<(), RekordboxError> {
    let (tag, track_no): (Option<String>, Option<i64>) = tx.query_row(
        "DELETE FROM djmdSongMyTag WHERE ID = ?1 RETURNING MyTagID, TrackNo",
        [row_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let next_usn = current_usn(tx)? + 1;
    tx.execute(
        "UPDATE djmdSongMyTag SET TrackNo = TrackNo - 1, updated_at = ?1, rb_local_usn = ?2 \
         WHERE MyTagID = ?3 AND TrackNo > ?4",
        params![now, next_usn, tag, track_no.unwrap_or_default()],
    )?;
    increment_usn(tx, 1)?;
    Ok(())
}

/// Adds a track to a tag as rbox `models/djmd_my_tag.rs`
/// `NewDjmdSongMyTag::insert` does: the content must exist, the row goes
/// after the tag's existing tracks, and the counter is incremented by two
/// ("1 for creating, 1 for renaming from 'New Tag'", as rbox puts it).
fn insert_song_my_tag(
    tx: &Transaction<'_>,
    tag_id: &str,
    content_id: &str,
    now: &str,
) -> Result<(), RekordboxError> {
    let exists = tx
        .query_row(
            "SELECT 1 FROM djmdContent WHERE ID = ?1",
            [content_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(RekordboxError::Unsupported(format!(
            "track {content_id} is not in djmdContent"
        )));
    }
    let id = random_id(tx, "djmdSongMyTag")?;
    let track_no: i64 = tx.query_row(
        "SELECT COALESCE(max(TrackNo), 0) + 1 FROM djmdSongMyTag WHERE MyTagID = ?1",
        [tag_id],
        |row| row.get(0),
    )?;
    let usn = increment_usn(tx, 2)?;
    tx.execute(
        "INSERT INTO djmdSongMyTag (ID, MyTagID, ContentID, TrackNo, UUID, rb_data_status, \
         rb_local_data_status, rb_local_deleted, rb_local_synced, usn, rb_local_usn, \
         created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0, NULL, ?6, ?7, ?7)",
        params![
            id,
            tag_id,
            content_id,
            track_no,
            Uuid::new_v4().to_string(),
            usn,
            now
        ],
    )?;
    Ok(())
}

/// A timestamp the way Rekordbox and rbox (`models/mod.rs`, `DATEFMT`
/// `%Y-%m-%d %H:%M:%S%.3f %:z`, always UTC) write `created_at` and
/// `updated_at`: `2025-06-28 12:34:56.789 +00:00`.
pub(crate) fn rekordbox_timestamp(time: SystemTime) -> String {
    let since = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let date = crate::date_from_unix_secs(days * 86_400);
    format!(
        "{date} {:02}:{:02}:{:02}.{:03} +00:00",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        since.subsec_millis()
    )
}
