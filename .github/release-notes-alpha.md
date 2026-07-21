## LowResourceCapture — alpha: crash-hunting build 🔬

The previous build still crashed the moment you hit **Start capture**, and the
old log couldn't show why (its last lines were being lost when the process was
killed). This build is built to **catch the crash red-handed** — and it moves
the logs + config into the install folder as requested.

### What's new for debugging
- **The log now flushes every line to disk instantly**, so it's complete right
  up to the crash — the last line you see is the last thing that actually ran.
- **A crash handler** logs the exact failure (a Rust error vs. a GPU/driver
  "access violation," with the faulting thread + address) before the process
  dies.
- **Step-by-step markers** through the first video frame, the encoder, and the
  first audio buffer, so the log names the precise operation that crashed.
- **New tray item: "Start capture — video only (debug)"** — lets us split the
  problem in half (video pipeline vs. audio pipeline) in one click.

### Logs + config moved into the install folder
- Log: **`C:\Program Files (x86)\LowResourceCapture\logs\lowresourcecapture.log`**
- Config: **`C:\Program Files (x86)\LowResourceCapture\config.toml`**

The installer grants your account write access to that folder so the app (which
runs non-elevated) can write there. **Nothing lives in `%APPDATA%` anymore** —
those were the only two files that ever did.

---

## Please run this quick sequence and send me the log 🙏
1. Install over the top (SmartScreen → *More info → Run anyway*, then UAC → *Yes*).
2. Tray → **Start capture — video only (debug)**. Wait ~5 seconds.
   - If it **crashes**, the problem is in the video pipeline.
   - If it **survives**, press **F9** to save a clip, then do step 3.
3. Tray → **Start capture (debug)** (video **+** audio). Wait ~5 seconds.
   - If this one crashes but video-only didn't, the problem is in the audio path.
4. Either way, send me
   **`C:\Program Files (x86)\LowResourceCapture\logs\lowresourcecapture.log`**.

The new last lines (a `frame1: …`, `pump: …`, or `audio-…: …` marker, and
hopefully an `UNHANDLED EXCEPTION …` line) will tell me exactly where it dies —
no more guessing.

> Still **video only** in saved clips (audio muxing is L4b). Capture is started
> from the tray; auto game-detect is L5. Clips still save to
> `<Videos>\LowResourceCapture\<source>\`.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
