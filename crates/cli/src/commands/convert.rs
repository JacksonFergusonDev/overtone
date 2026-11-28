use clap::Args;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use overtone_core::CoreError;
use overtone_core::convert::{self, ConvertOptions};
use overtone_core::library::{Library, LibraryError};
use overtone_core::model::{AudioFormat, ConversionStatus, TrackId};

use super::{find_vibe, parse_vibe, resolve_track};
use crate::output::Output;

#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// WAV or AIFF tracks to convert (ids or paths).
    tracks: Vec<String>,
    /// Convert every WAV and AIFF track directly in this vibe.
    #[arg(long)]
    vibe: Option<String>,
    /// Convert every WAV and AIFF track in the library.
    #[arg(long, conflicts_with_all = ["tracks", "vibe"])]
    all: bool,
    /// Write the FLACs. Without it, only previews.
    #[arg(long)]
    write: bool,
    /// Delete the FLACs Overtone wrote for these tracks (never the
    /// originals) instead of converting.
    #[arg(long, conflicts_with = "write")]
    discard: bool,
    /// List conversions waiting for relocation.
    #[arg(long, conflicts_with_all = ["tracks", "vibe", "all", "write", "discard"])]
    pending: bool,
    #[arg(long, default_value_t = overtone_core::audit::DEFAULT_AUDIT_THREADS)]
    threads: usize,
}

fn selected(library: &Library, args: &ConvertArgs) -> Result<Vec<TrackId>, CoreError> {
    let lossless_pcm =
        |format: Option<AudioFormat>| matches!(format, Some(AudioFormat::Wav | AudioFormat::Aiff));
    let mut ids: Vec<TrackId> = args
        .tracks
        .iter()
        .map(|t| resolve_track(library, t).map(|t| t.id))
        .collect::<Result<_, _>>()?;
    if args.all || args.vibe.is_some() {
        let vibe = args
            .vibe
            .as_deref()
            .map(|v| find_vibe(library, &parse_vibe(v)?))
            .transpose()?;
        for track in library.tracks()? {
            let in_scope = vibe.as_ref().is_none_or(|v| track.vibes.contains(&v.id));
            if in_scope
                && track.missing_since.is_none()
                && lossless_pcm(track.properties.format)
                && !ids.contains(&track.id)
            {
                ids.push(track.id);
            }
        }
    }
    Ok(ids)
}

pub fn run(library: &Library, args: &ConvertArgs, out: &Output) -> Result<(), CoreError> {
    if args.pending {
        let pending = library.conversions(Some(ConversionStatus::PendingRelocation))?;
        out.emit(&pending, || {
            if pending.is_empty() {
                println!("No conversions are waiting for relocation.");
            }
            for c in &pending {
                println!("track {}: {}", c.track_id.0, c.output_path);
            }
        });
        return Ok(());
    }
    let ids = selected(library, args)?;
    if ids.is_empty() {
        return Err(CoreError::Library(LibraryError::NotFoundByName {
            kind: "WAV or AIFF tracks to convert",
            name: "(pass track ids or paths, --vibe, or --all)".into(),
        }));
    }
    if args.discard {
        let mut removed = Vec::new();
        for id in &ids {
            if convert::discard_conversion(library, *id)? {
                removed.push(*id);
            }
        }
        out.emit(&removed, || {
            println!(
                "Deleted {} FLAC(s) Overtone wrote; originals untouched",
                removed.len()
            );
        });
        return Ok(());
    }

    let bar = if out.is_json() || !args.write {
        ProgressBar::hidden()
    } else {
        ProgressBar::with_draw_target(Some(ids.len() as u64), ProgressDrawTarget::stderr())
    };
    bar.set_style(
        ProgressStyle::with_template("Converting {wide_bar} {pos}/{len} {eta}")
            .expect("valid template"),
    );
    let report = convert::convert_tracks(
        library,
        &ids,
        &ConvertOptions {
            write: args.write,
            threads: args.threads.max(1),
            cancel: None,
        },
        &|p| bar.set_position(u64::from(p.done)),
    )?;
    bar.finish_and_clear();
    out.emit(&report, || {
        for plan in &report.planned {
            println!("{}", plan.source);
            println!("  -> {}", plan.output);
            if !plan.serato.is_empty() {
                println!("  Serato data carried over: {}", plan.serato.join(", "));
            }
            if !plan.dropped.is_empty() {
                println!("  Not carried over (no FLAC equivalent): {}", plan.dropped.join(", "));
            }
        }
        for skip in &report.skipped {
            println!("skipped track {}: {}", skip.track_id.0, skip.reason);
        }
        for failure in &report.failures {
            println!("FAILED track {}: {}", failure.track_id.0, failure.reason);
        }
        if report.written {
            println!(
                "Converted and verified {} track(s). Originals are untouched and stay the tracks Overtone and your DJ software use; the FLACs wait for relocation.",
                report.converted.len()
            );
        } else {
            println!(
                "Preview: {} track(s) would be converted. Add --write to convert.",
                report.planned.len()
            );
        }
    });
    Ok(())
}
