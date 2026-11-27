//! Builders for small tagged audio files in temporary directories.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use overtone_testkit::{Fixture, Signal, write_fixture};

pub fn short_signal() -> Signal {
    Signal {
        seconds: 0.25,
        ..Signal::cd(16_000.0)
    }
}

pub fn synchsafe(value: usize) -> [u8; 4] {
    let v = u32::try_from(value).expect("small");
    [
        u8::try_from((v >> 21) & 0x7F).expect("7 bits"),
        u8::try_from((v >> 14) & 0x7F).expect("7 bits"),
        u8::try_from((v >> 7) & 0x7F).expect("7 bits"),
        u8::try_from(v & 0x7F).expect("7 bits"),
    ]
}

/// An ID3v2 tag from `(id, flags, body)` frames.
pub fn id3_tag(major: u8, frames: &[(&str, u16, Vec<u8>)], padding: usize) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, flags, data) in frames {
        body.extend_from_slice(id.as_bytes());
        if major == 4 {
            body.extend_from_slice(&synchsafe(data.len()));
        } else {
            body.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
        }
        body.extend_from_slice(&flags.to_be_bytes());
        body.extend_from_slice(data);
    }
    body.resize(body.len() + padding, 0);
    let mut tag = b"ID3".to_vec();
    tag.extend_from_slice(&[major, 0, 0]);
    tag.extend_from_slice(&synchsafe(body.len()));
    tag.extend_from_slice(&body);
    tag
}

pub fn text(value: &str) -> Vec<u8> {
    let mut body = vec![0];
    body.extend_from_slice(value.as_bytes());
    body
}

pub fn comm(lang: [u8; 3], desc: &str, value: &str) -> Vec<u8> {
    let mut body = vec![0];
    body.extend_from_slice(&lang);
    body.extend_from_slice(desc.as_bytes());
    body.push(0);
    body.extend_from_slice(value.as_bytes());
    body
}

pub fn geob(description: &str, data: &[u8]) -> Vec<u8> {
    let mut body = vec![0];
    body.extend_from_slice(b"application/octet-stream\0\0");
    body.extend_from_slice(description.as_bytes());
    body.push(0);
    body.extend_from_slice(data);
    body
}

pub fn priv_frame(owner: &str, data: &[u8]) -> Vec<u8> {
    let mut body = owner.as_bytes().to_vec();
    body.push(0);
    body.extend_from_slice(data);
    body
}

pub const FAKE_JPEG: &[u8] =
    b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xFF\xD9";

/// An APIC frame with the `image/jpg` MIME type some taggers write.
pub fn apic() -> Vec<u8> {
    let mut body = vec![0];
    body.extend_from_slice(b"image/jpg\0");
    body.push(3);
    body.extend_from_slice(b"cover\0");
    body.extend_from_slice(FAKE_JPEG);
    body
}

/// MPEG-1 Layer III frames (128 kbps, 44.1 kHz) with silent bodies.
pub fn mpeg_frames(count: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..count {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        frame[200] = u8::try_from(i % 251).expect("byte");
        out.extend_from_slice(&frame);
    }
    out
}

pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

pub fn mp3(dir: &Path, name: &str, tag: &[u8]) -> PathBuf {
    let mut bytes = tag.to_vec();
    bytes.extend_from_slice(&mpeg_frames(60));
    write(dir, name, &bytes)
}

pub fn serato_tag() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tags/geob_serato.id3");
    std::fs::read(path).expect("geob_serato fixture")
}

/// A WAV or AIFF fixture with extra chunks (id, body) appended.
pub fn iff_with_chunks(
    dir: &Path,
    name: &str,
    aiff: bool,
    chunks: &[(&[u8; 4], Vec<u8>)],
) -> PathBuf {
    let path = dir.join(name);
    let fixture = if aiff {
        Fixture::Aiff { bits: 16 }
    } else {
        Fixture::Wav { bits: 16 }
    };
    write_fixture(&path, &short_signal(), fixture).expect("fixture");
    let mut bytes = std::fs::read(&path).expect("read");
    for (id, body) in chunks {
        bytes.extend_from_slice(*id);
        let len = u32::try_from(body.len()).expect("small");
        bytes.extend_from_slice(&if aiff {
            len.to_be_bytes()
        } else {
            len.to_le_bytes()
        });
        bytes.extend_from_slice(body);
        if body.len() % 2 == 1 {
            bytes.push(0);
        }
    }
    let size = u32::try_from(bytes.len() - 8).expect("small");
    bytes[4..8].copy_from_slice(&if aiff {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    std::fs::write(&path, &bytes).expect("write");
    path
}

pub fn vorbis_block(vendor: &str, entries: &[&str]) -> Vec<u8> {
    let len = |n: usize| u32::try_from(n).expect("small").to_le_bytes();
    let mut out = Vec::new();
    out.extend_from_slice(&len(vendor.len()));
    out.extend_from_slice(vendor.as_bytes());
    out.extend_from_slice(&len(entries.len()));
    for entry in entries {
        out.extend_from_slice(&len(entry.len()));
        out.extend_from_slice(entry.as_bytes());
    }
    out
}

pub fn picture_block() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&3u32.to_be_bytes());
    out.extend_from_slice(&10u32.to_be_bytes());
    out.extend_from_slice(b"image/jpeg");
    out.extend_from_slice(&5u32.to_be_bytes());
    out.extend_from_slice(b"cover");
    for value in [1u32, 1, 24, 0] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(&u32::try_from(FAKE_JPEG.len()).expect("small").to_be_bytes());
    out.extend_from_slice(FAKE_JPEG);
    out
}

/// A FLAC fixture whose metadata is STREAMINFO followed by `blocks`.
pub fn flac_with_blocks(dir: &Path, name: &str, blocks: &[(u8, Vec<u8>)]) -> PathBuf {
    let path = dir.join(name);
    write_fixture(&path, &short_signal(), Fixture::Flac { bits: 16 }).expect("fixture");
    let bytes = std::fs::read(&path).expect("read");
    assert_eq!(&bytes[..4], b"fLaC");
    let mut pos = 4;
    let mut streaminfo = None;
    loop {
        let last = bytes[pos] & 0x80 != 0;
        let kind = bytes[pos] & 0x7F;
        let len = usize::from(bytes[pos + 1]) << 16
            | usize::from(bytes[pos + 2]) << 8
            | usize::from(bytes[pos + 3]);
        if kind == 0 {
            streaminfo = Some(bytes[pos + 4..pos + 4 + len].to_vec());
        }
        pos += 4 + len;
        if last {
            break;
        }
    }
    let mut all = vec![(0u8, streaminfo.expect("streaminfo"))];
    all.extend(blocks.iter().cloned());
    let mut out = b"fLaC".to_vec();
    for (i, (kind, body)) in all.iter().enumerate() {
        let last = if i + 1 == all.len() { 0x80 } else { 0 };
        out.push(last | kind);
        let len = u32::try_from(body.len()).expect("small").to_be_bytes();
        out.extend_from_slice(&len[1..]);
        out.extend_from_slice(body);
    }
    out.extend_from_slice(&bytes[pos..]);
    std::fs::write(&path, &out).expect("write");
    path
}
