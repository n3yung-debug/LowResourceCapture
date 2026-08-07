//! Per-game detection profiles, loaded from TOML on disk.
//!
//! The game-specific part of detection is **data, not Rust** — adding a second
//! game should be dropping a file into `profiles\`, not cutting a release.
//!
//! Regions are stored normalized (0.0..=1.0) and the cropped region is
//! resampled to a fixed canonical size before matching, so one calibration
//! covers 1080p and 1440p without a resolution switch. That handles *scaling*;
//! it does not handle a game that re-lays-out its HUD between resolutions, so
//! `Region::only_at` allows a per-resolution override when one proves needed.

use serde::{Deserialize, Serialize};

/// A normalized region of interest. `(0,0)` is top-left, `(1,1)` bottom-right.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Restrict this region to one source height (e.g. 1080). `None` = any.
    #[serde(default)]
    pub only_at: Option<u32>,
}

impl Region {
    /// Convert to pixel coordinates in a `width` x `height` frame, clamped to
    /// the frame so a sloppy profile can never index out of bounds.
    pub fn to_pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let clamp01 = |v: f64| v.clamp(0.0, 1.0);
        let x = (clamp01(self.x) * width as f64).round() as u32;
        let y = (clamp01(self.y) * height as f64).round() as u32;
        let w = (clamp01(self.w) * width as f64).round() as u32;
        let h = (clamp01(self.h) * height as f64).round() as u32;
        (x, y, w.min(width - x.min(width)), h.min(height - y.min(height)))
    }

    pub fn applies_to(&self, height: u32) -> bool {
        self.only_at.map_or(true, |h| h == height)
    }
}

/// Regions that must be ignored — burnt-in stream overlays, webcam, alerts.
///
/// These are per-*capture*, not per-game: Nick's Twitch VODs carry a webcam in
/// the bottom left and follower alerts at top centre, and a detector that
/// doesn't mask them will happily fire on them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exclusions {
    #[serde(default)]
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameProfile {
    /// Human name, e.g. "Mistfall Hunter".
    pub game: String,
    /// Folder names a clip may be filed under that imply this game.
    #[serde(default)]
    pub folders: Vec<String>,
    /// The game build this was calibrated against. Detection profiles depend on
    /// a game version exactly as a benchmark ceiling depends on a driver — when
    /// the game patches its HUD, the profile is stale and must say so rather
    /// than silently returning nothing.
    #[serde(default)]
    pub calibrated_against: String,
    /// Source height the templates were captured at, for the record.
    #[serde(default)]
    pub calibrated_height: Option<u32>,
    /// Seconds of roll either side of an event when building clips.
    #[serde(default = "default_pre")]
    pub pre_roll: f64,
    #[serde(default = "default_post")]
    pub post_roll: f64,
    /// Detections of one kind closer together than this collapse into one.
    #[serde(default = "default_cluster")]
    pub cluster_window: f64,
    #[serde(default)]
    pub exclude: Exclusions,
}

fn default_pre() -> f64 {
    12.0
}
fn default_post() -> f64 {
    4.0
}
fn default_cluster() -> f64 {
    3.0
}

impl GameProfile {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// True if this profile was calibrated against a different game build.
    pub fn is_stale(&self, current_build: &str) -> bool {
        !self.calibrated_against.is_empty()
            && !current_build.is_empty()
            && self.calibrated_against != current_build
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
game = "Mistfall Hunter"
folders = ["MistfallHunter"]
calibrated_against = "2026.08.06"
calibrated_height = 1080
pre_roll = 10.0

[[exclude.regions]]
x = 0.036
y = 0.694
w = 0.219
h = 0.306
"#;

    #[test]
    fn parses_a_profile() {
        let p = GameProfile::from_toml(SAMPLE).expect("parses");
        assert_eq!(p.game, "Mistfall Hunter");
        assert_eq!(p.pre_roll, 10.0);
        assert_eq!(p.post_roll, 4.0, "unspecified fields take defaults");
        assert_eq!(p.exclude.regions.len(), 1);
    }

    #[test]
    fn region_scales_to_any_resolution() {
        let r = Region { x: 0.5, y: 0.5, w: 0.25, h: 0.1, only_at: None };
        assert_eq!(r.to_pixels(1920, 1080), (960, 540, 480, 108));
        assert_eq!(r.to_pixels(2560, 1440), (1280, 720, 640, 144));
    }

    #[test]
    fn region_clamps_rather_than_overflowing_the_frame() {
        let r = Region { x: 0.9, y: 0.9, w: 0.5, h: 0.5, only_at: None };
        let (x, y, w, h) = r.to_pixels(1920, 1080);
        assert!(x + w <= 1920 && y + h <= 1080, "never indexes past the frame");
    }

    #[test]
    fn per_resolution_override_is_respected() {
        let r = Region { x: 0.0, y: 0.0, w: 0.1, h: 0.1, only_at: Some(1080) };
        assert!(r.applies_to(1080));
        assert!(!r.applies_to(1440));
    }

    #[test]
    fn staleness_is_flagged_when_the_game_patches() {
        let p = GameProfile::from_toml(SAMPLE).unwrap();
        assert!(p.is_stale("2026.09.01"), "a patched HUD invalidates the profile");
        assert!(!p.is_stale("2026.08.06"));
        assert!(!p.is_stale(""), "unknown build is not evidence of staleness");
    }
}
