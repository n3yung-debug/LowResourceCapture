# CLAUDE.md — LowResourceCapture project conventions

Project-scoped validated conventions for this repo. Follows Nick's
truth-maintenance schema: status tags, confidence signaling, supersession.
Cross-project facts get promoted to the Book of Truth; everything here is
specific to this project.

## What this is

A low-overhead retroactive ("instant replay") game clip recorder for Windows.
Continuously encodes the active game into an in-RAM ring buffer via the GPU
hardware encoder; a hotkey saves the last N seconds to `.mp4`. Design goal:
minimal CPU/GPU/RAM.

## Target hardware (from Book of Truth v1.7, 2026-07-20) — VERIFIED

- CPU: Ryzen 7 9800X3D. GPU: **RTX 5070 Ti PRIME OC 16GB (Blackwell)**,
  driver 610.62. OS: Windows 11 (HAGS on). Native gaming res: **1440p**,
  360Hz QD-OLED primary.
- Implication: encoder target is **NVIDIA NVENC** (9th-gen Blackwell —
  supports H.264, HEVC, **and AV1**). Single-PC scope, no AMD/Intel fallback.

## Locked design decisions

| Decision | Choice | Status |
|---|---|---|
| Language/stack | Rust + windows-rs | agreed |
| Capture | Windows.Graphics.Capture (GPU frames, no CPU copy) | agreed |
| Capture target | **Primary monitor** (NOT per-game window) | confirmed on-device 2026-07-21 |
| Encode | NVENC HEVC (H.265) | agreed |
| Buffer | In-RAM, time- **and** RAM-capped | agreed |
| Save model | Retroactive ring buffer (last N seconds) | agreed |
| Hotkeys | One key per length (F9/F10/F11), configurable | agreed |
| Audio | Game + mic, separate tracks | agreed |
| Game detect | Drives capture **start/stop** only; target stays the monitor | agreed |
| Capture lifetime | Always-on while the app is open (auto-start on launch) | confirmed on-device 2026-07-21 |

- **Monitor-capture rationale (Nick, on-device):** capturing the whole primary
  monitor survives alt-tabbing between fullscreen apps mid-clip (one continuous
  stream, constant resolution). L5 game-detect will gate start/stop around
  games rather than switching the capture target to a game window.
- **Pipeline runtime-verified on Nick's PC (2026-07-21, v0.1.8+):** WGC capture
  → NVENC HEVC encode → in-RAM ring → IMFSinkWriter mp4 save produces a clip
  that plays in VLC. The yellow WGC border is disabled via
  `GraphicsCaptureSession::SetIsBorderRequired(false)`. This graduates L2–L4a
  from "compiles" to VERIFIED on-hardware.

## Verified facts

- **Crate versions (docs.rs primary source, 2026-07-21) — VERIFIED:**
  `windows` 0.62.2 · `tray-icon` 0.24.1 · `global-hotkey` 0.8.0.
  Re-verify against docs.rs before bumping; do NOT trust a search summary for
  release state (truth-maintenance v1.8, Recency Guard rule 5).
- **`windows::...::GetMessageW` hwnd param is `Option<HWND>`** in windows 0.62
  — pass `None` for "any window on this thread." VERIFIED (official
  windows-docs-rs binding).
- **NVENC HEVC vs H.264 (VERIFIED via research 2026-07-21):** H.264 encodes
  *slightly faster*; HEVC is marginally heavier BUT the real-time overhead is
  tiny (~2–3% GPU for 4K60 HEVC on RTX 40-series; the dedicated encoder block
  is separate silicon from graphics/CUDA). HEVC roughly halves bitrate for
  equal quality. Net: HEVC is the correct "light + best quality" pick.
  - SUPERSEDES an earlier in-chat claim that HEVC was "same-or-lighter load"
    than H.264 — that was an unverified inference and was wrong on direction.
- **NVENC on Windows access paths (VERIFIED via research 2026-07-21):**
  exposed via vendor Media Foundation Transforms (MFTs) *and* the direct
  NVIDIA Video Codec SDK. NVIDIA's H.264 MFT is well-established; the **HEVC
  MFT path is less certain / historically less robust** than H.264.
  - DESIGN FORK for layer 2 (INFERENCE, PENDING VALIDATION): if the MF
    hardware HEVC MFT proves flaky on this GPU, fall back to the NVIDIA Video
    Codec SDK directly for encode. Decide empirically when layer 2 is built.

## Workspace layout (since L6a, 2026-08-07)

Cargo workspace, three crates, **two installers**:

| crate | ships as | notes |
|---|---|---|
| `crates/recorder` | `LowResourceCapture-Setup-*.exe` | the tray recorder; `opt-level = "z"` |
| `crates/analyzer` | `ClipAnalyzer-Setup-*.exe` | offline VOD analyzer; `opt-level = 3` |
| `crates/shared` | (library) | config, logging, ffmpeg wrappers, clip editor |

- **Why two installers:** the recorder is a tiny always-resident process tuned
  for minimal footprint while gaming; the analyzer runs offline and is free to
  use every core. Separate `AppId`s and install dirs — either installs,
  upgrades, and uninstalls without the other.
- Recorder modules keep their `crate::config::…` paths via a `pub use
  shared::{clips, config, ffmpeg, logging};` re-export at the crate root.
- The block-timeline editor lives in `shared` **on purpose** so the analyzer
  opens the same editor rather than forking a second copy.

## Build

`cargo test --workspace` (pure-Rust logic: ring buffer, game-detection,
analyzer event/profile logic) · `cargo run -p lowresourcecapture` (debug,
console) · `cargo build --release` (both exes). Needs Rust MSVC toolchain + VS
Build Tools (C++ workload).

## Layered build plan

L1 foundation → L2 WGC capture + NVENC HEVC encode → L3 WASAPI audio → L4 mp4
mux/save → L5 game auto-detect wiring, toast, live hotkey reload,
run-at-startup — **all done.** `game_detect.rs`, `toast.rs`, `startup.rs`, and
the settings `gui.rs` are real implementations, not stubs (corrected
2026-08-07; this line previously undersold shipped work — the block-timeline
clip editor and NVENC-backed preview/export also shipped since, in `shared`).

**L6 — VOD analyzer** (kills/deaths → clip timeline). L6a workspace split +
second installer (done) → L6b death-card detector (done, validated over a
full VOD) → L6c review UI: background scan, scrub-and-mark, confirm/reject,
export (done) → **now adding**: `BBox` + `Kind::MonsterKill` in the label
store (done) as the data model for a trained nameplate detector + event
classifier, since colour-only detection of a kill was ruled out — box-drawing
UI, training pipeline, and ONNX inference are not built yet.

Repo layout note (2026-08-07): `profiles/` moved to `crates/analyzer/profiles/`
(analyzer-owned, not shared) and `installer/analyzer.iss` was renamed to
`installer/clipanalyzer.iss` to match its own product name/output filename.
Local recordings used for calibration/training go in
`crates/analyzer/samples/` (videos git-ignored, `.labels.json` sidecars
tracked — see that folder's README).

## Mistfall Hunter detection findings (VOD analysis 2026-08-07)

Source: Nick's own Twitch VOD, 9:22, 1080p60, 14.7 Mbps, **stereo but
effectively mono** (side channel 25–30 dB below mid). Burnt-in stream overlay:
webcam bottom-left, follower alerts top-centre. Timestamps annotated by Nick.

- **Death card — VERIFIED (n=1), the strong signal.** "YOU DIED" in red centred
  text, gold **Spectate** / **Return to Camp** buttons at y≈0.90, screen
  darkened, red vignette. Fixed position, large, unambiguous. Template match,
  no ML.
- **Player kill — VERIFIED (n=2).** Three correlates: a red enemy nameplate +
  health bar (arbitrary screen position, 2D UI sprite); a gold particle burst;
  and an **"F Loot" prompt at a fixed position** (x≈0.50, y≈0.58).
- **The game has NO kill feed** (removed by patch; the game launched
  2026-07-29 and is patching weekly). Third-party kills therefore have no HUD
  artifact at all and are out of scope — detecting them would need scene
  understanding.
- **Kill audio sting — NOT DETECTABLE in this source. Two tests, both
  negative.** Spectrogram cross-correlation between the two player kills scored
  0.44 in tight 2s windows — *below* monster-kill pairs (0.455) and level with
  unrelated controls (0.425). An earlier 8s-window result of 0.55 was an
  artifact of matching **silence** (combat noise stopping ~2.5s after a kill),
  not a shared sound. Mid/side separation is unavailable (near-mono source).
  Audio is **off the critical path**; revisit only with a local recording
  (game/mic on separate tracks) or a pristine reference from the game's assets.
- **Gold burst does NOT discriminate player kills from monster kills —
  RESOLVED 2026-08-07, n=5.** Warm-particle rise: players 3.1 / 13.2 · monsters
  4.5 / 10.8 / **21.1**. The largest burst in the sample is a monster kill and
  the smallest is a player kill; the ranges overlap completely. It is a *death*
  effect (anything dying drops loot), not a PvP one.
  - **SUPERSEDED 2026-08-08: it is NOT usable as a candidate generator
    either.** The line above previously called it "a cheap high-recall
    'something died' candidate generator". Sweeping both full recordings
    disproves that. Best case on the Twitch VOD: **4 of 6 real events found
    with 32 false positives**; on the native local recording it never exceeds
    3 of 6. A rolling-baseline variant (rise above a 3s trailing median) is no
    better — the local file's median warm-particle fraction is 6.3%, so
    continuous combat VFX swamp any threshold.
  - **Why the earlier claim was wrong, and it's the same mistake twice:** the
    burst was only ever measured *at known kill instants*. That proves it
    fires on kills; it says nothing about how often it fires otherwise. The
    death card is trustworthy precisely because it was swept across all 1123
    frames and the false-positive ceiling was measured. **Rule, now earned
    twice: a detector is not validated until it has been swept over footage
    where nothing happens.**
  - Took three attempts; the first two were instrument failures, worth
    remembering as a method warning:
    1. Guessed colour thresholds (`sat > 0.45`) — missed a burst plainly
       visible in the frame, and 4× downscale diluted the particles further.
    2. Diffed against a baseline 2.5s earlier — in a game with a moving camera
       that measures *camera motion*, not particles. Its "appeared" pixels were
       neutral grey (R−B = +4) across 15% of the frame.
    3. Tight 0.3s baseline + a warmth requirement (R−B > 30) — works, fires
       cleanly at all five kills.
  - **Rule earned: derive pixel thresholds from the pixels, and keep the
    temporal baseline short enough that camera motion can't dominate.**
- **Death card detector — VALIDATED over the full VOD (L6b, 2026-08-07).**
  Gold button row at normalized `(0.250, 0.866, 0.500, 0.065)`. Sweeping all
  1123 frames at 2 fps: card scores **0.443–0.500**, loudest non-death frame in
  the other nine minutes **0.178**. Threshold 0.30, ~2.5× margin both ways; the
  five top-scoring frames in the VOD are the death, in order. Calibrated on a
  Twitch transcode — margin is wide enough to expect it carries to native
  footage, but that is INFERENCE until tested.
- **Enemy nameplate is NOT findable by colour — VERIFIED negative,
  2026-08-07.** Strong-red pixel fraction with a plate on screen: 0.396% /
  0.387%. Without one: 0.343% (monster fight) / 0.332% (traversal) / **0.433%
  (matchmaking screen, no enemy at all)**. The largest red blob in every frame
  is scene content — blood, embers, UI — never the plate, which is only
  ~160–270 px. The game's palette is saturated with red, so colour carries no
  information here. **The PvP gate needs structural template matching, not a
  colour mask** — and it is now the hardest remaining piece, plausibly where a
  trained model earns its place rather than hand-engineered features.
- **Profiles record the game build they were calibrated against** — a patched
  HUD invalidates a profile exactly as a driver change invalidates a benchmark
  ceiling. `GameProfile::is_stale()` enforces the flag.

## Local recording, second sample (2026-08-07 continued)

Nick's own OBS-side local capture of his stream output — **not a
LowResourceCapture recording**. H.264 in an MKV container (`.mp4` extension,
misleading), 2560×1440 native, 5 AAC audio tracks (2 live at ~-42/-45 dB mean,
3 pure digital silence at -91 dB). The 5 tracks are an artifact of Nick's OBS
multitrack setup, not of LowResourceCapture's 2-track design — noted so a
future session doesn't mistake this for what the recorder itself produces.

**Critical scope correction, established through image review, not assumed:**
this footage is **Training Room combat against practice dummies, not live
matches.** Confirmed visually — "Total Damage / Progress Record" panel, "F1
Training Room Settings" label, no minimap, no "Safe Phase" timer — all absent
from the live-match Twitch VOD's HUD. Caught by actually looking at the
frames rather than trusting the annotation's "player"/"monster" wording at
face value.

- **Nick confirms (2026-08-07): the training dummy IS the "Mercenary" class
  from real PvP, and kill VFX — including the gold particle burst — are
  pixel-identical to a live kill.** Known differences: no minimap, and dummy
  behavior is "too consistent" (i.e., less varied than a live opponent).
  Loot-prompt behavior is unaffected — the game has no physical ground loot at
  all, training or otherwise. **This means client-side VISUAL kill signals
  (the gold burst, general hit VFX) generalize from this footage — VERIFIED
  by Nick, not inferred.**
- **Audio does NOT get the same pass, and this is a deliberate, reasoned
  hedge, not caution for its own sake.** Particle VFX are client-side and fire
  identically whether or not a server-confirmed kill happened. A dummy has no
  real elimination to confirm, so if Mistfall Hunter's kill sting (if one
  exists) is tied to a server-side kill-credit event rather than the hit
  itself, a dummy kill would legitimately produce silence where a live PvP
  kill would not — with no contradiction. Nick vouched for the particles
  explicitly; he did not vouch for the audio, and I'm not extending his
  confirmation past what he said.
- **Kill-sting search, native audio, n=3 (up from n=2 on the Twitch source) —
  NEGATIVE, on both live tracks independently.** Same validated method (tight
  2s windows, per-bin-whitened spectrogram cross-correlation, 0.17s run) run
  against three player-kill instants (9s, 31s, 109s) and two monster-kill
  instants (20s, 88s). Player~player pairs scored 0.517–0.545 on both tracks
  — *below* random unrelated control-pair scores (up to 0.681 on track 1,
  0.664 on track 2). **What this DOES settle:** the earlier Twitch-VOD null
  result was not a compression artifact — the same blind-search method finds
  nothing on genuinely clean, native, un-transcoded audio either, mic
  confirmed literally silent (not just quiet — Nick didn't speak, and the
  presumed-mic track shows no speech-shaped content). **What this does NOT
  settle:** whether a live PvP kill has a server-triggered sting a dummy kill
  never fires — per the hedge above, this file structurally cannot test that.
  A recording of actual live-match kills is still the only thing that can
  close this question either way.
- **Death-card threshold: still untested on native/1440p footage.** This
  sample contains no death, so the 0.443–0.500 vs 0.178 margin measured on
  the Twitch VOD remains INFERENCE-carries-to-native, not verified.
- **Nick confirms (2026-08-07, second pass): the kill audio cue IS the same
  between Training Room and live PvP, and describes it as "a slight audio
  sound"** — subtle, not a loud sting. This lifts the earlier hedge (dummy
  kills might not trigger a server-side audio cue); audio now gets the same
  generalization pass as the particle VFX, per Nick's direct statement, not
  inference.
- **Precise onset correction — annotated timestamps were off by up to 1.85s.**
  Re-running the warm-particle burst detector (validated on this same game)
  against this file located the true kill instants at 9.45s / 32.85s / 110.25s
  vs. the annotated 9 / 31 / 109. For a cue described as "slight," that much
  drift could put it outside a tight analysis window entirely — worth
  remembering for any future quiet-signal search: don't trust an eyeballed
  timestamp, re-derive the onset from a validated visual detector first.
- **Re-run at the corrected onsets, band-limited envelope correlation —
  INCONCLUSIVE, not negative, and the pattern argues against a universal
  sting.** pk1~pk2 and pk2~pk3 both spike to 0.65–0.77 correlation across
  *every* band tested (bass through 16kHz) on both live tracks; pk1~pk3 stays
  near zero in all of them. A real kill-confirmation cue should show up in
  all three pairs, not two of three, and broadband correlation across bass
  *and* treble simultaneously looks more like a shared weapon-swing/impact
  sound (pk1 and pk2 coincidentally ending on the same finisher animation)
  than a narrowband UI-style sting. This was the fourth automated variant of
  this search (whole-VOD, tight-window, precisely-realigned, band-limited) —
  per this project's failure-handling rule, two-plus automated misses is the
  trigger to change the *kind* of evidence rather than try a fifth blind
  variant, hence handing it to a human ear.
  - **CORRECTION (2026-08-07): the pair labeled as the strong match in the
    entry above was reported backwards.** `itertools.combinations(["pk1",
    "pk2","pk3"], 2)` prints as `(pk1,pk2), (pk1,pk3), (pk2,pk3)` — the raw
    per-band values were `0.010, 0.672, -0.090` (bass), i.e. **pk1~pk3 was the
    spike (0.65–0.77 across every band); pk1~pk2 and pk2~pk3 were the
    near-zero ones.** The prose written up after the run inverted this. Nick's
    "it's a monster's ringing bell" answer was given in response to the
    mislabeled framing, so it's not certain which pair he was actually
    evaluating — flagged rather than quietly trusted.
  - **Player-kill damage numbers are RED (headshot per Nick's read of the
    game's own color convention) for every player kill checked so far: pk1
    (143), pk3 (125), and a newly-added pk4 at 59.65s (149).** No white
    (non-headshot) player kill has been confirmed in this sample yet, so the
    headshot/non-headshot audio hypothesis (Nick's, 2026-08-07) can't be
    contrast-tested against this file — only same-type (all-headshot)
    comparisons are possible here.
  - **Re-run with correct pairing, now n=3 same-type (all-headshot) kills
    (pk1, pk3, pk4) — all three pairs elevated, both tracks:** whitened
    matched filter 0.45–0.58; band-limited envelope 0.42–0.83 across bass
    through 16kHz. This is a real change from the earlier (mislabeled)
    picture. **NOT YET RESOLVED whether this is the kill cue or the same
    monster-bell ambiance persisting across a wider stretch of the
    recording than assumed** — a repeat sample size of 3 windows can't
    distinguish "shared kill sound" from "shared background ambiance still
    playing in all three windows." Sent Nick the pk4 clip (already has pk1
    and pk3 from the prior round) with the corrected question. PENDING.
  - **Nick's headshot/non-headshot hypothesis (2026-08-08): confirmed
    non-headshot example found and cross-tested — INCONCLUSIVE, not
    negative.** Nick identified the Twitch VOD's 2:22 kill (k1, precise
    onset 142.4s) as a white/non-headshot damage number; independently
    verified from the frame ("249"/"124", both white). Cross-correlated
    against the three confirmed red/headshot local instances (pk1/pk3/pk4):
    **white~red scores (0.44–0.51 whitened) land in the same range as
    red~red scores (0.45–0.58)** — no separation by headshot status, which
    argues against the hypothesis, but the comparison is confounded by
    crossing sources (Twitch transcode, single mixed track vs. native
    local, clean separated tracks), so a real distinction could be getting
    washed out by that mismatch rather than not existing. Sent Nick the
    white-kill clip for direct listening comparison against the pk1/pk3/pk4
    reds, per the established pattern in this investigation that his ears
    have caught things (the monster bell) blind correlation could not.
  - **CLOSED — kill audio sting: VERIFIED NEGATIVE, no reservation. Nick
    confirms (2026-08-08), listening directly: there is no audio cue when a
    player dies.** This is ground truth, not another correlation score — his
    ears are the actual instrument this whole investigation kept deferring
    to, and every automated attempt either found nothing or found a false
    lead (the monster bell). The headshot/non-headshot hypothesis is now
    moot along with it: there being no cue at all supersedes the question of
    whether two different cues exist. **Audio is off the table as a kill
    signal, permanently, not just off the critical path.** Do not re-open
    this without new evidence Nick hasn't already ruled out by ear.
  - **Consequence for the project: every hand-engineered shortcut for kill
    detection has now failed.** No kill feed (game mechanic). Nameplate
    colour doesn't discriminate (verified negative, 2026-08-07). Gold burst
    doesn't discriminate player from monster (verified negative,
    2026-08-07). Kill audio doesn't exist (verified negative, 2026-08-08).
    **The trained nameplate detector + event classifier (L6's `BBox` /
    `Kind::MonsterKill` data model) is no longer one option among several —
    it is the only remaining path**, and it depends entirely on the boxes
    and marks Nick produces in the review window.

## Confidence discipline for this repo

Nothing touching `windows-rs` is VERIFIED until it compiles/runs on Nick's
Windows machine. Logic with `cargo test` coverage (ring buffer, detection
rules) IS verified cross-platform. Keep the two categories distinct in any
status report.
