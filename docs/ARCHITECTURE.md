# How Tandem works

A tour of the design for someone reading the code for the first time. The
[README](../README.md) says what Tandem does for a user; this page says how, and where each
part lives. Everything here is written from the code: when the two disagree, the code is right
and this page needs fixing.

Contents: [the pieces](#the-pieces) · [the audio pipeline](#the-audio-pipeline) ·
[delay is a buffer level](#delay-is-a-buffer-level) · [drift control](#drift-control) ·
[volume and channels](#volume-and-channels) · [microphone auto-sync](#microphone-auto-sync) ·
[the supervisor](#the-supervisor) · [settings](#settings) · [the UI](#the-ui) ·
[updates](#updates) · [releases](#releases) ·
[what is verified, and how](#what-is-verified-and-how)

## The pieces

| Part | Language | What it is |
|---|---|---|
| `engine/` | Rust | `tandem-engine`, a library with no UI: devices, loopback capture, one render thread per output, drift control, channel mixing, microphone recording, the calibration maths. Also `tandem-cli`, a small command-line front end for testing it. |
| `src-tauri/` | Rust | The desktop app (Tauri 2). It owns one engine and keeps it matching the settings and the connected devices (the *supervisor*), runs auto-sync, stores the settings, draws the tray icon, and exposes commands to the UI. |
| `src/` | TypeScript | The UI: React 19, zustand, CSS modules. Two windows share one bundle, the main window and the tray popup. |

The engine knows nothing about settings or windows, and the UI never touches audio: the desktop
crate sits between them.

```
            commands (invoke)                    owns, reconciles
   UI  ───────────────────────────►  src-tauri  ─────────────────►  engine  ──►  WASAPI
 (src/) ◄───────────────────────────             ◄─────────────────
          events: status, levels,                 status, errors,
          settings, calibration                   levels
```

## The audio pipeline

`engine/src/engine.rs`. One **source** device (what Windows plays on) and any number of
**outputs** that each receive a delayed copy.

```
                         source device (e.g. a Bluetooth amplifier)
                            ▲                        │
        keepalive thread ───┘                        │ WASAPI loopback
        silence, clicks, test sweeps                 ▼
                                               capture thread
                                                     │ copies every packet into
                              ┌──────────────────────┼──────────────────────┐
                              ▼                      ▼                      ▼
                           FIFO A                 FIFO B                   ...
                              │                      │
                     render thread A        render thread B
                     resampler (ratio set by the drift controller)
                     channel mix, gain
                              │                      │
                              ▼                      ▼
                          output A               output B
```

Three kinds of thread, all in Windows' shared mode (no exclusive access, no driver):

- **Keepalive** (`keepalive_loop`) plays silence on the source device. Loopback capture only
  delivers packets while something renders to the device, so this keeps the capture flowing
  during silence, and keeps a Bluetooth link awake so that its latency stays the same. The
  same stream carries the calibration clicks and the auto-sync sweeps, and it counts frames,
  which gives every injected signal an exact start position on the source's clock. It also
  reads the source's volume every few device periods (see [volume](#volume-and-channels)).
- **Capture** (`capture_loop`) opens the source for loopback, which yields the mix exactly as
  Windows hands it to the device, ahead of whatever latency the device adds afterwards. Every
  couple of milliseconds it appends the new samples to one FIFO per output. A FIFO is capped
  (the longest delay plus a second) so that a stalled output cannot grow it without bound.
- **Render** (`render_loop`), one per output, wakes when its device wants audio, pulls 128
  source frames at a time from its FIFO through an asynchronous sinc resampler
  ([rubato](https://github.com/HEnquist/rubato)), mixes the channels to the output's layout,
  applies the gain with a ramp, and writes to the device.

Each audio thread asks Windows for the "Pro Audio" scheduling class (`engine/src/sys.rs`).
That is best effort: when the request is refused, the thread simply keeps normal priority.

A failing output stops only itself; a failing source stops the engine. Neither restarts on its
own: that is the [supervisor](#the-supervisor)'s job.

The FIFO between capture and render is a `Mutex<VecDeque<f32>>`, held for short copies. It is
not lock-free; a lock-free ring is a possible refinement, not something the code has.

## Delay is a buffer level

Tandem does not timestamp audio or schedule it against a clock. An output's delay *is* the
amount of audio sitting between the capture and the output device, and the render thread's job
is to keep that amount constant.

Each time the device asks for audio, the render thread measures what is buffered for this
output, in seconds:

```
buffered = frames waiting in the FIFO                    (source rate)
         + frames resampled but not yet written          (output rate)
         + frames in the device buffer (its "padding")   (output rate)
         + the resampler's own delay                     (output rate)
```

and compares it with a target:

```
target = floor + the delay the user set
```

**The floor is what "0 ms" means.** It is the least that can stay buffered without dropouts:
the device buffer (one period, the smallest shared mode allows), one 128-frame resampler chunk
and the resampler's lookahead, one loopback packet of capture jitter, and 3 ms of headroom. It
is computed per output when its stream opens and shown in the app as the output's *minimum
lag*. The user's delay is added on top, so 0 ms means "as early as this output can play", not
"at the same time as the source". An output can therefore only ever be held back. That is why
the slowest device has to be the source: every other output waits for it.

Three things follow from treating delay as a level:

- **Starting** (*priming*): the thread writes silence until the buffered amount reaches the
  target, trims the overshoot so it starts exactly on target, and fades in.
- **Changing the delay** is immediate: silence is inserted at the head of the FIFO, or frames
  are dropped from it, and the controller is told about the jump. The user hears the new delay
  at once, which is what makes tuning by ear workable.
- **Running dry** (an *underrun*): the last few milliseconds are faded to silence so the
  dropout is a dip and not a click, the counter shown in the app goes up, and the output primes
  again.

## Drift control

`engine/src/control.rs`. Two devices never run at exactly the same rate: tens of parts per
million apart is normal, which is a few milliseconds per minute. Left alone, the buffered
amount would creep until the output is audibly early or late, or runs dry.

A PI controller per output watches the buffered amount and nudges the resampling ratio:

- The measurement is low-pass filtered (one second) because it moves in packet-sized steps.
- The proportional term answers the current error (10 ms of error asks for 1000 ppm); the
  integral term learns the steady drift between the two clocks, so the error settles at zero
  instead of at a constant offset. The gains are chosen for a critically damped response.
- The correction is clamped to ±1000 ppm, under two cents of pitch: inaudible.
- An error beyond 50 ms is not worth minutes of gentle resampling. The controller asks for a
  *resync* instead: the FIFO is shifted in one step, as for a delay change.

The app shows the current correction per output (the "drift" figure, in ppm).

Because every output is resampled anyway, a source and an output at different sample rates
work too: the nominal ratio is simply no longer 1. Matching rates keeps the resampling a tiny
correction, so the app flags a mismatch.

## Volume and channels

- **Volume** (`engine/src/sys.rs`, `EndpointVolume`). Loopback captures after Windows'
  software volume, so that volume is already in the copies. A device that applies the volume
  itself (most Bluetooth devices) does it after the loopback tap, where Tandem cannot see it:
  the volume keys would change the source alone. When "Volume keys act on every output" is on,
  the keepalive thread reads the source's volume and mute, and the render threads apply the
  part loopback did not carry. A software volume is never applied a second time.
- **Channels** (`engine/src/mix.rs`). The layout is assumed from the channel count. A channel
  the output lacks is folded into its nearest speakers and not dropped, so a 5.1 source on
  stereo speakers keeps its centre and surround channels.

## Microphone auto-sync

The maths is in `engine/src/calibrate.rs` (no audio I/O, so it can be tested on synthetic
recordings); the procedure is in `src-tauri/src/calibration.rs`.

The problem: find how far an output is from the source, with a microphone whose own latency is
unknown. The trick is to put both arrivals in one recording, so that the microphone's latency
is the same for both and cancels out.

1. Mute every output and start recording.
2. Play a one-second logarithmic sweep on the source. The microphone hears **the source
   alone**.
3. Unmute the output being synced and play the same sweep again. The microphone hears the
   source **and** the output's delayed copy. The time between the two sweeps is known to the
   sample, from the keepalive thread's frame counter.
4. Analyse the recording:
   - **Deconvolve.** The recording's spectrum is multiplied by a regularised inverse of the
     sweep's spectrum, which turns each arrival of the sweep into a sharp peak. The
     regularisation keeps noise outside the sweep's band from being amplified. Keeping only the
     positive frequencies gives a smooth envelope.
   - **Find the source.** Only the source arrives twice, exactly one spacing apart. The lag
     whose envelope is strong both at that lag and one spacing later is the source, and the
     earliest such lag within 6 dB of the best is taken, so the direct sound wins over a later,
     louder reflection.
   - **Find the output.** The envelope around the first sweep's arrival (direct sound and room
     reflections) is aligned onto the second sweep and subtracted. What is left near the second
     sweep is the output alone, even when it is only a couple of milliseconds from the source
     or sits on top of the source's reflections.
   - The gap between the two is the correction.
5. Apply the correction to the delay and measure a second time to verify. If the first
   measurement was already within 1.5 ms, the second pass is skipped.

Edge cases the procedure handles:

- **The output is late even at 0 ms.** It cannot be played earlier, so auto-sync sets 0 ms and
  reports how late it still is ("limited"): the source is too fast for this output.
- **A source nobody listens to** (several outputs of one interface, the source being an unused
  one): the microphone cannot hear it. With a "Line up with" output chosen, that output plays
  the part of the source in both sweeps, and the measurement is between the two outputs. If the
  synced output is behind the reference even at 0 ms, the reference is delayed instead.
- **Mutes are restored on every exit path**, including failures, and a failed or interrupted
  run never leaves auto-sync marked busy.

Failures are reported as codes the UI translates: no microphone, microphone missing or
unusable, output not playing, sweep not heard, output not heard, recording too short.

## The supervisor

`src-tauri/src/supervisor.rs`. The engine does what it is told and stops when a device goes
away. The supervisor is the loop that decides what it should be doing: about four times per
second it compares three things and closes the gaps.

```
   settings (what the user wants)  ─┐
   connected devices (scanned 1/s) ─┼─►  reconcile  ─►  start / stop the engine,
   the running engine (what is)    ─┘                   add / remove / retry outputs,
                                                         apply delay and gain,
                                                         publish a status snapshot
```

In one pass:

- **Source.** Pick the source (the Windows default output, or a pinned device). Start the
  engine on it; tear it down when mirroring is off, the engine died, or the source changed.
  A failed start is retried after two seconds.
- **Outputs.** For each output in the settings: remove it when it is disabled, absent, or is
  the source itself; add it when it is wanted and missing; drop it and retry later when its
  thread died. Delay and gain are applied to the running output.
- **Devices that come back under a new id.** A remembered device is found by id, then by name
  (Windows sometimes gives a re-paired Bluetooth device a new id). A match by name counts only
  if no other output already owns that device, so two outputs never end up on one device.
- **Pause when nothing plays** (optional). After two minutes of silence the engine is closed so
  that Windows can sleep, and the source's own level meter, which needs no stream of ours, is
  watched to wake up.
- **Re-sync after a reconnect** (optional). A source or output that comes back gets an
  auto-sync a few seconds later, except right after launch or after waking from a pause.
- **Publish.** A full status snapshot goes to the UI and the tray on every pass. Between two
  passes the meter levels alone are sent three more times, so the meters move smoothly.

### The locking rule

The engine lives in a `Runtime` behind one mutex (`AppState::runtime` in
`src-tauri/src/state.rs`), shared by the supervisor and by every command the UI sends. Two rules
keep the UI responsive and the state consistent:

1. **Never wait for an audio thread while holding the runtime lock.** A thread on a device that
   has just vanished can take a second to notice. Engines and outputs that are going away are
   only *told* to stop under the lock; they are collected in `Retired` and dropped, which is
   what waits for their threads, after the lock is released.
2. **Engine and settings change together, under the runtime lock** (`apply_delay`,
   `set_output_gain`), and the supervisor reads the settings under that same lock. Otherwise it
   could read an older delay and put it back over the one a command had just applied. The lock
   order is always runtime, then settings.

## Settings

`src-tauri/src/settings.rs`. One JSON file, `settings.json`, in the app's configuration
directory (`%APPDATA%\com.vdouay.tandem\`).

- **Delays are remembered per source.** The right delay for an output depends on which device
  is the source, so each output keeps a map from source id to delay, plus a default for a
  source it has never been paired with (the most recent value, which is usually close).
- **Saved atomically:** written to a temporary file, then renamed.
- **Never silently lost.** A file that cannot be parsed is copied to `settings.unreadable.json`
  before defaults replace it; a file from a newer version of the app is copied to
  `settings.v<N>.json`, because this version would drop the fields it does not know.
- **Forward compatible:** every field has a default, so older files load. A change in the
  *meaning* of a field needs a version bump and a migration.

## The UI

- `src/lib/ipc.ts` is the only door to the Rust side: typed wrappers over the Tauri commands
  (registered in `src-tauri/src/lib.rs`), and the events `status`, `levels`, `settings` and
  `calibration`.
- `src/state/store.ts` loads the state once, then follows those events. The UI holds no audio
  state of its own.
- Every string is in `src/i18n/en.ts` and `src/i18n/fr.ts`. The design language (tokens,
  layout, controls) is adapted from [Plisto](https://github.com/Altervayne/plisto) by Florian
  Douay; see [NOTICE](../NOTICE).
- **Simulated backend.** Outside the desktop shell, `src/lib/mock.ts` answers the same commands
  and events from an in-memory model. It exists in exactly two places: the dev server
  (`npm run dev` in a browser) and the demo build (`npm run build:demo`). The condition is a
  build-time constant, so the bundle that ships in the installer does not contain it.

## Updates

`src/lib/updater.ts`, `src/state/updater.ts`, `src/components/updater/UpdatePrompt.tsx`, on top
of Tauri's updater plugin.

- The app reads `latest.json` from the repository's latest published release, five seconds after
  launch and every six hours (this can be turned off), or on demand.
- An update is offered only when the feed's version is higher. Nothing is installed without the
  user's go-ahead; "Skip this version" keeps it quiet until a newer one ships.
- The download is verified against a signature with the public key compiled into the app
  (`src-tauri/tauri.conf.json`) before the installer runs. The private key is never in the
  repository.
- A failed check is an error and is never shown as "up to date"; an automatic check that fails
  stays silent.

## Releases

A release is a Git tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`, which checks that
the tag matches the app version and that the signing key is present, runs the engine tests,
builds and signs the installer, and publishes a GitHub release with the installer, its
signature and `latest.json`. Publishing is what installed copies see on their next check, so the
tag is the release decision. The full runbook is [RELEASING.md](RELEASING.md).

Every pull request and every push to `main` runs `.github/workflows/ci.yml`: Rust formatting,
the UI lint, the type-check and UI build, and the Rust tests, on Windows.

## What is verified, and how

Tandem's hard part is real audio hardware, which no automated check reaches. This table is the
honest split.

| Area | Automated (`cargo test`, on Windows) | Only on hardware, by hand |
|---|---|---|
| Calibration analysis | Synthetic recordings: an output late, early, or 2 ms away; louder than the source; a reflection stronger than the direct sound; heavy noise; a missing output; pure noise; recordings cut short (`calibrate.rs`) | A real room, speaker and microphone. Confirmed in November 2026: auto-sync landed 4 ms from a delay set by ear (checklist step 11). Lining up with another output (step 13) is **not yet confirmed.** |
| Drift controller | A simulated clock mismatch of +100 and −80 ppm with packet-sized jitter settles on target; a large error triggers a resync (`control.rs`) | Real clocks over a long listen (step 3 of the hardware checklist) |
| Channel mixing | Identity, mono, stereo, 5.1 and 7.1 folds (`mix.rs`) | A surround source on stereo speakers |
| FIFO shifts, underrun fade, clicks | Unit tests on the buffer helpers (`engine.rs`) | Dropouts under CPU load |
| Thread priority | One test asks Windows for the "Pro Audio" class (`sys.rs`) | Whether it prevents dropouts under load |
| Settings | Round trip, old and partial files, unreadable and newer files kept aside, per-source delays (`settings.rs`) | |
| Supervisor | Only its pure helpers: finding a device by id then name, sorting errors into kinds | The reconciliation loop itself: disconnects, reconnects, a changed default device, sleep and wake |
| Capture, render, keepalive threads | Nothing: they need devices | Everything |
| Volume following | Nothing | A source that applies its own volume. **Not yet confirmed.** |
| Pause when nothing plays | Nothing | Whether Windows actually sleeps and wakes cleanly. **Not yet confirmed.** |
| Tray icon | The drawn icon has three notes and a centre (`tray.rs`) | Popup placement on other taskbar positions and monitors |
| UI | No tests. Type-checked (`tsc`) and linted (ESLint); reviewed by hand against the simulated backend | The same screens against the real backend |
| In-app update | Nothing | One published release updating to the next. **Not yet confirmed.** |

The hardware checks, step by step, are in [hardware-test.md](hardware-test.md). Mirroring with
delays set by ear or by microphone auto-sync is the part that has been used on real devices,
and it works well there. The two features marked *experimental* in the README are the ones
this table marks as not yet confirmed.
