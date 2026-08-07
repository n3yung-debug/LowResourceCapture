//! Offline VOD analyzer — finds kills and deaths in a recording so they can be
//! reviewed and edited on the same block timeline as saved clips.
//!
//! Ships as a **separate installer** from the recorder on purpose. The recorder
//! is a tiny always-resident process tuned for minimal footprint while gaming;
//! this runs offline, never during a session, and is free to use every core.
//!
//! Scope note: this analyzes video files you point it at (Twitch VODs, OBS or
//! NVIDIA recordings). It does not record anything itself — the recorder's
//! in-RAM ring buffer design is untouched by any of this.
//!
//! Detection status as of this commit: **skeleton only.** The event model and
//! profile format below are real and tested; the detectors are not written yet.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The event model and profile format are exercised by their unit tests but not
// yet wired into a detector, so every item reads as dead code. Remove this once
// L6b calls into them.
#![allow(dead_code)]

mod detect;
mod event;
mod frames;
mod labels;
mod profile;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("clipanalyzer {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|a| a == "--help" || a == "-h") || args.is_empty() {
        eprintln!(
            "clipanalyzer {}\n\
             \n\
             Usage: clipanalyzer <video-file>\n\
             \n\
             Scans a recording for kills and deaths, then opens them on the clip\n\
             timeline for editing.\n\
             \n\
             Detection is not implemented yet — this build only validates that a\n\
             file is readable and that the bundled ffmpeg is available.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let input = &args[0];
    if !std::path::Path::new(input).exists() {
        anyhow::bail!("no such file: {input}");
    }

    if !shared::ffmpeg::available() {
        anyhow::bail!(
            "bundled ffmpeg.exe not found next to the executable — the analyzer \
             cannot decode video without it"
        );
    }

    let duration = shared::ffmpeg::duration_secs(input)
        .ok_or_else(|| anyhow::anyhow!("could not read a duration from {input}"))?;
    let (w, h) = frames::dimensions(input)
        .ok_or_else(|| anyhow::anyhow!("could not read video dimensions from {input}"))?;
    println!("{input}: {w}x{h}, {duration:.1}s — scanning for deaths…");

    let started = std::time::Instant::now();
    let mut stream =
        frames::RoiStream::open(input, &detect::DEATH_ROI, w, h, detect::SAMPLE_FPS)?;
    let mut buf = Vec::new();
    let mut hits = Vec::new();
    let mut scanned = 0u64;
    while let Some(at) = stream.next_frame(&mut buf)? {
        scanned += 1;
        if let Some(ev) = detect::score_frame(&buf, at) {
            hits.push(ev);
        }
    }

    // A death card is on screen for seconds, so one death produces a run of
    // detections. Collapse each run into a single event.
    let deaths = event::cluster(hits, 3.0);

    println!(
        "scanned {scanned} frames in {:.1}s — {} death(s)",
        started.elapsed().as_secs_f64(),
        deaths.len()
    );

    // Fold into the label set beside the video, keeping any verdicts already
    // recorded. Re-running a re-tuned detector must never cost you a review.
    let label_path = std::path::Path::new(input).with_extension("labels.json");
    let mut set = if label_path.exists() {
        labels::LabelSet::load(&label_path)?
    } else {
        labels::LabelSet::new(input, "")
    };
    let added = set.merge_detections(deaths, 3.0);
    set.save(&label_path)?;

    let (ok, no, pending) = set.tally();
    println!(
        "labels: {added} new, {ok} confirmed, {no} rejected, {pending} awaiting review \
         -> {}",
        label_path.display()
    );

    let approved = set.confirmed();
    if approved.is_empty() {
        println!(
            "  nothing confirmed yet — the review UI (next layer) is where detections \
             become clips, and where your verdicts become training data"
        );
    } else {
        for (ev, (s, e)) in approved.iter().zip(
            event::segments(&approved, 12.0, 4.0, duration).iter(),
        ) {
            println!("  {:?} at {:>8.1}s  clip {:.1}s–{:.1}s", ev.kind, ev.at, s, e);
        }
    }

    Ok(())
}
