## LowResourceCapture — alpha: it SAVES CLIPS now! 🎬

**Pressing a clip hotkey now saves a real `.mp4` of the last N seconds.** The
core "instant replay" recorder works end to end: continuous GPU capture →
NVENC encode → in-RAM ring buffer → hit a hotkey → muxed to `.mp4` (no
re-encode, near-instant).

> **Video only for now** — game + mic audio muxing lands in the very next
> build. Capture is still started from the tray (auto game-detect is L5).

### Try it
1. Install (SmartScreen → **More info → Run anyway** — unsigned).
2. Tray → **Start capture (debug)** (captures your primary monitor).
3. Wait ~5 seconds (or play a game / move windows around).
4. Press a clip hotkey: **F9** = 15s, **F10** = 30s, **F11** = 60s
   (all editable in tray → **Settings…**).
5. Open your clips folder (default `D:\Videos\LowResourceCapture`) and **play
   `clip_<date-time>_<len>s.mp4`**. If it plays back your last N seconds — the
   whole pipeline works on your GPU. 🎉

The log (`%APPDATA%\LowResourceCapture\lowresourcecapture.log`) shows
`saved ... clip -> <path>`, whether the encoder used **HEVC** or **H.264**,
and live encode stats.

### Also in this build
- Settings GUI (hotkeys, durations, codec, bitrate, buffer).
- Per-user install; uninstaller keeps your saved clips.
- Audio is being captured + AAC-encoded already — it just isn't muxed into the
  `.mp4` yet (next build).

### Coming next
- **Next build:** game + mic audio in the clips.
- Then: automatic game detection (no debug menu), a "clip saved" toast,
  run-at-startup, and a feature-rich clip editor.

### Note
Unsigned installer — SmartScreen warning is expected.
