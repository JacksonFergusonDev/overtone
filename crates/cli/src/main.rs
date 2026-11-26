//! Command-line companion for Overtone.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use overtone_core::CoreError;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if cli.json {
                let payload = serde_json::json!({
                    "error": { "code": err.code(), "message": err.to_string() }
                });
                println!("{payload}");
            } else {
                eprintln!("error: {err}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), CoreError> {
    match cli.command {
        Command::Status => {
            let (library, path) = overtone_core::open_library(cli.library.as_deref())?;
            let status = overtone_core::status(&library, &path)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status).expect("status serializes")
                );
            } else {
                println!("Overtone {}", status.version);
                println!("Library: {}", status.library_path);
                println!(
                    "Tracks: {} ({} missing)",
                    status.library.track_count, status.library.missing_count
                );
                println!("Vibes: {}", status.library.vibe_count);
                println!("Needs review: {}", status.library.needs_review_count);
            }
        }
    }
    Ok(())
}
