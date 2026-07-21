## LowResourceCapture — alpha: app icon + clean uninstall 🎨

Polish pass now that capture is proven and always-on.

### New app icon
A proper icon — a replay-loop ring around a red record dot on a dark badge,
matching the app's theme and reading as "instant-replay recorder." It now shows
up everywhere: the **exe in Explorer**, the **taskbar / Alt-Tab**, the **system
tray**, and the **installer**, all consistent.

### Clean uninstall
Uninstalling now also removes the `lowresourcecapture.exe.WebView2` folder (the
WebView2 runtime's data folder, created when the settings window opens). It was
being left behind before — now nothing is left in the install folder after
uninstall. *(Your saved clips in your Videos folder are still never touched.)*

### Confirmed: primary-monitor capture
Per your call, capture stays locked to the **primary monitor** (not per-game
window). It's the robust choice — a clip that spans an alt-tab between fullscreen
apps stays one continuous video. Future game-detection will only start/stop
capture around games; it won't change what's captured.

> No functional changes to capture/encode/save — this is icon + uninstall polish
> on top of v0.1.10 (always-on capture, no border).

### Try it
1. Install over the top — note the new icon on the installer and the app.
2. Check the tray icon, and the exe icon in Explorer / taskbar.
3. (Optional) Uninstall and confirm the install folder is fully gone, including
   the old `…WebView2` folder.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
