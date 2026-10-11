//! Portable, validated rack presets. File operations run on the GUI or dialog
//! worker thread, never in the audio callback; editor state is not serialized.

use crate::params::ShapeRackParams;
use nih_plug::prelude::Params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PRESET_FORMAT: &str = "shaperack-preset";
pub const PRESET_VERSION: u32 = 1;
pub const MAX_PRESET_BYTES: usize = 1024 * 1024;
const MAX_NAME_CHARACTERS: usize = 80;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackPreset {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// Normalized values keyed by stable parameter ID, independent of host API.
    pub parameters: BTreeMap<String, f32>,
}

impl RackPreset {
    pub fn capture(params: &ShapeRackParams, name: &str) -> Result<Self, String> {
        let parameters = params
            .param_map()
            .into_iter()
            .map(|(id, pointer, _)| {
                // param_map() returns pointers owned by params, which remains
                // borrowed and alive for this entire read-only capture.
                let value = unsafe { pointer.unmodulated_normalized_value() };
                (id, value)
            })
            .collect();
        let preset = Self {
            format: PRESET_FORMAT.to_owned(),
            version: PRESET_VERSION,
            name: name.trim().to_owned(),
            parameters,
        };
        preset.validate(params)?;
        Ok(preset)
    }

    pub fn validate(&self, params: &ShapeRackParams) -> Result<(), String> {
        self.validate_values()?;
        let expected: BTreeSet<_> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        let actual: BTreeSet<_> = self.parameters.keys().cloned().collect();
        if let Some(id) = expected.difference(&actual).next() {
            return Err(format!("Preset is missing parameter '{id}'."));
        }
        if let Some(id) = actual.difference(&expected).next() {
            return Err(format!("Preset contains unknown parameter '{id}'."));
        }
        Ok(())
    }

    fn validate_values(&self) -> Result<(), String> {
        if self.format != PRESET_FORMAT {
            return Err("This file is not a ShapeRack rack preset.".to_owned());
        }
        if self.version != PRESET_VERSION {
            return Err(format!(
                "Unsupported preset schema version {}.",
                self.version
            ));
        }
        if self.name.is_empty()
            || self.name.trim() != self.name
            || self.name.chars().count() > MAX_NAME_CHARACTERS
            || self.name.chars().any(char::is_control)
        {
            return Err("Preset name must contain 1–80 characters without surrounding whitespace or control characters.".to_owned());
        }
        for (id, value) in &self.parameters {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(format!(
                    "Preset parameter '{id}' must be finite and between 0 and 1."
                ));
            }
        }
        Ok(())
    }

    pub fn read(path: &Path, params: &ShapeRackParams) -> Result<Self, String> {
        let file = File::open(path).map_err(|error| format!("Could not open preset: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Could not inspect preset: {error}"))?;
        if !metadata.is_file() {
            return Err("Preset path must refer to a file.".to_owned());
        }
        if metadata.len() > MAX_PRESET_BYTES as u64 {
            return Err("Preset file exceeds the 1 MiB size limit.".to_owned());
        }
        // The cap still applies if another process grows the file after metadata().
        let mut bytes = Vec::new();
        file.take(MAX_PRESET_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read preset: {error}"))?;
        if bytes.len() > MAX_PRESET_BYTES {
            return Err("Preset file exceeds the 1 MiB size limit.".to_owned());
        }
        let preset: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid preset JSON: {error}"))?;
        preset.validate(params)?;
        Ok(preset)
    }

    pub fn write(&self, path: &Path, overwrite: bool) -> Result<(), String> {
        // write() accepts public struct values too, so never assume capture() or
        // read() was used; validate the complete current product map here as well.
        self.validate(&ShapeRackParams::default())?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not encode preset: {error}"))?;
        if bytes.len() > MAX_PRESET_BYTES {
            return Err("Preset file exceeds the 1 MiB size limit.".to_owned());
        }
        let file_name = path
            .file_name()
            .ok_or_else(|| "Choose a preset filename.".to_owned())?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let (temporary, mut file) = create_temporary(parent, file_name)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not write preset: {error}"))?;
        drop(file);

        if overwrite {
            // std::fs::rename replaces files on Windows and Unix. The completed,
            // synced temp is in the same directory; the original is never truncated.
            fs::rename(&temporary.0, path)
                .map_err(|error| format!("Could not replace preset: {error}"))?;
        } else {
            // A hard link publishes this completed file only if the destination
            // does not exist. Unlike exists()+rename(), this cannot race-overwrite
            // another saved preset. NTFS and ordinary Unix filesystems support it.
            fs::hard_link(&temporary.0, path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists || path.exists() {
                    "Preset already exists. Choose a different name or explicitly allow replacement.".to_owned()
                } else {
                    format!("Could not save preset without replacing an existing file: {error}")
                }
            })?;
        }
        Ok(())
    }
}

/// Removes only the exact file this write operation created, including failures.
struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn create_temporary(
    parent: &Path,
    file_name: &std::ffi::OsStr,
) -> Result<(TemporaryFile, File), String> {
    for _ in 0..32 {
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(
            ".shaperack-tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((TemporaryFile(path), file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create preset temporary file: {error}")),
        }
    }
    Err("Could not allocate a unique preset temporary file.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::SEGMENT_COUNT;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> PathBuf {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/preset-tests");
        fs::create_dir_all(&base).unwrap();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = base.join(format!(
            "{}-{timestamp}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        // Fixtures intentionally remain in ignored target/, avoiding recursive deletion.
        directory
    }

    fn captured(name: &str) -> (ShapeRackParams, RackPreset) {
        let parameters = ShapeRackParams::default();
        let preset = RackPreset::capture(&parameters, name).unwrap();
        (parameters, preset)
    }

    fn raw_write(path: &Path, preset: &RackPreset) {
        fs::write(path, serde_json::to_vec(preset).unwrap()).unwrap();
    }

    #[test]
    fn capture_contains_every_audio_parameter_and_no_editor_fields() {
        let (parameters, preset) = captured("  My rhythm  ");
        assert_eq!(preset.format, PRESET_FORMAT);
        assert_eq!(preset.version, PRESET_VERSION);
        assert_eq!(preset.name, "My rhythm");
        assert_eq!(preset.parameters.len(), parameters.param_map().len());
        assert!(preset.parameters.contains_key("bypass"));
        assert!(!preset.parameters.contains_key("editor-state"));
        assert!(!preset.parameters.contains_key("preset-name"));
        for module in ["volume", "filter", "pan", "drive"] {
            for segment in 1..=SEGMENT_COUNT {
                assert_eq!(
                    preset.parameters[&format!("{module}_tension_{segment}")],
                    0.5
                );
            }
        }
    }

    #[test]
    fn portable_roundtrip_preserves_all_values_including_segment_tensions() {
        let (parameters, mut preset) = captured("Groove α");
        let denominator = (preset.parameters.len() + 1) as f32;
        for (index, value) in preset.parameters.values_mut().enumerate() {
            *value = (index + 1) as f32 / denominator;
        }
        let path = fixture().join("groove.shaperack.json");
        preset.write(&path, false).unwrap();
        let restored = RackPreset::read(&path, &parameters).unwrap();
        assert_eq!(restored, preset);
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("shaperack-preset"));
    }

    #[test]
    fn metadata_and_names_are_validated() {
        let (parameters, original) = captured("Valid");
        let mut invalid = original.clone();
        invalid.format = "another-product".to_owned();
        assert!(invalid.validate(&parameters).is_err());
        invalid = original.clone();
        invalid.version += 1;
        assert!(invalid.validate(&parameters).is_err());
        for name in ["", " leading", "trailing ", "line\nbreak"] {
            invalid = original.clone();
            invalid.name = name.to_owned();
            assert!(invalid.validate(&parameters).is_err());
        }
        assert!(RackPreset::capture(&parameters, &"é".repeat(80)).is_ok());
        assert!(RackPreset::capture(&parameters, &"é".repeat(81)).is_err());
    }

    #[test]
    fn nonfinite_and_out_of_range_values_are_rejected() {
        let (parameters, original) = captured("Values");
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
            let mut invalid = original.clone();
            invalid.parameters.insert("mix".to_owned(), value);
            assert!(invalid.validate(&parameters).is_err());
        }
    }

    #[test]
    fn missing_unknown_and_invalid_file_parameters_are_rejected() {
        let (parameters, original) = captured("Map");
        let path = fixture().join("invalid.shaperack.json");
        let mut invalid = original.clone();
        invalid.parameters.remove("volume_tension_1");
        raw_write(&path, &invalid);
        assert!(RackPreset::read(&path, &parameters)
            .unwrap_err()
            .contains("missing"));
        invalid = original.clone();
        invalid.parameters.insert("unknown-control".to_owned(), 0.5);
        raw_write(&path, &invalid);
        assert!(RackPreset::read(&path, &parameters)
            .unwrap_err()
            .contains("unknown"));
        invalid = original;
        invalid.parameters.insert("mix".to_owned(), 1.1);
        raw_write(&path, &invalid);
        assert!(RackPreset::read(&path, &parameters).is_err());
    }

    #[test]
    fn malformed_unknown_field_and_oversized_files_are_rejected() {
        let (parameters, preset) = captured("JSON");
        let path = fixture().join("bad.shaperack.json");
        fs::write(&path, b"{bad JSON").unwrap();
        assert!(RackPreset::read(&path, &parameters).is_err());
        let mut value = serde_json::to_value(preset).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unknown-field".to_owned(), serde_json::json!(true));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(RackPreset::read(&path, &parameters).is_err());
        fs::write(&path, vec![b' '; MAX_PRESET_BYTES + 1]).unwrap();
        assert!(RackPreset::read(&path, &parameters)
            .unwrap_err()
            .contains("size limit"));
    }

    #[test]
    fn overwrite_is_explicit_and_invalid_replacements_preserve_original() {
        let (parameters, mut preset) = captured("First");
        let directory = fixture();
        let path = directory.join("existing.shaperack.json");
        preset.write(&path, false).unwrap();
        let original = fs::read(&path).unwrap();
        preset.name = "Second".to_owned();
        assert!(preset.write(&path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        preset.parameters.insert("mix".to_owned(), f32::NAN);
        assert!(preset.write(&path, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        preset.parameters.insert("mix".to_owned(), 0.25);
        preset.write(&path, true).unwrap();
        assert_eq!(RackPreset::read(&path, &parameters).unwrap(), preset);
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            1,
            "Temporary file leaked"
        );
    }

    #[test]
    fn failed_file_operations_leave_no_partial_preset_or_temporary_file() {
        let (parameters, preset) = captured("IO");
        let directory = fixture();
        assert!(RackPreset::read(&directory.join("missing.json"), &parameters).is_err());
        assert!(preset
            .write(&directory.join("missing-parent/preset.json"), false)
            .is_err());
        assert!(!directory.join("missing-parent").exists());
        let blocked = directory.join("not-a-file.json");
        fs::create_dir(&blocked).unwrap();
        assert!(preset.write(&blocked, true).is_err());
        assert!(blocked.is_dir());
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            1,
            "Temporary file leaked"
        );
    }
}
