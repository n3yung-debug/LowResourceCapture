//! Detectors. Currently: the death card.
//!
//! Every threshold here was measured against Nick's 9:22 Mistfall Hunter VOD
//! rather than guessed — three earlier attempts at a particle metric failed
//! precisely because they guessed. See CLAUDE.md for the method warning.
//!
//! **Death card measurement (1080p, 2 fps, full VOD, 1123 frames):**
//! the card's gold button row scores 0.443–0.500 across the five frames it is
//! on screen, and the loudest non-death frame in the other nine minutes scores
//! 0.178. A threshold of 0.30 sits in the gap with ~2.5x margin either way.

use crate::event::{Event, Kind};
use crate::profile::Region;

/// Where the Spectate / Return-to-Camp button row sits, normalized.
pub const DEATH_ROI: Region =
    Region { x: 0.250, y: 0.866, w: 0.500, h: 0.065, only_at: None };

/// Fraction of the ROI that must read as button-gold to call it a death.
/// Measured floor 0.443, measured ceiling for non-deaths 0.178.
pub const DEATH_THRESHOLD: f64 = 0.30;

/// Frames per second to sample. The card is up for >2.5s, so 2 fps sees it
/// several times over and clustering collapses those into one event.
pub const SAMPLE_FPS: u32 = 2;

/// Fraction of RGB24 pixels matching the death card's button gold.
///
/// Bright, strongly red-dominant over blue, and never green-dominant — that
/// last clause is what keeps the game's teal/blue UI out of the count.
pub fn gold_fraction(rgb: &[u8]) -> f64 {
    if rgb.len() < 3 {
        return 0.0;
    }
    let px = rgb.len() / 3;
    let mut hits = 0usize;
    for p in rgb.chunks_exact(3) {
        let (r, g, b) = (p[0] as i16, p[1] as i16, p[2] as i16);
        if r > 120 && r - b > 60 && g - b > 30 && r >= g {
            hits += 1;
        }
    }
    hits as f64 / px as f64
}

/// Score one sampled frame and, if it clears the bar, emit a death event.
///
/// Score maps onto 0..=1 confidence by how far past the threshold it sits, so
/// the review UI can sort the marginal ones to the top for a human look.
pub fn score_frame(rgb: &[u8], at: f64) -> Option<Event> {
    let f = gold_fraction(rgb);
    if f < DEATH_THRESHOLD {
        return None;
    }
    let headroom = ((f - DEATH_THRESHOLD) / (0.45 - DEATH_THRESHOLD)).clamp(0.0, 1.0);
    Some(Event::new(Kind::Death, at, 0.5 + 0.5 * headroom))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an RGB24 buffer where `frac` of pixels are the given colour and
    /// the rest are a dark neutral, as the scene behind the card is.
    fn buf(frac: f64, colour: (u8, u8, u8)) -> Vec<u8> {
        let n = 1000;
        let gold = (n as f64 * frac).round() as usize;
        let mut v = Vec::with_capacity(n * 3);
        for i in 0..n {
            let (r, g, b) = if i < gold { colour } else { (40, 42, 48) };
            v.extend_from_slice(&[r, g, b]);
        }
        v
    }

    const BUTTON_GOLD: (u8, u8, u8) = (200, 165, 60);

    #[test]
    fn gold_fraction_counts_button_pixels() {
        assert!((gold_fraction(&buf(0.5, BUTTON_GOLD)) - 0.5).abs() < 0.01);
        assert!(gold_fraction(&buf(0.0, BUTTON_GOLD)) < 0.001);
    }

    #[test]
    fn dark_scene_scores_near_zero() {
        assert!(gold_fraction(&buf(1.0, (40, 42, 48))) < 0.001);
    }

    #[test]
    fn teal_ui_is_not_gold() {
        // The game's ability icons and minimap are blue/teal — the r >= g and
        // r - b > 60 clauses must reject them or every HUD frame scores high.
        assert!(gold_fraction(&buf(1.0, (60, 180, 200))) < 0.001);
    }

    #[test]
    fn measured_death_level_fires_and_non_death_level_does_not() {
        // 0.443 was the quietest measured death frame; 0.178 the loudest
        // measured non-death frame, both over the full VOD.
        assert!(score_frame(&buf(0.443, BUTTON_GOLD), 10.0).is_some());
        assert!(score_frame(&buf(0.178, BUTTON_GOLD), 10.0).is_none());
    }

    #[test]
    fn score_rises_with_confidence_and_stays_bounded() {
        let weak = score_frame(&buf(0.31, BUTTON_GOLD), 1.0).unwrap();
        let strong = score_frame(&buf(0.50, BUTTON_GOLD), 1.0).unwrap();
        assert!(strong.score > weak.score);
        assert!(strong.score <= 1.0 && weak.score >= 0.5);
    }

    #[test]
    fn event_carries_its_timestamp_and_kind() {
        let e = score_frame(&buf(0.45, BUTTON_GOLD), 559.0).unwrap();
        assert_eq!(e.kind, Kind::Death);
        assert_eq!(e.at, 559.0);
        assert_eq!(e.confirmed, None, "detections are suggestions, not verdicts");
    }

    #[test]
    fn empty_and_ragged_buffers_do_not_panic() {
        assert_eq!(gold_fraction(&[]), 0.0);
        assert_eq!(gold_fraction(&[255, 255]), 0.0);
    }
}
