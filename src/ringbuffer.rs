//! In-RAM ring buffer of *encoded* frames.
//!
//! We never keep raw frames in RAM — those are huge (a single 1440p frame is
//! ~10 MB). Instead the GPU encoder hands us small compressed samples, and we
//! keep only the last N seconds of those. Encoded 1440p60 at 40 Mbps is
//! ~5 MB/s, so a 120 s cap is ~600 MB worst case — bounded by both a time cap
//! and a hard RAM cap.
//!
//! On a "save last N seconds" request we walk back N seconds, then back a
//! little further to the nearest keyframe at or before that point, so the
//! extracted clip is independently decodable. (You can't start an H.264/HEVC
//! stream mid-GOP.)
//!
//! This module is pure Rust and unit-tested — it has no Windows dependency,
//! so its logic is `VERIFIED` by `cargo test` even though the surrounding
//! capture/encode code can only be validated on Windows.

use std::collections::VecDeque;

/// One encoded video (or audio) frame sitting in the buffer.
#[derive(Clone)]
pub struct EncodedFrame {
    /// Compressed bytes (one NAL access unit for video, one packet for audio).
    pub data: Vec<u8>,
    /// Presentation timestamp in 100-ns units (Media Foundation convention).
    pub pts_100ns: i64,
    /// Frame duration in 100-ns units.
    pub dur_100ns: i64,
    /// True if this is an IDR/keyframe (a valid clip start point). Always
    /// true for audio frames.
    pub keyframe: bool,
}

impl EncodedFrame {
    fn len_bytes(&self) -> usize {
        self.data.len()
    }
}

/// A time- and memory-bounded ring of encoded frames.
pub struct RingBuffer {
    frames: VecDeque<EncodedFrame>,
    /// Retention window in 100-ns units (max_seconds from config).
    window_100ns: i64,
    /// Hard RAM ceiling in bytes.
    max_bytes: usize,
    /// Running total of `data` bytes currently held.
    bytes: usize,
}

const HNS_PER_SEC: i64 = 10_000_000;

impl RingBuffer {
    pub fn new(max_seconds: u32, max_ram_mb: u32) -> Self {
        Self {
            frames: VecDeque::new(),
            window_100ns: max_seconds as i64 * HNS_PER_SEC,
            max_bytes: max_ram_mb as usize * 1024 * 1024,
            bytes: 0,
        }
    }

    /// Push a freshly-encoded frame and evict anything older than the window
    /// or over the RAM cap.
    pub fn push(&mut self, frame: EncodedFrame) {
        let newest_pts = frame.pts_100ns;
        self.bytes += frame.len_bytes();
        self.frames.push_back(frame);
        self.evict(newest_pts);
    }

    fn evict(&mut self, newest_pts: i64) {
        // Time-based eviction: drop frames older than the retention window.
        let cutoff = newest_pts - self.window_100ns;
        while let Some(front) = self.frames.front() {
            if front.pts_100ns < cutoff {
                let f = self.frames.pop_front().unwrap();
                self.bytes -= f.len_bytes();
            } else {
                break;
            }
        }
        // RAM-based eviction: if still over the hard cap, drop oldest until
        // under it. Keep at least one frame so the buffer is never empty
        // mid-capture.
        while self.bytes > self.max_bytes && self.frames.len() > 1 {
            let f = self.frames.pop_front().unwrap();
            self.bytes -= f.len_bytes();
        }
    }

    /// Current RAM footprint of buffered encoded data, in bytes.
    pub fn bytes_used(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Extract the last `seconds` of frames for saving, snapped back to the
    /// nearest keyframe at or before the cutoff so the result is decodable.
    ///
    /// Returns frames in presentation order. Empty if the buffer is empty.
    pub fn extract_last(&self, seconds: u32) -> Vec<EncodedFrame> {
        let Some(&last_pts) = self.frames.back().map(|f| &f.pts_100ns) else {
            return Vec::new();
        };
        let raw_cutoff = last_pts - seconds as i64 * HNS_PER_SEC;

        // Find the index of the newest keyframe whose pts <= raw_cutoff.
        // If none exists (buffer shorter than requested, or first frame is
        // after the cutoff), fall back to the first keyframe in the buffer.
        let mut start_idx = None;
        for (i, f) in self.frames.iter().enumerate() {
            if f.keyframe && f.pts_100ns <= raw_cutoff {
                start_idx = Some(i);
            }
            if f.pts_100ns > raw_cutoff {
                break;
            }
        }
        let start_idx = start_idx.or_else(|| {
            // No keyframe before the cutoff: use the earliest keyframe we have.
            self.frames.iter().position(|f| f.keyframe)
        });

        match start_idx {
            Some(i) => self.frames.iter().skip(i).cloned().collect(),
            // No keyframe at all (shouldn't happen once encoding is running).
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pts_sec: f64, dur_sec: f64, key: bool, size: usize) -> EncodedFrame {
        EncodedFrame {
            data: vec![0u8; size],
            pts_100ns: (pts_sec * HNS_PER_SEC as f64) as i64,
            dur_100ns: (dur_sec * HNS_PER_SEC as f64) as i64,
            keyframe: key,
        }
    }

    /// Build a buffer of `n` seconds at 60fps with a keyframe every 2s.
    fn fill(rb: &mut RingBuffer, n_secs: u32) {
        let fps = 60u32;
        let total = n_secs * fps;
        for i in 0..total {
            let t = i as f64 / fps as f64;
            let key = i % (2 * fps) == 0; // keyframe every 2 seconds
            rb.push(frame(t, 1.0 / fps as f64, key, 1000));
        }
    }

    #[test]
    fn evicts_beyond_time_window() {
        let mut rb = RingBuffer::new(10, 4096); // 10 s window, big RAM cap
        fill(&mut rb, 30); // push 30 s
        // Newest pts ~30s; window 10s => oldest retained pts >= ~20s.
        let frames = rb.extract_last(60); // ask for more than we hold
        let earliest = frames.first().unwrap().pts_100ns as f64 / HNS_PER_SEC as f64;
        assert!(earliest >= 18.0, "earliest was {earliest}");
    }

    #[test]
    fn extract_snaps_to_keyframe() {
        let mut rb = RingBuffer::new(60, 4096);
        fill(&mut rb, 30);
        let frames = rb.extract_last(5);
        assert!(frames.first().unwrap().keyframe, "clip must start on a keyframe");
        // Should include roughly the last 5s (+ up to 2s of keyframe snap).
        let span = (frames.last().unwrap().pts_100ns - frames.first().unwrap().pts_100ns)
            as f64
            / HNS_PER_SEC as f64;
        assert!(span >= 5.0 && span <= 7.0, "span was {span}");
    }

    #[test]
    fn respects_ram_cap() {
        // 1 MB cap; each frame 1000 bytes => ~1048 frames max.
        let mut rb = RingBuffer::new(3600, 1);
        fill(&mut rb, 60); // 3600 frames of 1000 bytes = 3.6 MB pushed
        assert!(rb.bytes_used() <= 1024 * 1024, "used {}", rb.bytes_used());
    }

    #[test]
    fn empty_buffer_extract_is_empty() {
        let rb = RingBuffer::new(60, 1024);
        assert!(rb.extract_last(10).is_empty());
    }
}
