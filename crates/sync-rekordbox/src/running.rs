//! Detects a running Rekordbox.
//!
//! XML export doesn't need Rekordbox closed, because the file is only read
//! when you import it. The check exists so the report can say so. My Tag sync
//! into `master.db` refuses while Rekordbox or `rekordboxAgent` runs.
//!
//! What counts as running: a process whose executable is named `rekordbox`
//! (any case), or lives inside an app bundle named `rekordbox*.app` (for
//! example `/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordbox`).
//! Helpers with "agent" in their name, such as `rekordboxAgent`, don't count
//! as the app; [`rekordbox_agent_running`] detects them separately, because
//! the agent may keep `master.db` open after the app quits.
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

/// Whether a process executable is a Rekordbox agent helper, such as
/// `rekordboxAgent` (its file name starts with "rekordbox" and contains
/// "agent", any case).
pub fn is_rekordbox_agent_executable(exe: &Path) -> bool {
    exe.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_lowercase)
        .is_some_and(|name| name.starts_with("rekordbox") && name.contains("agent"))
}

/// Which Rekordbox processes are running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunningState {
    pub app: bool,
    pub agent: bool,
}

/// Refuses while the app or the agent runs, naming which one.
pub fn ensure_closed(state: RunningState) -> Result<(), crate::RekordboxError> {
    if state.app {
        return Err(crate::RekordboxError::AppRunning);
    }
    if state.agent {
        return Err(crate::RekordboxError::AgentRunning);
    }
    Ok(())
}

/// Scans the process list once for the app and the agent. When the list
/// can't be read at all, both are reported running, so writers refuse
/// rather than guess.
pub fn running_state() -> RunningState {
    let Some(exes) = process_exes() else {
        return RunningState {
            app: true,
            agent: true,
        };
    };
    RunningState {
        app: exes.iter().any(|exe| is_rekordbox_executable(exe)),
        agent: exes.iter().any(|exe| is_rekordbox_agent_executable(exe)),
    }
}

fn process_exes() -> Option<Vec<std::path::PathBuf>> {
    let mut system = System::new().ok()?;
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    Some(
        system
            .processes()
            .values()
            .filter_map(sysinfo::Process::exe)
            .map(Path::to_path_buf)
            .collect(),
    )
}

/// Whether Rekordbox is running now. Reports "not running" only when the
/// process list can't be read at all.
pub fn rekordbox_running() -> bool {
    process_exes().is_some_and(|exes| exes.iter().any(|exe| is_rekordbox_executable(exe)))
}

/// Whether `rekordboxAgent` is running now. Reports "not running" only when
/// the process list can't be read at all.
pub fn rekordbox_agent_running() -> bool {
    process_exes().is_some_and(|exes| exes.iter().any(|exe| is_rekordbox_agent_executable(exe)))
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
    fn matches_agents_separately() {
        for exe in [
            "/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordboxAgent",
            "/Library/Application Support/Pioneer/rekordbox/rekordbox_agent",
            "/Applications/rekordbox 7/rekordboxAgent.app/Contents/MacOS/RekordboxAgent",
        ] {
            assert!(is_rekordbox_agent_executable(Path::new(exe)), "{exe}");
            assert!(!is_rekordbox_executable(Path::new(exe)), "{exe}");
        }
        for exe in [
            "/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordbox",
            "/usr/local/bin/agent",
            "/Applications/Other.app/Contents/MacOS/travelagent",
        ] {
            assert!(!is_rekordbox_agent_executable(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn refusal_names_the_running_process() {
        use crate::RekordboxError;
        assert!(ensure_closed(RunningState::default()).is_ok());
        let both = RunningState {
            app: true,
            agent: true,
        };
        assert!(matches!(
            ensure_closed(both),
            Err(RekordboxError::AppRunning)
        ));
        let agent = RunningState {
            app: false,
            agent: true,
        };
        let err = ensure_closed(agent).expect_err("agent");
        assert!(matches!(err, RekordboxError::AgentRunning));
        assert!(err.to_string().contains("rekordboxAgent"));
    }

    #[test]
    fn process_scan_runs() {
        let _ = rekordbox_running();
        let _ = rekordbox_agent_running();
        let _ = running_state();
    }
}
