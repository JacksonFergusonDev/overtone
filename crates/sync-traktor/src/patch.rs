//! Splices the Overtone folder into `collection.nml` by byte offsets and
//! verifies the result.

use crate::TraktorError;
use crate::scan::{CollectionScan, scan};

/// Returns `bytes` with the Overtone folder directly under `$ROOT` set to
/// `new_folder` (from [`crate::render_overtone_folder`]), or removed when
/// `new_folder` is `None`.
///
/// An existing folder is replaced in place. A new folder goes last in
/// `$ROOT`, right before its `</SUBNODES>`, and the `$ROOT` `SUBNODES COUNT`
/// goes up by one (down by one on removal). Every other byte is copied
/// unchanged.
pub fn patch(bytes: &[u8], new_folder: Option<&str>) -> Result<Vec<u8>, TraktorError> {
    let scan = scan(bytes)?;
    let (splice, delta) = match (&scan.overtone_span, new_folder) {
        (Some(span), Some(_)) => (span.clone(), 0),
        (Some(span), None) => (span.clone(), -1),
        (None, Some(_)) => (scan.root_subnodes_end..scan.root_subnodes_end, 1),
        (None, None) => return Ok(bytes.to_vec()),
    };
    let count = scan
        .root_subnodes_count
        .checked_add_signed(delta)
        .ok_or_else(|| TraktorError::Malformed("$ROOT SUBNODES COUNT is already 0".into()))?;
    let count_span = scan.root_subnodes_count_span.clone();
    debug_assert!(count_span.end <= splice.start);

    let mut out = Vec::with_capacity(bytes.len() + new_folder.map_or(0, str::len));
    out.extend_from_slice(&bytes[..count_span.start]);
    if delta == 0 {
        out.extend_from_slice(&bytes[count_span.clone()]);
    } else {
        out.extend_from_slice(count.to_string().as_bytes());
    }
    out.extend_from_slice(&bytes[count_span.end..splice.start]);
    out.extend_from_slice(new_folder.unwrap_or_default().as_bytes());
    out.extend_from_slice(&bytes[splice.end..]);
    Ok(out)
}

/// The bytes outside the Overtone folder and the `$ROOT` COUNT digits:
/// everything before the digits, then everything after them with the
/// folder cut out.
fn untouched<'a>(bytes: &'a [u8], scan: &CollectionScan) -> (&'a [u8], Vec<u8>) {
    let count = &scan.root_subnodes_count_span;
    let mut rest = Vec::with_capacity(bytes.len());
    match &scan.overtone_span {
        Some(folder) => {
            rest.extend_from_slice(&bytes[count.end..folder.start]);
            rest.extend_from_slice(&bytes[folder.end..]);
        }
        None => rest.extend_from_slice(&bytes[count.end..]),
    }
    (&bytes[..count.start], rest)
}

/// Checks a [`patch`] result against the original before it is written:
///
/// - the patched file parses as well-formed XML with a `$ROOT` folder;
/// - its Overtone folder is exactly `new_folder` (or absent for `None`);
/// - every byte outside that folder and the `$ROOT` COUNT digits is
///   identical to the original, and the root COUNT is as consistent with
///   the actual number of children as it was before;
/// - inside the folder, every `SUBNODES COUNT` and `PLAYLIST ENTRIES`
///   matches what it holds, and every track key is a collection entry.
///
/// Returns the patched file's scan.
pub fn verify(
    original: &[u8],
    patched: &[u8],
    new_folder: Option<&str>,
) -> Result<CollectionScan, TraktorError> {
    let fail = |msg: String| Err(TraktorError::Verify(msg));
    let before = scan(original)?;
    let after = scan(patched).map_err(|err| TraktorError::Verify(err.to_string()))?;

    match (after.overtone_bytes(patched), new_folder) {
        (Some(found), Some(expected)) if found == expected.as_bytes() => {}
        (None, None) => {}
        _ => return fail("the Overtone folder doesn't match what was rendered".into()),
    }
    if untouched(original, &before) != untouched(patched, &after) {
        return fail("bytes outside the Overtone folder changed".into());
    }
    let drift = |s: &CollectionScan| {
        i128::try_from(s.root_subnodes_count).unwrap_or(i128::MAX)
            - i128::try_from(s.root_child_count).unwrap_or(i128::MAX)
    };
    if drift(&before) != drift(&after) {
        return fail(format!(
            "$ROOT SUBNODES COUNT is {} but it holds {} folders and playlists",
            after.root_subnodes_count, after.root_child_count
        ));
    }
    if let Some(problem) = after.overtone_problems.first() {
        return fail(problem.clone());
    }
    if let Some(key) = after
        .overtone_keys
        .iter()
        .find(|key| after.lookup(key) != Some(key.as_str()))
    {
        return fail(format!("{key} isn't in the collection"));
    }
    if (before.bom, &before.version) != (after.bom, &after.version) {
        return fail("the file header changed".into());
    }
    Ok(after)
}
