//! Tag blocks that sit before or after a raw audio stream: ID3v2 at the
//! start, and ID3v1, APEv2, Lyrics3v2, and footer-terminated ID3v2 at the
//! end. MP3 and ADTS streams carry these; some taggers add them to FLAC too.

use std::io::{Read, Seek};

use super::source::{PayloadError, Source, u32_le};

const CONTAINER: &str = "tag";

/// Size of an ID3v2 header or footer.
const ID3V2_HEADER: u64 = 10;
/// ID3v1 is a fixed 128-byte block starting with `TAG`.
const ID3V1_LEN: u64 = 128;
/// APEv2 header and footer are 32 bytes each.
const APE_FOOTER: u64 = 32;
/// `LYRICS200` plus its six-digit size prefix.
const LYRICS3V2_TRAILER: u64 = 15;

/// If `bytes` starts with a valid ID3v2 header, the whole tag's length:
/// header, body, and footer when the footer flag is set.
///
/// The size field is "syncsafe" (7 bits per byte, high bit always clear),
/// which doubles as a validity check against random data.
pub(super) fn id3v2_len(bytes: &[u8]) -> Option<u64> {
    id3v2_len_with_magic(bytes, *b"ID3")
}

fn id3v2_len_with_magic(bytes: &[u8], magic: [u8; 3]) -> Option<u64> {
    let header = bytes.get(..10)?;
    let (major, revision, flags) = (header[3], header[4], header[5]);
    if header[..3] != magic || !(2..=4).contains(&major) || revision == 0xFF {
        return None;
    }
    let size = syncsafe(&header[6..10])?;
    let footer = if major == 4 && flags & 0x10 != 0 {
        ID3V2_HEADER
    } else {
        0
    };
    Some(ID3V2_HEADER + size + footer)
}

/// If `bytes` is an ID3v2.4 footer (`3DI`), the whole tag's length. Only
/// v2.4 defines footers, and the footer must carry the footer flag.
fn id3v2_footer_len(bytes: &[u8]) -> Option<u64> {
    let len = id3v2_len_with_magic(bytes, *b"3DI")?;
    (bytes[3] == 4 && bytes[5] & 0x10 != 0).then_some(len)
}

fn syncsafe(bytes: &[u8]) -> Option<u64> {
    bytes.iter().try_fold(0_u64, |acc, &byte| {
        (byte & 0x80 == 0).then_some((acc << 7) | u64::from(byte))
    })
}

/// Skips consecutive ID3v2 tags starting at `pos` and returns the first
/// offset after them. Several tags in a row happen when tools prepend a new
/// tag instead of replacing the old one.
pub(super) fn skip_id3v2_tags<R: Read + Seek>(
    src: &mut Source<R>,
    mut pos: u64,
) -> Result<u64, PayloadError> {
    while let Some(len) = id3v2_len(&src.read_up_to(pos, 10)?) {
        let next = pos.saturating_add(len);
        if next > src.len() {
            return Err(PayloadError::malformed(
                "ID3v2",
                format!("tag at byte {pos} claims {len} bytes, past the end of the file"),
            ));
        }
        pos = next;
    }
    Ok(pos)
}

/// Returns the offset where trailing tag blocks begin, given a stream that
/// runs from `start` to `end`. Strips any order of ID3v1, APEv2 (with or
/// without header), Lyrics3v2, and ID3v2-with-footer blocks.
///
/// Lyrics3 v1 is not stripped: it has no length field and has been
/// obsolete since 2000.
pub(super) fn trailing_tags_start<R: Read + Seek>(
    src: &mut Source<R>,
    start: u64,
    mut end: u64,
) -> Result<u64, PayloadError> {
    loop {
        let available = end - start;
        let len = usize::try_from(available.min(ID3V1_LEN)).unwrap_or(0);
        let mut tail = vec![0_u8; len];
        src.read_exact_at(CONTAINER, end - len as u64, &mut tail)?;

        let stripped = if len == 128 && tail.starts_with(b"TAG") {
            ID3V1_LEN
        } else if len >= 32 && tail[len - 32..].starts_with(b"APETAGEX") {
            ape_len(&tail[len - 32..], available)?
        } else if len >= 15 && tail.ends_with(b"LYRICS200") {
            lyrics3v2_len(src, &tail[len - 15..], end, available)?
        } else if len >= 10
            && let Some(tag_len) = id3v2_footer_len(&tail[len - 10..])
        {
            if tag_len > available {
                return Err(PayloadError::malformed(
                    "ID3v2",
                    "appended tag is longer than the stream",
                ));
            }
            tag_len
        } else {
            return Ok(end);
        };
        end -= stripped;
    }
}

/// Length of an APEv2 tag given its 32-byte footer: the footer's size field
/// counts items plus footer, and a header adds 32 more when flagged.
fn ape_len(footer: &[u8], available: u64) -> Result<u64, PayloadError> {
    let size = u64::from(u32_le(&footer[12..16]));
    let flags = u32_le(&footer[20..24]);
    let header = if flags & (1 << 31) != 0 {
        APE_FOOTER
    } else {
        0
    };
    let total = size + header;
    if size < APE_FOOTER || total > available {
        return Err(PayloadError::malformed(
            "APEv2",
            format!("tag claims {total} bytes"),
        ));
    }
    Ok(total)
}

/// Length of a Lyrics3v2 block given its 15-byte trailer (six ASCII digits
/// of body size, then `LYRICS200`). The body starts with `LYRICSBEGIN`.
fn lyrics3v2_len<R: Read + Seek>(
    src: &mut Source<R>,
    trailer: &[u8],
    end: u64,
    available: u64,
) -> Result<u64, PayloadError> {
    let digits = &trailer[..6];
    let size = digits
        .iter()
        .try_fold(0_u64, |acc, &d| {
            d.is_ascii_digit().then(|| acc * 10 + u64::from(d - b'0'))
        })
        .ok_or_else(|| PayloadError::malformed("Lyrics3v2", "size is not six digits"))?;
    let total = size + LYRICS3V2_TRAILER;
    if total > available || src.array_at::<11>(CONTAINER, end - total)? != *b"LYRICSBEGIN" {
        return Err(PayloadError::malformed(
            "Lyrics3v2",
            "size doesn't point at LYRICSBEGIN",
        ));
    }
    Ok(total)
}
