use super::types::*;
use crate::commands::secret_policy::{
    contains_sensitive_path, looks_like_secret, redact_secret_text,
};
use crate::commands::structural_graph::types::{stable_graph_id, GraphSourceAnchor, GraphTrust};
use crate::DbState;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::{fs, io::Read};
use tauri::State;

pub fn deterministic_evidence_id(
    adapter_id: &str,
    source_record_id: &str,
    effective_at: Option<&str>,
) -> String {
    stable_graph_id(
        "history-evidence",
        &format!(
            "{adapter_id}\0{source_record_id}\0{}",
            effective_at.unwrap_or("")
        ),
    )
}

#[tauri::command]
pub async fn get_history_evidence_adapters(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Vec<HistoryEvidenceAdapterDescriptor>, String> {
    let repo_path = canonical_repo_path(&repo_path)?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        adapter_descriptors(&connection, &repo_path)
    })
    .await
    .map_err(|error| format!("History adapter status worker failed: {error}"))?
}

#[tauri::command]
pub async fn import_history_evidence_export(
    repo_path: String,
    file_path: String,
    db: State<'_, DbState>,
) -> Result<HistoryEvidenceRefreshResult, String> {
    let repo_path = canonical_repo_path(&repo_path)?;
    let file_path = PathBuf::from(file_path.trim());
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let export = read_local_evidence_export(&file_path)?;
        let refreshed_at = Utc::now().to_rfc3339();
        let records = normalize_local_export(export)?;
        let mut connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        persist_imported_records(&mut connection, &repo_path, &records, &refreshed_at)
    })
    .await
    .map_err(|error| format!("History evidence import worker failed: {error}"))?
}

fn read_local_evidence_export(file_path: &Path) -> Result<HistoryLocalEvidenceExport, String> {
    if file_path.extension().and_then(|value| value.to_str()) != Some("json") {
        return Err("History evidence imports must be JSON files".to_string());
    }
    let mut file = fs::File::open(file_path)
        .map_err(|error| format!("Open local evidence export: {error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("Read local evidence export metadata: {error}"))?
        .len();
    if size > 16 * 1024 * 1024 {
        return Err("History evidence export exceeds the 16 MiB local import bound".to_string());
    }
    let mut json = String::with_capacity(size as usize);
    file.read_to_string(&mut json)
        .map_err(|error| format!("Read local evidence export: {error}"))?;
    let export: HistoryLocalEvidenceExport =
        serde_json::from_str(&json).map_err(|error| format!("Parse evidence export: {error}"))?;
    if export.schema_version != 1 {
        return Err(format!(
            "Unsupported history evidence export schema {}",
            export.schema_version
        ));
    }
    if export.records.len() > 10_000 {
        return Err("History evidence export exceeds the 10,000-record bound".to_string());
    }
    Ok(export)
}

fn normalize_local_export(
    export: HistoryLocalEvidenceExport,
) -> Result<Vec<HistoryEvidenceRecord>, String> {
    let source = export.source.trim();
    if source.is_empty() || source.len() > 120 {
        return Err("Evidence export source must be between 1 and 120 bytes".to_string());
    }
    if looks_like_secret(source) || contains_sensitive_path(source) {
        return Err("Evidence export source contains credential-like data".to_string());
    }
    let cursor_redacted = export
        .cursor
        .as_deref()
        .is_some_and(|cursor| looks_like_secret(cursor) || contains_sensitive_path(cursor));
    let safe_cursor = (!cursor_redacted).then_some(export.cursor).flatten();
    let allowed = [
        "analytics_provider_ingestion",
        "analytics_provider_delivery",
        "deploy",
        "incident",
        "observed_outcome",
        "log_observation",
        "pull_request",
        "issue",
    ];
    export
        .records
        .into_iter()
        .map(|record| {
            if record.id.trim().is_empty() || record.id.len() > 240 {
                return Err("Every evidence export record needs a bounded ID".to_string());
            }
            if looks_like_secret(&record.id) || contains_sensitive_path(&record.id) {
                return Err("Evidence export record ID contains credential-like data".to_string());
            }
            if !allowed.contains(&record.event_kind.as_str()) {
                return Err(format!(
                    "Unsupported local evidence event kind: {}",
                    record.event_kind
                ));
            }
            chrono::DateTime::parse_from_rfc3339(&record.observed_at)
                .map_err(|error| format!("Evidence observed_at must be RFC3339: {error}"))?;
            if let Some(effective_at) = record.effective_at.as_deref() {
                chrono::DateTime::parse_from_rfc3339(effective_at)
                    .map_err(|error| format!("Evidence effective_at must be RFC3339: {error}"))?;
            }
            let source_record_id = format!("{source}:{}", record.id);
            let summary_was_bounded = record.summary.chars().count() > 1_000;
            let (summary, summary_redacted) = redact_secret_text(&record.summary);
            let source_count = record.source_paths.len();
            let safe_sources = record
                .source_paths
                .into_iter()
                .filter(|path| !contains_sensitive_path(path) && !looks_like_secret(path))
                .take(50)
                .map(|path| GraphSourceAnchor {
                    path,
                    start_line: None,
                    start_column: None,
                    end_line: None,
                    end_column: None,
                    excerpt: None,
                })
                .collect::<Vec<_>>();
            let sources_redacted = safe_sources.len() < source_count.min(50);
            let safe_identifier =
                |value: &String| !looks_like_secret(value) && !contains_sensitive_path(value);
            Ok(HistoryEvidenceRecord {
                id: deterministic_evidence_id(
                    "provider-export",
                    &source_record_id,
                    record.effective_at.as_deref(),
                ),
                source_id: "provider-export".to_string(),
                source_record_id,
                source_cursor: safe_cursor.clone(),
                event_kind: record.event_kind,
                observed_at: record.observed_at,
                effective_at: record.effective_at,
                entity_candidates: record
                    .entity_ids
                    .into_iter()
                    .filter(safe_identifier)
                    .take(100)
                    .collect(),
                release_candidates: record
                    .release_ids
                    .into_iter()
                    .filter(safe_identifier)
                    .take(100)
                    .collect(),
                episode_keys: record
                    .episode_keys
                    .into_iter()
                    .filter(safe_identifier)
                    .take(100)
                    .collect(),
                trust: GraphTrust::Extracted,
                redacted: summary_was_bounded
                    || summary_redacted
                    || cursor_redacted
                    || sources_redacted
                    || source_count > 50,
                summary: bounded_summary(&summary, 1_000),
                sources: safe_sources,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|mut records| {
            records.sort_by(|left, right| left.id.cmp(&right.id));
            records.dedup_by(|left, right| left.id == right.id);
            records
        })
}

fn persist_imported_records(
    connection: &mut Connection,
    repo_path: &Path,
    records: &[HistoryEvidenceRecord],
    refreshed_at: &str,
) -> Result<HistoryEvidenceRefreshResult, String> {
    let canonical = repo_path.to_string_lossy().to_string();
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Start evidence import transaction: {error}"))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, ?2, 'pending', ?3, ?3)",
            params![
                canonical,
                stable_graph_id("repository", &canonical),
                refreshed_at
            ],
        )
        .map_err(|error| format!("Ensure evidence import repository: {error}"))?;
    let mut statement = transaction
        .prepare(
            "INSERT OR IGNORE INTO history_graph_events (
                id, repo_path, event_kind, entity_id, trust, origin, source_id,
                source_cursor, payload_json, evidence_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'metadata', ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(|error| format!("Prepare evidence import: {error}"))?;
    let mut imported = 0;
    let mut by_adapter = std::collections::BTreeMap::<String, usize>::new();
    for record in records {
        let primary_entity = record.entity_candidates.first();
        let changed = statement
            .execute(params![
                record.id,
                canonical,
                record.event_kind,
                primary_entity,
                record.trust.as_str(),
                record.source_id,
                record.source_cursor,
                serde_json::json!({
                    "source_record_id": record.source_record_id,
                    "effective_at": record.effective_at,
                    "entity_candidates": record.entity_candidates,
                    "release_candidates": record.release_candidates,
                    "episode_keys": record.episode_keys,
                    "summary": record.summary,
                    "redacted": record.redacted,
                })
                .to_string(),
                serde_json::to_string(&record.sources).map_err(|error| error.to_string())?,
                record.observed_at,
            ])
            .map_err(|error| format!("Persist imported evidence: {error}"))?;
        if changed > 0 {
            imported += 1;
            *by_adapter.entry(record.source_id.clone()).or_default() += 1;
        }
    }
    drop(statement);
    transaction
        .commit()
        .map_err(|error| format!("Commit evidence import: {error}"))?;
    Ok(HistoryEvidenceRefreshResult {
        repo_path: canonical,
        imported,
        already_present: records.len().saturating_sub(imported),
        adapters: by_adapter.into_iter().collect(),
        network_requests: 0,
        refreshed_at: refreshed_at.to_string(),
    })
}

pub(crate) fn refresh_builtin_adapters(
    connection: &mut Connection,
    repo_path: &Path,
) -> Result<HistoryEvidenceRefreshResult, String> {
    let canonical = repo_path.to_string_lossy().to_string();
    let refreshed_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    records.extend(collect_review_records(connection, &canonical)?);
    records.extend(collect_qa_records(connection, &canonical)?);
    records.extend(collect_session_records(connection, &canonical)?);
    records.extend(collect_decision_file_records(repo_path, &refreshed_at)?);
    records.sort_by(|left, right| left.id.cmp(&right.id));
    records.dedup_by(|left, right| left.id == right.id);

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Start evidence refresh transaction: {error}"))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, ?2, 'pending', ?3, ?3)",
            params![
                canonical,
                stable_graph_id("repository", &canonical),
                refreshed_at
            ],
        )
        .map_err(|error| format!("Ensure evidence repository: {error}"))?;
    let mut statement = transaction
        .prepare(
            "INSERT OR IGNORE INTO history_graph_events (
                id, repo_path, event_kind, trust, origin, source_id, source_cursor,
                payload_json, evidence_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, 'metadata', ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(|error| format!("Prepare normalized evidence insert: {error}"))?;
    let mut imported = 0;
    let mut by_adapter = std::collections::BTreeMap::<String, usize>::new();
    for record in &records {
        let changed = statement
            .execute(params![
                record.id,
                canonical,
                record.event_kind,
                record.trust.as_str(),
                record.source_id,
                record.source_cursor,
                serde_json::json!({
                    "source_record_id": record.source_record_id,
                    "effective_at": record.effective_at,
                    "entity_candidates": record.entity_candidates,
                    "release_candidates": record.release_candidates,
                    "episode_keys": record.episode_keys,
                    "summary": record.summary,
                    "redacted": record.redacted,
                })
                .to_string(),
                serde_json::to_string(&record.sources).map_err(|error| error.to_string())?,
                record.observed_at,
            ])
            .map_err(|error| format!("Persist normalized evidence: {error}"))?;
        if changed > 0 {
            imported += 1;
            *by_adapter.entry(record.source_id.clone()).or_default() += 1;
        }
    }
    drop(statement);
    transaction
        .commit()
        .map_err(|error| format!("Commit history evidence refresh: {error}"))?;
    Ok(HistoryEvidenceRefreshResult {
        repo_path: canonical,
        imported,
        already_present: records.len().saturating_sub(imported),
        adapters: by_adapter.into_iter().collect(),
        network_requests: 0,
        refreshed_at,
    })
}

fn collect_review_records(
    connection: &Connection,
    repo_path: &str,
) -> Result<Vec<HistoryEvidenceRecord>, String> {
    let mut records = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT id, COALESCE(completed_at, started_at, created_at), status,
                    COALESCE(summary_markdown, ''), pr_number
             FROM local_reviews WHERE repo_path = ?1
             ORDER BY created_at DESC, id LIMIT 500",
        )
        .map_err(|error| format!("Prepare local review adapter: {error}"))?;
    let rows = statement
        .query_map(params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|error| format!("Query local review adapter: {error}"))?;
    for row in rows {
        let (id, observed_at, status, summary, pr_number) =
            row.map_err(|error| format!("Read local review adapter: {error}"))?;
        records.push(HistoryEvidenceRecord {
            id: deterministic_evidence_id("reviews", &id, Some(&observed_at)),
            source_id: "reviews".to_string(),
            source_record_id: id.clone(),
            source_cursor: Some(format!("{observed_at}:{id}")),
            event_kind: if pr_number.is_some() {
                "pull_request_review"
            } else {
                "review"
            }
            .to_string(),
            observed_at,
            effective_at: None,
            entity_candidates: Vec::new(),
            release_candidates: Vec::new(),
            episode_keys: std::iter::once(format!("review:{id}"))
                .chain(pr_number.map(|number| format!("pr:{number}")))
                .collect(),
            trust: GraphTrust::Extracted,
            summary: bounded_summary(&format!("{status}: {summary}"), 1_000),
            sources: Vec::new(),
            redacted: summary.len() > 1_000,
        });
    }
    drop(statement);

    let mut procedure_statement = connection
        .prepare(
            "SELECT e.id, e.created_at, e.status, e.step_id, e.summary, e.artifact,
                    e.review_id
             FROM review_procedure_events e
             JOIN local_reviews r ON r.id = e.review_id
             WHERE r.repo_path = ?1 ORDER BY e.created_at DESC, e.id LIMIT 500",
        )
        .map_err(|error| format!("Prepare review procedure adapter: {error}"))?;
    let rows = procedure_statement
        .query_map(params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| format!("Query review procedure adapter: {error}"))?;
    for row in rows {
        let (id, observed_at, status, step, summary, artifact, review_id) =
            row.map_err(|error| format!("Read review procedure adapter: {error}"))?;
        records.push(HistoryEvidenceRecord {
            id: deterministic_evidence_id("reviews", &id, Some(&observed_at)),
            source_id: "reviews".to_string(),
            source_record_id: id.clone(),
            source_cursor: Some(format!("{observed_at}:{id}")),
            event_kind: "verification_attempt".to_string(),
            observed_at,
            effective_at: None,
            entity_candidates: Vec::new(),
            release_candidates: Vec::new(),
            episode_keys: vec![format!("review:{review_id}")],
            trust: GraphTrust::Extracted,
            summary: bounded_summary(&format!("{step} {status}: {summary}"), 1_000),
            sources: artifact
                .map(|path| GraphSourceAnchor {
                    path,
                    start_line: None,
                    start_column: None,
                    end_line: None,
                    end_column: None,
                    excerpt: None,
                })
                .into_iter()
                .collect(),
            redacted: summary.len() > 1_000,
        });
    }
    Ok(records)
}

fn collect_qa_records(
    connection: &Connection,
    repo_path: &str,
) -> Result<Vec<HistoryEvidenceRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, created_at, pass, runner_type, COALESCE(goal, ''),
                    COALESCE(notes, ''), screenshot_path, review_id, loop_id
             FROM synthetic_qa_runs WHERE repo_path = ?1
             ORDER BY created_at DESC, id LIMIT 500",
        )
        .map_err(|error| format!("Prepare synthetic QA adapter: {error}"))?;
    let rows = statement
        .query_map(params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|error| format!("Query synthetic QA adapter: {error}"))?;
    rows.map(|row| {
        let (id, observed_at, passed, runner, goal, notes, screenshot, review_id, loop_id) =
            row.map_err(|error| format!("Read synthetic QA adapter: {error}"))?;
        Ok(HistoryEvidenceRecord {
            id: deterministic_evidence_id("synthetic-qa", &id, Some(&observed_at)),
            source_id: "synthetic-qa".to_string(),
            source_record_id: id.clone(),
            source_cursor: Some(format!("{observed_at}:{id}")),
            event_kind: "synthetic_qa".to_string(),
            observed_at,
            effective_at: None,
            entity_candidates: Vec::new(),
            release_candidates: Vec::new(),
            episode_keys: review_id
                .map(|id| format!("review:{id}"))
                .into_iter()
                .chain(std::iter::once(format!("qa-loop:{loop_id}")))
                .collect(),
            trust: GraphTrust::Extracted,
            summary: bounded_summary(
                &format!(
                    "{runner} {}: {goal}. {notes}",
                    if passed { "passed" } else { "failed" }
                ),
                1_000,
            ),
            sources: screenshot
                .map(|path| GraphSourceAnchor {
                    path,
                    start_line: None,
                    start_column: None,
                    end_line: None,
                    end_column: None,
                    excerpt: None,
                })
                .into_iter()
                .collect(),
            redacted: notes.len() > 1_000,
        })
    })
    .collect()
}

fn collect_session_records(
    connection: &Connection,
    repo_path: &str,
) -> Result<Vec<HistoryEvidenceRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT s.id, COALESCE(s.indexed_at, s.last_message, ''), s.agent_type,
                    s.message_count, s.git_branch
             FROM cc_sessions s JOIN cc_projects p ON p.id = s.project_id
             WHERE p.dir_path = ?1 OR s.cwd = ?1
             ORDER BY COALESCE(s.last_message, s.indexed_at) DESC, s.id LIMIT 500",
        )
        .map_err(|error| format!("Prepare local session adapter: {error}"))?;
    let rows = statement
        .query_map(params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|error| format!("Query local session adapter: {error}"))?;
    rows.map(|row| {
        let (id, mut observed_at, agent, message_count, branch) =
            row.map_err(|error| format!("Read local session adapter: {error}"))?;
        if observed_at.is_empty() {
            observed_at = Utc::now().to_rfc3339();
        }
        Ok(HistoryEvidenceRecord {
            id: deterministic_evidence_id("agent-sessions", &id, Some(&observed_at)),
            source_id: "agent-sessions".to_string(),
            source_record_id: id.clone(),
            source_cursor: Some(format!("{observed_at}:{id}")),
            event_kind: "agent_session".to_string(),
            observed_at,
            effective_at: None,
            entity_candidates: Vec::new(),
            release_candidates: branch.into_iter().collect(),
            episode_keys: vec![format!("session:{id}")],
            trust: GraphTrust::Extracted,
            summary: format!("{agent} session metadata: {message_count} indexed messages"),
            sources: Vec::new(),
            redacted: true,
        })
    })
    .collect()
}

fn collect_decision_file_records(
    repo_path: &Path,
    observed_at: &str,
) -> Result<Vec<HistoryEvidenceRecord>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["ls-files", "-s", "-z"])
        .output()
        .map_err(|error| format!("Read decision-file index: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Read decision-file index: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut records = Vec::new();
    for entry in output.stdout.split(|byte| *byte == 0) {
        let entry = String::from_utf8_lossy(entry);
        let Some((metadata, path)) = entry.split_once('\t') else {
            continue;
        };
        let lower = path.to_ascii_lowercase();
        if !(lower.contains("changelog")
            || lower.contains("/adr")
            || lower.starts_with("adr")
            || lower.contains("decision")
            || lower.starts_with(".planning/"))
        {
            continue;
        }
        let object_id = metadata.split_whitespace().nth(1).unwrap_or_default();
        let source_record_id = format!("{path}:{object_id}");
        records.push(HistoryEvidenceRecord {
            id: deterministic_evidence_id("decision-files", &source_record_id, None),
            source_id: "decision-files".to_string(),
            source_record_id: source_record_id.clone(),
            source_cursor: Some(source_record_id),
            event_kind: "decision_marker".to_string(),
            observed_at: observed_at.to_string(),
            effective_at: None,
            entity_candidates: Vec::new(),
            release_candidates: Vec::new(),
            episode_keys: vec![format!("decision-file:{path}")],
            trust: GraphTrust::Extracted,
            summary: format!("Tracked decision-bearing file: {path}"),
            sources: vec![GraphSourceAnchor {
                path: path.to_string(),
                start_line: None,
                start_column: None,
                end_line: None,
                end_column: None,
                excerpt: None,
            }],
            redacted: true,
        });
        if records.len() >= 500 {
            break;
        }
    }
    Ok(records)
}

fn bounded_summary(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn adapter_descriptors(
    connection: &Connection,
    repo_path: &Path,
) -> Result<Vec<HistoryEvidenceAdapterDescriptor>, String> {
    let canonical = repo_path.to_string_lossy();
    let definitions = [
        (
            "git",
            "Git commits, tags, and releases",
            "git",
            HistoryAdapterConsent::LocalDefault,
            vec!["Git object database"],
        ),
        (
            "decision-files",
            "Changelogs, ADRs, and decision markers",
            "local_files",
            HistoryAdapterConsent::LocalDefault,
            vec!["tracked repository paths and bounded source anchors"],
        ),
        (
            "agent-sessions",
            "Indexed local agent sessions",
            "sqlite",
            HistoryAdapterConsent::LocalDefault,
            vec!["cc_projects and cc_sessions metadata"],
        ),
        (
            "reviews",
            "Reviews and fix attempts",
            "sqlite",
            HistoryAdapterConsent::LocalDefault,
            vec!["local_reviews, findings, and procedure events"],
        ),
        (
            "synthetic-qa",
            "Synthetic QA runs",
            "sqlite",
            HistoryAdapterConsent::LocalDefault,
            vec!["synthetic_qa_runs metadata and artifact paths"],
        ),
        (
            "tasks",
            "Local tasks and follow-ups",
            "sqlite",
            HistoryAdapterConsent::LocalDefault,
            vec!["agent_tasks metadata"],
        ),
        (
            "provider-export",
            "Analytics, logs, incidents, deploys, and PR exports",
            "explicit_import",
            HistoryAdapterConsent::ExplicitImport,
            vec!["only a user-selected local export"],
        ),
        (
            "hosted-provider",
            "Configured hosted provider",
            "external_provider",
            HistoryAdapterConsent::ExplicitImport,
            vec!["nothing until a separate adapter is configured"],
        ),
    ];
    let mut descriptors = Vec::with_capacity(definitions.len());
    for (id, label, source_kind, consent, reads) in definitions {
        let (count, cursor, observed_at) = local_adapter_state(connection, &canonical, id)?;
        let configured = consent == HistoryAdapterConsent::LocalDefault || count > 0;
        let availability = if consent == HistoryAdapterConsent::ExplicitImport && !configured {
            HistoryAdapterAvailability::NeedsConfiguration
        } else if (id == "git" && repo_path.join(".git").exists()) || count > 0 {
            HistoryAdapterAvailability::Available
        } else {
            HistoryAdapterAvailability::Empty
        };
        descriptors.push(HistoryEvidenceAdapterDescriptor {
            id: id.to_string(),
            label: label.to_string(),
            source_kind: source_kind.to_string(),
            availability,
            consent,
            configured,
            local_only: true,
            network_access: false,
            reads: reads.into_iter().map(str::to_string).collect(),
            redaction: "Store normalized bounded metadata and source anchors; omit credentials and unrestricted raw payloads".to_string(),
            source_cursor: cursor,
            last_observed_at: observed_at,
            freshness: if count > 0 {
                format!("{count} normalized local records")
            } else {
                "No normalized records imported".to_string()
            },
        });
    }
    Ok(descriptors)
}

fn local_adapter_state(
    connection: &Connection,
    repo_path: &str,
    adapter_id: &str,
) -> Result<(usize, Option<String>, Option<String>), String> {
    connection
        .query_row(
            "SELECT COUNT(*), MAX(source_cursor), MAX(recorded_at)
             FROM history_graph_events WHERE repo_path = ?1 AND source_id = ?2",
            params![repo_path, adapter_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load adapter state: {error}"))
        .map(|row| row.unwrap_or((0, None, None)))
}

fn canonical_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(repo_path.trim())
        .canonicalize()
        .map_err(|error| format!("Cannot resolve repository path: {error}"))?;
    if !path.is_dir() {
        return Err("Repository path is not a directory".to_string());
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
