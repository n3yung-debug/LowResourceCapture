## LowResourceCapture — alpha: fixes the "on and off" / stuck-at-15s bug 🩹

Your log nailed it. Thank you — it made the bug obvious.

### What was happening
Clips were muxed **on the engine's command thread**, and each save got
progressively slower (your log: ~50 ms → 8 s → 53 s → minutes). So the whole
command loop stalled: most hotkey presses never got their clip written
(25 presses → 7 saves in your log), and the longer 30/60/90 s clips were queued
furthest back and effectively **never finished** — hence "works on and off" and
"never past 15 seconds." Clips that *did* eventually write landed in the wrong
folder (explorer, Spotify) because the folder was chosen minutes later when they
finally completed.

### The fix
- **Clips are now written on a dedicated background thread.** A slow save can
  never block the command loop or capture again — hotkeys stay responsive and
  every press is handled immediately.
- **The folder is decided at the moment you press the hotkey** (by majority
  time), so clips file correctly even if a save takes a bit.
- **The queue is bounded** — mashing the hotkey won't pile up memory.
- **Saving settings no longer wipes your replay buffer** (it was rebuilding the
  ring on every settings save).
- **Per-phase timing is logged** for each save (`write_clip phases: setup … ms,
  write … ms, finalize … ms`) so if any save is still slow, the log will show
  exactly which step — and I'll optimize that specifically.

### Try it
1. Install over the top.
2. Let it capture a while, then press **F9 (15s)**, **F10 (30s)**, **F11 (60s)**
   — each should now save promptly, at the right length, in the right folder.
3. If anything's still slow, send the log — the new `write_clip phases:` line
   tells me where the time goes.

*(Next build: the toast notification, run-at-startup toggle, live tray tooltip,
and the mixed-audio option you asked for.)*

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
