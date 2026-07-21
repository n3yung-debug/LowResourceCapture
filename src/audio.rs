//! Audio capture via WASAPI: desktop/game audio (loopback) + microphone.
//!
//! Depending on `AudioMode` we capture one or two sources:
//!   * **Game/desktop** audio from the default render endpoint in *loopback*
//!     mode (`AUDCLNT_STREAMFLAGS_LOOPBACK`).
//!   * **Microphone** from the default capture endpoint.
//!
//! Each source is encoded to AAC (via a Media Foundation AAC MFT) and pushed
//! into its own ring of `EncodedFrame`s, time-stamped on the same clock as
//! video so the muxer can align them. "Separate tracks" writes two audio
//! streams into the mp4; "mixed" sums the PCM before encoding.
//!
//! WASAPI in shared mode is extremely light — the OS already mixes this audio;
//! we're just tapping it. Event-driven (`AUDCLNT_STREAMFLAGS_EVENTCALLBACK`)
//! so there's no polling spin.
//!
//! Intended implementation (layer 3):
//!   1. `IMMDeviceEnumerator` -> default render (loopback) + default capture.
//!   2. `IAudioClient::Initialize` shared + event callback; `IAudioCaptureClient`.
//!   3. On the audio event, pull packets, timestamp, encode to AAC, push to the
//!      matching audio ring buffer.
//!   4. Handle format (mix format), silence padding on loopback gaps.
//!
//! `UNVERIFIED` — needs Windows audio endpoints.

use anyhow::Result;

use crate::config::AudioMode;
use crate::ringbuffer::EncodedFrame;

/// Which encoded audio tracks a clip should carry.
pub struct AudioTracks {
    /// Game/desktop audio frames (empty if `AudioMode::None`).
    pub game: Vec<EncodedFrame>,
    /// Mic frames (empty unless a mic mode is selected).
    pub mic: Vec<EncodedFrame>,
}

/// Running audio capture session feeding per-source ring buffers.
pub struct AudioCapture {
    pub mode: AudioMode,
}

impl AudioCapture {
    /// Start capturing per `mode`. Layer 1 stub.
    pub fn start(mode: AudioMode) -> Result<AudioCapture> {
        if matches!(mode, AudioMode::None) {
            return Ok(AudioCapture { mode });
        }
        anyhow::bail!("WASAPI audio capture not implemented until layer 3")
    }

    /// Extract the last `seconds` of audio to pair with a saved video clip.
    /// Layer 1 stub.
    pub fn extract_last(&self, _seconds: u32) -> AudioTracks {
        AudioTracks {
            game: Vec::new(),
            mic: Vec::new(),
        }
    }
}
