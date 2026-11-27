//! Audio-only content hashing for duplicate detection.
//!
//! Tag writes change file bytes (see `engineering/decisions/vibe-tags.md`),
//! so a whole-file hash would make every retagged track look new. This
//! module finds the byte ranges that hold the encoded audio stream in each
//! container and hashes only those, so editing, resizing, or adding tags
//! never changes the result. Changing the audio itself always does.
//!
//! No crate exposes these byte ranges (`lofty` and `symphonia` parse around
//! them), so each container has a small finder in a submodule. Format comes
//! from magic bytes, never the extension. Files the finders don't recognize
//! or can't make sense of return an error rather than a whole-file hash,
//! which would silently break the "tags never matter" promise.
//!
//! See `engineering/decisions/content-hashing.md` for the scheme, the
//! per-format rules, and the benchmark behind the I/O strategy.

mod aiff;
mod flac;
mod mp4;
mod mpeg;
mod riff;
mod source;
mod tag_blocks;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{Read, Seek};
use std::ops::Range;
use std::path::Path;

use overtone_model::AudioFormat;

use crate::AudioError;
use source::{PayloadError, Source};

/// Identifies how [`audio_hash`] computes its digest. Stored as the prefix of
/// every hash so a future scheme can coexist with old values instead of
/// silently comparing unlike digests.
pub const HASH_SCHEME: &str = "blake3-payload-v1";

/// Read size for streaming payload bytes into the hasher. The benchmark in
/// `examples/hash_bench.rs` found 64 KiB as fast as 1 MiB with a warm cache
/// and faster single-threaded from disk; see the decision record.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Where a file's encoded audio lives, as found from its container structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadLayout {
    /// Codec family implied by the container, when the container says.
    /// `None` for MP4 files whose sample description isn't AAC or ALAC.
    pub format: Option<AudioFormat>,
    /// Byte ranges of the audio payload, in file order. Never empty, and no
    /// range is empty.
    pub ranges: Vec<Range<u64>>,
}

impl PayloadLayout {
    /// A layout whose payload is one contiguous run of bytes.
    fn contiguous(format: Option<AudioFormat>, range: Range<u64>) -> Self {
        Self {
            format,
            ranges: std::iter::once(range).collect(),
        }
    }

    /// Total payload bytes across all ranges.
    pub fn payload_len(&self) -> u64 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }
}

/// An audio hash together with what was learned while computing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadHash {
    /// `"blake3-payload-v1:<64 lowercase hex>"`.
    pub hash: String,
    pub format: Option<AudioFormat>,
    pub payload_len: u64,
}

/// Hashes the audio payload of the file at `path`, ignoring all tags.
///
/// Returns `"blake3-payload-v1:<64 hex>"`. Opens the file read-only.
pub fn audio_hash(path: &Path) -> Result<String, AudioError> {
    hash_payload(path).map(|payload| payload.hash)
}

/// Like [`audio_hash`], but also returns the container's codec family and
/// the number of bytes hashed.
pub fn hash_payload(path: &Path) -> Result<PayloadHash, AudioError> {
    let file = open_read_only(path)?;
    let mut source = Source::new(file).map_err(|err| io_error(path, err))?;
    let layout = find_layout(&mut source).map_err(|err| err.into_audio_error(path))?;
    let digest = hash_ranges(&mut source, &layout.ranges).map_err(|err| match err {
        PayloadError::Io(source) => io_error(path, source),
        other => other.into_audio_error(path),
    })?;
    Ok(PayloadHash {
        hash: format!("{HASH_SCHEME}:{}", digest.to_hex()),
        format: layout.format,
        payload_len: layout.payload_len(),
    })
}

/// Finds the audio payload byte ranges of the file at `path` without
/// hashing them. Opens the file read-only.
pub fn payload_layout(path: &Path) -> Result<PayloadLayout, AudioError> {
    let file = open_read_only(path)?;
    let mut source = Source::new(file).map_err(|err| io_error(path, err))?;
    find_layout(&mut source).map_err(|err| err.into_audio_error(path))
}

fn open_read_only(path: &Path) -> Result<File, AudioError> {
    // `File::open` opens read-only; the scanner relies on that.
    File::open(path).map_err(|err| io_error(path, err))
}

fn io_error(path: &Path, source: std::io::Error) -> AudioError {
    AudioError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Detects the container from its magic bytes and finds the payload.
fn find_layout<R: Read + Seek>(src: &mut Source<R>) -> Result<PayloadLayout, PayloadError> {
    let head = src.read_up_to(0, 12)?;
    let layout = if head.starts_with(b"fLaC") {
        flac::layout(src, 0)?
    } else if riff::is_riff_wave(&head) {
        riff::layout(src)?
    } else if aiff::is_form_aiff(&head) {
        aiff::layout(src)?
    } else if mp4::is_mp4(&head) {
        mp4::layout(src)?
    } else if tag_blocks::id3v2_len(&head).is_some() {
        // Taggers prepend ID3v2 to MP3 and ADTS streams, and some to FLAC.
        let after = tag_blocks::skip_id3v2_tags(src, 0)?;
        if src.read_up_to(after, 4)? == b"fLaC" {
            flac::layout(src, after)?
        } else {
            mpeg::layout(src, after)?
        }
    } else if mpeg::starts_with_frame(&head) {
        mpeg::layout(src, 0)?
    } else {
        // Possibly an MPEG audio or ADTS stream after leading junk.
        // `mpeg::layout` requires several consecutive valid frames, so
        // unrelated bytes don't pass for MP3.
        mpeg::layout(src, 0).map_err(|err| match err {
            PayloadError::Io(_) => err,
            _ => PayloadError::Unrecognized,
        })?
    };
    debug_assert!(!layout.ranges.is_empty());
    Ok(layout)
}

/// Streams each range through BLAKE3, failing if the file ends early.
fn hash_ranges<R: Read + Seek>(
    src: &mut Source<R>,
    ranges: &[Range<u64>],
) -> Result<blake3::Hash, PayloadError> {
    let mut hasher = blake3::Hasher::new();
    let largest = ranges.iter().map(|r| r.end - r.start).max().unwrap_or(0);
    let capacity = usize::try_from(largest).map_or(READ_BUFFER_BYTES, |n| n.min(READ_BUFFER_BYTES));
    let mut buffer = vec![0_u8; capacity];
    for range in ranges {
        src.stream_range(range.clone(), &mut buffer, |chunk| {
            hasher.update(chunk);
        })?;
    }
    Ok(hasher.finalize())
}
