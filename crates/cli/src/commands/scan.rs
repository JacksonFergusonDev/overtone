use std::path::PathBuf;
use std::sync::Mutex;

use clap::Args;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use overtone_core::CoreError;
use overtone_core::library::Library;
use overtone_core::scan::{self, ScanOptions, ScanProgress};

use crate::output::Output;

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Folders to add and scan. Without any, scans every indexed folder.
    paths: Vec<PathBuf>,
    /// Reread tags of every file, not just new or changed ones.
    #[arg(long)]
    refresh: bool,
}

pub fn run(library: &mut Library, args: &ScanArgs, out: &Output) -> Result<(), CoreError> {
    let roots: Vec<PathBuf> = if args.paths.is_empty() {
        Vec::new()
    } else {
        args.paths
            .iter()
            .map(|path| scan::add_root(library, path).map(|root| PathBuf::from(root.path)))
            .collect::<Result<_, _>>()?
    };

    let bar = if out.is_json() {
        ProgressBar::with_draw_target(None, ProgressDrawTarget::hidden())
    } else {
        ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr())
    };
    bar.set_style(
        ProgressStyle::with_template("{spinner} {msg} {wide_bar} {pos}/{len}")
            .expect("valid template"),
    );
    let bar = Mutex::new(bar);
    let on_progress = |progress: ScanProgress| {
        let Ok(bar) = bar.lock() else { return };
        match progress {
            ScanProgress::Walking { found } => {
                bar.set_message(format!("Finding audio files ({found})"));
                bar.tick();
            }
            ScanProgress::Processing { done, total } => {
                bar.set_message("Reading tags and hashing");
                bar.set_length(total);
                bar.set_position(done);
            }
            ScanProgress::Saving { done, total } => {
                bar.set_message("Saving");
                bar.set_length(total);
                bar.set_position(done);
            }
        }
    };

    let options = ScanOptions {
        refresh: args.refresh,
    };
    let report = if roots.is_empty() {
        scan::scan_library_with(library, options, &on_progress)?
    } else {
        scan::scan_roots_with(library, &roots, options, &on_progress)?
    };
    if let Ok(bar) = bar.lock() {
        bar.finish_and_clear();
    }

    out.emit(&report, || {
        println!(
            "{} added, {} updated, {} unchanged, {} missing ({:.1} s)",
            report.added,
            report.updated,
            report.unchanged,
            report.missing,
            report.elapsed_ms as f64 / 1000.0
        );
        for root in &report.unavailable_roots {
            println!("  Not found (drive unplugged?): {root}. Its tracks were left as they were.");
        }
        if !report.failed.is_empty() {
            println!("  {} file(s) had problems:", report.failed.len());
            for failure in report.failed.iter().take(20) {
                println!("    {}: {}", failure.path, failure.reason);
            }
            if report.failed.len() > 20 {
                println!(
                    "    … and {} more (use --json for all)",
                    report.failed.len() - 20
                );
            }
        }
    });
    Ok(())
}
