use clap::Subcommand;
use overtone_core::CoreError;
use overtone_core::library::Library;
use overtone_core::model::TrackId;

use super::{find_vibe, parse_vibe, resolve_track};
use crate::output::{Output, table};

#[derive(Debug, Subcommand)]
pub enum VibeCommand {
    /// List vibes with their track counts.
    Ls,
    /// Create a vibe. Nested vibes use `/`, for example `Dubs/Deep`.
    Create { vibe: String },
    /// Rename or move a vibe; vibes nested inside it move too.
    Rename { vibe: String, new_name: String },
    /// Delete a vibe and the vibes nested inside it. Tracks are untouched.
    Delete { vibe: String },
    /// Add tracks (ids or paths) to a vibe, creating the vibe if needed.
    Add {
        vibe: String,
        #[arg(required = true)]
        tracks: Vec<String>,
    },
    /// Remove tracks (ids or paths) from a vibe.
    Rm {
        vibe: String,
        #[arg(required = true)]
        tracks: Vec<String>,
    },
}

pub fn run(library: &mut Library, command: &VibeCommand, out: &Output) -> Result<(), CoreError> {
    match command {
        VibeCommand::Ls => {
            let vibes = library.vibes()?;
            let collisions = library.hashtag_prefix_collisions()?;
            out.emit(&vibes, || {
                if vibes.is_empty() {
                    println!("No vibes yet. Create one with `overtone-cli vibe create <name>`.");
                    return;
                }
                let mut t = table(&["Vibe", "Tracks", "Hashtag"]);
                for vibe in &vibes {
                    let indent = "  ".repeat(vibe.path.depth() - 1);
                    t.add_row(vec![
                        format!("{indent}{}", vibe.path.name()),
                        vibe.track_count.to_string(),
                        vibe.path.hashtag(),
                    ]);
                }
                println!("{t}");
                for (short, long) in &collisions {
                    println!(
                        "note: a Comment rule matching {} also matches {}",
                        short.hashtag(),
                        long.hashtag()
                    );
                }
            });
        }
        VibeCommand::Create { vibe } => {
            let created = library.create_vibe(&parse_vibe(vibe)?)?;
            out.emit(&created, || println!("Vibe {}", created.path));
        }
        VibeCommand::Rename { vibe, new_name } => {
            let existing = find_vibe(library, &parse_vibe(vibe)?)?;
            let renamed = library.rename_vibe(existing.id, &parse_vibe(new_name)?)?;
            out.emit(&renamed, || {
                println!("Renamed {} to {}", existing.path, renamed.path);
            });
        }
        VibeCommand::Delete { vibe } => {
            let existing = find_vibe(library, &parse_vibe(vibe)?)?;
            library.delete_vibe(existing.id)?;
            out.emit(&existing, || {
                println!("Deleted {} (tracks are untouched)", existing.path);
            });
        }
        VibeCommand::Add { vibe, tracks } => {
            let target = library.create_vibe(&parse_vibe(vibe)?)?;
            let ids = resolve_all(library, tracks)?;
            library.assign_vibe(&ids, target.id)?;
            let updated = library.vibe(target.id)?;
            out.emit(&updated, || {
                println!(
                    "Added {} track(s) to {} ({} total)",
                    ids.len(),
                    updated.path,
                    updated.track_count
                );
            });
        }
        VibeCommand::Rm { vibe, tracks } => {
            let target = find_vibe(library, &parse_vibe(vibe)?)?;
            let ids = resolve_all(library, tracks)?;
            library.unassign_vibe(&ids, target.id)?;
            let updated = library.vibe(target.id)?;
            out.emit(&updated, || {
                println!(
                    "Removed {} track(s) from {} ({} left)",
                    ids.len(),
                    updated.path,
                    updated.track_count
                );
            });
        }
    }
    Ok(())
}

fn resolve_all(library: &Library, selectors: &[String]) -> Result<Vec<TrackId>, CoreError> {
    selectors
        .iter()
        .map(|s| resolve_track(library, s).map(|t| t.id))
        .collect()
}
