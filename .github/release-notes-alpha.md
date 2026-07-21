## LowResourceCapture — alpha: stall fix + toast, startup, live tray, mixed audio 🩹✨

Bundles the critical stall fix with all four requested extras.

### The big fix: clips no longer stall
Your log showed clip writes running on the engine's command thread and getting
progressively slower (~50 ms → 8 s → 53 s → minutes), which jammed everything —
most hotkey presses never produced a clip, and 30/60/90 s clips effectively never
finished ("works on and off" / "never past 15 seconds"). Now:
- **Clips are muxed on a dedicated background thread**, so a slow save never
  blocks hotkeys or capture. Every press is handled immediately.
- **Folder is chosen at press time**, so delayed writes can't misfile clips.
- **Bounded write queue** (spamming the hotkey can't balloon RAM).
- **Saving settings no longer wipes your replay buffer.**
- **Per-phase timing** is logged per save (`write_clip phases: setup/write/finalize`)
  so any remaining slowness is pinpointed in the log.

### New extras
- **🔔 "Clip saved" toast** — a desktop notification with the length and folder
  each time you clip. (Your main "it worked" signal now that there are no
  buttons.)
- **🚀 Run-at-Windows-startup toggle** — Settings → General, flip it anytime
  (green ON / red OFF), no reinstall needed.
- **📊 Live tray tooltip** — hover the tray icon to see
  `recording · last Ns buffered · NN MB`.
- **🎚️ Mixed audio option** — Settings → Audio → *Mix game + mic into one track*.
  On = one combined track (players play game **and** mic together on normal
  playback); off = separate tracks. A real-time mixer sums the desktop-loopback
  and mic streams (the mic is resampled to match), so this is a first cut — if
  the balance or sync is off, tell me and I'll tune it.

### Try it
1. Install over the top.
2. Press **F9 / F10 / F11** — each saves promptly, at the right length, in the
   right folder, with a toast.
3. Hover the tray icon (live status); open **Settings → General** for the
   startup toggle.
4. If any save still drags, send the log — the `write_clip phases:` line shows
   exactly where.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
