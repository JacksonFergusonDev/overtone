//! Export copies: new files in another format for USB sticks, older CDJs,
//! and phones. Originals are only read; an export never replaces a file.
//!
//! ffmpeg encodes, Symphonia checks the result (format, sample rate,
//! channels, and length), and only a verified file is moved into place.
//! Tags come from the source plus the track's current values in Overtone,
//! so a value the DJ set or accepted is in the copy. DJ software data
//! (Serato cues and beatgrids in GEOB frames) is not carried over.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overtone_audio::analysis::Decoder;
use overtone_ffmpeg::Ffmpeg;
use overtone_fs::{FsError, ProtectedRoots};
use overtone_library::Library;
use overtone_model::{AudioFormat, MetadataField, TrackId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;
use crate::audit::DEFAULT_AUDIT_THREADS;

/// What to export to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// AAC in `.m4a`, with Apple's encoder when available.
    Aac,
    /// MP3 (LAME), constant bitrate, ID3v2.3 tags.
    Mp3,
    Flac,
    /// AIFF with ID3v2 tags: the format every CDJ plays.
    Aiff,
    Wav,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Aac => "m4a",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Aiff => "aiff",
            Self::Wav => "wav",
        }
    }

    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Aiff | Self::Wav)
    }

    fn muxer(self) -> &'static str {
        match self {
            Self::Aac => "ipod",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Aiff => "aiff",
            Self::Wav => "wav",
        }
    }

    fn decoded_as(self) -> AudioFormat {
        match self {
            Self::Aac => AudioFormat::Aac,
            Self::Mp3 => AudioFormat::Mp3,
            Self::Flac => AudioFormat::Flac,
            Self::Aiff => AudioFormat::Aiff,
            Self::Wav => AudioFormat::Wav,
        }
    }
}

/// How to export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ExportSettings {
    pub format: ExportFormat,
    /// Lossy bitrate in kbps (AAC default 256, MP3 default 320).
    pub bitrate_kbps: Option<u32>,
    /// Resample to this rate; `None` keeps the source's.
    pub sample_rate_hz: Option<u32>,
    /// 16 or 24 for lossless formats; `None` keeps the source's (24 when
    /// the source is deeper).
    pub bit_depth: Option<u8>,
}

impl ExportSettings {
    /// AIFF, 16-bit, 44.1 kHz: plays on every Pioneer CDJ, including
    /// players without FLAC or hi-res support.
    pub fn cdj() -> Self {
        Self {
            format: ExportFormat::Aiff,
            bitrate_kbps: None,
            sample_rate_hz: Some(44_100),
            bit_depth: Some(16),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub threads: usize,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ExportedFile {
    pub track_id: TrackId,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ExportFailure {
    pub track_id: TrackId,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ExportReport {
    pub exported: Vec<ExportedFile>,
    pub failures: Vec<ExportFailure>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ExportProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
}

struct Job {
    track: TrackId,
    source: PathBuf,
    dest: PathBuf,
    source_rate: Option<u32>,
    source_bits: Option<u8>,
    duration_ms: Option<u64>,
    tags: Vec<(String, String)>,
}

fn failed(message: impl Into<String>) -> FsError {
    FsError::io(PathBuf::new(), std::io::Error::other(message.into()))
}

/// The ffmpeg tag keys for the track's current values. ID3 formats also
/// get BPM and key frames.
fn tags_for(
    library: &Library,
    track: TrackId,
    format: ExportFormat,
) -> Result<Vec<(String, String)>, CoreError> {
    let mut tags = Vec::new();
    let id3 = matches!(format, ExportFormat::Mp3 | ExportFormat::Aiff);
    for value in library.field_values(track)? {
        let Some(text) = value.value else { continue };
        let key = match value.field {
            MetadataField::Title => "title",
            MetadataField::Artist => "artist",
            MetadataField::Album => "album",
            MetadataField::AlbumArtist => "album_artist",
            MetadataField::Genre => "genre",
            MetadataField::Comment => "comment",
            MetadataField::Grouping => "grouping",
            MetadataField::Label => "publisher",
            MetadataField::Year => "date",
            MetadataField::TrackNumber => "track",
            MetadataField::DiscNumber => "disc",
            MetadataField::Isrc if id3 => "TSRC",
            MetadataField::Bpm if id3 => "TBPM",
            MetadataField::Key if id3 => "TKEY",
            _ => continue,
        };
        tags.push((key.to_string(), text));
    }
    Ok(tags)
}

/// A destination path in `dir` named after the source, with " (2)" and so
/// on when taken by an existing file or another export in this batch.
fn destination(dir: &Path, source: &Path, extension: &str, taken: &mut Vec<PathBuf>) -> PathBuf {
    let stem = source
        .file_stem()
        .map_or_else(|| "track".into(), |s| s.to_string_lossy().into_owned());
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({n}).{extension}")
        };
        let candidate = dir.join(name);
        if !candidate.exists() && !taken.contains(&candidate) {
            taken.push(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

fn ffmpeg_args(
    job: &Job,
    settings: &ExportSettings,
    ffmpeg_caps: &Caps,
    temp: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-y".into(), "-i".into(), job.source.clone().into()];
    let artwork = matches!(
        settings.format,
        ExportFormat::Aac | ExportFormat::Mp3 | ExportFormat::Flac | ExportFormat::Aiff
    );
    args.extend(["-map", "0:a:0"].map(OsString::from));
    if artwork {
        args.extend(
            [
                "-map",
                "0:v?",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ]
            .map(OsString::from),
        );
    }
    args.extend(["-map_metadata", "0"].map(OsString::from));
    for (key, value) in &job.tags {
        args.push("-metadata".into());
        args.push(format!("{key}={value}").into());
    }

    let rate = settings.sample_rate_hz.or(job.source_rate);
    let resample = settings
        .sample_rate_hz
        .is_some_and(|r| Some(r) != job.source_rate);
    let bits = match settings.format {
        ExportFormat::Flac | ExportFormat::Aiff | ExportFormat::Wav => {
            Some(settings.bit_depth.unwrap_or_else(|| {
                if job.source_bits.unwrap_or(16) > 16 {
                    24
                } else {
                    16
                }
            }))
        }
        _ => None,
    };
    let reduce_depth = bits.is_some_and(|b| job.source_bits.is_some_and(|s| s > b));
    if resample || reduce_depth {
        let engine = if ffmpeg_caps.soxr {
            "resampler=soxr:precision=28"
        } else {
            "filter_size=64:phase_shift=10:cutoff=0.97:linear_interp=1"
        };
        let mut filter = format!("aresample={engine}");
        if reduce_depth {
            filter.push_str(":dither_method=triangular_hp");
        }
        if let Some(rate) = rate {
            filter.push_str(&format!(":osr={rate}"));
        }
        args.extend(["-af".into(), filter.into()]);
    }
    match settings.format {
        ExportFormat::Aac => {
            let codec = if ffmpeg_caps.aac_at { "aac_at" } else { "aac" };
            let kbps = settings.bitrate_kbps.unwrap_or(256);
            args.extend(["-c:a", codec, "-b:a"].map(OsString::from));
            args.push(format!("{kbps}k").into());
        }
        ExportFormat::Mp3 => {
            let kbps = settings.bitrate_kbps.unwrap_or(320);
            args.extend(["-c:a", "libmp3lame", "-b:a"].map(OsString::from));
            args.push(format!("{kbps}k").into());
            args.extend(["-id3v2_version", "3"].map(OsString::from));
        }
        ExportFormat::Flac => {
            args.extend(["-c:a", "flac", "-compression_level", "8"].map(OsString::from));
            if bits == Some(24) {
                args.extend(["-sample_fmt", "s32"].map(OsString::from));
            } else {
                args.extend(["-sample_fmt", "s16"].map(OsString::from));
            }
        }
        ExportFormat::Aiff => {
            let codec = if bits == Some(24) {
                "pcm_s24be"
            } else {
                "pcm_s16be"
            };
            args.extend(
                ["-c:a", codec, "-write_id3v2", "1", "-id3v2_version", "3"].map(OsString::from),
            );
        }
        ExportFormat::Wav => {
            let codec = if bits == Some(24) {
                "pcm_s24le"
            } else {
                "pcm_s16le"
            };
            args.extend(["-c:a", codec].map(OsString::from));
        }
    }
    args.extend(["-f", settings.format.muxer()].map(OsString::from));
    args.push(temp.into());
    args
}

struct Caps {
    aac_at: bool,
    soxr: bool,
}

/// Decodes the copy with Symphonia and checks it is what was asked for.
fn verify(job: &Job, settings: &ExportSettings, temp: &Path) -> Result<(), String> {
    let mut decoder = Decoder::open(temp).map_err(|e| format!("the copy doesn't decode: {e}"))?;
    let info = decoder.info().clone();
    if info.format != settings.format.decoded_as() {
        return Err(format!("the copy decodes as {:?}", info.format));
    }
    if let Some(rate) = settings.sample_rate_hz
        && info.sample_rate_hz != rate
    {
        return Err(format!(
            "the copy is {} Hz, not {rate} Hz",
            info.sample_rate_hz
        ));
    }
    while decoder
        .next_block()
        .map_err(|e| format!("the copy doesn't decode: {e}"))?
        .is_some()
    {}
    let ms = decoder.decoded_frames() * 1000 / u64::from(info.sample_rate_hz.max(1));
    if let Some(expected) = job.duration_ms {
        // Lossy encoders add or trim up to a few frames of delay.
        let tolerance = if settings.format.is_lossless() {
            50
        } else {
            500
        };
        if ms.abs_diff(expected) > tolerance {
            return Err(format!(
                "the copy is {ms} ms long, the source {expected} ms"
            ));
        }
    }
    Ok(())
}

/// Exports `ids` into `dir`. Never replaces an existing file.
pub fn export_tracks(
    library: &Library,
    ids: &[TrackId],
    dir: &Path,
    settings: &ExportSettings,
    options: &ExportOptions,
    on_progress: &(dyn Fn(ExportProgress) + Sync),
) -> Result<ExportReport, CoreError> {
    let ffmpeg = Ffmpeg::locate()?;
    if settings.format == ExportFormat::Mp3 && !ffmpeg.has_encoder("libmp3lame")? {
        return Err(CoreError::Ffmpeg(overtone_ffmpeg::FfmpegError::Failed {
            input: dir.to_path_buf(),
            message: "this ffmpeg has no MP3 encoder (libmp3lame)".into(),
        }));
    }
    let caps = Caps {
        aac_at: ffmpeg.has_encoder("aac_at")?,
        soxr: ffmpeg.has_soxr()?,
    };
    std::fs::create_dir_all(dir).map_err(|source| FsError::io(dir, source))?;
    let roots = ProtectedRoots::global();
    let mut taken = Vec::new();
    let mut jobs = Vec::with_capacity(ids.len());
    for &id in ids {
        let track = library.track(id)?;
        let source = PathBuf::from(&track.path);
        jobs.push(Job {
            track: id,
            dest: destination(dir, &source, settings.format.extension(), &mut taken),
            source,
            source_rate: track.properties.sample_rate_hz,
            source_bits: track.properties.bit_depth,
            duration_ms: track.properties.duration_ms,
            tags: tags_for(library, id, settings.format)?,
        });
    }
    let total = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let done = std::sync::atomic::AtomicU32::new(0);
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
    let results: Vec<Option<Result<ExportedFile, ExportFailure>>> = pool.install(|| {
        jobs.par_iter()
            .map(|job| {
                if cancelled() {
                    return None;
                }
                let outcome = overtone_fs::create_new_via_temp(roots, &job.dest, |temp| {
                    let args = ffmpeg_args(job, settings, &caps, temp);
                    ffmpeg
                        .job(&job.source, args)
                        .run(&AtomicBool::new(false), |_| {})
                        .map_err(|e| failed(e.to_string()))?;
                    verify(job, settings, temp).map_err(failed)
                });
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                on_progress(ExportProgress {
                    done: n,
                    total,
                    track_id: job.track,
                });
                Some(match outcome {
                    Ok(()) => Ok(ExportedFile {
                        track_id: job.track,
                        path: job.dest.display().to_string(),
                    }),
                    Err(err) => Err(ExportFailure {
                        track_id: job.track,
                        message: err.to_string(),
                    }),
                })
            })
            .collect()
    });
    let mut report = ExportReport {
        exported: Vec::new(),
        failures: Vec::new(),
        cancelled: false,
    };
    for result in results {
        match result {
            Some(Ok(file)) => report.exported.push(file),
            Some(Err(failure)) => report.failures.push(failure),
            None => report.cancelled = true,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_model::{AudioProperties, TrackTags};
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    use super::*;

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a fixture length in milliseconds"
    )]
    fn library_with_fixture(temp: &mut TempLibrary, bits: u16, rate: u32) -> TrackId {
        let signal = Signal {
            sample_rate: rate,
            ..Signal::cd(16_000.0)
        };
        let path = temp.add_fixture("Source Track", &signal, Fixture::Flac { bits });
        let (id, _) = temp
            .library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: 1,
                modified_at: 1,
                audio_hash: None,
                properties: AudioProperties {
                    format: Some(AudioFormat::Flac),
                    sample_rate_hz: Some(rate),
                    bit_depth: Some(u8::try_from(bits).expect("bits")),
                    channels: Some(2),
                    bitrate_kbps: None,
                    duration_ms: Some((f64::from(signal.seconds) * 1000.0) as u64),
                },
                tags: TrackTags {
                    title: Some("From the file".into()),
                    ..TrackTags::default()
                },
            })
            .expect("insert");
        id
    }

    #[test]
    fn cdj_preset_downsamples_hi_res_to_16_bit_aiff_with_the_djs_title() {
        if Ffmpeg::locate().is_err() {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let mut temp = TempLibrary::new();
        let id = library_with_fixture(&mut temp, 24, 96_000);
        temp.library
            .set_user_field(id, MetadataField::Title, Some("My Title"))
            .expect("set");
        let out = temp.dir.path().join("usb");
        let report = export_tracks(
            &temp.library,
            &[id],
            &out,
            &ExportSettings::cdj(),
            &ExportOptions::default(),
            &|_| {},
        )
        .expect("export");
        assert!(report.failures.is_empty(), "{report:?}");
        let path = PathBuf::from(&report.exported[0].path);
        assert_eq!(path.file_name().expect("name"), "Source Track.aiff");
        let decoder = Decoder::open(&path).expect("open");
        assert_eq!(decoder.info().sample_rate_hz, 44_100);
        assert_eq!(decoder.info().bits_per_sample, Some(16));
        let tags = overtone_audio::tags::read_metadata(&path)
            .expect("tags")
            .tags;
        assert_eq!(tags.title.as_deref(), Some("My Title"));

        // A second export never replaces the first.
        let again = export_tracks(
            &temp.library,
            &[id],
            &out,
            &ExportSettings::cdj(),
            &ExportOptions::default(),
            &|_| {},
        )
        .expect("export again");
        assert!(again.exported[0].path.ends_with("Source Track (2).aiff"));
    }

    #[test]
    fn lossy_exports_verify_and_land() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("skipping: needs ffmpeg");
            return;
        };
        let mut temp = TempLibrary::new();
        let id = library_with_fixture(&mut temp, 16, 44_100);
        let out = temp.dir.path().join("phone");
        let mut formats = vec![ExportFormat::Aac, ExportFormat::Flac, ExportFormat::Wav];
        if ffmpeg.has_encoder("libmp3lame").expect("encoders") {
            formats.push(ExportFormat::Mp3);
        }
        for format in formats {
            let report = export_tracks(
                &temp.library,
                &[id],
                &out,
                &ExportSettings {
                    format,
                    bitrate_kbps: None,
                    sample_rate_hz: None,
                    bit_depth: None,
                },
                &ExportOptions::default(),
                &|_| {},
            )
            .expect("export");
            assert!(report.failures.is_empty(), "{format:?}: {report:?}");
            assert!(report.exported[0].path.ends_with(format.extension()));
        }
        // Only the exports are in the folder: no temporary files.
        let names: Vec<_> = std::fs::read_dir(&out)
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(
            names
                .iter()
                .all(|n| !n.to_string_lossy().starts_with(".overtone-"))
        );
    }
}
