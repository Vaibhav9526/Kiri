use kiri_persistence::{Database, RecentProject};
use kiri_project::{create_project as create_project_domain, open_project as open_project_domain};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Manager, State};
use thiserror::Error;

struct AppState {
    database: Mutex<Database>,
}
#[derive(Debug, Error)]
enum CommandError {
    #[error("{0}")]
    Project(#[from] kiri_project::ProjectError),
    #[error("{0}")]
    Persistence(#[from] kiri_persistence::PersistenceError),
    #[error("invalid project location: {0}")]
    InvalidPath(String),
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

fn safe_project_name(title: &str) -> Result<String, CommandError> {
    let value: String = title
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_log::Builder::new().build())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;
            let crash_directory = app_data.join("crashes");
            std::fs::create_dir_all(&crash_directory)?;
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
                let report = format!("timestamp={timestamp}\npanic={info}\n");
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
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            list_recent_projects
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kiri");
}
