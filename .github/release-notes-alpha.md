## LowResourceCapture — alpha: trim clips ✂️

The clip editor gets its first real editing feature: **frame-accurate trim**,
right inside the app. No Windows HEVC codec extension needed — a small **ffmpeg**
is now bundled and used only by the editor (capture never touches it).

### New: Trim (Clip library → **Trim**)
Open the clip library, hit **Trim** on any clip, and you get a timeline:
- A **filmstrip** of the clip so you can see where you are.
- Drag the **In / Out** handles (or click the strip) to set the section to keep.
- Live **In / Length / Out** times as you drag.
- **Save trimmed copy** writes a new `…-trim.mp4` next to the original — your
  original clip is never modified.

The trimmed copy is re-encoded to H.264 + AAC, so it's frame-accurate and plays
and uploads everywhere (Discord, editors, browsers) without a codec extension.

### About the bundled ffmpeg
It's a single `ffmpeg.exe` installed next to the app and invoked only when you
trim (or build a timeline). It runs hidden — no console flash. It adds nothing to
the recorder's footprint while you're gaming; capture is unchanged.

### Try it
1. Install over the top.
2. Tray icon → **Clip library…** → **Trim** on a clip.
3. Drag the handles, **Save trimmed copy**, then play the new `…-trim.mp4`.

### Still to come (editor)
- In-app **preview / playback** while trimming.
- Thumbnails on each library row, audio-track pick, GIF / share-size export.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
