//! Process-wide capture stats for the live tray tooltip.
//!
//! A couple of atomics the encoder pump updates each second and the tray reads
//! on a timer — no plumbing through the engine/capture signatures.

use std::sync::atomic::{AtomicU64, Ordering};

static BUFFER_BYTES: AtomicU64 = AtomicU64::new(0);
static BUFFER_MS: AtomicU64 = AtomicU64::new(0);

/// Called by the encoder pump: current buffered bytes and time span (ms).
pub fn set_buffer(bytes: u64, span_ms: u64) {
    BUFFER_BYTES.store(bytes, Ordering::Relaxed);
    BUFFER_MS.store(span_ms, Ordering::Relaxed);
}

/// A short human-readable status line for the tray tooltip.
pub fn tooltip() -> String {
    let mb = BUFFER_BYTES.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
    let secs = BUFFER_MS.load(Ordering::Relaxed) / 1000;
    if secs == 0 {
        "LowResourceCapture — starting…".to_string()
    } else {
        format!("LowResourceCapture — recording · last {secs}s buffered · {mb:.0} MB")
    }
}
