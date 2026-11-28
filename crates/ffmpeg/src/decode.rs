//! Streaming decode to 32-bit float PCM through a pipe.
//!
//! ffmpeg writes a WAV stream (`pcm_f32le`) to stdout. The WAV header makes
//! the stream self-describing, so no separate probe is needed for the
//! sample rate and channel count.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Stdio};
use std::thread::JoinHandle;

use crate::{Ffmpeg, FfmpegError};

/// How to decode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Resample to this rate. `None` keeps the stream's own rate.
    pub sample_rate: Option<u32>,
    /// Mix to this many channels. `None` keeps the stream's own.
    pub channels: Option<u16>,
}

/// Interleaved float samples streamed from a running ffmpeg. Dropping it
/// before the end kills the process.
pub struct PcmReader {
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr: Option<JoinHandle<String>>,
    input: PathBuf,
    sample_rate: u32,
    channels: u16,
    pending: Vec<u8>,
    done: bool,
}

/// Collects a child's stderr on a thread, so a chatty process can't block
/// on a full pipe. Keeps at most 64 KiB.
pub(crate) fn drain_stderr(child: &mut Child) -> Option<JoinHandle<String>> {
    let mut stderr = child.stderr.take()?;
    Some(std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            if bytes.len() < 64 * 1024 {
                bytes.extend_from_slice(&buf[..n]);
            }
        }
        String::from_utf8_lossy(&bytes).trim().to_string()
    }))
}

pub(crate) fn join_stderr(handle: Option<JoinHandle<String>>) -> String {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

pub(crate) fn start(
    ffmpeg: &Ffmpeg,
    input: &Path,
    options: &DecodeOptions,
) -> Result<PcmReader, FfmpegError> {
    let mut command = ffmpeg.command();
    command
        .arg("-i")
        .arg(input)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-map_metadata", "-1"])
        .args(["-fflags", "+bitexact", "-c:a", "pcm_f32le"]);
    if let Some(rate) = options.sample_rate {
        command.args(["-ar", &rate.to_string()]);
    }
    if let Some(channels) = options.channels {
        command.args(["-ac", &channels.to_string()]);
    }
    command
        .args(["-f", "wav", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|source| FfmpegError::Spawn {
        path: ffmpeg.path().to_path_buf(),
        source,
    })?;
    let stderr = drain_stderr(&mut child);
    let stdout = child.stdout.take().ok_or_else(|| FfmpegError::BadOutput {
        input: input.to_path_buf(),
        message: "no stdout".into(),
    })?;
    let mut reader = PcmReader {
        child,
        stdout: BufReader::with_capacity(1 << 16, stdout),
        stderr,
        input: input.to_path_buf(),
        sample_rate: 0,
        channels: 0,
        pending: Vec::new(),
        done: false,
    };
    match reader.read_header() {
        Ok((rate, channels)) => {
            reader.sample_rate = rate;
            reader.channels = channels;
            Ok(reader)
        }
        Err(header_error) => Err(reader.fail(&header_error)),
    }
}

impl PcmReader {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    fn read_header(&mut self) -> Result<(u32, u16), String> {
        let mut riff = [0u8; 12];
        self.stdout
            .read_exact(&mut riff)
            .map_err(|_| "no audio was decoded".to_string())?;
        if &riff[..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
            return Err("ffmpeg didn't write a WAV stream".into());
        }
        let mut format = None;
        loop {
            let mut head = [0u8; 8];
            self.stdout
                .read_exact(&mut head)
                .map_err(|_| "the WAV stream ended before its data".to_string())?;
            let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            match &head[..4] {
                b"data" => break,
                b"fmt " => {
                    let mut body = vec![0u8; size as usize];
                    self.stdout
                        .read_exact(&mut body)
                        .map_err(|_| "truncated fmt chunk".to_string())?;
                    format = Some(parse_fmt(&body)?);
                }
                _ => {
                    let skip = u64::from(size) + u64::from(size % 2);
                    std::io::copy(&mut (&mut self.stdout).take(skip), &mut std::io::sink())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        format.ok_or_else(|| "the WAV stream has no fmt chunk".into())
    }

    /// Reads interleaved samples into `buf`, returning how many were read
    /// (a whole number of samples, not necessarily of frames). `Ok(0)`
    /// means the stream ended; call [`Self::finish`] to learn whether
    /// ffmpeg succeeded.
    pub fn read(&mut self, buf: &mut [f32]) -> Result<usize, FfmpegError> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len() * 4;
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.reserve(want.saturating_sub(bytes.len()));
        let mut chunk = vec![0u8; want.saturating_sub(bytes.len()).max(4)];
        while bytes.len() < 4 {
            let n = self.stdout.read(&mut chunk)?;
            if n == 0 {
                self.done = true;
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        let whole = (bytes.len() / 4).min(buf.len());
        for (sample, raw) in buf.iter_mut().zip(bytes.as_chunks::<4>().0).take(whole) {
            *sample = f32::from_le_bytes(*raw);
        }
        self.pending = bytes.split_off(whole * 4);
        Ok(whole)
    }

    /// Reads everything that's left.
    pub fn read_to_end(&mut self) -> Result<Vec<f32>, FfmpegError> {
        let mut out = Vec::new();
        let mut buf = vec![0f32; 1 << 15];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    /// Waits for ffmpeg to exit. Returns the errors it reported while
    /// decoding (empty for a clean file), or an error if it failed.
    pub fn finish(mut self) -> Result<Vec<String>, FfmpegError> {
        // Drain anything unread so ffmpeg isn't blocked writing.
        std::io::copy(&mut self.stdout, &mut std::io::sink())?;
        self.done = true;
        let status = self.child.wait()?;
        let stderr = join_stderr(self.stderr.take());
        if status.success() {
            Ok(stderr.lines().map(str::to_string).collect())
        } else {
            Err(FfmpegError::Failed {
                input: self.input.clone(),
                message: if stderr.is_empty() {
                    format!("exited with {status}")
                } else {
                    stderr
                },
            })
        }
    }

    fn fail(mut self, detail: &str) -> FfmpegError {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.done = true;
        let stderr = join_stderr(self.stderr.take());
        FfmpegError::Failed {
            input: self.input.clone(),
            message: if stderr.is_empty() {
                detail.to_string()
            } else {
                stderr
            },
        }
    }
}

impl Drop for PcmReader {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The sample rate and channel count from a `fmt ` chunk, which must
/// describe 32-bit float samples.
fn parse_fmt(body: &[u8]) -> Result<(u32, u16), String> {
    if body.len() < 16 {
        return Err("short fmt chunk".into());
    }
    let tag = u16::from_le_bytes([body[0], body[1]]);
    let channels = u16::from_le_bytes([body[2], body[3]]);
    let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let bits = u16::from_le_bytes([body[14], body[15]]);
    // 3 is IEEE float; 0xFFFE is WAVE_FORMAT_EXTENSIBLE, whose subformat
    // GUID starts with the same tag.
    let float = tag == 3 || (tag == 0xFFFE && body.len() >= 26 && body[24] == 3 && body[25] == 0);
    if !float || bits != 32 || channels == 0 || rate == 0 {
        return Err(format!(
            "unexpected PCM format (tag {tag:#x}, {bits} bits, {channels} channels, {rate} Hz)"
        ));
    }
    Ok((rate, channels))
}

#[cfg(test)]
mod tests {
    use overtone_testkit::{Fixture, Signal, write_fixture};
    use tempfile::TempDir;

    use super::*;
    use crate::testing::ffmpeg;

    #[test]
    fn decodes_flac_to_the_same_samples() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.flac");
        let signal = Signal::cd(16_000.0);
        write_fixture(&path, &signal, Fixture::Flac { bits: 16 }).expect("fixture");

        let mut pcm = ffmpeg
            .decode(&path, &DecodeOptions::default())
            .expect("decode");
        assert_eq!(pcm.sample_rate(), signal.sample_rate);
        assert_eq!(pcm.channels(), signal.channels);
        let samples = pcm.read_to_end().expect("read");
        assert!(pcm.finish().expect("finish").is_empty());
        let expected = signal.samples();
        assert_eq!(samples.len(), expected.len());
        let worst = samples
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1.0 / 32_000.0, "16-bit rounding only, got {worst}");
    }

    #[test]
    fn resamples_and_downmixes_on_request() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.wav");
        write_fixture(&path, &Signal::cd(16_000.0), Fixture::Wav { bits: 24 }).expect("fixture");
        let pcm = ffmpeg
            .decode(
                &path,
                &DecodeOptions {
                    sample_rate: Some(22_050),
                    channels: Some(1),
                },
            )
            .expect("decode");
        assert_eq!((pcm.sample_rate(), pcm.channels()), (22_050, 1));
    }

    #[test]
    fn reports_files_that_are_not_audio() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("junk.mp3");
        std::fs::write(&path, b"definitely not audio").expect("write");
        let err = ffmpeg
            .decode(&path, &DecodeOptions::default())
            .err()
            .expect("must fail");
        assert!(matches!(err, FfmpegError::Failed { .. }), "{err}");
    }

    #[test]
    fn parses_float_and_extensible_fmt_chunks() {
        let mut plain = vec![3, 0, 2, 0];
        plain.extend_from_slice(&44_100u32.to_le_bytes());
        plain.extend_from_slice(&[0; 6]);
        plain.extend_from_slice(&32u16.to_le_bytes());
        assert_eq!(parse_fmt(&plain), Ok((44_100, 2)));
        let mut pcm16 = plain.clone();
        pcm16[0] = 1;
        assert!(parse_fmt(&pcm16).is_err());
    }
}
