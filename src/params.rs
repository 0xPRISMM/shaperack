use crate::dsp::{Curve, Interpolation, Preset, POINT_COUNT};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveStyle {
    Linear,
    Smooth,
    Step,
}

impl CurveStyle {
    pub fn interpolation(self) -> Interpolation {
        match self {
            Self::Linear => Interpolation::Linear,
            Self::Smooth => Interpolation::Smooth,
            Self::Step => Interpolation::Step,
        }
    }
}

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Division {
    #[name = "1/32"]
    ThirtySecond,
    #[name = "1/16"]
    Sixteenth,
    #[name = "1/8"]
    Eighth,
    #[name = "1/4"]
    Quarter,
    #[name = "1/2"]
    Half,
    #[name = "1 bar (4 beats)"]
    Bar,
    #[name = "2 bars (8 beats)"]
    TwoBars,
    #[name = "4 bars (16 beats)"]
    FourBars,
}

impl Division {
    pub fn beats(self) -> f64 {
        match self {
            Self::ThirtySecond => 0.125,
            Self::Sixteenth => 0.25,
            Self::Eighth => 0.5,
            Self::Quarter => 1.0,
            Self::Half => 2.0,
            Self::Bar => 4.0,
            Self::TwoBars => 8.0,
            Self::FourBars => 16.0,
        }
    }
}

#[derive(Params)]
pub struct CurvePoint {
    #[id = "value"]
    pub value: FloatParam,
}

#[derive(Params)]
pub struct ModuleParams {
    #[id = "enabled"]
    pub enabled: BoolParam,
    #[id = "amount"]
    pub amount: FloatParam,
    #[id = "style"]
    pub style: EnumParam<CurveStyle>,
    #[nested(array)]
    pub points: [CurvePoint; POINT_COUNT],
}

impl ModuleParams {
    fn new(name: &str, enabled: bool, amount: f32, preset: Preset) -> Self {
        let curve = Curve::from_preset(preset);
        Self {
            enabled: BoolParam::new(format!("{name} enabled"), enabled),
            amount: percent(&format!("{name} amount"), amount),
            style: EnumParam::new(
                format!("{name} interpolation"),
                match curve.interpolation {
                    Interpolation::Linear => CurveStyle::Linear,
                    Interpolation::Smooth => CurveStyle::Smooth,
                    Interpolation::Step => CurveStyle::Step,
                },
            ),
            points: std::array::from_fn(|i| CurvePoint {
                value: FloatParam::new(
                    format!("{name} point {i:02}"),
                    curve.points[i],
                    FloatRange::Linear { min: 0.0, max: 1.0 },
                ),
            }),
        }
    }

    pub fn curve(&self) -> Curve {
        Curve {
            points: std::array::from_fn(|i| self.points[i].value.value()),
            interpolation: self.style.value().interpolation(),
        }
    }
}

#[derive(Params)]
pub struct ShapeRackParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    #[id = "sync"]
    pub sync: BoolParam,
    #[id = "division"]
    pub division: EnumParam<Division>,
    #[id = "free-hz"]
    pub free_hz: FloatParam,
    #[id = "offset"]
    pub offset: FloatParam,
    #[id = "stereo"]
    pub stereo: FloatParam,
    #[id = "smooth-ms"]
    pub smoothing: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "bypass"]
    pub bypass: BoolParam,
    #[nested(id_prefix = "volume", group = "Volume")]
    pub volume: ModuleParams,
    #[nested(id_prefix = "filter", group = "Filter")]
    pub filter: ModuleParams,
    #[nested(id_prefix = "pan", group = "Pan")]
    pub pan: ModuleParams,
    #[nested(id_prefix = "drive", group = "Drive")]
    pub drive: ModuleParams,
}

fn percent(name: &str, value: f32) -> FloatParam {
    FloatParam::new(name, value, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(10.0))
        .with_unit("%")
        .with_value_to_string(Arc::new(|v| format!("{:.0}", v * 100.0)))
        .with_string_to_value(Arc::new(|s| {
            s.trim()
                .trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .map(|v| v / 100.0)
        }))
}

impl Default for ShapeRackParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(960, 640),
            sync: BoolParam::new("Tempo sync", true),
            division: EnumParam::new("Cycle length", Division::Quarter),
            free_hz: FloatParam::new(
                "Free rate",
                2.0,
                FloatRange::Skewed {
                    min: 0.05,
                    max: 20.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(Arc::new(|v| format!("{v:.2}"))),
            offset: percent("Phase offset", 0.0),
            stereo: percent("Right channel offset", 0.0),
            smoothing: FloatParam::new(
                "De-click",
                2.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 30.0,
                },
            )
            .with_unit(" ms")
            .with_value_to_string(Arc::new(|v| format!("{v:.1}"))),
            mix: percent("Mix", 1.0),
            output: FloatParam::new(
                "Output",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 12.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" dB")
            .with_value_to_string(Arc::new(|v| format!("{v:.1}"))),
            bypass: BoolParam::new("Bypass", false).make_bypass(),
            volume: ModuleParams::new("Volume", true, 1.0, Preset::Pump),
            filter: ModuleParams::new("Filter", false, 0.7, Preset::Sine),
            pan: ModuleParams::new("Pan", false, 0.8, Preset::Sine),
            drive: ModuleParams::new("Drive", false, 0.5, Preset::Ramp),
        }
    }
}

impl ShapeRackParams {
    pub fn module(&self, index: usize) -> &ModuleParams {
        match index {
            1 => &self.filter,
            2 => &self.pan,
            3 => &self.drive,
            _ => &self.volume,
        }
    }
}
