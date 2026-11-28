//! Lossless WAV and AIFF to FLAC: the tag translation and the checks.
//!
//! ffmpeg encodes the audio (`overtone_core::convert` runs it); this module
//! translates the source's tags into FLAC metadata, assembles the final
//! file, and verifies it independently:
//!
//! - **Samples:** Symphonia decodes source and output and every sample must
//!   be identical. The content hash covers encoded bytes, so it changes by
//!   design and can't be the check.
//! - **Stream:** sample rate, channels, bit depth, and length match, and
//!   STREAMINFO carries an MD5 of the audio for later checks.
//! - **Tags:** the scanner reads the same title, artist, BPM, key, Comment,
//!   and so on from the FLAC as from the source.
//! - **Serato data:** every Serato GEOB frame (cues, loops, beatgrid,
//!   overview, autogain) is in the FLAC as Serato writes it there: a
//!   `SERATO_*` Vorbis field holding base64 of the GEOB body without its
//!   encoding byte (see `engineering/plans/metadata-and-conversion.md`).
//! - **Artwork:** every APIC frame becomes a PICTURE block.
//!
//! Frames with no FLAC equivalent are listed in [`FlacMetadata::dropped`]
//! so the DJ sees exactly what a conversion leaves behind.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use lofty::file::FileType;
use overtone_model::TrackTags;

use crate::AudioError;
use crate::analysis::Decoder;
use crate::tags::raw::{self, Container, Layout, RawFrame, decode_text_frame};

/// How Serato stores one GEOB frame in FLAC: the Vorbis key, and whether
/// the base64 keeps `=` padding and ends with a newline. Measured on 300
/// Serato-analysed FLAC files: Serato's own output differs by field.
const SERATO_FIELDS: &[(&str, &str, bool, bool)] = &[
    ("Serato Analysis", "SERATO_ANALYSIS", false, false),
    ("Serato Autotags", "SERATO_AUTOGAIN", true, true),
    ("Serato BeatGrid", "SERATO_BEATGRID", false, false),
    ("Serato Markers2", "SERATO_MARKERS_V2", false, false),
    ("Serato Overview", "SERATO_OVERVIEW", true, true),
    ("Serato Tempo Map", "SERATO_TEMPO_MAP", true, true),
    ("Serato RelVolAd", "SERATO_RELVOL", false, false),
    ("Serato VidAssoc", "SERATO_VIDEO_ASSOC", false, false),
];

/// GEOB frames Serato only uses in MP3 and AIFF/WAV. In FLAC their data
/// lives in `Serato Markers2`.
const SERATO_ID3_ONLY: &[&str] = &["Serato Markers_", "Serato Offsets_"];

/// Plain text frames and their Vorbis keys. The core fields the scanner
/// reads (title, artist, and so on) are written from its own reading
/// instead, so they read back identically.
const TEXT_FRAMES: &[(&str, &str)] = &[
    ("TRCK", "TRACKNUMBER"),
    ("TPOS", "DISCNUMBER"),
    ("TSRC", "ISRC"),
    ("TCOM", "COMPOSER"),
    ("TEXT", "LYRICIST"),
    ("TENC", "ENCODEDBY"),
    ("TSSE", "ENCODER"),
    ("TCOP", "COPYRIGHT"),
    ("TSOP", "ARTISTSORT"),
    ("TSOA", "ALBUMSORT"),
    ("TSOT", "TITLESORT"),
    ("TSO2", "ALBUMARTISTSORT"),
    ("TMED", "MEDIA"),
    ("TLAN", "LANGUAGE"),
    ("TDOR", "ORIGINALDATE"),
    ("TORY", "ORIGINALDATE"),
    ("TDRL", "RELEASEDATE"),
    ("TOPE", "ORIGINALARTIST"),
    ("TIT3", "SUBTITLE"),
    ("TPE3", "CONDUCTOR"),
    ("TMOO", "MOOD"),
    ("TCMP", "COMPILATION"),
];

/// Frames whose values come from the scanner's reading.
const CORE_FRAMES: &[&str] = &[
    "TIT2", "TPE1", "TALB", "TPE2", "TPE4", "TPUB", "TCON", "TDRC", "TYER", "TDAT", "TBPM", "TKEY",
    "GRP1", "TIT1",
];

/// Frames that describe the encoding, not the music, and are dropped
/// silently (FLAC records its own).
const IGNORED_FRAMES: &[&str] = &["TLEN", "TFLT"];

/// Everything that goes into the FLAC's metadata blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlacMetadata {
    /// Vorbis comment entries, `KEY=value`.
    pub entries: Vec<Vec<u8>>,
    /// PICTURE block bodies.
    pub pictures: Vec<Vec<u8>>,
    /// Serato GEOB descriptions carried over, such as `Serato Markers2`.
    pub serato: Vec<String>,
    /// Frames with no FLAC equivalent, described for the DJ.
    pub dropped: Vec<String>,
    /// What the scanner read from the source; the FLAC must read the same.
    pub expected_tags: TrackTags,
}

fn fail(path: &Path, reason: impl Into<String>) -> AudioError {
    AudioError::WriteRefused {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// Checks that `source` is WAV or AIFF holding integer PCM that FLAC can
/// store losslessly (up to 24 bits), and returns its layout.
pub fn check_convertible(source: &Path) -> Result<Layout, AudioError> {
    let container = crate::tags::write::writable_container(source)?;
    if !matches!(container, Container::Wav | Container::Aiff) {
        return Err(AudioError::UnsupportedFormat(
            "only WAV and AIFF files are converted to FLAC".into(),
        ));
    }
    let layout = raw::inspect(source, container).map_err(|e| fail(source, e.0))?;
    let iff = layout
        .iff
        .as_ref()
        .ok_or_else(|| fail(source, "no chunk layout"))?;
    let mut file = File::open(source).map_err(|e| AudioError::io(source, e))?;
    let format_chunk: &[u8; 4] = if container == Container::Wav {
        b"fmt "
    } else {
        b"COMM"
    };
    let chunk = iff
        .chunks
        .iter()
        .find(|c| &c.id == format_chunk)
        .ok_or_else(|| fail(source, "it has no format chunk"))?;
    let body = raw::read_range(&mut file, chunk.data.clone()).map_err(|e| fail(source, e.0))?;
    if container == Container::Wav {
        let tag = u16::from_le_bytes([body[0], body[1]]);
        let bits = u16::from_le_bytes([body[14], body[15]]);
        // 0xFFFE (extensible) carries the real tag in its subformat GUID.
        let real = if tag == 0xFFFE && body.len() >= 26 {
            u16::from_le_bytes([body[24], body[25]])
        } else {
            tag
        };
        if real != 1 {
            return Err(fail(
                source,
                format!(
                    "it holds floating-point or compressed audio (format {real}), which FLAC can't store losslessly"
                ),
            ));
        }
        if bits > 24 {
            return Err(fail(
                source,
                format!("it is {bits}-bit, deeper than FLAC's 24 bits"),
            ));
        }
    } else {
        let bits = u16::from_be_bytes([body[6], body[7]]);
        if &iff.form_type == b"AIFC" && body.len() >= 22 {
            let compression = &body[18..22];
            if !matches!(compression, b"NONE" | b"sowt" | b"twos") {
                return Err(fail(
                    source,
                    format!(
                        "it is AIFF-C with {} audio, which FLAC can't store losslessly",
                        String::from_utf8_lossy(compression)
                    ),
                ));
            }
        }
        if bits > 24 {
            return Err(fail(
                source,
                format!("it is {bits}-bit, deeper than FLAC's 24 bits"),
            ));
        }
    }
    Ok(layout)
}

fn entry(key: &str, value: &str) -> Vec<u8> {
    let mut out = key.as_bytes().to_vec();
    out.push(b'=');
    out.extend_from_slice(value.as_bytes());
    out
}

/// Whether `key` is a valid Vorbis comment field name.
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| (0x20..=0x7D).contains(&b) && b != b'=')
}

/// Builds the FLAC metadata for `source`.
pub fn flac_metadata_for(source: &Path, layout: &Layout) -> Result<FlacMetadata, AudioError> {
    let tags = crate::tags::write::scanner_view(source, layout)?;
    let mut meta = FlacMetadata {
        expected_tags: tags.clone(),
        ..FlacMetadata::default()
    };
    let raw_text = |id: &str| -> Option<Vec<String>> {
        layout
            .id3v2
            .as_ref()?
            .frames
            .iter()
            .find(|f| f.id == id)
            .and_then(decode_text_frame)
    };
    let mut push = |key: &str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            meta.entries.push(entry(key, &value));
        }
    };
    push("TITLE", tags.title.clone());
    push("ARTIST", tags.artist.clone());
    push("ALBUM", tags.album.clone());
    push("ALBUMARTIST", tags.album_artist.clone());
    push("REMIXER", tags.remixer.clone());
    push("LABEL", tags.label.clone());
    push("GENRE", tags.genre.clone());
    // Keep a full date when the source has one; the scanner reads the year.
    let date = raw_text("TDRC")
        .or_else(|| raw_text("TYER"))
        .and_then(|v| v.into_iter().next())
        .filter(|d| tags.year.is_some_and(|y| d.starts_with(&y.to_string())))
        .or_else(|| tags.year.map(|y| y.to_string()));
    push("DATE", date);
    // BPM as written (keeps "127.99"), when it parses the same.
    let bpm = raw_text("TBPM")
        .and_then(|v| v.into_iter().next())
        .filter(|text| crate::tags::parse_bpm(text) == tags.bpm)
        .or_else(|| tags.bpm.map(|b| b.to_string()));
    push("BPM", bpm);
    push("INITIALKEY", tags.key.clone());
    push("COMMENT", tags.comment.clone());
    push("GROUPING", tags.grouping.clone());

    let Some(id3) = &layout.id3v2 else {
        return Ok(meta);
    };
    let target_comment = id3.target_comment().map(|(index, _, _)| index);
    let mut serato_seen: Vec<String> = Vec::new();
    for (index, frame) in id3.frames.iter().enumerate() {
        let id = frame.id.as_str();
        if CORE_FRAMES.contains(&id) || IGNORED_FRAMES.contains(&id) {
            continue;
        }
        if id == "COMM" {
            if Some(index) != target_comment {
                let what = frame
                    .comment()
                    .map_or_else(|| "?".into(), |(key, _)| key.description);
                meta.dropped.push(format!("COMM ({what})"));
            }
            continue;
        }
        if let Some((_, key)) = TEXT_FRAMES.iter().find(|(f, _)| *f == id) {
            if let Some(values) = decode_text_frame(frame) {
                for value in values.into_iter().filter(|v| !v.is_empty()) {
                    meta.entries.push(entry(key, &value));
                }
            }
            continue;
        }
        match id {
            "TXXX" => match decode_text_frame(frame) {
                Some(strings) if strings.len() >= 2 && valid_key(&strings[0].to_uppercase()) => {
                    let key = strings[0].to_uppercase();
                    for value in strings[1..].iter().filter(|v| !v.is_empty()) {
                        meta.entries.push(entry(&key, value));
                    }
                }
                _ => meta.dropped.push("TXXX (unreadable description)".into()),
            },
            "USLT" => match decode_text_frame(frame) {
                // [language, description, lyrics]
                Some(strings) if strings.len() >= 3 => {
                    meta.entries.push(entry("LYRICS", &strings[2..].join("\n")));
                }
                _ => meta.dropped.push("USLT (unreadable)".into()),
            },
            "APIC" => match picture_block(frame) {
                Some(block) => meta.pictures.push(block),
                None => meta.dropped.push("APIC (unreadable artwork)".into()),
            },
            "GEOB" => match geob_parts(frame) {
                Some((description, payload)) => {
                    if let Some((_, key, pad, newline)) =
                        SERATO_FIELDS.iter().find(|(d, ..)| *d == description)
                    {
                        if !serato_seen.contains(&description) {
                            serato_seen.push(description.clone());
                            let mut body = b"application/octet-stream\0\0".to_vec();
                            body.extend_from_slice(description.as_bytes());
                            body.push(0);
                            body.extend_from_slice(&payload);
                            let mut value = key.as_bytes().to_vec();
                            value.push(b'=');
                            value
                                .extend_from_slice(serato_base64(&body, *pad, *newline).as_bytes());
                            meta.entries.push(value);
                            meta.serato.push(description);
                        }
                    } else if SERATO_ID3_ONLY.contains(&description.as_str()) {
                        // Superseded by Markers2 in FLAC; nothing is lost.
                    } else {
                        meta.dropped.push(format!("GEOB ({description})"));
                    }
                }
                None => meta.dropped.push("GEOB (unreadable)".into()),
            },
            "PRIV" => {
                let owner: String = frame
                    .body
                    .iter()
                    .take_while(|b| **b != 0)
                    .map(|b| char::from(*b))
                    .collect();
                meta.dropped.push(format!("PRIV ({owner})"));
            }
            other => meta.dropped.push(other.to_string()),
        }
    }
    Ok(meta)
}

/// A GEOB frame's description and payload.
fn geob_parts(frame: &RawFrame) -> Option<(String, Vec<u8>)> {
    let body = &frame.body;
    let encoding = *body.first()?;
    let mut pos = 1;
    // MIME type: always Latin-1, NUL-terminated.
    pos += body[pos..].iter().position(|b| *b == 0)? + 1;
    let wide = matches!(encoding, 1 | 2);
    let take_string = |pos: &mut usize| -> Option<Vec<u8>> {
        if wide {
            let mut i = *pos;
            while i + 1 < body.len() {
                if body[i] == 0 && body[i + 1] == 0 {
                    let s = body[*pos..i].to_vec();
                    *pos = i + 2;
                    return Some(s);
                }
                i += 2;
            }
            None
        } else {
            let end = *pos + body[*pos..].iter().position(|b| *b == 0)?;
            let s = body[*pos..end].to_vec();
            *pos = end + 1;
            Some(s)
        }
    };
    let _filename = take_string(&mut pos)?;
    let description = take_string(&mut pos)?;
    let description = if wide {
        let bytes = description
            .strip_prefix(&[0xFF, 0xFE])
            .map(|b| (b, true))
            .or_else(|| description.strip_prefix(&[0xFE, 0xFF]).map(|b| (b, false)))
            .unwrap_or((&description[..], false));
        let units: Vec<u16> = bytes
            .0
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if bytes.1 {
                    u16::from_le_bytes(*p)
                } else {
                    u16::from_be_bytes(*p)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        description.iter().map(|b| char::from(*b)).collect()
    };
    Some((description, body[pos..].to_vec()))
}

/// A FLAC PICTURE block body from an APIC frame.
fn picture_block(frame: &RawFrame) -> Option<Vec<u8>> {
    let body = &frame.body;
    let encoding = *body.first()?;
    let mime_end = 1 + body[1..].iter().position(|b| *b == 0)?;
    let mut mime = String::from_utf8_lossy(&body[1..mime_end]).into_owned();
    if mime == "image/jpg" {
        mime = "image/jpeg".into();
    }
    let kind = *body.get(mime_end + 1)?;
    let mut pos = mime_end + 2;
    let description = if matches!(encoding, 1 | 2) {
        let mut i = pos;
        loop {
            if i + 1 >= body.len() {
                return None;
            }
            if body[i] == 0 && body[i + 1] == 0 {
                break;
            }
            i += 2;
        }
        let strings = decode_text_frame(&RawFrame {
            id: "TXXX".into(),
            flags: 0,
            body: [&[encoding][..], &body[pos..i]].concat(),
        })
        .and_then(|v| v.into_iter().next())
        .unwrap_or_default();
        pos = i + 2;
        strings
    } else {
        let end = pos + body[pos..].iter().position(|b| *b == 0)?;
        let s: String = if encoding == 3 {
            String::from_utf8_lossy(&body[pos..end]).into_owned()
        } else {
            body[pos..end].iter().map(|b| char::from(*b)).collect()
        };
        pos = end + 1;
        s
    };
    let data = &body[pos..];
    let be = |n: usize| u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(&u32::from(kind).to_be_bytes());
    out.extend_from_slice(&be(mime.len()));
    out.extend_from_slice(mime.as_bytes());
    out.extend_from_slice(&be(description.len()));
    out.extend_from_slice(description.as_bytes());
    // Width, height, depth, and palette size: 0 means unknown.
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&be(data.len()));
    out.extend_from_slice(data);
    Some(out)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64, wrapped at 72 characters, the way Serato writes FLAC
/// fields.
pub fn serato_base64(data: &[u8], pad: bool, trailing_newline: bool) -> String {
    let mut plain = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                plain.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else if pad {
                plain.push('=');
            }
        }
    }
    let mut out = String::with_capacity(plain.len() + plain.len() / 72 + 1);
    for (i, line) in plain.as_bytes().chunks(72).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
    }
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// Decodes base64 leniently: ignores newlines and padding, and drops the
/// single stray character Serato sometimes leaves at the end.
pub fn decode_base64_lenient(text: &[u8]) -> Option<Vec<u8>> {
    let mut chars: Vec<u8> = text
        .iter()
        .copied()
        .filter(|c| !matches!(c, b'\n' | b'\r' | b'='))
        .collect();
    if chars.len() % 4 == 1 {
        chars.pop();
    }
    let value = |c: u8| {
        B64.iter()
            .position(|x| *x == c)
            .and_then(|p| u32::try_from(p).ok())
    };
    let mut out = Vec::with_capacity(chars.len() * 3 / 4);
    for chunk in chars.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// Writes the final FLAC at `out`: the encoded file's STREAMINFO and
/// SEEKTABLE, then `meta`'s Vorbis comment and pictures, 8 KiB of padding
/// for later tag edits, and the encoded audio frames.
pub fn assemble_flac(encoded: &Path, out: &Path, meta: &FlacMetadata) -> Result<(), AudioError> {
    let layout = raw::inspect(encoded, Container::Flac).map_err(|e| fail(encoded, e.0))?;
    let flac = layout
        .flac
        .as_ref()
        .ok_or_else(|| fail(encoded, "not FLAC"))?;
    let vendor = flac
        .vorbis_comment()
        .ok()
        .flatten()
        .map_or_else(|| b"Overtone".to_vec(), |vc| vc.vendor);
    let mut vorbis = Vec::new();
    let le = |n: usize| u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes();
    vorbis.extend_from_slice(&le(vendor.len()));
    vorbis.extend_from_slice(&vendor);
    vorbis.extend_from_slice(&le(meta.entries.len()));
    for e in &meta.entries {
        vorbis.extend_from_slice(&le(e.len()));
        vorbis.extend_from_slice(e);
    }
    let mut blocks: Vec<(u8, &[u8])> = flac
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, 0 | 3))
        .map(|b| (b.kind, b.body.as_slice()))
        .collect();
    blocks.push((4, &vorbis));
    for picture in &meta.pictures {
        blocks.push((6, picture));
    }
    let padding = vec![0u8; 8192];
    blocks.push((1, &padding));

    let mut dst = BufWriter::new(File::create(out).map_err(|e| AudioError::io(out, e))?);
    let io = |e| AudioError::io(out, e);
    dst.write_all(b"fLaC").map_err(io)?;
    for (i, (kind, body)) in blocks.iter().enumerate() {
        let len = u32::try_from(body.len())
            .ok()
            .filter(|l| *l < 1 << 24)
            .ok_or_else(|| fail(out, "a metadata block is too large for FLAC"))?;
        let last = if i + 1 == blocks.len() { 0x80 } else { 0 };
        dst.write_all(&[last | kind]).map_err(io)?;
        dst.write_all(&len.to_be_bytes()[1..]).map_err(io)?;
        dst.write_all(body).map_err(io)?;
    }
    let mut src = File::open(encoded).map_err(|e| AudioError::io(encoded, e))?;
    src.seek(SeekFrom::Start(flac.audio_start))
        .map_err(|e| AudioError::io(encoded, e))?;
    std::io::copy(&mut src.take(layout.file_len - flac.audio_start), &mut dst).map_err(io)?;
    let file = dst
        .into_inner()
        .map_err(|e| AudioError::io(out, e.into_error()))?;
    file.sync_all().map_err(|e| AudioError::io(out, e))?;
    Ok(())
}

/// What [`verify_conversion`] checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionCheck {
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u32,
}

fn mismatch(path: &Path, check: &'static str, detail: impl Into<String>) -> AudioError {
    AudioError::VerificationFailed {
        path: path.to_path_buf(),
        check,
        detail: detail.into(),
    }
}

/// Parses STREAMINFO: sample rate, channels, bits per sample, total
/// samples, and the MD5.
fn streaminfo(body: &[u8]) -> Option<(u32, u16, u32, u64, [u8; 16])> {
    if body.len() != 34 {
        return None;
    }
    let packed = u64::from_be_bytes(body[10..18].try_into().ok()?);
    let rate = u32::try_from(packed >> 44).ok()?;
    let channels = u16::try_from(((packed >> 41) & 0x7) + 1).ok()?;
    let bits = u32::try_from(((packed >> 36) & 0x1F) + 1).ok()?;
    let total = packed & 0xF_FFFF_FFFF;
    Some((rate, channels, bits, total, body[18..34].try_into().ok()?))
}

/// Checks the FLAC at `output` against `source` (see the module docs).
pub fn verify_conversion(
    source: &Path,
    output: &Path,
    meta: &FlacMetadata,
) -> Result<ConversionCheck, AudioError> {
    // Samples, block by block.
    let mut a = Decoder::open(source)?;
    let mut b = Decoder::open(output)?;
    let (ia, ib) = (a.info().clone(), b.info().clone());
    if ia.sample_rate_hz != ib.sample_rate_hz || ia.channels != ib.channels {
        return Err(mismatch(source, "stream", format!("{ia:?} became {ib:?}")));
    }
    if ia.bits_per_sample != ib.bits_per_sample {
        return Err(mismatch(
            source,
            "bit depth",
            format!("{:?} became {:?}", ia.bits_per_sample, ib.bits_per_sample),
        ));
    }
    let mut pending_a: Vec<f32> = Vec::new();
    let mut pending_b: Vec<f32> = Vec::new();
    let mut compared = 0u64;
    let (mut done_a, mut done_b) = (false, false);
    loop {
        while !done_a && pending_a.len() < 1 << 16 {
            match a.next_block()? {
                Some(block) => pending_a.extend_from_slice(block.samples),
                None => done_a = true,
            }
        }
        while !done_b && pending_b.len() < 1 << 16 {
            match b.next_block()? {
                Some(block) => pending_b.extend_from_slice(block.samples),
                None => done_b = true,
            }
        }
        let n = pending_a.len().min(pending_b.len());
        if let Some(at) = pending_a[..n]
            .iter()
            .zip(&pending_b[..n])
            .position(|(x, y)| x.to_bits() != y.to_bits())
        {
            return Err(mismatch(
                source,
                "samples",
                format!("sample {} differs", compared + at as u64),
            ));
        }
        compared += n as u64;
        pending_a.drain(..n);
        pending_b.drain(..n);
        if done_a && done_b {
            break;
        }
        if (done_a && pending_a.is_empty() && !pending_b.is_empty())
            || (done_b && pending_b.is_empty() && !pending_a.is_empty())
        {
            return Err(mismatch(
                source,
                "length",
                "the FLAC has a different number of samples",
            ));
        }
    }
    if !pending_a.is_empty() || !pending_b.is_empty() || compared == 0 {
        return Err(mismatch(
            source,
            "length",
            "the FLAC has a different number of samples",
        ));
    }
    let channels = u64::from(ia.channels.max(1));

    // STREAMINFO.
    let layout =
        raw::inspect(output, Container::Flac).map_err(|e| mismatch(output, "structure", e.0))?;
    let flac = layout
        .flac
        .as_ref()
        .ok_or_else(|| mismatch(output, "structure", "not FLAC"))?;
    let (rate, ch, bits, total, md5) = streaminfo(&flac.blocks[0].body)
        .ok_or_else(|| mismatch(output, "streaminfo", "unreadable STREAMINFO"))?;
    if rate != ia.sample_rate_hz || ch != ia.channels || total != compared / channels {
        return Err(mismatch(
            output,
            "streaminfo",
            "STREAMINFO disagrees with the audio",
        ));
    }
    if md5 == [0; 16] {
        return Err(mismatch(
            output,
            "streaminfo",
            "the FLAC has no MD5 of its audio",
        ));
    }

    // Vorbis comment and pictures, exactly as planned.
    let vc = flac
        .vorbis_comment()
        .map_err(|e| mismatch(output, "vorbis comment", e.0))?
        .ok_or_else(|| mismatch(output, "vorbis comment", "missing"))?;
    if vc.entries != meta.entries {
        return Err(mismatch(
            output,
            "vorbis comment",
            "the fields differ from the plan",
        ));
    }
    let pictures: Vec<&Vec<u8>> = flac
        .blocks
        .iter()
        .filter(|b| b.kind == 6)
        .map(|b| &b.body)
        .collect();
    if pictures.len() != meta.pictures.len()
        || pictures.iter().zip(&meta.pictures).any(|(x, y)| *x != y)
    {
        return Err(mismatch(
            output,
            "artwork",
            "PICTURE blocks differ from the plan",
        ));
    }

    // Serato payloads decode back to the source's GEOB payloads.
    for entry in &vc.entries {
        let Some(eq) = entry.iter().position(|b| *b == b'=') else {
            continue;
        };
        let key = String::from_utf8_lossy(&entry[..eq]).into_owned();
        if let Some((description, ..)) = SERATO_FIELDS.iter().find(|(_, k, ..)| *k == key) {
            let decoded = decode_base64_lenient(&entry[eq + 1..])
                .ok_or_else(|| mismatch(output, "serato", format!("{key} isn't base64")))?;
            let header = format!("application/octet-stream\0\0{description}\0");
            if !decoded.starts_with(header.as_bytes()) {
                return Err(mismatch(
                    output,
                    "serato",
                    format!("{key} has the wrong header"),
                ));
            }
        }
    }

    // The scanner reads the same tags.
    let read =
        crate::tags::read::read_as(output, FileType::Flac, crate::tags::read::scan_options())?;
    if read.tags != meta.expected_tags {
        return Err(mismatch(
            output,
            "tags",
            format!("expected {:?}, read {:?}", meta.expected_tags, read.tags),
        ));
    }
    Ok(ConversionCheck {
        frames: compared / channels,
        sample_rate: rate,
        channels: ch,
        bits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_serato_variants_and_round_trips() {
        assert_eq!(serato_base64(b"Ma", false, false), "TWE");
        assert_eq!(serato_base64(b"Ma", true, true), "TWE=\n");
        let long = vec![7u8; 100];
        let text = serato_base64(&long, true, true);
        assert!(text.lines().next().is_some_and(|l| l.len() == 72));
        assert_eq!(
            decode_base64_lenient(text.as_bytes()).as_deref(),
            Some(&long[..])
        );
        // Serato's stray last character is ignored.
        let whole = vec![7u8; 99];
        let mut junk = serato_base64(&whole, false, false).into_bytes();
        junk.push(b'A');
        assert_eq!(decode_base64_lenient(&junk).as_deref(), Some(&whole[..]));
    }

    #[test]
    fn reads_geob_parts_in_latin1_and_utf16() {
        let mut latin = vec![0u8];
        latin.extend_from_slice(b"application/octet-stream\0\0Serato Markers2\0\x01\x01AQ");
        let frame = RawFrame {
            id: "GEOB".into(),
            flags: 0,
            body: latin,
        };
        assert_eq!(
            geob_parts(&frame),
            Some(("Serato Markers2".into(), b"\x01\x01AQ".to_vec()))
        );
        let mut wide = vec![1u8];
        wide.extend_from_slice(b"application/octet-stream\0");
        wide.extend_from_slice(&[0xFF, 0xFE, 0, 0]);
        wide.extend_from_slice(&[0xFF, 0xFE]);
        for u in "Serato Overview".encode_utf16() {
            wide.extend_from_slice(&u.to_le_bytes());
        }
        wide.extend_from_slice(&[0, 0, 9, 9]);
        let frame = RawFrame {
            id: "GEOB".into(),
            flags: 0,
            body: wide,
        };
        assert_eq!(
            geob_parts(&frame),
            Some(("Serato Overview".into(), vec![9, 9]))
        );
    }

    #[test]
    fn apic_becomes_a_picture_block() {
        let mut body = vec![0u8];
        body.extend_from_slice(b"image/jpg\0");
        body.push(3);
        body.extend_from_slice(b"cover\0");
        body.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        let block = picture_block(&RawFrame {
            id: "APIC".into(),
            flags: 0,
            body,
        })
        .expect("picture");
        assert_eq!(&block[..4], &3u32.to_be_bytes());
        assert_eq!(&block[4..8], &10u32.to_be_bytes());
        assert_eq!(&block[8..18], b"image/jpeg");
        assert!(block.ends_with(&[0, 0, 0, 3, 0xFF, 0xD8, 0xFF]));
    }
}
