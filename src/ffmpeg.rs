//! Thin wrappers over the bundled `ffmpeg.exe` for the clip editor.
//!
//! FFmpeg has its own HEVC decoder, so trimming/preview works without the
//! Windows HEVC extension. It's invoked only from the clip library window —
//! never on the capture path.

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use base64::Engine;

/// Don't flash a console window when spawning ffmpeg from the windowed app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ffmpeg() -> Option<std::path::PathBuf> {
    crate::config::ffmpeg_path()
}

/// True if the bundled ffmpeg is available.
pub fn available() -> bool {
    ffmpeg().is_some()
}

/// Clip duration in seconds, parsed from ffmpeg's `Duration:` line.
pub fn duration_secs(input: &str) -> Option<f64> {
    let ff = ffmpeg()?;
    let out = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-hide_banner", "-i", input])
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let idx = stderr.find("Duration:")?;
    let after = stderr[idx + "Duration:".len()..].trim_start();
    let hms = after.split(',').next()?.trim();
    parse_hms(hms)
}

fn parse_hms(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let h: f64 = it.next()?.trim().parse().ok()?;
    let m: f64 = it.next()?.trim().parse().ok()?;
    let sec: f64 = it.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// A horizontal filmstrip of `tiles` evenly-spaced frames as a JPEG data URI,
/// for the trim timeline.
pub fn filmstrip_data_uri(input: &str, dur: f64, tiles: u32) -> Option<String> {
    let ff = ffmpeg()?;
    let tmp = std::env::temp_dir().join(format!("lrc_strip_{}.jpg", std::process::id()));
    let fps = (tiles as f64 / dur.max(0.1)).clamp(0.01, 60.0);
    let vf = format!("fps={fps},scale=160:-1,tile={tiles}x1");
    let ok = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-hide_banner", "-y", "-i", input, "-vf", &vf, "-frames:v", "1", "-q:v", "4"])
        .arg(&tmp)
        .status()
        .ok()?
        .success();
    if !ok {
        return None;
    }
    let bytes = std::fs::read(&tmp).ok()?;
    let _ = std::fs::remove_file(&tmp);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:image/jpeg;base64,{b64}"))
}

/// A lightweight, playable H.264 + AAC copy of the whole clip as a
/// `data:video/mp4` URI, for an in-app `<video>`. Chromium can't decode HEVC,
/// so we transcode; downscaled to `height` px and capped at 30fps to keep the
/// data URI manageable. Playback quality only — trim/split always re-encode
/// from the original at full resolution.
///
/// The trim scrubber uses a small 480p copy (fast, tiny); the Play window uses
/// a nicer 720p copy.
pub fn preview_data_uri(input: &str, height: u32, crf: u32) -> Option<String> {
    let ff = ffmpeg()?;
    let tmp = std::env::temp_dir().join(format!("lrc_preview_{}.mp4", std::process::id()));
    let ok = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-hide_banner", "-y", "-i", input,
            "-vf", &format!("scale=-2:{height}"),
            "-r", "30",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", &crf.to_string(),
            "-c:a", "aac",
            "-movflags", "+faststart",
        ])
        .arg(&tmp)
        .status()
        .ok()?
        .success();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    let bytes = std::fs::read(&tmp).ok()?;
    let _ = std::fs::remove_file(&tmp);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:video/mp4;base64,{b64}"))
}

/// Re-encode a range of `input` to a frame-accurate H.264 + AAC file at `out`.
/// `dur` = `None` means "to the end of the clip". Shared by trim and split.
fn encode(input: &str, start: f64, dur: Option<f64>, out: &Path) -> Result<()> {
    let ff = ffmpeg().context("ffmpeg.exe not found next to the app")?;
    let mut cmd = Command::new(ff);
    cmd.creation_flags(CREATE_NO_WINDOW)
        .args(["-hide_banner", "-y", "-ss", &format!("{start}"), "-i", input]);
    if let Some(d) = dur {
        cmd.args(["-t", &format!("{d}")]);
    }
    cmd.args([
        "-c:v", "libx264", "-crf", "18", "-preset", "veryfast",
        "-c:a", "aac", "-movflags", "+faststart",
    ]);
    let status = cmd.arg(out).status().context("running ffmpeg")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited with {status}");
    }
    Ok(())
}

/// Frame-accurate trim, re-encoded to H.264 + AAC so the result is exact and
/// plays/previews everywhere. `start` and `dur` are seconds.
pub fn trim(input: &str, start: f64, dur: f64, out: &Path) -> Result<()> {
    encode(input, start, Some(dur), out)
}

/// Split `input` at `at` seconds into two frame-accurate H.264 + AAC files:
/// `part1` gets 0..at, `part2` gets at..end.
pub fn split(input: &str, at: f64, part1: &Path, part2: &Path) -> Result<()> {
    encode(input, 0.0, Some(at), part1).context("writing part 1")?;
    encode(input, at, None, part2).context("writing part 2")?;
    Ok(())
}
