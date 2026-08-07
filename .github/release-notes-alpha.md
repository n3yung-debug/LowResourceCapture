## alpha: ClipAnalyzer actually launches now 🔍

### Fixed: the analyzer did nothing when you double-clicked it
`clipanalyzer.exe` is a windowed build, so it has no console — and its shortcut
launched it with no arguments, which hit a "print usage and exit" path. The
usage text went to a stderr that doesn't exist, so the app just vanished.

- **No arguments now opens a file picker** instead of exiting.
- **Errors show a message box** as well as logging. Before this, any failure was
  indistinguishable from "it doesn't start".
- **The analyzer writes a log at all** now — `<install>\logs\clipanalyzer.log`.
  It previously never initialized logging, so there was nothing to inspect.

### Known gap
If a scan finds **zero** detections, the review window opens with no video
loaded, so there's nothing to scrub and you can't hand-mark anything. That
matters for a VOD with kills but no deaths. Being fixed separately.

---

## Everything below shipped in 0.1.23 — a second app 🔍

**This release ships two installers.** The recorder is unchanged in behaviour;
the new one is an offline tool that scans a recording for deaths, lets you
judge what it found, and exports the good bits as one clip.

| installer | what it is |
|---|---|
| `LowResourceCapture-Setup-*.exe` | the tray recorder, as before |
| `ClipAnalyzer-Setup-*.exe` | **new** — offline VOD analyzer |

They install, upgrade and uninstall independently. You can have either or both.

### Why two apps
The recorder is a tiny always-resident process tuned for minimal footprint
while you game. Scene analysis wants the opposite — every core, offline, no
regard for size. Those don't belong in one binary, so they aren't in one.
**Nothing about the recorder's footprint changes.**

### ClipAnalyzer: what it does today
Point it at a video file — a Twitch VOD, an OBS or NVIDIA recording:

1. It scans for **death screens** and lists what it found.
2. A **review window** shows each detection with a preview of the seconds
   around it. **Y** confirms, **N** rejects, **←/→** walks the queue,
   **Space** plays.
3. **K** and **D** mark a player kill or death at the playhead — for anything
   it missed.
4. **Export confirmed → clip** assembles everything you approved into one
   `<video>_highlights.mp4`.

Your verdicts save to `<video>.labels.json` beside the source and **survive
re-scanning**, so re-running an improved detector never costs you a review.

### What it does NOT do yet
**Kills are not detected — you mark them by hand.** That's deliberate rather
than unfinished: Mistfall Hunter has no kill feed, and measurement showed the
usual shortcuts don't work. The kill audio sting isn't recoverable from a
Twitch transcode, the gold death-burst fires just as hard on monsters as on
players, and the enemy nameplate can't be found by colour — a matchmaking
screen with no enemy present has *more* red than an actual fight.

So the kill detector needs training data, and hand-marking in the review window
is how that data gets made. Each **K** you press is one example.

### Death detection, measured
Swept across a full 9:22 VOD at 2 fps (1123 frames): the death card scores
**0.443–0.500**, and the loudest false positive in the other nine minutes
scores **0.178**. The threshold sits at 0.30 — roughly 2.5× margin either way,
and the five highest-scoring frames in the whole recording are the death.

Calibrated against a 1080p Twitch transcode. The margin is wide enough that it
should carry to native 1440p footage, but that's expectation, not measurement.

### Try it
1. Install `ClipAnalyzer-Setup-*.exe`.
2. Run it on a VOD with a death in it.
3. Confirm what it found, mark a kill or two by hand, export.

### Note
Unsigned installers — SmartScreen + UAC prompts are expected. The analyzer's
review window has never run outside CI, so expect rough edges and say what's
wrong rather than working around it.
