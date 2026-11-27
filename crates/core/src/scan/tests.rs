use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use overtone_library::Library;
use overtone_model::AudioFormat;
use overtone_testkit::{Fixture, Signal, write_fixture};
use tempfile::TempDir;

use super::{ScanProgress, ScanReport, add_root, scan_library, scan_roots};

struct Setup {
    _dir: TempDir,
    music: PathBuf,
    outside: PathBuf,
    library: Library,
}

fn signal(seed: u64) -> Signal {
    Signal {
        seconds: 0.1,
        seed,
        ..Signal::cd(18_000.0)
    }
}

fn write(path: &Path, seed: u64, fixture: Fixture) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    write_fixture(path, &signal(seed), fixture).expect("fixture");
}

fn set_mtime(path: &Path, time: SystemTime) {
    File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(time)
        .expect("set mtime");
}

/// A temp folder with a library, a `music` root holding three audio files
/// among things the scan must skip, and an `outside` folder.
fn setup() -> Setup {
    let dir = TempDir::new().expect("tempdir");
    // Canonical, so paths match what the scanner stores (on macOS the temp
    // dir sits behind the /var -> /private/var symlink).
    let base = std::fs::canonicalize(dir.path()).expect("canonical");
    let music = base.join("music");
    write(&music.join("a.flac"), 1, Fixture::Flac { bits: 16 });
    write(&music.join("sub/b.wav"), 2, Fixture::Wav { bits: 16 });
    write(
        &music.join("sub/deeper/c.aiff"),
        3,
        Fixture::Aiff { bits: 16 },
    );
    // Skipped: hidden folders, AppleDouble files, system folders, non-audio.
    write(&music.join(".hidden/d.flac"), 4, Fixture::Flac { bits: 16 });
    write(
        &music.join(".Trashes/501/e.wav"),
        5,
        Fixture::Wav { bits: 16 },
    );
    std::fs::write(music.join("._a.flac"), b"\x00\x05\x16\x07AppleDouble").expect("._");
    std::fs::write(music.join("sub/cover.jpg"), b"\xFF\xD8\xFF").expect("jpg");
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let library = Library::open(&base.join("library.sqlite")).expect("library");
    Setup {
        _dir: dir,
        music,
        outside,
        library,
    }
}

fn scan(setup: &mut Setup) -> ScanReport {
    let roots = [setup.music.clone()];
    scan_roots(&mut setup.library, &roots, &|_| {}).expect("scan")
}

fn track(library: &Library, path: &Path) -> overtone_model::Track {
    library
        .track_by_path(path.to_str().expect("utf-8"))
        .expect("query")
        .unwrap_or_else(|| panic!("no track for {}", path.display()))
}

#[test]
fn first_scan_adds_audio_and_skips_hidden_and_other_files() {
    let mut setup = setup();
    let report = scan(&mut setup);
    assert_eq!(
        (
            report.added,
            report.updated,
            report.unchanged,
            report.missing
        ),
        (3, 0, 0, 0)
    );
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.unavailable_roots.is_empty());

    let tracks = setup.library.tracks().expect("tracks");
    let paths: Vec<&str> = tracks.iter().map(|t| t.path.as_str()).collect();
    let expected: Vec<String> = ["a.flac", "sub/b.wav", "sub/deeper/c.aiff"]
        .iter()
        .map(|rel| setup.music.join(rel).to_str().expect("utf-8").to_owned())
        .collect();
    assert_eq!(paths, expected);
    let formats: Vec<_> = tracks.iter().map(|t| t.properties.format).collect();
    assert_eq!(
        formats,
        [
            Some(AudioFormat::Flac),
            Some(AudioFormat::Wav),
            Some(AudioFormat::Aiff)
        ]
    );
    for track in &tracks {
        let hash = track.audio_hash.as_deref().expect("hashed");
        assert!(hash.starts_with("blake3-payload-v1:"), "{hash}");
        assert!(track.file_size > 0 && track.modified_at > 0);
    }
}

#[test]
fn rescans_are_incremental() {
    let mut setup = setup();
    scan(&mut setup);
    let report = scan(&mut setup);
    assert_eq!(
        (
            report.added,
            report.updated,
            report.unchanged,
            report.missing
        ),
        (0, 0, 3, 0)
    );

    // New audio, same size: only the modification time reveals the change.
    let b = setup.music.join("sub/b.wav");
    let before = track(&setup.library, &b);
    write(&b, 99, Fixture::Wav { bits: 16 });
    set_mtime(&b, SystemTime::now() + Duration::from_secs(5));
    // A touch without content changes is re-read too, and keeps its hash.
    let a = setup.music.join("a.flac");
    let a_before = track(&setup.library, &a);
    set_mtime(&a, SystemTime::now() + Duration::from_secs(7));

    let report = scan(&mut setup);
    assert_eq!((report.added, report.updated, report.unchanged), (0, 2, 1));
    let after = track(&setup.library, &b);
    assert_eq!(after.id, before.id);
    assert_eq!(after.file_size, before.file_size);
    assert_ne!(after.audio_hash, before.audio_hash);
    assert_eq!(track(&setup.library, &a).audio_hash, a_before.audio_hash);
}

#[test]
fn deleted_files_are_marked_missing_and_found_again() {
    let mut setup = setup();
    scan(&mut setup);
    let c = setup.music.join("sub/deeper/c.aiff");
    let parked = setup.outside.join("c.aiff");
    // Renaming keeps size and mtime, so coming back is "unchanged".
    std::fs::rename(&c, &parked).expect("move away");

    let report = scan(&mut setup);
    assert_eq!((report.unchanged, report.missing), (2, 1));
    let missing_since = track(&setup.library, &c).missing_since.expect("missing");
    assert_eq!(setup.library.summary().expect("summary").missing_count, 1);

    // Still gone: still counted, and the first-missing time is kept.
    let report = scan(&mut setup);
    assert_eq!(report.missing, 1);
    assert_eq!(track(&setup.library, &c).missing_since, Some(missing_since));

    std::fs::rename(&parked, &c).expect("move back");
    let report = scan(&mut setup);
    assert_eq!((report.unchanged, report.missing), (3, 0));
    assert_eq!(track(&setup.library, &c).missing_since, None);

    // Deleted and recreated with new content: updated, and found again.
    std::fs::remove_file(&c).expect("delete");
    scan(&mut setup);
    write(&c, 42, Fixture::Aiff { bits: 16 });
    set_mtime(&c, SystemTime::now() + Duration::from_secs(9));
    let report = scan(&mut setup);
    assert_eq!((report.updated, report.missing), (1, 0));
    assert_eq!(track(&setup.library, &c).missing_since, None);
}

#[test]
fn unplugged_roots_are_reported_and_their_tracks_kept() {
    let mut setup = setup();
    let drive = setup.outside.join("drive");
    write(&drive.join("x.wav"), 7, Fixture::Wav { bits: 16 });
    let roots = [setup.music.clone(), drive.clone()];
    let report = scan_roots(&mut setup.library, &roots, &|_| {}).expect("scan");
    assert_eq!(report.added, 4);

    let unplugged = setup.outside.join("unplugged");
    std::fs::rename(&drive, &unplugged).expect("unplug");
    let report = scan_roots(&mut setup.library, &roots, &|_| {}).expect("scan");
    assert_eq!(report.missing, 0);
    assert_eq!(
        report.unavailable_roots,
        [drive.to_str().expect("utf-8").to_owned()]
    );
    assert_eq!(
        track(&setup.library, &drive.join("x.wav")).missing_since,
        None
    );
}

#[test]
fn unreadable_audio_is_stored_and_reported() {
    let mut setup = setup();
    let broken = setup.music.join("broken.mp3");
    std::fs::write(&broken, b"this is not an mp3").expect("write");
    let report = scan(&mut setup);
    assert_eq!(report.added, 4);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].path, broken.to_str().expect("utf-8"));
    assert!(
        report.failed[0].reason.contains("audio hash"),
        "{:?}",
        report.failed
    );
    let stored = track(&setup.library, &broken);
    assert_eq!(stored.audio_hash, None);
    // No metadata and no recognizable stream: the extension decides.
    assert_eq!(stored.properties.format, Some(AudioFormat::Mp3));
}

#[test]
fn reports_progress_through_every_phase() {
    let mut setup = setup();
    let events = Mutex::new(Vec::new());
    let roots = [setup.music.clone()];
    scan_roots(&mut setup.library, &roots, &|event| {
        events.lock().expect("lock").push(event);
    })
    .expect("scan");
    let events = events.into_inner().expect("lock");
    assert_eq!(events.first(), Some(&ScanProgress::Walking { found: 3 }));
    assert!(events.contains(&ScanProgress::Processing { done: 3, total: 3 }));
    assert_eq!(
        events.last(),
        Some(&ScanProgress::Saving { done: 3, total: 3 })
    );
}

#[test]
fn add_root_stores_the_canonical_folder_and_scan_library_uses_it() {
    let mut setup = setup();
    let spelled = setup.music.join("sub/../");
    let root = add_root(&setup.library, &spelled).expect("add");
    assert_eq!(root.path, setup.music.to_str().expect("utf-8"));
    let report = scan_library(&mut setup.library, &|_| {}).expect("scan");
    assert_eq!(report.added, 3);

    let err = add_root(&setup.library, &setup.outside.join("nope")).expect_err("missing");
    assert_eq!(err.code(), "invalid_root");
    let err = add_root(&setup.library, &setup.music.join("a.flac")).expect_err("file");
    assert_eq!(err.code(), "invalid_root");
}

#[test]
fn scanning_never_modifies_audio_files() {
    let mut setup = setup();
    let a = setup.music.join("a.flac");
    let bytes = std::fs::read(&a).expect("read");
    let mtime = std::fs::metadata(&a)
        .and_then(|m| m.modified())
        .expect("mtime");
    scan(&mut setup);
    scan(&mut setup);
    assert_eq!(std::fs::read(&a).expect("read"), bytes);
    assert_eq!(
        std::fs::metadata(&a)
            .and_then(|m| m.modified())
            .expect("mtime"),
        mtime
    );
}
