# CLAUDE.md — LowResourceCapture project conventions

Project-scoped validated conventions for this repo. Follows Nick's
truth-maintenance schema: status tags, confidence signaling, supersession.
Cross-project facts get promoted to the Book of Truth; everything here is
specific to this project.

## What this is

A low-overhead retroactive ("instant replay") game clip recorder for Windows.
Continuously encodes the active game into an in-RAM ring buffer via the GPU
hardware encoder; a hotkey saves the last N seconds to `.mp4`. Design goal:
minimal CPU/GPU/RAM.

## Target hardware (from Book of Truth v1.7, 2026-07-20) — VERIFIED

- CPU: Ryzen 7 9800X3D. GPU: **RTX 5070 Ti PRIME OC 16GB (Blackwell)**,
  driver 610.62. OS: Windows 11 (HAGS on). Native gaming res: **1440p**,
  360Hz QD-OLED primary.
- Implication: encoder target is **NVIDIA NVENC** (9th-gen Blackwell —
  supports H.264, HEVC, **and AV1**). Single-PC scope, no AMD/Intel fallback.

## Locked design decisions

| Decision | Choice | Status |
|---|---|---|
| Language/stack | Rust + windows-rs | agreed |
| Capture | Windows.Graphics.Capture (GPU frames, no CPU copy) | agreed |
| Capture target | **Primary monitor** (NOT per-game window) | confirmed on-device 2026-07-21 |
| Encode | NVENC HEVC (H.265) | agreed |
| Buffer | In-RAM, time- **and** RAM-capped | agreed |
| Save model | Retroactive ring buffer (last N seconds) | agreed |
| Hotkeys | One key per length (F9/F10/F11), configurable | agreed |
| Audio | Game + mic, separate tracks | agreed |
| Game detect | Drives capture **start/stop** only; target stays the monitor | agreed |
| Capture lifetime | Always-on while the app is open (auto-start on launch) | confirmed on-device 2026-07-21 |

- **Monitor-capture rationale (Nick, on-device):** capturing the whole primary
  monitor survives alt-tabbing between fullscreen apps mid-clip (one continuous
  stream, constant resolution). L5 game-detect will gate start/stop around
  games rather than switching the capture target to a game window.
- **Pipeline runtime-verified on Nick's PC (2026-07-21, v0.1.8+):** WGC capture
  → NVENC HEVC encode → in-RAM ring → IMFSinkWriter mp4 save produces a clip
  that plays in VLC. The yellow WGC border is disabled via
  `GraphicsCaptureSession::SetIsBorderRequired(false)`. This graduates L2–L4a
  from "compiles" to VERIFIED on-hardware.

## Verified facts

- **Crate versions (docs.rs primary source, 2026-07-21) — VERIFIED:**
  `windows` 0.62.2 · `tray-icon` 0.24.1 · `global-hotkey` 0.8.0.
  Re-verify against docs.rs before bumping; do NOT trust a search summary for
  release state (truth-maintenance v1.8, Recency Guard rule 5).
- **`windows::...::GetMessageW` hwnd param is `Option<HWND>`** in windows 0.62
  — pass `None` for "any window on this thread." VERIFIED (official
  windows-docs-rs binding).
- **NVENC HEVC vs H.264 (VERIFIED via research 2026-07-21):** H.264 encodes
  *slightly faster*; HEVC is marginally heavier BUT the real-time overhead is
  tiny (~2–3% GPU for 4K60 HEVC on RTX 40-series; the dedicated encoder block
  is separate silicon from graphics/CUDA). HEVC roughly halves bitrate for
  equal quality. Net: HEVC is the correct "light + best quality" pick.
  - SUPERSEDES an earlier in-chat claim that HEVC was "same-or-lighter load"
    than H.264 — that was an unverified inference and was wrong on direction.
- **NVENC on Windows access paths (VERIFIED via research 2026-07-21):**
  exposed via vendor Media Foundation Transforms (MFTs) *and* the direct
  NVIDIA Video Codec SDK. NVIDIA's H.264 MFT is well-established; the **HEVC
  MFT path is less certain / historically less robust** than H.264.
  - DESIGN FORK for layer 2 (INFERENCE, PENDING VALIDATION): if the MF
    hardware HEVC MFT proves flaky on this GPU, fall back to the NVIDIA Video
    Codec SDK directly for encode. Decide empirically when layer 2 is built.

## Workspace layout (since L6a, 2026-08-07)

Cargo workspace, three crates, **two installers**:

| crate | ships as | notes |
|---|---|---|
| `crates/recorder` | `LowResourceCapture-Setup-*.exe` | the tray recorder; `opt-level = "z"` |
| `crates/analyzer` | `ClipAnalyzer-Setup-*.exe` | offline VOD analyzer; `opt-level = 3` |
| `crates/shared` | (library) | config, logging, ffmpeg wrappers, clip editor |

- **Why two installers:** the recorder is a tiny always-resident process tuned
  for minimal footprint while gaming; the analyzer runs offline and is free to
  use every core. Separate `AppId`s and install dirs — either installs,
  upgrades, and uninstalls without the other.
- Recorder modules keep their `crate::config::…` paths via a `pub use
  shared::{clips, config, ffmpeg, logging};` re-export at the crate root.
- The block-timeline editor lives in `shared` **on purpose** so the analyzer
  opens the same editor rather than forking a second copy.

## Build

`cargo test --workspace` (pure-Rust logic: ring buffer, game-detection,
analyzer event/profile logic) · `cargo run -p lowresourcecapture` (debug,
console) · `cargo build --release` (both exes). Needs Rust MSVC toolchain + VS
Build Tools (C++ workload).

## Layered build plan

L1 foundation (done) → L2 WGC capture + NVENC HEVC encode → L3 WASAPI audio →
L4 mp4 mux/save → L5 game auto-detect wiring, toast, live hotkey reload,
run-at-startup. Each layer must `cargo build` on Nick's PC before the next.

**L6 — VOD analyzer** (kills/deaths → clip timeline). L6a workspace split +
second installer (done, this commit) → L6b detectors → L6c review UI with
confirm/reject marking → L6d train/export → L6e ONNX inference.

## Mistfall Hunter detection findings (VOD analysis 2026-08-07)

Source: Nick's own Twitch VOD, 9:22, 1080p60, 14.7 Mbps, **stereo but
effectively mono** (side channel 25–30 dB below mid). Burnt-in stream overlay:
webcam bottom-left, follower alerts top-centre. Timestamps annotated by Nick.

- **Death card — VERIFIED (n=1), the strong signal.** "YOU DIED" in red centred
  text, gold **Spectate** / **Return to Camp** buttons at y≈0.90, screen
  darkened, red vignette. Fixed position, large, unambiguous. Template match,
  no ML.
- **Player kill — VERIFIED (n=2).** Three correlates: a red enemy nameplate +
  health bar (arbitrary screen position, 2D UI sprite); a gold particle burst;
  and an **"F Loot" prompt at a fixed position** (x≈0.50, y≈0.58).
- **The game has NO kill feed** (removed by patch; the game launched
  2026-07-29 and is patching weekly). Third-party kills therefore have no HUD
  artifact at all and are out of scope — detecting them would need scene
  understanding.
- **Kill audio sting — NOT DETECTABLE in this source. Two tests, both
  negative.** Spectrogram cross-correlation between the two player kills scored
  0.44 in tight 2s windows — *below* monster-kill pairs (0.455) and level with
  unrelated controls (0.425). An earlier 8s-window result of 0.55 was an
  artifact of matching **silence** (combat noise stopping ~2.5s after a kill),
  not a shared sound. Mid/side separation is unavailable (near-mono source).
  Audio is **off the critical path**; revisit only with a local recording
  (game/mic on separate tracks) or a pristine reference from the game's assets.
- **Gold burst as a player-vs-monster discriminator — INCONCLUSIVE, instrument
  suspect.** A saturated-gold pixel-fraction sweep showed monster kills rising
  *more* than player kills (0.55/0.73 vs 0.01/0.23), which contradicts the
  frames — the burst is visibly present at both player kills. Most likely the
  thresholds (`sat > 0.45`) reject pale-cream particles, and 4× downscaling
  dilutes them further. **Re-measure before trusting either direction.**
- **Profiles record the game build they were calibrated against** — a patched
  HUD invalidates a profile exactly as a driver change invalidates a benchmark
  ceiling. `GameProfile::is_stale()` enforces the flag.

## Confidence discipline for this repo

Nothing touching `windows-rs` is VERIFIED until it compiles/runs on Nick's
Windows machine. Logic with `cargo test` coverage (ring buffer, detection
rules) IS verified cross-platform. Keep the two categories distinct in any
status report.
