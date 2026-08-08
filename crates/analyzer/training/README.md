# Training the kill classifier

Offline, on your own machine. Nothing here ships in the installer — the
analyzer only loads the resulting `.onnx`.

## The loop

```
1. Record          →  a match, via LowResourceCapture or OBS
2. Review          →  clipanalyzer <video> --build 2026.08.06
                      Y confirm · K player kill · M monster kill · D death
3. Export          →  clipanalyzer <video> --export-dataset D:\clipanalyzer-dataset --build 2026.08.06
4. Train           →  python train.py --data D:\clipanalyzer-dataset
5. Repeat from 1 — the dataset grows, the model improves
```

Steps 1–3 repeat per recording; step 4 whenever you've added enough to be
worth retraining.

## Why this approach

The PvP-specific nameplate icon is about **11 pixels** at 1080p — hard
small-object detection, and it would need you to draw a box around every
instance. The dying *character model* is a much bigger target, and since
enemy cosmetics don't render for you, the visual vocabulary is closed: a
given class in a given armor tier looks the same every time, and each map's
monsters are a fixed roster.

So this is an ordinary image classifier over frames around each death, and it
trains on the marks you already make. **No boxes required.**

Fight length doesn't matter, even though it varies a lot between solos and
trios — the model only ever sees a second either side of the death instant,
never the fight.

## Setup

```powershell
pip install torch torchvision pillow --index-url https://download.pytorch.org/whl/cu124
```

CUDA build for the 5070 Ti. It'll fall back to CPU if Torch can't see the GPU,
just slower.

## Surviving patches

The game patches weekly and has already removed a kill feed once. A model
trained on one build's armor sets can quietly stop working — and a silently
degraded classifier is worse than none, because it still answers confidently.

So:

- **Every exported frame records its `game_build`.** Pass `--build` when
  reviewing or exporting.
- **The model card records every build it saw.** ClipAnalyzer compares that to
  the current build and flags a stale model rather than trusting it.
- **Nothing is thrown away on a patch.** Retraining mixes old and new footage.
  Classes a patch didn't touch keep all their examples; ones it did get
  corrected as new footage accumulates. A staleness warning means "record and
  retrain", not "start over".

After a patch: record a match or two, review, export with the new `--build`,
retrain. The card will then list both builds and stop warning.

## Reading the output honestly

**The split is by source recording, not by frame.** Nine frames from one death
are near-duplicates; a random split would put them on both sides and report a
meaningless accuracy. That also means a held-out score needs **at least two
recordings**, and really wants several — with one, the script says so and the
number should be ignored.

**Watch per-class recall, not just accuracy.** With far more `none` frames
than kills, a model can score well by never predicting a kill at all. The
script weights classes by inverse frequency to push against that and prints
recall per class. A class under ~50 frames is flagged and shouldn't be trusted
whatever the headline number says.

## Classes

| class | from | meaning |
|---|---|---|
| `player` | **K** | a player died on screen |
| `monster` | **M** | a monster died on screen — the negative that matters most |
| `own_death` | **D** | you died |
| `none` | auto | sampled ≥8s from any mark |

`monster` is the important one. The gold death-burst fires identically for
monsters and players (measured, n=5 — the largest burst in that sample was a
monster kill), so a model that never sees monster deaths has no way to learn
the only distinction that matters. **Press M as diligently as K.**
