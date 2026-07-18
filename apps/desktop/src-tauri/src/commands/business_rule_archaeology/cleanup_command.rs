//! Strict desktop boundary for owner-safe archaeology generation cleanup.

use super::contracts::ARCHAEOLOGY_SCHEMA_VERSION;
use super::jobs::{self, ArchaeologyCleanup, ArchaeologyCleanupMode};
use super::repository_resolution::resolve_repository;
use crate::DbState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

const MAX_JOB_ID_BYTES: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyCleanupCommandInput {
    repo_path: String,
    job_id: String,
    apply: bool,
    retain_superseded: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologyCleanupCommandResult {
    schema_version: u32,
    job_id: String,
    dry_run: bool,
    candidate_generations: u64,
    search_index_rows: u64,
    synthesis_cache_rows: u64,
    synthesis_attempt_rows: u64,
    synthesis_response_bytes: u64,
    truncated: bool,
    deleted_generations: u64,
    deleted_search_index_rows: u64,
    deleted_synthesis_cache_rows: u64,
    deleted_synthesis_attempt_rows: u64,
    deleted_synthesis_response_bytes: u64,
    unavailable_resources: Vec<String>,
}

fn run_cleanup(
    connection: &rusqlite::Connection,
    input: ArchaeologyCleanupCommandInput,
) -> Result<ArchaeologyCleanupCommandResult, String> {
    let job_id = input.job_id.trim();
    if job_id.is_empty() || job_id.len() > MAX_JOB_ID_BYTES {
        return Err("Archaeology cleanup request is invalid".into());
    }
    let resolution = resolve_repository(connection, &input.repo_path)?;
    let repository_id = resolution
        .repository_id
        .ok_or_else(|| "Archaeology cleanup is unavailable".to_string())?;
    let job = jobs::load_job(connection, job_id)
        .map_err(|_| "Archaeology cleanup is unavailable".to_string())?;
    if job.repository_id.as_deref() != Some(repository_id.as_str()) {
        return Err("Archaeology cleanup is unavailable".into());
    }
    let owner_id = job
        .owner_id
        .as_deref()
        .ok_or_else(|| "Archaeology cleanup is unavailable".to_string())?;
    let report = jobs::cleanup_generations(
        connection,
        ArchaeologyCleanup {
            job_id,
            owner_id,
            mode: if input.apply {
                ArchaeologyCleanupMode::Apply
            } else {
                ArchaeologyCleanupMode::DryRun
            },
            retain_superseded: input.retain_superseded,
            now: &chrono::Utc::now().to_rfc3339(),
        },
    )
    .map_err(|_| "Archaeology cleanup is unavailable".to_string())?;

    let candidate_generations = u64::try_from(report.candidates.len())
        .map_err(|_| "Archaeology cleanup result exceeds bounds".to_string())?;
    Ok(ArchaeologyCleanupCommandResult {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        job_id: job_id.to_string(),
        dry_run: report.dry_run,
        candidate_generations,
        search_index_rows: report
            .candidates
            .iter()
            .map(|candidate| candidate.search_index_rows)
            .sum(),
        synthesis_cache_rows: report
            .candidates
            .iter()
            .map(|candidate| candidate.synthesis_cache_rows)
            .sum(),
        synthesis_attempt_rows: report
            .candidates
            .iter()
            .map(|candidate| candidate.synthesis_attempt_rows)
            .sum(),
        synthesis_response_bytes: report
            .candidates
            .iter()
            .map(|candidate| candidate.synthesis_response_bytes)
            .sum(),
        truncated: report.truncated,
        deleted_generations: report.deleted_generations,
        deleted_search_index_rows: report.deleted_search_index_rows,
        deleted_synthesis_cache_rows: report.deleted_synthesis_cache_rows,
        deleted_synthesis_attempt_rows: report.deleted_synthesis_attempt_rows,
        deleted_synthesis_response_bytes: report.deleted_synthesis_response_bytes,
        unavailable_resources: report.unavailable_resources,
    })
}

#[tauri::command]
pub async fn cleanup_business_rule_archaeology_index(
    db: State<'_, DbState>,
    input: ArchaeologyCleanupCommandInput,
) -> Result<ArchaeologyCleanupCommandResult, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        run_cleanup(&connection, input)
    })
    .await
    .map_err(|error| format!("Archaeology cleanup worker failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;
    use rusqlite::params;
    use tempfile::tempdir;

    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const NOW: &str = "2026-07-17T00:00:00Z";

    #[test]
    fn strict_cleanup_hides_owner_and_deletes_only_owned_non_ready_generation() {
        let connection = rusqlite::Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        let root = tempdir().expect("repository");
        let canonical = root.path().canonicalize().expect("canonical repository");
        insert_repository(&connection, &canonical.to_string_lossy());
        insert_job(
            &connection,
            "job:ready",
            "generation:ready",
            "owner:private",
        );
        connection
            .execute(
                "UPDATE archaeology_jobs
                 SET state='completed',stage='idle',finished_at=?2,updated_at=?2
                 WHERE job_id=?1",
                params!["job:ready", NOW],
            )
            .expect("complete ready job");
        insert_job(
            &connection,
            "job:failed",
            "generation:failed",
            "owner:private",
        );
        connection
            .execute(
                "UPDATE archaeology_jobs
                 SET state='failed',stage='idle',finished_at=?2,updated_at=?2
                 WHERE job_id=?1",
                params!["job:failed", NOW],
            )
            .expect("fail old job");

        let dry_run = run_cleanup(
            &connection,
            input(&canonical.to_string_lossy(), "job:ready", false),
        )
        .expect("dry run");
        assert!(dry_run.dry_run);
        assert_eq!(dry_run.candidate_generations, 1);
        let json = serde_json::to_string(&dry_run).expect("serialize result");
        assert!(!json.contains("owner:private"));
        assert!(!json.contains(canonical.to_string_lossy().as_ref()));

        let applied = run_cleanup(
            &connection,
            input(&canonical.to_string_lossy(), "job:ready", true),
        )
        .expect("apply cleanup");
        assert_eq!(applied.deleted_generations, 1);
        let ready_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_generations WHERE generation_id='generation:ready'",
                [],
                |row| row.get(0),
            )
            .expect("ready count");
        assert_eq!(ready_count, 1);
    }

    #[test]
    fn cleanup_rejects_unknown_fields_cross_repository_jobs_and_oversized_ids() {
        assert!(
            serde_json::from_value::<ArchaeologyCleanupCommandInput>(serde_json::json!({
                "repo_path": "/tmp/repo",
                "job_id": "job:one",
                "apply": false,
                "retain_superseded": 1,
                "owner_id": "owner:forbidden"
            }))
            .is_err()
        );

        let connection = rusqlite::Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        let first = tempdir().expect("first repository");
        let second = tempdir().expect("second repository");
        insert_repository(&connection, &first.path().to_string_lossy());
        let second_path = second.path().canonicalize().expect("second canonical");
        let error = run_cleanup(
            &connection,
            input(&second_path.to_string_lossy(), "job:ready", false),
        )
        .expect_err("cross-repository job must fail");
        assert_eq!(error, "Archaeology cleanup is unavailable");

        let oversized = "x".repeat(MAX_JOB_ID_BYTES + 1);
        let error = run_cleanup(
            &connection,
            input(&first.path().to_string_lossy(), &oversized, false),
        )
        .expect_err("oversized job id must fail");
        assert_eq!(error, "Archaeology cleanup request is invalid");
    }

    fn input(repo_path: &str, job_id: &str, apply: bool) -> ArchaeologyCleanupCommandInput {
        ArchaeologyCleanupCommandInput {
            repo_path: repo_path.to_string(),
            job_id: job_id.to_string(),
            apply,
            retain_superseded: 0,
        }
    }

    fn insert_repository(connection: &rusqlite::Connection, repo_path: &str) {
        let canonical = std::path::Path::new(repo_path)
            .canonicalize()
            .expect("canonical repository");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
                  created_at,updated_at)
                 VALUES ('repository:one',?1,'source:one',?2,'generation:ready',?3,?3)",
                params![canonical.to_string_lossy(), REVISION, NOW],
            )
            .expect("repository row");
        connection
            .execute(
                "INSERT INTO archaeology_generations
                 (generation_id,repository_id,schema_version,revision_sha,source_identity,
                  parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
                 VALUES ('generation:ready','repository:one',2,?1,'source:one','parser:one',
                         'algorithm:one','config:one','ready','{}',?2),
                        ('generation:failed','repository:one',2,?1,'source:one','parser:old',
                         'algorithm:one','config:one','failed','{}',?2)",
                params![REVISION, NOW],
            )
            .expect("generation rows");
    }

    fn insert_job(
        connection: &rusqlite::Connection,
        job_id: &str,
        generation_id: &str,
        owner_id: &str,
    ) {
        connection
            .execute(
                "INSERT INTO archaeology_jobs
                 (job_id,repository_id,generation_id,owner_id,stage,state,checkpoint_json,
                  completed_units,total_units,cancellation_requested,errors_json,started_at,updated_at)
                 VALUES (?1,'repository:one',?2,?3,'inventory','running','{}',0,1,0,'[]',?4,?4)",
                params![job_id, generation_id, owner_id, NOW],
            )
            .expect("job row");
    }
}
