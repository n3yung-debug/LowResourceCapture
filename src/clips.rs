//! Clip library — a WebView2 window (separate `--clips` process) that lists the
//! saved clips and lets you play, reveal, rename, or delete them. Runs only
//! when opened, so it costs nothing during capture.
//!
//! Playback opens the clip in the system's default player (which handles HEVC);
//! an in-app preview is a later phase (HEVC needs the Windows codec extension).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

use crate::config::Config;

const CLIPS_HTML: &str = include_str!("clips.html");

enum UserEvent {
    Ipc(String),
}

/// One saved clip, sent to the page.
#[derive(Serialize)]
struct ClipInfo {
    path: String,
    name: String,
    folder: String,
    seconds: u32,
    size_mb: f64,
    modified: u64,
}

#[derive(Deserialize)]
struct IpcMsg {
    cmd: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    new_path: String,
}

/// Show the clip library window and block until it's closed.
pub fn run() -> Result<()> {
    let config = Config::load_or_create()?;
    let output_dir = config.output_dir.clone();
    let clips = scan_clips(&output_dir);
    let init = format!(
        "window.__CLIPS__ = {};",
        serde_json::to_string(&clips).unwrap_or_else(|_| "[]".into())
    );

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("LowResourceCapture — Clips")
        .with_inner_size(LogicalSize::new(760.0, 620.0))
        .with_min_inner_size(LogicalSize::new(520.0, 400.0))
        .build(&event_loop)
        .context("create clips window")?;

    let _webview = WebViewBuilder::new()
        .with_html(CLIPS_HTML)
        .with_initialization_script(init)
        .with_ipc_handler(move |req: Request<String>| {
            let _ = proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .build(&window)
        .context("create webview")?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Ipc(msg)) => {
                if handle_ipc(&msg, &output_dir) {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    })
}

/// Returns true if the window should close.
fn handle_ipc(msg: &str, output_dir: &Path) -> bool {
    let m: IpcMsg = match serde_json::from_str(msg) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("clips: bad ipc: {e}");
            return false;
        }
    };
    match m.cmd.as_str() {
        "play" => open_default(&m.path),
        "reveal" => reveal(&m.path),
        "openFolder" => open_folder(output_dir),
        "delete" => {
            if let Err(e) = std::fs::remove_file(&m.path) {
                log::warn!("clips: delete failed: {e}");
            }
        }
        "rename" => {
            if !m.new_path.is_empty() {
                if let Err(e) = std::fs::rename(&m.path, &m.new_path) {
                    log::warn!("clips: rename failed: {e}");
                }
            }
        }
        "close" => return true,
        _ => {}
    }
    false
}

/// Scan `root` and its immediate subfolders for `.mp4` clips, newest first.
fn scan_clips(root: &Path) -> Vec<ClipInfo> {
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }

    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("mp4") {
                continue;
            }
            let meta = e.metadata().ok();
            let size_mb = meta.as_ref().map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let name = p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = p
                .parent()
                .and_then(|x| x.file_name())
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(ClipInfo {
                path: p.to_string_lossy().into_owned(),
                seconds: parse_len(&name),
                name,
                folder,
                size_mb,
                modified,
            });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// Parse the length from `clip_<stamp>_<len>s.mp4`.
fn parse_len(name: &str) -> u32 {
    name.trim_end_matches(".mp4")
        .rsplit('_')
        .next()
        .and_then(|s| s.strip_suffix('s'))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn open_default(path: &str) {
    // `cmd /C start "" <path>` opens the file with its default handler.
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", path])
        .spawn();
}

fn reveal(path: &str) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn();
}

fn open_folder(dir: &Path) {
    std::fs::create_dir_all(dir).ok();
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}
