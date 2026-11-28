//! Schema migrations tracked with SQLite's `user_version` pragma.
//!
//! Each migration is an append-only SQL file applied in its own
//! transaction. Never edit a migration that has shipped; add a new one.

use rusqlite::Connection;

use crate::LibraryError;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_waveforms.sql"),
    include_str!("../migrations/0003_metadata_provenance.sql"),
    include_str!("../migrations/0004_integrity_checks.sql"),
    include_str!("../migrations/0005_lookup_runs.sql"),
    include_str!("../migrations/0006_loudness.sql"),
    include_str!("../migrations/0007_conversions.sql"),
];

/// Schema version this build creates and understands.
#[allow(clippy::cast_possible_wrap, reason = "a handful of migrations")]
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub(crate) fn current_version(conn: &Connection) -> Result<i64, LibraryError> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(crate) fn apply(conn: &mut Connection) -> Result<(), LibraryError> {
    let found = current_version(conn)?;
    if found > SCHEMA_VERSION {
        return Err(LibraryError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    for (index, sql) in MIGRATIONS
        .iter()
        .enumerate()
        .skip(usize::try_from(found).unwrap_or(0))
    {
        let version = i64::try_from(index + 1).unwrap_or(i64::MAX);
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_all_migrations_once() {
        let mut conn = Connection::open_in_memory().expect("open");
        apply(&mut conn).expect("first apply");
        assert_eq!(current_version(&conn).expect("version"), SCHEMA_VERSION);
        apply(&mut conn).expect("second apply is a no-op");
        assert_eq!(current_version(&conn).expect("version"), SCHEMA_VERSION);
    }

    #[test]
    fn refuses_newer_schema() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("set version");
        assert!(matches!(
            apply(&mut conn),
            Err(LibraryError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn schema_passes_integrity_and_foreign_key_checks() {
        let mut conn = Connection::open_in_memory().expect("open");
        apply(&mut conn).expect("apply");
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("integrity");
        assert_eq!(integrity, "ok");
        let fk_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .expect("fk check");
        assert_eq!(fk_violations, 0);
    }
}
