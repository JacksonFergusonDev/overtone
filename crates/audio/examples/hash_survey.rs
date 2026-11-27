//! Hashes every audio file under a folder and reports successes and
//! failures per extension, to check the payload finders against real
//! libraries. Reads files only; never writes.
//!
//! ```text
//! cargo run --release -p overtone-audio --example hash_survey -- <dir>
//! ```
//!
//! The directory can also come from `OVERTONE_BENCH_DIR`. Also reports
//! files whose payload hash matches another file's, as a sanity check that
//! duplicates are real duplicates.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use overtone_audio::hash::hash_payload;
use rayon::prelude::*;

fn main() {
    let Some(dir) = std::env::args_os()
        .nth(1)
        .or_else(|| std::env::var_os("OVERTONE_BENCH_DIR"))
        .map(PathBuf::from)
    else {
        eprintln!("hash_survey: pass a directory or set OVERTONE_BENCH_DIR");
        std::process::exit(2);
    };
    let files: Vec<PathBuf> = walkdir::WalkDir::new(&dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(overtone_model::is_audio_extension)
        })
        .collect();

    let results: Vec<_> = files
        .par_iter()
        .map(|path| (path, hash_payload(path)))
        .collect();

    let mut per_extension: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_hash: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    let mut failures = Vec::new();
    for (path, result) in &results {
        let extension = path
            .extension()
            .map_or_else(String::new, |e| e.to_string_lossy().to_ascii_lowercase());
        let counts = per_extension.entry(extension).or_default();
        match result {
            Ok(payload) => {
                counts.0 += 1;
                by_hash.entry(payload.hash.clone()).or_default().push(path);
            }
            Err(err) => {
                counts.1 += 1;
                failures.push(err.to_string());
            }
        }
    }

    println!("| extension | ok | failed |\n|---|---|---|");
    for (extension, (ok, failed)) in &per_extension {
        println!("| {extension} | {ok} | {failed} |");
    }
    println!("\n{} failures", failures.len());
    for failure in &failures {
        println!("  {failure}");
    }
    // Payloads much smaller than their file usually mean big artwork, but
    // could also mean a finder stopped early, so list the extremes.
    let mut ratios: Vec<(f64, &PathBuf)> = results
        .iter()
        .filter_map(|(path, result)| {
            let payload = result.as_ref().ok()?.payload_len as f64;
            let len = std::fs::metadata(path).ok()?.len() as f64;
            Some((payload / len, *path))
        })
        .collect();
    ratios.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("\nsmallest payload share of file size");
    for (ratio, path) in ratios.iter().take(8) {
        println!("  {:.1}%  {}", ratio * 100.0, path.display());
    }

    let mut duplicates: Vec<_> = by_hash.values().filter(|paths| paths.len() > 1).collect();
    duplicates.sort();
    println!("\n{} groups of identical audio", duplicates.len());
    for group in duplicates {
        println!("  {}", group.len());
        for path in group {
            println!("    {}", path.display());
        }
    }
}
