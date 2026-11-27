//! Waveform peaks for the UI's scrub bar.
//!
//! The track length isn't reliably known up front (VBR MP3s, truncated
//! files), so peaks stream into fine blocks of min/max, and whenever the
//! block list grows past a bound adjacent pairs merge, doubling the block
//! length. Memory stays under a few hundred kilobytes for any track length,
//! and the final reduction to [`WAVEFORM_BUCKETS`] maps blocks to buckets by
//! index. Each block takes the minimum and maximum across all channels, so
//! a clip in one channel still shows.

#![allow(
    clippy::cast_possible_truncation,
    reason = "sample values are clamped to -1..=1 before scaling to i8"
)]

/// Buckets in a finished waveform (fewer only for very short audio).
pub const WAVEFORM_BUCKETS: usize = 1200;
/// Version byte at the start of [`Waveform::encode`] output.
pub const WAVEFORM_ENCODING_VERSION: u8 = 1;

const INITIAL_BLOCK_FRAMES: u64 = 16;
const MAX_BLOCKS: usize = WAVEFORM_BUCKETS * 8;

/// Min/max amplitude pairs across the track, each in -1.0..=1.0.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Waveform {
    /// `[min, max]` per bucket, in time order.
    pub buckets: Vec<[f32; 2]>,
}

impl Waveform {
    /// Compact encoding for storage: one version byte
    /// ([`WAVEFORM_ENCODING_VERSION`]), then for each bucket its min and max
    /// as signed 8-bit values (`round(x * 127)`). About 2.4 KB per track.
    /// 8 bits is plenty for a scrub bar a few hundred pixels tall.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.buckets.len() * 2);
        out.push(WAVEFORM_ENCODING_VERSION);
        for [min, max] in &self.buckets {
            out.push(quantize(*min).to_le_bytes()[0]);
            out.push(quantize(*max).to_le_bytes()[0]);
        }
        out
    }

    /// Decodes [`Waveform::encode`] output; `None` for an unknown version or
    /// a truncated buffer.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&version, body) = bytes.split_first()?;
        if version != WAVEFORM_ENCODING_VERSION || body.len() % 2 != 0 {
            return None;
        }
        let buckets = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                [
                    f32::from(i8::from_le_bytes([pair[0]])) / 127.0,
                    f32::from(i8::from_le_bytes([pair[1]])) / 127.0,
                ]
            })
            .collect();
        Some(Self { buckets })
    }
}

fn quantize(value: f32) -> i8 {
    (value.clamp(-1.0, 1.0) * 127.0).round() as i8
}

pub(crate) struct WaveformBuilder {
    block_frames: u64,
    blocks: Vec<[f32; 2]>,
    current: [f32; 2],
    current_frames: u64,
}

impl WaveformBuilder {
    pub fn new() -> Self {
        Self {
            block_frames: INITIAL_BLOCK_FRAMES,
            blocks: Vec::with_capacity(MAX_BLOCKS),
            current: [f32::INFINITY, f32::NEG_INFINITY],
            current_frames: 0,
        }
    }

    /// Adds one frame's min and max across channels.
    pub fn push(&mut self, min: f32, max: f32) {
        self.current[0] = self.current[0].min(min);
        self.current[1] = self.current[1].max(max);
        self.current_frames += 1;
        if self.current_frames == self.block_frames {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.blocks.push(self.current);
        self.current = [f32::INFINITY, f32::NEG_INFINITY];
        self.current_frames = 0;
        if self.blocks.len() >= MAX_BLOCKS {
            self.blocks = self
                .blocks
                .chunks(2)
                .map(|pair| merge(pair.iter().copied()))
                .collect();
            self.block_frames *= 2;
        }
    }

    pub fn finish(mut self) -> Waveform {
        if self.current_frames > 0 {
            self.blocks.push(self.current);
        }
        let count = self.blocks.len();
        if count <= WAVEFORM_BUCKETS {
            return Waveform {
                buckets: self.blocks,
            };
        }
        let buckets = (0..WAVEFORM_BUCKETS)
            .map(|b| {
                let lo = b * count / WAVEFORM_BUCKETS;
                let hi = ((b + 1) * count / WAVEFORM_BUCKETS).max(lo + 1);
                merge(self.blocks[lo..hi].iter().copied())
            })
            .collect();
        Waveform { buckets }
    }
}

fn merge(blocks: impl Iterator<Item = [f32; 2]>) -> [f32; 2] {
    blocks.fold([f32::INFINITY, f32::NEG_INFINITY], |acc, [min, max]| {
        [acc[0].min(min), acc[1].max(max)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_long_input_to_fixed_bucket_count() {
        let mut builder = WaveformBuilder::new();
        for i in 0..2_000_000_u32 {
            let value = if i % 1000 == 0 { 0.9 } else { 0.1 };
            builder.push(-value, value);
        }
        let waveform = builder.finish();
        assert_eq!(waveform.buckets.len(), WAVEFORM_BUCKETS);
        assert!(waveform.buckets.iter().all(|[min, max]| min <= max));
        assert!(
            waveform
                .buckets
                .iter()
                .all(|[_, max]| (*max - 0.9).abs() < 1e-6)
        );
    }

    #[test]
    fn encoding_roundtrips_within_quantization() {
        let waveform = Waveform {
            buckets: vec![[-1.0, 1.0], [-0.5, 0.25], [0.0, 0.0]],
        };
        let bytes = waveform.encode();
        assert_eq!(bytes.len(), 7);
        let back = Waveform::decode(&bytes).expect("decode");
        for (a, b) in waveform.buckets.iter().zip(&back.buckets) {
            assert!((a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01);
        }
        assert!(Waveform::decode(&[9, 0, 0]).is_none());
        assert!(Waveform::decode(&[1, 0]).is_none());
    }
}
