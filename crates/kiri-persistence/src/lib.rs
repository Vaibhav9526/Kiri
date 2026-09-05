use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_initial",
    include_str!("../migrations/0001_initial.sql"),
)];

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub project_id: String,
    pub title: String,
    pub path: PathBuf,
    pub updated_at: String,
    pub missing: bool,
}

pub struct Database {
    connection: Connection,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self, PersistenceError> {
        let connection = Connection::open(path)?;
        let db = Self { connection };
        db.migrate()?;
        Ok(db)
    }
    pub fn in_memory() -> Result<Self, PersistenceError> {
        let connection = Connection::open_in_memory()?;
        let db = Self { connection };
        db.migrate()?;
        Ok(db)
    }
    pub fn migrate(&self) -> Result<(), PersistenceError> {
        self.connection.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE IF NOT EXISTS schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);")?;
        for (name, sql) in MIGRATIONS {
            let exists: Option<i64> = self
                .connection
                .query_row(
                    "SELECT 1 FROM schema_migrations WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                self.connection.execute_batch(sql)?;
                self.connection.execute(
                    "INSERT INTO schema_migrations(name, applied_at) VALUES (?1, ?2)",
                    params![name, Utc::now().to_rfc3339()],
                )?;
            }
        }
        Ok(())
    }
    pub fn upsert_recent(
        &self,
        project_id: &str,
        title: &str,
        path: &Path,
    ) -> Result<(), PersistenceError> {
        self.connection.execute("INSERT INTO recent_projects(project_id,title,path,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(project_id) DO UPDATE SET title=excluded.title,path=excluded.path,updated_at=excluded.updated_at", params![project_id, title, path.to_string_lossy(), Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn recent_projects(&self) -> Result<Vec<RecentProject>, PersistenceError> {
        let mut statement = self.connection.prepare(
            "SELECT project_id,title,path,updated_at FROM recent_projects ORDER BY updated_at DESC",
        )?;
        let values = statement
            .query_map([], |row| {
                let path = PathBuf::from(row.get::<_, String>(2)?);
                Ok(RecentProject {
                    project_id: row.get(0)?,
                    title: row.get(1)?,
                    missing: !path.join("project.json").is_file(),
                    path,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values)
    }
    pub fn migration_count(&self) -> Result<i64, PersistenceError> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrations_are_idempotent() {
        let db = Database::in_memory().unwrap();
        db.migrate().unwrap();
        assert_eq!(db.migration_count().unwrap(), 1);
    }
    #[test]
    fn recent_project_marks_missing_path() {
        let db = Database::in_memory().unwrap();
        db.upsert_recent("id", "Missing", Path::new("Z:/does-not-exist/demo.kiri"))
            .unwrap();
        assert!(db.recent_projects().unwrap()[0].missing);
    }
}
