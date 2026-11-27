//! AIFF and AIFF-C: the payload is the sample data in the `SSND` chunk.
//!
//! Tags live in `ID3 `, `NAME`, `AUTH`, `ANNO`, and similar chunks, which
//! taggers resize or add around `SSND` without touching its samples.

use std::io::{Read, Seek};

use overtone_model::AudioFormat;

use super::PayloadLayout;
use super::source::{PayloadError, Source, u32_be};

const CONTAINER: &str = "AIFF";

/// `SSND` starts with a 4-byte offset and a 4-byte block size before the
/// samples.
const SSND_PREFIX: u64 = 8;

pub(super) fn is_form_aiff(head: &[u8]) -> bool {
    head.len() >= 12 && &head[..4] == b"FORM" && matches!(&head[8..12], b"AIFF" | b"AIFC")
}

/// Walks big-endian chunks from byte 12 until `SSND`. As with WAV, the FORM
/// size is ignored in favor of chunk sizes and the real file length.
pub(super) fn layout<R: Read + Seek>(src: &mut Source<R>) -> Result<PayloadLayout, PayloadError> {
    let mut pos = 12;
    while src.len() - pos >= 8 {
        let header: [u8; 8] = src.array_at(CONTAINER, pos)?;
        let id = &header[..4];
        let size = u64::from(u32_be(&header[4..8]));
        let body = pos + 8;
        if id == b"SSND" {
            return ssnd_layout(src, body, size);
        }
        pos = body + size + (size & 1);
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
    Err(PayloadError::malformed(CONTAINER, "no SSND chunk"))
}

fn ssnd_layout<R: Read + Seek>(
    src: &mut Source<R>,
    body: u64,
    size: u64,
) -> Result<PayloadLayout, PayloadError> {
    let end = body + size;
    if size < SSND_PREFIX || end > src.len() {
        return Err(PayloadError::malformed(
            CONTAINER,
            format!("SSND chunk claims {size} bytes, past the end of the file"),
        ));
    }
    // The offset says how many alignment bytes precede the first sample.
    let offset = u64::from(u32_be(&src.array_at::<4>(CONTAINER, body)?));
    let start = body + SSND_PREFIX + offset;
    if start >= end {
        return Err(PayloadError::malformed(
            CONTAINER,
            "SSND chunk has no samples",
        ));
    }
    Ok(PayloadLayout::contiguous(
        Some(AudioFormat::Aiff),
        start..end,
    ))
}
