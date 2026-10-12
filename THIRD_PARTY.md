# Third-party components

- NIH-plug, nih_plug_egui, nih_plug_xtask: ISC, Robbert van der Helm and contributors. Source snapshot from revision `de421011f41a6d10fc8c7a6084e4f4dee0143683`, with the CLAP timestamp fix documented in `vendor/README.md`.
- vst3-sys bindings used by that NIH-plug revision: GPLv3. ShapeRack is distributed under GPL-3.0-or-later to accommodate these bindings.
- egui and epaint: MIT OR Apache-2.0, egui contributors.
- egui-baseview and baseview: MIT, respective contributors.
- clap-sys: MIT/Apache-2.0, contributors. CLAP ABI: MIT, CLAP contributors.
- [Serde](https://github.com/serde-rs/serde) and [serde_json](https://github.com/serde-rs/json): MIT OR Apache-2.0, Serde contributors; used for portable rack preset files.
- [rfd](https://github.com/PolyMeilex/rfd): MIT, Bartłomiej Maryńczak and contributors; used for native Windows preset file dialogs. See its [license](https://github.com/PolyMeilex/rfd/blob/master/LICENSE).

Full dependency versions are in Cargo.lock. Bundled notices for the plugin framework are in `licenses/`; the release package includes those notices. Other dependency licenses can be inspected with `cargo tree` and the linked upstream projects. Source for the binary release is available at the corresponding version tag in this repository.

## Windows installer

The v0.2.2 setup EXE uses [Inno Setup 6](https://jrsoftware.org/isinfo.php), by Jordan Russell and Martijn Laan. Its installer engine is distributed under the [Inno Setup License](https://jrsoftware.org/files/is/license.txt). ShapeRack's plugin source remains GPL-3.0-or-later.

Inno Setup is required to rebuild the setup EXE with `scripts/build-installer.ps1`, after preparing the plugin package with `scripts/package.ps1`. Users do not need to install Inno Setup to run the resulting installer or use ShapeRack.

## DSP references

ShapeRack's two-pole low-pass implementation follows the trapezoidal state-variable filter equations described by Andrew Simper in [Linear Trapezoidal Integrated State Variable Filter](https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf). Cytomic publishes this technical knowledge in the public domain on its [technical papers page](https://cytomic.com/technical-papers/).

The first-order tanh antiderivative antialiasing implementation uses the analytical divided-difference method and near-equal-input fallback described in equations 6–10 of [Simplifying Antiderivative Antialiasing with Lookup Table Integration](https://dafx.de/paper-archive/2025/DAFx25_paper_30.pdf), by Leonardo Gabrielli and Stefano Squartini (DAFx 2025). ShapeRack evaluates the analytical formula directly; it does not use the paper's lookup-table implementation.
