//! Rack presets live on the GUI thread; no file access occurs in audio processing.
use crate::dsp::{Curve, Preset};
use crate::params::{CurveStyle, Division, ModuleParams, ShapeRackParams};
use crate::preset_file::RackPreset;
use nih_plug::prelude::*;
use nih_plug::wrapper::state::ParamValue;
use nih_plug_egui::egui::{self, Color32, RichText};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};
use std::thread::JoinHandle;

const MINT: Color32 = Color32::from_rgb(108, 231, 190);
const MUTED: Color32 = Color32::from_rgb(137, 151, 171);

#[derive(Default)]
pub struct PresetUiState {
    initialized: bool,
    directory: Option<PathBuf>,
    library: Vec<(PathBuf, RackPreset)>,
    saved_values: Option<BTreeMap<String, f32>>,
    save_open: bool,
    save_name: String,
    replace_existing: bool,
    status: String,
    error: bool,
    dialog: Option<DialogJob>,
}

enum DialogResult {
    Import(Result<Option<RackPreset>, String>),
    Export(Result<Option<PathBuf>, String>),
}

struct DialogJob {
    result: Receiver<DialogResult>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for DialogJob {
    fn drop(&mut self) {
        // Finish the native dialog before editor teardown can unload this
        // plugin's code. The worker never touches GUI or host parameter APIs.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn start_dialog(task: impl FnOnce() -> DialogResult + Send + 'static) -> Result<DialogJob, String> {
    let (sender, result) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("ShapeRack preset dialog".into())
        .spawn(move || {
            let _ = sender.send(task());
        })
        .map_err(|error| format!("Could not open file dialog: {error}"))?;
    Ok(DialogJob {
        result,
        worker: Some(worker),
    })
}

fn library_directory() -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os("SHAPERACK_PRESET_DIR") {
        return Some(PathBuf::from(directory));
    }
    std::env::var_os("APPDATA")
        .map(|directory| PathBuf::from(directory).join("PRISMM/ShapeRack/Presets"))
}

fn preset_filename(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    format!("preset-{}.shaperack.json", safe.trim_matches('-'))
}

fn values_match(left: &BTreeMap<String, f32>, right: &BTreeMap<String, f32>) -> bool {
    left.len() == right.len()
        && left.iter().all(|(id, value)| {
            right
                .get(id)
                .is_some_and(|other| (value - other).abs() < 1.0e-5)
        })
}

impl PresetUiState {
    fn message(&mut self, message: impl Into<String>, error: bool) {
        self.status = message.into();
        self.error = error;
    }

    fn refresh(&mut self, params: &ShapeRackParams) {
        self.library.clear();
        let Some(directory) = &self.directory else {
            return;
        };
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.message(format!("Could not read preset library: {error}"), true);
                return;
            }
        };
        let mut skipped = 0;
        for entry in entries.take(256).flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            match RackPreset::read(&path, params) {
                Ok(preset) => self.library.push((path, preset)),
                Err(_) => skipped += 1,
            }
        }
        self.library
            .sort_by_key(|entry| entry.1.name.to_lowercase());
        if skipped > 0 {
            self.message(
                format!("Skipped {skipped} incompatible preset file(s)."),
                true,
            );
        }
    }
}

/// Validates every value before constructing a single host state transaction.
fn preset_state(
    preset: &RackPreset,
    params: &ShapeRackParams,
    mut state: PluginState,
) -> Result<PluginState, String> {
    preset.validate(params)?;
    for (id, pointer, _) in params.param_map() {
        let normalized = preset.parameters[&id];
        // The parameter pointers belong to params, which stays alive for this call.
        let plain = unsafe { pointer.preview_plain(normalized) };
        let value = match pointer {
            ParamPtr::FloatParam(_) => ParamValue::F32(plain),
            ParamPtr::IntParam(_) | ParamPtr::EnumParam(_) => ParamValue::I32(plain.round() as i32),
            ParamPtr::BoolParam(_) => ParamValue::Bool(plain >= 0.5),
        };
        state.params.insert(id, value);
    }
    state.version = env!("CARGO_PKG_VERSION").into();
    state.fields.insert(
        "preset-name".into(),
        serde_json::to_string(&preset.name).map_err(|error| error.to_string())?,
    );
    Ok(state)
}

fn apply(
    preset: &RackPreset,
    params: &ShapeRackParams,
    setter: &ParamSetter,
) -> Result<(), String> {
    // Replacing a rack is one state change at an audio block boundary, so the
    // host cannot record a partially recalled preset as 153 separate gestures.
    let state = preset_state(preset, params, setter.raw_context.get_state())?;
    setter.raw_context.set_state(state);
    Ok(())
}

fn factory_value<P: Param>(
    preset: &mut RackPreset,
    params: &ShapeRackParams,
    param: &P,
    value: P::Plain,
) {
    if let Some((id, _, _)) = params
        .param_map()
        .into_iter()
        .find(|(_, pointer, _)| *pointer == param.as_ptr())
    {
        preset
            .parameters
            .insert(id, param.preview_normalized(value));
    }
}

fn factory_curve(
    preset: &mut RackPreset,
    params: &ShapeRackParams,
    module: &ModuleParams,
    shape: Preset,
) {
    let curve = Curve::from_preset(shape);
    for (point, value) in module.points.iter().zip(curve.points) {
        factory_value(preset, params, &point.value, value);
    }
    factory_value(
        preset,
        params,
        &module.style,
        match curve.interpolation {
            crate::dsp::Interpolation::Linear => CurveStyle::Linear,
            crate::dsp::Interpolation::Smooth => CurveStyle::Smooth,
            crate::dsp::Interpolation::Step => CurveStyle::Step,
        },
    );
}

fn factory_presets() -> Vec<RackPreset> {
    let params = ShapeRackParams::default();
    let Ok(pump) = RackPreset::capture(&params, "Quarter-note pump") else {
        return Vec::new();
    };
    let mut clean = pump.clone();
    clean.name = "Clean rack".into();
    factory_value(&mut clean, &params, &params.volume.enabled, false);
    let mut sweep = clean.clone();
    sweep.name = "Slow filter sweep".into();
    factory_value(&mut sweep, &params, &params.filter.enabled, true);
    factory_value(&mut sweep, &params, &params.filter.amount, 1.0);
    factory_value(&mut sweep, &params, &params.division, Division::Bar);
    factory_curve(&mut sweep, &params, &params.filter, Preset::Sine);
    let mut pan = clean.clone();
    pan.name = "Stereo sway".into();
    factory_value(&mut pan, &params, &params.pan.enabled, true);
    factory_value(&mut pan, &params, &params.division, Division::Half);
    let mut warm = pump.clone();
    warm.name = "Warm rhythmic drive".into();
    factory_value(&mut warm, &params, &params.drive.enabled, true);
    factory_value(&mut warm, &params, &params.drive.amount, 0.28);
    vec![clean, pump, sweep, pan, warm]
}

#[cfg(windows)]
fn import_path(directory: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Load a ShapeRack preset")
        .add_filter("ShapeRack preset", &["json"]);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    dialog.pick_file()
}

#[cfg(windows)]
fn export_path(directory: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export ShapeRack preset")
        .add_filter("ShapeRack preset", &["json"])
        .set_file_name(preset_filename(name));
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    dialog.save_file()
}

#[cfg(not(windows))]
fn import_path(_directory: Option<&Path>) -> Option<PathBuf> {
    None
}
#[cfg(not(windows))]
fn export_path(_directory: Option<&Path>, _name: &str) -> Option<PathBuf> {
    None
}

/// Returns true only after a full rack preset has been successfully recalled.
pub fn show_controls(
    ui: &mut egui::Ui,
    params: &Arc<ShapeRackParams>,
    setter: &ParamSetter,
    state: &mut PresetUiState,
) -> bool {
    let mut selected = None;
    if let Some(job) = &state.dialog {
        match job.result.try_recv() {
            Ok(DialogResult::Import(Ok(preset))) => {
                selected = preset;
                state.dialog = None;
                if selected.is_none() {
                    state.message("Import cancelled.", false);
                }
            }
            Ok(DialogResult::Export(Ok(path))) => {
                state.dialog = None;
                state.message(
                    if path.is_some() {
                        "Preset exported."
                    } else {
                        "Export cancelled."
                    },
                    false,
                );
            }
            Ok(DialogResult::Import(Err(error)) | DialogResult::Export(Err(error))) => {
                state.dialog = None;
                state.message(error, true);
            }
            Err(TryRecvError::Disconnected) => {
                state.dialog = None;
                state.message("The file dialog could not be completed.", true);
            }
            Err(TryRecvError::Empty) => {}
        }
    }
    if !state.initialized {
        state.initialized = true;
        state.directory = library_directory();
        state.refresh(params);
    }
    let current_name = params
        .preset_name
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let current = RackPreset::capture(params, &current_name).ok();
    if state.saved_values.is_none() {
        state.saved_values = current.as_ref().map(|preset| preset.parameters.clone());
    }
    let modified = match (&current, &state.saved_values) {
        (Some(current), Some(saved)) => !values_match(&current.parameters, saved),
        _ => false,
    };
    let mut applied = false;
    let dialog_busy = state.dialog.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("RACK PRESET").small().color(MUTED));
        egui::ComboBox::from_id_salt("rack-presets").width(208.0).selected_text(format!("{current_name}{}", if modified { " *" } else { "" })).show_ui(ui, |ui| {
            ui.label(RichText::new("Factory").small().color(MUTED));
            for preset in factory_presets() {
                if ui.selectable_label(false, &preset.name).clicked() { selected = Some(preset); }
            }
            ui.separator();
            ui.label(RichText::new("My presets").small().color(MUTED));
            if state.library.is_empty() { ui.label(RichText::new("Save a preset to start your library.").small().color(MUTED)); }
            for (_, preset) in &state.library {
                if ui.selectable_label(false, &preset.name).clicked() { selected = Some(preset.clone()); }
            }
        }).response.on_hover_text("Load the entire rack: all four modules, curves, tempo controls, mix and output. * indicates edits since loading.");
        if ui.add_enabled(!dialog_busy, egui::Button::new("Save...")).on_hover_text("Save the current rack with a name in your personal preset library.").clicked() {
            state.save_name = if current_name == "Custom" { "My preset".into() } else { current_name.clone() };
            state.save_open = true;
            state.replace_existing = false;
        }
        if ui.add_enabled(!dialog_busy, egui::Button::new("Import...")).on_hover_text("Load a shared .shaperack.json preset. Use Save to add it to your library.").clicked() {
            let directory = state.directory.clone();
            let parameters = params.clone();
            match start_dialog(move || {
                DialogResult::Import(match import_path(directory.as_deref()) {
                    Some(path) => RackPreset::read(&path, &parameters).map(Some),
                    None => Ok(None),
                })
            }) {
                Ok(job) => { state.dialog = Some(job); state.message("Choose a preset in the file dialog.", false); }
                Err(error) => state.message(error, true),
            }
        }
        if ui.add_enabled(!dialog_busy, egui::Button::new("Export...")).on_hover_text("Save the current rack as a portable file you can share or back up.").clicked() {
            if let Some(preset) = current.clone() {
                let directory = state.directory.clone();
                match start_dialog(move || {
                    DialogResult::Export(match export_path(directory.as_deref(), &preset.name) {
                        Some(path) => preset.write(&path, true).map(|()| Some(path)),
                        None => Ok(None),
                    })
                }) {
                    Ok(job) => { state.dialog = Some(job); state.message("Choose an export location in the file dialog.", false); }
                    Err(error) => state.message(error, true),
                }
            }
        }
        if ui.small_button("Refresh").on_hover_text("Refresh presets saved by other ShapeRack instances.").clicked() { state.refresh(params); }
    });
    if let Some(preset) = selected {
        match apply(&preset, params, setter) {
            Ok(()) => {
                state.saved_values = Some(preset.parameters.clone());
                state.message(format!("Loaded {}.", preset.name), false);
                applied = true;
            }
            Err(error) => state.message(error, true),
        }
    }
    if state.save_open {
        let mut open = true;
        egui::Window::new("Save rack preset")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("Preset name");
                ui.add(
                    egui::TextEdit::singleline(&mut state.save_name)
                        .desired_width(290.0)
                        .char_limit(80),
                );
                let destination = state
                    .directory
                    .as_ref()
                    .map(|directory| directory.join(preset_filename(state.save_name.trim())));
                let exists = destination.as_ref().is_some_and(|path| path.exists());
                if exists {
                    ui.checkbox(
                        &mut state.replace_existing,
                        "Replace the existing preset with this name",
                    );
                }
                ui.label(
                    RichText::new("Saves all modules, curves and rack controls.")
                        .small()
                        .color(MUTED),
                );
                let enabled = destination.is_some()
                    && !state.save_name.trim().is_empty()
                    && (!exists || state.replace_existing);
                if ui
                    .add_enabled(enabled, egui::Button::new("Save preset"))
                    .clicked()
                {
                    let result = (|| {
                        let preset = RackPreset::capture(params, state.save_name.trim())?;
                        let path = destination
                            .as_ref()
                            .ok_or("Preset directory is unavailable.")?;
                        std::fs::create_dir_all(
                            path.parent().ok_or("Preset directory is invalid.")?,
                        )
                        .map_err(|error| error.to_string())?;
                        preset.write(path, state.replace_existing)?;
                        *params
                            .preset_name
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = preset.name.clone();
                        state.saved_values = Some(preset.parameters);
                        Ok::<_, String>(())
                    })();
                    match result {
                        Ok(()) => {
                            state.save_open = false;
                            state.refresh(params);
                            state.message("Preset saved to My presets.", false);
                        }
                        Err(error) => state.message(error, true),
                    }
                }
                if state.directory.is_none() {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "Your user preset folder is unavailable.",
                    );
                }
            });
        if !open {
            state.save_open = false;
        }
    }
    if !state.status.is_empty() {
        ui.label(RichText::new(&state.status).small().color(if state.error {
            Color32::from_rgb(255, 156, 140)
        } else {
            MINT
        }));
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_rack_state_recall_converts_parameters_and_preserves_editor_fields() {
        let params = ShapeRackParams::default();
        let mut preset = RackPreset::capture(&params, "All controls").unwrap();
        let map = params.param_map();
        for (id, pointer, _) in &map {
            preset.parameters.insert(id.clone(), unsafe {
                pointer.preview_normalized(pointer.preview_plain(0.75))
            });
        }
        let mut fields = BTreeMap::new();
        fields.insert("editor-state".into(), "preserved editor size".into());
        let original = PluginState {
            version: "old".into(),
            params: BTreeMap::new(),
            fields,
        };
        let recalled = preset_state(&preset, &params, original).unwrap();
        assert_eq!(recalled.params.len(), map.len());
        assert_eq!(recalled.fields["editor-state"], "preserved editor size");
        assert_eq!(recalled.fields["preset-name"], "\"All controls\"");
        for (id, pointer, _) in map {
            let expected = unsafe { pointer.preview_plain(preset.parameters[&id]) };
            match &recalled.params[&id] {
                ParamValue::F32(value) => assert!((value - expected).abs() < 1e-5),
                ParamValue::I32(value) => assert_eq!(*value, expected.round() as i32),
                ParamValue::Bool(value) => assert_eq!(*value, expected >= 0.5),
                ParamValue::String(_) => panic!("This rack has no string enum IDs"),
            }
        }
    }

    #[test]
    fn malformed_preset_cannot_construct_a_partial_host_state() {
        let params = ShapeRackParams::default();
        let mut preset = RackPreset::capture(&params, "Invalid").unwrap();
        preset.parameters.insert("mix".into(), f32::NAN);
        let original = PluginState {
            version: String::new(),
            params: BTreeMap::new(),
            fields: BTreeMap::new(),
        };
        assert!(preset_state(&preset, &params, original).is_err());
    }

    #[test]
    fn factory_presets_are_complete_valid_and_distinct() {
        let params = ShapeRackParams::default();
        let factory = factory_presets();
        assert_eq!(factory.len(), 5);
        for preset in &factory {
            preset.validate(&params).unwrap();
        }
        for (index, left) in factory.iter().enumerate() {
            for right in factory.iter().skip(index + 1) {
                assert_ne!(left.parameters, right.parameters);
            }
        }
    }

    #[test]
    fn preset_filenames_cannot_traverse_or_target_windows_device_names() {
        for name in ["../..", "CON", "LPT1", "a/b\\c", "New preset", "Øresund"] {
            let filename = preset_filename(name);
            assert!(filename.starts_with("preset-"));
            assert!(filename.ends_with(".shaperack.json"));
            assert!(!filename.contains(['/', '\\', ':']));
            assert_eq!(Path::new(&filename).components().count(), 1);
        }
    }
}
