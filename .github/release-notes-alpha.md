## LowResourceCapture — alpha: microphone toggle 🎙️

### What audio is captured
The recorder captures **both**:
- **Everything you hear** — desktop/game audio via loopback of your default
  output device (your headphones).
- **Your microphone** — on a **separate track**.

*(Neither is muxed into the saved `.mp4` yet — that's the next step, L4b — but
both are captured and encoded.)*

### New: microphone ON/OFF toggle
Tray → **Settings…** → **Audio** now has a **Record microphone** switch:
- **Green ON** = your mic is captured (separate track).
- **Red OFF** = mic off; only desktop/game audio is captured.

This is the app's standard on/off control — a real toggle, not a checkbox — and
any future on/off option will use the same switch.

> Includes everything from v0.1.8: the crash fix (mic AAC encoder) and the
> frame-throttle fix, so capture actually records now.

### Try it
1. Install over the top.
2. Tray → **Settings…** → **Audio** → flip **Record microphone** OFF, **Save**.
3. Tray → **Start capture (debug)** — the log should now show **no** mic worker
   starting (only the desktop-audio worker), confirming the toggle took effect.
4. Flip it back **ON** in Settings to record your mic again.

Settings changes apply the next time capture starts.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
