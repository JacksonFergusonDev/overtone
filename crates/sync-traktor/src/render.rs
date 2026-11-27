//! Renders the Overtone playlist folder in Traktor's own layout: no
//! indentation, a line break after every end tag, explicit end tags.

use std::collections::HashMap;

use crate::OVERTONE_FOLDER;

/// A node in the playlist tree under the Overtone folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistNode {
    Folder {
        name: String,
        children: Vec<PlaylistNode>,
    },
    Playlist {
        name: String,
        /// Collection keys, as the collection spells them.
        keys: Vec<String>,
    },
}

impl PlaylistNode {
    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } | Self::Playlist { name, .. } => name,
        }
    }
}

/// Escapes text for a double-quoted attribute value.
fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            _ => out.push(ch),
        }
    }
    out
}

/// A stable 32-hex-digit playlist UUID derived from the playlist's place in
/// the tree: the names of the folders above it, its own name, and how many
/// earlier siblings share that name. Reruns produce the same UUIDs, and
/// renaming a vibe gives its playlist a new one.
pub fn playlist_uuid(path: &[String], occurrence: usize) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"overtone traktor playlist\0");
    for segment in path {
        hasher.update(segment.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(occurrence.to_string().as_bytes());
    let hash = hasher.finalize();
    hash.as_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Traktor stores stem files with `PRIMARYKEY TYPE="STEM"`.
fn key_type(key: &str) -> &'static str {
    let lower = key.to_lowercase();
    if lower.ends_with(".stem.mp4") || lower.ends_with(".stem.m4a") {
        "STEM"
    } else {
        "TRACK"
    }
}

fn render_nodes(nodes: &[PlaylistNode], path: &mut Vec<String>, nl: &str, out: &mut String) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for node in nodes {
        let occurrence = seen.entry(node.name()).or_default();
        path.push(node.name().to_string());
        let name = escape_attr(node.name());
        match node {
            PlaylistNode::Folder { children, .. } => {
                out.push_str(&format!(
                    "<NODE TYPE=\"FOLDER\" NAME=\"{name}\"><SUBNODES COUNT=\"{}\">",
                    children.len()
                ));
                render_nodes(children, path, nl, out);
                out.push_str(&format!("</SUBNODES>{nl}</NODE>{nl}"));
            }
            PlaylistNode::Playlist { keys, .. } => {
                out.push_str(&format!(
                    "<NODE TYPE=\"PLAYLIST\" NAME=\"{name}\"><PLAYLIST ENTRIES=\"{}\" TYPE=\"LIST\" UUID=\"{}\">",
                    keys.len(),
                    playlist_uuid(path, *occurrence)
                ));
                for key in keys {
                    out.push_str(&format!(
                        "<ENTRY><PRIMARYKEY TYPE=\"{}\" KEY=\"{}\"></PRIMARYKEY>{nl}</ENTRY>{nl}",
                        key_type(key),
                        escape_attr(key)
                    ));
                }
                out.push_str(&format!("</PLAYLIST>{nl}</NODE>{nl}"));
            }
        }
        *occurrence += 1;
        path.pop();
    }
}

/// The `NODE TYPE="FOLDER" NAME="Overtone"` element holding `children`,
/// ending with a line break. `newline` is the collection's line ending.
pub fn render_overtone_folder(children: &[PlaylistNode], newline: &str) -> String {
    let mut out = String::new();
    render_nodes(
        &[PlaylistNode::Folder {
            name: OVERTONE_FOLDER.to_string(),
            children: children.to_vec(),
        }],
        &mut Vec::new(),
        newline,
        &mut out,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_traktor_layout() {
        let folder = render_overtone_folder(
            &[
                PlaylistNode::Folder {
                    name: "Dubs & Bass".into(),
                    children: vec![PlaylistNode::Playlist {
                        name: "Deep".into(),
                        keys: vec!["Macintosh HD/:Users/:me/:a.mp3".into()],
                    }],
                },
                PlaylistNode::Playlist {
                    name: "Stems".into(),
                    keys: vec!["Gig/:x.stem.mp4".into()],
                },
                PlaylistNode::Playlist {
                    name: "Empty".into(),
                    keys: vec![],
                },
            ],
            "\n",
        );
        let uuid = |path: &[&str]| {
            playlist_uuid(&path.iter().map(ToString::to_string).collect::<Vec<_>>(), 0)
        };
        let expected = format!(
            "<NODE TYPE=\"FOLDER\" NAME=\"Overtone\"><SUBNODES COUNT=\"3\">\
<NODE TYPE=\"FOLDER\" NAME=\"Dubs &amp; Bass\"><SUBNODES COUNT=\"1\">\
<NODE TYPE=\"PLAYLIST\" NAME=\"Deep\"><PLAYLIST ENTRIES=\"1\" TYPE=\"LIST\" UUID=\"{}\">\
<ENTRY><PRIMARYKEY TYPE=\"TRACK\" KEY=\"Macintosh HD/:Users/:me/:a.mp3\"></PRIMARYKEY>\n</ENTRY>\n\
</PLAYLIST>\n</NODE>\n</SUBNODES>\n</NODE>\n\
<NODE TYPE=\"PLAYLIST\" NAME=\"Stems\"><PLAYLIST ENTRIES=\"1\" TYPE=\"LIST\" UUID=\"{}\">\
<ENTRY><PRIMARYKEY TYPE=\"STEM\" KEY=\"Gig/:x.stem.mp4\"></PRIMARYKEY>\n</ENTRY>\n\
</PLAYLIST>\n</NODE>\n\
<NODE TYPE=\"PLAYLIST\" NAME=\"Empty\"><PLAYLIST ENTRIES=\"0\" TYPE=\"LIST\" UUID=\"{}\"></PLAYLIST>\n</NODE>\n\
</SUBNODES>\n</NODE>\n",
            uuid(&["Overtone", "Dubs & Bass", "Deep"]),
            uuid(&["Overtone", "Stems"]),
            uuid(&["Overtone", "Empty"]),
        );
        assert_eq!(folder, expected);
    }

    #[test]
    fn uuids_are_stable_hex_and_distinct() {
        let path = vec!["Overtone".to_string(), "Dubs".to_string()];
        let a = playlist_uuid(&path, 0);
        assert_eq!(a, playlist_uuid(&path, 0));
        assert_eq!(a.len(), 32);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, playlist_uuid(&path, 1));
    }

    #[test]
    fn escapes_attribute_values() {
        assert_eq!(
            escape_attr("a&b<\"c\">\n"),
            "a&amp;b&lt;&quot;c&quot;&gt;&#xA;"
        );
    }
}
