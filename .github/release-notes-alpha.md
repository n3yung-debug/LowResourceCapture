## LowResourceCapture — alpha: cut, split & reassemble ▶️✂️🧩

The trim window is now a little editor. Play clips in-app, make cuts, then
**piece the parts together into one clip** — all in the window, powered by the
bundled ffmpeg (editor-only; capture never touches it, and HEVC just works with
no Windows codec extension).

### New: split & assemble (Trim window)
- **✂ Split at playhead** adds a cut where the white playhead is, dividing that
  piece in two. Split as many times as you like.
- Each piece shows up as a chip below the timeline. For each one you can:
  - **◀ / ▶** — move it earlier/later in the order,
  - **click the time / ✓** — include or exclude it.
- **Save assembled clip** stitches the included pieces, in your chosen order,
  into one new frame-accurate `…-edit.mp4` next to the original. The original is
  never modified.

So you can cut out a dull middle and rejoin the rest, or reorder moments — then
save a single clip.

### Also in this build
- **In-app player** — **Play** opens the clip in a player window inside the app
  (plays HEVC via ffmpeg), with an **Open in default player (full quality)**
  button for external playback.
- Live preview with sound, draggable **In / Out** handles, **Play selection**,
  **Set In / Set Out to playhead**, and **Save trimmed copy** for a simple
  single-range trim.

### Try it
1. Install over the top.
2. Tray → **Clip library…** → **Play** a clip (opens in-app).
3. **Trim** a clip → scrub → **✂ Split at playhead** a few times → exclude/reorder
   pieces → **Save assembled clip**, then play the new `…-edit.mp4`.

*(The clip list is a snapshot from when you open the window — reopen it to see
newly saved edits.)*

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
