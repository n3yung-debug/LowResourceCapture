//! Model metadata — what a trained model was built from, and when it's stale.
//!
//! Mistfall Hunter patches weekly and has already removed a kill feed once. A
//! model trained against one build's armor sets and monster roster can quietly
//! stop working after a patch, and a classifier that has silently degraded is
//! worse than no classifier: it still returns confident answers.
//!
//! So every model ships a card recording exactly which builds it saw, and the
//! analyzer checks the current build against it. Same discipline as
//! `GameProfile::is_stale` — a patched game invalidates a model the way a
//! driver change invalidates a benchmark ceiling.
//!
//! **How a model evolves across patches.** Training data is never discarded on
//! a patch. Each exported example records its own `game_build`, so retraining
//! after a patch mixes old and new footage and the card accumulates builds.
//! A class that didn't change across a patch keeps all its old examples; one
//! that did gets corrected by the newer ones as they accumulate. The staleness
//! warning is a prompt to record and retrain, not a reason to throw anything
//! away.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Sidecar written next to the exported `.onnx`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCard {
    /// ISO-8601 date the model was trained.
    #[serde(default)]
    pub trained_at: String,
    /// Every game build represented in the training data, oldest first.
    #[serde(default)]
    pub game_builds: Vec<String>,
    /// Class names, in the output order the model emits.
    #[serde(default)]
    pub classes: Vec<String>,
    /// Training examples per class — the honest read on whether a class has
    /// enough data to be trusted, independent of overall accuracy.
    #[serde(default)]
    pub examples_per_class: BTreeMap<String, usize>,
    #[serde(default)]
    pub input_width: u32,
    #[serde(default)]
    pub input_height: u32,
    /// Accuracy on held-out data. `None` means it was never evaluated, which
    /// is not the same as zero and must not be displayed as a score.
    #[serde(default)]
    pub holdout_accuracy: Option<f64>,
    /// Per-class held-out recall. An overall accuracy number hides a class
    /// the model never gets right, which is exactly the failure that matters
    /// when one class is rare.
    #[serde(default)]
    pub holdout_recall: BTreeMap<String, f64>,
    #[serde(default)]
    pub notes: String,
}

impl ModelCard {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("reading model card {}", path.display()))?;
        serde_json::from_str(&s).context("parsing model card")
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing model card {}", path.display()))
    }

    /// True if `current_build` isn't among the builds this model was trained
    /// on. An unknown current build is *not* treated as stale — guessing
    /// "stale" on missing information would cry wolf on every scan.
    pub fn is_stale(&self, current_build: &str) -> bool {
        if current_build.is_empty() || self.game_builds.is_empty() {
            return false;
        }
        !self.game_builds.iter().any(|b| b == current_build)
    }

    /// Classes with too few examples to trust, whatever the headline accuracy
    /// says. Surfacing this is the difference between "the model is 94%
    /// accurate" and "the model is 94% accurate because it never sees
    /// monsters and there are barely any in the set".
    pub fn undertrained_classes(&self, min_examples: usize) -> Vec<String> {
        self.classes
            .iter()
            .filter(|c| self.examples_per_class.get(*c).copied().unwrap_or(0) < min_examples)
            .cloned()
            .collect()
    }

    /// One-line status for the review window.
    pub fn summary(&self, current_build: &str) -> String {
        let total: usize = self.examples_per_class.values().sum();
        let acc = match self.holdout_accuracy {
            Some(a) => format!("{:.0}% held-out", a * 100.0),
            None => "not evaluated".to_string(),
        };
        let stale = if self.is_stale(current_build) {
            format!(" — STALE: trained on {}, game is {current_build}", self.game_builds.join(", "))
        } else {
            String::new()
        };
        format!("model: {total} examples, {acc}{stale}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> ModelCard {
        ModelCard {
            trained_at: "2026-08-08".into(),
            game_builds: vec!["2026.08.06".into()],
            classes: vec!["player".into(), "monster".into(), "none".into()],
            examples_per_class: [("player".to_string(), 120), ("monster".to_string(), 8),
                                 ("none".to_string(), 200)].into_iter().collect(),
            input_width: 384,
            input_height: 216,
            holdout_accuracy: Some(0.94),
            holdout_recall: [("player".to_string(), 0.91)].into_iter().collect(),
            notes: String::new(),
        }
    }

    #[test]
    fn a_patched_game_makes_a_model_stale() {
        assert!(card().is_stale("2026.08.13"));
        assert!(!card().is_stale("2026.08.06"));
    }

    #[test]
    fn unknown_current_build_is_not_stale() {
        assert!(!card().is_stale(""), "missing info must not cry wolf on every scan");
    }

    #[test]
    fn a_model_trained_across_several_builds_accepts_all_of_them() {
        let mut c = card();
        c.game_builds = vec!["2026.08.06".into(), "2026.08.13".into()];
        assert!(!c.is_stale("2026.08.06"));
        assert!(!c.is_stale("2026.08.13"));
        assert!(c.is_stale("2026.08.20"));
    }

    #[test]
    fn undertrained_classes_are_named_even_when_accuracy_looks_good() {
        let under = card().undertrained_classes(30);
        assert_eq!(under, vec!["monster"], "8 monster examples behind 94% accuracy");
    }

    #[test]
    fn summary_flags_staleness_and_missing_evaluation() {
        let c = card();
        assert!(c.summary("2026.08.13").contains("STALE"));
        assert!(!c.summary("2026.08.06").contains("STALE"));

        let mut unevaluated = card();
        unevaluated.holdout_accuracy = None;
        assert!(unevaluated.summary("2026.08.06").contains("not evaluated"));
        assert!(!unevaluated.summary("2026.08.06").contains('%'));
    }

    #[test]
    fn round_trips_through_json() {
        let c = card();
        let back: ModelCard = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.game_builds, c.game_builds);
        assert_eq!(back.examples_per_class, c.examples_per_class);
        assert_eq!(back.holdout_accuracy, c.holdout_accuracy);
    }

    #[test]
    fn an_empty_card_is_never_stale_and_never_claims_a_score() {
        let c = ModelCard::default();
        assert!(!c.is_stale("2026.08.13"));
        assert!(c.summary("2026.08.13").contains("not evaluated"));
    }
}
