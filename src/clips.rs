//! Clip library + trim editor — a WebView2 window (separate `--clips` process).
//!
//! Lists saved clips (play / reveal / rename / delete) and trims them with the
//! bundled ffmpeg: a filmstrip timeline with draggable in/out handles, saved to
//! a new frame-accurate `.mp4`. ffmpeg work runs on worker threads and pushes
//! results back to the page via `EventLoopProxy` → `evaluate_script`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

use crate::config::Config;

const CLIPS_HTML: &str = include_str!("clips.html");

enum UserEvent {
    Ipc(String),
    /// A JS snippet to run in the page (results from worker threads).
    Eval(String),
}

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
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
}

/// Show the clip library window and block until it's closed.
pub fn run() -> Result<()> {
    let config = Config::load_or_create()?;
    let output_dir = config.output_dir.clone();
    let clips = scan_clips(&output_dir);
    let init = format!(
        "window.__CLIPS__ = {}; window.__FFMPEG__ = {};",
        serde_json::to_string(&clips).unwrap_or_else(|_| "[]".into()),
        crate::ffmpeg::available()
    );

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();

    let window = WindowBuilder::new()
        .with_title("LowResourceCapture — Clips")
        .with_inner_size(LogicalSize::new(820.0, 660.0))
        .with_min_inner_size(LogicalSize::new(560.0, 440.0))
        .build(&event_loop)
        .context("create clips window")?;

    let webview = WebViewBuilder::new()
        .with_html(CLIPS_HTML)
        .with_initialization_script(init)
        .with_ipc_handler(move |req: Request<String>| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .build(&window)
        .context("create webview")?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Ipc(msg)) => {
                if handle_ipc(&msg, &output_dir, &proxy) {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
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
fn handle_ipc(msg: &str, output_dir: &Path, proxy: &EventLoopProxy<UserEvent>) -> bool {
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
        // Build the trim timeline (duration + filmstrip), then a playable
        // preview — both off-thread. The timeline lands first so scrubbing works
        // immediately; the H.264 preview follows a beat later.
        "trimOpen" => {
            let p = proxy.clone();
            let path = m.path.clone();
            std::thread::spawn(move || {
                let dur = match crate::ffmpeg::duration_secs(&path) {
                    Some(d) => d,
                    None => {
                        let _ = p.send_event(UserEvent::Eval(
                            "window.trimError('Could not read this clip.')".to_string(),
                        ));
                        return;
                    }
                };
                let strip = crate::ffmpeg::filmstrip_data_uri(&path, dur, 12).unwrap_or_default();
                let _ = p.send_event(UserEvent::Eval(format!(
                    "window.trimReady({{duration:{dur},strip:{}}})",
                    js_str(&strip)
                )));
                // Chromium can't decode HEVC, so transcode a light 480p H.264
                // copy for the trim scrubber.
                let js = match crate::ffmpeg::preview_data_uri(&path, 480, 30) {
                    Some(uri) => format!("window.previewReady({})", js_str(&uri)),
                    None => "window.previewError('Preview could not be generated.')".to_string(),
                };
                let _ = p.send_event(UserEvent::Eval(js));
            });
        }
        // In-app player: transcode a nicer 720p H.264 copy and hand it to the
        // player <video>.
        "playPreview" => {
            let p = proxy.clone();
            let path = m.path.clone();
            std::thread::spawn(move || {
                let js = match crate::ffmpeg::preview_data_uri(&path, 720, 26) {
                    Some(uri) => format!("window.playerReady({})", js_str(&uri)),
                    None => "window.playerError('Could not build a preview for this clip.')".to_string(),
                };
                let _ = p.send_event(UserEvent::Eval(js));
            });
        }
        // Full-quality playback in the OS default player (from the player window).
        "openExternal" => open_default(&m.path),
        // Split the clip at the playhead into two files (off-thread).
        "split" => {
            let p = proxy.clone();
            let path = m.path.clone();
            let at = m.start;
            std::thread::spawn(move || {
                let (p1, p2) = split_out_paths(&path);
                let js = match crate::ffmpeg::split(&path, at, &p1, &p2) {
                    Ok(()) => format!(
                        "window.splitDone(true,{})",
                        js_str(&format!(
                            "{} + {}",
                            p1.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            p2.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        ))
                    ),
                    Err(e) => format!("window.splitDone(false,{})", js_str(&format!("{e:#}"))),
                };
                let _ = p.send_event(UserEvent::Eval(js));
            });
        }
        // Run the trim off-thread, then tell the page how it went.
        "trimSave" => {
            let p = proxy.clone();
            let path = m.path.clone();
            let (start, end) = (m.start, m.end);
            std::thread::spawn(move || {
                let out = trim_out_path(&path);
                let dur = (end - start).max(0.1);
                let js = match crate::ffmpeg::trim(&path, start, dur, &out) {
                    Ok(()) => format!(
                        "window.trimDone(true,{})",
                        js_str(&out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                    ),
                    Err(e) => format!("window.trimDone(false,{})", js_str(&format!("{e:#}"))),
                };
                let _ = p.send_event(UserEvent::Eval(js));
            });
        }
        "close" => return true,
        _ => {}
    }
    false
}

/// `<dir>/<stem>-trim.mp4`, avoiding overwrite by appending a number.
fn trim_out_path(input: &str) -> PathBuf {
    let p = Path::new(input);
    let dir = p.parent().unwrap_or_else(|| Path::new("."));
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    let mut out = dir.join(format!("{stem}-trim.mp4"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{stem}-trim{n}.mp4"));
        n += 1;
    }
    out
}

/// `<dir>/<stem>-part1.mp4` and `-part2.mp4`, sharing a numeric suffix if
/// needed so the pair never overwrites existing files.
fn split_out_paths(input: &str) -> (PathBuf, PathBuf) {
    let p = Path::new(input);
    let dir = p.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    let mut suffix = String::new();
    let mut n = 2;
    loop {
        let p1 = dir.join(format!("{stem}-part1{suffix}.mp4"));
        let p2 = dir.join(format!("{stem}-part2{suffix}.mp4"));
        if !p1.exists() && !p2.exists() {
            return (p1, p2);
        }
        suffix = format!("_{n}");
        n += 1;
    }
}

/// JSON-encode a string so it's safe to embed in a `evaluate_script` call.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
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
