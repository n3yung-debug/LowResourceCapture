## LowResourceCapture — alpha: mic noise gate + volume 🎙️

The mixer's confirmed working (game + Discord + mic all in one track), so this
build adds mic controls and makes mixed the default.

### Mixed audio is now the default
Fresh installs record game + mic on one combined track out of the box — no
setup. (Your existing setting is kept.)

### Noise gate (kills the breathing)
A **noise gate** now silences your mic between words, so breathing / idle hiss
doesn't sit under your game audio. On by default. In **Settings → Audio**:
- **Noise gate** toggle (ON/OFF).
- **Gate threshold** slider — closer to 0 cuts more aggressively. Default −45 dB;
  raise it (e.g. −40, −35) if breathing still sneaks through, lower it if the
  start of your words ever gets clipped.

### Mic volume slider
**Settings → Audio → Mic volume** (0–200%) to balance your voice against the
game/Discord in the mix. Applies live on save.

### Try it
1. Install over the top.
2. Settings → Audio — the noise gate is already on. Record a clip and talk with
   pauses; the breathing between words should be gone.
3. Nudge **Mic volume** and **Gate threshold** to taste (both apply the moment
   you save), and tell me if the defaults need adjusting.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
