//! FLAC: the payload is every byte after the last metadata block.
//!
//! Tags (VORBIS_COMMENT), artwork (PICTURE), PADDING, and everything else
//! taggers touch are metadata blocks, so skipping to the block flagged as
//! last leaves only audio frames. STREAMINFO is skipped too: it describes
//! the frames rather than being audio, and taggers leave it alone anyway.

use std::io::{Read, Seek};

use super::PayloadLayout;
use super::source::{PayloadError, Source};
use super::tag_blocks::trailing_tags_start;
use overtone_model::AudioFormat;

const CONTAINER: &str = "FLAC";
const STREAMINFO: u8 = 0;
/// Block type 127 is reserved as invalid by the FLAC format.
const INVALID_BLOCK: u8 = 127;

/// Finds the frames of a FLAC stream whose `fLaC` marker is at `start`
/// (after any ID3v2 tags a tagger prepended).
pub(super) fn layout<R: Read + Seek>(
    src: &mut Source<R>,
    start: u64,
) -> Result<PayloadLayout, PayloadError> {
    let audio_start = end_of_metadata(src, start)?;
    let audio_end = trailing_tags_start(src, audio_start, src.len())?;
    if audio_start >= audio_end {
        return Err(PayloadError::malformed(CONTAINER, "no audio frames"));
    }
    // Every FLAC frame begins with the 14-bit sync code 0b11111111111110
    // followed by a reserved zero bit, so the next byte is 0xF8 or 0xF9.
    let sync: [u8; 2] = src.array_at(CONTAINER, audio_start)?;
    if sync[0] != 0xFF || sync[1] & 0xFE != 0xF8 {
        return Err(PayloadError::malformed(
            CONTAINER,
            format!("no frame sync at byte {audio_start}, after the last metadata block"),
        ));
    }
    Ok(PayloadLayout::contiguous(
        Some(AudioFormat::Flac),
        audio_start..audio_end,
    ))
}

/// Walks metadata block headers (1 byte last-flag and type, 3 bytes
/// big-endian length) and returns the offset after the last one.
fn end_of_metadata<R: Read + Seek>(src: &mut Source<R>, start: u64) -> Result<u64, PayloadError> {
    let mut pos = start + 4;
    let mut first = true;
    loop {
        let header: [u8; 4] = src.array_at(CONTAINER, pos)?;
        let is_last = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7F;
        let len = u64::from(u32::from_be_bytes([0, header[1], header[2], header[3]]));
        if block_type == INVALID_BLOCK {
            return Err(PayloadError::malformed(
                CONTAINER,
                format!("invalid metadata block type at byte {pos}"),
            ));
        }
        if first && block_type != STREAMINFO {
            return Err(PayloadError::malformed(
                CONTAINER,
                "first metadata block is not STREAMINFO",
            ));
        }
        first = false;
        pos += 4 + len;
        if pos > src.len() {
            return Err(PayloadError::malformed(
                CONTAINER,
                "metadata block runs past the end of the file",
            ));
        }
        if is_last {
            return Ok(pos);
        }
    }
}
