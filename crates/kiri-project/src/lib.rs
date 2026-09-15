use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod editor;
pub use editor::EditorState;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;
pub const MANIFEST_FILE: &str = "project.json";
pub const PROJECT_DIRECTORIES: &[&str] = &[
    "media",
    "telemetry",
    "transcript",
    "assets",
    "cache/proxies",
    "cache/thumbnails",
    "cache/waveforms",
    "recovery",
    "exports",
];

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project validation failed: {0}")]
    Validation(String),
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeMicros(pub i64);

impl TimeMicros {
    pub fn validate(self) -> Result<(), ProjectError> {
        if self.0 < 0 {
            return Err(ProjectError::Validation("time cannot be negative".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub fn validate(self) -> Result<(), ProjectError> {
        if self.numerator == 0 || self.denominator == 0 {
            return Err(ProjectError::Validation(
                "frame-rate values must be non-zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub frame_rate: FrameRate,
    pub duration: TimeMicros,
    #[serde(default)]
    pub sources: Vec<SourceMedia>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub edit_regions: Vec<EditRegion>,
    #[serde(default)]
    pub artifacts: Vec<GeneratedArtifact>,
    #[serde(default)]
    pub cache_entries: Vec<CacheEntry>,
    #[serde(default)]
    pub recording_sessions: Vec<RecordingSessionMetadata>,
    #[serde(default)]
    pub editor: EditorState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSessionMetadata {
    pub id: Uuid,
    pub wall_time_utc: DateTime<Utc>,
    pub qpc_origin_ticks: i64,
    pub qpc_frequency: i64,
    pub paused_duration: TimeMicros,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceMedia {
    pub id: Uuid,
    pub kind: SourceKind,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ScreenVideo,
    CameraVideo,
    MicrophoneAudio,
    SystemAudio,
    NarrationAudio,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: Uuid,
    pub kind: TrackKind,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Screen,
    Camera,
    Microphone,
    SystemAudio,
    Narration,
    Music,
    Cursor,
    Zoom,
    Captions,
    Annotation,
    Background,
    Markers,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: Uuid,
    pub source_id: Uuid,
    pub start: TimeMicros,
    pub duration: TimeMicros,
    pub source_offset: TimeMicros,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditRegion {
    pub id: Uuid,
    pub kind: String,
    pub start: TimeMicros,
    pub duration: TimeMicros,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedArtifact {
    pub id: Uuid,
    pub kind: String,
    pub relative_path: String,
    pub regenerable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub id: Uuid,
    pub kind: String,
    pub relative_path: String,
}

impl ProjectManifest {
    pub fn empty(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            frame_rate: FrameRate {
                numerator: 30,
                denominator: 1,
            },
            duration: TimeMicros(0),
            sources: vec![],
            tracks: vec![],
            edit_regions: vec![],
            artifacts: vec![],
            cache_entries: vec![],
            recording_sessions: vec![],
            editor: EditorState::default(),
        }
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchema(self.schema_version));
        }
        if self.id.is_nil() {
            return Err(ProjectError::Validation("project ID cannot be nil".into()));
        }
        if self.title.trim().is_empty() {
            return Err(ProjectError::Validation(
                "project title cannot be empty".into(),
            ));
        }
        self.frame_rate.validate()?;
        self.duration.validate()?;
        for item in &self.sources {
            if item.id.is_nil() {
                return Err(ProjectError::Validation("source ID cannot be nil".into()));
            }
            validate_relative_path(&item.relative_path)?;
        }
        let mut seen_sources = std::collections::HashSet::new();
        for item in &self.sources {
            if !seen_sources.insert(item.id) {
                return Err(ProjectError::Validation("duplicate source ID".into()));
            }
        }
        for item in &self.artifacts {
            if item.id.is_nil() {
                return Err(ProjectError::Validation("artifact ID cannot be nil".into()));
            }
            if item.kind.trim().is_empty() {
                return Err(ProjectError::Validation(
                    "artifact kind cannot be empty".into(),
                ));
            }
            validate_relative_path(&item.relative_path)?;
        }
        for item in &self.cache_entries {
            if item.id.is_nil() {
                return Err(ProjectError::Validation(
                    "cache entry ID cannot be nil".into(),
                ));
            }
            if item.kind.trim().is_empty() {
                return Err(ProjectError::Validation(
                    "cache entry kind cannot be empty".into(),
                ));
            }
            validate_relative_path(&item.relative_path)?;
        }
        let mut seen_tracks = std::collections::HashSet::new();
        for track in &self.tracks {
            if track.id.is_nil() {
                return Err(ProjectError::Validation("track ID cannot be nil".into()));
            }
            if !seen_tracks.insert(track.id) {
                return Err(ProjectError::Validation("duplicate track ID".into()));
            }
            let mut seen_clips = std::collections::HashSet::new();
            for clip in &track.clips {
                if clip.id.is_nil() {
                    return Err(ProjectError::Validation("clip ID cannot be nil".into()));
                }
                if !seen_clips.insert(clip.id) {
                    return Err(ProjectError::Validation("duplicate clip ID".into()));
                }
                if clip.source_id.is_nil() {
                    return Err(ProjectError::Validation(
                        "clip source ID cannot be nil".into(),
                    ));
                }
                clip.start.validate()?;
                clip.duration.validate()?;
                clip.source_offset.validate()?;
            }
        }
        let mut seen_regions = std::collections::HashSet::new();
        for region in &self.edit_regions {
            if region.id.is_nil() {
                return Err(ProjectError::Validation(
                    "edit region ID cannot be nil".into(),
                ));
            }
            if !seen_regions.insert(region.id) {
                return Err(ProjectError::Validation("duplicate edit region ID".into()));
            }
            if region.kind.trim().is_empty() {
                return Err(ProjectError::Validation(
                    "edit region kind cannot be empty".into(),
                ));
            }
            region.start.validate()?;
            region.duration.validate()?;
        }
        let mut seen_sessions = std::collections::HashSet::new();
        for session in &self.recording_sessions {
            if session.id.is_nil() || session.qpc_frequency <= 0 {
                return Err(ProjectError::Validation(
                    "recording clock metadata is invalid".into(),
                ));
            }
            if !seen_sessions.insert(session.id) {
                return Err(ProjectError::Validation(
                    "duplicate recording session ID".into(),
                ));
            }
            session.paused_duration.validate()?;
        }
        Ok(())
    }
}

pub fn validate_relative_path(value: &str) -> Result<PathBuf, ProjectError> {
    fn rejected(value: &str) -> ProjectError {
        ProjectError::Validation(format!("path must stay inside project: {value}"))
    }
    if value.is_empty() || value.contains('\0') {
        return Err(rejected(value));
    }
    // Any drive/ADS separator outside a Windows prefix is a jail or ADS risk.
    // Prefixes themselves are rejected below; remaining colons (e.g.
    // `media/evil:stream`) must never pass.
    let normalized = value.replace('\\', "/");
    if normalized.contains(':') {
        return Err(rejected(value));
    }
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(rejected(value));
    }
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(rejected(value));
            }
            // `.` segments are never needed for collected assets and only
            // obscure the real location (`./x`, `media/./x`). Reject to keep
            // the jail lexical and auditable.
            Component::CurDir => return Err(rejected(value)),
            Component::Normal(part) => {
                saw_normal = true;
                let part = part.to_str().ok_or_else(|| rejected(value))?;
                if part.is_empty() {
                    return Err(rejected(value));
                }
                // Windows rejects these in file names; allowing them would
                // make validation pass while the later fs call fails (or, for
                // trailing dots/spaces, silently aliases another file).
                if part
                    .chars()
                    .any(|c| matches!(c, '*' | '?' | '<' | '>' | '|' | '"' | '\0'))
                    || part.ends_with('.')
                    || part.ends_with(' ')
                {
                    return Err(rejected(value));
                }
                // Windows device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
                // resolve to devices, not project files.
                let stem = part.split('.').next().unwrap_or(part);
                let upper = stem.to_ascii_uppercase();
                let reserved = upper == "CON"
                    || upper == "PRN"
                    || upper == "AUX"
                    || upper == "NUL"
                    || (upper.len() == 4
                        && (upper.starts_with("COM") || upper.starts_with("LPT"))
                        && upper.as_bytes()[3].is_ascii_digit());
                if reserved {
                    return Err(rejected(value));
                }
            }
        }
    }
    if !saw_normal {
        return Err(rejected(value));
    }
    Ok(path.to_path_buf())
}

pub fn create_project(root: &Path, title: &str) -> Result<ProjectManifest, ProjectError> {
    let extension_ok = root
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("kiri"));
    if !extension_ok {
        return Err(ProjectError::Validation(
            "project directory must end in .kiri".into(),
        ));
    }
    if title.trim().is_empty() {
        return Err(ProjectError::Validation(
            "project title cannot be empty".into(),
        ));
    }
    // Never silently overwrite an existing project: creating over a manifest
    // would destroy user work with no recovery path.
    if root.join(MANIFEST_FILE).exists() {
        return Err(ProjectError::Validation(
            "project already exists at destination".into(),
        ));
    }
    fs::create_dir_all(root)?;
    for dir in PROJECT_DIRECTORIES {
        fs::create_dir_all(root.join(dir))?;
    }
    let manifest = ProjectManifest::empty(title);
    save_project(root, &manifest)?;
    Ok(manifest)
}

pub fn open_project(root: &Path) -> Result<ProjectManifest, ProjectError> {
    let bytes = fs::read(root.join(MANIFEST_FILE))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let migrated = migrate(value)?;
    let mut manifest: ProjectManifest = serde_json::from_value(migrated)?;
    // Repair loaded editor state (inverted ranges, NaN/inf clamps) in memory.
    // Validation still runs afterwards so genuinely corrupt manifests error
    // instead of silently persisting bad data.
    manifest.editor = manifest.editor.normalized();
    manifest.validate()?;
    Ok(manifest)
}

pub fn save_project(root: &Path, manifest: &ProjectManifest) -> Result<(), ProjectError> {
    manifest.validate()?;
    fs::create_dir_all(root)?;
    let target = root.join(MANIFEST_FILE);
    let backup = root.join("recovery/project.json.bak");
    fs::create_dir_all(root.join("recovery"))?;
    let mut temporary = NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut temporary, manifest)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    if target.exists() {
        fs::copy(&target, &backup)?;
        // Best-effort durability for the backup itself; a crash right after
        // the copy must still leave a readable backup.
        if let Ok(backup_file) = fs::File::open(&backup) {
            let _ = backup_file.sync_all();
        }
    }
    temporary
        .persist(&target)
        .map_err(|e| ProjectError::Io(e.error))?;
    if let Ok(directory) = fs::File::open(root) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Persists a project mutation immediately using the same atomic-save path as an explicit save.
/// Later editor phases may debounce calls to this boundary without changing its durability rules.
pub fn autosave_project(root: &Path, manifest: &mut ProjectManifest) -> Result<(), ProjectError> {
    let previous = manifest.updated_at;
    manifest.updated_at = Utc::now();
    if let Err(error) = save_project(root, manifest) {
        // Don't leave the in-memory timestamp ahead of durable state: callers
        // use `updated_at` for save-status UI and recovery ordering.
        manifest.updated_at = previous;
        return Err(error);
    }
    Ok(())
}

pub fn save_as(
    source: &Path,
    destination: &Path,
    title: &str,
) -> Result<ProjectManifest, ProjectError> {
    if destination.exists() {
        return Err(ProjectError::Validation(
            "Save As destination already exists".into(),
        ));
    }
    let destination_ok = destination
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("kiri"));
    if !destination_ok {
        return Err(ProjectError::Validation(
            "Save As destination must end in .kiri".into(),
        ));
    }
    if title.trim().is_empty() {
        return Err(ProjectError::Validation(
            "project title cannot be empty".into(),
        ));
    }
    // Copying a project into its own subtree would recurse forever
    // (`copy_dir` reads `source` while writing `destination` inside it).
    if destination.starts_with(source) {
        return Err(ProjectError::Validation(
            "Save As destination cannot be inside the source project".into(),
        ));
    }
    if let Err(error) = copy_dir(source, destination) {
        let _ = fs::remove_dir_all(destination);
        return Err(error);
    }
    let mut manifest = match open_project(destination) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(destination);
            return Err(error);
        }
    };
    manifest.id = Uuid::new_v4();
    manifest.title = title.into();
    manifest.updated_at = Utc::now();
    if let Err(error) = save_project(destination, &manifest) {
        let _ = fs::remove_dir_all(destination);
        return Err(error);
    }
    Ok(manifest)
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), ProjectError> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let to = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

pub fn migrate(mut value: serde_json::Value) -> Result<serde_json::Value, ProjectError> {
    // Accept string-encoded versions from hand-edited or foreign payloads
    // (`"2"`); anything unparseable falls back to 0 (oldest) so we attempt a
    // forward migration instead of misreporting a future schema.
    let version = value
        .get("schemaVersion")
        .or_else(|| value.get("schema_version"))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
        .unwrap_or(0) as u32;
    match version {
        0 | 1 => {
            let object = value.as_object_mut().ok_or_else(|| {
                ProjectError::Validation("manifest root must be an object".into())
            })?;
            normalize_manifest_keys(object);
            coerce_manifest_numbers(object);
            object.remove("schema_version");
            object.insert("schemaVersion".into(), CURRENT_SCHEMA_VERSION.into());
            object
                .entry("sources")
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("tracks")
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("editRegions")
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("artifacts")
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("cacheEntries")
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("recordingSessions")
                .or_insert_with(|| serde_json::json!([]));
            Ok(value)
        }
        CURRENT_SCHEMA_VERSION => {
            // Current-version payloads still get snake_case tolerance so a
            // hand-edited or older-writer manifest with `frame_rate` etc.
            // opens instead of failing deserialization. Never fails here:
            // unknown shapes surface as deserialization errors in
            // `open_project`, leaving the original file untouched.
            if let Some(object) = value.as_object_mut() {
                normalize_manifest_keys(object);
                coerce_manifest_numbers(object);
            }
            Ok(value)
        }
        other => Err(ProjectError::UnsupportedSchema(other)),
    }
}

/// Move a snake_case key to its camelCase twin when the twin is absent.
/// Total: never panics, never overwrites an explicit camelCase value.
fn move_key(object: &mut serde_json::Map<String, serde_json::Value>, from: &str, to: &str) {
    if object.contains_key(to) {
        object.remove(from);
        return;
    }
    if let Some(v) = object.remove(from) {
        object.insert(to.into(), v);
    }
}

fn normalize_manifest_keys(object: &mut serde_json::Map<String, serde_json::Value>) {
    move_key(object, "schema_version", "schemaVersion");
    move_key(object, "created_at", "createdAt");
    move_key(object, "updated_at", "updatedAt");
    move_key(object, "frame_rate", "frameRate");
    move_key(object, "edit_regions", "editRegions");
    move_key(object, "cache_entries", "cacheEntries");
    move_key(object, "recording_sessions", "recordingSessions");

    if let Some(sources) = object.get_mut("sources").and_then(|v| v.as_array_mut()) {
        for source in sources.iter_mut() {
            if let Some(obj) = source.as_object_mut() {
                move_key(obj, "relative_path", "relativePath");
            }
        }
    }
    if let Some(tracks) = object.get_mut("tracks").and_then(|v| v.as_array_mut()) {
        for track in tracks.iter_mut() {
            if let Some(track_obj) = track.as_object_mut()
                && let Some(clips) = track_obj.get_mut("clips").and_then(|v| v.as_array_mut())
            {
                for clip in clips.iter_mut() {
                    if let Some(clip_obj) = clip.as_object_mut() {
                        move_key(clip_obj, "source_id", "sourceId");
                        move_key(clip_obj, "source_offset", "sourceOffset");
                    }
                }
            }
        }
    }
    for key in ["artifacts", "cacheEntries"] {
        if let Some(entries) = object.get_mut(key).and_then(|v| v.as_array_mut()) {
            for entry in entries.iter_mut() {
                if let Some(obj) = entry.as_object_mut() {
                    move_key(obj, "relative_path", "relativePath");
                }
            }
        }
    }
    if let Some(sessions) = object
        .get_mut("recordingSessions")
        .and_then(|v| v.as_array_mut())
    {
        for session in sessions.iter_mut() {
            if let Some(obj) = session.as_object_mut() {
                move_key(obj, "wall_time_utc", "wallTimeUtc");
                move_key(obj, "qpc_origin_ticks", "qpcOriginTicks");
                move_key(obj, "qpc_frequency", "qpcFrequency");
                move_key(obj, "paused_duration", "pausedDuration");
            }
        }
    }
    // Nested editor aliases (Recordly `zoomRegions` etc. vs `zooms`).
    // We map what has a 1:1 twin and leave the rest for serde defaults so
    // foreign payloads degrade to empty regions instead of failing to open.
    if let Some(editor) = object.get_mut("editor").and_then(|v| v.as_object_mut()) {
        move_key(editor, "zoom_regions", "zooms");
        move_key(editor, "zoomRegions", "zooms");
        move_key(editor, "clip_regions", "clips");
        move_key(editor, "clipRegions", "clips");
        move_key(editor, "trim_regions", "trims");
        move_key(editor, "trimRegions", "trims");
        move_key(editor, "speed_regions", "speeds");
        move_key(editor, "speedRegions", "speeds");
        move_key(editor, "auto_captions", "captions");
        move_key(editor, "autoCaptions", "captions");
        move_key(editor, "annotation_regions", "annotations");
        move_key(editor, "annotationRegions", "annotations");
        move_key(editor, "audio_regions", "audioRegions");
    }
}

/// Coerce float-encoded integer micros to rounded i64 so JS-origin payloads
/// with `123.0`/`123.4` open instead of failing deserialization. Total:
/// non-finite floats become 0 (validated later); out-of-range floats
/// saturate via `as` casts without panicking.
fn coerce_number_to_i64(value: &mut serde_json::Value) {
    if let Some(f) = value.as_f64()
        && value.is_number()
        && !value.is_i64()
        && !value.is_u64()
    {
        let coerced = if f.is_finite() { f.round() as i64 } else { 0 };
        *value = serde_json::Value::from(coerced);
    }
}

fn coerce_manifest_numbers(object: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(duration) = object.get_mut("duration") {
        if duration.is_number() {
            coerce_number_to_i64(duration);
        } else if duration.is_object() {
            // Tolerate `{ "micros": 123 }` wrappers from foreign writers.
            let micros = duration
                .get("micros")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    duration
                        .get("micros")
                        .and_then(|v| v.as_f64())
                        .filter(|f| f.is_finite())
                        .map(|f| f.round() as i64)
                })
                .unwrap_or(0);
            *duration = serde_json::Value::from(micros);
        }
    }
    if let Some(tracks) = object.get_mut("tracks").and_then(|v| v.as_array_mut()) {
        for track in tracks.iter_mut() {
            if let Some(track_obj) = track.as_object_mut()
                && let Some(clips) = track_obj.get_mut("clips").and_then(|v| v.as_array_mut())
            {
                for clip in clips.iter_mut() {
                    if let Some(clip_obj) = clip.as_object_mut() {
                        for key in ["start", "duration", "sourceOffset", "source_offset"] {
                            if let Some(field) = clip_obj.get_mut(key)
                                && field.is_number()
                            {
                                coerce_number_to_i64(field);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_round_trip() {
        let project = ProjectManifest::empty("Demo");
        let json = serde_json::to_string(&project).unwrap();
        let decoded: ProjectManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(project, decoded);
        decoded.validate().unwrap();
    }
    #[test]
    fn ids_and_times_are_validated() {
        let mut project = ProjectManifest::empty("Demo");
        project.id = Uuid::nil();
        assert!(project.validate().is_err());
        project.id = Uuid::new_v4();
        project.duration = TimeMicros(-1);
        assert!(project.validate().is_err());
        assert!(validate_relative_path("../secret").is_err());
    }
    #[test]
    fn failed_save_leaves_previous_manifest_readable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("demo.kiri");
        let project = create_project(&root, "Original").unwrap();
        let mut invalid = project;
        invalid.title = "".into();
        assert!(save_project(&root, &invalid).is_err());
        assert_eq!(open_project(&root).unwrap().title, "Original");
    }
    #[test]
    fn autosave_persists_a_project_mutation_for_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("demo.kiri");
        let mut project = create_project(&root, "Original").unwrap();
        let previous_updated_at = project.updated_at;

        project.title = "Autosaved".into();
        autosave_project(&root, &mut project).unwrap();

        let reopened = open_project(&root).unwrap();
        assert_eq!(reopened.title, "Autosaved");
        assert!(reopened.updated_at >= previous_updated_at);
    }
    #[test]
    fn migrates_schema_zero_fixture() {
        let value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/schema-v0.json")).unwrap();
        let migrated = migrate(value).unwrap();
        let project: ProjectManifest = serde_json::from_value(migrated).unwrap();
        project.validate().unwrap();
    }
}
