//! One pass over `collection.nml` that records what the splicer needs:
//! collection keys and byte offsets in the playlist tree.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::key::nfc;
use crate::{DEFAULT_STARTUP_VOLUME, OVERTONE_FOLDER, TraktorError};

const BOM: &[u8] = b"\xEF\xBB\xBF";
const ROOT_NAME: &str = "$ROOT";

/// What [`scan`] found in a `collection.nml`. Offsets index the bytes
/// passed to [`scan`], byte order mark included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionScan {
    /// The root `NML VERSION`: `19` for Traktor Pro 3, `20` for Traktor Pro 4.
    pub version: Option<String>,
    /// Whether the file starts with a UTF-8 byte order mark.
    pub bom: bool,
    /// The file's line ending, `"\n"` or `"\r\n"`.
    pub newline: &'static str,
    /// How many collection entries live on each `LOCATION/@VOLUME`.
    pub volume_counts: BTreeMap<String, usize>,
    /// Volume name of the startup disk: the most common volume among
    /// entries under `/Users/`, else [`DEFAULT_STARTUP_VOLUME`].
    pub startup_volume: String,
    /// Every collection entry's key, NFC-normalized, mapped to the key as the
    /// collection spells it.
    pub keys: HashMap<String, String>,
    /// The digits of the `$ROOT` `SUBNODES COUNT` attribute.
    pub root_subnodes_count_span: Range<usize>,
    /// The `$ROOT` `SUBNODES COUNT` value.
    pub root_subnodes_count: usize,
    /// How many `NODE` children `$ROOT`'s `SUBNODES` actually holds.
    pub root_child_count: usize,
    /// Offset of the `</SUBNODES>` that closes `$ROOT`'s children.
    pub root_subnodes_end: usize,
    /// The first `NODE TYPE="FOLDER" NAME="Overtone"` directly under
    /// `$ROOT`, through the end of its line. Same-named folders deeper in
    /// the tree are your own and aren't included.
    pub overtone_span: Option<Range<usize>>,
    /// Every playlist key inside the Overtone folder.
    pub overtone_keys: Vec<String>,
    /// Count attributes inside the Overtone folder that don't match what
    /// they count.
    pub overtone_problems: Vec<String>,
}

impl CollectionScan {
    /// The collection's own spelling of `key`, ignoring Unicode
    /// normalization, if the collection has that entry.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.keys.get(&nfc(key)).map(String::as_str)
    }

    /// The bytes of the existing Overtone folder, if any.
    pub fn overtone_bytes<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        self.overtone_span.clone().map(|span| &bytes[span])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Other,
    Collection,
    CollectionEntry,
    Playlists,
    RootNode,
    RootSubnodes,
    /// A `NODE` directly under `$ROOT`'s `SUBNODES`.
    TopNode,
    /// The Overtone folder `NODE`.
    Overtone,
}

struct Frame {
    name: Vec<u8>,
    role: Role,
    inside_overtone: bool,
    /// Declared `COUNT` (`SUBNODES`) or `ENTRIES` (`PLAYLIST`).
    declared: Option<usize>,
    children: usize,
}

fn malformed(err: impl std::fmt::Display) -> TraktorError {
    TraktorError::Malformed(err.to_string())
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, TraktorError> {
    for attr in e.attributes() {
        let attr = attr.map_err(malformed)?;
        if attr.key.as_ref().as_bytes() == name {
            let value = attr
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map_err(malformed)?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn count_attr(e: &BytesStart<'_>, name: &[u8]) -> Result<Option<usize>, TraktorError> {
    attr(e, name)?
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|_| malformed(format!("bad {} value {value:?}", show(name))))
        })
        .transpose()
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The byte range of attribute `name`'s value inside a raw tag.
fn attr_value_span(tag: &[u8], name: &[u8]) -> Option<Range<usize>> {
    let mut from = 0;
    while let Some(found) = find(&tag[from..], name) {
        let start = from + found;
        from = start + 1;
        if start == 0 || !tag[start - 1].is_ascii_whitespace() {
            continue;
        }
        let mut i = start + name.len();
        while tag.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if tag.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while tag.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let quote = *tag.get(i)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let value_start = i + 1;
        let len = tag[value_start..].iter().position(|&b| b == quote)?;
        return Some(value_start..value_start + len);
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn newline_after(bytes: &[u8], at: usize) -> usize {
    if bytes[at..].starts_with(b"\r\n") {
        at + 2
    } else if bytes[at..].starts_with(b"\n") {
        at + 1
    } else {
        at
    }
}

struct Scanner<'a> {
    bytes: &'a [u8],
    stack: Vec<Frame>,
    out: CollectionScan,
    users_volumes: BTreeMap<String, usize>,
    found_root: bool,
    found_root_subnodes: bool,
    root_closed: bool,
    overtone_start: Option<usize>,
}

impl Scanner<'_> {
    fn parent_role(&self) -> Option<Role> {
        self.stack.last().map(|f| f.role)
    }

    fn inside_overtone(&self) -> bool {
        self.stack.last().is_some_and(|f| f.inside_overtone)
    }

    fn open(&mut self, e: &BytesStart<'_>, tag: Range<usize>) -> Result<(), TraktorError> {
        let name = e.name().as_ref().as_bytes().to_vec();
        let parent = self.parent_role();
        let inside = self.inside_overtone();
        if let Some(parent_frame) = self.stack.last_mut() {
            let counted = match parent_frame.name.as_slice() {
                b"SUBNODES" => name == b"NODE",
                b"PLAYLIST" => name == b"ENTRY",
                _ => false,
            };
            if counted {
                parent_frame.children += 1;
            }
        }
        let mut declared = None;
        let role = match (parent, name.as_slice()) {
            (None, b"NML") => {
                self.out.version = attr(e, b"VERSION")?;
                Role::Other
            }
            (Some(Role::Other), b"COLLECTION") if self.stack.len() == 1 => Role::Collection,
            (Some(Role::Other), b"PLAYLISTS") if self.stack.len() == 1 => Role::Playlists,
            (Some(Role::Collection), b"ENTRY") => Role::CollectionEntry,
            (Some(Role::CollectionEntry), b"LOCATION") => {
                self.location(e)?;
                Role::Other
            }
            (Some(Role::Playlists), b"NODE")
                if !self.found_root && attr(e, b"NAME")?.as_deref() == Some(ROOT_NAME) =>
            {
                self.found_root = true;
                Role::RootNode
            }
            (Some(Role::RootNode), b"SUBNODES") if !self.found_root_subnodes => {
                self.found_root_subnodes = true;
                let raw = &self.bytes[tag.clone()];
                let span = attr_value_span(raw, b"COUNT")
                    .ok_or_else(|| malformed("$ROOT SUBNODES has no COUNT"))?;
                self.out.root_subnodes_count_span = tag.start + span.start..tag.start + span.end;
                self.out.root_subnodes_count = count_attr(e, b"COUNT")?.unwrap_or(0);
                Role::RootSubnodes
            }
            (Some(Role::RootSubnodes), b"NODE") => {
                self.out.root_child_count += 1;
                let is_overtone = self.overtone_start.is_none()
                    && self.out.overtone_span.is_none()
                    && attr(e, b"TYPE")?.as_deref() == Some("FOLDER")
                    && attr(e, b"NAME")?.as_deref() == Some(OVERTONE_FOLDER);
                if is_overtone {
                    self.overtone_start = Some(tag.start);
                    Role::Overtone
                } else {
                    Role::TopNode
                }
            }
            _ => Role::Other,
        };
        let inside_overtone = inside || role == Role::Overtone;
        if inside_overtone {
            match name.as_slice() {
                b"SUBNODES" => declared = Some(count_attr(e, b"COUNT")?.unwrap_or(0)),
                b"PLAYLIST" => declared = Some(count_attr(e, b"ENTRIES")?.unwrap_or(0)),
                b"PRIMARYKEY" => {
                    let key = attr(e, b"KEY")?.unwrap_or_default();
                    self.out.overtone_keys.push(key);
                }
                _ => {}
            }
        }
        self.stack.push(Frame {
            name,
            role,
            inside_overtone,
            declared,
            children: 0,
        });
        Ok(())
    }

    fn location(&mut self, e: &BytesStart<'_>) -> Result<(), TraktorError> {
        let volume = attr(e, b"VOLUME")?.unwrap_or_default();
        let dir = attr(e, b"DIR")?.unwrap_or_default();
        let file = attr(e, b"FILE")?.unwrap_or_default();
        *self.out.volume_counts.entry(volume.clone()).or_default() += 1;
        if dir.starts_with("/:Users/:") {
            *self.users_volumes.entry(volume.clone()).or_default() += 1;
        }
        let key = format!("{volume}{dir}{file}");
        self.out.keys.entry(nfc(&key)).or_insert(key);
        Ok(())
    }

    fn close(&mut self, tag: &Range<usize>) -> Result<(), TraktorError> {
        let frame = self
            .stack
            .pop()
            .ok_or_else(|| malformed("unexpected end tag"))?;
        if frame.inside_overtone
            && let Some(declared) = frame.declared
            && declared != frame.children
        {
            self.out.overtone_problems.push(format!(
                "{} declares {declared} but holds {}",
                show(&frame.name),
                frame.children
            ));
        }
        match frame.role {
            Role::RootSubnodes => {
                self.out.root_subnodes_end = tag.start;
                self.root_closed = true;
            }
            Role::Overtone => {
                let start = self.overtone_start.take().unwrap_or(tag.start);
                let end = newline_after(self.bytes, tag.end);
                self.out.overtone_span = Some(start..end);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Scans `collection.nml`. The whole file is parsed, so this also checks
/// that it is well-formed UTF-8 XML. Fails when it has no `PLAYLISTS`
/// `$ROOT` folder with a `SUBNODES` element to add playlists to.
pub fn scan(bytes: &[u8]) -> Result<CollectionScan, TraktorError> {
    std::str::from_utf8(bytes).map_err(|err| malformed(format!("not UTF-8: {err}")))?;
    let bom = bytes.starts_with(BOM);
    let offset = if bom { BOM.len() } else { 0 };
    let newline = match bytes.iter().position(|&b| b == b'\n') {
        Some(i) if i > 0 && bytes[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    };
    let mut reader = Reader::from_reader(&bytes[offset..]);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;

    let mut scanner = Scanner {
        bytes,
        stack: Vec::new(),
        out: CollectionScan {
            version: None,
            bom,
            newline,
            volume_counts: BTreeMap::new(),
            startup_volume: DEFAULT_STARTUP_VOLUME.to_string(),
            keys: HashMap::new(),
            root_subnodes_count_span: 0..0,
            root_subnodes_count: 0,
            root_child_count: 0,
            root_subnodes_end: 0,
            overtone_span: None,
            overtone_keys: Vec::new(),
            overtone_problems: Vec::new(),
        },
        users_volumes: BTreeMap::new(),
        found_root: false,
        found_root_subnodes: false,
        root_closed: false,
        overtone_start: None,
    };
    let position = |reader: &Reader<&[u8]>| {
        offset + usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
    };
    loop {
        let start = position(&reader);
        let event = reader
            .read_event()
            .map_err(|err| malformed(format!("at byte {start}: {err}")))?;
        let end = position(&reader);
        let tag = start..end;
        match &event {
            Event::Start(_) | Event::Empty(_) | Event::End(_)
                if bytes.get(start) != Some(&b'<') =>
            {
                return Err(malformed(format!("lost track of offsets at byte {start}")));
            }
            _ => {}
        }
        match event {
            Event::Start(e) => scanner.open(&e, tag)?,
            Event::Empty(e) => {
                if scanner.parent_role() == Some(Role::RootNode)
                    && e.name().as_ref().as_bytes() == b"SUBNODES"
                {
                    return Err(malformed(
                        "$ROOT has an empty SUBNODES element, which Overtone can't add to",
                    ));
                }
                scanner.open(&e, tag.clone())?;
                scanner.close(&(tag.end..tag.end))?;
            }
            Event::End(_) => scanner.close(&tag)?,
            Event::Eof => break,
            _ => {}
        }
    }
    if !scanner.stack.is_empty() {
        return Err(malformed("the file ends before its elements are closed"));
    }
    if !scanner.root_closed {
        return Err(malformed(
            "no PLAYLISTS $ROOT folder with SUBNODES to add playlists to",
        ));
    }
    let mut out = scanner.out;
    if let Some((volume, _)) = scanner
        .users_volumes
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
    {
        out.startup_volume.clone_from(volume);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_attribute_value_spans() {
        let tag = b"<SUBNODES  COUNT = '12'>";
        assert_eq!(attr_value_span(tag, b"COUNT"), Some(20..22));
        assert_eq!(
            attr_value_span(br#"<X XCOUNT="1" COUNT="2">"#, b"COUNT"),
            Some(21..22)
        );
        assert_eq!(attr_value_span(br#"<X OTHER="1">"#, b"COUNT"), None);
    }

    #[test]
    fn rejects_files_without_a_root_folder() {
        let nml = br#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<NML VERSION="19"><COLLECTION ENTRIES="0"></COLLECTION>
</NML>
"#;
        assert!(matches!(scan(nml), Err(TraktorError::Malformed(_))));
        assert!(matches!(
            scan(b"<NML><PLAYLISTS>"),
            Err(TraktorError::Malformed(_))
        ));
        assert!(matches!(scan(b"\xff\xfe"), Err(TraktorError::Malformed(_))));
    }
}
