//! Detects a running Traktor Pro.
//!
//! Traktor keeps the collection in memory and rewrites `collection.nml`
//! when it quits, so changes written while it runs are lost. Overtone
//! refuses to sync while it runs.
//!
//! What counts as running: a process whose executable name starts with
//! "Traktor" (any case), such as `Traktor Pro 3` or `Traktor Pro 4`, or that
//! lives inside an app bundle named `Traktor*.app` (for example
//! `/Applications/Native Instruments/Traktor Pro 4/Traktor Pro 4.app/Contents/MacOS/Traktor Pro 4`).
//! Crash reporters, agents, and helpers are ignored. The Traktor Pro 3 name
//! and the install layout are unverified against a real install, see
//! `engineering/manual-verification.md`.

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const IGNORED: [&str; 3] = ["crash", "agent", "helper"];

/// Whether a process executable belongs to the Traktor app itself.
pub fn is_traktor_executable(exe: &Path) -> bool {
    let file = exe
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(String::new, str::to_lowercase);
    if IGNORED.iter().any(|word| file.contains(word)) {
        return false;
    }
    if file.starts_with("traktor") {
        return true;
    }
    exe.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|part| {
            part.to_lowercase().starts_with("traktor")
                && Path::new(part)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    })
}

/// Whether Traktor is running now. Reports "not running" only when the
/// process list can't be read at all.
pub fn traktor_running() -> bool {
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
        .any(is_traktor_executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_app_executable() {
        for exe in [
            "/Applications/Native Instruments/Traktor Pro 4/Traktor Pro 4.app/Contents/MacOS/Traktor Pro 4",
            "/Applications/Native Instruments/Traktor Pro 3/Traktor Pro 3.app/Contents/MacOS/Traktor Pro 3",
            "/Applications/Traktor Pro 4.app/Contents/MacOS/Other",
            "/opt/traktor",
        ] {
            assert!(is_traktor_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn ignores_helpers_and_unrelated_processes() {
        for exe in [
            "/Applications/Native Instruments/Traktor Pro 4/Traktor Pro 4.app/Contents/MacOS/Traktor Crash Reporter",
            "/Library/Application Support/Native Instruments/Hardware/NIHardwareAgent.app/Contents/MacOS/NIHardwareAgent",
            "/Applications/Native Instruments/Traktor Pro 4/Traktor Pro 4.app/Contents/Helpers/TraktorAgent",
            "/usr/bin/vim",
            "/Users/dj/Traktor notes/tool",
        ] {
            assert!(!is_traktor_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn process_scan_runs() {
        let _ = traktor_running();
    }
}
