# LowResourceCapture

A low-overhead, retroactive **game clip recorder** for Windows — think Medal /
ShadowPlay "Instant Replay," built to sip CPU/GPU/RAM.

It sits in the system tray, continuously encodes your game into a rolling
in-RAM buffer using the GPU's dedicated hardware encoder, and when you press a
clip hotkey it instantly saves the **last N seconds** to an `.mp4`. Nothing
heavy runs until a game is detected.

> **Status: in active, layered construction.** The foundation (tray app,
> config, hotkeys, ring buffer, game-detection logic) is written; the Windows
> GPU capture/encode/audio/mux pipeline is being filled in layer by layer. See
> [Build status](#build-status-by-layer) for exactly what works today. It was
> authored on Linux and has **not yet been compiled on Windows** — treat the
> first `cargo build` as the initial validation pass.

---

## Why it's light

| Concern | Approach |
|---|---|
| **GPU** | Encoding runs on the GPU's **dedicated NVENC block**, separate silicon from the graphics/CUDA cores your game uses — near-zero impact on FPS. |
| **CPU** | Capture uses **Windows.Graphics.Capture**: frames stay on the GPU and go straight into the encoder. The CPU never copies pixels. |
| **RAM** | The buffer holds only **encoded** frames (compressed), time- **and** RAM-capped. ~5 MB/s at 1440p60/40 Mbps, so 60 s ≈ 300 MB, hard-limited. |
| **Idle** | When no game is in the foreground, capture/encode is **fully stopped** — just a tiny tray process watching for a game. |
| **Saving** | Clips are **remuxed, not re-encoded** — saving is near-instant and costs almost no CPU. |

## Defaults

- **Codec:** HEVC (H.265) via NVENC — smaller files, best quality/bitrate.
  (Correction vs. an earlier claim: HEVC is *marginally heavier* to encode
  than H.264, not lighter — but the delta is negligible on the dedicated
  NVENC block, single-digit % GPU, while HEVC roughly halves bitrate for the
  same quality. So it stays the "light + best quality" pick. AV1 is an even
  better option this Blackwell GPU supports — a future codec choice.)
- **Buffer:** in-RAM, capped at 120 s / 1 GB (configurable).
- **Hotkeys:** `F9` = 15 s, `F10` = 30 s, `F11` = 60 s (all configurable).
- **Audio:** game + mic as **separate tracks**.
- **Games:** auto-detected by fullscreen foreground window, with allow/block lists.

All of this lives in `%APPDATA%\LowResourceCapture\config.toml`, created on
first run. Edit it from the tray (**Edit settings**), then **Reload settings**.

---

## Building (on your Windows PC)

Prereqs — you almost certainly have these already; if not:

1. **Rust (MSVC toolchain):** install from <https://rustup.rs>. Accept defaults
   (this pulls the MSVC target).
2. **Visual Studio Build Tools** with the *Desktop development with C++*
   workload (provides the MSVC linker windows-rs needs). Rustup will warn if
   it's missing.
3. An **NVIDIA GPU** for NVENC (that's the current encoder target).

Then, from the repo root in a normal PowerShell/CMD:

```powershell
# Debug build (keeps a console window for live logs) — use this while we iterate
cargo run

# Run the pure-Rust unit tests (ring buffer + game-detection logic)
cargo test

# Release build (windowed, no console, size-optimized) — the shippable exe
cargo build --release
# -> target\release\lowresourcecapture.exe
```

Logs are written to
`%APPDATA%\LowResourceCapture\data\lowresourcecapture.log` (and to the console
in debug builds).

### To start on login (later)

Drop a shortcut to `lowresourcecapture.exe` in
`shell:startup`, or we'll add a proper "Run at startup" toggle in a later layer.

---

## Build status by layer

This is being built in independently-testable layers so each `cargo build` on
your machine validates real progress.

- [x] **Layer 1 — Foundation (this commit).** Tray app, TOML config, global
  hotkeys, in-RAM ring buffer, game-detection *logic*, engine thread +
  command channel. Pipeline modules are documented interface stubs.
  **Testable now:** `cargo test` passes the ring-buffer/detection logic;
  `cargo run` should give you a tray icon whose hotkeys log "save last Ns"
  (no video yet).
- [ ] **Layer 2 — Capture + encode.** WGC capture → NVENC HEVC via Media
  Foundation, feeding the ring buffer. First real footage in the buffer.
- [ ] **Layer 3 — Audio.** WASAPI loopback (game) + mic capture → AAC, per
  the audio mode.
- [ ] **Layer 4 — Save.** Mux extracted frames → `.mp4` (video + audio
  tracks), proper local-time filenames.
- [ ] **Layer 5 — Polish.** Game auto-detect wiring, on-screen "clip saved"
  toast, live hotkey re-registration on reload, run-at-startup, optional
  small settings window.

### What's `VERIFIED` vs not

- **VERIFIED (via `cargo test`, platform-independent logic):** ring-buffer
  eviction/keyframe-snap/RAM-cap, game-detection allow/block/fullscreen rules.
- **UNVERIFIED until you build on Windows:** everything touching `windows-rs`
  (tray/message-loop, and all of layers 2–5). Written to be correct but not
  yet compiler- or hardware-checked. The `GetMessageW(None, ...)` call is
  confirmed correct against the windows 0.62 binding; the likelier first-build
  friction is minor API drift in `tray-icon` 0.24 / `global-hotkey` 0.8 (the
  menu-builder or event-receiver calls), which a compile error will pinpoint.

### Verified crate versions (docs.rs, 2026-07-21)

Pinned to current releases, confirmed against the registry (not a search
summary): `windows` 0.62.2, `tray-icon` 0.24.1, `global-hotkey` 0.8.0.

---

## Project layout

```
src/
  main.rs         Tray app, Win32 message loop, event routing
  config.rs       TOML config: hotkeys, lengths, encoder/buffer/audio settings
  hotkeys.rs      Global hotkey registration + routing
  ringbuffer.rs   In-RAM encoded-frame ring buffer (unit-tested)
  engine.rs       Capture pipeline coordinator (own thread + command channel)
  capture.rs      Windows.Graphics.Capture session          (stub → layer 2)
  encoder.rs      NVENC HEVC/H.264 via Media Foundation      (stub → layer 2/3)
  audio.rs        WASAPI loopback + mic capture              (stub → layer 3)
  muxer.rs        Mux encoded frames → .mp4                  (stub → layer 4)
  game_detect.rs  Foreground-window game detection (logic unit-tested; Win32 → layer 5)
```

## License

MIT.
