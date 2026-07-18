//! Additive persistence for bounded local differential-verification summaries.
//!
//! Differential evidence is informative only: it never replaces a warm run or
//! rewrites historical warm/synthetic QA rows.

use crate::{db, DbState};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::Path;
use tauri::State;

const MAX_SUMMARY_BYTES: usize = 262_144;
const MAX_LIST_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize)]
pub struct StoredDifferentialVerificationRun {
    id: String,
    repo_path: String,
    summary: Value,
    created_at: String,
}

fn object<'a>(value: &'a Value, field: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object"))
}

fn text<'a>(value: &'a Map<String, Value>, key: &str, field: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 16_384)
        .ok_or_else(|| format!("{field}.{key} must be a bounded non-empty string"))
}

fn valid_id(value: &str) -> bool {
    value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn valid_hash(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_repo_path(repo_path: &str) -> Result<String, String> {
    let repo_path = repo_path.trim();
    if repo_path.is_empty() || repo_path.len() > 4_096 || !Path::new(repo_path).is_absolute() {
        return Err("repo_path must be a bounded absolute path".into());
    }
    let canonical = Path::new(repo_path)
        .canonicalize()
        .map_err(|_| "repo_path is not accessible".to_string())?;
    canonical
        .is_dir()
        .then_some(canonical)
        .ok_or_else(|| "repo_path must be a directory".to_string())?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "repo_path must be valid UTF-8".to_string())
}

fn nullable_hash(value: &Map<String, Value>, key: &str, minimum: usize, maximum: usize) -> bool {
    matches!(value.get(key), Some(Value::Null))
        || value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| valid_hash(value, minimum, maximum))
}

fn exact_keys(value: &Map<String, Value>, expected: &[&str]) -> bool {
    value.len() == expected.len() && value.keys().all(|key| expected.contains(&key.as_str()))
}

fn valid_text_array(value: Option<&Value>, maximum: usize, hashes_only: bool) -> bool {
    value.and_then(Value::as_array).is_some_and(|values| {
        values.len() <= maximum
            && values.iter().all(|value| {
                value.as_str().is_some_and(|value| {
                    if hashes_only {
                        valid_hash(value, 64, 64)
                    } else {
                        !value.is_empty() && value.len() <= 16_384
                    }
                })
            })
    })
}

fn valid_delta_previews(root: &Map<String, Value>, delta_count: u64) -> bool {
    let Some(previews) = root.get("delta_previews").and_then(Value::as_array) else {
        return false;
    };
    if previews.len() > 20 || previews.len() as u64 > delta_count {
        return false;
    }
    let expected_truncated = previews.len() as u64 != delta_count;
    if root
        .get("delta_previews_truncated")
        .and_then(Value::as_bool)
        != Some(expected_truncated)
    {
        return false;
    }
    previews.iter().all(|preview| {
        let Some(preview) = preview.as_object() else {
            return false;
        };
        exact_keys(
            preview,
            &[
                "id",
                "scenario_id",
                "kind",
                "direction",
                "blocking",
                "policy_id",
            ],
        ) && ["id", "scenario_id", "policy_id"].iter().all(|key| {
            preview
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(valid_id)
        }) && matches!(
            preview.get("kind").and_then(Value::as_str),
            Some(
                "visual"
                    | "visible_text"
                    | "route"
                    | "network"
                    | "runtime_error"
                    | "mutation"
                    | "accessibility"
                    | "performance"
                    | "assertion"
            )
        ) && matches!(
            preview.get("direction").and_then(Value::as_str),
            Some(
                "candidate_only"
                    | "reference_only"
                    | "worsened"
                    | "improved"
                    | "changed"
                    | "shared_failure"
            )
        ) && preview.get("blocking").and_then(Value::as_bool).is_some()
    })
}

fn validate_summary(summary: &Value) -> Result<String, String> {
    let serialized = serde_json::to_string(summary).map_err(|error| error.to_string())?;
    if serialized.len() > MAX_SUMMARY_BYTES {
        return Err(format!(
            "differential summary exceeds {MAX_SUMMARY_BYTES} bytes"
        ));
    }
    let root = object(summary, "summary")?;
    let expected_keys = [
        "schema_version",
        "run_id",
        "status",
        "classification",
        "plan_identity",
        "reference_sha",
        "candidate_kind",
        "candidate_identity",
        "scenario_count",
        "delta_count",
        "blocking_delta_count",
        "delta_previews",
        "delta_previews_truncated",
        "reason_codes",
        "comparison_policy_identities",
        "duration_ms",
        "cleanup_complete",
        "creates_pass_evidence",
        "model_call_count",
    ];
    if !exact_keys(root, &expected_keys)
        || root.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !valid_id(text(root, "run_id", "summary")?)
        || !matches!(
            root.get("status").and_then(Value::as_str),
            Some("complete" | "incomparable")
        )
        || !matches!(
            root.get("classification").and_then(Value::as_str),
            Some("regressed" | "improved" | "unchanged" | "incomparable")
        )
        || !nullable_hash(root, "reference_sha", 40, 64)
        || !matches!(
            root.get("candidate_kind").and_then(Value::as_str),
            Some("worktree" | "staged" | "commit" | "range")
        )
        || !nullable_hash(root, "candidate_identity", 64, 64)
        || !nullable_hash(root, "plan_identity", 64, 64)
        || root.get("creates_pass_evidence").and_then(Value::as_bool) != Some(false)
        || root.get("model_call_count").and_then(Value::as_u64) != Some(0)
        || root
            .get("cleanup_complete")
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err("Differential summary has an unsupported contract".into());
    }
    let scenario_count = root.get("scenario_count").and_then(Value::as_u64);
    let delta_count = root.get("delta_count").and_then(Value::as_u64);
    let blocking_delta_count = root.get("blocking_delta_count").and_then(Value::as_u64);
    if scenario_count.is_none_or(|count| count > 500)
        || delta_count.is_none_or(|count| count > 2_000)
        || blocking_delta_count.is_none_or(|count| count > delta_count.unwrap_or(0))
        || !valid_delta_previews(root, delta_count.unwrap_or(0))
        || !valid_text_array(root.get("reason_codes"), 100, false)
        || !valid_text_array(root.get("comparison_policy_identities"), 100, true)
    {
        return Err("Differential summary contains invalid bounded evidence".into());
    }
    if root
        .get("duration_ms")
        .and_then(Value::as_f64)
        .is_none_or(|value| !(0.0..=300_000.0).contains(&value))
    {
        return Err("summary.duration_ms is out of bounds".into());
    }
    Ok(serialized)
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDifferentialVerificationRun> {
    let serialized: String = row.get(2)?;
    let summary = serde_json::from_str(&serialized).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            serialized.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(StoredDifferentialVerificationRun {
        id: row.get(0)?,
        repo_path: row.get(1)?,
        summary,
        created_at: row.get(3)?,
    })
}

pub(crate) fn persist_validated_run(
    conn: &Connection,
    repo_path: &str,
    summary: &Value,
) -> Result<StoredDifferentialVerificationRun, String> {
    let repo_path = validate_repo_path(repo_path)?;
    let summary_json = validate_summary(summary)?;
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    db::with_busy_retry(
        || {
            conn.execute(
                "INSERT INTO differential_verification_runs (
                   id, repo_path, run_id, schema_version, status, classification,
                   reference_sha, candidate_kind, candidate_identity, plan_identity,
                   duration_ms, cleanup_complete, summary_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    id,
                    repo_path,
                    summary["run_id"].as_str(),
                    summary["schema_version"].as_u64(),
                    summary["status"].as_str(),
                    summary["classification"].as_str(),
                    summary["reference_sha"].as_str(),
                    summary["candidate_kind"].as_str(),
                    summary["candidate_identity"].as_str(),
                    summary["plan_identity"].as_str(),
                    summary["duration_ms"].as_f64(),
                    summary["cleanup_complete"].as_bool(),
                    summary_json,
                    created_at,
                ],
            )
        },
        5,
    )
    .map_err(|error| error.to_string())?;
    Ok(StoredDifferentialVerificationRun {
        id,
        repo_path,
        summary: summary.clone(),
        created_at,
    })
}

#[tauri::command]
pub async fn list_differential_verification_runs(
    db: State<'_, DbState>,
    repo_path: String,
    limit: Option<i64>,
) -> Result<Vec<StoredDifferentialVerificationRun>, String> {
    let repo_path = validate_repo_path(&repo_path)?;
    let limit = limit.unwrap_or(20);
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(format!("limit must be between 1 and {MAX_LIST_LIMIT}"));
    }
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    db::with_busy_retry(
        || {
            let mut statement = conn.prepare(
                "SELECT id, repo_path, summary_json, created_at
                 FROM differential_verification_runs
                 WHERE repo_path = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![repo_path, limit], map_row)?;
            rows.collect()
        },
        5,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(run_id: &str) -> Value {
        json!({
            "schema_version": 1, "run_id": run_id, "status": "complete", "classification": "unchanged",
            "plan_identity": "a".repeat(64), "reference_sha": "b".repeat(40), "candidate_kind": "worktree",
            "candidate_identity": "c".repeat(64), "scenario_count": 1, "delta_count": 0,
            "blocking_delta_count": 0, "delta_previews": [], "delta_previews_truncated": false,
            "reason_codes": [], "comparison_policy_identities": [], "duration_ms": 12.0,
            "cleanup_complete": true, "creates_pass_evidence": false, "model_call_count": 0
        })
    }

    #[test]
    fn accepts_bounded_additive_summary_only() {
        assert!(validate_summary(&summary("differential-1")).is_ok());
        let mut invalid = summary("differential-1");
        invalid["creates_pass_evidence"] = Value::Bool(true);
        assert!(validate_summary(&invalid).is_err());
        let mut unknown = summary("differential-1");
        unknown["raw_response_body"] = Value::String("secret".into());
        assert!(validate_summary(&unknown).is_err());
        let mut inconsistent = summary("differential-1");
        inconsistent["delta_count"] = Value::from(1);
        assert!(validate_summary(&inconsistent).is_err());
    }

    #[test]
    fn migration_and_persistence_leave_legacy_evidence_untouched() {
        let conn = Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&conn).expect("migrations");
        conn.execute(
            "INSERT INTO synthetic_qa_runs (
               id, loop_id, runner_type, pass, duration_ms, console_errors, created_at
             ) VALUES ('qa-1', 'loop-1', 'playwright', 1, 1, 0, '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("legacy synthetic row");
        conn.execute(
            "INSERT INTO warm_verification_runs (
               id, repo_path, run_id, schema_version, protocol_version, outcome, target_sha,
               change_set_kind, change_set_id, started_at, finished_at, warm, stale,
               result_json, created_at
             ) VALUES (
               'warm-1', '/tmp/repo', 'warm-run-1', 1, 1, 'passed', ?1,
               'worktree', 'change-1', '2026-01-01T00:00:00Z', '2026-01-01T00:00:01Z',
               1, 0, '{}', '2026-01-01T00:00:01Z'
             )",
            ["a".repeat(40)],
        )
        .expect("legacy warm row");

        let repo = tempfile::tempdir().expect("repo");
        let stored = persist_validated_run(
            &conn,
            repo.path().to_str().expect("repo path"),
            &summary("differential-migration-1"),
        )
        .expect("differential row");
        assert_eq!(stored.summary["run_id"], "differential-migration-1");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM synthetic_qa_runs", [], |row| row
                .get::<_, i64>(0))
                .expect("synthetic count"),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM warm_verification_runs", [], |row| row
                .get::<_, i64>(0))
                .expect("warm count"),
            1
        );
        crate::db::schema::run_migrations(&conn).expect("idempotent migrations");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM differential_verification_runs",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .expect("differential count"),
            1
        );
    }
}
