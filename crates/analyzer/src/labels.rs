//! The label store — what you told the analyzer, kept next to what it guessed.
//!
//! This is the piece that turns reviewing into training data. Every confirm,
//! reject, or "you missed one" is a labelled example, produced as a byproduct
//! of scrubbing rather than as a separate chore.
//!
//! The rule that shapes the design: **re-running detection must never discard a
//! ruling you already made.** Detectors will be re-tuned constantly, and a
//! label set that resets on every scan is worse than useless — it silently
//! throws away the only expensive input in the system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::event::{Event, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// The detector proposed it.
    Detected,
    /// You marked it by hand — the detector missed it entirely.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub kind: Kind,
    pub at: f64,
    pub origin: Origin,
    /// `None` = not yet reviewed. `Some(false)` is a real datum (a confirmed
    /// false positive), not an absence — it is what teaches a model precision.
    pub confirmed: Option<bool>,
    #[serde(default)]
    pub score: f64,
}

impl Label {
    /// True once you've ruled on it either way.
    pub fn reviewed(&self) -> bool {
        self.confirmed.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelSet {
    /// Source video this describes, for provenance when files move around.
    #[serde(default)]
    pub video: String,
    /// Which profile and game build produced the detections, so a label set
    /// calibrated against a since-patched HUD can be spotted rather than
    /// silently trusted.
    #[serde(default)]
    pub game_build: String,
    #[serde(default)]
    pub labels: Vec<Label>,
}

impl LabelSet {
    pub fn new(video: &str, game_build: &str) -> Self {
        Self { video: video.into(), game_build: game_build.into(), labels: Vec::new() }
    }

    /// Fold a fresh detection run into the set, preserving every existing
    /// ruling.
    ///
    /// A detection within `tol` seconds of an existing label of the same kind
    /// is the *same event* re-found: its timestamp and score refresh, and your
    /// verdict is left alone. Detections matching nothing are appended
    /// unreviewed. Manual labels are never touched — the detector missing them
    /// again is not evidence they didn't happen.
    ///
    /// Returns how many were genuinely new.
    pub fn merge_detections(&mut self, detections: Vec<Event>, tol: f64) -> usize {
        let mut added = 0;
        for det in detections {
            let hit = self
                .labels
                .iter_mut()
                .filter(|l| l.kind == det.kind && (l.at - det.at).abs() <= tol)
                // If several are in range, refresh the nearest.
                .min_by(|a, b| {
                    (a.at - det.at)
                        .abs()
                        .partial_cmp(&(b.at - det.at).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            match hit {
                Some(existing) => {
                    existing.score = det.score;
                    // Only move an unreviewed label; a reviewed one is anchored
                    // to the moment you actually looked at.
                    if !existing.reviewed() && existing.origin == Origin::Detected {
                        existing.at = det.at;
                    }
                }
                None => {
                    self.labels.push(Label {
                        kind: det.kind,
                        at: det.at,
                        origin: Origin::Detected,
                        confirmed: None,
                        score: det.score,
                    });
                    added += 1;
                }
            }
        }
        self.labels.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
        added
    }

    /// Record a hand-marked event the detector missed.
    pub fn mark(&mut self, kind: Kind, at: f64) {
        self.labels.push(Label {
            kind,
            at,
            origin: Origin::Manual,
            confirmed: Some(true),
            score: 1.0,
        });
        self.labels.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Everything you've confirmed as real, in time order — what the editor
    /// turns into clips.
    pub fn confirmed(&self) -> Vec<Event> {
        self.labels
            .iter()
            .filter(|l| l.confirmed == Some(true))
            .map(|l| Event { kind: l.kind, at: l.at, score: l.score, confirmed: Some(true) })
            .collect()
    }

    /// Detections still awaiting your verdict — the review queue.
    pub fn pending(&self) -> Vec<&Label> {
        self.labels.iter().filter(|l| !l.reviewed()).collect()
    }

    /// Counts as (confirmed, rejected, pending) — enough to say whether there
    /// is yet a dataset worth training on.
    pub fn tally(&self) -> (usize, usize, usize) {
        let mut t = (0, 0, 0);
        for l in &self.labels {
            match l.confirmed {
                Some(true) => t.0 += 1,
                Some(false) => t.1 += 1,
                None => t.2 += 1,
            }
        }
        t
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("reading labels from {}", path.display()))?;
        serde_json::from_str(&s).context("parsing label file")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let s = serde_json::to_string_pretty(self).context("serializing labels")?;
        std::fs::write(path, s).with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(kind: Kind, at: f64, score: f64) -> Event {
        Event::new(kind, at, score)
    }

    fn set() -> LabelSet {
        LabelSet::new("vod.mp4", "2026.08.06")
    }

    #[test]
    fn detections_arrive_unreviewed() {
        let mut s = set();
        assert_eq!(s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0), 1);
        assert_eq!(s.tally(), (0, 0, 1));
        assert_eq!(s.labels[0].origin, Origin::Detected);
    }

    #[test]
    fn rerunning_detection_does_not_add_duplicates() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        let added = s.merge_detections(vec![det(Kind::Death, 100.5, 0.95)], 3.0);
        assert_eq!(added, 0, "same event re-found, not a new one");
        assert_eq!(s.labels.len(), 1);
    }

    #[test]
    fn rerunning_detection_preserves_a_confirmation() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        s.labels[0].confirmed = Some(true);
        s.merge_detections(vec![det(Kind::Death, 100.4, 0.99)], 3.0);
        assert_eq!(s.labels[0].confirmed, Some(true), "the whole point of the store");
        assert_eq!(s.labels[0].at, 100.0, "a reviewed label stays where you judged it");
        assert_eq!(s.labels[0].score, 0.99, "but the score refreshes");
    }

    #[test]
    fn rerunning_detection_preserves_a_rejection() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 50.0, 0.4)], 3.0);
        s.labels[0].confirmed = Some(false);
        s.merge_detections(vec![det(Kind::Death, 50.0, 0.4)], 3.0);
        assert_eq!(s.tally(), (0, 1, 0), "a known false positive stays rejected");
    }

    #[test]
    fn an_unreviewed_detection_tracks_the_newer_timestamp() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        s.merge_detections(vec![det(Kind::Death, 101.0, 0.9)], 3.0);
        assert_eq!(s.labels[0].at, 101.0, "a better-tuned detector may localize better");
    }

    #[test]
    fn manual_marks_survive_a_detector_that_never_finds_them() {
        let mut s = set();
        s.mark(Kind::PlayerKill, 42.0);
        s.merge_detections(vec![det(Kind::Death, 900.0, 0.9)], 3.0);
        let manual: Vec<_> = s.labels.iter().filter(|l| l.origin == Origin::Manual).collect();
        assert_eq!(manual.len(), 1);
        assert_eq!(manual[0].confirmed, Some(true));
    }

    #[test]
    fn different_kinds_at_the_same_time_do_not_merge() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::PlayerKill, 100.0, 0.9)], 3.0);
        let added = s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        assert_eq!(added, 1, "dying just after a kill is one of the commonest clips");
    }

    #[test]
    fn distant_detections_are_separate_events() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        assert_eq!(s.merge_detections(vec![det(Kind::Death, 400.0, 0.9)], 3.0), 1);
    }

    #[test]
    fn confirmed_returns_only_what_you_approved_in_time_order() {
        let mut s = set();
        s.merge_detections(
            vec![det(Kind::Death, 300.0, 0.9), det(Kind::Death, 100.0, 0.9)],
            3.0,
        );
        s.labels.iter_mut().for_each(|l| l.confirmed = Some(l.at > 200.0));
        let c = s.confirmed();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].at, 300.0);
    }

    #[test]
    fn pending_is_the_review_queue() {
        let mut s = set();
        s.merge_detections(
            vec![det(Kind::Death, 100.0, 0.9), det(Kind::Death, 400.0, 0.5)],
            3.0,
        );
        s.labels[0].confirmed = Some(true);
        assert_eq!(s.pending().len(), 1);
        assert_eq!(s.pending()[0].at, 400.0);
    }

    #[test]
    fn round_trips_through_json() {
        let mut s = set();
        s.merge_detections(vec![det(Kind::Death, 100.0, 0.9)], 3.0);
        s.labels[0].confirmed = Some(true);
        s.mark(Kind::PlayerKill, 42.0);
        let json = serde_json::to_string(&s).unwrap();
        let back: LabelSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back.labels, s.labels);
        assert_eq!(back.game_build, "2026.08.06");
    }
}
