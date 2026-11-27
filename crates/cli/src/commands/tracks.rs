use std::collections::HashMap;

use clap::Args;
use overtone_core::CoreError;
use overtone_core::library::Library;
use overtone_core::model::{AudioFormat, QualityVerdict, ReviewStatus, Track, Vibe, VibeId};
use serde::Serialize;

use super::{find_vibe, parse_vibe, resolve_track};
use crate::output::{Output, or_blank, table};

#[derive(Debug, Args)]
pub struct LsArgs {
    /// Only tracks directly in this vibe (for example `Dubs/Deep`).
    #[arg(long)]
    vibe: Option<String>,
    /// Only tracks whose files have gone missing.
    #[arg(long, conflicts_with = "needs_review")]
    missing: bool,
    /// Only tracks flagged by quality analysis and not reviewed yet.
    #[arg(long)]
    needs_review: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Track id or file path.
    track: String,
}

pub fn ls(library: &Library, args: &LsArgs, out: &Output) -> Result<(), CoreError> {
    let vibe_filter = args
        .vibe
        .as_deref()
        .map(|name| find_vibe(library, &parse_vibe(name)?))
        .transpose()?;

    let tracks: Vec<Track> = library
        .tracks()?
        .into_iter()
        .filter(|t| !args.missing || t.missing_since.is_some())
        .filter(|t| args.missing || t.missing_since.is_none())
        .filter(|t| !args.needs_review || needs_review(t))
        .filter(|t| vibe_filter.as_ref().is_none_or(|v| t.vibes.contains(&v.id)))
        .collect();

    let names = vibe_names(library)?;
    out.emit(&tracks, || {
        if tracks.is_empty() {
            println!("No tracks match.");
            return;
        }
        let mut t = table(&[
            "ID", "Artist", "Title", "BPM", "Key", "Format", "Quality", "Vibes",
        ]);
        for track in &tracks {
            t.add_row(vec![
                track.id.0.to_string(),
                or_blank(track.tags.artist.as_deref()),
                or_blank(track.tags.title.as_deref()).or_file_name(&track.path),
                or_blank(track.tags.bpm.map(|bpm| format!("{bpm:.0}"))),
                or_blank(track.tags.key.as_deref()),
                or_blank(track.properties.format.map(AudioFormat::as_str)),
                quality_label(track),
                vibe_list(&track.vibes, &names),
            ]);
        }
        println!("{t}");
        println!("{} tracks", tracks.len());
    });
    Ok(())
}

#[derive(Serialize)]
struct ShowOutput<'a> {
    track: &'a Track,
    vibes: Vec<&'a Vibe>,
    duplicates: Vec<i64>,
}

pub fn show(library: &Library, args: &ShowArgs, out: &Output) -> Result<(), CoreError> {
    let track = resolve_track(library, &args.track)?;
    let all_vibes = library.vibes()?;
    let vibes: Vec<&Vibe> = all_vibes
        .iter()
        .filter(|v| track.vibes.contains(&v.id))
        .collect();
    let duplicates: Vec<i64> = library
        .duplicates_of(track.id)?
        .into_iter()
        .map(|id| id.0)
        .collect();
    let payload = ShowOutput {
        track: &track,
        vibes,
        duplicates,
    };
    out.emit(&payload, || {
        let t = &track.tags;
        let p = &track.properties;
        println!("{}", track.path);
        if track.missing_since.is_some() {
            println!("  (file is missing)");
        }
        let rows: [(&str, String); 12] = [
            ("ID", track.id.0.to_string()),
            ("Title", or_blank(t.title.as_deref())),
            ("Artist", or_blank(t.artist.as_deref())),
            ("Album", or_blank(t.album.as_deref())),
            ("Genre", or_blank(t.genre.as_deref())),
            ("BPM", or_blank(t.bpm)),
            ("Key", or_blank(t.key.as_deref())),
            ("Format", or_blank(p.format.map(AudioFormat::as_str))),
            (
                "Sample rate",
                or_blank(p.sample_rate_hz.map(|hz| format!("{hz} Hz"))),
            ),
            ("Bit depth", or_blank(p.bit_depth)),
            (
                "Bitrate",
                or_blank(p.bitrate_kbps.map(|k| format!("{k} kbps"))),
            ),
            ("Quality", quality_label(&track)),
        ];
        for (label, value) in rows {
            if !value.is_empty() {
                println!("  {label:<12} {value}");
            }
        }
        if let Some(reason) = track.quality.as_ref().and_then(|q| q.reason.as_deref()) {
            println!("  {:<12} {reason}", "Why");
        }
        if !payload.vibes.is_empty() {
            let names: Vec<String> = payload.vibes.iter().map(|v| v.path.to_string()).collect();
            println!("  {:<12} {}", "Vibes", names.join(", "));
        }
        if !payload.duplicates.is_empty() {
            let ids: Vec<String> = payload.duplicates.iter().map(i64::to_string).collect();
            println!(
                "  {:<12} same audio as track {}",
                "Duplicates",
                ids.join(", ")
            );
        }
    });
    Ok(())
}

fn needs_review(track: &Track) -> bool {
    track
        .quality
        .as_ref()
        .is_some_and(|q| q.verdict.needs_review() && q.review == ReviewStatus::Unreviewed)
}

fn quality_label(track: &Track) -> String {
    let Some(q) = &track.quality else {
        return String::new();
    };
    let verdict = match q.verdict {
        QualityVerdict::Unchecked => "unchecked",
        QualityVerdict::Pass => "pass",
        QualityVerdict::Warning => "warning",
        QualityVerdict::Suspect => "suspect",
        QualityVerdict::Inconclusive => "inconclusive",
    };
    match q.review {
        ReviewStatus::Unreviewed => verdict.to_string(),
        ReviewStatus::VerifiedOk => format!("{verdict} (verified OK)"),
        ReviewStatus::ReplaceWanted => format!("{verdict} (replace)"),
    }
}

fn vibe_names(library: &Library) -> Result<HashMap<VibeId, String>, CoreError> {
    Ok(library
        .vibes()?
        .into_iter()
        .map(|v| (v.id, v.path.to_string()))
        .collect())
}

fn vibe_list(ids: &[VibeId], names: &HashMap<VibeId, String>) -> String {
    ids.iter()
        .filter_map(|id| names.get(id).cloned())
        .collect::<Vec<_>>()
        .join(", ")
}

trait OrFileName {
    fn or_file_name(self, path: &str) -> String;
}

impl OrFileName for String {
    /// Falls back to the file name when a track has no title tag.
    fn or_file_name(self, path: &str) -> String {
        if !self.is_empty() {
            return self;
        }
        std::path::Path::new(path)
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }
}
