## LowResourceCapture — alpha: play + split ▶️✂️

Building on the trim editor: clips now **play in their own in-app window**, and
you can **split a clip in two** at the playhead. Still powered by the bundled
ffmpeg (editor-only — capture never touches it), so HEVC clips just work with no
Windows codec extension.

### New: in-app player (Clip library → **Play**)
Hitting **Play** now opens the clip in a player window right inside the app —
same integrated feel as the trim window, and it plays HEVC too (ffmpeg does the
decode). Inside the player there's an **Open in default player (full quality)**
button if you'd rather watch it full-resolution in VLC / your default app.

*(The in-app player is a quick 720p transcode for convenience; the file on disk
is untouched and full quality.)*

### New: split at the playhead (Trim window → **✂ Split at playhead**)
Open **Trim**, move the white playhead to the cut point (scrub the preview or
click the filmstrip), then **Split at playhead**. You get two new frame-accurate
files next to the original — `…-part1.mp4` and `…-part2.mp4`. The original is
never modified.

### Also in the trim window (recap)
- Live preview with sound, draggable **In / Out** handles, **Play selection**,
  **Set In / Set Out to playhead**, and **Save trimmed copy**.

### Try it
1. Install over the top.
2. Tray → **Clip library…** → **Play** a clip (it opens in-app).
3. **Trim** a clip → scrub to a moment → **Split at playhead** → check for the
   two `…-part1/2.mp4` files.

*(The list is a snapshot from when you open the window — reopen it to see newly
saved trims/splits.)*

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
