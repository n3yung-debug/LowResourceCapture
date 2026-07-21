//! Muxing extracted encoded frames into a finished .mp4 clip.
//!
//! The frames from the ring buffer are already compressed (HEVC/H.264 video,
//! AAC audio). We wrap them in an MP4 container with Media Foundation's
//! `IMFSinkWriter` — no re-encoding — so saving a clip is near-instant and
//! costs almost no CPU. L4a writes video only; L4b adds the audio streams.
//!
//! Because `RingBuffer::extract_last` snaps to a keyframe, the video is always
//! independently decodable from frame 0.
//!
//! `UNVERIFIED` until it builds/runs on Windows.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, IMFSinkWriter, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFSampleExtension_CleanPoint,
};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::ringbuffer::EncodedFrame;

/// Build the output filename: `<game>_<yyyymmdd-hhmmss>_<len>s.mp4`
pub fn clip_filename(output_dir: &Path, game: &str, len_secs: u32) -> PathBuf {
    let st = unsafe { GetLocalTime() };
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    );
    output_dir.join(format!("{}_{}_{}s.mp4", sanitize(game), stamp, len_secs))
}

/// Mux pre-encoded video frames into `out_path` (mp4) without re-encoding.
pub fn write_clip(out_path: &Path, video_type: &IMFMediaType, video: &[EncodedFrame]) -> Result<()> {
    if video.is_empty() {
        anyhow::bail!("no video frames to write");
    }
    unsafe {
        let url = HSTRING::from(out_path.to_string_lossy().as_ref());
        let writer: IMFSinkWriter =
            MFCreateSinkWriterFromURL(&url, None, None).context("MFCreateSinkWriterFromURL")?;

        // Same input and output type => the sink writer just muxes, no encode.
        let stream = writer.AddStream(video_type).context("AddStream")?;
        writer
            .SetInputMediaType(stream, video_type, None)
            .context("SetInputMediaType")?;
        writer.BeginWriting().context("BeginWriting")?;

        // Rebase timestamps so the clip starts at 0.
        let first = video[0].pts_100ns;
        for f in video {
            let sample = build_sample(&f.data, f.pts_100ns - first, f.dur_100ns, f.keyframe)?;
            writer
                .WriteSample(stream, &sample)
                .context("WriteSample")?;
        }
        writer.Finalize().context("Finalize")?;
    }
    Ok(())
}

/// Wrap encoded bytes in an `IMFSample` with timing.
unsafe fn build_sample(data: &[u8], pts_100ns: i64, dur_100ns: i64, keyframe: bool) -> Result<IMFSample> {
    let buffer = MFCreateMemoryBuffer(data.len() as u32).context("MFCreateMemoryBuffer")?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    buffer.Lock(&mut ptr, None, None).context("buffer Lock")?;
    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
    buffer.Unlock().ok();
    buffer.SetCurrentLength(data.len() as u32).ok();

    let sample = MFCreateSample().context("MFCreateSample")?;
    sample.AddBuffer(&buffer)?;
    sample.SetSampleTime(pts_100ns)?;
    sample.SetSampleDuration(dur_100ns)?;
    if keyframe {
        sample.SetUINT32(&MFSampleExtension_CleanPoint, 1).ok();
    }
    Ok(sample)
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "clip".to_string()
    } else {
        cleaned
    }
}
