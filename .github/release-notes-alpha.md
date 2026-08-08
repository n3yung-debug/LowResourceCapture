## alpha: train a kill detector from inside the app 🧠

No terminal. The analyzer finds Python, installs what's missing, runs the
trainer, and streams the output into its own window.

### New: Training panel
In the review window, press **🧠 Training…**

1. **Dataset** — pick a folder and the game build, see live per-class counts,
   and **Add this recording to the dataset**.
2. **Python** — shows the detected version and whether the dependencies are
   actually importable. If they aren't, **Install dependencies** runs pip and
   streams the download.
3. **Train** — run it, watch the log, cancel mid-run.

### The loop
```
Record  →  Review (Y / K / M / D)  →  Add to dataset  →  Train  →  repeat
```
Every recording you review makes the dataset bigger. Retrain whenever.

### Why a trained model at all
Every cheap shortcut for detecting kills in Mistfall Hunter was tried and
measured, and all of them failed:

- **No kill feed** — removed by a patch.
- **No kill audio cue** — confirmed by ear after four different automated
  searches turned up nothing but a monster's ambient bell.
- **The gold death-burst doesn't discriminate** — it fires just as hard for
  monsters as players; the largest burst measured was a monster kill.
- **The enemy nameplate can't be found by colour** — a matchmaking screen with
  no enemy present has *more* red than a real fight.

A trained model is what's left. It works here because enemy cosmetics don't
render: a given class in a given armor tier looks identical every time, and
each map's monsters are a fixed roster — a small, closed problem rather than
"recognize an arbitrary player".

### Press M as often as K
Monster kills aren't filler, they're half the signal. The death-burst is
identical for both, so a model that never sees a monster death has no way to
learn the only distinction that matters.

### Honest about the numbers
- **Two recordings minimum** before an accuracy figure means anything. Frames
  from one death are near-duplicates, so the split holds out whole recordings
  — with one, there's nothing to hold out and the panel says so.
- **Per-class recall is shown, not just accuracy.** With far more "nothing
  happened" frames than kills, a model scores well by never predicting a kill.
  Classes are weighted against that, and thin ones are flagged in amber.
- **Models record the game build they trained on** and flag themselves stale
  after a patch. Nothing is discarded when the game updates — retraining mixes
  old and new footage.

### Still to come
The analyzer doesn't yet *use* a trained model to find kills — you can build
one and read its score, but detection still means marking by hand. Wiring
inference in is next.

### Note
Unsigned installers, so SmartScreen and UAC prompts are expected. The training
panel has never run outside CI — if something breaks, the log pane should show
it rather than failing silently.
