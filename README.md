# ShapeRack
Check [Releases](https://github.com/0xPRISMM/shaperack/releases) for ZIP file (EXE download coming tomorrow + quality improvements and UI)

An original Rust modulation rack for **Windows x64 VST3 and CLAP** hosts. Draw a rhythm and use it to animate volume, a low-pass filter, stereo balance, or soft saturation. Designed for FL Studio mixer inserts.

ShapeRack is an independent project inspired by drawable modulation effects. It is not affiliated with Cableguys and does not use ShaperBox source code, artwork, or presets.

![ShapeRack native editor](docs/ShapeRack-editor.png)

## Features

- Four independently editable modules in a fixed chain: **Volume → Filter → Pan → Drive**.
- Seventeen editable points per module, with freehand drawing and node dragging.
- Linear, smoothstep, and step interpolation; Pump, Gate, Sine, Ramp, and Flat curve starters.
- Host tempo sync from 1/32 note to 16 beats; free-running rate from 0.05–20 Hz.
- Host seek/loop alignment, global phase shift, and right-channel phase offset.
- Adjustable de-click smoothing, per-module amount and enable controls, dry/wet mix, output gain, and smoothed bypass.
- Automatable curve points and controls, saved with the DAW project.
- Mono/stereo audio, zero reported latency, native resizable editor and live output meter.

## Install in FL Studio

Download the Windows ZIP from [Releases](https://github.com/0xPRISMM/shaperack/releases). Follow [INSTALL.md](INSTALL.md): copy the entire `ShapeRack.vst3` folder into `C:\Program Files\Common Files\VST3`, scan plugins, and load ShapeRack in a mixer effect slot.

## Quick start

1. Put ShapeRack on a sustained bass, pad, or synth mixer channel.
2. Leave Volume enabled, choose **Pump**, and use **Sync / 1/4** for a duck on every quarter-note beat.
3. Drag across the curve in Draw mode, or move individual vertical values in Nodes mode.
4. Enable Filter for a cutoff sweep, Pan for stereo balance movement, or Drive for changing saturation. Each tab has its own curve; all modules share the clock.
5. Adjust Amount, Mix, and De-click to taste. Pan has no effect on mono audio.

The curve resets each cycle, so a mismatched start/end value can click at zero De-click. Use smoothing for pump/gate shapes. In Sync mode modulation holds while transport is stopped if the host supplies a position. If timing is unavailable, ShapeRack falls back to 120 BPM and runs its own clock.

## Build from source

Install current stable Rust and the Windows Visual Studio C++ build tools, then:

```powershell
cargo test --locked --lib
cargo xtask bundle shaperack --release --locked
powershell -NoProfile -File scripts/package.ps1
```

The bundled VST3 and CLAP files are in `target/bundled/`; the ZIP is in `dist/`. The dependency lockfile and required NIH-plug sources are checked in. [Vendor notes](vendor/README.md) describe the small CLAP automation timing fix applied to the framework.

For a native editor preview without a DAW (dummy backend produces no audio):

```powershell
cargo run --locked --features standalone --bin shaperack-preview -- --backend dummy
```

GitHub Actions checks formatting, linting, Rust tests, and the real CLAP binary, then builds Windows packages. Version tags publish those packages as GitHub releases.

## First-release scope

Version 0.1 is an experimental first release. The filter is a smoothed first-order low-pass. Drive is normalized `tanh` saturation without oversampling, so strong drive can alias. Pan is stereo balance, not a spatial panner. Bar labels assume four quarter-note beats. Modules share one clock and their order is fixed. This version does not include multiband processing, audio/MIDI triggering, time warping, envelope followers, or preset files.

See [VALIDATION.md](VALIDATION.md) for checks actually performed and remaining host validation.

## License and credits

GPL-3.0-or-later; see [LICENSE](LICENSE). Uses [NIH-plug](https://github.com/robbert-vdh/nih-plug) (ISC), [egui](https://github.com/emilk/egui) (MIT/Apache-2.0), and the framework's GPLv3 VST3 bindings. See [THIRD_PARTY.md](THIRD_PARTY.md).
