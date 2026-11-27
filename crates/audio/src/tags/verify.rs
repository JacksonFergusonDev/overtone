//! Checks an edited copy against the original before it may replace it.
//!
//! Every check uses the independent walkers in [`super::raw`], never lofty.

use std::collections::BTreeMap;
use std::path::Path;

use super::WriteReport;
use super::raw::{
    CommentKey, Container, FlacLayout, Id3v2Layout, IffChunk, IffLayout, Layout, RawFrame,
    VorbisComment, block_type_name, decode_text_frame, ranges_equal,
};
use crate::AudioError;

pub(crate) struct Verifier<'a> {
    pub original_path: &'a Path,
    pub original: &'a Layout,
    pub temp_path: &'a Path,
    pub temp: &'a Layout,
    /// The Comment the edited copy must hold (`None`: no Comment).
    pub expected: Option<&'a str>,
}

type Check = Result<(), (&'static str, String)>;

impl Verifier<'_> {
    pub(crate) fn run(&self) -> Result<WriteReport, AudioError> {
        let mut report = WriteReport::default();
        self.check_all(&mut report)
            .map_err(|(check, detail)| AudioError::VerificationFailed {
                path: self.original_path.to_path_buf(),
                check,
                detail,
            })?;
        Ok(report)
    }

    fn check_all(&self, report: &mut WriteReport) -> Check {
        if self.original.container != self.temp.container {
            return Err(("container", "the file type changed".into()));
        }
        match self.original.container {
            Container::Mp3 => {
                self.check_id3(report)?;
                self.check_trailer()?;
            }
            Container::Wav | Container::Aiff => {
                self.check_id3(report)?;
                self.check_chunks(report)?;
                self.check_trailer()?;
            }
            Container::Flac => self.check_flac(report)?,
        }
        verify_payload(self.original_path, self.original, self.temp_path, self.temp)
    }

    fn io(&self, err: &std::io::Error) -> (&'static str, String) {
        (
            "read",
            format!("couldn't compare {}: {err}", self.temp_path.display()),
        )
    }

    fn check_trailer(&self) -> Check {
        let same = ranges_equal(
            self.original_path,
            self.original.trailer.clone(),
            self.temp_path,
            self.temp.trailer.clone(),
        )
        .map_err(|e| self.io(&e))?;
        if same {
            Ok(())
        } else {
            Err((
                "trailing tags",
                "the bytes after the audio (ID3v1, APE, or trailing data) changed".into(),
            ))
        }
    }

    fn check_id3(&self, report: &mut WriteReport) -> Check {
        let original = self.original.id3v2.as_ref();
        let temp = self.temp.id3v2.as_ref();
        report.tag_bytes_before = original.map_or(0, |t| t.range.end - t.range.start);
        report.tag_bytes_after = temp.map_or(0, |t| t.range.end - t.range.start);

        let (target_index, key) = match original.and_then(Id3v2Layout::target_comment) {
            Some((index, key, _)) => (Some(index), key),
            None => (None, CommentKey::default_target()),
        };
        let original_rest: Vec<&RawFrame> = original.map_or_default(|tag| {
            tag.frames
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != target_index)
                .map(|(_, f)| f)
                .collect()
        });
        if original_rest
            .iter()
            .any(|f| f.comment().is_some_and(|(k, _)| k == key))
        {
            return Err((
                "comment frame",
                "the original has several Comment frames with the same language and description"
                    .into(),
            ));
        }

        let Some(temp) = temp else {
            if original_rest.is_empty() && self.expected.is_none() {
                return Ok(());
            }
            return Err(("id3 frames", "the edited copy lost its ID3v2 tag".into()));
        };
        match original {
            Some(original) if original.major != temp.major => {
                return Err((
                    "id3 version",
                    format!("ID3v2.{} became ID3v2.{}", original.major, temp.major),
                ));
            }
            _ => {}
        }

        let mut targets = Vec::new();
        let mut temp_rest = Vec::new();
        for frame in &temp.frames {
            match frame.comment() {
                Some((k, text)) if k == key => targets.push(text),
                _ => temp_rest.push(frame),
            }
        }
        let found = match targets.as_slice() {
            [] => None,
            [one] => Some(one.as_str()),
            _ => {
                return Err((
                    "comment frame",
                    "the edited copy has more than one target Comment frame".into(),
                ));
            }
        };
        if found != self.expected {
            return Err((
                "comment value",
                format!("expected {:?}, found {:?}", self.expected, found),
            ));
        }

        compare_frames(&original_rest, &temp_rest, report)
    }

    fn check_chunks(&self, report: &mut WriteReport) -> Check {
        let (Some(original), Some(temp)) = (&self.original.iff, &self.temp.iff) else {
            return Err(("chunks", "missing chunk layout".into()));
        };
        if original.form_type != temp.form_type {
            return Err(("chunks", "the RIFF/FORM type changed".into()));
        }
        let before = untouched_chunks(original);
        let after = untouched_chunks(temp);
        let ids = |chunks: &[&IffChunk]| chunks.iter().map(|c| c.id_str()).collect::<Vec<_>>();
        if ids(&before) != ids(&after) {
            return Err((
                "chunks",
                format!(
                    "chunk list changed from {:?} to {:?}",
                    ids(&before),
                    ids(&after)
                ),
            ));
        }
        for (a, b) in before.iter().zip(&after) {
            let same = ranges_equal(
                self.original_path,
                a.data.clone(),
                self.temp_path,
                b.data.clone(),
            )
            .map_err(|e| self.io(&e))?;
            if !same {
                return Err(("chunks", format!("the {} chunk changed", a.id_str())));
            }
        }
        report.reordered |= original
            .chunks
            .iter()
            .position(IffChunk::is_id3)
            .zip(temp.chunks.iter().position(IffChunk::is_id3))
            .is_some_and(|(a, b)| a != b);
        Ok(())
    }

    fn check_flac(&self, report: &mut WriteReport) -> Check {
        let (Some(original), Some(temp)) = (&self.original.flac, &self.temp.flac) else {
            return Err(("flac blocks", "missing FLAC layout".into()));
        };
        if original.id3_prefix.is_some() || temp.id3_prefix.is_some() {
            return Err(("flac blocks", "ID3v2 in front of FLAC".into()));
        }
        if original.blocks[0] != temp.blocks[0] {
            return Err(("flac blocks", "STREAMINFO changed".into()));
        }
        let before = group_blocks(original);
        let after = group_blocks(temp);
        for kind in before.keys().chain(after.keys()) {
            if before.get(kind) != after.get(kind) {
                return Err((
                    "flac blocks",
                    format!(
                        "{} blocks changed ({} before, {} after)",
                        block_type_name(*kind),
                        before.get(kind).map_or(0, Vec::len),
                        after.get(kind).map_or(0, Vec::len)
                    ),
                ));
            }
        }
        let order = |layout: &FlacLayout| {
            layout
                .blocks
                .iter()
                .map(|b| b.kind)
                .filter(|k| *k != 1)
                .collect::<Vec<_>>()
        };
        report.reordered |= order(original) != order(temp);
        report.tag_bytes_before = original.audio_start;
        report.tag_bytes_after = temp.audio_start;

        let vc = |layout: &FlacLayout| layout.vorbis_comment().map_err(|e| ("vorbis comment", e.0));
        let (before, after) = (vc(original)?, vc(temp)?);
        let empty = VorbisComment {
            vendor: Vec::new(),
            entries: Vec::new(),
        };
        let after = match after {
            Some(after) => after,
            None if before
                .as_ref()
                .is_none_or(|b| b.entries.iter().all(|e| VorbisComment::is_comment_entry(e)))
                && self.expected.is_none() =>
            {
                return Ok(());
            }
            None => return Err(("vorbis comment", "the VORBIS_COMMENT block was lost".into())),
        };
        let before = before.unwrap_or(VorbisComment {
            vendor: after.vendor.clone(),
            ..empty
        });
        if before.vendor != after.vendor {
            return Err(("vorbis comment", "the vendor string changed".into()));
        }
        let others = |v: &VorbisComment| {
            v.entries
                .iter()
                .filter(|e| !VorbisComment::is_comment_entry(e))
                .cloned()
                .collect::<Vec<_>>()
        };
        if others(&before) != others(&after) {
            let keys = |v: &VorbisComment| {
                others(v)
                    .iter()
                    .map(|e| {
                        let end = e.iter().position(|b| *b == b'=').unwrap_or(e.len());
                        String::from_utf8_lossy(&e[..end]).into_owned()
                    })
                    .collect::<Vec<_>>()
            };
            return Err((
                "vorbis comment",
                format!(
                    "fields other than COMMENT changed: {:?} became {:?}",
                    keys(&before),
                    keys(&after)
                ),
            ));
        }
        let comments = |v: &VorbisComment| {
            v.entries
                .iter()
                .filter(|e| VorbisComment::is_comment_entry(e))
                .map(|e| e[8..].to_vec())
                .collect::<Vec<_>>()
        };
        let mut expected = comments(&before);
        match (expected.is_empty(), self.expected) {
            (false, Some(text)) => expected[0] = text.as_bytes().to_vec(),
            (false, None) => {
                expected.remove(0);
            }
            (true, Some(text)) => expected.push(text.as_bytes().to_vec()),
            (true, None) => {}
        }
        if comments(&after) != expected {
            return Err((
                "comment value",
                format!(
                    "expected COMMENT values {:?}, found {:?}",
                    expected
                        .iter()
                        .map(|v| String::from_utf8_lossy(v))
                        .collect::<Vec<_>>(),
                    comments(&after)
                        .iter()
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                        .collect::<Vec<_>>()
                ),
            ));
        }
        Ok(())
    }
}

/// Non-padding, non-comment FLAC blocks by type, bodies sorted.
fn group_blocks(layout: &FlacLayout) -> BTreeMap<u8, Vec<&[u8]>> {
    let mut map: BTreeMap<u8, Vec<&[u8]>> = BTreeMap::new();
    for block in &layout.blocks[1..] {
        if !matches!(block.kind, 1 | 4) {
            map.entry(block.kind).or_default().push(&block.body);
        }
    }
    for bodies in map.values_mut() {
        bodies.sort_unstable();
    }
    map
}

fn group_frames<'f>(frames: &[&'f RawFrame]) -> BTreeMap<String, Vec<&'f RawFrame>> {
    let mut map: BTreeMap<String, Vec<&'f RawFrame>> = BTreeMap::new();
    for frame in frames {
        map.entry(frame.id.clone()).or_default().push(frame);
    }
    map
}

/// Chunks that must survive byte for byte: everything except the ID3 chunk
/// and `JUNK` padding chunks directly after it (where lofty puts the space
/// a shrunken tag no longer needs).
fn untouched_chunks(layout: &IffLayout) -> Vec<&IffChunk> {
    let mut out = Vec::new();
    let mut after_id3 = false;
    for chunk in &layout.chunks {
        if chunk.is_id3() {
            after_id3 = true;
            continue;
        }
        if after_id3 && &chunk.id == b"JUNK" {
            continue;
        }
        after_id3 = false;
        out.push(chunk);
    }
    out
}

/// Every non-target frame must survive with the same ID, flags, and count.
/// Binary frames (GEOB, PRIV, APIC, and anything that isn't text) must be
/// byte-identical; text frames may be re-encoded but must decode to the
/// same strings.
fn compare_frames(before: &[&RawFrame], after: &[&RawFrame], report: &mut WriteReport) -> Check {
    let a = group_frames(before);
    let b = group_frames(after);
    let counts = |m: &BTreeMap<String, Vec<&RawFrame>>| {
        m.iter()
            .map(|(id, frames)| format!("{id}×{}", frames.len()))
            .collect::<Vec<_>>()
    };
    if a.keys().ne(b.keys()) || a.values().map(Vec::len).ne(b.values().map(Vec::len)) {
        return Err((
            "id3 frames",
            format!("frames changed from {:?} to {:?}", counts(&a), counts(&b)),
        ));
    }
    for (id, frames) in &a {
        for (x, y) in frames.iter().zip(&b[id]) {
            if x.flags != y.flags {
                return Err(("id3 frames", format!("the {id} frame's flags changed")));
            }
            if x.body == y.body {
                continue;
            }
            match (decode_text_frame(x), decode_text_frame(y)) {
                (Some(dx), Some(dy)) if dx == dy => {
                    if !report.reencoded_frames.contains(id) {
                        report.reencoded_frames.push(id.clone());
                    }
                }
                (Some(dx), Some(dy)) => {
                    return Err((
                        "id3 frames",
                        format!("the {id} frame's text changed from {dx:?} to {dy:?}"),
                    ));
                }
                _ => {
                    return Err((
                        "id3 frames",
                        format!(
                            "the {id} frame's bytes changed ({})",
                            first_difference(&x.body, &y.body)
                        ),
                    ));
                }
            }
        }
    }
    let order = |frames: &[&RawFrame]| frames.iter().map(|f| f.id.clone()).collect::<Vec<_>>();
    report.reordered |= order(before) != order(after);
    Ok(())
}

/// Describes where two byte strings first differ, for error messages.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let show = |bytes: &[u8]| {
        bytes[at.min(bytes.len())..(at + 12).min(bytes.len())]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "{} bytes became {}; first difference at byte {at}: [{}] vs [{}]",
        a.len(),
        b.len(),
        show(a),
        show(b)
    )
}

/// Checks the audio itself is byte-identical: MPEG frames between the
/// ID3v2 tag and any trailing tags, FLAC frames after the metadata blocks,
/// or the `data` / `SSND` chunk body.
pub(crate) fn verify_payload(
    original_path: &Path,
    original: &Layout,
    temp_path: &Path,
    temp: &Layout,
) -> Check {
    let (a, b) = (original.payload.clone(), temp.payload.clone());
    if a.end - a.start != b.end - b.start {
        return Err((
            "audio payload",
            format!(
                "the audio is {} bytes, was {} bytes",
                b.end - b.start,
                a.end - a.start
            ),
        ));
    }
    match ranges_equal(original_path, a, temp_path, b) {
        Ok(true) => Ok(()),
        Ok(false) => Err(("audio payload", "the audio bytes changed".into())),
        Err(err) => Err(("audio payload", format!("couldn't compare: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::raw::id3v2::tests::{build_tag, comm};
    use crate::tags::raw::{self, Container};
    use tempfile::TempDir;

    fn mp3(dir: &Path, name: &str, tag: &[u8], audio: u8) -> std::path::PathBuf {
        let mut bytes = tag.to_vec();
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        frame[100] = audio;
        for _ in 0..8 {
            bytes.extend_from_slice(&frame);
        }
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    fn geob(byte: u8) -> Vec<u8> {
        let mut body = b"\x00application/octet-stream\0\0Serato Markers2\0".to_vec();
        body.extend_from_slice(&[1, 2, byte]);
        body
    }

    fn verify(
        original: &Path,
        temp: &Path,
        expected: Option<&str>,
    ) -> Result<WriteReport, AudioError> {
        let a = raw::inspect(original, Container::Mp3).expect("walk original");
        let b = raw::inspect(temp, Container::Mp3).expect("walk temp");
        Verifier {
            original_path: original,
            original: &a,
            temp_path: temp,
            temp: &b,
            expected,
        }
        .run()
    }

    fn check_name(result: Result<WriteReport, AudioError>) -> &'static str {
        match result {
            Err(AudioError::VerificationFailed { check, .. }) => check,
            other => panic!("expected a verification failure, got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_comment_only_change_and_reports_reencoded_text() {
        let dir = TempDir::new().expect("tempdir");
        let before = build_tag(
            4,
            &[("TIT2", 0, b"\x00Title".to_vec()), ("GEOB", 0, geob(3))],
            50,
        );
        let mut utf16_title = vec![1, 0xFF, 0xFE];
        for unit in "Title".encode_utf16() {
            utf16_title.extend_from_slice(&unit.to_le_bytes());
        }
        let after = build_tag(
            4,
            &[
                ("TIT2", 0, utf16_title),
                ("GEOB", 0, geob(3)),
                ("COMM", 0, comm(0, *b"eng", "", "#warmup")),
            ],
            10,
        );
        let a = mp3(dir.path(), "a.mp3", &before, 7);
        let b = mp3(dir.path(), "b.mp3", &after, 7);
        let report = verify(&a, &b, Some("#warmup")).expect("passes");
        assert_eq!(report.reencoded_frames, ["TIT2"]);
    }

    #[test]
    fn rejects_changed_geob_payload_version_and_comment() {
        let dir = TempDir::new().expect("tempdir");
        let base = build_tag(
            4,
            &[("GEOB", 0, geob(3)), ("COMM", 0, comm(0, *b"eng", "", "a"))],
            0,
        );
        let original = mp3(dir.path(), "o.mp3", &base, 7);

        let geob_changed = build_tag(
            4,
            &[("GEOB", 0, geob(4)), ("COMM", 0, comm(0, *b"eng", "", "b"))],
            0,
        );
        let temp = mp3(dir.path(), "g.mp3", &geob_changed, 7);
        assert_eq!(
            check_name(verify(&original, &temp, Some("b"))),
            "id3 frames"
        );

        let geob_dropped = build_tag(4, &[("COMM", 0, comm(0, *b"eng", "", "b"))], 0);
        let temp = mp3(dir.path(), "d.mp3", &geob_dropped, 7);
        assert_eq!(
            check_name(verify(&original, &temp, Some("b"))),
            "id3 frames"
        );

        let good = build_tag(
            4,
            &[("GEOB", 0, geob(3)), ("COMM", 0, comm(0, *b"eng", "", "b"))],
            0,
        );
        let temp = mp3(dir.path(), "p.mp3", &good, 8);
        assert_eq!(
            check_name(verify(&original, &temp, Some("b"))),
            "audio payload"
        );

        let v23 = build_tag(
            3,
            &[("GEOB", 0, geob(3)), ("COMM", 0, comm(0, *b"eng", "", "b"))],
            0,
        );
        let temp = mp3(dir.path(), "v.mp3", &v23, 7);
        assert_eq!(
            check_name(verify(&original, &temp, Some("b"))),
            "id3 version"
        );

        let temp = mp3(dir.path(), "c.mp3", &good, 7);
        assert_eq!(
            check_name(verify(&original, &temp, Some("c"))),
            "comment value"
        );
        assert!(verify(&original, &temp, Some("b")).is_ok());
    }
}
