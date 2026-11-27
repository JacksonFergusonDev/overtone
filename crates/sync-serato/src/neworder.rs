//! `neworder.pref`: the legacy crate order list.
//!
//! UTF-16BE without a BOM, `\n` line breaks, one record:
//!
//! ```text
//! [begin record]
//! [crate]Bass
//! [crate]Bass%%Flip Mix
//! [end record]
//! ```
//!
//! Crates are listed depth-first, parents before children. Lines are kept
//! verbatim so an unmodified file serializes to the same bytes.

use crate::SeratoError;
use crate::codec::{decode_utf16be, encode_utf16be};
use crate::names::{SUBCRATE_SEPARATOR, name_key};

const CRATE_PREFIX: &str = "[crate]";
const END_RECORD: &str = "[end record]";
const BEGIN_RECORD: &str = "[begin record]";

/// A parsed `neworder.pref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOrder {
    /// Every line, split on `\n`. A file ending in a newline has a final
    /// empty line, which keeps the round trip exact.
    lines: Vec<String>,
}

impl NewOrder {
    /// Parses file contents.
    pub fn parse(bytes: &[u8]) -> Result<Self, SeratoError> {
        let text = decode_utf16be(bytes)?;
        let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        if !lines.iter().any(|line| line == BEGIN_RECORD) {
            return Err(SeratoError::InvalidFormat(
                "neworder.pref has no [begin record] line".to_owned(),
            ));
        }
        Ok(Self { lines })
    }

    /// Serializes back to UTF-16BE.
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_utf16be(&self.lines.join("\n"))
    }

    /// Crate names in listed order, without the `[crate]` prefix.
    pub fn crates(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|line| line.strip_prefix(CRATE_PREFIX))
            .collect()
    }

    /// Whether a crate with this name (case and normalization insensitive)
    /// is listed.
    pub fn contains(&self, name: &str) -> bool {
        let key = name_key(name);
        self.crates().iter().any(|listed| name_key(listed) == key)
    }

    /// Lists `name` if it isn't already. A new crate goes after the last
    /// entry of its nearest listed ancestor's subtree, so it stays with its
    /// parent and after existing siblings. A top-level crate goes at the end
    /// of the list. Returns whether anything was added. Add parents before
    /// children.
    pub fn insert(&mut self, name: &str) -> bool {
        if self.contains(name) {
            return false;
        }
        let segments: Vec<&str> = name.split(SUBCRATE_SEPARATOR).collect();
        let mut anchor = None;
        for depth in (1..segments.len()).rev() {
            let ancestor = name_key(&segments[..depth].join(SUBCRATE_SEPARATOR));
            let prefix = format!("{ancestor}{SUBCRATE_SEPARATOR}");
            let last = self.lines.iter().rposition(|line| {
                line.strip_prefix(CRATE_PREFIX).is_some_and(|listed| {
                    let key = name_key(listed);
                    key == ancestor || key.starts_with(&prefix)
                })
            });
            if let Some(index) = last {
                anchor = Some(index);
                break;
            }
        }
        let index = anchor
            .or_else(|| {
                self.lines
                    .iter()
                    .rposition(|line| line.starts_with(CRATE_PREFIX))
            })
            .map(|last| last + 1)
            .or_else(|| self.lines.iter().position(|line| line == END_RECORD))
            .unwrap_or(self.lines.len());
        self.lines.insert(index, format!("{CRATE_PREFIX}{name}"));
        true
    }

    /// Removes the entry for `name` if listed. Returns whether it was.
    pub fn remove(&mut self, name: &str) -> bool {
        let key = name_key(name);
        let before = self.lines.len();
        self.lines.retain(|line| {
            line.strip_prefix(CRATE_PREFIX)
                .is_none_or(|listed| name_key(listed) != key)
        });
        self.lines.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(names: &[&str]) -> Vec<u8> {
        let mut text = String::from("[begin record]\n");
        for name in names {
            text.push_str(&format!("[crate]{name}\n"));
        }
        text.push_str("[end record]\n");
        encode_utf16be(&text)
    }

    #[test]
    fn round_trips_exactly() {
        let bytes = file(&["Bass", "Bass%%Flip", "Pop"]);
        let parsed = NewOrder::parse(&bytes).unwrap();
        assert_eq!(parsed.to_bytes(), bytes);
        assert_eq!(parsed.crates(), ["Bass", "Bass%%Flip", "Pop"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(NewOrder::parse(&[0, 0x41, 0]).is_err());
        assert!(NewOrder::parse(&encode_utf16be("hello")).is_err());
    }

    #[test]
    fn appends_top_level_crates_at_the_end() {
        let mut order = NewOrder::parse(&file(&["A", "A%%B", "Z"])).unwrap();
        assert!(order.insert("Overtone"));
        assert_eq!(order.crates(), ["A", "A%%B", "Z", "Overtone"]);
        assert!(!order.insert("overtone"), "already listed, any case");
    }

    #[test]
    fn places_children_after_parent_subtree_and_keeps_order() {
        let mut order = NewOrder::parse(&file(&["Overtone", "Overtone%%B", "Pop"])).unwrap();
        order.insert("Overtone%%A");
        order.insert("Overtone%%A%%Deep");
        order.insert("Overtone%%C");
        assert_eq!(
            order.crates(),
            [
                "Overtone",
                "Overtone%%B",
                "Overtone%%A",
                "Overtone%%A%%Deep",
                "Overtone%%C",
                "Pop"
            ]
        );
        assert_eq!(
            NewOrder::parse(&order.to_bytes()).unwrap(),
            order,
            "output parses back"
        );
    }

    #[test]
    fn builds_a_whole_chain_in_order() {
        let mut order = NewOrder::parse(&file(&["Pop"])).unwrap();
        for name in ["Overtone", "Overtone%%Dubs", "Overtone%%Dubs%%Deep"] {
            order.insert(name);
        }
        assert_eq!(
            order.crates(),
            ["Pop", "Overtone", "Overtone%%Dubs", "Overtone%%Dubs%%Deep"]
        );
    }

    #[test]
    fn inserts_into_an_empty_record() {
        let mut order = NewOrder::parse(&file(&[])).unwrap();
        order.insert("Overtone");
        assert_eq!(order.to_bytes(), file(&["Overtone"]));
    }

    #[test]
    fn removes_entries() {
        let mut order = NewOrder::parse(&file(&["A", "B"])).unwrap();
        assert!(order.remove("a"));
        assert!(!order.remove("A"));
        assert_eq!(order.to_bytes(), file(&["B"]));
    }
}
