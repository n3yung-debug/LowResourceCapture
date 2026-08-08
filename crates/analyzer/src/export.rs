//! Writes the training dataset to disk: label store → JPEG frames + manifest.
//!
//! Windows/ffmpeg-dependent glue. The decisions worth testing (which offsets,
//! which negatives, how classes map) live in `dataset.rs` and are covered
//! there; this file is the plumbing that runs them.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::dataset::{
    self, Example, Manifest, CLASS_NONE, FRAME_H, FRAME_W, NEGATIVE_MIN_GAP, SAMPLE_FPS, WINDOW,
};
use crate::labels::LabelSet;
use crate::profile::GameProfile;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Blackout filter for burnt-in overlays (webcam, alerts, stream banner).
///
/// Without this the model can learn the webcam instead of the game — and worse,
/// learn it as a *shortcut*, since the overlay is present in exactly the
/// recordings the training data comes from. Blacking it out is consistent
/// across sources, which a crop wouldn't be.
fn blackout_filter(profile: Option<&GameProfile>, height: u32) -> String {
    let Some(p) = profile else { return String::new() };
    p.exclude
        .regions
        .iter()
        .filter(|r| r.applies_to(height))
        .map(|r| {
            format!(
                ",drawbox=x=iw*{:.5}:y=ih*{:.5}:w=iw*{:.5}:h=ih*{:.5}:color=black:t=fill",
                r.x, r.y, r.w, r.h
            )
        })
        .collect()
}

/// Extract the frames for one event into `dir`, returning their file names.
fn extract_window(
    video: &str,
    at: f64,
    dir: &Path,
    stem: &str,
    blackout: &str,
) -> Result<Vec<(String, f64)>> {
    let ff = shared::config::ffmpeg_path()
        .context("bundled ffmpeg.exe not found next to the executable")?;
    std::fs::create_dir_all(dir)?;

    let start = (at - WINDOW).max(0.0);
    let dur = WINDOW * 2.0;
    let pattern = dir.join(format!("{stem}_%02d.jpg"));

    let vf = format!(
        "fps={SAMPLE_FPS},scale={FRAME_W}:{FRAME_H}{blackout}"
    );
    let ok = Command::new(&ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-nostdin", "-v", "error", "-y"])
        .args(["-ss", &format!("{start:.3}"), "-i", video, "-t", &format!("{dur:.3}")])
        .args(["-vf", &vf, "-q:v", "3"])
        .arg(&pattern)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        anyhow::bail!("ffmpeg failed extracting frames at {at:.2}s");
    }

    // Collect what ffmpeg actually produced — it may emit fewer frames than
    // requested near the very start or end of a recording.
    let mut out = Vec::new();
    for (i, off) in dataset::offsets().iter().enumerate() {
        let name = format!("{stem}_{:02}.jpg", i + 1);
        if dir.join(&name).exists() {
            out.push((name, *off));
        }
    }
    Ok(out)
}

/// Export every confirmed mark in `set` as training frames under `root`.
///
/// Appends to whatever is already there: a dataset grows across recordings and
/// across patches, and nothing is discarded when the game updates.
pub fn export(
    video: &str,
    set: &LabelSet,
    profile: Option<&GameProfile>,
    root: &Path,
    duration: f64,
    source_height: u32,
) -> Result<Manifest> {
    let blackout = blackout_filter(profile, source_height);
    let build = set.game_build.clone();
    let stamp = Path::new(video)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "clip".into());

    let mut manifest = Manifest::new();
    let mut marks = Vec::new();

    for (n, label) in set.labels.iter().enumerate() {
        if label.confirmed != Some(true) {
            continue; // only things you actually vouched for
        }
        marks.push(label.at);
        let class = dataset::class_of(label.kind);
        let dir = root.join(class);
        let stem = format!("{stamp}_{n:04}");
        let frames = extract_window(video, label.at, &dir, &stem, &blackout)?;
        for (file, offset) in frames {
            manifest.examples.push(Example {
                file: format!("{class}/{file}"),
                class: class.to_string(),
                source: video.to_string(),
                game_build: build.clone(),
                at: label.at,
                offset,
            });
        }
    }

    // Negatives: roughly match the positive count so the model isn't trained
    // into always answering "none".
    let want = (manifest.examples.len() / dataset::offsets().len()).max(1);
    let negatives = dataset::negative_times(&marks, duration, want, NEGATIVE_MIN_GAP);
    let dir = root.join(CLASS_NONE);
    for (n, t) in negatives.iter().enumerate() {
        let stem = format!("{stamp}_neg{n:04}");
        let frames = extract_window(video, *t, &dir, &stem, &blackout)?;
        for (file, offset) in frames {
            manifest.examples.push(Example {
                file: format!("{CLASS_NONE}/{file}"),
                class: CLASS_NONE.to_string(),
                source: video.to_string(),
                game_build: build.clone(),
                at: *t,
                offset,
            });
        }
    }

    merge_manifest(root, &manifest)?;
    Ok(manifest)
}

/// Fold this export into `manifest.json`, replacing any prior entries for the
/// same source video so a re-export corrects rather than duplicates.
fn merge_manifest(root: &Path, fresh: &Manifest) -> Result<()> {
    let path = root.join("manifest.json");
    let mut combined = if path.exists() {
        let s = std::fs::read_to_string(&path)?;
        serde_json::from_str::<Manifest>(&s).unwrap_or_else(|_| Manifest::new())
    } else {
        Manifest::new()
    };

    if let Some(src) = fresh.examples.first().map(|e| e.source.clone()) {
        combined.examples.retain(|e| e.source != src);
    }
    combined.examples.extend(fresh.examples.iter().cloned());
    combined.frame_width = FRAME_W;
    combined.frame_height = FRAME_H;

    std::fs::create_dir_all(root).ok();
    std::fs::write(&path, serde_json::to_string_pretty(&combined)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
