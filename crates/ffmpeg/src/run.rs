//! Integrity checks and encode jobs with progress and cancellation.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::decode::{drain_stderr, join_stderr};
use crate::{Ffmpeg, FfmpegError};

/// What a full decode of a file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    /// Errors ffmpeg reported while decoding, one per line. Empty for a
    /// clean file.
    pub errors: Vec<String>,
    /// Whether ffmpeg gave up before the end of the stream.
    pub aborted: bool,
}

impl IntegrityReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && !self.aborted
    }
}

pub(crate) fn check_integrity(
    ffmpeg: &Ffmpeg,
    input: &Path,
) -> Result<IntegrityReport, FfmpegError> {
    let output = ffmpeg
        .command()
        .args(["-err_detect", "crccheck+bitstream+buffer"])
        .arg("-i")
        .arg(input)
        .args(["-map", "0:a:0", "-f", "null", "-"])
        .stdout(Stdio::null())
        .output()
        .map_err(|source| FfmpegError::Spawn {
            path: ffmpeg.path().to_path_buf(),
            source,
        })?;
    let errors: Vec<String> = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    Ok(IntegrityReport {
        aborted: !output.status.success(),
        errors,
    })
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutcome {
    /// Non-fatal errors ffmpeg printed.
    pub warnings: Vec<String>,
}

/// An ffmpeg run with `-progress` reporting, for encodes and remuxes.
pub struct Job {
    ffmpeg: PathBuf,
    input: PathBuf,
    args: Vec<OsString>,
}

impl Job {
    pub(crate) fn new<I, S>(ffmpeg: &Ffmpeg, input: &Path, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            ffmpeg: ffmpeg.path().to_path_buf(),
            input: input.to_path_buf(),
            args: args.into_iter().map(|a| a.as_ref().to_owned()).collect(),
        }
    }

    /// The full argument list, for logs and tests.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// Runs the job. `progress` gets the output position as ffmpeg reports
    /// it (about twice a second). Setting `cancel` kills the process and
    /// returns [`FfmpegError::Cancelled`].
    pub fn run(
        &self,
        cancel: &AtomicBool,
        mut progress: impl FnMut(Duration),
    ) -> Result<JobOutcome, FfmpegError> {
        let mut child = std::process::Command::new(&self.ffmpeg)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
            .args(["-progress", "pipe:1", "-nostats"])
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| FfmpegError::Spawn {
                path: self.ffmpeg.clone(),
                source,
            })?;
        let stderr = drain_stderr(&mut child);
        let stdout = child.stdout.take().expect("piped stdout");

        // Read progress on a thread so cancellation is checked even while
        // ffmpeg is quiet.
        let (tx, rx) = std::sync::mpsc::channel::<Duration>();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(us) = line.strip_prefix("out_time_us=")
                    && let Ok(us) = us.trim().parse::<u64>()
                {
                    let _ = tx.send(Duration::from_micros(us));
                }
            }
        });
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(FfmpegError::Cancelled);
            }
            while let Ok(position) = rx.try_recv() {
                progress(position);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let _ = reader.join();
        while let Ok(position) = rx.try_recv() {
            progress(position);
        }
        let stderr = join_stderr(stderr);
        if status.success() {
            Ok(JobOutcome {
                warnings: stderr.lines().map(str::to_string).collect(),
            })
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
}

#[cfg(test)]
mod tests {
    use overtone_testkit::{Fixture, Signal, write_fixture};
    use tempfile::TempDir;

    use super::*;
    use crate::testing::ffmpeg;

    #[test]
    fn clean_file_passes_and_corrupt_file_reports_errors() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a.flac");
        write_fixture(&path, &Signal::cd(16_000.0), Fixture::Flac { bits: 16 }).expect("fixture");
        assert!(ffmpeg.check_integrity(&path).expect("check").is_clean());

        // Flip bytes in the middle of the audio frames.
        let mut bytes = std::fs::read(&path).expect("read");
        let mid = bytes.len() / 2;
        for b in &mut bytes[mid..mid + 64] {
            *b ^= 0x5A;
        }
        let broken = dir.path().join("broken.flac");
        std::fs::write(&broken, bytes).expect("write");
        let report = ffmpeg.check_integrity(&broken).expect("check");
        assert!(!report.is_clean(), "{report:?}");
    }

    #[test]
    fn job_reports_progress_and_writes_output() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let input = dir.path().join("a.wav");
        write_fixture(&input, &Signal::cd(16_000.0), Fixture::Wav { bits: 16 }).expect("fixture");
        let output = dir.path().join("a.flac");
        let job = ffmpeg.job(
            &input,
            [
                OsStr::new("-i"),
                input.as_os_str(),
                OsStr::new("-c:a"),
                OsStr::new("flac"),
                output.as_os_str(),
            ],
        );
        let mut last = Duration::ZERO;
        job.run(&AtomicBool::new(false), |p| last = p).expect("run");
        assert!(output.exists());
        assert!(last > Duration::ZERO);
    }

    #[test]
    fn cancelled_job_stops() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = TempDir::new().expect("tempdir");
        let input = dir.path().join("a.wav");
        write_fixture(&input, &Signal::cd(16_000.0), Fixture::Wav { bits: 16 }).expect("fixture");
        let job = ffmpeg.job(
            &input,
            ["-re", "-i"]
                .map(OsString::from)
                .into_iter()
                .chain([input.clone().into_os_string()])
                .chain(["-f", "null", "-"].map(OsString::from)),
        );
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            job.run(&cancel, |_| {}),
            Err(FfmpegError::Cancelled)
        ));
    }
}
