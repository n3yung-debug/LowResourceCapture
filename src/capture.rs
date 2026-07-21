//! Screen capture via Windows.Graphics.Capture (WGC).
//!
//! WGC is the modern, low-overhead capture path: the compositor hands us the
//! window's frames as GPU textures (`ID3D11Texture2D`) with no CPU-side
//! screen-scraping and no per-frame readback. Frames stay on the GPU and go
//! straight into the NVENC encoder's input surface, so the CPU never touches
//! pixel data.
//!
//! Intended implementation (layer 2):
//!   1. Create an `ID3D11Device` (hardware, BGRA support).
//!   2. `GraphicsCaptureItem` for the target window (via interop from HWND) or
//!      monitor.
//!   3. `Direct3D11CaptureFramePool::CreateFreeThreaded` with 2 buffers at the
//!      item size, `B8G8R8A8UIntNormalized`.
//!   4. On each `FrameArrived`, get the `ID3D11Texture2D`, hand it to the
//!      encoder, close the frame. Cap to `config.encoder.fps`.
//!   5. Handle size changes (`Recreate` the pool) and the item closing.
//!
//! Everything here is `UNVERIFIED` — it needs a Windows GPU to run.

use anyhow::Result;

/// A captured GPU frame handed to the encoder. In layer 2 this wraps an
/// `ID3D11Texture2D` (kept on the GPU) plus its capture timestamp.
pub struct CapturedFrame {
    pub timestamp_100ns: i64,
    // TODO(layer 2): pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
}

/// Placeholder for the WGC session. Layer 2 turns this into a live capture
/// that pushes `CapturedFrame`s to the encoder.
pub struct CaptureSession {
    pub width: u32,
    pub height: u32,
}

impl CaptureSession {
    /// Open a capture session for the foreground game window.
    ///
    /// Layer 1 stub: returns an error so the engine logs "not yet implemented"
    /// rather than pretending to capture.
    pub fn open_foreground(_target_fps: u32) -> Result<CaptureSession> {
        anyhow::bail!("WGC capture not implemented until layer 2")
    }
}
