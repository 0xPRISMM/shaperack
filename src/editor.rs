//! Native, host-automatable curve editor. The audio callback never touches UI state.

use crate::dsp::{Curve, Interpolation, Preset, POINT_COUNT, SEGMENT_COUNT};
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
    tool: CurveTool,
    snap: ValueGrid,
    gesture_module: Option<usize>,
    previous: Option<(usize, f32)>,
    node: Option<usize>,
    node_origin: Option<(f32, f32)>,
    bend_segment: Option<usize>,
    bend_origin: Option<(f32, f32)>,
    line_anchor: Option<(usize, f32)>,
    curve_start: Option<Curve>,
    curve_preview: Option<Curve>,
    curve_just_finished: bool,
    history: [CurveHistory; 4],
    copied_curve: Option<Curve>,
    preset_ui: crate::presets::PresetUiState,
    select_module_after_load: bool,
    slider_drag: Option<SliderDrag>,
    value_entry: Option<ValueEntry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CurveTool {
    Nodes,
    Draw,
    Line,
    #[default]
    Bend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveHandle {
    Node(usize),
    Bend(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ValueGrid {
    #[default]
    Off,
    Quarter,
    Eighth,
    Sixteenth,
}

impl ValueGrid {
    fn divisions(self) -> Option<f32> {
        match self {
            Self::Off => None,
            Self::Quarter => Some(4.0),
            Self::Eighth => Some(8.0),
            Self::Sixteenth => Some(16.0),
        }
    }
}

#[derive(Default)]
struct CurveHistory {
    undo: Vec<Curve>,
    redo: Vec<Curve>,
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
            configure_style(context);
        },
        move |context, setter, state| {
            let mut locked_state = state.lock().unwrap_or_else(|error| error.into_inner());
            let state = &mut *locked_state;
            state.curve_just_finished = false;
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
                    draw_editor_contents(ui, &params, &meters, setter, state);
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

fn configure_style(context: &egui::Context) {
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
}

fn draw_editor_contents(
    ui: &mut egui::Ui,
    params: &Arc<ShapeRackParams>,
    meters: &Meters,
    setter: &ParamSetter,
    state: &mut UiState,
) -> Rect {
    if state.select_module_after_load {
        state.module = (0..4)
            .find(|&index| params.module(index).enabled.value())
            .unwrap_or(0);
        state.select_module_after_load = false;
    }
    egui::Frame::NONE
        .inner_margin(12.0)
        .show(ui, |ui| {
            header(ui, params, meters, setter);
            ui.add_space(4.0);
            if crate::presets::show_controls(ui, params, setter, &mut state.preset_ui) {
                state.history = std::array::from_fn(|_| CurveHistory::default());
                state.select_module_after_load = true;
            }
            ui.add_space(8.0);
            module_tabs(ui, params, state);
            ui.add_space(9.0);
            curve_section(ui, params, meters, setter, state);
            ui.add_space(12.0);
            global_controls(ui, params, setter, state);
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Curve modulation · VST3 / CLAP")
                        .small()
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new("Drag corner to resize  ↘")
                            .small()
                            .color(MUTED),
                    );
                });
            });
        })
        .response
        .rect
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
            let synced = params.sync.value();
            ui.label(
                RichText::new(if !synced {
                        "FREE RUNNING"
                } else if playing {
                        "SYNC PLAYING"
                } else {
                        "SYNC PREVIEW"
                })
                .small()
                .color(if playing || !synced { MINT } else { LAVENDER }),
            ).on_hover_text(if !synced {
                "Modulation runs at the selected rate in Hz, independently of song playback."
            } else if playing {
                "Modulation follows the project tempo and aligns with song position when the host provides it."
            } else {
                "Modulation keeps cycling at the project tempo for note previews. Starting playback realigns it with the song."
            });
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
            let label = NAMES[index];
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
            let response = ui.add_sized(vec2(width, 40.0), button);
            let led = pos2(response.rect.left() + 15.0, response.rect.center().y);
            ui.painter().circle_filled(
                led,
                3.0,
                if enabled {
                    color
                } else {
                    MUTED.gamma_multiply(0.4)
                },
            );
            ui.painter().text(
                pos2(response.rect.right() - 11.0, response.rect.center().y),
                egui::Align2::RIGHT_CENTER,
                if enabled { "ON" } else { "OFF" },
                FontId::monospace(9.0),
                if enabled { color } else { MUTED },
            );
            if response.on_hover_text(DESCRIPTIONS[index]).clicked()
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
        module_power_button(ui, module, setter, state.module);
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
                    .on_hover_text(match style {
                        CurveStyle::Linear => {
                            "Straight interpolation. Segment bends still shape the transition."
                        }
                        CurveStyle::Smooth => {
                            "Ease smoothly through each segment; bend handles adjust its timing."
                        }
                        CurveStyle::Step => {
                            "Hold each value until the next point. Segment bends are ignored."
                        }
                    })
                    .clicked()
                {
                    let mut curve = module.curve();
                    curve.interpolation = style.interpolation();
                    apply_curve_edit(module, setter, state, curve);
                }
            }
        });
    });
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Shape").small().color(MUTED));
        for (preset, label) in [
            (Preset::Pump, "Pump"),
            (Preset::Gate, "Gate"),
            (Preset::Sine, "Sine"),
            (Preset::Ramp, "Ramp"),
            (Preset::Flat, "Flat"),
        ] {
            if ui.small_button(label).on_hover_text("Replace only this module's curve. Your rack settings stay as they are.").clicked() {
                apply_curve_edit(module, setter, state, Curve::from_preset(preset));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let index = state.module;
            if ui.add_enabled(!state.history[index].redo.is_empty(), egui::Button::new("Redo").small()).on_hover_text("Redo this module's curve edit").clicked() { redo_curve(module, setter, state); }
            if ui.add_enabled(!state.history[index].undo.is_empty(), egui::Button::new("Undo").small()).on_hover_text("Undo this module's curve edit").clicked() { undo_curve(module, setter, state); }
            curve_tools_menu(ui, module, setter, state);
            egui::ComboBox::from_id_salt("value-grid").width(70.0).selected_text(match state.snap { ValueGrid::Off => "Off", ValueGrid::Quarter => "1/4", ValueGrid::Eighth => "1/8", ValueGrid::Sixteenth => "1/16" }).show_ui(ui, |ui| {
                for (grid, label) in [(ValueGrid::Off, "Off"), (ValueGrid::Quarter, "1/4"), (ValueGrid::Eighth, "1/8"), (ValueGrid::Sixteenth, "1/16")] {
                    ui.selectable_value(&mut state.snap, grid, label);
                }
            }).response.on_hover_text("Snap vertical curve values to a grid. Control enables temporary quarter snapping; Shift bypasses snapping.");
            ui.label(RichText::new("Snap").small().color(MUTED));
            egui::ComboBox::from_id_salt("edit-tool").width(74.0).selected_text(match state.tool { CurveTool::Nodes => "Nodes", CurveTool::Draw => "Draw", CurveTool::Line => "Line", CurveTool::Bend => "Bend" }).show_ui(ui, |ui| {
                for (tool, name, tooltip) in [
                    (CurveTool::Nodes, "Nodes", "Drag fixed grid points vertically. Shift-drag for fine changes. Double-click a point to blend it into its neighbours."),
                    (CurveTool::Draw, "Draw", "Paint across the graph to redraw points. Crossed grid values are filled in automatically."),
                    (CurveTool::Line, "Line", "Draw a straight line from your first point to the current pointer. Release to keep it."),
                    (CurveTool::Bend, "Bend", "Drag points vertically or drag diamonds to bend segments. Double-click a point to blend it, or a segment to straighten it. Alt-drag always bends. Bending a Step curve switches it to Linear."),
                ] {
                    ui.selectable_value(&mut state.tool, tool, name).on_hover_text(tooltip);
                }
            }).response.on_hover_text("Choose how to edit this curve. Time positions stay on the 1/16 grid.");
            ui.label(RichText::new("Edit").small().color(MUTED));
        });
    });
    ui.add_space(5.0);
    let graph_height = (ui.available_height() - 209.0).max(145.0);
    let _ = draw_canvas(ui, params, meters, setter, state, graph_height);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(match state.tool {
                CurveTool::Nodes => {
                    "Drag points  ·  Alt-drag bends  ·  Shift = fine  ·  Ctrl = snap"
                }
                CurveTool::Draw => "Paint a curve  ·  Ctrl = snap  ·  Shift bypasses snap",
                CurveTool::Line => "Drag from start to end to draw a line  ·  Ctrl = snap",
                CurveTool::Bend => {
                    "Drag points or bend diamonds  ·  Double-click = reset  ·  Shift = fine"
                }
            })
            .small()
            .color(MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(if module.enabled.value() {
                    "17 points · 16 segments"
                } else {
                    "Module is off · enable to hear edits"
                })
                .small()
                .color(if module.enabled.value() { MUTED } else { color }),
            );
        });
    });
}

fn curve_tools_menu(
    ui: &mut egui::Ui,
    module: &ModuleParams,
    setter: &ParamSetter,
    state: &mut UiState,
) {
    ui.menu_button("Tools", |ui| {
        if ui
            .button("Copy curve")
            .on_hover_text("Copy this module's points, bends, and interpolation.")
            .clicked()
        {
            state.copied_curve = Some(module.curve());
            ui.close_menu();
        }
        if ui
            .add_enabled(
                state.copied_curve.is_some(),
                egui::Button::new("Paste curve"),
            )
            .clicked()
        {
            if let Some(curve) = state.copied_curve {
                apply_curve_edit(module, setter, state, curve);
            }
            ui.close_menu();
        }
        ui.separator();
        let mut curve = module.curve();
        if ui
            .button("Invert vertically")
            .on_hover_text("Turn peaks into dips without changing their timing.")
            .clicked()
        {
            for point in &mut curve.points {
                *point = 1.0 - *point;
            }
            apply_curve_edit(module, setter, state, curve);
            ui.close_menu();
        }
        if ui
            .button("Reverse in time")
            .on_hover_text("Mirror the points and bends across the cycle.")
            .clicked()
        {
            curve.points.reverse();
            curve.tensions.reverse();
            for tension in &mut curve.tensions {
                *tension = -*tension;
            }
            apply_curve_edit(module, setter, state, curve);
            ui.close_menu();
        }
        if ui
            .button("Smooth points")
            .on_hover_text("Soften abrupt changes while keeping both endpoints.")
            .clicked()
        {
            let original = curve.points;
            for index in 1..SEGMENT_COUNT {
                curve.points[index] =
                    original[index - 1] * 0.25 + original[index] * 0.5 + original[index + 1] * 0.25;
            }
            curve.tensions.fill(0.0);
            curve.interpolation = Interpolation::Smooth;
            apply_curve_edit(module, setter, state, curve);
            ui.close_menu();
        }
        if ui.button("Clear segment bends").clicked() {
            curve.tensions.fill(0.0);
            apply_curve_edit(module, setter, state, curve);
            ui.close_menu();
        }
        if ui
            .button("Flatten to neutral")
            .on_hover_text("Set a flat, open volume/filter curve, centered pan, or clean drive.")
            .clicked()
        {
            curve.points.fill(match state.module {
                2 => 0.5,
                3 => 0.0,
                _ => 1.0,
            });
            curve.tensions.fill(0.0);
            curve.interpolation = Interpolation::Linear;
            apply_curve_edit(module, setter, state, curve);
            ui.close_menu();
        }
    });
}

fn paint_points(points: &mut [f32; POINT_COUNT], start: (usize, f32), end: (usize, f32)) {
    for (node, point) in points
        .iter_mut()
        .enumerate()
        .take(start.0.max(end.0) + 1)
        .skip(start.0.min(end.0))
    {
        let t = if start.0 == end.0 {
            1.0
        } else {
            (node as f32 - start.0 as f32) / (end.0 as f32 - start.0 as f32)
        };
        *point = (start.1 + (end.1 - start.1) * t).clamp(0.0, 1.0);
    }
}

fn clear_tensions_between(tensions: &mut [f32; SEGMENT_COUNT], first: usize, last: usize) {
    tensions[first.min(last)..first.max(last)].fill(0.0);
}

fn snap_value(value: f32, grid: ValueGrid, modifiers: egui::Modifiers) -> f32 {
    let divisions = if modifiers.shift {
        None
    } else if modifiers.command && grid == ValueGrid::Off {
        Some(4.0)
    } else {
        grid.divisions()
    };
    let value = value.clamp(0.0, 1.0);
    divisions
        .map(|divisions| (value * divisions).round() / divisions)
        .unwrap_or(value)
}

fn graph_value(rect: Rect, pointer_y: f32, grid: ValueGrid, modifiers: egui::Modifiers) -> f32 {
    snap_value(
        1.0 - (pointer_y - rect.top()) / rect.height(),
        grid,
        modifiers,
    )
}

fn curve_style(interpolation: Interpolation) -> CurveStyle {
    match interpolation {
        Interpolation::Linear => CurveStyle::Linear,
        Interpolation::Smooth => CurveStyle::Smooth,
        Interpolation::Step => CurveStyle::Step,
    }
}

fn write_curve(module: &ModuleParams, setter: &ParamSetter, curve: Curve) {
    for (point, value) in module.points.iter().zip(curve.points) {
        set_discrete(setter, &point.value, value);
    }
    for (segment, value) in module.segments.iter().zip(curve.tensions) {
        set_discrete(setter, &segment.tension, value);
    }
    set_discrete(setter, &module.style, curve_style(curve.interpolation));
}

fn remember_curve_edit(history: &mut CurveHistory, before: Curve, after: Curve) {
    if before != after {
        if history.undo.len() == 32 {
            history.undo.remove(0);
        }
        history.undo.push(before);
        history.redo.clear();
    }
}

fn apply_curve_edit(
    module: &ModuleParams,
    setter: &ParamSetter,
    state: &mut UiState,
    curve: Curve,
) {
    if state.gesture_module.is_some() {
        return;
    }
    remember_curve_edit(&mut state.history[state.module], module.curve(), curve);
    write_curve(module, setter, curve);
}

fn undo_curve(module: &ModuleParams, setter: &ParamSetter, state: &mut UiState) {
    if let Some(curve) = state.history[state.module].undo.pop() {
        state.history[state.module].redo.push(module.curve());
        write_curve(module, setter, curve);
    }
}

fn redo_curve(module: &ModuleParams, setter: &ParamSetter, state: &mut UiState) {
    if let Some(curve) = state.history[state.module].redo.pop() {
        state.history[state.module].undo.push(module.curve());
        write_curve(module, setter, curve);
    }
}

fn begin_curve_gesture(
    module: &ModuleParams,
    setter: &ParamSetter,
    state: &mut UiState,
    curve: Curve,
) {
    for point in &module.points {
        setter.begin_set_parameter(&point.value);
    }
    for segment in &module.segments {
        setter.begin_set_parameter(&segment.tension);
    }
    setter.begin_set_parameter(&module.style);
    state.gesture_module = Some(state.module);
    state.curve_start = Some(curve);
    state.curve_preview = Some(curve);
    state.previous = None;
    state.node = None;
    state.node_origin = None;
    state.line_anchor = None;
    state.bend_segment = None;
    state.bend_origin = None;
}

fn send_preview_curve(
    module: &ModuleParams,
    setter: &ParamSetter,
    state: &mut UiState,
    curve: Curve,
) {
    let previous = state.curve_preview.unwrap_or_else(|| module.curve());
    for (index, value) in curve.points.iter().enumerate() {
        if *value != previous.points[index] {
            setter.set_parameter(&module.points[index].value, *value);
        }
    }
    for (index, value) in curve.tensions.iter().enumerate() {
        if *value != previous.tensions[index] {
            setter.set_parameter(&module.segments[index].tension, *value);
        }
    }
    if curve.interpolation != previous.interpolation {
        setter.set_parameter(&module.style, curve_style(curve.interpolation));
    }
    state.curve_preview = Some(curve);
}

fn draw_canvas(
    ui: &mut egui::Ui,
    params: &ShapeRackParams,
    meters: &Meters,
    setter: &ParamSetter,
    state: &mut UiState,
    height: f32,
) -> egui::Response {
    let module = params.module(state.module);
    let accent = COLORS[state.module];
    let (outer, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let rect = canvas_plot_rect(outer);
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 8.0, Color32::from_rgb(12, 17, 24));
    painter.rect_stroke(
        outer,
        8.0,
        Stroke::new(1.0_f32, Color32::from_rgb(41, 51, 66)),
        egui::StrokeKind::Inside,
    );

    let modifiers = ui.input(|input| input.modifiers);
    let down =
        response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_down());
    if down && state.gesture_module.is_none() && state.slider_drag.is_none() {
        if let Some(position) = response.interact_pointer_pos() {
            let x = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let index = (x * SEGMENT_COUNT as f32).round() as usize;
            let curve = module.curve();
            let handle = curve_handle_at(rect, &curve, position, state.tool == CurveTool::Bend);
            begin_curve_gesture(module, setter, state, curve);
            if modifiers.alt
                || (state.tool == CurveTool::Bend && !matches!(handle, Some(CurveHandle::Node(_))))
            {
                let segment = ((x * SEGMENT_COUNT as f32) as usize).min(SEGMENT_COUNT - 1);
                state.bend_segment = Some(segment);
                state.bend_origin = Some((position.y, curve.tensions[segment]));
            } else if matches!(state.tool, CurveTool::Nodes | CurveTool::Bend) {
                let index = match handle {
                    Some(CurveHandle::Node(node)) => node,
                    _ => index,
                };
                state.node = Some(index);
                let node_position = curve_position(
                    rect,
                    index as f32 / SEGMENT_COUNT as f32,
                    curve.points[index],
                );
                let initial_value = if position.distance(node_position) <= 12.0 {
                    curve.points[index]
                } else {
                    graph_value(rect, position.y, state.snap, modifiers)
                };
                state.node_origin = Some((position.y, initial_value));
            } else if state.tool == CurveTool::Line {
                state.line_anchor =
                    Some((index, graph_value(rect, position.y, state.snap, modifiers)));
            }
        }
    }
    if down && state.gesture_module == Some(state.module) {
        if let Some(position) = response.interact_pointer_pos() {
            let x = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let index = (x * SEGMENT_COUNT as f32).round() as usize;
            let value = graph_value(rect, position.y, state.snap, modifiers);
            let mut curve = state.curve_preview.unwrap_or_else(|| module.curve());
            if let (Some(segment), Some((initial_y, initial_tension))) =
                (state.bend_segment, state.bend_origin)
            {
                let rising = curve.points[segment + 1] >= curve.points[segment];
                let polarity = if rising { 1.0 } else { -1.0 };
                let sensitivity = if modifiers.shift { 0.3 } else { 2.8 };
                curve.tensions[segment] = (initial_tension
                    + (initial_y - position.y) / rect.height() * polarity * sensitivity)
                    .clamp(-1.0, 1.0);
                if curve.interpolation == Interpolation::Step {
                    curve.interpolation = Interpolation::Linear;
                }
            } else if let (Some(node), Some((initial_y, initial_value))) =
                (state.node, state.node_origin)
            {
                let sensitivity = if modifiers.shift { 0.15 } else { 1.0 };
                let value = initial_value + (initial_y - position.y) / rect.height() * sensitivity;
                curve.points[node] = snap_value(value, state.snap, modifiers);
            } else if let Some(anchor) = state.line_anchor {
                curve = state.curve_start.unwrap_or(curve);
                paint_points(&mut curve.points, anchor, (index, value));
                clear_tensions_between(&mut curve.tensions, anchor.0, index);
                curve.interpolation = Interpolation::Linear;
            } else {
                let previous = state.previous.unwrap_or((index, value));
                paint_points(&mut curve.points, previous, (index, value));
                clear_tensions_between(&mut curve.tensions, previous.0, index);
                state.previous = Some((index, value));
            }
            send_preview_curve(module, setter, state, curve);
        }
    }

    let curve = state
        .curve_preview
        .filter(|_| state.gesture_module == Some(state.module))
        .unwrap_or_else(|| module.curve());
    if response.double_clicked() && state.gesture_module.is_none() {
        if let Some(position) = response.interact_pointer_pos() {
            let x = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let mut edited = curve;
            let handle = curve_handle_at(rect, &curve, position, state.tool == CurveTool::Bend);
            if modifiers.alt
                || (state.tool == CurveTool::Bend && !matches!(handle, Some(CurveHandle::Node(_))))
            {
                let segment = ((x * SEGMENT_COUNT as f32) as usize).min(SEGMENT_COUNT - 1);
                edited.tensions[segment] = 0.0;
            } else if matches!(state.tool, CurveTool::Nodes | CurveTool::Bend) {
                let node = match handle {
                    Some(CurveHandle::Node(node)) => node,
                    _ => (x * SEGMENT_COUNT as f32).round() as usize,
                };
                if node > 0 && node < SEGMENT_COUNT {
                    edited.points[node] = (curve.points[node - 1] + curve.points[node + 1]) * 0.5;
                }
                if node > 0 {
                    edited.tensions[node - 1] = 0.0;
                }
                if node < SEGMENT_COUNT {
                    edited.tensions[node] = 0.0;
                }
            }
            apply_curve_edit(module, setter, state, edited);
        }
    } else if response.clicked()
        && state.gesture_module.is_none()
        && !state.curve_just_finished
        && state.tool != CurveTool::Bend
        && !modifiers.alt
    {
        if let Some(position) = response.interact_pointer_pos() {
            let x = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let node = (x * SEGMENT_COUNT as f32).round() as usize;
            let node_position =
                curve_position(rect, node as f32 / SEGMENT_COUNT as f32, curve.points[node]);
            if state.tool != CurveTool::Nodes || position.distance(node_position) > 12.0 {
                let mut edited = curve;
                edited.points[node] = graph_value(rect, position.y, state.snap, modifiers);
                if edited != curve {
                    apply_curve_edit(module, setter, state, edited);
                }
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
                ["0", "¼", "½", "¾", "1 cycle"][index / 4],
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
            match state.module {
                1 if index == 0 => "Open".to_owned(),
                1 if index == 4 => "Dark".to_owned(),
                2 if index == 0 => "R".to_owned(),
                2 if index == 2 => "C".to_owned(),
                2 if index == 4 => "L".to_owned(),
                2 => if index == 1 { "+50" } else { "−50" }.to_owned(),
                _ => format!("{}", 100 - index * 25),
            },
            FontId::proportional(10.0),
            MUTED,
        );
    }

    painter.text(
        pos2(rect.left(), outer.top() + 5.0),
        egui::Align2::LEFT_TOP,
        ["Gain", "Cutoff", "Balance", "Saturation"][state.module],
        FontId::proportional(10.0),
        MUTED,
    );
    painter.text(
        pos2(rect.right(), outer.top() + 5.0),
        egui::Align2::RIGHT_TOP,
        "1/16 time grid",
        FontId::proportional(10.0),
        MUTED,
    );
    if let Some(node) = state
        .node
        .filter(|_| state.gesture_module == Some(state.module))
    {
        let position = curve_position(rect, node as f32 / SEGMENT_COUNT as f32, curve.points[node]);
        let guide = Stroke::new(1.0_f32, accent.gamma_multiply(0.28));
        painter.line_segment(
            [
                pos2(position.x, rect.top()),
                pos2(position.x, rect.bottom()),
            ],
            guide,
        );
        painter.line_segment(
            [
                pos2(rect.left(), position.y),
                pos2(rect.right(), position.y),
            ],
            guide,
        );
    }
    // Draw using the same evaluator as the DSP, including exact step corners.
    let mut line = Vec::with_capacity(SEGMENT_COUNT * 17 + 1);
    if curve.interpolation == Interpolation::Step {
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
    if let Some(segment) = state
        .bend_segment
        .filter(|_| state.gesture_module == Some(state.module))
    {
        let emphasized: Vec<_> = (0..=32)
            .map(|sample| {
                let phase = (segment as f32 + sample as f32 / 32.0) / SEGMENT_COUNT as f32;
                curve_position(rect, phase, curve.value(phase as f64))
            })
            .collect();
        painter.add(egui::Shape::line(emphasized, Stroke::new(3.3_f32, TEXT)));
    }
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

    let active_node = state
        .node
        .filter(|_| state.gesture_module == Some(state.module));
    let active_bend = state
        .bend_segment
        .filter(|_| state.gesture_module == Some(state.module));
    let show_bends = state.tool == CurveTool::Bend || modifiers.alt || active_bend.is_some();
    let hover_handle = response
        .hover_pos()
        .and_then(|position| curve_handle_at(rect, &curve, position, show_bends));
    let hover_index = match hover_handle {
        Some(CurveHandle::Node(node)) if !modifiers.alt && active_bend.is_none() => Some(node),
        _ => None,
    };
    let hover_segment = match hover_handle {
        Some(CurveHandle::Bend(segment)) if active_node.is_none() => Some(segment),
        _ => None,
    };
    for (index, value) in curve.points.iter().enumerate() {
        let position = curve_position(rect, index as f32 / SEGMENT_COUNT as f32, *value);
        let hovered =
            active_node == Some(index) || (active_node.is_none() && hover_index == Some(index));
        painter.circle_filled(position, if hovered { 5.0 } else { 3.0 }, BACKGROUND);
        painter.circle_stroke(
            position,
            if hovered { 5.0 } else { 3.0 },
            Stroke::new(if hovered { 2.0_f32 } else { 1.3_f32 }, accent),
        );
    }
    if show_bends {
        for segment in 0..SEGMENT_COUNT {
            let phase = (segment as f32 + 0.5) / SEGMENT_COUNT as f32;
            let position = curve_position(rect, phase, curve.value(phase as f64));
            let highlighted = active_bend == Some(segment)
                || (active_bend.is_none() && hover_segment == Some(segment));
            let size = if highlighted { 5.0 } else { 3.5 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    position + vec2(0.0, -size),
                    position + vec2(size, 0.0),
                    position + vec2(0.0, size),
                    position + vec2(-size, 0.0),
                ],
                if highlighted { accent } else { PANEL },
                Stroke::new(1.0_f32, accent),
            ));
        }
    }
    if let Some(position) = response.hover_pos() {
        let phase = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let segment = ((phase * SEGMENT_COUNT as f32) as usize).min(SEGMENT_COUNT - 1);
        let bending = active_bend.or(hover_segment).or_else(|| {
            (modifiers.alt && hover_handle.is_some() && active_node.is_none()).then_some(segment)
        });
        let text = if let Some(node) = active_node.or(hover_index) {
            format!(
                "Point {} · {:.0}% · cycle {:.0}%",
                node + 1,
                curve.points[node] * 100.0,
                node as f32 / SEGMENT_COUNT as f32 * 100.0
            )
        } else if let Some(segment) = bending {
            format!(
                "Segment {} · bend {:+.2}",
                segment + 1,
                curve.tensions[segment]
            )
        } else if state.module == 2 {
            format!(
                "Balance {:+.0}% · cycle {:.0}%",
                (curve.value(phase as f64) * 2.0 - 1.0) * 100.0,
                phase * 100.0
            )
        } else {
            format!(
                "Value {:.0}% · cycle {:.0}%",
                curve.value(phase as f64) * 100.0,
                phase * 100.0
            )
        };
        painter.text(
            pos2(rect.center().x, outer.top() + 5.0),
            egui::Align2::CENTER_TOP,
            text,
            FontId::monospace(10.0),
            accent,
        );
    }
    if state.gesture_module == Some(state.module) || response.hovered() {
        ui.ctx().set_cursor_icon(
            if active_node.is_some()
                || active_bend.is_some()
                || ((matches!(state.tool, CurveTool::Bend | CurveTool::Nodes) || modifiers.alt)
                    && hover_handle.is_some())
            {
                egui::CursorIcon::ResizeVertical
            } else if matches!(state.tool, CurveTool::Draw | CurveTool::Line) {
                egui::CursorIcon::Crosshair
            } else {
                egui::CursorIcon::Default
            },
        );
    }
    response.on_hover_text(match state.tool {
        CurveTool::Nodes => "Drag fixed grid points vertically. Shift = fine, Control = snap, Alt = bend. Double-click a point to blend it into its neighbours.",
        CurveTool::Draw => "Paint across the 1/16 grid. Control = snap, Shift bypasses snapping. Each module keeps its own points and bends.",
        CurveTool::Line => "Drag a straight line across grid points. Line clears bends along the edited span and uses Linear interpolation.",
        CurveTool::Bend => "Drag points vertically or diamonds to bend. Double-click a point to blend it into neighbours; double-click a segment to straighten. Alt-drag always bends. Step switches to Linear when you bend it.",
    })
}

fn canvas_plot_rect(outer: Rect) -> Rect {
    Rect::from_min_max(outer.min + vec2(44.0, 21.0), outer.max - vec2(18.0, 25.0))
}

/// Share exact handle hit testing between press, double-click, and hover. Small
/// windows can overlap hit areas; choose the closest handle, with nodes winning ties.
fn curve_handle_at(
    rect: Rect,
    curve: &Curve,
    pointer: egui::Pos2,
    include_bends: bool,
) -> Option<CurveHandle> {
    let mut closest = None;
    let mut closest_distance = f32::INFINITY;
    for (index, value) in curve.points.iter().enumerate() {
        let distance = pointer.distance_sq(curve_position(
            rect,
            index as f32 / SEGMENT_COUNT as f32,
            *value,
        ));
        if distance <= 12.0 * 12.0 && distance < closest_distance {
            closest = Some(CurveHandle::Node(index));
            closest_distance = distance;
        }
    }
    if include_bends {
        for index in 0..SEGMENT_COUNT {
            let phase = (index as f32 + 0.5) / SEGMENT_COUNT as f32;
            let distance =
                pointer.distance_sq(curve_position(rect, phase, curve.value(phase as f64)));
            if distance <= 9.0 * 9.0 && distance < closest_distance {
                closest = Some(CurveHandle::Bend(index));
                closest_distance = distance;
            }
        }
    }
    closest
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
                            .on_hover_text(if value {
                                "Follow the project tempo. Stopped note previews keep cycling; playback aligns with the song."
                            } else {
                                "Choose an independent modulation speed in Hz."
                            })
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

fn module_power_button(
    ui: &mut egui::Ui,
    module: &ModuleParams,
    setter: &ParamSetter,
    index: usize,
) {
    let enabled = module.enabled.value();
    let accent = COLORS[index];
    let label = format!("{} {}", NAMES[index], if enabled { "ON" } else { "OFF" });
    let response = ui.add_sized(
        vec2(112.0, 28.0),
        egui::Button::new(RichText::new(label).strong().color(if enabled {
            accent
        } else {
            MUTED
        }))
        .fill(if enabled {
            accent.gamma_multiply(0.18)
        } else {
            PANEL
        })
        .stroke(Stroke::new(
            1.0_f32,
            if enabled {
                accent
            } else {
                MUTED.gamma_multiply(0.35)
            },
        )),
    );
    ui.painter().circle_filled(
        pos2(response.rect.left() + 9.0, response.rect.center().y),
        3.0,
        if enabled {
            accent
        } else {
            MUTED.gamma_multiply(0.4)
        },
    );
    if response
        .on_hover_text(format!(
            "Click to turn {} {}.",
            NAMES[index],
            if enabled { "off" } else { "on" }
        ))
        .clicked()
    {
        set_discrete(setter, &module.enabled, !enabled);
    }
}

fn set_discrete<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

fn end_curve_gesture(params: &ShapeRackParams, setter: &ParamSetter, state: &mut UiState) {
    if let Some(index) = state.gesture_module.take() {
        state.curve_just_finished = true;
        let module = params.module(index);
        for point in &module.points {
            setter.end_set_parameter(&point.value);
        }
        for segment in &module.segments {
            setter.end_set_parameter(&segment.tension);
        }
        setter.end_set_parameter(&module.style);
        if let (Some(before), Some(after)) = (state.curve_start, state.curve_preview) {
            remember_curve_edit(&mut state.history[index], before, after);
        }
    }
    state.previous = None;
    state.node = None;
    state.node_origin = None;
    state.line_anchor = None;
    state.bend_segment = None;
    state.bend_origin = None;
    state.curve_start = None;
    state.curve_preview = None;
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
    fn closing_editor_mid_drag_ends_active_modules_points_bends_and_style() {
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
            .chain(
                params
                    .filter
                    .segments
                    .iter()
                    .map(|segment| segment.tension.as_ptr()),
            )
            .chain(std::iter::once(params.filter.style.as_ptr()))
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
        assert_eq!(
            context.ended.lock().unwrap().len(),
            POINT_COUNT + SEGMENT_COUNT + 1
        );
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

    fn canvas_frame(
        context: &egui::Context,
        params: &ShapeRackParams,
        host: &SliderContext,
        state: &mut UiState,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Rect {
        canvas_frame_with_cursor(context, params, host, state, events, modifiers).0
    }

    fn canvas_frame_with_cursor(
        context: &egui::Context,
        params: &ShapeRackParams,
        host: &SliderContext,
        state: &mut UiState,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> (Rect, egui::CursorIcon) {
        let mut outer = Rect::NOTHING;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(540.0, 320.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |context| {
                let setter = ParamSetter::new(host);
                state.curve_just_finished = false;
                if !context.input(|input| input.pointer.primary_down()) {
                    end_curve_gesture(params, &setter, state);
                }
                egui::CentralPanel::default().show(context, |ui| {
                    outer = draw_canvas(ui, params, &Meters::default(), &setter, state, 280.0).rect;
                });
            },
        );
        (canvas_plot_rect(outer), output.platform_output.cursor_icon)
    }

    #[test]
    fn bend_drag_changes_segment_without_moving_nodes_and_release_outside_ends_every_gesture() {
        let context = egui::Context::default();
        let params = ShapeRackParams::default();
        let host = SliderContext::default();
        let mut state = UiState::default();
        let modifiers = egui::Modifiers::default();
        let plot = canvas_frame(&context, &params, &host, &mut state, vec![], modifiers);
        let original = params.volume.curve();
        let phase = 4.5 / SEGMENT_COUNT as f32;
        let start = curve_position(plot, phase, original.value(phase as f64));
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(start),
                button(start, true, modifiers),
            ],
            modifiers,
        );
        assert_eq!(
            host.active.lock().unwrap().len(),
            POINT_COUNT + SEGMENT_COUNT + 1
        );
        let dragged = start - vec2(0.0, 28.0);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(dragged)],
            modifiers,
        );
        let preview = state.curve_preview.unwrap();
        assert_eq!(preview.points, original.points);
        assert!(preview.tensions[4] > original.tensions[4]);
        assert!(preview.value(phase as f64) > original.value(phase as f64));
        let outside = pos2(plot.right() + 100.0, plot.bottom() + 100.0);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(outside),
                button(outside, false, modifiers),
            ],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
        assert!(state.curve_preview.is_none());
        assert_eq!(state.history[0].undo, vec![original]);
        assert!(state.history[1].undo.is_empty());
        assert!(*host.writes.lock().unwrap() > 0);
    }

    #[test]
    fn line_tool_fills_crossed_nodes_and_rebuilds_from_original_anchor() {
        let context = egui::Context::default();
        let params = ShapeRackParams::default();
        let host = SliderContext::default();
        let mut state = UiState {
            tool: CurveTool::Line,
            ..Default::default()
        };
        let modifiers = egui::Modifiers::default();
        let plot = canvas_frame(&context, &params, &host, &mut state, vec![], modifiers);
        let original = params.volume.curve();
        let start = curve_position(plot, 2.0 / SEGMENT_COUNT as f32, 0.9);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(start),
                button(start, true, modifiers),
            ],
            modifiers,
        );
        let end = curve_position(plot, 10.0 / SEGMENT_COUNT as f32, 0.1);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(end)],
            modifiers,
        );
        let preview = state.curve_preview.unwrap();
        assert!((preview.points[2] - 0.9).abs() < 1e-5);
        assert!((preview.points[6] - 0.5).abs() < 1e-5);
        assert!((preview.points[10] - 0.1).abs() < 1e-5);
        assert_eq!(preview.interpolation, Interpolation::Linear);
        // Moving the endpoint back restores values outside the new line's extent.
        let shorter = curve_position(plot, 6.0 / SEGMENT_COUNT as f32, 0.3);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(shorter)],
            modifiers,
        );
        let preview = state.curve_preview.unwrap();
        assert!((preview.points[4] - 0.6).abs() < 1e-5);
        assert_eq!(preview.points[7..], original.points[7..]);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(shorter, false, modifiers)],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
        assert_eq!(state.history[0].undo.len(), 1);
    }

    #[test]
    fn node_drag_stays_on_its_time_slot_and_control_snaps_values() {
        for tool in [CurveTool::Nodes, CurveTool::Bend] {
            let context = egui::Context::default();
            let params = ShapeRackParams::default();
            let host = SliderContext::default();
            let mut state = UiState {
                tool,
                ..Default::default()
            };
            let modifiers = egui::Modifiers {
                command: true,
                ctrl: true,
                ..Default::default()
            };
            let plot = canvas_frame(&context, &params, &host, &mut state, vec![], modifiers);
            let original = params.volume.curve();
            let start = curve_position(plot, 4.0 / SEGMENT_COUNT as f32, original.points[4]);
            canvas_frame(
                &context,
                &params,
                &host,
                &mut state,
                vec![
                    egui::Event::PointerMoved(start),
                    button(start, true, modifiers),
                ],
                modifiers,
            );
            assert_eq!(state.node, Some(4));
            assert!(state.bend_segment.is_none());
            let target = curve_position(plot, 10.0 / SEGMENT_COUNT as f32, 0.62);
            canvas_frame(
                &context,
                &params,
                &host,
                &mut state,
                vec![egui::Event::PointerMoved(target)],
                modifiers,
            );
            let preview = state.curve_preview.unwrap();
            assert_eq!(preview.points[4], 0.5);
            assert_eq!(preview.points[10], original.points[10]);
            assert_eq!(preview.tensions, original.tensions);
            assert_eq!(state.node, Some(4));
            canvas_frame(
                &context,
                &params,
                &host,
                &mut state,
                vec![button(target, false, modifiers)],
                modifiers,
            );
            assert!(host.active.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn bend_hover_uses_closest_handle_and_arrow_over_empty_space_and_latches_segment() {
        let context = egui::Context::default();
        let params = ShapeRackParams::default();
        let host = SliderContext::default();
        let mut state = UiState::default();
        let modifiers = egui::Modifiers::default();
        let plot = canvas_frame(&context, &params, &host, &mut state, vec![], modifiers);
        let curve = params.volume.curve();
        let node = curve_position(plot, 5.0 / SEGMENT_COUNT as f32, curve.points[5]);
        let phase = 5.5 / SEGMENT_COUNT as f32;
        let diamond = curve_position(plot, phase, curve.value(phase as f64));
        let empty = curve_position(plot, phase, 0.2);
        for (position, expected) in [
            (empty, egui::CursorIcon::Default),
            (node, egui::CursorIcon::ResizeVertical),
            (diamond, egui::CursorIcon::ResizeVertical),
        ] {
            canvas_frame(
                &context,
                &params,
                &host,
                &mut state,
                vec![egui::Event::PointerMoved(position)],
                modifiers,
            );
            let (current_plot, cursor) =
                canvas_frame_with_cursor(&context, &params, &host, &mut state, vec![], modifiers);
            assert_eq!(
                cursor,
                expected,
                "pointer {position:?}, plot {current_plot:?}, hit {:?}",
                curve_handle_at(current_plot, &curve, position, true)
            );
        }
        // At narrow graph widths the two hit areas overlap. Pick the closer
        // diamond instead of allowing the larger point radius to steal it.
        let flat = Curve::from_preset(Preset::Flat);
        let flat_node = curve_position(plot, 12.0 / SEGMENT_COUNT as f32, 1.0);
        let flat_diamond = curve_position(plot, 12.5 / SEGMENT_COUNT as f32, 1.0);
        let overlap = flat_node.lerp(flat_diamond, 0.65);
        assert!(overlap.distance(flat_node) < 12.0);
        assert!(overlap.distance(flat_diamond) < 9.0);
        assert_eq!(
            curve_handle_at(plot, &flat, overlap, true),
            Some(CurveHandle::Bend(12))
        );
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![
                egui::Event::PointerMoved(diamond),
                button(diamond, true, modifiers),
            ],
            modifiers,
        );
        assert_eq!(state.bend_segment, Some(5));
        assert!(state.node.is_none());
        let (_, cursor) = canvas_frame_with_cursor(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(node)],
            modifiers,
        );
        assert_eq!(state.bend_segment, Some(5));
        assert!(state.node.is_none());
        assert_eq!(state.curve_preview.unwrap().points, curve.points);
        assert_eq!(cursor, egui::CursorIcon::ResizeVertical);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(node, false, modifiers)],
            modifiers,
        );
        assert!(host.active.lock().unwrap().is_empty());
    }

    #[test]
    fn alt_press_on_a_node_always_bends_and_keeps_target_after_alt_is_released() {
        let context = egui::Context::default();
        let params = ShapeRackParams::default();
        let host = SliderContext::default();
        let mut state = UiState::default();
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let plot = canvas_frame(&context, &params, &host, &mut state, vec![], alt);
        let curve = params.volume.curve();
        let node = curve_position(plot, 4.0 / SEGMENT_COUNT as f32, curve.points[4]);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(node), button(node, true, alt)],
            alt,
        );
        assert_eq!(state.bend_segment, Some(4));
        assert!(state.node.is_none());
        let normal = egui::Modifiers::default();
        let dragged = node - vec2(0.0, 28.0);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![egui::Event::PointerMoved(dragged)],
            normal,
        );
        let preview = state.curve_preview.unwrap();
        assert_eq!(preview.points, curve.points);
        assert!(preview.tensions[4] > 0.0);
        canvas_frame(
            &context,
            &params,
            &host,
            &mut state,
            vec![button(dragged, false, normal)],
            normal,
        );
        assert!(host.active.lock().unwrap().is_empty());
    }

    #[test]
    fn editor_contents_fit_minimum_and_default_window_sizes() {
        for size in [vec2(820.0, 620.0), vec2(960.0, 640.0)] {
            let context = egui::Context::default();
            configure_style(&context);
            let params = Arc::new(ShapeRackParams::default());
            let host = SliderContext::default();
            let mut state = UiState::default();
            let mut rect = Rect::NOTHING;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
                    ..Default::default()
                },
                |context| {
                    ResizableWindow::new("layout-test")
                        .min_size(vec2(820.0, 620.0))
                        .show(context, params.editor_state.as_ref(), |ui| {
                            rect = draw_editor_contents(
                                ui,
                                &params,
                                &Meters::default(),
                                &ParamSetter::new(&host),
                                &mut state,
                            );
                        });
                },
            );
            assert!(
                rect.max.x <= size.x + 1.0,
                "horizontal overflow: {size:?} {rect:?}"
            );
            assert!(
                rect.max.y <= size.y + 1.0,
                "vertical overflow: {size:?} {rect:?}"
            );
            assert!(host.active.lock().unwrap().is_empty());
        }
    }
}
