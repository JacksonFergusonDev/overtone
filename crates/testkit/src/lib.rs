//! Test helpers shared across Overtone's crates.
//!
//! Generates small synthetic audio fixtures in temporary directories so
//! tests never depend on real music files, and opens throwaway libraries.
//! Use only from `[dev-dependencies]`.

// Sample math converts between float and integer domains on bounded values.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::path::{Path, PathBuf};

use overtone_library::Library;
use tempfile::TempDir;

/// A deterministic multi-tone test signal whose energy stops at a chosen
/// frequency, like audio that passed through an encoder's low-pass filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signal {
    pub sample_rate: u32,
    pub channels: u16,
    pub seconds: f32,
    /// Highest frequency with energy, in Hz. Nothing is generated above it.
    pub cutoff_hz: f32,
    /// Peak amplitude, from 0.0 to 1.0.
    pub amplitude: f32,
    /// Seeds the tone phases, so the same spec always gives the same samples.
    pub seed: u64,
    /// Sparse tones or dense noise.
    pub kind: SignalKind,
    /// Optional gentle roll-off applied above a frequency, like a warm
    /// master or a vinyl rip, instead of (or before) the hard cutoff.
    pub rolloff: Option<Rolloff>,
}

/// The spectral character of a [`Signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalKind {
    /// 160 log-spaced tones: sparse in the treble, like hi-hats every few bars.
    #[default]
    Tones,
    /// Dense pink-sloped noise (random-phase spectrum, flat when smoothed),
    /// like a full mix. Use it for spectral-cutoff tests.
    Noise,
}

/// A gentle roll-off: level falls by `db_per_octave` above `start_hz`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rolloff {
    pub start_hz: f32,
    pub db_per_octave: f32,
}

impl Signal {
    /// Two seconds of 44.1 kHz stereo with energy up to `cutoff_hz`.
    pub fn cd(cutoff_hz: f32) -> Self {
        Self {
            sample_rate: 44_100,
            channels: 2,
            seconds: 2.0,
            cutoff_hz,
            amplitude: 0.5,
            seed: 1,
            kind: SignalKind::Tones,
            rolloff: None,
        }
    }

    /// Four seconds of 44.1 kHz stereo noise with energy up to `cutoff_hz`
    /// and nothing above, like a decoded lossy file.
    pub fn noise(cutoff_hz: f32) -> Self {
        Self {
            seconds: 4.0,
            kind: SignalKind::Noise,
            ..Self::cd(cutoff_hz)
        }
    }

    /// Four seconds of 96 kHz stereo noise with energy up to `cutoff_hz`.
    pub fn hires(cutoff_hz: f32) -> Self {
        Self {
            sample_rate: 96_000,
            ..Self::noise(cutoff_hz)
        }
    }

    /// Linear gain of the roll-off at `freq`.
    fn rolloff_gain(&self, freq: f32) -> f32 {
        match self.rolloff {
            Some(r) if freq > r.start_hz => {
                10.0_f32.powf(-r.db_per_octave * (freq / r.start_hz).log2() / 20.0)
            }
            _ => 1.0,
        }
    }

    pub fn frames(&self) -> usize {
        (self.seconds * self.sample_rate as f32).round() as usize
    }

    /// Interleaved samples in -1.0..=1.0.
    ///
    /// The signal sums tones spaced logarithmically from 30 Hz to the cutoff
    /// with a pink-like slope (amplitude falls as 1/sqrt(f)), so the spectrum
    /// resembles music rather than a single sine.
    pub fn samples(&self) -> Vec<f32> {
        if self.kind == SignalKind::Noise {
            return self.noise_samples();
        }
        let nyquist = self.sample_rate as f32 / 2.0;
        let top = self.cutoff_hz.min(nyquist * 0.999).max(40.0);
        let bottom: f32 = 30.0;
        let tone_count = 160;
        let mut rng = XorShift(self.seed.max(1));
        let tones: Vec<(f32, f32, f32)> = (0..tone_count)
            .map(|i| {
                let t = i as f32 / (tone_count - 1) as f32;
                let freq = bottom * (top / bottom).powf(t);
                let weight = self.rolloff_gain(freq) / freq.sqrt();
                let phase = rng.next_f32() * std::f32::consts::TAU;
                (freq, weight, phase)
            })
            .collect();
        let total_weight: f32 = tones.iter().map(|(_, w, _)| w).sum();

        let frames = self.frames();
        let channels = usize::from(self.channels.max(1));
        let mut out = Vec::with_capacity(frames * channels);
        for n in 0..frames {
            let time = n as f32 / self.sample_rate as f32;
            let mut value = 0.0;
            for (freq, weight, phase) in &tones {
                value += weight * (std::f32::consts::TAU * freq * time + phase).sin();
            }
            let sample = self.amplitude * value / total_weight * 3.0;
            let sample = sample.clamp(-1.0, 1.0);
            for _ in 0..channels {
                out.push(sample);
            }
        }
        out
    }
}

impl Signal {
    /// Noise synthesized in the frequency domain: every bin from 30 Hz to
    /// the cutoff gets a pink (1/sqrt f) magnitude and a random phase, and
    /// everything above is exactly zero. The result is normalised so its
    /// peak equals `amplitude`.
    fn noise_samples(&self) -> Vec<f32> {
        use realfft::RealFftPlanner;
        use realfft::num_complex::Complex;

        let frames = self.frames().max(2);
        let len = frames + frames % 2;
        let inverse = RealFftPlanner::<f32>::new().plan_fft_inverse(len);
        let mut spectrum = inverse.make_input_vec();
        let bin_hz = self.sample_rate as f32 / len as f32;
        let mut rng = XorShift(self.seed.max(1));
        let last = spectrum.len() - 1;
        for (k, value) in spectrum.iter_mut().enumerate() {
            let freq = k as f32 * bin_hz;
            if k == 0 || k == last || freq < 30.0 || freq > self.cutoff_hz {
                continue;
            }
            let phase = rng.next_f32() * std::f32::consts::TAU;
            let magnitude = self.rolloff_gain(freq) / freq.sqrt();
            *value = Complex::from_polar(magnitude, phase);
        }
        let mut mono = inverse.make_output_vec();
        inverse
            .process(&mut spectrum, &mut mono)
            .expect("buffers come from the planner");
        mono.truncate(frames);
        let peak = mono.iter().fold(0.0_f32, |m, v| m.max(v.abs())).max(1e-12);
        let gain = self.amplitude / peak;
        let channels = usize::from(self.channels.max(1));
        let mut out = Vec::with_capacity(frames * channels);
        for sample in mono.iter().take(self.frames()) {
            for _ in 0..channels {
                out.push(sample * gain);
            }
        }
        out
    }
}

/// Container format and sample format for a generated fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// PCM WAV with the given bit depth (16, 24, or 32 for float).
    Wav { bits: u16 },
    /// FLAC with the given bit depth (16 or 24).
    Flac { bits: u16 },
    /// Big-endian PCM AIFF with the given bit depth (16 or 24).
    Aiff { bits: u16 },
}

impl Fixture {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav { .. } => "wav",
            Self::Flac { .. } => "flac",
            Self::Aiff { .. } => "aiff",
        }
    }
}

/// Writes `signal` to `path` in the `fixture` format.
pub fn write_fixture(path: &Path, signal: &Signal, fixture: Fixture) -> std::io::Result<()> {
    write_samples(path, &signal.samples(), signal, fixture)
}

/// Writes interleaved `samples` (for example a decoded MP3, to build a fake
/// lossless file) using `signal`'s sample rate and channel count.
pub fn write_samples(
    path: &Path,
    samples: &[f32],
    signal: &Signal,
    fixture: Fixture,
) -> std::io::Result<()> {
    let samples: Vec<f32> = samples.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    let samples = samples.as_slice();
    match fixture {
        Fixture::Wav { bits } => write_wav(path, samples, signal, bits),
        Fixture::Flac { bits } => write_flac(path, samples, signal, bits),
        Fixture::Aiff { bits } => write_aiff(path, samples, signal, bits),
    }
}

fn quantize(sample: f32, bits: u16) -> i32 {
    let max = ((1_i64 << (bits - 1)) - 1) as f32;
    (sample * max).round() as i32
}

fn write_wav(path: &Path, samples: &[f32], signal: &Signal, bits: u16) -> std::io::Result<()> {
    let spec = hound::WavSpec {
        channels: signal.channels,
        sample_rate: signal.sample_rate,
        bits_per_sample: bits,
        sample_format: if bits == 32 {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(std::io::Error::other)?;
    for &sample in samples {
        if bits == 32 {
            writer.write_sample(sample)
        } else {
            writer.write_sample(quantize(sample, bits))
        }
        .map_err(std::io::Error::other)?;
    }
    writer.finalize().map_err(std::io::Error::other)
}

fn write_flac(path: &Path, samples: &[f32], signal: &Signal, bits: u16) -> std::io::Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let ints: Vec<i32> = samples.iter().map(|&s| quantize(s, bits)).collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, err)| std::io::Error::other(format!("{err:?}")))?;
    let source = flacenc::source::MemSource::from_samples(
        &ints,
        usize::from(signal.channels),
        usize::from(bits),
        signal.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|err| std::io::Error::other(format!("{err:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|err| std::io::Error::other(format!("{err:?}")))?;
    let mut bytes = sink.as_slice().to_vec();
    // Two flacenc STREAMINFO quirks make strict decoders (symphonia)
    // reject the file. FLAC wants the minimum block size to exclude the
    // final, shorter block of a fixed-blocksize stream, but flacenc counts
    // it; and its frame-size bounds can be inconsistent for 24-bit noise.
    // Copy the maximum block size over the minimum (bytes 8-9 and 10-11,
    // after "fLaC" and the 4-byte block header) and mark both frame sizes
    // unknown (zero, bytes 12-17), which the format allows.
    if bytes.len() > 18 && &bytes[..4] == b"fLaC" {
        let (min, max) = bytes[8..12].split_at_mut(2);
        min.copy_from_slice(max);
        bytes[12..18].fill(0);
    }
    std::fs::write(path, bytes)
}

fn write_aiff(path: &Path, samples: &[f32], signal: &Signal, bits: u16) -> std::io::Result<()> {
    let bytes_per_sample = usize::from(bits / 8);
    let channels = usize::from(signal.channels.max(1));
    let frames = samples.len() / channels;
    let sound_len = samples.len() * bytes_per_sample;

    let mut comm = Vec::with_capacity(18);
    comm.extend_from_slice(&signal.channels.to_be_bytes());
    comm.extend_from_slice(
        &u32::try_from(frames)
            .map_err(std::io::Error::other)?
            .to_be_bytes(),
    );
    comm.extend_from_slice(&bits.to_be_bytes());
    comm.extend_from_slice(&extended_80(f64::from(signal.sample_rate)));

    let mut ssnd = Vec::with_capacity(8 + sound_len);
    ssnd.extend_from_slice(&0_u32.to_be_bytes()); // offset
    ssnd.extend_from_slice(&0_u32.to_be_bytes()); // block size
    for &sample in samples {
        let value = quantize(sample, bits).to_be_bytes();
        ssnd.extend_from_slice(&value[4 - bytes_per_sample..]);
    }

    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    let form_len = 4 + 8 + comm.len() + 8 + ssnd.len() + (ssnd.len() % 2);
    file.extend_from_slice(&chunk_len(form_len)?);
    file.extend_from_slice(b"AIFF");
    file.extend_from_slice(b"COMM");
    file.extend_from_slice(&chunk_len(comm.len())?);
    file.extend_from_slice(&comm);
    file.extend_from_slice(b"SSND");
    file.extend_from_slice(&chunk_len(ssnd.len())?);
    file.extend_from_slice(&ssnd);
    if ssnd.len() % 2 == 1 {
        file.push(0);
    }
    std::fs::write(path, file)
}

fn chunk_len(len: usize) -> std::io::Result<[u8; 4]> {
    Ok(u32::try_from(len)
        .map_err(std::io::Error::other)?
        .to_be_bytes())
}

/// Encodes a positive integer-valued sample rate as an IEEE 754 80-bit
/// extended float, as AIFF's COMM chunk requires.
fn extended_80(value: f64) -> [u8; 10] {
    let mut out = [0_u8; 10];
    if value <= 0.0 {
        return out;
    }
    let integer = value as u64;
    let shift = integer.leading_zeros();
    let exponent = 16_383 + 63 - shift;
    let mantissa = integer << shift;
    out[..2].copy_from_slice(&u16::try_from(exponent).unwrap_or(0).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

struct XorShift(u64);

impl XorShift {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1_u64 << 24) as f32
    }
}

/// A temporary folder with an open library, removed when dropped.
pub struct TempLibrary {
    pub dir: TempDir,
    pub library: Library,
    pub library_path: PathBuf,
}

impl TempLibrary {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let library_path = dir.path().join("library.sqlite");
        let library = Library::open(&library_path).expect("open temp library");
        Self {
            dir,
            library,
            library_path,
        }
    }

    /// A folder inside the temp dir for audio fixtures, created on first use.
    pub fn music_dir(&self) -> PathBuf {
        let dir = self.dir.path().join("music");
        std::fs::create_dir_all(&dir).expect("create music dir");
        dir
    }

    /// Writes a fixture under `music_dir()` and returns its path.
    pub fn add_fixture(&self, name: &str, signal: &Signal, fixture: Fixture) -> PathBuf {
        let path = self
            .music_dir()
            .join(format!("{name}.{}", fixture.extension()));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture folder");
        }
        write_fixture(&path, signal, fixture).expect("write fixture");
        path
    }
}

impl Default for TempLibrary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_is_deterministic_and_bounded() {
        let signal = Signal {
            seconds: 0.1,
            ..Signal::cd(16_000.0)
        };
        let a = signal.samples();
        assert_eq!(a, signal.samples());
        assert_eq!(a.len(), signal.frames() * 2);
        assert!(a.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(a.iter().any(|s| s.abs() > 0.05), "signal is not silent");
    }

    #[test]
    fn writes_readable_wav() {
        let temp = TempLibrary::new();
        let signal = Signal {
            seconds: 0.25,
            ..Signal::cd(19_000.0)
        };
        let path = temp.add_fixture("tone", &signal, Fixture::Wav { bits: 24 });
        let reader = hound::WavReader::open(&path).expect("open wav");
        assert_eq!(reader.spec().sample_rate, 44_100);
        assert_eq!(reader.spec().bits_per_sample, 24);
        assert_eq!(reader.duration() as usize, signal.frames());
    }

    #[test]
    fn writes_flac_and_aiff_with_correct_headers() {
        let temp = TempLibrary::new();
        let signal = Signal {
            seconds: 0.25,
            ..Signal::cd(21_000.0)
        };
        let flac = temp.add_fixture("a", &signal, Fixture::Flac { bits: 16 });
        let bytes = std::fs::read(&flac).expect("read flac");
        assert_eq!(&bytes[..4], b"fLaC");

        let aiff = temp.add_fixture("b", &signal, Fixture::Aiff { bits: 16 });
        let bytes = std::fs::read(&aiff).expect("read aiff");
        assert_eq!(&bytes[..4], b"FORM");
        assert_eq!(&bytes[8..12], b"AIFF");
        assert_eq!(
            u32::from_be_bytes(bytes[4..8].try_into().expect("len")) as usize,
            bytes.len() - 8
        );
    }

    #[test]
    fn noise_is_deterministic_bounded_and_band_limited() {
        let signal = Signal {
            seconds: 0.5,
            ..Signal::noise(16_000.0)
        };
        let a = signal.samples();
        assert_eq!(a, signal.samples());
        assert_eq!(a.len(), signal.frames() * 2);
        let peak = a.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 1e-3, "peak {peak}");
        let hires = Signal::hires(20_000.0);
        assert_eq!(hires.sample_rate, 96_000);
        let rolled = Signal {
            rolloff: Some(Rolloff {
                start_hz: 1000.0,
                db_per_octave: 12.0,
            }),
            ..signal
        };
        assert!((rolled.rolloff_gain(2000.0) - 0.251).abs() < 0.01);
    }

    #[test]
    fn encodes_44100_as_80_bit_float() {
        assert_eq!(
            extended_80(44_100.0),
            [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn temp_library_opens_empty() {
        let temp = TempLibrary::new();
        assert_eq!(temp.library.summary().expect("summary").track_count, 0);
        assert!(temp.library_path.exists());
    }
}
