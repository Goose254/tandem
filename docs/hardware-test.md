# Hardware test checklist

The engine's maths (calibration analysis, drift controller, channel mixing) and the settings
file are covered by automated tests (`cargo test`). The UI has no automated tests: it is
type-checked and linted, and reviewed by hand against a simulated backend (`npm run dev`).
None of that can cover real hardware: a Bluetooth link, a real microphone, a real disconnect.
This is the list to run through with the actual devices. Each step says what to expect, so a
miss is easy to report (open an issue with the step number and what happened instead).

Setup used below: the Bluetooth amp as the Windows default output (the source), one or more
wired outputs added in Tandem.

## 1. Basics

1. Play music. **Expect:** the source card says "Capturing", each output "In sync", meters move.
2. Turn the calibration clicks on (Sync > By ear) and set the delay until the two blips merge.
   **Expect:** a delay somewhere between 100 and 300 ms for a typical Bluetooth amp.
3. Leave music playing for 30 minutes. **Expect:** still in sync by ear, 0 dropouts shown, and
   the drift figure on the output card staying within a few tens of ppm.

## 2. Load

4. With music playing, start something heavy (a game, a build, a video export).
   **Expect:** no crackles and the dropout counter stays at 0.

## 3. Volume

5. Press the volume keys. **Expect:** the amp and every output get quieter or louder together.
6. Mute Windows. **Expect:** everything goes silent, and comes back on unmute.
7. Turn "Volume keys act on every output" off (Settings > Audio) and repeat step 5.
   **Expect:** only the amp changes if it applies the volume itself (most Bluetooth devices do).

## 4. Disconnects

8. Switch the amp off, wait ten seconds, switch it back on. **Expect:** the app stays
   responsive throughout, shows "Waiting" or "Source unavailable", then recovers on its own.
9. Unplug a wired output while it plays and plug it back. **Expect:** the same, for that output
   only; the others keep playing.
10. Change an output's sample rate in Windows Sound settings while it plays.
    **Expect:** a short interruption, then "In sync" again, with a rate-mismatch note if the
    rates now differ.

## 5. Microphone auto-sync

Steps 11 and 12 are the mode that was confirmed in November 2026. Run them again after any
change to auto-sync: they must still behave the same.

11. Put the microphone where you listen, pick it in Sync, leave "Line up with" on "The
    source", make sure nothing else is playing, run auto-sync.
    **Expect:** two sweeps per pass, from the source and then from the source and the output,
    then "Done" with a delay close to the one found by ear in step 2 (within a few
    milliseconds).
12. Run it again straight away. **Expect:** "Already in sync".
13. **Line up with another output, with a source you can hear.** Two wired outputs, A and B,
    plus the usual source (the Bluetooth amp), all audible from where the microphone is.
    Sync A to the source first (step 11). Then in Sync, pick B, set "Line up with" to A, and
    run auto-sync.
    **Expect:**
    - the sweeps come out of A, then of A and B. **The source stays silent** the whole time;
    - "Done" (or "Already in sync");
    - with the calibration clicks on afterwards, A and B sound as one blip;
    - A's delay has not changed, unless the result says that A "now waits for" B, which
      happens only when B is still late at 0 ms.
14. Run step 13 again straight away. **Expect:** "Already in sync", and no delay moves.
15. **Another program is playing.** Start music in a player and a video in a browser, both
    audible. Open the Windows volume mixer (right-click the speaker icon > Open volume mixer)
    to watch them. Run auto-sync as in step 11.
    **Expect:**
    - both programs go silent within a moment and show as muted in the mixer;
    - the sweeps play alone, and the result is as good as in step 11;
    - when it ends, with a result or with an error, both programs are audible again and
      unmuted in the mixer, at the volume they had;
    - a program that was already muted in the mixer before the run is still muted after it.
16. Repeat step 15 with "Line up with" set as in step 13. **Expect:** the same, and nothing of
    the music comes out of A or B during the measurement.
17. **Tandem dies mid-measurement.** With music playing, run auto-sync and, while the sweeps
    play, end `tandem.exe` from the Task Manager. The music program stays muted (that is the
    case being tested). Start Tandem again.
    **Expect:** within a few seconds of the window appearing, the music is audible again and
    unmuted in the mixer. `%APPDATA%\com.vdouay.tandem\muted-sessions.json` exists while the
    program is muted and is gone afterwards.
18. **Automatic re-sync never silences anyone.** Turn on "Re-sync when a device reconnects".
    With music playing, switch the amp off and on again.
    **Expect:** the music keeps playing untouched and no sweep is heard. Pause the music:
    a few seconds later the sweeps play and the delay is checked. If the music never stops,
    nothing happens, and after ten minutes the re-sync is dropped.

Result so far (maintainer's setup, November 2026): steps 11 and 12 passed. Auto-sync moved a
delay that had been set by ear by 4 ms, the result sounds very good, and a second run
answered "Already in sync". That was before the sweeps of "Line up with" were moved off the
source and before other programs were silenced: steps 13 to 18 have not been run yet.

## 6. Pause when nothing plays

19. Turn it on (Settings > Audio), stop all sound and wait a little over two minutes.
    **Expect:** the status reads "Asleep until something plays".
20. In an administrator terminal, run `powercfg /requests`. **Expect:** no audio stream listed
    under SYSTEM for Tandem. Compare with the option off and Tandem running: an audio stream
    listed there is what keeps Windows awake. This has not been confirmed on any machine yet,
    so please report what you see either way.
21. Play something. **Expect:** outputs join within about a second.

## 7. Surround

22. If the source is set to 5.1 or 7.1 in Windows and an output is stereo, play a film or a
    surround test. **Expect:** dialogue (centre channel) is audible on the stereo output.

## 8. Tray

23. Click the tray icon. **Expect:** the popup opens next to it, fully on screen, also with the
    taskbar on another edge or another monitor.

## 9. First run

These need Tandem as a new installation would see it, without losing your settings: quit
Tandem, then start it from a terminal with an empty settings folder (your own settings are
not touched, and come back when you start Tandem normally).

```bash
TANDEM_CONFIG_DIR="$(mktemp -d)" ./target/release/tandem.exe
```

24. **Expect:** the window opens on "Getting started", step 1 of 5, and no sound plays.
25. Go through the steps: keep the source, add an output, set its delay by ear or with the
    microphone, finish. **Expect:** each step shows the same controls as the Outputs and Sync
    screens; clicks and sweeps only play when you turn the clicks on or press "Run
    auto-sync"; after "Finish" the Outputs screen shows what you set up.
26. Start again with a new empty folder, go to step 3, close Tandem from the tray, start it
    again with the *same* folder. **Expect:** it reopens on step 3. Add an output, close the
    window (not from the tray), open it again from the tray: the guide is gone.
27. Switch the source's device off while on step 2 or 4. **Expect:** the same "Not connected"
    state as on the Outputs screen, and the guide carries on when it comes back.
28. Start Tandem normally, with your own settings. **Expect:** no guide, everything as you
    left it. Settings > Getting started > "Show it again" opens it.

Result so far (November 2026): a script drove the real app through steps 24 to 26 and the
"Show it again" part of step 28, hidden, with one output nobody hears and without turning the
clicks on or running auto-sync. Every expectation it could read from the page and the
settings file held. Not done by anyone yet: the same by hand, anything with sound (the clicks
and auto-sync from step 4 of the guide), and step 27.
