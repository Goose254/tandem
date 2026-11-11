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

11. Put the microphone where you listen, pick it in Sync, pause the music, run auto-sync.
    **Expect:** two sweeps per pass, then "Done" with a delay close to the one found by ear in
    step 2 (within a few milliseconds).
12. Run it again straight away. **Expect:** "Already in sync".
13. With two wired outputs and a source you cannot hear: set "Line up with" to one of them and
    run auto-sync on the other. **Expect:** "Done", and the clicks from both outputs merge.

## 6. Pause when nothing plays

14. Turn it on (Settings > Audio), stop all sound and wait a little over two minutes.
    **Expect:** the status reads "Asleep until something plays".
15. In an administrator terminal, run `powercfg /requests`. **Expect:** no audio stream listed
    under SYSTEM for Tandem. Compare with the option off and Tandem running: an audio stream
    listed there is what keeps Windows awake. This has not been confirmed on any machine yet,
    so please report what you see either way.
16. Play something. **Expect:** outputs join within about a second.

## 7. Surround

17. If the source is set to 5.1 or 7.1 in Windows and an output is stereo, play a film or a
    surround test. **Expect:** dialogue (centre channel) is audible on the stereo output.

## 8. Tray

18. Click the tray icon. **Expect:** the popup opens next to it, fully on screen, also with the
    taskbar on another edge or another monitor.
