//! Detects a running Serato DJ.
//!
//! Serato keeps the library in memory while it runs and writes it back on
//! exit, so crate files written meanwhile are overwritten or ignored.
//!
//! What counts as running: any process whose executable lives inside an app
//! bundle named `Serato DJ*.app` (for example
//! `/Applications/Serato DJ Pro.app/Contents/MacOS/Serato DJ Pro`). Helper
//! executables named `seratodj-monitor*` are ignored even when they sit
//! inside a bundle, because the background monitor can outlive the app and
//! doesn't hold the library open. Serato Studio and Serato Sample are
//! different products and don't match.
//!
//! This is its own module so a lookup through `NSWorkspace` can replace it
//! later without touching the sync code.

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const BUNDLE_PREFIX: &str = "Serato DJ";
const IGNORED_HELPER_PREFIX: &str = "seratodj-monitor";

/// Whether a process executable belongs to the Serato DJ app itself.
pub fn is_serato_dj_executable(exe: &Path) -> bool {
    let helper = exe
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().starts_with(IGNORED_HELPER_PREFIX));
    if helper {
        return false;
    }
    exe.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|part| {
            part.starts_with(BUNDLE_PREFIX)
                && Path::new(part)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    })
}

/// Whether Serato DJ is running now. Errs on the side of "not running" only
/// when the process list can't be read at all.
pub fn serato_running() -> bool {
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
        .any(is_serato_dj_executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_executables_inside_the_app_bundle() {
        for exe in [
            "/Applications/Serato DJ Pro.app/Contents/MacOS/Serato DJ Pro",
            "/Applications/Serato DJ Lite.app/Contents/MacOS/Serato DJ Lite",
            "/Applications/Serato DJ Pro.app/Contents/Frameworks/Helper (GPU).app/Contents/MacOS/Helper",
        ] {
            assert!(is_serato_dj_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn ignores_other_products_and_the_monitor_helper() {
        for exe in [
            "/Applications/Serato Studio.app/Contents/MacOS/Serato Studio",
            "/Applications/Serato Sample.app/Contents/MacOS/Serato Sample",
            "/Applications/Serato DJ Pro.app/Contents/Helpers/seratodj-monitor",
            "/Library/Application Support/Serato/seratodj-monitor",
            "/usr/bin/vim",
            "/Users/dj/Serato DJ notes/tool",
        ] {
            assert!(!is_serato_dj_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn process_scan_runs() {
        // Result depends on the machine; this only checks it doesn't panic.
        let _ = serato_running();
    }
}
