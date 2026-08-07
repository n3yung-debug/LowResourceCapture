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
//! Detection status: the death card is detected and validated; player kills are
//! marked by hand in the review window (see CLAUDE.md for why).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Parts of the profile format are exercised by unit tests but not yet read by a
// detector. Remove this once the kill detector consumes them.
#![allow(dead_code)]

mod detect;
mod event;
mod frames;
mod labels;
mod profile;
mod review;
mod winui;

use anyhow::Result;

fn main() {
    init_logging();
    if let Err(e) = run() {
        // No console in a windowed build, so an error has to be shown or it is
        // invisible — the app would simply appear not to start.
        log::error!("{e:#}");
        winui::error(&format!("{e:#}"));
        std::process::exit(1);
    }
}

fn init_logging() {
    let path = shared::config::install_dir()
        .map(|d| d.join("logs").join("clipanalyzer.log"))
        .unwrap_or_else(|_| std::env::temp_dir().join("clipanalyzer.log"));
    shared::logging::init(&path, false);
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        winui::info(&format!("ClipAnalyzer {}", env!("CARGO_PKG_VERSION")));
        return Ok(());
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        winui::info(&format!(
            "ClipAnalyzer {}\n\n\
             Usage: clipanalyzer [video-file] [--scan-only]\n\n\
             Run it with no arguments and it asks you to choose a recording.\n\n\
             It scans for deaths, then opens a review window where you confirm \
             or reject each one and mark any it missed. Verdicts save beside the \
             video as <video>.labels.json and survive re-scans, so a re-tuned \
             detector never costs you a review.\n\n\
             Only death detection exists so far — player kills are marked by hand \
             (press K at the moment), and those marks are what a kill detector \
             will eventually be trained on.",
            env!("CARGO_PKG_VERSION")
        ));
        return Ok(());
    }

    // Launched from the Start Menu or desktop shortcut there are no arguments,
    // so ask for a file rather than exiting silently.
    let positional = args.iter().find(|a| !a.starts_with("--"));
    let input = match positional {
        Some(p) => p.clone(),
        None => match winui::pick_video() {
            Some(p) => p,
            None => return Ok(()), // cancelled
        },
    };
    let input = &input;
    log::info!("analyzing {input}");

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
    let label_path = review::label_path_for(input);
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

    // Hand straight over to the review window — judging the detections is the
    // point, and every verdict made there is also a training example.
    if args.iter().any(|a| a == "--scan-only") {
        for ev in set.confirmed() {
            println!("  {:?} at {:>8.1}s", ev.kind, ev.at);
        }
        return Ok(());
    }
    review::run(input)
}
