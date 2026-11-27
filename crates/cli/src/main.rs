//! Command-line companion for Overtone.
//!
//! Every command supports `--json`: stdout then carries only the JSON
//! payload (or `{"error": {...}}` on failure) and nothing decorative.

mod commands;
mod output;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use overtone_core::CoreError;

use crate::commands::{roots, scan, sync, tracks, vibes};
use crate::output::Output;

#[derive(Debug, Parser)]
#[command(
    name = "overtone-cli",
    version,
    about = "Manage your Overtone DJ library from the terminal"
)]
struct Cli {
    /// Emit machine-readable JSON on stdout and suppress decorative output.
    #[arg(long, global = true)]
    json: bool,

    /// Library database to use instead of the one shared with the desktop app.
    #[arg(long, global = true, value_name = "PATH", env = "OVERTONE_LIBRARY")]
    library: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show library status and version information.
    Status,
    /// Manage the folders Overtone indexes.
    #[command(subcommand)]
    Roots(roots::RootsCommand),
    /// Index audio files in your folders. Files are only read, never changed.
    Scan(scan::ScanArgs),
    /// List tracks.
    Ls(tracks::LsArgs),
    /// Show one track in detail.
    Show(tracks::ShowArgs),
    /// Create, rename, and assign vibes.
    #[command(subcommand)]
    Vibe(vibes::VibeCommand),
    /// Sync vibes to DJ software. Close the DJ app first.
    #[command(subcommand)]
    Sync(sync::SyncCommand),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Output::new(cli.json);
    match run(&cli, &out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            out.error(&err);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli, out: &Output) -> Result<(), CoreError> {
    let (mut library, path) = overtone_core::open_library(cli.library.as_deref())?;
    match &cli.command {
        Command::Status => {
            let status = overtone_core::status(&library, &path)?;
            out.emit(&status, || {
                println!("Overtone {}", status.version);
                println!("Library: {}", status.library_path);
                println!(
                    "Tracks: {} ({} missing)",
                    status.library.track_count, status.library.missing_count
                );
                println!("Vibes: {}", status.library.vibe_count);
                println!("Needs review: {}", status.library.needs_review_count);
            });
            Ok(())
        }
        Command::Roots(command) => roots::run(&library, command, out),
        Command::Scan(args) => scan::run(&mut library, args, out),
        Command::Ls(args) => tracks::ls(&library, args, out),
        Command::Show(args) => tracks::show(&library, args, out),
        Command::Vibe(command) => vibes::run(&mut library, command, out),
        Command::Sync(command) => sync::run(&library, command, out),
    }
}
