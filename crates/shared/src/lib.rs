//! Code shared by the recorder and the offline VOD analyzer.
//!
//! The two ship as separate installers on purpose: the recorder is a tiny
//! always-resident process tuned for minimal footprint while gaming, and the
//! analyzer is an offline tool that is free to use every core. What they
//! genuinely share lives here — config and paths, the crash-handling logger,
//! the bundled-ffmpeg wrappers, and the block-timeline clip editor.
//!
//! Keeping the editor here is the point of the split: the analyzer opens the
//! *same* editor on a VOD rather than growing a second copy that drifts.

pub mod clips;
pub mod config;
pub mod ffmpeg;
pub mod logging;
