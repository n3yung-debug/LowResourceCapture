//! Audio capture via WASAPI.
//!
//! L3a: capture desktop/game audio from the default render endpoint in
//! *loopback* mode and confirm it works (log the mix format + throughput).
//! L3b will add AAC encoding, the mic, and per-source audio ring buffers for
//! muxing at save time.
//!
//! All COM lives on the capture thread (WASAPI interfaces aren't `Send` in
//! windows-rs), so nothing crosses a thread boundary.
//!
//! `UNVERIFIED` until it builds/runs on Windows.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::config::AudioMode;
use crate::ringbuffer::EncodedFrame;

/// Which encoded audio tracks a saved clip should carry (used from L4).
pub struct AudioTracks {
    pub game: Vec<EncodedFrame>,
    pub mic: Vec<EncodedFrame>,
}

/// Running audio capture. Dropping it stops the capture thread.
pub struct AudioCapture {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Start capturing per `mode`. `None` is a no-op that returns a handle
    /// which does nothing.
    pub fn start(mode: AudioMode) -> Result<AudioCapture> {
        if matches!(mode, AudioMode::None) {
            return Ok(AudioCapture {
                shutdown: Arc::new(AtomicBool::new(true)),
                thread: None,
            });
        }
        let shutdown = Arc::new(AtomicBool::new(false));
        let sd = shutdown.clone();
        let thread = std::thread::Builder::new()
            .name("audio-loopback".into())
            .spawn(move || {
                if let Err(e) = loopback_loop(&sd) {
                    log::error!("audio loopback capture failed: {e:#}");
                }
            })
            .expect("spawn audio capture thread");
        Ok(AudioCapture {
            shutdown,
            thread: Some(thread),
        })
    }

    /// L4 will return the buffered audio to pair with a saved clip.
    pub fn extract_last(&self, _seconds: u32) -> AudioTracks {
        AudioTracks {
            game: Vec::new(),
            mic: Vec::new(),
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// WASAPI loopback capture loop for the default render endpoint.
fn loopback_loop(shutdown: &AtomicBool) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("CoInitializeEx")?;

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .context("GetDefaultAudioEndpoint")?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).context("Activate IAudioClient")?;

        let mix = client.GetMixFormat().context("GetMixFormat")?;
        let (rate, channels, bits, block_align) = {
            let w = &*mix;
            (w.nSamplesPerSec, w.nChannels, w.wBitsPerSample, w.nBlockAlign)
        };

        // 1-second shared buffer, loopback + event-driven.
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                10_000_000,
                0,
                mix,
                None,
            )
            .context("IAudioClient::Initialize")?;

        let event = CreateEventW(None, false, false, None).context("CreateEventW")?;
        client.SetEventHandle(event).context("SetEventHandle")?;

        let capture: IAudioCaptureClient = client.GetService().context("GetService capture")?;
        client.Start().context("IAudioClient::Start")?;

        log::info!("audio(loopback) started: {rate} Hz, {channels} ch, {bits}-bit");

        let mut bytes: u64 = 0;
        let mut last = Instant::now();
        while !shutdown.load(Ordering::Relaxed) {
            let _ = WaitForSingleObject(event, 200);
            loop {
                let mut pdata: *mut u8 = std::ptr::null_mut();
                let mut nframes: u32 = 0;
                let mut flags: u32 = 0;
                if capture
                    .GetBuffer(&mut pdata, &mut nframes, &mut flags, None, None)
                    .is_err()
                    || nframes == 0
                {
                    break;
                }
                bytes += (nframes as u64) * (block_align as u64);
                let _ = capture.ReleaseBuffer(nframes);
            }
            if last.elapsed().as_secs() >= 1 {
                log::info!("audio(loopback): {} KB/s", bytes / 1024);
                bytes = 0;
                last = Instant::now();
            }
        }

        let _ = client.Stop();
        CoTaskMemFree(Some(mix as *const _));
        CoUninitialize();
    }
    Ok(())
}
