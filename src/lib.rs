//! ShapeRack: an original tempo-synced four-module modulation effect.
// The pinned framework's VST3 COM macro triggers this upstream compatibility lint.
#![allow(semicolon_in_expressions_from_non_local_macros)]
pub mod dsp;
mod editor;
pub mod params;
mod preset_file;
mod presets;

use dsp::{AntialiasedDrive, PhaseClock, Rate, StereoLowpass, VolumeShaper};
use nih_plug::prelude::*;
use params::ShapeRackParams;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct Meters {
    pub phase: AtomicF32,
    pub peak: AtomicF32,
    pub tempo: AtomicF32,
    pub playing: AtomicBool,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            phase: AtomicF32::new(0.0),
            peak: AtomicF32::new(0.0),
            tempo: AtomicF32::new(120.0),
            playing: AtomicBool::new(false),
        }
    }
}

pub struct ShapeRack {
    params: Arc<ShapeRackParams>,
    meters: Arc<Meters>,
    clock: PhaseClock,
    volume: VolumeShaper,
    filter: StereoLowpass,
    drive: AntialiasedDrive,
    sample_rate: f64,
    transition: f32,
    enable: [f32; 4],
    bypass_weight: f32,
    pan_position: f32,
    drive_amount: [f32; 2],
}

impl Default for ShapeRack {
    fn default() -> Self {
        Self {
            params: Arc::new(ShapeRackParams::default()),
            meters: Arc::new(Meters::default()),
            clock: PhaseClock::new(44100.0),
            volume: VolumeShaper::new(44100.0),
            filter: StereoLowpass::new(44100.0),
            drive: AntialiasedDrive::new(),
            sample_rate: 44100.0,
            transition: 0.0,
            enable: [1.0, 0.0, 0.0, 0.0],
            bypass_weight: 1.0,
            pan_position: 0.0,
            drive_amount: [0.0; 2],
        }
    }
}

fn blend(dry: (f32, f32), wet: (f32, f32), amount: f32) -> (f32, f32) {
    (
        dry.0 + (wet.0 - dry.0) * amount,
        dry.1 + (wet.1 - dry.1) * amount,
    )
}

impl Plugin for ShapeRack {
    const NAME: &'static str = "ShapeRack";
    const VENDOR: &'static str = "PRISMM";
    const URL: &'static str = "https://github.com/0xPRISMM/shaperack";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.meters.clone())
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = config.sample_rate as f64;
        self.clock.set_sample_rate(self.sample_rate);
        self.volume.set_sample_rate(self.sample_rate);
        self.filter.set_sample_rate(self.sample_rate);
        self.transition = (-1.0 / (self.sample_rate * 0.003)).exp() as f32;
        true
    }

    fn reset(&mut self) {
        self.clock.reset();
        self.volume.reset();
        self.filter.reset();
        self.drive.reset();
        self.enable = std::array::from_fn(|i| {
            if self.params.module(i).enabled.value() {
                1.0
            } else {
                0.0
            }
        });
        self.bypass_weight = if self.params.bypass.value() { 0.0 } else { 1.0 };
        self.pan_position = 0.0;
        self.drive_amount = [0.0; 2];
        self.meters.peak.store(0.0, Ordering::Relaxed);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let transport = context.transport();
        self.process_audio(
            buffer,
            transport.tempo.unwrap_or(120.0),
            transport.pos_beats(),
            transport.playing,
        );
        ProcessStatus::Normal
    }
}

impl ShapeRack {
    fn process_audio(
        &mut self,
        buffer: &mut Buffer,
        tempo: f64,
        beat_position: Option<f64>,
        playing: bool,
    ) {
        self.clock.set_tempo(tempo);
        let synced = self.params.sync.value();
        self.clock.set_rate(if synced {
            Rate::Synced {
                beats: self.params.division.value().beats(),
            }
        } else {
            Rate::Free {
                hz: self.params.free_hz.value() as f64,
            }
        });
        // During playback the framework supplies the position for each
        // automation sub-block, keeping seeks, loop wraps, and resume aligned.
        // Stopped hosts may keep reporting the same position while previewing
        // notes or routing live audio. Ignore that stationary playhead and let
        // the tempo-based clock continue, so Pump/Gate cannot stay stuck shut.
        if synced && playing {
            if let Some(beats) = beat_position {
                self.clock.seek_beats(beats);
            }
        }
        self.volume
            .set_smoothing_ms(self.params.smoothing.value() as f64);
        self.filter
            .set_smoothing_ms(self.params.smoothing.value() as f64);
        let curves = std::array::from_fn::<_, 4, _>(|i| self.params.module(i).curve());
        let targets = std::array::from_fn::<_, 4, _>(|i| {
            if self.params.module(i).enabled.value() {
                1.0
            } else {
                0.0
            }
        });
        let bypass_target = if self.params.bypass.value() { 0.0 } else { 1.0 };
        let offset = self.params.offset.value() as f64;
        let stereo = self.params.stereo.value() as f64;
        let modulation_coefficient = if self.params.smoothing.value() <= 0.0 {
            0.0
        } else {
            (-1.0 / (self.sample_rate * self.params.smoothing.value() as f64 * 0.001)).exp() as f32
        };
        let mut peak = 0.0_f32;
        let is_stereo = buffer.channels() > 1;
        for channel_samples in buffer.iter_samples() {
            let mut channels = channel_samples.into_iter();
            let Some(left_sample) = channels.next() else {
                continue;
            };
            let right_sample = channels.next();
            let left = if left_sample.is_finite() {
                *left_sample
            } else {
                0.0
            };
            let right = right_sample
                .as_deref()
                .map(|v| if v.is_finite() { *v } else { 0.0 })
                .unwrap_or(left);
            let dry = (left, right);
            let phase = (self.clock.phase() + offset).rem_euclid(1.0);
            let phase_right = (phase + stereo).rem_euclid(1.0);
            for (weight, target) in self.enable.iter_mut().zip(targets) {
                *weight = target + self.transition * (*weight - target);
                if (*weight - target).abs() < 1.0e-6 {
                    *weight = target;
                }
            }
            let volume_amount = self.params.volume.amount.smoothed.next();
            let filter_amount = self.params.filter.amount.smoothed.next();
            let pan_amount = self.params.pan.amount.smoothed.next();
            let drive_amount = self.params.drive.amount.smoothed.next();
            let mut wet = self.volume.process_frame(
                left,
                right,
                &curves[0],
                volume_amount * self.enable[0],
                phase,
                stereo,
            );
            if self.enable[1] > 0.0 {
                // A logarithmic sweep from 60 Hz to min(20 kHz, 0.45 Fs).
                let ceiling = (self.sample_rate as f32 * 0.45).clamp(60.0, 20000.0);
                let cutoff = |p| 60.0 * (ceiling / 60.0).powf(curves[1].value(p));
                let filtered =
                    self.filter
                        .process_frame(wet.0, wet.1, cutoff(phase), cutoff(phase_right));
                wet = blend(wet, filtered, filter_amount * self.enable[1]);
            } else {
                self.filter.reset();
            }
            let pan_target = (curves[2].value(phase) * 2.0 - 1.0) * pan_amount * self.enable[2];
            self.pan_position =
                pan_target + modulation_coefficient * (self.pan_position - pan_target);
            if right_sample.is_some() {
                wet = dsp::stereo_balance(wet.0, wet.1, self.pan_position);
            }
            for (i, p) in [phase, phase_right].into_iter().enumerate() {
                let target = curves[3].value(p) * drive_amount * self.enable[3];
                self.drive_amount[i] =
                    target + modulation_coefficient * (self.drive_amount[i] - target);
            }
            wet =
                self.drive
                    .process_frame(wet.0, wet.1, self.drive_amount[0], self.drive_amount[1]);
            self.bypass_weight =
                bypass_target + self.transition * (self.bypass_weight - bypass_target);
            if (self.bypass_weight - bypass_target).abs() < 1.0e-6 {
                self.bypass_weight = bypass_target;
            }
            let mix = self.params.mix.smoothed.next();
            let output_gain = util::db_to_gain(self.params.output.smoothed.next());
            let effected = blend(dry, wet, mix);
            let effected = (effected.0 * output_gain, effected.1 * output_gain);
            let out = blend(dry, effected, self.bypass_weight);
            *left_sample = out.0;
            if let Some(right_sample) = right_sample {
                *right_sample = out.1;
            }
            peak = peak.max(out.0.abs());
            if is_stereo {
                peak = peak.max(out.1.abs());
            }
            // Preview cycles also advance through silence. Audio activity does
            // not retrigger the curve; the next playing block realigns to host.
            self.clock.advance();
        }
        if self.params.editor_state.is_open() {
            let old = self.meters.peak.load(Ordering::Relaxed);
            let decay = (-(buffer.samples() as f64) / (self.sample_rate * 0.2)).exp() as f32;
            self.meters
                .peak
                .store(peak.max(old * decay), Ordering::Relaxed);
            self.meters.phase.store(
                (self.clock.phase() + offset).rem_euclid(1.0) as f32,
                Ordering::Relaxed,
            );
            self.meters.tempo.store(tempo as f32, Ordering::Relaxed);
            self.meters.playing.store(playing, Ordering::Relaxed);
        }
    }
}

impl ClapPlugin for ShapeRack {
    const CLAP_ID: &'static str = "audio.prismm.shaperack";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Drawable volume, filter, pan and drive modulation");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::Tremolo,
        ClapFeature::Filter,
        ClapFeature::Distortion,
    ];
}

impl Vst3Plugin for ShapeRack {
    const VST3_CLASS_ID: [u8; 16] = *b"ShapeRackPRISM01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nih_export_clap!(ShapeRack);
nih_export_vst3!(ShapeRack);

#[cfg(test)]
mod tests;
