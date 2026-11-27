//! Comment-write spike over a folder of real files, run on clones.
//!
//! `cargo run --release -p overtone-audio --example tag_spike -- <folder> [limit]`
//!
//! Every file is cloned (`cp -c`, APFS copy-on-write) into a temporary
//! folder and only the clone is written. The source folder is only read.
//! Prints one line per file and a summary of what survived.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overtone_audio::tags::raw::{self, Container};
use overtone_audio::tags::{self, WriteOutcome};
use overtone_fs::ProtectedRoots;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "aif" | "aiff" | "m4a"
            )
        }) {
            out.push(path);
        }
    }
}

fn container_of(path: &Path) -> Option<Container> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp3" => Some(Container::Mp3),
        "flac" => Some(Container::Flac),
        "wav" => Some(Container::Wav),
        "aif" | "aiff" => Some(Container::Aiff),
        _ => None,
    }
}

/// Descriptions of GEOB frames (Serato stores markers, beatgrid, overview,
/// and autotags in them) and owners of PRIV frames.
fn vendor_frames(layout: &raw::Layout) -> Vec<String> {
    let Some(tag) = &layout.id3v2 else {
        return Vec::new();
    };
    tag.frames
        .iter()
        .filter_map(|f| match f.id.as_str() {
            "GEOB" if f.body.first() == Some(&0) => {
                let parts: Vec<&[u8]> = f.body[1..].splitn(4, |b| *b == 0).collect();
                Some(format!(
                    "GEOB:{}",
                    String::from_utf8_lossy(parts.get(2).copied().unwrap_or_default())
                ))
            }
            "GEOB" => Some("GEOB:(utf16)".into()),
            "PRIV" => {
                let owner: Vec<u8> = f.body.iter().take_while(|b| **b != 0).copied().collect();
                Some(format!("PRIV:{}", String::from_utf8_lossy(&owner)))
            }
            "APIC" => Some("APIC".into()),
            _ => None,
        })
        .collect()
}

fn vorbis_vendor_keys(layout: &raw::Layout) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(flac) = &layout.flac {
        for block in &flac.blocks {
            if block.kind == 6 {
                out.push("PICTURE".into());
            }
            if block.kind == 2 {
                out.push(format!(
                    "APPLICATION:{}",
                    String::from_utf8_lossy(&block.body[..4.min(block.body.len())])
                ));
            }
        }
        if let Ok(Some(vc)) = flac.vorbis_comment() {
            for entry in &vc.entries {
                let key: Vec<u8> = entry.iter().take_while(|b| **b != b'=').copied().collect();
                let key = String::from_utf8_lossy(&key).to_uppercase();
                if key.starts_with("SERATO") || key.contains("TRAKTOR") {
                    out.push(key);
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Summary {
    by_format: BTreeMap<String, [usize; 3]>,
    refusals: BTreeMap<String, Vec<String>>,
    reencoded: BTreeMap<String, usize>,
    vendor: BTreeMap<String, usize>,
    versions: BTreeMap<String, usize>,
    reordered: usize,
    metadata_lost: Vec<String>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().expect("usage: tag_spike <folder> [limit]"));
    let limit: usize = args
        .next()
        .map_or(usize::MAX, |n| n.parse().expect("limit"));
    let mut files = Vec::new();
    collect(&source, &mut files);
    files.truncate(limit);
    let work = tempfile::TempDir::new().expect("tempdir");
    let roots = ProtectedRoots::from_environment();
    let mut summary = Summary::default();

    for (index, original) in files.iter().enumerate() {
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let clone = work.path().join(format!("{index}.{ext}"));
        let status = Command::new("cp")
            .arg("-c")
            .arg(original)
            .arg(&clone)
            .status()
            .expect("cp");
        assert!(status.success(), "clone failed for {}", original.display());
        // Give the clone a Finder tag and a known creation date to see what survives.
        let _ = xattr::set(
            &clone,
            "com.apple.metadata:_kMDItemUserTags",
            b"bplist00\xa1\x01UGreen\n\x08\x0a\x00\x00\x00\x00\x00\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x10",
        );
        let before_meta = std::fs::metadata(&clone).expect("stat");
        let before_xattrs: Vec<String> = xattr::list(&clone)
            .map_or_default(|l| l.map(|n| n.to_string_lossy().into_owned()).collect());

        let format = ext.replace("aif", "aiff").replace("aiffff", "aiff");
        let name = original
            .strip_prefix(&source)
            .unwrap_or(original)
            .display()
            .to_string();
        let counts = summary.by_format.entry(format.clone()).or_default();
        counts[0] += 1;

        let layout = container_of(&clone).map(|c| raw::inspect(&clone, c));
        if let Some(Ok(layout)) = &layout {
            for tag in vendor_frames(layout)
                .into_iter()
                .chain(vorbis_vendor_keys(layout))
            {
                *summary.vendor.entry(format!("{format} {tag}")).or_default() += 1;
            }
            if let Some(tag) = &layout.id3v2 {
                *summary
                    .versions
                    .entry(format!("{format} ID3v2.{}", tag.major))
                    .or_default() += 1;
            }
        }

        let current = tags::read_metadata(&clone)
            .ok()
            .and_then(|m| m.tags.comment);
        let desired = format!("{} #overtone-spike", current.as_deref().unwrap_or(""))
            .trim()
            .to_string();
        match tags::write_comment(&roots, &clone, Some(&desired)) {
            Ok(WriteOutcome::Written(report)) => {
                counts[1] += 1;
                for id in &report.reencoded_frames {
                    *summary
                        .reencoded
                        .entry(format!("{format} {id}"))
                        .or_default() += 1;
                }
                summary.reordered += usize::from(report.reordered);
                let after_meta = std::fs::metadata(&clone).expect("stat");
                let after_xattrs: Vec<String> = xattr::list(&clone)
                    .map_or_default(|l| l.map(|n| n.to_string_lossy().into_owned()).collect());
                let lost: Vec<&String> = before_xattrs
                    .iter()
                    .filter(|x| !after_xattrs.contains(x))
                    .collect();
                let created_same = before_meta.created().ok() == after_meta.created().ok();
                let perms_same = before_meta.permissions() == after_meta.permissions();
                if !lost.is_empty() || !created_same || !perms_same {
                    summary
                        .metadata_lost
                        .push(format!("{name}: xattrs lost {lost:?}, created kept {created_same}, perms kept {perms_same}"));
                }
                // Second identical write must be a no-op.
                let bytes = std::fs::read(&clone).expect("read");
                match tags::write_comment(&roots, &clone, Some(&desired)) {
                    Ok(WriteOutcome::Unchanged) => {}
                    other => println!("  !! second write not a no-op: {other:?}"),
                }
                assert_eq!(bytes, std::fs::read(&clone).expect("read"), "idempotence");
                // Restore the original value.
                if let Err(err) = tags::write_comment(&roots, &clone, current.as_deref()) {
                    println!("  !! restore failed: {err}");
                }
                println!(
                    "ok     {format:5} {name} (re-encoded {:?}, tag {}→{} bytes{})",
                    report.reencoded_frames,
                    report.tag_bytes_before,
                    report.tag_bytes_after,
                    if report.reordered { ", reordered" } else { "" }
                );
            }
            Ok(WriteOutcome::Unchanged) => println!("same   {format:5} {name}"),
            Err(err) => {
                counts[2] += 1;
                let reason = err.to_string();
                let short = reason
                    .split_once(": ")
                    .map_or(reason.as_str(), |(_, rest)| rest)
                    .replace(&clone.display().to_string(), "<file>");
                let key = match &err {
                    overtone_audio::AudioError::WriteRefused { reason, .. } => {
                        format!("{format}: refused: {reason}")
                    }
                    overtone_audio::AudioError::VerificationFailed { check, detail, .. } => {
                        format!(
                            "{format}: {check} check failed: {}",
                            detail.chars().take(160).collect::<String>()
                        )
                    }
                    other => format!(
                        "{format}: {}",
                        other
                            .to_string()
                            .replace(&clone.display().to_string(), "<file>")
                    ),
                };
                summary.refusals.entry(key).or_default().push(name.clone());
                println!("FAIL   {format:5} {name}: {short}");
            }
        }
        let _ = std::fs::remove_file(&clone);
        std::thread::sleep(Duration::from_millis(0));
    }

    println!("\n== Per format: files, written, refused/failed");
    for (format, [n, ok, bad]) in &summary.by_format {
        println!("{format:5} {n:5} {ok:5} {bad:5}");
    }
    println!("\n== ID3 versions");
    for (k, v) in &summary.versions {
        println!("{k}: {v}");
    }
    println!("\n== Vendor frames/blocks present (files)");
    for (k, v) in &summary.vendor {
        println!("{k}: {v}");
    }
    println!("\n== Text frames re-encoded (bytes differ, text identical)");
    for (k, v) in &summary.reencoded {
        println!("{k}: {v}");
    }
    println!(
        "\nFiles with reordered frames/blocks: {}",
        summary.reordered
    );
    println!("\n== Refusals and failures");
    for (k, v) in &summary.refusals {
        println!("{k} ({} files)", v.len());
        for name in v.iter().take(5) {
            println!("    {name}");
        }
    }
    println!("\n== File metadata changes");
    for line in summary.metadata_lost.iter().take(20) {
        println!("{line}");
    }
    println!("({} files)", summary.metadata_lost.len());
}
