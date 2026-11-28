//! Field writes (title, artist, label, year, ...) on synthetic files.

mod common;

use std::path::Path;

use common::*;
use overtone_audio::AudioError;
use overtone_audio::tags::raw::{self, Container, RawFrame};
use overtone_audio::tags::{FieldEdit, WriteOutcome, read_metadata, write_fields};
use overtone_fs::ProtectedRoots;
use overtone_model::MetadataField;
use tempfile::TempDir;

fn roots() -> ProtectedRoots {
    ProtectedRoots::none()
}

fn edit(field: MetadataField, value: Option<&str>) -> FieldEdit {
    FieldEdit {
        field,
        value: value.map(str::to_string),
    }
}

fn frames(path: &Path, container: Container) -> Vec<RawFrame> {
    raw::inspect(path, container)
        .expect("walk")
        .id3v2
        .map_or_default(|tag| tag.frames)
}

fn binary(frames: &[RawFrame]) -> Vec<RawFrame> {
    frames
        .iter()
        .filter(|f| matches!(f.id.as_str(), "GEOB" | "PRIV" | "APIC" | "COMM" | "TXXX"))
        .cloned()
        .collect()
}

fn payload(path: &Path, container: Container) -> Vec<u8> {
    let layout = raw::inspect(path, container).expect("walk");
    let bytes = std::fs::read(path).expect("read");
    let start = usize::try_from(layout.payload.start).expect("offset");
    let end = usize::try_from(layout.payload.end).expect("offset");
    bytes[start..end].to_vec()
}

fn vorbis_entries(path: &Path) -> Vec<String> {
    raw::inspect(path, Container::Flac)
        .expect("walk")
        .flac
        .expect("flac")
        .vorbis_comment()
        .expect("parse")
        .expect("block")
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(e).into_owned())
        .collect()
}

#[test]
fn serato_mp3_gets_new_fields_and_keeps_everything_else() {
    let dir = TempDir::new().expect("tempdir");
    let path = mp3(dir.path(), "serato.mp3", &serato_tag());
    let before = binary(&frames(&path, Container::Mp3));
    let audio = payload(&path, Container::Mp3);
    let tags_before = read_metadata(&path).expect("read").tags;

    let outcome = write_fields(
        &roots(),
        &path,
        &[
            edit(MetadataField::Label, Some("Hospital Records")),
            edit(MetadataField::Year, Some("2015-06-04")),
            edit(MetadataField::Title, Some("  Cold Water (VIP)  ")),
        ],
    )
    .expect("write");
    assert!(matches!(outcome, WriteOutcome::Written(_)));
    let tags = read_metadata(&path).expect("read").tags;
    assert_eq!(tags.label.as_deref(), Some("Hospital Records"));
    assert_eq!(tags.year, Some(2015));
    assert_eq!(tags.title.as_deref(), Some("Cold Water (VIP)"));
    assert_eq!(tags.artist, tags_before.artist);
    assert_eq!(tags.comment, tags_before.comment);
    assert_eq!(binary(&frames(&path, Container::Mp3)), before);
    assert_eq!(payload(&path, Container::Mp3), audio);

    let again = write_fields(
        &roots(),
        &path,
        &[edit(MetadataField::Label, Some("Hospital Records"))],
    )
    .expect("again");
    assert_eq!(again, WriteOutcome::Unchanged);
}

#[test]
fn replaces_the_frame_the_scanner_reads_and_keeps_its_encoding() {
    let dir = TempDir::new().expect("tempdir");
    let tag = id3_tag(
        3,
        &[
            ("TIT2", 0, text("Old")),
            ("TYER", 0, text("2019")),
            ("TORY", 0, text("1999")),
            ("GRP1", 0, text("Rollers")),
            ("TIT1", 0, text("Work name")),
        ],
        128,
    );
    let path = mp3(dir.path(), "v23.mp3", &tag);
    write_fields(
        &roots(),
        &path,
        &[
            edit(MetadataField::Year, Some("2021")),
            edit(MetadataField::Grouping, Some("Steppers")),
            edit(MetadataField::Title, Some("Ünïcode ✓")),
        ],
    )
    .expect("write");
    let layout = raw::inspect(&path, Container::Mp3).expect("walk");
    let tag = layout.id3v2.expect("tag");
    assert_eq!(tag.major, 3);
    let get = |id: &str| {
        tag.frames
            .iter()
            .find(|f| f.id == id)
            .and_then(raw::decode_text_frame)
            .expect(id)
    };
    assert_eq!(get("TYER"), ["2021"]);
    assert_eq!(get("TORY"), ["1999"], "original year untouched");
    assert_eq!(get("GRP1"), ["Steppers"]);
    assert_eq!(get("TIT1"), ["Work name"], "iTunes Work untouched");
    assert_eq!(get("TIT2"), ["Ünïcode ✓"]);
    let tags = read_metadata(&path).expect("read").tags;
    assert_eq!(tags.year, Some(2021));
    assert_eq!(tags.grouping.as_deref(), Some("Steppers"));
}

#[test]
fn clearing_removes_every_frame_the_field_is_read_from() {
    let dir = TempDir::new().expect("tempdir");
    let tag = id3_tag(
        4,
        &[
            ("TDRC", 0, text("2019")),
            ("TDOR", 0, text("1999")),
            ("TPUB", 0, text("Label")),
            ("TIT2", 0, text("Title")),
        ],
        64,
    );
    let path = mp3(dir.path(), "clear.mp3", &tag);
    write_fields(
        &roots(),
        &path,
        &[
            edit(MetadataField::Year, None),
            edit(MetadataField::Label, Some("   ")),
        ],
    )
    .expect("write");
    let ids: Vec<String> = frames(&path, Container::Mp3)
        .into_iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(ids, ["TIT2"]);
    let tags = read_metadata(&path).expect("read").tags;
    assert_eq!(tags.year, None);
    assert_eq!(tags.label, None);
}

#[test]
fn wav_without_tags_gets_an_id3_chunk_with_the_fields() {
    let dir = TempDir::new().expect("tempdir");
    let path = iff_with_chunks(dir.path(), "plain.wav", false, &[]);
    let audio = payload(&path, Container::Wav);
    write_fields(
        &roots(),
        &path,
        &[
            edit(MetadataField::Artist, Some("Calibre")),
            edit(MetadataField::Bpm, Some("174")),
            edit(MetadataField::Key, Some("8A")),
        ],
    )
    .expect("write");
    let tags = read_metadata(&path).expect("read").tags;
    assert_eq!(tags.artist.as_deref(), Some("Calibre"));
    assert_eq!(tags.bpm, Some(174.0));
    assert_eq!(tags.key.as_deref(), Some("8A"));
    assert_eq!(payload(&path, Container::Wav), audio);
}

#[test]
fn aiff_with_serato_tag_keeps_geob_frames() {
    let dir = TempDir::new().expect("tempdir");
    let path = iff_with_chunks(dir.path(), "a.aiff", true, &[(b"ID3 ", serato_tag())]);
    let before = binary(&frames(&path, Container::Aiff));
    write_fields(
        &roots(),
        &path,
        &[edit(MetadataField::Genre, Some("Liquid"))],
    )
    .expect("write");
    assert_eq!(binary(&frames(&path, Container::Aiff)), before);
    assert_eq!(
        read_metadata(&path).expect("read").tags.genre.as_deref(),
        Some("Liquid")
    );
}

#[test]
fn flac_edits_the_key_in_use_and_leaves_serato_fields() {
    let dir = TempDir::new().expect("tempdir");
    let path = flac_with_blocks(
        dir.path(),
        "a.flac",
        &[
            (
                4,
                vorbis_block(
                    "reference libFLAC 1.4.3",
                    &[
                        "TITLE=Song",
                        "Organization=Old Label",
                        "SERATO_MARKERS_V2=YXBwbGljYXRpb24vb2N0ZXQtc3RyZWFtAABTZXJhdG8gTWFya2Vyc",
                        "COMMENT=8A",
                        "ARTIST=First",
                        "ARTIST=Second",
                        "DATE=",
                        "YEAR=2001",
                    ],
                ),
            ),
            (6, picture_block()),
            (1, vec![0; 512]),
        ],
    );
    let audio = payload(&path, Container::Flac);
    write_fields(
        &roots(),
        &path,
        &[
            edit(MetadataField::Label, Some("Hospital")),
            edit(MetadataField::Artist, Some("Calibre")),
            edit(MetadataField::Year, Some("2015")),
            edit(MetadataField::Remixer, Some("Lenzman")),
        ],
    )
    .expect("write");
    assert_eq!(
        vorbis_entries(&path),
        [
            "TITLE=Song",
            "Organization=Hospital",
            "SERATO_MARKERS_V2=YXBwbGljYXRpb24vb2N0ZXQtc3RyZWFtAABTZXJhdG8gTWFya2Vyc",
            "COMMENT=8A",
            "ARTIST=Calibre",
            "ARTIST=Second",
            "DATE=",
            "YEAR=2015",
            "REMIXER=Lenzman",
        ]
    );
    let tags = read_metadata(&path).expect("read").tags;
    assert_eq!(tags.label.as_deref(), Some("Hospital"));
    assert_eq!(tags.year, Some(2015));
    assert_eq!(tags.remixer.as_deref(), Some("Lenzman"));
    assert_eq!(payload(&path, Container::Flac), audio);

    write_fields(&roots(), &path, &[edit(MetadataField::Year, None)]).expect("clear");
    assert!(
        !vorbis_entries(&path)
            .iter()
            .any(|e| e.starts_with("DATE=") || e.starts_with("YEAR="))
    );
}

#[test]
fn refuses_bad_values_unwritable_fields_and_duplicates() {
    let dir = TempDir::new().expect("tempdir");
    let tag = id3_tag(4, &[("TPUB", 0, text("One")), ("TPUB", 0, text("Two"))], 64);
    let path = mp3(dir.path(), "dupe.mp3", &tag);
    let before = std::fs::read(&path).expect("read");
    let refused = |edits: &[FieldEdit], needle: &str| {
        match write_fields(&roots(), &path, edits) {
            Err(AudioError::WriteRefused { reason, .. }) => {
                assert!(reason.contains(needle), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(std::fs::read(&path).expect("read"), before);
    };
    refused(&[edit(MetadataField::Label, Some("Three"))], "several TPUB");
    refused(&[edit(MetadataField::Year, Some("soon"))], "isn't a year");
    refused(&[edit(MetadataField::Bpm, Some("fast"))], "isn't a BPM");
    refused(&[edit(MetadataField::Comment, Some("x"))], "doesn't write");
    refused(&[edit(MetadataField::Isrc, Some("x"))], "doesn't write");
    refused(
        &[
            edit(MetadataField::Title, Some("a")),
            edit(MetadataField::Title, Some("b")),
        ],
        "twice",
    );
}
