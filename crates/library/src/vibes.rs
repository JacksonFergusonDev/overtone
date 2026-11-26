use overtone_model::{TrackId, Vibe, VibeId, VibePath};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{Library, LibraryError, now_millis};

impl Library {
    /// Creates a vibe and any enclosing vibes that don't exist yet. Returns
    /// the existing vibe unchanged if one with the same slug exists.
    pub fn create_vibe(&self, path: &VibePath) -> Result<Vibe, LibraryError> {
        let tx = self.conn.unchecked_transaction()?;
        let id = ensure_vibe(&tx, path, now_millis())?;
        tx.commit()?;
        self.vibe(id)
    }

    pub fn vibe(&self, id: VibeId) -> Result<Vibe, LibraryError> {
        self.conn
            .query_row(
                "SELECT v.id, v.path, v.created_at, \
                 (SELECT COUNT(*) FROM track_vibes tv WHERE tv.vibe_id = v.id) \
                 FROM vibes v WHERE v.id = ?1",
                [id.0],
                vibe_from_row,
            )
            .optional()?
            .ok_or(LibraryError::NotFound {
                kind: "vibe",
                id: id.0,
            })?
    }

    /// Finds a vibe by path, ignoring case and whitespace differences.
    pub fn vibe_by_path(&self, path: &VibePath) -> Result<Option<Vibe>, LibraryError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM vibes WHERE slug = ?1",
                [path.slug()],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.vibe(VibeId(id))).transpose()
    }

    /// All vibes ordered so parents come before their children.
    pub fn vibes(&self) -> Result<Vec<Vibe>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT v.id, v.path, v.created_at, COUNT(tv.track_id) FROM vibes v \
             LEFT JOIN track_vibes tv ON tv.vibe_id = v.id GROUP BY v.id ORDER BY v.slug",
        )?;
        let vibes = stmt
            .query_map([], vibe_from_row)?
            .map(|row| row?)
            .collect::<Result<Vec<_>, LibraryError>>()?;
        Ok(vibes)
    }

    /// Renames a vibe, moving its nested vibes with it. Enclosing vibes of
    /// the new path are created if needed.
    pub fn rename_vibe(&self, id: VibeId, new_path: &VibePath) -> Result<Vibe, LibraryError> {
        let old = self.vibe(id)?;
        if old.path.is_ancestor_of(new_path) {
            return Err(LibraryError::VibeExists(format!(
                "{new_path} (can't move a vibe inside itself)"
            )));
        }
        let tx = self.conn.unchecked_transaction()?;
        let clash: Option<i64> = tx
            .query_row(
                "SELECT id FROM vibes WHERE slug = ?1 AND id != ?2",
                params![new_path.slug(), id.0],
                |row| row.get(0),
            )
            .optional()?;
        if clash.is_some() {
            return Err(LibraryError::VibeExists(new_path.to_string()));
        }

        let descendants = descendants_in(&tx, &old.path)?;
        let old_depth = old.path.depth();
        let mut renamed = vec![(id, new_path.clone())];
        for (child_id, child_path) in descendants {
            let mut segments = new_path.segments().to_vec();
            segments.extend_from_slice(&child_path.segments()[old_depth..]);
            renamed.push((child_id, VibePath::parse(&segments.join("/"))?));
        }
        // Move every row to a temporary slug first (`#` never appears in a
        // real slug) so the unique index can't
        // trip on intermediate states, then write the final values.
        for (vibe_id, _) in &renamed {
            tx.execute(
                "UPDATE vibes SET slug = '#renaming/' || id WHERE id = ?1",
                [vibe_id.0],
            )?;
        }
        for (vibe_id, path) in &renamed {
            let clash: Option<i64> = tx
                .query_row(
                    "SELECT id FROM vibes WHERE slug = ?1",
                    [path.slug()],
                    |row| row.get(0),
                )
                .optional()?;
            if clash.is_some() {
                return Err(LibraryError::VibeExists(path.to_string()));
            }
            tx.execute(
                "UPDATE vibes SET path = ?2, slug = ?3 WHERE id = ?1",
                params![vibe_id.0, path.to_string(), path.slug()],
            )?;
        }
        let now = now_millis();
        for ancestor in new_path.ancestors() {
            ensure_vibe(&tx, &ancestor, now)?;
        }
        tx.commit()?;
        self.vibe(id)
    }

    /// Deletes a vibe and every vibe nested inside it. Tracks are untouched.
    pub fn delete_vibe(&self, id: VibeId) -> Result<(), LibraryError> {
        let vibe = self.vibe(id)?;
        let tx = self.conn.unchecked_transaction()?;
        for (child, _) in descendants_in(&tx, &vibe.path)? {
            tx.execute("DELETE FROM vibes WHERE id = ?1", [child.0])?;
        }
        tx.execute("DELETE FROM vibes WHERE id = ?1", [id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// Adds tracks to a vibe. Tracks already in it are left as they are.
    pub fn assign_vibe(&self, tracks: &[TrackId], vibe: VibeId) -> Result<(), LibraryError> {
        self.vibe(vibe)?;
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO track_vibes (track_id, vibe_id, added_at) VALUES (?1, ?2, ?3)",
            )?;
            let now = now_millis();
            for track in tracks {
                stmt.execute(params![track.0, vibe.0, now]).map_err(|err| {
                    if is_foreign_key_error(&err) {
                        LibraryError::NotFound {
                            kind: "track",
                            id: track.0,
                        }
                    } else {
                        err.into()
                    }
                })?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes tracks from a vibe.
    pub fn unassign_vibe(&self, tracks: &[TrackId], vibe: VibeId) -> Result<(), LibraryError> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare("DELETE FROM track_vibes WHERE track_id = ?1 AND vibe_id = ?2")?;
            for track in tracks {
                stmt.execute(params![track.0, vibe.0])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn vibe_ids_for(&self, track: TrackId) -> Result<Vec<VibeId>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT vibe_id FROM track_vibes WHERE track_id = ?1 ORDER BY vibe_id")?;
        let ids = stmt
            .query_map([track.0], |row| Ok(VibeId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Tracks assigned directly to `vibe`, ordered by path.
    pub fn tracks_in_vibe(&self, vibe: VibeId) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM track_vibes tv JOIN tracks t ON t.id = tv.track_id \
             WHERE tv.vibe_id = ?1 ORDER BY t.path",
        )?;
        let ids = stmt
            .query_map([vibe.0], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Pairs of unrelated vibes where a Comment "contains" rule for the first
    /// hashtag would also match the second, such as `#dub` and `#dubtechno`.
    pub fn hashtag_prefix_collisions(&self) -> Result<Vec<(VibePath, VibePath)>, LibraryError> {
        let vibes = self.vibes()?;
        let mut collisions = Vec::new();
        for short in &vibes {
            let short_slug = short.path.slug();
            for long in &vibes {
                let long_slug = long.path.slug();
                if long_slug.len() > short_slug.len()
                    && long_slug.starts_with(&short_slug)
                    && !short.path.is_ancestor_of(&long.path)
                {
                    collisions.push((short.path.clone(), long.path.clone()));
                }
            }
        }
        Ok(collisions)
    }
}

fn ensure_vibe(tx: &Transaction<'_>, path: &VibePath, now: i64) -> Result<VibeId, LibraryError> {
    for ancestor in path.ancestors() {
        insert_if_missing(tx, &ancestor, now)?;
    }
    insert_if_missing(tx, path, now)
}

fn insert_if_missing(
    tx: &Transaction<'_>,
    path: &VibePath,
    now: i64,
) -> Result<VibeId, LibraryError> {
    let slug = path.slug();
    if let Some(id) = tx
        .query_row("SELECT id FROM vibes WHERE slug = ?1", [&slug], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?
    {
        return Ok(VibeId(id));
    }
    tx.execute(
        "INSERT INTO vibes (path, slug, created_at) VALUES (?1, ?2, ?3)",
        params![path.to_string(), slug, now],
    )?;
    Ok(VibeId(tx.last_insert_rowid()))
}

fn descendants_in(
    tx: &Transaction<'_>,
    parent: &VibePath,
) -> Result<Vec<(VibeId, VibePath)>, LibraryError> {
    let prefix = format!("{}/", parent.slug());
    let mut stmt = tx.prepare(
        "SELECT id, path FROM vibes WHERE substr(slug, 1, length(?1)) = ?1 ORDER BY slug",
    )?;
    let rows = stmt
        .query_map([&prefix], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, path)| Ok((VibeId(id), parse_stored(path)?)))
        .collect()
}

fn parse_stored(path: String) -> Result<VibePath, LibraryError> {
    VibePath::parse(&path).map_err(|_| LibraryError::Corrupt {
        column: "vibes.path",
        value: path,
    })
}

fn vibe_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Vibe, LibraryError>> {
    let path: String = row.get(1)?;
    let id = VibeId(row.get(0)?);
    let created_at = row.get(2)?;
    let track_count = row.get(3)?;
    Ok(parse_stored(path).map(|path| Vibe {
        id,
        path,
        created_at,
        track_count,
    }))
}

fn is_foreign_key_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::tests::scanned;

    fn path(input: &str) -> VibePath {
        input.parse().expect("vibe path")
    }

    #[test]
    fn creating_a_nested_vibe_creates_its_parents() {
        let lib = Library::open_in_memory().expect("open");
        let deep = lib.create_vibe(&path("Dubs/Deep/Late")).expect("create");
        assert_eq!(deep.path, path("Dubs/Deep/Late"));
        let names: Vec<String> = lib
            .vibes()
            .expect("list")
            .iter()
            .map(|v| v.path.to_string())
            .collect();
        assert_eq!(names, ["Dubs", "Dubs/Deep", "Dubs/Deep/Late"]);
    }

    #[test]
    fn same_slug_returns_existing_vibe() {
        let lib = Library::open_in_memory().expect("open");
        let a = lib.create_vibe(&path("Deep House")).expect("a");
        let b = lib.create_vibe(&path("deep   house")).expect("b");
        assert_eq!(a.id, b.id);
        assert_eq!(b.path.to_string(), "Deep House");
        assert_eq!(
            lib.vibe_by_path(&path("DEEP HOUSE"))
                .expect("find")
                .map(|v| v.id),
            Some(a.id)
        );
    }

    #[test]
    fn assigns_and_counts_overlapping_vibes() {
        let mut lib = Library::open_in_memory().expect("open");
        let (t1, _) = lib.upsert_scanned(&scanned("/m/1.flac")).expect("t1");
        let (t2, _) = lib.upsert_scanned(&scanned("/m/2.flac")).expect("t2");
        let warmup = lib.create_vibe(&path("Warmup")).expect("warmup");
        let dubs = lib.create_vibe(&path("Dubs")).expect("dubs");
        lib.assign_vibe(&[t1, t2], warmup.id).expect("assign");
        lib.assign_vibe(&[t1], dubs.id).expect("assign");
        lib.assign_vibe(&[t1], dubs.id)
            .expect("assign twice is fine");

        assert_eq!(lib.vibe(warmup.id).expect("load").track_count, 2);
        assert_eq!(lib.track(t1).expect("t1").vibes, vec![warmup.id, dubs.id]);
        assert_eq!(
            lib.tracks_in_vibe(warmup.id).expect("members"),
            vec![t1, t2]
        );

        lib.unassign_vibe(&[t1], warmup.id).expect("unassign");
        assert_eq!(lib.tracks_in_vibe(warmup.id).expect("members"), vec![t2]);
        assert!(matches!(
            lib.assign_vibe(&[TrackId(999)], dubs.id),
            Err(LibraryError::NotFound { kind: "track", .. })
        ));
    }

    #[test]
    fn rename_moves_nested_vibes_and_keeps_tracks() {
        let mut lib = Library::open_in_memory().expect("open");
        let (track, _) = lib.upsert_scanned(&scanned("/m/1.flac")).expect("track");
        let dubs = lib.create_vibe(&path("Dubs")).expect("dubs");
        let deep = lib.create_vibe(&path("Dubs/Deep")).expect("deep");
        lib.assign_vibe(&[track], deep.id).expect("assign");

        lib.rename_vibe(dubs.id, &path("Bass/Dub")).expect("rename");
        let names: Vec<String> = lib
            .vibes()
            .expect("list")
            .iter()
            .map(|v| v.path.to_string())
            .collect();
        assert_eq!(names, ["Bass", "Bass/Dub", "Bass/Dub/Deep"]);
        assert_eq!(lib.vibe(deep.id).expect("deep").track_count, 1);
    }

    #[test]
    fn rename_rejects_clashes_and_self_nesting() {
        let lib = Library::open_in_memory().expect("open");
        let dubs = lib.create_vibe(&path("Dubs")).expect("dubs");
        lib.create_vibe(&path("Warmup")).expect("warmup");
        assert!(matches!(
            lib.rename_vibe(dubs.id, &path("warmup")),
            Err(LibraryError::VibeExists(_))
        ));
        assert!(matches!(
            lib.rename_vibe(dubs.id, &path("Dubs/Inner")),
            Err(LibraryError::VibeExists(_))
        ));
        let renamed = lib
            .rename_vibe(dubs.id, &path("dubs"))
            .expect("case-only rename");
        assert_eq!(renamed.path.to_string(), "dubs");
    }

    #[test]
    fn delete_removes_nested_vibes_only() {
        let lib = Library::open_in_memory().expect("open");
        let dubs = lib.create_vibe(&path("Dubs/Deep")).expect("deep");
        lib.create_vibe(&path("Dubstep")).expect("dubstep");
        let parent = lib
            .vibe_by_path(&path("Dubs"))
            .expect("find")
            .expect("exists");
        lib.delete_vibe(parent.id).expect("delete");
        let names: Vec<String> = lib
            .vibes()
            .expect("list")
            .iter()
            .map(|v| v.path.to_string())
            .collect();
        assert_eq!(names, ["Dubstep"]);
        assert!(matches!(
            lib.vibe(dubs.id),
            Err(LibraryError::NotFound { .. })
        ));
    }

    #[test]
    fn reports_hashtag_prefix_collisions_but_not_nesting() {
        let lib = Library::open_in_memory().expect("open");
        lib.create_vibe(&path("Dub")).expect("dub");
        lib.create_vibe(&path("Dub/Deep")).expect("nested");
        lib.create_vibe(&path("Dubtechno")).expect("dubtechno");
        let collisions = lib.hashtag_prefix_collisions().expect("collisions");
        assert_eq!(collisions, vec![(path("Dub"), path("Dubtechno"))]);
    }
}
