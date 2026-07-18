//! Trusted local path-to-scope resolution for desktop and MCP adapters.
//!
//! Paths stop at this boundary. Canonical reads and MCP continue to accept only
//! opaque repository identities.

use super::contracts::ARCHAEOLOGY_STORAGE_SCHEMA_VERSION;
use crate::DbState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{path::Path, sync::Arc};
use tauri::State;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologyRepositoryResolution {
    pub repository_id: Option<String>,
    pub ready: bool,
    pub generation_id: Option<String>,
}

pub(crate) fn resolve_repository(
    connection: &Connection,
    repo_path: &str,
) -> Result<ArchaeologyRepositoryResolution, String> {
    let canonical = Path::new(repo_path.trim())
        .canonicalize()
        .map_err(|_| "Archaeology repository is unavailable".to_string())?;
    if !canonical.is_dir() {
        return Err("Archaeology repository is unavailable".to_string());
    }
    let canonical = canonical
        .to_str()
        .ok_or_else(|| "Archaeology repository is unavailable".to_string())?;
    let resolved = connection
        .query_row(
            "SELECT repository.repository_id,ready.generation_id
             FROM archaeology_repositories repository
             LEFT JOIN archaeology_generations ready
               ON ready.generation_id=repository.ready_generation_id
              AND ready.repository_id=repository.repository_id
              AND ready.status='ready'
              AND ready.schema_version=?2
             WHERE repository.repo_path=?1",
            params![canonical, i64::from(ARCHAEOLOGY_STORAGE_SCHEMA_VERSION)],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|_| "Archaeology repository lookup failed".to_string())?;
    Ok(match resolved {
        Some((repository_id, generation_id)) => ArchaeologyRepositoryResolution {
            repository_id: Some(repository_id),
            ready: generation_id.is_some(),
            generation_id,
        },
        None => ArchaeologyRepositoryResolution {
            repository_id: None,
            ready: false,
            generation_id: None,
        },
    })
}

#[tauri::command]
pub async fn resolve_business_rule_archaeology_repository(
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<ArchaeologyRepositoryResolution, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        resolve_repository(&connection, &repo_path)
    })
    .await
    .map_err(|error| format!("Archaeology repository lookup worker failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;
    use tempfile::tempdir;

    #[test]
    fn canonical_path_resolves_only_opaque_ready_scope() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("archaeology schema");
        let root = tempdir().expect("temporary repository");
        let child = root.path().join("child");
        std::fs::create_dir(&child).expect("child directory");
        let canonical = root.path().canonicalize().expect("canonical repository");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
                  created_at,updated_at)
                 VALUES (?1,?2,'source:one',?3,'generation:ready',?4,?4)",
                params![
                    "archaeology-repository:opaque",
                    canonical.to_string_lossy(),
                    "a".repeat(40),
                    "2026-01-01T00:00:00Z"
                ],
            )
            .expect("repository row");
        connection
            .execute(
                "INSERT INTO archaeology_generations
                 (generation_id,repository_id,schema_version,revision_sha,source_identity,
                  parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
                 VALUES ('generation:ready','archaeology-repository:opaque',2,?1,
                         'source:one','parser:one','algorithm:one','config:one','ready','{}',?2)",
                params!["a".repeat(40), "2026-01-01T00:00:00Z"],
            )
            .expect("ready generation");

        let non_canonical = child.join("..");
        let resolution = resolve_repository(&connection, &non_canonical.to_string_lossy())
            .expect("repository resolution");
        assert_eq!(
            resolution,
            ArchaeologyRepositoryResolution {
                repository_id: Some("archaeology-repository:opaque".into()),
                ready: true,
                generation_id: Some("generation:ready".into()),
            }
        );
        assert!(!serde_json::to_string(&resolution)
            .expect("serialize")
            .contains(&canonical.to_string_lossy().to_string()));
    }

    #[test]
    fn unindexed_repository_returns_an_empty_non_disclosing_status() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("archaeology schema");
        let root = tempdir().expect("temporary repository");
        assert_eq!(
            resolve_repository(&connection, &root.path().to_string_lossy())
                .expect("empty resolution"),
            ArchaeologyRepositoryResolution {
                repository_id: None,
                ready: false,
                generation_id: None,
            }
        );
    }
}
