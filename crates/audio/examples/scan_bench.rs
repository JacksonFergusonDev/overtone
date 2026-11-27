//! Measures `tags::read_metadata` throughput over a folder, in parallel.
//!
//! `cargo run --release -p overtone-audio --example scan_bench -- <folder> [passes]`
//!
//! Only reads files. The first pass is as cold as the OS page cache allows
//! without root; later passes are warm.

use std::path::{Path, PathBuf};
use std::time::Instant;

use overtone_model::is_audio_extension;
use rayon::prelude::*;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(is_audio_extension)
        {
            out.push(path);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: scan_bench <folder> [passes]"));
    let passes: usize = args.next().map_or(3, |n| n.parse().expect("passes"));
    let mut files = Vec::new();
    collect(&root, &mut files);
    println!(
        "{} audio files, {} threads",
        files.len(),
        rayon::current_num_threads()
    );
    for pass in 1..=passes {
        let start = Instant::now();
        let failures: Vec<String> = files
            .par_iter()
            .filter_map(|path| {
                overtone_audio::tags::read_metadata(path)
                    .err()
                    .map(|err| err.to_string())
            })
            .collect();
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "pass {pass}: {:.2} s, {:.0} files/s, {} failures",
            elapsed,
            files.len() as f64 / elapsed,
            failures.len()
        );
        for failure in failures.iter().take(5) {
            println!("  {failure}");
        }
    }
}
