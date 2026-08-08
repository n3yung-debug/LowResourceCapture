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
use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tao::dpi::LogicalSize;
use tao::event::{Event as WinEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::http::{Request, Response};
use wry::WebViewBuilder;

use crate::event::Kind;
use crate::labels::LabelSet;

const REVIEW_HTML: &str = include_str!("review.html");

/// Seconds either side of an event when building clips from confirmed events.
const PRE: f64 = 12.0;
const POST: f64 = 4.0;

/// Bytes served per range request. Big enough that seeking isn't chatty, small
/// enough that a 1 GB VOD never lands in memory.
const CHUNK: u64 = 4 * 1024 * 1024;

/// Serve the source video over a custom protocol with HTTP range support.
///
/// This is what makes the window usable on a recording where **nothing was
/// detected** — which is the common case, since you extract far more often than
/// you die. The `<video>` element seeks natively against the original file, so
/// there is no transcode, no wait, and scrubbing works whether the scan found
/// anything or not.
fn serve_range(path: &Path, range: Option<&str>) -> Response<Cow<'static, [u8]>> {
    let fail = |code: u16| {
        Response::builder()
            .status(code)
            .body(Cow::Owned(Vec::new()))
            .unwrap_or_else(|_| Response::new(Cow::Owned(Vec::new())))
    };

    let Ok(meta) = std::fs::metadata(path) else { return fail(404) };
    let total = meta.len();
    if total == 0 {
        return fail(404);
    }

    // "bytes=START-[END]". An absent header is treated as "from the start",
    // which is what Chromium's media stack asks for first anyway.
    let (start, end) = match range.and_then(|r| r.strip_prefix("bytes=")) {
        Some(spec) => {
            let mut parts = spec.splitn(2, '-');
            let s: u64 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
            let e = parts
                .next()
                .and_then(|e| e.trim().parse::<u64>().ok())
                .unwrap_or(s + CHUNK - 1);
            (s, e.min(total - 1))
        }
        None => (0, (CHUNK - 1).min(total - 1)),
    };
    if start >= total {
        return fail(416);
    }
    let end = end.max(start).min(start + CHUNK - 1).min(total - 1);
    let len = end - start + 1;

    let mut buf = vec![0u8; len as usize];
    let read = std::fs::File::open(path)
        .and_then(|mut f| {
            f.seek(SeekFrom::Start(start))?;
            f.read_exact(&mut buf)?;
            Ok(())
        })
        .is_ok();
    if !read {
        return fail(500);
    }

    Response::builder()
        .status(206)
        .header("Content-Type", "video/mp4")
        .header("Accept-Ranges", "bytes")
        .header("Content-Range", format!("bytes {start}-{end}/{total}"))
        .header("Content-Length", len.to_string())
        .body(Cow::Owned(buf))
        .unwrap_or_else(|_| fail(500))
}

enum UserEvent {
    Ipc(String),
    Eval(String),
    /// Scan progress, 0.0..=1.0.
    Progress(f64),
    /// Scan finished — detections, or the reason it couldn't run.
    Scanned(Result<Vec<crate::event::Event>, String>),
    /// A line of output from an export/install/train subprocess.
    TrainLog(String),
    /// A long-running training task started (true) or finished (false).
    TrainBusy(bool),
    /// Re-send dataset/Python status to the page.
    TrainRefresh,
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
    #[serde(default)]
    dir: String,
    #[serde(default)]
    build: String,
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
    let served = PathBuf::from(video);
    let webview = WebViewBuilder::new()
        .with_html(REVIEW_HTML)
        .with_custom_protocol("vod".into(), move |_id, req: Request<Vec<u8>>| {
            let range = req
                .headers()
                .get("Range")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            serve_range(&served, range.as_deref())
        })
        .with_ipc_handler(move |req: Request<String>| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .build(&window)
        .context("creating review webview")?;

    let mut state = State {
        set,
        label_path,
        video: video.to_string(),
        duration,
        scan: "scanning for deaths…".into(),
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    // Scan behind the window rather than before it. The window is usable
    // immediately — you can scrub and mark while this runs.
    {
        let p = proxy.clone();
        let path = video.to_string();
        std::thread::spawn(move || {
            let mut last = -1.0f64;
            let result = crate::detect::scan_deaths(&path, |frac| {
                // Only wake the UI on visible movement, not every frame.
                if frac - last >= 0.01 || frac >= 1.0 {
                    last = frac;
                    let _ = p.send_event(UserEvent::Progress(frac));
                }
            });
            let _ = p.send_event(UserEvent::Scanned(result.map_err(|e| format!("{e:#}"))));
        });
    }

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
            WinEvent::UserEvent(UserEvent::TrainLog(line)) => {
                let _ = webview
                    .evaluate_script(&format!("window.trainLog({})", js_str(&line)));
            }
            WinEvent::UserEvent(UserEvent::TrainBusy(on)) => {
                let _ = webview.evaluate_script(&format!("window.trainBusy({on})"));
            }
            WinEvent::UserEvent(UserEvent::TrainRefresh) => state.push_training_status(&proxy),
            WinEvent::UserEvent(UserEvent::Progress(frac)) => {
                state.scan = format!("scanning for deaths… {}%", (frac * 100.0).round() as u32);
                let _ = webview
                    .evaluate_script(&format!("window.scanProgress({:.4})", frac));
            }
            WinEvent::UserEvent(UserEvent::Scanned(result)) => match result {
                Ok(found) => {
                    let n = found.len();
                    log::info!("scan complete: {n} death(s)");
                    state.scan = if n == 0 {
                        "scan complete — no deaths found. Scrub and press K to mark kills."
                            .into()
                    } else {
                        format!("scan complete — {n} death{} found", if n == 1 { "" } else { "s" })
                    };
                    state.set.merge_detections(found, 3.0);
                    state.save();
                    state.push(&proxy, "updated");
                    let _ = webview.evaluate_script(&format!("window.scanDone({n})"));
                }
                Err(e) => {
                    log::warn!("scan failed: {e}");
                    state.scan = format!("scan failed — {e}");
                    let _ = webview.evaluate_script(&format!(
                        "window.scanFailed({})",
                        js_str(&e)
                    ));
                }
            },
            _ => {}
        }
    });
}

struct State {
    set: LabelSet,
    label_path: PathBuf,
    video: String,
    duration: f64,
    /// Current scan status, carried in the payload so a page that finishes
    /// loading *after* the scan doesn't sit on a stale "scanning…" label.
    scan: String,
    /// Set to cancel a running install/train subprocess.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
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

            // ---- training panel ----
            "trainingStatus" => self.push_training_status(proxy),

            "exportDataset" => {
                let dir = if m.dir.is_empty() {
                    crate::training::default_dataset_dir()
                } else {
                    PathBuf::from(&m.dir)
                };
                if !m.build.is_empty() && self.set.game_build != m.build {
                    self.set.game_build = m.build.clone();
                    self.save();
                }
                let confirmed = self.set.labels.iter().filter(|l| l.confirmed == Some(true)).count();
                if confirmed == 0 {
                    let _ = proxy.send_event(UserEvent::TrainLog(
                        "Nothing confirmed on this recording yet — press Y, K, M or D first."
                            .into(),
                    ));
                    return;
                }

                let set = self.set.clone();
                let video = self.video.clone();
                let duration = self.duration;
                let p = proxy.clone();
                let _ = p.send_event(UserEvent::TrainBusy(true));
                std::thread::spawn(move || {
                    let height = crate::frames::dimensions(&video).map(|d| d.1).unwrap_or(1080);
                    let profile = crate::training::installed_profile();
                    let msg = match crate::export::export(
                        &video, &set, profile.as_ref(), &dir, duration, height,
                    ) {
                        Ok(man) => format!(
                            "Added {} frames from this recording to {}",
                            man.examples.len(),
                            dir.display()
                        ),
                        Err(e) => format!("Export failed: {e:#}"),
                    };
                    let _ = p.send_event(UserEvent::TrainLog(msg));
                    let _ = p.send_event(UserEvent::TrainBusy(false));
                    let _ = p.send_event(UserEvent::TrainRefresh);
                });
            }

            "installDeps" => {
                let p = proxy.clone();
                let flag = self.cancel.clone();
                flag.store(false, std::sync::atomic::Ordering::Relaxed);
                let _ = p.send_event(UserEvent::TrainBusy(true));
                std::thread::spawn(move || {
                    let py = crate::training::detect_python();
                    let p2 = p.clone();
                    let f2 = flag.clone();
                    let ok = crate::training::install_dependencies(
                        &py,
                        |line| {
                            let _ = p2.send_event(UserEvent::TrainLog(line));
                        },
                        &move || f2.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    let _ = p.send_event(UserEvent::TrainLog(match ok {
                        Ok(true) => "Dependencies installed.".into(),
                        Ok(false) => "Install did not complete.".to_string(),
                        Err(e) => format!("Install failed: {e:#}"),
                    }));
                    let _ = p.send_event(UserEvent::TrainBusy(false));
                    let _ = p.send_event(UserEvent::TrainRefresh);
                });
            }

            "trainModel" => {
                let dir = if m.dir.is_empty() {
                    crate::training::default_dataset_dir()
                } else {
                    PathBuf::from(&m.dir)
                };
                let build = m.build.clone();
                let p = proxy.clone();
                let flag = self.cancel.clone();
                flag.store(false, std::sync::atomic::Ordering::Relaxed);
                let _ = p.send_event(UserEvent::TrainBusy(true));
                std::thread::spawn(move || {
                    let py = crate::training::detect_python();
                    if !py.deps_ok {
                        let _ = p.send_event(UserEvent::TrainLog(py.detail.clone()));
                        let _ = p.send_event(UserEvent::TrainBusy(false));
                        return;
                    }
                    let p2 = p.clone();
                    let f2 = flag.clone();
                    let ok = crate::training::train(
                        &py,
                        &dir,
                        &build,
                        |line| {
                            let _ = p2.send_event(UserEvent::TrainLog(line));
                        },
                        &move || f2.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    let _ = p.send_event(UserEvent::TrainLog(match ok {
                        Ok(true) => "Training finished.".into(),
                        Ok(false) => "Training stopped before finishing.".to_string(),
                        Err(e) => format!("Training failed: {e:#}"),
                    }));
                    let _ = p.send_event(UserEvent::TrainBusy(false));
                    let _ = p.send_event(UserEvent::TrainRefresh);
                });
            }

            "cancelTraining" => {
                self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            other => log::warn!("review: unknown command {other}"),
        }
    }

    /// Re-render the page from what Rust holds, so the UI mirrors disk.
    fn push(&self, proxy: &EventLoopProxy<UserEvent>, func: &str) {
        #[derive(serde::Serialize)]
        struct Payload<'a> {
            video: &'a str,
            /// Served by the custom protocol above; the page seeks against it.
            url: &'a str,
            duration: f64,
            pre: f64,
            post: f64,
            scan: &'a str,
            labels: &'a [crate::labels::Label],
        }
        let payload = Payload {
            video: &self.video,
            url: "http://vod.localhost/source",
            duration: self.duration,
            pre: PRE,
            post: POST,
            scan: &self.scan,
            labels: &self.set.labels,
        };
        match serde_json::to_string(&payload) {
            Ok(json) => {
                let _ = proxy.send_event(UserEvent::Eval(format!("window.{func}({json})")));
            }
            Err(e) => log::warn!("review: could not serialize labels: {e}"),
        }
    }

    /// Dataset + Python + model status for the training panel.
    fn push_training_status(&self, proxy: &EventLoopProxy<UserEvent>) {
        #[derive(serde::Serialize)]
        struct Payload {
            dataset: crate::training::DatasetStatus,
            python: crate::training::PythonStatus,
            build: String,
            model: String,
        }
        let dir = crate::training::default_dataset_dir();
        let model = crate::training::model_summary(&dir, &self.set.game_build);
        let payload = Payload {
            dataset: crate::training::dataset_status(&dir),
            python: crate::training::detect_python(),
            build: self.set.game_build.clone(),
            model,
        };
        match serde_json::to_string(&payload) {
            Ok(json) => {
                let _ = proxy.send_event(UserEvent::Eval(format!("window.trainingStatus({json})")));
            }
            Err(e) => log::warn!("review: could not serialize training status: {e}"),
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
