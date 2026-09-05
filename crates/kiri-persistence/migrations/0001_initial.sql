CREATE TABLE recent_projects (
  project_id TEXT PRIMARY KEY NOT NULL,
  title TEXT NOT NULL,
  path TEXT NOT NULL UNIQUE,
  updated_at TEXT NOT NULL
);
CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE presets (id TEXT PRIMARY KEY NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL, value_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE background_jobs (id TEXT PRIMARY KEY NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL, progress_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE provider_metadata (id TEXT PRIMARY KEY NOT NULL, kind TEXT NOT NULL, display_name TEXT NOT NULL, config_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE mcp_server_metadata (id TEXT PRIMARY KEY NOT NULL, display_name TEXT NOT NULL, transport TEXT NOT NULL, config_json TEXT NOT NULL, updated_at TEXT NOT NULL);
