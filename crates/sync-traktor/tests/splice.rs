//! Splicing the Overtone folder into synthetic Traktor Pro 3 and 4
//! collections. The fixtures follow Traktor's own layout and contain no
//! real track metadata.

use overtone_sync_traktor::{
    CollectionScan, PlaylistNode, TraktorError, patch, render_overtone_folder, scan, verify,
};

const TP4: &[u8] = include_bytes!("fixtures/collection-tp4.nml");
const TP3: &[u8] = include_bytes!("fixtures/collection-tp3-overtone.nml");

const SIGNAL: &str = "Macintosh HD/:Users/:dj/:Music/:Garage/:signal & noise.mp3";
const CAFE_NFD: &str = "Macintosh HD/:Users/:dj/:Music/:Garage/:Cafe\u{301} del Mar.mp3";
const GIG: &str = "Gig/:Sets/:gig.flac";
const STEM: &str = "Macintosh HD/:Users/:dj/:Music/:Stems/:stem thing.stem.mp4";

fn playlist(name: &str, keys: &[&str]) -> PlaylistNode {
    PlaylistNode::Playlist {
        name: name.to_string(),
        keys: keys.iter().map(ToString::to_string).collect(),
    }
}

/// The tree the TP3 fixture's existing Overtone folder was rendered from.
fn tp3_tree() -> Vec<PlaylistNode> {
    vec![
        PlaylistNode::Folder {
            name: "Dubs".into(),
            children: vec![
                playlist("Dubs", &[SIGNAL]),
                playlist("Deep", &[SIGNAL, CAFE_NFD]),
            ],
        },
        playlist("Warmup", &[]),
    ]
}

fn crlf(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .expect("utf8")
        .replace('\n', "\r\n")
        .into_bytes()
}

fn bom(bytes: &[u8]) -> Vec<u8> {
    let mut out = b"\xEF\xBB\xBF".to_vec();
    out.extend_from_slice(bytes);
    out
}

fn variants() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = Vec::new();
    for (name, bytes) in [("tp4", TP4), ("tp3", TP3)] {
        out.push((name, bytes.to_vec()));
        out.push((name, crlf(bytes)));
        out.push((name, bom(bytes)));
        out.push((name, bom(&crlf(bytes))));
    }
    out
}

fn root_count(bytes: &[u8], scan: &CollectionScan) -> String {
    String::from_utf8(bytes[scan.root_subnodes_count_span.clone()].to_vec()).expect("utf8")
}

#[test]
fn scans_versions_keys_and_volumes() {
    let tp4 = scan(TP4).expect("scan");
    assert_eq!(tp4.version.as_deref(), Some("20"));
    assert_eq!(tp4.keys.len(), 6);
    assert_eq!(tp4.startup_volume, "Macintosh HD");
    assert_eq!(tp4.volume_counts.get("Gig"), Some(&1));
    assert_eq!(tp4.lookup(SIGNAL), Some(SIGNAL));
    assert_eq!(tp4.lookup(STEM), Some(STEM));
    assert_eq!(tp4.lookup(GIG), Some(GIG));
    assert_eq!(
        tp4.lookup("Macintosh HD/:Users/:dj/:Music/:Live// 2024/:b//c.wav"),
        Some("Macintosh HD/:Users/:dj/:Music/:Live// 2024/:b//c.wav")
    );
    // An NFC spelling finds the collection's NFD key.
    let cafe_nfc = "Macintosh HD/:Users/:dj/:Music/:Garage/:Caf\u{e9} del Mar.mp3";
    assert_eq!(tp4.lookup(cafe_nfc), Some(CAFE_NFD));
    assert_eq!(tp4.newline, "\n");
    assert!(!tp4.bom);
    assert_eq!((tp4.root_subnodes_count, tp4.root_child_count), (4, 4));
    assert_eq!(
        tp4.overtone_span, None,
        "a folder named Overtone deeper in the tree is not Overtone's"
    );

    let tp3 = scan(TP3).expect("scan");
    assert_eq!(tp3.version.as_deref(), Some("19"));
    let span = tp3
        .overtone_span
        .clone()
        .expect("top-level Overtone folder");
    let text = std::str::from_utf8(&TP3[span]).expect("utf8");
    assert!(text.starts_with("<NODE TYPE=\"FOLDER\" NAME=\"Overtone\">"));
    assert!(text.ends_with("</SUBNODES>\n</NODE>\n"));
    assert_eq!(tp3.overtone_keys.len(), 3);
    assert!(tp3.overtone_problems.is_empty());

    let crlf_scan = scan(&bom(&crlf(TP3))).expect("scan");
    assert_eq!(crlf_scan.newline, "\r\n");
    assert!(crlf_scan.bom);
}

#[test]
fn rendering_the_existing_folder_again_is_byte_identical() {
    for (name, bytes) in variants() {
        let scanned = scan(&bytes).expect("scan");
        if name == "tp3" {
            let folder = render_overtone_folder(&tp3_tree(), scanned.newline);
            assert_eq!(
                scanned.overtone_bytes(&bytes),
                Some(folder.as_bytes()),
                "the fixture holds what Overtone renders"
            );
            let patched = patch(&bytes, Some(&folder)).expect("patch");
            assert_eq!(patched, bytes, "{name}");
            verify(&bytes, &patched, Some(&folder)).expect("verify");
        } else {
            let patched = patch(&bytes, None).expect("patch");
            assert_eq!(patched, bytes, "nothing to remove in {name}");
        }
    }
}

#[test]
fn inserts_a_new_folder_last_in_root() {
    for (name, bytes) in variants().into_iter().filter(|(n, _)| *n == "tp4") {
        let before = scan(&bytes).expect("scan");
        let tree = vec![
            playlist("Warmup", &[SIGNAL, GIG, STEM]),
            PlaylistNode::Folder {
                name: "Dubs & Bass".into(),
                children: vec![playlist("Deep", &[CAFE_NFD])],
            },
        ];
        let folder = render_overtone_folder(&tree, before.newline);
        let patched = patch(&bytes, Some(&folder)).expect("patch");
        let after = verify(&bytes, &patched, Some(&folder)).expect("verify");

        assert_eq!(root_count(&patched, &after), "5", "{name}");
        assert_eq!(after.root_child_count, 5);
        let span = after.overtone_span.clone().expect("inserted");
        assert_eq!(span.start, before.root_subnodes_end, "goes last in $ROOT");
        assert_eq!(
            &patched[..before.root_subnodes_count_span.start],
            &bytes[..before.root_subnodes_count_span.start]
        );
        assert_eq!(&patched[span.end..], &bytes[before.root_subnodes_end..]);
        let text = String::from_utf8(patched.clone()).expect("utf8");
        assert!(
            text.contains("<NODE TYPE=\"FOLDER\" NAME=\"Dubs &amp; Bass\"><SUBNODES COUNT=\"1\">")
        );
        assert!(text.contains(&format!(
            "<ENTRY><PRIMARYKEY TYPE=\"STEM\" KEY=\"{STEM}\"></PRIMARYKEY>{}</ENTRY>",
            before.newline
        )));
        assert!(
            text.contains("KEY=\"Macintosh HD/:Users/:dj/:Music/:Garage/:signal &amp; noise.mp3\"")
        );
        if before.newline == "\r\n" {
            assert!(
                !text.replace("\r\n", "").contains('\n'),
                "line endings stay CRLF"
            );
        }

        // Removing it again restores the original exactly.
        let removed = patch(&patched, None).expect("remove");
        assert_eq!(removed, bytes, "{name}");
        verify(&patched, &removed, None).expect("verify removal");
    }
}

#[test]
fn replaces_and_removes_an_existing_folder_in_place() {
    for (name, bytes) in variants().into_iter().filter(|(n, _)| *n == "tp3") {
        let before = scan(&bytes).expect("scan");
        let old = before.overtone_span.clone().expect("existing");
        let folder = render_overtone_folder(&[playlist("Peak", &[GIG])], before.newline);
        let patched = patch(&bytes, Some(&folder)).expect("patch");
        let after = verify(&bytes, &patched, Some(&folder)).expect("verify");
        assert_eq!(root_count(&patched, &after), "4", "{name}");
        assert_eq!(
            after.overtone_span.clone().map(|s| s.start),
            Some(old.start)
        );
        assert_eq!(&patched[..old.start], &bytes[..old.start]);
        assert_eq!(&patched[old.start + folder.len()..], &bytes[old.end..]);

        let removed = patch(&bytes, None).expect("remove");
        let after = verify(&bytes, &removed, None).expect("verify");
        assert_eq!(root_count(&removed, &after), "3");
        assert_eq!(after.overtone_span, None);
        assert_eq!(
            &removed[..before.root_subnodes_count_span.start],
            &bytes[..before.root_subnodes_count_span.start]
        );
        assert_eq!(&removed[old.start..], &bytes[old.end..]);
        let text = String::from_utf8(removed).expect("utf8");
        assert!(
            text.contains("NAME=\"Mine\""),
            "your own Overtone folder stays"
        );
    }
}

#[test]
fn verify_catches_bad_patches() {
    let folder = render_overtone_folder(&[playlist("Peak", &[GIG])], "\n");
    let good = patch(TP4, Some(&folder)).expect("patch");
    verify(TP4, &good, Some(&folder)).expect("good patch");

    let outside = String::from_utf8(good.clone())
        .expect("utf8")
        .replace("Test Artist", "Test Artistx");
    assert!(matches!(
        verify(TP4, outside.as_bytes(), Some(&folder)),
        Err(TraktorError::Verify(_))
    ));

    let unknown = render_overtone_folder(&[playlist("Peak", &["Gig/:nope.mp3"])], "\n");
    let bad_key = patch(TP4, Some(&unknown)).expect("patch");
    assert!(matches!(
        verify(TP4, &bad_key, Some(&unknown)),
        Err(TraktorError::Verify(msg)) if msg.contains("nope")
    ));

    let miscounted = folder.replace("ENTRIES=\"1\"", "ENTRIES=\"2\"");
    let bad_count = patch(TP4, Some(&miscounted)).expect("patch");
    assert!(matches!(
        verify(TP4, &bad_count, Some(&miscounted)),
        Err(TraktorError::Verify(_))
    ));

    let broken = patch(TP4, Some("<NODE TYPE=\"FOLDER\" NAME=\"Overtone\">\n")).expect("patch");
    assert!(matches!(
        verify(TP4, &broken, Some("x")),
        Err(TraktorError::Verify(_))
    ));
    assert!(matches!(
        verify(TP4, &good, Some("something else")),
        Err(TraktorError::Verify(_))
    ));
}

#[test]
fn refuses_collections_without_a_root_folder() {
    let nml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n<NML VERSION=\"20\"><COLLECTION ENTRIES=\"0\"></COLLECTION>\n</NML>\n";
    assert!(matches!(
        patch(nml, Some("x")),
        Err(TraktorError::Malformed(_))
    ));
}

#[test]
fn inserts_into_an_empty_root() {
    let nml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n<NML VERSION=\"20\"><HEAD COMPANY=\"www.native-instruments.com\" PROGRAM=\"Traktor Pro 4\"></HEAD>\n<COLLECTION ENTRIES=\"0\"></COLLECTION>\n<PLAYLISTS><NODE TYPE=\"FOLDER\" NAME=\"$ROOT\"><SUBNODES COUNT=\"0\"></SUBNODES>\n</NODE>\n</PLAYLISTS>\n</NML>\n";
    let folder = render_overtone_folder(&[playlist("Empty", &[])], "\n");
    let patched = patch(nml.as_bytes(), Some(&folder)).expect("patch");
    verify(nml.as_bytes(), &patched, Some(&folder)).expect("verify");
    let text = String::from_utf8(patched.clone()).expect("utf8");
    assert!(text.contains(&format!("<SUBNODES COUNT=\"1\">{folder}</SUBNODES>")));
    assert_eq!(patch(&patched, None).expect("remove"), nml.as_bytes());
}
