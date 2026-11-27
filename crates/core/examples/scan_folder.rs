//! Scans a folder into an explicitly named library file and prints the
//! report, for manual checks against real collections. Reads audio files
//! only; writes nothing but the library file you name.
//!
//! ```text
//! cargo run --release -p overtone-core --example scan_folder -- <folder> <library.sqlite>
//! ```
//!
//! Run it twice to time an incremental rescan.

use std::path::PathBuf;
use std::time::Instant;

use overtone_core::library::Library;
use overtone_core::scan::{ScanProgress, add_root, scan_library};

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [folder, library_path] = args.as_slice() else {
        eprintln!("usage: scan_folder <folder> <library.sqlite>");
        std::process::exit(2);
    };
    let mut library = Library::open(library_path).expect("open library");
    add_root(&library, folder).expect("add folder");

    let started = Instant::now();
    let report = scan_library(&mut library, &|progress| {
        if let ScanProgress::Saving { done, total } = progress {
            eprintln!("saved {done}/{total}");
        }
    })
    .expect("scan");
    println!(
        "added {}, updated {}, unchanged {}, missing {}, failed {}, unavailable roots {:?}",
        report.added,
        report.updated,
        report.unchanged,
        report.missing,
        report.failed.len(),
        report.unavailable_roots
    );
    for failure in &report.failed {
        println!("  {}: {}", failure.path, failure.reason);
    }
    println!(
        "elapsed {} ms (wall {:.2} s)",
        report.elapsed_ms,
        started.elapsed().as_secs_f64()
    );
    println!("{:?}", library.summary().expect("summary"));
}
