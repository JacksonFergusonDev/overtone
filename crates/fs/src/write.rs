//! Atomic file replacement and timestamped backups.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{FsError, ProtectedRoots};

/// Writes `contents` to `path` atomically: a temporary sibling file is
/// written and flushed to disk, then renamed over the target. Readers see
/// either the old file or the new one, never a partial write. An existing
/// file's permissions are kept.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), FsError> {
    write_atomic_guarded(ProtectedRoots::global(), path, contents)
}

/// [`write_atomic`] with explicit protected roots, for tests.
pub fn write_atomic_guarded(
    roots: &ProtectedRoots,
    path: &Path,
    contents: &[u8],
) -> Result<(), FsError> {
    replace_guarded(roots, path, |file| {
        file.write_all(contents)
            .map_err(|source| FsError::io(path, source))
    })
}

/// Replaces `path` atomically with whatever `fill` writes into a fresh
/// temporary file.
pub fn replace_guarded(
    roots: &ProtectedRoots,
    path: &Path,
    fill: impl FnOnce(&mut File) -> Result<(), FsError>,
) -> Result<(), FsError> {
    roots.check(path)?;
    let dir = parent_dir(path);
    let mut temp = tempfile::Builder::new()
        .prefix(".overtone-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|source| FsError::io(dir, source))?;
    fill(temp.as_file_mut())?;
    if let Ok(existing) = std::fs::metadata(path) {
        std::fs::set_permissions(temp.path(), existing.permissions())
            .map_err(|source| FsError::io(temp.path(), source))?;
    }
    // On Apple targets `sync_all` issues F_FULLFSYNC, so the data reaches the
    // disk platters rather than only the drive cache.
    temp.as_file()
        .sync_all()
        .map_err(|source| FsError::io(temp.path(), source))?;
    temp.persist(path)
        .map_err(|err| FsError::io(path, err.error))?;
    sync_dir(dir);
    Ok(())
}

/// Edits a copy of `path` and swaps it in atomically.
///
/// The original is cloned (APFS copy-on-write where available, a plain copy
/// elsewhere) to a temporary sibling, `edit` modifies that copy in place,
/// and the copy is then renamed over the original. If `edit` fails, the
/// original is untouched. Cloning keeps extended attributes and other
/// metadata that a fresh file would lose.
pub fn edit_via_copy(
    roots: &ProtectedRoots,
    path: &Path,
    edit: impl FnOnce(&Path) -> Result<(), FsError>,
) -> Result<(), FsError> {
    roots.check(path)?;
    let dir = parent_dir(path);
    let temp = tempfile::Builder::new()
        .prefix(".overtone-")
        .suffix(".tmp")
        .make_in(dir, |temp_path| {
            reflink_copy::reflink_or_copy(path, temp_path).map(|_| ())
        })
        .map_err(|source| FsError::io(path, source))?;
    edit(temp.path())?;
    File::open(temp.path())
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::io(temp.path(), source))?;
    temp.persist(path)
        .map_err(|err| FsError::io(path, err.error))?;
    sync_dir(dir);
    Ok(())
}

/// Creates a new file at `path` from whatever `fill` writes to a temporary
/// sibling path (given to `fill` as a path, so another program such as
/// ffmpeg can write it). The temporary file is flushed and then moved into
/// place only if nothing exists at `path` yet: an existing file is never
/// replaced, and a failed `fill` leaves nothing behind.
pub fn create_new_via_temp(
    roots: &ProtectedRoots,
    path: &Path,
    fill: impl FnOnce(&Path) -> Result<(), FsError>,
) -> Result<(), FsError> {
    roots.check(path)?;
    let dir = parent_dir(path);
    let temp = tempfile::Builder::new()
        .prefix(".overtone-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|source| FsError::io(dir, source))?
        .into_temp_path();
    fill(&temp)?;
    // Temporary files are created private (0600); a new file should get
    // the usual permissions, so other apps and users can read it.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o644))
            .map_err(|source| FsError::io(&*temp, source))?;
    }
    File::open(&temp)
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::io(&*temp, source))?;
    // `persist_noclobber` uses a hard link plus unlink, so it fails rather
    // than replacing a file that appeared meanwhile.
    temp.persist_noclobber(path)
        .map_err(|err| FsError::io(path, err.error))?;
    sync_dir(dir);
    Ok(())
}

/// Copies `source` into a new timestamped folder under `backups_dir` and
/// returns the backup's path. Uses an APFS clone when possible, so backups
/// of large files are instant and take no extra space until either copy
/// changes.
///
/// A clone shares storage with the original, so it protects against bad
/// writes, not against drive failure.
pub fn backup_file(
    roots: &ProtectedRoots,
    source: &Path,
    backups_dir: &Path,
) -> Result<PathBuf, FsError> {
    let folder = backups_dir.join(timestamp_folder_name(std::time::SystemTime::now()));
    roots.check(&folder)?;
    std::fs::create_dir_all(&folder).map_err(|err| FsError::io(&folder, err))?;
    let name = source
        .file_name()
        .ok_or_else(|| FsError::io(source, std::io::Error::other("path has no file name")))?;
    let dest = unique_path(&folder.join(name));
    reflink_copy::reflink_or_copy(source, &dest).map_err(|err| FsError::io(source, err))?;
    Ok(dest)
}

/// Copies every file in `sources` into one new timestamped folder under
/// `backups_dir`, keeping their names, and returns the folder. For files
/// that only make sense together, such as a SQLite database and its `-wal`.
/// Same cloning behavior as [`backup_file`].
pub fn backup_files(
    roots: &ProtectedRoots,
    sources: &[&Path],
    backups_dir: &Path,
) -> Result<PathBuf, FsError> {
    let folder =
        unique_path(&backups_dir.join(timestamp_folder_name(std::time::SystemTime::now())));
    roots.check(&folder)?;
    std::fs::create_dir_all(&folder).map_err(|err| FsError::io(&folder, err))?;
    for source in sources {
        let name = source
            .file_name()
            .ok_or_else(|| FsError::io(source, std::io::Error::other("path has no file name")))?;
        let dest = unique_path(&folder.join(name));
        reflink_copy::reflink_or_copy(source, &dest).map_err(|err| FsError::io(source, err))?;
    }
    Ok(folder)
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Flushes a directory entry change (the rename) to disk. Best effort:
/// some filesystems don't support syncing directories.
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
    (2..u32::MAX)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

/// `YYYY-MM-DD_HH-MM-SS.mmm` in UTC, sortable and safe in file names.
pub(crate) fn timestamp_folder_name(time: std::time::SystemTime) -> String {
    let since_epoch = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}.{millis:03}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Converts days since 1970-01-01 to a proleptic Gregorian date
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn writes_new_and_replaces_existing_files() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.crate");
        let roots = ProtectedRoots::none();
        write_atomic_guarded(&roots, &path, b"one").expect("write");
        write_atomic_guarded(&roots, &path, b"two").expect("replace");
        assert_eq!(std::fs::read(&path).expect("read"), b"two");
        let leftovers = std::fs::read_dir(dir.path()).expect("list").count();
        assert_eq!(leftovers, 1, "temporary files are cleaned up");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions_of_replaced_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"old").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod");
        write_atomic_guarded(&ProtectedRoots::none(), &path, b"new").expect("replace");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn refuses_protected_targets_without_touching_them() {
        let dir = TempDir::new().expect("tempdir");
        let protected = dir.path().join("Music");
        std::fs::create_dir(&protected).expect("mkdir");
        let path = protected.join("track.flac");
        std::fs::write(&path, b"original").expect("seed");
        let roots = ProtectedRoots::new([protected]);

        assert!(matches!(
            write_atomic_guarded(&roots, &path, b"bad"),
            Err(FsError::Protected { .. })
        ));
        assert!(matches!(
            edit_via_copy(&roots, &path, |_| Ok(())),
            Err(FsError::Protected { .. })
        ));
        assert_eq!(std::fs::read(&path).expect("read"), b"original");
    }

    #[test]
    fn edit_via_copy_applies_edits_and_leaves_original_on_failure() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("track.flac");
        std::fs::write(&path, b"original").expect("seed");
        let roots = ProtectedRoots::none();

        let failed = edit_via_copy(&roots, &path, |temp| {
            std::fs::write(temp, b"half-written").expect("write temp");
            Err(FsError::io(
                temp,
                std::io::Error::other("tag writer failed"),
            ))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).expect("read"), b"original");

        edit_via_copy(&roots, &path, |temp| {
            assert_eq!(std::fs::read(temp).expect("read temp"), b"original");
            std::fs::write(temp, b"edited").map_err(|err| FsError::io(temp, err))
        })
        .expect("edit");
        assert_eq!(std::fs::read(&path).expect("read"), b"edited");
        assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 1);
    }

    #[test]
    fn backups_land_in_timestamped_folders_without_overwriting() {
        let dir = TempDir::new().expect("tempdir");
        let source = dir.path().join("collection.nml");
        std::fs::write(&source, b"v1").expect("seed");
        let backups = dir.path().join("backups");
        let roots = ProtectedRoots::none();

        let first = backup_file(&roots, &source, &backups).expect("backup");
        std::fs::write(&source, b"v2").expect("change");
        let second = backup_file(&roots, &source, &backups).expect("backup");
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).expect("read"), b"v1");
        assert_eq!(std::fs::read(&second).expect("read"), b"v2");
        assert!(first.starts_with(&backups));
    }

    #[test]
    fn backs_up_several_files_into_one_folder() {
        let dir = TempDir::new().expect("tempdir");
        let db = dir.path().join("master.db");
        let wal = dir.path().join("master.db-wal");
        std::fs::write(&db, b"db").expect("seed");
        std::fs::write(&wal, b"wal").expect("seed");
        let backups = dir.path().join("backups");
        let roots = ProtectedRoots::none();
        let folder = backup_files(&roots, &[&db, &wal], &backups).expect("backup");
        assert!(folder.starts_with(&backups));
        assert_eq!(
            std::fs::read(folder.join("master.db")).expect("read"),
            b"db"
        );
        assert_eq!(
            std::fs::read(folder.join("master.db-wal")).expect("read"),
            b"wal"
        );
        let again = backup_files(&roots, &[&db], &backups).expect("backup again");
        assert_ne!(folder, again);

        let protected = ProtectedRoots::new([backups.clone()]);
        assert!(matches!(
            backup_files(&protected, &[&db], &backups),
            Err(FsError::Protected { .. })
        ));
    }

    #[test]
    fn unique_path_appends_a_counter() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.crate");
        assert_eq!(unique_path(&path), path);
        std::fs::write(&path, b"").expect("seed");
        assert_eq!(unique_path(&path), dir.path().join("a (2).crate"));
    }

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(timestamp_folder_name(UNIX_EPOCH), "1970-01-01_00-00-00.000");
        let time = UNIX_EPOCH + Duration::from_millis(1_795_651_200_123);
        assert_eq!(timestamp_folder_name(time), "2026-11-26_00-00-00.123");
        let leap = UNIX_EPOCH + Duration::from_hours(264_384);
        assert_eq!(timestamp_folder_name(leap), "2000-02-29_00-00-00.000");
    }
}

#[cfg(test)]
mod create_new_tests {
    use super::*;

    #[test]
    fn creates_once_and_never_replaces() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("out.m4a");
        create_new_via_temp(&ProtectedRoots::none(), &path, |temp| {
            std::fs::write(temp, b"first").map_err(|e| FsError::io(temp, e))
        })
        .expect("create");
        assert!(
            create_new_via_temp(&ProtectedRoots::none(), &path, |temp| {
                std::fs::write(temp, b"second").map_err(|e| FsError::io(temp, e))
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&path).expect("read"), b"first");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o644);
        }
        let leftovers = std::fs::read_dir(dir.path()).expect("list").count();
        assert_eq!(leftovers, 1, "no temporary files left behind");
    }
}
