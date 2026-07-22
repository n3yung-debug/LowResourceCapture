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
/// `data:video/mp4` URI, for the in-app preview `<video>`. Chromium can't
/// decode HEVC, so we transcode; downscaled to 480p30 to keep the data URI
/// small and quick. Preview quality only — the trim always re-encodes from the
/// original at full resolution.
pub fn preview_data_uri(input: &str) -> Option<String> {
    let ff = ffmpeg()?;
    let tmp = std::env::temp_dir().join(format!("lrc_preview_{}.mp4", std::process::id()));
    let ok = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-hide_banner", "-y", "-i", input,
            "-vf", "scale=-2:480",
            "-r", "30",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "30",
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

/// Frame-accurate trim, re-encoded to H.264 + AAC so the result is exact and
/// plays/previews everywhere. `start` and `dur` are seconds.
pub fn trim(input: &str, start: f64, dur: f64, out: &Path) -> Result<()> {
    let ff = ffmpeg().context("ffmpeg.exe not found next to the app")?;
    let status = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-hide_banner",
            "-y",
            "-ss",
            &format!("{start}"),
            "-i",
            input,
            "-t",
            &format!("{dur}"),
            "-c:v",
            "libx264",
            "-crf",
            "18",
            "-preset",
            "veryfast",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
        ])
        .arg(out)
        .status()
        .context("running ffmpeg trim")?;
    if !status.success() {
        anyhow::bail!("ffmpeg exited with {status}");
    }
    Ok(())
}
