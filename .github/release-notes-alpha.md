## LowResourceCapture — alpha: audio settings now apply live 🎚️

### Fix: changing the audio setting actually takes effect now
The mic-on/off and **Mix game + mic into one track** toggles were saving, but the
already-running audio capture never picked up the new mode — so toggling did
nothing until an app restart. Now **saving an audio change restarts audio capture
immediately**, so the new mode applies right away.

### For your setup (Discord + commentary)
Turn on **Settings → Audio → "Mix game + mic into one track"** and save. From then
on, clips have a single combined track with **game audio + Discord friends'
voices** (they play out your headphones, so the desktop-loopback catches them)
**+ your mic** — all audible on normal playback, no track-switching.

*(In the previous build your mic was actually recorded, just on a separate track 2
that most players don't play by default. Mixed mode puts everything on one track.)*

### Try it
1. Install over the top.
2. Settings → Audio → flip **Mix game + mic into one track** ON → **Save**.
   (The log should show `audio restarted for new mode: GameAndMicMixed`.)
3. Record a clip while talking → you should now hear your mic mixed with
   everything on normal playback.
4. Tell me how the balance sounds (mic vs game level, any drift) and I'll tune
   the mixer — and if it sounds right I'll make mixed the default.

### Note
Unsigned installer — SmartScreen + UAC prompts are expected.
