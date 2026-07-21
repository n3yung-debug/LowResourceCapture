//! Configuration: hotkeys, clip lengths, output paths, encoder + buffer settings.
//!
//! Stored as human-editable TOML in the user's config dir
//! (`%APPDATA%\LowResourceCapture\config.toml`). The settings UI writes this
//! file; on startup we load it (creating a default if missing).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One clip preset: a hotkey bound to a retroactive save length.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipPreset {
    /// Human label, e.g. "Short".
    pub name: String,
    /// How many seconds back from "now" to save when this hotkey fires.
    pub seconds: u32,
    /// Hotkey string in `global-hotkey` syntax, e.g. "F9", "Ctrl+Shift+F9".
    pub hotkey: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// H.265 — smaller files, best quality/bitrate. Default.
    Hevc,
    /// H.264 — maximum editor/upload compatibility.
    H264,
}

impl Default for Codec {
    fn default() -> Self {
        Codec::Hevc
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    /// Desktop/game audio + mic as two separate tracks. Default.
    GameAndMicSeparate,
    /// Desktop/game audio + mic mixed into one track.
    GameAndMicMixed,
    /// Desktop/game audio only.
    GameOnly,
    /// No audio.
    None,
}

impl Default for AudioMode {
    fn default() -> Self {
        AudioMode::GameAndMicSeparate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EncoderConfig {
    pub codec: Codec,
    /// Target average bitrate in Mbps. NVENC VBR around this value.
    pub bitrate_mbps: u32,
    /// Capture/encode framerate cap. 60 is typical; 30 halves buffer RAM.
    pub fps: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            codec: Codec::default(),
            bitrate_mbps: 40,
            fps: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BufferConfig {
    /// Hard cap on how many seconds the ring buffer may hold. The largest
    /// clip preset must be <= this. Bounds worst-case RAM.
    pub max_seconds: u32,
    /// Safety ceiling on RAM the encoded buffer may use, in MB. If encoded
    /// data would exceed this, the oldest frames are dropped early.
    pub max_ram_mb: u32,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            max_seconds: 120,
            max_ram_mb: 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directory where finished clips are written.
    pub output_dir: PathBuf,
    /// Clip presets (each is a hotkey + a save length).
    pub presets: Vec<ClipPreset>,
    pub encoder: EncoderConfig,
    pub buffer: BufferConfig,
    pub audio: AudioMode,
    /// If true, only capture when a detected game is in the foreground.
    /// If false, capture the foreground window whenever the app is running.
    pub auto_detect_games: bool,
    /// Extra executable names (lowercase, e.g. "rust.exe") to always treat
    /// as games, on top of the built-in heuristics.
    pub game_allowlist: Vec<String>,
    /// Executable names to never capture even if detected as a game.
    pub game_blocklist: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            presets: vec![
                ClipPreset { name: "Short".into(), seconds: 15, hotkey: "F9".into() },
                ClipPreset { name: "Medium".into(), seconds: 30, hotkey: "F10".into() },
                ClipPreset { name: "Long".into(), seconds: 60, hotkey: "F11".into() },
            ],
            encoder: EncoderConfig::default(),
            buffer: BufferConfig::default(),
            audio: AudioMode::default(),
            auto_detect_games: true,
            game_allowlist: Vec::new(),
            game_blocklist: Vec::new(),
        }
    }
}

impl Config {
    /// Load config from disk, creating a default file if none exists.
    pub fn load_or_create() -> Result<Config> {
        let path = config_path()?;
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading config {}", path.display()))?;
            let mut cfg: Config =
                toml::from_str(&text).with_context(|| "parsing config.toml")?;
            cfg.validate_and_clamp();
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save()?;
            Ok(cfg)
        }
    }

    /// Write the current config back to disk.
    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&path, text)
            .with_context(|| format!("writing config {}", path.display()))?;
        Ok(())
    }

    /// The largest preset length actually needed by the buffer.
    pub fn required_buffer_seconds(&self) -> u32 {
        self.presets.iter().map(|p| p.seconds).max().unwrap_or(60)
    }

    /// Clamp nonsensical values so the rest of the app can trust the config.
    fn validate_and_clamp(&mut self) {
        if self.encoder.fps == 0 {
            self.encoder.fps = 60;
        }
        if self.encoder.bitrate_mbps == 0 {
            self.encoder.bitrate_mbps = 40;
        }
        let needed = self.required_buffer_seconds();
        if self.buffer.max_seconds < needed {
            self.buffer.max_seconds = needed;
        }
    }
}

/// The install directory (where the running exe lives) — e.g.
/// `C:\Program Files (x86)\LowResourceCapture`. Config and logs live here now
/// (the installer grants the current user write access to this folder, since
/// Program Files is otherwise read-only to a non-elevated process).
pub fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating current exe")?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .context("exe has no parent directory")
}

/// `<install>\config.toml`
pub fn config_path() -> Result<PathBuf> {
    Ok(install_dir()?.join("config.toml"))
}

/// `<install>\logs\lowresourcecapture.log`
pub fn log_path() -> Result<PathBuf> {
    Ok(install_dir()?.join("logs").join("lowresourcecapture.log"))
}

/// Default clips output directory: `<Videos>\LowResourceCapture`.
///
/// `<Videos>` is resolved from the Windows "Videos" **known folder**, so it
/// honors a relocated library automatically — if your Videos library lives on
/// `D:\Videos`, that's what this returns, with no hardcoded drive letter. The
/// `LowResourceCapture` folder is created on first save if it doesn't exist.
///
/// This is only the *default* — `output_dir` in config.toml overrides it, so
/// point it wherever you like.
fn default_output_dir() -> PathBuf {
    let videos = directories::UserDirs::new()
        .and_then(|d| d.video_dir().map(|v| v.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    videos.join("LowResourceCapture")
}
