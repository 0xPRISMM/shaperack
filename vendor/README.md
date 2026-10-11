# NIH-plug source snapshot

The `nih-plug` folder contains the required core, derive, egui adapter, and bundler crates from upstream revision `de421011f41a6d10fc8c7a6084e4f4dee0143683` of https://github.com/robbert-vdh/nih-plug, under its original ISC license.

Examples, unused UI adapters, and the upstream workspace definition are omitted. Dependency source code and credits are retained. Upstream transitive Git dependencies remain locked in the root Cargo.lock.

## Local fix

`src/wrapper/clap/wrapper.rs`, `handle_in_events_until`: inspect each event's timestamp before handling it, including the first pending event. The upstream one-event-lookahead implementation handled the first event before checking whether its timestamp required an audio-buffer split. A parameter event scheduled at sample 128 could therefore affect samples 0–127. The patch prevents that early mutation and resumes at the same event after processing the preceding sub-block.

`scripts/validate_clap.py` compares the pre-event samples with an unautomated reference, verifies later changes, and tests state roundtripping against the built plugin. This fix is included in ShapeRack's source and binary releases.
