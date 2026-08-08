//! Training-dataset export: turns the label store into images a model can
//! learn from.
//!
//! **Why a whole-frame classifier rather than a nameplate detector.** The
//! PvP-specific nameplate icon is ~11 px at 1080p — genuinely hard small-object
//! detection. The dying *character model* is a much larger target, and Nick
//! confirmed (2026-08-08) that enemy cosmetics don't render for him, so the
//! visual vocabulary is closed: a given class in a given armor tier looks
//! identical every time, and each map's monsters are a fixed roster. That's a
//! far smaller learning problem, and — critically — it trains on the K/M/D
//! marks he already makes. No boxes required.
//!
//! Fight length is irrelevant here even though it varies a lot (solos vs
//! trios): the window sampled is a second either side of the death instant,
//! not the fight. Fight length only affects clip pre/post-roll, which is
//! already per-profile.

use serde::{Deserialize, Serialize};

use crate::event::Kind;

/// Export resolution. 16:9 keeps HUD geometry undistorted, and it's small
/// enough that training on one GPU is quick.
pub const FRAME_W: u32 = 384;
pub const FRAME_H: u32 = 216;

/// Seconds either side of a mark to sample.
///
/// Deliberately wider than the death animation. Marks land wherever the
/// playhead was when the key was pressed, and re-deriving true onsets from the
/// burst detector previously found up to 1.85 s of drift against eyeballed
/// timestamps. A ±1 s spread absorbs that, and the off-centre frames double as
/// augmentation rather than waste.
pub const WINDOW: f64 = 1.0;

/// Frames per second within that window: 9 frames per marked event.
pub const SAMPLE_FPS: f64 = 4.0;

/// Minimum distance from any labelled event for a frame to count as "nothing
/// died here". Generous, because an unmarked kill leaking into the negatives
/// is far more damaging than having fewer negatives.
pub const NEGATIVE_MIN_GAP: f64 = 8.0;

/// Class label for the model. Free-form strings rather than an enum so a new
/// class can be added without breaking previously-exported manifests.
pub fn class_of(kind: Kind) -> &'static str {
    match kind {
        Kind::PlayerKill => "player",
        Kind::MonsterKill => "monster",
        Kind::Death => "own_death",
    }
}

pub const CLASS_NONE: &str = "none";

/// One exported training image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Example {
    /// Path relative to the dataset root.
    pub file: String,
    pub class: String,
    /// Source video, for provenance and for grouping when splitting train/test.
    pub source: String,
    /// Game build this footage came from — the mechanism that lets a model
    /// evolve across patches instead of silently rotting. See `model.rs`.
    pub game_build: String,
    /// Timestamp of the event this frame belongs to.
    pub at: f64,
    /// Offset of this frame from `at`, in seconds.
    pub offset: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub frame_width: u32,
    pub frame_height: u32,
    pub examples: Vec<Example>,
}

impl Manifest {
    pub fn new() -> Self {
        Self { frame_width: FRAME_W, frame_height: FRAME_H, examples: Vec::new() }
    }

    /// Count per class — the first thing to check before training, since a
    /// wildly imbalanced set will train to a useless majority-class predictor.
    pub fn class_counts(&self) -> std::collections::BTreeMap<String, usize> {
        let mut m = std::collections::BTreeMap::new();
        for e in &self.examples {
            *m.entry(e.class.clone()).or_insert(0) += 1;
        }
        m
    }

    /// Distinct game builds represented, so a model card can record exactly
    /// what it was trained against.
    pub fn game_builds(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .examples
            .iter()
            .map(|e| e.game_build.clone())
            .filter(|b| !b.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Frame offsets sampled around one event.
pub fn offsets() -> Vec<f64> {
    let n = (2.0 * WINDOW * SAMPLE_FPS).round() as i32;
    (0..=n).map(|i| -WINDOW + i as f64 / SAMPLE_FPS).collect()
}

/// Deterministically choose timestamps that are far from every labelled event.
///
/// Deterministic rather than random so a re-export produces the same negatives
/// — otherwise the train/test split silently shifts under you between runs and
/// two training runs aren't comparable.
pub fn negative_times(marks: &[f64], duration: f64, want: usize, min_gap: f64) -> Vec<f64> {
    if duration <= 0.0 || want == 0 {
        return Vec::new();
    }
    // Oversample candidate positions, then keep the ones far enough away.
    let step = (duration / (want as f64 * 4.0)).max(0.5);
    let mut out = Vec::new();
    let mut t = step;
    while t < duration && out.len() < want {
        if marks.iter().all(|m| (m - t).abs() >= min_gap) {
            out.push((t * 1000.0).round() / 1000.0);
        }
        t += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_span_the_window_and_centre_on_zero() {
        let o = offsets();
        assert_eq!(o.len(), 9, "9 frames per event at 4fps over ±1s");
        assert!((o[0] + WINDOW).abs() < 1e-9);
        assert!((o[o.len() - 1] - WINDOW).abs() < 1e-9);
        assert!(o.iter().any(|v| v.abs() < 1e-9), "one frame sits on the mark");
    }

    #[test]
    fn classes_are_distinct_per_kind() {
        assert_eq!(class_of(Kind::PlayerKill), "player");
        assert_eq!(class_of(Kind::MonsterKill), "monster");
        assert_eq!(class_of(Kind::Death), "own_death");
        assert_ne!(class_of(Kind::PlayerKill), class_of(Kind::MonsterKill));
    }

    #[test]
    fn negatives_stay_clear_of_every_mark() {
        let marks = vec![10.0, 50.0, 120.0];
        let negs = negative_times(&marks, 200.0, 10, 8.0);
        assert!(!negs.is_empty());
        for n in &negs {
            for m in &marks {
                assert!((n - m).abs() >= 8.0, "negative at {n} too close to mark at {m}");
            }
        }
    }

    #[test]
    fn negatives_are_deterministic() {
        let marks = vec![10.0, 50.0];
        let a = negative_times(&marks, 200.0, 8, 8.0);
        let b = negative_times(&marks, 200.0, 8, 8.0);
        assert_eq!(a, b, "a re-export must not reshuffle the train/test split");
    }

    #[test]
    fn negatives_respect_the_requested_count() {
        let negs = negative_times(&[], 300.0, 5, 8.0);
        assert_eq!(negs.len(), 5);
    }

    #[test]
    fn a_densely_marked_recording_yields_few_or_no_negatives() {
        // Every second marked: there is no clean negative to be had, and the
        // exporter must return nothing rather than mislabel a kill as "none".
        let marks: Vec<f64> = (0..60).map(|i| i as f64).collect();
        let negs = negative_times(&marks, 60.0, 10, 8.0);
        assert!(negs.is_empty());
    }

    #[test]
    fn zero_duration_is_handled() {
        assert!(negative_times(&[1.0], 0.0, 5, 8.0).is_empty());
    }

    #[test]
    fn manifest_counts_and_builds() {
        let mut m = Manifest::new();
        for (class, build) in [("player", "2026.08.06"), ("player", "2026.08.06"),
                               ("monster", "2026.08.13")] {
            m.examples.push(Example {
                file: format!("{class}/x.jpg"),
                class: class.into(),
                source: "v.mp4".into(),
                game_build: build.into(),
                at: 1.0,
                offset: 0.0,
            });
        }
        let counts = m.class_counts();
        assert_eq!(counts.get("player"), Some(&2));
        assert_eq!(counts.get("monster"), Some(&1));
        assert_eq!(m.game_builds(), vec!["2026.08.06", "2026.08.13"]);
    }

    #[test]
    fn empty_builds_are_not_recorded() {
        let mut m = Manifest::new();
        m.examples.push(Example {
            file: "a.jpg".into(),
            class: "player".into(),
            source: "v.mp4".into(),
            game_build: String::new(),
            at: 1.0,
            offset: 0.0,
        });
        assert!(m.game_builds().is_empty(), "unknown build must not become a build named \"\"");
    }
}
