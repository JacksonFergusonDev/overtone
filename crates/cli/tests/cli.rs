//! End-to-end checks of the CLI binary against a temporary library.

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
