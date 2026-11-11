# Working on Tandem

Notes for anyone (person or coding agent) picking this project up cold. The README explains
what Tandem is for users; this file explains how to work on it without breaking things, and
records the decisions that are not visible in the code.

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
| `src/` | The UI: React 19, TypeScript, CSS modules, zustand. `lib/mock.ts` is a simulated backend for the browser preview. |
| `docs/` | Release runbook, release notes for the next version, hardware checklist. |
| `.github/workflows/` | `ci.yml` (pull requests and pushes to `main`) and `release.yml` (on a `v*` tag). |

Key files:

- `engine/src/engine.rs`: the audio threads. `keepalive_loop` plays silence on the source (and
  clicks or test sweeps), `capture_loop` reads loopback, `render_loop` runs per output.
- `engine/src/control.rs`: the PI drift controller. `engine/src/mix.rs`: channel mixing.
  `engine/src/sys.rs`: "Pro Audio" thread priority and the endpoint volume.
  `engine/src/calibrate.rs`: sweep generation and analysis for microphone auto-sync.
- `src-tauri/src/supervisor.rs`: a loop that reconciles the running engine with the settings and
  the connected devices, and publishes status to the UI.
- `src-tauri/src/calibration.rs`: the auto-sync procedure. `settings.rs`: the settings file.
  `tray.rs`: tray menu, popup placement, and the tray icon drawn in code.
- `src/state/store.ts`: app state. `src/state/updater.ts` and `src/lib/updater.ts`: updates.

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
  [Plisto](https://github.com/Altervayne/plisto) by the maintainer's brother, also AGPL. The
  Plisto name and logo are reserved: never use them in Tandem.
- New dependencies must be AGPL-compatible; run `npm run licenses` after adding one.

**Design**

- Design tokens live in `src/styles/tokens.css`. One solid accent button per screen, quiet
  controls elsewhere, no boxes around settings rows.
- Every user-facing string exists in both `src/i18n/en.ts` and `src/i18n/fr.ts` with the same
  keys. The tray menu and tooltip strings are in `src-tauri/src/tray.rs`.

**Audio behaviour worth knowing before changing it**

- Delays are added on top of each output's minimum path, so 0 ms means "as early as this output
  can play". Outputs can only be delayed, never advanced: the slowest device must be the source.
- Loopback capture already contains Windows' *software* volume (measured: a device at 90% came
  through 1.6 dB quieter). A *hardware* volume (most Bluetooth devices) is invisible to
  loopback, so the engine applies it to the outputs itself when "Volume keys act on every
  output" is on. Do not apply a software volume a second time.
- The supervisor must never wait for audio threads while holding the runtime lock: the UI's
  commands share that lock. Things going away are told to stop under the lock and dropped after
  it (`Retired` in `supervisor.rs`).
- Engine and settings change together under the runtime lock (`apply_delay`,
  `set_output_gain`), and the supervisor reads the settings under the same lock. Lock order is
  always runtime, then settings.
- Microphone auto-sync mutes every output, plays one sweep from the source alone, then one with
  the output being synced. With a "Line up with" output chosen, that output stands in for the
  source.
- "Pause when nothing plays" closes every stream after two minutes of silence and watches the
  source's level meter to wake up. It is off by default because the first sound after a pause
  reaches the outputs late.

**Settings file**

- `%APPDATA%\com.vdouay.tandem\settings.json`, saved atomically. New fields need a default so
  older files still load. If the meaning of an existing field changes, bump `SETTINGS_VERSION`
  in `src-tauri/src/settings.rs` and migrate in `Settings::load`.
- An unreadable file is copied to `settings.unreadable.json` before defaults replace it.

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

The busy spinner (`SyncSpinner` in `src/components/common/Layout.tsx`) still animates the three
bars of the previous logo.

## Verifying a change

Automated checks do not cover real audio hardware, so say what was and was not verified.

- **Lint, types and tests:** `npm run lint`, `npm run build`, `cargo fmt --all --check`, then
  `cargo test --release`. CI runs the same four. Clippy is not a gate yet: it reports a handful
  of style lints in `engine/src/calibrate.rs`, `engine/src/engine.rs` and
  `src-tauri/src/supervisor.rs` that have to be settled first.
- **UI in the browser preview:** `npm run dev`, open <http://localhost:1420>. The simulated
  backend has a Bluetooth source and two outputs. Useful switches: `?lang=fr` for French,
  `?update=1.2.3` to simulate an available update, `?update=fail` for a failed update check.
  In the preview, commit typed values with real key presses (type, then Enter).
- **Engine on real streams, silently:** this machine has virtual devices nobody hears
  ("Steam Streaming Speakers", "Steam Streaming Microphone"). For example:

  ```bash
  cargo run --release -p tandem-engine --bin tandem-cli -- run --source "Steam Streaming Speakers" --target "Steam Streaming Microphone" --delay 0 --duration 6
  ```

  Expect "playing" and 0 underruns. `tandem-cli list` shows the devices.
- **Everything else** (Bluetooth, a real microphone, disconnects, sleep, the tray on other
  taskbar positions): docs/hardware-test.md, run by the maintainer.

## What has been built, and what is still open

Done, in order: the engine and CLI; the desktop app with tray, installer, English and French;
0 ms delays and sample-rate display; an audit that fixed twenty findings (thread priority,
channel mixing, volume following, pause when silent, reference-output auto-sync, non-blocking
disconnects, settings backup, undo, translated errors, tray placement, CI and releases); the
three-note icon; in-app updates (1.0.1); the first published release (1.0.2).

Known gaps:

- Microphone auto-sync, volume following on a hardware-volume device and "pause when nothing
  plays" have not been confirmed on real hardware. Sync by ear has, and works well.
- The installer is not code-signed, so SmartScreen warns on first run. That needs a paid
  certificate.
- An in-app update has not yet been exercised end to end from one published release to the
  next.
- The busy spinner still shows the old logo's bars.
- `IAudioClient3` low-latency shared mode could shrink the minimum lag; untested.
