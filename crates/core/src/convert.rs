//! Lossless WAV and AIFF to FLAC conversion (Phase 6 step 8a).
//!
//! Each FLAC is written next to its original, verified sample for sample
//! (and for tags, artwork, and Serato data) by `overtone_audio::convert`,
//! and recorded as waiting for relocation. The original is never touched:
//! it stays the track in Overtone and in the DJ apps until relocation
//! (step 8b) moves them to the FLAC in lockstep. Scans skip waiting FLACs
//! so they don't appear as duplicate tracks.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use overtone_audio::convert::{
    FlacMetadata, assemble_flac, check_convertible, flac_metadata_for, verify_conversion,
};
use overtone_ffmpeg::Ffmpeg;
use overtone_fs::{FsError, ProtectedRoots};
use overtone_library::Library;
use overtone_model::{ConversionStatus, TrackId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;
use crate::audit::DEFAULT_AUDIT_THREADS;

/// One track's planned conversion, for previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ConversionPlan {
    pub track_id: TrackId,
    pub source: String,
    pub output: String,
    /// Serato data that will be carried over.
    pub serato: Vec<String>,
    /// Source frames with no FLAC equivalent.
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ConversionSkip {
    pub track_id: TrackId,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ConvertReport {
    /// What was (or, in a preview, would be) converted.
    pub planned: Vec<ConversionPlan>,
    /// Tracks converted and verified, in a run that writes.
    pub converted: Vec<TrackId>,
    pub skipped: Vec<ConversionSkip>,
    pub failures: Vec<ConversionSkip>,
    pub written: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ConvertProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
}

#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// `false` previews only.
    pub write: bool,
    pub threads: usize,
    pub cancel: Option<Arc<AtomicBool>>,
}

struct Job {
    track: TrackId,
    source: PathBuf,
    output: PathBuf,
    meta: FlacMetadata,
}

fn io_error(message: impl Into<String>) -> FsError {
    FsError::io(PathBuf::new(), std::io::Error::other(message.into()))
}

fn encode(ffmpeg: &Ffmpeg, source: &Path, encoded: &Path) -> Result<(), FsError> {
    let args: Vec<OsString> = [
        "-y".into(),
        "-i".into(),
        source.as_os_str().to_owned(),
        "-map".into(),
        "0:a:0".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-fflags".into(),
        "+bitexact".into(),
        "-c:a".into(),
        "flac".into(),
        "-compression_level".into(),
        "8".into(),
        "-f".into(),
        "flac".into(),
        encoded.as_os_str().to_owned(),
    ]
    .into();
    ffmpeg
        .job(source, args)
        .run(&AtomicBool::new(false), |_| {})
        .map(|_| ())
        .map_err(|e| io_error(e.to_string()))
}

/// Previews or performs the conversion of `ids`. Tracks that aren't WAV or
/// AIFF integer PCM, already have a waiting conversion, or whose `.flac`
/// name is taken are skipped with a reason.
pub fn convert_tracks(
    library: &Library,
    ids: &[TrackId],
    options: &ConvertOptions,
    on_progress: &(dyn Fn(ConvertProgress) + Sync),
) -> Result<ConvertReport, CoreError> {
    let mut report = ConvertReport {
        planned: Vec::new(),
        converted: Vec::new(),
        skipped: Vec::new(),
        failures: Vec::new(),
        written: options.write,
        cancelled: false,
    };
    let mut jobs = Vec::new();
    for &id in ids {
        let track = library.track(id)?;
        let skip = |reason: String| ConversionSkip {
            track_id: id,
            reason,
        };
        if track.missing_since.is_some() {
            report.skipped.push(skip("the file is missing".into()));
            continue;
        }
        if library.pending_conversion(id)?.is_some() {
            report
                .skipped
                .push(skip("already converted; waiting for relocation".into()));
            continue;
        }
        let source = PathBuf::from(&track.path);
        let output = source.with_extension("flac");
        if output.exists() {
            report.skipped.push(skip(format!(
                "{} already exists",
                output
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            )));
            continue;
        }
        let planned =
            check_convertible(&source).and_then(|layout| flac_metadata_for(&source, &layout));
        match planned {
            Ok(meta) => {
                report.planned.push(ConversionPlan {
                    track_id: id,
                    source: track.path.clone(),
                    output: output.display().to_string(),
                    serato: meta.serato.clone(),
                    dropped: meta.dropped.clone(),
                });
                jobs.push(Job {
                    track: id,
                    source,
                    output,
                    meta,
                });
            }
            Err(overtone_audio::AudioError::WriteRefused { reason, .. }) => {
                report.skipped.push(skip(reason));
            }
            Err(err) => report.skipped.push(skip(err.to_string())),
        }
    }
    if !options.write || jobs.is_empty() {
        return Ok(report);
    }

    let ffmpeg = Ffmpeg::locate()?;
    let roots = ProtectedRoots::global();
    let total = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let done = AtomicU32::new(0);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(if options.threads == 0 {
            DEFAULT_AUDIT_THREADS
        } else {
            options.threads
        })
        .build()
        .map_err(|err| CoreError::Worker(err.to_string()))?;
    let cancelled = || {
        options
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    };
    let results: Vec<Option<(TrackId, Result<u64, String>)>> = pool.install(|| {
        jobs.par_iter()
            .map(|job| {
                if cancelled() {
                    return None;
                }
                let mut frames = 0;
                let outcome = overtone_fs::create_new_via_temp(roots, &job.output, |temp| {
                    let dir = job.output.parent().unwrap_or(Path::new("."));
                    let encoded = tempfile::Builder::new()
                        .prefix(".overtone-")
                        .suffix(".tmp")
                        .tempfile_in(dir)
                        .map_err(|e| FsError::io(dir, e))?
                        .into_temp_path();
                    encode(&ffmpeg, &job.source, &encoded)?;
                    assemble_flac(&encoded, temp, &job.meta)
                        .map_err(|e| io_error(e.to_string()))?;
                    let check = verify_conversion(&job.source, temp, &job.meta)
                        .map_err(|e| io_error(e.to_string()))?;
                    frames = check.frames;
                    Ok(())
                });
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                on_progress(ConvertProgress {
                    done: n,
                    total,
                    track_id: job.track,
                });
                Some((
                    job.track,
                    outcome.map(|()| frames).map_err(|e| e.to_string()),
                ))
            })
            .collect()
    });
    for (job, result) in jobs.iter().zip(results) {
        match result {
            None => report.cancelled = true,
            Some((track, Ok(frames))) => {
                library.record_conversion(
                    track,
                    &job.output.display().to_string(),
                    "flac",
                    &job.meta.dropped,
                    &job.meta.serato,
                    frames,
                )?;
                report.converted.push(track);
            }
            Some((track, Err(reason))) => report.failures.push(ConversionSkip {
                track_id: track,
                reason,
            }),
        }
    }
    Ok(report)
}

/// Discards a waiting conversion: deletes the FLAC Overtone wrote (never
/// the original) and records it as discarded.
pub fn discard_conversion(library: &Library, track: TrackId) -> Result<bool, CoreError> {
    let Some(conversion) = library.pending_conversion(track)? else {
        return Ok(false);
    };
    let path = Path::new(&conversion.output_path);
    let original = library.track(track)?.path;
    if path == Path::new(&original) {
        return Ok(false);
    }
    ProtectedRoots::global().check(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(FsError::io(path, err).into()),
    }
    library.set_conversion_status(conversion.id, ConversionStatus::Discarded)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    use super::*;

    fn add_wav(temp: &mut TempLibrary, name: &str, fixture: Fixture) -> (TrackId, PathBuf) {
        let path = temp.add_fixture(name, &Signal::cd(16_000.0), fixture);
        let metadata = overtone_audio::tags::read_metadata(&path).expect("read");
        let (id, _) = temp
            .library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: 1,
                modified_at: 1,
                audio_hash: None,
                properties: metadata.properties,
                tags: metadata.tags,
            })
            .expect("insert");
        (id, path)
    }

    #[test]
    fn converts_wav_and_aiff_to_verified_flac_and_keeps_originals() {
        if Ffmpeg::locate().is_err() {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let mut temp = TempLibrary::new();
        let (wav, wav_path) = add_wav(&mut temp, "a", Fixture::Wav { bits: 24 });
        let (aiff, aiff_path) = add_wav(&mut temp, "b", Fixture::Aiff { bits: 16 });
        let before = std::fs::read(&wav_path).expect("read");

        let preview = convert_tracks(
            &temp.library,
            &[wav, aiff],
            &ConvertOptions::default(),
            &|_| {},
        )
        .expect("preview");
        assert_eq!(preview.planned.len(), 2);
        assert!(
            !wav_path.with_extension("flac").exists(),
            "a preview writes nothing"
        );

        let report = convert_tracks(
            &temp.library,
            &[wav, aiff],
            &ConvertOptions {
                write: true,
                ..ConvertOptions::default()
            },
            &|_| {},
        )
        .expect("convert");
        assert_eq!(report.converted.len(), 2, "{report:?}");
        assert!(wav_path.with_extension("flac").exists());
        assert!(aiff_path.with_extension("flac").exists());
        assert_eq!(
            std::fs::read(&wav_path).expect("read"),
            before,
            "original untouched"
        );
        assert!(
            temp.library
                .pending_conversion_outputs()
                .expect("outputs")
                .contains(&wav_path.with_extension("flac").display().to_string())
        );

        // Converting again skips; discarding removes only the FLAC.
        let again = convert_tracks(
            &temp.library,
            &[wav],
            &ConvertOptions {
                write: true,
                ..ConvertOptions::default()
            },
            &|_| {},
        )
        .expect("again");
        assert_eq!(again.skipped.len(), 1);
        assert!(discard_conversion(&temp.library, wav).expect("discard"));
        assert!(!wav_path.with_extension("flac").exists());
        assert!(wav_path.exists());
    }

    #[test]
    fn refuses_float_wav() {
        let mut temp = TempLibrary::new();
        let (id, _) = add_wav(&mut temp, "float", Fixture::Wav { bits: 32 });
        let report = convert_tracks(&temp.library, &[id], &ConvertOptions::default(), &|_| {})
            .expect("preview");
        assert!(report.planned.is_empty());
        assert!(
            report.skipped[0].reason.contains("floating-point"),
            "{report:?}"
        );
    }
}
