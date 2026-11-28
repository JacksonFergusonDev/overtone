//! Compares Symphonia with ffmpeg over a pipe (`overtone-ffmpeg`) on real
//! files: decode time, sample counts, and the largest sample difference.
//!
//! `cargo run --release -p overtone-audio --example decode_bench -- <folder> [per-format]`
//!
//! Files are only read. Single-threaded, so the numbers compare per-file
//! cost; batch work runs one file per core either way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use overtone_audio::analysis::Decoder;
use overtone_ffmpeg::{DecodeOptions, Ffmpeg};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn symphonia(path: &Path) -> Option<(Vec<f32>, Duration)> {
    let start = Instant::now();
    let mut decoder = Decoder::open(path).ok()?;
    let mut samples = Vec::new();
    while let Ok(Some(block)) = decoder.next_block() {
        samples.extend_from_slice(block.samples);
    }
    Some((samples, start.elapsed()))
}

fn ffmpeg_decode(ffmpeg: &Ffmpeg, path: &Path) -> Option<(Vec<f32>, Duration)> {
    let start = Instant::now();
    let mut pcm = ffmpeg.decode(path, &DecodeOptions::default()).ok()?;
    let samples = pcm.read_to_end().ok()?;
    pcm.finish().ok()?;
    Some((samples, start.elapsed()))
}

#[derive(Default)]
struct Row {
    files: usize,
    seconds_audio: f64,
    symphonia: Duration,
    ffmpeg: Duration,
    symphonia_failed: usize,
    ffmpeg_failed: usize,
    length_mismatch: usize,
    max_diff: f32,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = PathBuf::from(
        args.next()
            .expect("usage: decode_bench <folder> [per-format]"),
    );
    let per_format: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);
    let ffmpeg = Ffmpeg::locate().expect("ffmpeg");
    println!("ffmpeg {} at {}", ffmpeg.version(), ffmpeg.path().display());

    let spawn_start = Instant::now();
    for _ in 0..20 {
        Ffmpeg::at(ffmpeg.path()).expect("probe");
    }
    println!(
        "process spawn + `-version`: {:.1} ms each",
        spawn_start.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    let mut files = Vec::new();
    collect(&folder, &mut files);
    let mut by_ext: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in files {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if matches!(
            ext.as_str(),
            "flac" | "mp3" | "wav" | "aif" | "aiff" | "m4a"
        ) {
            let list = by_ext.entry(ext).or_default();
            if list.len() < per_format {
                list.push(path);
            }
        }
    }

    println!(
        "\n| Format | Files | Audio (min) | Symphonia (s) | ffmpeg pipe (s) | ffmpeg / Symphonia | Symphonia failed | ffmpeg failed | Length differs | Max sample diff |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for (ext, paths) in &by_ext {
        let mut row = Row::default();
        for path in paths {
            row.files += 1;
            let s = symphonia(path);
            let f = ffmpeg_decode(&ffmpeg, path);
            if let Some((_, t)) = &s {
                row.symphonia += *t;
            } else {
                row.symphonia_failed += 1;
            }
            if let Some((_, t)) = &f {
                row.ffmpeg += *t;
            } else {
                row.ffmpeg_failed += 1;
            }
            if let (Some((a, _)), Some((b, _))) = (&s, &f) {
                if a.len() == b.len() {
                    let diff = a
                        .iter()
                        .zip(b)
                        .map(|(x, y)| (x - y).abs())
                        .fold(0f32, f32::max);
                    row.max_diff = row.max_diff.max(diff);
                } else {
                    row.length_mismatch += 1;
                }
                if let Ok(d) = Decoder::open(path) {
                    let info = d.info();
                    row.seconds_audio += a.len() as f64
                        / f64::from(info.channels.max(1))
                        / f64::from(info.sample_rate_hz.max(1));
                }
            }
        }
        let ratio = row.ffmpeg.as_secs_f64() / row.symphonia.as_secs_f64().max(1e-9);
        println!(
            "| {ext} | {} | {:.1} | {:.2} | {:.2} | {ratio:.2}x | {} | {} | {} | {:.2e} |",
            row.files,
            row.seconds_audio / 60.0,
            row.symphonia.as_secs_f64(),
            row.ffmpeg.as_secs_f64(),
            row.symphonia_failed,
            row.ffmpeg_failed,
            row.length_mismatch,
            row.max_diff
        );
    }
}
