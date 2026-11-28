//! Safe writes of the text fields the scanner reads (title, artist, album,
//! album artist, remixer, label, genre, year, BPM, key, grouping).
//!
//! Each edit targets the one frame or Vorbis field the scanner reads its
//! value from, so the file reads back as intended without touching aliases
//! the DJ's other software may rely on. Writes go through the same splicer,
//! independent walkers, payload comparison, and re-read as Comment writes.
//! Comment itself stays with [`super::write_comment`].

use std::collections::HashSet;
use std::path::Path;

use overtone_fs::{FsError, ProtectedRoots};
use overtone_model::{AudioProperties, MetadataField, TrackTags};

use super::raw::{self, Id3v2Layout, Layout, RawFrame, VorbisComment, decode_text_frame};
use super::read::{detect, parse_year, read_as, read_with, scan_options};
use super::splice::{self, Editor, text_body};
use super::verify::Verifier;
use super::write::{prepare, scanner_view};
use super::{WriteOutcome, WriteReport, meta, parse_bpm};
use crate::AudioError;

/// The fields [`write_fields`] can write.
pub const WRITABLE_FIELDS: [MetadataField; 16] = [
    MetadataField::Title,
    MetadataField::Artist,
    MetadataField::Album,
    MetadataField::AlbumArtist,
    MetadataField::Remixer,
    MetadataField::Label,
    MetadataField::Genre,
    MetadataField::Year,
    MetadataField::Bpm,
    MetadataField::Key,
    MetadataField::Grouping,
    MetadataField::CatalogNumber,
    MetadataField::Isrc,
    MetadataField::ReleaseDate,
    MetadataField::TrackNumber,
    MetadataField::DiscNumber,
];

/// One field to write. `None` (or blank) clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEdit {
    pub field: MetadataField,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Year,
    Bpm,
    Date,
}

/// Where a field lives. The scanner reads the first ID3 frame, or the first
/// Vorbis key, in these lists that holds a usable value. An ID3 entry is a
/// frame ID, or `TXXX:DESCRIPTION` for a user-defined text frame.
struct Spec {
    kind: Kind,
    id3: &'static [&'static str],
    id3_new_v3: &'static str,
    id3_new_v4: &'static str,
    vorbis: &'static [&'static str],
    /// For track and disc numbers: the Vorbis keys holding the total, which
    /// Vorbis keeps apart from the number (`TRACKNUMBER=3`, `TRACKTOTAL=12`).
    vorbis_totals: &'static [&'static str],
}

fn spec(field: MetadataField) -> Option<Spec> {
    fn text(id3: &'static [&'static str], vorbis: &'static [&'static str]) -> Spec {
        Spec {
            kind: Kind::Text,
            id3,
            id3_new_v3: id3[0],
            id3_new_v4: id3[0],
            vorbis,
            vorbis_totals: &[],
        }
    }
    Some(match field {
        MetadataField::Title => text(&["TIT2"], &["TITLE"]),
        MetadataField::Artist => text(&["TPE1"], &["ARTIST"]),
        MetadataField::Album => text(&["TALB"], &["ALBUM"]),
        MetadataField::AlbumArtist => {
            text(&["TPE2"], &["ALBUMARTIST", "ALBUM ARTIST", "ALBUM_ARTIST"])
        }
        MetadataField::Remixer => text(&["TPE4"], &["REMIXER", "MIXARTIST"]),
        MetadataField::Label => text(&["TPUB"], &["LABEL", "ORGANIZATION", "PUBLISHER"]),
        MetadataField::Genre => text(&["TCON"], &["GENRE"]),
        MetadataField::Key => text(&["TKEY"], &["INITIALKEY", "KEY"]),
        MetadataField::Year => Spec {
            kind: Kind::Year,
            id3: &["TDRC", "TYER", "TDOR", "TORY"],
            id3_new_v3: "TYER",
            id3_new_v4: "TDRC",
            vorbis: &["DATE", "YEAR", "ORIGINALDATE"],
            vorbis_totals: &[],
        },
        MetadataField::Bpm => Spec {
            kind: Kind::Bpm,
            id3: &["TBPM"],
            id3_new_v3: "TBPM",
            id3_new_v4: "TBPM",
            vorbis: &["BPM", "TEMPO"],
            vorbis_totals: &[],
        },
        MetadataField::Grouping => Spec {
            kind: Kind::Text,
            id3: &["GRP1", "TIT1"],
            id3_new_v3: "TIT1",
            id3_new_v4: "TIT1",
            vorbis: &["GROUPING"],
            vorbis_totals: &[],
        },
        MetadataField::CatalogNumber => text(&["TXXX:CATALOGNUMBER"], &["CATALOGNUMBER"]),
        MetadataField::Isrc => text(&["TSRC"], &["ISRC"]),
        MetadataField::TrackNumber => Spec {
            vorbis_totals: &["TRACKTOTAL", "TOTALTRACKS"],
            ..text(&["TRCK"], &["TRACKNUMBER"])
        },
        MetadataField::DiscNumber => Spec {
            vorbis_totals: &["DISCTOTAL", "TOTALDISCS"],
            ..text(&["TPOS"], &["DISCNUMBER"])
        },
        MetadataField::ReleaseDate => Spec {
            kind: Kind::Date,
            id3: &["TDRL", "TXXX:RELEASEDATE"],
            id3_new_v3: "TXXX:RELEASEDATE",
            id3_new_v4: "TDRL",
            vorbis: &["RELEASEDATE"],
            vorbis_totals: &[],
        },
        _ => return None,
    })
}

/// `2021`, `2021-06`, or `2021-06-04`: the dates ID3v2.4 timestamps hold.
fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [y] => digits(y, 4),
        [y, m] => digits(y, 4) && digits(m, 2),
        [y, m, d] => digits(y, 4) && digits(m, 2) && digits(d, 2),
        _ => false,
    }
}

/// Whether the scanner would take `text` as this field's value.
fn accepted(kind: Kind, text: &str) -> bool {
    match kind {
        Kind::Text | Kind::Date => text.split('\0').any(|part| !part.trim().is_empty()),
        Kind::Year => parse_year(text).is_some(),
        Kind::Bpm => parse_bpm(text).is_some(),
    }
}

fn refused(path: &Path, reason: impl Into<String>) -> AudioError {
    AudioError::WriteRefused {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// The value to store: trimmed, `None` when blank; years become four
/// digits. Errors name the problem.
fn normalize(
    field: MetadataField,
    kind: Kind,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.contains('\0') {
        return Err(format!(
            "the new {} contains a NUL character",
            field.as_str()
        ));
    }
    match kind {
        Kind::Text => Ok(Some(value.to_string())),
        Kind::Year => parse_year(value)
            .map(|year| Some(format!("{year:04}")))
            .ok_or_else(|| format!("{value:?} isn't a year")),
        Kind::Bpm => parse_bpm(value)
            .map(|_| Some(value.to_string()))
            .ok_or_else(|| format!("{value:?} isn't a BPM")),
        Kind::Date if is_date(value) => Ok(Some(value.to_string())),
        Kind::Date => Err(format!(
            "{value:?} isn't a date (use YYYY, YYYY-MM, or YYYY-MM-DD)"
        )),
    }
}

/// `tags` as the scanner should read them after the edits.
fn expected_tags(tags: &TrackTags, edits: &[(MetadataField, Option<String>)]) -> TrackTags {
    let mut out = tags.clone();
    for (field, value) in edits {
        let text = value.clone();
        match field {
            MetadataField::Title => out.title = text,
            MetadataField::Artist => out.artist = text,
            MetadataField::Album => out.album = text,
            MetadataField::AlbumArtist => out.album_artist = text,
            MetadataField::Remixer => out.remixer = text,
            MetadataField::Label => out.label = text,
            MetadataField::Genre => out.genre = text,
            MetadataField::Key => out.key = text,
            MetadataField::Grouping => out.grouping = text,
            MetadataField::Year => out.year = text.as_deref().and_then(parse_year),
            MetadataField::Bpm => out.bpm = text.as_deref().and_then(parse_bpm),
            MetadataField::CatalogNumber => out.catalog_number = text,
            MetadataField::Isrc => out.isrc = text,
            MetadataField::ReleaseDate => out.release_date = text,
            MetadataField::TrackNumber => out.track_number = text,
            MetadataField::DiscNumber => out.disc_number = text,
            _ => {}
        }
    }
    out
}

/// Whether a file whose tags read as `tags` already holds `value` for
/// `field`, as [`write_fields`] would store it. Fields it can't write never
/// match. Use it to find values that still need writing.
pub fn reads_as(field: MetadataField, tags: &TrackTags, value: Option<&str>) -> bool {
    let Some(spec) = spec(field) else {
        return false;
    };
    match normalize(field, spec.kind, value) {
        Ok(value) => expected_tags(tags, &[(field, value)]) == *tags,
        Err(_) => false,
    }
}

/// One ID3 edit: frames with these keys ([`frame_key`]) are replaced or
/// removed, and afterwards exactly `write`'s frame (or none) holds the
/// field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Id3Op {
    pub owned: Vec<&'static str>,
    pub write: Option<(&'static str, String)>,
}

/// The Vorbis comment after the edits, and the keys they could touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VorbisPlan {
    pub entries: Vec<Vec<u8>>,
    pub changed: bool,
    /// Every alias key of every edited field, upper case.
    pub edited_keys: Vec<&'static str>,
    /// For each edit: its aliases, kind, and the value a reader must find.
    checks: Vec<VorbisCheck>,
}

/// What a reader must find for one edited field after the write.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VorbisCheck {
    aliases: &'static [&'static str],
    totals: &'static [&'static str],
    kind: Kind,
    value: Option<String>,
}

/// Everything a field write changes, planned from the original's layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldPlan {
    pub id3: Vec<Id3Op>,
    pub vorbis: Option<VorbisPlan>,
}

/// A frame's identity for field writes: its ID, or `TXXX:DESCRIPTION`
/// (upper case) for a user-defined text frame.
fn frame_key(frame: &RawFrame) -> String {
    if frame.id == "TXXX"
        && let Some(strings) = decode_text_frame(frame)
    {
        let description = strings.first().map_or("", String::as_str);
        return format!("TXXX:{}", description.to_ascii_uppercase());
    }
    frame.id.clone()
}

/// The value a frame holds: its text, or for `TXXX` the text after the
/// description.
fn frame_text(frame: &RawFrame) -> Option<String> {
    let strings = decode_text_frame(frame)?;
    let values = if frame.id == "TXXX" {
        strings.get(1..).unwrap_or_default()
    } else {
        &strings[..]
    };
    Some(values.join("\0"))
}

/// A frame holding `text` under `key`, keeping `old`'s flags, encoding, and
/// (for `TXXX`) description when there is one.
fn build_frame(major: u8, key: &str, old: Option<&RawFrame>, text: &str) -> RawFrame {
    let flags = old.map_or(0, |f| f.flags);
    match key.strip_prefix("TXXX:") {
        Some(new_description) => {
            let description = old
                .and_then(decode_text_frame)
                .and_then(|s| s.into_iter().next())
                .unwrap_or_else(|| new_description.to_string());
            RawFrame {
                id: "TXXX".into(),
                flags,
                body: splice::user_text_body(major, old, &description, text),
            }
        }
        None => RawFrame {
            id: key.to_string(),
            flags,
            body: text_body(major, old, text),
        },
    }
}

fn plan_id3(tag: &Id3v2Layout, edits: &[(Spec, Option<String>)]) -> Result<Vec<Id3Op>, String> {
    let mut ops = Vec::new();
    for (spec, value) in edits {
        for key in spec.id3 {
            if tag.frames.iter().filter(|f| frame_key(f) == *key).count() > 1 {
                return Err(format!("it has several {key} frames"));
            }
        }
        let frame = |key: &str| tag.frames.iter().find(|f| frame_key(f) == key);
        let op = match value {
            None => Id3Op {
                owned: spec.id3.to_vec(),
                write: None,
            },
            Some(text) => {
                let chosen = spec
                    .id3
                    .iter()
                    .find(|id| {
                        frame(id)
                            .and_then(frame_text)
                            .is_some_and(|t| accepted(spec.kind, &t))
                    })
                    .or_else(|| spec.id3.iter().find(|id| frame(id).is_some()))
                    .copied()
                    .unwrap_or(if tag.major == 4 {
                        spec.id3_new_v4
                    } else {
                        spec.id3_new_v3
                    });
                Id3Op {
                    owned: vec![chosen],
                    write: Some((chosen, text.clone())),
                }
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

fn vorbis_key(entry: &[u8]) -> &[u8] {
    let end = entry.iter().position(|b| *b == b'=').unwrap_or(entry.len());
    &entry[..end]
}

fn vorbis_value(entry: &[u8]) -> String {
    let start = entry
        .iter()
        .position(|b| *b == b'=')
        .map_or(entry.len(), |i| i + 1);
    String::from_utf8_lossy(&entry[start..]).into_owned()
}

fn has_key(entry: &[u8], key: &str) -> bool {
    vorbis_key(entry).eq_ignore_ascii_case(key.as_bytes())
}

/// What a reader that takes the first entry of the first alias with a
/// usable value finds. Independent of how the plan was made.
fn read_vorbis(entries: &[Vec<u8>], aliases: &[&str], kind: Kind) -> Option<String> {
    aliases.iter().find_map(|key| {
        let value = vorbis_value(entries.iter().find(|e| has_key(e, key))?);
        accepted(kind, &value).then_some(value)
    })
}

/// Points the first usable entry among `aliases` (else the first present,
/// else a new `aliases[0]` entry) at `text`.
fn set_entry(entries: &mut Vec<Vec<u8>>, aliases: &[&str], kind: Kind, text: &str) {
    let first = |entries: &[Vec<u8>], key: &str| entries.iter().position(|e| has_key(e, key));
    let target = aliases
        .iter()
        .find_map(|key| first(entries, key).filter(|i| accepted(kind, &vorbis_value(&entries[*i]))))
        .or_else(|| aliases.iter().find_map(|key| first(entries, key)));
    let entry = |key: &[u8]| {
        let mut entry = key.to_vec();
        entry.push(b'=');
        entry.extend_from_slice(text.as_bytes());
        entry
    };
    match target {
        Some(i) => entries[i] = entry(vorbis_key(&entries[i])),
        None => entries.push(entry(aliases[0].as_bytes())),
    }
}

fn remove_entries(entries: &mut Vec<Vec<u8>>, keys: &[&str]) {
    entries.retain(|e| !keys.iter().any(|key| has_key(e, key)));
}

fn plan_vorbis(existing: Option<&VorbisComment>, edits: &[(Spec, Option<String>)]) -> VorbisPlan {
    let original = existing.map_or_else(Vec::new, |vc| vc.entries.clone());
    let mut entries = original.clone();
    let mut edited_keys = Vec::new();
    let mut checks = Vec::new();
    for (spec, value) in edits {
        edited_keys.extend_from_slice(spec.vorbis);
        edited_keys.extend_from_slice(spec.vorbis_totals);
        checks.push(VorbisCheck {
            aliases: spec.vorbis,
            totals: spec.vorbis_totals,
            kind: spec.kind,
            value: value.clone(),
        });
        let Some(text) = value else {
            remove_entries(&mut entries, spec.vorbis);
            remove_entries(&mut entries, spec.vorbis_totals);
            continue;
        };
        if spec.vorbis_totals.is_empty() {
            set_entry(&mut entries, spec.vorbis, spec.kind, text);
            continue;
        }
        let (number, total) = match text.split_once('/') {
            Some((n, t)) => (n.trim(), Some(t.trim()).filter(|t| !t.is_empty())),
            None => (text.as_str(), None),
        };
        set_entry(&mut entries, spec.vorbis, spec.kind, number);
        match total {
            Some(total) => set_entry(&mut entries, spec.vorbis_totals, Kind::Text, total),
            None => remove_entries(&mut entries, spec.vorbis_totals),
        }
    }
    VorbisPlan {
        changed: entries != original,
        entries,
        edited_keys,
        checks,
    }
}

struct FieldsEditor<'a>(&'a FieldPlan);

impl Editor for FieldsEditor<'_> {
    fn id3_frames(&self, tag: &Id3v2Layout) -> Result<Vec<RawFrame>, raw::RawError> {
        let ops = &self.0.id3;
        let mut written = vec![false; ops.len()];
        let mut frames = Vec::with_capacity(tag.frames.len() + ops.len());
        for frame in &tag.frames {
            let key = frame_key(frame);
            let owner = ops.iter().position(|op| op.owned.contains(&key.as_str()));
            let Some(i) = owner else {
                frames.push(frame.clone());
                continue;
            };
            if let Some((target, text)) = &ops[i].write
                && *target == key
                && !written[i]
            {
                frames.push(build_frame(tag.major, target, Some(frame), text));
                written[i] = true;
            }
        }
        for (op, done) in ops.iter().zip(written) {
            if let (Some((target, text)), false) = (&op.write, done) {
                frames.push(build_frame(tag.major, target, None, text));
            }
        }
        Ok(frames)
    }

    fn vorbis_entries(
        &self,
        _existing: Option<&VorbisComment>,
    ) -> Result<Option<Vec<Vec<u8>>>, raw::RawError> {
        Ok(self
            .0
            .vorbis
            .as_ref()
            .filter(|plan| plan.changed)
            .map(|plan| plan.entries.clone()))
    }
}

/// Checks the edited copy's ID3 frames: frames the edits own hold exactly
/// the planned values, and every other frame survives (binary frames byte
/// for byte, text frames decoding to the same strings).
pub(crate) fn check_id3(
    ops: &[Id3Op],
    original: Option<&Id3v2Layout>,
    temp: Option<&Id3v2Layout>,
    report: &mut WriteReport,
) -> Result<(), (&'static str, String)> {
    let owned: HashSet<&str> = ops.iter().flat_map(|op| op.owned.iter().copied()).collect();
    let rest = |tag: Option<&Id3v2Layout>| -> Vec<RawFrame> {
        tag.map_or_else(Vec::new, |t| {
            t.frames
                .iter()
                .filter(|f| !owned.contains(frame_key(f).as_str()))
                .cloned()
                .collect()
        })
    };
    let original_rest = rest(original);
    let Some(temp_tag) = temp else {
        if original_rest.is_empty() && ops.iter().all(|op| op.write.is_none()) {
            return Ok(());
        }
        return Err(("id3 frames", "the edited copy lost its ID3v2 tag".into()));
    };
    if let Some(original) = original
        && original.major != temp_tag.major
    {
        return Err((
            "id3 version",
            format!("ID3v2.{} became ID3v2.{}", original.major, temp_tag.major),
        ));
    }
    for op in ops {
        let found: Vec<&RawFrame> = temp_tag
            .frames
            .iter()
            .filter(|f| op.owned.contains(&frame_key(f).as_str()))
            .collect();
        match (&op.write, found.as_slice()) {
            (None, []) => {}
            (Some((key, text)), [frame])
                if frame_key(frame) == *key && frame_text(frame).as_ref() == Some(text) => {}
            _ => {
                return Err((
                    "field value",
                    format!(
                        "expected {:?}, found {:?}",
                        op.write,
                        found
                            .iter()
                            .map(|f| (frame_key(f), frame_text(f)))
                            .collect::<Vec<_>>()
                    ),
                ));
            }
        }
    }
    let temp_rest = rest(Some(temp_tag));
    let a: Vec<&RawFrame> = original_rest.iter().collect();
    let b: Vec<&RawFrame> = temp_rest.iter().collect();
    super::verify::compare_frames(&a, &b, report)
}

/// Checks the edited copy's Vorbis entries: exactly the planned list, the
/// same vendor, entries under other keys unchanged and in order, and a
/// reader finds each edited value.
pub(crate) fn check_vorbis(
    plan: &VorbisPlan,
    before: Option<&VorbisComment>,
    after: Option<&VorbisComment>,
) -> Result<(), (&'static str, String)> {
    let Some(after) = after else {
        if !plan.changed {
            return Ok(());
        }
        return Err(("vorbis comment", "the VORBIS_COMMENT block was lost".into()));
    };
    if let Some(before) = before
        && before.vendor != after.vendor
    {
        return Err(("vorbis comment", "the vendor string changed".into()));
    }
    let untouched = |entries: &[Vec<u8>]| {
        entries
            .iter()
            .filter(|e| !plan.edited_keys.iter().any(|key| has_key(e, key)))
            .cloned()
            .collect::<Vec<_>>()
    };
    let original = before.map_or_else(Vec::new, |b| b.entries.clone());
    if untouched(&original) != untouched(&after.entries) {
        return Err((
            "vorbis comment",
            "fields the write didn't target changed".into(),
        ));
    }
    if after.entries != plan.entries {
        return Err((
            "vorbis comment",
            "the entries differ from the planned edit".into(),
        ));
    }
    for check in &plan.checks {
        let (aliases, value) = (check.aliases, &check.value);
        let found =
            read_vorbis(&after.entries, aliases, check.kind).map(|number| {
                match read_vorbis(&after.entries, check.totals, Kind::Text) {
                    Some(total) if !number.contains('/') => format!("{number}/{total}"),
                    _ => number,
                }
            });
        if found != *value {
            return Err((
                "field value",
                format!("{}: expected {value:?}, found {found:?}", aliases[0]),
            ));
        }
    }
    Ok(())
}

/// Writes `edits` into the file's tags, for MP3, WAV, AIFF (ID3v2) and FLAC
/// (Vorbis comments). Only [`WRITABLE_FIELDS`] are accepted.
///
/// Each field goes into the frame or key the scanner reads it from, or a
/// new standard one; clearing a field removes every frame or key it could
/// be read from. Like [`super::write_comment`], a copy is written with only
/// those frames or fields rebuilt, verified against the original (other
/// frames, chunks, FLAC blocks, the audio bytes, and a re-read of every
/// tag and stream property), and only then replaces the original. When the
/// file already reads as requested, nothing is written.
pub fn write_fields(
    roots: &ProtectedRoots,
    path: &Path,
    edits: &[FieldEdit],
) -> Result<WriteOutcome, AudioError> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for edit in edits {
        let spec = spec(edit.field).ok_or_else(|| {
            refused(
                path,
                format!("Overtone doesn't write the {} field", edit.field.as_str()),
            )
        })?;
        if !seen.insert(edit.field) {
            return Err(refused(
                path,
                format!("{} is edited twice", edit.field.as_str()),
            ));
        }
        let value = normalize(edit.field, spec.kind, edit.value.as_deref())
            .map_err(|reason| refused(path, reason))?;
        normalized.push((edit.field, spec, value));
    }
    roots.check(path)?;
    let (layout, tags) = prepare(path)?;
    let values: Vec<(MetadataField, Option<String>)> = normalized
        .iter()
        .map(|(field, _, value)| (*field, value.clone()))
        .collect();
    let expected = expected_tags(&tags, &values);
    if expected == tags {
        return Ok(WriteOutcome::Unchanged);
    }
    let specs: Vec<(Spec, Option<String>)> = normalized
        .into_iter()
        .map(|(_, spec, value)| (spec, value))
        .collect();
    let plan = if let Some(flac) = &layout.flac {
        let existing = flac.vorbis_comment().map_err(|err| refused(path, err.0))?;
        FieldPlan {
            id3: Vec::new(),
            vorbis: Some(plan_vorbis(existing.as_ref(), &specs)),
        }
    } else {
        let empty = splice::empty_id3();
        let tag = layout.id3v2.as_ref().unwrap_or(&empty);
        FieldPlan {
            id3: plan_id3(tag, &specs).map_err(|reason| refused(path, reason))?,
            vorbis: None,
        }
    };
    let properties = read_with(path, scan_options().read_tags(false))?.properties;

    let mut failure: Option<AudioError> = None;
    let mut report = None;
    let result = overtone_fs::edit_via_copy(roots, path, |temp| {
        match edit_and_verify(path, &layout, &expected, &properties, temp, &plan) {
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

fn failed(path: &Path, check: &'static str, detail: impl Into<String>) -> AudioError {
    AudioError::VerificationFailed {
        path: path.to_path_buf(),
        check,
        detail: detail.into(),
    }
}

fn edit_and_verify(
    original: &Path,
    layout: &Layout,
    expected: &TrackTags,
    properties: &AudioProperties,
    temp: &Path,
    plan: &FieldPlan,
) -> Result<WriteReport, AudioError> {
    splice::write_edited(original, layout, temp, &FieldsEditor(plan))
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
        expected: None,
        fields: Some(plan),
    }
    .run()?;

    let reread = scanner_view(temp, &temp_layout)
        .map_err(|err| failed(original, "re-read", err.to_string()))?;
    if &reread != expected {
        return Err(failed(
            original,
            "re-read",
            format!("tags changed: expected {expected:?}, read back {reread:?}"),
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
