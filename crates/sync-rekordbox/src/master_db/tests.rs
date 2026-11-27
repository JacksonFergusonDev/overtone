use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use tempfile::TempDir;

use super::fixture::{self, INITIAL_USN};
use super::*;

/// A fixture db with three tracks; the second is stored in NFD form.
fn fixture_db(dir: &TempDir) -> (PathBuf, Vec<PathBuf>) {
    let music = dir.path().join("music");
    let a = music.join("a.flac");
    // "Café" with a combining acute accent, as macOS may report it.
    let b_nfd = music.join("Cafe\u{301}.flac");
    let c = music.join("c.flac");
    let path = dir.path().join("master.db");
    let tracks = [
        ("11", a.to_str().expect("utf8")),
        ("12", b_nfd.to_str().expect("utf8")),
        ("13", c.to_str().expect("utf8")),
    ];
    fixture::create(&path, &tracks).expect("fixture");
    // Overtone sees the NFC form.
    let b_nfc = music.join("Caf\u{e9}.flac");
    (path, vec![a, b_nfc, c])
}

fn desired(tags: &[(&str, &[&PathBuf])]) -> Vec<DesiredTag> {
    tags.iter()
        .map(|(name, tracks)| DesiredTag {
            name: (*name).to_string(),
            tracks: tracks.iter().map(|p| (*p).clone()).collect(),
        })
        .collect()
}

fn rows(db: &MasterDb, sql: &str) -> Vec<Vec<Value>> {
    let mut stmt = db.conn.prepare(sql).expect("prepare");
    let columns = stmt.column_count();
    stmt.query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// Every row Overtone must not touch: user and default My Tags and their
/// song-tag rows, and all of djmdContent.
fn user_rows(db: &MasterDb) -> Vec<Vec<Value>> {
    let mut out = rows(
        db,
        "SELECT * FROM djmdMyTag WHERE ID IN ('1','2','3','4','1001','1002','1003') ORDER BY ID",
    );
    out.extend(rows(
        db,
        "SELECT * FROM djmdSongMyTag WHERE MyTagID IN ('1001','1002','1003') ORDER BY ID",
    ));
    out.extend(rows(db, "SELECT * FROM djmdContent ORDER BY ID"));
    out
}

fn tag_contents(db: &MasterDb) -> Vec<(String, BTreeSet<String>)> {
    let tags = db.overtone_tags().expect("tags");
    let ids: Vec<String> = tags.iter().map(|t| t.id.clone()).collect();
    let assignments = db.tag_assignments(&ids).expect("assignments");
    tags.iter()
        .map(|t| {
            (
                t.name.clone(),
                assignments[&t.id]
                    .iter()
                    .map(|r| r.content_id.clone())
                    .collect(),
            )
        })
        .collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn decodes_the_published_key() {
    let key = passphrase();
    assert_eq!(key.len(), 64);
    assert!(key.starts_with("402fd482c38817c35ffa8"));
}

#[test]
fn refuses_wrong_files_and_never_creates_one() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("missing.db");
    assert!(MasterDb::open(&missing).is_err());
    assert!(!missing.exists(), "open never creates a database");

    let plain = dir.path().join("plain.db");
    rusqlite::Connection::open(&plain)
        .expect("plain")
        .execute_batch("CREATE TABLE t (x)")
        .expect("table");
    assert!(matches!(
        MasterDb::open(&plain),
        Err(RekordboxError::NotMasterDb(_))
    ));
}

#[test]
fn reads_contents_by_nfc_path() {
    let dir = TempDir::new().expect("tempdir");
    let (path, tracks) = fixture_db(&dir);
    let db = MasterDb::open(&path).expect("open");
    let contents = db.contents_by_path().expect("contents");
    assert_eq!(contents.len(), 3);
    let nfc = tracks[1].to_str().expect("utf8");
    assert_eq!(contents[nfc], ["12"]);
    assert_eq!(db.local_usn().expect("usn"), INITIAL_USN);
    assert!(db.overtone_category().expect("category").is_none());
}

#[test]
fn creates_assigns_and_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    let (path, t) = fixture_db(&dir);
    let missing = dir.path().join("music/not-in-rekordbox.flac");
    let mut db = MasterDb::open(&path).expect("open");
    let before = user_rows(&db);
    let want = desired(&[
        ("Dubs", &[&t[0]]),
        ("Dubs/Deep", &[&t[0], &t[1], &missing, &t[1]]),
        ("Warmup", &[]),
    ]);

    let plan = plan(&db, &want).expect("plan");
    assert!(plan.create_category);
    assert_eq!(plan.create_tags, ["Dubs", "Dubs/Deep", "Warmup"]);
    assert_eq!(plan.add.len(), 3, "duplicates are added once");
    assert_eq!(plan.skipped_tracks, std::slice::from_ref(&missing));
    assert_eq!(plan.expected_usn_delta(), 1 + 3 + 2 * 3);

    let outcome = apply(&mut db, &plan).expect("apply");
    assert_eq!(outcome.usn_before, INITIAL_USN);
    assert_eq!(outcome.usn_after, INITIAL_USN + 10);
    assert_eq!(
        tag_contents(&db),
        [
            ("Dubs".to_string(), set(&["11"])),
            ("Dubs/Deep".to_string(), set(&["11", "12"])),
            ("Warmup".to_string(), BTreeSet::new()),
        ]
    );
    assert_eq!(user_rows(&db), before, "user rows are untouched");

    // The category goes after the four default ones; tags are 1-based.
    let category = db.overtone_category().expect("category").expect("exists");
    assert_eq!(category.seq, 5);
    assert_eq!(category.attribute, ATTRIBUTE_CATEGORY);
    let seqs: Vec<i64> = db
        .overtone_tags()
        .expect("tags")
        .iter()
        .map(|t| t.seq)
        .collect();
    assert_eq!(seqs, [1, 2, 3]);

    // USNs: category 1001, tags 1002-1004, song tags +2 each: 1006, 1008, 1010.
    let stamped = rows(
        &db,
        &format!(
            "SELECT rb_local_usn FROM djmdMyTag WHERE ID = '{0}' OR ParentID = '{0}' ORDER BY Seq",
            category.id
        ),
    );
    let category_usn = rows(
        &db,
        &format!(
            "SELECT rb_local_usn FROM djmdMyTag WHERE ID = '{}'",
            category.id
        ),
    );
    assert_eq!(category_usn, [[Value::Integer(1001)]]);
    assert_eq!(stamped.len(), 4);
    let song_usns: Vec<Vec<Value>> = rows(
        &db,
        "SELECT rb_local_usn FROM djmdSongMyTag WHERE rb_local_usn > 1000 ORDER BY rb_local_usn",
    );
    assert_eq!(
        song_usns,
        [
            [Value::Integer(1006)],
            [Value::Integer(1008)],
            [Value::Integer(1010)]
        ]
    );
    let deep = rows(
        &db,
        "SELECT s.TrackNo, s.UUID, s.created_at, s.usn, s.rb_local_deleted FROM djmdSongMyTag s \
         JOIN djmdMyTag t ON t.ID = s.MyTagID WHERE t.Name = 'Dubs/Deep' ORDER BY s.TrackNo",
    );
    assert_eq!(deep.len(), 2);
    assert_eq!(deep[0][0], Value::Integer(1));
    assert_eq!(deep[1][0], Value::Integer(2));
    let Value::Text(uuid) = &deep[0][1] else {
        panic!("uuid")
    };
    assert_eq!(uuid.len(), 36);
    let Value::Text(created) = &deep[0][2] else {
        panic!("created_at")
    };
    assert!(
        created.ends_with(" +00:00") && created.len() == 30,
        "{created}"
    );
    assert_eq!(deep[0][3], Value::Null);
    assert_eq!(deep[0][4], Value::Integer(0));

    let again = super::plan(&db, &want).expect("plan again");
    assert!(again.is_empty(), "{again:?}");
    assert_eq!(again.unchanged_tags.len(), 3);
}

#[test]
fn rename_and_track_changes() {
    let dir = TempDir::new().expect("tempdir");
    let (path, t) = fixture_db(&dir);
    let mut db = MasterDb::open(&path).expect("open");
    let before = user_rows(&db);
    let first = plan(
        &db,
        &desired(&[("Dubs", &[&t[0], &t[1]]), ("Peak", &[&t[2]])]),
    )
    .expect("plan");
    apply(&mut db, &first).expect("apply");
    let usn = db.local_usn().expect("usn");

    // Rename Dubs to Bass/Dubs, drop t[0] from Peak's neighbour, swap Peak's track.
    let second = plan(
        &db,
        &desired(&[("Peak", &[&t[0]]), ("Bass/Dubs", &[&t[0], &t[1]])]),
    )
    .expect("plan");
    assert_eq!(second.create_tags, ["Bass/Dubs"]);
    assert_eq!(
        second
            .delete_tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Dubs"]
    );
    assert_eq!(second.delete_tags[0].track_count, 2);
    assert_eq!(second.remove.len(), 1);
    assert_eq!(second.remove[0].content_id, "13");
    // delete 1 + create 1 + remove 1 + 3 adds * 2.
    assert_eq!(second.expected_usn_delta(), 9);
    let outcome = apply(&mut db, &second).expect("apply");
    assert_eq!(outcome.usn_after, usn + 9);
    assert!(outcome.usn_after > outcome.usn_before);
    assert_eq!(
        tag_contents(&db),
        [
            ("Peak".to_string(), set(&["11"])),
            ("Bass/Dubs".to_string(), set(&["11", "12"])),
        ]
    );
    let seqs: Vec<i64> = db
        .overtone_tags()
        .expect("tags")
        .iter()
        .map(|t| t.seq)
        .collect();
    assert_eq!(seqs, [1, 2], "Peak moved up after Dubs was deleted");
    let orphans = rows(
        &db,
        "SELECT ID FROM djmdSongMyTag WHERE MyTagID NOT IN (SELECT ID FROM djmdMyTag)",
    );
    assert!(
        orphans.is_empty(),
        "deleted tags take their song rows along"
    );
    assert_eq!(user_rows(&db), before);
    // The shifted Peak row is stamped with a USN no later than the counter.
    let peak = rows(
        &db,
        "SELECT rb_local_usn FROM djmdMyTag WHERE Name = 'Peak'",
    );
    let Value::Integer(peak_usn) = peak[0][0] else {
        panic!("usn")
    };
    assert!(peak_usn > usn && peak_usn <= outcome.usn_after);
}

#[test]
fn removes_everything_when_no_vibes_remain() {
    let dir = TempDir::new().expect("tempdir");
    let (path, t) = fixture_db(&dir);
    let mut db = MasterDb::open(&path).expect("open");
    let first = plan(&db, &desired(&[("Dubs", &[&t[0]])])).expect("plan");
    apply(&mut db, &first).expect("apply");
    let empty = plan(&db, &[]).expect("plan");
    assert_eq!(empty.delete_tags.len(), 1);
    assert!(!empty.create_category);
    apply(&mut db, &empty).expect("apply");
    assert!(db.overtone_tags().expect("tags").is_empty());
    assert!(db.overtone_category().expect("category").is_some());

    let fresh = TempDir::new().expect("tempdir");
    let (path, _) = fixture_db(&fresh);
    let db = MasterDb::open(&path).expect("open");
    assert!(
        plan(&db, &[]).expect("plan").is_empty(),
        "no empty category"
    );
}

#[test]
fn refuses_two_overtone_categories_and_stale_plans() {
    let dir = TempDir::new().expect("tempdir");
    let (path, t) = fixture_db(&dir);
    let mut db = MasterDb::open(&path).expect("open");
    let stale = plan(&db, &desired(&[("Dubs", &[&t[0]])])).expect("plan");
    apply(&mut db, &stale).expect("apply");
    assert!(matches!(
        apply(&mut db, &stale),
        Err(RekordboxError::Unsupported(_))
    ));
    db.conn
        .execute(
            "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, created_at, updated_at) \
             VALUES ('9', 6, 'Overtone', 1, 'root', 'x', 'x')",
            [],
        )
        .expect("second category");
    assert!(matches!(
        plan(&db, &[]),
        Err(RekordboxError::AmbiguousCategory(2))
    ));
}

#[test]
fn never_reads_or_changes_the_credentials_row() {
    let dir = TempDir::new().expect("tempdir");
    let (path, t) = fixture_db(&dir);
    let mut db = MasterDb::open(&path).expect("open");
    let plan = plan(&db, &desired(&[("Dubs", &[&t[0]])])).expect("plan");
    let outcome = apply(&mut db, &plan).expect("apply");
    let text = format!("{plan:?} {outcome:?} {db:?}");
    assert!(!text.contains(fixture::CREDENTIAL_SECRET));
    db.close().expect("close");
    assert_eq!(
        fixture::credential_row(&path).expect("read").as_deref(),
        Some(fixture::CREDENTIAL_SECRET)
    );
}

#[test]
fn live_fixture_keeps_frames_in_the_wal() {
    let dir = TempDir::new().expect("tempdir");
    let live =
        fixture::create_live(&dir.path().join("Pioneer"), &[("11", "/x/a.flac")]).expect("live");
    let wal = live.with_file_name("master.db-wal");
    assert!(std::fs::metadata(&wal).expect("wal").len() > 0);

    // The main file alone lacks the WAL-only tag; with the WAL it is there.
    let alone = dir.path().join("alone");
    std::fs::create_dir(&alone).expect("mkdir");
    std::fs::copy(&live, alone.join("master.db")).expect("copy");
    let db = MasterDb::open(&alone.join("master.db")).expect("open");
    assert!(!name_exists(&db, fixture::WAL_ONLY_TAG));
    let both = dir.path().join("both");
    std::fs::create_dir(&both).expect("mkdir");
    std::fs::copy(&live, both.join("master.db")).expect("copy");
    std::fs::copy(&wal, both.join("master.db-wal")).expect("copy wal");
    let db = MasterDb::open(&both.join("master.db")).expect("open");
    assert!(name_exists(&db, fixture::WAL_ONLY_TAG));
}

fn name_exists(db: &MasterDb, name: &str) -> bool {
    !rows(
        db,
        &format!("SELECT ID FROM djmdMyTag WHERE Name = '{name}'"),
    )
    .is_empty()
}

#[test]
fn formats_timestamps_like_rekordbox() {
    use std::time::{Duration, UNIX_EPOCH};
    let time = UNIX_EPOCH + Duration::from_millis(1_751_114_096_789);
    assert_eq!(
        apply::rekordbox_timestamp(time),
        "2025-06-28 12:34:56.789 +00:00"
    );
}

/// Plan, apply, and verify against a copy of a real master.db. Opt in with
/// `OVERTONE_RB_FIXTURE=/path/to/master.db`; the file is copied into a
/// temporary folder first and never opened in place.
#[test]
#[ignore = "needs OVERTONE_RB_FIXTURE pointing at a master.db"]
fn real_database_copy() {
    let Some(source) = std::env::var_os("OVERTONE_RB_FIXTURE") else {
        return;
    };
    let source = Path::new(&source);
    let dir = TempDir::new().expect("tempdir");
    let copy = dir.path().join("master.db");
    std::fs::copy(source, &copy).expect("copy");
    let wal = source.with_file_name("master.db-wal");
    if wal.exists() {
        std::fs::copy(&wal, dir.path().join("master.db-wal")).expect("copy wal");
    }
    // The source may be read-only, and copies keep its permissions.
    for entry in std::fs::read_dir(dir.path()).expect("list") {
        let path = entry.expect("entry").path();
        let mut permissions = std::fs::metadata(&path).expect("stat").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).expect("chmod");
    }
    let mut db = MasterDb::open(&copy).expect("open copy");
    let mut contents: Vec<String> = db
        .contents_by_path()
        .expect("contents")
        .into_keys()
        .collect();
    contents.sort();
    let tracks: Vec<PathBuf> = contents.iter().take(5).map(PathBuf::from).collect();
    let want = vec![
        DesiredTag {
            name: "Test/One".into(),
            tracks: tracks.clone(),
        },
        DesiredTag {
            name: "Test/Two".into(),
            tracks: tracks.iter().skip(2).cloned().collect(),
        },
    ];
    let usn = db.local_usn().expect("usn");
    let first = plan(&db, &want).expect("plan");
    let outcome = apply(&mut db, &first).expect("apply and verify");
    assert_eq!(outcome.usn_after, usn + first.expected_usn_delta());
    assert!(plan(&db, &want).expect("replan").is_empty());
    let shrink = plan(&db, &want[..1]).expect("plan shrink");
    apply(&mut db, &shrink).expect("apply shrink");
    db.checkpoint().expect("checkpoint");
    db.close().expect("close");
    let db = MasterDb::open(&copy).expect("reopen");
    assert_eq!(db.overtone_tags().expect("tags").len(), 1);
    println!(
        "real copy: {} tracks, usn {usn} -> {}, plan {first:?}",
        contents.len(),
        db.local_usn().expect("usn")
    );
}
