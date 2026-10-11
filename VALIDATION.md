# ShapeRack validation

ShapeRack 0.2.1 passed the build, test, binary-host, and isolated installer checks below on Windows x64 on 2026-10-12 using Rust 1.99.0, Windows PowerShell 5.1, and Inno Setup 6.7.3. The rebuilt CLAP descriptor reports version 0.2.1 and vendor eM4; the VST3 factory also reports eM4. The separate FL Studio checks used the existing v0.2.0 VST3, before the v0.2.1 default-tool change. ShapeRack remains experimental.

## Audio, curves, and state

- `cargo test --locked --lib`: **62 tests passed**. Covers interpolation and segment bends, malformed inputs, filter response/stability, drive antialiasing, parameter identity, dry bypass/mix, mono/stereo behavior, smoothing, sample-rate/tempo changes, seeks, and block-size independence.
- Stopped Sync regressions reproduce a fixed host beat position with Volume enabled. Short notes after silent processing remain audible at 44.1/48/96 kHz. The preview clock continues through silence and summed stereo sources, ignores stale stopped positions, and is independent of host buffer boundaries. Playing transport resumes at the supplied beat on the first sample, including negative preroll and loop seeks.
- `scripts/validate_clap.py` exercises the actual release CLAP library: initialization, all **153 stable parameter IDs**, 64 segment bends, stereo pumping, stopped Sync note previews, two source signals summed into the main bus, buffer sizes 1/17/64/127/512, 44.1/48/96 kHz, 83/128/173 BPM, playback/seek alignment, timestamped automation, dry bypass/mix, and partial-stream state saving/loading. All 153 parameters survive a **5179-byte** state roundtrip. A fresh instance loads v0.1.1-style state with neutral bends and preserved existing controls.
- `scripts/validate_vst3.py` checks the actual Windows VST3 module, factory, component/controller/processor interfaces, stereo bus negotiation, all 153 parameters, and audio processing on a dedicated worker. It verifies stopped Sync notes after silence, fixed/moving stopped positions, variable buffers, summed sources, and exact first-sample playback alignment at positive and negative beat positions.
- The filter's cutoff response is checked against a two-pole Butterworth reference at 8/44.1/48/96/192 kHz. Drive tests check channel isolation, malformed values, changing drive amounts, and folded-harmonic reduction for a coherent high-frequency tone. These are targeted DSP checks, not a claim of inaudible aliasing under every input.

## Presets and editor

- Eight file-format tests cover complete parameter roundtrips, malformed/oversized files, missing and unknown parameters, finite normalized values, metadata, explicit overwrite, and failure cleanup. Four preset integration tests cover full-rack state construction, preserved editor state, invalid recall, factory presets, and safe filenames.
- Seven editor tests use real egui pointer events to exercise Nodes, Line, Bend, snapping, host gesture ordering, closing mid-drag, and layout bounds at 820x620 and 960x640.
- The native standalone editor was visually inspected on Windows using the dummy audio backend and OpenGL. Factory recall, library saving, native export, and native import/cancellation were checked. Native file dialogs run on a separate worker so their Windows message loop does not reenter the editor's borrowed window handler. Host state changes stay on the GUI thread.
- Module buttons and tabs display explicit ON/OFF state. The header distinguishes SYNC PREVIEW, SYNC PLAYING, and FREE RUNNING. `docs/ShapeRack-editor.png` shows the updated native editor.
- The rebuilt v0.2.1 native standalone editor was opened with the dummy backend and visually confirmed to start in Bend. The existing Bend pointer-event test also starts from the default editor state. Nodes, Draw, and Line remain in the tool dropdown.

## FL Studio 2025 checks

Manual checks on Windows used the v0.2.0 VST3 at 130 BPM with a six-note synth pattern. Evidence was visible activity in ShapeRack's output meter, FL Studio's meters, and waveform displays. No speaker listening or exported-audio analysis was performed.

- Pattern playback showed **SYNC PLAYING** and nonzero plugin and host meters.
- With transport stopped and Volume enabled, **SYNC PREVIEW** with the factory Quarter-note pump produced nonzero output from piano-roll note previews (observed output about -43.5 dB). Stopped **Free** previews also produced output (about -34.1 dB).
- The factory Slow filter sweep, Stereo sway, and Warm rhythmic drive racks all produced nonzero output during pattern playback. The last enables Volume and Drive together.
- A kick source was temporarily routed into the same effect insert as the synth. Kick previews were checked during pattern playback and while stopped in Sync with Volume and Drive enabled (observed output about -28.6 dB). Its original routing was restored and no pattern notes were changed.
- Native Export wrote a portable JSON preset containing all 153 audio parameters. Native Import restored the original custom rack, including its segment bends. The original rack was restored and playback resumed afterward.

The private project, custom preset contents, and session screenshots are not included in the repository. These observations establish signal flow and UI/preset operation for the tested setup; they do not measure perceived sound quality or cover every routing configuration.

## Builds and installer

- `cargo fmt --package shaperack --package xtask --check` and `cargo clippy --locked --all-targets --all-features -- -D warnings` pass for ShapeRack. Included upstream framework dependencies emit existing compatibility/unused-import warnings.
- `cargo xtask bundle shaperack --release --locked` builds Windows x64 VST3 and CLAP bundles. The production setup EXE and manual-install ZIP are built from these bundles.
- `scripts/validate-installer.ps1` compiles the same installer source into an isolated, nonprivileged staging variant. Default installation places VST3 only; full installation places VST3 and CLAP. Installed plugins, documents, and licenses match source SHA-256 hashes. Installed plugins pass the binary checks. Both uninstall paths remove owned files while preserving an unrelated test file.
- The production installer targets the standard 64-bit Common Files plugin directories and provides a Windows uninstaller. The signed Inno Setup compiler was verified before use; ShapeRack's installer is unsigned. A privileged install into the real system plugin directories has not been performed.

Repeat staging validation with `scripts/validate-installer.ps1 -InnoCompiler 'C:\path\to\ISCC.exe'`; use a new `-TestRoot` if an earlier fixture remains.

## Remaining host checks

Automated tests cover the stopped Sync timing cause and routed stereo input through both exported plugin formats, and the FL Studio checks above cover playback and stopped previews in one real project. Speaker listening, exported-audio analysis, saving/reopening projects inside FL Studio, the full Steinberg validator, high-DPI rendering, and other operating systems remain untested. The v0.2.1 release binary has not yet been checked in FL Studio. The dummy editor backend produces no audio; CLAP and VST3 integration use small headless test hosts. These checks cannot certify every DAW configuration.

README.md records the fixed-grid editor, shared clock, fixed module order, and DSP limitations. `vendor/README.md` documents the six-line CLAP automation timing fix in the bundled framework.
