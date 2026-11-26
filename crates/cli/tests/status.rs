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
