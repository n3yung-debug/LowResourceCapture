//! Muxing extracted encoded frames into a finished .mp4 clip.
//!
//! The frames coming out of the ring buffer are already compressed
//! (HEVC/H.264 video, AAC audio). Muxing just wraps them in an MP4 container —
//! no re-encoding — so saving a clip is nearly instant and costs almost no CPU.
//!
//! Intended implementation (layer 4), Media Foundation `IMFSinkWriter`:
//!   1. `MFCreateSinkWriterFromURL` for the output .mp4.
//!   2. Add a video stream with the encoder's output media type; add one or
//!      two AAC audio streams per `AudioMode`.
//!   3. Rebase timestamps so the clip starts at 0, wrap each `EncodedFrame` in
//!      an `IMFSample` (marking keyframes), `WriteSample` in pts order.
//!   4. `Finalize`. Name files `<game>_<yyyymmdd-hhmmss>_<len>s.mp4`.
//!
//! Because the clip must start on a keyframe (guaranteed by
//! `RingBuffer::extract_last`), no re-encode is ever needed.
//!
//! `UNVERIFIED` — needs Windows Media Foundation.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::audio::AudioTracks;
use crate::ringbuffer::EncodedFrame;

/// Build the output filename for a clip.
pub fn clip_filename(output_dir: &Path, game: &str, len_secs: u32) -> PathBuf {
    let stamp = now_stamp();
    let safe_game = sanitize(game);
    output_dir.join(format!("{safe_game}_{stamp}_{len_secs}s.mp4"))
}

/// Mux video + audio frames into `out_path`. Layer 1 stub.
pub fn write_clip(
    _out_path: &Path,
    _video: &[EncodedFrame],
    _audio: &AudioTracks,
) -> Result<()> {
    anyhow::bail!("mp4 muxing not implemented until layer 4")
}

/// `YYYYMMDD-HHMMSS` local timestamp without pulling in a datetime crate.
fn now_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Coarse UTC stamp; layer 4 can swap to local time via the Win32 API.
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    // Not calendar-accurate labelling here — placeholder until layer 4 uses
    // GetLocalTime for a proper date. Kept unique via the day count.
    format!("{days:05}-{h:02}{m:02}{s:02}")
}

fn sanitize(name: &str) -> String {
    let name = name.trim();
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "clip".to_string()
    } else {
        cleaned
    }
}
