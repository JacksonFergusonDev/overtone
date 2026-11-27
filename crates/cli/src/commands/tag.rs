use clap::{Args, Subcommand};
use overtone_core::CoreError;
use overtone_core::fs::ProtectedRoots;
use overtone_core::library::Library;
use overtone_core::model::TrackId;
use overtone_core::tags::{self, MirrorReport};

use super::{find_vibe, parse_vibe, resolve_track};
use crate::output::Output;

#[derive(Debug, Subcommand)]
pub enum TagCommand {
    /// Write each track's vibes as hashtags into its Comment tag. Other text
    /// in the Comment is kept. Previews by default; pass --write to apply.
    Mirror(Selection),
    /// Put back the Comment each track had before Overtone first changed it.
    Restore(Selection),
}

#[derive(Debug, Args)]
pub struct Selection {
    /// Tracks (ids or paths). Without any, uses every track with a vibe.
    tracks: Vec<String>,
    /// Only tracks in this vibe.
    #[arg(long)]
    vibe: Option<String>,
    /// Actually write the files. Without it, only shows what would change.
    #[arg(long)]
    write: bool,
}

pub fn run(library: &mut Library, command: &TagCommand, out: &Output) -> Result<(), CoreError> {
    match command {
        TagCommand::Mirror(selection) => {
            let ids = select(library, selection, false)?;
            let changes = tags::plan_vibe_mirroring(library, &ids)?;
            if !selection.write {
                out.emit(&changes, || {
                    if changes.is_empty() {
                        println!("Every Comment already matches its vibes.");
                        return;
                    }
                    for change in &changes {
                        println!("{}", change.path);
                        println!("  before: {}", change.before.as_deref().unwrap_or(""));
                        println!("  after:  {}", change.after.as_deref().unwrap_or(""));
                    }
                    println!(
                        "{} file(s) would change. Nothing was written; rerun with --write to apply.",
                        changes.len()
                    );
                });
                return Ok(());
            }
            let report = tags::apply_vibe_mirroring(library, &changes, ProtectedRoots::global());
            print_report(out, &report, "Wrote");
        }
        TagCommand::Restore(selection) => {
            let ids = select(library, selection, true)?;
            if !selection.write {
                out.emit(&ids, || {
                    println!(
                        "{} file(s) have an original Comment saved. Nothing was written; rerun \
                         with --write to restore them.",
                        ids.len()
                    );
                });
                return Ok(());
            }
            let report = tags::restore_original_comments(library, &ids, ProtectedRoots::global());
            print_report(out, &report, "Restored");
        }
    }
    Ok(())
}

fn select(
    library: &Library,
    selection: &Selection,
    with_snapshot: bool,
) -> Result<Vec<TrackId>, CoreError> {
    let mut ids: Vec<TrackId> = if selection.tracks.is_empty() {
        if with_snapshot {
            library.tracks_with_tag_snapshot(overtone_core::library::COMMENT_FIELD)?
        } else {
            library
                .tracks()?
                .into_iter()
                .filter(|t| t.missing_since.is_none())
                .map(|t| t.id)
                .collect()
        }
    } else {
        selection
            .tracks
            .iter()
            .map(|s| resolve_track(library, s).map(|t| t.id))
            .collect::<Result<_, _>>()?
    };
    if let Some(name) = &selection.vibe {
        let vibe = find_vibe(library, &parse_vibe(name)?)?;
        let members = library.tracks_in_vibe(vibe.id)?;
        ids.retain(|id| members.contains(id));
    }
    Ok(ids)
}

fn print_report(out: &Output, report: &MirrorReport, verb: &str) {
    out.emit(report, || {
        println!(
            "{verb} {} file(s); {} already matched; {} not written.",
            report.written.len(),
            report.unchanged.len(),
            report.failed.len()
        );
        for failure in &report.failed {
            println!("  {}: {}", failure.path, failure.message);
        }
        if !report.failed.is_empty() {
            println!("Files that weren't written are unchanged.");
        }
    });
}
