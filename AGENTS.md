# Working on Tandem

Notes for anyone (person or coding agent) picking this project up cold. The README explains
what Tandem is for users; this file explains how to work on it without breaking things, and
records the decisions that are not visible in the code.

- How it works, for a human reader (pipeline, delay, drift control, auto-sync, supervisor,
  what is and is not verified): [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Read it before
  touching the engine or the supervisor; this file does not repeat it.
- Publishing a version: [docs/RELEASING.md](docs/RELEASING.md).
- Checks that need real hardware: [docs/hardware-test.md](docs/hardware-test.md).

## What it is

Tandem is a Windows tray app that mirrors what Windows plays on one device (the *source*,
normally the slowest one, such as a Bluetooth amplifier) to any number of other outputs, each
delayed so they all sound together, with continuous clock-drift correction.

| Path | What |
|---|---|
| `engine/` | Rust library `tandem-engine`: WASAPI loopback capture, one render thread per output, drift control, channel mixing, microphone recording, calibration maths. Also the `tandem-cli` test tool (behind the `cli` feature, on by default; the app turns it off). |
| `src-tauri/` | The desktop app (Tauri 2): supervisor, auto-sync orchestration, settings, tray, commands. |
| `src/` | The UI: React 19, TypeScript, CSS modules, zustand. `lib/mock.ts` is a simulated backend for the browser preview and the demo build. |
| `docs/` | Architecture, release runbook, release notes for the next version, hardware checklist, README images. |
| `.github/workflows/` | `ci.yml` (pull requests and pushes to `main`) and `release.yml` (on a `v*` tag). |

Key files:

- `engine/src/engine.rs`: the audio threads. `keepalive_loop` plays silence on the source (and
  clicks or test sweeps), `capture_loop` reads loopback, `render_loop` runs per output.
- `engine/src/control.rs`: the PI drift controller. `engine/src/mix.rs`: channel mixing.
  `engine/src/sys.rs`: "Pro Audio" thread priority and the endpoint volume.
  `engine/src/sessions.rs`: muting other programs' audio sessions for a measurement.
  `engine/src/calibrate.rs`: sweep generation and analysis for microphone auto-sync.
- `src-tauri/src/supervisor.rs`: a loop that reconciles the running engine with the settings and
  the connected devices, and publishes status to the UI.
- `src-tauri/src/calibration.rs`: the auto-sync procedure. `quiet.rs`: the crash-safe record
  of programs muted for a measurement. `settings.rs`: the settings file.
  `tray.rs`: tray menu, popup placement, and the tray icon drawn in code.
- `src/state/store.ts`: app state. `src/state/updater.ts` and `src/lib/updater.ts`: updates.
- `src/components/guide/`: the first-run guide. It composes parts of the other screens
  (`SourceSection`, `OutputsSection`, `SyncPanel`, `AutostartRow`) and adds no logic.

## Commands

```bash
npm install
npm run dev              # UI only in a browser at http://localhost:1420, simulated backend
npm run tauri dev        # the real app with hot reload
npm run build            # type-check and build the UI into dist/ (no simulated backend in it)
npm run build:demo       # the UI with the simulated backend into dist-demo/, for embedding
npm run lint             # ESLint on the UI (eslint.config.js); CI fails on any error or warning
cargo fmt --all --check  # Rust formatting; CI fails on a difference, so run `cargo fmt --all` first
cargo test --release     # all Rust tests (the desktop crate needs dist/ to exist: build the UI first)
npm run release:unsigned # installer without update signing: use this for local testing
npm run release          # signed installer: needs the private key, see docs/RELEASING.md
npm run icons            # regenerate icon files from src-tauri/icons/source/app-icon.svg
npm run licenses         # regenerate THIRD_PARTY_LICENSES.md after dependency changes
```

Requirements: Windows 10 or 11, Rust (MSVC toolchain, edition 2024), Node 20 or newer.

## Rules that are not obvious from the code

**Working with the maintainer**

- Commit and push only when asked. History so far is plain commits on `main`.
- GitHub sign-in and repository secrets are the maintainer's steps. A release needs neither:
  pushing a version tag builds and publishes it (docs/RELEASING.md), so tag only on request.
- Do not close the maintainer's running Tandem. It is often running from
  `target/release/tandem.exe`, which makes a build fail with "Access is denied"; rename the
  file and build again (see docs/RELEASING.md, "Building locally").
- Do not change Windows settings on the maintainer's machine (device formats, volume, default
  device). The app itself only reads them and offers a button to open Sound settings.
- Do not play test sounds on real speakers without saying so. For engine checks use devices
  nobody hears, with silence or calibration clicks (see "Verifying a change").

**Licensing and naming**

- Tandem is AGPL-3.0-or-later. The design language is adapted from
  [Plisto](https://github.com/Altervayne/plisto) by Florian Douay, also AGPL, and is credited
  in `NOTICE` and the README: keep that credit, and never present the design as original to
  Tandem. The Plisto name and logo are reserved: never use them in Tandem.
- New dependencies must be AGPL-compatible; run `npm run licenses` after adding one.

**Design**

- Design tokens live in `src/styles/tokens.css`. One solid accent button per screen, quiet
  controls elsewhere, no boxes around settings rows.
- Every user-facing string exists in both `src/i18n/en.ts` and `src/i18n/fr.ts` with the same
  keys. The tray menu and tooltip strings are in `src-tauri/src/tray.rs`.
- The guide must stay a composition. A step shows the same part the Outputs, Sync or Settings
  screen shows, driving the same store and commands; when a screen changes, the guide follows
  without being touched. Do not give it a second way to add an output, set a delay or run
  auto-sync. Nothing in it starts a sound on its own: clicks and sweeps start from the switch
  and the button of the Sync part.
- A drawing never carries what the text beside it does not. The sentence is the source of
  truth; drawings are `aria-hidden`; an auto-sync outcome is announced once, through the live
  region in `SyncView.tsx`. Colours come from the tokens (the warn tone marks the part to fix).
- Motion is CSS and inline SVG, no animation library. Every animation either ends on its own
  or runs only while something is in progress, and has a still state under
  `prefers-reduced-motion` (the global rule in `src/styles/base.css` jumps it to its last
  frame, so design that frame as the still state).

**Audio behaviour: the rules to keep** (the reasons are in docs/ARCHITECTURE.md)

- Delays are added on top of each output's minimum path, so 0 ms means "as early as this output
  can play". Outputs can only be delayed, never advanced: the slowest device must be the source.
- Loopback capture already contains Windows' *software* volume (measured: a device at 90% came
  through 1.6 dB quieter). Only a *hardware* volume (most Bluetooth devices) is applied by the
  engine. Do not apply a software volume a second time.
- The supervisor must never wait for audio threads while holding the runtime lock: the UI's
  commands share that lock. Things going away are told to stop under the lock and dropped after
  it (`Retired` in `supervisor.rs`).
- Engine and settings change together under the runtime lock (`apply_delay`,
  `set_output_gain`), and the supervisor reads the settings under the same lock. Lock order is
  always runtime, then settings.
- Auto-sync must restore everything it silences on every exit path: every output's mute
  state, the outputs' isolation from the source (`set_outputs_isolated`), and other programs.
- Other programs are muted only by a measurement the user started, never by an automatic one,
  and only through `quiet.rs`: the session is written to `muted-sessions.json` *before* it is
  muted, because Windows keeps a program's mute across restarts. Never call
  `sessions::mute` from anywhere else in the app.
- The source mode of auto-sync (no "Line up with") is the one confirmed on hardware. Keep its
  sequence of calls and waits as it is unless the maintainer asks; `keepalive_loop` plays its
  sweeps, and the fan-out must stay a pass-through outside a measurement (there is a test).
- "Pause when nothing plays" stays off by default: the first sound after a pause reaches the
  outputs late.
- The simulated backend (`src/lib/mock.ts`) is loaded by `src/lib/ipc.ts` on demand, in the
  dev server and the demo build only. Never import it statically: it would ship in the app.

**Unsafe code**

- Avoid it. Both crates deny `unsafe` (`#![deny(unsafe_code)]`), and the two binaries forbid
  it. Before writing a direct Windows call, look for a safe wrapper in `wasapi` or Tauri.
- Where there is none, the call lives in `engine/src/sys.rs` or `engine/src/sessions.rs`
  (which opt in at the top of the file) or in a single function marked
  `#[allow(unsafe_code)]`. Keep each block to the calls themselves, put a `// SAFETY:` comment
  on it that says why Windows' requirements hold, wrap it in a safe function, and move every
  decision taken from the result into plain code with a unit test.
- Each block must be verified: by a test that runs the real call (read-only, skipped when the
  `CI` variable is set), or, when it changes the machine, by a `tandem-cli` command run on a
  device nobody hears. docs/ARCHITECTURE.md lists which is which.

**Settings file**

- `%APPDATA%\com.vdouay.tandem\settings.json`, saved atomically. New fields need a default so
  older files still load. If the meaning of an existing field changes, bump `SETTINGS_VERSION`
  in `src-tauri/src/settings.rs` and migrate in `Settings::load`.
- An unreadable file is copied to `settings.unreadable.json` before defaults replace it.
- `guide_step` is where the first-run guide is: a step number while it is pending, `null` once
  finished or skipped. A file from before the guide existed has no such field: it is read as
  finished if it has outputs and as pending if it has none. A recorded step is kept even once
  outputs exist, so the guide resumes after a restart. Closing the window with an output set
  up finishes it.
- Never delete, move or overwrite the maintainer's settings file to get a fresh state. Set
  the `TANDEM_CONFIG_DIR` environment variable to an empty folder: the app then keeps its
  settings there. Only one Tandem can run at a time, so theirs has to be quit first, by them.

## The icon

The mark is three music notes wrapped round a centre point; each note's flag sweeps a third of
the ring. It exists in three places that must stay in step:

1. `src/assets/tandem-mark.svg`: the logo used in the app and the README.
2. `src-tauri/icons/source/app-icon.svg`: the same mark on a dark tile. After changing it run
   `npm run icons`, then delete the `src-tauri/icons/android` and `src-tauri/icons/ios` folders
   the tool also creates.
3. `draw_icon` in `src-tauri/src/tray.rs`: the tray icon, drawn in code from the same geometry,
   slightly bolder, grey when nothing plays. To look at it, run
   `cargo test --release -p tandem tray_icon_preview -- --ignored`, which writes
   `tandem-tray-*.ppm` files to the temp directory.

A fourth drawing follows the mark without being it: the busy indicator (`SyncSpinner` in
`src/components/common/Layout.tsx`) draws the same note three times from the same path data
and turns it a third of the way at a time. If the mark's geometry ever changes, change it
there too.

## Verifying a change

Automated checks do not cover real audio hardware, so say what was and was not verified.

- **Lint, types and tests:** `npm run lint`, `npm run build`, `cargo fmt --all --check`, then
  `cargo test --release`. CI runs the same four. Clippy is not a gate yet: it reports a handful
  of style lints in `engine/src/calibrate.rs`, `engine/src/engine.rs` and
  `src-tauri/src/supervisor.rs` that have to be settled first.
- **UI in the browser preview:** `npm run dev`, open <http://localhost:1420>. The simulated
  backend has a Bluetooth source and two outputs. Useful switches: `?lang=fr` for French,
  `?theme=dark` or `?theme=light` to force a theme (the default follows the system),
  `?update=1.2.3` to simulate an available update, `?update=fail` for a failed update check.
  Any state can be opened directly: `?sync=<state>` for auto-sync (each step of a running
  pass, each ending, each failure, each reason the Run button is unavailable),
  `?outputs=<state>` for the Outputs screen, `?view=settings&autostart=fail`, and `?autorun=1`
  to start a simulated run by itself. `?firstRun=1` is a new installation (no output, the
  guide at its first step), `?firstRun=noDevices` one with no other playback device,
  `?firstRun=noMic` the guide on a machine without a microphone, and `?guide=<0..4>` opens the
  guide at a step. The names are listed at the top of `src/lib/mock.ts`;
  add a name there whenever a new state is added to the UI.
  The demo build (`npm run build:demo`) takes the same switches.
  In the preview, commit typed values with real key presses (type, then Enter).
- **Screenshots of every state:** `npm run build:demo`, then `node scripts/screenshots.mjs`.
  It drives the installed Microsoft Edge in headless mode (no dependency) through every state,
  in both languages and themes, at 1040×720 and 820×560, into `screens/` (git-ignored), with
  one contact sheet per combination. `node scripts/screenshots.mjs --capture` re-records the
  README's `docs/images/auto-sync.gif` and `.mp4` the same way; it needs `ffmpeg` on the PATH.
  Look at the result: the script only takes the pictures.
- **Engine on real streams, silently:** use two playback devices nobody hears, for example
  the virtual ones that a remote-play or virtual-cable driver installs. `tandem-cli list` shows
  what the machine has; ask the maintainer which ones are safe before picking. For example:

  ```bash
  cargo run --release -p tandem-engine --bin tandem-cli -- run --source "<silent device A>" --target "<silent device B>" --delay 0 --duration 6
  ```

  Expect "playing" and 0 underruns. Three more commands cover what auto-sync adds:

  - `tandem-cli probe --source "<A>" --target "<B>"` plays two test sweeps on B alone, cut
    off from A, as "Line up with" does. Expect the spacing counted in frames to match the
    one asked for, 0 discontinuities, 0 dropouts, a loudest level above zero on B, and
    0.00000 captured on A.
  - `tandem-cli sessions "<device>"` lists the programs playing on a device and says which
    ones a measurement would mute. It changes nothing.
  - `tandem-cli quiet-probe "<device>"` mutes those programs, waits, and unmutes them. It
    writes to the Windows mixer and keeps no record: use it only on a device nobody hears,
    with a stand-in program (a second `tandem-cli run` on that device), and check with
    `sessions` first that the stand-in is the only one it would mute.
- **The real app as a new installation would see it:** quit any running Tandem (the
  maintainer's is theirs to quit), then start it with an empty settings folder:

  ```bash
  TANDEM_CONFIG_DIR="$(mktemp -d)" npm run tauri dev
  ```

  It opens on the guide. Outputs added there play on real devices, so say so first.
- **Everything else** (Bluetooth, a real microphone, disconnects, sleep, the tray on other
  taskbar positions): docs/hardware-test.md, run by the maintainer.

## What has been built, and what is still open

Done, in order: the engine and CLI; the desktop app with tray, installer, English and French;
0 ms delays and sample-rate display; an audit that fixed twenty findings (thread priority,
channel mixing, volume following, pause when silent, reference-output auto-sync, non-blocking
disconnects, settings backup, undo, translated errors, tray placement, CI and releases); the
three-note icon; in-app updates (1.0.1); the first published release (1.0.2); auto-sync that
keeps "Line up with" sweeps off the source and silences other programs for a measurement
started by hand (branch `sync-quiet`, not merged); a drawing for every auto-sync state, the
busy indicator redrawn from the mark, causes shown on device errors (branch `sync-feedback`,
on top of `sync-quiet`, not merged); a first-run guide in five steps (branch `first-run`, on
top of `sync-feedback`, not merged).

Known gaps:

- Volume following on a hardware-volume device and "pause when nothing plays" have not been
  confirmed on real hardware. Sync by ear and microphone auto-sync have, and work well
  (November 2026: auto-sync landed 4 ms from a delay set by ear). Since then, not checked on
  hardware (docs/hardware-test.md, steps 13 to 18): lining an output up with another output
  ("Line up with"), whose sweeps now play on the outputs only; other programs being muted for
  a measurement started by hand and unmuted afterwards, including after Tandem is killed;
  automatic re-sync waiting for silence. Steps 11 and 12 must be run again too, to confirm
  the source mode still behaves as it did.
- The first-run guide has been driven on the real app by a script only (November 2026: a
  copy with its own identifier and an empty settings folder, hidden, one output nobody hears,
  no click and no sweep). Nobody has gone through it by hand, with sound, or with the source
  switched off on the way: docs/hardware-test.md, steps 24 to 28.
- The installer is not code-signed, so SmartScreen warns on first run. That needs a paid
  certificate.
- An in-app update has not yet been confirmed end to end. 1.0.3 is the first release that an
  installed copy (1.0.2) is offered, so it is the first chance to do it: docs/RELEASING.md,
  checklist step 10, with the result noted in its History table.
- The update card's states that need a click (check failed, downloading, install failed) and
  the tray popup's update line are not in the screenshot script.
- `IAudioClient3` low-latency shared mode could shrink the minimum lag; untested.
