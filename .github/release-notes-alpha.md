## LowResourceCapture — alpha: clips now have sound 🔊

**L4b: the captured audio is now muxed into the saved `.mp4`.** Until now clips
were video-only even though audio was being captured; this build writes the
audio into the file.

### What's in the clip
- **Game / desktop audio** — everything you heard (loopback of your output).
- **Your microphone** — as a **separate track** (only when the mic toggle is ON;
  Settings → Audio).

Both are muxed with **no re-encoding** (they're already AAC), so saving stays
near-instant and cheap.

### A/V sync
Audio is timestamped with WASAPI's **QPC clock — the same clock as the video** —
and every stream is rebased to the video's start and written in timestamp order.
So audio lines up with the picture instead of drifting, even though the clip
starts a little before the exact N-second mark (video snaps back to a keyframe).

> **Note on playback:** per the design, game and mic are **separate audio
> tracks**. Most players (VLC, etc.) play the **first track (game audio)** by
> default and let you switch to the mic track; video editors see both. If you'd
> rather have them pre-mixed into one track for one-click playback, say so and
> I'll add a mix option.

### Try it
1. Install over the top, launch (it's capturing immediately).
2. Make some game/desktop sound (and talk, if the mic toggle is ON).
3. Press **F9**, open the clip in `<Videos>\LowResourceCapture\<source>\`.
4. You should now **hear** it. The log shows `saved '…' clip: …, N video frames
   + K audio track(s)`.

If audio is out of sync or missing, grab the log
(`C:\Program Files (x86)\LowResourceCapture\logs\`) and I'll tune it.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
