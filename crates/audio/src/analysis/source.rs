//! Where analysis gets its samples: Symphonia, or ffmpeg as a fallback for
//! files Symphonia can't decode (HE-AAC, Opus, WMA, damaged containers).

use std::path::{Path, PathBuf};

use overtone_ffmpeg::{DecodeOptions, Ffmpeg, PcmReader};
use overtone_model::AudioFormat;

use super::decode::{Block, Decoder, StreamInfo};
use crate::AudioError;

/// Frames per block read from ffmpeg.
const FFMPEG_BLOCK_FRAMES: usize = 4096;

pub(crate) enum Source {
    Symphonia(Decoder),
    Ffmpeg(FfmpegSource),
}

pub(crate) struct FfmpegSource {
    path: PathBuf,
    pcm: Option<PcmReader>,
    buf: Vec<f32>,
    info: StreamInfo,
    frames: u64,
    bitrate_kbps: Option<u32>,
}

impl Source {
    pub(crate) fn ffmpeg(ffmpeg: &Ffmpeg, path: &Path) -> Result<Self, AudioError> {
        let pcm = ffmpeg
            .decode(path, &DecodeOptions::default())
            .map_err(|err| AudioError::Decode {
                path: path.to_path_buf(),
                message: err.to_string(),
            })?;
        let properties = crate::tags::read_metadata(path).ok().map(|m| m.properties);
        let format = properties
            .as_ref()
            .and_then(|p| p.format)
            .or_else(|| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .and_then(AudioFormat::from_extension)
            })
            .unwrap_or(AudioFormat::Other);
        let info = StreamInfo {
            format,
            container: "ffmpeg",
            sample_rate_hz: pcm.sample_rate(),
            channels: pcm.channels(),
            bits_per_sample: properties.as_ref().and_then(|p| p.bit_depth.map(u32::from)),
            frames_hint: None,
        };
        let channels = usize::from(pcm.channels());
        Ok(Self::Ffmpeg(FfmpegSource {
            path: path.to_path_buf(),
            pcm: Some(pcm),
            buf: vec![0.0; FFMPEG_BLOCK_FRAMES * channels],
            info,
            frames: 0,
            bitrate_kbps: properties.and_then(|p| p.bitrate_kbps),
        }))
    }

    pub(crate) fn info(&self) -> &StreamInfo {
        match self {
            Self::Symphonia(d) => d.info(),
            Self::Ffmpeg(f) => &f.info,
        }
    }

    pub(crate) fn next_block(&mut self) -> Result<Option<Block<'_>>, AudioError> {
        match self {
            Self::Symphonia(d) => d.next_block(),
            Self::Ffmpeg(f) => f.next_block(),
        }
    }

    pub(crate) fn decoded_frames(&self) -> u64 {
        match self {
            Self::Symphonia(d) => d.decoded_frames(),
            Self::Ffmpeg(f) => f.frames,
        }
    }

    pub(crate) fn skipped_packets(&self) -> u64 {
        match self {
            Self::Symphonia(d) => d.skipped_packets(),
            Self::Ffmpeg(_) => 0,
        }
    }

    /// Average bitrate over `duration_secs`: measured from packet sizes
    /// with Symphonia, or the container's figure with ffmpeg.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bitrates are small positive values"
    )]
    pub(crate) fn bitrate_kbps(&self, duration_secs: f64) -> Option<u32> {
        match self {
            Self::Symphonia(d) => (duration_secs > 0.0)
                .then(|| (d.packet_bytes() as f64 * 8.0 / duration_secs / 1000.0).round() as u32),
            Self::Ffmpeg(f) => f.bitrate_kbps,
        }
    }

    pub(crate) fn decoded_by_ffmpeg(&self) -> bool {
        matches!(self, Self::Ffmpeg(_))
    }
}

impl FfmpegSource {
    fn next_block(&mut self) -> Result<Option<Block<'_>>, AudioError> {
        let Some(pcm) = self.pcm.as_mut() else {
            return Ok(None);
        };
        let channels = usize::from(self.info.channels);
        let mut filled = 0;
        // Fill whole frames; the reader can return partial frames.
        while filled < self.buf.len() {
            let n = pcm
                .read(&mut self.buf[filled..])
                .map_err(|err| AudioError::Decode {
                    path: self.path.clone(),
                    message: err.to_string(),
                })?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        let whole = filled - filled % channels;
        if whole == 0 {
            let pcm = self.pcm.take().expect("checked above");
            pcm.finish().map_err(|err| AudioError::Decode {
                path: self.path.clone(),
                message: err.to_string(),
            })?;
            return Ok(None);
        }
        self.frames += (whole / channels) as u64;
        Ok(Some(Block {
            samples: &self.buf[..whole],
            channels,
            sample_rate: self.info.sample_rate_hz,
        }))
    }
}
