//! Unified typed settings ported from Recordly's `*-settings.json` files.
//!
//! Recordly mapping:
//! - `app-settings.json` -> [`AppSettings`]
//! - recording preferences -> [`RecordingPreferences`]
//! - `countdown-settings.json` -> [`CountdownSettings`]
//! - `hud-overlay-settings.json` -> [`HudOverlaySettings`]
//! - `shortcuts.json` -> [`ShortcutProfile`]
//!
//! Stored as individual JSON files under the app-data directory by the
//! Tauri host. This crate owns defaults, validation, and merge rules.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default = "default_true")]
    pub reduce_motion_follows_system: bool,
    #[serde(default)]
    pub recordings_dir: Option<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            reduce_motion_follows_system: true,
            recordings_dir: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPreferences {
    #[serde(default)]
    pub microphone_id: Option<String>,
    #[serde(default = "default_true")]
    pub system_audio: bool,
    #[serde(default)]
    pub camera_id: Option<String>,
    #[serde(default = "default_fps")]
    pub fps: u32,
}

fn default_fps() -> u32 {
    60
}

impl Default for RecordingPreferences {
    fn default() -> Self {
        Self {
            microphone_id: None,
            system_audio: true,
            camera_id: None,
            fps: 60,
        }
    }
}

impl RecordingPreferences {
    pub fn normalized(mut self) -> Self {
        if self.fps != 30 && self.fps != 60 {
            self.fps = 60;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountdownSettings {
    #[serde(default = "default_countdown_secs")]
    pub seconds: u32,
}

fn default_countdown_secs() -> u32 {
    3
}

impl Default for CountdownSettings {
    fn default() -> Self {
        Self { seconds: 3 }
    }
}

impl CountdownSettings {
    pub fn normalized(mut self) -> Self {
        if self.seconds != 0 && self.seconds != 3 && self.seconds != 5 {
            self.seconds = 3;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HudOverlaySettings {
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub click_effects: bool,
}

impl Default for HudOverlaySettings {
    fn default() -> Self {
        Self {
            visible: true,
            click_effects: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShortcutProfile {
    #[default]
    CtrlShift,
    CtrlAlt,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllSettings {
    #[serde(default)]
    pub app: AppSettings,
    #[serde(default)]
    pub recording: RecordingPreferences,
    #[serde(default)]
    pub countdown: CountdownSettings,
    #[serde(default)]
    pub hud_overlay: HudOverlaySettings,
    #[serde(default)]
    pub shortcuts: ShortcutProfile,
}

/// File-backed store for one settings document.
pub struct SettingsStore<T> {
    path: PathBuf,
    _marker: std::marker::PhantomData<T>,
}

impl<T> SettingsStore<T>
where
    T: Default + Serialize + for<'de> Deserialize<'de>,
{
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn load(&self) -> Result<T, SettingsError> {
        if !self.path.exists() {
            return Ok(T::default());
        }
        let raw = fs::read(&self.path)?;
        if raw.is_empty() {
            return Ok(T::default());
        }
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn save(&self, value: &T) -> Result<(), SettingsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn settings_file(app_data: &Path, name: &str) -> PathBuf {
    app_data.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::<AllSettings>::new(dir.path().join("settings.json"));
        let loaded = store.load().unwrap();
        assert_eq!(loaded, AllSettings::default());
        store.save(&loaded).unwrap();
        assert_eq!(store.load().unwrap(), AllSettings::default());
    }

    #[test]
    fn recording_prefs_normalize_fps() {
        let prefs = RecordingPreferences {
            fps: 24,
            ..Default::default()
        }
        .normalized();
        assert_eq!(prefs.fps, 60);
    }

    #[test]
    fn countdown_normalizes_to_supported_values() {
        let settings = CountdownSettings { seconds: 9 }.normalized();
        assert_eq!(settings.seconds, 3);
    }

    #[test]
    fn corrupt_settings_surface_json_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        let store = SettingsStore::<AllSettings>::new(path);
        assert!(matches!(store.load().unwrap_err(), SettingsError::Json(_)));
    }
}
