## LowResourceCapture — alpha: always-on capture, no border 🎬

The pipeline is proven (v0.1.8 saved a real clip on-device), so this build makes
it behave like a real instant-replay recorder.

### Always capturing while open
No more manually starting capture. The moment the app launches it begins
capturing your primary monitor into the rolling buffer, so the **last N seconds
are always ready** — just press a clip hotkey (F9/F10/F11) any time. Quitting the
app stops it. (The debug Start/Stop menu items still work if you want manual
control.)

### Yellow recording border removed
The yellow "you're being captured" border (a Windows 11 privacy indicator) is now
turned off, so it won't sit around your screen while the app runs. If a future
Windows policy ever blocks turning it off, capture still works — the border just
comes back; you'll see `could not disable capture border` in the log if so.

> Still **video only** in the saved `.mp4` (audio muxing is next, L4b). The mic
> ON/OFF toggle from v0.1.9 is in Settings → Audio. Auto game-detection (capture
> only when a game is focused) is the step after audio muxing.

### Try it
1. Install over the top.
2. Launch it — it's now capturing immediately (log shows `ready; capturing`).
3. Play/move things for a few seconds, press **F9**, and the clip is in
   `<Videos>\LowResourceCapture\<source>\`. No Start-capture step needed.
4. Confirm the yellow border is gone.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
