//! Additive persistence for validated, versioned warm-verifier evidence.

use crate::{db, DbState};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Component, Path};
use tauri::State;

const MAX_RESULT_BYTES: usize = 1_048_576;
const MAX_STRING_BYTES: usize = 16_384;
const MAX_ARRAY_ITEMS: usize = 1_000;
const MAX_OBJECT_KEYS: usize = 128;
const MAX_DEPTH: usize = 12;
const MAX_LIST_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize)]
pub struct StoredWarmVerificationRun {
    id: String,
    repo_path: String,
    result: Value,
    created_at: String,
}

fn object<'a>(value: &'a Value, field: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object"))
}

fn text<'a>(object: &'a Map<String, Value>, key: &str, field: &str) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field}.{key} must be a non-empty string"))?;
    if value.is_empty() || value.len() > MAX_STRING_BYTES {
        return Err(format!("{field}.{key} must be a bounded non-empty string"));
    }
    Ok(value)
}

fn optional_text<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<Option<&'a str>, String> {
    object
        .contains_key(key)
        .then(|| text(object, key, field))
        .transpose()
}

fn bool_field(object: &Map<String, Value>, key: &str, field: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{field}.{key} must be a boolean"))
}

fn array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    field: &str,
    max: usize,
) -> Result<&'a [Value], String> {
    let values = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}.{key} must be an array"))?;
    if values.len() > max {
        return Err(format!("{field}.{key} exceeds {max} items"));
    }
    Ok(values)
}

fn bounded(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("result exceeds nesting depth {MAX_DEPTH}"));
    }
    match value {
        Value::String(value) if value.len() > MAX_STRING_BYTES => Err(format!(
            "result contains a string over {MAX_STRING_BYTES} bytes"
        )),
        Value::Array(values) if values.len() > MAX_ARRAY_ITEMS => Err(format!(
            "result contains an array over {MAX_ARRAY_ITEMS} items"
        )),
        Value::Object(values) if values.len() > MAX_OBJECT_KEYS => Err(format!(
            "result contains an object over {MAX_OBJECT_KEYS} keys"
        )),
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| bounded(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| bounded(value, depth + 1)),
        _ => Ok(()),
    }
}

fn valid_id(value: &str) -> bool {
    value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn valid_hash(value: &str, min_length: usize, max_length: usize) -> bool {
    (min_length..=max_length).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn timestamp(value: &str, field: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| format!("{field} must be an ISO-8601 timestamp"))
}

fn duration(object: &Map<String, Value>, field: &str) -> Result<(), String> {
    let value = object
        .get("duration_ms")
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{field}.duration_ms must be a number"))?;
    if !(0.0..=300_000.0).contains(&value) {
        return Err(format!("{field}.duration_ms is out of bounds"));
    }
    Ok(())
}

fn validate_result(result: &Value) -> Result<String, String> {
    bounded(result, 0)?;
    let serialized = serde_json::to_string(result).map_err(|error| error.to_string())?;
    if serialized.len() > MAX_RESULT_BYTES {
        return Err(format!("result exceeds {MAX_RESULT_BYTES} bytes"));
    }

    let root = object(result, "result")?;
    if root.get("schema_version").and_then(Value::as_u64) != Some(1)
        || root.get("protocol_version").and_then(Value::as_u64) != Some(1)
    {
        return Err("unsupported warm result schema or protocol version".into());
    }
    let run_id = text(root, "run_id", "result")?;
    if !valid_id(run_id) {
        return Err("result.run_id has an invalid identifier".into());
    }
    let outcome = text(root, "outcome", "result")?;
    if !["passed", "regression", "no_confidence"].contains(&outcome) {
        return Err("result.outcome is invalid".into());
    }
    let started_at = timestamp(text(root, "started_at", "result")?, "result.started_at")?;
    let finished_at = timestamp(text(root, "finished_at", "result")?, "result.finished_at")?;
    if finished_at < started_at {
        return Err("result.finished_at precedes result.started_at".into());
    }
    bool_field(root, "warm", "result")?;
    let stale = bool_field(root, "stale", "result")?;
    if root.get("model_call_count").and_then(Value::as_u64) != Some(0) {
        return Err("result.model_call_count must be zero".into());
    }

    let source = object(root.get("source").unwrap_or(&Value::Null), "result.source")?;
    if !valid_hash(text(source, "target_sha", "result.source")?, 40, 64)
        || !valid_hash(
            text(source, "change_set_identity", "result.source")?,
            64,
            64,
        )
    {
        return Err("result.source contains an invalid target or change-set hash".into());
    }
    if !["worktree", "staged", "commit", "range"].contains(&text(
        source,
        "change_set_kind",
        "result.source",
    )?) {
        return Err("result.source.change_set_kind is invalid".into());
    }
    optional_text(source, "change_set_revision", "result.source")?;
    for key in [
        "config_hash",
        "manifest_hash",
        "source_hash_before",
        "source_hash_after",
    ] {
        if !valid_hash(text(source, key, "result.source")?, 64, 64) {
            return Err(format!("result.source.{key} has an invalid hash"));
        }
    }

    let policy = object(
        root.get("observation_policy").unwrap_or(&Value::Null),
        "result.observation_policy",
    )?;
    if policy.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !valid_id(text(policy, "profile_id", "result.observation_policy")?)
    {
        return Err("result.observation_policy is invalid".into());
    }

    let selection = object(
        root.get("selection").unwrap_or(&Value::Null),
        "result.selection",
    )?;
    for (key, max) in [
        ("changed_paths", 2_000),
        ("selected_scenario_ids", 500),
        ("mandatory_smoke_ids", 500),
        ("fallback_scenario_ids", 500),
    ] {
        for value in array(selection, key, "result.selection", max)? {
            if value.as_str().is_none_or(str::is_empty) {
                return Err(format!("result.selection.{key} contains an invalid string"));
            }
        }
    }
    let selection_complete = bool_field(selection, "complete", "result.selection")?;
    text(selection, "explanation", "result.selection")?;

    let scenarios = array(root, "scenarios", "result", 500)?;
    for (index, scenario) in scenarios.iter().enumerate() {
        let scenario = object(scenario, &format!("result.scenarios[{index}]"))?;
        if !valid_id(text(scenario, "scenario_id", "scenario")?)
            || !["passed", "regression", "no_confidence"]
                .contains(&text(scenario, "outcome", "scenario")?)
        {
            return Err("result.scenarios contains invalid metadata".into());
        }
        duration(scenario, "scenario")?;
    }

    let timings = array(root, "timings", "result", 2_000)?;
    for timing in timings {
        let timing = object(timing, "result.timings item")?;
        if ![
            "diff",
            "selection",
            "context",
            "auth",
            "state",
            "navigation",
            "actions",
            "observation",
            "screenshots",
            "reporting",
            "teardown",
            "total",
        ]
        .contains(&text(timing, "stage", "timing")?)
        {
            return Err("result.timings contains an invalid stage".into());
        }
        duration(timing, "timing")?;
        if optional_text(timing, "scenario_id", "timing")?.is_some_and(|id| !valid_id(id)) {
            return Err("result.timings.scenario_id is invalid".into());
        }
    }

    let observations = array(root, "observations", "result", 2_000)?;
    for observation in observations {
        let observation = object(observation, "result.observations item")?;
        for key in ["id", "scenario_id", "policy_id"] {
            if !valid_id(text(observation, key, "observation")?) {
                return Err(format!("result.observations.{key} is invalid"));
            }
        }
        if ![
            "page_error",
            "console_error",
            "request_failed",
            "http_failure",
            "unexpected_request",
            "mutation",
            "duplicate_mutation",
            "route",
            "interaction_timing",
            "accessibility_smoke",
            "accessibility_audit",
            "screenshot",
        ]
        .contains(&text(observation, "kind", "observation")?)
            || !["passed", "regression", "no_confidence", "informational"].contains(&text(
                observation,
                "disposition",
                "observation",
            )?)
        {
            return Err("result.observations contains invalid classification".into());
        }
        text(observation, "message", "observation")?;
        optional_text(observation, "checkpoint", "observation")?;
        if let Some(evidence) = observation.get("evidence") {
            let evidence = object(evidence, "observation.evidence")?;
            if evidence.values().any(|value| {
                !matches!(
                    value,
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
                )
            }) {
                return Err("result.observations.evidence must contain scalar metadata".into());
            }
        }
        timestamp(
            text(observation, "occurred_at", "observation")?,
            "observation.occurred_at",
        )?;
    }

    let limitations = array(root, "limitations", "result", 100)?;
    for limitation in limitations {
        let limitation = object(limitation, "result.limitations item")?;
        if ![
            "cancelled",
            "config_invalid",
            "daemon_unavailable",
            "manifest_invalid",
            "selection_incomplete",
            "source_stale",
            "state_unavailable",
            "target_unavailable",
            "browser_unavailable",
            "timeout",
            "unsupported_version",
            "artifact_limit",
            "other",
        ]
        .contains(&text(limitation, "code", "limitation")?)
        {
            return Err("result.limitations contains an invalid code".into());
        }
        text(limitation, "message", "limitation")?;
        bool_field(limitation, "affects_confidence", "limitation")?;
        optional_text(limitation, "remediation", "limitation")?;
        if optional_text(limitation, "scenario_id", "limitation")?.is_some_and(|id| !valid_id(id)) {
            return Err("result.limitations.scenario_id is invalid".into());
        }
    }

    let artifacts = array(root, "artifacts", "result", 100)?;
    for artifact in artifacts {
        let artifact = object(artifact, "result.artifacts item")?;
        if !valid_id(text(artifact, "id", "artifact")?)
            || !["screenshot", "trace", "network", "console", "report"]
                .contains(&text(artifact, "kind", "artifact")?)
            || !valid_hash(text(artifact, "sha256", "artifact")?, 64, 64)
            || !bool_field(artifact, "redacted", "artifact")?
            || artifact.get("bytes").and_then(Value::as_u64).is_none()
        {
            return Err("result.artifacts contains invalid metadata".into());
        }
        let relative_path = text(artifact, "relative_path", "artifact")?;
        if Path::new(relative_path).is_absolute()
            || Path::new(relative_path)
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("artifact.relative_path must be non-traversing and relative".into());
        }
        let created_at = timestamp(
            text(artifact, "created_at", "artifact")?,
            "artifact.created_at",
        )?;
        let retained_until = timestamp(
            text(artifact, "retained_until", "artifact")?,
            "artifact.retained_until",
        )?;
        if retained_until < created_at {
            return Err("artifact.retained_until precedes artifact.created_at".into());
        }
        if optional_text(artifact, "scenario_id", "artifact")?.is_some_and(|id| !valid_id(id)) {
            return Err("result.artifacts.scenario_id is invalid".into());
        }
    }

    let cancellation = object(
        root.get("cancellation").unwrap_or(&Value::Null),
        "result.cancellation",
    )?;
    let cancellation_state = text(cancellation, "state", "result.cancellation")?;
    if !["not_requested", "requested", "completed"].contains(&cancellation_state) {
        return Err("result.cancellation.state is invalid".into());
    }
    let requested_at = if cancellation_state != "not_requested" {
        let requested_at = timestamp(
            text(cancellation, "requested_at", "result.cancellation")?,
            "result.cancellation.requested_at",
        )?;
        optional_text(cancellation, "reason", "result.cancellation")?;
        Some(requested_at)
    } else {
        None
    };
    if cancellation_state == "completed" {
        let completed_at = timestamp(
            text(cancellation, "completed_at", "result.cancellation")?,
            "result.cancellation.completed_at",
        )?;
        if requested_at.is_some_and(|requested_at| completed_at < requested_at) {
            return Err("result.cancellation.completed_at precedes requested_at".into());
        }
    }

    let source_changed = text(source, "source_hash_before", "result.source")?
        != text(source, "source_hash_after", "result.source")?;
    if (stale || source_changed || cancellation_state != "not_requested" || !selection_complete)
        && outcome != "no_confidence"
    {
        return Err(
            "stale, changed-source, cancelled, or incomplete results must be no_confidence".into(),
        );
    }
    if outcome == "passed"
        && (scenarios.iter().any(|value| value["outcome"] != "passed")
            || observations.iter().any(|value| {
                matches!(
                    value["disposition"].as_str(),
                    Some("regression" | "no_confidence")
                )
            })
            || limitations
                .iter()
                .any(|value| value["affects_confidence"] == true))
    {
        return Err("a passing result contains failing evidence".into());
    }

    Ok(serialized)
}

fn validate_repo_path(repo_path: &str) -> Result<String, String> {
    let repo_path = repo_path.trim();
    if repo_path.is_empty() || repo_path.len() > 4_096 || !Path::new(repo_path).is_absolute() {
        return Err("repo_path must be a bounded absolute path".into());
    }
    let canonical = Path::new(repo_path)
        .canonicalize()
        .map_err(|_| "repo_path is not accessible".to_string())?;
    if !canonical.is_dir() {
        return Err("repo_path must be a directory".into());
    }
    canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "repo_path must be valid UTF-8".to_string())
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredWarmVerificationRun> {
    let result_json: String = row.get(2)?;
    let result = serde_json::from_str(&result_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            result_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(StoredWarmVerificationRun {
        id: row.get(0)?,
        repo_path: row.get(1)?,
        result,
        created_at: row.get(3)?,
    })
}

fn insert_run(
    conn: &Connection,
    repo_path: &str,
    result: &Value,
    result_json: &str,
) -> rusqlite::Result<StoredWarmVerificationRun> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let source = &result["source"];
    conn.execute(
        "INSERT INTO warm_verification_runs (
            id, repo_path, run_id, schema_version, protocol_version,
            outcome, target_sha, change_set_kind, change_set_id, started_at,
            finished_at, warm, stale, result_json, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            id,
            repo_path,
            result["run_id"].as_str(),
            result["schema_version"].as_u64(),
            result["protocol_version"].as_u64(),
            result["outcome"].as_str(),
            source["target_sha"].as_str(),
            source["change_set_kind"].as_str(),
            source["change_set_identity"].as_str(),
            result["started_at"].as_str(),
            result["finished_at"].as_str(),
            result["warm"].as_bool(),
            result["stale"].as_bool(),
            result_json,
            created_at,
        ],
    )?;
    Ok(StoredWarmVerificationRun {
        id,
        repo_path: repo_path.to_owned(),
        result: result.clone(),
        created_at,
    })
}

pub(crate) fn persist_validated_run(
    conn: &Connection,
    repo_path: &str,
    result: &Value,
) -> Result<StoredWarmVerificationRun, String> {
    let repo_path = validate_repo_path(repo_path)?;
    let result_json = validate_result(result)?;
    db::with_busy_retry(|| insert_run(conn, &repo_path, result, &result_json), 5)
        .map_err(|error| error.to_string())
}

fn list_runs(
    conn: &Connection,
    repo_path: &str,
    limit: i64,
) -> rusqlite::Result<Vec<StoredWarmVerificationRun>> {
    let mut stmt = conn.prepare(
        "SELECT id, repo_path, result_json, created_at FROM warm_verification_runs
         WHERE repo_path = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![repo_path, limit], map_row)?
        .collect();
    rows
}

#[tauri::command]
pub async fn list_warm_verification_runs(
    db: State<'_, DbState>,
    repo_path: String,
    limit: Option<i64>,
) -> Result<Vec<StoredWarmVerificationRun>, String> {
    let repo_path = validate_repo_path(&repo_path)?;
    let limit = limit.unwrap_or(20);
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(format!("limit must be between 1 and {MAX_LIST_LIMIT}"));
    }
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    db::with_busy_retry(|| list_runs(&conn, &repo_path, limit), 5)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(run_id: &str) -> Value {
        json!({
            "schema_version": 1, "protocol_version": 1, "run_id": run_id,
            "outcome": "passed", "started_at": "2026-07-15T00:00:00Z",
            "finished_at": "2026-07-15T00:00:01Z", "warm": true, "stale": false,
            "model_call_count": 0,
            "source": {
                "target_sha": "a".repeat(40), "change_set_kind": "worktree",
                "change_set_identity": "b".repeat(64), "config_hash": "c".repeat(64),
                "manifest_hash": "d".repeat(64), "source_hash_before": "e".repeat(64),
                "source_hash_after": "e".repeat(64)
            },
            "observation_policy": { "schema_version": 1, "profile_id": "strict" },
            "selection": {
                "changed_paths": ["src/App.tsx"], "selected_scenario_ids": ["app-smoke"],
                "mandatory_smoke_ids": ["app-smoke"], "fallback_scenario_ids": [],
                "complete": true, "explanation": "App change selects smoke"
            },
            "scenarios": [{ "scenario_id": "app-smoke", "outcome": "passed", "duration_ms": 700 }],
            "timings": [{ "stage": "total", "duration_ms": 1000 }],
            "observations": [{
                "id": "route-1", "scenario_id": "app-smoke", "kind": "route",
                "disposition": "passed", "policy_id": "route-policy",
                "message": "Expected route retained", "occurred_at": "2026-07-15T00:00:00Z",
                "evidence": { "pathname": "/", "matched": true }
            }],
            "limitations": [{
                "code": "other", "message": "Local Chromium only", "affects_confidence": false
            }],
            "artifacts": [{
                "id": "report-1", "kind": "report", "relative_path": "runs/report.json",
                "sha256": "f".repeat(64), "bytes": 128, "redacted": true,
                "created_at": "2026-07-15T00:00:01Z", "retained_until": "2026-07-16T00:00:01Z"
            }],
            "cancellation": { "state": "not_requested" }
        })
    }

    #[test]
    fn additive_migration_is_idempotent_and_legacy_qa_remains_operational() {
        let conn = Connection::open_in_memory().expect("db");
        db::schema::run_migrations(&conn).expect("schema");
        conn.execute("INSERT INTO synthetic_qa_runs (id, loop_id, runner_type, pass, notes, created_at) VALUES ('legacy','old','playwright_builtin',1,'unchanged','2026-01-01')", []).expect("legacy");
        let result = result("warm-run-1");
        let json = validate_result(&result).expect("valid");
        insert_run(&conn, "/repo", &result, &json).expect("insert");

        db::schema::run_migrations(&conn).expect("idempotent schema rerun");
        db::schema::run_migrations(&conn).expect("second idempotent schema rerun");

        let rows = list_runs(&conn, "/repo", 10).expect("list");
        assert_eq!(
            rows[0].result["selection"]["selected_scenario_ids"][0],
            "app-smoke"
        );
        assert_eq!(rows[0].result["timings"].as_array().unwrap().len(), 1);
        assert_eq!(rows[0].result["observations"].as_array().unwrap().len(), 1);
        assert_eq!(rows[0].result["limitations"].as_array().unwrap().len(), 1);
        assert_eq!(rows[0].result["artifacts"].as_array().unwrap().len(), 1);
        let legacy: (String, i64, String) = conn
            .query_row(
                "SELECT loop_id, pass, notes FROM synthetic_qa_runs WHERE id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("legacy remains");
        assert_eq!(legacy, ("old".into(), 1, "unchanged".into()));

        let later_legacy = db::queries::insert_synthetic_qa_run(
            &conn,
            &db::queries::SyntheticQaRunInput {
                review_id: None,
                repo_path: Some("/repo".into()),
                loop_id: "legacy-after-warm".into(),
                runner_type: "playwright_builtin".into(),
                base_url: None,
                route: Some("/".into()),
                goal: Some("Legacy QA remains available".into()),
                pass: true,
                duration_ms: 20,
                notes: Some("legacy path still works".into()),
                screenshot_path: None,
                artifacts: Vec::new(),
                console_errors: 0,
                error: None,
                trace_json: None,
            },
        )
        .expect("legacy insert after warm data");
        let legacy_runs = db::queries::list_synthetic_qa_runs_for_repo(&conn, "/repo", 10)
            .expect("legacy list after warm data");
        assert_eq!(legacy_runs.len(), 1);
        assert_eq!(legacy_runs[0].id, later_legacy.id);
        assert_eq!(list_runs(&conn, "/repo", 10).unwrap().len(), 1);
    }

    #[test]
    fn canonicalizes_repository_filters_and_rejects_files() {
        let temp = tempfile::tempdir().expect("temp repo");
        std::fs::create_dir_all(temp.path().join("nested")).expect("nested");
        let alias = temp.path().join("nested").join("..");
        assert_eq!(
            validate_repo_path(alias.to_str().expect("path")).expect("canonical"),
            temp.path()
                .canonicalize()
                .expect("canonical temp")
                .to_string_lossy()
        );
        let file = temp.path().join("file");
        std::fs::write(&file, "not a repo").expect("file");
        assert!(validate_repo_path(file.to_str().expect("file path")).is_err());
    }

    #[test]
    fn invalid_or_duplicate_results_cannot_replace_evidence() {
        let conn = Connection::open_in_memory().expect("db");
        db::schema::run_migrations(&conn).expect("schema");
        let first_result = result("warm-run-1");
        let json = validate_result(&first_result).expect("valid");
        insert_run(&conn, "/repo", &first_result, &json).expect("insert");
        assert!(insert_run(&conn, "/repo", &first_result, &json).is_err());
        let mut stale_pass = result("warm-run-2");
        stale_pass["stale"] = json!(true);
        assert!(validate_result(&stale_pass).is_err());
        assert_eq!(list_runs(&conn, "/repo", 10).unwrap().len(), 1);
    }

    #[test]
    fn rejects_invalid_nested_contracts_and_pass_invariants() {
        let cases = [
            ("outcome", "/outcome", json!("unknown")),
            ("model calls", "/model_call_count", json!(1)),
            ("negative duration", "/timings/0/duration_ms", json!(-1)),
            (
                "scenario regression",
                "/scenarios/0/outcome",
                json!("regression"),
            ),
            (
                "observation regression",
                "/observations/0/disposition",
                json!("no_confidence"),
            ),
            (
                "nested observation evidence",
                "/observations/0/evidence/pathname",
                json!({ "nested": true }),
            ),
            (
                "confidence limitation",
                "/limitations/0/affects_confidence",
                json!(true),
            ),
            (
                "artifact traversal",
                "/artifacts/0/relative_path",
                json!("../secret"),
            ),
            ("unredacted artifact", "/artifacts/0/redacted", json!(false)),
            (
                "source drift",
                "/source/source_hash_after",
                json!("9".repeat(64)),
            ),
        ];

        for (name, pointer, replacement) in cases {
            let mut candidate = result(&format!("invalid-{name}"));
            *candidate.pointer_mut(pointer).expect("fixture pointer") = replacement;
            assert!(validate_result(&candidate).is_err(), "accepted {name}");
        }

        let mut reversed = result("invalid-time-order");
        reversed["finished_at"] = json!("2026-07-14T23:59:59Z");
        assert!(validate_result(&reversed).is_err());

        let mut reversed_cancellation = result("invalid-cancellation-order");
        reversed_cancellation["outcome"] = json!("no_confidence");
        reversed_cancellation["cancellation"] = json!({
            "state": "completed",
            "requested_at": "2026-07-15T00:00:01Z",
            "completed_at": "2026-07-15T00:00:00Z"
        });
        assert!(validate_result(&reversed_cancellation).is_err());
    }
}
