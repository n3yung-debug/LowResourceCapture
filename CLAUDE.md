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

## Build

`cargo test` (pure-Rust logic: ring buffer, game-detection) · `cargo run`
(debug, console) · `cargo build --release` (windowed, size-optimized exe).
Needs Rust MSVC toolchain + VS Build Tools (C++ workload).

## Layered build plan

L1 foundation (done) → L2 WGC capture + NVENC HEVC encode → L3 WASAPI audio →
L4 mp4 mux/save → L5 game auto-detect wiring, toast, live hotkey reload,
run-at-startup. Each layer must `cargo build` on Nick's PC before the next.

## Confidence discipline for this repo

Nothing touching `windows-rs` is VERIFIED until it compiles/runs on Nick's
Windows machine. Logic with `cargo test` coverage (ring buffer, detection
rules) IS verified cross-platform. Keep the two categories distinct in any
status report.
