//! The trainable part of the kill classifier — pure Rust, no Python.
//!
//! **Why this can be a linear model.** Fine-tuning a whole CNN needs PyTorch,
//! a CUDA toolkit and gigabytes of dependencies. Instead a *frozen* pretrained
//! vision model turns each frame into a feature vector, and only this small
//! classifier is trained on top. That's fast enough to run in-app with a
//! progress bar, and it works here specifically because enemy cosmetics don't
//! render for Nick: the classes are visually distinct known objects, not
//! subtle variations of each other, which is the case frozen features handle
//! well.
//!
//! Everything in this file is plain arithmetic with no Windows or ffmpeg
//! dependency, so its tests genuinely verify it on any platform — unlike the
//! inference glue that has to be checked on hardware.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Softmax regression over pre-extracted features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classifier {
    pub classes: Vec<String>,
    /// `[n_classes][n_features]`
    pub weights: Vec<Vec<f32>>,
    pub bias: Vec<f32>,
    /// Feature standardization, stored with the model so inference applies
    /// exactly the same transform training did. Getting this wrong silently
    /// destroys accuracy at inference time while training still looks fine.
    pub feature_mean: Vec<f32>,
    pub feature_std: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct TrainConfig {
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f32,
    /// L2 penalty. Small datasets overfit fast; this is the main guard.
    pub l2: f32,
    pub seed: u64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self { epochs: 60, batch_size: 32, learning_rate: 0.05, l2: 1e-4, seed: 0 }
    }
}

/// Held-out evaluation. Accuracy alone hides a class the model never gets
/// right, which is exactly the failure that matters when one class is rare —
/// so per-class recall is reported alongside it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evaluation {
    pub accuracy: f64,
    pub recall: BTreeMap<String, f64>,
    pub support: BTreeMap<String, usize>,
}

/// Deterministic shuffling, so two training runs on the same data are
/// comparable. A silently reshuffled run makes "did that change help?"
/// unanswerable.
struct Lcg(u64);

impl Lcg {
    fn next_usize(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

fn softmax(logits: &mut [f32]) {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for v in logits.iter_mut() {
        *v = (*v - max).exp(); // shift for numerical stability
        sum += *v;
    }
    for v in logits.iter_mut() {
        *v /= sum.max(1e-12);
    }
}

impl Classifier {
    /// Fit on `features` with integer `labels` indexing into `classes`.
    ///
    /// Classes are weighted by inverse frequency: with far more "nothing
    /// happened" frames than kills, an unweighted model scores well by never
    /// predicting a kill at all.
    pub fn train(
        features: &[Vec<f32>],
        labels: &[usize],
        classes: Vec<String>,
        cfg: TrainConfig,
        mut on_progress: impl FnMut(usize, usize, f32),
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!features.is_empty(), "no training examples");
        anyhow::ensure!(features.len() == labels.len(), "features/labels length mismatch");
        anyhow::ensure!(!classes.is_empty(), "no classes");
        let n_features = features[0].len();
        anyhow::ensure!(n_features > 0, "features are empty");
        anyhow::ensure!(
            features.iter().all(|f| f.len() == n_features),
            "all feature vectors must be the same length"
        );
        anyhow::ensure!(
            labels.iter().all(|&l| l < classes.len()),
            "a label indexes past the end of the class list"
        );

        let n_classes = classes.len();
        let (mean, std) = standardize_params(features, n_features);
        let x: Vec<Vec<f32>> = features
            .iter()
            .map(|f| apply_standardize(f, &mean, &std))
            .collect();

        // Inverse-frequency class weights.
        let mut counts = vec![0usize; n_classes];
        for &l in labels {
            counts[l] += 1;
        }
        let total = labels.len() as f32;
        let weight: Vec<f32> = counts
            .iter()
            .map(|&c| if c == 0 { 0.0 } else { total / (n_classes as f32 * c as f32) })
            .collect();

        let mut w = vec![vec![0.0f32; n_features]; n_classes];
        let mut b = vec![0.0f32; n_classes];

        let mut order: Vec<usize> = (0..x.len()).collect();
        let mut rng = Lcg(cfg.seed.wrapping_add(0x9E37_79B9_7F4A_7C15));

        for epoch in 0..cfg.epochs {
            // Fisher-Yates with the deterministic generator.
            for i in (1..order.len()).rev() {
                let j = rng.next_usize(i + 1);
                order.swap(i, j);
            }

            let mut epoch_loss = 0.0f32;
            for chunk in order.chunks(cfg.batch_size.max(1)) {
                let mut gw = vec![vec![0.0f32; n_features]; n_classes];
                let mut gb = vec![0.0f32; n_classes];
                let scale = 1.0 / chunk.len() as f32;

                for &idx in chunk {
                    let xi = &x[idx];
                    let yi = labels[idx];
                    let cw = weight[yi];

                    let mut logits = vec![0.0f32; n_classes];
                    for c in 0..n_classes {
                        let mut s = b[c];
                        for f in 0..n_features {
                            s += w[c][f] * xi[f];
                        }
                        logits[c] = s;
                    }
                    softmax(&mut logits);
                    epoch_loss -= cw * logits[yi].max(1e-12).ln();

                    for c in 0..n_classes {
                        let d = cw * (logits[c] - if c == yi { 1.0 } else { 0.0 });
                        gb[c] += d * scale;
                        for f in 0..n_features {
                            gw[c][f] += d * xi[f] * scale;
                        }
                    }
                }

                for c in 0..n_classes {
                    b[c] -= cfg.learning_rate * gb[c];
                    for f in 0..n_features {
                        // L2 shrinkage applied to weights only, never bias —
                        // penalizing the bias just fights class priors.
                        w[c][f] -= cfg.learning_rate * (gw[c][f] + cfg.l2 * w[c][f]);
                    }
                }
            }
            on_progress(epoch + 1, cfg.epochs, epoch_loss / x.len() as f32);
        }

        Ok(Self { classes, weights: w, bias: b, feature_mean: mean, feature_std: std })
    }

    /// Class probabilities for one raw (un-standardized) feature vector.
    pub fn predict_proba(&self, features: &[f32]) -> Vec<f32> {
        let x = apply_standardize(features, &self.feature_mean, &self.feature_std);
        let mut logits: Vec<f32> = self
            .weights
            .iter()
            .zip(&self.bias)
            .map(|(wc, bc)| bc + wc.iter().zip(&x).map(|(a, b)| a * b).sum::<f32>())
            .collect();
        softmax(&mut logits);
        logits
    }

    /// Most likely class and its probability.
    pub fn predict(&self, features: &[f32]) -> (usize, f32) {
        let p = self.predict_proba(features);
        let mut best = 0;
        for i in 1..p.len() {
            if p[i] > p[best] {
                best = i;
            }
        }
        (best, p[best])
    }

    pub fn evaluate(&self, features: &[Vec<f32>], labels: &[usize]) -> Evaluation {
        let mut correct = 0usize;
        let mut per_class: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for (f, &y) in features.iter().zip(labels) {
            let (p, _) = self.predict(f);
            let entry = per_class.entry(self.classes[y].clone()).or_insert((0, 0));
            entry.1 += 1;
            if p == y {
                entry.0 += 1;
                correct += 1;
            }
        }
        Evaluation {
            accuracy: if features.is_empty() {
                0.0
            } else {
                correct as f64 / features.len() as f64
            },
            recall: per_class
                .iter()
                .map(|(k, (c, n))| (k.clone(), if *n == 0 { 0.0 } else { *c as f64 / *n as f64 }))
                .collect(),
            support: per_class.iter().map(|(k, (_, n))| (k.clone(), *n)).collect(),
        }
    }
}

fn standardize_params(features: &[Vec<f32>], n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut mean = vec![0.0f32; n];
    for f in features {
        for i in 0..n {
            mean[i] += f[i];
        }
    }
    for m in mean.iter_mut() {
        *m /= features.len() as f32;
    }
    let mut std = vec![0.0f32; n];
    for f in features {
        for i in 0..n {
            let d = f[i] - mean[i];
            std[i] += d * d;
        }
    }
    for s in std.iter_mut() {
        // A constant feature has zero variance; clamp so it becomes zero after
        // standardizing rather than producing NaN and poisoning every weight.
        *s = (*s / features.len() as f32).sqrt().max(1e-6);
    }
    (mean, std)
}

fn apply_standardize(f: &[f32], mean: &[f32], std: &[f32]) -> Vec<f32> {
    f.iter()
        .zip(mean)
        .zip(std)
        .map(|((v, m), s)| (v - m) / s)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes() -> Vec<String> {
        vec!["a".to_string(), "b".to_string()]
    }

    /// Two clearly separated clusters in 2D.
    fn separable() -> (Vec<Vec<f32>>, Vec<usize>) {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in 0..40 {
            let t = i as f32 * 0.1;
            x.push(vec![1.0 + t * 0.05, 1.0 - t * 0.05]);
            y.push(0);
            x.push(vec![5.0 + t * 0.05, 5.0 - t * 0.05]);
            y.push(1);
        }
        (x, y)
    }

    #[test]
    fn learns_a_separable_problem() {
        let (x, y) = separable();
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let ev = m.evaluate(&x, &y);
        assert!(ev.accuracy > 0.95, "accuracy was {}", ev.accuracy);
    }

    #[test]
    fn training_is_deterministic() {
        let (x, y) = separable();
        let a = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let b = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        assert_eq!(a.weights, b.weights, "same data + seed must give the same model");
    }

    #[test]
    fn a_rare_class_is_still_learned() {
        // 100 of class 0, 6 of class 1. Without inverse-frequency weighting a
        // model scores 94% by always answering 0 and never finding a kill.
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in 0..100 {
            x.push(vec![0.0 + (i % 5) as f32 * 0.01, 0.0]);
            y.push(0);
        }
        for i in 0..6 {
            x.push(vec![4.0 + i as f32 * 0.01, 4.0]);
            y.push(1);
        }
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let ev = m.evaluate(&x, &y);
        assert!(
            ev.recall.get("b").copied().unwrap_or(0.0) > 0.8,
            "rare-class recall was {:?} — the weighting isn't working",
            ev.recall
        );
    }

    #[test]
    fn evaluation_reports_per_class_recall_and_support() {
        let (x, y) = separable();
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let ev = m.evaluate(&x, &y);
        assert_eq!(ev.support.get("a"), Some(&40));
        assert_eq!(ev.support.get("b"), Some(&40));
        assert!(ev.recall.contains_key("a") && ev.recall.contains_key("b"));
    }

    #[test]
    fn probabilities_are_a_distribution() {
        let (x, y) = separable();
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let p = m.predict_proba(&x[0]);
        let sum: f32 = p.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4, "probabilities summed to {sum}");
        assert!(p.iter().all(|v| *v >= 0.0 && *v <= 1.0));
    }

    #[test]
    fn predict_agrees_with_the_highest_probability() {
        let (x, y) = separable();
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        for f in x.iter().take(10) {
            let p = m.predict_proba(f);
            let (idx, prob) = m.predict(f);
            let best = p.iter().cloned().fold(f32::MIN, f32::max);
            assert!((prob - best).abs() < 1e-6);
            assert!((p[idx] - best).abs() < 1e-6);
        }
    }

    #[test]
    fn a_constant_feature_does_not_produce_nan() {
        // Zero-variance column: if standardization divides by zero the whole
        // model becomes NaN and every prediction is garbage.
        let x = vec![vec![1.0, 7.0], vec![2.0, 7.0], vec![8.0, 7.0], vec![9.0, 7.0]];
        let y = vec![0, 0, 1, 1];
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        assert!(m.weights.iter().flatten().all(|v| v.is_finite()));
        assert!(m.predict_proba(&x[0]).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn round_trips_through_json() {
        let (x, y) = separable();
        let m = Classifier::train(&x, &y, classes(), TrainConfig::default(), |_, _, _| {}).unwrap();
        let back: Classifier = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.predict(&x[0]).0, m.predict(&x[0]).0);
        assert_eq!(back.feature_mean, m.feature_mean);
    }

    #[test]
    fn progress_is_reported_once_per_epoch() {
        let (x, y) = separable();
        let cfg = TrainConfig { epochs: 5, ..Default::default() };
        let mut seen = Vec::new();
        Classifier::train(&x, &y, classes(), cfg, |e, total, _| {
            seen.push((e, total));
        })
        .unwrap();
        assert_eq!(seen.len(), 5);
        assert_eq!(seen[4], (5, 5));
    }

    #[test]
    fn malformed_input_is_rejected_rather_than_producing_a_bad_model() {
        let cfg = TrainConfig::default();
        assert!(Classifier::train(&[], &[], classes(), cfg, |_, _, _| {}).is_err());
        assert!(Classifier::train(&[vec![1.0]], &[], classes(), cfg, |_, _, _| {}).is_err());
        // Ragged features.
        assert!(
            Classifier::train(&[vec![1.0], vec![1.0, 2.0]], &[0, 1], classes(), cfg, |_, _, _| {})
                .is_err()
        );
        // Label past the end of the class list.
        assert!(
            Classifier::train(&[vec![1.0], vec![2.0]], &[0, 9], classes(), cfg, |_, _, _| {})
                .is_err()
        );
    }
}
