## LowResourceCapture — alpha: crash-on-capture fixed 🛠️

**Fixes the crash when you hit "Start capture."** The previous build brought the
HEVC encoder up correctly, then died the instant frames started flowing.

**Cause:** the one D3D11 GPU device is shared across three threads (WGC capture,
the BGRA→NV12 converter, and the NVENC encoder via Media Foundation's DXGI
device manager). Direct3D 11's immediate context isn't thread-safe by default,
so those threads raced and the process hit an access violation — a hard crash
that never reached the log. This build enables **D3D11 multithread protection**
(`ID3D11Multithread::SetMultithreadProtected`), which Media Foundation requires
whenever a device is shared this way.

### New: clips filed by source, in your Videos library
Clips now save to **`<Videos>\LowResourceCapture\<source>\`**, where `<Videos>`
is your real Windows "Videos" **known folder** (so a relocated library like
`D:\Videos` is picked up automatically — no hardcoded drive), and `<source>` is
the app that was in the foreground when you hit the hotkey:
- a game → its own folder (e.g. `…\LowResourceCapture\eldenring\`),
- any browser → `…\LowResourceCapture\Browser\`,
- desktop / unknown → `…\LowResourceCapture\Desktop\`.

The base folder and each subfolder are **created on demand** — nothing to set
up. (You can still override the base location in tray → **Settings…**.)

> Still **video only** (game + mic audio muxing is the next build). Capture is
> started from the tray (auto game-detect is L5).

### Try it
1. Install (SmartScreen → **More info → Run anyway** — unsigned).
2. Tray → **Start capture (debug)** (captures your primary monitor).
3. Wait ~5 seconds (or play a game / move windows around).
4. Press a clip hotkey: **F9** = 15s, **F10** = 30s, **F11** = 60s
   (all editable in tray → **Settings…**).
5. Open your clips folder (tray → **Open clips folder**, e.g.
   `D:\Videos\LowResourceCapture`), go into the **`<source>`** subfolder, and
   **play `clip_<date-time>_<len>s.mp4`**. If it plays back your last N
   seconds — the whole GPU pipeline works. 🎉

The log (`%APPDATA%\LowResourceCapture\lowresourcecapture.log`) should now show
`capture->encode: … frames` and `encode: … frames (… keyframes)` ticking every
second while capturing, then `saved … clip -> <path>` on a hotkey.

### If it still crashes
Grab `%APPDATA%\LowResourceCapture\lowresourcecapture.log` — the last lines
(especially anything with `error`, `failed`, or a `frame … pipeline error`)
tell me exactly where it stopped.

### Note
Unsigned installer — SmartScreen warning is expected.
