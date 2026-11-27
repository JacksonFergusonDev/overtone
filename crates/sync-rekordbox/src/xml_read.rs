//! A minimal reader for the files [`crate::to_xml`] writes, enough to
//! diff an export against the previous one and to test round trips. It
//! ignores elements and attributes it doesn't model.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::RekordboxError;
use crate::model::{PlaylistNode, RekordboxXml, XmlTrack};
use crate::uri::location_from_uri;

struct OpenNode {
    name: String,
    is_playlist: bool,
    children: Vec<PlaylistNode>,
    track_ids: Vec<u32>,
}

impl OpenNode {
    fn finish(self) -> PlaylistNode {
        if self.is_playlist {
            PlaylistNode::Playlist {
                name: self.name,
                track_ids: self.track_ids,
            }
        } else {
            PlaylistNode::Folder {
                name: self.name,
                children: self.children,
            }
        }
    }
}

fn xml_err(err: impl std::fmt::Display) -> RekordboxError {
    RekordboxError::Xml(err.to_string())
}

fn attrs(e: &BytesStart<'_>) -> Result<Vec<(String, String)>, RekordboxError> {
    e.attributes()
        .map(|attr| {
            let attr = attr.map_err(xml_err)?;
            let key = attr.key.as_ref().to_string();
            let value = attr
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map_err(xml_err)?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}

fn get<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn parse_num<T: std::str::FromStr>(attrs: &[(String, String)], key: &str) -> Option<T> {
    get(attrs, key).and_then(|v| v.parse().ok())
}

fn text(attrs: &[(String, String)], key: &str) -> Option<String> {
    get(attrs, key).map(str::to_string)
}

fn track_from(attrs: &[(String, String)]) -> Result<XmlTrack, RekordboxError> {
    let track_id = parse_num(attrs, "TrackID")
        .ok_or_else(|| RekordboxError::Xml("TRACK without a TrackID".into()))?;
    let location = get(attrs, "Location")
        .and_then(location_from_uri)
        .ok_or_else(|| RekordboxError::Xml(format!("TRACK {track_id} has no usable Location")))?;
    Ok(XmlTrack {
        track_id,
        location,
        name: text(attrs, "Name"),
        artist: text(attrs, "Artist"),
        album: text(attrs, "Album"),
        genre: text(attrs, "Genre"),
        kind: text(attrs, "Kind"),
        size_bytes: parse_num(attrs, "Size"),
        total_time_s: parse_num(attrs, "TotalTime"),
        average_bpm: parse_num(attrs, "AverageBpm"),
        tonality: text(attrs, "Tonality"),
        bit_rate_kbps: parse_num(attrs, "BitRate"),
        sample_rate_hz: parse_num(attrs, "SampleRate"),
        comments: text(attrs, "Comments"),
        remixer: text(attrs, "Remixer"),
        label: text(attrs, "Label"),
        year: parse_num(attrs, "Year"),
        date_added: text(attrs, "DateAdded"),
    })
}

/// Parses a `rekordbox.xml` written by Overtone (or any file of the same
/// shape). Playlist nodes outside `ROOT` are ignored.
pub fn parse_xml(s: &str) -> Result<RekordboxXml, RekordboxError> {
    let mut reader = Reader::from_str(s);
    let mut doc = RekordboxXml::default();
    let mut in_collection = false;
    let mut in_playlists = false;
    let mut stack: Vec<OpenNode> = Vec::new();
    let mut saw_root = false;

    loop {
        let event = reader.read_event().map_err(xml_err)?;
        let (element, is_empty) = match &event {
            Event::Start(e) => (Some(e), false),
            Event::Empty(e) => (Some(e), true),
            Event::Eof => break,
            Event::End(e) => {
                match e.name().as_ref() {
                    "COLLECTION" => in_collection = false,
                    "PLAYLISTS" => in_playlists = false,
                    "NODE" => close_node(&mut stack, &mut doc),
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };
        let Some(e) = element else { continue };
        let a = attrs(e)?;
        match e.name().as_ref() {
            "DJ_PLAYLISTS" => saw_root = true,
            "PRODUCT" => doc.product_version = get(&a, "Version").unwrap_or_default().to_string(),
            "COLLECTION" => in_collection = !is_empty,
            "PLAYLISTS" => in_playlists = !is_empty,
            "TRACK" if in_collection => doc.tracks.push(track_from(&a)?),
            "TRACK" if in_playlists => {
                let id = parse_num(&a, "Key")
                    .ok_or_else(|| RekordboxError::Xml("playlist TRACK without a Key".into()))?;
                if let Some(open) = stack.last_mut() {
                    open.track_ids.push(id);
                }
            }
            "NODE" if in_playlists => {
                stack.push(OpenNode {
                    name: get(&a, "Name").unwrap_or_default().to_string(),
                    is_playlist: get(&a, "Type") == Some("1"),
                    children: Vec::new(),
                    track_ids: Vec::new(),
                });
                if is_empty {
                    close_node(&mut stack, &mut doc);
                }
            }
            _ => {}
        }
    }
    if !saw_root {
        return Err(RekordboxError::Xml("no DJ_PLAYLISTS element".into()));
    }
    Ok(doc)
}

/// Finishes the innermost open node. The outermost one is `ROOT`, whose
/// children become the document's playlists.
fn close_node(stack: &mut Vec<OpenNode>, doc: &mut RekordboxXml) {
    let Some(open) = stack.pop() else { return };
    if stack.is_empty() {
        doc.playlists = open.children;
    } else if let Some(parent) = stack.last_mut() {
        parent.children.push(open.finish());
    }
}
