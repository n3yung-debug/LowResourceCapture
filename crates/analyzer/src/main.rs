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

mod dataset;
mod detect;
mod event;
mod export;
mod frames;
mod labels;
mod model;
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
             Usage: clipanalyzer [video-file] [options]\n\n\
             Run it with no arguments and it asks you to choose a recording.\n\n\
             It scans for deaths, then opens a review window where you confirm \
             or reject each one and mark any it missed. Verdicts save beside the \
             video as <video>.labels.json and survive re-scans, so a re-tuned \
             detector never costs you a review.\n\n\
             Player kills are marked by hand (K), monster kills with M, deaths \
             with D. Those marks are the training data for the kill detector.\n\n\
             Options:\n\
             \u{20} --scan-only            scan and print, no window\n\
             \u{20} --build <version>      tag this recording's marks with the game build\n\
             \u{20} --export-dataset <dir> write training frames from confirmed marks",
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

    // Which game build this footage is from. Recorded per-recording so a
    // dataset spanning several patches knows which examples came from which,
    // and a model can say what it was trained against.
    let build = flag_value(&args, "--build").unwrap_or_default();

    // Export training frames from the marks already confirmed on this video.
    if let Some(dir) = flag_value(&args, "--export-dataset") {
        return export_dataset(input, &dir, &build);
    }

    // Console mode: scan and print, no window.
    if args.iter().any(|a| a == "--scan-only") {
        let started = std::time::Instant::now();
        let deaths = detect::scan_deaths(input, |_| {})?;
        println!(
            "scanned in {:.1}s — {} death(s)",
            started.elapsed().as_secs_f64(),
            deaths.len()
        );

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
            "labels: {added} new, {ok} confirmed, {no} rejected, {pending} pending -> {}",
            label_path.display()
        );
        return Ok(());
    }

    // Otherwise open the window straight away and scan behind it. Scanning
    // first meant the app sat invisible for the length of a decode, which is
    // indistinguishable from never starting.
    review::run(input)
}

/// Value of a `--flag value` pair, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).filter(|v| !v.starts_with("--")).cloned()
}

/// Turn this recording's confirmed marks into training frames.
fn export_dataset(input: &str, dir: &str, build: &str) -> Result<()> {
    let label_path = review::label_path_for(input);
    if !label_path.exists() {
        anyhow::bail!(
            "no labels for {input} — review it first, then export.\n\
             Nothing is exported from an unreviewed recording: only marks you \
             confirmed become training data."
        );
    }
    let mut set = labels::LabelSet::load(&label_path)?;
    if !build.is_empty() && set.game_build != build {
        set.game_build = build.to_string();
        set.save(&label_path)?;
    }
    if set.game_build.is_empty() {
        log::warn!(
            "no game build recorded for {input}; pass --build <version> so the \
             dataset knows which patch this footage came from"
        );
    }

    let duration = shared::ffmpeg::duration_secs(input).unwrap_or(0.0);
    let (_, height) = frames::dimensions(input).unwrap_or((1920, 1080));
    let profile = load_profile();

    let root = std::path::Path::new(dir);
    let manifest = export::export(input, &set, profile.as_ref(), root, duration, height)?;

    println!("exported {} frames -> {}", manifest.examples.len(), root.display());
    for (class, n) in manifest.class_counts() {
        println!("  {class:10} {n}");
    }
    if manifest.examples.is_empty() {
        println!(
            "\nNothing to export: this recording has no *confirmed* marks yet. \
             Open it in the review window and press Y / K / M / D."
        );
    }
    Ok(())
}

/// Detection profile shipped next to the exe, if there is one.
fn load_profile() -> Option<profile::GameProfile> {
    let dir = shared::config::install_dir().ok()?.join("profiles");
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        if e.path().extension().is_some_and(|x| x == "toml") {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                if let Ok(p) = profile::GameProfile::from_toml(&text) {
                    return Some(p);
                }
            }
        }
    }
    None
}
