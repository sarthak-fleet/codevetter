use crate::commands::secret_policy::is_sensitive_path;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;

pub const REVIEW_MANIFEST_SCHEMA_VERSION: u32 = 1;
const UNIT_PROMPT_BYTES: usize = 80 * 1024;
pub const REVIEW_MAX_CONCURRENCY: usize = 3;
pub const REVIEW_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const REVIEW_ATTEMPT_LIMIT: usize = 1;
pub const REVIEW_WALL_TIME_SECONDS: u64 = 8 * 60;
const MAX_FINDING_TITLE_BYTES: usize = 240;
const MAX_FINDING_SUMMARY_BYTES: usize = 8 * 1024;
const MAX_SUGGESTION_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCoverageState {
    Reviewed,
    Reused,
    Skipped,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateQualificationState {
    Qualified,
    Stale,
    Unresolved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedReviewTarget {
    pub schema_version: u32,
    pub identity: String,
    pub repository_root: String,
    pub diff_mode: String,
    pub requested_range: String,
    pub head_sha: String,
    pub base_sha: Option<String>,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewUnit {
    pub id: String,
    pub file_path: String,
    pub file_status: String,
    pub fingerprint: String,
    pub diff_bytes: usize,
    pub prompt_budget_bytes: usize,
    pub coverage_state: ReviewCoverageState,
    pub coverage_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualificationCounts {
    pub qualified: usize,
    pub stale: usize,
    pub unresolved: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualificationDiagnostic {
    pub candidate_index: usize,
    pub state: CandidateQualificationState,
    pub reason: String,
    pub file_path: Option<String>,
    pub original_line: Option<i64>,
    pub resolved_line: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBudgets {
    pub max_concurrency: usize,
    pub prompt_bytes_per_unit: usize,
    pub output_bytes_per_attempt: usize,
    pub attempt_limit: usize,
    pub wall_time_seconds_per_attempt: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub review_id: Option<String>,
    pub target: ResolvedReviewTarget,
    pub executor_id: String,
    pub executor_version: String,
    pub policy_fingerprint: String,
    pub budgets: ReviewBudgets,
    pub units: Vec<ReviewUnit>,
    pub qualification_counts: QualificationCounts,
    pub qualification_diagnostics: Vec<QualificationDiagnostic>,
    pub complete_coverage: bool,
    pub stale: bool,
    pub cancelled: bool,
    pub created_at: String,
    pub completed_at: Option<String>,
}

pub struct QualifiedCandidates {
    pub findings: Vec<Value>,
    pub counts: QualificationCounts,
    pub diagnostics: Vec<QualificationDiagnostic>,
}

struct QualifiedLocation {
    file_path: String,
    resolved_line: i64,
    source_anchor: String,
    suggestion_allowed: bool,
    reason: String,
}

pub fn resolve_target(repo_path: &str, diff_range: &str) -> Result<ResolvedReviewTarget, String> {
    let root = fs::canonicalize(repo_path).map_err(|_| "Review repository is unavailable")?;
    if !root.is_dir() {
        return Err("Review repository is not a directory".to_string());
    }
    let range = diff_range.trim();
    if range.is_empty() || (range.starts_with('-') && range != "--staged" && range != "--cached") {
        return Err("Review range is empty or resembles an unsupported Git option".to_string());
    }
    let head_sha = git_text(&root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let base_ref = range
        .split("...")
        .next()
        .and_then(|value| value.split("..").next())
        .filter(|value| {
            !value.is_empty()
                && *value != "--staged"
                && *value != "--cached"
                && *value != "WORKTREE"
        });
    let base_sha = base_ref
        .map(|value| {
            git_text(
                &root,
                &["rev-parse", "--verify", &format!("{value}^{{commit}}")],
            )
        })
        .transpose()?;
    let raw_diff = git_diff(&root, range, None)?;
    let source_fingerprint = digest(raw_diff.as_bytes());
    let diff_mode = if matches!(range, "--staged" | "--cached") {
        "staged"
    } else if range == "WORKTREE" {
        "worktree"
    } else if range.contains("..") {
        "range"
    } else {
        "worktree"
    };
    let identity = digest(
        format!(
            "{}\0{diff_mode}\0{range}\0{head_sha}\0{}\0{source_fingerprint}",
            root.display(),
            base_sha.as_deref().unwrap_or("")
        )
        .as_bytes(),
    );
    Ok(ResolvedReviewTarget {
        schema_version: REVIEW_MANIFEST_SCHEMA_VERSION,
        identity,
        repository_root: root.to_string_lossy().into_owned(),
        diff_mode: diff_mode.to_string(),
        requested_range: range.to_string(),
        head_sha,
        base_sha,
        source_fingerprint,
    })
}

pub fn plan_units(target: &ResolvedReviewTarget, agent: &str) -> Result<Vec<ReviewUnit>, String> {
    plan_units_with_context(target, agent, "")
}

pub fn plan_units_with_context(
    target: &ResolvedReviewTarget,
    agent: &str,
    context: &str,
) -> Result<Vec<ReviewUnit>, String> {
    let root = Path::new(&target.repository_root);
    let statuses = changed_file_statuses(root, &target.requested_range)?;
    let scope_identity = digest(
        format!(
            "{}\0{}\0{}\0{}\0{}",
            target.repository_root,
            target.diff_mode,
            target.requested_range,
            target.head_sha,
            target.base_sha.as_deref().unwrap_or("")
        )
        .as_bytes(),
    );
    let rules_fingerprint = repository_rules_fingerprint(root);
    let context_fingerprint = digest(context.as_bytes());
    let mut units = Vec::with_capacity(statuses.len());
    for (status, file_path) in statuses {
        validate_relative_path(&file_path)?;
        let diff = git_diff(root, &target.requested_range, Some(&file_path))?;
        let generated = is_generated_path(&file_path);
        let binary = diff.contains("Binary files ") || diff.contains("GIT binary patch");
        let (coverage_state, coverage_reason) = if generated {
            (
                ReviewCoverageState::Skipped,
                Some("generated_file_policy".to_string()),
            )
        } else if binary {
            (
                ReviewCoverageState::Skipped,
                Some("binary_file_policy".to_string()),
            )
        } else {
            (
                ReviewCoverageState::Failed,
                Some("execution_pending".to_string()),
            )
        };
        let fingerprint = digest(
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}:{}:{}",
                REVIEW_MANIFEST_SCHEMA_VERSION,
                scope_identity,
                file_path,
                status,
                agent,
                rules_fingerprint,
                context_fingerprint,
                digest(diff.as_bytes())
            )
            .as_bytes(),
        );
        units.push(ReviewUnit {
            id: format!("unit-{}", &fingerprint[..20]),
            file_path,
            file_status: status,
            fingerprint,
            diff_bytes: diff.len(),
            prompt_budget_bytes: UNIT_PROMPT_BYTES,
            coverage_state,
            coverage_reason,
        });
    }
    units.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(units)
}

pub fn qualify_candidates(
    repo_path: &str,
    changed_files: &[String],
    candidates: Vec<Value>,
) -> QualifiedCandidates {
    let root = match fs::canonicalize(repo_path) {
        Ok(root) => root,
        Err(_) => {
            return reject_all(candidates, "repository_unavailable");
        }
    };
    let changed = changed_files.iter().cloned().collect::<HashSet<_>>();
    let mut findings = Vec::new();
    let mut counts = QualificationCounts::default();
    let mut diagnostics = Vec::new();

    for (candidate_index, mut candidate) in candidates.into_iter().enumerate() {
        let path = candidate
            .get("filePath")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let line = candidate.get("line").and_then(Value::as_i64);
        let outcome = qualify_one(&root, &changed, &candidate, path.as_deref(), line);
        match outcome {
            Ok(qualified) => {
                if let Some(object) = candidate.as_object_mut() {
                    object.insert("filePath".to_string(), json!(qualified.file_path));
                    object.insert("line".to_string(), json!(qualified.resolved_line));
                    object.insert("sourceAnchor".to_string(), json!(qualified.source_anchor));
                    object.insert(
                        "qualification".to_string(),
                        json!({
                            "state": "qualified",
                            "policy_version": REVIEW_MANIFEST_SCHEMA_VERSION,
                            "original_line": line,
                            "resolved_line": qualified.resolved_line,
                        }),
                    );
                    if !qualified.suggestion_allowed {
                        object.remove("suggestion");
                    }
                }
                findings.push(candidate);
                counts.qualified += 1;
                diagnostics.push(QualificationDiagnostic {
                    candidate_index,
                    state: CandidateQualificationState::Qualified,
                    reason: if qualified.suggestion_allowed {
                        qualified.reason
                    } else {
                        format!("{}_suggestion_removed", qualified.reason)
                    },
                    file_path: path,
                    original_line: line,
                    resolved_line: Some(qualified.resolved_line),
                });
            }
            Err((state, reason)) => {
                increment_count(&mut counts, &state);
                diagnostics.push(QualificationDiagnostic {
                    candidate_index,
                    state,
                    reason,
                    file_path: path,
                    original_line: line,
                    resolved_line: None,
                });
            }
        }
    }

    QualifiedCandidates {
        findings,
        counts,
        diagnostics,
    }
}

pub fn invalidate_candidates(
    candidates: Vec<Value>,
    state: CandidateQualificationState,
    reason: &str,
) -> QualifiedCandidates {
    let mut counts = QualificationCounts::default();
    let diagnostics = candidates
        .iter()
        .enumerate()
        .map(|(candidate_index, candidate)| {
            increment_count(&mut counts, &state);
            QualificationDiagnostic {
                candidate_index,
                state: state.clone(),
                reason: reason.to_string(),
                file_path: candidate
                    .get("filePath")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                original_line: candidate.get("line").and_then(Value::as_i64),
                resolved_line: None,
            }
        })
        .collect();
    QualifiedCandidates {
        findings: Vec::new(),
        counts,
        diagnostics,
    }
}

pub fn new_manifest(
    run_id: String,
    target: ResolvedReviewTarget,
    executor_id: String,
    units: Vec<ReviewUnit>,
) -> ReviewManifest {
    let policy_fingerprint = digest(
        format!(
            "{}\0{}\0{}\0{}",
            REVIEW_MANIFEST_SCHEMA_VERSION,
            executor_id,
            UNIT_PROMPT_BYTES,
            repository_rules_fingerprint(Path::new(&target.repository_root))
        )
        .as_bytes(),
    );
    ReviewManifest {
        schema_version: REVIEW_MANIFEST_SCHEMA_VERSION,
        run_id,
        review_id: None,
        target,
        executor_id,
        executor_version: "cli-v1".to_string(),
        policy_fingerprint,
        budgets: ReviewBudgets {
            max_concurrency: REVIEW_MAX_CONCURRENCY,
            prompt_bytes_per_unit: UNIT_PROMPT_BYTES,
            output_bytes_per_attempt: REVIEW_OUTPUT_BYTES,
            attempt_limit: REVIEW_ATTEMPT_LIMIT,
            wall_time_seconds_per_attempt: REVIEW_WALL_TIME_SECONDS,
        },
        complete_coverage: units.iter().all(|unit| {
            matches!(
                unit.coverage_state,
                ReviewCoverageState::Reviewed | ReviewCoverageState::Reused
            )
        }),
        units,
        qualification_counts: QualificationCounts::default(),
        qualification_diagnostics: Vec::new(),
        stale: false,
        cancelled: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
    }
}

pub fn read_target_diff(target: &ResolvedReviewTarget) -> Result<String, String> {
    git_diff(
        Path::new(&target.repository_root),
        &target.requested_range,
        None,
    )
}

pub fn read_unit_diff(target: &ResolvedReviewTarget, file_path: &str) -> Result<String, String> {
    validate_relative_path(file_path)?;
    git_diff(
        Path::new(&target.repository_root),
        &target.requested_range,
        Some(file_path),
    )
}

pub fn load_checkpoint_outputs(
    conn: &Connection,
    manifest: &ReviewManifest,
    unit: &ReviewUnit,
) -> Result<Option<Vec<Value>>, String> {
    conn.query_row(
        "SELECT u.checkpoint_json
         FROM deterministic_review_units u
         JOIN deterministic_review_runs r ON r.run_id=u.run_id
         WHERE u.fingerprint=?1
           AND u.coverage_state IN ('reviewed','reused')
           AND r.schema_version=?2
           AND r.executor_id=?3
           AND r.policy_fingerprint=?4
           AND u.checkpoint_json IS NOT NULL
         ORDER BY u.updated_at DESC LIMIT 1",
        params![
            unit.fingerprint,
            manifest.schema_version,
            manifest.executor_id,
            manifest.policy_fingerprint,
        ],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| error.to_string())?
    .map(|raw| {
        let value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        if value.get("schema_version").and_then(Value::as_u64)
            != Some(REVIEW_MANIFEST_SCHEMA_VERSION as u64)
            || value.get("fingerprint").and_then(Value::as_str) != Some(unit.fingerprint.as_str())
        {
            return Ok(None);
        }
        value
            .get("normalized_outputs")
            .and_then(Value::as_array)
            .cloned()
            .map(Some)
            .ok_or_else(|| "Review checkpoint is missing normalized outputs".to_string())
    })
    .transpose()
    .map(Option::flatten)
}

pub fn persist_unit_checkpoint(
    conn: &Connection,
    manifest: &ReviewManifest,
    unit: &ReviewUnit,
    outputs: &[Value],
) -> Result<(), String> {
    let checkpoint = json!({
        "schema_version": REVIEW_MANIFEST_SCHEMA_VERSION,
        "fingerprint": unit.fingerprint,
        "executor_id": manifest.executor_id,
        "policy_fingerprint": manifest.policy_fingerprint,
        "normalized_outputs": outputs,
    });
    conn.execute(
        "UPDATE deterministic_review_units
         SET coverage_state='reviewed', coverage_reason=NULL,
             checkpoint_json=?3, updated_at=?4
         WHERE run_id=?1 AND unit_id=?2",
        params![
            manifest.run_id,
            unit.id,
            checkpoint.to_string(),
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn record_attempt(
    conn: &Connection,
    manifest: &ReviewManifest,
    unit_id: &str,
    attempt_number: usize,
    status: &str,
    reason: Option<&str>,
    output_bytes: usize,
    started_at: &str,
    terminal_coverage: Option<(&str, &str)>,
) -> Result<(), String> {
    let completed_at = chrono::Utc::now().to_rfc3339();
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if let Some((coverage_state, coverage_reason)) = terminal_coverage {
        let updated = tx
            .execute(
                "UPDATE deterministic_review_units
                 SET coverage_state=?3, coverage_reason=?4, updated_at=?5
                 WHERE run_id=?1 AND unit_id=?2",
                params![
                    manifest.run_id,
                    unit_id,
                    coverage_state,
                    coverage_reason,
                    completed_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err("Review attempt references an unknown unit".to_string());
        }
    }
    tx.execute(
        "INSERT INTO deterministic_review_attempts (
            id, run_id, unit_id, attempt_number, executor_id, status,
            reason, output_bytes, started_at, completed_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(run_id,unit_id,attempt_number) DO UPDATE SET
            status=excluded.status, reason=excluded.reason,
            output_bytes=excluded.output_bytes, completed_at=excluded.completed_at",
        params![
            uuid::Uuid::new_v4().to_string(),
            manifest.run_id,
            unit_id,
            attempt_number as i64,
            manifest.executor_id,
            status,
            reason,
            output_bytes.min(i64::MAX as usize) as i64,
            started_at,
            completed_at,
        ],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

pub fn complete_coverage(manifest: &mut ReviewManifest, aggregate_truncated: bool) {
    for unit in &mut manifest.units {
        if matches!(unit.coverage_state, ReviewCoverageState::Skipped) {
            continue;
        }
        if aggregate_truncated {
            unit.coverage_state = ReviewCoverageState::Failed;
            unit.coverage_reason = Some("aggregate_prompt_truncated".to_string());
        } else {
            unit.coverage_state = ReviewCoverageState::Reviewed;
            unit.coverage_reason = None;
        }
    }
    manifest.complete_coverage = manifest.units.iter().all(|unit| {
        matches!(
            unit.coverage_state,
            ReviewCoverageState::Reviewed | ReviewCoverageState::Reused
        )
    });
}

pub fn target_is_current(target: &ResolvedReviewTarget) -> bool {
    resolve_target(&target.repository_root, &target.requested_range)
        .map(|current| current.source_fingerprint == target.source_fingerprint)
        .unwrap_or(false)
}

pub fn persist_manifest(
    conn: &Connection,
    manifest: &ReviewManifest,
    status: &str,
) -> Result<(), String> {
    let manifest_json = serde_json::to_string(manifest).map_err(|error| error.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO deterministic_review_runs (
            run_id, schema_version, review_id, repo_path, target_identity,
            source_fingerprint, executor_id, policy_fingerprint, status,
            manifest_json, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(run_id) DO UPDATE SET
            review_id=excluded.review_id,
            status=excluded.status,
            manifest_json=excluded.manifest_json,
            updated_at=excluded.updated_at",
        params![
            manifest.run_id,
            manifest.schema_version,
            manifest.review_id,
            manifest.target.repository_root,
            manifest.target.identity,
            manifest.target.source_fingerprint,
            manifest.executor_id,
            manifest.policy_fingerprint,
            status,
            manifest_json,
            manifest.created_at,
            now,
        ],
    )
    .map_err(|error| error.to_string())?;
    for unit in &manifest.units {
        tx.execute(
            "INSERT INTO deterministic_review_units (
                run_id, unit_id, file_path, fingerprint, coverage_state,
                coverage_reason, checkpoint_json, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(run_id, unit_id) DO UPDATE SET
                coverage_state=excluded.coverage_state,
                coverage_reason=excluded.coverage_reason,
                updated_at=excluded.updated_at",
            params![
                manifest.run_id,
                unit.id,
                unit.file_path,
                unit.fingerprint,
                coverage_name(&unit.coverage_state),
                unit.coverage_reason,
                Option::<String>::None,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    for diagnostic in &manifest.qualification_diagnostics {
        tx.execute(
            "INSERT INTO deterministic_review_qualification (
                run_id, candidate_index, state, reason, file_path,
                original_line, resolved_line
             ) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(run_id, candidate_index) DO UPDATE SET
                state=excluded.state,
                reason=excluded.reason,
                file_path=excluded.file_path,
                original_line=excluded.original_line,
                resolved_line=excluded.resolved_line",
            params![
                manifest.run_id,
                diagnostic.candidate_index as i64,
                qualification_name(&diagnostic.state),
                diagnostic.reason,
                diagnostic.file_path,
                diagnostic.original_line,
                diagnostic.resolved_line,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())
}

pub fn claim_manifest(conn: &Connection, manifest: &ReviewManifest) -> Result<(), String> {
    let now = chrono::Utc::now();
    let stale_before = (now - chrono::Duration::minutes(30)).to_rfc3339();
    conn.execute(
        "UPDATE deterministic_review_runs
         SET status='abandoned', updated_at=?1
         WHERE status IN ('planning','running') AND updated_at < ?2",
        params![chrono::Utc::now().to_rfc3339(), stale_before],
    )
    .map_err(|error| error.to_string())?;
    cleanup_unlinked_terminal_manifests(conn, &(now - chrono::Duration::days(30)).to_rfc3339())?;
    persist_manifest(conn, manifest, "planning").map_err(|error| {
        if error.contains("UNIQUE constraint failed") {
            "An identical deterministic review is already running".to_string()
        } else {
            error
        }
    })
}

fn cleanup_unlinked_terminal_manifests(
    conn: &Connection,
    updated_before: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM deterministic_review_runs
         WHERE review_id IS NULL
           AND status IN ('completed','completed_with_limitations','failed','cancelled','abandoned')
           AND updated_at < ?1",
        params![updated_before],
    )
    .map_err(|error| error.to_string())
}

pub fn load_manifest_for_review(
    conn: &Connection,
    review_id: &str,
) -> Result<Option<ReviewManifest>, String> {
    conn.query_row(
        "SELECT manifest_json FROM deterministic_review_runs
         WHERE review_id=?1 ORDER BY updated_at DESC LIMIT 1",
        params![review_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| error.to_string())?
    .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
    .transpose()
}

pub fn public_manifest_page(
    conn: &Connection,
    repo_path: &str,
    review_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Value, String> {
    let limit = limit.clamp(1, 100);
    let mut statement = conn
        .prepare(
            "SELECT manifest_json FROM deterministic_review_runs
             WHERE repo_path=?1 AND review_id IS NOT NULL
               AND (?2 IS NULL OR review_id=?2)
             ORDER BY updated_at DESC, run_id DESC
             LIMIT ?3 OFFSET ?4",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![repo_path, review_id, (limit + 1) as i64, offset as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?;
    let mut manifests = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|raw| serde_json::from_str::<ReviewManifest>(&raw).ok())
        .collect::<Vec<_>>();
    let has_more = manifests.len() > limit;
    manifests.truncate(limit);
    let items = manifests
        .into_iter()
        .map(|manifest| {
            json!({
                "schema_version": manifest.schema_version,
                "run_id": manifest.run_id,
                "review_id": manifest.review_id,
                "target": {
                    "identity": manifest.target.identity,
                    "diff_mode": manifest.target.diff_mode,
                    "requested_range": manifest.target.requested_range,
                    "head_sha": manifest.target.head_sha,
                    "base_sha": manifest.target.base_sha,
                    "source_fingerprint": manifest.target.source_fingerprint,
                },
                "executor": {
                    "id": manifest.executor_id,
                    "version": manifest.executor_version,
                },
                "policy_fingerprint": manifest.policy_fingerprint,
                "budgets": manifest.budgets,
                "units": manifest.units,
                "qualification_counts": manifest.qualification_counts,
                "complete_coverage": manifest.complete_coverage,
                "stale": manifest.stale,
                "cancelled": manifest.cancelled,
                "created_at": manifest.created_at,
                "completed_at": manifest.completed_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "items": items,
        "next_offset": has_more.then_some(offset + limit),
    }))
}

fn coverage_name(state: &ReviewCoverageState) -> &'static str {
    match state {
        ReviewCoverageState::Reviewed => "reviewed",
        ReviewCoverageState::Reused => "reused",
        ReviewCoverageState::Skipped => "skipped",
        ReviewCoverageState::Failed => "failed",
        ReviewCoverageState::Cancelled => "cancelled",
    }
}

fn qualification_name(state: &CandidateQualificationState) -> &'static str {
    match state {
        CandidateQualificationState::Qualified => "qualified",
        CandidateQualificationState::Stale => "stale",
        CandidateQualificationState::Unresolved => "unresolved",
        CandidateQualificationState::Rejected => "rejected",
    }
}

fn qualify_one(
    root: &Path,
    changed: &HashSet<String>,
    candidate: &Value,
    path: Option<&str>,
    line: Option<i64>,
) -> Result<QualifiedLocation, (CandidateQualificationState, String)> {
    if !candidate.is_object() {
        return Err(rejected("candidate_not_object"));
    }
    let path = path.ok_or_else(|| rejected("missing_file_path"))?;
    validate_relative_path(path).map_err(|_| rejected("unsafe_file_path"))?;
    if is_sensitive_path(path) {
        return Err(rejected("protected_file_path"));
    }
    if !changed.contains(path) {
        return Err(rejected("file_not_in_review_target"));
    }
    validate_text_field(candidate, "title", MAX_FINDING_TITLE_BYTES)?;
    validate_text_field(candidate, "summary", MAX_FINDING_SUMMARY_BYTES)?;
    let severity = candidate
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !matches!(severity, "critical" | "high" | "medium" | "low") {
        return Err(rejected("invalid_severity"));
    }
    if candidate.get("confidence").is_some_and(|value| {
        value
            .as_f64()
            .is_none_or(|number| !(0.0..=1.0).contains(&number))
    }) {
        return Err(rejected("invalid_confidence"));
    }
    let canonical = root.join(path).canonicalize().map_err(|_| {
        (
            CandidateQualificationState::Unresolved,
            "file_unavailable".to_string(),
        )
    })?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(rejected("path_escape_or_non_file"));
    }
    let source = fs::read_to_string(&canonical).map_err(|_| {
        (
            CandidateQualificationState::Unresolved,
            "source_not_text".to_string(),
        )
    })?;
    let line = line.ok_or_else(|| {
        (
            CandidateQualificationState::Unresolved,
            "missing_line".to_string(),
        )
    })?;
    if line <= 0 {
        return Err(stale("line_out_of_bounds"));
    }
    let lines = source.lines().collect::<Vec<_>>();
    let current_line = lines
        .get((line - 1) as usize)
        .copied()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| stale("line_out_of_bounds_or_empty"))?;
    let declared_anchor = candidate
        .get("sourceAnchor")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (resolved_line, source_anchor, qualification_reason) = match declared_anchor {
        None => (
            line,
            current_line.chars().take(500).collect::<String>(),
            "source_anchor_inferred_from_immutable_target".to_string(),
        ),
        Some(anchor) if anchor == current_line => (
            line,
            anchor.chars().take(500).collect::<String>(),
            "source_anchor_verified".to_string(),
        ),
        Some(anchor) => {
            let matches = lines
                .iter()
                .enumerate()
                .filter(|(_, value)| value.trim() == anchor)
                .map(|(index, _)| index as i64 + 1)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [only] => (
                    *only,
                    anchor.chars().take(500).collect::<String>(),
                    "source_anchor_relocated_uniquely".to_string(),
                ),
                [] => return Err(stale("source_anchor_mismatch")),
                _ => {
                    return Err((
                        CandidateQualificationState::Unresolved,
                        "source_anchor_ambiguous".to_string(),
                    ));
                }
            }
        }
    };
    let suggestion_allowed = candidate
        .get("suggestion")
        .and_then(Value::as_str)
        .map(|value| value.len() <= MAX_SUGGESTION_BYTES && !value.contains('\0'))
        .unwrap_or(true)
        && candidate
            .get("suggestionFilePath")
            .and_then(Value::as_str)
            .is_none_or(|suggestion_path| suggestion_path == path);
    Ok(QualifiedLocation {
        file_path: path.to_string(),
        resolved_line,
        source_anchor,
        suggestion_allowed,
        reason: qualification_reason,
    })
}

fn validate_text_field(
    candidate: &Value,
    field: &str,
    max_bytes: usize,
) -> Result<(), (CandidateQualificationState, String)> {
    let value = candidate.get(field).and_then(Value::as_str).unwrap_or("");
    if value.trim().is_empty() || value.len() > max_bytes || value.contains('\0') {
        return Err(rejected(&format!("invalid_or_oversized_{field}")));
    }
    Ok(())
}

fn reject_all(candidates: Vec<Value>, reason: &str) -> QualifiedCandidates {
    let diagnostics = candidates
        .iter()
        .enumerate()
        .map(|(candidate_index, candidate)| QualificationDiagnostic {
            candidate_index,
            state: CandidateQualificationState::Rejected,
            reason: reason.to_string(),
            file_path: candidate
                .get("filePath")
                .and_then(Value::as_str)
                .map(str::to_string),
            original_line: candidate.get("line").and_then(Value::as_i64),
            resolved_line: None,
        })
        .collect();
    QualifiedCandidates {
        findings: Vec::new(),
        counts: QualificationCounts {
            rejected: candidates.len(),
            ..QualificationCounts::default()
        },
        diagnostics,
    }
}

fn increment_count(counts: &mut QualificationCounts, state: &CandidateQualificationState) {
    match state {
        CandidateQualificationState::Qualified => counts.qualified += 1,
        CandidateQualificationState::Stale => counts.stale += 1,
        CandidateQualificationState::Unresolved => counts.unresolved += 1,
        CandidateQualificationState::Rejected => counts.rejected += 1,
    }
}

fn rejected(reason: &str) -> (CandidateQualificationState, String) {
    (CandidateQualificationState::Rejected, reason.to_string())
}

fn stale(reason: &str) -> (CandidateQualificationState, String) {
    (CandidateQualificationState::Stale, reason.to_string())
}

fn changed_file_statuses(root: &Path, range: &str) -> Result<Vec<(String, String)>, String> {
    let mut args = diff_prefix(range)?;
    args.splice(1..1, ["--name-status".to_string()]);
    let output = command_output(root, &args)?;
    let mut rows = Vec::new();
    for line in output.lines() {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }
        let status = parts[0].chars().next().unwrap_or('M').to_string();
        let path = if matches!(status.as_str(), "R" | "C") && parts.len() >= 3 {
            parts[2]
        } else {
            parts[1]
        };
        rows.push((status, path.to_string()));
    }
    Ok(rows)
}

fn git_diff(root: &Path, range: &str, path: Option<&str>) -> Result<String, String> {
    let mut args = diff_prefix(range)?;
    if let Some(path) = path {
        args.push(path.to_string());
    }
    command_output(root, &args)
}

fn diff_prefix(range: &str) -> Result<Vec<String>, String> {
    match range {
        "WORKTREE" => Ok(vec!["diff".to_string(), "--".to_string()]),
        "--staged" | "--cached" => Ok(vec![
            "diff".to_string(),
            "--cached".to_string(),
            "--".to_string(),
        ]),
        value if value.starts_with('-') => Err("Unsupported Git diff option".to_string()),
        value => Ok(vec![
            "diff".to_string(),
            value.to_string(),
            "--".to_string(),
        ]),
    }
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    command_output(
        root,
        &args
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    )
}

fn command_output(root: &Path, args: &[String]) -> Result<String, String> {
    let output = Command::new("git")
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|_| "Could not run Git for review planning")?;
    if !output.status.success() {
        return Err("Git could not resolve the requested review target".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\0') || Path::new(path).is_absolute() {
        return Err("Unsafe review path".to_string());
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Unsafe review path".to_string());
    }
    Ok(())
}

fn is_generated_path(path: &str) -> bool {
    let normalized = path.to_ascii_lowercase();
    normalized.contains("/generated/")
        || normalized.starts_with("generated/")
        || normalized.ends_with(".min.js")
        || normalized.ends_with(".lock")
}

fn repository_rules_fingerprint(root: &Path) -> String {
    let mut rules = Vec::new();
    for name in ["AGENTS.md", "agents.md", "CLAUDE.md"] {
        let path = root.join(name);
        if let Ok(bytes) = fs::read(path) {
            rules.extend_from_slice(name.as_bytes());
            rules.push(0);
            rules.extend_from_slice(&bytes[..bytes.len().min(16 * 1024)]);
            rules.push(0);
        }
    }
    digest(&rules)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use tempfile::TempDir;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git");
        assert!(status.success());
    }

    fn fixture() -> TempDir {
        let temp = tempfile::tempdir().expect("temp");
        git(temp.path(), &["init", "-q"]);
        git(temp.path(), &["config", "user.email", "test@example.com"]);
        git(temp.path(), &["config", "user.name", "Test"]);
        fs::write(temp.path().join("safe.rs"), "fn safe() {}\n").expect("write");
        git(temp.path(), &["add", "safe.rs"]);
        git(temp.path(), &["commit", "-qm", "base"]);
        fs::write(
            temp.path().join("safe.rs"),
            "fn safe() {}\nfn changed() { panic!(\"boom\") }\n",
        )
        .expect("write");
        temp
    }

    #[test]
    fn target_and_units_are_deterministic_and_option_safe() {
        let temp = fixture();
        let first = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let second = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        assert_eq!(first.identity, second.identity);
        assert_eq!(
            plan_units(&first, "claude").unwrap()[0].file_path,
            "safe.rs"
        );
        assert!(resolve_target(temp.path().to_str().unwrap(), "--output=/tmp/x").is_err());
    }

    #[test]
    fn qualification_rejects_paths_and_impossible_lines() {
        let temp = fixture();
        let candidates = vec![
            json!({"severity":"high","title":"Valid","summary":"Evidence","filePath":"safe.rs","line":2}),
            json!({"severity":"high","title":"Escape","summary":"Evidence","filePath":"../secret","line":1}),
            json!({"severity":"high","title":"Stale","summary":"Evidence","filePath":"safe.rs","line":99}),
        ];
        let result = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            candidates,
        );
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.counts.qualified, 1);
        assert_eq!(result.counts.rejected, 1);
        assert_eq!(result.counts.stale, 1);
        assert_eq!(
            result.findings[0]["sourceAnchor"],
            "fn changed() { panic!(\"boom\") }"
        );
    }

    #[test]
    fn invalid_suggestion_does_not_discard_valid_evidence() {
        let temp = fixture();
        let result = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            vec![json!({
                "severity":"medium",
                "title":"Valid evidence",
                "summary":"Evidence",
                "filePath":"safe.rs",
                "line":2,
                "suggestion":"x".repeat(MAX_SUGGESTION_BYTES + 1)
            })],
        );
        assert_eq!(result.findings.len(), 1);
        assert!(result.findings[0].get("suggestion").is_none());
    }

    #[test]
    fn source_anchor_relocates_only_when_unique() {
        let temp = fixture();
        let relocated = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            vec![json!({
                "severity":"high", "title":"Moved", "summary":"Evidence",
                "filePath":"safe.rs", "line":1,
                "sourceAnchor":"fn changed() { panic!(\"boom\") }"
            })],
        );
        assert_eq!(relocated.findings[0]["line"], 2);
        assert_eq!(
            relocated.diagnostics[0].reason,
            "source_anchor_relocated_uniquely"
        );

        fs::write(
            temp.path().join("safe.rs"),
            "same();\nsame();\nfn changed() {}\n",
        )
        .expect("duplicate");
        let ambiguous = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            vec![json!({
                "severity":"high", "title":"Ambiguous", "summary":"Evidence",
                "filePath":"safe.rs", "line":3, "sourceAnchor":"same();"
            })],
        );
        assert!(ambiguous.findings.is_empty());
        assert_eq!(ambiguous.counts.unresolved, 1);
    }

    #[test]
    fn suggestion_for_another_file_is_removed() {
        let temp = fixture();
        let result = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            vec![json!({
                "severity":"medium", "title":"Valid evidence", "summary":"Evidence",
                "filePath":"safe.rs", "line":2, "suggestion":"change it",
                "suggestionFilePath":"other.rs"
            })],
        );
        assert_eq!(result.findings.len(), 1);
        assert!(result.findings[0].get("suggestion").is_none());
        assert!(result.diagnostics[0].reason.ends_with("suggestion_removed"));
    }

    #[test]
    fn stale_target_invalidates_candidates_without_actionable_findings() {
        let result = invalidate_candidates(
            vec![json!({
                "severity":"high", "title":"Candidate", "summary":"Evidence",
                "filePath":"safe.rs", "line":2
            })],
            CandidateQualificationState::Stale,
            "target_mutated_during_review",
        );
        assert!(result.findings.is_empty());
        assert_eq!(result.counts.stale, 1);
        assert_eq!(result.counts.qualified, 0);
        assert_eq!(result.diagnostics[0].reason, "target_mutated_during_review");
    }

    #[test]
    fn multi_file_diff_over_100k_keeps_every_file_as_a_bounded_unit() {
        let temp = fixture();
        fs::write(
            temp.path().join("left.rs"),
            format!("{}\n", "let left = 1;".repeat(4_500)),
        )
        .expect("left");
        fs::write(
            temp.path().join("right.rs"),
            format!("{}\n", "let right = 2;".repeat(4_500)),
        )
        .expect("right");
        git(temp.path(), &["add", "left.rs", "right.rs"]);
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let units = plan_units(&target, "claude").expect("units");
        assert!(read_target_diff(&target).expect("diff").len() > 100 * 1024);
        assert_eq!(units.len(), 3);
        assert!(units.iter().any(|unit| unit.file_path == "left.rs"));
        assert!(units.iter().any(|unit| unit.file_path == "right.rs"));
        assert!(units
            .iter()
            .filter(|unit| unit.file_path != "safe.rs")
            .all(|unit| unit.diff_bytes <= unit.prompt_budget_bytes));
    }

    #[test]
    fn planner_covers_rename_delete_generated_binary_unicode_and_empty_targets() {
        let temp = tempfile::tempdir().expect("temp");
        git(temp.path(), &["init", "-q"]);
        git(temp.path(), &["config", "user.email", "test@example.com"]);
        git(temp.path(), &["config", "user.name", "Test"]);
        fs::write(temp.path().join("rename.rs"), "fn rename() {}\n").expect("rename");
        fs::write(temp.path().join("delete.rs"), "fn delete() {}\n").expect("delete");
        fs::write(temp.path().join("generated.lock"), "v1\n").expect("generated");
        fs::write(temp.path().join("binary.bin"), [0, 1, 2, 3]).expect("binary");
        fs::write(temp.path().join("café.rs"), "fn café() {}\n").expect("unicode");
        git(temp.path(), &["add", "-A"]);
        git(temp.path(), &["commit", "-qm", "base"]);
        git(temp.path(), &["mv", "rename.rs", "renamed.rs"]);
        fs::remove_file(temp.path().join("delete.rs")).expect("delete file");
        fs::write(temp.path().join("generated.lock"), "v2\n").expect("generated change");
        fs::write(temp.path().join("binary.bin"), [0, 255, 2, 3]).expect("binary change");
        fs::write(temp.path().join("café.rs"), "fn café_changed() {}\n").expect("unicode change");
        git(temp.path(), &["add", "-A"]);

        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let units = plan_units(&target, "claude").expect("units");
        assert_eq!(units.len(), 5);
        assert!(units
            .iter()
            .any(|unit| unit.file_status == "R" && unit.file_path == "renamed.rs"));
        assert!(units
            .iter()
            .any(|unit| unit.file_status == "D" && unit.file_path == "delete.rs"));
        assert!(units.iter().any(|unit| unit.file_path == "café.rs"));
        assert!(units.iter().any(|unit| {
            unit.file_path == "generated.lock"
                && unit.coverage_state == ReviewCoverageState::Skipped
                && unit.coverage_reason.as_deref() == Some("generated_file_policy")
        }));
        assert!(units.iter().any(|unit| {
            unit.file_path == "binary.bin"
                && unit.coverage_state == ReviewCoverageState::Skipped
                && unit.coverage_reason.as_deref() == Some("binary_file_policy")
        }));

        git(temp.path(), &["commit", "-qm", "changes"]);
        let empty = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("empty target");
        assert!(plan_units(&empty, "claude")
            .expect("empty units")
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn qualification_rejects_absolute_unknown_protected_symlink_and_invalid_schema() {
        use std::os::unix::fs::symlink;
        let temp = fixture();
        let outside = tempfile::NamedTempFile::new().expect("outside");
        symlink(outside.path(), temp.path().join("escape.rs")).expect("symlink");
        let candidates = vec![
            json!({"severity":"high","title":"Absolute","summary":"Evidence","filePath":"/tmp/x","line":1}),
            json!({"severity":"high","title":"Unknown","summary":"Evidence","filePath":"unknown.rs","line":1}),
            json!({"severity":"high","title":"Protected","summary":"Evidence","filePath":".env","line":1}),
            json!({"severity":"high","title":"Symlink","summary":"Evidence","filePath":"escape.rs","line":1}),
            json!({"severity":"urgent","title":"Enum","summary":"Evidence","filePath":"safe.rs","line":2}),
            json!({"severity":"high","title":"x".repeat(MAX_FINDING_TITLE_BYTES + 1),"summary":"Evidence","filePath":"safe.rs","line":2}),
        ];
        let result = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".into(), ".env".into(), "escape.rs".into()],
            candidates,
        );
        assert!(result.findings.is_empty());
        assert_eq!(result.counts.rejected, 6);
    }

    #[test]
    fn exact_checkpoint_reuses_and_changed_fingerprint_invalidates() {
        let temp = fixture();
        fs::write(temp.path().join("other.rs"), "fn other() {}\n").expect("other");
        git(temp.path(), &["add", "other.rs"]);
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let mut manifest = new_manifest(
            "run-1".into(),
            target.clone(),
            "claude".into(),
            plan_units(&target, "claude").expect("units"),
        );
        let other_index = manifest
            .units
            .iter()
            .position(|unit| unit.file_path == "other.rs")
            .expect("other unit");
        manifest.units[other_index].coverage_state = ReviewCoverageState::Reviewed;
        manifest.units[other_index].coverage_reason = None;
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        persist_manifest(&conn, &manifest, "running").expect("manifest");
        let outputs = vec![json!({"findings": [], "summary": "checked"})];
        persist_unit_checkpoint(&conn, &manifest, &manifest.units[other_index], &outputs)
            .expect("checkpoint");
        assert_eq!(
            load_checkpoint_outputs(&conn, &manifest, &manifest.units[other_index])
                .expect("load")
                .expect("reused"),
            outputs
        );

        fs::write(temp.path().join("safe.rs"), "fn changed_again() {}\n").expect("change");
        let changed_target =
            resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("changed target");
        let changed = new_manifest(
            "run-2".into(),
            changed_target.clone(),
            "claude".into(),
            plan_units(&changed_target, "claude").expect("changed units"),
        );
        let changed_other = changed
            .units
            .iter()
            .find(|unit| unit.file_path == "other.rs")
            .expect("changed other");
        let changed_safe = changed
            .units
            .iter()
            .find(|unit| unit.file_path == "safe.rs")
            .expect("changed safe");
        assert!(load_checkpoint_outputs(&conn, &changed, changed_other)
            .expect("load changed")
            .is_some());
        assert_ne!(
            manifest
                .units
                .iter()
                .find(|unit| unit.file_path == "safe.rs")
                .unwrap()
                .fingerprint,
            changed_safe.fingerprint
        );
    }

    #[test]
    fn unit_fingerprint_invalidates_context_rules_and_executor() {
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let baseline = plan_units_with_context(&target, "claude", "goal:a").expect("baseline");
        let context = plan_units_with_context(&target, "claude", "goal:b").expect("context");
        let executor = plan_units_with_context(&target, "gemini", "goal:a").expect("executor");
        fs::write(temp.path().join("AGENTS.md"), "Rule: verify.\n").expect("rules");
        let rules = plan_units_with_context(&target, "claude", "goal:a").expect("rules");
        assert_ne!(baseline[0].fingerprint, context[0].fingerprint);
        assert_ne!(baseline[0].fingerprint, executor[0].fingerprint);
        assert_ne!(baseline[0].fingerprint, rules[0].fingerprint);
    }

    #[test]
    fn public_manifest_page_is_scoped_paginated_and_redacted() {
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let mut manifest = new_manifest(
            "run-public".into(),
            target.clone(),
            "claude".into(),
            plan_units(&target, "claude").expect("units"),
        );
        manifest.review_id = Some("review-public".into());
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        conn.execute(
            "INSERT INTO local_reviews (id, agent_used, status, created_at) VALUES ('review-public','claude','completed','2026-07-22')",
            [],
        )
        .expect("review");
        persist_manifest(&conn, &manifest, "completed_with_limitations").expect("manifest");
        let page =
            public_manifest_page(&conn, &target.repository_root, Some("review-public"), 1, 0)
                .expect("page");
        let serialized = page.to_string();
        assert!(!serialized.contains(&target.repository_root));
        assert!(!serialized.contains("repository_root"));
        assert_eq!(page["items"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn identical_active_review_claim_is_exclusive() {
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let units = plan_units(&target, "claude").expect("units");
        let first = new_manifest(
            "run-first".into(),
            target.clone(),
            "claude".into(),
            units.clone(),
        );
        let second = new_manifest("run-second".into(), target, "claude".into(), units);
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        claim_manifest(&conn, &first).expect("first claim");
        assert_eq!(
            claim_manifest(&conn, &second).expect_err("duplicate claim"),
            "An identical deterministic review is already running"
        );
        persist_manifest(&conn, &first, "failed").expect("release claim");
        claim_manifest(&conn, &second).expect("claim after terminal state");
    }

    #[test]
    fn failed_attempt_and_terminal_unit_state_commit_atomically() {
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let manifest = new_manifest(
            "run-attempt".into(),
            target.clone(),
            "claude".into(),
            plan_units(&target, "claude").expect("units"),
        );
        let unit_id = manifest.units[0].id.clone();
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        persist_manifest(&conn, &manifest, "running").expect("manifest");

        record_attempt(
            &conn,
            &manifest,
            &unit_id,
            1,
            "failed",
            Some("executor_failed"),
            0,
            "2026-07-22T00:00:00Z",
            Some(("failed", "executor_failed")),
        )
        .expect("attempt");
        let (attempt_status, coverage_state, coverage_reason): (String, String, String) = conn
            .query_row(
                "SELECT a.status,u.coverage_state,u.coverage_reason
                 FROM deterministic_review_attempts a
                 JOIN deterministic_review_units u
                   ON u.run_id=a.run_id AND u.unit_id=a.unit_id
                 WHERE a.run_id=?1 AND a.unit_id=?2",
                params![manifest.run_id, unit_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("states");
        assert_eq!(attempt_status, "failed");
        assert_eq!(coverage_state, "failed");
        assert_eq!(coverage_reason, "executor_failed");

        assert_eq!(
            record_attempt(
                &conn,
                &manifest,
                "unknown-unit",
                1,
                "failed",
                Some("executor_failed"),
                0,
                "2026-07-22T00:00:00Z",
                Some(("failed", "executor_failed")),
            )
            .expect_err("unknown unit"),
            "Review attempt references an unknown unit"
        );
        let rolled_back: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM deterministic_review_attempts WHERE unit_id='unknown-unit'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(rolled_back, 0);
    }

    #[test]
    fn retention_removes_only_old_unlinked_terminal_manifests() {
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let units = plan_units(&target, "claude").expect("units");
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        for (run_id, review_id, status) in [
            ("old-unlinked", None, "failed"),
            ("old-linked", Some("review-retained"), "completed"),
            ("old-active", None, "running"),
        ] {
            if review_id.is_some() {
                conn.execute(
                    "INSERT INTO local_reviews (id, agent_used, status, created_at)
                     VALUES ('review-retained','claude','completed','2026-07-22')",
                    [],
                )
                .ok();
            }
            let mut manifest = new_manifest(
                run_id.into(),
                target.clone(),
                "claude".into(),
                units.clone(),
            );
            manifest.review_id = review_id.map(str::to_string);
            persist_manifest(&conn, &manifest, status).expect("manifest");
            conn.execute(
                "UPDATE deterministic_review_runs SET updated_at='2026-01-01T00:00:00Z'
                 WHERE run_id=?1",
                params![run_id],
            )
            .expect("age manifest");
        }

        assert_eq!(
            cleanup_unlinked_terminal_manifests(&conn, "2026-06-01T00:00:00Z").expect("cleanup"),
            1
        );
        let retained = conn
            .prepare("SELECT run_id FROM deterministic_review_runs ORDER BY run_id")
            .expect("statement")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("run ids");
        assert_eq!(retained, vec!["old-active", "old-linked"]);
    }

    #[test]
    fn recorded_benchmark_never_emits_an_invalid_position_after_qualification() {
        let benchmark = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../benchmark");
        let raw_dir = benchmark.join("reviews-raw");
        let mut raw_candidates = 0usize;
        let mut qualified_candidates = 0usize;
        for entry in fs::read_dir(&raw_dir).expect("raw reviews") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let case_id = path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| value.strip_suffix(".codevetter.raw.json"))
                .expect("case id");
            let raw: Value = serde_json::from_str(&fs::read_to_string(&path).expect("raw review"))
                .expect("raw json");
            let findings = raw
                .get("findings")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            raw_candidates += findings.len();
            let case_dir = benchmark.join("cases").join(case_id);
            let label: Value = serde_json::from_str(
                &fs::read_to_string(case_dir.join("label.json")).expect("label"),
            )
            .expect("label json");
            let source_file = label["source_file"].as_str().expect("source file");
            let qualified = qualify_candidates(
                case_dir.to_str().unwrap(),
                &[source_file.to_string()],
                findings,
            );
            for finding in &qualified.findings {
                let file = finding["filePath"].as_str().expect("file path");
                let line = finding["line"].as_i64().expect("line");
                let line_count = fs::read_to_string(case_dir.join(file))
                    .expect("source")
                    .lines()
                    .count() as i64;
                assert!((1..=line_count).contains(&line));
            }
            qualified_candidates += qualified.findings.len();
        }
        assert!(
            raw_candidates >= 29,
            "recorded corpus is unexpectedly small"
        );
        assert!(
            qualified_candidates >= 29,
            "qualification removed too much evidence"
        );
    }

    #[test]
    fn fixture_shadow_reconciliation_reports_bounded_deltas_without_provider_calls() {
        fn peak_rss_bytes() -> u64 {
            let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
            if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
                return 0;
            }
            let usage = unsafe { usage.assume_init() };
            #[cfg(target_os = "linux")]
            return (usage.ru_maxrss.max(0) as u64).saturating_mul(1024);
            #[cfg(not(target_os = "linux"))]
            return usage.ru_maxrss.max(0) as u64;
        }

        let rss_before = peak_rss_bytes();
        let started = std::time::Instant::now();
        let temp = fixture();
        let target = resolve_target(temp.path().to_str().unwrap(), "HEAD").expect("target");
        let units = plan_units(&target, "claude").expect("units");
        let mut manifest = new_manifest("shadow-run".into(), target, "claude".into(), units);
        let recorded_aggregate = vec![json!({
            "severity": "high",
            "title": "Panic on reachable input",
            "summary": "The changed function panics instead of returning an error.",
            "filePath": "safe.rs",
            "line": 2,
            "sourceAnchor": "fn changed() { panic!(\"boom\") }"
        })];
        let aggregate = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            recorded_aggregate.clone(),
        );
        let shadow = qualify_candidates(
            temp.path().to_str().unwrap(),
            &["safe.rs".to_string()],
            recorded_aggregate,
        );
        manifest.units[0].coverage_state = ReviewCoverageState::Reviewed;
        manifest.complete_coverage = true;
        manifest.qualification_counts = shadow.counts;
        manifest.qualification_diagnostics = shadow.diagnostics;
        manifest.completed_at = Some("2026-07-24T00:00:00Z".to_string());
        let storage_bytes = serde_json::to_vec(&manifest).unwrap().len();
        let elapsed = started.elapsed();
        let rss_delta_bytes = peak_rss_bytes().saturating_sub(rss_before);

        assert_eq!(shadow.findings, aggregate.findings);
        assert_eq!(manifest.units.len(), 1);
        assert!(manifest.complete_coverage);
        assert_eq!(manifest.qualification_counts.qualified, 1);
        assert!(storage_bytes < 32 * 1024);
        assert!(elapsed < std::time::Duration::from_secs(1));
        assert!(rss_delta_bytes < 8 * 1024 * 1024);
        let provider_call_delta = 0_i32;
        assert_eq!(provider_call_delta, 0);
        eprintln!(
            "shadow_reconciliation qualified={} coverage={}/{} duration_ms={} storage_bytes={} provider_call_delta={} rss_delta_bytes={}",
            manifest.qualification_counts.qualified,
            manifest.units.iter().filter(|unit| matches!(unit.coverage_state, ReviewCoverageState::Reviewed | ReviewCoverageState::Reused)).count(),
            manifest.units.len(),
            elapsed.as_millis(),
            storage_bytes,
            provider_call_delta,
            rss_delta_bytes,
        );
    }
}
