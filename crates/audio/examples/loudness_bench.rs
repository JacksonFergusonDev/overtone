//! Cost of EBU R128 metering next to decoding and analysis, per file,
//! single-threaded. `cargo run --release -p overtone-audio --example loudness_bench -- <files...>`
use std::time::Instant;

use overtone_audio::analysis::{AnalysisOptions, Decoder, analyze};

fn main() {
    let (mut t_an, mut secs) = (0.0, 0.0);
    let modes = [
        ("I", ebur128::Mode::I),
        ("I+LRA", ebur128::Mode::I | ebur128::Mode::LRA),
        ("SAMPLE_PEAK", ebur128::Mode::SAMPLE_PEAK),
        ("TRUE_PEAK", ebur128::Mode::TRUE_PEAK),
    ];
    let mut times = vec![0.0f64; modes.len()];
    for path in std::env::args().skip(1) {
        let p = std::path::Path::new(&path);
        let start = Instant::now();
        analyze(p, &AnalysisOptions::default()).expect("analyze");
        t_an += start.elapsed().as_secs_f64();
        let mut d = Decoder::open(p).expect("open");
        let (rate, ch) = (d.info().sample_rate_hz, u32::from(d.info().channels));
        let mut all = Vec::new();
        while let Ok(Some(b)) = d.next_block() {
            all.extend_from_slice(b.samples);
        }
        secs += all.len() as f64 / f64::from(ch) / f64::from(rate);
        for ((_, modes), t) in modes.iter().zip(times.iter_mut()) {
            let modes = *modes;
            let start = Instant::now();
            let mut m = ebur128::EbuR128::new(ch, rate, modes).expect("meter");
            for chunk in all.chunks(4096 * ch as usize) {
                m.add_frames_f32(chunk).expect("add");
            }
            let _ = m.loudness_global();
            *t += start.elapsed().as_secs_f64();
        }
    }
    println!("audio {:.0} min | full analysis {t_an:.2} s", secs / 60.0);
    for ((name, _), t) in modes.iter().zip(&times) {
        println!("  {name}: {t:.2} s");
    }
}
