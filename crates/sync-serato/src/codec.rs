//! Lossless codec for Serato's TLV container format.
//!
//! A `.crate` file is a stream of `[4-byte ASCII tag][u32 BE length][payload]`
//! records with no magic number, padding, or checksum. The container tags
//! `osrt`, `ovct`, and `otrk` hold nested records. Everything else is opaque
//! data, and unknown tags are kept verbatim so a parse followed by a
//! serialize returns the original bytes.

use crate::SeratoError;

/// A four-byte ASCII record tag.
pub type Tag = [u8; 4];

pub const TAG_VERSION: Tag = *b"vrsn";
pub const TAG_SORT: Tag = *b"osrt";
pub const TAG_COLUMN: Tag = *b"ovct";
pub const TAG_TRACK: Tag = *b"otrk";
pub const TAG_COLUMN_NAME: Tag = *b"tvcn";
pub const TAG_COLUMN_WIDTH: Tag = *b"tvcw";
pub const TAG_REVERSE: Tag = *b"brev";
pub const TAG_PATH: Tag = *b"ptrk";

/// Tags whose payload is a list of nested records.
const CONTAINER_TAGS: [Tag; 3] = [TAG_SORT, TAG_COLUMN, TAG_TRACK];

/// Deepest nesting accepted. Real files nest two levels.
const MAX_DEPTH: usize = 16;

/// Standard `vrsn` payload written at the start of Serato `.crate` files.
pub const CRATE_VERSION: &str = "1.0/Serato ScratchLive Crate";

/// Columns Serato writes into new crates. Most sampled crates (105 of 140)
/// give every one a width of `0`, which means automatic.
pub const DEFAULT_COLUMNS: [&str; 8] = [
    "song",
    "artist",
    "bpm",
    "key",
    "playCount",
    "album",
    "length",
    "comment",
];

/// What a record holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Opaque payload bytes.
    Data(Vec<u8>),
    /// Nested records of a container tag.
    Children(Vec<Record>),
}

/// One TLV record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub tag: Tag,
    pub content: Content,
}

impl Record {
    /// A record holding raw bytes.
    pub fn data(tag: Tag, bytes: Vec<u8>) -> Self {
        Self {
            tag,
            content: Content::Data(bytes),
        }
    }

    /// A record holding UTF-16BE text.
    pub fn text(tag: Tag, text: &str) -> Self {
        Self::data(tag, encode_utf16be(text))
    }

    /// A container record.
    pub fn container(tag: Tag, children: Vec<Record>) -> Self {
        Self {
            tag,
            content: Content::Children(children),
        }
    }

    /// A track record: `otrk` wrapping one `ptrk` path.
    pub fn track(ptrk: &str) -> Self {
        Self::container(TAG_TRACK, vec![Self::text(TAG_PATH, ptrk)])
    }

    /// Nested records, empty for data records.
    pub fn children(&self) -> &[Record] {
        match &self.content {
            Content::Children(children) => children,
            Content::Data(_) => &[],
        }
    }

    /// First nested record with `tag`.
    pub fn child(&self, tag: Tag) -> Option<&Record> {
        self.children().iter().find(|child| child.tag == tag)
    }

    /// Payload decoded as UTF-16BE text.
    pub fn as_text(&self) -> Result<String, SeratoError> {
        match &self.content {
            Content::Data(bytes) => decode_utf16be(bytes),
            Content::Children(_) => Err(SeratoError::InvalidFormat(format!(
                "{} is a container, not text",
                tag_name(self.tag)
            ))),
        }
    }

    /// The `ptrk` path of an `otrk` record.
    pub fn track_path(&self) -> Result<Option<String>, SeratoError> {
        match self.child(TAG_PATH) {
            Some(path) => path.as_text().map(Some),
            None => Ok(None),
        }
    }

    fn serialized_len(&self) -> usize {
        8 + match &self.content {
            Content::Data(bytes) => bytes.len(),
            Content::Children(children) => children.iter().map(Record::serialized_len).sum(),
        }
    }

    fn write_to(&self, out: &mut Vec<u8>) -> Result<(), SeratoError> {
        let payload_len = self.serialized_len() - 8;
        let len = u32::try_from(payload_len).map_err(|_| {
            SeratoError::InvalidFormat(format!("{} payload exceeds 4 GiB", tag_name(self.tag)))
        })?;
        out.extend_from_slice(&self.tag);
        out.extend_from_slice(&len.to_be_bytes());
        match &self.content {
            Content::Data(bytes) => out.extend_from_slice(bytes),
            Content::Children(children) => {
                for child in children {
                    child.write_to(out)?;
                }
            }
        }
        Ok(())
    }
}

/// A column definition from an `ovct` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// Width as Serato stores it: a decimal string, `0` for automatic.
    pub width: String,
}

/// The sort order from the `osrt` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortOrder {
    pub column: String,
    pub reverse: bool,
}

/// A parsed `.crate` file: an ordered list of top-level records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateFile {
    pub records: Vec<Record>,
}

impl Default for CrateFile {
    fn default() -> Self {
        Self::new()
    }
}

impl CrateFile {
    /// An empty crate with the header Serato writes: version, an `osrt`
    /// sorting by song, and the standard eight columns.
    pub fn new() -> Self {
        let mut records = vec![
            Record::text(TAG_VERSION, CRATE_VERSION),
            Record::container(
                TAG_SORT,
                vec![
                    Record::text(TAG_COLUMN_NAME, "song"),
                    Record::data(TAG_REVERSE, vec![0]),
                ],
            ),
        ];
        for name in DEFAULT_COLUMNS {
            records.push(Record::container(
                TAG_COLUMN,
                vec![
                    Record::text(TAG_COLUMN_NAME, name),
                    Record::text(TAG_COLUMN_WIDTH, "0"),
                ],
            ));
        }
        Self { records }
    }

    /// Parses a whole file. Any length that overruns its container is an
    /// error; nothing panics on malformed input.
    pub fn parse(bytes: &[u8]) -> Result<Self, SeratoError> {
        Ok(Self {
            records: parse_records(bytes, 0, 0)?,
        })
    }

    /// Serializes back to bytes. For a parsed file this reproduces the
    /// input exactly.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SeratoError> {
        let mut out = Vec::with_capacity(self.records.iter().map(Record::serialized_len).sum());
        for record in &self.records {
            record.write_to(&mut out)?;
        }
        Ok(out)
    }

    /// The `vrsn` string.
    pub fn version(&self) -> Result<Option<String>, SeratoError> {
        self.records
            .iter()
            .find(|record| record.tag == TAG_VERSION)
            .map(Record::as_text)
            .transpose()
    }

    /// The `osrt` sort column and direction, when present. Two of 140
    /// sampled crates have none.
    pub fn sort_order(&self) -> Result<Option<SortOrder>, SeratoError> {
        let Some(sort) = self.records.iter().find(|record| record.tag == TAG_SORT) else {
            return Ok(None);
        };
        let column = match sort.child(TAG_COLUMN_NAME) {
            Some(name) => name.as_text()?,
            None => String::new(),
        };
        let reverse = match sort.child(TAG_REVERSE).map(|r| &r.content) {
            Some(Content::Data(bytes)) => bytes.first().is_some_and(|byte| *byte != 0),
            _ => false,
        };
        Ok(Some(SortOrder { column, reverse }))
    }

    /// The `ovct` columns in order.
    pub fn columns(&self) -> Result<Vec<Column>, SeratoError> {
        self.records
            .iter()
            .filter(|record| record.tag == TAG_COLUMN)
            .map(|record| {
                let name = record
                    .child(TAG_COLUMN_NAME)
                    .map(Record::as_text)
                    .transpose()?
                    .unwrap_or_default();
                let width = record
                    .child(TAG_COLUMN_WIDTH)
                    .map(Record::as_text)
                    .transpose()?
                    .unwrap_or_default();
                Ok(Column { name, width })
            })
            .collect()
    }

    /// The `otrk` records in order.
    pub fn track_records(&self) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(|record| record.tag == TAG_TRACK)
    }

    /// Every track's `ptrk` path, in order. Paths are returned exactly as
    /// stored: volume-relative, no leading slash, and in the file system's
    /// own Unicode form.
    pub fn track_paths(&self) -> Result<Vec<String>, SeratoError> {
        let mut paths = Vec::new();
        for record in self.track_records() {
            if let Some(path) = record.track_path()? {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    /// Replaces the `otrk` records. Tracks go after every other record, where
    /// Serato puts them.
    pub fn set_track_records(&mut self, tracks: Vec<Record>) {
        self.records.retain(|record| record.tag != TAG_TRACK);
        self.records.extend(tracks);
    }

    /// Replaces all tracks with fresh records for `ptrk_paths`.
    pub fn set_track_paths<S: AsRef<str>>(&mut self, ptrk_paths: &[S]) {
        self.set_track_records(
            ptrk_paths
                .iter()
                .map(|path| Record::track(path.as_ref()))
                .collect(),
        );
    }
}

/// Printable form of a tag for error messages.
pub fn tag_name(tag: Tag) -> String {
    tag.iter()
        .map(|byte| {
            if byte.is_ascii_graphic() {
                char::from(*byte)
            } else {
                '?'
            }
        })
        .collect()
}

fn parse_records(bytes: &[u8], base: usize, depth: usize) -> Result<Vec<Record>, SeratoError> {
    if depth > MAX_DEPTH {
        return Err(SeratoError::InvalidFormat(format!(
            "records nested deeper than {MAX_DEPTH} levels at offset {base}"
        )));
    }
    let mut records = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let remaining = bytes.len() - pos;
        if remaining < 8 {
            return Err(SeratoError::InvalidFormat(format!(
                "truncated record header at offset {}: {remaining} byte(s) left",
                base + pos
            )));
        }
        let tag: Tag = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let len = u32::from_be_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]);
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        let start = pos + 8;
        if len > bytes.len() - start {
            return Err(SeratoError::InvalidFormat(format!(
                "record {} at offset {} claims {len} byte(s) but only {} remain",
                tag_name(tag),
                base + pos,
                bytes.len() - start
            )));
        }
        let payload = &bytes[start..start + len];
        let content = if CONTAINER_TAGS.contains(&tag) {
            Content::Children(parse_records(payload, base + start, depth + 1)?)
        } else {
            Content::Data(payload.to_vec())
        };
        records.push(Record { tag, content });
        pos = start + len;
    }
    Ok(records)
}

/// Encodes text as UTF-16BE without a BOM or terminator.
pub fn encode_utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// Decodes UTF-16BE without a BOM. Odd lengths and unpaired surrogates are
/// errors rather than being replaced, so bad data never turns into a
/// different path.
pub fn decode_utf16be(bytes: &[u8]) -> Result<String, SeratoError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(SeratoError::InvalidFormat(format!(
            "UTF-16 text has odd length {}",
            bytes.len()
        )));
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|err| SeratoError::InvalidFormat(format!("invalid UTF-16 text: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn new_crate_round_trips_and_has_standard_header() {
        let file = CrateFile::new();
        let bytes = file.to_bytes().unwrap();
        let parsed = CrateFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
        assert_eq!(parsed.version().unwrap().as_deref(), Some(CRATE_VERSION));
        let sort = parsed.sort_order().unwrap().unwrap();
        assert_eq!((sort.column.as_str(), sort.reverse), ("song", false));
        let names: Vec<_> = parsed
            .columns()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, DEFAULT_COLUMNS);
        assert!(
            (380..=400).contains(&bytes.len()),
            "about the size of an empty Serato crate: {}",
            bytes.len()
        );
    }

    #[test]
    fn tracks_round_trip_with_unicode_paths() {
        let mut file = CrateFile::new();
        let nfd = "Users/dj/Music/Beyonce\u{301}/Cr\u{e8}me 🎧.flac";
        file.set_track_paths(&[nfd, "Music/b.mp3"]);
        let bytes = file.to_bytes().unwrap();
        let parsed = CrateFile::parse(&bytes).unwrap();
        assert_eq!(parsed.track_paths().unwrap(), [nfd, "Music/b.mp3"]);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn preserves_unknown_records_and_nested_unknowns() {
        let mut bytes = rec(b"vrsn", &encode_utf16be("1.0/x"));
        bytes.extend(rec(b"zzzz", &[1, 2, 3]));
        bytes.extend(rec(
            b"otrk",
            &[rec(b"ptrk", &encode_utf16be("a")), rec(b"xxxx", &[9])].concat(),
        ));
        let parsed = CrateFile::parse(&bytes).unwrap();
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
        assert_eq!(parsed.track_paths().unwrap(), ["a"]);
    }

    #[test]
    fn rejects_truncated_header_and_overrun_lengths() {
        assert!(CrateFile::parse(&[]).unwrap().records.is_empty());
        assert!(CrateFile::parse(b"vrsn\0\0").is_err());
        assert!(CrateFile::parse(&rec(b"vrsn", &[0; 4])[..10]).is_err());
        let mut huge = b"vrsn".to_vec();
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            CrateFile::parse(&huge),
            Err(SeratoError::InvalidFormat(_))
        ));
    }

    #[test]
    fn rejects_child_overrunning_its_container() {
        // otrk whose ptrk claims 100 bytes it does not have.
        let mut inner = b"ptrk".to_vec();
        inner.extend_from_slice(&100u32.to_be_bytes());
        inner.extend_from_slice(&[0; 4]);
        assert!(CrateFile::parse(&rec(b"otrk", &inner)).is_err());
        // Container with trailing junk shorter than a header.
        assert!(CrateFile::parse(&rec(b"otrk", &[0, 0, 0])).is_err());
    }

    #[test]
    fn rejects_runaway_nesting() {
        let mut bytes = rec(b"otrk", &[]);
        for _ in 0..40 {
            bytes = rec(b"otrk", &bytes);
        }
        assert!(CrateFile::parse(&bytes).is_err());
    }

    #[test]
    fn bad_text_is_an_error_not_replacement() {
        let odd = rec(b"otrk", &rec(b"ptrk", &[0, 0x41, 0]));
        let parsed = CrateFile::parse(&odd).unwrap();
        assert!(parsed.track_paths().is_err());
        let lone = rec(b"otrk", &rec(b"ptrk", &[0xD8, 0x00]));
        assert!(CrateFile::parse(&lone).unwrap().track_paths().is_err());
        assert_eq!(parsed.to_bytes().unwrap(), odd, "still lossless");
    }

    #[test]
    fn set_track_paths_replaces_and_keeps_header_order() {
        let mut file = CrateFile::new();
        file.set_track_paths(&["a", "b"]);
        file.set_track_paths(&["c"]);
        assert_eq!(file.track_paths().unwrap(), ["c"]);
        assert_eq!(file.records.first().unwrap().tag, TAG_VERSION);
        assert_eq!(file.records.last().unwrap().tag, TAG_TRACK);
    }

    #[test]
    fn crate_without_osrt_reports_none() {
        let bytes = rec(b"vrsn", &encode_utf16be(CRATE_VERSION));
        let parsed = CrateFile::parse(&bytes).unwrap();
        assert_eq!(parsed.sort_order().unwrap(), None);
    }
}
