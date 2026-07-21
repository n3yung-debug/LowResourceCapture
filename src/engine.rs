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

use crate::config::Config;
use crate::ringbuffer::RingBuffer;

/// Commands sent to the engine thread.
pub enum EngineCommand {
    /// Save the last `seconds` of buffered footage. `label` is the preset name.
    SaveClip { seconds: u32, label: String },
    /// A game became active — begin capturing this window/process.
    StartCapture { window_title: String },
    /// The active game exited — stop capturing and free GPU resources.
    StopCapture,
    /// Reload settings (hotkeys are re-registered by the caller).
    ReloadConfig(Box<Config>),
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
    let mut ring = RingBuffer::new(config.buffer.max_seconds, config.buffer.max_ram_mb);
    let mut capturing = false;

    log::info!("engine started (idle)");

    // Layer 1: a simple command loop. In layer 2 this becomes a select over
    // (a) commands and (b) freshly-encoded frames from the capture pipeline,
    // which get pushed into `ring`.
    while let Ok(cmd) = rx.recv() {
        match cmd {
            EngineCommand::StartCapture { window_title } => {
                if !capturing {
                    log::info!("start capture: '{window_title}'");
                    // TODO(layer 2): create D3D11 device, open WGC session for
                    // this window, spin up NVENC encoder + WASAPI audio, begin
                    // feeding `ring`.
                    capturing = true;
                }
            }
            EngineCommand::StopCapture => {
                if capturing {
                    log::info!("stop capture; releasing GPU resources");
                    // TODO(layer 2): tear down WGC/encoder/audio, keep buffer
                    // contents so a clip right after alt-tab still works, then
                    // let it age out.
                    capturing = false;
                }
            }
            EngineCommand::SaveClip { seconds, label } => {
                let frames = ring.extract_last(seconds);
                if frames.is_empty() {
                    log::warn!(
                        "clip '{label}' ({seconds}s) requested but buffer is empty \
                         (no game captured yet?)"
                    );
                    // TODO(layer 5): toast "Nothing to clip yet".
                } else {
                    log::info!(
                        "saving '{label}' clip: {seconds}s, {} frames, {} KB",
                        frames.len(),
                        ring.bytes_used() / 1024
                    );
                    // TODO(layer 4): hand `frames` (+ matching audio frames) to
                    // the muxer to write an .mp4 in config.output_dir.
                }
            }
            EngineCommand::ReloadConfig(new_cfg) => {
                log::info!("engine reloading config");
                config = *new_cfg;
                // Rebuild the ring if buffer sizing changed.
                ring = RingBuffer::new(config.buffer.max_seconds, config.buffer.max_ram_mb);
            }
            EngineCommand::Shutdown => {
                log::info!("engine shutting down");
                break;
            }
        }
    }
}
