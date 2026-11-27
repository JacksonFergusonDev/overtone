use overtone_model::{DjApp, UnixMillis};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Library, LibraryError, now_millis};

/// One recorded sync attempt to a DJ app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncRun {
    pub id: i64,
    pub app: DjApp,
    /// What was synced to, such as a `_Serato_` folder.
    pub target: String,
    pub dry_run: bool,
    pub status: SyncStatus,
    pub started_at: UnixMillis,
    pub finished_at: Option<UnixMillis>,
    pub backup_path: Option<String>,
    /// JSON summary of what changed.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SyncStatus {
    Running,
    Succeeded,
    Failed,
}

impl SyncStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

fn app_str(app: DjApp) -> &'static str {
    match app {
        DjApp::Serato => "serato",
        DjApp::Rekordbox => "rekordbox",
        DjApp::Traktor => "traktor",
    }
}

fn parse_app(value: &str) -> Option<DjApp> {
    Some(match value {
        "serato" => DjApp::Serato,
        "rekordbox" => DjApp::Rekordbox,
        "traktor" => DjApp::Traktor,
        _ => return None,
    })
}

impl Library {
    /// Records the start of a sync and returns its id.
    pub fn start_sync_run(
        &self,
        app: DjApp,
        target: &str,
        dry_run: bool,
    ) -> Result<i64, LibraryError> {
        self.conn.execute(
            "INSERT INTO sync_runs (app, target, dry_run, status, started_at) \
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![app_str(app), target, dry_run, now_millis()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records how a sync ended.
    pub fn finish_sync_run(
        &self,
        id: i64,
        status: SyncStatus,
        backup_path: Option<&str>,
        summary: Option<&str>,
    ) -> Result<(), LibraryError> {
        let updated = self.conn.execute(
            "UPDATE sync_runs SET status = ?2, finished_at = ?3, backup_path = ?4, summary = ?5 \
             WHERE id = ?1",
            params![id, status.as_str(), now_millis(), backup_path, summary],
        )?;
        if updated == 0 {
            return Err(LibraryError::NotFound {
                kind: "sync run",
                id,
            });
        }
        Ok(())
    }

    /// The most recent sync runs, newest first.
    pub fn sync_runs(&self, limit: u32) -> Result<Vec<SyncRun>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, app, target, dry_run, status, started_at, finished_at, backup_path, \
             summary FROM sync_runs ORDER BY started_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    id,
                    app,
                    target,
                    dry_run,
                    status,
                    started_at,
                    finished_at,
                    backup_path,
                    summary,
                )| {
                    Ok(SyncRun {
                        id,
                        app: parse_app(&app).ok_or(LibraryError::Corrupt {
                            column: "sync_runs.app",
                            value: app.clone(),
                        })?,
                        target,
                        dry_run,
                        status: SyncStatus::parse(&status).ok_or(LibraryError::Corrupt {
                            column: "sync_runs.status",
                            value: status.clone(),
                        })?,
                        started_at,
                        finished_at,
                        backup_path,
                        summary,
                    })
                },
            )
            .collect()
    }

    /// The last successful, non-dry-run sync to `app`, if any.
    pub fn last_successful_sync(&self, app: DjApp) -> Result<Option<UnixMillis>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT MAX(finished_at) FROM sync_runs \
                 WHERE app = ?1 AND status = 'succeeded' AND dry_run = 0",
                [app_str(app)],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_lists_sync_runs() {
        let lib = Library::open_in_memory().expect("open");
        let first = lib
            .start_sync_run(DjApp::Serato, "/tmp/_Serato_", true)
            .expect("start");
        lib.finish_sync_run(first, SyncStatus::Succeeded, None, Some("{}"))
            .expect("finish");
        assert_eq!(lib.last_successful_sync(DjApp::Serato).expect("last"), None);

        let second = lib
            .start_sync_run(DjApp::Serato, "/tmp/_Serato_", false)
            .expect("start");
        lib.finish_sync_run(second, SyncStatus::Succeeded, Some("/b"), None)
            .expect("finish");
        assert!(
            lib.last_successful_sync(DjApp::Serato)
                .expect("last")
                .is_some()
        );

        let runs = lib.sync_runs(10).expect("list");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, second);
        assert_eq!(runs[0].backup_path.as_deref(), Some("/b"));
        assert!(matches!(
            lib.finish_sync_run(999, SyncStatus::Failed, None, None),
            Err(LibraryError::NotFound { .. })
        ));
    }
}
