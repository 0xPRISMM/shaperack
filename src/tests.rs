//! Host-independent integration tests using NIH-plug's real audio buffers.
use super::*;
use crate::dsp::{Curve, Preset};
use crate::params::{CurveStyle, ModuleParams};
use nih_plug_egui::EguiState;
use std::collections::HashSet;

fn float(value: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new("Test value", value, FloatRange::Linear { min, max })
}

fn apply_preset(module: &mut ModuleParams, preset: Preset) {
    let curve = Curve::from_preset(preset);
    module.style = EnumParam::new(
        "Test interpolation",
        match curve.interpolation {
            dsp::Interpolation::Linear => CurveStyle::Linear,
            dsp::Interpolation::Smooth => CurveStyle::Smooth,
            dsp::Interpolation::Step => CurveStyle::Step,
        },
    );
    for (point, value) in module.points.iter_mut().zip(curve.points) {
        point.value = float(value, 0.0, 1.0);
    }
    for segment in &mut module.segments {
        segment.tension = float(0.0, -1.0, 1.0);
    }
}

fn instance(sample_rate: f64, configure: impl FnOnce(&mut ShapeRackParams)) -> ShapeRack {
    let mut parameters = ShapeRackParams::default();
    configure(&mut parameters);
    // The host initializes all parameter smoothers before calling Plugin::reset.
    // These tests enter process_audio directly, so perform the same setup here.
    for parameter in [
        &parameters.volume.amount,
        &parameters.filter.amount,
        &parameters.pan.amount,
        &parameters.drive.amount,
        &parameters.mix,
        &parameters.output,
    ] {
        parameter.smoothed.reset(parameter.value());
    }
    let mut plugin = ShapeRack {
        params: Arc::new(parameters),
        sample_rate,
        ..ShapeRack::default()
    };
    plugin.clock.set_sample_rate(sample_rate);
    plugin.volume.set_sample_rate(sample_rate);
    plugin.filter.set_sample_rate(sample_rate);
    plugin.transition = (-1.0 / (sample_rate * 0.003)).exp() as f32;
    plugin.reset();
    plugin
}

fn process_channels(
    plugin: &mut ShapeRack,
    channels: &mut [Vec<f32>],
    tempo: f64,
    position: Option<f64>,
    playing: bool,
) {
    let frames = channels.first().map_or(0, Vec::len);
    assert!(channels.iter().all(|channel| channel.len() == frames));
    let mut buffer = Buffer::default();
    // All channel slices are disjoint, equally sized, and remain alive for the
    // entire process call. The buffer is dropped before channels can be reused.
    unsafe {
        buffer.set_slices(frames, |slices| {
            *slices = channels.iter_mut().map(Vec::as_mut_slice).collect();
        });
    }
    plugin.process_audio(&mut buffer, tempo, position, playing);
}

fn audio_signal(frames: usize, sample_rate: f64) -> Vec<Vec<f32>> {
    let mut channels = vec![vec![0.0; frames]; 2];
    let (left, right) = channels.split_at_mut(1);
    for (index, (left_sample, right_sample)) in
        left[0].iter_mut().zip(right[0].iter_mut()).enumerate()
    {
        let phase = index as f64 / sample_rate * std::f64::consts::TAU;
        *left_sample = (0.8 * (phase * 220.0).sin()) as f32;
        *right_sample = (0.45 * (phase * 733.0).cos()) as f32;
    }
    channels
}

fn render_in_blocks(
    plugin: &mut ShapeRack,
    input: &[Vec<f32>],
    sizes: &[usize],
    tempo: f64,
) -> Vec<Vec<f32>> {
    render_with_transport(plugin, input, sizes, tempo, true)
}

fn render_with_transport(
    plugin: &mut ShapeRack,
    input: &[Vec<f32>],
    sizes: &[usize],
    tempo: f64,
    playing: bool,
) -> Vec<Vec<f32>> {
    assert!(!sizes.is_empty() && sizes.iter().all(|size| *size > 0));
    let mut result = vec![Vec::new(); input.len()];
    let mut start = 0;
    let mut block_index = 0;
    while start < input[0].len() {
        let end = (start + sizes[block_index % sizes.len()]).min(input[0].len());
        let mut block: Vec<Vec<f32>> = input
            .iter()
            .map(|channel| channel[start..end].to_vec())
            .collect();
        let beats = if playing {
            start as f64 / plugin.sample_rate * tempo / 60.0
        } else {
            0.0
        };
        process_channels(plugin, &mut block, tempo, Some(beats), playing);
        for (output, channel) in result.iter_mut().zip(block) {
            output.extend(channel);
        }
        start = end;
        block_index += 1;
    }
    result
}

#[test]
fn global_bypass_after_reset_is_exact_dry_in_mono_and_stereo() {
    for channel_count in [1, 2] {
        let mut plugin = instance(48000.0, |params| {
            params.bypass = BoolParam::new("Bypass", true);
            params.filter.enabled = BoolParam::new("Filter", true);
            params.pan.enabled = BoolParam::new("Pan", true);
            params.drive.enabled = BoolParam::new("Drive", true);
            params.output = float(12.0, -24.0, 12.0);
        });
        let mut channels = audio_signal(1024, 48000.0);
        channels.truncate(channel_count);
        let dry = channels.clone();
        process_channels(&mut plugin, &mut channels, 150.0, Some(0.0), true);
        assert_eq!(channels, dry);
    }
}

#[test]
fn zero_mix_with_unity_output_is_exact_dry() {
    let mut plugin = instance(44100.0, |params| {
        params.mix = float(0.0, 0.0, 1.0);
        params.filter.enabled = BoolParam::new("Filter", true);
        params.pan.enabled = BoolParam::new("Pan", true);
        params.drive.enabled = BoolParam::new("Drive", true);
    });
    let mut channels = audio_signal(4096, 44100.0);
    let dry = channels.clone();
    process_channels(&mut plugin, &mut channels, 90.0, Some(12.3), true);
    assert_eq!(channels, dry);
}

#[test]
fn default_volume_pump_is_independent_of_host_block_sizes() {
    let sample_rate = 48000.0;
    let input = audio_signal(52001, sample_rate);
    let mut whole_plugin = instance(sample_rate, |_| {});
    let expected = render_in_blocks(&mut whole_plugin, &input, &[52001], 127.0);
    for sizes in [&[64][..], &[512][..], &[1, 13, 67, 511, 2048][..]] {
        let mut plugin = instance(sample_rate, |_| {});
        let actual = render_in_blocks(&mut plugin, &input, sizes, 127.0);
        let maximum_difference = actual
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            maximum_difference < 2e-5,
            "block sizes {sizes:?}: error {maximum_difference}"
        );
    }
    assert!(expected[0]
        .iter()
        .zip(&input[0])
        .any(|(output, input)| output.abs() < input.abs() * 0.2));
}

#[test]
fn transport_seeks_realign_the_next_sample_at_any_tempo() {
    let mut plugin = instance(48000.0, |params| {
        apply_preset(&mut params.volume, Preset::Ramp);
        params.smoothing = float(0.0, 0.0, 30.0);
    });
    for (tempo, beats, expected) in [
        (60.0, 12.25, 0.25),
        (180.0, -0.25, 0.75),
        (97.0, 0.0, 0.0),
        (140.0, 88.5, 0.5),
    ] {
        let mut channels = vec![vec![1.0], vec![-0.5]];
        process_channels(&mut plugin, &mut channels, tempo, Some(beats), true);
        assert_eq!(channels[0][0], expected);
        assert_eq!(channels[1][0], expected * -0.5);
        let expected_next_phase = (expected as f64 + tempo / (60.0 * 48000.0)).rem_euclid(1.0);
        assert!((plugin.clock.phase() - expected_next_phase).abs() < 1e-12);
    }
}

#[test]
fn stopped_synced_transport_previews_continuously_despite_a_fixed_host_position() {
    let mut plugin = instance(48000.0, |params| {
        apply_preset(&mut params.volume, Preset::Ramp);
        params.smoothing = float(0.0, 0.0, 30.0);
    });
    for block in 0..3 {
        let mut channels = vec![vec![1.0; 512]];
        process_channels(&mut plugin, &mut channels, 120.0, Some(4.375), false);
        let first_phase = (block * 512) as f32 / 24000.0;
        assert!((channels[0][0] - first_phase).abs() < 1e-7);
        assert!(channels[0][511] > channels[0][0]);
        assert!((plugin.clock.phase() - ((block + 1) * 512) as f64 / 24000.0).abs() < 1e-12);
    }
}

#[test]
fn stopped_sync_short_note_preview_is_audible_after_silent_processing() {
    for sample_rate in [44100.0, 48000.0, 96000.0] {
        let mut plugin = instance(sample_rate, |_| {});
        // Real hosts may process silence for a long time before a keyboard note.
        // The old stopped-position hold settled Pump's gain to exactly zero.
        let mut silence = vec![vec![0.0; (sample_rate * 0.125) as usize]];
        process_channels(&mut plugin, &mut silence, 120.0, Some(0.0), false);
        let note: Vec<f32> = (0..(sample_rate * 0.03) as usize)
            .map(|index| {
                let seconds = index as f64 / sample_rate;
                (0.8 * (-seconds / 0.015).exp() * (std::f64::consts::TAU * 440.0 * seconds).sin())
                    as f32
            })
            .collect();
        let dry_energy: f32 = note.iter().map(|sample| sample * sample).sum();
        let preview = render_with_transport(&mut plugin, &[note], &[17, 128, 256], 120.0, false);
        let wet_energy: f32 = preview[0].iter().map(|sample| sample * sample).sum();
        assert!(
            wet_energy > dry_energy * 0.05,
            "stopped Sync muted the short note at {sample_rate} Hz: wet/dry {}",
            wet_energy / dry_energy
        );
        assert!(preview[0].iter().any(|sample| sample.abs() > 0.1));
    }
}

#[test]
fn stopped_sync_processes_a_sum_of_routed_sources_audibly() {
    let sample_rate = 48000.0;
    let mut plugin = instance(sample_rate, |_| {});
    let mut silence = vec![vec![0.0; 10000]; 2];
    process_channels(&mut plugin, &mut silence, 120.0, Some(0.0), false);
    // A mixer sends the sum of routed sources to one main stereo input bus.
    let mut bus = vec![vec![0.0; 4800]; 2];
    let (left, right) = bus.split_at_mut(1);
    for (index, (left, right)) in left[0].iter_mut().zip(&mut right[0]).enumerate() {
        let phase = std::f64::consts::TAU * index as f64 / sample_rate;
        let first_source = 0.3 * (220.0 * phase).sin();
        let second_source = 0.25 * (659.0 * phase).sin();
        *left = (first_source + second_source) as f32;
        *right = (first_source * 0.7 + second_source * 0.4) as f32;
    }
    let preview = render_with_transport(&mut plugin, &bus, &[64, 127, 512], 120.0, false);
    for (output, input) in preview.iter().zip(bus) {
        let wet_energy: f32 = output.iter().map(|sample| sample * sample).sum();
        let dry_energy: f32 = input.iter().map(|sample| sample * sample).sum();
        assert!(wet_energy > dry_energy * 0.15);
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn stopped_sync_preview_is_independent_of_host_block_sizes() {
    let input = audio_signal(28001, 48000.0);
    let mut whole = instance(48000.0, |_| {});
    let expected = render_with_transport(&mut whole, &input, &[28001], 127.0, false);
    for sizes in [&[64][..], &[1, 17, 127, 1024][..]] {
        let mut split = instance(48000.0, |_| {});
        let actual = render_with_transport(&mut split, &input, sizes, 127.0, false);
        assert_eq!(
            actual, expected,
            "preview changed with block sizes {sizes:?}"
        );
        assert_eq!(split.clock.phase(), whole.clock.phase());
    }
}

#[test]
fn stopped_sync_preview_uses_tempo_and_cycle_length() {
    let mut plugin = instance(48000.0, |params| {
        params.division = EnumParam::new("Cycle length", params::Division::Half);
    });
    let mut silence = vec![vec![0.0; 6000]];
    process_channels(&mut plugin, &mut silence, 120.0, Some(88.0), false);
    assert!((plugin.clock.phase() - 0.125).abs() < 1e-12);
    process_channels(&mut plugin, &mut silence, 60.0, Some(88.0), false);
    assert!((plugin.clock.phase() - 0.1875).abs() < 1e-12);
}

#[test]
fn stopping_continues_preview_and_resuming_realigns_the_first_sample() {
    let mut plugin = instance(48000.0, |params| {
        apply_preset(&mut params.volume, Preset::Ramp);
        params.smoothing = float(0.0, 0.0, 30.0);
    });
    let mut playing = vec![vec![1.0; 256]];
    process_channels(&mut plugin, &mut playing, 120.0, Some(8.75), true);
    let phase_at_stop = plugin.clock.phase();
    let mut stopped = vec![vec![1.0; 512]];
    process_channels(&mut plugin, &mut stopped, 120.0, Some(0.0), false);
    assert!((stopped[0][0] as f64 - phase_at_stop).abs() < 1e-7);
    assert!(plugin.clock.phase() > phase_at_stop);
    let before_cursor_move = plugin.clock.phase();
    let mut one_sample = vec![vec![1.0]];
    process_channels(&mut plugin, &mut one_sample, 120.0, Some(111.0), false);
    assert!((one_sample[0][0] as f64 - before_cursor_move).abs() < 1e-7);
    one_sample[0][0] = 1.0;
    process_channels(&mut plugin, &mut one_sample, 180.0, Some(3.25), true);
    assert_eq!(one_sample[0][0], 0.25);
    assert!((plugin.clock.phase() - (0.25 + 180.0 / (60.0 * 48000.0))).abs() < 1e-12);
    // A subsequent playing loop/seek still uses the exact new host position.
    one_sample[0][0] = 1.0;
    process_channels(&mut plugin, &mut one_sample, 180.0, Some(0.0), true);
    assert_eq!(one_sample[0][0], 0.0);
}

#[test]
fn free_rate_keeps_advancing_when_transport_is_stopped() {
    let mut plugin = instance(48000.0, |params| {
        params.sync = BoolParam::new("Tempo sync", false);
        params.free_hz = float(2.0, 0.05, 20.0);
    });
    let mut channels = vec![vec![0.5; 6000]];
    process_channels(&mut plugin, &mut channels, 200.0, Some(87.0), false);
    assert!((plugin.clock.phase() - 0.25).abs() < 1e-11);
}

#[test]
fn pan_module_leaves_mono_audio_unchanged() {
    let mut plugin = instance(48000.0, |params| {
        params.volume.enabled = BoolParam::new("Volume", false);
        params.pan.enabled = BoolParam::new("Pan", true);
        params.pan.amount = float(1.0, 0.0, 1.0);
        params.smoothing = float(0.0, 0.0, 30.0);
        apply_preset(&mut params.pan, Preset::Flat);
    });
    let mut channels = audio_signal(1024, 48000.0);
    channels.truncate(1);
    let dry = channels.clone();
    process_channels(&mut plugin, &mut channels, 120.0, Some(0.0), true);
    assert_eq!(channels, dry);
    // The same full-right curve should mute the left channel in stereo mode.
    let mut stereo = vec![vec![0.25; 16], vec![-0.5; 16]];
    process_channels(&mut plugin, &mut stereo, 120.0, Some(0.0), true);
    assert!(stereo[0].iter().all(|sample| *sample == 0.0));
    assert!(stereo[1].iter().all(|sample| *sample == -0.5));
}

#[test]
fn all_modules_produce_finite_bounded_stereo_with_different_phases() {
    for sample_rate in [44100.0, 48000.0, 96000.0] {
        let mut plugin = instance(sample_rate, |params| {
            params.filter.enabled = BoolParam::new("Filter", true);
            params.pan.enabled = BoolParam::new("Pan", true);
            params.drive.enabled = BoolParam::new("Drive", true);
            params.stereo = float(0.37, 0.0, 1.0);
            params.offset = float(0.2, 0.0, 1.0);
            params.smoothing = float(0.0, 0.0, 30.0);
            params.output = float(12.0, -24.0, 12.0);
            apply_preset(&mut params.volume, Preset::Gate);
        });
        let mut input = audio_signal(12000, sample_rate);
        input[0][17] = f32::NAN;
        input[1][37] = f32::INFINITY;
        let actual = render_in_blocks(&mut plugin, &input, &[7, 127, 512], 220.0);
        assert!(actual
            .iter()
            .flatten()
            .all(|sample| sample.is_finite() && sample.abs() < 4.1));
        assert!(actual.iter().flatten().any(|sample| sample.abs() > 0.01));
        assert!(actual[0]
            .iter()
            .zip(&actual[1])
            .any(|(left, right)| (left - right).abs() > 0.05));
    }
}

#[test]
fn bypass_transition_settles_to_exact_dry() {
    let mut plugin = instance(48000.0, |_| {});
    let mut before = audio_signal(1024, 48000.0);
    process_channels(&mut plugin, &mut before, 120.0, Some(0.0), true);
    Arc::get_mut(&mut plugin.params)
        .expect("test owns parameter Arc")
        .bypass = BoolParam::new("Bypass", true);
    let mut settling = audio_signal(4096, 48000.0);
    process_channels(&mut plugin, &mut settling, 120.0, Some(0.1), true);
    let mut after = audio_signal(128, 48000.0);
    let dry = after.clone();
    process_channels(&mut plugin, &mut after, 120.0, Some(0.2), true);
    assert_eq!(plugin.bypass_weight, 0.0);
    assert_eq!(after, dry);
}

#[test]
fn host_parameter_ids_are_unique_and_include_every_curve_point() {
    let parameters = ShapeRackParams::default();
    let map = parameters.param_map();
    // Nine globals; each module has 17 points, 16 bends, and 3 controls.
    assert_eq!(
        map.len(),
        9 + 4 * (dsp::POINT_COUNT + dsp::SEGMENT_COUNT + 3)
    );
    let mut ids = HashSet::new();
    let mut pointers = HashSet::new();
    for (id, pointer, _) in &map {
        assert!(ids.insert(id), "duplicate parameter ID {id}");
        assert!(pointers.insert(*pointer), "aliased parameter {id}");
        // The pointers come from parameters, which stays alive throughout.
        let normalized = unsafe { pointer.unmodulated_normalized_value() };
        assert!(
            (0.0..=1.0).contains(&normalized),
            "invalid default for {id}"
        );
    }
    for module in ["volume", "filter", "pan", "drive"] {
        for point in 1..=dsp::POINT_COUNT {
            let expected = format!("{module}_value_{point}");
            assert!(
                ids.contains(&expected),
                "missing curve parameter {expected}"
            );
        }
        for segment in 1..=dsp::SEGMENT_COUNT {
            let expected = format!("{module}_tension_{segment}");
            assert!(ids.contains(&expected), "missing bend parameter {expected}");
        }
    }
}

#[test]
fn persisted_editor_size_serializes_and_restores() {
    let original = ShapeRackParams {
        editor_state: EguiState::from_size(1100, 720),
        ..ShapeRackParams::default()
    };
    let state = original.serialize_fields();
    assert!(state.contains_key("editor-state"));
    let restored = ShapeRackParams::default();
    assert_ne!(restored.editor_state.size(), original.editor_state.size());
    restored.deserialize_fields(&state);
    assert_eq!(restored.editor_state.size(), (1100, 720));
    assert_eq!(restored.serialize_fields(), state);
}

#[test]
fn segment_bends_reach_audio_without_changing_curve_point_ids() {
    let mut plugin = instance(48000.0, |params| {
        apply_preset(&mut params.volume, Preset::Ramp);
        params.volume.points[0].value = float(0.0, 0.0, 1.0);
        params.volume.points[1].value = float(1.0, 0.0, 1.0);
        params.volume.segments[0].tension = float(0.5, -1.0, 1.0);
        params.smoothing = float(0.0, 0.0, 30.0);
    });
    let mut channels = vec![vec![1.0]];
    process_channels(&mut plugin, &mut channels, 120.0, Some(0.5 / 16.0), true);
    assert!(channels[0]
        .iter()
        .all(|sample| (*sample - 0.7375).abs() < 1e-6));
    assert_eq!(plugin.params.volume.points[0].value.value(), 0.0);
    assert_eq!(plugin.params.volume.points[1].value.value(), 1.0);
}

#[test]
fn new_bend_parameters_default_to_neutral_for_older_projects() {
    let parameters = ShapeRackParams::default();
    for module in 0..4 {
        assert!(parameters
            .module(module)
            .segments
            .iter()
            .all(|segment| segment.tension.value() == 0.0));
        assert_eq!(
            parameters.module(module).curve().tensions,
            [0.0; dsp::SEGMENT_COUNT]
        );
    }
}

#[test]
fn preset_label_persists_without_becoming_an_audio_parameter() {
    let original = ShapeRackParams::default();
    *original.preset_name.lock().unwrap() = "Wide pulse".to_string();
    let restored = ShapeRackParams::default();
    restored.deserialize_fields(&original.serialize_fields());
    assert_eq!(*restored.preset_name.lock().unwrap(), "Wide pulse");
    assert!(restored
        .param_map()
        .iter()
        .all(|(id, _, _)| id != "preset-name"));
}

#[test]
fn complete_rack_with_bends_is_independent_of_block_boundaries() {
    fn configure(params: &mut ShapeRackParams) {
        params.filter.enabled = BoolParam::new("Filter", true);
        params.pan.enabled = BoolParam::new("Pan", true);
        params.drive.enabled = BoolParam::new("Drive", true);
        params.stereo = float(0.23, 0.0, 1.0);
        for segment in &mut params.volume.segments {
            segment.tension = float(0.4, -1.0, 1.0);
        }
        for segment in &mut params.filter.segments {
            segment.tension = float(-0.3, -1.0, 1.0);
        }
    }
    let input = audio_signal(12001, 48000.0);
    let mut whole = instance(48000.0, configure);
    let expected = render_in_blocks(&mut whole, &input, &[12001], 173.0);
    let mut split = instance(48000.0, configure);
    let actual = render_in_blocks(&mut split, &input, &[1, 17, 128, 255], 173.0);
    let difference = actual
        .iter()
        .flatten()
        .zip(expected.iter().flatten())
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        difference < 2e-5,
        "block-dependent rack audio: {difference}"
    );
}
