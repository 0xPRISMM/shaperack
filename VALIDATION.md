# ShapeRack validation

ShapeRack 0.1.1 adds an unsigned Windows x64 setup EXE and retains a ZIP installation option. This remains an experimental release.

## Version 0.1.1 installer verification

Checked on Windows x64 on 2026-10-11 using Inno Setup 6.7.3 and Windows PowerShell 5.1:

- The production EXE compiles without warnings and exposes ShapeRack 0.1.1 installer metadata. It requires administrator rights and targets the standard 64-bit Common Files VST3 directory, with optional CLAP and a Windows uninstaller.
- `scripts/validate-installer.ps1` compiles a staging variant of the same installer source. Its paths are fixed to a workspace directory, it requires no elevation, and it creates no Windows Installed apps entry.
- Default install places VST3 only. Full install places VST3 and CLAP. Installed plugin, documentation, and license files match source SHA-256 hashes.
- Both installed VST3 copies load and expose the expected factory. The installed CLAP copy passes real audio processing, automation timing, transport, bypass, and state checks.
- Both uninstall paths remove their owned files. An unrelated file deliberately placed in the documentation directory survives removal.
- The signed Inno Setup compiler download was verified before execution. The resulting ShapeRack installer is unsigned.

These checks exercise isolated installation and uninstallation. A privileged install into the real system Common Files directories has not been performed. Run `scripts/validate-installer.ps1 -InnoCompiler 'C:\path\to\ISCC.exe'` to repeat the checks; use a new `-TestRoot` if a previous fixture remains.

## Completed plugin checks

Checked on Windows x64 on 2026-10-11 using Rust 1.99.0. Tests, formatting, lint, release builds, CLAP integration, and VST3 loading were rerun for version 0.1.1.

- `cargo test --locked --lib`: **29 tests passed**. Covers curve endpoints/interpolation, malformed DSP values, timing across sample rates, stereo offset, smoothing, filter response, exact dry bypass/mix, mono/stereo behavior, host block-size invariance, transport seeks, stopped/free timing, parameter ID uniqueness, persisted editor size, and three editor gesture lifecycle tests.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: ShapeRack and its preview pass. The included upstream framework still emits existing compatibility and unused-import warnings when compiled as a dependency.
- `cargo fmt --package shaperack --package xtask --check`: passes.
- `cargo xtask bundle shaperack --release --locked`: builds Windows x64 VST3 and CLAP bundles.
- Actual release CLAP library, through `scripts/validate_clap.py`: initializes, exposes 89 parameters and host bypass metadata, processes stereo pumping, handles buffers of 1/17/64/127/512 samples at 44.1/48/96 kHz and 83/128/173 BPM, follows transport seeks, and preserves sample timing of first and later automation events. Mix-zero and bypass settle to dry audio. Partial stream read/write saves and restores all 89 parameters and an edited curve (2749-byte state).
- Actual VST3 library, through `scripts/validate_vst3.py`: module initialization and factory metadata expose ShapeRack by PRISMM as an audio effect.
- The version 0.1.0 native standalone editor started successfully using the dummy audio backend and OpenGL; its default 960×640 layout was visually inspected. The editor code is unchanged in 0.1.1. A screenshot is in `docs/ShapeRack-editor.png`. Simulated real egui pointer events verify slider click/drag/reset automation ordering; closing during a curve or slider drag ends the gesture.

## Remaining host checks

FL Studio project playback, saving/reopening projects in FL Studio, the full Steinberg VST3 validator, minimum-size/high-DPI editor rendering, and other operating systems have not been tested. The CLAP harness is a small test host, and the VST3 script checks loading and metadata rather than full VST3 audio processing.

The first-release scope and DSP limitations are described in README.md. The six-line framework fix for CLAP automation is documented in vendor/README.md and guarded by the real-binary harness.
