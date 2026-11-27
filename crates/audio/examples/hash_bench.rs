//! Compares ways of hashing audio files, to choose the I/O strategy behind
//! `overtone_audio::hash`. Reads files only; never writes.
//!
//! ```text
//! cargo run --release -p overtone-audio --example hash_bench -- <dir> [options]
//!   --max-mb N      only use the first N MB of files (sorted by path), to
//!                   fit the set in the page cache for warm runs
//!   --modes a,b     modes to run (default: all)
//!   --repeat N      passes per mode (default: 1); with a set that fits in
//!                   memory, pass 1 warms the cache
//!   --threads N     rayon threads for parallel runs (default: all cores)
//! ```
//!
//! The directory can also come from `OVERTONE_BENCH_DIR`. Results are
//! recorded in `engineering/decisions/content-hashing.md`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use overtone_audio::hash::{audio_hash, payload_layout};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Finding payload ranges only, without hashing.
    Layout,
    /// The production path: `audio_hash`, 64 KiB reads of payload ranges.
    Payload,
    /// Payload ranges with 1 MiB reads.
    Payload1m,
    /// Payload ranges through `blake3::Hasher::update_reader`.
    PayloadUpdateReader,
    /// Payload ranges with xxh3-128 and 1 MiB reads.
    PayloadXxh3,
    /// Whole file through `Hasher::update_mmap` (reference ceiling).
    WholeMmap,
    /// Whole file through `Hasher::update_mmap_rayon`.
    WholeMmapRayon,
}

const ALL_MODES: [Mode; 7] = [
    Mode::Layout,
    Mode::Payload,
    Mode::Payload1m,
    Mode::PayloadUpdateReader,
    Mode::PayloadXxh3,
    Mode::WholeMmap,
    Mode::WholeMmapRayon,
];

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Payload => "payload",
            Self::Payload1m => "payload-1m",
            Self::PayloadUpdateReader => "payload-update-reader",
            Self::PayloadXxh3 => "payload-xxh3",
            Self::WholeMmap => "whole-mmap",
            Self::WholeMmapRayon => "whole-mmap-rayon",
        }
    }

    /// Bytes this mode reads from `path`, or `None` if it can't hash it.
    fn run(self, path: &Path) -> Option<u64> {
        match self {
            Self::Layout => payload_layout(path).ok().map(|_| 0),
            Self::Payload => {
                audio_hash(path).ok()?;
                payload_layout(path).ok().map(|l| l.payload_len())
            }
            Self::Payload1m => hash_payload_ranges(path, 1 << 20, Hasher::Blake3),
            Self::PayloadXxh3 => hash_payload_ranges(path, 1 << 20, Hasher::Xxh3),
            Self::PayloadUpdateReader => {
                let layout = payload_layout(path).ok()?;
                let mut file = File::open(path).ok()?;
                let mut hasher = blake3::Hasher::new();
                for range in &layout.ranges {
                    file.seek(SeekFrom::Start(range.start)).ok()?;
                    hasher
                        .update_reader((&mut file).take(range.end - range.start))
                        .ok()?;
                }
                std::hint::black_box(hasher.finalize());
                Some(layout.payload_len())
            }
            Self::WholeMmap => {
                let mut hasher = blake3::Hasher::new();
                hasher.update_mmap(path).ok()?;
                std::hint::black_box(hasher.finalize());
                std::fs::metadata(path).ok().map(|m| m.len())
            }
            Self::WholeMmapRayon => {
                let mut hasher = blake3::Hasher::new();
                hasher.update_mmap_rayon(path).ok()?;
                std::hint::black_box(hasher.finalize());
                std::fs::metadata(path).ok().map(|m| m.len())
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Hasher {
    Blake3,
    Xxh3,
}

fn hash_payload_ranges(path: &Path, buffer_len: usize, kind: Hasher) -> Option<u64> {
    let layout = payload_layout(path).ok()?;
    let mut file = File::open(path).ok()?;
    let mut buffer = vec![0_u8; buffer_len];
    let mut blake = blake3::Hasher::new();
    let mut xxh = xxhash_rust::xxh3::Xxh3::new();
    for range in &layout.ranges {
        read_range(&mut file, range.clone(), &mut buffer, |chunk| match kind {
            Hasher::Blake3 => {
                blake.update(chunk);
            }
            Hasher::Xxh3 => xxh.update(chunk),
        })?;
    }
    match kind {
        Hasher::Blake3 => {
            std::hint::black_box(blake.finalize());
        }
        Hasher::Xxh3 => {
            std::hint::black_box(xxh.digest128());
        }
    }
    Some(layout.payload_len())
}

fn read_range(
    file: &mut File,
    range: Range<u64>,
    buffer: &mut [u8],
    mut sink: impl FnMut(&[u8]),
) -> Option<()> {
    file.seek(SeekFrom::Start(range.start)).ok()?;
    let mut remaining = range.end - range.start;
    while remaining > 0 {
        let step = usize::try_from(remaining).map_or(buffer.len(), |n| n.min(buffer.len()));
        file.read_exact(&mut buffer[..step]).ok()?;
        sink(&buffer[..step]);
        remaining -= step as u64;
    }
    Some(())
}

struct Options {
    dir: PathBuf,
    max_bytes: Option<u64>,
    modes: Vec<Mode>,
    repeat: usize,
    threads: Option<usize>,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut dir = std::env::var_os("OVERTONE_BENCH_DIR").map(PathBuf::from);
    let mut options = Options {
        dir: PathBuf::new(),
        max_bytes: None,
        modes: ALL_MODES.to_vec(),
        repeat: 1,
        threads: None,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--max-mb" => {
                let mb: u64 = value()?.parse().map_err(|e| format!("--max-mb: {e}"))?;
                options.max_bytes = Some(mb * 1_000_000);
            }
            "--repeat" => options.repeat = value()?.parse().map_err(|e| format!("{e}"))?,
            "--threads" => options.threads = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--modes" => {
                options.modes = value()?
                    .split(',')
                    .map(|name| {
                        ALL_MODES
                            .into_iter()
                            .find(|m| m.name() == name)
                            .ok_or(format!("unknown mode {name}"))
                    })
                    .collect::<Result<_, _>>()?;
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option {arg}")),
            _ => dir = Some(PathBuf::from(arg)),
        }
    }
    options.dir = dir.ok_or("pass a directory or set OVERTONE_BENCH_DIR")?;
    Ok(options)
}

fn audio_files(dir: &Path, max_bytes: Option<u64>) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<(PathBuf, u64)> = walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy();
            !name.starts_with('.')
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(overtone_model::is_audio_extension)
        })
        .filter_map(|entry| Some((entry.path().to_path_buf(), entry.metadata().ok()?.len())))
        .collect();
    if let Some(limit) = max_bytes {
        let mut total = 0;
        files.retain(|(_, len)| {
            total += len;
            total <= limit
        });
    }
    files
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("hash_bench: {message}");
            std::process::exit(2);
        }
    };
    if let Some(threads) = options.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .expect("thread pool");
    }
    let files = audio_files(&options.dir, options.max_bytes);
    let file_bytes: u64 = files.iter().map(|(_, len)| len).sum();
    println!(
        "{} files, {:.1} GB on disk, {} rayon threads",
        files.len(),
        file_bytes as f64 / 1e9,
        rayon::current_num_threads()
    );
    println!(
        "| mode | threads | pass | files ok | GB hashed | seconds | MB/s |\n|---|---|---|---|---|---|---|"
    );
    for &mode in &options.modes {
        for parallel in [false, true] {
            // update_mmap_rayon already uses every core within a file.
            if parallel && mode == Mode::WholeMmapRayon {
                continue;
            }
            for pass in 1..=options.repeat {
                let start = Instant::now();
                let results: Vec<Option<u64>> = if parallel {
                    files.par_iter().map(|(path, _)| mode.run(path)).collect()
                } else {
                    files.iter().map(|(path, _)| mode.run(path)).collect()
                };
                let seconds = start.elapsed().as_secs_f64();
                let ok = results.iter().flatten().count();
                let bytes: u64 = results.iter().flatten().sum();
                let rate = if bytes == 0 {
                    String::from("-")
                } else {
                    format!("{:.0}", bytes as f64 / 1e6 / seconds)
                };
                println!(
                    "| {} | {} | {pass} | {ok}/{} | {:.2} | {seconds:.2} | {rate} |",
                    mode.name(),
                    if parallel {
                        rayon::current_num_threads()
                    } else {
                        1
                    },
                    files.len(),
                    bytes as f64 / 1e9,
                );
            }
        }
    }
}
