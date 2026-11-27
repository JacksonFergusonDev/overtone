//! Safe Comment writes: lofty edits a copy, independent walkers verify it,
//! and only then does the copy atomically replace the original.

use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Cursor};
use std::path::Path;

use lofty::TextEncoding;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v2::{CommentFrame, Frame, Id3v2Tag, Id3v2Version};
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::picture::{MimeType, Picture};
use lofty::tag::TagExt;
use overtone_fs::{FsError, ProtectedRoots};
use overtone_model::TrackTags;

use super::raw::{self, Container, Id3v2Layout, Layout};
use super::read::{
    detect, id3_comment_index, id3_tags, read_with, scan_options, tag_error, write_options_parse,
};
use super::verify::Verifier;
use super::{WriteOutcome, WriteReport, meta};
use crate::AudioError;

fn refused(path: &Path, reason: impl Into<String>) -> AudioError {
    AudioError::WriteRefused {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn failed(path: &Path, check: &'static str, detail: impl Into<String>) -> AudioError {
    AudioError::VerificationFailed {
        path: path.to_path_buf(),
        check,
        detail: detail.into(),
    }
}

/// The container Overtone can write for this file, or why it can't.
pub(crate) fn writable_container(path: &Path) -> Result<Container, AudioError> {
    match detect(path)? {
        FileType::Mpeg => Ok(Container::Mp3),
        FileType::Flac => Ok(Container::Flac),
        FileType::Wav => Ok(Container::Wav),
        FileType::Aiff => Ok(Container::Aiff),
        FileType::Mp4 => Err(AudioError::UnsupportedFormat(
            "writing M4A tags is not supported yet".into(),
        )),
        other => Err(AudioError::UnsupportedFormat(format!(
            "writing tags to {other:?} files is not supported"
        ))),
    }
}

/// Inspects the original and refuses structures the writer can't preserve.
pub(crate) fn preflight(path: &Path, container: Container) -> Result<Layout, AudioError> {
    let layout = raw::inspect(path, container).map_err(|err| refused(path, err.0))?;
    if let Some(tag) = &layout.id3v2 {
        check_id3_preservable(path, tag)?;
    }
    if let Some(flac) = &layout.flac {
        if flac.id3_prefix.is_some() {
            return Err(refused(
                path,
                "it has an ID3v2 tag in front of the FLAC stream",
            ));
        }
        flac.vorbis_comment().map_err(|err| refused(path, err.0))?;
    }
    Ok(layout)
}

// lofty drops ID3v2.4-only frames when it writes an ID3v2.3 tag.
const V24_ONLY: &[&str] = &[
    "ASPI", "EQU2", "RVA2", "SEEK", "SIGN", "TDEN", "TDRL", "TDTG", "TMOO", "TPRO", "TSOA", "TSOP",
    "TSOT", "TSST",
];

/// Refuses ID3v2 tags lofty 0.25.5 is known to alter on write, so the
/// DJ gets a clear reason instead of a failed verification.
fn check_id3_preservable(path: &Path, tag: &Id3v2Layout) -> Result<(), AudioError> {
    if let Some((_, key, _)) = tag.target_comment() {
        let same = tag
            .frames
            .iter()
            .filter(|f| f.comment().is_some_and(|(k, _)| k == key))
            .count();
        if same > 1 {
            return Err(refused(
                path,
                "it has several Comment frames with the same language and description",
            ));
        }
    }
    if tag.major == 3 {
        let found: Vec<&str> = tag
            .frames
            .iter()
            .map(|f| f.id.as_str())
            .filter(|id| V24_ONLY.contains(id))
            .collect();
        if !found.is_empty() {
            return Err(refused(
                path,
                format!(
                    "its ID3v2.3 tag contains ID3v2.4-only frames ({}) that would be lost",
                    found.join(", ")
                ),
            ));
        }
    }
    // lofty keeps one frame per identity; exact duplicates would collapse.
    for (i, frame) in tag.frames.iter().enumerate() {
        if tag.frames[..i].contains(frame) {
            return Err(refused(
                path,
                format!("it has duplicate {} frames that would be merged", frame.id),
            ));
        }
    }
    Ok(())
}

/// The Comment the independent walker sees, to cross-check lofty's reading.
fn raw_comment(layout: &Layout) -> Option<String> {
    let text = if let Some(flac) = &layout.flac {
        flac.vorbis_comment()
            .ok()
            .flatten()
            .and_then(|vc| vc.first_comment())
    } else {
        layout
            .id3v2
            .as_ref()
            .and_then(Id3v2Layout::target_comment)
            .map(|(_, _, text)| text)
    };
    text.filter(|t| !t.trim().is_empty())
}

/// lofty's strict view of the tag Overtone writes.
///
/// For MP3 only the ID3v2 tag region (plus a little audio, so lofty
/// recognises the stream) is parsed. Trailing ID3v1 and APE tags are never
/// written and are compared byte for byte instead; parsing them strictly
/// would refuse thousands of real files over an empty ID3v1 year field.
pub(crate) struct StrictView {
    pub tags: TrackTags,
    pub id3: Option<Id3v2Tag>,
}

const MP3_PROBE_BYTES: u64 = 64 * 1024;

pub(crate) fn strict_view(path: &Path, layout: &Layout) -> Result<StrictView, AudioError> {
    let strict = write_options_parse();
    if layout.container == Container::Mp3 {
        let end = layout
            .payload
            .end
            .min(layout.payload.start + MP3_PROBE_BYTES);
        let mut file = File::open(path).map_err(|e| AudioError::io(path, e))?;
        let bytes = raw::read_range(&mut file, 0..end).map_err(|e| refused(path, e.0))?;
        let parsed = MpegFile::read_from(
            &mut Cursor::new(bytes),
            write_options_parse().read_properties(false),
        )
        .map_err(|e| tag_error(path, e))?;
        let id3 = parsed.id3v2().cloned();
        Ok(StrictView {
            tags: id3_tags(id3.as_ref(), None),
            id3,
        })
    } else {
        let metadata = read_with(path, strict)?;
        Ok(StrictView {
            tags: metadata.tags,
            id3: None,
        })
    }
}

/// Checks everything `write_comment` checks before writing, and returns
/// the file's layout and strict view.
pub(crate) fn prepare(path: &Path) -> Result<(Layout, StrictView), AudioError> {
    let container = writable_container(path)?;
    let layout = preflight(path, container)?;
    let view = strict_view(path, &layout).map_err(|err| match err {
        AudioError::TagRead { message, .. } => {
            refused(path, format!("its tags don't parse cleanly: {message}"))
        }
        other => other,
    })?;
    let raw = raw_comment(&layout);
    if raw != view.tags.comment {
        return Err(refused(
            path,
            format!(
                "two tag readers disagree about its Comment ({raw:?} vs {:?})",
                view.tags.comment
            ),
        ));
    }
    Ok((layout, view))
}

/// Writes `new_comment` into the file's Comment field (`None` or blank
/// removes it), for MP3, WAV, AIFF (ID3v2) and FLAC (Vorbis `COMMENT`).
///
/// The edit happens on a copy next to the original. Before the copy
/// replaces the original it must pass a strict re-read, a frame-by-frame
/// comparison with an independent walker, an ID3 version check, and a
/// byte comparison of the audio payload. Any failure leaves the original
/// untouched and returns an error naming the failed check. When the Comment
/// already has the requested value, nothing is written.
pub fn write_comment(
    roots: &ProtectedRoots,
    path: &Path,
    new_comment: Option<&str>,
) -> Result<WriteOutcome, AudioError> {
    let new_comment = new_comment.filter(|text| !text.trim().is_empty());
    if new_comment.is_some_and(|text| text.contains('\0')) {
        return Err(refused(path, "the new Comment contains a NUL character"));
    }
    roots.check(path)?;
    let (layout, view) = prepare(path)?;
    if view.tags.comment.as_deref() == new_comment {
        return Ok(WriteOutcome::Unchanged);
    }
    let properties = read_with(path, scan_options().read_tags(false))?.properties;

    let mut failure: Option<AudioError> = None;
    let mut report = None;
    let result = overtone_fs::edit_via_copy(roots, path, |temp| {
        let outcome = edit_and_verify(path, &layout, &view, &properties, temp, new_comment);
        match outcome {
            Ok(r) => {
                report = Some(r);
                Ok(())
            }
            Err(err) => {
                let message = err.to_string();
                failure = Some(err);
                Err(FsError::io(temp, std::io::Error::other(message)))
            }
        }
    });
    match (result, failure) {
        (Ok(()), _) => Ok(WriteOutcome::Written(report.unwrap_or_default())),
        (Err(_), Some(err)) => Err(err),
        (Err(err), None) => Err(err.into()),
    }
}

fn edit_and_verify(
    original: &Path,
    layout: &Layout,
    view: &StrictView,
    properties: &overtone_model::AudioProperties,
    temp: &Path,
    new_comment: Option<&str>,
) -> Result<WriteReport, AudioError> {
    apply_edit(temp, layout, view, new_comment).map_err(|err| match err {
        AudioError::TagRead { message, .. } => refused(
            original,
            format!("the tag library couldn't write it: {message}"),
        ),
        other => other,
    })?;
    meta::carry_over(original, temp)?;

    let temp_layout = raw::inspect(temp, layout.container).map_err(|err| {
        failed(
            original,
            "structure",
            format!("the edited copy doesn't walk cleanly: {}", err.0),
        )
    })?;
    let report = Verifier {
        original_path: original,
        original: layout,
        temp_path: temp,
        temp: &temp_layout,
        expected: new_comment,
    }
    .run()?;

    let reread = strict_view(temp, &temp_layout)
        .map_err(|err| failed(original, "strict re-read", err.to_string()))?;
    if reread.tags.comment.as_deref() != new_comment {
        return Err(failed(
            original,
            "strict re-read",
            format!(
                "expected Comment {new_comment:?}, read back {:?}",
                reread.tags.comment
            ),
        ));
    }
    let mut expected_tags = view.tags.clone();
    expected_tags.comment = new_comment.map(str::to_string);
    if reread.tags != expected_tags {
        return Err(failed(
            original,
            "strict re-read",
            format!(
                "other tags changed: {:?} became {:?}",
                view.tags, reread.tags
            ),
        ));
    }
    let reread_properties = read_with(temp, scan_options().read_tags(false))
        .map_err(|err| failed(original, "re-read", err.to_string()))?
        .properties;
    if &reread_properties != properties {
        return Err(failed(
            original,
            "re-read",
            format!("stream properties changed: {properties:?} became {reread_properties:?}"),
        ));
    }
    Ok(report)
}

fn write_options(version: Option<Id3v2Version>, uppercase_chunk: bool) -> WriteOptions {
    WriteOptions::new()
        .uppercase_id3v2_chunk(uppercase_chunk)
        .use_id3v23(version == Some(Id3v2Version::V3))
        .lossy_text_encoding(false)
        .parse_options(write_options_parse())
}

fn apply_edit(
    temp: &Path,
    layout: &Layout,
    view: &StrictView,
    new_comment: Option<&str>,
) -> Result<(), AudioError> {
    let parse: ParseOptions = write_options_parse();
    let output = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp)
            .map_err(|e| AudioError::io(temp, e))
    };
    let reader = || -> Result<BufReader<File>, AudioError> {
        Ok(BufReader::new(
            File::open(temp).map_err(|e| AudioError::io(temp, e))?,
        ))
    };
    let original_version = layout.id3v2.as_ref().map(|tag| match tag.major {
        3 => Id3v2Version::V3,
        _ => Id3v2Version::V4,
    });
    let uppercase_chunk = layout
        .iff
        .as_ref()
        .and_then(|iff| iff.id3_chunk())
        .is_none_or(|chunk| &chunk.id == b"ID3 ");
    let options = write_options(original_version, uppercase_chunk);

    let id3 = |tag: Option<&Id3v2Tag>| -> Result<Id3v2Tag, AudioError> {
        let mut tag = tag.cloned().unwrap_or_default();
        if let Some(version) = original_version
            && tag.original_version() != version
        {
            return Err(refused(
                temp,
                "the two tag readers disagree about the ID3 version",
            ));
        }
        set_id3_comment(&mut tag, new_comment);
        if let Some(raw) = &layout.id3v2 {
            keep_frame_details(&mut tag, raw);
        }
        Ok(tag)
    };
    match layout.container {
        Container::Mp3 => {
            let tag = id3(view.id3.as_ref())?;
            tag.save_to(&mut output()?, options)
                .map_err(|e| tag_error(temp, e))?;
        }
        Container::Wav => {
            let file = WavFile::read_from(&mut reader()?, parse).map_err(|e| tag_error(temp, e))?;
            let tag = id3(file.id3v2())?;
            tag.save_to(&mut output()?, options)
                .map_err(|e| tag_error(temp, e))?;
        }
        Container::Aiff => {
            let file =
                AiffFile::read_from(&mut reader()?, parse).map_err(|e| tag_error(temp, e))?;
            let tag = id3(file.id3v2())?;
            tag.save_to(&mut output()?, options)
                .map_err(|e| tag_error(temp, e))?;
        }
        Container::Flac => {
            let mut file =
                FlacFile::read_from(&mut reader()?, parse).map_err(|e| tag_error(temp, e))?;
            if file.vorbis_comments().is_none() {
                file.set_vorbis_comments(VorbisComments::new());
            }
            if let Some(tag) = file.vorbis_comments_mut() {
                set_vorbis_comment(tag, new_comment);
            }
            // FlacFile::save_to (not VorbisComments::save_to) so PICTURE
            // blocks, which live on the file, are written back.
            file.save_to(&mut output()?, options)
                .map_err(|e| tag_error(temp, e))?;
        }
    }
    Ok(())
}

/// Undoes two normalisations lofty 0.25.5 applies to frames it re-encodes,
/// so non-Comment frames keep their original bytes or meaning:
///
/// - `APIC`: lofty rewrites the MIME type `image/jpg` as `image/jpeg`.
///   Restore the original MIME string so the frame stays byte-identical.
/// - `WXXX`: lofty encodes the URL in the description's text encoding,
///   but the spec (and every reader) expects Latin-1. When the description
///   is Latin-1, write the frame as Latin-1 so the URL survives.
fn keep_frame_details(tag: &mut Id3v2Tag, raw: &Id3v2Layout) {
    let mut mimes = raw
        .frames
        .iter()
        .filter(|f| f.id == "APIC")
        .map(|f| {
            f.body
                .get(1..)
                .unwrap_or_default()
                .iter()
                .take_while(|b| **b != 0)
                .map(|b| char::from(*b))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .into_iter();
    tag.retain_mut(|frame| {
        match frame {
            Frame::Picture(apic) => {
                if let Some(mime) = mimes.next()
                    && apic.picture.mime_type().map(MimeType::as_str) != Some(mime.as_str())
                {
                    let old = &apic.picture;
                    let mut builder = Picture::unchecked(old.data().to_vec())
                        .pic_type(old.pic_type())
                        .mime_type(MimeType::Unknown(mime));
                    if let Some(description) = old.description() {
                        builder = builder.description(description.to_string());
                    }
                    apic.picture = Cow::Owned(builder.build());
                }
            }
            Frame::UserUrl(wxxx)
                if wxxx.encoding != TextEncoding::Latin1 && is_latin1(&wxxx.description) =>
            {
                wxxx.encoding = TextEncoding::Latin1;
            }
            _ => {}
        }
        true
    });
}

/// Edits the target Comment frame in place, keeping its language,
/// description, and position. Other `COMM` frames are untouched.
pub(crate) fn set_id3_comment(tag: &mut Id3v2Tag, new_comment: Option<&str>) {
    let target = id3_comment_index(tag);
    match (target, new_comment) {
        (Some(index), Some(text)) => {
            let mut position = 0;
            tag.retain_mut(|frame| {
                if position == index
                    && let Frame::Comment(comment) = frame
                {
                    if comment.encoding == TextEncoding::Latin1 && !is_latin1(text) {
                        comment.encoding = TextEncoding::UTF16;
                    }
                    comment.content = Cow::Owned(text.to_string());
                }
                position += 1;
                true
            });
        }
        (Some(index), None) => {
            let mut position = 0;
            tag.retain(|_| {
                let keep = position != index;
                position += 1;
                keep
            });
        }
        (None, Some(text)) => {
            let encoding = if text.is_ascii() {
                TextEncoding::Latin1
            } else {
                TextEncoding::UTF16
            };
            tag.insert(Frame::Comment(CommentFrame::new(
                encoding,
                *b"eng",
                String::new(),
                text.to_string(),
            )));
        }
        (None, None) => {}
    }
}

fn is_latin1(text: &str) -> bool {
    text.chars().all(|c| u32::from(c) <= 0xFF)
}

/// Replaces the first `COMMENT` value in place (keeping its key's case and
/// position), removes it for `None`, or appends one. Other fields keep
/// their order and spelling.
pub(crate) fn set_vorbis_comment(tag: &mut VorbisComments, new_comment: Option<&str>) {
    let mut items: Vec<(String, String)> = tag.take_items().collect();
    let index = items
        .iter()
        .position(|(key, _)| key.eq_ignore_ascii_case("COMMENT"));
    match (index, new_comment) {
        (Some(i), Some(text)) => items[i].1 = text.to_string(),
        (Some(i), None) => {
            items.remove(i);
        }
        (None, Some(text)) => items.push(("COMMENT".to_string(), text.to_string())),
        (None, None) => {}
    }
    for (key, value) in items {
        tag.push(key, value);
    }
}
