//! Detected events, and the pure logic that turns them into editable clips.
//!
//! Deliberately free of Windows and ffmpeg so it is `cargo test`-able on any
//! platform — per this repo's confidence discipline, that makes it genuinely
//! VERIFIED rather than merely "compiles on Nick's PC".

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The local player died. Detected from the full-screen death card.
    Death,
    /// A player died on screen — the thing we ultimately want to detect.
    PlayerKill,
    /// A monster died on screen. Recorded as an explicit **negative**: the
    /// particle burst fires identically for these, so a classifier that never
    /// sees them has no way to learn the distinction that matters.
    MonsterKill,
}

/// One detection, at `at` seconds into the source video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub kind: Kind,
    pub at: f64,
    /// Detector confidence, 0.0..=1.0. Nothing here is ground truth — every
    /// event is a suggestion the user accepts or rejects in the review UI.
    pub score: f64,
    /// Set once the user has ruled on it; `None` means unreviewed.
    #[serde(default)]
    pub confirmed: Option<bool>,
}

impl Event {
    pub fn new(kind: Kind, at: f64, score: f64) -> Self {
        Self { kind, at, score, confirmed: None }
    }
}

impl Kind {
    /// Every variant, so callers that map to/from strings can be checked
    /// against the full set instead of quietly handling a subset.
    pub const ALL: [Kind; 3] = [Kind::Death, Kind::PlayerKill, Kind::MonsterKill];

    /// The wire name used by the review UI. Must match serde's `lowercase`
    /// renaming so a mark round-trips through the label file unchanged.
    pub fn wire_name(self) -> &'static str {
        match self {
            Kind::Death => "death",
            Kind::PlayerKill => "playerkill",
            Kind::MonsterKill => "monsterkill",
        }
    }

    /// Parse a wire name. `None` for anything unrecognized — deliberately not
    /// defaulting, because a mark silently stored as the wrong kind is worse
    /// than one refused outright.
    pub fn from_wire(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.wire_name() == s)
    }
}

/// Collapse a burst of detections of the same kind into one event.
///
/// A detector sampling at several frames a second fires repeatedly across the
/// seconds a death card is on screen. Without this, one death becomes thirty
/// events. Keeps the highest-scoring member of each burst.
pub fn cluster(mut events: Vec<Event>, window: f64) -> Vec<Event> {
    events.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Event> = Vec::new();
    for ev in events {
        match out.last_mut() {
            Some(last) if last.kind == ev.kind && ev.at - last.at <= window => {
                if ev.score > last.score {
                    // Keep the strongest detection, but anchor the event at the
                    // *start* of the burst — that's when the thing happened.
                    let at = last.at;
                    *last = ev;
                    last.at = at;
                }
            }
            _ => out.push(ev),
        }
    }
    out
}

/// A source range to cut, in seconds.
pub type Segment = (f64, f64);

/// Turn events into clip segments, merging any that overlap.
///
/// `pre`/`post` are the roll either side of the event. Ranges are clamped to
/// `[0, duration]`, and overlapping ranges merge so two kills five seconds
/// apart become one continuous clip rather than two clips sharing footage.
pub fn segments(events: &[Event], pre: f64, post: f64, duration: f64) -> Vec<Segment> {
    let mut ranges: Vec<Segment> = events
        .iter()
        .map(|e| ((e.at - pre).max(0.0), (e.at + post).min(duration)))
        .filter(|(s, e)| e > s)
        .collect();
    ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged: Vec<Segment> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: Kind, at: f64, score: f64) -> Event {
        Event::new(kind, at, score)
    }

    #[test]
    fn every_kind_round_trips_through_its_wire_name() {
        // This is the test that was missing. Adding Kind::MonsterKill left the
        // review window's string match with a catch-all that recorded it as a
        // Death — the mark was wrong on disk, not just mislabeled on screen.
        for k in Kind::ALL {
            assert_eq!(Kind::from_wire(k.wire_name()), Some(k), "{k:?} did not round-trip");
        }
    }

    #[test]
    fn wire_names_match_the_serialized_form() {
        // The UI sends what serde writes. If these drift, a mark saved by one
        // and read by the other silently changes kind.
        for k in Kind::ALL {
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json.trim_matches('"'), k.wire_name(), "{k:?} name/serde mismatch");
        }
    }

    #[test]
    fn wire_names_are_all_distinct() {
        let mut names: Vec<&str> = Kind::ALL.iter().map(|k| k.wire_name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two kinds share a wire name");
    }

    #[test]
    fn an_unknown_wire_name_is_refused_rather_than_defaulted() {
        assert_eq!(Kind::from_wire("bogus"), None);
        assert_eq!(Kind::from_wire(""), None);
        assert_eq!(Kind::from_wire("Death"), None, "matching is exact, not case-folded");
    }

    #[test]
    fn cluster_collapses_a_burst_into_one_event() {
        let raw = vec![
            ev(Kind::Death, 100.0, 0.7),
            ev(Kind::Death, 100.25, 0.9),
            ev(Kind::Death, 100.5, 0.8),
        ];
        let out = cluster(raw, 1.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].at, 100.0, "event anchors at the start of the burst");
        assert_eq!(out[0].score, 0.9, "keeps the strongest detection");
    }

    #[test]
    fn cluster_keeps_events_further_apart_than_the_window() {
        let out = cluster(vec![ev(Kind::Death, 10.0, 0.9), ev(Kind::Death, 40.0, 0.9)], 1.0);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn cluster_does_not_merge_across_kinds() {
        let out = cluster(vec![ev(Kind::PlayerKill, 10.0, 0.9), ev(Kind::Death, 10.1, 0.9)], 1.0);
        assert_eq!(out.len(), 2, "a kill and a death at the same moment are both real");
    }

    #[test]
    fn cluster_sorts_unordered_input() {
        let out = cluster(vec![ev(Kind::Death, 50.0, 0.9), ev(Kind::Death, 10.0, 0.9)], 1.0);
        assert_eq!(out[0].at, 10.0);
    }

    #[test]
    fn segments_apply_pre_and_post_roll() {
        let out = segments(&[ev(Kind::PlayerKill, 100.0, 1.0)], 12.0, 4.0, 600.0);
        assert_eq!(out, vec![(88.0, 104.0)]);
    }

    #[test]
    fn segments_clamp_to_the_source() {
        let out = segments(&[ev(Kind::Death, 3.0, 1.0)], 12.0, 4.0, 5.0);
        assert_eq!(out, vec![(0.0, 5.0)], "no negative start, no reading past the end");
    }

    #[test]
    fn segments_merge_when_events_are_close() {
        let out = segments(
            &[ev(Kind::PlayerKill, 100.0, 1.0), ev(Kind::PlayerKill, 105.0, 1.0)],
            12.0,
            4.0,
            600.0,
        );
        assert_eq!(out, vec![(88.0, 109.0)], "one continuous clip, not two overlapping ones");
    }

    #[test]
    fn segments_keep_distant_events_separate() {
        let out = segments(
            &[ev(Kind::PlayerKill, 100.0, 1.0), ev(Kind::PlayerKill, 300.0, 1.0)],
            12.0,
            4.0,
            600.0,
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn segments_of_nothing_is_nothing() {
        assert!(segments(&[], 12.0, 4.0, 600.0).is_empty());
    }
}
