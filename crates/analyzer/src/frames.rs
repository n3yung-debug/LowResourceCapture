//! Streams a cropped region of interest out of a video as raw RGB frames.
//!
//! One ffmpeg process does the seeking, decoding, sampling and cropping, and
//! writes only the ROI to stdout — so a 9-minute 1080p VOD sampled at 2 fps
//! moves a few MB through this process rather than a few GB. Decode is the
//! expensive part, so `-hwaccel auto` hands it to the GPU where possible.

use anyhow::{Context, Result};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};

use crate::profile::Region;

/// Don't flash a console window when spawning ffmpeg from the windowed app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Source video dimensions, parsed from ffmpeg's stream line.
pub fn dimensions(input: &str) -> Option<(u32, u32)> {
    let ff = shared::config::ffmpeg_path()?;
    let out = Command::new(ff)
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-hide_banner", "-i", input])
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    // e.g. "Stream #0:0: Video: h264 ..., yuv420p(tv), 1920x1080 [SAR 1:1 ...]"
    for line in stderr.lines().filter(|l| l.contains("Video:")) {
        for tok in line.split(|c: char| !(c.is_ascii_digit() || c == 'x')) {
            if let Some((w, h)) = tok.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    // Guard against matching things like "0x1" in stream ids.
                    if w >= 256 && h >= 144 {
                        return Some((w, h));
                    }
                }
            }
        }
    }
    None
}

/// An ffmpeg process emitting `w x h` RGB24 frames of one ROI at `fps`.
pub struct RoiStream {
    child: Child,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    frame_bytes: usize,
    index: u64,
}

impl RoiStream {
    pub fn open(input: &str, roi: &Region, src_w: u32, src_h: u32, fps: u32) -> Result<Self> {
        let ff = shared::config::ffmpeg_path()
            .context("bundled ffmpeg.exe not found next to the executable")?;
        let (x, y, w, h) = roi.to_pixels(src_w, src_h);
        anyhow::ensure!(w > 0 && h > 0, "region of interest is empty at {src_w}x{src_h}");

        let child = Command::new(ff)
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "-nostdin",
                "-v", "error",
                "-hwaccel", "auto",
                "-i", input,
                "-vf", &format!("fps={fps},crop={w}:{h}:{x}:{y}"),
                "-f", "rawvideo",
                "-pix_fmt", "rgb24",
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn ffmpeg for ROI streaming")?;

        Ok(Self { child, width: w, height: h, fps, frame_bytes: (w * h * 3) as usize, index: 0 })
    }

    /// Next frame as `(timestamp_seconds, rgb24)`, or `None` at end of stream.
    pub fn next_frame(&mut self, buf: &mut Vec<u8>) -> Result<Option<f64>> {
        buf.resize(self.frame_bytes, 0);
        let stdout = self.child.stdout.as_mut().context("ffmpeg stdout closed")?;
        match stdout.read_exact(buf) {
            Ok(()) => {
                let at = self.index as f64 / self.fps as f64;
                self.index += 1;
                Ok(Some(at))
            }
            // A short read is the normal end of stream, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(e).context("reading frame from ffmpeg"),
        }
    }
}

impl Drop for RoiStream {
    fn drop(&mut self) {
        // Don't leave ffmpeg decoding a 9-minute VOD after we've stopped caring.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
