//! Spectral analysis on deterministic synthetic fixtures and three small
//! real lossy encodes.
//!
//! Synthetic fixtures are written to temporary folders by overtone-testkit.
//! The checked-in files in `tests/fixtures/` (about 180 KB) were made once
//! from 2.5 s of ffmpeg pink noise (`anoisesrc=color=pink:amplitude=0.25:
//! seed=7`, stereo, 16-bit 44.1 kHz) with LAME 4.0 and macOS afconvert:
//!
//! - `noise-128k.mp3`: `lame -b 128 src.wav`
//! - `noise-320k-from-128k.mp3`: the 128 kbps file decoded with
//!   `lame --decode` and re-encoded with `lame -b 320`
//! - `noise-aac-128k.m4a`: `afconvert -f m4af -d aac -b 128000 src.wav`

use std::path::{Path, PathBuf};

use overtone_audio::AudioError;
use overtone_audio::analysis::{
    ANALYZER_VERSION, Analysis, AnalysisOptions, Decoder, SpectrogramOptions, WAVEFORM_BUCKETS,
    analyze,
};
use overtone_model::{AudioFormat, QualityVerdict, ReviewStatus};
use overtone_testkit::{Fixture, Rolloff, Signal, SignalKind, TempLibrary, write_samples};

fn run(temp: &TempLibrary, name: &str, signal: &Signal, fixture: Fixture) -> Analysis {
    let path = temp.add_fixture(name, signal, fixture);
    analyze(&path, &AnalysisOptions::default()).expect("analyze")
}

fn flac(temp: &TempLibrary, name: &str, signal: &Signal) -> Analysis {
    run(temp, name, signal, Fixture::Flac { bits: 16 })
}

fn verdict(analysis: &Analysis) -> QualityVerdict {
    analysis.report.verdict
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn decode_all(path: &Path) -> (Vec<f32>, u32, u16) {
    let mut decoder = Decoder::open(path).expect("open");
    let mut samples = Vec::new();
    let mut channels = 0;
    while let Some(block) = decoder.next_block().expect("decode") {
        channels = block.channels;
        samples.extend_from_slice(block.samples);
    }
    (
        samples,
        decoder.info().sample_rate_hz,
        u16::try_from(channels).expect("channels"),
    )
}

/// Decodes a lossy fixture and rewraps it as 16-bit FLAC: a fake lossless file.
fn rewrap_as_flac(temp: &TempLibrary, source: &Path, name: &str) -> PathBuf {
    let (samples, sample_rate, channels) = decode_all(source);
    let signal = Signal {
        sample_rate,
        channels,
        ..Signal::noise(22_050.0)
    };
    let path = temp.music_dir().join(format!("{name}.flac"));
    write_samples(&path, &samples, &signal, Fixture::Flac { bits: 16 }).expect("write flac");
    path
}

#[test]
fn full_band_cd_flac_passes() {
    let temp = TempLibrary::new();
    let analysis = flac(&temp, "full", &Signal::noise(22_050.0));
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Pass,
        "{:?}",
        analysis.report
    );
    let report = &analysis.report;
    assert!(report.confidence.expect("confidence") > 0.8);
    assert_eq!(report.analyzer_version, ANALYZER_VERSION);
    assert_eq!(report.review, ReviewStatus::Unreviewed);
    assert!(report.analyzed_at > 0);
    assert_eq!(analysis.stream.format, AudioFormat::Flac);
    assert_eq!(analysis.sample_rate_hz, 44_100);
}

#[test]
fn hard_16_khz_wall_in_flac_is_suspect() {
    let temp = TempLibrary::new();
    let analysis = flac(&temp, "c16", &Signal::noise(16_000.0));
    assert_eq!(verdict(&analysis), QualityVerdict::Suspect);
    let cutoff = analysis.report.cutoff_hz.expect("cutoff");
    assert!((cutoff - 16_000.0).abs() <= 150.0, "cutoff {cutoff}");
    let reason = analysis.report.reason.expect("reason");
    assert!(
        reason.contains("16.0 kHz") && reason.contains("dB/oct"),
        "{reason}"
    );
    assert!(reason.contains("128 kbps"), "{reason}");
}

#[test]
fn hard_19_khz_wall_is_suspect_or_warning() {
    let temp = TempLibrary::new();
    let analysis = flac(&temp, "c19", &Signal::noise(19_000.0));
    assert!(
        matches!(
            verdict(&analysis),
            QualityVerdict::Suspect | QualityVerdict::Warning
        ),
        "{:?}",
        analysis.report
    );
}

#[test]
fn wall_at_20_5_khz_is_only_a_warning() {
    let temp = TempLibrary::new();
    let analysis = flac(&temp, "c205", &Signal::noise(20_500.0));
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Warning,
        "{:?}",
        analysis.report
    );
}

#[test]
fn gentle_roll_off_is_never_suspect() {
    let temp = TempLibrary::new();
    for (name, start_hz, db_per_octave) in [
        ("gentle", 15_000.0, 12.0),
        // A steep analog-style filter: 60 dB down by 20 kHz, but no cliff.
        ("dull", 10_000.0, 60.0),
    ] {
        let signal = Signal {
            rolloff: Some(Rolloff {
                start_hz,
                db_per_octave,
            }),
            ..Signal::noise(22_050.0)
        };
        let analysis = flac(&temp, name, &signal);
        assert_ne!(
            verdict(&analysis),
            QualityVerdict::Suspect,
            "{name}: {:?}",
            analysis.report
        );
    }
}

#[test]
fn hires_container_with_cd_band_content_is_suspect() {
    let temp = TempLibrary::new();
    let analysis = run(
        &temp,
        "fake-hires",
        &Signal::hires(20_000.0),
        Fixture::Flac { bits: 24 },
    );
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Suspect,
        "{:?}",
        analysis.report
    );
    assert!(
        analysis
            .report
            .reason
            .expect("reason")
            .contains("upsampled")
    );
}

#[test]
fn genuine_hires_passes() {
    let temp = TempLibrary::new();
    let analysis = run(
        &temp,
        "hires",
        &Signal::hires(48_000.0),
        Fixture::Flac { bits: 24 },
    );
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Pass,
        "{:?}",
        analysis.report
    );
    assert_eq!(analysis.sample_rate_hz, 96_000);
}

#[test]
fn near_silent_and_bass_only_tracks_are_inconclusive() {
    let temp = TempLibrary::new();
    let quiet = Signal {
        amplitude: 0.0003,
        ..Signal::noise(22_050.0)
    };
    assert_eq!(
        verdict(&flac(&temp, "quiet", &quiet)),
        QualityVerdict::Inconclusive
    );
    let bass = flac(&temp, "bass", &Signal::noise(9_000.0));
    assert_eq!(
        verdict(&bass),
        QualityVerdict::Inconclusive,
        "{:?}",
        bass.report
    );
    assert!(bass.report.confidence.is_none());
}

#[test]
fn sparse_treble_is_not_suspect() {
    let temp = TempLibrary::new();
    let sparse = Signal {
        seconds: 4.0,
        kind: SignalKind::Tones,
        ..Signal::cd(21_000.0)
    };
    assert_ne!(
        verdict(&flac(&temp, "sparse", &sparse)),
        QualityVerdict::Suspect
    );
}

#[test]
fn wav_aiff_and_24_bit_variants_decode() {
    let temp = TempLibrary::new();
    let signal = Signal {
        seconds: 2.5,
        ..Signal::noise(22_050.0)
    };
    for (name, fixture, format, bits) in [
        ("w16", Fixture::Wav { bits: 16 }, AudioFormat::Wav, Some(16)),
        ("w24", Fixture::Wav { bits: 24 }, AudioFormat::Wav, Some(24)),
        (
            "wf32",
            Fixture::Wav { bits: 32 },
            AudioFormat::Wav,
            Some(32),
        ),
        (
            "a16",
            Fixture::Aiff { bits: 16 },
            AudioFormat::Aiff,
            Some(16),
        ),
        (
            "a24",
            Fixture::Aiff { bits: 24 },
            AudioFormat::Aiff,
            Some(24),
        ),
        (
            "f24",
            Fixture::Flac { bits: 24 },
            AudioFormat::Flac,
            Some(24),
        ),
    ] {
        let analysis = run(&temp, name, &signal, fixture);
        assert_eq!(analysis.stream.format, format, "{name}");
        assert_eq!(analysis.stream.bits_per_sample, bits, "{name}");
        assert_eq!(analysis.stream.channels, 2, "{name}");
        assert_eq!(analysis.duration_ms, 2500, "{name}");
        assert_eq!(
            verdict(&analysis),
            QualityVerdict::Pass,
            "{name}: {:?}",
            analysis.report
        );
    }
}

#[test]
fn corrupt_missing_and_empty_files_are_errors_not_panics() {
    let temp = TempLibrary::new();
    let dir = temp.music_dir();
    let garbage = dir.join("garbage.flac");
    std::fs::write(&garbage, b"fLaC\0\0\0\x22 this is not audio").expect("write");
    let empty = dir.join("empty.mp3");
    std::fs::write(&empty, b"").expect("write");
    let options = AnalysisOptions::default();
    assert!(analyze(&garbage, &options).is_err());
    assert!(analyze(&empty, &options).is_err());
    assert!(matches!(
        analyze(&dir.join("missing.wav"), &options),
        Err(AudioError::Io { .. })
    ));

    // A valid file cut off halfway still analyzes what it holds.
    let good = temp.add_fixture("cut", &Signal::noise(22_050.0), Fixture::Wav { bits: 16 });
    let bytes = std::fs::read(&good).expect("read");
    let truncated = dir.join("truncated.wav");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("write");
    let analysis = analyze(&truncated, &options).expect("truncated file analyzes");
    assert!(analysis.duration_ms < 2100, "{}", analysis.duration_ms);
}

#[test]
fn analysis_never_modifies_the_audio_file() {
    let temp = TempLibrary::new();
    let path = temp.add_fixture("ro", &Signal::noise(16_000.0), Fixture::Flac { bits: 16 });
    let before = std::fs::read(&path).expect("read");
    let modified = std::fs::metadata(&path)
        .expect("stat")
        .modified()
        .expect("mtime");
    analyze(&path, &AnalysisOptions::default()).expect("analyze");
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert_eq!(
        std::fs::metadata(&path)
            .expect("stat")
            .modified()
            .expect("mtime"),
        modified
    );
}

#[test]
fn waveform_has_expected_length_and_range() {
    let temp = TempLibrary::new();
    let signal = Signal {
        amplitude: 0.8,
        ..Signal::noise(22_050.0)
    };
    let analysis = flac(&temp, "wave", &signal);
    let buckets = &analysis.waveform.buckets;
    assert_eq!(buckets.len(), WAVEFORM_BUCKETS);
    assert!(
        buckets
            .iter()
            .all(|[min, max]| -1.0 <= *min && min <= max && *max <= 1.0)
    );
    let peak = buckets.iter().map(|[_, max]| *max).fold(0.0_f32, f32::max);
    assert!((peak - 0.8).abs() < 0.01, "peak {peak}");
    assert!(
        analysis
            .channel_peaks
            .iter()
            .all(|p| (p - 0.8).abs() < 0.01)
    );
    assert_eq!(analysis.clipped_runs, 0);
    assert_eq!(analysis.spectrum_summary.len(), 512);
}

#[test]
fn counts_clipped_runs() {
    let temp = TempLibrary::new();
    let signal = Signal {
        seconds: 2.5,
        ..Signal::noise(22_050.0)
    };
    // Drive the noise 4x into the rails; write_samples clamps to full scale.
    let loud: Vec<f32> = signal.samples().iter().map(|s| s * 4.0).collect();
    let path = temp.music_dir().join("clipped.wav");
    write_samples(&path, &loud, &signal, Fixture::Wav { bits: 16 }).expect("write");
    let analysis = analyze(&path, &AnalysisOptions::default()).expect("analyze");
    assert!(analysis.clipped_runs > 10, "{}", analysis.clipped_runs);
}

#[test]
fn writes_spectrogram_png_with_requested_size() {
    let temp = TempLibrary::new();
    let path = temp.add_fixture("spec", &Signal::noise(16_000.0), Fixture::Flac { bits: 16 });
    for log_frequency in [false, true] {
        let png_path = temp.dir.path().join(format!("spec-{log_frequency}.png"));
        let options = AnalysisOptions {
            spectrogram: Some(SpectrogramOptions {
                path: png_path.clone(),
                width: 400,
                height: 256,
                log_frequency,
            }),
            ..AnalysisOptions::default()
        };
        analyze(&path, &options).expect("analyze");
        let decoder = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(&png_path).expect("open png"),
        ));
        let mut reader = decoder.read_info().expect("png header");
        let info = reader.info();
        assert_eq!((info.width, info.height), (400, 256));
        assert_eq!(info.color_type, png::ColorType::Indexed);
        let mut pixels = vec![0; reader.output_buffer_size().expect("size")];
        reader.next_frame(&mut pixels).expect("pixels");
        if !log_frequency {
            // Linear axis: the top rows (above 16 kHz) are dark, the bottom bright.
            let top = pixels[..400].iter().map(|&p| u32::from(p)).sum::<u32>() / 400;
            let bottom = pixels[255 * 400..]
                .iter()
                .map(|&p| u32::from(p))
                .sum::<u32>()
                / 400;
            assert!(top < 20 && bottom > 100, "top {top} bottom {bottom}");
        }
    }
}

#[test]
fn real_128k_mp3_passes_as_what_it_is() {
    let analysis = analyze(&fixture("noise-128k.mp3"), &AnalysisOptions::default()).expect("mp3");
    assert_eq!(analysis.stream.format, AudioFormat::Mp3);
    let kbps = analysis.features.bitrate_kbps.expect("bitrate");
    assert!((120..=136).contains(&kbps), "{kbps} kbps");
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Pass,
        "{:?}",
        analysis.report
    );
}

#[test]
fn real_mp3_rewrapped_as_flac_is_suspect() {
    let temp = TempLibrary::new();
    let path = rewrap_as_flac(&temp, &fixture("noise-128k.mp3"), "fake-lossless");
    let analysis = analyze(&path, &AnalysisOptions::default()).expect("flac");
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Suspect,
        "{:?}",
        analysis.report
    );
}

#[test]
fn real_128k_upscaled_to_320k_is_suspect() {
    let analysis = analyze(
        &fixture("noise-320k-from-128k.mp3"),
        &AnalysisOptions::default(),
    )
    .expect("mp3");
    assert!(analysis.features.bitrate_kbps.expect("bitrate") >= 300);
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Suspect,
        "{:?}",
        analysis.report
    );
}

#[test]
fn real_aac_decodes_and_its_flac_rewrap_is_flagged() {
    let source = fixture("noise-aac-128k.m4a");
    let analysis = analyze(&source, &AnalysisOptions::default()).expect("m4a");
    assert_eq!(analysis.stream.format, AudioFormat::Aac);
    assert_eq!(
        verdict(&analysis),
        QualityVerdict::Pass,
        "{:?}",
        analysis.report
    );

    let temp = TempLibrary::new();
    let path = rewrap_as_flac(&temp, &source, "fake-aac-lossless");
    let rewrapped = analyze(&path, &AnalysisOptions::default()).expect("flac");
    assert!(
        rewrapped.report.verdict.needs_review(),
        "{:?}",
        rewrapped.report
    );
}

/// An HE-AAC file made by Apple's encoder through ffmpeg, or `None` when
/// ffmpeg (with AudioToolbox) isn't installed.
fn he_aac(temp: &TempLibrary) -> Option<(overtone_ffmpeg::Ffmpeg, PathBuf)> {
    let ffmpeg = overtone_ffmpeg::Ffmpeg::locate().ok()?;
    if !ffmpeg.has_encoder("aac_at").ok()? {
        return None;
    }
    let wav = temp.add_fixture(
        "he-source",
        &Signal::noise(16_000.0),
        Fixture::Wav { bits: 16 },
    );
    let out = temp.dir.path().join("he.m4a");
    let status = ffmpeg
        .command()
        .arg("-i")
        .arg(&wav)
        .args(["-c:a", "aac_at", "-profile:a", "4", "-b:a", "48k"])
        .arg(&out)
        .status()
        .ok()?;
    status.success().then_some((ffmpeg, out))
}

#[test]
fn he_aac_falls_back_to_ffmpeg() {
    let temp = TempLibrary::new();
    let Some((ffmpeg, path)) = he_aac(&temp) else {
        eprintln!("skipping: needs ffmpeg with aac_at");
        return;
    };
    // Without a fallback this is the Unchecked case from PROGRESS.md.
    assert!(analyze(&path, &AnalysisOptions::default()).is_err());

    let analysis = analyze(
        &path,
        &AnalysisOptions {
            fallback: Some(ffmpeg),
            ..AnalysisOptions::default()
        },
    )
    .expect("ffmpeg fallback");
    assert!(analysis.decoded_by_ffmpeg);
    assert_eq!(analysis.stream.format, AudioFormat::Aac);
    assert_ne!(verdict(&analysis), QualityVerdict::Unchecked);
    assert!(
        analysis
            .report
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("Decoded with ffmpeg")),
        "{:?}",
        analysis.report
    );
    assert!(analysis.duration_ms > 1_000);
}

#[test]
fn files_symphonia_reads_never_use_the_fallback() {
    let temp = TempLibrary::new();
    let path = temp.add_fixture("plain", &Signal::cd(16_000.0), Fixture::Flac { bits: 16 });
    let Ok(ffmpeg) = overtone_ffmpeg::Ffmpeg::locate() else {
        return;
    };
    let analysis = analyze(
        &path,
        &AnalysisOptions {
            fallback: Some(ffmpeg),
            ..AnalysisOptions::default()
        },
    )
    .expect("analyze");
    assert!(!analysis.decoded_by_ffmpeg);
}
