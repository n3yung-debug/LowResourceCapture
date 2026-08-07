# LowResourceCapture

Two Windows apps, one repo, shipped as **two separate installers**:

| | ships as | what it does |
|---|---|---|
| **LowResourceCapture** | `LowResourceCapture-Setup-*.exe` | Sits in the tray during a game, keeps a rolling in-RAM buffer of the last N seconds via GPU hardware encode, saves a clip on a hotkey. |
| **ClipAnalyzer** | `ClipAnalyzer-Setup-*.exe` | Offline tool: scans a recording for deaths, lets you scrub it and mark kills by hand, exports the confirmed moments as one clip. |

They install, upgrade, and uninstall independently — take either one or both.
Why two apps instead of one: the recorder is a tiny always-resident process
tuned for minimal footprint while you're gaming; the analyzer runs offline and
is free to use every core. Those don't belong in the same binary.

---

## LowResourceCapture — the recorder

Think Medal / ShadowPlay "Instant Replay," built to sip CPU/GPU/RAM.

It continuously encodes your primary monitor into a rolling in-RAM buffer
using the GPU's dedicated NVENC block, and a hotkey instantly saves the last
N seconds to an `.mp4`. Nothing heavy runs until a game is detected — capture
starts and stops automatically around fullscreen games, and the whole
capture/encode/mux pipeline is fully idle the rest of the time.

**Status: the core pipeline is runtime-verified on hardware** — capture,
NVENC HEVC encode, in-RAM ring buffer, and mp4 save produce clips that play
back cleanly. Game auto-detect, the "clip saved" toast, live hotkey reload,
and run-at-startup are implemented. See [Build status](#build-status) for
what that claim rests on.

### Why it's light

| Concern | Approach |
|---|---|
| **GPU** | Encoding runs on the GPU's **dedicated NVENC block** — separate silicon from the shader cores your game uses — near-zero FPS impact. |
| **CPU** | Capture uses **Windows.Graphics.Capture**: frames stay on the GPU and go straight into the encoder. The CPU never copies pixels. |
| **RAM** | The buffer holds only **encoded** (compressed) frames, time- and RAM-capped. Default: 120 s / 1 GB. |
| **Idle** | No game in the foreground → capture/encode is **fully stopped**, just a tray process watching for one. |
| **Saving** | Clips are **remuxed, not re-encoded** — near-instant, almost no CPU. |

### Clip library and editor

Saved clips get their own in-app viewer (tray → **Clip library…**) with a
**block-timeline editor**: split at the playhead, drag blocks to reorder,
trim their edges, delete what you don't want, preview the edit before
committing, then save. Edits are written to `Edits\<category>\` alongside your
recordings rather than overwriting the original. Preview and export use
**NVENC** when available, with a thread-capped software fallback so an edit
never competes with a game running at the same time.

### Defaults

- **Codec:** HEVC (H.265) via NVENC. The dedicated encoder block makes the
  extra encode cost over H.264 negligible (~2–3% GPU), while HEVC roughly
  halves bitrate for the same quality — the "light + best quality" pick.
- **Buffer:** in-RAM, capped at 120 s / 1 GB (configurable).
- **Hotkeys:** `F9` = 15 s, `F10` = 30 s, `F11` = 60 s (all configurable, live
  reload from tray → **Reload settings**).
- **Audio:** game + mic as **separate tracks**.
- **Capture target:** the primary monitor, not a per-game window — this
  survives alt-tabbing between fullscreen apps mid-clip, since it's one
  continuous stream at a constant resolution. Game detection only gates
  *when* capture runs, not *what* it captures.
- **Games:** auto-detected by fullscreen foreground window, with allow/block
  lists you can edit from the tray.

All of this lives in `config.toml` next to the installed exe (see
[Where things live](#where-things-live)) — edit it via tray → **Edit
settings**, then **Reload settings**.

---

## ClipAnalyzer — the offline VOD analyzer

Point it at a recording — a Twitch VOD, an OBS or NVIDIA capture, or a
LowResourceCapture recording — and it:

1. **Scans for death screens** while its window is already open and usable
   (scanning runs in the background; you can scrub immediately, you don't
   wait on it).
2. Opens a **review window**: every detection listed with a status, the video
   seeks natively against the source (no transcode, no waiting).
3. You judge what it found and mark what it missed:

   | key | action |
   |---|---|
   | `Y` | confirm the selected detection |
   | `N` | reject it |
   | `K` | mark a **player kill** at the playhead |
   | `D` | mark a **death** at the playhead |
   | `M` | mark a **monster kill** at the playhead (the negative case) |
   | `←` / `→` | previous / next detection |
   | `Space` | play / pause |

4. **Export confirmed → clip** assembles everything you approved into one
   `<video>_highlights.mp4`, reusing the same block-timeline editor the
   recorder ships.

Your verdicts and marks save to `<video>.labels.json` beside the source and
**survive re-scanning** — running a re-tuned detector never costs you a
review, and a hand mark is never overwritten by a detector that fails to find
it again.

### What's actually detected vs. hand-marked, and why

**Death screens are detected.** Measured across a full 9:22 test VOD at 2 fps
(1123 frames): the death card scores 0.443–0.500, the loudest false positive
in the other nine minutes scores 0.178 — about 2.5× margin either way, no
machine learning involved, just a fixed-position colour match.

**Kills are marked by hand — there is no kill detector yet, and that's a
measured conclusion, not a missing feature.** For the first game this targets
(Mistfall Hunter), every cheap shortcut was tested and ruled out: there's no
kill feed to read, the kill audio sting doesn't survive a Twitch transcode,
the death particle burst fires identically for monster and player kills, and
the enemy nameplate — the one PvP-specific signal — is too small (~11 px at
1080p) and the game's palette too red-saturated for colour matching to find
it. Full findings, with numbers, are in `CLAUDE.md`.

A real kill detector needs training data — boxes drawn around nameplates, and
timestamped positive/negative examples — which is exactly what marking in the
review window produces. See `crates/analyzer/samples/README.md` for where to
put recordings you want to build that dataset from.

### Detection profiles

Game-specific detection is **data, not code** — `crates/analyzer/profiles/`
holds one TOML file per game (regions of interest, thresholds, what game
build it was calibrated against). Adding a second game means dropping in a
profile, not cutting a release. A profile records the game build it was
calibrated against so a patch that moves the HUD gets flagged as stale
instead of silently returning nothing.

---

## Building

Prereqs (you likely have these already):

1. **Rust (MSVC toolchain):** <https://rustup.rs>, accept defaults.
2. **Visual Studio Build Tools**, *Desktop development with C++* workload
   (the MSVC linker `windows-rs` needs).
3. An **NVIDIA GPU** for NVENC — the current encoder target for both the
   recorder's capture path and the analyzer's/editor's preview & export.

From the repo root:

```powershell
# Build both apps (debug)
cargo build --workspace

# Run just the pure-Rust unit tests — ring buffer, game-detection rules,
# analyzer event/profile/label logic. All of this is genuinely
# platform-independent and passes without Windows.
cargo test --workspace

# Run the recorder directly (debug, keeps a console for live logs)
cargo run -p lowresourcecapture

# Release build — both exes, ready for their installers
cargo build --release
# -> target\release\lowresourcecapture.exe
# -> target\release\clipanalyzer.exe
```

Installers are built with [Inno Setup](https://jrsoftware.org/isinfo.php)
from `installer\lowresourcecapture.iss` and `installer\clipanalyzer.iss`
(CI does this automatically on release — see `.github/workflows/release.yml`).

### Where things live

Both apps install to their own directory under `Program Files (x86)` and keep
config + logs there (not `%APPDATA%`):

- `...\LowResourceCapture\config.toml`, `...\LowResourceCapture\logs\`
- `...\ClipAnalyzer\logs\clipanalyzer.log`

Saved clips default to `<your Videos folder>\LowResourceCapture\`, with edits
under a `\Edits\<category>\` subfolder — both configurable from the recorder's
settings.

---

## Project layout

A Cargo workspace, three crates, organized so it's obvious at a glance which
files belong to which app:

```
crates/
  recorder/            → LowResourceCapture-Setup-*.exe
    src/
      main.rs           tray app, Win32 message loop, event routing
      config.rs          (re-exported from shared — see below)
      hotkeys.rs         global hotkey registration + routing
      ringbuffer.rs      in-RAM encoded-frame ring buffer (unit-tested)
      engine.rs          capture pipeline coordinator (own thread + commands)
      capture.rs         Windows.Graphics.Capture session
      encoder.rs         NVENC HEVC/H.264 via Media Foundation
      audio.rs           WASAPI loopback (game) + mic capture
      convert.rs         pixel format conversion for the encoder
      muxer.rs           mux encoded frames → .mp4
      game_detect.rs     foreground-window game detection (logic unit-tested)
      gui.rs             settings window (WebView2, separate `--gui` process)
      toast.rs            "clip saved" Windows toast notifications
      startup.rs          run-at-startup toggle

  analyzer/            → ClipAnalyzer-Setup-*.exe
    src/
      main.rs           entry point: file picker, --scan-only, error reporting
      detect.rs          death-card detector (measured thresholds, tested)
      event.rs            detection event model: clustering, event→clip segments
      frames.rs            streams a cropped region of a video as raw RGB frames
      labels.rs            the label store — verdicts, marks, and drawn boxes,
                            preserved across re-scans
      profile.rs            per-game detection profile format (normalized ROIs)
      review.rs / .html    the review window: scan progress, scrub, mark, export
      winui.rs              native file picker / message boxes (no console app)
    profiles/            per-game detection profiles (TOML, not code)
    samples/              local recordings + their .labels.json — see its README

  shared/               (library — no installer of its own)
    src/
      config.rs          TOML config + install-dir/log-path resolution
      logging.rs          crash-safe file logger, shared by both apps
      ffmpeg.rs            wrappers over the bundled ffmpeg.exe
      clips.rs / .html    the block-timeline clip editor — lives here so both
                          apps open the *same* editor rather than forking one
```

`crates/recorder` re-exports `shared::{clips, config, ffmpeg, logging}` at its
crate root, so its modules still use `crate::config::…` paths unchanged.

## Build status

- **Genuinely platform-independent and `cargo test`-verified today:** the ring
  buffer, game-detection rules, and every piece of analyzer logic that doesn't
  touch Windows or ffmpeg — event clustering, clip-segment math, the label
  store's merge/preserve rules, the detection-profile format.
- **VERIFIED on Nick's hardware:** the recorder's full capture → NVENC HEVC
  encode → ring buffer → mp4 save pipeline, and the death-card detector
  (measured over a full test recording, see above).
- **Compiles and passes CI, not yet run by a human on real footage:** the
  analyzer's review window UI (background scan, scrub-and-mark, export) as of
  its most recent changes — CI proves it builds on Windows; it doesn't prove
  the UI is pleasant or correct in practice.

This repo keeps those two claims — "compiles" vs. "verified on hardware" —
explicitly distinct rather than blurring them. See `CLAUDE.md` for the full,
dated findings this status is built on.

## License

MIT.
