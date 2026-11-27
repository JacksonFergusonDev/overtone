//! Detects a running Rekordbox.
//!
//! XML export doesn't need Rekordbox closed, because the file is only read
//! when you import it. The check exists so the report can say so, and for the
//! future `master.db` writer.
//!
//! What counts as running: a process whose executable is named `rekordbox`
//! (any case), or lives inside an app bundle named `rekordbox*.app` (for
//! example `/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordbox`).
//! Helpers with "agent" in their name, such as `rekordboxAgent`, are ignored.
//! The bundle ID and install layout are unverified against a real install:
//! see `engineering/manual-verification.md`.

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Whether a process executable belongs to the Rekordbox app itself.
pub fn is_rekordbox_executable(exe: &Path) -> bool {
    let file = exe
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_lowercase);
    if file.as_deref().is_some_and(|name| name.contains("agent")) {
        return false;
    }
    if file.as_deref() == Some("rekordbox") {
        return true;
    }
    exe.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|part| {
            part.to_lowercase().starts_with("rekordbox")
                && Path::new(part)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    })
}

/// Whether Rekordbox is running now. Reports "not running" only when the
/// process list can't be read at all.
pub fn rekordbox_running() -> bool {
    let Ok(mut system) = System::new() else {
        return false;
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    system
        .processes()
        .values()
        .filter_map(sysinfo::Process::exe)
        .any(is_rekordbox_executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_app_executable() {
        for exe in [
            "/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordbox",
            "/Applications/rekordbox 6/rekordbox.app/Contents/MacOS/rekordbox",
            "/Applications/rekordbox.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper",
            "/opt/Rekordbox",
        ] {
            assert!(is_rekordbox_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn ignores_agents_and_unrelated_processes() {
        for exe in [
            "/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordboxAgent",
            "/Library/Application Support/Pioneer/rekordbox/rekordbox_agent",
            "/usr/bin/vim",
            "/Users/dj/rekordbox notes/tool",
        ] {
            assert!(!is_rekordbox_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn process_scan_runs() {
        let _ = rekordbox_running();
    }
}
