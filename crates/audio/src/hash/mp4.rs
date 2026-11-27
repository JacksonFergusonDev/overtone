//! MP4 / M4A: the payload is the contents of every top-level `mdat` atom.
//!
//! iTunes-style tags live in `moov/udta/meta/ilst`, and taggers resize
//! `moov`, add `free` atoms, or move `moov` before or after `mdat`. None of
//! that touches the sample data in `mdat`, so hashing only `mdat` bodies (in
//! file order, which is what fragmented files with several `mdat` atoms
//! need) keeps the hash stable across tag edits.

use std::io::{Read, Seek};
use std::ops::Range;

use overtone_model::AudioFormat;

use super::PayloadLayout;
use super::source::{PayloadError, Source, u32_be, u64_be};

const CONTAINER: &str = "MP4";

/// Whether `head` starts with an `ftyp` atom, which every MP4 brand
/// (M4A, M4B, isom, mp42) begins with.
pub(super) fn is_mp4(head: &[u8]) -> bool {
    head.len() >= 8 && &head[4..8] == b"ftyp"
}

#[derive(Debug, Clone)]
struct Atom {
    kind: [u8; 4],
    /// The atom's contents, after its 8- or 16-byte header.
    body: Range<u64>,
}

pub(super) fn layout<R: Read + Seek>(src: &mut Source<R>) -> Result<PayloadLayout, PayloadError> {
    let atoms = atoms_in(src, 0..src.len())?;
    let ranges: Vec<Range<u64>> = atoms
        .iter()
        .filter(|atom| &atom.kind == b"mdat" && !atom.body.is_empty())
        .map(|atom| atom.body.clone())
        .collect();
    if ranges.is_empty() {
        return Err(PayloadError::malformed(CONTAINER, "no non-empty mdat atom"));
    }
    // The codec only fills in `format`; a damaged sample table mustn't stop
    // the hash, since the hash never depends on it.
    let format = atoms
        .iter()
        .find(|atom| &atom.kind == b"moov")
        .and_then(|moov| audio_codec(src, moov.body.clone()).ok().flatten());
    Ok(PayloadLayout { format, ranges })
}

/// Reads the sequence of atoms that exactly fills `within`.
fn atoms_in<R: Read + Seek>(
    src: &mut Source<R>,
    within: Range<u64>,
) -> Result<Vec<Atom>, PayloadError> {
    let mut atoms = Vec::new();
    let mut pos = within.start;
    while pos < within.end {
        let atom = atom_at(src, pos, within.end)?;
        pos = atom.body.end;
        atoms.push(atom);
    }
    Ok(atoms)
}

/// Parses the atom header at `pos`. A 32-bit size of 1 means a 64-bit size
/// follows the type; 0 means the atom runs to the end of its parent.
fn atom_at<R: Read + Seek>(
    src: &mut Source<R>,
    pos: u64,
    parent_end: u64,
) -> Result<Atom, PayloadError> {
    if parent_end - pos < 8 {
        return Err(PayloadError::malformed(
            CONTAINER,
            format!("{} stray bytes at byte {pos}", parent_end - pos),
        ));
    }
    let header: [u8; 8] = src.array_at(CONTAINER, pos)?;
    let kind = [header[4], header[5], header[6], header[7]];
    if !kind.iter().all(|b| (0x20..=0x7E).contains(b) || *b == 0xA9) {
        return Err(PayloadError::malformed(
            CONTAINER,
            format!("garbled atom type at byte {pos}"),
        ));
    }
    let (size, header_len) = match u32_be(&header[..4]) {
        0 => (parent_end - pos, 8),
        1 => (u64_be(&src.array_at::<8>(CONTAINER, pos + 8)?), 16),
        n => (u64::from(n), 8),
    };
    let end = pos.checked_add(size).filter(|&end| end <= parent_end);
    match end {
        Some(end) if size >= header_len => Ok(Atom {
            kind,
            body: pos + header_len..end,
        }),
        _ => Err(PayloadError::malformed(
            CONTAINER,
            format!(
                "{} atom at byte {pos} claims {size} bytes, past the end of its parent",
                String::from_utf8_lossy(&kind)
            ),
        )),
    }
}

/// Finds the first track's audio sample entry under `moov` and maps its
/// four-character code to a format: `alac` is ALAC, `mp4a` is AAC.
fn audio_codec<R: Read + Seek>(
    src: &mut Source<R>,
    moov: Range<u64>,
) -> Result<Option<AudioFormat>, PayloadError> {
    for trak in children(src, moov, *b"trak")? {
        let Some(stsd) = descend(src, trak, &[*b"mdia", *b"minf", *b"stbl", *b"stsd"])? else {
            continue;
        };
        // stsd: version and flags (4), entry count (4), then sample entries,
        // each an atom whose type is the codec.
        let entries = stsd.start + 8..stsd.end;
        if let Some(entry) = atoms_in(src, entries)?.first() {
            match &entry.kind {
                b"alac" => return Ok(Some(AudioFormat::Alac)),
                b"mp4a" => return Ok(Some(AudioFormat::Aac)),
                _ => {}
            }
        }
    }
    Ok(None)
}

fn children<R: Read + Seek>(
    src: &mut Source<R>,
    parent: Range<u64>,
    kind: [u8; 4],
) -> Result<Vec<Range<u64>>, PayloadError> {
    Ok(atoms_in(src, parent)?
        .into_iter()
        .filter(|atom| atom.kind == kind)
        .map(|atom| atom.body)
        .collect())
}

fn descend<R: Read + Seek>(
    src: &mut Source<R>,
    mut range: Range<u64>,
    path: &[[u8; 4]],
) -> Result<Option<Range<u64>>, PayloadError> {
    for kind in path {
        match children(src, range, *kind)?.into_iter().next() {
            Some(child) => range = child,
            None => return Ok(None),
        }
    }
    Ok(Some(range))
}
