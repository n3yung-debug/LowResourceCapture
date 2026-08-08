//! Drives dataset export and model training from the GUI, so nothing has to be
//! typed into a terminal.
//!
//! Python still does the training — it's the right tool and it's already
//! written — but the app finds the interpreter, checks the dependencies, runs
//! the script, and streams its output into the review window. The user never
//! opens a shell.

use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::Serialize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What we found when looking for a usable Python.
#[derive(Debug, Clone, Serialize)]
pub struct PythonStatus {
    /// Command that works, e.g. `py` or `python`.
    pub command: Option<String>,
    /// Extra args needed with it, e.g. `["-3"]` for the Windows launcher.
    pub args: Vec<String>,
    pub version: String,
    /// Whether torch/torchvision/pillow import cleanly.
    pub deps_ok: bool,
    /// Human-readable explanation when something's missing.
    pub detail: String,
}

impl PythonStatus {
    pub fn missing(detail: &str) -> Self {
        Self {
            command: None,
            args: Vec::new(),
            version: String::new(),
            deps_ok: false,
            detail: detail.to_string(),
        }
    }

    fn cmd(&self) -> Option<Command> {
        let c = self.command.as_ref()?;
        let mut cmd = Command::new(c);
        cmd.args(&self.args).creation_flags(CREATE_NO_WINDOW);
        Some(cmd)
    }
}

/// Look for a Python that can run the trainer.
///
/// Tries the Windows launcher first (`py -3`), which is the most reliable way
/// to reach a real install when several are on PATH, then the plain names.
pub fn detect_python() -> PythonStatus {
    let candidates: [(&str, &[&str]); 3] =
        [("py", &["-3"]), ("python", &[]), ("python3", &[])];

    for (cmd, args) in candidates {
        let out = Command::new(cmd)
            .args(args)
            .arg("--version")
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let Ok(out) = out else { continue };
        if !out.status.success() {
            continue;
        }
        let version = String::from_utf8_lossy(&out.stdout)
            .trim()
            .to_string()
            .or_else_nonempty(String::from_utf8_lossy(&out.stderr).trim());

        // A Python that can't import torch can't train, and finding that out
        // now is far friendlier than failing halfway through a run.
        let dep_check = Command::new(cmd)
            .args(args)
            .args(["-c", "import torch, torchvision, PIL"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let deps_ok = dep_check.map(|o| o.status.success()).unwrap_or(false);

        return PythonStatus {
            command: Some(cmd.to_string()),
            args: args.iter().map(|s| s.to_string()).collect(),
            version,
            deps_ok,
            detail: if deps_ok {
                "ready".into()
            } else {
                "Python found, but torch/torchvision/pillow aren't installed.".into()
            },
        };
    }

    PythonStatus::missing(
        "No Python found. Install Python 3.10+ from python.org (tick \"Add to PATH\"), \
         then use Install dependencies here.",
    )
}

trait OrElseNonEmpty {
    fn or_else_nonempty(self, other: &str) -> String;
}

impl OrElseNonEmpty for String {
    fn or_else_nonempty(self, other: &str) -> String {
        if self.is_empty() { other.to_string() } else { self }
    }
}

/// Where training data accumulates: alongside the user's clips, not in
/// Program Files — a dataset of JPEGs grows into the gigabytes.
pub fn default_dataset_dir() -> PathBuf {
    shared::config::Config::load_or_create()
        .map(|c| c.output_dir.join("ClipAnalyzer-Dataset"))
        .unwrap_or_else(|_| std::env::temp_dir().join("ClipAnalyzer-Dataset"))
}

/// The bundled trainer script, installed next to the exe.
pub fn train_script() -> Option<PathBuf> {
    let p = shared::config::install_dir().ok()?.join("training").join("train.py");
    p.exists().then_some(p)
}

/// Run a command, streaming each output line to `on_line`, and report success.
///
/// stderr is merged into stdout so pip and torch progress — which mostly goes
/// to stderr — shows up in the window instead of vanishing.
fn run_streaming(
    mut cmd: Command,
    mut on_line: impl FnMut(String),
    cancel: &dyn Fn() -> bool,
) -> Result<bool> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to start the process")?;

    // Drain stderr on its own thread so a full pipe can't deadlock the child.
    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tx_err = tx.clone();
    if let Some(err) = stderr {
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx_err.send(line);
            }
        });
    }
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
    } else {
        drop(tx);
    }

    loop {
        while let Ok(line) = rx.try_recv() {
            on_line(line);
        }
        if cancel() {
            let _ = child.kill();
            on_line("— cancelled —".to_string());
            return Ok(false);
        }
        match child.try_wait()? {
            Some(status) => {
                // Drain whatever is still buffered before reporting.
                std::thread::sleep(std::time::Duration::from_millis(120));
                while let Ok(line) = rx.try_recv() {
                    on_line(line);
                }
                return Ok(status.success());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(80)),
        }
    }
}

/// `pip install` the training dependencies, streaming progress.
///
/// The CUDA wheel is a large download; the UI says so before starting rather
/// than appearing to hang.
pub fn install_dependencies(
    py: &PythonStatus,
    on_line: impl FnMut(String),
    cancel: &dyn Fn() -> bool,
) -> Result<bool> {
    let mut cmd = py.cmd().context("no Python interpreter available")?;
    cmd.args([
        "-m", "pip", "install", "--upgrade",
        "torch", "torchvision", "pillow",
        "--index-url", "https://download.pytorch.org/whl/cu124",
    ]);
    run_streaming(cmd, on_line, cancel)
}

/// Run the trainer over the accumulated dataset.
pub fn train(
    py: &PythonStatus,
    dataset_dir: &Path,
    build: &str,
    on_line: impl FnMut(String),
    cancel: &dyn Fn() -> bool,
) -> Result<bool> {
    let script = train_script().context(
        "train.py not found next to the executable — reinstall ClipAnalyzer",
    )?;
    let mut cmd = py.cmd().context("no Python interpreter available")?;
    cmd.arg(&script)
        .arg("--data")
        .arg(dataset_dir)
        // Unbuffered, or Python's block buffering means the window shows
        // nothing until the run is already over.
        .env("PYTHONUNBUFFERED", "1");
    if !build.is_empty() {
        cmd.args(["--build", build]);
    }
    run_streaming(cmd, on_line, cancel)
}

/// Detection profile shipped next to the exe, used to black out burnt-in
/// overlays during export so the model can't learn the webcam as a shortcut.
pub fn installed_profile() -> Option<crate::profile::GameProfile> {
    let dir = shared::config::install_dir().ok()?.join("profiles");
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if e.path().extension().is_some_and(|x| x == "toml") {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                if let Ok(p) = crate::profile::GameProfile::from_toml(&text) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// One line describing the trained model beside the dataset, if there is one.
pub fn model_summary(dataset_dir: &Path, current_build: &str) -> String {
    let card = dataset_dir.join("model.card.json");
    if !card.exists() {
        return "No model trained yet.".into();
    }
    match crate::model::ModelCard::load(&card) {
        Ok(c) => {
            let mut s = c.summary(current_build);
            let thin = c.undertrained_classes(50);
            if !thin.is_empty() {
                // An overall accuracy figure hides a class the model never
                // gets right, so name those explicitly rather than let the
                // headline number speak for them.
                s.push_str(&format!(" — thin classes: {}", thin.join(", ")));
            }
            s
        }
        Err(e) => format!("Model card unreadable: {e}"),
    }
}

/// Summary of what's currently in the dataset, for the GUI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DatasetStatus {
    pub dir: String,
    pub exists: bool,
    pub total: usize,
    pub per_class: std::collections::BTreeMap<String, usize>,
    pub sources: usize,
    pub builds: Vec<String>,
    /// Recordings held out for evaluation need at least two sources; below
    /// that any accuracy number is meaningless and the UI says so.
    pub enough_to_train: bool,
}

pub fn dataset_status(dir: &Path) -> DatasetStatus {
    let mut s = DatasetStatus { dir: dir.display().to_string(), ..Default::default() };
    let manifest = dir.join("manifest.json");
    if !manifest.exists() {
        return s;
    }
    s.exists = true;
    let Ok(text) = std::fs::read_to_string(&manifest) else { return s };
    let Ok(m) = serde_json::from_str::<crate::dataset::Manifest>(&text) else { return s };
    s.total = m.examples.len();
    s.per_class = m.class_counts();
    s.builds = m.game_builds();
    let mut sources: Vec<&str> = m.examples.iter().map(|e| e.source.as_str()).collect();
    sources.sort_unstable();
    sources.dedup();
    s.sources = sources.len();
    s.enough_to_train = s.sources >= 2 && s.total >= 50;
    s
}
