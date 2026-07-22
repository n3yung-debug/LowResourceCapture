## LowResourceCapture — alpha: clip library 🎬

The recorder's done, so this starts the clip editor — **Phase 1: a clip library**
to manage your saved clips.

### New: Clip library (tray → **Clip library…**)
A window listing every clip you've saved, newest first, with its source folder,
length, size, and date. For each clip:
- **Play** — opens it in your default player (VLC etc.).
- **Reveal** — shows the file highlighted in Explorer.
- **Rename** — rename the file.
- **Delete** — remove it from disk (with a confirm).

Like Settings, it runs as a separate window only when open, so it costs nothing
during capture.

### Coming next (editor phases 2–3)
- **In-app preview + trim** (scrub, set in/out, save a cut). Note: previewing
  HEVC clips inside the app needs the Windows "HEVC Video Extensions" codec —
  we'll sort that (or offer an H.264 record option) when we build trim.
- Thumbnails, audio-track pick, GIF / share-size export.

### Try it
1. Install over the top.
2. Tray icon → **Clip library…** → your clips are listed. Try Play / Reveal /
   Rename / Delete.

*(The list is a snapshot from when you open the window — reopen it to refresh
after recording more.)*

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
