//! Hardware video encoding (NVENC HEVC / H.264) via Media Foundation.
//!
//! We drive the GPU's dedicated encoder block, which is separate silicon from
//! the CUDA/graphics cores — so encoding a game while playing it costs almost
//! nothing on the parts of the GPU the game uses. This is the same mechanism
//! ShadowPlay uses.
//!
//! Two viable paths; we take the Media Foundation one for maintainability:
//!   * **Media Foundation hardware MFT** (chosen): enumerate the async
//!     hardware H.265/H.264 encoder transform, feed it the WGC `ID3D11Texture2D`
//!     surfaces directly (via a DXGI device manager, zero-copy), and pull out
//!     encoded `IMFSample`s. On NVIDIA hardware MF routes this to NVENC.
//!   * Raw NVIDIA Video Codec SDK: lowest-level, but requires linking the
//!     NVENC SDK and hand-rolling surface management. More control, more code.
//!
//! Intended implementation (layer 2/3):
//!   1. `MFStartup`. Create a `IMFDXGIDeviceManager` around the D3D11 device
//!      shared with capture so encoder input stays on the GPU.
//!   2. Enumerate `MFT_CATEGORY_VIDEO_ENCODER` for HEVC (or H264), hardware +
//!      async, prefer the NVIDIA vendor MFT.
//!   3. Set output type (codec, bitrate = config, fps, resolution), input type
//!      (NV12 or the capture format), configure VBR + a keyframe interval of
//!      ~2 s so the ring buffer always has a nearby clip start point.
//!   4. Pump: on each captured frame, `ProcessInput`; drain `ProcessOutput`
//!      into `EncodedFrame`s (marking keyframes from
//!      `MFSampleExtension_CleanPoint`) and push to the ring buffer.
//!
//! `UNVERIFIED` — needs Windows + an NVENC-capable GPU.

use anyhow::Result;

use crate::config::EncoderConfig;
use crate::ringbuffer::EncodedFrame;

/// Handle to a running hardware encoder session.
pub struct VideoEncoder {
    pub width: u32,
    pub height: u32,
    pub cfg: EncoderConfig,
}

impl VideoEncoder {
    /// Create the hardware encoder for the given resolution and settings.
    ///
    /// Layer 1 stub.
    pub fn new(_width: u32, _height: u32, _cfg: EncoderConfig) -> Result<VideoEncoder> {
        anyhow::bail!("hardware encoder not implemented until layer 2/3")
    }

    /// Submit a captured GPU frame; returns any encoded frames now ready.
    ///
    /// Layer 1 stub.
    pub fn submit(&mut self, _timestamp_100ns: i64) -> Result<Vec<EncodedFrame>> {
        Ok(Vec::new())
    }

    /// Flush the encoder and return any remaining buffered output.
    pub fn drain(&mut self) -> Result<Vec<EncodedFrame>> {
        Ok(Vec::new())
    }
}
