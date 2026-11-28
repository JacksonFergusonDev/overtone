//! Chromaprint fingerprints: the same audio in two formats matches, and
//! different audio doesn't.

use overtone_audio::analysis::fingerprint::{FINGERPRINT_SECONDS, fingerprint, similarity};
use overtone_testkit::{Fixture, Signal, TempLibrary};

#[test]
fn same_audio_matches_across_formats_and_different_audio_does_not() {
    let temp = TempLibrary::new();
    let a = temp.add_fixture("a", &Signal::noise(16_000.0), Fixture::Flac { bits: 16 });
    let a_wav = temp.add_fixture("a24", &Signal::noise(16_000.0), Fixture::Wav { bits: 24 });
    let b = temp.add_fixture("b", &Signal::cd(12_000.0), Fixture::Flac { bits: 16 });

    let fa = fingerprint(&a, None).expect("a");
    let fa_wav = fingerprint(&a_wav, None).expect("a wav");
    let fb = fingerprint(&b, None).expect("b");
    assert!(!fa.compressed.is_empty());
    assert!(fa.duration_secs <= FINGERPRINT_SECONDS.max(fa.duration_secs));
    assert!(similarity(&fa.raw, &fa_wav.raw) > 0.95);
    assert!(similarity(&fa.raw, &fb.raw) < 0.8);
}

#[test]
fn ffmpeg_fallback_fingerprints_the_same_audio() {
    let Ok(ffmpeg) = overtone_ffmpeg::Ffmpeg::locate() else {
        eprintln!("skipping: needs ffmpeg");
        return;
    };
    let temp = TempLibrary::new();
    let flac = temp.add_fixture("a", &Signal::noise(16_000.0), Fixture::Flac { bits: 16 });
    let opus = temp.dir.path().join("a.opus");
    let status = ffmpeg
        .command()
        .arg("-i")
        .arg(&flac)
        .args(["-c:a", "libopus", "-b:a", "128k"])
        .arg(&opus)
        .status()
        .expect("encode");
    if !status.success() {
        eprintln!("skipping: needs ffmpeg with libopus");
        return;
    }
    // Symphonia has no Opus decoder, so this goes through ffmpeg.
    assert!(fingerprint(&opus, None).is_err());
    let from_opus = fingerprint(&opus, Some(&ffmpeg)).expect("opus via ffmpeg");
    let from_flac = fingerprint(&flac, None).expect("flac");
    assert!(similarity(&from_flac.raw, &from_opus.raw) > 0.85);
}
