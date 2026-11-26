//! Protected roots: folders Overtone refuses to write into.
//!
//! Development builds protect the real music library and DJ software data
//! by default so a bug cannot damage them. See
//! `engineering/decisions/architecture-and-ui.md#protected-paths`.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::FsError;

/// Colon-separated list of extra folders to protect.
pub const PROTECTED_ROOTS_ENV: &str = "OVERTONE_PROTECTED_ROOTS";

/// Folders under the home directory that development builds protect.
const DEVELOPMENT_HOME_ROOTS: &[&str] = &[
    "Music",
    "Library/Application Support/Serato",
    "Library/Pioneer",
    "Documents/Native Instruments",
];

/// A set of folders that must not be written into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtectedRoots {
    roots: Vec<PathBuf>,
    /// Case-folded lexical and symlink-resolved forms of every root.
    keys: Vec<(PathBuf, usize)>,
}

impl ProtectedRoots {
    /// Protects nothing.
    pub fn none() -> Self {
        Self::default()
    }

    /// Protects exactly `roots`.
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut out = Self::default();
        for root in roots {
            out.push(&root);
        }
        out
    }

    fn push(&mut self, root: &Path) {
        let index = self.roots.len();
        for form in forms(root) {
            self.keys.push((case_fold(&form), index));
        }
        self.roots.push(lexically_normalize(&absolute(root)));
    }

    /// The roots development builds protect under `home`.
    pub fn development_defaults(home: &Path) -> Self {
        Self::new(DEVELOPMENT_HOME_ROOTS.iter().map(|rel| home.join(rel)))
    }

    /// Builds the roots for this process: the development defaults in debug
    /// builds, plus any folders listed in `OVERTONE_PROTECTED_ROOTS`.
    pub fn from_environment() -> Self {
        let mut out = Self::default();
        if cfg!(debug_assertions)
            && let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
        {
            for rel in DEVELOPMENT_HOME_ROOTS {
                out.push(&Path::new(&home).join(rel));
            }
        }
        if let Some(extra) = std::env::var_os(PROTECTED_ROOTS_ENV) {
            for root in parse_root_list(&extra) {
                out.push(&root);
            }
        }
        out
    }

    /// The process-wide roots, computed once from the environment.
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<ProtectedRoots> = OnceLock::new();
        GLOBAL.get_or_init(Self::from_environment)
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Returns an error if writing to `path` would land inside a protected root.
    ///
    /// Both the lexical path and the path with symlinks resolved are checked,
    /// so a link pointing into a protected folder is caught. Comparison
    /// ignores case because APFS volumes are case-insensitive by default.
    pub fn check(&self, path: &Path) -> Result<(), FsError> {
        for form in forms(path) {
            let key = case_fold(&form);
            if let Some((_, index)) = self.keys.iter().find(|(root, _)| key.starts_with(root)) {
                return Err(FsError::Protected {
                    path: path.to_path_buf(),
                    root: self.roots[*index].clone(),
                });
            }
        }
        Ok(())
    }
}

fn parse_root_list(value: &OsString) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The lexical form of `path` and the form with symlinks in its longest
/// existing ancestor resolved.
fn forms(path: &Path) -> [PathBuf; 2] {
    let lexical = lexically_normalize(&absolute(path));
    let resolved = resolve_existing(&lexical);
    [lexical, resolved]
}

/// Resolves symlinks in the longest existing ancestor of an absolute,
/// lexically normalized path and appends the remaining components.
fn resolve_existing(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if let Ok(mut resolved) = existing.canonicalize() {
            for name in rest.iter().rev() {
                resolved.push(name);
            }
            return resolved;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn case_fold(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn blocks_paths_inside_roots_and_allows_others() {
        let dir = TempDir::new().expect("tempdir");
        let music = dir.path().join("Music");
        std::fs::create_dir(&music).expect("mkdir");
        let roots = ProtectedRoots::new([music.clone()]);

        assert!(roots.check(&music.join("High Quality/track.flac")).is_err());
        assert!(roots.check(&music).is_err());
        assert!(
            roots
                .check(&dir.path().join("MusicBackup/track.flac"))
                .is_ok()
        );
        assert!(roots.check(&dir.path().join("other.flac")).is_ok());
    }

    #[test]
    fn ignores_case_and_dot_segments() {
        let dir = TempDir::new().expect("tempdir");
        let music = dir.path().join("Music");
        std::fs::create_dir(&music).expect("mkdir");
        let roots = ProtectedRoots::new([music]);

        assert!(roots.check(&dir.path().join("music/track.flac")).is_err());
        assert!(
            roots
                .check(&dir.path().join("elsewhere/../Music/track.flac"))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinks_into_protected_roots() {
        let dir = TempDir::new().expect("tempdir");
        let music = dir.path().join("Music");
        std::fs::create_dir(&music).expect("mkdir");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&music, &link).expect("symlink");
        let roots = ProtectedRoots::new([music]);

        assert!(roots.check(&link.join("track.flac")).is_err());
    }

    #[test]
    fn development_defaults_cover_dj_data() {
        let dir = TempDir::new().expect("tempdir");
        let roots = ProtectedRoots::development_defaults(dir.path());
        for rel in [
            "Music/_Serato_/Subcrates/a.crate",
            "Music/High Quality/DnB/x.flac",
            "Library/Application Support/Serato/Library/master.sqlite",
            "Library/Pioneer/rekordbox/master.db",
            "Documents/Native Instruments/Traktor 4.0.0/collection.nml",
        ] {
            assert!(roots.check(&dir.path().join(rel)).is_err(), "{rel}");
        }
        assert!(
            roots
                .check(&dir.path().join("Developer/overtone/local/music/a.flac"))
                .is_ok()
        );
    }

    #[test]
    fn none_allows_everything() {
        assert!(ProtectedRoots::none().check(Path::new("/")).is_ok());
    }

    #[test]
    fn parses_colon_separated_list() {
        let parsed = parse_root_list(&OsString::from("/a:/b::/c"));
        assert_eq!(
            parsed,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
    }
}
