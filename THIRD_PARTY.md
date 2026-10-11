# Third-party components

- NIH-plug, nih_plug_egui, nih_plug_xtask: ISC, Robbert van der Helm and contributors. Source snapshot from revision `de421011f41a6d10fc8c7a6084e4f4dee0143683`, with the CLAP timestamp fix documented in `vendor/README.md`.
- vst3-sys bindings used by that NIH-plug revision: GPLv3. ShapeRack is distributed under GPL-3.0-or-later to accommodate these bindings.
- egui and epaint: MIT OR Apache-2.0, egui contributors.
- egui-baseview and baseview: MIT, respective contributors.
- clap-sys: MIT/Apache-2.0, contributors. CLAP ABI: MIT, CLAP contributors.

Full dependency versions are in Cargo.lock. Bundled notices for the plugin framework are in `licenses/`; the release package includes those notices. Other dependency licenses can be inspected with `cargo tree` and the linked upstream projects. Source for the binary release is available at the corresponding version tag in this repository.
