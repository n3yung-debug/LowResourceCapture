## LowResourceCapture — alpha (foundation + settings GUI)

**Still a pre-release. It does NOT save clips yet** — the capture/encode
pipeline is mid-build (Layer 2). This build is for trying the install flow and
the new **settings GUI**.

### New in this build
- **Settings GUI** — right-click the tray icon → **Settings…**. A window to:
  - Edit each clip preset's **name**, **duration** (type a number or pick from a
    15-second-interval dropdown), and **hotkey** (click "Set", then press the
    key/combo you want — e.g. Ctrl+Shift+F9).
  - Adjust output folder, codec (HEVC/H.264), bitrate, fps, and buffer caps.
  - On close, hotkeys re-register **live** — no restart needed.

### What works
- Per-user install (no admin), Start Menu shortcut, optional run-at-startup,
  clean uninstaller that **keeps your saved clips**.
- Tray menu: Settings… / Open clips folder / Reload / Quit.
- Global clip hotkeys (default F9=15s, F10=30s, F11=60s), editable in the GUI.
- Hotkey presses are logged to
  `%APPDATA%\LowResourceCapture\lowresourcecapture.log`.

### What does NOT work yet
- **No saved clips.** Screen capture + NV12 conversion are in (Layers 2a/2b),
  but NVENC encoding (2c), saving to .mp4 (Layer 4), and audio (Layer 3) are
  not done. Pressing a clip hotkey logs "buffer is empty" — expected.

### Coming next
- L2c: NVENC HEVC encode into the ring buffer (first real footage).
- L2d: debug "dump raw .hevc" to view captured footage.
- L4: save to .mp4. Then a feature-rich clip editor in the same GUI window.

### Note
Unsigned installer — SmartScreen may warn "unknown publisher." Choose
**More info → Run anyway**.
