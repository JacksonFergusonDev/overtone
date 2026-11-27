use super::*;
use crate::codec::CrateFile;
use crate::neworder::NewOrder;
use crate::plan::{DEFAULT_ROOT_CRATE, DesiredCrate, plan_sync};
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

fn order(fx: &Fixture) -> Vec<String> {
    let bytes = std::fs::read(fx.serato.join("neworder.pref")).unwrap();
    NewOrder::parse(&bytes)
        .unwrap()
        .crates()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn backups(fx: &Fixture) -> PathBuf {
    fx.dir.path().join("backups")
}

#[test]
fn creates_files_and_updates_neworder() {
    let fx = Fixture::new(&["Pop"]);
    fx.write_crate("Pop.crate", &[&fx.ptrk("pop.flac")]);
    let plan = make_plan(&fx, &[want(&fx, &["Dubs", "Deep"], &["a.flac", "b.flac"])]);
    let report = apply(
        &plan,
        &ProtectedRoots::none(),
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap();
    assert_eq!(report.created.len(), 3);
    assert!(report.neworder_written);
    assert_eq!(
        fx.read_tracks("Overtone%%Dubs%%Deep.crate"),
        [fx.ptrk("a.flac"), fx.ptrk("b.flac")]
    );
    assert_eq!(fx.read_tracks("Pop.crate"), [fx.ptrk("pop.flac")]);
    assert_eq!(
        order(&fx),
        ["Pop", "Overtone", "Overtone%%Dubs", "Overtone%%Dubs%%Deep"]
    );
    // Only neworder.pref existed to back up.
    let folder = report.backup_dir.unwrap();
    assert!(folder.starts_with(backups(&fx)));
    let saved: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
    assert_eq!(saved.len(), 1);
    // No temporary files left behind.
    assert!(
        fx.list()
            .iter()
            .all(|name| name.contains(".crate") && !name.contains(".tmp"))
    );
}

#[test]
fn applying_twice_is_a_no_op() {
    let fx = Fixture::new(&[]);
    let desired = [want(&fx, &["A"], &["a.flac"])];
    let roots = ProtectedRoots::none();
    apply(
        &make_plan(&fx, &desired),
        &roots,
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap();
    let again = make_plan(&fx, &desired);
    assert!(!again.has_changes());
    let report = apply(&again, &roots, &backups(&fx), RunningCheck::Skip).unwrap();
    assert!(report.backup_dir.is_none());
    assert_eq!(report.unchanged, 2);
}

#[test]
fn backs_up_overwritten_and_deleted_files_into_one_folder() {
    let fx = Fixture::new(&["Overtone", "Overtone%%A", "Overtone%%Gone"]);
    fx.write_crate("Overtone.crate", &[]);
    fx.write_crate("Overtone%%A.crate", &[&fx.ptrk("old.flac")]);
    fx.write_crate("Overtone%%Gone.crate", &[&fx.ptrk("x.flac")]);
    let before_a = std::fs::read(fx.crate_path("Overtone%%A.crate")).unwrap();
    let before_gone = std::fs::read(fx.crate_path("Overtone%%Gone.crate")).unwrap();
    let before_order = std::fs::read(fx.serato.join("neworder.pref")).unwrap();

    let plan = make_plan(&fx, &[want(&fx, &["A"], &["new.flac"])]);
    let report = apply(
        &plan,
        &ProtectedRoots::none(),
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap();

    assert_eq!(report.updated, ["Overtone%%A.crate"]);
    assert_eq!(report.deleted, ["Overtone%%Gone.crate"]);
    assert_eq!(fx.read_tracks("Overtone%%A.crate"), [fx.ptrk("new.flac")]);
    assert!(!fx.crate_path("Overtone%%Gone.crate").exists());
    assert_eq!(order(&fx), ["Overtone", "Overtone%%A"]);

    let folder = report.backup_dir.unwrap();
    assert_eq!(
        std::fs::read(folder.join("Overtone%%A.crate")).unwrap(),
        before_a
    );
    assert_eq!(
        std::fs::read(folder.join("Overtone%%Gone.crate")).unwrap(),
        before_gone
    );
    assert_eq!(
        std::fs::read(folder.join("neworder.pref")).unwrap(),
        before_order
    );
    assert!(
        !folder.join("Overtone.crate").exists(),
        "untouched files aren't backed up"
    );
    assert_eq!(std::fs::read_dir(backups(&fx)).unwrap().count(), 1);
}

#[test]
fn refuses_protected_roots_before_changing_anything() {
    let fx = Fixture::new(&[]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac"])]);
    let roots = ProtectedRoots::new([fx.home.join("Music")]);
    let err = apply(&plan, &roots, &backups(&fx), RunningCheck::Skip).unwrap_err();
    assert!(matches!(err, SeratoError::Fs(FsError::Protected { .. })));
    assert!(fx.list().is_empty());
    assert!(!backups(&fx).exists());

    // A protected backup folder is refused too.
    let roots = ProtectedRoots::new([backups(&fx)]);
    assert!(apply(&plan, &roots, &backups(&fx), RunningCheck::Skip).is_err());
    assert!(fx.list().is_empty());
}

#[test]
fn refuses_stale_plans() {
    let fx = Fixture::new(&["Overtone", "Overtone%%A"]);
    fx.write_crate("Overtone.crate", &[]);
    fx.write_crate("Overtone%%A.crate", &[&fx.ptrk("a.flac")]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["b.flac"])]);
    fx.write_crate("Overtone%%A.crate", &[&fx.ptrk("changed-meanwhile.flac")]);
    let err = apply(
        &plan,
        &ProtectedRoots::none(),
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap_err();
    assert!(matches!(err, SeratoError::StalePlan(_)));
    assert_eq!(
        fx.read_tracks("Overtone%%A.crate"),
        [fx.ptrk("changed-meanwhile.flac")]
    );

    // A file that appeared after planning counts as a change too.
    let fx = Fixture::new(&[]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &[])]);
    fx.write_crate("Overtone.crate", &[]);
    assert!(matches!(
        apply(
            &plan,
            &ProtectedRoots::none(),
            &backups(&fx),
            RunningCheck::Skip
        ),
        Err(SeratoError::StalePlan(_))
    ));
}

#[test]
fn plans_that_lost_their_contents_cannot_be_applied() {
    let fx = Fixture::new(&[]);
    let mut plan = make_plan(&fx, &[want(&fx, &["A"], &[])]);
    plan.files[0].new_bytes = None;
    assert!(matches!(
        apply(
            &plan,
            &ProtectedRoots::none(),
            &backups(&fx),
            RunningCheck::Skip
        ),
        Err(SeratoError::StalePlan(_))
    ));
}

#[test]
fn creates_the_subcrates_folder_when_missing() {
    let fx = Fixture::new(&[]);
    std::fs::remove_dir_all(fx.serato.join("Subcrates")).unwrap();
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac"])]);
    apply(
        &plan,
        &ProtectedRoots::none(),
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap();
    assert_eq!(fx.list(), ["Overtone%%A.crate", "Overtone.crate"]);
}

#[test]
fn written_files_parse_back_to_the_planned_bytes() {
    let fx = Fixture::new(&[]);
    let plan = make_plan(&fx, &[want(&fx, &["A"], &["a.flac"])]);
    apply(
        &plan,
        &ProtectedRoots::none(),
        &backups(&fx),
        RunningCheck::Skip,
    )
    .unwrap();
    let bytes = std::fs::read(fx.crate_path("Overtone%%A.crate")).unwrap();
    assert_eq!(CrateFile::parse(&bytes).unwrap().to_bytes().unwrap(), bytes);
}
