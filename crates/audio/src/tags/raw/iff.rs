//! A minimal, strict RIFF (WAV) and FORM (AIFF) chunk lister.

use std::fs::File;
use std::ops::Range;

use super::{RawError, read_range};

/// The top-level chunks of a WAV or AIFF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffLayout {
    pub big_endian: bool,
    /// `WAVE`, `AIFF`, or `AIFC`.
    pub form_type: [u8; 4],
    pub chunks: Vec<IffChunk>,
    /// End of the RIFF/FORM chunk; anything after it is trailing data.
    pub end: u64,
}

/// One chunk: its ID, and the file range of its body (without the 8-byte
/// header or pad byte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffChunk {
    pub id: [u8; 4],
    pub data: Range<u64>,
}

impl IffChunk {
    pub fn id_str(&self) -> String {
        String::from_utf8_lossy(&self.id).into_owned()
    }

    pub fn is_id3(&self) -> bool {
        self.id.eq_ignore_ascii_case(b"ID3 ")
    }

    /// The range including header and pad byte.
    pub fn whole(&self) -> Range<u64> {
        let len = self.data.end - self.data.start;
        self.data.start - 8..self.data.end + (len % 2)
    }
}

impl IffLayout {
    pub fn id3_chunk(&self) -> Option<&IffChunk> {
        self.chunks.iter().find(|chunk| chunk.is_id3())
    }

    pub fn audio_chunk(&self) -> Result<&IffChunk, RawError> {
        let id: &[u8; 4] = if self.big_endian { b"SSND" } else { b"data" };
        let mut matching = self.chunks.iter().filter(|chunk| &chunk.id == id);
        let chunk = matching
            .next()
            .ok_or_else(|| RawError::new(format!("no {} chunk", String::from_utf8_lossy(id))))?;
        if matching.next().is_some() {
            return Err(RawError::new(format!(
                "more than one {} chunk",
                String::from_utf8_lossy(id)
            )));
        }
        Ok(chunk)
    }
}

pub(crate) fn walk(file: &mut File, file_len: u64, aiff: bool) -> Result<IffLayout, RawError> {
    if file_len < 12 {
        return Err(RawError::new("the file is too short to be WAV or AIFF"));
    }
    let header = read_range(file, 0..12)?;
    let (magic, big_endian): (&[u8], bool) = if aiff {
        (b"FORM", true)
    } else {
        (b"RIFF", false)
    };
    if &header[..4] != magic {
        return Err(RawError::new(format!(
            "expected a {} header",
            String::from_utf8_lossy(magic)
        )));
    }
    let size = read_u32(&header[4..8], big_endian);
    let form_type = [header[8], header[9], header[10], header[11]];
    let valid_form = if aiff {
        &form_type == b"AIFF" || &form_type == b"AIFC"
    } else {
        &form_type == b"WAVE"
    };
    if !valid_form {
        return Err(RawError::new("unexpected form type"));
    }
    let end = 8 + u64::from(size);
    if end > file_len {
        return Err(RawError::new(
            "the RIFF/FORM size is larger than the file (truncated file)",
        ));
    }
    let mut chunks = Vec::new();
    let mut pos = 12u64;
    let mut id3_count = 0;
    while pos + 8 <= end {
        let head = read_range(file, pos..pos + 8)?;
        let id = [head[0], head[1], head[2], head[3]];
        if !id.iter().all(|b| (0x20..0x7F).contains(b)) {
            return Err(RawError::new(format!(
                "invalid chunk ID at byte {pos} (chunk sizes don't line up)"
            )));
        }
        let len = u64::from(read_u32(&head[4..8], big_endian));
        let data = pos + 8..pos + 8 + len;
        if data.end > end {
            return Err(RawError::new(format!(
                "the {} chunk runs past the end of the RIFF/FORM chunk",
                String::from_utf8_lossy(&id)
            )));
        }
        let chunk = IffChunk { id, data };
        if chunk.is_id3() {
            id3_count += 1;
        }
        // A final odd-sized chunk sometimes omits its pad byte.
        let padded = chunk.whole().end;
        pos = if padded > end { chunk.data.end } else { padded };
        chunks.push(chunk);
    }
    if pos != end {
        return Err(RawError::new(
            "the chunks don't add up to the RIFF/FORM size",
        ));
    }
    if id3_count > 1 {
        return Err(RawError::new("the file has more than one ID3 chunk"));
    }
    Ok(IffLayout {
        big_endian,
        form_type,
        chunks,
        end,
    })
}

fn read_u32(bytes: &[u8], big_endian: bool) -> u32 {
    let array = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if big_endian {
        u32::from_be_bytes(array)
    } else {
        u32::from_le_bytes(array)
    }
}
