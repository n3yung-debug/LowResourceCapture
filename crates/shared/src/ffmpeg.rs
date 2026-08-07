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
///
/// Decodes **keyframes only** (`-skip_frame nokey`). A full decode of a 60s
/// 1440p HEVC clip is ~3600 frames of software HEVC — the single biggest cause
/// of the CPU spike when opening a clip. Our recordings carry a keyframe about
/// once a second, which is far more than a 12-tile strip needs, so this gets
/// the same picture for ~1/60th of the decode work.
pub fn filmstrip_data_uri(input: &str, dur: f64, tiles: u32) -> Option<String> {
    let ff = ffmpeg()?;
    let t0 = std::time::Instant::now();
    let tmp = std::env::temp_dir().join(format!("lrc_strip_{}.jpg", std::process::id()));
    let fps = (tiles as f64 / dur.max(0.1)).clamp(0.01, 60.0);
    let vf = format!("fps={fps},scale=160:-1,tile={tiles}x1");
    let ok = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-hide_banner",
            "-y",
            "-skip_frame", "nokey",   // decode only keyframes (cheap)
            "-hwaccel", "auto",       // GPU decode when available; silently ignored if not
            "-i", input,
            "-an", "-sn",             // no audio/subtitle decode
            "-vf", &vf,
            "-frames:v", "1",
            "-q:v", "4",
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
    log::info!("ffmpeg: filmstrip in {} ms", t0.elapsed().as_millis());
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
    let scale = format!("scale=-2:{height}");
    let crf_s = crf.to_string();

    // Try the GPU encoder first, then fall back to software. NVENC runs on the
    // dedicated encoder block (separate silicon from the CPU *and* the shaders),
    // so the preview stops pegging cores. The software fallback is thread-capped
    // for the same reason — a background preview must never eat the whole CPU
    // while a game is running.
    let attempts: [(&str, Vec<&str>); 2] = [
        (
            "h264_nvenc",
            vec!["-c:v", "h264_nvenc", "-preset", "p1", "-b:v", "2500k"],
        ),
        (
            "libx264",
            vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", &crf_s, "-threads", "4"],
        ),
    ];

    for (name, venc) in &attempts {
        let t0 = std::time::Instant::now();
        let mut cmd = Command::new(&ff);
        cmd.creation_flags(CREATE_NO_WINDOW)
            .args(["-hide_banner", "-y", "-hwaccel", "auto", "-i", input])
            .args(["-vf", &scale, "-r", "30"])
            .args(venc)
            .args(["-c:a", "aac", "-movflags", "+faststart"])
            .arg(&tmp);
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        if ok {
            if let Ok(bytes) = std::fs::read(&tmp) {
                let _ = std::fs::remove_file(&tmp);
                log::info!(
                    "ffmpeg: {height}p preview via {name} in {} ms ({} KB)",
                    t0.elapsed().as_millis(),
                    bytes.len() / 1024
                );
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Some(format!("data:video/mp4;base64,{b64}"));
            }
        }
        log::warn!("ffmpeg: preview via {name} failed, trying next encoder");
        let _ = std::fs::remove_file(&tmp);
    }
    None
}

/// A preview of just `dur` seconds starting at `start`, as a data URI.
///
/// The whole-file [`preview_data_uri`] is fine for a 60s clip but hopeless for
/// a 9-minute VOD — base64 of the entire transcode would be hundreds of MB in
/// a single string. Reviewing detections only ever needs the seconds around
/// each one, so this encodes that window and nothing else.
///
/// `-ss` goes before `-i` for a fast keyframe seek; the window may therefore
/// begin up to a keyframe early, which is harmless for review.
pub fn preview_range_data_uri(
    input: &str,
    start: f64,
    dur: f64,
    height: u32,
    crf: u32,
) -> Option<String> {
    let ff = ffmpeg()?;
    let tmp = std::env::temp_dir()
        .join(format!("lrc_review_{}_{}.mp4", std::process::id(), start as u64));
    let scale = format!("scale=-2:{height}");
    let crf_s = crf.to_string();
    let start_s = format!("{:.3}", start.max(0.0));
    let dur_s = format!("{dur:.3}");

    let attempts: [(&str, Vec<&str>); 2] = [
        ("h264_nvenc", vec!["-c:v", "h264_nvenc", "-preset", "p1", "-b:v", "2500k"]),
        (
            "libx264",
            vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", &crf_s, "-threads", "4"],
        ),
    ];

    for (name, venc) in &attempts {
        let t0 = std::time::Instant::now();
        let ok = Command::new(&ff)
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-hide_banner", "-y", "-hwaccel", "auto"])
            .args(["-ss", &start_s, "-i", input, "-t", &dur_s])
            .args(["-vf", &scale, "-r", "30"])
            .args(venc)
            .args(["-c:a", "aac", "-movflags", "+faststart"])
            .arg(&tmp)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            if let Ok(bytes) = std::fs::read(&tmp) {
                let _ = std::fs::remove_file(&tmp);
                log::info!(
                    "ffmpeg: review window {start_s}s+{dur_s}s via {name} in {} ms ({} KB)",
                    t0.elapsed().as_millis(),
                    bytes.len() / 1024
                );
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Some(format!("data:video/mp4;base64,{b64}"));
            }
        }
        log::warn!("ffmpeg: review window via {name} failed, trying next encoder");
        let _ = std::fs::remove_file(&tmp);
    }
    None
}

/// Re-encode a range of `input` to a frame-accurate H.264 + AAC file at `out`.
/// `dur` = `None` means "to the end of the clip". Shared by trim and split.
fn encode(input: &str, start: f64, dur: Option<f64>, out: &Path) -> Result<()> {
    let ff = ffmpeg().context("ffmpeg.exe not found next to the app")?;

    // GPU encode first (high bitrate so the exported edit stays visually clean),
    // software as a fallback. Same reasoning as the preview: keep the CPU free.
    let attempts: [(&str, &[&str]); 2] = [
        (
            "h264_nvenc",
            &["-c:v", "h264_nvenc", "-preset", "p5", "-b:v", "30M", "-maxrate", "40M", "-bufsize", "60M"],
        ),
        (
            "libx264",
            &["-c:v", "libx264", "-crf", "18", "-preset", "veryfast", "-threads", "4"],
        ),
    ];

    let mut last_err = String::new();
    for (name, venc) in &attempts {
        let t0 = std::time::Instant::now();
        let mut cmd = Command::new(&ff);
        cmd.creation_flags(CREATE_NO_WINDOW).args([
            "-hide_banner", "-y", "-hwaccel", "auto",
            "-ss", &format!("{start}"), "-i", input,
        ]);
        if let Some(d) = dur {
            cmd.args(["-t", &format!("{d}")]);
        }
        cmd.args(*venc)
            .args(["-c:a", "aac", "-movflags", "+faststart"])
            .arg(out);
        match cmd.status() {
            Ok(s) if s.success() => {
                log::info!("ffmpeg: encoded piece via {name} in {} ms", t0.elapsed().as_millis());
                return Ok(());
            }
            Ok(s) => last_err = format!("{name} exited with {s}"),
            Err(e) => last_err = format!("{name} failed to run: {e}"),
        }
        log::warn!("ffmpeg: {last_err}; trying next encoder");
    }
    anyhow::bail!("ffmpeg encode failed ({last_err})")
}

/// Assemble one clip from an ordered list of `(start, end)` source ranges:
/// each range is re-encoded to a frame-accurate H.264 + AAC segment, then the
/// segments are concatenated (stream-copied) into `out` in the given order.
/// This backs the "piece together your splits" editor.
pub fn assemble(input: &str, segments: &[(f64, f64)], out: &Path) -> Result<()> {
    if segments.is_empty() {
        anyhow::bail!("no pieces to assemble");
    }
    // One piece: just a trim, no concat needed.
    if segments.len() == 1 {
        let (s, e) = segments[0];
        return encode(input, s, Some((e - s).max(0.1)), out);
    }

    let tmp = std::env::temp_dir();
    let pid = std::process::id();
    let mut parts: Vec<std::path::PathBuf> = Vec::new();
    let result = (|| -> Result<()> {
        for (i, (s, e)) in segments.iter().enumerate() {
            let part = tmp.join(format!("lrc_seg_{pid}_{i}.mp4"));
            encode(input, *s, Some((e - s).max(0.1)), &part)
                .with_context(|| format!("encoding piece {}", i + 1))?;
            parts.push(part);
        }
        // concat demuxer list: forward-slashed, single-quoted absolute paths.
        let list = tmp.join(format!("lrc_concat_{pid}.txt"));
        let mut txt = String::new();
        for part in &parts {
            txt.push_str(&format!("file '{}'\n", part.to_string_lossy().replace('\\', "/")));
        }
        std::fs::write(&list, &txt).context("writing concat list")?;

        let ff = ffmpeg().context("ffmpeg.exe not found next to the app")?;
        let status = Command::new(ff)
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-hide_banner", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(out)
            .status()
            .context("running ffmpeg concat")?;
        let _ = std::fs::remove_file(&list);
        if !status.success() {
            anyhow::bail!("ffmpeg concat exited with {status}");
        }
        Ok(())
    })();

    // Always clean up the temp segment files.
    for part in &parts {
        let _ = std::fs::remove_file(part);
    }
    result
}
