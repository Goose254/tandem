<div align="center">
  <img src="src/assets/tandem-mark.svg" alt="" width="96" />

# Tandem

**Play your PC's audio on several outputs at once, in sync.**

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](./LICENSE)
![Platform: Windows](https://img.shields.io/badge/platform-Windows_x64-informational)

[Source code](https://github.com/Goose254/tandem) · [Report an issue](https://github.com/Goose254/tandem/issues)
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
  An output can be lined up with another output instead of the source, for a source nobody
  listens to.
- **Volume keys act on every output**, including when the source applies the Windows volume
  itself (most Bluetooth devices), which loopback capture cannot see.
- **Surround and mono are mixed, not cut:** a 5.1 or 7.1 source keeps its centre and surround
  channels on stereo speakers.
- **Optional pause when nothing plays**, so Windows can go to sleep.
- **Tray icon** with a status popup and a quick on/off switch. Close-to-tray, launch at login,
  start hidden.
- **Updates itself** from the GitHub releases, when you say so.
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
- **Priority**: the audio threads run in Windows' "Pro Audio" scheduling class, so a busy CPU
  does not cause dropouts.
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

Download `Tandem_<version>_x64-setup.exe` from the
[Releases page](https://github.com/Goose254/tandem/releases) and run it. It installs for the
current user (no admin rights) and adds a Start menu entry and an uninstaller. Windows 10 or 11,
64-bit.

The installer is not code-signed, so Windows SmartScreen may show "Windows protected your PC"
the first time: choose "More info", then "Run anyway". To avoid trusting a download, build it
yourself (see [Develop](#develop)); the result lands in `target/release/bundle/nsis/`.

**Updates** arrive inside the app. Tandem looks at this repository's latest published release
shortly after it starts and a few times a day; when there is a newer version, a card in the
window (and a line in the tray popup) offers "Update now", "Later" or "Skip this version".
Updating downloads the signed installer, runs it and restarts Tandem; the sound stops for a few
seconds and your settings are kept. Nothing is installed without your go-ahead, the automatic
check can be turned off in Settings > Updates, and "Check for updates" is always there. Each
download is verified against a signature before it runs. Version 1.0.0 has no updater: install
1.0.1 or later by hand once.

**Recommended setup:** make the slowest device (the Bluetooth amp) the Windows default output,
leave the source on "Windows default output", and add the other devices as outputs.

**Delays** are added on top of each output's minimum path (its device buffer, the resampler and
one loopback packet), so **0 ms means "as early as this output can play"**. The app shows that
minimum lag per output. If an output still sounds late at 0 ms, the source is too fast for it.
Tandem can only delay outputs, never play them earlier, so pick a slower source.

**Several outputs of one interface** (e.g. Main Output and Output 3/4): make the source a device
you don't listen to (an unused output) and add every speaker as an output. They then all go
through the same path and line up at 0 ms.

**Syncing those with a microphone:** the microphone cannot hear a source nobody listens to. In
Sync, set "Line up with" to one of the outputs: it plays the first sweep in the source's place.
If the output being synced is still behind it at 0 ms, the reference is delayed instead.

**Volume:** Windows' software volume is part of what loopback captures, so the copies follow it
on their own. A device that applies the volume itself (most Bluetooth devices) hides it from
loopback; with "Volume keys act on every output" on (the default), Tandem reads the source's
volume and mute and applies them to every output.

**Sleep:** open audio streams keep Windows awake. Turn on "Pause when nothing plays" to have
Tandem close its streams after two minutes of silence and reopen them at the first sound. The
first sound reaches the outputs a moment late, and a short notification may play on the source
only, so this is off by default.

**Sample rates:** every output is resampled from the source rate anyway, to absorb clock drift.
Still, give all devices the same rate in Windows (Sound settings > device > Properties >
Advanced) so that resampling stays a tiny drift correction. Tandem flags any output whose rate
differs from the source.

## What Tandem does not do

- **Apps that bypass the source are not mirrored.** Tandem copies what plays on the source
  device. An app set to another device in Windows' volume mixer, or one that takes a device in
  exclusive mode (some players' "WASAPI exclusive" or ASIO output), never reaches the copies.
  While an app holds an output in exclusive mode, Tandem cannot open that output and retries
  until it is released.
- **It cannot make a device play earlier.** Outputs are only ever delayed to wait for the
  source. This is why the slowest device has to be the source.
- **It does not fix lip-sync.** With a Bluetooth source, sound arrives late compared with the
  picture, on every output alike. Use the player's audio-delay setting for that (VLC, mpv and
  most players have one).
- **Protected audio** that Windows refuses to hand to loopback capture stays on the source only.

The real-hardware checks that automated tests cannot cover are listed in
[docs/hardware-test.md](docs/hardware-test.md).

## Develop

Requirements: Rust (MSVC toolchain), Node 20+, and WebView2 (built into Windows 11).

```bash
git clone https://github.com/Goose254/tandem.git
cd tandem
npm install
npm run tauri dev      # run the app with hot reload
npm run dev            # UI only, in a browser, with a simulated backend
npm run release        # build the signed release app, installer and update files (needs the key)
npm run release:unsigned  # the same without update signing: for local builds and forks
npm run licenses       # regenerate THIRD_PARTY_LICENSES.md after changing dependencies
cargo test --release   # engine, calibration and settings tests
```

Continuous integration (`.github/workflows/ci.yml`) runs the type-check, the UI build and the
tests on every push.

### Releases

In-app updates are signed. The public key is in `src-tauri/tauri.conf.json`; the private key
never goes in the repository. It lives in the maintainer's `~/.tauri/tandem-updater.key` and in
the repository secret `TAURI_SIGNING_PRIVATE_KEY` (plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if
the key has a password). Keep a backup of that file: without it no installed copy can be
updated any more. To build a signed release locally, point the same variable at the key file
before `npm run release`.

To publish a release:

1. Set the new version in `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`
   and `engine/Cargo.toml`, and commit.
2. Push a tag with that version, for example `git tag v1.0.1` then `git push origin v1.0.1`.
3. `.github/workflows/release.yml` builds and signs the installer and attaches it, its signature
   and `latest.json` to a draft release.
4. Write the release notes on the draft (the update card shows their first lines) and publish
   it. Installed copies find it on their next check.

A fork that wants its own updates needs its own key (`npm run tauri signer generate`), its
public key and release URL in `src-tauri/tauri.conf.json`.

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
[LICENSE](LICENSE) for the full text and [NOTICE](NOTICE) for attributions. The complete source code is at
<https://github.com/Goose254/tandem>.

The design language (tokens, layout, controls) is adapted from
[Plisto](https://github.com/Altervayne/plisto) by Florian Douay, also AGPL-3.0-or-later. Tandem
does not use the Plisto name or logo, which are reserved by their author.

Third-party components keep their own (permissive, AGPL-compatible) licenses. See
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). The installer shows the license before
installing and places `LICENSE.txt`, `NOTICE.txt` and `THIRD_PARTY_LICENSES.md` next to the app.
