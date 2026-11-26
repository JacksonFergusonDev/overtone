//! Command-line companion for Overtone.

use anyhow::Result;
use clap::{Parser, Subcommand};
use overtone_library::Library;

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

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show library status and version information.
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status => {
            let library = Library::open_in_memory()?;
            let status = overtone_core::status(&library)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "Overtone {} (tracks: {}, vibes: {})",
                    status.version, status.library.track_count, status.library.vibe_count
                );
            }
        }
    }
    Ok(())
}
