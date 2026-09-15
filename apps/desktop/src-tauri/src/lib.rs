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
    state
        .database
        .lock()
        .expect("database mutex poisoned")
        .upsert_recent(&manifest.id.to_string(), &manifest.title, &root)?;
    Ok(summary(&root, &manifest))
}

#[tauri::command]
fn open_project(
    request: OpenProjectRequest,
    state: State<'_, AppState>,
) -> Result<ProjectSummary, CommandError> {
    let manifest = open_project_domain(&request.path)?;
    state
        .database
        .lock()
        .expect("database mutex poisoned")
        .upsert_recent(&manifest.id.to_string(), &manifest.title, &request.path)?;
    Ok(summary(&request.path, &manifest))
}

#[tauri::command]
fn set_active_project(path: PathBuf, state: State<'_, AppState>) -> Result<(), CommandError> {
    state
        .session
        .lock()
        .expect("session mutex poisoned")
        .active_project = Some(path);
    Ok(())
}

#[tauri::command]
fn get_active_project(state: State<'_, AppState>) -> Option<PathBuf> {
    state
        .session
        .lock()
        .expect("session mutex poisoned")
        .active_project
        .clone()
}

#[tauri::command]
fn set_selected_source(source_id: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    state
        .session
        .lock()
        .expect("session mutex poisoned")
        .selected_source = Some(source_id);
    Ok(())
}

#[tauri::command]
fn get_selected_source(state: State<'_, AppState>) -> Option<String> {
    state
        .session
        .lock()
        .expect("session mutex poisoned")
        .selected_source
        .clone()
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

#[tauri::command]
fn list_recent_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, CommandError> {
    Ok(state
        .database
        .lock()
        .expect("database mutex poisoned")
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
    let root = active.request.project_path.clone();
    let index = active.segment_index;
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
    active
        .recovery
        .commit(&root)
        .map_err(|e| CommandError::Recording(e.to_string()))?;

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
    .map_err(|e| CommandError::Recording(e.to_string()))?;

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
                return Err(CommandError::Recording(error.to_string()));
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
                return Err(CommandError::Recording(error.to_string()));
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
                return Err(CommandError::Recording(error.to_string()));
            }
        }
    } else {
        None
    };

    if camera.is_some() {
        mark_source_ready(active, SegmentKind::Camera);
    }
    active
        .recovery
        .commit(&root)
        .map_err(|e| CommandError::Recording(e.to_string()))?;

    let enumerated = kiri_capture::windows::enumerate_sources()
        .map_err(|e| CommandError::Recording(e.to_string()))?;
    let source = enumerated
        .into_iter()
        .find(|source| source.id == active.request.source_id);
    let Some(source) = source else {
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
        return Err(CommandError::Recording(
            "selected source closed before telemetry started".into(),
        ));
    };
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
            return Err(CommandError::Recording(error.to_string()));
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

fn finish_segment(active: &mut ActiveRecording) -> Result<(), CommandError> {
    let end_micros = active.clock.elapsed_micros();
    if let Some(screen) = active.screen.take() {
        let (frames, dropped, duration) = screen
            .stop()
            .map_err(|e| CommandError::Recording(e.to_string()))?;
        active.diagnostics.encoded_frames += frames;
        active.diagnostics.dropped_frames += dropped;
        active.diagnostics.source_fps = if duration > 0.0 {
            frames as f64 / duration
        } else {
            0.0
        };
    }
    if let Some(audio) = active.microphone.take() {
        audio
            .stop()
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    if let Some(input) = active.input.take() {
        input
            .stop()
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    if let Some(camera) = active.camera.take() {
        camera
            .stop()
            .map_err(|e| CommandError::Recording(e.to_string()))?;
    }
    if let Some(audio) = active.system_audio.take() {
        audio
            .stop()
            .map_err(|e| CommandError::Recording(e.to_string()))?;
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
        .map_err(|e| CommandError::Recording(e.to_string()))
}

#[tauri::command]
fn start_recording(
    request: StartRecordingRequest,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingStatus, CommandError> {
    let mut guard = state.recording.lock().expect("recording mutex poisoned");
    if guard.is_some() {
        return Err(CommandError::Recording(
            "a recording is already active".into(),
        ));
    }
    let mut project = open_project_domain(&request.project_path)?;
    let clock = RecordingClock::start().map_err(|e| CommandError::Recording(e.to_string()))?;
    let recovery = RecoveryManifest::new(project.id, clock.origin().clone());
    project.recording_sessions.push(RecordingSessionMetadata {
        id: recovery.session_id,
        wall_time_utc: recovery.clock.wall_time_utc,
        qpc_origin_ticks: recovery.clock.qpc_ticks,
        qpc_frequency: recovery.clock.qpc_frequency,
        paused_duration: TimeMicros(0),
        interrupted: true,
    });
    project.frame_rate.numerator = request.fps;
    autosave_project(&request.project_path, &mut project)?;
    let mut active = ActiveRecording {
        request,
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
    start_segment(&mut active)?;
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
    let mut guard = state.recording.lock().expect("recording mutex poisoned");
    let active = guard
        .as_mut()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    if active.paused {
        return Err(CommandError::Recording(
            "recording is already paused".into(),
        ));
    }
    finish_segment(active)?;
    active.clock.pause();
    active.paused = true;
    let status = RecordingStatus {
        state: "paused".into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message: "Recording paused; segment finalized".into(),
    };
    let _ = app.emit("recording-status", &status);
    Ok(status)
}

#[tauri::command]
fn resume_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingStatus, CommandError> {
    let mut guard = state.recording.lock().expect("recording mutex poisoned");
    let active = guard
        .as_mut()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    if !active.paused {
        return Err(CommandError::Recording("recording is not paused".into()));
    }
    active.clock.resume();
    active.segment_index += 1;
    start_segment(active)?;
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
    let mut active = state
        .recording
        .lock()
        .expect("recording mutex poisoned")
        .take()
        .ok_or_else(|| CommandError::Recording("no recording is active".into()))?;
    if !active.paused {
        finish_segment(&mut active)?;
    }
    active.recovery.active = false;
    active
        .recovery
        .commit(&active.request.project_path)
        .map_err(|e| CommandError::Recording(e.to_string()))?;
    let mut project = open_project_domain(&active.request.project_path)?;
    project.duration = TimeMicros(active.clock.elapsed_micros());
    if let Some(session) = project
        .recording_sessions
        .iter_mut()
        .find(|session| session.id == active.recovery.session_id)
    {
        session.interrupted = false;
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
    let status = RecordingStatus {
        state: "stopped".into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message: "All finalized segments were committed to the project".into(),
    };
    let _ = app.emit("recording-status", &status);
    Ok(StopRecordingResult {
        status,
        diagnostics: active.diagnostics,
    })
}

#[tauri::command]
#[allow(clippy::collapsible_if)]
fn recording_status(state: State<'_, AppState>) -> Option<RecordingStatus> {
    let mut guard = state.recording.lock().expect("recording mutex poisoned");
    let active = guard.as_mut()?;
    if let Ok(devices) = kiri_audio::enumerate_devices() {
        if let Some(id) = active.request.microphone_id.as_ref() {
            if !devices.iter().any(|device| &device.id == id)
                && !active
                    .diagnostics
                    .device_events
                    .iter()
                    .any(|event| event == "microphone disconnected")
            {
                active
                    .diagnostics
                    .device_events
                    .push("microphone disconnected".into());
            }
        }
    }
    if let Ok(cameras) = kiri_camera::enumerate_devices() {
        if let Some(id) = active.request.camera_id.as_ref() {
            if !cameras.iter().any(|device| &device.id == id)
                && !active
                    .diagnostics
                    .device_events
                    .iter()
                    .any(|event| event == "camera disconnected")
            {
                active
                    .diagnostics
                    .device_events
                    .push("camera disconnected".into());
            }
        }
    }
    Some(RecordingStatus {
        state: if active.paused { "paused" } else { "recording" }.into(),
        elapsed_micros: active.clock.elapsed_micros(),
        segment_index: active.segment_index,
        message: active
            .diagnostics
            .device_events
            .last()
            .cloned()
            .unwrap_or_else(|| "Recording session is active".into()),
    })
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
    let recents = state
        .database
        .lock()
        .expect("database mutex poisoned")
        .recent_projects()?;
    let mut candidates = Vec::new();
    for recent in recents.into_iter().filter(|value| !value.missing) {
        if let Some(recovery) = RecoveryManifest::replay(&recent.path)
            .map_err(|e| CommandError::Recording(e.to_string()))?
            .filter(|v| v.active)
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
        .map_err(|e| CommandError::Recording(e.to_string()))?
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
            recover_recording
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kiri");
}
