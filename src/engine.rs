//! Capture engine coordinator.
//!
//! Owns the capture -> encode -> ring-buffer pipeline and services "save the
//! last N seconds" requests. Runs on its own thread so hotkey handling and the
//! UI message loop never block on disk I/O or muxing.
//!
//! Lifecycle:
//!   * `spawn()` starts the engine thread (idle until a game is active).
//!   * The game detector (or manual start) flips capture on/off; while on, the
//!     encoder continuously feeds `RingBuffer`.
//!   * A `SaveClip { seconds }` command extracts the last N seconds from the
//!     ring buffer and hands it to the muxer, which writes an .mp4.
//!
//! Layer 1 status: the thread, command channel, and ring buffer are real. The
//! capture/encode feed and muxer calls are stubbed (they log) until layers 2+
//! wire in the Windows GPU pipeline. `UNVERIFIED` end-to-end until then.

use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use windows::Win32::Media::MediaFoundation::IMFMediaType;

use crate::audio::AudioCapture;
use crate::capture::{CaptureTarget, MonitorCapture};
use crate::config::{Codec, Config};
use crate::muxer;
use crate::ringbuffer::RingBuffer;

/// Commands sent to the engine thread.
pub enum EngineCommand {
    /// Save the last `seconds` of buffered footage. `label` is the preset name.
    SaveClip { seconds: u32, label: String },
    /// A game became active — begin capturing this window/process. `with_audio`
    /// is false for the "video only" debug path (used to bisect crashes).
    StartCapture { window_title: String, with_audio: bool },
    /// The active game exited — stop capturing and free GPU resources.
    StopCapture,
    /// Reload settings (hotkeys are re-registered by the caller).
    ReloadConfig(Box<Config>),
    /// Debug: dump the whole ring buffer to a raw .hevc/.h264 file for viewing.
    DumpBuffer,
    /// Shut the engine down.
    Shutdown,
}

/// Handle used by the UI/hotkey thread to talk to the engine.
pub struct EngineHandle {
    tx: Sender<EngineCommand>,
    join: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: EngineCommand) {
        // If the engine thread is gone, dropping the command is fine — we're
        // shutting down.
        let _ = self.tx.send(cmd);
    }

    /// Signal shutdown and wait for the engine thread to finish.
    pub fn shutdown(mut self) {
        let _ = self.tx.send(EngineCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Start the engine thread and return a handle for sending it commands.
pub fn spawn(config: Config) -> (EngineHandle, Sender<EngineCommand>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx_for_caller = tx.clone();
    let join = std::thread::Builder::new()
        .name("capture-engine".into())
        .spawn(move || engine_loop(config, rx))
        .expect("spawning capture engine thread");
    (
        EngineHandle {
            tx,
            join: Some(join),
        },
        tx_for_caller,
    )
}

fn engine_loop(mut config: Config, rx: Receiver<EngineCommand>) {
    // Windows.Graphics.Capture is WinRT — initialize a multithreaded apartment
    // on this thread before touching it.
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        );
    }

    // Shared with the encoder pump thread, which pushes encoded frames.
    let ring = std::sync::Arc::new(std::sync::Mutex::new(RingBuffer::new(
        config.buffer.max_seconds,
        config.buffer.max_ram_mb,
    )));
    // Parallel rings for encoded (AAC) game + mic audio; muxed at save.
    let audio_ring = std::sync::Arc::new(std::sync::Mutex::new(RingBuffer::new(
        config.buffer.max_seconds,
        config.buffer.max_ram_mb,
    )));
    let mic_ring = std::sync::Arc::new(std::sync::Mutex::new(RingBuffer::new(
        config.buffer.max_seconds,
        config.buffer.max_ram_mb,
    )));
    // Samples the foreground app once a second so a saved clip is filed under
    // wherever it spent the most time (see SaveClip).
    let fg_tracker = crate::game_detect::ForegroundTracker::start(config.buffer.max_seconds);
    let mut capture: Option<MonitorCapture> = None;
    let mut audio: Option<AudioCapture> = None;
    // The video encoder's output media type, needed to mux. Set when capture
    // starts; kept after stop so a clip right after alt-tab can still save.
    let mut video_out_type: Option<IMFMediaType> = None;

    log::info!("engine started (idle)");

    // Layer 1: a simple command loop. In layer 2 this becomes a select over
    // (a) commands and (b) freshly-encoded frames from the capture pipeline,
    // which get pushed into `ring`.
    while let Ok(cmd) = rx.recv() {
        match cmd {
            EngineCommand::StartCapture { window_title, with_audio } => {
                if capture.is_none() {
                    log::info!(
                        "start capture (primary monitor); requested '{window_title}' \
                         (audio: {with_audio})"
                    );
                    // L2a: capture the primary monitor. L2c will attach the
                    // NVENC encoder here and begin feeding `ring`; L2e switches
                    // the target to the foreground window.
                    match MonitorCapture::start(
                        CaptureTarget::PrimaryMonitor,
                        config.encoder.clone(),
                        ring.clone(),
                    ) {
                        Ok(c) => {
                            video_out_type = Some(c.video_output_type());
                            capture = Some(c);
                        }
                        Err(e) => log::error!("could not start capture: {e:#}"),
                    }
                    if with_audio && audio.is_none() {
                        audio =
                            AudioCapture::start(config.audio, audio_ring.clone(), mic_ring.clone())
                                .ok();
                    }
                }
            }
            EngineCommand::StopCapture => {
                if capture.take().is_some() {
                    log::info!("stop capture; releasing GPU resources");
                    // Dropping MonitorCapture closes the session. Buffer
                    // contents are kept so a clip right after alt-tab still
                    // works, then age out normally.
                }
                // Stop audio too (dropping it joins the capture thread).
                audio = None;
            }
            EngineCommand::SaveClip { seconds, label } => {
                let frames = ring.lock().unwrap().extract_last(seconds);
                if frames.is_empty() {
                    log::warn!(
                        "clip '{label}' ({seconds}s) requested but buffer is empty \
                         (start capture first?)"
                    );
                    // TODO(layer 5): toast "Nothing to clip yet".
                } else if let Some(vtype) = video_out_type.as_ref() {
                    // File the clip under a per-source subfolder named for the
                    // app the clip spent the most time in (ties -> the app at the
                    // clip's start); fall back to the live foreground app if there
                    // is no history yet. <output_dir>\<source>\clip_...mp4.
                    let source = fg_tracker
                        .majority_folder(std::time::Duration::from_secs(seconds as u64))
                        .unwrap_or_else(crate::game_detect::foreground_app_folder);
                    let dir = config.output_dir.join(&source);
                    std::fs::create_dir_all(&dir).ok();
                    let path = muxer::clip_filename(&dir, seconds);

                    // Gather the matching game + mic audio for the same window.
                    // Their media types come from the running AudioCapture; the
                    // frames share the QPC clock with the video (see muxer).
                    let game_frames = audio_ring.lock().unwrap().extract_last(seconds);
                    let mic_frames = mic_ring.lock().unwrap().extract_last(seconds);
                    let game_type = audio.as_ref().and_then(|a| a.game_type());
                    let mic_type = audio.as_ref().and_then(|a| a.mic_type());
                    let mut tracks: Vec<muxer::AudioTrack> = Vec::new();
                    if let Some(t) = game_type.as_ref() {
                        if !game_frames.is_empty() {
                            tracks.push(muxer::AudioTrack { media_type: t, frames: &game_frames });
                        }
                    }
                    if let Some(t) = mic_type.as_ref() {
                        if !mic_frames.is_empty() {
                            tracks.push(muxer::AudioTrack { media_type: t, frames: &mic_frames });
                        }
                    }

                    match muxer::write_clip(&path, vtype, &frames, &tracks) {
                        Ok(()) => log::info!(
                            "saved '{label}' clip: {seconds}s, {} video frames + {} audio track(s) -> {}",
                            frames.len(),
                            tracks.len(),
                            path.display()
                        ),
                        Err(e) => log::error!("failed to save clip: {e:#}"),
                    }
                } else {
                    log::warn!("clip '{label}': no encoder output type yet (start capture first)");
                }
            }
            EngineCommand::ReloadConfig(new_cfg) => {
                log::info!("engine reloading config");
                config = *new_cfg;
                // Rebuild the ring contents in place (shared Arc stays valid).
                *ring.lock().unwrap() =
                    RingBuffer::new(config.buffer.max_seconds, config.buffer.max_ram_mb);
            }
            EngineCommand::DumpBuffer => {
                let frames = ring.lock().unwrap().extract_last(u32::MAX / 2);
                if frames.is_empty() {
                    log::warn!("dump requested but buffer is empty (start capture first)");
                } else {
                    let mut data = Vec::new();
                    for f in &frames {
                        data.extend_from_slice(&f.data);
                    }
                    let dir = config.output_dir.clone();
                    std::fs::create_dir_all(&dir).ok();
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let ext = match config.encoder.codec {
                        Codec::Hevc => "hevc",
                        Codec::H264 => "h264",
                    };
                    let path = dir.join(format!("debug_{stamp}.{ext}"));
                    match std::fs::write(&path, &data) {
                        Ok(()) => log::info!(
                            "dumped {} frames ({} KB) to {} — play in VLC",
                            frames.len(),
                            data.len() / 1024,
                            path.display()
                        ),
                        Err(e) => log::error!("dump write failed: {e}"),
                    }
                }
            }
            EngineCommand::Shutdown => {
                log::info!("engine shutting down");
                break;
            }
        }
    }
}
