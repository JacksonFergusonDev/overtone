//! End-to-end checks of the CLI binary against a temporary library.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn cli(dir: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_overtone-cli"));
    command
        .env("OVERTONE_DATA_DIR", dir.path().join("data"))
        .env_remove("OVERTONE_LIBRARY");
    command
}

#[test]
fn status_json_is_the_only_stdout() {
    let dir = TempDir::new().expect("tempdir");
    let library = dir.path().join("lib.sqlite");
    let output = cli(&dir)
        .args(["--json", "--library"])
        .arg(&library)
        .arg("status")
        .output()
        .expect("run cli");
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(json["library"]["track_count"], 0);
    assert_eq!(json["library_path"], library.display().to_string());
    assert!(library.exists());
}

#[test]
fn default_library_lives_in_the_data_dir() {
    let dir = TempDir::new().expect("tempdir");
    let output = cli(&dir)
        .args(["--json", "status"])
        .output()
        .expect("run cli");
    assert!(output.status.success(), "{output:?}");
    assert!(dir.path().join("data/library.sqlite").exists());
}

#[test]
fn errors_are_json_in_json_mode() {
    let dir = TempDir::new().expect("tempdir");
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, b"").expect("seed");
    let output = cli(&dir)
        .args(["--json", "--library"])
        .arg(blocker.join("lib.sqlite"))
        .arg("status")
        .output()
        .expect("run cli");
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert!(json["error"]["code"].is_string());
}

fn run_json(dir: &TempDir, args: &[&str]) -> (bool, serde_json::Value) {
    let output = cli(dir).arg("--json").args(args).output().expect("run cli");
    let json = serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null);
    (output.status.success(), json)
}

#[test]
fn vibe_lifecycle_through_the_cli() {
    let dir = TempDir::new().expect("tempdir");
    let (ok, created) = run_json(&dir, &["vibe", "create", "Dubs/Deep"]);
    assert!(ok);
    assert_eq!(created["path"], "Dubs/Deep");

    let (ok, vibes) = run_json(&dir, &["vibe", "ls"]);
    assert!(ok);
    let names: Vec<&str> = vibes
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v["path"].as_str().expect("path"))
        .collect();
    assert_eq!(names, ["Dubs", "Dubs/Deep"]);

    let (ok, renamed) = run_json(&dir, &["vibe", "rename", "dubs", "Bass/Dub"]);
    assert!(ok, "{renamed}");
    assert_eq!(renamed["path"], "Bass/Dub");

    let (ok, err) = run_json(&dir, &["vibe", "delete", "Nope"]);
    assert!(!ok);
    assert_eq!(err["error"]["code"], "not_found");

    let (ok, err) = run_json(&dir, &["vibe", "add", "Warmup", "999"]);
    assert!(!ok);
    assert_eq!(err["error"]["code"], "not_found");

    let (ok, err) = run_json(&dir, &["vibe", "create", "a%%b"]);
    assert!(!ok);
    assert_eq!(err["error"]["code"], "invalid_vibe");
}

#[test]
fn roots_and_ls_on_an_empty_library() {
    let dir = TempDir::new().expect("tempdir");
    let music = dir.path().join("music");
    std::fs::create_dir(&music).expect("mkdir");
    let (ok, root) = run_json(&dir, &["roots", "add", music.to_str().expect("utf8")]);
    assert!(ok);
    assert!(root["path"].as_str().expect("path").ends_with("/music"));
    let (ok, roots) = run_json(&dir, &["roots", "ls"]);
    assert!(ok);
    assert_eq!(roots.as_array().expect("array").len(), 1);
    let (ok, tracks) = run_json(&dir, &["ls"]);
    assert!(ok);
    assert_eq!(tracks, serde_json::json!([]));
}

#[test]
fn serato_dry_run_writes_nothing_and_sync_writes_crates() {
    let dir = TempDir::new().expect("tempdir");
    let serato = dir.path().join("_Serato_");
    let serato_arg = serato.to_str().expect("utf8");
    let (ok, _) = run_json(&dir, &["vibe", "create", "Dubs/Deep"]);
    assert!(ok);

    let (ok, plans) = run_json(
        &dir,
        &["sync", "serato", "--dry-run", "--serato-dir", serato_arg],
    );
    assert!(ok, "{plans}");
    assert_eq!(plans.as_array().expect("plans").len(), 1);
    assert!(!serato.exists());

    let (ok, report) = run_json(&dir, &["sync", "serato", "--serato-dir", serato_arg]);
    assert!(ok, "{report}");
    assert!(serato.join("Subcrates/Overtone%%Dubs%%Deep.crate").exists());
    assert!(serato.join("Subcrates/Overtone.crate").exists());
}

#[test]
fn rekordbox_dry_run_writes_nothing_and_sync_writes_xml() {
    let dir = TempDir::new().expect("tempdir");
    let xml = dir.path().join("out/rekordbox.xml");
    let xml_arg = xml.to_str().expect("utf8");
    let (ok, _) = run_json(&dir, &["vibe", "create", "Dubs/Deep"]);
    assert!(ok);

    let (ok, plan) = run_json(&dir, &["sync", "rekordbox", "--dry-run", "--out", xml_arg]);
    assert!(ok, "{plan}");
    assert_eq!(plan["changed"], true);
    assert_eq!(plan["exists"], false);
    assert_eq!(plan["playlist_count"], 1);
    assert!(!xml.exists());

    let (ok, report) = run_json(&dir, &["sync", "rekordbox", "--out", xml_arg]);
    assert!(ok, "{report}");
    assert_eq!(report["changed"], true);
    let text = std::fs::read_to_string(&xml).expect("xml written");
    assert!(text.contains("<NODE Type=\"0\" Name=\"Overtone\" Count=\"1\">"));
    assert!(text.contains("<NODE Type=\"1\" Name=\"Deep\" KeyType=\"0\" Entries=\"0\"/>"));

    let (ok, again) = run_json(&dir, &["sync", "rekordbox", "--out", xml_arg]);
    assert!(ok, "{again}");
    assert_eq!(again["changed"], false);
}

#[test]
fn scan_indexes_fixtures_and_rescans_incrementally() {
    let dir = TempDir::new().expect("tempdir");
    let music = dir.path().join("music");
    std::fs::create_dir_all(music.join("House")).expect("mkdir");
    let signal = overtone_testkit::Signal {
        seconds: 0.2,
        ..overtone_testkit::Signal::cd(20_000.0)
    };
    for (name, fixture) in [
        ("House/a.flac", overtone_testkit::Fixture::Flac { bits: 16 }),
        ("b.wav", overtone_testkit::Fixture::Wav { bits: 16 }),
    ] {
        overtone_testkit::write_fixture(&music.join(name), &signal, fixture).expect("fixture");
    }
    let music_arg = music.to_str().expect("utf8");
    let (ok, report) = run_json(&dir, &["scan", music_arg]);
    assert!(ok, "{report}");
    assert_eq!(report["added"], 2);
    let (ok, report) = run_json(&dir, &["scan"]);
    assert!(ok, "{report}");
    assert_eq!(report["unchanged"], 2);
    let (ok, tracks) = run_json(&dir, &["ls"]);
    assert!(ok);
    assert_eq!(tracks.as_array().expect("tracks").len(), 2);
}

#[test]
fn audit_review_spectrogram_and_comment_mirroring() {
    let dir = TempDir::new().expect("tempdir");
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).expect("mkdir");
    let full = overtone_testkit::Signal::noise(22_050.0);
    let walled = overtone_testkit::Signal::noise(16_000.0);
    let fixture = overtone_testkit::Fixture::Flac { bits: 16 };
    let good = music.join("good.flac");
    let bad = music.join("bad.flac");
    overtone_testkit::write_fixture(&good, &full, fixture).expect("good");
    overtone_testkit::write_fixture(&bad, &walled, fixture).expect("bad");
    let music_arg = music.to_str().expect("utf8");

    let (ok, _) = run_json(&dir, &["scan", music_arg]);
    assert!(ok);
    let (ok, report) = run_json(&dir, &["audit"]);
    assert!(ok, "{report}");
    assert_eq!(report["analyzed"], 2);
    assert_eq!(report["flagged"].as_array().expect("flagged").len(), 1);

    let bad_arg = bad.to_str().expect("utf8");
    let (ok, flagged) = run_json(&dir, &["ls", "--needs-review"]);
    assert!(ok, "{flagged}");
    assert_eq!(flagged.as_array().expect("tracks").len(), 1);
    let (ok, reviewed) = run_json(&dir, &["review", bad_arg, "ok"]);
    assert!(ok, "{reviewed}");
    let (ok, flagged) = run_json(&dir, &["ls", "--needs-review"]);
    assert!(ok);
    assert_eq!(flagged, serde_json::json!([]));

    let png = dir.path().join("bad.png");
    let (ok, spec) = run_json(
        &dir,
        &["spectrogram", bad_arg, "--out", png.to_str().expect("utf8")],
    );
    assert!(ok, "{spec}");
    assert!(png.exists());

    let good_arg = good.to_str().expect("utf8");
    let (ok, _) = run_json(&dir, &["vibe", "add", "Dubs/Deep", good_arg]);
    assert!(ok);
    let before = std::fs::read(&good).expect("read");
    let (ok, preview) = run_json(&dir, &["tag", "mirror"]);
    assert!(ok, "{preview}");
    assert_eq!(preview[0]["after"], "#dubs/deep");
    assert_eq!(
        std::fs::read(&good).expect("read"),
        before,
        "preview writes nothing"
    );

    let (ok, written) = run_json(&dir, &["tag", "mirror", "--write"]);
    assert!(ok, "{written}");
    assert_eq!(written["written"].as_array().expect("written").len(), 1);
    let (ok, rescan) = run_json(&dir, &["scan"]);
    assert!(ok);
    assert_eq!(rescan["updated"], 0, "written files don't look changed");

    let (ok, restored) = run_json(&dir, &["tag", "restore", "--write"]);
    assert!(ok, "{restored}");
    let (ok, show) = run_json(&dir, &["show", good_arg]);
    assert!(ok);
    assert_eq!(show["track"]["tags"]["comment"], serde_json::Value::Null);
}

#[test]
fn rekordbox_tags_dry_run_then_sync_into_a_fixture_db() {
    use overtone_sync_rekordbox::master_db::{MasterDb, fixture};

    let dir = TempDir::new().expect("tempdir");
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).expect("mkdir");
    let signal = overtone_testkit::Signal {
        seconds: 0.2,
        ..overtone_testkit::Signal::cd(20_000.0)
    };
    for name in ["a.flac", "b.flac"] {
        overtone_testkit::write_fixture(
            &music.join(name),
            &signal,
            overtone_testkit::Fixture::Flac { bits: 16 },
        )
        .expect("fixture");
    }
    let (ok, _) = run_json(&dir, &["scan", music.to_str().expect("utf8")]);
    assert!(ok);
    let (ok, tracks) = run_json(&dir, &["ls"]);
    assert!(ok);
    let paths: Vec<String> = tracks
        .as_array()
        .expect("tracks")
        .iter()
        .map(|t| t["path"].as_str().expect("path").to_string())
        .collect();
    assert_eq!(paths.len(), 2);
    // Rekordbox knows only the first track. The paths are what the scan
    // stored, so they match however the temporary folder resolves.
    let db = fixture::create_live(&dir.path().join("Pioneer"), &[("11", &paths[0])]).expect("db");
    let db_arg = db.to_str().expect("utf8");
    for path in &paths {
        let (ok, out) = run_json(&dir, &["vibe", "add", "Dubs/Deep", path]);
        assert!(ok, "{out}");
    }

    let (ok, plan) = run_json(
        &dir,
        &["sync", "rekordbox-tags", "--dry-run", "--db", db_arg],
    );
    assert!(ok, "{plan}");
    assert_eq!(plan["changed"], true);
    assert_eq!(plan["create_category"], true);
    assert_eq!(plan["added"], 1);
    assert_eq!(plan["skipped_tracks"].as_array().expect("skipped").len(), 1);
    assert!(
        db.with_file_name("master.db-wal").exists(),
        "a dry run only copies the database"
    );

    if overtone_sync_rekordbox::rekordbox_running()
        || overtone_sync_rekordbox::rekordbox_agent_running()
    {
        eprintln!("Rekordbox is running here; skipping the write half of this test");
        return;
    }
    let (ok, report) = run_json(&dir, &["sync", "rekordbox-tags", "--db", db_arg]);
    assert!(ok, "{report}");
    assert_eq!(report["changed"], true);
    assert_eq!(
        report["created_tags"],
        serde_json::json!(["Dubs", "Dubs/Deep"])
    );
    let backup = PathBuf::from(report["backup_dir"].as_str().expect("backup"));
    assert!(backup.starts_with(dir.path().join("data/backups/rekordbox-db")));
    assert!(backup.join("master.db-wal").exists());
    assert!(!db.with_file_name("master.db-wal").exists());

    let copy_dir = TempDir::new().expect("tempdir");
    let copy = copy_dir.path().join("master.db");
    std::fs::copy(&db, &copy).expect("copy");
    let opened = MasterDb::open(&copy).expect("open");
    let names: Vec<String> = opened
        .overtone_tags()
        .expect("tags")
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["Dubs", "Dubs/Deep"]);

    let (ok, again) = run_json(&dir, &["sync", "rekordbox-tags", "--db", db_arg]);
    assert!(ok, "{again}");
    assert_eq!(again["changed"], false);
}
