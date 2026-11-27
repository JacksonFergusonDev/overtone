use std::path::PathBuf;

use clap::{Args, Subcommand};
use overtone_core::CoreError;
use overtone_core::fs::ProtectedRoots;
use overtone_core::library::Library;
use overtone_core::sync;
use overtone_sync_serato::{FileAction, SeratoSyncPlan};

use crate::output::{Output, table};

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Write vibes as Serato crates under a top-level "Overtone" crate.
    Serato(SeratoArgs),
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
