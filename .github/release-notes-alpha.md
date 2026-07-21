## LowResourceCapture — alpha: found it, both bugs fixed 🎯

The diagnostic build did its job — the log pinpointed **two** separate bugs, and
this build fixes both. This should be the first build that actually captures and
saves a clip.

### Bug #1 — the crash (audio) ✅ fixed
The crash was in the **microphone** encoder. The log's last line was an
`UNHANDLED EXCEPTION (access violation)` on the mic thread, right after it fed
the first audio buffer to the AAC encoder. Cause: the AAC encoder is a
*synchronous* encoder that doesn't allocate its own output buffers — the app has
to provide them, and it was handing over a null buffer. Now it allocates a
proper output buffer (sized from the encoder), so draining AAC no longer
crashes.

### Bug #2 — no frames were ever captured ✅ fixed
Even in video-only mode, the log showed **`encode: 0 frames`** the entire time —
not one frame made it through. The frame-rate limiter started from a sentinel
value and used wrapping math that underflowed on **every** frame, so it dropped
100% of them (which is why the buffer was always empty and clips wouldn't save).
One-line fix. Frames should now flow.

---

## Please test (this is the real end-to-end run) 🎮
1. Install over the top (SmartScreen → *More info → Run anyway*, then UAC → *Yes*).
2. Tray → **Start capture (debug)** (video **+** audio — the one that used to crash).
3. Wait ~5–10 seconds (move some windows / play something so there's motion).
4. Press **F9** (15s).
5. Open **`<Videos>\LowResourceCapture\<source>\`** and play `clip_*.mp4`.

The log (`C:\Program Files (x86)\LowResourceCapture\logs\lowresourcecapture.log`)
should now show `FrameArrived: first callback fired`, `frame1: …` markers, then
`encode: N frames` with **N climbing**, and finally `saved 'Short' clip -> …` on
F9.

> Still **video only** in the saved `.mp4` (audio muxing is the next step, L4b) —
> but audio is now being captured + encoded without crashing. If this build
> saves a playable clip, the whole capture→encode→ring→save pipeline is proven.

### The `lowresourcecapture.exe.WebView2` folder
That's harmless — it's the WebView2 runtime's data folder, created when the
settings window opens. I can relocate it out of the install folder in a later
build if you'd like it tidier; it doesn't affect anything.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
