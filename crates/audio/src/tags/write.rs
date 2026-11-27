//! Safe Comment writes: the byte-level splicer ([`super::splice`]) writes
//! a copy with only the Comment changed, independent walkers verify it,
//! and only then does the copy atomically replace the original.

use std::path::Path;

use lofty::file::FileType;
use overtone_fs::{FsError, ProtectedRoots};
use overtone_model::TrackTags;

use super::raw::{self, Container, Id3v2Layout, Layout};
use super::read::{detect, read_as, read_tolerant, read_with, scan_options};
use super::verify::Verifier;
use super::{WriteOutcome, WriteReport, meta, splice};
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
    if let Some(tag) = &layout.id3v2
        && let Some((_, key, _)) = tag.target_comment()
    {
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

/// The Comment the independent walker sees.
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

/// The tags as the scanner reads them (tolerant lofty parse). For MP3 only
/// the ID3v2 tag counts, because that is the tag Overtone writes.
pub(crate) fn scanner_view(path: &Path, layout: &Layout) -> Result<TrackTags, AudioError> {
    if layout.container == Container::Mp3 {
        return super::read::mp3_id3v2_tags(path);
    }
    Ok(read_tolerant(path)?.tags)
}

/// Checks everything `write_comment` checks before writing, and returns
/// the file's layout and the scanner's view of its tags.
pub(crate) fn prepare(path: &Path) -> Result<(Layout, TrackTags), AudioError> {
    let container = writable_container(path)?;
    let layout = preflight(path, container)?;
    let tags = scanner_view(path, &layout).map_err(|err| match err {
        AudioError::TagRead { message, .. } => {
            refused(path, format!("its tags can't be read: {message}"))
        }
        other => other,
    })?;
    let raw = raw_comment(&layout);
    if raw != tags.comment {
        return Err(refused(
            path,
            format!(
                "two tag readers disagree about its Comment ({raw:?} vs {:?})",
                tags.comment
            ),
        ));
    }
    Ok((layout, tags))
}

/// Writes `new_comment` into the file's Comment field (`None` or blank
/// removes it), for MP3, WAV, AIFF (ID3v2) and FLAC (Vorbis `COMMENT`).
///
/// A copy is written next to the original with only the Comment frame or
/// field rebuilt; every other byte is copied as is. Before the copy
/// replaces the original it must pass a frame-by-frame comparison with an
/// independent walker, an ID3 version check, a byte comparison of the audio
/// payload, and a re-read of tags and stream properties. Any failure leaves
/// the original untouched and returns an error naming the failed check.
/// When the Comment already has the requested value, nothing is written.
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
    let (layout, tags) = prepare(path)?;
    if tags.comment.as_deref() == new_comment {
        return Ok(WriteOutcome::Unchanged);
    }
    let properties = read_with(path, scan_options().read_tags(false))?.properties;

    let mut failure: Option<AudioError> = None;
    let mut report = None;
    let result = overtone_fs::edit_via_copy(roots, path, |temp| {
        match edit_and_verify(path, &layout, &tags, &properties, temp, new_comment) {
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
    tags: &TrackTags,
    properties: &overtone_model::AudioProperties,
    temp: &Path,
    new_comment: Option<&str>,
) -> Result<WriteReport, AudioError> {
    splice::write_spliced(original, layout, temp, new_comment)
        .map_err(|err| refused(original, format!("couldn't write the copy: {}", err.0)))?;
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

    let reread = scanner_view(temp, &temp_layout)
        .map_err(|err| failed(original, "re-read", err.to_string()))?;
    let mut expected_tags = tags.clone();
    expected_tags.comment = new_comment.map(str::to_string);
    if reread != expected_tags {
        return Err(failed(
            original,
            "re-read",
            format!("tags changed: expected {expected_tags:?}, read back {reread:?}"),
        ));
    }
    let file_type = detect(original)?;
    let reread_properties = read_as(temp, file_type, scan_options().read_tags(false))
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
