## LowResourceCapture — alpha: real timeline editor 🎬

The clip editor now works like a proper timeline: **split into blocks, drag them
around, trim their edges, delete what you don't want, then save one clip.**
Plus edits get their own organized folder, and opening a clip is much lighter on
the CPU.

### New: block timeline (Clip library → **Edit**)
- **✂ Split** cuts the block under the playhead in two.
- **Drag a block** to move it earlier/later in the sequence.
- **Drag a block's edges** to trim just that block.
- **🗑 Delete** removes the selected block (or press Del).
- **▶ Play** previews the *edited* sequence — playback hops across your cuts, so
  you hear/see exactly what you'll get.
- **Save edited clip** renders the timeline into one new file.

Keyboard: **Space** play/pause · **S** split · **Del** delete · **Esc** close.
**Reset** puts the clip back to a single block.

### New: edits go to their own folder
Edited clips now save to `…\LowResourceCapture\Edits\<category>\`, mirroring the
same per-game folders your recordings use — e.g. an edit of a clip in
`…\LowResourceCapture\FCN\` lands in `…\LowResourceCapture\Edits\FCN\`. Originals
are never modified, and the library now lists clips from those nested folders too
(the badge shows `Edits\FCN`).

### Much lighter on the CPU when opening a clip
Opening a 60s clip used to spike the CPU hard. Two causes, both fixed:
- The filmstrip decoded **every** frame of a 1440p HEVC clip (~3600 frames). It
  now decodes **keyframes only** — same strip, a fraction of the work.
- The preview and the export encoded in software across all cores. They now use
  your **GPU encoder (NVENC)** when available, falling back to a **thread-capped**
  software encode. Timings are written to the log so it's measurable.

### Fixed
- **Clip-library actions were missing from the log.** The recorder's stat lines
  were overwriting anything the editor window wrote, so editor problems left no
  trace. All processes now append properly.
- **Reveal** is renamed **Open in Folder** — clearer about what it does.

### Try it
1. Install over the top.
2. Tray → **Clip library…** → **Edit** on a clip.
3. Split a few times, drag a block somewhere else, delete a boring bit, press
   **Play** to check it, then **Save edited clip** and look in
   `…\LowResourceCapture\Edits\`.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
