//! Independent, byte-level walkers for the tag structures Overtone writes.
//!
//! These deliberately share no code with `lofty`. Before a Comment write
//! replaces a file, [`super::verify`] compares the walker's view of the
//! original and the edited copy, so a bug in the tag library can't silently
//! drop or alter a frame that Serato, Traktor, or Rekordbox owns.
//!
//! The walkers are strict: anything they can't account for byte by byte
//! (unsynchronisation, extended headers, compressed frames, chunk layouts
//! that don't add up) is an error, and Overtone refuses to write the file.

pub(crate) mod flac;
pub(crate) mod id3v2;
mod iff;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use thiserror::Error;

pub use flac::{FlacBlock, FlacLayout, VorbisComment, block_type_name, parse_vorbis_comment};
pub use id3v2::{CommentKey, Id3v2Layout, RawFrame, decode_text_frame};
pub use iff::{IffChunk, IffLayout};

/// How many bytes before an ID3v2 tag the writer tolerates, on read and on write.
pub const MAX_JUNK_BYTES: usize = 1024;

/// A structural problem that makes a file unsafe to write.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct RawError(pub String);

impl RawError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<io::Error> for RawError {
    fn from(err: io::Error) -> Self {
        Self(format!("couldn't read the file: {err}"))
    }
}

/// The container families Overtone can write a Comment into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp3,
    Flac,
    Wav,
    Aiff,
}

impl Container {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Wav => "WAV",
            Self::Aiff => "AIFF",
        }
    }
}

/// Where everything lives in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub container: Container,
    pub file_len: u64,
    /// The ID3v2 tag: at the start of an MP3, or inside the `ID3 ` chunk of
    /// a WAV or AIFF file.
    pub id3v2: Option<Id3v2Layout>,
    pub flac: Option<FlacLayout>,
    pub iff: Option<IffLayout>,
    /// The audio bytes: MPEG frames, the FLAC frames after the metadata
    /// blocks, or the body of the `data` / `SSND` chunk.
    pub payload: Range<u64>,
    /// Bytes after the audio that the writer must not touch: ID3v1 and APE
    /// tags at the end of an MP3, or trailing bytes after a RIFF/FORM chunk.
    pub trailer: Range<u64>,
}

/// Walks `path` as `container` and accounts for every byte.
pub fn inspect(path: &Path, container: Container) -> Result<Layout, RawError> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    match container {
        Container::Mp3 => inspect_mp3(&mut file, file_len),
        Container::Flac => {
            let flac = flac::walk(&mut file, file_len)?;
            Ok(Layout {
                container,
                file_len,
                id3v2: None,
                payload: flac.audio_start..file_len,
                trailer: file_len..file_len,
                flac: Some(flac),
                iff: None,
            })
        }
        Container::Wav | Container::Aiff => {
            let iff = iff::walk(&mut file, file_len, container == Container::Aiff)?;
            let id3v2 = match iff.id3_chunk() {
                Some(chunk) => {
                    let bytes = read_range(&mut file, chunk.data.clone())?;
                    let tag = id3v2::parse(&bytes, chunk.data.start)?;
                    if tag.range.end != chunk.data.end {
                        let extra = chunk.data.end - tag.range.end;
                        if !read_range(&mut file, tag.range.end..chunk.data.end)?
                            .iter()
                            .all(|b| *b == 0)
                        {
                            return Err(RawError::new(format!(
                                "the ID3 chunk has {extra} unexplained bytes after its tag"
                            )));
                        }
                    }
                    Some(tag)
                }
                None => None,
            };
            let payload = iff.audio_chunk()?.data.clone();
            Ok(Layout {
                container,
                file_len,
                id3v2,
                payload,
                trailer: iff.end..file_len,
                flac: None,
                iff: Some(iff),
            })
        }
    }
}

fn inspect_mp3(file: &mut File, file_len: u64) -> Result<Layout, RawError> {
    let head_len = usize::try_from(file_len.min(10)).unwrap_or(10);
    let mut head = vec![0; head_len];
    file.read_exact(&mut head)?;
    let id3v2 = if head.starts_with(b"ID3") && head.len() == 10 {
        let total = 10 + u64::from(id3v2::synchsafe(&head[6..10])?);
        if total > file_len {
            return Err(RawError::new(
                "the ID3v2 tag claims to be longer than the file",
            ));
        }
        let bytes = read_range(file, 0..total)?;
        Some(id3v2::parse(&bytes, 0)?)
    } else {
        // lofty looks for a tag after up to MAX_JUNK_BYTES of leading junk and
        // would splice there; refuse instead of guessing.
        let window = file_len.min(MAX_JUNK_BYTES as u64 + 10);
        let start = read_range(file, 0..window)?;
        if start.windows(3).any(|w| w == b"ID3") {
            return Err(RawError::new(
                "the ID3v2 tag doesn't start at the beginning of the file",
            ));
        }
        None
    };
    let payload_start = id3v2.as_ref().map_or(0, |tag| tag.range.end);
    let trailer_start = mp3_trailer_start(file, payload_start, file_len)?;
    Ok(Layout {
        container: Container::Mp3,
        file_len,
        id3v2,
        flac: None,
        iff: None,
        payload: payload_start..trailer_start,
        trailer: trailer_start..file_len,
    })
}

/// Finds where trailing ID3v1 and APE tags begin.
fn mp3_trailer_start(file: &mut File, floor: u64, file_len: u64) -> Result<u64, RawError> {
    let mut end = file_len;
    if end >= floor + 128 && read_range(file, end - 128..end - 125)? == b"TAG" {
        end -= 128;
    }
    if end >= floor + 32 {
        let footer = read_range(file, end - 32..end)?;
        if footer.starts_with(b"APETAGEX") {
            let size = u64::from(u32::from_le_bytes([
                footer[12], footer[13], footer[14], footer[15],
            ]));
            let flags = u32::from_le_bytes([footer[20], footer[21], footer[22], footer[23]]);
            let has_header = flags & 0x8000_0000 != 0;
            let total = size + if has_header { 32 } else { 0 };
            if total > end - floor {
                return Err(RawError::new(
                    "the APE tag claims to be longer than the file",
                ));
            }
            end -= total;
        }
    }
    Ok(end)
}

pub(crate) fn read_range(file: &mut File, range: Range<u64>) -> Result<Vec<u8>, RawError> {
    let len = usize::try_from(range.end.saturating_sub(range.start))
        .map_err(|_| RawError::new("a tag region is too large to read"))?;
    let mut bytes = vec![0; len];
    file.seek(SeekFrom::Start(range.start))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Whether two byte ranges in two files hold identical bytes. Streams in
/// 1 MiB blocks so large audio payloads never sit in memory.
pub fn ranges_equal(a: &Path, ra: Range<u64>, b: &Path, rb: Range<u64>) -> io::Result<bool> {
    if ra.end - ra.start != rb.end - rb.start {
        return Ok(false);
    }
    let mut fa = File::open(a)?;
    let mut fb = File::open(b)?;
    fa.seek(SeekFrom::Start(ra.start))?;
    fb.seek(SeekFrom::Start(rb.start))?;
    let mut remaining = ra.end - ra.start;
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    while remaining > 0 {
        let n = usize::try_from(remaining.min(1 << 20)).unwrap_or(1 << 20);
        fa.read_exact(&mut ba[..n])?;
        fb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
        remaining -= n as u64;
    }
    Ok(true)
}
