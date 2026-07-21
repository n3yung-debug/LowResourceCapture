## LowResourceCapture — alpha: UX cleanup 🧹

Polish pass from your feedback.

### Clips filed by where they spent the most time
The clip's folder is no longer just "whatever's focused when you hit the hotkey."
The app now samples the foreground app once a second, and a saved clip goes to
the app it spent the **most time in** over that window (ties break toward the app
you were in at the **start** of the clip). So a clip that's mostly gameplay lands
in the game's folder even if you tabbed to the desktop right before saving.

### One duration field
The clip-length dropdown and the number box are now a **single field**: type any
number of seconds, or click it to pick from the 15-second-step suggestions.

### Simpler tray menu
Since capture is always-on, the tray menu is trimmed to what you actually use:
**Settings…**, **Open clips folder**, **Quit**. Removed the debug **Start/Stop
capture** items, **Reload settings** (settings already reload automatically when
you save in the GUI), and **Dump raw buffer** (an early debug tool).

### Try it
1. Install over the top.
2. Spend time in a game, tab to the desktop briefly, hit **F9** — the clip should
   still land in the **game's** folder.
3. Open **Settings…** → the duration field is now one box with a dropdown built
   into it.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
