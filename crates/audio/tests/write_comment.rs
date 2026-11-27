//! Comment writes on synthetic files and the Serato GEOB fixture.

mod common;

use std::path::Path;

use common::*;
use overtone_audio::AudioError;
use overtone_audio::tags::raw::{self, Container, RawFrame};
use overtone_audio::tags::{WriteOutcome, check_writable, read_metadata, write_comment};
use overtone_fs::ProtectedRoots;
use tempfile::TempDir;

fn roots() -> ProtectedRoots {
    ProtectedRoots::none()
}

fn binary_frames(path: &Path, container: Container) -> Vec<RawFrame> {
    raw::inspect(path, container)
        .expect("walk")
        .id3v2
        .map_or_default(|tag| {
            tag.frames
                .into_iter()
                .filter(|f| matches!(f.id.as_str(), "GEOB" | "PRIV" | "APIC" | "TXXX"))
                .collect()
        })
}

fn payload(path: &Path, container: Container) -> Vec<u8> {
    let layout = raw::inspect(path, container).expect("walk");
    let bytes = std::fs::read(path).expect("read");
    let start = usize::try_from(layout.payload.start).expect("offset");
    let end = usize::try_from(layout.payload.end).expect("offset");
    bytes[start..end].to_vec()
}

fn comment(path: &Path) -> Option<String> {
    read_metadata(path).expect("read").tags.comment
}

#[test]
fn serato_mp3_keeps_every_geob_frame_byte_for_byte() {
    let dir = TempDir::new().expect("tempdir");
    let path = mp3(dir.path(), "serato.mp3", &serato_tag());
    let before = binary_frames(&path, Container::Mp3);
    assert_eq!(before.iter().filter(|f| f.id == "GEOB").count(), 14);
    let audio = payload(&path, Container::Mp3);

    let outcome = write_comment(&roots(), &path, Some("8A - Energy 6 #warmup")).expect("write");
    assert!(matches!(outcome, WriteOutcome::Written(_)));
    assert_eq!(comment(&path).as_deref(), Some("8A - Energy 6 #warmup"));
    assert_eq!(binary_frames(&path, Container::Mp3), before);
    assert_eq!(payload(&path, Container::Mp3), audio);
    let layout = raw::inspect(&path, Container::Mp3).expect("walk");
    assert_eq!(layout.id3v2.expect("tag").major, 4);
}

#[test]
fn second_identical_write_is_a_byte_identical_no_op() {
    let dir = TempDir::new().expect("tempdir");
    let path = mp3(dir.path(), "a.mp3", &serato_tag());
    write_comment(&roots(), &path, Some("#warmup")).expect("first");
    let bytes = std::fs::read(&path).expect("read");
    let modified = std::fs::metadata(&path)
        .expect("stat")
        .modified()
        .expect("mtime");
    let outcome = write_comment(&roots(), &path, Some("#warmup")).expect("second");
    assert_eq!(outcome, WriteOutcome::Unchanged);
    assert_eq!(std::fs::read(&path).expect("read"), bytes);
    assert_eq!(
        std::fs::metadata(&path)
            .expect("stat")
            .modified()
            .expect("mtime"),
        modified
    );
}

#[test]
fn id3v23_stays_id3v23_and_other_comments_survive() {
    let dir = TempDir::new().expect("tempdir");
    let tag = id3_tag(
        3,
        &[
            ("TIT2", 0, text("Title")),
            ("TYER", 0, text("2019")),
            ("COMM", 0, comm(*b"eng", "iTunNORM", " 0000 0001")),
            ("COMM", 0, comm(*b"XXX", "", "other language")),
            ("COMM", 0, comm(*b"eng", "", "8A - Energy 6")),
            (
                "GEOB",
                0,
                geob("Serato Markers2", &[1, 2, 3, 0xFF, 0x00, 0xE0]),
            ),
            ("PRIV", 0, priv_frame("TRAKTOR4", b"\x00\x01binary\xFF")),
            ("APIC", 0, apic()),
        ],
        256,
    );
    let path = mp3(dir.path(), "v23.mp3", &tag);
    let before = binary_frames(&path, Container::Mp3);
    write_comment(&roots(), &path, Some("8A - Energy 6 #dubs")).expect("write");

    let layout = raw::inspect(&path, Container::Mp3).expect("walk");
    let tag = layout.id3v2.expect("tag");
    assert_eq!(tag.major, 3);
    let comments: Vec<_> = tag.frames.iter().filter_map(RawFrame::comment).collect();
    assert_eq!(comments.len(), 3);
    assert!(
        comments
            .iter()
            .any(|(k, v)| k.description == "iTunNORM" && v == " 0000 0001")
    );
    assert!(
        comments
            .iter()
            .any(|(k, v)| &k.language == b"XXX" && v == "other language")
    );
    assert_eq!(
        binary_frames(&path, Container::Mp3),
        before,
        "APIC keeps image/jpg"
    );
    let read = read_metadata(&path).expect("read").tags;
    assert_eq!(read.comment.as_deref(), Some("8A - Energy 6 #dubs"));
    assert_eq!(read.title.as_deref(), Some("Title"));
    assert_eq!(read.year, Some(2019));
}

#[test]
fn creates_and_removes_the_comment() {
    let dir = TempDir::new().expect("tempdir");
    let tag = id3_tag(4, &[("TIT2", 0, text("Title"))], 64);
    let path = mp3(dir.path(), "new.mp3", &tag);
    assert_eq!(comment(&path), None);
    write_comment(&roots(), &path, Some("#warmup ünïcode")).expect("create");
    assert_eq!(comment(&path).as_deref(), Some("#warmup ünïcode"));
    write_comment(&roots(), &path, None).expect("remove");
    assert_eq!(comment(&path), None);
    assert_eq!(
        write_comment(&roots(), &path, Some("   ")).expect("blank"),
        WriteOutcome::Unchanged
    );
}

#[test]
fn mp3_with_trailing_id3v1_with_empty_year_is_writable_and_trailer_kept() {
    let dir = TempDir::new().expect("tempdir");
    let mut bytes = id3_tag(3, &[("COMM", 0, comm(*b"eng", "", "old"))], 32);
    bytes.extend_from_slice(&mpeg_frames(40));
    let mut v1 = vec![0u8; 128];
    v1[..3].copy_from_slice(b"TAG");
    v1[3..8].copy_from_slice(b"Title");
    v1[93..97].copy_from_slice(b"    ");
    bytes.extend_from_slice(&v1);
    let path = write(dir.path(), "v1.mp3", &bytes);
    write_comment(&roots(), &path, Some("new")).expect("write");
    let after = std::fs::read(&path).expect("read");
    assert_eq!(&after[after.len() - 128..], v1.as_slice());
    assert_eq!(comment(&path).as_deref(), Some("new"));
}

fn assert_refused(path: &Path, needle: &str) {
    let before = std::fs::read(path).expect("read");
    let err = write_comment(&roots(), path, Some("#x")).expect_err("must refuse");
    assert!(
        matches!(err, AudioError::WriteRefused { .. }),
        "unexpected error {err}"
    );
    assert!(err.to_string().contains(needle), "{err}");
    assert_eq!(
        std::fs::read(path).expect("read"),
        before,
        "original untouched"
    );
    assert!(check_writable(path).is_err());
}

#[test]
fn refuses_structures_it_cannot_preserve() {
    let dir = TempDir::new().expect("tempdir");
    let mut v22 = id3_tag(3, &[("TIT2", 0, text("a"))], 0);
    v22[3] = 2;
    assert_refused(&mp3(dir.path(), "v22.mp3", &v22), "ID3v2.2");

    let mut unsync = id3_tag(3, &[("TIT2", 0, text("a"))], 0);
    unsync[5] = 0x80;
    assert_refused(&mp3(dir.path(), "unsync.mp3", &unsync), "unsynchronised");

    let compressed = id3_tag(3, &[("TIT2", 0x0080, text("a"))], 0);
    assert_refused(&mp3(dir.path(), "zlib.mp3", &compressed), "compressed");

    let dup = id3_tag(
        4,
        &[
            ("COMM", 0, comm(*b"eng", "", "a")),
            ("COMM", 0, comm(*b"eng", "", "b")),
        ],
        0,
    );
    assert_refused(&mp3(dir.path(), "dup.mp3", &dup), "several Comment frames");

    let mut bad_padding = id3_tag(4, &[("TIT2", 0, text("a"))], 16);
    let last = bad_padding.len() - 1;
    bad_padding[last] = 9;
    assert_refused(&mp3(dir.path(), "pad.mp3", &bad_padding), "padding");
}

/// Frames lofty used to drop, merge, or normalise (and so were refused)
/// now survive byte for byte, because only the Comment frame is rebuilt.
#[test]
fn frames_a_tag_library_would_alter_survive_byte_for_byte() {
    let dir = TempDir::new().expect("tempdir");
    let mut popm = b"x@y\0".to_vec();
    popm.push(196);
    let frames = [
        ("TSOP", 0, text("Artist, The")),
        ("TXXX", 0, text("ARTISTS\0A")),
        ("TXXX", 0, text("ARTISTS\0A")),
        ("TDRC", 0, text("20220310")),
        ("POPM", 0, popm),
        ("USLT", 0, comm(*b"\0\0\0", "", "la la")),
        ("COMM", 0, comm(*b"eng", "", "old")),
    ];
    let tag = id3_tag(3, &frames, 64);
    let path = mp3(dir.path(), "odd.mp3", &tag);
    let before = raw::inspect(&path, Container::Mp3)
        .expect("walk")
        .id3v2
        .expect("tag");
    let len = std::fs::metadata(&path).expect("stat").len();
    write_comment(&roots(), &path, Some("new #vibe")).expect("write");
    let after = raw::inspect(&path, Container::Mp3)
        .expect("walk")
        .id3v2
        .expect("tag");
    assert_eq!(after.major, 3);
    assert_eq!(after.frames.len(), before.frames.len());
    for (a, b) in before.frames.iter().zip(&after.frames).take(6) {
        assert_eq!(a, b, "{} frame changed", a.id);
    }
    assert_eq!(after.target_comment().expect("comment").2, "new #vibe");
    assert_eq!(
        std::fs::metadata(&path).expect("stat").len(),
        len,
        "the tag kept its size, so the audio didn't move"
    );
}

#[test]
fn refuses_m4a_and_protected_paths() {
    let dir = TempDir::new().expect("tempdir");
    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&24u32.to_be_bytes());
    mp4.extend_from_slice(b"ftypM4A \0\0\0\0M4A mp42");
    mp4.extend_from_slice(&8u32.to_be_bytes());
    mp4.extend_from_slice(b"mdat");
    let path = write(dir.path(), "a.m4a", &mp4);
    let err = write_comment(&roots(), &path, Some("#x")).expect_err("m4a");
    assert!(
        err.to_string()
            .contains("writing M4A tags is not supported yet")
    );

    let protected = dir.path().join("Music");
    std::fs::create_dir(&protected).expect("mkdir");
    let path = mp3(&protected, "a.mp3", &serato_tag());
    let before = std::fs::read(&path).expect("read");
    let err =
        write_comment(&ProtectedRoots::new([protected]), &path, Some("#x")).expect_err("protected");
    assert!(matches!(
        err,
        AudioError::Fs(overtone_fs::FsError::Protected { .. })
    ));
    assert_eq!(std::fs::read(&path).expect("read"), before);
}

#[test]
fn wav_and_aiff_keep_geob_and_other_chunks() {
    let dir = TempDir::new().expect("tempdir");
    for aiff in [false, true] {
        let name = if aiff { "a.aiff" } else { "a.wav" };
        let container = if aiff {
            Container::Aiff
        } else {
            Container::Wav
        };
        let chunks: Vec<(&[u8; 4], Vec<u8>)> = if aiff {
            vec![
                (b"ID3 ", serato_tag()),
                (b"APPL", b"stoc\x01\x02\x03".to_vec()),
            ]
        } else {
            vec![
                (b"LIST", b"INFOINAM\x05\x00\x00\x00Name\x00\x00".to_vec()),
                (b"ID3 ", serato_tag()),
                (b"cue ", vec![0, 0, 0, 0]),
            ]
        };
        let path = iff_with_chunks(dir.path(), name, aiff, &chunks);
        let before = binary_frames(&path, container);
        let audio = payload(&path, container);
        write_comment(&roots(), &path, Some("8A #warmup")).expect("write");
        assert_eq!(comment(&path).as_deref(), Some("8A #warmup"));
        assert_eq!(binary_frames(&path, container), before);
        assert_eq!(payload(&path, container), audio);
        let layout = raw::inspect(&path, container).expect("walk");
        let ids: Vec<String> = layout
            .iff
            .expect("chunks")
            .chunks
            .iter()
            .map(raw::IffChunk::id_str)
            .collect();
        for (id, _) in &chunks {
            assert!(
                ids.contains(&String::from_utf8_lossy(*id).into_owned()),
                "{ids:?}"
            );
        }
        assert_eq!(
            write_comment(&roots(), &path, Some("8A #warmup")).expect("again"),
            WriteOutcome::Unchanged
        );
    }
}

#[test]
fn wav_without_tags_gets_an_id3_chunk() {
    let dir = TempDir::new().expect("tempdir");
    let path = iff_with_chunks(dir.path(), "plain.wav", false, &[]);
    write_comment(&roots(), &path, Some("#warmup")).expect("write");
    assert_eq!(comment(&path).as_deref(), Some("#warmup"));
}

#[test]
fn flac_keeps_serato_fields_pictures_and_application_blocks() {
    let dir = TempDir::new().expect("tempdir");
    let path = flac_with_blocks(
        dir.path(),
        "a.flac",
        &[
            (2, b"riffbinary-app-data".to_vec()),
            (
                4,
                vorbis_block(
                    "reference libFLAC 1.4.3",
                    &[
                        "TITLE=Song",
                        "Comment=8A - Energy 6",
                        "SERATO_MARKERS_V2=YXBwbGljYXRpb24vb2N0ZXQtc3RyZWFtAABTZXJhdG8gTWFya2Vyc",
                        "serato_overview=AQ",
                        "COMMENT=second comment",
                    ],
                ),
            ),
            (6, picture_block()),
            (1, vec![0; 512]),
        ],
    );
    let audio = payload(&path, Container::Flac);
    write_comment(&roots(), &path, Some("8A - Energy 6 #dubs/deep")).expect("write");
    assert_eq!(comment(&path).as_deref(), Some("8A - Energy 6 #dubs/deep"));
    let layout = raw::inspect(&path, Container::Flac).expect("walk");
    let flac = layout.flac.expect("flac");
    let vc = flac.vorbis_comment().expect("parse").expect("block");
    let entries: Vec<String> = vc
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(e).into_owned())
        .collect();
    assert_eq!(
        entries,
        [
            "TITLE=Song",
            "Comment=8A - Energy 6 #dubs/deep",
            "SERATO_MARKERS_V2=YXBwbGljYXRpb24vb2N0ZXQtc3RyZWFtAABTZXJhdG8gTWFya2Vyc",
            "serato_overview=AQ",
            "COMMENT=second comment",
        ]
    );
    assert!(
        flac.blocks
            .iter()
            .any(|b| b.kind == 6 && b.body == picture_block())
    );
    assert!(
        flac.blocks
            .iter()
            .any(|b| b.kind == 2 && b.body == b"riffbinary-app-data")
    );
    assert_eq!(payload(&path, Container::Flac), audio);
}

#[cfg(target_os = "macos")]
#[test]
fn finder_tags_creation_date_and_permissions_survive() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("tempdir");
    let path = mp3(dir.path(), "meta.mp3", &serato_tag());
    let finder = b"bplist00\xa1\x01UGreen\x0a";
    xattr::set(&path, "com.apple.metadata:_kMDItemUserTags", finder).expect("xattr");
    xattr::set(&path, "com.example.custom", b"value").expect("xattr");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod");
    let created = std::fs::metadata(&path)
        .expect("stat")
        .created()
        .expect("btime");
    std::thread::sleep(std::time::Duration::from_millis(20));

    write_comment(&roots(), &path, Some("#warmup")).expect("write");
    assert_eq!(
        xattr::get(&path, "com.apple.metadata:_kMDItemUserTags")
            .expect("get")
            .as_deref(),
        Some(&finder[..])
    );
    assert_eq!(
        xattr::get(&path, "com.example.custom")
            .expect("get")
            .as_deref(),
        Some(&b"value"[..])
    );
    let meta = std::fs::metadata(&path).expect("stat");
    assert_eq!(meta.permissions().mode() & 0o777, 0o640);
    assert_eq!(meta.created().expect("btime"), created);
    let leftovers = std::fs::read_dir(dir.path()).expect("list").count();
    assert_eq!(leftovers, 1, "no temporary files left behind");
}
