//! WAV (RIFF), and its 64-bit variants RF64 and BW64: the payload is the
//! body of the `data` chunk.
//!
//! Taggers store metadata in `LIST`/`INFO`, `id3 `, `bext`, and similar
//! chunks before or after `data`. Resizing them shifts where `data` sits but
//! not what it holds.

use std::io::{Read, Seek};

use overtone_model::AudioFormat;

use super::PayloadLayout;
use super::source::{PayloadError, Source, u32_le, u64_le};

const CONTAINER: &str = "WAV";

/// The 32-bit size that RF64 and BW64 use to say "see the ds64 chunk".
const SIZE_IN_DS64: u32 = u32::MAX;

pub(super) fn is_riff_wave(head: &[u8]) -> bool {
    head.len() >= 12 && matches!(&head[..4], b"RIFF" | b"RF64" | b"BW64") && &head[8..12] == b"WAVE"
}

/// Walks chunks from byte 12 until `data`. The RIFF size in the file header
/// is ignored, since writers often leave it wrong; chunk sizes and the real
/// file length are what count.
pub(super) fn layout<R: Read + Seek>(src: &mut Source<R>) -> Result<PayloadLayout, PayloadError> {
    let is_64_bit = &src.array_at::<4>(CONTAINER, 0)? != b"RIFF";
    let mut ds64_data_size = None;
    let mut pos = 12;
    while src.len() - pos >= 8 {
        let header: [u8; 8] = src.array_at(CONTAINER, pos)?;
        let id = &header[..4];
        let size32 = u32_le(&header[4..8]);
        let body = pos + 8;
        if is_64_bit && id == b"ds64" {
            // ds64: RIFF size (8), data size (8), sample count (8), table.
            let fields: [u8; 16] = src.array_at(CONTAINER, body)?;
            ds64_data_size = Some(u64_le(&fields[8..16]));
        }
        let size = match ds64_data_size {
            Some(size) if id == b"data" && size32 == SIZE_IN_DS64 => size,
            _ => u64::from(size32),
        };
        if id == b"data" {
            return data_layout(src, body, size);
        }
        // Chunks are padded to an even length.
        pos = body.saturating_add(size).saturating_add(size & 1);
        if pos > src.len() {
            return Err(PayloadError::malformed(
                CONTAINER,
                format!(
                    "{} chunk runs past the end of the file",
                    String::from_utf8_lossy(id)
                ),
            ));
        }
    }
    Err(PayloadError::malformed(CONTAINER, "no data chunk"))
}

fn data_layout<R: Read + Seek>(
    src: &Source<R>,
    body: u64,
    size: u64,
) -> Result<PayloadLayout, PayloadError> {
    let end = body.saturating_add(size);
    if size == 0 {
        return Err(PayloadError::malformed(CONTAINER, "data chunk is empty"));
    }
    if end > src.len() {
        return Err(PayloadError::malformed(
            CONTAINER,
            format!(
                "data chunk claims {size} bytes but the file ends {} bytes in",
                src.len() - body
            ),
        ));
    }
    Ok(PayloadLayout::contiguous(Some(AudioFormat::Wav), body..end))
}
