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

mod event;
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

    match shared::ffmpeg::duration_secs(input) {
        Some(d) => println!("{input}: {d:.2}s — readable, detection not implemented yet"),
        None => anyhow::bail!("could not read a duration from {input}"),
    }

    Ok(())
}
