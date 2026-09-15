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
            validate_relative_path(&item.relative_path)?;
        }
        for item in &self.artifacts {
            validate_relative_path(&item.relative_path)?;
        }
        for item in &self.cache_entries {
            validate_relative_path(&item.relative_path)?;
        }
        for track in &self.tracks {
            for clip in &track.clips {
                clip.start.validate()?;
                clip.duration.validate()?;
                clip.source_offset.validate()?;
            }
        }
        for region in &self.edit_regions {
            region.start.validate()?;
            region.duration.validate()?;
        }
        for session in &self.recording_sessions {
            if session.id.is_nil() || session.qpc_frequency <= 0 {
                return Err(ProjectError::Validation(
                    "recording clock metadata is invalid".into(),
                ));
            }
            session.paused_duration.validate()?;
        }
        Ok(())
    }
}

pub fn validate_relative_path(value: &str) -> Result<PathBuf, ProjectError> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(ProjectError::Validation(format!(
            "path must stay inside project: {value}"
        )));
    }
    Ok(path.to_path_buf())
}

pub fn create_project(root: &Path, title: &str) -> Result<ProjectManifest, ProjectError> {
    if root.extension().and_then(|v| v.to_str()) != Some("kiri") {
        return Err(ProjectError::Validation(
            "project directory must end in .kiri".into(),
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
    let manifest: ProjectManifest = serde_json::from_value(migrated)?;
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
    manifest.updated_at = Utc::now();
    save_project(root, manifest)
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
    copy_dir(source, destination)?;
    let mut manifest = open_project(destination)?;
    manifest.id = Uuid::new_v4();
    manifest.title = title.into();
    manifest.updated_at = Utc::now();
    save_project(destination, &manifest)?;
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
    let version = value
        .get("schemaVersion")
        .or_else(|| value.get("schema_version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    match version {
        0 | 1 => {
            let object = value.as_object_mut().ok_or_else(|| {
                ProjectError::Validation("manifest root must be an object".into())
            })?;
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
        CURRENT_SCHEMA_VERSION => Ok(value),
        other => Err(ProjectError::UnsupportedSchema(other)),
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
