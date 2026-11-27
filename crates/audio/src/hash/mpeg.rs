//! MP3 (MPEG-1/2/2.5 audio) and ADTS (raw AAC) elementary streams.
//!
//! These have no container: the file is a run of self-synchronizing frames
//! with tag blocks bolted on at either end. The payload runs from the first
//! real frame to the start of the trailing tags.
//!
//! The first frame is found by scanning for a frame header that is followed
//! by further consistent headers, rather than trusting the ID3v2 size alone,
//! because taggers often leave zero padding or stale bytes after the tag.
//! Scanning makes the start point a property of the audio, not the tag.
//!
//! A Xing, Info, or LAME header frame counts as payload. It is a real MPEG
//! frame (silent, carrying encoder data) that tag editors never touch, and
//! recognizing it would mean decoding side information for no gain in
//! stability. Tools that rewrite it (VBR header fixers, MP3Gain) change the
//! stream, so a different hash is the honest result.

use std::io::{Read, Seek};

use overtone_model::AudioFormat;

use super::PayloadLayout;
use super::source::{PayloadError, Source};
use super::tag_blocks::{id3v2_len, trailing_tags_start};

const CONTAINER: &str = "MPEG audio";

/// How far past the leading tags to search for the first frame. Covers
/// generous tag padding and junk without reading whole non-MP3 files.
const SYNC_SEARCH_BYTES: usize = 256 * 1024;

/// Frames that must chain together (each header found exactly where the
/// previous frame ends) before a sync candidate is accepted. Three makes a
/// false match in random bytes vanishingly unlikely.
const FRAMES_TO_CONFIRM: usize = 3;

/// Bytes needed to parse either header kind (ADTS needs six).
const HEADER_BYTES: usize = 6;

/// Bitrates in kbit/s, indexed by `[table][bitrate_index]`.
const BITRATES: [[u32; 15]; 5] = [
    // MPEG-1 Layer I
    [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
    ],
    // MPEG-1 Layer II
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ],
    // MPEG-1 Layer III
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ],
    // MPEG-2/2.5 Layer I
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
    ],
    // MPEG-2/2.5 Layer II and III
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

/// Sample rates in Hz for MPEG-1, MPEG-2, and MPEG-2.5, indexed by the
/// header's sample rate index.
const SAMPLE_RATES: [[u32; 3]; 3] = [
    [44_100, 48_000, 32_000],
    [22_050, 24_000, 16_000],
    [11_025, 12_000, 8_000],
];

/// The header fields that stay fixed for a whole stream. Consecutive frames
/// must agree on these, which is how false syncs are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    /// MPEG audio. `version` is 0, 1, 2 for MPEG-1, 2, 2.5; `layer` is 1 to 3.
    Mpeg { version: u8, layer: u8, rate: u8 },
    /// ADTS AAC: MPEG ID, profile, sample rate index, and channel layout.
    Adts { fixed: [u8; 3] },
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    kind: StreamKind,
    len: u64,
}

/// Whether `head` begins with a plausible MPEG audio or ADTS frame header.
pub(super) fn starts_with_frame(head: &[u8]) -> bool {
    head.len() >= HEADER_BYTES && parse_header(&head[..HEADER_BYTES]).is_some()
}

/// Finds the payload of an MPEG audio or ADTS stream. `start` is just past
/// any leading ID3v2 tags.
pub(super) fn layout<R: Read + Seek>(
    src: &mut Source<R>,
    start: u64,
) -> Result<PayloadLayout, PayloadError> {
    let end = trailing_tags_start(src, start, src.len())?;
    let (first, kind) = find_first_frame(src, start, end)?;
    let format = match kind {
        StreamKind::Mpeg { layer: 3, .. } => AudioFormat::Mp3,
        // MPEG Layer I or II (often MP2 renamed to .mp3).
        StreamKind::Mpeg { .. } => AudioFormat::Other,
        StreamKind::Adts { .. } => AudioFormat::Aac,
    };
    Ok(PayloadLayout::contiguous(Some(format), first..end))
}

fn find_first_frame<R: Read + Seek>(
    src: &mut Source<R>,
    start: u64,
    end: u64,
) -> Result<(u64, StreamKind), PayloadError> {
    let window_len =
        usize::try_from(end - start).map_or(SYNC_SEARCH_BYTES, |n| n.min(SYNC_SEARCH_BYTES));
    let window = src.read_up_to(start, window_len)?;
    let mut i = 0;
    while i + HEADER_BYTES <= window.len() {
        // Tools sometimes leave a second ID3v2 tag after padding.
        if let Some(tag_len) = id3v2_len(&window[i..]) {
            i = i.saturating_add(usize::try_from(tag_len).unwrap_or(usize::MAX));
            continue;
        }
        if let Some(frame) = parse_header(&window[i..i + HEADER_BYTES]) {
            let pos = start + i as u64;
            if confirm_chain(src, pos, frame, end)? {
                return Ok((pos, frame.kind));
            }
        }
        i += 1;
    }
    Err(PayloadError::malformed(
        CONTAINER,
        format!(
            "no run of {FRAMES_TO_CONFIRM} frames within {} KiB of byte {start}",
            SYNC_SEARCH_BYTES / 1024
        ),
    ))
}

/// Whether frames follow each other from `pos`, each header exactly where
/// the previous frame ends and of the same kind. Reaching the end of the
/// stream counts as success, so very short files still hash.
fn confirm_chain<R: Read + Seek>(
    src: &mut Source<R>,
    pos: u64,
    first: Frame,
    end: u64,
) -> Result<bool, PayloadError> {
    let mut next = pos + first.len;
    for _ in 1..FRAMES_TO_CONFIRM {
        if end.saturating_sub(next) < HEADER_BYTES as u64 {
            // The stream ends here, possibly mid-frame (a truncated last
            // frame is common and still part of the stream).
            return Ok(next <= end || next - end < first.len);
        }
        let header: [u8; HEADER_BYTES] = src.array_at(CONTAINER, next)?;
        match parse_header(&header) {
            Some(frame) if frame.kind == first.kind => next += frame.len,
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Parses an MPEG audio or ADTS frame header from `HEADER_BYTES` bytes,
/// rejecting reserved and free-format values (free-format frames have no
/// computable length).
fn parse_header(bytes: &[u8]) -> Option<Frame> {
    if bytes[0] != 0xFF || bytes[1] & 0xE0 != 0xE0 {
        return None;
    }
    if bytes[1] & 0b0110 == 0 {
        // Layer 00 is reserved in MPEG audio; ADTS uses it with a 12-bit sync.
        return parse_adts(bytes);
    }
    parse_mpeg(bytes)
}

fn parse_mpeg(bytes: &[u8]) -> Option<Frame> {
    // Version bits: 00 = MPEG-2.5, 01 = reserved, 10 = MPEG-2, 11 = MPEG-1.
    let version: u8 = match (bytes[1] >> 3) & 0b11 {
        0b11 => 0,
        0b10 => 1,
        0b00 => 2,
        _ => return None,
    };
    // Layer bits: 01 = Layer III, 10 = Layer II, 11 = Layer I.
    let layer = 4 - ((bytes[1] >> 1) & 0b11);
    let bitrate_index = usize::from(bytes[2] >> 4);
    let rate = (bytes[2] >> 2) & 0b11;
    if bitrate_index == 0 || bitrate_index == 15 || rate == 3 {
        return None;
    }
    let padding = u64::from((bytes[2] >> 1) & 1);
    let table = match (version, layer) {
        (0, layer) => usize::from(layer - 1),
        (_, 1) => 3,
        _ => 4,
    };
    let bitrate = u64::from(BITRATES[table][bitrate_index]) * 1000;
    let sample_rate = u64::from(SAMPLE_RATES[usize::from(version)][usize::from(rate)]);
    let len = match (version, layer) {
        (_, 1) => (12 * bitrate / sample_rate + padding) * 4,
        (0, _) | (_, 2) => 144 * bitrate / sample_rate + padding,
        _ => 72 * bitrate / sample_rate + padding,
    };
    Some(Frame {
        kind: StreamKind::Mpeg {
            version,
            layer,
            rate,
        },
        len,
    })
}

/// ADTS: 12-bit sync, then a 13-bit frame length (header included) split
/// across bytes 3 to 5.
fn parse_adts(bytes: &[u8]) -> Option<Frame> {
    if bytes[1] & 0xF0 != 0xF0 {
        return None;
    }
    let rate_index = (bytes[2] >> 2) & 0x0F;
    let len = (u64::from(bytes[3] & 0b11) << 11)
        | (u64::from(bytes[4]) << 3)
        | (u64::from(bytes[5]) >> 5);
    if rate_index > 12 || len < 7 {
        return None;
    }
    Some(Frame {
        kind: StreamKind::Adts {
            // Keep ID, layer, protection, profile, rate, and channels;
            // drop the private and copyright bits, which may vary.
            fixed: [bytes[1], bytes[2] & 0xFD, bytes[3] & 0xC0],
        },
        len,
    })
}
