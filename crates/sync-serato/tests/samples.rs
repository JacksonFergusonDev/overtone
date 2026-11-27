//! Checks against a real `_Serato_` folder. Ignored by default.
//!
//! Run manually with a copy of a real library (the test only reads):
//!
//! ```sh
//! OVERTONE_SERATO_SAMPLES=/path/to/copy/_Serato_ \
//!     cargo test -p overtone-sync-serato --test samples -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use overtone_fs::ProtectedRoots;
use overtone_sync_serato::{
    CrateFile, DesiredCrate, FileAction, NewOrder, RunningCheck, apply, parse_crate_filename,
    plan_sync,
};

fn samples_dir() -> PathBuf {
    let dir = std::env::var_os("OVERTONE_SERATO_SAMPLES")
        .expect("set OVERTONE_SERATO_SAMPLES to a _Serato_ folder");
    PathBuf::from(dir)
}

#[test]
#[ignore = "needs OVERTONE_SERATO_SAMPLES"]
fn real_crates_round_trip_byte_for_byte() {
    let subcrates = samples_dir().join("Subcrates");
    let (mut crates, mut tracks, mut with_osrt) = (0, 0, 0);
    for entry in std::fs::read_dir(&subcrates).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if parse_crate_filename(&name).is_none() {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let parsed = CrateFile::parse(&bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(parsed.to_bytes().unwrap(), bytes, "{name} round trip");
        assert!(parsed.version().unwrap().is_some(), "{name} has vrsn");
        assert!(!parsed.columns().unwrap().is_empty(), "{name} has columns");
        if parsed.sort_order().unwrap().is_some() {
            with_osrt += 1;
        }
        tracks += parsed.track_paths().unwrap().len();
        crates += 1;
    }
    println!("{crates} crates, {tracks} tracks, {with_osrt} with osrt: all byte-identical");
    assert!(crates > 0, "no crates found in {}", subcrates.display());
}

#[test]
#[ignore = "needs OVERTONE_SERATO_SAMPLES"]
fn real_neworder_round_trips_and_lists_every_crate() {
    let dir = samples_dir();
    let bytes = std::fs::read(dir.join("neworder.pref")).unwrap();
    let order = NewOrder::parse(&bytes).unwrap();
    assert_eq!(order.to_bytes(), bytes);
    let files = std::fs::read_dir(dir.join("Subcrates"))
        .unwrap()
        .filter(|entry| {
            parse_crate_filename(&entry.as_ref().unwrap().file_name().to_string_lossy()).is_some()
        })
        .count();
    println!(
        "neworder.pref lists {} crates, {files} files",
        order.crates().len()
    );
    assert_eq!(order.crates().len(), files);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Copies the samples into a temporary home, syncs three vibes made from
/// real track paths, and checks the result. The source is only read.
#[test]
#[ignore = "needs OVERTONE_SERATO_SAMPLES"]
fn end_to_end_sync_into_a_copy() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let serato = home.join("Music/_Serato_");
    copy_dir(&samples_dir(), &serato);
    let subcrates = serato.join("Subcrates");

    // Real track paths from three different existing crates.
    let mut names: Vec<String> = std::fs::read_dir(&subcrates)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| parse_crate_filename(n).is_some())
        .collect();
    names.sort();
    let before: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|n| (n.clone(), std::fs::read(subcrates.join(n)).unwrap()))
        .collect();
    let mut pools: Vec<Vec<std::path::PathBuf>> = Vec::new();
    for (_, bytes) in &before {
        let paths = CrateFile::parse(bytes).unwrap().track_paths().unwrap();
        if paths.len() >= 6 && pools.len() < 3 {
            pools.push(
                paths
                    .iter()
                    .take(6)
                    .map(|p| std::path::PathBuf::from(format!("/{p}")))
                    .collect(),
            );
        }
    }
    assert_eq!(pools.len(), 3, "need three crates with six tracks");
    let desired = vec![
        DesiredCrate {
            segments: vec!["Dubs".into(), "Deep".into()],
            tracks: pools[0].clone(),
        },
        DesiredCrate {
            segments: vec!["Dubs".into(), "Light".into()],
            tracks: pools[1][..3].to_vec(),
        },
        DesiredCrate {
            segments: vec!["Warmup".into()],
            tracks: pools[2].clone(),
        },
    ];

    let plan = plan_sync(&serato, &home, "Overtone", &desired).unwrap();
    println!("{}", serde_json::to_string_pretty(&plan).unwrap());
    assert!(plan.skipped_tracks.is_empty());
    assert!(
        plan.files
            .iter()
            .all(|f| matches!(f.action, FileAction::Create { .. }))
    );
    let report = apply(
        &plan,
        &ProtectedRoots::none(),
        &temp.path().join("backups"),
        RunningCheck::Skip,
    )
    .unwrap();
    println!("{report:#?}");
    assert_eq!(
        report.created.len(),
        5,
        "Overtone, Dubs, Deep, Light, Warmup"
    );

    // Existing crates are byte-identical.
    for (name, bytes) in &before {
        assert_eq!(
            &std::fs::read(subcrates.join(name)).unwrap(),
            bytes,
            "{name}"
        );
    }
    // New crates parse and carry the right tracks.
    for file in &plan.files {
        let bytes = std::fs::read(subcrates.join(&file.filename)).unwrap();
        let parsed = CrateFile::parse(&bytes).unwrap();
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
        assert_eq!(parsed.track_paths().unwrap().len(), file.track_count);
    }
    let deep =
        CrateFile::parse(&std::fs::read(subcrates.join("Overtone%%Dubs%%Deep.crate")).unwrap())
            .unwrap();
    assert_eq!(deep.track_paths().unwrap().len(), 6);
    // neworder.pref keeps every old entry in place and adds the new ones.
    let order = NewOrder::parse(&std::fs::read(serato.join("neworder.pref")).unwrap()).unwrap();
    let original =
        NewOrder::parse(&std::fs::read(samples_dir().join("neworder.pref")).unwrap()).unwrap();
    let kept: Vec<&str> = order
        .crates()
        .into_iter()
        .filter(|n| !n.starts_with("Overtone"))
        .collect();
    assert_eq!(kept, original.crates());
    let tail: Vec<&str> = order
        .crates()
        .into_iter()
        .skip(original.crates().len())
        .collect();
    println!("new neworder tail: {tail:?}");
    assert_eq!(
        tail,
        [
            "Overtone",
            "Overtone%%Dubs",
            "Overtone%%Dubs%%Deep",
            "Overtone%%Dubs%%Light",
            "Overtone%%Warmup"
        ]
    );
    // A second plan finds nothing to do.
    let again = plan_sync(&serato, &home, "Overtone", &desired).unwrap();
    assert!(!again.has_changes());
    // Dropping a vibe plans a delete and one backup folder holds the originals.
    let fewer = &desired[..2];
    let drop_plan = plan_sync(&serato, &home, "Overtone", fewer).unwrap();
    let deleted: Vec<_> = drop_plan
        .files
        .iter()
        .filter(|f| matches!(f.action, FileAction::Delete { .. }))
        .map(|f| f.filename.as_str())
        .collect();
    assert_eq!(deleted, ["Overtone%%Warmup.crate"]);
    let report = apply(
        &drop_plan,
        &ProtectedRoots::none(),
        &temp.path().join("backups"),
        RunningCheck::Skip,
    )
    .unwrap();
    assert!(
        report
            .backup_dir
            .unwrap()
            .join("Overtone%%Warmup.crate")
            .exists()
    );
    assert!(!subcrates.join("Overtone%%Warmup.crate").exists());
}
