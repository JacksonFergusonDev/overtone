//! Serializes a [`RekordboxXml`] with `quick-xml`.

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};

use crate::PRODUCT_NAME;
use crate::model::{PlaylistNode, RekordboxXml, XmlTrack};
use crate::uri::location_uri;

/// Renders the document as indented UTF-8 XML.
///
/// Attribute values are escaped by `quick-xml`. Characters XML 1.0 can't
/// carry (most control characters) are dropped, and tabs and line breaks
/// become spaces, because XML readers normalize them inside attributes
/// anyway.
pub fn to_xml(doc: &RekordboxXml) -> String {
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    // Writing to a Vec<u8> can't fail.
    let _ = write_document(&mut writer, doc);
    String::from_utf8(writer.into_inner()).unwrap_or_default()
}

type Result = std::io::Result<()>;

fn write_document(w: &mut Writer<Vec<u8>>, doc: &RekordboxXml) -> Result {
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    let mut root = BytesStart::new("DJ_PLAYLISTS");
    root.push_attribute(("Version", "1.0.0"));
    w.write_event(Event::Start(root))?;

    let mut product = BytesStart::new("PRODUCT");
    product.push_attribute(("Name", PRODUCT_NAME));
    product.push_attribute(("Version", clean(&doc.product_version).as_str()));
    product.push_attribute(("Company", PRODUCT_NAME));
    w.write_event(Event::Empty(product))?;

    let mut collection = BytesStart::new("COLLECTION");
    collection.push_attribute(("Entries", doc.tracks.len().to_string().as_str()));
    if doc.tracks.is_empty() {
        w.write_event(Event::Empty(collection))?;
    } else {
        w.write_event(Event::Start(collection))?;
        for track in &doc.tracks {
            write_track(w, track)?;
        }
        w.write_event(Event::End(BytesEnd::new("COLLECTION")))?;
    }

    w.write_event(Event::Start(BytesStart::new("PLAYLISTS")))?;
    let mut node = BytesStart::new("NODE");
    node.push_attribute(("Type", "0"));
    node.push_attribute(("Name", "ROOT"));
    node.push_attribute(("Count", doc.playlists.len().to_string().as_str()));
    if doc.playlists.is_empty() {
        w.write_event(Event::Empty(node))?;
    } else {
        w.write_event(Event::Start(node))?;
        for child in &doc.playlists {
            write_node(w, child)?;
        }
        w.write_event(Event::End(BytesEnd::new("NODE")))?;
    }
    w.write_event(Event::End(BytesEnd::new("PLAYLISTS")))?;
    w.write_event(Event::End(BytesEnd::new("DJ_PLAYLISTS")))
}

fn write_track(w: &mut Writer<Vec<u8>>, track: &XmlTrack) -> Result {
    let mut e = BytesStart::new("TRACK");
    e.push_attribute(("TrackID", track.track_id.to_string().as_str()));
    let text = [
        ("Name", &track.name),
        ("Artist", &track.artist),
        ("Album", &track.album),
        ("Genre", &track.genre),
        ("Kind", &track.kind),
    ];
    for (key, value) in text {
        if let Some(value) = value {
            e.push_attribute((key, clean(value).as_str()));
        }
    }
    let numbers = [
        ("Size", track.size_bytes.map(|v| v.to_string())),
        ("TotalTime", track.total_time_s.map(|v| v.to_string())),
        ("Year", track.year.map(|v| v.to_string())),
        ("AverageBpm", track.average_bpm.map(|v| format!("{v:.2}"))),
        ("DateAdded", track.date_added.clone()),
        ("BitRate", track.bit_rate_kbps.map(|v| v.to_string())),
        ("SampleRate", track.sample_rate_hz.map(|v| v.to_string())),
    ];
    for (key, value) in numbers {
        if let Some(value) = value {
            e.push_attribute((key, clean(&value).as_str()));
        }
    }
    let more = [
        ("Comments", &track.comments),
        ("Remixer", &track.remixer),
        ("Tonality", &track.tonality),
        ("Label", &track.label),
    ];
    e.push_attribute(("Location", clean(&location_uri(&track.location)).as_str()));
    for (key, value) in more {
        if let Some(value) = value {
            e.push_attribute((key, clean(value).as_str()));
        }
    }
    w.write_event(Event::Empty(e))
}

fn write_node(w: &mut Writer<Vec<u8>>, node: &PlaylistNode) -> Result {
    let mut e = BytesStart::new("NODE");
    match node {
        PlaylistNode::Folder { name, children } => {
            e.push_attribute(("Type", "0"));
            e.push_attribute(("Name", clean(name).as_str()));
            e.push_attribute(("Count", children.len().to_string().as_str()));
            if children.is_empty() {
                return w.write_event(Event::Empty(e));
            }
            w.write_event(Event::Start(e))?;
            for child in children {
                write_node(w, child)?;
            }
        }
        PlaylistNode::Playlist { name, track_ids } => {
            e.push_attribute(("Type", "1"));
            e.push_attribute(("Name", clean(name).as_str()));
            e.push_attribute(("KeyType", "0"));
            e.push_attribute(("Entries", track_ids.len().to_string().as_str()));
            if track_ids.is_empty() {
                return w.write_event(Event::Empty(e));
            }
            w.write_event(Event::Start(e))?;
            for id in track_ids {
                let mut key = BytesStart::new("TRACK");
                key.push_attribute(("Key", id.to_string().as_str()));
                w.write_event(Event::Empty(key))?;
            }
        }
    }
    w.write_event(Event::End(BytesEnd::new("NODE")))
}

/// Drops characters XML 1.0 can't represent and flattens whitespace
/// controls to spaces.
fn clean(value: &str) -> String {
    value
        .chars()
        .filter_map(|c| match c {
            '\t' | '\n' | '\r' => Some(' '),
            c if c < ' ' || c == '\u{7f}' || c == '\u{fffe}' || c == '\u{ffff}' => None,
            c => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::parse_xml;

    fn track(id: u32, path: &str) -> XmlTrack {
        XmlTrack {
            track_id: id,
            location: PathBuf::from(path),
            ..XmlTrack::default()
        }
    }

    fn sample() -> RekordboxXml {
        RekordboxXml {
            product_version: "0.1.0".into(),
            tracks: vec![
                XmlTrack {
                    name: Some("Dub \"One\" <A&B>".into()),
                    artist: Some("Artist".into()),
                    album: Some("Album".into()),
                    genre: Some("Dub".into()),
                    kind: Some("FLAC File".into()),
                    size_bytes: Some(12_345),
                    total_time_s: Some(301),
                    average_bpm: Some(140.5),
                    tonality: Some("8A".into()),
                    bit_rate_kbps: Some(1411),
                    sample_rate_hz: Some(44_100),
                    comments: Some("it's fine".into()),
                    remixer: Some("Remixer".into()),
                    label: Some("Label".into()),
                    year: Some(2024),
                    date_added: Some("2026-11-27".into()),
                    ..track(1, "/Music/My Tune (v2) #1.flac")
                },
                track(2, "/Music/e\u{301}.mp3"),
            ],
            playlists: vec![PlaylistNode::Folder {
                name: "Overtone".into(),
                children: vec![
                    PlaylistNode::Playlist {
                        name: "Warmup".into(),
                        track_ids: vec![1, 2],
                    },
                    PlaylistNode::Folder {
                        name: "Dubs".into(),
                        children: vec![
                            PlaylistNode::Playlist {
                                name: "Dubs".into(),
                                track_ids: vec![2],
                            },
                            PlaylistNode::Playlist {
                                name: "Deep".into(),
                                track_ids: vec![],
                            },
                        ],
                    },
                ],
            }],
        }
    }

    #[test]
    fn writes_the_expected_structure() {
        let xml = to_xml(&sample());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<DJ_PLAYLISTS Version=\"1.0.0\">"));
        assert!(
            xml.contains("<PRODUCT Name=\"Overtone\" Version=\"0.1.0\" Company=\"Overtone\"/>")
        );
        assert!(xml.contains("<COLLECTION Entries=\"2\">"));
        assert!(xml.contains("<NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">"));
        assert!(xml.contains("<NODE Type=\"0\" Name=\"Overtone\" Count=\"2\">"));
        assert!(xml.contains("<NODE Type=\"0\" Name=\"Dubs\" Count=\"2\">"));
        assert!(xml.contains("<NODE Type=\"1\" Name=\"Warmup\" KeyType=\"0\" Entries=\"2\">"));
        assert!(xml.contains("<NODE Type=\"1\" Name=\"Deep\" KeyType=\"0\" Entries=\"0\"/>"));
        assert!(xml.contains("<TRACK Key=\"2\"/>"));
        assert!(xml.contains("\n  <PRODUCT"), "indented: {xml}");
    }

    #[test]
    fn escapes_markup_in_attributes() {
        let xml = to_xml(&sample());
        assert!(
            xml.contains("Name=\"Dub &quot;One&quot; &lt;A&amp;B&gt;\""),
            "{xml}"
        );
        assert!(!xml.contains("<A&B>"));
    }

    #[test]
    fn leaves_unset_attributes_out() {
        let xml = to_xml(&sample());
        let bare = xml
            .lines()
            .find(|line| line.contains("TrackID=\"2\""))
            .expect("track 2");
        for absent in [
            "Name=",
            "Artist=",
            "AverageBpm=",
            "Tonality=",
            "Year=",
            "Size=",
        ] {
            assert!(!bare.contains(absent), "{absent} in {bare}");
        }
        assert!(bare.contains("Location=\"file://localhost/Music/e%CC%81.mp3\""));
    }

    #[test]
    fn strips_control_characters() {
        let mut doc = sample();
        doc.tracks[1].name = Some("a\u{0}b\u{1b}c\td\ne\u{fffe}".into());
        let xml = to_xml(&doc);
        assert!(xml.contains("Name=\"abc d e\""), "{xml}");
        assert!(parse_xml(&xml).is_ok());
    }

    #[test]
    fn round_trips_through_the_reader() {
        let doc = sample();
        assert_eq!(parse_xml(&to_xml(&doc)).expect("parse"), doc);
    }

    #[test]
    fn empty_document_round_trips() {
        let doc = RekordboxXml {
            product_version: "1".into(),
            ..RekordboxXml::default()
        };
        let xml = to_xml(&doc);
        assert!(xml.contains("<COLLECTION Entries=\"0\"/>"));
        assert_eq!(parse_xml(&xml).expect("parse"), doc);
    }
}
