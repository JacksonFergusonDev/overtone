use std::path::PathBuf;

use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use overtone_core::CoreError;
use overtone_core::export::{self, ExportFormat, ExportOptions, ExportSettings};
use overtone_core::library::{Library, LibraryError};
use overtone_core::model::TrackId;

use super::{find_vibe, parse_vibe, resolve_track};
use crate::output::Output;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// AAC in .m4a (256 kbps by default).
    Aac,
    /// MP3, constant bitrate (320 kbps by default).
    Mp3,
    Flac,
    Aiff,
    Wav,
    /// AIFF, 16-bit, 44.1 kHz: plays on every CDJ.
    Cdj,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Tracks to export (ids or paths).
    tracks: Vec<String>,
    /// Export every track directly in this vibe.
    #[arg(long)]
    vibe: Option<String>,
    /// Folder to write the copies into. Existing files are never replaced.
    #[arg(long, value_name = "FOLDER")]
    to: PathBuf,
    #[arg(long, value_enum, default_value = "cdj")]
    format: Format,
    /// Bitrate in kbps for AAC and MP3.
    #[arg(long)]
    bitrate: Option<u32>,
    /// Sample rate in Hz, for example 44100.
    #[arg(long)]
    sample_rate: Option<u32>,
    /// 16 or 24, for FLAC, AIFF, and WAV.
    #[arg(long, value_parser = ["16", "24"])]
    bit_depth: Option<String>,
    /// Files to encode at once.
    #[arg(long, default_value_t = overtone_core::audit::DEFAULT_AUDIT_THREADS)]
    threads: usize,
}

pub fn run(library: &Library, args: &ExportArgs, out: &Output) -> Result<(), CoreError> {
    let mut ids: Vec<TrackId> = args
        .tracks
        .iter()
        .map(|t| resolve_track(library, t).map(|t| t.id))
        .collect::<Result<_, _>>()?;
    if let Some(vibe) = &args.vibe {
        let vibe = find_vibe(library, &parse_vibe(vibe)?)?;
        for track in library.tracks()? {
            if track.vibes.contains(&vibe.id)
                && track.missing_since.is_none()
                && !ids.contains(&track.id)
            {
                ids.push(track.id);
            }
        }
    }
    if ids.is_empty() {
        return Err(CoreError::Library(LibraryError::NotFoundByName {
            kind: "tracks to export",
            name: "(pass track ids or paths, or --vibe)".into(),
        }));
    }
    let mut settings = match args.format {
        Format::Cdj => ExportSettings::cdj(),
        other => ExportSettings {
            format: match other {
                Format::Aac => ExportFormat::Aac,
                Format::Mp3 => ExportFormat::Mp3,
                Format::Flac => ExportFormat::Flac,
                Format::Aiff => ExportFormat::Aiff,
                Format::Wav | Format::Cdj => ExportFormat::Wav,
            },
            bitrate_kbps: None,
            sample_rate_hz: None,
            bit_depth: None,
        },
    };
    settings.bitrate_kbps = args.bitrate.or(settings.bitrate_kbps);
    settings.sample_rate_hz = args.sample_rate.or(settings.sample_rate_hz);
    if let Some(bits) = &args.bit_depth {
        settings.bit_depth = bits.parse().ok();
    }

    let bar = if out.is_json() {
        ProgressBar::hidden()
    } else {
        ProgressBar::with_draw_target(Some(ids.len() as u64), ProgressDrawTarget::stderr())
    };
    bar.set_style(
        ProgressStyle::with_template("Exporting {wide_bar} {pos}/{len} {eta}")
            .expect("valid template"),
    );
    let report = export::export_tracks(
        library,
        &ids,
        &args.to,
        &settings,
        &ExportOptions {
            threads: args.threads.max(1),
            cancel: None,
        },
        &|p| bar.set_position(u64::from(p.done)),
    )?;
    bar.finish_and_clear();
    out.emit(&report, || {
        println!(
            "Exported {} track(s) to {} as {}; your originals were only read",
            report.exported.len(),
            args.to.display(),
            settings.format.extension()
        );
        for failure in &report.failures {
            println!("  track {}: {}", failure.track_id.0, failure.message);
        }
        println!(
            "Copies carry tags and artwork, not Serato or Traktor cue data; analyze them in your DJ app."
        );
    });
    Ok(())
}
