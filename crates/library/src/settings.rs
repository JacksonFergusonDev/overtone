use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Library, LibraryError};

impl Library {
    /// Reads a setting stored as JSON. Returns `None` if it was never set or
    /// no longer matches `T`'s shape.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, LibraryError> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value.and_then(|json| serde_json::from_str(&json).ok()))
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), LibraryError> {
        let json = serde_json::to_string(value).map_err(|err| LibraryError::Corrupt {
            column: "settings.value",
            value: err.to_string(),
        })?;
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_json_settings() {
        let lib = Library::open_in_memory().expect("open");
        assert_eq!(lib.setting::<bool>("mirror_comment").expect("get"), None);
        lib.set_setting("mirror_comment", &true).expect("set");
        lib.set_setting("mirror_comment", &false)
            .expect("overwrite");
        assert_eq!(
            lib.setting::<bool>("mirror_comment").expect("get"),
            Some(false)
        );
        assert_eq!(
            lib.setting::<Vec<String>>("mirror_comment").expect("get"),
            None
        );
    }
}
