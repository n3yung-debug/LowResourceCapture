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

/// Build the output filename: `clip_<yyyymmdd-hhmmss>_<len>s.mp4`.
///
/// `output_dir` is the per-source subfolder (e.g. `...\LowResourceCapture\
/// Elden Ring`), so the source is conveyed by the folder rather than repeated
/// in every filename.
pub fn clip_filename(output_dir: &Path, len_secs: u32) -> PathBuf {
    let st = unsafe { GetLocalTime() };
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    );
    output_dir.join(format!("clip_{}_{}s.mp4", stamp, len_secs))
}

/// One encoded audio track to mux alongside the video (game or mic).
pub struct AudioTrack<'a> {
    /// The AAC output media type (from the audio encoder).
    pub media_type: &'a IMFMediaType,
    /// The AAC frames for the clip window.
    pub frames: &'a [EncodedFrame],
}

/// Mux pre-encoded video + audio into `out_path` (mp4) without re-encoding.
///
/// Video and audio timestamps share the QPC/100ns clock, so we rebase every
/// stream by the video's first (keyframe) pts and write all samples in a single
/// global timestamp order — that keeps A/V in sync and keeps the sink writer's
/// internal buffering small.
pub fn write_clip(
    out_path: &Path,
    video_type: &IMFMediaType,
    video: &[EncodedFrame],
    audio: &[AudioTrack],
) -> Result<()> {
    if video.is_empty() {
        anyhow::bail!("no video frames to write");
    }
    unsafe {
        let url = HSTRING::from(out_path.to_string_lossy().as_ref());
        let writer: IMFSinkWriter =
            MFCreateSinkWriterFromURL(&url, None, None).context("MFCreateSinkWriterFromURL")?;

        // Same input and output type on every stream => pure mux, no re-encode.
        let vstream = writer.AddStream(video_type).context("AddStream(video)")?;
        writer
            .SetInputMediaType(vstream, video_type, None)
            .context("SetInputMediaType(video)")?;

        let mut astreams = Vec::with_capacity(audio.len());
        for (i, a) in audio.iter().enumerate() {
            let s = writer
                .AddStream(a.media_type)
                .with_context(|| format!("AddStream(audio {i})"))?;
            writer
                .SetInputMediaType(s, a.media_type, None)
                .with_context(|| format!("SetInputMediaType(audio {i})"))?;
            astreams.push(s);
        }

        writer.BeginWriting().context("BeginWriting")?;

        // Rebase all streams by the video start; collect (pts, stream, frame)
        // and write in global timestamp order.
        let origin = video[0].pts_100ns;
        let mut items: Vec<(i64, u32, &EncodedFrame)> = Vec::new();
        for f in video {
            items.push((f.pts_100ns - origin, vstream, f));
        }
        for (a, &s) in audio.iter().zip(&astreams) {
            for f in a.frames {
                let pts = f.pts_100ns - origin;
                // Drop audio that precedes the clip's first video frame.
                if pts >= 0 {
                    items.push((pts, s, f));
                }
            }
        }
        items.sort_by_key(|(pts, _, _)| *pts);

        for (pts, stream, f) in items {
            let sample = build_sample(&f.data, pts, f.dur_100ns, f.keyframe)?;
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

