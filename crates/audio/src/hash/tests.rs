//! Byte-level tests: build each container, splice tag blocks in and out,
//! and check that only audio changes move the hash.

// Four-character codes read best as `b"mdat"` literals.
#![allow(clippy::trivially_copy_pass_by_ref)]

use std::path::{Path, PathBuf};

use overtone_model::AudioFormat;
use overtone_testkit::{Fixture, Signal, write_fixture};
use tempfile::TempDir;

use super::{HASH_SCHEME, audio_hash, hash_payload, payload_layout};
use crate::AudioError;

struct Scratch {
    dir: TempDir,
    count: std::cell::Cell<usize>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            dir: TempDir::new().expect("tempdir"),
            count: std::cell::Cell::new(0),
        }
    }

    /// Writes `bytes` to a fresh file. The `.bin` extension proves detection
    /// uses magic bytes, not the file name.
    fn file(&self, bytes: &[u8]) -> PathBuf {
        let n = self.count.get();
        self.count.set(n + 1);
        let path = self.dir.path().join(format!("{n}.bin"));
        std::fs::write(&path, bytes).expect("write");
        path
    }

    fn hash(&self, bytes: &[u8]) -> String {
        audio_hash(&self.file(bytes)).expect("hash")
    }

    fn hash_result(&self, bytes: &[u8]) -> Result<String, AudioError> {
        audio_hash(&self.file(bytes))
    }

    fn fixture(&self, fixture: Fixture) -> Vec<u8> {
        let signal = Signal {
            seconds: 0.2,
            ..Signal::cd(18_000.0)
        };
        let path = self
            .dir
            .path()
            .join(format!("fixture.{}", fixture.extension()));
        write_fixture(&path, &signal, fixture).expect("fixture");
        std::fs::read(&path).expect("read fixture")
    }
}

/// Flips one byte inside the payload, to prove audio changes move the hash.
fn flip_payload_byte(scratch: &Scratch, bytes: &[u8]) -> Vec<u8> {
    let layout = payload_layout(&scratch.file(bytes)).expect("layout");
    let range = &layout.ranges[0];
    let middle = usize::try_from(range.start + (range.end - range.start) / 2).expect("usize");
    let mut changed = bytes.to_vec();
    changed[middle] ^= 0x55;
    changed
}

/// Every truncation of `bytes` must return without panicking.
fn assert_no_panic_on_truncation(scratch: &Scratch, bytes: &[u8], step: usize) {
    for len in (0..bytes.len()).step_by(step.max(1)) {
        let _ = scratch.hash_result(&bytes[..len]);
    }
}

fn is_hash(value: &str) -> bool {
    value
        .strip_prefix(&format!("{HASH_SCHEME}:"))
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------- tag blocks

fn syncsafe(n: usize) -> [u8; 4] {
    let n = u32::try_from(n).expect("u32");
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// An ID3v2.3 tag with a TIT2 frame and `padding` zero bytes.
fn id3v2(title: &str, padding: usize) -> Vec<u8> {
    let mut frame = b"TIT2".to_vec();
    let text_len = u32::try_from(title.len() + 1).expect("u32");
    frame.extend_from_slice(&text_len.to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0]); // flags, then ISO-8859-1 encoding
    frame.extend_from_slice(title.as_bytes());
    frame.resize(frame.len() + padding, 0);
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend_from_slice(&syncsafe(frame.len()));
    tag.extend_from_slice(&frame);
    tag
}

/// An ID3v2.4 tag with a footer, as appended to the end of a stream.
fn id3v24_with_footer(title: &str) -> Vec<u8> {
    let mut body = b"TIT2".to_vec();
    body.extend_from_slice(&syncsafe(title.len() + 1));
    body.extend_from_slice(&[0, 0, 3]);
    body.extend_from_slice(title.as_bytes());
    let mut tag = b"ID3\x04\x00\x10".to_vec();
    tag.extend_from_slice(&syncsafe(body.len()));
    tag.extend_from_slice(&body);
    tag.extend_from_slice(b"3DI\x04\x00\x10");
    tag.extend_from_slice(&syncsafe(body.len()));
    tag
}

fn id3v1(title: &str) -> Vec<u8> {
    let mut tag = b"TAG".to_vec();
    tag.extend_from_slice(title.as_bytes());
    tag.resize(128, 0);
    tag
}

/// An APEv2 tag with header and footer around one text item.
fn ape(value: &str) -> Vec<u8> {
    let mut item = Vec::new();
    item.extend_from_slice(&u32::try_from(value.len()).expect("u32").to_le_bytes());
    item.extend_from_slice(&0_u32.to_le_bytes());
    item.extend_from_slice(b"Title\0");
    item.extend_from_slice(value.as_bytes());
    let size = u32::try_from(item.len() + 32).expect("u32");
    let block = |is_header: bool| {
        let mut b = b"APETAGEX".to_vec();
        b.extend_from_slice(&2000_u32.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&1_u32.to_le_bytes());
        let flags: u32 = (1 << 31) | if is_header { 1 << 29 } else { 0 };
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&[0; 8]);
        b
    };
    let mut tag = block(true);
    tag.extend_from_slice(&item);
    tag.extend_from_slice(&block(false));
    tag
}

fn lyrics3v2(text: &str) -> Vec<u8> {
    let mut body = b"LYRICSBEGIN".to_vec();
    body.extend_from_slice(b"LYR");
    body.extend_from_slice(format!("{:05}", text.len()).as_bytes());
    body.extend_from_slice(text.as_bytes());
    let mut tag = body.clone();
    tag.extend_from_slice(format!("{:06}", body.len()).as_bytes());
    tag.extend_from_slice(b"LYRICS200");
    tag
}

// ---------------------------------------------------------------- MP3 / ADTS

/// `count` MPEG-1 Layer III frames at 128 kbit/s, 44.1 kHz (417 bytes each),
/// with deterministic pseudo-random bodies. The first is a Xing-style frame.
fn mp3_frames(count: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..count {
        out.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        if i == 0 {
            let mut xing = vec![0_u8; 32];
            xing.extend_from_slice(b"Xing");
            xing.resize(413, 0);
            out.extend_from_slice(&xing);
        } else {
            out.extend((0..413_u32).map(|j| {
                u8::try_from((j * 31 + u32::try_from(i).expect("u32") * 7) % 251).expect("u8")
                    ^ seed
            }));
        }
    }
    out
}

/// ADTS AAC-LC frames, 44.1 kHz stereo, 200 bytes each.
fn adts_frames(count: usize) -> Vec<u8> {
    let len: u16 = 200;
    let mut out = Vec::new();
    for i in 0..count {
        let header = [
            0xFF,
            0xF1,
            0x50,
            0x80 | u8::try_from(len >> 11).expect("u8"),
            u8::try_from((len >> 3) & 0xFF).expect("u8"),
            (u8::try_from(len & 0x7).expect("u8") << 5) | 0x1F,
            0xFC,
        ];
        out.extend_from_slice(&header);
        out.extend((0..193).map(|j: usize| u8::try_from((j + i) % 200).expect("u8")));
    }
    out
}

#[test]
fn mp3_hash_ignores_leading_and_trailing_tags() {
    let scratch = Scratch::new();
    let audio = mp3_frames(20, 0);
    let bare = scratch.hash(&audio);
    assert!(is_hash(&bare), "{bare}");

    let variants: Vec<Vec<u8>> = vec![
        [id3v2("A", 0), audio.clone()].concat(),
        [id3v2("A much longer title", 4096), audio.clone()].concat(),
        // Two prepended tags, as left by tools that don't replace old tags.
        [id3v2("New", 10), id3v2("Old", 100), audio.clone()].concat(),
        // Padding zeros beyond the declared tag size.
        [id3v2("A", 0), vec![0; 777], audio.clone()].concat(),
        [audio.clone(), id3v1("Title")].concat(),
        [audio.clone(), ape("Title"), id3v1("Title")].concat(),
        [audio.clone(), lyrics3v2("la la"), id3v1("Title")].concat(),
        [audio.clone(), id3v24_with_footer("Appended")].concat(),
        [
            id3v2("Everything", 64),
            audio.clone(),
            ape("x"),
            lyrics3v2("y"),
            id3v1("z"),
        ]
        .concat(),
    ];
    for (i, variant) in variants.iter().enumerate() {
        assert_eq!(scratch.hash(variant), bare, "variant {i}");
    }

    let changed = flip_payload_byte(&scratch, &audio);
    assert_ne!(scratch.hash(&changed), bare);
    assert_ne!(scratch.hash(&mp3_frames(20, 1)), bare);
    // The Xing frame is payload: rewriting it changes the hash.
    let mut xing_changed = audio.clone();
    xing_changed[40] = 1;
    assert_ne!(scratch.hash(&xing_changed), bare);
}

#[test]
fn mp3_reports_format_and_payload_length() {
    let scratch = Scratch::new();
    let audio = mp3_frames(5, 0);
    let tagged = [id3v2("T", 30), audio.clone(), id3v1("T")].concat();
    let result = hash_payload(&scratch.file(&tagged)).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Mp3));
    assert_eq!(result.payload_len, audio.len() as u64);
}

#[test]
fn mp3_after_leading_junk_is_found_by_frame_chain() {
    let scratch = Scratch::new();
    let audio = mp3_frames(10, 0);
    let mut junky = vec![0x00, 0x13, 0xFF, 0xFB, 0x00];
    junky.extend_from_slice(&audio);
    assert_eq!(scratch.hash(&junky), scratch.hash(&audio));
}

#[test]
fn adts_hash_ignores_tags() {
    let scratch = Scratch::new();
    let audio = adts_frames(10);
    let bare = hash_payload(&scratch.file(&audio)).expect("hash");
    assert_eq!(bare.format, Some(AudioFormat::Aac));
    let tagged = [id3v2("A", 50), audio.clone(), id3v1("B")].concat();
    assert_eq!(scratch.hash(&tagged), bare.hash);
    assert_ne!(
        scratch.hash(&flip_payload_byte(&scratch, &audio)),
        bare.hash
    );
}

#[test]
fn mpeg_garbage_is_rejected_without_panicking() {
    let scratch = Scratch::new();
    let tagged = [id3v2("A", 10), mp3_frames(4, 0), ape("B"), id3v1("C")].concat();
    assert_no_panic_on_truncation(&scratch, &tagged, 7);

    // An ID3v2 tag with nothing after it.
    assert!(scratch.hash_result(&id3v2("only a tag", 100)).is_err());
    // A tag whose size runs past the end of the file.
    let mut short = id3v2("A", 100);
    short.truncate(50);
    assert!(scratch.hash_result(&short).is_err());
    // Frame-like bytes that don't chain into a stream.
    let mut fake = vec![0xFF, 0xFB, 0x90, 0x64];
    fake.extend_from_slice(&[0x11; 2000]);
    assert!(scratch.hash_result(&fake).is_err());
    // A corrupted APE footer that claims more bytes than exist.
    let mut bad_ape = [mp3_frames(4, 0), ape("B")].concat();
    let footer = bad_ape.len() - 32;
    bad_ape[footer + 12..footer + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        scratch.hash_result(&bad_ape),
        Err(AudioError::Malformed { .. })
    ));
}

// ---------------------------------------------------------------- FLAC

/// Splits a FLAC file into (STREAMINFO body, frames).
fn split_flac(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut pos = 4;
    let mut streaminfo = None;
    loop {
        let header = &bytes[pos..pos + 4];
        let len =
            usize::from(header[1]) << 16 | usize::from(header[2]) << 8 | usize::from(header[3]);
        if header[0] | 0x80 == 0x80 {
            streaminfo = Some(bytes[pos + 4..pos + 4 + len].to_vec());
        }
        pos += 4 + len;
        if header[0] & 0x80 != 0 {
            break;
        }
    }
    (streaminfo.expect("streaminfo"), bytes[pos..].to_vec())
}

/// Builds a FLAC file from STREAMINFO, extra `(type, body)` blocks, and frames.
fn build_flac(streaminfo: &[u8], blocks: &[(u8, Vec<u8>)], frames: &[u8]) -> Vec<u8> {
    let mut all = vec![(0_u8, streaminfo.to_vec())];
    all.extend(blocks.iter().cloned());
    let mut out = b"fLaC".to_vec();
    for (i, (kind, body)) in all.iter().enumerate() {
        let last = if i == all.len() - 1 { 0x80 } else { 0 };
        let len = u32::try_from(body.len()).expect("u32").to_be_bytes();
        out.push(last | kind);
        out.extend_from_slice(&len[1..]);
        out.extend_from_slice(body);
    }
    out.extend_from_slice(frames);
    out
}

fn vorbis_comment(entries: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    let vendor = b"overtone test";
    out.extend_from_slice(&u32::try_from(vendor.len()).expect("u32").to_le_bytes());
    out.extend_from_slice(vendor);
    out.extend_from_slice(&u32::try_from(entries.len()).expect("u32").to_le_bytes());
    for entry in entries {
        out.extend_from_slice(&u32::try_from(entry.len()).expect("u32").to_le_bytes());
        out.extend_from_slice(entry.as_bytes());
    }
    out
}

#[test]
fn flac_hash_ignores_metadata_blocks() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Flac { bits: 16 });
    let (streaminfo, frames) = split_flac(&original);
    let bare = build_flac(&streaminfo, &[], &frames);
    let expected = scratch.hash(&original);
    assert_eq!(scratch.hash(&bare), expected);

    let picture = vec![7_u8; 5000];
    let variants = [
        build_flac(&streaminfo, &[(4, vorbis_comment(&["TITLE=A"]))], &frames),
        build_flac(
            &streaminfo,
            &[
                (
                    4,
                    vorbis_comment(&["TITLE=A much longer title", "COMMENT=Dubs"]),
                ),
                (6, picture.clone()),
                (1, vec![0; 8192]),
            ],
            &frames,
        ),
        build_flac(&streaminfo, &[(1, vec![0; 10]), (6, picture)], &frames),
        // ID3v2 prepended and ID3v1 appended by MP3-minded taggers.
        [id3v2("Stray", 20), bare.clone(), id3v1("Stray")].concat(),
        [bare.clone(), ape("Stray")].concat(),
    ];
    for (i, variant) in variants.iter().enumerate() {
        assert_eq!(scratch.hash(variant), expected, "variant {i}");
    }
    let result = hash_payload(&scratch.file(&bare)).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Flac));
    assert_eq!(result.payload_len, frames.len() as u64);

    assert_ne!(scratch.hash(&flip_payload_byte(&scratch, &bare)), expected);
}

#[test]
fn flac_garbage_is_rejected_without_panicking() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Flac { bits: 16 });
    let (streaminfo, frames) = split_flac(&original);
    let tagged = build_flac(&streaminfo, &[(4, vorbis_comment(&["A=B"]))], &frames);
    assert_no_panic_on_truncation(&scratch, &tagged, 97);
    // Truncated inside the metadata.
    assert!(scratch.hash_result(&tagged[..30]).is_err());
    // Metadata only, no frames.
    assert!(
        scratch
            .hash_result(&build_flac(&streaminfo, &[], &[]))
            .is_err()
    );
    // A last-block flag that's never set runs off the end.
    let mut no_last = build_flac(&streaminfo, &[], &[]);
    no_last[4] &= 0x7F;
    assert!(scratch.hash_result(&no_last).is_err());
    // Bytes after the metadata that aren't a frame.
    let garbled = build_flac(&streaminfo, &[], &[0x12; 100]);
    assert!(matches!(
        scratch.hash_result(&garbled),
        Err(AudioError::Malformed { .. })
    ));
}

// ---------------------------------------------------------------- WAV

/// Splits a RIFF/FORM file into its `(id, body)` chunks after the 12-byte header.
fn chunks(bytes: &[u8], big_endian: bool) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[pos..pos + 4].try_into().expect("id");
        let size_bytes: [u8; 4] = bytes[pos + 4..pos + 8].try_into().expect("size");
        let size = if big_endian {
            u32::from_be_bytes(size_bytes)
        } else {
            u32::from_le_bytes(size_bytes)
        } as usize;
        out.push((id, bytes[pos + 8..pos + 8 + size].to_vec()));
        pos += 8 + size + (size & 1);
    }
    out
}

fn build_chunked(
    magic: &[u8; 4],
    form: &[u8; 4],
    chunks: &[([u8; 4], Vec<u8>)],
    be: bool,
) -> Vec<u8> {
    let size_bytes = |n: usize| {
        let n = u32::try_from(n).expect("u32");
        if be { n.to_be_bytes() } else { n.to_le_bytes() }
    };
    let mut body = form.to_vec();
    for (id, data) in chunks {
        body.extend_from_slice(id);
        body.extend_from_slice(&size_bytes(data.len()));
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = magic.to_vec();
    out.extend_from_slice(&size_bytes(body.len()));
    out.extend_from_slice(&body);
    out
}

fn list_info(title: &str) -> Vec<u8> {
    let mut out = b"INFO".to_vec();
    out.extend_from_slice(b"INAM");
    let mut text = title.as_bytes().to_vec();
    text.push(0);
    out.extend_from_slice(&u32::try_from(text.len()).expect("u32").to_le_bytes());
    out.extend_from_slice(&text);
    if text.len() % 2 == 1 {
        out.push(0);
    }
    out
}

#[test]
fn wav_hash_ignores_info_and_id3_chunks() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Wav { bits: 24 });
    let parts = chunks(&original, false);
    let fmt = parts
        .iter()
        .find(|(id, _)| id == b"fmt ")
        .expect("fmt")
        .clone();
    let data = parts
        .iter()
        .find(|(id, _)| id == b"data")
        .expect("data")
        .clone();
    let expected = scratch.hash(&original);
    let wav = |chunks: &[([u8; 4], Vec<u8>)]| build_chunked(b"RIFF", b"WAVE", chunks, false);

    let variants = [
        wav(&[fmt.clone(), (*b"LIST", list_info("A")), data.clone()]),
        wav(&[
            fmt.clone(),
            (*b"LIST", list_info("Much longer")),
            data.clone(),
        ]),
        wav(&[fmt.clone(), data.clone(), (*b"id3 ", id3v2("Title", 300))]),
        wav(&[
            fmt.clone(),
            (*b"bext", vec![1; 603]),
            data.clone(),
            (*b"LIST", list_info("After")),
        ]),
    ];
    for (i, variant) in variants.iter().enumerate() {
        assert_eq!(scratch.hash(variant), expected, "variant {i}");
    }
    let result = hash_payload(&scratch.file(&variants[3])).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Wav));
    assert_eq!(result.payload_len, data.1.len() as u64);
    assert_ne!(
        scratch.hash(&flip_payload_byte(&scratch, &original)),
        expected
    );
}

#[test]
fn rf64_uses_the_ds64_data_size() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Wav { bits: 16 });
    let parts = chunks(&original, false);
    let fmt = parts
        .iter()
        .find(|(id, _)| id == b"fmt ")
        .expect("fmt")
        .clone();
    let data = parts
        .iter()
        .find(|(id, _)| id == b"data")
        .expect("data")
        .1
        .clone();

    let mut ds64 = Vec::new();
    ds64.extend_from_slice(&0_u64.to_le_bytes()); // RIFF size, unused here
    ds64.extend_from_slice(&(data.len() as u64).to_le_bytes());
    ds64.extend_from_slice(&0_u64.to_le_bytes());
    ds64.extend_from_slice(&0_u32.to_le_bytes());
    let mut rf64 = build_chunked(b"RF64", b"WAVE", &[(*b"ds64", ds64), fmt], false);
    rf64.extend_from_slice(b"data");
    rf64.extend_from_slice(&u32::MAX.to_le_bytes());
    rf64.extend_from_slice(&data);
    assert_eq!(scratch.hash(&rf64), scratch.hash(&original));
}

#[test]
fn wav_garbage_is_rejected_without_panicking() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Wav { bits: 16 });
    assert_no_panic_on_truncation(&scratch, &original, 113);
    // Truncated inside the data chunk.
    assert!(
        scratch
            .hash_result(&original[..original.len() - 1])
            .is_err()
    );
    // No data chunk.
    let parts = chunks(&original, false);
    let fmt_only = build_chunked(b"RIFF", b"WAVE", &parts[..1], false);
    assert!(scratch.hash_result(&fmt_only).is_err());
}

// ---------------------------------------------------------------- AIFF

#[test]
fn aiff_hash_ignores_tag_chunks() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Aiff { bits: 16 });
    let parts = chunks(&original, true);
    let comm = parts
        .iter()
        .find(|(id, _)| id == b"COMM")
        .expect("COMM")
        .clone();
    let ssnd = parts
        .iter()
        .find(|(id, _)| id == b"SSND")
        .expect("SSND")
        .clone();
    let expected = scratch.hash(&original);
    let aiff = |chunks: &[([u8; 4], Vec<u8>)]| build_chunked(b"FORM", b"AIFF", chunks, true);

    let variants = [
        aiff(&[comm.clone(), (*b"NAME", b"Odd".to_vec()), ssnd.clone()]),
        aiff(&[comm.clone(), ssnd.clone(), (*b"ID3 ", id3v2("Title", 10))]),
        aiff(&[
            (*b"ID3 ", id3v2("Much longer title", 2000)),
            comm.clone(),
            ssnd.clone(),
        ]),
        build_chunked(b"FORM", b"AIFC", &[comm.clone(), ssnd.clone()], true),
    ];
    for (i, variant) in variants.iter().enumerate() {
        assert_eq!(scratch.hash(variant), expected, "variant {i}");
    }
    let result = hash_payload(&scratch.file(&original)).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Aiff));
    assert_eq!(result.payload_len, ssnd.1.len() as u64 - 8);
    assert_ne!(
        scratch.hash(&flip_payload_byte(&scratch, &original)),
        expected
    );
}

#[test]
fn aiff_garbage_is_rejected_without_panicking() {
    let scratch = Scratch::new();
    let original = scratch.fixture(Fixture::Aiff { bits: 16 });
    assert_no_panic_on_truncation(&scratch, &original, 101);
    assert!(
        scratch
            .hash_result(&original[..original.len() - 3])
            .is_err()
    );
    let parts = chunks(&original, true);
    let comm_only = build_chunked(b"FORM", b"AIFF", &parts[..1], true);
    assert!(scratch.hash_result(&comm_only).is_err());
}

// ---------------------------------------------------------------- MP4

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(body.len() + 8)
        .expect("u32")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn atom64(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = 1_u32.to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(&(body.len() as u64 + 16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn ftyp() -> Vec<u8> {
    atom(b"ftyp", b"M4A \x00\x00\x02\x00M4A mp42isom")
}

/// A minimal `moov` with one track whose sample entry is `codec`, and a
/// `udta/meta/ilst` title of `title`.
fn moov(codec: &[u8; 4], title: &str) -> Vec<u8> {
    let mut stsd_body = vec![0, 0, 0, 0, 0, 0, 0, 1];
    stsd_body.extend_from_slice(&atom(codec, &[0; 28]));
    let stbl = atom(b"stbl", &atom(b"stsd", &stsd_body));
    let trak = atom(b"trak", &atom(b"mdia", &atom(b"minf", &stbl)));
    let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
    data.extend_from_slice(title.as_bytes());
    let ilst = atom(b"ilst", &atom(b"\xA9nam", &atom(b"data", &data)));
    let mut meta = vec![0, 0, 0, 0];
    meta.extend_from_slice(&ilst);
    let udta = atom(b"udta", &atom(b"meta", &meta));
    atom(b"moov", &[atom(b"mvhd", &[0; 100]), trak, udta].concat())
}

fn sample_data(seed: u8) -> Vec<u8> {
    (0..5000_u32)
        .map(|i| u8::try_from(i % 241).expect("u8") ^ seed)
        .collect()
}

#[test]
fn mp4_hash_covers_only_mdat() {
    let scratch = Scratch::new();
    let samples = sample_data(0);
    let base = [ftyp(), moov(b"mp4a", "A"), atom(b"mdat", &samples)].concat();
    let expected = scratch.hash(&base);

    let variants = [
        [
            ftyp(),
            moov(b"mp4a", "A much longer title"),
            atom(b"mdat", &samples),
        ]
        .concat(),
        // moov after mdat, as many encoders write it.
        [ftyp(), atom(b"mdat", &samples), moov(b"mp4a", "B")].concat(),
        [
            ftyp(),
            moov(b"mp4a", "C"),
            atom(b"free", &[0; 333]),
            atom64(b"mdat", &samples),
        ]
        .concat(),
        // Size 0: the last atom runs to the end of the file.
        [ftyp(), moov(b"mp4a", "D"), {
            let mut mdat = atom(b"mdat", &samples);
            mdat[..4].copy_from_slice(&[0; 4]);
            mdat
        }]
        .concat(),
    ];
    for (i, variant) in variants.iter().enumerate() {
        assert_eq!(scratch.hash(variant), expected, "variant {i}");
    }

    let result = hash_payload(&scratch.file(&base)).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Aac));
    assert_eq!(result.payload_len, samples.len() as u64);
    let alac = [ftyp(), moov(b"alac", "A"), atom(b"mdat", &samples)].concat();
    let result = hash_payload(&scratch.file(&alac)).expect("hash");
    assert_eq!(result.format, Some(AudioFormat::Alac));
    assert_eq!(result.hash, expected);

    let changed = [ftyp(), moov(b"mp4a", "A"), atom(b"mdat", &sample_data(1))].concat();
    assert_ne!(scratch.hash(&changed), expected);
}

#[test]
fn mp4_with_several_mdat_atoms_hashes_them_in_order() {
    let scratch = Scratch::new();
    let (a, b) = (sample_data(3), sample_data(4));
    let split = [
        ftyp(),
        moov(b"mp4a", "A"),
        atom(b"mdat", &a),
        atom(b"mdat", &b),
    ]
    .concat();
    let swapped = [
        ftyp(),
        moov(b"mp4a", "A"),
        atom(b"mdat", &b),
        atom(b"mdat", &a),
    ]
    .concat();
    let layout = payload_layout(&scratch.file(&split)).expect("layout");
    assert_eq!(layout.ranges.len(), 2);
    assert_ne!(scratch.hash(&split), scratch.hash(&swapped));
}

#[test]
fn mp4_garbage_is_rejected_without_panicking() {
    let scratch = Scratch::new();
    let file = [ftyp(), moov(b"mp4a", "A"), atom(b"mdat", &sample_data(0))].concat();
    assert_no_panic_on_truncation(&scratch, &file, 13);
    // Truncated mdat.
    assert!(scratch.hash_result(&file[..file.len() - 10]).is_err());
    // No mdat at all.
    assert!(
        scratch
            .hash_result(&[ftyp(), moov(b"mp4a", "A")].concat())
            .is_err()
    );
    // An atom size smaller than its header.
    let mut tiny = [ftyp(), atom(b"mdat", &sample_data(0))].concat();
    let at = ftyp().len();
    tiny[at..at + 4].copy_from_slice(&4_u32.to_be_bytes());
    assert!(scratch.hash_result(&tiny).is_err());
    // A garbled atom type.
    let mut garbled = file.clone();
    garbled[ftyp().len() + 4] = 0x01;
    assert!(scratch.hash_result(&garbled).is_err());
}

// ---------------------------------------------------------------- general

#[test]
fn unknown_and_empty_files_are_errors() {
    let scratch = Scratch::new();
    for bytes in [
        Vec::new(),
        b"just some text, not audio".to_vec(),
        vec![0; 4096],
        (0..4096_u32)
            .map(|i| u8::try_from(i * 7 % 256).expect("u8"))
            .collect(),
    ] {
        assert!(
            matches!(
                scratch.hash_result(&bytes),
                Err(AudioError::UnsupportedFormat(_))
            ),
            "{bytes:?}"
        );
    }
    assert!(matches!(
        audio_hash(Path::new("/nonexistent/overtone/test.flac")),
        Err(AudioError::Io { .. })
    ));
}

#[test]
fn random_corruption_never_panics() {
    let scratch = Scratch::new();
    let flac = scratch.fixture(Fixture::Flac { bits: 16 });
    let wav = scratch.fixture(Fixture::Wav { bits: 16 });
    let mp3 = [id3v2("A", 10), mp3_frames(6, 0), id3v1("B")].concat();
    let mp4 = [ftyp(), moov(b"mp4a", "A"), atom(b"mdat", &sample_data(0))].concat();
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for base in [&flac, &wav, &mp3, &mp4] {
        // Corrupt the header region, where the finders make decisions.
        let header_len = base.len().min(600);
        for _ in 0..200 {
            let mut bytes = base.clone();
            for _ in 0..4 {
                let at = usize::try_from(next()).expect("usize") % header_len;
                bytes[at] = u8::try_from(next() & 0xFF).expect("u8");
            }
            let _ = scratch.hash_result(&bytes);
        }
    }
}
