<div align="center">
  <img src="src/assets/tandem-mark.svg" alt="" width="96" />

# Tandem

**Play your PC's audio on several outputs at once, in sync.**

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](./LICENSE)
![Platform: Windows](https://img.shields.io/badge/platform-Windows_x64-informational)
</div>

Tandem takes what Windows plays on one device (the **source**, typically the slowest one,
like a Bluetooth amplifier) and mirrors it to any number of other outputs: a USB
interface, HDMI speakers, headphones. Each output is **delayed** so everything comes out at the
same time, and **clock drift** between devices is corrected continuously, so they stay in sync
for hours, not minutes.

No virtual audio cable or driver is needed.

## Features

- **Any number of outputs**, each with its own delay, volume, on/off switch and live level meter.
- **Delays remembered per source.** The right delay for an output depends on which device is
  the source, so each pairing keeps its own value.
- **Follows the Windows default output**, or a pinned device.
- **Survives disconnects.** When the Bluetooth amp drops and comes back, or an output is
  unplugged and replugged, Tandem picks it up again on its own, even if Windows gave it a new id.
- **Sync by ear:** calibration clicks plus a delay control with 1 ms and 10 ms steps.
- **Sync with a microphone:** two sweeps, deconvolution and a verification pass. The
  microphone's own latency cancels out. Optionally re-syncs automatically after a reconnect.
- **Tray icon** with a status popup and a quick on/off switch. Close-to-tray, launch at login,
  start hidden.
- English and French, light and dark themes (follows the system by default).

## How it works

```
 Windows apps ──► Source device (default output, e.g. Bluetooth amp) ──► speakers (slow)
                         │
                         │ WASAPI loopback capture (before the slow link)
                         ▼
                  ┌── FIFO ──► async sinc resampler (PI drift control) ──► Output A (delayed)
   capture ───────┼── FIFO ──► async sinc resampler (PI drift control) ──► Output B (delayed)
                  └── …
```

- **Loopback capture** reads the source's mix as it is handed to the device, ahead of the
  Bluetooth link's latency. Every other output gets a copy delayed by the difference.
- **Keepalive**: Tandem plays inaudible silence on the source, so loopback never stalls during
  silence and the Bluetooth link stays awake with a stable latency.
- **Drift control**: two devices never run at exactly the same rate (typically tens of ppm,
  a few ms per minute). A PI controller watches how much audio is buffered per output and
  nudges its resampling ratio by a few ppm. That's far below anything audible, and the delay
  stays put.
- **Microphone auto-sync** (`engine/src/calibrate.rs`): with every output muted, sweep #1 plays
  on the source; then the output being synced is unmuted and sweep #2 plays. A regularized
  inverse filter turns each arrival into a sharp peak. The source is the peak that repeats
  exactly one spacing later; subtracting its pattern leaves the output's arrival. The gap
  between them is the correction.

## Install

Run `Tandem_1.0.0_x64-setup.exe` from `target/release/bundle/nsis/`. It installs for the current
user (no admin rights) and adds a Start menu entry and an uninstaller.

**Recommended setup:** make the slowest device (the Bluetooth amp) the Windows default output,
leave the source on "Windows default output", and add the other devices as outputs.

## Develop

Requirements: Rust (MSVC toolchain), Node 20+, and WebView2 (built into Windows 11).

```bash
npm install
npm run tauri dev      # run the app with hot reload
npm run dev            # UI only, in a browser, with a simulated backend
npm run release        # build the release app and the NSIS installer
npm run licenses       # regenerate THIRD_PARTY_LICENSES.md after changing dependencies
cargo test --release   # engine, calibration and settings tests
```

The headless engine is also available as a command-line tool for testing:

```bash
cargo run --release -p tandem-engine --bin tandem-cli -- list
cargo run --release -p tandem-engine --bin tandem-cli -- run --target "Main Output" --delay 200
```

### Layout

| Path | What |
|---|---|
| `engine/` | The audio engine library: devices, capture, per-output render, drift controller, microphone recording, calibration DSP. Plus `tandem-cli`. |
| `src-tauri/` | The desktop app: supervisor (device reconciliation), auto-sync orchestration, settings, tray, commands. |
| `src/` | The UI (React + CSS modules). `lib/mock.ts` serves the browser preview. |

## License

Tandem is free software: you can redistribute it and/or modify it under the terms of the
**GNU Affero General Public License, version 3 or (at your option) any later version**. See
[LICENSE](LICENSE) for the full text and [NOTICE](NOTICE) for attributions.

The design language (tokens, layout, controls) is adapted from
[Plisto](https://github.com/Altervayne/plisto) by Florian Douay, also AGPL-3.0-or-later. Tandem
does not use the Plisto name or logo, which are reserved by their author.

Third-party components keep their own (permissive, AGPL-compatible) licenses. See
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). The installer shows the license before
installing and places `LICENSE.txt`, `NOTICE.txt` and `THIRD_PARTY_LICENSES.md` next to the app.
