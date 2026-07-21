## LowResourceCapture — alpha (foundation build)

**This is a pre-release foundation build. It does NOT record clips yet.**

What this build proves out: the app installs, sits in the system tray, loads
its config, registers the clip hotkeys, and the install/release pipeline
works end to end. It is intentionally shipped before the capture engine so the
distribution plumbing is solved early.

### What works
- Installs per-user (no admin), Start Menu shortcut, optional run-at-startup,
  clean uninstaller.
- Red-dot tray icon with a menu: Open clips folder / Edit settings / Reload / Quit.
- Global clip hotkeys (default **F9**=15s, **F10**=30s, **F11**=60s), all
  configurable in `%APPDATA%\LowResourceCapture\config.toml`.
- Pressing a hotkey is logged to
  `%APPDATA%\LowResourceCapture\data\lowresourcecapture.log`.

### What does NOT work yet
- No screen capture, no encoding, no audio, **no saved clips**. Pressing a
  hotkey logs "buffer is empty" — expected at this stage.

### Coming next
- Layer 2: Windows.Graphics.Capture + NVENC HEVC encode (first real footage).
- Layer 3: audio. Layer 4: save to .mp4. Layer 5: game auto-detect + polish.

### Note
Unsigned installer — Windows SmartScreen may warn "unknown publisher." That's
expected for a personal test build; choose **More info → Run anyway**.
