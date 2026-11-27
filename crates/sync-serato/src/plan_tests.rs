use super::*;
use crate::testutil::Fixture;

fn want(fx: &Fixture, segments: &[&str], tracks: &[&str]) -> DesiredCrate {
    DesiredCrate {
        segments: segments.iter().map(|s| (*s).to_owned()).collect(),
        tracks: tracks.iter().map(|t| fx.track(t)).collect(),
    }
}

fn make_plan(fx: &Fixture, desired: &[DesiredCrate]) -> SeratoSyncPlan {
    plan_sync(&fx.serato, &fx.home, DEFAULT_ROOT_CRATE, desired).unwrap()
}

fn action<'a>(plan: &'a SeratoSyncPlan, filename: &str) -> &'a FileAction {
    &plan
        .files
        .iter()
        .find(|f| f.filename == filename)
        .unwrap_or_else(|| panic!("no plan for {filename}"))
        .action
}

fn new_tracks(plan: &SeratoSyncPlan, filename: &str) -> Vec<String> {
    let file = plan.files.iter().find(|f| f.filename == filename).unwrap();
    CrateFile::parse(file.new_bytes.as_ref().unwrap())
        .unwrap()
        .track_paths()
        .unwrap()
}

#[test]
fn creates_vibe_crates_and_every_parent() {
    let fx = Fixture::new(&["Pop"]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs", "Deep"], &["a.flac", "b.flac"])]);
    let names: Vec<_> = plan.files.iter().map(|f| f.filename.as_str()).collect();
    assert_eq!(
        names,
        [
            "Overtone.crate",
            "Overtone%%Dubs.crate",
            "Overtone%%Dubs%%Deep.crate"
        ]
    );
    assert_eq!(
        *action(&plan, "Overtone%%Dubs%%Deep.crate"),
        FileAction::Create { tracks: 2 }
    );
    assert_eq!(
        *action(&plan, "Overtone.crate"),
        FileAction::Create { tracks: 0 }
    );
    assert_eq!(
        plan.neworder.added,
        ["Overtone", "Overtone%%Dubs", "Overtone%%Dubs%%Deep"]
    );
    assert!(plan.has_changes());
    assert!(fx.list().is_empty(), "planning writes nothing");
}

#[test]
fn update_keeps_order_appends_new_and_drops_removed() {
    let fx = Fixture::new(&["Overtone", "Overtone%%Dubs"]);
    fx.write_crate("Overtone.crate", &[]);
    fx.write_crate(
        "Overtone%%Dubs.crate",
        &[&fx.ptrk("a.flac"), &fx.ptrk("b.flac"), &fx.ptrk("c.flac")],
    );
    let plan = make_plan(
        &fx,
        &[want(&fx, &["Dubs"], &["c.flac", "d.flac", "a.flac"])],
    );
    assert_eq!(
        *action(&plan, "Overtone%%Dubs.crate"),
        FileAction::Update {
            added: 1,
            removed: 1
        }
    );
    assert_eq!(*action(&plan, "Overtone.crate"), FileAction::Unchanged);
    assert_eq!(
        new_tracks(&plan, "Overtone%%Dubs.crate"),
        [fx.ptrk("a.flac"), fx.ptrk("c.flac"), fx.ptrk("d.flac")]
    );
    assert!(plan.neworder.added.is_empty());
    assert!(plan.neworder.new_bytes.is_none());
}

#[test]
fn update_preserves_header_and_unknown_records() {
    let fx = Fixture::new(&["Overtone", "Overtone%%A"]);
    fx.write_crate("Overtone.crate", &[]);
    let mut file = CrateFile::new();
    file.records
        .insert(2, Record::data(*b"zzzz", vec![1, 2, 3]));
    file.set_track_paths(&[fx.ptrk("a.flac")]);
    std::fs::write(fx.crate_path("Overtone%%A.crate"), file.to_bytes().unwrap()).unwrap();

    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac", "b.flac"])]);
    let written = plan
        .files
        .iter()
        .find(|f| f.filename == "Overtone%%A.crate")
        .unwrap()
        .new_bytes
        .clone()
        .unwrap();
    let parsed = CrateFile::parse(&written).unwrap();
    assert_eq!(
        parsed.records[..parsed.records.len() - 2],
        file.records[..file.records.len() - 1]
    );
    assert_eq!(parsed.track_paths().unwrap().len(), 2);
}

#[test]
fn unchanged_when_tracks_match() {
    let fx = Fixture::new(&["Overtone", "Overtone%%Dubs"]);
    fx.write_crate("Overtone.crate", &[]);
    fx.write_crate("Overtone%%Dubs.crate", &[&fx.ptrk("a.flac")]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs"], &["a.flac", "a.flac"])]);
    assert!(!plan.has_changes());
    assert!(plan.files.iter().all(|f| f.action == FileAction::Unchanged));
}

#[test]
fn parents_that_only_need_to_exist_keep_their_tracks() {
    let fx = Fixture::new(&["Overtone", "Overtone%%Dubs"]);
    fx.write_crate("Overtone.crate", &[&fx.ptrk("manual.flac")]);
    fx.write_crate("Overtone%%Dubs.crate", &[]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs"], &[])]);
    assert_eq!(*action(&plan, "Overtone.crate"), FileAction::Unchanged);
    assert!(!plan.has_changes());
}

#[test]
fn matches_existing_tracks_across_unicode_forms_without_rewriting() {
    let fx = Fixture::new(&["Overtone", "Overtone%%Dubs"]);
    fx.write_crate("Overtone.crate", &[]);
    let nfd = fx.ptrk("Beyonce\u{301}/a.flac");
    fx.write_crate("Overtone%%Dubs.crate", &[&nfd]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs"], &["Beyonc\u{e9}/a.flac"])]);
    assert!(!plan.has_changes(), "NFC and NFD name the same track");
}

#[test]
fn deletes_only_managed_crates_that_are_no_longer_wanted() {
    let fx = Fixture::new(&[
        "Dubs",
        "Overtone",
        "Overtone%%Old",
        "Overtone%%Old%%Child",
        "Overtone%%Keep",
    ]);
    fx.write_crate("Dubs.crate", &[&fx.ptrk("user.flac")]);
    fx.write_crate("Overtone.crate", &[]);
    fx.write_crate("Overtone%%Old.crate", &[&fx.ptrk("x.flac")]);
    fx.write_crate("Overtone%%Old%%Child.crate", &[]);
    fx.write_crate("Overtone%%Keep.crate", &[&fx.ptrk("k.flac")]);
    fx.write_crate("Overtoned.crate", &[]);
    let plan = make_plan(&fx, &[want(&fx, &["Keep"], &["k.flac"])]);
    let deletes: Vec<_> = plan
        .files
        .iter()
        .filter(|f| matches!(f.action, FileAction::Delete { .. }))
        .map(|f| f.filename.as_str())
        .collect();
    assert_eq!(
        deletes,
        ["Overtone%%Old%%Child.crate", "Overtone%%Old.crate"],
        "children before parents"
    );
    assert_eq!(
        *action(&plan, "Overtone%%Old.crate"),
        FileAction::Delete { tracks: 1 }
    );
    assert!(plan.files.iter().all(|f| f.filename != "Dubs.crate"));
    assert!(plan.files.iter().all(|f| f.filename != "Overtoned.crate"));
    assert_eq!(
        plan.neworder.removed,
        ["Overtone%%Old%%Child", "Overtone%%Old"]
    );
}

#[test]
fn a_user_crate_with_the_same_name_outside_the_namespace_is_untouched() {
    let fx = Fixture::new(&["Dubs", "Dubs%%Deep"]);
    fx.write_crate("Dubs.crate", &[&fx.ptrk("mine.flac")]);
    fx.write_crate("Dubs%%Deep.crate", &[&fx.ptrk("mine2.flac")]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs", "Deep"], &["a.flac"])]);
    for file in &plan.files {
        assert!(file.filename.starts_with("Overtone"), "{}", file.filename);
    }
    assert!(
        plan.neworder
            .added
            .iter()
            .all(|name| name.starts_with("Overtone"))
    );
    assert!(plan.neworder.removed.is_empty());
}

#[test]
fn existing_file_name_spelling_wins_over_case_differences() {
    let fx = Fixture::new(&["overtone"]);
    fx.write_crate("overtone.crate", &[]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac"])]);
    assert_eq!(*action(&plan, "overtone.crate"), FileAction::Unchanged);
    assert!(plan.files.iter().any(|f| f.filename == "Overtone%%A.crate"));
}

#[test]
fn duplicate_vibes_merge_their_tracks() {
    let fx = Fixture::new(&[]);
    let plan = make_plan(
        &fx,
        &[
            want(&fx, &["Dubs"], &["a.flac"]),
            want(&fx, &["dubs"], &["b.flac", "a.flac"]),
        ],
    );
    assert_eq!(
        new_tracks(&plan, "Overtone%%Dubs.crate"),
        [fx.ptrk("a.flac"), fx.ptrk("b.flac")]
    );
}

#[test]
fn tracks_on_other_volumes_are_reported_not_dropped() {
    let fx = Fixture::new(&[]);
    let mut spec = want(&fx, &["Dubs"], &["a.flac"]);
    spec.tracks
        .push(PathBuf::from("/Volumes/Big Disk/Music/b.flac"));
    let plan = make_plan(&fx, &[spec]);
    assert_eq!(plan.skipped_tracks.len(), 1);
    assert_eq!(
        plan.skipped_tracks[0].serato_dir,
        Path::new("/Volumes/Big Disk/_Serato_")
    );
    assert_eq!(plan.skipped_tracks[0].crate_name, "Overtone%%Dubs");
    assert_eq!(
        *action(&plan, "Overtone%%Dubs.crate"),
        FileAction::Create { tracks: 1 }
    );
    assert!(!plan.warnings.is_empty());
}

#[test]
fn external_volume_plans_use_volume_relative_paths() {
    let fx = Fixture::new(&[]);
    let spec = DesiredCrate {
        segments: vec!["A".to_owned()],
        tracks: vec![PathBuf::from("/Volumes/X/Music/a.flac")],
    };
    let wrong_volume = fx.dir.path().join("Volumes/X/_Serato_");
    let plan = plan_sync(
        &wrong_volume,
        &fx.home,
        "Overtone",
        std::slice::from_ref(&spec),
    )
    .unwrap();
    assert_eq!(plan.skipped_tracks.len(), 1);
    // Planning is read-only, so a path under /Volumes is safe to plan for.
    let volume = PathBuf::from("/Volumes/X/_Serato_");
    let plan = plan_sync(&volume, &fx.home, "Overtone", &[spec]).unwrap();
    assert!(plan.skipped_tracks.is_empty());
    assert_eq!(new_tracks(&plan, "Overtone%%A.crate"), ["Music/a.flac"]);
}

#[test]
fn rejects_bad_names_and_custom_roots_work() {
    let fx = Fixture::new(&[]);
    assert!(plan_sync(&fx.serato, &fx.home, "Over%%tone", &[]).is_err());
    let bad = want(&fx, &["a/b"], &[]);
    assert!(plan_sync(&fx.serato, &fx.home, "Overtone", &[bad]).is_err());
    let empty = want(&fx, &[], &[]);
    assert!(plan_sync(&fx.serato, &fx.home, "Overtone", &[empty]).is_err());
    let ok = want(&fx, &["A"], &[]);
    let plan = plan_sync(&fx.serato, &fx.home, "My Vibes", &[ok]).unwrap();
    assert_eq!(plan.files[1].filename, "My Vibes%%A.crate");
}

#[test]
fn missing_neworder_is_warned_not_created() {
    let fx = Fixture::new(&[]);
    std::fs::remove_file(fx.serato.join("neworder.pref")).unwrap();
    let plan = make_plan(&fx, &[want(&fx, &["A"], &[])]);
    assert!(!plan.neworder.exists);
    assert!(plan.neworder.new_bytes.is_none());
    assert!(plan.warnings.iter().any(|w| w.contains("neworder.pref")));
}

#[test]
fn unreadable_neworder_is_left_alone() {
    let fx = Fixture::new(&[]);
    std::fs::write(fx.serato.join("neworder.pref"), b"junk").unwrap();
    let plan = make_plan(&fx, &[want(&fx, &["A"], &[])]);
    assert!(plan.neworder.new_bytes.is_none());
    assert!(plan.warnings.iter().any(|w| w.contains("left alone")));
}

#[test]
fn missing_subcrates_folder_plans_creates() {
    let fx = Fixture::new(&[]);
    std::fs::remove_dir_all(fx.serato.join("Subcrates")).unwrap();
    let plan = make_plan(&fx, &[want(&fx, &["A"], &[])]);
    assert!(
        plan.files
            .iter()
            .all(|f| matches!(f.action, FileAction::Create { .. }))
    );
}

#[test]
fn corrupt_managed_crate_fails_the_plan() {
    let fx = Fixture::new(&[]);
    std::fs::write(fx.crate_path("Overtone%%A.crate"), b"vrsn\0\0\0\x09").unwrap();
    let err = plan_sync(
        &fx.serato,
        &fx.home,
        "Overtone",
        &[want(&fx, &["A"], &["a.flac"])],
    )
    .unwrap_err();
    assert!(matches!(err, SeratoError::InvalidFormat(_)));
}

#[test]
fn plan_serializes_without_file_contents() {
    let fx = Fixture::new(&[]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac"])]);
    let json = serde_json::to_value(&plan).unwrap();
    assert_eq!(json["files"][1]["action"]["action"], "create");
    assert!(json["files"][1].get("new_bytes").is_none());
}
