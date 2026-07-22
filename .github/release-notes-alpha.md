## LowResourceCapture — alpha: trim clips (with live preview) ✂️▶️

The clip editor gets its first real editing feature: **frame-accurate trim with
in-app preview**, right inside the app. No Windows HEVC codec extension needed —
a small **ffmpeg** is now bundled and used only by the editor (capture never
touches it).

### New: Trim (Clip library → **Trim**)
Open the clip library, hit **Trim** on any clip, and you get:
- A **live preview player** — watch the clip right in the window (with sound),
  so you can find the exact moment. (HEVC clips play here too — ffmpeg handles
  the decode, no Windows codec extension required.)
- A **filmstrip timeline** with draggable **In / Out** handles, plus a playhead
  that tracks the video.
- **Play selection** to preview just the part you're keeping, and **Set In /
  Set Out to playhead** to mark trim points at the current video position.
- Click the filmstrip to seek the preview there.
- Live **In / Length / Out** times as you adjust.
- **Save trimmed copy** writes a new `…-trim.mp4` next to the original — your
  original clip is never modified.

*(The preview is a lightweight 480p transcode just for playback; the saved trim
is re-encoded from your original at full resolution.)*

The trimmed copy is re-encoded to H.264 + AAC, so it's frame-accurate and plays
and uploads everywhere (Discord, editors, browsers) without a codec extension.

### About the bundled ffmpeg
It's a single `ffmpeg.exe` installed next to the app and invoked only when you
trim (or build a timeline). It runs hidden — no console flash. It adds nothing to
the recorder's footprint while you're gaming; capture is unchanged.

### Try it
1. Install over the top.
2. Tray icon → **Clip library…** → **Trim** on a clip.
3. Watch the preview, use **Play selection** / **Set In** / **Set Out** (or drag
   the handles), then **Save trimmed copy** and play the new `…-trim.mp4`.

### Still to come (editor)
- Thumbnails on each library row, audio-track pick, GIF / share-size export.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
