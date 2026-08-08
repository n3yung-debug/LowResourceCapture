## alpha: fix — M was recording monster kills as deaths 🐛

**Install this before marking anything else.** In 0.1.27 the **M** key wrote
its mark to disk as a *death*, not a monster kill. The sidebar wasn't
mislabeling it — the data itself was wrong.

That matters more than a cosmetic bug, because monster marks are the negative
class that teaches the model to tell a monster death from a player one. Marks
made with M in 0.1.27 are training data pointing the wrong way.

### Recovering marks you already made
They can't be fixed automatically — the intent isn't recoverable from the
file. But they're easy to spot by hand. In `<video>.labels.json`, look for:

```json
"kind": "death", "origin": "manual"
```

Real deaths are almost always found by the detector (`"origin": "detected"`),
so a *hand-marked* death is very likely a mangled monster kill. Change those
to `"monsterkill"`, or delete them and re-mark. Leave `"origin": "detected"`
entries alone.

### The cause
Adding the monster-kill class left the review window's string match with a
catch-all that swallowed the new name:

```rust
"playerkill" => Kind::PlayerKill,
_            => Kind::Death,      // "monsterkill" landed here
```

Now there's one shared mapping with no catch-all, so an unrecognized mark is
refused and reported rather than silently becoming something else — plus tests
that every class round-trips through its name, and that those names match what
actually gets written to the label file.

---

## Also in this release: train a kill detector from inside the app 🧠

No terminal. The analyzer finds Python, installs what's missing, runs the
trainer, and streams output into its own window.

**Review window → 🧠 Training…**

1. **Dataset** — folder, game build, live per-class counts, and
   **Add this recording to the dataset**.
2. **Python** — detected version and whether dependencies actually import;
   **Install dependencies** runs pip if not.
3. **Train** — run it, watch the log, cancel mid-run.

```
Record  →  Review (Y / K / M / D)  →  Add to dataset  →  Train  →  repeat
```

### Press M as often as K
The gold death-burst is identical for monsters and players — the largest one
measured was a monster kill. A model that never sees a monster death has no
way to learn the only distinction that matters.

### Honest about the numbers
- **Two recordings minimum** before an accuracy figure means anything. Frames
  from one death are near-duplicates, so the split holds out whole recordings.
- **Per-class recall is shown, not just accuracy** — with far more "nothing
  happened" frames than kills, a model scores well by never predicting a kill.
  Thin classes are flagged in amber.
- **Models record the build they trained on** and flag themselves stale after
  a patch. Nothing is discarded when the game updates.

### Still to come
The analyzer doesn't yet *use* a trained model to find kills — you can build
one and read its score, but detection still means marking by hand.

### Note
Unsigned installers, so SmartScreen and UAC prompts are expected.
