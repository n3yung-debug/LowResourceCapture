# Sample recordings

Put local recordings you want to analyze or label here — this folder is the
convention, not a requirement, but it keeps training data in one findable
place instead of scattered across `Downloads`.

**Videos themselves are git-ignored** (`*.mp4`, `*.mkv`, `*.mov`, `*.avi`,
`*.ts`, `*.webm` — see `.gitignore`). Point ClipAnalyzer at a file here, review
it, and the tool writes `<name>.labels.json` right next to it automatically —
**that sidecar file IS tracked in git**. It's small plain text (timestamps,
confirm/reject verdicts, drawn boxes) and it's the actual dataset: the videos
are just what produced it and can be regenerated or re-supplied, but a label
you made by hand can't be.

## Layout

```
crates/analyzer/samples/
  mistfall_hunter/
    2026-08-07_first_match.mp4        (git-ignored)
    2026-08-07_first_match.labels.json (tracked)
    2026-08-14_local_capture.mp4       (git-ignored)
    2026-08-14_local_capture.labels.json (tracked)
```

One subfolder per game keeps profiles and their calibration samples together
as more games are added.

## What to drop in for the local recording

A LowResourceCapture recording with a **few kills and a death**, ideally at
native 1440p with game audio and mic on separate tracks (the recorder already
does this) — that's the one sample so far calibrated only against a Twitch
transcode, so a native capture is what would confirm or correct the death
threshold and let the audio-sting question be tested honestly instead of on a
recompressed, near-mono source. See `CLAUDE.md` for what was measured on the
Twitch VOD and why a local recording matters for each of those findings.
