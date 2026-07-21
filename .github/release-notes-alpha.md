## LowResourceCapture — alpha (capture + NVENC encode works!)

**The core recorder now works end to end in RAM.** Screen capture → GPU NV12
conversion → **NVENC hardware encode** → encoded footage living in the ring
buffer. Saving to `.mp4` (Layer 4) and audio (Layer 3) are still to come, so
this is a **validation build**, not a finished clipper yet.

### Try the capture pipeline (debug)
1. Install and let it sit in the tray.
2. Right-click the tray icon → **Start capture (debug)** (captures your
   primary monitor).
3. Wait ~5 seconds.
4. Right-click → **Dump raw buffer (debug)**.
5. Open your clips folder (default `D:\Videos\LowResourceCapture`) and play the
   `debug_*.hevc` (or `.h264`) file in **VLC**. If you see your screen — the
   whole capture→encode pipeline works on your GPU. 🎉
6. The log at `%APPDATA%\LowResourceCapture\lowresourcecapture.log` shows
   **`encoder ready: HEVC`** vs **`H.264`** (which your GPU gave), plus
   per-second `encode: N frames` stats and buffer size.

### Also in this build
- **Settings GUI** (tray → Settings…): edit clip presets (name, duration,
  click-to-capture hotkey) + codec/bitrate/fps/buffer; applies live on close.
- Per-user install, clean uninstaller that keeps your saved clips.

### What does NOT work yet
- **No `.mp4` clips on hotkey yet** — that's Layer 4 (mux the buffered frames
  to mp4). No audio yet (Layer 3). Capture currently targets the whole primary
  monitor (per-game window + fps throttle is Layer 2e).

### Note
Unsigned installer — SmartScreen may warn "unknown publisher." Choose
**More info → Run anyway**.
