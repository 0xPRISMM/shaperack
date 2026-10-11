//! Allocation-free curve evaluation, transport clock, and stereo gain shaping.
//!
//! All phase values use cycles (0..1), and host positions use quarter-note beats.
//! The DSP deliberately has no dependency on the plugin framework so its timing
//! and edge cases can be tested without a host.

pub const POINT_COUNT: usize = 17;
pub const SEGMENT_COUNT: usize = POINT_COUNT - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    #[default]
    Smooth,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Pump,
    Gate,
    Sine,
    Ramp,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    /// Seventeen evenly spaced values, including the right-hand endpoint.
    /// Public for inexpensive parameter snapshots; evaluation sanitizes values.
    pub points: [f32; POINT_COUNT],
    pub interpolation: Interpolation,
}

impl Default for Curve {
    fn default() -> Self {
        Self::from_preset(Preset::Pump)
    }
}

impl Curve {
    pub fn from_preset(preset: Preset) -> Self {
        let mut points = [1.0; POINT_COUNT];
        let interpolation = match preset {
            Preset::Pump => {
                points = [
                    0.0, 0.01, 0.05, 0.14, 0.30, 0.49, 0.66, 0.79, 0.88, 0.94, 0.98, 1.0, 1.0, 1.0,
                    1.0, 1.0, 1.0,
                ];
                Interpolation::Smooth
            }
            Preset::Gate => {
                for (index, point) in points.iter_mut().enumerate() {
                    *point = if index < SEGMENT_COUNT / 2 || index == SEGMENT_COUNT {
                        1.0
                    } else {
                        0.0
                    };
                }
                Interpolation::Step
            }
            Preset::Sine => {
                for (index, point) in points.iter_mut().enumerate() {
                    let phase = index as f64 / SEGMENT_COUNT as f64;
                    *point = (0.5 - 0.5 * (phase * std::f64::consts::TAU).cos()) as f32;
                }
                Interpolation::Smooth
            }
            Preset::Ramp => {
                for (index, point) in points.iter_mut().enumerate() {
                    *point = index as f32 / SEGMENT_COUNT as f32;
                }
                Interpolation::Linear
            }
            Preset::Flat => Interpolation::Linear,
        };
        Self {
            points,
            interpolation,
        }
    }

    /// Evaluate a position within the editor, including the endpoint at 1.0.
    /// Positions outside the editor are clamped; non-finite positions use 0.0.
    pub fn value(&self, phase: f64) -> f32 {
        let position = finite_or(phase, 0.0).clamp(0.0, 1.0) * SEGMENT_COUNT as f64;
        let index = position as usize;
        if index >= SEGMENT_COUNT {
            return unit_or(self.points[SEGMENT_COUNT], 1.0);
        }

        let start = unit_or(self.points[index], 1.0);
        if self.interpolation == Interpolation::Step {
            return start;
        }
        let end = unit_or(self.points[index + 1], 1.0);
        let fraction = (position - index as f64) as f32;
        let amount = match self.interpolation {
            Interpolation::Smooth => fraction * fraction * (3.0 - 2.0 * fraction),
            _ => fraction,
        };
        (start + (end - start) * amount).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    /// Number of host quarter-note beats in one whole curve cycle.
    Synced {
        beats: f64,
    },
    Free {
        hz: f64,
    },
}

impl Default for Rate {
    fn default() -> Self {
        Self::Synced { beats: 1.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PhaseClock {
    sample_rate: f64,
    tempo: f64,
    rate: Rate,
    phase: f64,
    increment: f64,
}

impl PhaseClock {
    pub fn new(sample_rate: f64) -> Self {
        let mut clock = Self {
            sample_rate: valid_sample_rate(sample_rate),
            tempo: 120.0,
            rate: Rate::default(),
            phase: 0.0,
            increment: 0.0,
        };
        clock.update_increment();
        clock
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = valid_sample_rate(sample_rate);
        self.update_increment();
    }

    pub fn set_tempo(&mut self, bpm: f64) {
        self.tempo = finite_or(bpm, 120.0).clamp(1.0, 1000.0);
        self.update_increment();
    }

    pub fn set_rate(&mut self, rate: Rate) {
        self.rate = match rate {
            Rate::Synced { beats } => Rate::Synced {
                beats: finite_or(beats, 1.0).clamp(1.0 / 1024.0, 1024.0),
            },
            Rate::Free { hz } => Rate::Free {
                hz: finite_or(hz, 1.0).clamp(0.0, 1000.0),
            },
        };
        self.update_increment();
    }

    /// Align the next sample with a host position. Call at block boundaries when
    /// a beat position is available, including after a seek or loop wrap.
    /// Free-running mode deliberately retains its phase when the host seeks.
    pub fn seek_beats(&mut self, position_beats: f64) {
        if let Rate::Synced { beats } = self.rate {
            self.phase = wrap_phase(finite_or(position_beats, 0.0) / beats);
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Advance one sample and return the *next* sample's phase.
    /// Evaluate `phase()` before calling this function to begin exactly at zero.
    pub fn advance(&mut self) -> f64 {
        self.phase = wrap_phase(self.phase + self.increment);
        self.phase
    }

    fn update_increment(&mut self) {
        let hz = match self.rate {
            Rate::Synced { beats } => self.tempo / (60.0 * beats),
            Rate::Free { hz } => hz,
        };
        self.increment = hz / self.sample_rate;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VolumeShaper {
    sample_rate: f64,
    smoothing_ms: f64,
    smoothing_coefficient: f32,
    gain_left: f32,
    gain_right: f32,
}

impl VolumeShaper {
    pub fn new(sample_rate: f64) -> Self {
        let mut shaper = Self {
            sample_rate: valid_sample_rate(sample_rate),
            smoothing_ms: 2.0,
            smoothing_coefficient: 0.0,
            gain_left: 1.0,
            gain_right: 1.0,
        };
        shaper.update_coefficient();
        shaper
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = valid_sample_rate(sample_rate);
        self.update_coefficient();
    }

    /// One-pole smoothing time constant, in milliseconds. Zero disables it.
    /// This smooths transitions at curve wraps and steps, as well as edits.
    pub fn set_smoothing_ms(&mut self, milliseconds: f64) {
        self.smoothing_ms = finite_or(milliseconds, 2.0).clamp(0.0, 200.0);
        self.update_coefficient();
    }

    pub fn reset(&mut self) {
        self.gain_left = 1.0;
        self.gain_right = 1.0;
    }

    /// Apply unipolar volume modulation. Depth is 0..1; stereo_offset is the
    /// right channel's phase offset in cycles. At zero depth finite input is
    /// passed through exactly, with no residual attenuation from smoothing.
    pub fn process_frame(
        &mut self,
        left: f32,
        right: f32,
        curve: &Curve,
        depth: f32,
        phase: f64,
        stereo_offset: f64,
    ) -> (f32, f32) {
        let left = audio_or_silence(left);
        let right = audio_or_silence(right);
        let depth = unit_or(depth, 0.0);
        if depth == 0.0 {
            self.reset();
            return (left, right);
        }

        let phase = wrap_phase(phase);
        let offset = wrap_phase(stereo_offset);
        let target_left = 1.0 - depth + depth * curve.value(phase);
        let target_right = 1.0 - depth + depth * curve.value(wrap_phase(phase + offset));
        let coefficient = self.smoothing_coefficient;
        self.gain_left = target_left + coefficient * (self.gain_left - target_left);
        self.gain_right = target_right + coefficient * (self.gain_right - target_right);
        // Truncate imperceptible tails to avoid denormal work on silent audio.
        if self.gain_left.abs() < 1.0e-20 {
            self.gain_left = 0.0;
        }
        if self.gain_right.abs() < 1.0e-20 {
            self.gain_right = 0.0;
        }
        (left * self.gain_left, right * self.gain_right)
    }

    fn update_coefficient(&mut self) {
        self.smoothing_coefficient = if self.smoothing_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (self.sample_rate * self.smoothing_ms * 0.001)).exp() as f32
        };
    }
}

/// Stereo one-pole low-pass with smoothed coefficients and independent cutoffs.
/// Cutoffs are clamped between 5 Hz and 0.49 times the sample rate.
#[derive(Clone, Copy, Debug)]
pub struct StereoLowpass {
    sample_rate: f64,
    smoothing_ms: f64,
    smoothing_coefficient: f64,
    alpha_left: f64,
    alpha_right: f64,
    output_left: f64,
    output_right: f64,
    initialized: bool,
}

impl StereoLowpass {
    pub fn new(sample_rate: f64) -> Self {
        let mut filter = Self {
            sample_rate: valid_sample_rate(sample_rate),
            smoothing_ms: 2.0,
            smoothing_coefficient: 0.0,
            alpha_left: 1.0,
            alpha_right: 1.0,
            output_left: 0.0,
            output_right: 0.0,
            initialized: false,
        };
        filter.update_smoothing();
        filter
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = valid_sample_rate(sample_rate);
        self.update_smoothing();
        self.initialized = false;
    }

    pub fn set_smoothing_ms(&mut self, milliseconds: f64) {
        self.smoothing_ms = finite_or(milliseconds, 2.0).clamp(0.0, 200.0);
        self.update_smoothing();
    }

    pub fn reset(&mut self) {
        self.output_left = 0.0;
        self.output_right = 0.0;
        self.initialized = false;
    }

    pub fn process_frame(
        &mut self,
        left: f32,
        right: f32,
        cutoff_left_hz: f32,
        cutoff_right_hz: f32,
    ) -> (f32, f32) {
        let target_left = self.cutoff_coefficient(cutoff_left_hz);
        let target_right = if cutoff_left_hz == cutoff_right_hz {
            target_left
        } else {
            self.cutoff_coefficient(cutoff_right_hz)
        };
        if self.initialized {
            let coefficient = self.smoothing_coefficient;
            self.alpha_left = target_left + coefficient * (self.alpha_left - target_left);
            self.alpha_right = target_right + coefficient * (self.alpha_right - target_right);
        } else {
            self.alpha_left = target_left;
            self.alpha_right = target_right;
            self.initialized = true;
        }
        let left = audio_or_silence(left) as f64;
        let right = audio_or_silence(right) as f64;
        self.output_left += self.alpha_left * (left - self.output_left);
        self.output_right += self.alpha_right * (right - self.output_right);
        if self.output_left.abs() < 1.0e-20 {
            self.output_left = 0.0;
        }
        if self.output_right.abs() < 1.0e-20 {
            self.output_right = 0.0;
        }
        (self.output_left as f32, self.output_right as f32)
    }

    fn cutoff_coefficient(&self, cutoff_hz: f32) -> f64 {
        let maximum = self.sample_rate * 0.49;
        let cutoff = finite_or(cutoff_hz as f64, 20000.0).clamp(5.0_f64.min(maximum), maximum);
        1.0 - (-std::f64::consts::TAU * cutoff / self.sample_rate).exp()
    }

    fn update_smoothing(&mut self) {
        self.smoothing_coefficient = if self.smoothing_ms == 0.0 {
            0.0
        } else {
            (-1.0 / (self.sample_rate * self.smoothing_ms * 0.001)).exp()
        };
    }
}

/// Stereo balance: -1 is left, +1 is right, and center passes both channels
/// through exactly. This attenuates the opposite channel without crossfeeding.
pub fn stereo_balance(left: f32, right: f32, position: f32) -> (f32, f32) {
    let position = if position.is_finite() {
        position.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let left_gain = 1.0 - position.max(0.0);
    let right_gain = 1.0 + position.min(0.0);
    (
        audio_or_silence(left) * left_gain,
        audio_or_silence(right) * right_gain,
    )
}

/// Normalized soft saturation blended with dry audio. Amount 0 is exact
/// passthrough; amount 1 uses 16x tanh drive. A full-scale input remains 1.0
/// at all drive amounts. As a lightweight effect this does not oversample.
pub fn soft_clip(sample: f32, amount: f32) -> f32 {
    let sample = audio_or_silence(sample);
    let amount = unit_or(amount, 0.0);
    if amount == 0.0 {
        return sample;
    }
    let drive = 1.0 + amount as f64 * 15.0;
    let saturated = ((sample as f64 * drive).tanh() / drive.tanh()) as f32;
    sample * (1.0 - amount) + saturated * amount
}

fn valid_sample_rate(value: f64) -> f64 {
    finite_or(value, 44100.0).clamp(1.0, 768000.0)
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn unit_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

fn audio_or_silence(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn wrap_phase(value: f64) -> f64 {
    let wrapped = finite_or(value, 0.0).rem_euclid(1.0);
    // Floating-point rem_euclid can round a tiny negative input to exactly 1.
    if wrapped >= 1.0 {
        0.0
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} differs from {expected} by more than {tolerance}"
        );
    }

    fn cycle_close(actual: f64, expected: f64, tolerance: f64) {
        let delta = (actual - expected).abs();
        assert!(delta.min((1.0 - delta).abs()) < tolerance);
    }

    #[test]
    fn curve_includes_both_endpoints_and_clamps_outside_positions() {
        let curve = Curve::from_preset(Preset::Ramp);
        assert_eq!(curve.value(0.0), 0.0);
        assert_eq!(curve.value(1.0), 1.0);
        assert_eq!(curve.value(-3.0), 0.0);
        assert_eq!(curve.value(7.0), 1.0);
        for index in 0..POINT_COUNT {
            let phase = index as f64 / SEGMENT_COUNT as f64;
            assert_eq!(curve.value(phase), curve.points[index]);
        }
    }

    #[test]
    fn interpolation_has_expected_values_and_no_overshoot() {
        let mut curve = Curve {
            points: [1.0; POINT_COUNT],
            interpolation: Interpolation::Linear,
        };
        curve.points[0] = 0.0;
        let quarter_segment = 0.25 / SEGMENT_COUNT as f64;
        assert_eq!(curve.value(quarter_segment), 0.25);
        curve.interpolation = Interpolation::Smooth;
        assert_eq!(curve.value(quarter_segment), 0.15625);
        curve.interpolation = Interpolation::Step;
        assert_eq!(curve.value(0.999 / SEGMENT_COUNT as f64), 0.0);
        assert_eq!(curve.value(1.0 / SEGMENT_COUNT as f64), 1.0);
        for interpolation in [
            Interpolation::Linear,
            Interpolation::Smooth,
            Interpolation::Step,
        ] {
            curve.interpolation = interpolation;
            for index in 0..=1000 {
                assert!((0.0..=1.0).contains(&curve.value(index as f64 / 1000.0)));
            }
        }
    }

    #[test]
    fn all_presets_have_safe_values_and_intended_extremes() {
        for preset in [
            Preset::Pump,
            Preset::Gate,
            Preset::Sine,
            Preset::Ramp,
            Preset::Flat,
        ] {
            let curve = Curve::from_preset(preset);
            assert!(curve.points.iter().all(|value| (0.0..=1.0).contains(value)));
        }
        assert_eq!(Curve::from_preset(Preset::Pump).value(0.0), 0.0);
        assert_eq!(Curve::from_preset(Preset::Pump).value(1.0), 1.0);
        assert_eq!(Curve::from_preset(Preset::Gate).value(0.49), 1.0);
        assert_eq!(Curve::from_preset(Preset::Gate).value(0.5), 0.0);
        close(
            Curve::from_preset(Preset::Sine).value(0.5) as f64,
            1.0,
            1e-7,
        );
        assert_eq!(Curve::from_preset(Preset::Flat).value(0.72), 1.0);
    }

    #[test]
    fn tempo_sync_is_consistent_at_multiple_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            let mut clock = PhaseClock::new(sample_rate);
            clock.set_tempo(120.0);
            clock.set_rate(Rate::Synced { beats: 4.0 });
            for _ in 0..sample_rate as usize {
                clock.advance();
            }
            close(clock.phase(), 0.5, 1e-10);
            for _ in 0..sample_rate as usize {
                clock.advance();
            }
            cycle_close(clock.phase(), 0.0, 1e-10);
        }
    }

    #[test]
    fn sync_seek_handles_preroll_loops_and_tempo_changes() {
        let mut clock = PhaseClock::new(48000.0);
        clock.set_rate(Rate::Synced { beats: 4.0 });
        clock.seek_beats(-1.0);
        assert_eq!(clock.phase(), 0.75);
        clock.seek_beats(33.0);
        assert_eq!(clock.phase(), 0.25);
        clock.seek_beats(0.0);
        assert_eq!(clock.phase(), 0.0);
        clock.set_tempo(60.0);
        for _ in 0..48000 {
            clock.advance();
        }
        close(clock.phase(), 0.25, 1e-11);
        clock.set_tempo(120.0);
        for _ in 0..24000 {
            clock.advance();
        }
        close(clock.phase(), 0.5, 1e-11);
    }

    #[test]
    fn free_rate_is_independent_of_tempo_and_transport_seek() {
        let mut clock = PhaseClock::new(48000.0);
        clock.set_rate(Rate::Free { hz: 3.0 });
        clock.set_tempo(190.0);
        for _ in 0..4000 {
            clock.advance();
        }
        close(clock.phase(), 0.25, 1e-12);
        clock.seek_beats(99.0);
        close(clock.phase(), 0.25, 1e-12);
        clock.set_rate(Rate::Free { hz: 0.0 });
        let phase = clock.phase();
        clock.advance();
        assert_eq!(clock.phase(), phase);
    }

    #[test]
    fn zero_depth_is_exact_passthrough_even_after_attenuation() {
        let curve = Curve::from_preset(Preset::Ramp);
        let mut shaper = VolumeShaper::new(48000.0);
        shaper.set_smoothing_ms(0.0);
        assert_eq!(
            shaper.process_frame(0.5, -0.3, &curve, 1.0, 0.0, 0.0),
            (0.0, -0.0)
        );
        shaper.set_smoothing_ms(20.0);
        assert_eq!(
            shaper.process_frame(0.5, -0.3, &curve, 0.0, 0.0, 0.0),
            (0.5, -0.3)
        );
    }

    #[test]
    fn stereo_offset_wraps_and_depth_blends_toward_unity() {
        let curve = Curve::from_preset(Preset::Ramp);
        let mut shaper = VolumeShaper::new(48000.0);
        shaper.set_smoothing_ms(0.0);
        assert_eq!(
            shaper.process_frame(1.0, 1.0, &curve, 1.0, 0.25, 0.5),
            (0.25, 0.75)
        );
        assert_eq!(
            shaper.process_frame(1.0, 1.0, &curve, 1.0, 0.75, 0.5),
            (0.75, 0.25)
        );
        assert_eq!(
            shaper.process_frame(1.0, 1.0, &curve, 0.5, 0.0, -0.5),
            (0.5, 0.75)
        );
        assert_eq!(
            shaper.process_frame(1.0, 1.0, &curve, 1.0, 1.0, 0.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn smoothing_has_same_time_constant_at_different_sample_rates() {
        let curve = Curve::from_preset(Preset::Ramp);
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            let mut shaper = VolumeShaper::new(sample_rate);
            shaper.set_smoothing_ms(10.0);
            let mut result = (1.0, 1.0);
            for _ in 0..(sample_rate * 0.01) as usize {
                result = shaper.process_frame(1.0, 1.0, &curve, 1.0, 0.0, 0.0);
            }
            close(result.0 as f64, (-1.0_f64).exp(), 2e-5);
            assert_eq!(result.0, result.1);
        }
    }

    #[test]
    fn malformed_curve_parameters_and_audio_remain_finite() {
        let mut curve = Curve::from_preset(Preset::Ramp);
        curve.points[0] = f32::NAN;
        curve.points[1] = f32::INFINITY;
        curve.points[2] = -4.0;
        curve.points[3] = 4.0;
        for phase in [
            f64::NAN,
            f64::INFINITY,
            -f64::INFINITY,
            -1e-30,
            0.0,
            0.13,
            1.0,
        ] {
            assert!(curve.value(phase).is_finite());
        }
        let mut clock = PhaseClock::new(f64::NAN);
        clock.set_sample_rate(0.0);
        clock.set_tempo(f64::INFINITY);
        clock.set_rate(Rate::Synced { beats: f64::NAN });
        clock.seek_beats(f64::INFINITY);
        for _ in 0..1000 {
            assert!((0.0..1.0).contains(&clock.advance()));
        }
        let mut shaper = VolumeShaper::new(f64::NAN);
        shaper.set_sample_rate(f64::INFINITY);
        shaper.set_smoothing_ms(f64::NAN);
        let result = shaper.process_frame(
            f32::NAN,
            f32::INFINITY,
            &curve,
            f32::NAN,
            f64::NAN,
            f64::INFINITY,
        );
        assert_eq!(result, (0.0, 0.0));
        let result = shaper.process_frame(1.0, -1.0, &curve, 1.0, f64::NAN, f64::INFINITY);
        assert!(result.0.is_finite() && result.1.is_finite());
    }

    #[test]
    fn stereo_balance_has_unity_center_and_correct_channel_extremes() {
        assert_eq!(stereo_balance(0.25, -0.75, 0.0), (0.25, -0.75));
        assert_eq!(stereo_balance(0.25, -0.75, -1.0), (0.25, -0.0));
        assert_eq!(stereo_balance(0.25, -0.75, 1.0), (0.0, -0.75));
        assert_eq!(stereo_balance(0.25, -0.75, f32::NAN), (0.25, -0.75));
        assert_eq!(stereo_balance(f32::NAN, f32::INFINITY, 0.0), (0.0, 0.0));
    }

    #[test]
    fn soft_clip_is_normalized_symmetric_and_bypasses_at_zero() {
        for sample in [-2.0, -1.0, -0.1, 0.0, 0.1, 1.0, 2.0] {
            assert_eq!(soft_clip(sample, 0.0), sample);
            for amount in [0.1, 0.5, 1.0] {
                assert_eq!(soft_clip(-sample, amount), -soft_clip(sample, amount));
                assert!(soft_clip(sample, amount).is_finite());
            }
        }
        for amount in [0.0, 0.1, 0.5, 1.0] {
            close(soft_clip(1.0, amount) as f64, 1.0, 1e-7);
        }
        assert!(soft_clip(0.1, 1.0) > 0.5);
        assert!(soft_clip(f32::MAX, 1.0) <= 1.00001);
        assert_eq!(soft_clip(f32::NAN, 1.0), 0.0);
        assert_eq!(soft_clip(0.3, f32::NAN), 0.3);
    }

    #[test]
    fn lowpass_passes_dc_and_does_not_leak_between_channels() {
        let mut filter = StereoLowpass::new(48000.0);
        let mut result = (0.0, 0.0);
        for _ in 0..48000 {
            result = filter.process_frame(0.5, 0.0, 100.0, 100.0);
        }
        close(result.0 as f64, 0.5, 1e-7);
        assert_eq!(result.1, 0.0);
        filter.reset();
        assert_eq!(filter.process_frame(0.0, 0.0, 100.0, 100.0), (0.0, 0.0));
    }

    #[test]
    fn lowpass_attenuates_high_frequencies_at_multiple_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            let mut filter = StereoLowpass::new(sample_rate);
            filter.set_smoothing_ms(0.0);
            let mut energy_low = 0.0_f64;
            let mut energy_high = 0.0_f64;
            let frames = sample_rate as usize;
            for index in 0..frames {
                let phase = index as f64 / sample_rate * std::f64::consts::TAU;
                let result = filter.process_frame(
                    (phase * 100.0).sin() as f32,
                    (phase * 10000.0).sin() as f32,
                    1000.0,
                    1000.0,
                );
                if index >= frames / 2 {
                    energy_low += (result.0 as f64).powi(2);
                    energy_high += (result.1 as f64).powi(2);
                }
            }
            let amplitude_low = (energy_low * 4.0 / frames as f64).sqrt();
            let amplitude_high = (energy_high * 4.0 / frames as f64).sqrt();
            assert!(amplitude_low > 0.98 && amplitude_low <= 1.0);
            assert!(amplitude_high < 0.12);
        }
    }

    #[test]
    fn lowpass_safely_handles_malformed_values_and_changes_in_sample_rate() {
        let mut filter = StereoLowpass::new(f64::NAN);
        filter.set_smoothing_ms(f64::NAN);
        for sample_rate in [0.0, f64::INFINITY, 44100.0] {
            filter.set_sample_rate(sample_rate);
            for cutoff in [f32::NAN, f32::INFINITY, -100.0, 0.0, 100000.0] {
                let result = filter.process_frame(f32::NAN, f32::INFINITY, cutoff, cutoff);
                assert_eq!(result, (0.0, 0.0));
                let result = filter.process_frame(f32::MAX, -f32::MAX, cutoff, cutoff);
                assert!(result.0.is_finite() && result.1.is_finite());
                filter.reset();
            }
        }
    }
}
