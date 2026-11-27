use std::path::PathBuf;
use std::sync::Mutex;

use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use overtone_audio::analysis::{AnalysisOptions, SpectrogramOptions, analyze};
use overtone_core::CoreError;
use overtone_core::audit::{self, AuditOptions, AuditProgress, DEFAULT_AUDIT_THREADS};
use overtone_core::library::Library;
use overtone_core::model::{ReviewStatus, TrackId};

use super::resolve_track;
use crate::output::Output;

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Tracks to analyze (ids or paths). Without any, analyzes tracks that
    /// haven't been analyzed by this version of Overtone.
    tracks: Vec<String>,
    /// Reanalyze every track, not just pending ones.
    #[arg(long, conflicts_with = "tracks")]
    all: bool,
    /// Tracks to analyze at once. More is faster but heats a fanless Mac.
    #[arg(long, default_value_t = DEFAULT_AUDIT_THREADS)]
    threads: usize,
}

pub fn audit(library: &mut Library, args: &AuditArgs, out: &Output) -> Result<(), CoreError> {
    let ids: Vec<TrackId> = if args.all {
        library
            .tracks()?
            .into_iter()
            .filter(|t| t.missing_since.is_none())
            .map(|t| t.id)
            .collect()
    } else if args.tracks.is_empty() {
        library.tracks_needing_analysis(overtone_audio::analysis::ANALYZER_VERSION)?
    } else {
        args.tracks
            .iter()
            .map(|s| resolve_track(library, s).map(|t| t.id))
            .collect::<Result<_, _>>()?
    };

    let bar = if out.is_json() {
        ProgressBar::hidden()
    } else {
        ProgressBar::with_draw_target(Some(ids.len() as u64), ProgressDrawTarget::stderr())
    };
    bar.set_style(
        ProgressStyle::with_template("Analyzing {wide_bar} {pos}/{len} {eta}")
            .expect("valid template"),
    );
    let bar = Mutex::new(bar);
    let on_progress = |progress: AuditProgress| {
        if let Ok(bar) = bar.lock() {
            bar.set_position(u64::from(progress.done));
        }
    };
    let options = AuditOptions {
        threads: args.threads.max(1),
    };
    let report = audit::audit_tracks_with(library, &ids, options, &on_progress)?;
    if let Ok(bar) = bar.lock() {
        bar.finish_and_clear();
    }

    out.emit(&report, || {
        let v = &report.verdicts;
        println!(
            "Analyzed {} track(s) in {:.1} s: {} pass, {} warning, {} suspect, {} inconclusive, {} couldn't be read",
            report.analyzed,
            report.elapsed_ms as f64 / 1000.0,
            v.pass,
            v.warning,
            v.suspect,
            v.inconclusive,
            v.unchecked
        );
        if !report.flagged.is_empty() {
            println!(
                "{} flagged for review. Nothing was deleted or moved; listen and look with \
                 `overtone-cli ls --needs-review` and `overtone-cli spectrogram <id>`.",
                report.flagged.len()
            );
        }
    });
    Ok(())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReviewChoice {
    /// You listened and looked: the flag was a false positive.
    Ok,
    /// Confirmed bad; you want a better copy. The file stays where it is.
    Replace,
    /// Forget your earlier review.
    Clear,
}

#[derive(Debug, Args)]
pub struct ReviewArgs {
    /// Track id or path.
    track: String,
    decision: ReviewChoice,
}

pub fn review(library: &Library, args: &ReviewArgs, out: &Output) -> Result<(), CoreError> {
    let track = resolve_track(library, &args.track)?;
    let status = match args.decision {
        ReviewChoice::Ok => ReviewStatus::VerifiedOk,
        ReviewChoice::Replace => ReviewStatus::ReplaceWanted,
        ReviewChoice::Clear => ReviewStatus::Unreviewed,
    };
    library.set_review(track.id, status)?;
    let updated = library.track(track.id)?;
    out.emit(&updated, || {
        println!(
            "Marked {} as {}",
            updated.path,
            status.as_str().replace('_', " ")
        );
    });
    Ok(())
}

#[derive(Debug, Args)]
pub struct SpectrogramArgs {
    /// Track id or path.
    track: String,
    /// Where to write the PNG. Defaults to `<track id>.png` in the current folder.
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Use a logarithmic frequency axis.
    #[arg(long)]
    log: bool,
}

pub fn spectrogram(
    library: &Library,
    args: &SpectrogramArgs,
    out: &Output,
) -> Result<(), CoreError> {
    let track = resolve_track(library, &args.track)?;
    let png = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.png", track.id.0)));
    let mut options = SpectrogramOptions::new(png.clone());
    options.log_frequency = args.log;
    let analysis = analyze(
        std::path::Path::new(&track.path),
        &AnalysisOptions {
            spectrogram: Some(options),
        },
    )?;
    let payload = serde_json::json!({
        "track_id": track.id,
        "png": png,
        "report": analysis.report,
    });
    out.emit(&payload, || {
        println!("Wrote {}", png.display());
        if let Some(reason) = &analysis.report.reason {
            println!("{}: {reason}", analysis.report.verdict.as_str());
        }
    });
    Ok(())
}
