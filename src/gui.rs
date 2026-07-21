//! Settings GUI — a WebView2 window (separate `--gui` process) to edit clip
//! presets (hotkey + duration) and encoder/buffer settings, then write
//! config.toml. Because it runs only when opened, it costs nothing during
//! capture/gameplay.
//!
//! The page (settings.html) talks to Rust over wry's IPC: it posts a JSON
//! `{cmd:"save", ...}` or `{cmd:"cancel"}`; on save we merge the edited fields
//! into the current config and persist it, then close.

use anyhow::{Context, Result};
use serde::Deserialize;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

use crate::config::{AudioMode, ClipPreset, Codec, Config};

const SETTINGS_HTML: &str = include_str!("settings.html");

enum UserEvent {
    Ipc(String),
}

#[derive(Deserialize)]
struct PresetIn {
    name: String,
    seconds: u32,
    hotkey: String,
}

#[derive(Deserialize)]
struct SaveMsg {
    presets: Vec<PresetIn>,
    mic_enabled: Option<bool>,
    output_dir: Option<String>,
    codec: Option<String>,
    bitrate_mbps: Option<u32>,
    fps: Option<u32>,
    buffer_max_seconds: Option<u32>,
    buffer_max_ram_mb: Option<u32>,
}

enum IpcResult {
    Saved,
    Cancelled,
    Ignored,
}

/// Show the settings window and block until the user saves or closes it.
pub fn run() -> Result<()> {
    let mut config = Config::load_or_create()?;
    let init = format!(
        "window.__CONFIG__ = {};",
        serde_json::to_string(&config).context("serialize config for GUI")?
    );

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("LowResourceCapture — Settings")
        .with_inner_size(LogicalSize::new(580.0, 660.0))
        .with_min_inner_size(LogicalSize::new(480.0, 420.0))
        .build(&event_loop)
        .context("create settings window")?;

    let _webview = WebViewBuilder::new()
        .with_html(SETTINGS_HTML)
        .with_initialization_script(init)
        .with_ipc_handler(move |req: Request<String>| {
            let _ = proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .build(&window)
        .context("create webview")?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Ipc(msg)) => match handle_ipc(&msg, &mut config) {
                IpcResult::Saved => {
                    log::info!("settings saved via GUI");
                    *control_flow = ControlFlow::Exit;
                }
                IpcResult::Cancelled => *control_flow = ControlFlow::Exit,
                IpcResult::Ignored => {}
            },
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    })
}

fn handle_ipc(msg: &str, config: &mut Config) -> IpcResult {
    let value: serde_json::Value = match serde_json::from_str(msg) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("gui: bad ipc json: {e}");
            return IpcResult::Ignored;
        }
    };
    match value.get("cmd").and_then(|c| c.as_str()) {
        Some("save") => match serde_json::from_value::<SaveMsg>(value) {
            Ok(m) => {
                apply_and_save(config, m);
                IpcResult::Saved
            }
            Err(e) => {
                log::warn!("gui: bad save payload: {e}");
                IpcResult::Ignored
            }
        },
        Some("cancel") => IpcResult::Cancelled,
        _ => IpcResult::Ignored,
    }
}

fn apply_and_save(config: &mut Config, m: SaveMsg) {
    if !m.presets.is_empty() {
        config.presets = m
            .presets
            .into_iter()
            .map(|p| ClipPreset {
                name: if p.name.trim().is_empty() {
                    "Clip".to_string()
                } else {
                    p.name
                },
                seconds: p.seconds.max(1),
                hotkey: p.hotkey,
            })
            .collect();
    }
    if let Some(mic) = m.mic_enabled {
        // Toggle mic capture on/off while always keeping desktop/game audio.
        // Preserve a "mixed" choice when the mic stays on.
        config.audio = match (mic, config.audio) {
            (true, AudioMode::GameAndMicMixed) => AudioMode::GameAndMicMixed,
            (true, _) => AudioMode::GameAndMicSeparate,
            (false, _) => AudioMode::GameOnly,
        };
    }
    if let Some(d) = m.output_dir {
        if !d.trim().is_empty() {
            config.output_dir = d.into();
        }
    }
    if let Some(c) = m.codec {
        config.encoder.codec = if c.eq_ignore_ascii_case("h264") {
            Codec::H264
        } else {
            Codec::Hevc
        };
    }
    if let Some(b) = m.bitrate_mbps {
        if b > 0 {
            config.encoder.bitrate_mbps = b;
        }
    }
    if let Some(f) = m.fps {
        if f > 0 {
            config.encoder.fps = f;
        }
    }
    if let Some(s) = m.buffer_max_seconds {
        if s > 0 {
            config.buffer.max_seconds = s;
        }
    }
    if let Some(r) = m.buffer_max_ram_mb {
        if r > 0 {
            config.buffer.max_ram_mb = r;
        }
    }
    // Buffer must be able to hold the largest preset.
    let needed = config.required_buffer_seconds();
    if config.buffer.max_seconds < needed {
        config.buffer.max_seconds = needed;
    }
    if let Err(e) = config.save() {
        log::warn!("gui: could not save config: {e}");
    }
}
