//! Review window — where detections become verdicts, and verdicts become
//! training data.
//!
//! A WebView2 window listing every detection with its status, a preview of the
//! seconds around whichever one is selected, and confirm / reject / mark-by-hand
//! on the keyboard. Every change writes the label file immediately and the page
//! is re-rendered from what Rust actually holds, so the UI can never drift from
//! what's on disk.
//!
//! Previews are built per event rather than for the whole video: a 9-minute VOD
//! as a base64 data URI would be hundreds of MB, and reviewing only ever needs
//! the window around each detection.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use tao::dpi::LogicalSize;
use tao::event::{Event as WinEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use crate::event::Kind;
use crate::labels::LabelSet;

const REVIEW_HTML: &str = include_str!("review.html");

/// Seconds either side of an event shown in the preview.
const PRE: f64 = 12.0;
const POST: f64 = 4.0;

enum UserEvent {
    Ipc(String),
    Eval(String),
}

#[derive(Deserialize)]
struct Msg {
    cmd: String,
    #[serde(default)]
    index: usize,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    at: f64,
}

/// Open the review window for `video`, using the label file beside it.
pub fn run(video: &str) -> Result<()> {
    let label_path = label_path_for(video);
    let set = if label_path.exists() {
        LabelSet::load(&label_path)?
    } else {
        LabelSet::new(video, "")
    };
    let duration = shared::ffmpeg::duration_secs(video).unwrap_or(0.0);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Review detections")
        .with_inner_size(LogicalSize::new(1180.0, 720.0))
        .build(&event_loop)
        .context("creating review window")?;

    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(REVIEW_HTML)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .build(&window)
        .context("creating review webview")?;

    let mut state = State { set, label_path, video: video.to_string(), duration };

    event_loop.run(move |ev, _, flow| {
        *flow = ControlFlow::Wait;
        match ev {
            WinEvent::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *flow = ControlFlow::Exit
            }
            WinEvent::UserEvent(UserEvent::Ipc(msg)) => state.handle(&msg, &proxy),
            WinEvent::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            _ => {}
        }
    });
}

struct State {
    set: LabelSet,
    label_path: PathBuf,
    video: String,
    duration: f64,
}

impl State {
    fn handle(&mut self, msg: &str, proxy: &EventLoopProxy<UserEvent>) {
        let m: Msg = match serde_json::from_str(msg) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("review: bad ipc message: {e}");
                return;
            }
        };
        match m.cmd.as_str() {
            "load" => self.push(proxy, "reviewData"),

            // Build a preview of just this event's window, off the UI thread.
            "clip" => {
                let Some(l) = self.set.labels.get(m.index) else { return };
                let start = (l.at - PRE).max(0.0);
                let dur = (PRE + POST).min((self.duration - start).max(1.0));
                let video = self.video.clone();
                let p = proxy.clone();
                std::thread::spawn(move || {
                    let js = match shared::ffmpeg::preview_range_data_uri(
                        &video, start, dur, 720, 26,
                    ) {
                        Some(uri) => format!("window.clipReady({})", js_str(&uri)),
                        None => "window.clipError('Could not build a preview for this window.')"
                            .to_string(),
                    };
                    let _ = p.send_event(UserEvent::Eval(js));
                });
            }

            "verdict" => {
                if let Some(l) = self.set.labels.get_mut(m.index) {
                    l.confirmed = Some(m.ok);
                    self.save();
                    self.push(proxy, "updated");
                }
            }

            "mark" => {
                let kind = match m.kind.as_str() {
                    "playerkill" => Kind::PlayerKill,
                    _ => Kind::Death,
                };
                self.set.mark(kind, m.at);
                self.save();
                self.push(proxy, "updated");
            }

            "export" => self.export(proxy),

            other => log::warn!("review: unknown command {other}"),
        }
    }

    /// Re-render the page from what Rust holds, so the UI mirrors disk.
    fn push(&self, proxy: &EventLoopProxy<UserEvent>, func: &str) {
        #[derive(serde::Serialize)]
        struct Payload<'a> {
            video: &'a str,
            pre: f64,
            post: f64,
            labels: &'a [crate::labels::Label],
        }
        let payload = Payload {
            video: &self.video,
            pre: PRE,
            post: POST,
            labels: &self.set.labels,
        };
        match serde_json::to_string(&payload) {
            Ok(json) => {
                let _ = proxy.send_event(UserEvent::Eval(format!("window.{func}({json})")));
            }
            Err(e) => log::warn!("review: could not serialize labels: {e}"),
        }
    }

    fn save(&self) {
        if let Err(e) = self.set.save(&self.label_path) {
            log::warn!("review: could not save labels: {e}");
        }
    }

    /// Assemble every confirmed event into one clip beside the source video.
    fn export(&self, proxy: &EventLoopProxy<UserEvent>) {
        let approved = self.set.confirmed();
        if approved.is_empty() {
            let _ = proxy.send_event(UserEvent::Eval(
                "window.clipError('Nothing confirmed yet — press Y on a detection first.')"
                    .into(),
            ));
            return;
        }
        let segs = crate::event::segments(&approved, PRE, POST, self.duration);
        let out = Path::new(&self.video)
            .with_extension("")
            .to_string_lossy()
            .to_string()
            + "_highlights.mp4";
        let video = self.video.clone();
        let p = proxy.clone();
        std::thread::spawn(move || {
            let js = match shared::ffmpeg::assemble(&video, &segs, Path::new(&out)) {
                Ok(()) => format!(
                    "window.clipError({})",
                    js_str(&format!("Exported {} segment(s) to {out}", segs.len()))
                ),
                Err(e) => format!("window.clipError({})", js_str(&format!("Export failed: {e}"))),
            };
            let _ = p.send_event(UserEvent::Eval(js));
        });
    }
}

pub fn label_path_for(video: &str) -> PathBuf {
    Path::new(video).with_extension("labels.json")
}

/// JSON-quote a string for safe interpolation into an `evaluate_script` call.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}
