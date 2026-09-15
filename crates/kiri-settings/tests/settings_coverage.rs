use kiri_settings::{
    AllSettings, AppSettings, CountdownSettings, HudOverlaySettings, RecordingPreferences,
    SettingsStore, ShortcutProfile, ThemeMode, settings_file,
};
use std::fs;

fn store_in(dir: &tempfile::TempDir) -> SettingsStore<AllSettings> {
    SettingsStore::new(dir.path().join("settings.json"))
}

#[test]
fn missing_file_loads_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = store_in(&dir).load().unwrap();
    assert_eq!(loaded, AllSettings::default());
}

#[test]
fn empty_file_loads_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, b"").unwrap();
    let loaded = SettingsStore::<AllSettings>::new(path).load().unwrap();
    assert_eq!(loaded, AllSettings::default());
}

#[test]
fn custom_values_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let custom = AllSettings {
        app: AppSettings {
            theme: ThemeMode::Dark,
            reduce_motion_follows_system: false,
            recordings_dir: Some("D:/videos".into()),
        },
        recording: RecordingPreferences {
            microphone_id: Some("mic:1".into()),
            system_audio: false,
            camera_id: None,
            fps: 30,
        },
        countdown: CountdownSettings { seconds: 0 },
        hud_overlay: HudOverlaySettings {
            visible: false,
            click_effects: true,
        },
        shortcuts: ShortcutProfile::CtrlAlt,
    };
    store.save(&custom).unwrap();
    assert_eq!(store.load().unwrap(), custom);
}

#[test]
fn recording_prefs_keep_supported_fps_and_fix_the_rest() {
    for fps in [30, 60] {
        let prefs = RecordingPreferences {
            fps,
            ..Default::default()
        }
        .normalized();
        assert_eq!(prefs.fps, fps);
    }
    let fixed = RecordingPreferences {
        fps: 24,
        ..Default::default()
    }
    .normalized();
    assert_eq!(fixed.fps, 60);
}

#[test]
fn countdown_keeps_supported_values_and_repairs_the_rest() {
    for seconds in [0, 3, 5, 10] {
        let kept = CountdownSettings { seconds }.normalized();
        assert_eq!(kept.seconds, seconds);
    }
    for seconds in [9, u32::MAX] {
        let fixed = CountdownSettings { seconds }.normalized();
        assert_eq!(fixed.seconds, 3);
    }
}

#[test]
fn corrupt_settings_surface_json_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, b"{not json").unwrap();
    let err = SettingsStore::<AllSettings>::new(path).load().unwrap_err();
    assert!(matches!(err, kiri_settings::SettingsError::Json(_)));
}

#[test]
fn save_creates_missing_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("dir").join("settings.json");
    let store = SettingsStore::<AllSettings>::new(path.clone());
    store.save(&AllSettings::default()).unwrap();
    assert!(path.is_file());
    assert_eq!(store.load().unwrap(), AllSettings::default());
}

#[test]
fn settings_file_joins_app_data_dir() {
    let joined = settings_file(std::path::Path::new("D:/app-data"), "settings.json");
    assert_eq!(
        joined,
        std::path::PathBuf::from("D:/app-data/settings.json")
    );
}

#[test]
fn theme_and_shortcut_serde_use_kebab_case() {
    let theme: ThemeMode = serde_json::from_str("\"dark\"").unwrap();
    assert_eq!(theme, ThemeMode::Dark);
    assert_eq!(
        serde_json::to_string(&ThemeMode::Light).unwrap(),
        "\"light\""
    );
    let profile: ShortcutProfile = serde_json::from_str("\"ctrl-alt\"").unwrap();
    assert_eq!(profile, ShortcutProfile::CtrlAlt);
    assert_eq!(
        serde_json::to_string(&ShortcutProfile::CtrlShift).unwrap(),
        "\"ctrl-shift\""
    );
}

#[test]
fn partial_json_falls_back_to_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, br#"{"recording": {"fps": 30}}"#).unwrap();
    let loaded = SettingsStore::<AllSettings>::new(path).load().unwrap();
    assert_eq!(loaded.recording.fps, 30);
    assert_eq!(loaded.app, AppSettings::default());
    assert_eq!(loaded.shortcuts, ShortcutProfile::CtrlShift);
}

#[test]
fn store_exposes_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let store = SettingsStore::<AllSettings>::new(path.clone());
    assert_eq!(store.path(), path.as_path());
}
