//! Surgical Comment writes: rebuild only the target `COMM` frame (ID3v2)
//! or `COMMENT` field (FLAC) and copy every other byte of the file as is.
//!
//! No tag library is involved, so frames lofty would normalise, merge, or
//! drop (ID3v2.4 frames in a v2.3 tag, duplicate frames, odd dates or
//! languages) survive byte for byte. The tag keeps its size whenever the
//! existing padding can absorb the change, so the audio doesn't move.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::raw::{
    self, Container, FlacLayout, Id3v2Layout, IffChunk, IffLayout, Layout, RawError, RawFrame,
};

/// Padding added when a tag has to grow, so the next edit fits in place.
const GROW_PADDING: usize = 2048;

/// Writes the original's bytes with the Comment replaced into `out`, which
/// must be a different file from `original`.
pub(crate) fn write_spliced(
    original: &Path,
    layout: &Layout,
    out: &Path,
    new_comment: Option<&str>,
) -> Result<(), RawError> {
    let mut src = File::open(original)?;
    let mut dst = BufWriter::new(File::create(out)?);
    match layout.container {
        Container::Mp3 => splice_mp3(&mut src, &mut dst, layout, new_comment)?,
        Container::Wav | Container::Aiff => splice_iff(&mut src, &mut dst, layout, new_comment)?,
        Container::Flac => splice_flac(&mut src, &mut dst, layout, new_comment)?,
    }
    let file = dst
        .into_inner()
        .map_err(|e| RawError::from(e.into_error()))?;
    file.sync_all()?;
    Ok(())
}

fn copy_range(src: &mut File, dst: &mut impl Write, start: u64, end: u64) -> io::Result<()> {
    src.seek(SeekFrom::Start(start))?;
    let copied = io::copy(&mut src.take(end - start), dst)?;
    if copied == end - start {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the file ended early",
        ))
    }
}

// ---------------------------------------------------------------- ID3v2

fn splice_mp3(
    src: &mut File,
    dst: &mut impl Write,
    layout: &Layout,
    new_comment: Option<&str>,
) -> Result<(), RawError> {
    let (old_len, tag) = match &layout.id3v2 {
        Some(tag) => (tag.range.end, rebuild_id3(tag, new_comment, None)?),
        None => match new_comment {
            None => (0, Vec::new()),
            Some(text) => (0, new_id3(text)?),
        },
    };
    dst.write_all(&tag)?;
    copy_range(src, dst, old_len, layout.file_len)?;
    Ok(())
}

/// A new ID3v2.4 tag holding only the Comment.
fn new_id3(text: &str) -> Result<Vec<u8>, RawError> {
    let empty = Id3v2Layout {
        major: 4,
        revision: 0,
        flags: 0,
        range: 0..0,
        frames: Vec::new(),
        padding: 0,
    };
    rebuild_id3(&empty, Some(text), None)
}

/// Re-serialises `tag` with the target Comment replaced, removed, or
/// appended. Every other frame is written from its stored bytes. With
/// `fixed_len`, the tag is padded to exactly that many bytes when it fits.
fn rebuild_id3(
    tag: &Id3v2Layout,
    new_comment: Option<&str>,
    fixed_len: Option<u64>,
) -> Result<Vec<u8>, RawError> {
    let target = tag.target_comment().map(|(index, _, _)| index);
    let mut frames: Vec<RawFrame> = Vec::with_capacity(tag.frames.len() + 1);
    for (index, frame) in tag.frames.iter().enumerate() {
        if Some(index) == target {
            if let Some(text) = new_comment {
                frames.push(RawFrame {
                    id: frame.id.clone(),
                    flags: frame.flags,
                    body: comm_body(tag.major, Some(frame), text),
                });
            }
        } else {
            frames.push(frame.clone());
        }
    }
    if target.is_none()
        && let Some(text) = new_comment
    {
        frames.push(RawFrame {
            id: "COMM".into(),
            flags: 0,
            body: comm_body(tag.major, None, text),
        });
    }

    let mut body = Vec::new();
    for frame in &frames {
        body.extend_from_slice(frame.id.as_bytes());
        let len = u32::try_from(frame.body.len())
            .map_err(|_| RawError::new("an ID3v2 frame is too large"))?;
        if tag.major == 4 {
            body.extend_from_slice(&to_synchsafe(len)?);
        } else {
            body.extend_from_slice(&len.to_be_bytes());
        }
        body.extend_from_slice(&frame.flags.to_be_bytes());
        body.extend_from_slice(&frame.body);
    }
    let old_total = fixed_len.unwrap_or(tag.range.end - tag.range.start);
    let total = if old_total >= 10 && (body.len() as u64) <= old_total - 10 {
        usize::try_from(old_total).map_err(|_| RawError::new("the ID3v2 tag is too large"))?
    } else {
        10 + body.len() + GROW_PADDING
    };
    let size =
        u32::try_from(total - 10).map_err(|_| RawError::new("the ID3v2 tag is too large"))?;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[tag.major, tag.revision, tag.flags]);
    out.extend_from_slice(&to_synchsafe(size)?);
    out.extend_from_slice(&body);
    out.resize(total, 0);
    Ok(out)
}

/// The body of a `COMM` frame holding `text`, keeping the old frame's
/// language and (when the text can be stored in it) its encoding and
/// description bytes.
fn comm_body(major: u8, old: Option<&RawFrame>, text: &str) -> Vec<u8> {
    let (old_encoding, language, prefix) = match old {
        Some(frame) if frame.body.len() >= 4 => {
            let encoding = frame.body[0];
            let language = [frame.body[1], frame.body[2], frame.body[3]];
            let prefix = description_with_terminator(encoding, &frame.body[4..]);
            (Some(encoding), language, prefix)
        }
        _ => (None, *b"eng", None),
    };
    let encoding = match old_encoding {
        Some(0) | None if is_latin1(text) => 0,
        Some(3) if major == 4 => 3,
        Some(2) if major == 4 => 2,
        _ => 1,
    };
    let mut body = vec![encoding];
    body.extend_from_slice(&language);
    match prefix {
        Some(bytes) if old_encoding == Some(encoding) => body.extend_from_slice(bytes),
        _ => body.extend_from_slice(&encode(encoding, "", true)),
    }
    body.extend_from_slice(&encode(encoding, text, false));
    body
}

/// The description and its terminator, exactly as stored.
fn description_with_terminator(encoding: u8, bytes: &[u8]) -> Option<&[u8]> {
    if matches!(encoding, 1 | 2) {
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == 0 && bytes[i + 1] == 0 {
                return Some(&bytes[..i + 2]);
            }
            i += 2;
        }
        None
    } else {
        bytes.iter().position(|b| *b == 0).map(|i| &bytes[..=i])
    }
}

fn encode(encoding: u8, text: &str, terminated: bool) -> Vec<u8> {
    let mut out = Vec::new();
    match encoding {
        0 => out.extend(
            text.chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')),
        ),
        3 => out.extend_from_slice(text.as_bytes()),
        2 => {
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        _ => {
            out.extend_from_slice(&[0xFF, 0xFE]);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
    }
    if terminated {
        out.push(0);
        if matches!(encoding, 1 | 2) {
            out.push(0);
        }
    }
    out
}

fn is_latin1(text: &str) -> bool {
    text.chars().all(|c| u32::from(c) <= 0xFF)
}

fn to_synchsafe(value: u32) -> Result<[u8; 4], RawError> {
    if value >= 1 << 28 {
        return Err(RawError::new("an ID3v2 size is too large"));
    }
    Ok([
        ((value >> 21) & 0x7F) as u8,
        ((value >> 14) & 0x7F) as u8,
        ((value >> 7) & 0x7F) as u8,
        (value & 0x7F) as u8,
    ])
}

// ---------------------------------------------------------------- WAV / AIFF

fn splice_iff(
    src: &mut File,
    dst: &mut impl Write,
    layout: &Layout,
    new_comment: Option<&str>,
) -> Result<(), RawError> {
    let iff = layout
        .iff
        .as_ref()
        .ok_or_else(|| RawError::new("missing chunk layout"))?;
    match (iff.id3_chunk(), &layout.id3v2) {
        (Some(chunk), Some(tag)) => {
            let data_len = chunk.data.end - chunk.data.start;
            let new_tag = rebuild_id3(tag, new_comment, Some(data_len))?;
            replace_chunk(src, dst, layout, iff, chunk, chunk.id, &new_tag)
        }
        (Some(_), None) => Err(RawError::new("the ID3 chunk holds no tag")),
        (None, _) => match new_comment {
            None => copy_range(src, dst, 0, layout.file_len).map_err(RawError::from),
            Some(text) => {
                let mut tag = new_id3(text)?;
                if tag.len() % 2 == 1 {
                    tag.push(0);
                }
                let id = *b"ID3 ";
                append_chunk(src, dst, layout, iff, id, &tag)
            }
        },
    }
}

fn size_bytes(value: u64, big_endian: bool) -> Result<[u8; 4], RawError> {
    let value = u32::try_from(value).map_err(|_| RawError::new("the file would exceed 4 GiB"))?;
    Ok(if big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    })
}

fn chunk_bytes(id: [u8; 4], data: &[u8], big_endian: bool) -> Result<Vec<u8>, RawError> {
    let mut out = Vec::with_capacity(data.len() + 9);
    out.extend_from_slice(&id);
    out.extend_from_slice(&size_bytes(data.len() as u64, big_endian)?);
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(out)
}

fn write_form_header(
    src: &mut File,
    dst: &mut impl Write,
    iff: &IffLayout,
    new_form_size: u64,
) -> Result<(), RawError> {
    let head = raw::read_range(src, 0..12)?;
    dst.write_all(&head[..4])?;
    dst.write_all(&size_bytes(new_form_size, iff.big_endian)?)?;
    dst.write_all(&head[8..12])?;
    Ok(())
}

fn replace_chunk(
    src: &mut File,
    dst: &mut impl Write,
    layout: &Layout,
    iff: &IffLayout,
    chunk: &IffChunk,
    id: [u8; 4],
    data: &[u8],
) -> Result<(), RawError> {
    let old = chunk.whole();
    // A final odd chunk may lack its pad byte; never read past the form.
    let old_end = old.end.min(iff.end);
    let new_chunk = chunk_bytes(id, data, iff.big_endian)?;
    let form_size = (iff.end - 8) - (old_end - old.start) + new_chunk.len() as u64;
    write_form_header(src, dst, iff, form_size)?;
    copy_range(src, dst, 12, old.start)?;
    dst.write_all(&new_chunk)?;
    copy_range(src, dst, old_end, layout.file_len)?;
    Ok(())
}

fn append_chunk(
    src: &mut File,
    dst: &mut impl Write,
    layout: &Layout,
    iff: &IffLayout,
    id: [u8; 4],
    data: &[u8],
) -> Result<(), RawError> {
    // If the last chunk is odd and lacks its pad byte, add it first.
    let missing_pad = iff
        .chunks
        .last()
        .is_some_and(|last| last.whole().end > iff.end);
    let new_chunk = chunk_bytes(id, data, iff.big_endian)?;
    let form_size = (iff.end - 8) + u64::from(missing_pad) + new_chunk.len() as u64;
    write_form_header(src, dst, iff, form_size)?;
    copy_range(src, dst, 12, iff.end)?;
    if missing_pad {
        dst.write_all(&[0])?;
    }
    dst.write_all(&new_chunk)?;
    copy_range(src, dst, iff.end, layout.file_len)?;
    Ok(())
}

// ---------------------------------------------------------------- FLAC

fn splice_flac(
    src: &mut File,
    dst: &mut impl Write,
    layout: &Layout,
    new_comment: Option<&str>,
) -> Result<(), RawError> {
    let flac = layout
        .flac
        .as_ref()
        .ok_or_else(|| RawError::new("missing FLAC layout"))?;
    if flac.id3_prefix.is_some() {
        return Err(RawError::new("ID3v2 in front of FLAC"));
    }
    let blocks = rebuild_flac_blocks(flac, new_comment)?;
    dst.write_all(b"fLaC")?;
    let count = blocks.len();
    for (i, (kind, body)) in blocks.iter().enumerate() {
        let len = u32::try_from(body.len())
            .ok()
            .filter(|len| *len < 1 << 24)
            .ok_or_else(|| RawError::new("a FLAC metadata block is too large"))?;
        let last = if i + 1 == count { 0x80 } else { 0 };
        dst.write_all(&[last | kind])?;
        dst.write_all(&len.to_be_bytes()[1..])?;
        dst.write_all(body)?;
    }
    copy_range(src, dst, flac.audio_start, layout.file_len)?;
    Ok(())
}

/// The metadata blocks with `VORBIS_COMMENT` rebuilt. The first `PADDING`
/// block grows or shrinks to keep the metadata the same size when it can.
fn rebuild_flac_blocks(
    flac: &FlacLayout,
    new_comment: Option<&str>,
) -> Result<Vec<(u8, Vec<u8>)>, RawError> {
    use raw::flac::{PADDING, VORBIS_COMMENT};
    let mut blocks: Vec<(u8, Vec<u8>)> = flac
        .blocks
        .iter()
        .map(|b| (b.kind, b.body.clone()))
        .collect();
    let existing = flac.vorbis_comment()?;
    let old_len = blocks
        .iter()
        .find(|(kind, _)| *kind == VORBIS_COMMENT)
        .map(|(_, body)| body.len());

    let mut entries = existing
        .as_ref()
        .map_or_else(Vec::new, |vc| vc.entries.clone());
    let index = entries
        .iter()
        .position(|e| raw::VorbisComment::is_comment_entry(e));
    match (index, new_comment) {
        (Some(i), Some(text)) => {
            let key_len = 8;
            let mut entry = entries[i][..key_len].to_vec();
            entry.extend_from_slice(text.as_bytes());
            entries[i] = entry;
        }
        (Some(i), None) => {
            entries.remove(i);
        }
        (None, Some(text)) => {
            let mut entry = b"COMMENT=".to_vec();
            entry.extend_from_slice(text.as_bytes());
            entries.push(entry);
        }
        (None, None) => return Ok(blocks),
    }
    let vendor = existing
        .as_ref()
        .map_or_else(|| b"Overtone".to_vec(), |vc| vc.vendor.clone());
    let body = vorbis_body(&vendor, &entries)?;
    let new_len = body.len();
    match blocks.iter_mut().find(|(kind, _)| *kind == VORBIS_COMMENT) {
        Some(block) => block.1 = body,
        None => blocks.insert(1, (VORBIS_COMMENT, body)),
    }

    // Metadata bytes before and after; a new block also costs a 4-byte header.
    let before = old_len.map_or(0, |len| len + 4);
    let after = new_len + 4;
    if let Some(pad) = blocks.iter_mut().find(|(kind, _)| *kind == PADDING) {
        let available = pad.1.len() + before;
        pad.1 = if available >= after {
            vec![0; available - after]
        } else {
            vec![0; GROW_PADDING]
        };
    } else if before != after {
        blocks.push((PADDING, vec![0; GROW_PADDING]));
    }
    Ok(blocks)
}

fn vorbis_body(vendor: &[u8], entries: &[Vec<u8>]) -> Result<Vec<u8>, RawError> {
    let len = |n: usize| {
        u32::try_from(n)
            .map(u32::to_le_bytes)
            .map_err(|_| RawError::new("a Vorbis comment is too large"))
    };
    let mut out = Vec::new();
    out.extend_from_slice(&len(vendor.len())?);
    out.extend_from_slice(vendor);
    out.extend_from_slice(&len(entries.len())?);
    for entry in entries {
        out.extend_from_slice(&len(entry.len())?);
        out.extend_from_slice(entry);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::raw::id3v2::tests::{build_tag, comm};

    fn parse(bytes: &[u8]) -> Id3v2Layout {
        raw::id3v2::parse(bytes, 0).expect("parse")
    }

    #[test]
    fn keeps_other_frames_and_size_when_padding_fits() {
        let tag = build_tag(
            3,
            &[
                ("TSOP", 0, b"\x00Sort".to_vec()),
                ("TSOP", 0, b"\x00Sort".to_vec()),
                ("COMM", 0, comm(0, *b"eng", "", "old")),
                ("GEOB", 0, b"\x00x\0y\0Serato Markers2\0\x01".to_vec()),
            ],
            64,
        );
        let layout = parse(&tag);
        let out = rebuild_id3(&layout, Some("new #vibe"), None).expect("rebuild");
        assert_eq!(out.len(), tag.len());
        let after = parse(&out);
        assert_eq!(after.frames[0], layout.frames[0]);
        assert_eq!(after.frames[1], layout.frames[1]);
        assert_eq!(after.frames[3], layout.frames[3]);
        assert_eq!(after.target_comment().expect("comment").2, "new #vibe");
    }

    #[test]
    fn grows_the_tag_when_padding_is_too_small() {
        let tag = build_tag(4, &[("COMM", 0, comm(0, *b"eng", "", "a"))], 0);
        let layout = parse(&tag);
        let out = rebuild_id3(&layout, Some(&"x".repeat(100)), None).expect("rebuild");
        assert!(out.len() > tag.len());
        assert_eq!(parse(&out).padding, GROW_PADDING as u64);
    }

    #[test]
    fn switches_latin1_to_utf16_for_wide_text_and_keeps_language() {
        let tag = build_tag(3, &[("COMM", 0, comm(0, *b"XXX", "", "a"))], 32);
        let layout = parse(&tag);
        let out = rebuild_id3(&layout, Some("Ünïcödé ✓"), None).expect("rebuild");
        let after = parse(&out);
        let (_, key, text) = after.target_comment().expect("comment");
        assert_eq!(&key.language, b"XXX");
        assert_eq!(text, "Ünïcödé ✓");
        assert_eq!(after.frames[0].body[0], 1);
    }

    #[test]
    fn removes_and_adds_comment_frames() {
        let tag = build_tag(
            4,
            &[
                ("TIT2", 0, b"\x03T".to_vec()),
                ("COMM", 0, comm(0, *b"eng", "", "a")),
            ],
            16,
        );
        let layout = parse(&tag);
        let removed = parse(&rebuild_id3(&layout, None, None).expect("rebuild"));
        assert_eq!(removed.frames.len(), 1);
        assert!(removed.target_comment().is_none());
        let added = parse(&rebuild_id3(&removed, Some("b"), None).expect("rebuild"));
        assert_eq!(added.target_comment().expect("comment").2, "b");
        assert_eq!(&added.frames[1].body[1..4], b"eng");
    }

    #[test]
    fn leaves_other_comment_frames_alone() {
        let tag = build_tag(
            3,
            &[
                ("COMM", 0, comm(0, *b"eng", "iTunNORM", " 000")),
                ("COMM", 0, comm(0, *b"eng", "", "main")),
            ],
            32,
        );
        let layout = parse(&tag);
        let after = parse(&rebuild_id3(&layout, Some("x"), None).expect("rebuild"));
        assert_eq!(after.frames[0], layout.frames[0]);
        assert_eq!(after.target_comment().expect("comment").2, "x");
    }
}
