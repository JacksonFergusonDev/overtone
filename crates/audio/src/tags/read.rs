//! Reading stream properties and tags with lofty's concrete file types.

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use lofty::aac::AacFile;
use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag};
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mp4::{AtomData, AtomIdent, Ilst, Mp4Codec, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use overtone_model::{AudioFormat, AudioProperties, TrackTags};

use super::FileMetadata;
use super::raw::MAX_JUNK_BYTES;
use crate::AudioError;

/// Options for scanning: fast, tolerant, and without artwork.
pub(crate) fn scan_options() -> ParseOptions {
    ParseOptions::new()
        .read_properties(true)
        .read_tags(true)
        .read_cover_art(false)
        .parsing_mode(ParsingMode::BestAttempt)
        .max_junk_bytes(MAX_JUNK_BYTES)
}

/// Wraps a lofty error, keeping its whole cause chain in the message.
pub(crate) fn tag_error(path: &Path, err: impl std::error::Error) -> AudioError {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    AudioError::TagRead {
        path: path.to_path_buf(),
        message,
    }
}

/// Detects the container from the file's content (falling back to the
/// extension), as lofty sees it.
pub(crate) fn detect(path: &Path) -> Result<FileType, AudioError> {
    let probe = Probe::open(path)
        .map_err(|err| tag_error(path, err))?
        .guess_file_type()
        .map_err(|err| AudioError::io(path, err))?;
    probe.file_type().ok_or_else(|| {
        AudioError::UnsupportedFormat(format!("{} isn't a recognised audio file", path.display()))
    })
}

/// The scanner's read, with fallbacks for damaged tags.
pub(crate) fn read_tolerant(path: &Path) -> Result<FileMetadata, AudioError> {
    let first = match read_with(path, scan_options()) {
        Ok(metadata) => return Ok(metadata),
        Err(err @ AudioError::TagRead { .. }) => err,
        Err(other) => return Err(other),
    };
    for options in [
        scan_options().parsing_mode(ParsingMode::Relaxed),
        scan_options()
            .parsing_mode(ParsingMode::Relaxed)
            .implicit_conversions(false),
    ] {
        if let Ok(metadata) = read_with(path, options) {
            return Ok(metadata);
        }
    }
    let Ok(mut metadata) = read_with(
        path,
        scan_options()
            .parsing_mode(ParsingMode::Relaxed)
            .read_tags(false),
    ) else {
        return Err(first);
    };
    if metadata.properties.format == Some(AudioFormat::Mp3)
        && let Ok(tag) = mp3_id3v2_only(path)
    {
        metadata.tags = id3_tags(tag.as_ref(), None);
    }
    Ok(metadata)
}

/// The tags in an MP3's leading ID3v2 tag alone, parsed in relaxed mode.
pub(crate) fn mp3_id3v2_tags(path: &Path) -> Result<TrackTags, AudioError> {
    Ok(id3_tags(mp3_id3v2_only(path)?.as_ref(), None))
}

/// Parses only an MP3's leading ID3v2 tag (and a little audio, so lofty
/// recognises the stream), in relaxed mode.
fn mp3_id3v2_only(path: &Path) -> Result<Option<Id3v2Tag>, AudioError> {
    use std::io::Read;
    let mut file = File::open(path).map_err(|err| AudioError::io(path, err))?;
    let mut head = [0u8; 10];
    file.read_exact(&mut head)
        .map_err(|err| AudioError::io(path, err))?;
    if &head[..3] != b"ID3" {
        return Ok(None);
    }
    let size = head[6..10]
        .iter()
        .fold(0u64, |acc, b| (acc << 7) | u64::from(b & 0x7F));
    let mut bytes = head.to_vec();
    file.take(size + 64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|err| AudioError::io(path, err))?;
    let options = scan_options()
        .parsing_mode(ParsingMode::Relaxed)
        .read_properties(false);
    let parsed = MpegFile::read_from(&mut std::io::Cursor::new(bytes), options)
        .map_err(|err| tag_error(path, err))?;
    Ok(parsed.id3v2().cloned())
}

pub(crate) fn read_with(path: &Path, options: ParseOptions) -> Result<FileMetadata, AudioError> {
    read_as(path, detect(path)?, options)
}

/// [`read_with`] for a known file type, for copies whose temporary name
/// has no audio extension to fall back on.
pub(crate) fn read_as(
    path: &Path,
    file_type: FileType,
    options: ParseOptions,
) -> Result<FileMetadata, AudioError> {
    let file = File::open(path).map_err(|err| AudioError::io(path, err))?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|err| AudioError::io(path, err))?;
    let metadata = match file_type {
        FileType::Mpeg => {
            let file = MpegFile::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            FileMetadata {
                properties: AudioProperties {
                    format: Some(AudioFormat::Mp3),
                    sample_rate_hz: nonzero(p.sample_rate()),
                    bit_depth: None,
                    channels: nonzero_u8(p.channels()),
                    bitrate_kbps: nonzero(p.audio_bitrate()),
                    duration_ms: duration_ms(p.duration()),
                },
                tags: id3_tags(file.id3v2(), file.id3v1()),
            }
        }
        FileType::Flac => {
            let file = FlacFile::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            FileMetadata {
                properties: AudioProperties {
                    format: Some(AudioFormat::Flac),
                    sample_rate_hz: nonzero(p.sample_rate()),
                    bit_depth: nonzero_u8(p.bit_depth()),
                    channels: nonzero_u8(p.channels()),
                    bitrate_kbps: nonzero(p.audio_bitrate()),
                    duration_ms: duration_ms(p.duration()),
                },
                tags: file.vorbis_comments().map_or_default(vorbis_tags),
            }
        }
        FileType::Wav => {
            let file = WavFile::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            let mut tags = id3_tags(file.id3v2(), None);
            if let Some(info) = file.riff_info() {
                fill_missing(
                    &mut tags,
                    |key| info.get(key).map(str::to_string),
                    &RIFF_KEYS,
                );
            }
            FileMetadata {
                properties: AudioProperties {
                    format: Some(AudioFormat::Wav),
                    sample_rate_hz: nonzero(p.sample_rate()),
                    bit_depth: nonzero_u8(p.bit_depth()),
                    channels: nonzero_u8(p.channels()),
                    bitrate_kbps: nonzero(p.bitrate()),
                    duration_ms: duration_ms(p.duration()),
                },
                tags,
            }
        }
        FileType::Aiff => {
            let file = AiffFile::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            let mut tags = id3_tags(file.id3v2(), None);
            if let Some(text) = file.text_chunks() {
                tags.title = tags.title.or_else(|| clean(text.title()));
                tags.artist = tags.artist.or_else(|| clean(text.artist()));
            }
            FileMetadata {
                properties: AudioProperties {
                    format: Some(AudioFormat::Aiff),
                    sample_rate_hz: nonzero(p.sample_rate()),
                    bit_depth: u8::try_from(p.sample_size()).ok().filter(|b| *b > 0),
                    channels: u8::try_from(p.channels()).ok().filter(|c| *c > 0),
                    bitrate_kbps: nonzero(p.audio_bitrate()),
                    duration_ms: duration_ms(p.duration()),
                },
                tags,
            }
        }
        FileType::Mp4 => {
            let file = Mp4File::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            let format = match p.codec() {
                Some(Mp4Codec::ALAC) => AudioFormat::Alac,
                Some(Mp4Codec::AAC) => AudioFormat::Aac,
                Some(Mp4Codec::FLAC) => AudioFormat::Flac,
                Some(Mp4Codec::MP3) => AudioFormat::Mp3,
                _ => AudioFormat::Other,
            };
            FileMetadata {
                properties: AudioProperties {
                    format: Some(format),
                    sample_rate_hz: p.sample_rate().and_then(nonzero),
                    bit_depth: p.bit_depth().and_then(nonzero_u8),
                    channels: p.channels().and_then(nonzero_u8),
                    bitrate_kbps: p.audio_bitrate().and_then(nonzero),
                    duration_ms: duration_ms(p.duration()),
                },
                tags: file.ilst().map_or_default(mp4_tags),
            }
        }
        FileType::Aac => {
            let file = AacFile::read_from(&mut reader, options).map_err(|e| tag_error(path, e))?;
            let p = file.properties();
            FileMetadata {
                properties: AudioProperties {
                    format: Some(AudioFormat::Aac),
                    sample_rate_hz: nonzero(p.sample_rate()),
                    bit_depth: None,
                    channels: nonzero_u8(p.channels()),
                    bitrate_kbps: nonzero(p.audio_bitrate()),
                    duration_ms: duration_ms(p.duration()),
                },
                tags: id3_tags(file.id3v2(), file.id3v1()),
            }
        }
        other => {
            return Err(AudioError::UnsupportedFormat(format!(
                "{other:?} files aren't supported ({})",
                path.display()
            )));
        }
    };
    Ok(metadata)
}

fn nonzero(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

fn nonzero_u8(value: u8) -> Option<u8> {
    (value > 0).then_some(value)
}

fn duration_ms(duration: std::time::Duration) -> Option<u64> {
    u64::try_from(duration.as_millis())
        .ok()
        .filter(|ms| *ms > 0)
}

/// Trims, drops empty values, and joins multi-value separators.
fn clean<S: AsRef<str>>(value: Option<S>) -> Option<String> {
    let value = value?;
    let joined = value
        .as_ref()
        .split('\0')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    (!joined.is_empty()).then_some(joined)
}

/// Comments are kept exactly as stored (only NUL padding removed), because
/// the vibe mirroring must not change characters it doesn't own.
fn exact<S: AsRef<str>>(value: Option<S>) -> Option<String> {
    let value = value?;
    let value = value.as_ref().trim_end_matches('\0');
    (!value.trim().is_empty()).then(|| value.to_string())
}

pub fn parse_bpm(text: &str) -> Option<f64> {
    let normalized = text.trim().replace(',', ".");
    let bpm: f64 = normalized.parse().ok()?;
    (bpm.is_finite() && bpm > 0.0 && bpm < 1000.0).then_some(bpm)
}

pub(crate) fn parse_year(text: &str) -> Option<i32> {
    let digits: String = text.trim().chars().take(4).collect();
    if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().ok().filter(|year| *year > 0)
    } else {
        None
    }
}

fn frame_id(id: &'static str) -> FrameId<'static> {
    FrameId::Valid(Cow::Borrowed(id))
}

/// The Comment Overtone reads and writes in an ID3v2 tag: the first `COMM`
/// with language `eng` and an empty description, else the first with an
/// empty description in any language. Must match
/// [`super::raw::Id3v2Layout::target_comment`].
pub(crate) fn id3_comment_index(tag: &Id3v2Tag) -> Option<usize> {
    let is_main = |frame: &Frame<'_>, eng_only: bool| match frame {
        Frame::Comment(comment) => {
            comment.description.is_empty()
                && (!eng_only || comment.language.eq_ignore_ascii_case(b"eng"))
        }
        _ => false,
    };
    tag.iter()
        .position(|frame| is_main(frame, true))
        .or_else(|| tag.iter().position(|frame| is_main(frame, false)))
}

pub(crate) fn id3_comment(tag: &Id3v2Tag) -> Option<String> {
    let index = id3_comment_index(tag)?;
    match &tag[index] {
        Frame::Comment(comment) => exact(Some(comment.content.as_ref())),
        _ => None,
    }
}

pub(crate) fn id3_tags(tag: Option<&Id3v2Tag>, v1: Option<&Id3v1Tag>) -> TrackTags {
    let mut tags = TrackTags::default();
    if let Some(tag) = tag {
        let text = |id: &'static str| clean(tag.get_text(&frame_id(id)));
        tags.title = text("TIT2");
        tags.artist = text("TPE1");
        tags.album = text("TALB");
        tags.album_artist = text("TPE2");
        tags.remixer = text("TPE4");
        tags.label = text("TPUB");
        tags.genre = tag
            .genres()
            .map(|genres| genres.collect::<Vec<_>>().join(", "))
            .filter(|genre| !genre.trim().is_empty());
        tags.year = ["TDRC", "TYER", "TDOR", "TORY"].iter().find_map(|id| {
            match tag.get(&frame_id(id))? {
                Frame::Timestamp(frame) => {
                    Some(i32::from(frame.timestamp.year)).filter(|year| *year > 0)
                }
                Frame::Text(frame) => parse_year(&frame.value),
                _ => None,
            }
        });
        tags.bpm = tag.get_text(&frame_id("TBPM")).and_then(parse_bpm);
        tags.key = text("TKEY");
        tags.comment = id3_comment(tag);
        // iTunes 12.5+ writes Grouping to GRP1 and reuses TIT1 for Work;
        // everything else uses TIT1. Prefer GRP1 when present.
        tags.grouping = text("GRP1").or_else(|| text("TIT1"));
        tags.catalog_number = id3_user_text(tag, "CATALOGNUMBER");
        tags.isrc = text("TSRC");
        tags.release_date = match tag.get(&frame_id("TDRL")) {
            Some(Frame::Timestamp(frame)) => clean(Some(frame.timestamp.to_string())),
            Some(Frame::Text(frame)) => clean(Some(frame.value.as_ref())),
            _ => None,
        }
        .or_else(|| id3_user_text(tag, "RELEASEDATE"));
        tags.track_number = text("TRCK");
        tags.disc_number = text("TPOS");
    }
    if let Some(v1) = v1 {
        tags.title = tags.title.or_else(|| clean(v1.title()));
        tags.artist = tags.artist.or_else(|| clean(v1.artist()));
        tags.album = tags.album.or_else(|| clean(v1.album()));
        tags.year = tags.year.or_else(|| v1.year.map(i32::from));
    }
    tags
}

/// The first `TXXX` frame whose description matches, ignoring case.
fn id3_user_text(tag: &Id3v2Tag, description: &str) -> Option<String> {
    tag.iter().find_map(|frame| match frame {
        Frame::UserText(f) if f.description.eq_ignore_ascii_case(description) => {
            clean(Some(f.content.as_ref()))
        }
        _ => None,
    })
}

fn vorbis_tags(tag: &VorbisComments) -> TrackTags {
    let first = |keys: &[&str]| keys.iter().find_map(|key| clean(tag.get(key)));
    TrackTags {
        title: first(&["TITLE"]),
        artist: first(&["ARTIST"]),
        album: first(&["ALBUM"]),
        album_artist: first(&["ALBUMARTIST", "ALBUM ARTIST", "ALBUM_ARTIST"]),
        remixer: first(&["REMIXER", "MIXARTIST"]),
        label: first(&["LABEL", "ORGANIZATION", "PUBLISHER"]),
        genre: first(&["GENRE"]),
        year: ["DATE", "YEAR", "ORIGINALDATE"]
            .iter()
            .find_map(|key| tag.get(key).and_then(parse_year)),
        bpm: ["BPM", "TEMPO"]
            .iter()
            .find_map(|key| tag.get(key).and_then(parse_bpm)),
        key: first(&["INITIALKEY", "KEY"]),
        comment: exact(tag.get("COMMENT")),
        grouping: first(&["GROUPING"]),
        catalog_number: first(&["CATALOGNUMBER"]),
        isrc: first(&["ISRC"]),
        release_date: first(&["RELEASEDATE"]),
        track_number: first(&["TRACKNUMBER"]),
        disc_number: first(&["DISCNUMBER"]),
    }
}

const ITUNES: &str = "com.apple.iTunes";

fn mp4_tags(ilst: &Ilst) -> TrackTags {
    let fourcc = |code: &[u8; 4]| atom_text(ilst, &AtomIdent::Fourcc(*code));
    let freeform = |name: &'static str| {
        atom_text(
            ilst,
            &AtomIdent::Freeform {
                mean: Cow::Borrowed(ITUNES),
                name: Cow::Borrowed(name),
            },
        )
    };
    let tmpo = ilst
        .get(&AtomIdent::Fourcc(*b"tmpo"))
        .and_then(|atom| atom.data().next().cloned())
        .and_then(|data| match data {
            AtomData::SignedInteger(n) => Some(f64::from(n)),
            AtomData::UnsignedInteger(n) => Some(f64::from(n)),
            AtomData::Unknown { data, .. } if data.len() == 2 => {
                Some(f64::from(u16::from_be_bytes([data[0], data[1]])))
            }
            _ => None,
        })
        .filter(|bpm| *bpm > 0.0);
    TrackTags {
        title: clean(ilst.title()),
        artist: clean(ilst.artist()),
        album: clean(ilst.album()),
        album_artist: fourcc(b"aART"),
        remixer: freeform("REMIXER"),
        label: freeform("LABEL").or_else(|| freeform("publisher")),
        genre: clean(ilst.genre()),
        year: fourcc(b"\xa9day").as_deref().and_then(parse_year),
        bpm: freeform("BPM").as_deref().and_then(parse_bpm).or(tmpo),
        key: freeform("initialkey"),
        comment: atom_exact(ilst, &AtomIdent::Fourcc(*b"\xa9cmt")),
        grouping: fourcc(b"\xa9grp"),
        catalog_number: freeform("CATALOGNUMBER"),
        isrc: freeform("ISRC"),
        release_date: freeform("RELEASEDATE"),
        track_number: number_pair(ilst.track(), ilst.track_total()),
        disc_number: number_pair(ilst.disk(), ilst.disk_total()),
    }
}

/// `3` or `3/12`, from an MP4 number and total.
fn number_pair(number: Option<u32>, total: Option<u32>) -> Option<String> {
    let number = number.filter(|n| *n > 0)?;
    Some(match total.filter(|t| *t > 0) {
        Some(total) => format!("{number}/{total}"),
        None => number.to_string(),
    })
}

fn atom_raw(ilst: &Ilst, ident: &AtomIdent<'_>) -> Option<String> {
    ilst.get(ident)?.data().find_map(|data| match data {
        AtomData::UTF8(text) | AtomData::UTF16(text) => Some(text.clone()),
        _ => None,
    })
}

fn atom_text(ilst: &Ilst, ident: &AtomIdent<'_>) -> Option<String> {
    clean(atom_raw(ilst, ident))
}

fn atom_exact(ilst: &Ilst, ident: &AtomIdent<'_>) -> Option<String> {
    exact(atom_raw(ilst, ident))
}

const RIFF_KEYS: [(&str, Field); 5] = [
    ("INAM", Field::Title),
    ("IART", Field::Artist),
    ("IPRD", Field::Album),
    ("IGNR", Field::Genre),
    ("ICRD", Field::Year),
];

#[derive(Clone, Copy)]
enum Field {
    Title,
    Artist,
    Album,
    Genre,
    Year,
}

/// Fills fields the ID3 chunk left empty from a fallback store. Comment is
/// never filled this way: Overtone reads and writes Comment in one place.
fn fill_missing(
    tags: &mut TrackTags,
    get: impl Fn(&str) -> Option<String>,
    keys: &[(&str, Field)],
) {
    for (key, field) in keys {
        let value = get(key);
        match field {
            Field::Title => tags.title = tags.title.take().or_else(|| clean(value)),
            Field::Artist => tags.artist = tags.artist.take().or_else(|| clean(value)),
            Field::Album => tags.album = tags.album.take().or_else(|| clean(value)),
            Field::Genre => tags.genre = tags.genre.take().or_else(|| clean(value)),
            Field::Year => {
                tags.year = tags.year.or_else(|| value.as_deref().and_then(parse_year));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_and_comma_bpm() {
        assert_eq!(parse_bpm("174"), Some(174.0));
        assert_eq!(parse_bpm(" 128.50 "), Some(128.5));
        assert_eq!(parse_bpm("127,99"), Some(127.99));
        assert_eq!(parse_bpm("fast"), None);
        assert_eq!(parse_bpm("0"), None);
        assert_eq!(parse_bpm("NaN"), None);
    }

    #[test]
    fn parses_years_from_dates() {
        assert_eq!(parse_year("2019-04-12"), Some(2019));
        assert_eq!(parse_year("1998"), Some(1998));
        assert_eq!(parse_year("98"), None);
        assert_eq!(parse_year(""), None);
    }

    #[test]
    fn clean_joins_multi_values_and_drops_empty() {
        assert_eq!(clean(Some("A\0B")), Some("A; B".to_string()));
        assert_eq!(clean(Some("  ")), None);
        assert_eq!(
            exact(Some(" keep  spacing \0")),
            Some(" keep  spacing ".to_string())
        );
    }
}
