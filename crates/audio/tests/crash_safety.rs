//! Killing the process in the middle of Comment writes never damages the
//! original: the edit happens on a copy that only replaces it by rename.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::*;
use overtone_audio::tags::raw::{self, Container};
use overtone_audio::tags::{read_metadata, write_comment};
use overtone_fs::ProtectedRoots;
use tempfile::TempDir;

const CHILD_ENV: &str = "OVERTONE_CRASH_CHILD_FILE";

/// Runs only inside the child process: writes Comments until killed.
#[test]
#[ignore = "helper for crash_mid_write_leaves_a_valid_file"]
fn crash_child() {
    let Some(path) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let path = Path::new(&path);
    let mut i = 0u64;
    loop {
        write_comment(&ProtectedRoots::none(), path, Some(&format!("#n{i}"))).expect("write");
        i += 1;
    }
}

#[test]
fn crash_mid_write_leaves_a_valid_file() {
    let dir = TempDir::new().expect("tempdir");
    let mut bytes = serato_tag();
    bytes.extend_from_slice(&mpeg_frames(4000));
    let path = write(dir.path(), "big.mp3", &bytes);
    let layout = raw::inspect(&path, Container::Mp3).expect("walk");
    let audio = bytes[usize::try_from(layout.payload.start).expect("offset")..].to_vec();
    let geobs = layout.id3v2.expect("tag").frames;
    let geobs: Vec<_> = geobs.into_iter().filter(|f| f.id == "GEOB").collect();

    let mut writes_seen = 0;
    for round in 0..8u64 {
        let mut child = Command::new(std::env::current_exe().expect("exe"))
            .args(["--exact", "crash_child", "--ignored", "--nocapture"])
            .env(CHILD_ENV, &path)
            .spawn()
            .expect("spawn");
        std::thread::sleep(Duration::from_millis(40 + round * 37));
        child.kill().expect("kill");
        child.wait().expect("wait");

        let layout = raw::inspect(&path, Container::Mp3).expect("still walks");
        let now = std::fs::read(&path).expect("read");
        assert_eq!(
            &now[usize::try_from(layout.payload.start).expect("offset")..],
            audio.as_slice(),
            "audio intact after kill in round {round}"
        );
        let after: Vec<_> = layout
            .id3v2
            .expect("tag")
            .frames
            .into_iter()
            .filter(|f| f.id == "GEOB")
            .collect();
        assert_eq!(
            after, geobs,
            "GEOB frames intact after kill in round {round}"
        );
        let comment = read_metadata(&path).expect("still readable").tags.comment;
        writes_seen += usize::from(comment.is_some_and(|c| c.starts_with("#n")));
        // A kill can leave the hidden temporary copy behind; it never
        // replaces the original and is not an audio file to the scanner.
        for entry in std::fs::read_dir(dir.path()).expect("list") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            assert!(
                name == "big.mp3" || name.starts_with(".overtone-"),
                "{name}"
            );
        }
    }
    assert!(writes_seen > 0, "the child never completed a write");
}
