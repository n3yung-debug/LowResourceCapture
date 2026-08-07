## alpha: the analyzer window actually shows up 🔍

Three first-run bugs in a row meant ClipAnalyzer never got as far as being
usable. This release fixes the last of them.

### Fixed: nothing appeared after you picked a VOD
The analyzer scanned the **entire** recording before creating any window. On a
9-minute 1080p60 file that's a long silent wait — the process was visible in
Task Manager with nothing ever coming to the foreground, which is
indistinguishable from a hang.

The window now opens **immediately** and scans behind itself:

- **Progress shows as a percentage** while it works.
- **The video is scrubbable and K/D work during the scan** — playback never
  depended on it finishing.
- **Detections appear in the list as they land.**
- **A scan that fails says so in the window** instead of dying quietly.

### Also fixed, earlier in this run
- **Launching from the shortcut did nothing.** No arguments now opens a file
  picker rather than printing usage to a console that doesn't exist.
- **A clean scan left an empty window.** The recording is now served with range
  support, so you can scrub and hand-mark whether or not anything was detected
  — which matters, because you extract far more often than you die.
- **Errors are shown, not printed.** Plus a real log at
  `<install>\logs\clipanalyzer.log`.

## What ClipAnalyzer does

Point it at a video — a Twitch VOD, an OBS or NVIDIA recording:

1. It scans for **death screens**.
2. **Y** confirms a detection, **N** rejects it, **←/→** walks the queue,
   **Space** plays.
3. **K** and **D** mark a player kill or death at the playhead.
4. **Export confirmed → clip** assembles what you approved into one
   `<video>_highlights.mp4`.

Verdicts save to `<video>.labels.json` beside the source and **survive
re-scanning**, so a re-tuned detector never costs you a review.

## Kills are marked by hand, and that's deliberate

Mistfall Hunter has no kill feed, and measurement ruled out the shortcuts:
the kill audio sting isn't recoverable from a Twitch transcode, the gold
death-burst fires just as hard on monsters as on players (the largest burst
measured was a monster kill), and the enemy nameplate can't be found by colour
— a matchmaking screen with no enemy present has *more* red than a real fight.

So a kill detector needs training data, and hand-marking is how it gets made.
Every **K** is one example.

## Death detection, measured

Swept across a full 9:22 VOD at 2 fps (1123 frames): the death card scores
**0.443–0.500**; the loudest false positive in the other nine minutes scores
**0.178**. Threshold 0.30 — about 2.5× margin either way, and the five
highest-scoring frames in the whole recording are the death.

Calibrated on a 1080p Twitch transcode. The margin should carry to native
1440p, but that's expectation, not measurement.

## Known limits

- **HEVC may not play.** WebView2 doesn't always decode it. H.264 sources
  (Twitch VODs, OBS defaults) are fine; recordings from LowResourceCapture
  itself are HEVC and may not play here yet. The window says so rather than
  showing a blank player.
- **Scanning is slower than it needs to be** — it decodes every frame to sample
  two per second. Keyframe-only sampling would be far faster and should still
  catch a card that's up for 2.5s.
- Unsigned installers, so SmartScreen and UAC prompts are expected.
