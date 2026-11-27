//! Bounds-checked positional reads shared by the container finders.

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use crate::AudioError;

/// Why a payload couldn't be located or read.
#[derive(Debug)]
pub(crate) enum PayloadError {
    Io(io::Error),
    /// The container was recognized but its structure doesn't add up.
    Malformed {
        container: &'static str,
        reason: String,
    },
    /// No supported container's magic bytes matched.
    Unrecognized,
}

impl PayloadError {
    pub(crate) fn malformed(container: &'static str, reason: impl Into<String>) -> Self {
        Self::Malformed {
            container,
            reason: reason.into(),
        }
    }

    pub(crate) fn into_audio_error(self, path: &Path) -> AudioError {
        match self {
            Self::Io(source) => AudioError::Io {
                path: path.to_path_buf(),
                source,
            },
            Self::Malformed { container, reason } => AudioError::Malformed {
                path: path.to_path_buf(),
                container,
                reason,
            },
            Self::Unrecognized => AudioError::UnsupportedFormat(format!(
                "{}: no FLAC, MP3, ADTS, MP4, WAV, or AIFF structure found",
                path.display()
            )),
        }
    }
}

impl From<io::Error> for PayloadError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// A seekable reader with a known length. Every read is checked against
/// that length first, so a truncated or garbled file yields a
/// `Malformed` error instead of a short read or a huge allocation.
pub(crate) struct Source<R> {
    inner: R,
    len: u64,
}

impl<R: Read + Seek> Source<R> {
    pub(crate) fn new(mut inner: R) -> io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self { inner, len })
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// Fills `buf` from `pos`, or fails with `Malformed` if the file is too short.
    pub(crate) fn read_exact_at(
        &mut self,
        container: &'static str,
        pos: u64,
        buf: &mut [u8],
    ) -> Result<(), PayloadError> {
        let fits = pos
            .checked_add(buf.len() as u64)
            .is_some_and(|end| end <= self.len);
        if !fits {
            return Err(PayloadError::malformed(
                container,
                format!(
                    "file ends before byte {}",
                    pos.saturating_add(buf.len() as u64)
                ),
            ));
        }
        self.inner.seek(SeekFrom::Start(pos))?;
        self.inner.read_exact(buf)?;
        Ok(())
    }

    pub(crate) fn array_at<const N: usize>(
        &mut self,
        container: &'static str,
        pos: u64,
    ) -> Result<[u8; N], PayloadError> {
        let mut buf = [0_u8; N];
        self.read_exact_at(container, pos, &mut buf)?;
        Ok(buf)
    }

    /// Reads up to `max` bytes from `pos`, fewer if the file ends first.
    pub(crate) fn read_up_to(&mut self, pos: u64, max: usize) -> Result<Vec<u8>, PayloadError> {
        let available = self.len.saturating_sub(pos);
        let len = usize::try_from(available).map_or(max, |n| n.min(max));
        let mut buf = vec![0_u8; len];
        if len > 0 {
            self.inner.seek(SeekFrom::Start(pos))?;
            self.inner.read_exact(&mut buf)?;
        }
        Ok(buf)
    }

    /// Passes every byte of `range` to `sink` in `buffer`-sized chunks.
    /// Fails if the file is shorter than `range`, for example because it
    /// shrank after the layout was found.
    pub(crate) fn stream_range(
        &mut self,
        range: Range<u64>,
        buffer: &mut [u8],
        mut sink: impl FnMut(&[u8]),
    ) -> Result<(), PayloadError> {
        assert!(!buffer.is_empty() || range.is_empty());
        self.inner.seek(SeekFrom::Start(range.start))?;
        let mut remaining = range.end - range.start;
        while remaining > 0 {
            let step = usize::try_from(remaining).map_or(buffer.len(), |n| n.min(buffer.len()));
            let chunk = &mut buffer[..step];
            self.inner.read_exact(chunk)?;
            sink(chunk);
            remaining -= step as u64;
        }
        Ok(())
    }
}

pub(crate) fn u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(crate) fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(crate) fn u64_be(bytes: &[u8]) -> u64 {
    let mut out = [0_u8; 8];
    out.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(out)
}

pub(crate) fn u64_le(bytes: &[u8]) -> u64 {
    let mut out = [0_u8; 8];
    out.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(out)
}
