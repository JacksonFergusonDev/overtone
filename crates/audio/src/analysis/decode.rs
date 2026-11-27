//! Streaming, read-only decoding with symphonia.
//!
//! The decoder yields interleaved `f32` blocks as they come out of
//! symphonia, so analysis never holds a whole track in memory (a 10-minute
//! 96 kHz stereo file would be about 460 MB as `f32`). Files are opened
//! read-only; nothing here can modify audio.
//!
//! Known symphonia 0.6.1 gaps are turned into [`AudioError::UnsupportedCodec`]
//! up front instead of producing wrong samples:
//! - HE-AAC and HE-AACv2 (SBR, parametric stereo) aren't implemented, so
//!   such streams would decode as their half-rate core.
//! - 32-bit FLAC can decode wrongly (symphonia issue #585).

use std::fs::File;
use std::path::{Path, PathBuf};

use overtone_model::AudioFormat;
use symphonia::core::codecs::audio::well_known::profiles::{
    CODEC_PROFILE_AAC_HE, CODEC_PROFILE_AAC_HE_V2, CODEC_PROFILE_AAC_USAC,
};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3,
};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::AudioError;

/// Read buffer for the media source. Larger than symphonia's default to cut
/// syscalls on big lossless files.
const READ_BUFFER_BYTES: usize = 1 << 20;

/// Consecutive undecodable packets tolerated before the file counts as
/// corrupt. Isolated bad packets (a damaged MP3 frame) are skipped.
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 100_000;

/// What the container and codec headers declare about a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// Codec family, as decoded rather than as named by the file extension.
    pub format: AudioFormat,
    /// Container short name from symphonia, such as `flac`, `wave`, or `isomp4`.
    pub container: &'static str,
    pub sample_rate_hz: u32,
    pub channels: u16,
    /// Declared bits per sample for PCM-style codecs; `None` for lossy codecs.
    pub bits_per_sample: Option<u32>,
    /// Frame count from the container, when it declares one.
    pub frames_hint: Option<u64>,
}

/// A block of decoded audio.
#[derive(Debug)]
pub struct Block<'a> {
    /// Interleaved samples, nominally in -1.0..=1.0 (lossy decoders can overshoot).
    pub samples: &'a [f32],
    pub channels: usize,
    pub sample_rate: u32,
}

/// A streaming decoder over one file's default audio track.
pub struct Decoder {
    path: PathBuf,
    reader: Box<dyn FormatReader>,
    codec: Box<dyn AudioDecoder>,
    track_id: u32,
    info: StreamInfo,
    samples: Vec<f32>,
    packet_bytes: u64,
    decoded_frames: u64,
    skipped_packets: u64,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("path", &self.path)
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Decoder {
    /// Opens `path` read-only and prepares a decoder for its default audio track.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let file = File::open(path).map_err(|source| AudioError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let stream = MediaSourceStream::new(
            Box::new(file),
            MediaSourceStreamOptions {
                buffer_len: READ_BUFFER_BYTES,
            },
        );
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let reader = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|err| decode_error(path, &err))?;

        let container = reader.format_info().short_name;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Decode {
                path: path.to_path_buf(),
                message: "no audio track".into(),
            })?;
        let track_id = track.id;
        let frames_hint = track.num_frames;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or_else(|| AudioError::UnsupportedCodec {
                path: path.to_path_buf(),
                reason: "the audio track has no codec parameters".into(),
            })?
            .clone();

        let format = classify(params.codec, container);
        if params.codec == CODEC_ID_AAC {
            if let Some(profile) = params.profile
                && [
                    CODEC_PROFILE_AAC_HE,
                    CODEC_PROFILE_AAC_HE_V2,
                    CODEC_PROFILE_AAC_USAC,
                ]
                .contains(&profile)
            {
                return Err(AudioError::UnsupportedCodec {
                    path: path.to_path_buf(),
                    reason: "HE-AAC isn't supported by the decoder yet".into(),
                });
            }
            // Implicitly signalled HE-AAC declares only its half-rate core.
            if params.sample_rate.is_some_and(|rate| rate <= 24_000) {
                return Err(AudioError::UnsupportedCodec {
                    path: path.to_path_buf(),
                    reason: "a low-rate AAC stream is probably HE-AAC, which the decoder can't \
                             reproduce yet"
                        .into(),
                });
            }
        }
        if params.codec == CODEC_ID_FLAC && params.bits_per_sample == Some(32) {
            return Err(AudioError::UnsupportedCodec {
                path: path.to_path_buf(),
                reason: "32-bit FLAC decodes unreliably in this decoder version".into(),
            });
        }

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|err| AudioError::UnsupportedCodec {
                path: path.to_path_buf(),
                reason: err.to_string(),
            })?;

        let info = StreamInfo {
            format,
            container,
            sample_rate_hz: params.sample_rate.unwrap_or(0),
            channels: params
                .channels
                .as_ref()
                .and_then(|c| u16::try_from(c.count()).ok())
                .unwrap_or(0),
            bits_per_sample: if format.is_lossless() {
                params.bits_per_sample
            } else {
                None
            },
            frames_hint,
        };
        Ok(Self {
            path: path.to_path_buf(),
            reader,
            codec: decoder,
            track_id,
            info,
            samples: Vec::new(),
            packet_bytes: 0,
            decoded_frames: 0,
            skipped_packets: 0,
        })
    }

    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// Compressed bytes read so far for this track, for the true average bitrate.
    pub fn packet_bytes(&self) -> u64 {
        self.packet_bytes
    }

    /// Frames decoded so far.
    pub fn decoded_frames(&self) -> u64 {
        self.decoded_frames
    }

    /// Damaged packets skipped so far.
    pub fn skipped_packets(&self) -> u64 {
        self.skipped_packets
    }

    /// Decodes the next block, or returns `None` at the end of the stream.
    ///
    /// A truncated file ends the stream early rather than failing, so a file
    /// cut short still gets analyzed on what it holds.
    pub fn next_block(&mut self) -> Result<Option<Block<'_>>, AudioError> {
        let mut consecutive_errors = 0;
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(SymphoniaError::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::DecodeError(_)) => {
                    consecutive_errors += 1;
                    self.skipped_packets += 1;
                    if consecutive_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        return Err(self.corrupt("too many unreadable packets"));
                    }
                    continue;
                }
                Err(err) => return Err(decode_error(&self.path, &err)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            self.packet_bytes += packet.data.len() as u64;
            match self.codec.decode(&packet) {
                Ok(buffer) => {
                    let channels = buffer.spec().channels().count();
                    if buffer.frames() == 0 || channels == 0 {
                        continue;
                    }
                    if self.info.sample_rate_hz == 0 {
                        self.info.sample_rate_hz = buffer.spec().rate();
                    }
                    if self.info.channels == 0 {
                        self.info.channels = u16::try_from(channels).unwrap_or(u16::MAX);
                    }
                    buffer.copy_to_vec_interleaved(&mut self.samples);
                    self.decoded_frames += buffer.frames() as u64;
                    return Ok(Some(Block {
                        samples: &self.samples,
                        channels,
                        sample_rate: self.info.sample_rate_hz,
                    }));
                }
                Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => {
                    consecutive_errors += 1;
                    self.skipped_packets += 1;
                    if consecutive_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        return Err(self.corrupt("too many undecodable packets"));
                    }
                }
                Err(err) => return Err(decode_error(&self.path, &err)),
            }
        }
    }

    fn corrupt(&self, message: &str) -> AudioError {
        AudioError::Decode {
            path: self.path.clone(),
            message: message.into(),
        }
    }
}

fn classify(codec: symphonia::core::codecs::audio::AudioCodecId, container: &str) -> AudioFormat {
    if codec == CODEC_ID_FLAC {
        AudioFormat::Flac
    } else if codec == CODEC_ID_ALAC {
        AudioFormat::Alac
    } else if [CODEC_ID_MP3, CODEC_ID_MP2, CODEC_ID_MP1].contains(&codec) {
        AudioFormat::Mp3
    } else if codec == CODEC_ID_AAC {
        AudioFormat::Aac
    } else {
        // Every other codec symphonia is built with here is PCM, so the
        // container decides.
        match container {
            "wave" => AudioFormat::Wav,
            "aiff" => AudioFormat::Aiff,
            _ => AudioFormat::Other,
        }
    }
}

fn decode_error(path: &Path, err: &SymphoniaError) -> AudioError {
    match err {
        SymphoniaError::Unsupported(what) => AudioError::UnsupportedCodec {
            path: path.to_path_buf(),
            reason: format!("unsupported {what}"),
        },
        other => AudioError::Decode {
            path: path.to_path_buf(),
            message: other.to_string(),
        },
    }
}
