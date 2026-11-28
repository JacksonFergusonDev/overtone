//! A minimal, strict ID3v2.3 / ID3v2.4 frame lister.

use std::ops::Range;

use super::RawError;

/// One ID3v2 tag, frame by frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id3v2Layout {
    /// 3 or 4.
    pub major: u8,
    pub revision: u8,
    pub flags: u8,
    /// File offsets of the whole tag, header to end of padding.
    pub range: Range<u64>,
    pub frames: Vec<RawFrame>,
    pub padding: u64,
}

/// One frame exactly as stored: ID, the two flag bytes, and the body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawFrame {
    pub id: String,
    pub flags: u16,
    pub body: Vec<u8>,
}

/// The identity of a `COMM` frame: language and description.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommentKey {
    pub language: [u8; 3],
    pub description: String,
}

impl CommentKey {
    /// The frame Overtone creates when a file has no Comment yet.
    pub fn default_target() -> Self {
        Self {
            language: *b"eng",
            description: String::new(),
        }
    }
}

impl RawFrame {
    /// For a `COMM` frame: its identity and decoded text.
    pub fn comment(&self) -> Option<(CommentKey, String)> {
        if self.id != "COMM" || self.body.len() < 4 {
            return None;
        }
        let encoding = self.body[0];
        let language = [self.body[1], self.body[2], self.body[3]];
        let mut strings = split_strings(encoding, &self.body[4..])?;
        let description = strings.first().cloned().unwrap_or_default();
        let content = if strings.len() > 1 {
            strings.drain(1..).collect::<Vec<_>>().join("\0")
        } else {
            String::new()
        };
        Some((
            CommentKey {
                language,
                description,
            },
            content.trim_end_matches('\0').to_string(),
        ))
    }
}

impl Id3v2Layout {
    /// The `COMM` frame Overtone reads and writes as "Comment": the first
    /// one with language `eng` and an empty description, or failing that
    /// the first with an empty description in any language.
    pub fn target_comment(&self) -> Option<(usize, CommentKey, String)> {
        let comments = self
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| frame.comment().map(|(key, text)| (index, key, text)))
            .filter(|(_, key, _)| key.description.is_empty())
            .collect::<Vec<_>>();
        comments
            .iter()
            .find(|(_, key, _)| key.language.eq_ignore_ascii_case(b"eng"))
            .or_else(|| comments.first())
            .cloned()
    }
}

pub(crate) fn synchsafe(bytes: &[u8]) -> Result<u32, RawError> {
    if bytes.iter().any(|b| b & 0x80 != 0) {
        return Err(RawError::new(
            "an ID3v2 size isn't synchsafe (written by a buggy tagger)",
        ));
    }
    Ok(bytes.iter().fold(0u32, |acc, b| (acc << 7) | u32::from(*b)))
}

/// Parses a complete tag (header included). `file_offset` is where `bytes`
/// starts in the file.
pub(crate) fn parse(bytes: &[u8], file_offset: u64) -> Result<Id3v2Layout, RawError> {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return Err(RawError::new("missing ID3v2 header"));
    }
    let major = bytes[3];
    let revision = bytes[4];
    let flags = bytes[5];
    match major {
        2 => {
            return Err(RawError::new(
                "the file has an ID3v2.2 tag, which can't be rewritten without converting it",
            ));
        }
        3 | 4 => {}
        other => {
            return Err(RawError::new(format!(
                "the file has an unknown ID3v2.{other} tag"
            )));
        }
    }
    if flags & 0x80 != 0 {
        return Err(RawError::new("the ID3v2 tag is unsynchronised"));
    }
    if flags & 0x40 != 0 {
        return Err(RawError::new("the ID3v2 tag has an extended header"));
    }
    if flags & 0x20 != 0 {
        return Err(RawError::new("the ID3v2 tag is marked experimental"));
    }
    if major == 4 && flags & 0x10 != 0 {
        return Err(RawError::new("the ID3v2 tag has a footer"));
    }
    let known = if major == 4 { 0xF0 } else { 0xE0 };
    if flags & !known != 0 {
        return Err(RawError::new(format!(
            "the ID3v2 header has unknown flags {flags:#04x}"
        )));
    }
    let size = synchsafe(&bytes[6..10])? as usize;
    let end = 10 + size;
    if end > bytes.len() {
        return Err(RawError::new("the ID3v2 tag is truncated"));
    }

    let mut frames = Vec::new();
    let mut pos = 10;
    while pos < end {
        if bytes[pos] == 0 {
            if bytes[pos..end].iter().any(|b| *b != 0) {
                return Err(RawError::new(
                    "the ID3v2 padding contains data after a zero byte",
                ));
            }
            break;
        }
        if pos + 10 > end {
            return Err(RawError::new(
                "the ID3v2 tag ends in the middle of a frame header",
            ));
        }
        let id_bytes = &bytes[pos..pos + 4];
        if !id_bytes
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err(RawError::new(format!(
                "the ID3v2 tag has an invalid frame ID {:?} at byte {pos}",
                String::from_utf8_lossy(id_bytes)
            )));
        }
        let id = String::from_utf8_lossy(id_bytes).into_owned();
        let size_bytes = &bytes[pos + 4..pos + 8];
        let frame_size = if major == 4 {
            synchsafe(size_bytes)? as usize
        } else {
            u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]])
                as usize
        };
        let frame_flags = u16::from_be_bytes([bytes[pos + 8], bytes[pos + 9]]);
        check_frame_flags(major, &id, frame_flags)?;
        if frame_size == 0 {
            return Err(RawError::new(format!("the {id} frame is empty")));
        }
        let body_start = pos + 10;
        let body_end = body_start
            .checked_add(frame_size)
            .filter(|e| *e <= end)
            .ok_or_else(|| RawError::new(format!("the {id} frame runs past the end of the tag")))?;
        frames.push(RawFrame {
            id,
            flags: frame_flags,
            body: bytes[body_start..body_end].to_vec(),
        });
        pos = body_end;
    }
    Ok(Id3v2Layout {
        major,
        revision,
        flags,
        range: file_offset..file_offset + end as u64,
        frames,
        padding: (end - pos) as u64,
    })
}

fn check_frame_flags(major: u8, id: &str, flags: u16) -> Result<(), RawError> {
    // Format flags change how the body is stored. lofty decodes them on read
    // but doesn't re-encode them on write, so refuse.
    let (format_mask, names): (u16, &[(u16, &str)]) = if major == 4 {
        (
            0x004F,
            &[
                (0x0040, "grouped"),
                (0x0008, "compressed"),
                (0x0004, "encrypted"),
                (0x0002, "unsynchronised"),
                (0x0001, "length-prefixed"),
            ],
        )
    } else {
        (
            0x00E0,
            &[
                (0x0080, "compressed"),
                (0x0040, "encrypted"),
                (0x0020, "grouped"),
            ],
        )
    };
    if flags & format_mask != 0 {
        let what = names
            .iter()
            .filter(|(bit, _)| flags & bit != 0)
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(RawError::new(format!("the {id} frame is {what}")));
    }
    let status_mask: u16 = if major == 4 { 0x7000 } else { 0xE000 };
    if flags & !(format_mask | status_mask) != 0 {
        return Err(RawError::new(format!(
            "the {id} frame has unknown flags {flags:#06x}"
        )));
    }
    Ok(())
}

/// Decodes a text-bearing frame into its strings, independent of text
/// encoding, so frames a tag library re-encodes can still be compared.
/// Returns `None` for frames that aren't text (`GEOB`, `APIC`, `PRIV`, ...).
pub fn decode_text_frame(frame: &RawFrame) -> Option<Vec<String>> {
    let body = &frame.body;
    let id = frame.id.as_str();
    let mut strings = match id {
        "COMM" | "USLT" => {
            if body.len() < 4 {
                return None;
            }
            let mut out = vec![String::from_utf8_lossy(&body[1..4]).into_owned()];
            out.extend(split_strings(body[0], &body[4..])?);
            out
        }
        "WXXX" => {
            let encoding = *body.first()?;
            let (description, rest) = split_first(encoding, &body[1..])?;
            vec![description, latin1(rest)]
        }
        // iTunes' GRP1, MVNM, and MVIN are text frames outside the T range.
        _ if id.starts_with('T') || matches!(id, "GRP1" | "MVNM" | "MVIN") => {
            split_strings(*body.first()?, &body[1..])?
        }
        _ if id.starts_with('W') => vec![latin1(body)],
        _ => return None,
    };
    while strings.last().is_some_and(String::is_empty) && strings.len() > 1 {
        strings.pop();
    }
    for value in &mut strings {
        *value = value.trim_end_matches('\0').to_string();
    }
    Some(strings)
}

fn latin1(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| char::from(*b))
        .collect()
}

fn is_wide(encoding: u8) -> bool {
    matches!(encoding, 1 | 2)
}

/// Splits on the encoding's terminator and decodes every piece.
fn split_strings(encoding: u8, bytes: &[u8]) -> Option<Vec<String>> {
    if encoding > 3 {
        return None;
    }
    let mut out = Vec::new();
    let mut start = 0;
    if is_wide(encoding) {
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == 0 && bytes[i + 1] == 0 {
                out.push(decode(encoding, &bytes[start..i]));
                start = i + 2;
            }
            i += 2;
        }
    } else {
        for (i, b) in bytes.iter().enumerate() {
            if *b == 0 {
                out.push(decode(encoding, &bytes[start..i]));
                start = i + 1;
            }
        }
    }
    if start < bytes.len() {
        out.push(decode(encoding, &bytes[start..]));
    }
    Some(out)
}

fn split_first(encoding: u8, bytes: &[u8]) -> Option<(String, &[u8])> {
    if encoding > 3 {
        return None;
    }
    if is_wide(encoding) {
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == 0 && bytes[i + 1] == 0 {
                return Some((decode(encoding, &bytes[..i]), &bytes[i + 2..]));
            }
            i += 2;
        }
    } else if let Some(i) = bytes.iter().position(|b| *b == 0) {
        return Some((decode(encoding, &bytes[..i]), &bytes[i + 1..]));
    }
    Some((decode(encoding, bytes), &[]))
}

fn decode(encoding: u8, bytes: &[u8]) -> String {
    match encoding {
        0 => bytes.iter().map(|b| char::from(*b)).collect(),
        3 => String::from_utf8_lossy(bytes).into_owned(),
        _ => {
            let (little_endian, body) = match bytes {
                [0xFF, 0xFE, rest @ ..] => (true, rest),
                // A big-endian BOM, or none: ID3v2 defaults to big-endian.
                [0xFE, 0xFF, rest @ ..] | rest => (false, rest),
            };
            let units = body.as_chunks::<2>().0.iter().map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            });
            char::decode_utf16(units)
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a tag from `(id, flags, body)` frames.
    pub(crate) fn build_tag(major: u8, frames: &[(&str, u16, Vec<u8>)], padding: usize) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, flags, data) in frames {
            body.extend_from_slice(id.as_bytes());
            let len = u32::try_from(data.len()).expect("small frame");
            if major == 4 {
                body.extend_from_slice(&to_synchsafe(len));
            } else {
                body.extend_from_slice(&len.to_be_bytes());
            }
            body.extend_from_slice(&flags.to_be_bytes());
            body.extend_from_slice(data);
        }
        body.resize(body.len() + padding, 0);
        let mut tag = b"ID3".to_vec();
        tag.extend_from_slice(&[major, 0, 0]);
        tag.extend_from_slice(&to_synchsafe(u32::try_from(body.len()).expect("small tag")));
        tag.extend_from_slice(&body);
        tag
    }

    pub(crate) fn to_synchsafe(value: u32) -> [u8; 4] {
        [
            u8::try_from((value >> 21) & 0x7F).expect("7 bits"),
            u8::try_from((value >> 14) & 0x7F).expect("7 bits"),
            u8::try_from((value >> 7) & 0x7F).expect("7 bits"),
            u8::try_from(value & 0x7F).expect("7 bits"),
        ]
    }

    pub(crate) fn comm(encoding: u8, lang: [u8; 3], desc: &str, text: &str) -> Vec<u8> {
        let mut body = vec![encoding];
        body.extend_from_slice(&lang);
        body.extend_from_slice(desc.as_bytes());
        body.push(0);
        body.extend_from_slice(text.as_bytes());
        body
    }

    #[test]
    fn lists_frames_and_padding() {
        let tag = build_tag(
            4,
            &[
                ("TIT2", 0, b"\x03Title".to_vec()),
                (
                    "GEOB",
                    0,
                    b"\x00application/octet-stream\0\0Serato Markers2\0\x01\x01".to_vec(),
                ),
                ("COMM", 0, comm(0, *b"eng", "", "8A - Energy 6")),
            ],
            100,
        );
        let layout = parse(&tag, 0).expect("parse");
        assert_eq!(layout.major, 4);
        assert_eq!(layout.frames.len(), 3);
        assert_eq!(layout.padding, 100);
        assert_eq!(layout.range, 0..tag.len() as u64);
        let (index, key, text) = layout.target_comment().expect("comment");
        assert_eq!(index, 2);
        assert_eq!(key, CommentKey::default_target());
        assert_eq!(text, "8A - Energy 6");
    }

    #[test]
    fn prefers_eng_comment_then_any_language() {
        let tag = build_tag(
            3,
            &[
                ("COMM", 0, comm(0, *b"XXX", "", "other")),
                ("COMM", 0, comm(0, *b"eng", "iTunNORM", "000")),
                ("COMM", 0, comm(0, *b"eng", "", "main")),
            ],
            0,
        );
        let layout = parse(&tag, 0).expect("parse");
        assert_eq!(layout.target_comment().expect("target").0, 2);

        let tag = build_tag(3, &[("COMM", 0, comm(0, *b"XXX", "", "other"))], 0);
        let layout = parse(&tag, 0).expect("parse");
        let (_, key, text) = layout.target_comment().expect("target");
        assert_eq!((&key.language, text.as_str()), (b"XXX", "other"));
    }

    #[test]
    fn refuses_layouts_it_cannot_account_for() {
        let mut v22 = build_tag(3, &[], 10);
        v22[3] = 2;
        assert!(parse(&v22, 0).unwrap_err().0.contains("ID3v2.2"));

        let mut unsync = build_tag(3, &[("TIT2", 0, b"\x00a".to_vec())], 0);
        unsync[5] = 0x80;
        assert!(parse(&unsync, 0).unwrap_err().0.contains("unsynchronised"));

        let compressed = build_tag(3, &[("TIT2", 0x0080, b"\x00a".to_vec())], 0);
        assert!(parse(&compressed, 0).unwrap_err().0.contains("compressed"));

        let empty = build_tag(4, &[("TIT2", 0, Vec::new())], 0);
        assert!(parse(&empty, 0).unwrap_err().0.contains("empty"));

        let mut junk_padding = build_tag(4, &[("TIT2", 0, b"\x00a".to_vec())], 20);
        let last = junk_padding.len() - 1;
        junk_padding[last] = 7;
        assert!(parse(&junk_padding, 0).unwrap_err().0.contains("padding"));

        let mut bad_id = build_tag(4, &[("TIT2", 0, b"\x00a".to_vec())], 0);
        bad_id[10] = b'x';
        assert!(
            parse(&bad_id, 0)
                .unwrap_err()
                .0
                .contains("invalid frame ID")
        );
    }

    #[test]
    fn decodes_text_in_every_encoding() {
        let latin = RawFrame {
            id: "TIT2".into(),
            flags: 0,
            body: b"\x00Caf\xe9\x00".to_vec(),
        };
        let mut utf16 = vec![1, 0xFF, 0xFE];
        for unit in "Café".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let utf16 = RawFrame {
            id: "TIT2".into(),
            flags: 0,
            body: utf16,
        };
        let utf8 = RawFrame {
            id: "TIT2".into(),
            flags: 0,
            body: b"\x03Caf\xc3\xa9".to_vec(),
        };
        for frame in [latin, utf16, utf8] {
            assert_eq!(decode_text_frame(&frame), Some(vec!["Café".to_string()]));
        }
        let geob = RawFrame {
            id: "GEOB".into(),
            flags: 0,
            body: vec![0, 1, 2],
        };
        assert_eq!(decode_text_frame(&geob), None);
    }
}
