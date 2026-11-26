use overtone_model::UnixMillis;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Library, LibraryError, now_millis};

/// A folder the library indexes in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct LibraryRoot {
    pub id: i64,
    pub path: String,
    pub added_at: UnixMillis,
}

impl Library {
    /// Adds a folder to index. Adding the same path again is a no-op.
    pub fn add_root(&self, path: &str) -> Result<LibraryRoot, LibraryError> {
        let path = path.trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };
        self.conn.execute(
            "INSERT OR IGNORE INTO library_roots (path, added_at) VALUES (?1, ?2)",
            params![path, now_millis()],
        )?;
        Ok(self.conn.query_row(
            "SELECT id, path, added_at FROM library_roots WHERE path = ?1",
            [path],
            |row| {
                Ok(LibraryRoot {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    added_at: row.get(2)?,
                })
            },
        )?)
    }

    pub fn roots(&self) -> Result<Vec<LibraryRoot>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, added_at FROM library_roots ORDER BY path")?;
        let roots = stmt
            .query_map([], |row| {
                Ok(LibraryRoot {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    added_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(roots)
    }

    /// Stops indexing a folder. Track records under it are kept.
    pub fn remove_root(&self, id: i64) -> Result<(), LibraryError> {
        let removed = self
            .conn
            .execute("DELETE FROM library_roots WHERE id = ?1", [id])?;
        if removed == 0 {
            return Err(LibraryError::NotFound { kind: "root", id });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_lists_and_removes_roots() {
        let lib = Library::open_in_memory().expect("open");
        let a = lib.add_root("/music/b/").expect("add");
        let again = lib.add_root("/music/b").expect("add again");
        assert_eq!(a, again);
        lib.add_root("/music/a").expect("add");
        let paths: Vec<String> = lib
            .roots()
            .expect("list")
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(paths, ["/music/a", "/music/b"]);
        lib.remove_root(a.id).expect("remove");
        assert_eq!(lib.roots().expect("list").len(), 1);
    }
}
