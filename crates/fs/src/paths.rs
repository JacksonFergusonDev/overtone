//! Where Overtone keeps its own data, shared by the desktop app and CLI.

use std::path::PathBuf;

/// The desktop app's bundle identifier, which names its data folder.
pub const APP_IDENTIFIER: &str = "com.jacksonferguson.overtone";

/// Overrides the data folder, for tests and for keeping development data apart.
pub const DATA_DIR_ENV: &str = "OVERTONE_DATA_DIR";

/// The app data folder: `OVERTONE_DATA_DIR` if set, otherwise
/// `~/Library/Application Support/com.jacksonferguson.overtone` on macOS.
/// This matches Tauri's `app_data_dir()` (`dirs::data_dir()` joined with
/// the bundle identifier), so the app and CLI share one library.
pub fn app_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|dir| dir.join(APP_IDENTIFIER))
}

/// Default location of the library database.
pub fn default_library_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("library.sqlite"))
}

/// Folder holding timestamped backups made before writing DJ files.
pub fn backups_dir() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("backups"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn default_data_dir_matches_tauri_on_macos() {
        if std::env::var_os(DATA_DIR_ENV).is_some() {
            return;
        }
        let dir = app_data_dir().expect("data dir");
        assert!(dir.ends_with("Library/Application Support/com.jacksonferguson.overtone"));
        assert_eq!(
            default_library_path().expect("library"),
            dir.join("library.sqlite")
        );
    }
}
