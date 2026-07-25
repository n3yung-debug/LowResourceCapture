//! Clip library + timeline editor — a WebView2 window (separate `--clips`
//! process).
//!
//! Lists saved clips (play / open in folder / rename / delete), plays them in
//! an in-app window, and edits them on a block timeline: split at the playhead,
//! drag blocks to reorder, trim their edges, delete blocks, then render the
//! sequence to one new `.mp4` under `<clips root>\Edits\<category>\`.
//!
//! All ffmpeg work runs on worker threads and pushes results back to the page
//! via `EventLoopProxy` → `evaluate_script`, so the UI never blocks on it.

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
    /// Ordered `(start, end)` source ranges for the "assemble" command.
    #[serde(default)]
    segments: Vec<(f64, f64)>,
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
        // Assemble the chosen pieces (in order) into one new clip (off-thread).
        "assemble" => {
            let p = proxy.clone();
            let path = m.path.clone();
            let segments = m.segments.clone();
            let root = output_dir.to_path_buf();
            std::thread::spawn(move || {
                let out = assemble_out_path(&path, &root);
                let js = match crate::ffmpeg::assemble(&path, &segments, &out) {
                    Ok(()) => format!(
                        "window.assembleDone(true,{})",
                        js_str(&out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                    ),
                    Err(e) => format!("window.assembleDone(false,{})", js_str(&format!("{e:#}"))),
                };
                let _ = p.send_event(UserEvent::Eval(js));
            });
        }
        "close" => return true,
        _ => {}
    }
    false
}

/// Edited clips go to `<clips root>\Edits\<category>\<stem>-edit.mp4`, where
/// `<category>` mirrors the source clip's per-game subfolder (so an edit of
/// `…\LowResourceCapture\FCN\clip.mp4` lands in `…\LowResourceCapture\Edits\FCN\`).
/// Clips sitting directly in the root get `Edits\` with no category. Falls back
/// to the source folder if the Edits folder can't be created.
fn assemble_out_path(input: &str, root: &Path) -> PathBuf {
    let p = Path::new(input);
    let src_dir = p.parent().unwrap_or_else(|| Path::new("."));
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".into());

    // Category = the source subfolder name, when the clip lives under the root.
    // Re-editing something that's already in Edits keeps it where it is, so we
    // never build up Edits\Edits\… .
    let edits_root = root.join("Edits");
    let mut dir = if src_dir.starts_with(&edits_root) {
        src_dir.to_path_buf()
    } else if src_dir != root {
        match src_dir.file_name() {
            Some(cat) => edits_root.join(cat),
            None => edits_root.clone(),
        }
    } else {
        edits_root.clone()
    };
    if std::fs::create_dir_all(&dir).is_err() {
        log::warn!("clips: could not create {}, saving next to the source", dir.display());
        dir = src_dir.to_path_buf();
    }

    let mut out = dir.join(format!("{stem}-edit.mp4"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{stem}-edit{n}.mp4"));
        n += 1;
    }
    out
}

/// JSON-encode a string so it's safe to embed in a `evaluate_script` call.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Scan `root` and its subfolders (up to 2 levels down) for `.mp4` clips,
/// newest first. Two levels is what `Edits\<category>\` needs — edits are
/// nested one deeper than recorded clips, and they must show up in the library
/// like anything else.
fn scan_clips(root: &Path) -> Vec<ClipInfo> {
    fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        out.push(dir.to_path_buf());
        if depth == 0 {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    collect(&e.path(), depth - 1, out);
                }
            }
        }
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    collect(root, 2, &mut dirs);

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
            // Folder label relative to the clips root, so an edit shows as
            // "Edits\FCN" rather than a bare "FCN" that looks like a recording.
            let folder = p
                .parent()
                .map(|d| {
                    d.strip_prefix(root)
                        .unwrap_or(d)
                        .to_string_lossy()
                        .into_owned()
                })
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
