//! A minimal, strict FLAC metadata block lister and Vorbis comment parser.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use super::{RawError, read_range};

pub const STREAMINFO: u8 = 0;
pub const PADDING: u8 = 1;
pub const VORBIS_COMMENT: u8 = 4;
pub const PICTURE: u8 = 6;

/// The metadata blocks of a FLAC file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlacLayout {
    /// An ID3v2 tag in front of `fLaC`, which the spec discourages.
    pub id3_prefix: Option<Range<u64>>,
    pub blocks: Vec<FlacBlock>,
    /// First byte after the last metadata block.
    pub audio_start: u64,
}

/// One metadata block's type and body, exactly as stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlacBlock {
    pub kind: u8,
    pub body: Vec<u8>,
}

/// A parsed `VORBIS_COMMENT` block, with every entry kept as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VorbisComment {
    pub vendor: Vec<u8>,
    pub entries: Vec<Vec<u8>>,
}

impl VorbisComment {
    /// Whether `entry` is a `COMMENT=...` field (key compared case-insensitively).
    pub fn is_comment_entry(entry: &[u8]) -> bool {
        entry.len() >= 8 && entry[..8].eq_ignore_ascii_case(b"COMMENT=")
    }

    /// The value of the first `COMMENT` field, which Overtone reads and writes.
    pub fn first_comment(&self) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| Self::is_comment_entry(entry))
            .map(|entry| String::from_utf8_lossy(&entry[8..]).into_owned())
    }
}

pub fn block_type_name(kind: u8) -> &'static str {
    match kind {
        STREAMINFO => "STREAMINFO",
        PADDING => "PADDING",
        2 => "APPLICATION",
        3 => "SEEKTABLE",
        VORBIS_COMMENT => "VORBIS_COMMENT",
        5 => "CUESHEET",
        PICTURE => "PICTURE",
        _ => "reserved",
    }
}

impl FlacLayout {
    pub fn vorbis_comment(&self) -> Result<Option<VorbisComment>, RawError> {
        let mut comments = self.blocks.iter().filter(|b| b.kind == VORBIS_COMMENT);
        let Some(block) = comments.next() else {
            return Ok(None);
        };
        if comments.next().is_some() {
            return Err(RawError::new(
                "the FLAC file has more than one VORBIS_COMMENT block",
            ));
        }
        parse_vorbis_comment(&block.body).map(Some)
    }
}

pub(crate) fn walk(file: &mut File, file_len: u64) -> Result<FlacLayout, RawError> {
    let mut pos = 0u64;
    let mut id3_prefix = None;
    let head = read_range(file, 0..file_len.min(10))?;
    if head.starts_with(b"ID3") && head.len() == 10 {
        let size = u64::from(super::id3v2::synchsafe(&head[6..10])?);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        pos = 10 + size + footer;
        id3_prefix = Some(0..pos);
    }
    if pos + 4 > file_len || read_range(file, pos..pos + 4)? != b"fLaC" {
        return Err(RawError::new("missing the fLaC stream marker"));
    }
    pos += 4;
    file.seek(SeekFrom::Start(pos))?;
    let mut blocks = Vec::new();
    loop {
        let mut header = [0u8; 4];
        file.read_exact(&mut header)
            .map_err(|_| RawError::new("the FLAC metadata ends in the middle of a block"))?;
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7F;
        if kind == 127 {
            return Err(RawError::new(
                "the FLAC file has an invalid metadata block type",
            ));
        }
        let len = u64::from(u32::from_be_bytes([0, header[1], header[2], header[3]]));
        let body_start = pos + 4;
        if body_start + len > file_len {
            return Err(RawError::new(format!(
                "the FLAC {} block runs past the end of the file",
                block_type_name(kind)
            )));
        }
        let body = read_range(file, body_start..body_start + len)?;
        if blocks.is_empty() && (kind != STREAMINFO || len != 34) {
            return Err(RawError::new("the FLAC file doesn't start with STREAMINFO"));
        }
        blocks.push(FlacBlock { kind, body });
        pos = body_start + len;
        if last {
            break;
        }
    }
    Ok(FlacLayout {
        id3_prefix,
        blocks,
        audio_start: pos,
    })
}

pub fn parse_vorbis_comment(body: &[u8]) -> Result<VorbisComment, RawError> {
    let truncated = || RawError::new("the VORBIS_COMMENT block is truncated");
    let mut pos = 0usize;
    let mut take = |len: usize| -> Result<&[u8], RawError> {
        let slice = body.get(pos..pos + len).ok_or_else(truncated)?;
        pos += len;
        Ok(slice)
    };
    let read_len = |bytes: &[u8]| {
        usize::try_from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])).unwrap_or(0)
    };
    let vendor_len = read_len(take(4)?);
    let vendor = take(vendor_len)?.to_vec();
    let count = read_len(take(4)?);
    let mut entries = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let len = read_len(take(4)?);
        let entry = take(len)?.to_vec();
        if !entry.contains(&b'=') {
            return Err(RawError::new("a Vorbis comment field has no '='"));
        }
        entries.push(entry);
    }
    if pos != body.len() {
        // Trailing bytes (a framing bit from Ogg) are harmless but unexplained.
        return Err(RawError::new(
            "the VORBIS_COMMENT block has unexplained trailing bytes",
        ));
    }
    Ok(VorbisComment { vendor, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn vorbis_block(vendor: &str, entries: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let len = |n: usize| u32::try_from(n).expect("small").to_le_bytes();
        out.extend_from_slice(&len(vendor.len()));
        out.extend_from_slice(vendor.as_bytes());
        out.extend_from_slice(&len(entries.len()));
        for entry in entries {
            out.extend_from_slice(&len(entry.len()));
            out.extend_from_slice(entry.as_bytes());
        }
        out
    }

    #[test]
    fn parses_vorbis_entries_and_finds_comment() {
        let body = vorbis_block(
            "ref",
            &["TITLE=a", "comment=8A - Energy 6", "COMMENT=second"],
        );
        let parsed = parse_vorbis_comment(&body).expect("parse");
        assert_eq!(parsed.vendor, b"ref");
        assert_eq!(parsed.entries.len(), 3);
        assert_eq!(parsed.first_comment().as_deref(), Some("8A - Energy 6"));
    }

    #[test]
    fn rejects_truncated_or_malformed_comments() {
        let mut body = vorbis_block("ref", &["TITLE=a"]);
        body.pop();
        assert!(parse_vorbis_comment(&body).is_err());
        let body = vorbis_block("ref", &["NOEQUALS"]);
        assert!(parse_vorbis_comment(&body).is_err());
    }
}
