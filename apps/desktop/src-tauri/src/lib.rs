use kiri_audio::{AudioRecordingHandle, AudioSourceKind};
use kiri_camera::CameraRecordingHandle;
use kiri_capture::{
    CaptureDiagnostics, RecordingClock, RecoveryManifest, RecoverySegment, SegmentKind,
    windows::{ScreenRecordingConfig, ScreenRecordingHandle, start_screen_segment},
};
use kiri_input::InputRecordingHandle;
use kiri_persistence::{Database, RecentProject};
use kiri_project::{
    RecordingSessionMetadata, SourceKind, SourceMedia, TimeMicros, autosave_project,
    create_project as create_project_domain, open_project as open_project_domain,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;

struct AppState {
    database: Mutex<Database>,
    recording: Mutex<Option<ActiveRecording>>,
    /// Backend-held session state shared across windows. Recordly keeps the
    /// selected source/project in the Electron main process; Tauri webviews do
    /// not share `localStorage`, so the Rust host is the source of truth.
    session: Mutex<SessionState>,
}

#[derive(Debug, Default)]
struct SessionState {
    active_project: Option<PathBuf>,
    selected_source: Option<String>,
}

#[derive(Debug, Error)]
enum CommandError {
    #[error("{0}")]
    Project(#[from] kiri_project::ProjectError),
    #[error("{0}")]
    Persistence(#[from] kiri_persistence::PersistenceError),
    #[error("{0}")]
    Settings(#[from] kiri_settings::SettingsError),
    #[error("invalid project location: {0}")]
    InvalidPath(String),
    #[error("recording failed: {0}")]
    Recording(String),
    #[error("export failed: {0}")]
    Export(String),
    #[error("editor op failed: {0}")]
    Editor(String),
}
impl Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectRequest {
    parent: PathBuf,
    title: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenProjectRequest {
    path: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectSummary {
    id: String,
    title: String,
    path: PathBuf,
    updated_at: String,
    missing: bool,
}
impl ProjectSummary {
    fn from_recent(value: RecentProject) -> Self {
        Self {
            id: value.project_id,
            title: value.title,
            path: value.path,
            updated_at: value.updated_at,
            missing: value.missing,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRecordingRequest {
    project_path: PathBuf,
    source_id: String,
    microphone_id: Option<String>,
    system_audio: bool,
    camera_id: Option<String>,
    fps: u32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingStatus {
    state: String,
    elapsed_micros: i64,
    segment_index: u32,
    message: String,
}

struct ActiveRecording {
    request: StartRecordingRequest,
    clock: RecordingClock,
    recovery: RecoveryManifest,
    screen: Option<ScreenRecordingHandle>,
    microphone: Option<AudioRecordingHandle>,
    system_audio: Option<AudioRecordingHandle>,
    camera: Option<CameraRecordingHandle>,
    input: Option<InputRecordingHandle>,
    segment_index: u32,
    paused: bool,
    diagnostics: CaptureDiagnostics,
}

// --- Recording orchestration helpers --------------------------------------
// Recordly keeps recording state in the main process with explicit
// availability checks, best-effort teardown, and interruption events. The
// helpers below bring the same shape to the Rust host without changing the
// `.kiri` layout or the Tauri command names the frontend already calls.

fn lock_recording<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, Option<ActiveRecording>>, CommandError> {
    state.recording.lock().map_err(|_| {
        CommandError::Recording("internal state unavailable: recording lock is corrupted".into())
    })
}

fn lock_database<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, Database>, CommandError> {
    state.database.lock().map_err(|_| {
        CommandError::Recording("internal state unavailable: database lock is corrupted".into())
    })
}

fn validate_start_request(request: &StartRecordingRequest) -> Result<(), CommandError> {
    if !matches!(request.fps, 30 | 60) {
        return Err(CommandError::Recording(
            "unsupported frame rate; choose 30 or 60 fps in recording settings".into(),
        ));
    }
    if request.source_id.trim().is_empty() {
        return Err(CommandError::Recording(
            "no capture source selected; pick a display or window".into(),
        ));
    }
    if request.project_path.extension().and_then(|v| v.to_str()) != Some("kiri") {
        return Err(CommandError::Recording(
            "project path must point at a .kiri project folder".into(),
        ));
    }
    Ok(())
}

/// Fail fast with actionable messages before any file or device is touched.
/// Mirrors Recordly's availability phase (source + mic/camera/loopback).
fn preflight_request(request: &StartRecordingRequest) -> Result<(), CommandError> {
    validate_start_request(request)?;
    kiri_capture::windows::validate_source_for_capture(&request.source_id)
        .map_err(|e| CommandError::Recording(e.to_string()))?;
    if let Some(id) = request.microphone_id.as_deref() {
        kiri_audio::check_device_available(AudioSourceKind::Microphone, Some(id))
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    if request.system_audio {
        kiri_audio::check_device_available(AudioSourceKind::SystemLoopback, None)
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    if let Some(id) = request.camera_id.as_deref() {
        kiri_camera::check_device_available(id)
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    Ok(())
}

fn push_device_event(active: &mut ActiveRecording, event: impl Into<String>) {
    let event = event.into();
    if !active.diagnostics.device_events.iter().any(|e| e == &event) {
        active.diagnostics.device_events.push(event);
    }
}

/// Best-effort stop of one handle: returns the error message instead of
/// failing the whole segment, so remaining tracks are still finalized.
fn stop_screen(handle: ScreenRecordingHandle, active: &mut ActiveRecording) -> Option<String> {
    match handle.stop() {
        Ok((frames, dropped, duration)) => {
            active.diagnostics.encoded_frames += frames;
            active.diagnostics.dropped_frames += dropped;
            active.diagnostics.source_fps = if duration > 0.0 {
                frames as f64 / duration
            } else {
                0.0
            };
            None
        }
        Err(e) => Some(format!("screen capture stop failed: {e}")),
    }
}

fn safe_project_name(title: &str) -> Result<String, CommandError> {
    let value: String = title
        .trim()
        .chars()
        .map(|c| {
            if c == char::from(92) || matches!(c, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    if value.is_empty() || value == "." || value == ".." {
        return Err(CommandError::InvalidPath(title.into()));
    }
    Ok(value)
}

fn summary(root: &Path, manifest: &kiri_project::ProjectManifest) -> ProjectSummary {
    ProjectSummary {
        id: manifest.id.to_string(),
        title: manifest.title.clone(),
        path: root.to_path_buf(),
        updated_at: manifest.updated_at.to_rfc3339(),
        missing: false,
    }
}

#[tauri::command]
fn create_project(
    request: CreateProjectRequest,
    state: State<'_, AppState>,
) -> Result<ProjectSummary, CommandError> {
    let root = request
        .parent
        .join(format!("{}.kiri", safe_project_name(&request.title)?));
    let manifest = create_project_domain(&root, &request.title)?;
    lock_database(&state)?.upsert_recent(&manifest.id.to_string(), &manifest.title, &root)?;
    Ok(summary(&root, &manifest))
}

#[tauri::command]
fn open_project(
    request: OpenProjectRequest,
    state: State<'_, AppState>,
) -> Result<ProjectSummary, CommandError> {
    let manifest = open_project_domain(&request.path)?;
    lock_database(&state)?.upsert_recent(
        &manifest.id.to_string(),
        &manifest.title,
        &request.path,
    )?;
    Ok(summary(&request.path, &manifest))
}

fn lock_session<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, SessionState>, CommandError> {
    state.session.lock().map_err(|_| {
        CommandError::Recording("internal state unavailable: session lock is corrupted".into())
    })
}

#[tauri::command]
fn set_active_project(path: PathBuf, state: State<'_, AppState>) -> Result<(), CommandError> {
    lock_session(&state)?.active_project = Some(path);
    Ok(())
}

#[tauri::command]
fn get_active_project(state: State<'_, AppState>) -> Option<PathBuf> {
    state.session.lock().ok()?.active_project.clone()
}

#[tauri::command]
fn set_selected_source(source_id: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    lock_session(&state)?.selected_source = Some(source_id);
    Ok(())
}

#[tauri::command]
fn get_selected_source(state: State<'_, AppState>) -> Option<String> {
    state.session.lock().ok()?.selected_source.clone()
}

#[tauri::command]
fn get_platform() -> String {
    std::env::consts::OS.into()
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, CommandError> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| CommandError::Recording(e.to_string()))?
        .join("settings.json"))
}

#[tauri::command]
fn get_all_settings(app: tauri::AppHandle) -> Result<kiri_settings::AllSettings, CommandError> {
    let store = kiri_settings::SettingsStore::new(settings_path(&app)?);
    Ok(store.load()?)
}

#[tauri::command]
fn save_all_settings(
    app: tauri::AppHandle,
    settings: kiri_settings::AllSettings,
) -> Result<(), CommandError> {
    let normalized = kiri_settings::AllSettings {
        recording: settings.recording.normalized(),
        countdown: settings.countdown.normalized(),
        ..settings
    };
    kiri_settings::SettingsStore::new(settings_path(&app)?).save(&normalized)?;
    Ok(())
}

#[tauri::command]
fn get_editor_state(project_path: PathBuf) -> Result<kiri_project::EditorState, CommandError> {
    Ok(open_project_domain(&project_path)?.editor)
}

#[tauri::command]
fn save_editor_state(
    project_path: PathBuf,
    editor: kiri_project::EditorState,
) -> Result<(), CommandError> {
    let mut project = open_project_domain(&project_path)?;
    project.editor = editor.normalized();
    project.updated_at = chrono::Utc::now();
    autosave_project(&project_path, &mut project)?;
    Ok(())
}

#[tauri::command]
fn plan_export(
    request: kiri_export::ExportRequest,
) -> Result<kiri_export::ExportPlan, CommandError> {
    kiri_export::plan_export(&request).map_err(|e| CommandError::Recording(e.to_string()))
}

// --- Phase 2/3 scaffolding commands (additive, thin wrappers) ----------------
// These expose the pure backend helpers added for editor+export (Phase 2)
// and presentation intelligence (Phase 3, rule-based only) without touching
// recording flows or the frontend.

/// Delivery bitrate preview for a preset/fps/mode triple (pure).
#[tauri::command]
fn export_target_bitrate(
    preset: kiri_export::ExportPreset,
    fps: u32,
    mode: kiri_export::EncodingMode,
) -> u32 {
    kiri_export::target_bitrate_bps(preset, fps, mode)
}

/// Parse one ffmpeg output line into a progress event (pure, no I/O).
#[tauri::command]
fn parse_ffmpeg_progress_line(line: String) -> Option<kiri_export::FfmpegProgressEvent> {
    kiri_export::parse_ffmpeg_progress_line(&line)
}

/// Scaffold runner: plans then executes ffmpeg synchronously (no
/// cancellation token yet; async jobs-style progress/cancel wiring follows
/// in the next slice). Blocking: prefer a worker thread for large exports.
#[tauri::command]
fn run_export(
    request: kiri_export::ExportRequest,
) -> Result<kiri_export::ExportOutcome, CommandError> {
    let plan =
        kiri_export::plan_export(&request).map_err(|e| CommandError::Export(e.to_string()))?;
    kiri_export::run_export_blocking(&plan, "ffmpeg", None, None, |_| {})
        .map_err(|e| CommandError::Export(e.to_string()))
}

/// Heuristic zoom suggestions from cursor telemetry (pure, rule-based only).
#[tauri::command]
fn suggest_zoom_regions(
    samples: Vec<kiri_project::editor::CursorSample>,
    total_ms: i64,
) -> Vec<kiri_project::editor::ZoomRegion> {
    kiri_project::editor::suggest_zoom_regions(&samples, total_ms)
}

/// Silence intervals from PCM peak arrays (pure, rule-based only).
#[tauri::command]
fn detect_silences(
    peaks: Vec<f32>,
    peak_sample_ms: u64,
    threshold: f32,
    min_silence_ms: u64,
) -> Vec<kiri_export::SilenceInterval> {
    kiri_export::detect_silences(&peaks, peak_sample_ms, threshold, min_silence_ms)
}

fn mutate_editor(
    project_path: &Path,
    apply: impl FnOnce(
        &mut kiri_project::EditorState,
    ) -> Result<(), kiri_project::editor::RegionOpError>,
) -> Result<kiri_project::EditorState, CommandError> {
    let mut project = open_project_domain(project_path)?;
    apply(&mut project.editor).map_err(|e| CommandError::Editor(e.to_string()))?;
    let editor = std::mem::take(&mut project.editor);
    project.editor = editor.normalized();
    project.updated_at = chrono::Utc::now();
    autosave_project(project_path, &mut project)?;
    Ok(project.editor)
}

#[tauri::command]
fn add_clip_region(
    project_path: PathBuf,
    region: kiri_project::editor::ClipRegion,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::add_clip_region(&mut editor.clips, region)
    })
}

#[tauri::command]
fn move_clip_region(
    project_path: PathBuf,
    id: String,
    new_start_ms: i64,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::move_clip_region(&mut editor.clips, &id, new_start_ms)
    })
}

#[tauri::command]
fn split_clip_region(
    project_path: PathBuf,
    id: String,
    at_ms: i64,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::split_clip_region(&mut editor.clips, &id, at_ms)
    })
}

#[tauri::command]
fn trim_clip_region(
    project_path: PathBuf,
    id: String,
    new_start_ms: i64,
    new_end_ms: i64,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::trim_clip_region(&mut editor.clips, &id, new_start_ms, new_end_ms)
    })
}

#[tauri::command]
fn add_annotation(
    project_path: PathBuf,
    region: kiri_project::editor::AnnotationRegion,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::add_annotation(&mut editor.annotations, region)
    })
}

#[tauri::command]
fn add_audio_region(
    project_path: PathBuf,
    region: kiri_project::editor::AudioRegion,
) -> Result<kiri_project::EditorState, CommandError> {
    mutate_editor(&project_path, |editor| {
        kiri_project::editor::add_audio_region(&mut editor.audio_regions, region)
    })
}

#[tauri::command]
fn list_recent_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, CommandError> {
    Ok(lock_database(&state)?
        .recent_projects()?
        .into_iter()
        .map(ProjectSummary::from_recent)
        .collect())
}

#[tauri::command]
fn list_capture_sources() -> Result<Vec<kiri_capture::CaptureSource>, String> {
    kiri_capture::windows::enumerate_sources().map_err(|error| error.to_string())
}

#[tauri::command]
fn capture_source_thumbnail(source_id: String) -> Result<String, String> {
    kiri_capture::windows::capture_thumbnail(&source_id).map_err(|error| error.to_string())
}
#[tauri::command]
fn list_audio_devices() -> Result<Vec<kiri_audio::AudioDevice>, String> {
    kiri_audio::enumerate_devices().map_err(|error| error.to_string())
}

#[tauri::command]
fn audio_meter(
    kind: AudioSourceKind,
    device_id: Option<String>,
) -> Result<kiri_audio::AudioMeter, String> {
    kiri_audio::endpoint_meter(kind, device_id.as_deref()).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_camera_devices() -> Result<Vec<kiri_camera::CameraDevice>, String> {
    kiri_camera::enumerate_devices().map_err(|error| error.to_string())
}

fn segment_path(kind: SegmentKind, index: u32) -> PathBuf {
    match kind {
        SegmentKind::Screen => format!("media/screen-{index:04}.mp4").into(),
        SegmentKind::Microphone => format!("media/microphone-{index:04}.wav").into(),
        SegmentKind::SystemAudio => format!("media/system-audio-{index:04}.wav").into(),
        SegmentKind::Camera => format!("media/camera-{index:04}.mp4").into(),
        SegmentKind::Cursor => "telemetry/cursor.jsonl".into(),
        SegmentKind::Clicks => "telemetry/clicks.jsonl".into(),
    }
}

fn append_source(project: &mut kiri_project::ProjectManifest, kind: SourceKind, path: &Path) {
    let relative_path = path.to_string_lossy().replace(char::from(92), "/");
    if !project
        .sources
        .iter()
        .any(|source| source.relative_path == relative_path)
    {
        project.sources.push(SourceMedia {
            id: Uuid::new_v4(),
            kind,
            relative_path,
        });
    }
}

fn mark_source_ready(active: &mut ActiveRecording, kind: SegmentKind) {
    if let Some(segment) = active
        .recovery
        .segments
        .iter_mut()
        .rev()
        .find(|segment| segment.kind == kind && !segment.finalized)
    {
        segment.start_micros = active.clock.elapsed_micros();
    }
}
fn start_segment(active: &mut ActiveRecording) -> Result<(), CommandError> {
    // Availability first: never create files/handles when the source or a
    // requested device is already gone. This also turns protected/minimized
    // windows into clear messages instead of generic native errors.
    preflight_request(&active.request)?;

    let root = active.request.project_path.clone();
    let index = active.segment_index;
    let baseline_len = active.recovery.segments.len();
    let start_micros = active.clock.elapsed_micros();
    let screen_path = segment_path(SegmentKind::Screen, index);
    active.recovery.segments.push(RecoverySegment {
        id: Uuid::new_v4(),
        kind: SegmentKind::Screen,
        relative_path: screen_path.clone(),
        start_micros,
        duration_micros: None,
        finalized: false,
    });
    if active.request.microphone_id.is_some() {
        active.recovery.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Microphone,
            relative_path: segment_path(SegmentKind::Microphone, index),
            start_micros,
            duration_micros: None,
            finalized: false,
        });
    }
    if active.request.system_audio {
        active.recovery.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::SystemAudio,
            relative_path: segment_path(SegmentKind::SystemAudio, index),
            start_micros,
            duration_micros: None,
            finalized: false,
        });
    }
    if active.request.camera_id.is_some() {
        active.recovery.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Camera,
            relative_path: segment_path(SegmentKind::Camera, index),
            start_micros,
            duration_micros: None,
            finalized: false,
        });
    }
    if index == 1 {
        active.recovery.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Cursor,
            relative_path: segment_path(SegmentKind::Cursor, index),
            start_micros: 0,
            duration_micros: None,
            finalized: false,
        });
        active.recovery.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Clicks,
            relative_path: segment_path(SegmentKind::Clicks, index),
            start_micros: 0,
            duration_micros: None,
            finalized: false,
        });
    }

    // Roll back the manifest entries added above so a failed start never
    // leaves phantom unfinalized segments behind.
    let rollback = |active: &mut ActiveRecording| {
        active.recovery.segments.truncate(baseline_len);
        let _ = active.recovery.commit(&root);
    };

    if let Err(e) = active.recovery.commit(&root) {
        rollback(active);
        return Err(CommandError::Recording(format!(
            "cannot write recovery manifest: {e}"
        )));
    }

    // Re-resolve the source bounds after preflight so telemetry matches the
    // live source rectangle.
    let source = kiri_capture::windows::validate_source_for_capture(&active.request.source_id)
        .map_err(|e| {
            rollback(active);
            CommandError::Recording(e.to_string())
        })?;

    let screen = start_screen_segment(ScreenRecordingConfig {
        source_id: active.request.source_id.clone(),
        output: root.join(&screen_path),
        fps: active.request.fps,
        bitrate: if active.request.fps == 60 {
            16_000_000
        } else {
            10_000_000
        },
    })
    .map_err(|e| {
        rollback(active);
        CommandError::Recording(format!("screen capture failed to start: {e}"))
    })?;

    mark_source_ready(active, SegmentKind::Screen);

    let microphone = if let Some(device) = active.request.microphone_id.clone() {
        match kiri_audio::start_recording(
            AudioSourceKind::Microphone,
            Some(device),
            root.join(segment_path(SegmentKind::Microphone, index)),
        ) {
            Ok(handle) => Some(handle),
            Err(error) => {
                let _ = screen.stop();
                rollback(active);
                return Err(CommandError::Recording(format!(
                    "microphone failed to start: {error}"
                )));
            }
        }
    } else {
        None
    };

    if microphone.is_some() {
        mark_source_ready(active, SegmentKind::Microphone);
    }

    let system_audio = if active.request.system_audio {
        match kiri_audio::start_recording(
            AudioSourceKind::SystemLoopback,
            None,
            root.join(segment_path(SegmentKind::SystemAudio, index)),
        ) {
            Ok(handle) => Some(handle),
            Err(error) => {
                if let Some(handle) = microphone {
                    let _ = handle.stop();
                }
                let _ = screen.stop();
                rollback(active);
                return Err(CommandError::Recording(format!(
                    "system audio failed to start: {error}"
                )));
            }
        }
    } else {
        None
    };

    if system_audio.is_some() {
        mark_source_ready(active, SegmentKind::SystemAudio);
    }

    let camera = if let Some(device) = active.request.camera_id.clone() {
        match kiri_camera::start_recording(
            device,
            root.join(segment_path(SegmentKind::Camera, index)),
        ) {
            Ok(handle) => Some(handle),
            Err(error) => {
                if let Some(handle) = microphone {
                    let _ = handle.stop();
                }
                if let Some(handle) = system_audio {
                    let _ = handle.stop();
                }
                let _ = screen.stop();
                rollback(active);
                return Err(CommandError::Recording(format!(
                    "camera failed to start: {error}"
                )));
            }
        }
    } else {
        None
    };

    if camera.is_some() {
        mark_source_ready(active, SegmentKind::Camera);
    }
    if let Err(e) = active.recovery.commit(&root) {
        if let Some(handle) = microphone {
            let _ = handle.stop();
        }
        if let Some(handle) = system_audio {
            let _ = handle.stop();
        }
        if let Some(handle) = camera {
            let _ = handle.stop();
        }
        let _ = screen.stop();
        rollback(active);
        return Err(CommandError::Recording(format!(
            "cannot write recovery manifest: {e}"
        )));
    }

    let input = match kiri_input::start_recording(
        root.join(segment_path(SegmentKind::Cursor, index)),
        root.join(segment_path(SegmentKind::Clicks, index)),
        source.bounds,
        start_micros,
    ) {
        Ok(handle) => handle,
        Err(error) => {
            let _ = screen.stop();
            if let Some(handle) = microphone {
                let _ = handle.stop();
            }
            if let Some(handle) = system_audio {
                let _ = handle.stop();
            }
            if let Some(handle) = camera {
                let _ = handle.stop();
            }
            rollback(active);
            return Err(CommandError::Recording(format!(
                "input telemetry failed to start: {error}"
            )));
        }
    };

    active.screen = Some(screen);
    active.microphone = microphone;
    active.system_audio = system_audio;
    active.camera = camera;
    active.input = Some(input);
    active.paused = false;
    Ok(())
}

/// Best-effort segment teardown. Every handle is drained even when one
/// stop fails, so a single wedged track can never leak threads or lose the
/// other tracks. Track failures become `device_events` + returned warnings;
/// only recovery-commit I/O failures are hard errors.
#[allow(clippy::collapsible_if)]
fn finish_segment(active: &mut ActiveRecording) -> Result<Vec<String>, CommandError> {
    let mut warnings = Vec::new();
    let end_micros = active.clock.elapsed_micros();
    if let Some(screen) = active.screen.take() {
        if let Some(warning) = stop_screen(screen, active) {
            push_device_event(active, warning.clone());
            warnings.push(warning);
        }
    }
    if let Some(audio) = active.microphone.take() {
        if let Err(e) = audio.stop() {
            let warning = format!("microphone stop failed: {e}");
            push_device_event(active, warning.clone());
            warnings.push(warning);
        }
    }
    if let Some(audio) = active.system_audio.take() {
        if let Err(e) = audio.stop() {
            let warning = format!("system audio stop failed: {e}");
            push_device_event(active, warning.clone());
            warnings.push(warning);
        }
    }
    if let Some(camera) = active.camera.take() {
        if let Err(e) = camera.stop() {
            let warning = format!("camera stop failed: {e}");
            push_device_event(active, warning.clone());
            warnings.push(warning);
        }
    }
    if let Some(input) = active.input.take() {
        if let Err(e) = input.stop() {
            let warning = format!("input telemetry stop failed: {e}");
            push_device_event(active, warning.clone());
            warnings.push(warning);
        }
    }
    for segment in active
        .recovery
        .segments
        .iter_mut()
        .filter(|segment| !segment.finalized)
    {
        segment.duration_micros = Some(end_micros.saturating_sub(segment.start_micros));
        segment.finalized = true;
    }
    active
        .recovery
        .commit(&active.request.project_path)
        .map_err(|e| CommandError::Recording(format!("cannot write recovery manifest: {e}")))?;

    // Recordly-style diagnostics sidecar next to the screen segment plus
    // timing sidecars next to audio tracks. Failures are warnings, never
    // fatal to the stop path.
    let root = active.request.project_path.clone();
    let screen_rel = segment_path(SegmentKind::Screen, active.segment_index);
    let snapshot = serde_json::json!({
        "phase": "stop",
        "segmentIndex": active.segment_index,
        "elapsedMicros": end_micros,
        "encodedFrames": active.diagnostics.encoded_frames,
        "droppedFrames": active.diagnostics.dropped_frames,
        "sourceFps": active.diagnostics.source_fps,
        "encoder": active.diagnostics.encoder,
        "deviceEvents": active.diagnostics.device_events,
        "warnings": warnings,
    });
    if let Err(e) = kiri_capture::append_diagnostics_snapshot(&root.join(&screen_rel), &snapshot) {
        warnings.push(format!("diagnostics sidecar write failed: {e}"));
    }
    for kind in [SegmentKind::Microphone, SegmentKind::SystemAudio] {
        let wanted = matches!(
            (
                kind,
                &active.request.microphone_id,
                active.request.system_audio
            ),
            (SegmentKind::Microphone, Some(_), _) | (SegmentKind::SystemAudio, _, true)
        );
        if wanted {
            let audio_rel = segment_path(kind, active.segment_index);
            let start_delay_ms = active
                .recovery
                .segments
                .iter()
                .find(|s| s.kind == kind && s.relative_path == audio_rel)
                .map(|s| s.start_micros / 1000);
            if let Err(e) = kiri_capture::AudioTimingMetadata::write_sidecar(
                &root.join(&audio_rel),
                start_delay_ms,
            ) {
                warnings.push(format!("audio timing sidecar write failed: {e}"));
            }
        }
    }
    Ok(warnings)
}

#[tauri::command]
fn start_recording(
    request: StartRecordingRequest,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingStatus, CommandError> {
    // Validate before touching any mutex-protected project state so a bad
    // request never leaves an orphan interrupted session behind.
    preflight_request(&request)?;
    let mut guard = lock_recording(&state)?;
    if guard.is_some() {
        return Err(CommandError::Recording(
            "a recording is already active; stop it before starting a new one".into(),
        ));
    }
    // Open the project first (clear error when the .kiri folder is missing).
    let project = open_project_domain(&request.project_path)?;
    let clock = RecordingClock::start().map_err(|e| CommandError::Recording(e.to_string()))?;
    let recovery = RecoveryManifest::new(project.id, clock.origin().clone());
    let mut active = ActiveRecording {
        request: request.clone(),
        clock,
        recovery,
        screen: None,
        microphone: None,
        system_audio: None,
        camera: None,
        input: None,
        segment_index: 1,
        paused: false,
        diagnostics: CaptureDiagnostics {
            source_fps: 0.0,
            encoded_frames: 0,
            dropped_frames: 0,
            queue_depth: 0,
            encoder: "Media Foundation H.264".into(),
            gpu_path: true,
            audio_drift_millis: 0.0,
            device_events: Vec::new(),
        },
    };
    // Any failure here already rolled back handles + manifest entries inside
    // `start_segment`, and `active` is dropped without touching the project.
    start_segment(&mut active)?;

    // Only now mutate the project: frame rate + interrupted session row.
    let mut project = open_project_domain(&request.project_path)?;
    project.recording_sessions.push(RecordingSessionMetadata {
        id: active.recovery.session_id,
        wall_time_utc: active.recovery.clock.wall_time_utc,
        qpc_origin_ticks: active.recovery.clock.qpc_ticks,
        qpc_frequency: active.recovery.clock.qpc_frequency,
        paused_duration: TimeMicros(0),
        interrupted: true,
    });
    project.frame_rate.numerator = request.fps;
    if let Err(e) = autosave_project(&request.project_path, &mut project) {
        // Project save failed (disk full, locked file): tear down captures so
        // no thread leaks, keep recovery for replay, then report clearly.
        let _ = finish_segment(&mut active);
        return Err(CommandError::Recording(format!(
            "recording started but project could not be saved: {e}"
        )));
    }
    let status = RecordingStatus {
        state: "recording".into(),
        elapsed_micros: 0,
        segment_index: 1,
        message: "Recording started".into(),
    };
    let _ = app.emit("recording-status", &status);
    *guard = Some(active);
    Ok(status)
}

#[tauri::command]
fn pause_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingStatus, CommandError> {
    let mut guard = lock_recording(&state)?;
    let active = guard
        .as_mut()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    if active.paused {
        return Err(CommandError::Recording(
            "recording is already paused".into(),
        ));
    }
    // Best-effort: `finish_segment` always drains handles and finalizes, so
    // pause succeeds even when one track reports a stop warning.
    let warnings = finish_segment(active)?;
    active.clock.pause();
    active.paused = true;
    let message = if warnings.is_empty() {
        "Recording paused; segment finalized".to_string()
    } else {
        format!("Recording paused with warnings: {}", warnings.join("; "))
    };
    let status = RecordingStatus {
        state: "paused".into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message,
    };
    let _ = app.emit("recording-status", &status);
    Ok(status)
}

#[tauri::command]
fn resume_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingStatus, CommandError> {
    let mut guard = lock_recording(&state)?;
    let active = guard
        .as_mut()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    if !active.paused {
        return Err(CommandError::Recording("recording is not paused".into()));
    }
    // Resume the clock first so the new segment's start timestamp excludes
    // the paused gap; on failure revert both clock and index so a retry sees
    // the original paused state instead of a half-resumed one.
    active.clock.resume();
    let previous_index = active.segment_index;
    active.segment_index += 1;
    if let Err(e) = start_segment(active) {
        active.segment_index = previous_index;
        active.clock.pause();
        return Err(e);
    }
    let status = RecordingStatus {
        state: "recording".into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message: "Recording resumed in a new recoverable segment".into(),
    };
    let _ = app.emit("recording-status", &status);
    Ok(status)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StopRecordingResult {
    status: RecordingStatus,
    diagnostics: CaptureDiagnostics,
}

#[tauri::command]
fn stop_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StopRecordingResult, CommandError> {
    // Take out of the mutex first so a slow device stop never blocks status
    // polls; the guard is released before any I/O below.
    let mut active = lock_recording(&state)?
        .take()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    let mut warnings = Vec::new();
    if !active.paused {
        warnings = finish_segment(&mut active)?;
    }
    // Update the project BEFORE deactivating recovery: if the project save
    // fails, recovery stays active so the user can still replay the segments.
    let mut project = open_project_domain(&active.request.project_path)?;
    project.duration = TimeMicros(active.clock.elapsed_micros());
    if let Some(session) = project
        .recording_sessions
        .iter_mut()
        .find(|session| session.id == active.recovery.session_id)
    {
        session.interrupted = false;
        session.paused_duration = TimeMicros(active.clock.paused_duration_micros());
    }
    for segment in active
        .recovery
        .playable_segments(&active.request.project_path)
    {
        let kind = match segment.kind {
            SegmentKind::Screen => Some(SourceKind::ScreenVideo),
            SegmentKind::Microphone => Some(SourceKind::MicrophoneAudio),
            SegmentKind::SystemAudio => Some(SourceKind::SystemAudio),
            SegmentKind::Camera => Some(SourceKind::CameraVideo),
            SegmentKind::Cursor | SegmentKind::Clicks => None,
        };
        if let Some(kind) = kind {
            append_source(&mut project, kind, &segment.relative_path);
        }
    }
    autosave_project(&active.request.project_path, &mut project)?;
    active.recovery.active = false;
    active
        .recovery
        .commit(&active.request.project_path)
        .map_err(|e| CommandError::Recording(format!("cannot write recovery manifest: {e}")))?;
    let message = if warnings.is_empty() {
        "All finalized segments were committed to the project".to_string()
    } else {
        format!("Stopped with warnings: {}", warnings.join("; "))
    };
    let status = RecordingStatus {
        state: "stopped".into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message,
    };
    let _ = app.emit("recording-status", &status);
    Ok(StopRecordingResult {
        status,
        diagnostics: active.diagnostics,
    })
}

/// HUD/countdown poll hook. Never panics, never holds the recording lock
/// across device enumeration, and emits `recording-interrupted` (Recordly
/// parity) the first time a disconnect is observed so overlay windows can
/// react even though they only poll this command.
#[allow(clippy::collapsible_if)]
#[tauri::command]
fn recording_status(app: tauri::AppHandle, state: State<'_, AppState>) -> Option<RecordingStatus> {
    // Snapshot under a short lock, then release before any enumeration.
    struct Snapshot {
        microphone_id: Option<String>,
        camera_id: Option<String>,
        system_audio: bool,
        source_id: String,
    }
    let snapshot = {
        let Ok(guard) = state.recording.lock() else {
            return None;
        };
        let active = guard.as_ref()?;
        Snapshot {
            microphone_id: active.request.microphone_id.clone(),
            camera_id: active.request.camera_id.clone(),
            system_audio: active.request.system_audio,
            source_id: active.request.source_id.clone(),
        }
    };

    let mut fresh_events: Vec<String> = Vec::new();
    let mut push_once = |event: String| {
        if !fresh_events.iter().any(|e| e == &event) {
            fresh_events.push(event);
        }
    };

    if let Some(id) = snapshot.microphone_id.as_deref() {
        if let Ok(devices) = kiri_audio::enumerate_devices() {
            if !devices
                .iter()
                .any(|d| d.kind == AudioSourceKind::Microphone && d.id == id)
            {
                push_once(format!(
                    "microphone disconnected ({id}); audio after this point is missing"
                ));
            }
        }
    }
    if snapshot.system_audio {
        if let Ok(devices) = kiri_audio::enumerate_devices() {
            if !devices
                .iter()
                .any(|d| d.kind == AudioSourceKind::SystemLoopback)
            {
                push_once("system audio device disconnected; system track is silent".into());
            }
        }
    }
    if let Some(id) = snapshot.camera_id.as_deref() {
        if let Ok(cameras) = kiri_camera::enumerate_devices() {
            if !cameras.iter().any(|d| d.id == id) {
                push_once(format!(
                    "camera disconnected ({id}); camera track is frozen"
                ));
            }
        }
    }
    if let Ok(sources) = kiri_capture::windows::enumerate_sources() {
        match sources.into_iter().find(|s| s.id == snapshot.source_id) {
            None => push_once("capture source closed; video may be frozen".into()),
            Some(source) => match source.availability {
                kiri_capture::SourceAvailability::Available => {}
                kiri_capture::SourceAvailability::Minimized => {
                    push_once("capture source minimized; video may be frozen".into());
                }
                kiri_capture::SourceAvailability::Closed => {
                    push_once("capture source closed; video may be frozen".into());
                }
                kiri_capture::SourceAvailability::Protected => {
                    push_once("capture source is protected; video may be blank".into());
                }
                kiri_capture::SourceAvailability::Invalid => {
                    push_once("capture source is not capturable".into());
                }
            },
        }
    }

    // Re-lock briefly to merge new events and build the status.
    let Ok(mut guard) = state.recording.lock() else {
        return None;
    };
    let active = guard.as_mut()?;
    let mut newly_added = Vec::new();
    for event in fresh_events {
        if !active.diagnostics.device_events.iter().any(|e| e == &event) {
            active.diagnostics.device_events.push(event.clone());
            newly_added.push(event);
        }
    }
    let message = active
        .diagnostics
        .device_events
        .last()
        .cloned()
        .unwrap_or_else(|| {
            if active.paused {
                "Recording is paused".into()
            } else {
                "Recording session is active".into()
            }
        });
    let status = RecordingStatus {
        state: if active.paused { "paused" } else { "recording" }.into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message,
    };
    // Emit interruption once per new event so HUD/countdown windows (which
    // only poll) can surface it immediately, mirroring Recordly's
    // `emitRecordingInterrupted`.
    for event in newly_added {
        let _ = app.emit(
            "recording-interrupted",
            serde_json::json!({
                "reason": "device-disconnected",
                "message": event,
            }),
        );
    }
    Some(status)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryCandidate {
    project_path: PathBuf,
    project_title: String,
    session_id: String,
    finalized_segments: usize,
}

#[tauri::command]
fn list_recoverable_recordings(
    state: State<'_, AppState>,
) -> Result<Vec<RecoveryCandidate>, CommandError> {
    let recents = lock_database(&state)?.recent_projects()?;
    let mut candidates = Vec::new();
    for recent in recents.into_iter().filter(|value| !value.missing) {
        // Lenient: one corrupt manifest must not fail the whole list.
        if let Some(recovery) = RecoveryManifest::replay_lenient(&recent.path).filter(|v| v.active)
        {
            candidates.push(RecoveryCandidate {
                project_path: recent.path.clone(),
                project_title: recent.title,
                session_id: recovery.session_id.to_string(),
                finalized_segments: recovery.playable_segments(&recent.path).len(),
            });
        }
    }
    Ok(candidates)
}

#[tauri::command]
fn recover_recording(project_path: PathBuf) -> Result<ProjectSummary, CommandError> {
    let mut recovery = RecoveryManifest::replay(&project_path)
        .map_err(|e| {
            CommandError::Recording(format!(
                "recovery manifest is unreadable or corrupt and cannot be replayed: {e}"
            ))
        })?
        .ok_or_else(|| CommandError::Recording("no recovery manifest exists".into()))?;
    let mut project = open_project_domain(&project_path)?;
    for segment in recovery.playable_segments(&project_path) {
        let kind = match segment.kind {
            SegmentKind::Screen => Some(SourceKind::ScreenVideo),
            SegmentKind::Microphone => Some(SourceKind::MicrophoneAudio),
            SegmentKind::SystemAudio => Some(SourceKind::SystemAudio),
            SegmentKind::Camera => Some(SourceKind::CameraVideo),
            SegmentKind::Cursor | SegmentKind::Clicks => None,
        };
        if let Some(kind) = kind {
            append_source(&mut project, kind, &segment.relative_path);
        }
    }
    recovery.active = false;
    recovery
        .commit(&project_path)
        .map_err(|e| CommandError::Recording(e.to_string()))?;
    autosave_project(&project_path, &mut project)?;
    Ok(summary(&project_path, &project))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::new().build())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;
            let crash_directory = app_data.join("crashes");
            std::fs::create_dir_all(&crash_directory)?;
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
                let report = format!(
                    "timestamp={timestamp}
panic={info}
"
                );
                let _ = std::fs::write(
                    crash_directory.join(format!("crash-{timestamp}.log")),
                    report,
                );
                previous_hook(info);
            }));
            let database = Database::open(&app_data.join("kiri.db"))
                .map_err(Box::<dyn std::error::Error>::from)?;
            app.manage(AppState {
                database: Mutex::new(database),
                recording: Mutex::new(None),
                session: Mutex::new(SessionState::default()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            set_active_project,
            get_active_project,
            set_selected_source,
            get_selected_source,
            get_platform,
            get_all_settings,
            save_all_settings,
            get_editor_state,
            save_editor_state,
            plan_export,
            list_recent_projects,
            list_capture_sources,
            capture_source_thumbnail,
            list_audio_devices,
            audio_meter,
            list_camera_devices,
            start_recording,
            pause_recording,
            resume_recording,
            stop_recording,
            recording_status,
            list_recoverable_recordings,
            recover_recording,
            export_target_bitrate,
            parse_ffmpeg_progress_line,
            run_export,
            suggest_zoom_regions,
            detect_silences,
            add_clip_region,
            move_clip_region,
            split_clip_region,
            trim_clip_region,
            add_annotation,
            add_audio_region
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kiri");
}
