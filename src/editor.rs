//! Native, host-automatable curve editor. The audio callback never touches UI state.

use crate::dsp::{Curve, Preset, SEGMENT_COUNT};
use crate::params::{CurveStyle, Division, ModuleParams, ShapeRackParams};
use crate::Meters;
use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, pos2, vec2, Color32, FontId, Rect, RichText, Sense, Stroke},
    resizable_window::ResizableWindow,
};
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::time::Duration;

const BACKGROUND: Color32 = Color32::from_rgb(16, 20, 28);
const PANEL: Color32 = Color32::from_rgb(24, 30, 40);
const TEXT: Color32 = Color32::from_rgb(228, 235, 245);
const MUTED: Color32 = Color32::from_rgb(137, 151, 171);
const MINT: Color32 = Color32::from_rgb(108, 231, 190);
const LAVENDER: Color32 = Color32::from_rgb(174, 162, 255);
const BLUE: Color32 = Color32::from_rgb(115, 188, 250);
const ORANGE: Color32 = Color32::from_rgb(255, 185, 120);

const NAMES: [&str; 4] = ["Volume", "Filter", "Pan", "Drive"];
const COLORS: [Color32; 4] = [MINT, LAVENDER, BLUE, ORANGE];
const DESCRIPTIONS: [&str; 4] = [
    "Shape gain for pumping, gates, and rhythmic movement.",
    "Move a low-pass cutoff from dark to open.",
    "Move the stereo balance from left to right.",
    "Animate soft saturation from clean to driven.",
];

#[derive(Default)]
struct UiState {
    module: usize,
    nodes_mode: bool,
    gesture_module: Option<usize>,
    previous: Option<(usize, f32)>,
    node: Option<usize>,
    slider_drag: Option<SliderDrag>,
    value_entry: Option<ValueEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SliderKey {
    Amount(usize),
    FreeHz,
    Smoothing,
    Offset,
    Stereo,
    Mix,
    Output,
}

struct SliderDrag {
    key: SliderKey,
    initial_value: f32,
    initial_x: f32,
}

struct ValueEntry {
    key: SliderKey,
    text: String,
    focus: bool,
}

pub fn create(params: Arc<ShapeRackParams>, meters: Arc<Meters>) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    let state = Arc::new(Mutex::new(UiState::default()));
    let wrapper_params = params.clone();
    let wrapper_state = state.clone();
    let inner = create_egui_editor(
        editor_state.clone(),
        state,
        |context, _| {
            let mut style = (*context.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = BACKGROUND;
            style.visuals.window_fill = PANEL;
            style.visuals.extreme_bg_color = BACKGROUND;
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.selection.bg_fill = MINT.gamma_multiply(0.22);
            style.visuals.selection.stroke = Stroke::new(1.0_f32, MINT);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 43, 56);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 57, 71);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(55, 69, 82);
            style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
            style.spacing.item_spacing = vec2(8.0, 7.0);
            style.spacing.button_padding = vec2(12.0, 7.0);
            style.spacing.interact_size = vec2(40.0, 28.0);
            style
                .text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, FontId::proportional(11.0));
            context.set_style(style);
        },
        move |context, setter, state| {
            let mut locked_state = state.lock().unwrap_or_else(|error| error.into_inner());
            let state = &mut *locked_state;
            // Finish every point's automation gesture even if release occurs outside the canvas.
            if state.gesture_module.is_some()
                && !context.input(|input| input.pointer.primary_down())
            {
                end_curve_gesture(&params, setter, state);
            }
            if state.slider_drag.is_some() && !context.input(|input| input.pointer.primary_down()) {
                end_slider_gesture(&params, setter, state);
            }
            ResizableWindow::new("shaperack-window")
                .min_size(vec2(820.0, 620.0))
                .show(context, editor_state.as_ref(), |ui| {
                    egui::Frame::NONE.inner_margin(12.0).show(ui, |ui| {
                        header(ui, &params, &meters, setter);
                        ui.add_space(12.0);
                        module_tabs(ui, &params, state);
                        ui.add_space(12.0);
                        curve_section(ui, &params, &meters, setter, state);
                        ui.add_space(12.0);
                        global_controls(ui, &params, setter, state);
                        ui.add_space(7.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Curve modulation · VST3 / CLAP")
                                    .small()
                                    .color(MUTED),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("Drag corner to resize  ↘")
                                            .small()
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                    });
                });
            context.request_repaint_after(Duration::from_millis(16));
        },
    )?;
    Some(Box::new(GestureSafeEditor {
        inner,
        params: wrapper_params,
        state: wrapper_state,
    }))
}

/// The host drops this handle on the GUI thread. Ending gestures here also covers
/// closing the plugin window while the pointer is still down on the canvas.
struct GestureSafeEditor {
    inner: Box<dyn Editor>,
    params: Arc<ShapeRackParams>,
    state: Arc<Mutex<UiState>>,
}

impl Editor for GestureSafeEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        Box::new(GestureSafeHandle {
            inner: Some(self.inner.spawn(parent, context.clone())),
            context,
            params: self.params.clone(),
            state: self.state.clone(),
        })
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }
    fn set_scale_factor(&self, factor: f32) -> bool {
        self.inner.set_scale_factor(factor)
    }
    fn param_value_changed(&self, id: &str, value: f32) {
        self.inner.param_value_changed(id, value);
    }
    fn param_modulation_changed(&self, id: &str, value: f32) {
        self.inner.param_modulation_changed(id, value);
    }
    fn param_values_changed(&self) {
        self.inner.param_values_changed();
    }
}

struct GestureSafeHandle {
    inner: Option<Box<dyn std::any::Any + Send>>,
    context: Arc<dyn GuiContext>,
    params: Arc<ShapeRackParams>,
    state: Arc<Mutex<UiState>>,
}

impl Drop for GestureSafeHandle {
    fn drop(&mut self) {
        drop(self.inner.take());
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        end_curve_gesture(
            &self.params,
            &ParamSetter::new(self.context.as_ref()),
            &mut state,
        );
        end_slider_gesture(
            &self.params,
            &ParamSetter::new(self.context.as_ref()),
            &mut state,
        );
        state.value_entry = None;
    }
}

fn header(ui: &mut egui::Ui, params: &ShapeRackParams, meters: &Meters, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, MINT.gamma_multiply(0.12));
        let points: Vec<_> = (0..=28)
            .map(|index| {
                let x = index as f32 / 28.0;
                let y = (x * std::f32::consts::TAU).sin();
                pos2(rect.left() + 6.0 + x * 26.0, rect.center().y - y * 9.0)
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.4_f32, MINT)));
        ui.vertical(|ui| {
            ui.label(RichText::new("ShapeRack").size(25.0).strong());
            ui.label(RichText::new("DRAW YOUR RHYTHM").size(10.0).color(MUTED));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toggle_button(
                ui,
                &params.bypass,
                setter,
                "Bypass",
                "Pass the incoming audio through.",
            );
            ui.add_space(8.0);
            let playing = meters.playing.load(Ordering::Relaxed);
            ui.label(
                RichText::new(if playing {
                    "● PLAYING"
                } else {
                    "● STOPPED"
                })
                .small()
                .color(if playing { MINT } else { MUTED }),
            );
            ui.add_space(8.0);
            let tempo = meters.tempo.load(Ordering::Relaxed);
            ui.label(
                RichText::new(format!("{tempo:.1} BPM"))
                    .monospace()
                    .color(TEXT),
            );
            ui.add_space(8.0);
            output_meter(ui, meters);
        });
    });
}

fn output_meter(ui: &mut egui::Ui, meters: &Meters) {
    let peak = meters.peak.load(Ordering::Relaxed).max(0.0);
    let db = if peak > 1.0e-6 {
        20.0 * peak.log10()
    } else {
        -120.0
    };
    let amount = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(94.0, 32.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.text(
        rect.min,
        egui::Align2::LEFT_TOP,
        if db > -100.0 {
            format!("OUT  {db:.1} dB")
        } else {
            "OUT  −∞ dB".to_owned()
        },
        FontId::monospace(10.0),
        MUTED,
    );
    let track = Rect::from_min_size(rect.min + vec2(0.0, 22.0), vec2(rect.width(), 4.0));
    painter.rect_filled(track, 2.0, Color32::from_rgb(43, 52, 64));
    if amount > 0.0 {
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(track.width() * amount, 4.0)),
            2.0,
            if peak >= 1.0 { ORANGE } else { MINT },
        );
    }
    response
        .on_hover_text("Output peak level in dBFS. Orange indicates levels at or above 0 dBFS.");
}

fn module_tabs(ui: &mut egui::Ui, params: &ShapeRackParams, state: &mut UiState) {
    let width = (ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0;
    ui.horizontal(|ui| {
        for index in 0..4 {
            let module = params.module(index);
            let color = COLORS[index];
            let selected = state.module == index;
            let enabled = module.enabled.value();
            let label = format!("{}  {}", if enabled { "●" } else { "○" }, NAMES[index]);
            let button =
                egui::Button::new(RichText::new(label).strong().color(if selected || enabled {
                    color
                } else {
                    MUTED
                }))
                .fill(if selected {
                    color.gamma_multiply(0.12)
                } else {
                    PANEL
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected {
                        color.gamma_multiply(0.65)
                    } else {
                        Color32::from_rgb(37, 46, 60)
                    },
                ));
            if ui
                .add_sized(vec2(width, 40.0), button)
                .on_hover_text(DESCRIPTIONS[index])
                .clicked()
                && state.gesture_module.is_none()
            {
                state.module = index;
            }
        }
    });
}

fn curve_section(
    ui: &mut egui::Ui,
    params: &ShapeRackParams,
    meters: &Meters,
    setter: &ParamSetter,
    state: &mut UiState,
) {
    let module = params.module(state.module);
    let color = COLORS[state.module];
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} curve", NAMES[state.module]))
                .size(17.0)
                .strong()
                .color(color),
        );
        toggle_button(
            ui,
            &module.enabled,
            setter,
            "Enabled",
            "Activate this module in the rack.",
        );
        ui.label(RichText::new("Amount").color(MUTED));
        host_slider(
            ui,
            &module.amount,
            setter,
            state,
            SliderKey::Amount(state.module),
            135.0,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (style, label) in [
                (CurveStyle::Step, "Step"),
                (CurveStyle::Smooth, "Smooth"),
                (CurveStyle::Linear, "Linear"),
            ] {
                if ui
                    .selectable_label(module.style.value() == style, label)
                    .clicked()
                {
                    set_discrete(setter, &module.style, style);
                }
            }
        });
    });
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Start with").small().color(MUTED));
        for (preset, label) in [
            (Preset::Pump, "Pump"),
            (Preset::Gate, "Gate"),
            (Preset::Sine, "Sine"),
            (Preset::Ramp, "Ramp"),
            (Preset::Flat, "Flat"),
        ] {
            if ui.small_button(label).clicked() {
                apply_preset(module, setter, preset);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .selectable_label(state.nodes_mode, "Nodes")
                .on_hover_text("Drag one of the 17 curve values.")
                .clicked()
            {
                state.nodes_mode = true;
            }
            if ui
                .selectable_label(!state.nodes_mode, "Draw")
                .on_hover_text("Paint across the grid to redraw several points.")
                .clicked()
            {
                state.nodes_mode = false;
            }
        });
    });
    ui.add_space(7.0);
    let graph_height = (ui.available_height() - 173.0).max(165.0);
    draw_canvas(ui, params, meters, setter, state, graph_height);
}

fn draw_canvas(
    ui: &mut egui::Ui,
    params: &ShapeRackParams,
    meters: &Meters,
    setter: &ParamSetter,
    state: &mut UiState,
    height: f32,
) {
    let module = params.module(state.module);
    let accent = COLORS[state.module];
    let (outer, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let rect = Rect::from_min_max(outer.min + vec2(38.0, 16.0), outer.max - vec2(18.0, 28.0));
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 8.0, Color32::from_rgb(12, 17, 24));
    painter.rect_stroke(
        outer,
        8.0,
        Stroke::new(1.0_f32, Color32::from_rgb(41, 51, 66)),
        egui::StrokeKind::Inside,
    );

    if response.is_pointer_button_down_on() && state.gesture_module.is_none() {
        for point in &module.points {
            setter.begin_set_parameter(&point.value);
        }
        state.gesture_module = Some(state.module);
        state.previous = None;
        state.node = None;
    }
    if state.gesture_module == Some(state.module) {
        if let Some(position) = response.interact_pointer_pos() {
            let x = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let value = (1.0 - (position.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let index = (x * SEGMENT_COUNT as f32).round() as usize;
            if state.nodes_mode {
                let node = *state.node.get_or_insert(index);
                setter.set_parameter(&module.points[node].value, value);
            } else {
                if let Some((previous_index, previous_value)) = state.previous {
                    let first = previous_index.min(index);
                    let last = previous_index.max(index);
                    for node in first..=last {
                        let t = if previous_index == index {
                            1.0
                        } else {
                            (node as f32 - previous_index as f32)
                                / (index as f32 - previous_index as f32)
                        };
                        setter.set_parameter(
                            &module.points[node].value,
                            previous_value + (value - previous_value) * t,
                        );
                    }
                } else {
                    setter.set_parameter(&module.points[index].value, value);
                }
                state.previous = Some((index, value));
            }
        }
    }

    for index in 0..=SEGMENT_COUNT {
        let x = rect.left() + rect.width() * index as f32 / SEGMENT_COUNT as f32;
        let major = index % 4 == 0;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(
                1.0_f32,
                if major {
                    Color32::from_rgb(43, 53, 67)
                } else {
                    Color32::from_rgb(27, 35, 46)
                },
            ),
        );
        if major {
            painter.text(
                pos2(x, rect.bottom() + 8.0),
                egui::Align2::CENTER_TOP,
                format!("{}%", index * 100 / SEGMENT_COUNT),
                FontId::proportional(10.0),
                MUTED,
            );
        }
    }
    for index in 0..=4 {
        let y = rect.top() + rect.height() * index as f32 / 4.0;
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0_f32, Color32::from_rgb(33, 43, 56)),
        );
        painter.text(
            pos2(rect.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{}", 100 - index * 25),
            FontId::proportional(10.0),
            MUTED,
        );
    }

    let curve = module.curve();
    // Draw using the same evaluator as the DSP, including exact step corners.
    let mut line = Vec::with_capacity(SEGMENT_COUNT * 17 + 1);
    if module.style.value() == CurveStyle::Step {
        for index in 0..SEGMENT_COUNT {
            let x = index as f32 / SEGMENT_COUNT as f32;
            let next_x = (index + 1) as f32 / SEGMENT_COUNT as f32;
            line.push(curve_position(rect, x, curve.points[index]));
            line.push(curve_position(rect, next_x, curve.points[index]));
        }
        line.push(curve_position(rect, 1.0, curve.points[SEGMENT_COUNT]));
    } else {
        for index in 0..=256 {
            let x = index as f32 / 256.0;
            line.push(curve_position(rect, x, curve.value(x as f64)));
        }
    }
    for pair in line.windows(2) {
        let polygon = vec![
            pair[0],
            pair[1],
            pos2(pair[1].x, rect.bottom()),
            pos2(pair[0].x, rect.bottom()),
        ];
        painter.add(egui::Shape::convex_polygon(
            polygon,
            accent.gamma_multiply(0.075),
            Stroke::NONE,
        ));
    }
    painter.add(egui::Shape::line(
        line.clone(),
        Stroke::new(6.0_f32, accent.gamma_multiply(0.08)),
    ));
    painter.add(egui::Shape::line(line, Stroke::new(2.2_f32, accent)));
    let phase = meters.phase.load(Ordering::Relaxed).clamp(0.0, 1.0);
    let x = rect.left() + phase * rect.width();
    painter.line_segment(
        [pos2(x, rect.top()), pos2(x, rect.bottom())],
        Stroke::new(1.2_f32, TEXT.gamma_multiply(0.7)),
    );
    painter.circle_filled(
        curve_position(rect, phase, curve.value(phase as f64)),
        4.0,
        TEXT,
    );

    let hover_index = response.hover_pos().map(|position| {
        (((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * SEGMENT_COUNT as f32).round()
            as usize
    });
    for (index, value) in curve.points.iter().enumerate() {
        let position = curve_position(rect, index as f32 / SEGMENT_COUNT as f32, *value);
        let hovered = hover_index == Some(index);
        painter.circle_filled(position, if hovered { 5.0 } else { 3.0 }, BACKGROUND);
        painter.circle_stroke(
            position,
            if hovered { 5.0 } else { 3.0 },
            Stroke::new(if hovered { 2.0_f32 } else { 1.3_f32 }, accent),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    response.on_hover_text(if state.nodes_mode {
        "Drag to edit a point. Switch to Draw to paint a curve."
    } else {
        "Drag across the canvas to draw. Each module keeps its own curve."
    });
}

fn curve_position(rect: Rect, x: f32, y: f32) -> egui::Pos2 {
    pos2(
        rect.left() + rect.width() * x,
        rect.bottom() - rect.height() * y,
    )
}

fn global_controls(
    ui: &mut egui::Ui,
    params: &ShapeRackParams,
    setter: &ParamSetter,
    state: &mut UiState,
) {
    egui::Frame::NONE
        .fill(PANEL)
        .corner_radius(8.0)
        .inner_margin(11.0)
        .show(ui, |ui| {
            ui.columns(3, |columns| {
                columns[0].label(RichText::new("CLOCK").small().strong().color(MUTED));
                columns[0].horizontal(|ui| {
                    for (value, label) in [(true, "Sync"), (false, "Free")] {
                        if ui
                            .selectable_label(params.sync.value() == value, label)
                            .clicked()
                        {
                            set_discrete(setter, &params.sync, value);
                        }
                    }
                    if params.sync.value() {
                        division_selector(ui, params, setter);
                    } else {
                        let width = (ui.available_width() - 86.0).max(45.0);
                        host_slider(ui, &params.free_hz, setter, state, SliderKey::FreeHz, width);
                    }
                });
                labeled_slider(
                    &mut columns[0],
                    "De-click",
                    &params.smoothing,
                    setter,
                    state,
                    SliderKey::Smoothing,
                    "Smooth curve transitions to reduce clicks.",
                );
                columns[1].label(RichText::new("MOVEMENT").small().strong().color(MUTED));
                labeled_slider(
                    &mut columns[1],
                    "Phase",
                    &params.offset,
                    setter,
                    state,
                    SliderKey::Offset,
                    "Shift all curves within their cycle.",
                );
                labeled_slider(
                    &mut columns[1],
                    "Stereo",
                    &params.stereo,
                    setter,
                    state,
                    SliderKey::Stereo,
                    "Shift the right channel's volume, filter, and drive curves.",
                );
                columns[2].label(RichText::new("OUTPUT").small().strong().color(MUTED));
                labeled_slider(
                    &mut columns[2],
                    "Mix",
                    &params.mix,
                    setter,
                    state,
                    SliderKey::Mix,
                    "Blend the dry signal with the processed rack.",
                );
                labeled_slider(
                    &mut columns[2],
                    "Gain",
                    &params.output,
                    setter,
                    state,
                    SliderKey::Output,
                    "Adjust the final output level.",
                );
            });
        });
}

fn labeled_slider(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    setter: &ParamSetter,
    state: &mut UiState,
    key: SliderKey,
    tooltip: &str,
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            vec2(55.0, 24.0),
            egui::Label::new(RichText::new(label).color(MUTED)),
        );
        let slider_width = (ui.available_width() - 86.0).max(55.0);
        host_slider(ui, param, setter, state, key, slider_width).on_hover_text(tooltip);
    });
}

/// Own the gesture from pointer press, including clicks that never become drags.
/// The framework's ParamSlider currently writes those clicks before its drag gesture begins.
fn host_slider(
    ui: &mut egui::Ui,
    param: &FloatParam,
    setter: &ParamSetter,
    state: &mut UiState,
    key: SliderKey,
    width: f32,
) -> egui::Response {
    ui.push_id(key, |ui| {
        ui.horizontal(|ui| {
            let (rect, mut response) =
                ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
            let modifiers = ui.input(|input| input.modifiers);
            let down = response.is_pointer_button_down_on()
                && ui.input(|input| input.pointer.primary_down());
            if down && state.slider_drag.is_none() && state.gesture_module.is_none() {
                setter.begin_set_parameter(param);
                state.slider_drag = Some(SliderDrag {
                    key,
                    initial_value: param.unmodulated_normalized_value(),
                    initial_x: response
                        .interact_pointer_pos()
                        .map(|position| position.x)
                        .unwrap_or(rect.center().x),
                });
            }
            if down {
                if let (Some(drag), Some(position)) =
                    (&state.slider_drag, response.interact_pointer_pos())
                {
                    if drag.key == key {
                        let normalized = if modifiers.command {
                            param.default_normalized_value()
                        } else if modifiers.shift {
                            drag.initial_value + (position.x - drag.initial_x) * 0.0015
                        } else {
                            (position.x - rect.left()) / rect.width()
                        }
                        .clamp(0.0, 1.0);
                        let value = param.preview_plain(normalized);
                        if value != param.value() {
                            setter.set_parameter(param, value);
                            response.mark_changed();
                        }
                    }
                }
            }
            // A complete press/release can be delivered in one frame. That still
            // needs a balanced host gesture, as does double-click/Control reset.
            if response.double_clicked() || (response.clicked() && modifiers.command) {
                set_discrete(setter, param, param.default_plain_value());
                response.mark_changed();
            } else if response.clicked() && state.slider_drag.is_none() && !modifiers.shift {
                if let Some(position) = response.interact_pointer_pos() {
                    let normalized = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let value = param.preview_plain(normalized);
                    if value != param.value() {
                        set_discrete(setter, param, value);
                        response.mark_changed();
                    }
                }
            }
            let accent = if let SliderKey::Amount(index) = key {
                COLORS[index]
            } else {
                MINT
            };
            let track = rect.shrink2(vec2(0.0, 3.0));
            let painter = ui.painter();
            painter.rect_filled(track, 3.0, Color32::from_rgb(37, 46, 59));
            let amount = param.modulated_normalized_value();
            if amount > 0.0 {
                painter.rect_filled(
                    Rect::from_min_size(track.min, vec2(track.width() * amount, track.height())),
                    3.0,
                    accent.gamma_multiply(0.3),
                );
            }
            painter.line_segment(
                [
                    pos2(track.left() + track.width() * amount, track.top() + 2.0),
                    pos2(track.left() + track.width() * amount, track.bottom() - 2.0),
                ],
                Stroke::new(2.0_f32, accent),
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }

            let editing = state
                .value_entry
                .as_ref()
                .is_some_and(|entry| entry.key == key);
            let mut finish_entry = false;
            if editing {
                let entry = state.value_entry.as_mut().unwrap();
                let entry_response = ui.add_sized(
                    vec2(76.0, 24.0),
                    egui::TextEdit::singleline(&mut entry.text)
                        .id_salt("value-entry")
                        .font(egui::TextStyle::Monospace)
                        .margin(vec2(4.0, 4.0)),
                );
                if entry.focus {
                    entry_response.request_focus();
                    entry.focus = false;
                }
                let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
                if escape {
                    finish_entry = true;
                } else if entry_response.lost_focus()
                    || ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    if let Some(normalized) = param.string_to_normalized_value(&entry.text) {
                        let value = param.preview_plain(normalized);
                        if state
                            .slider_drag
                            .as_ref()
                            .is_some_and(|drag| drag.key == key)
                        {
                            setter.set_parameter(param, value);
                        } else {
                            set_discrete(setter, param, value);
                        }
                        response.mark_changed();
                    }
                    finish_entry = true;
                }
            } else if ui
                .add_sized(
                    vec2(76.0, 24.0),
                    egui::Button::new(RichText::new(param.to_string()).monospace().size(11.0)),
                )
                .on_hover_text(
                    "Click to type. Double-click slider to reset; Shift-drag for fine control.",
                )
                .clicked()
            {
                state.value_entry = Some(ValueEntry {
                    key,
                    text: param.to_string(),
                    focus: true,
                });
            }
            if finish_entry {
                state.value_entry = None;
            }
            response
        })
        .inner
    })
    .inner
}

fn slider_parameter(params: &ShapeRackParams, key: SliderKey) -> &FloatParam {
    match key {
        SliderKey::Amount(index) => &params.module(index).amount,
        SliderKey::FreeHz => &params.free_hz,
        SliderKey::Smoothing => &params.smoothing,
        SliderKey::Offset => &params.offset,
        SliderKey::Stereo => &params.stereo,
        SliderKey::Mix => &params.mix,
        SliderKey::Output => &params.output,
    }
}

fn end_slider_gesture(params: &ShapeRackParams, setter: &ParamSetter, state: &mut UiState) {
    if let Some(drag) = state.slider_drag.take() {
        setter.end_set_parameter(slider_parameter(params, drag.key));
    }
}

fn division_selector(ui: &mut egui::Ui, params: &ShapeRackParams, setter: &ParamSetter) {
    let divisions = [
        (Division::ThirtySecond, "1/32"),
        (Division::Sixteenth, "1/16"),
        (Division::Eighth, "1/8"),
        (Division::Quarter, "1/4"),
        (Division::Half, "1/2"),
        (Division::Bar, "1 bar"),
        (Division::TwoBars, "2 bars"),
        (Division::FourBars, "4 bars"),
    ];
    let current = divisions
        .iter()
        .find(|(division, _)| *division == params.division.value())
        .map(|(_, label)| *label)
        .unwrap_or("1/4");
    egui::ComboBox::from_id_salt("cycle-division")
        .width(84.0)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (value, label) in divisions {
                if ui
                    .selectable_label(params.division.value() == value, label)
                    .clicked()
                {
                    set_discrete(setter, &params.division, value);
                }
            }
        });
}

fn toggle_button(
    ui: &mut egui::Ui,
    param: &BoolParam,
    setter: &ParamSetter,
    label: &str,
    tooltip: &str,
) {
    if ui
        .selectable_label(param.value(), label)
        .on_hover_text(tooltip)
        .clicked()
    {
        set_discrete(setter, param, !param.value());
    }
}

fn set_discrete<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

fn apply_preset(module: &ModuleParams, setter: &ParamSetter, preset: Preset) {
    let curve = Curve::from_preset(preset);
    for (point, value) in module.points.iter().zip(curve.points) {
        set_discrete(setter, &point.value, value);
    }
    let style = match curve.interpolation {
        crate::dsp::Interpolation::Linear => CurveStyle::Linear,
        crate::dsp::Interpolation::Smooth => CurveStyle::Smooth,
        crate::dsp::Interpolation::Step => CurveStyle::Step,
    };
    set_discrete(setter, &module.style, style);
}

fn end_curve_gesture(params: &ShapeRackParams, setter: &ParamSetter, state: &mut UiState) {
    if let Some(index) = state.gesture_module.take() {
        for point in &params.module(index).points {
            setter.end_set_parameter(&point.value);
        }
    }
    state.previous = None;
    state.node = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct TestContext {
        closed: Arc<AtomicBool>,
        ended: Mutex<Vec<ParamPtr>>,
    }

    impl GuiContext for TestContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            false
        }
        unsafe fn raw_begin_set_parameter(&self, _: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _: ParamPtr, _: f32) {}
        unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
            assert!(
                self.closed.load(Ordering::Relaxed),
                "Window must close before gesture cleanup"
            );
            self.ended.lock().unwrap().push(param);
        }
        fn get_state(&self) -> PluginState {
            panic!("unused in this editor lifecycle test")
        }
        fn set_state(&self, _: PluginState) {
            panic!("unused in this editor lifecycle test")
        }
    }

    struct TestWindow(Arc<AtomicBool>);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn closing_editor_mid_drag_ends_only_active_modules_points_and_resets_gesture() {
        let params = Arc::new(ShapeRackParams::default());
        let state = Arc::new(Mutex::new(UiState {
            gesture_module: Some(1),
            previous: Some((7, 0.3)),
            node: Some(7),
            ..UiState::default()
        }));
        let closed = Arc::new(AtomicBool::new(false));
        let context = Arc::new(TestContext {
            closed: closed.clone(),
            ended: Mutex::new(Vec::new()),
        });
        drop(GestureSafeHandle {
            inner: Some(Box::new(TestWindow(closed))),
            context: context.clone(),
            params: params.clone(),
            state: state.clone(),
        });
        let expected: Vec<_> = params
            .filter
            .points
            .iter()
            .map(|point| point.value.as_ptr())
            .collect();
        assert_eq!(*context.ended.lock().unwrap(), expected);
        let state_guard = state.lock().unwrap();
        assert!(state_guard.gesture_module.is_none());
        assert!(state_guard.previous.is_none());
        assert!(state_guard.node.is_none());
        drop(state_guard);
        // A subsequent close with no active gesture must not emit duplicate ends.
        drop(GestureSafeHandle {
            inner: None,
            context: context.clone(),
            params,
            state,
        });
        assert_eq!(context.ended.lock().unwrap().len(), crate::dsp::POINT_COUNT);
    }

    #[derive(Default)]
    struct SliderContext {
        active: Mutex<Vec<ParamPtr>>,
        writes: Mutex<usize>,
    }

    impl GuiContext for SliderContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            false
        }
        unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
            let mut active = self.active.lock().unwrap();
            assert!(!active.contains(&param), "duplicate begin");
            active.push(param);
        }
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            assert!(
                self.active.lock().unwrap().contains(&param),
                "write before begin"
            );
            assert!(normalized.is_finite() && (0.0..=1.0).contains(&normalized));
            *self.writes.lock().unwrap() += 1;
        }
        unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
            let mut active = self.active.lock().unwrap();
            let index = active
                .iter()
                .position(|candidate| *candidate == param)
                .expect("end before begin");
            active.remove(index);
        }
        fn get_state(&self) -> PluginState {
            panic!("unused in slider event test")
        }
        fn set_state(&self, _: PluginState) {
            panic!("unused in slider event test")
        }
    }

    fn slider_frame(
        context: &egui::Context,
        params: &ShapeRackParams,
        host: &SliderContext,
        state: &mut UiState,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Rect {
        let mut rect = Rect::NOTHING;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 160.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |context| {
                let setter = ParamSetter::new(host);
                if !context.input(|input| input.pointer.primary_down()) {
                    end_slider_gesture(params, &setter, state);
                }
                egui::CentralPanel::default().show(context, |ui| {
                    rect = host_slider(
                        ui,
                        &params.smoothing,
                        &setter,
                        state,
                        SliderKey::Smoothing,
                        160.0,
                    )
                    .rect;
                });
            },
        );
        rect
    }

    fn button(position: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    #[test]
    fn slider_click_drag_single_frame_click_and_reset_all_balance_host_gestures() {
        let context = egui::Context::default();
        let params = ShapeRackParams::default();
        let host = SliderContext::default();
        let mut state = UiState::default();
        let modifiers = egui::Modifiers::default();
        let rect = slider_frame(&context, &params, &host, &mut state, vec![], modifiers);
        let position = pos2(rect.left() + rect.width() * 0.8, rect.center().y);
        // Ordinary click: egui need not emit drag_started for this to be automated.
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(position),
                button(position, true, modifiers),
            ],
            modifiers,
        );
        assert_eq!(host.active.lock().unwrap().len(), 1);
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(position, false, modifiers)],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
        // Drag can leave the slider's bounds. Release still closes the gesture.
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(position, true, modifiers)],
            modifiers,
        );
        let outside = pos2(rect.right() + 80.0, rect.center().y);
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(outside)],
            modifiers,
        );
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(outside, false, modifiers)],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
        // Backends can deliver a fast complete click in one UI frame.
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(position),
                button(position, true, modifiers),
                button(position, false, modifiers),
            ],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
        let command = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        slider_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                button(position, true, command),
                button(position, false, command),
            ],
            command,
        );
        assert!(host.active.lock().unwrap().is_empty());
        assert!(*host.writes.lock().unwrap() >= 4);
    }

    #[test]
    fn closing_editor_mid_slider_drag_ends_slider_gesture() {
        let params = Arc::new(ShapeRackParams::default());
        let state = Arc::new(Mutex::new(UiState {
            slider_drag: Some(SliderDrag {
                key: SliderKey::Smoothing,
                initial_value: 0.0,
                initial_x: 0.0,
            }),
            ..Default::default()
        }));
        let closed = Arc::new(AtomicBool::new(false));
        let context = Arc::new(TestContext {
            closed: closed.clone(),
            ended: Mutex::new(Vec::new()),
        });
        drop(GestureSafeHandle {
            inner: Some(Box::new(TestWindow(closed))),
            context: context.clone(),
            params: params.clone(),
            state: state.clone(),
        });
        assert_eq!(
            *context.ended.lock().unwrap(),
            vec![params.smoothing.as_ptr()]
        );
        assert!(state.lock().unwrap().slider_drag.is_none());
    }
}
