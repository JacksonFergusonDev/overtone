use std::path::PathBuf;

use clap::{Args, Subcommand};
use overtone_core::CoreError;
use overtone_core::fs::ProtectedRoots;
use overtone_core::library::Library;
use overtone_core::rekordbox_db::{self, RekordboxTagAction};
use overtone_core::sync;
use overtone_sync_serato::{FileAction, SeratoSyncPlan};

use crate::output::{Output, table};

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Write vibes as Serato crates under a top-level "Overtone" crate.
    Serato(SeratoArgs),
    /// Write vibes as a Rekordbox XML file under an "Overtone" playlist folder.
    Rekordbox(RekordboxArgs),
    /// Write vibes as My Tags in an "Overtone" category of Rekordbox's master.db.
    /// Rekordbox must be closed; master.db is backed up first.
    RekordboxTags(RekordboxTagsArgs),
}

#[derive(Debug, Args)]
pub struct RekordboxTagsArgs {
    /// Show what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Use this master.db instead of ~/Library/Pioneer/rekordbox/master.db.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct RekordboxArgs {
    /// Show what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Write the XML here instead of `rekordbox.xml` in Overtone's data folder.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct SeratoArgs {
    /// Show what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Sync into this `_Serato_` folder instead of each volume's own.
    #[arg(long, value_name = "PATH")]
    serato_dir: Option<PathBuf>,
}

pub fn run(library: &Library, command: &SyncCommand, out: &Output) -> Result<(), CoreError> {
    match command {
        SyncCommand::Serato(args) => serato(library, args, out),
        SyncCommand::Rekordbox(args) => rekordbox(library, args, out),
        SyncCommand::RekordboxTags(args) => rekordbox_tags(library, args, out),
    }
}

fn rekordbox_tags(
    library: &Library,
    args: &RekordboxTagsArgs,
    out: &Output,
) -> Result<(), CoreError> {
    if args.dry_run {
        let plan = rekordbox_db::plan_rekordbox_tags(library, args.db.as_deref())?;
        out.emit(&plan, || {
            println!("{}", plan.db_path.display());
            if plan.create_category {
                println!("  Would create the Overtone My Tag category.");
            }
            let changes: Vec<_> = plan
                .tags
                .iter()
                .filter(|t| t.action != RekordboxTagAction::Unchanged)
                .collect();
            if changes.is_empty() {
                println!("  Up to date.");
            } else {
                let mut t = table(&["My Tag", "Change", "Tracks"]);
                for tag in changes {
                    let change = match tag.action {
                        RekordboxTagAction::Create => "create".to_string(),
                        RekordboxTagAction::Update => format!("+{} / -{}", tag.added, tag.removed),
                        RekordboxTagAction::Delete => "delete".to_string(),
                        RekordboxTagAction::Unchanged => String::new(),
                    };
                    t.add_row(vec![tag.name.clone(), change, tag.track_count.to_string()]);
                }
                println!("{t}");
            }
            print_skipped(plan.skipped_tracks.len());
            if plan.rekordbox_running || plan.agent_running {
                println!("  Quit Rekordbox and rekordboxAgent before syncing.");
            }
            println!("Dry run: nothing was written.");
        });
        return Ok(());
    }
    let report =
        rekordbox_db::apply_rekordbox_tags(library, args.db.as_deref(), ProtectedRoots::global())?;
    out.emit(&report, || {
        println!("{}", report.db_path.display());
        if report.changed {
            println!(
                "  {} tag(s) created, {} deleted, {} track(s) added, {} removed.",
                report.created_tags.len(),
                report.deleted_tags.len(),
                report.added,
                report.removed
            );
        } else {
            println!("  Already up to date.");
        }
        print_skipped(report.skipped_tracks.len());
        if let Some(backup) = &report.backup_dir {
            println!("  Backup of the replaced database: {}", backup.display());
        }
    });
    Ok(())
}

fn print_skipped(count: usize) {
    if count > 0 {
        println!(
            "  {count} track(s) aren't in Rekordbox's collection and were skipped; import them in Rekordbox to tag them."
        );
    }
}

fn serato(library: &Library, args: &SeratoArgs, out: &Output) -> Result<(), CoreError> {
    let plans = sync::plan_serato(library, args.serato_dir.as_deref())?;
    if args.dry_run {
        out.emit(&plans, || print_plans(&plans));
        return Ok(());
    }
    let report = sync::apply_serato(library, &plans, ProtectedRoots::global())?;
    out.emit(&report, || {
        for folder in &report.folders {
            let r = &folder.report;
            println!(
                "{}: {} created, {} updated, {} deleted, {} unchanged",
                folder.serato_dir.display(),
                r.created.len(),
                r.updated.len(),
                r.deleted.len(),
                r.unchanged
            );
            if let Some(backup) = &r.backup_dir {
                println!("  Backup of replaced files: {}", backup.display());
            }
        }
        println!("Restart Serato DJ to see the changes.");
    });
    Ok(())
}

const IMPORT_HINT: &str = "In Rekordbox, open Preferences > Advanced > Database, set rekordbox xml > Imported Library to this file, then open the rekordbox xml view in the sidebar to import the playlists.";

fn rekordbox(library: &Library, args: &RekordboxArgs, out: &Output) -> Result<(), CoreError> {
    if args.dry_run {
        let plan = sync::plan_rekordbox_export(library, args.out.as_deref())?;
        out.emit(&plan, || {
            println!("{}", plan.path.display());
            println!(
                "  {} track(s) in {} playlist(s).",
                plan.track_count, plan.playlist_count
            );
            if plan.changed {
                println!(
                    "  Would {} the file.",
                    if plan.exists { "update" } else { "create" }
                );
            } else {
                println!("  Up to date.");
            }
            if !plan.playlists.is_empty() {
                let mut t = table(&["Playlist", "Tracks"]);
                for playlist in &plan.playlists {
                    t.add_row(vec![
                        playlist.path.join(" / "),
                        playlist.track_count.to_string(),
                    ]);
                }
                println!("{t}");
            }
            println!("Dry run: nothing was written.");
        });
        return Ok(());
    }
    let report =
        sync::export_rekordbox_xml(library, args.out.as_deref(), ProtectedRoots::global())?;
    out.emit(&report, || {
        println!("{}", report.path.display());
        println!(
            "  {} track(s) in {} playlist(s). {}",
            report.track_count,
            report.playlist_count,
            if report.changed {
                "Written."
            } else {
                "Already up to date."
            }
        );
        if let Some(backup) = &report.backup_path {
            println!("  Backup of the replaced file: {}", backup.display());
        }
        if report.rekordbox_running {
            println!("  Rekordbox is running: import the file again to see the changes.");
        }
        println!("{IMPORT_HINT}");
    });
    Ok(())
}

fn print_plans(plans: &[SeratoSyncPlan]) {
    for plan in plans {
        println!("{}", plan.serato_dir.display());
        let changes: Vec<_> = plan
            .files
            .iter()
            .filter(|f| f.action != FileAction::Unchanged)
            .collect();
        if changes.is_empty() {
            println!("  Up to date.");
        } else {
            let mut t = table(&["Crate", "Change", "Tracks"]);
            for file in changes {
                let change = match file.action {
                    FileAction::Create { .. } => "create".to_string(),
                    FileAction::Update { added, removed } => format!("+{added} / -{removed}"),
                    FileAction::Delete { .. } => "delete".to_string(),
                    FileAction::Unchanged => String::new(),
                };
                t.add_row(vec![
                    file.filename
                        .trim_end_matches(".crate")
                        .replace("%%", " / "),
                    change,
                    file.track_count.to_string(),
                ]);
            }
            println!("{t}");
        }
        for warning in &plan.warnings {
            println!("  note: {warning}");
        }
        if !plan.skipped_tracks.is_empty() {
            println!(
                "  {} track(s) live on another drive and sync to that drive's _Serato_ folder.",
                plan.skipped_tracks.len()
            );
        }
    }
    println!("Dry run: nothing was written.");
}
