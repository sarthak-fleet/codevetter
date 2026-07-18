use super::types::*;
use crate::commands::history_graph::{canonical_repo_path, git_text, resolve_revision};
use crate::commands::structural_graph::types::{stable_graph_id, GraphSourceAnchor, GraphTrust};
use crate::DbState;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

const MAX_EVENT_SCAN: usize = 5_000;
const DEFAULT_TRACE_LIMIT: usize = 120;
const MAX_TRACE_LIMIT: usize = 500;

#[derive(Debug, Clone)]
struct StoredHistoryEvent {
    event: HistoryCausalEvent,
    payload: Value,
    explicit_refs: Vec<String>,
}

pub(crate) fn build_review_history_slice(
    connection: &Connection,
    repo_path: &str,
    changed_files: &[String],
) -> Result<HistoryReviewSlice, String> {
    let repo_root = canonical_repo_path(repo_path)?;
    let canonical = repo_root.to_string_lossy().to_string();
    let current_head = git_text(&repo_root, &["rev-parse", "HEAD"])?;
    let (indexed_head, coverage) = connection
        .query_row(
            "SELECT indexed_head, coverage_json FROM history_graph_repositories
             WHERE repo_path = ?1",
            params![canonical],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load review history freshness: {error}"))?
        .map(|(head, coverage)| {
            (
                head.unwrap_or_default(),
                serde_json::from_str(&coverage).unwrap_or_else(|_| serde_json::json!({})),
            )
        })
        .unwrap_or_else(|| (String::new(), serde_json::json!({})));
    let mut files = changed_files
        .iter()
        .map(|path| path.trim().replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .take(100)
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    if indexed_head.is_empty() {
        return Ok(HistoryReviewSlice {
            schema_version: 1,
            repo_path: canonical,
            files,
            entity_ids: Vec::new(),
            episodes: Vec::new(),
            constraints: Vec::new(),
            verification: Vec::new(),
            failures: Vec::new(),
            regressions: Vec::new(),
            qualified_leads: Vec::new(),
            gaps: vec!["Temporal graph is not indexed for this repository".to_string()],
            indexed_head,
            stale: true,
            coverage,
            truncated: false,
        });
    }

    let entity_ids = review_entity_ids(connection, &canonical, &indexed_head, &files, 100)?;
    let revision_ids = review_revision_ids(connection, &canonical, &files, 120)?;
    let (events, scan_truncated) = load_event_pool(connection, &canonical, &repo_root, None)?;
    let entity_set = entity_ids.iter().cloned().collect::<HashSet<_>>();
    let file_set = files.iter().cloned().collect::<HashSet<_>>();
    let seed_ids = events
        .iter()
        .filter(|event| review_event_matches(event, &entity_set, &revision_ids, &file_set))
        .map(|event| event.event.id.clone())
        .take(30)
        .collect::<Vec<_>>();
    let mut components = BTreeMap::<String, HistoryChangeEpisode>::new();
    for event_id in seed_ids {
        let (episodes, _) =
            assemble_episodes(&events, &HistoryCausalSelector::Event { event_id }, 80);
        for episode in episodes {
            let mut event_ids = episode
                .events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>();
            event_ids.sort();
            components.entry(event_ids.join("\0")).or_insert(episode);
        }
    }
    let mut episodes = components.into_values().collect::<Vec<_>>();
    episodes.sort_by(|left, right| {
        right
            .ended_at
            .cmp(&left.ended_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let episode_truncated = episodes.len() > 6 || episodes.iter().any(|episode| episode.truncated);
    episodes.truncate(6);

    let mut all_events = episodes
        .iter()
        .flat_map(|episode| episode.events.iter().cloned())
        .collect::<Vec<_>>();
    all_events.sort_by(|left, right| {
        event_time(right)
            .cmp(event_time(left))
            .then_with(|| left.id.cmp(&right.id))
    });
    all_events.dedup_by(|left, right| left.id == right.id);
    let constraints = take_review_events(&all_events, 12, |event| {
        matches!(
            event.stage,
            HistoryCausalStage::Intent | HistoryCausalStage::FollowUp
        )
    });
    let verification = take_review_events(&all_events, 12, |event| {
        event.stage == HistoryCausalStage::Verification
    });
    let regressions = take_review_events(&all_events, 12, |event| {
        event.stage == HistoryCausalStage::Regression
    });
    let failures = take_review_events(&all_events, 12, |event| {
        let summary = event.summary.to_ascii_lowercase();
        event.stage == HistoryCausalStage::Regression
            || summary.contains("failed")
            || summary.contains("failure")
            || summary.contains("error")
            || summary.contains("reject")
    });
    let mut qualified_leads = episodes
        .iter()
        .flat_map(|episode| episode.qualified_lead_events.iter().cloned())
        .collect::<Vec<_>>();
    qualified_leads.sort_by(|left, right| {
        event_time(right)
            .cmp(event_time(left))
            .then_with(|| left.id.cmp(&right.id))
    });
    qualified_leads.dedup_by(|left, right| left.id == right.id);
    qualified_leads.truncate(12);
    let mut gaps = episodes
        .iter()
        .flat_map(|episode| episode.gaps.iter().cloned())
        .collect::<Vec<_>>();
    gaps.sort();
    gaps.dedup();
    if entity_ids.is_empty() {
        gaps.push("No indexed structural entities map to the changed files".to_string());
    }
    if episodes.is_empty() {
        gaps.push("No explicit temporal episodes map to the changed files".to_string());
    }
    if scan_truncated {
        gaps.push(format!(
            "Review history scanned only the newest {MAX_EVENT_SCAN} ledger events"
        ));
    }

    Ok(HistoryReviewSlice {
        schema_version: 1,
        repo_path: canonical,
        files,
        entity_ids,
        episodes,
        constraints,
        verification,
        failures,
        regressions,
        qualified_leads,
        gaps,
        stale: indexed_head != current_head,
        indexed_head,
        coverage,
        truncated: scan_truncated || episode_truncated,
    })
}

pub(crate) fn render_review_history_slice(slice: &HistoryReviewSlice) -> String {
    if slice.episodes.is_empty() && slice.constraints.is_empty() && slice.verification.is_empty() {
        return String::new();
    }
    const MAX_BYTES: usize = 3_500;
    let mut output = String::from(
        "\nTemporal history graph for changed files (cited context; inferred/qualified leads are not findings):\n",
    );
    for event in slice
        .constraints
        .iter()
        .chain(slice.failures.iter())
        .chain(slice.verification.iter())
        .take(12)
    {
        let source = event
            .sources
            .first()
            .map(|source| format!(" source={}", source.path))
            .unwrap_or_default();
        let line = format!(
            "- [{}|{}] {}{} event={}\n",
            stage_label(&event.stage),
            event.trust.as_str(),
            event.summary.replace('\n', " "),
            source,
            event.id
        );
        if output.len() + line.len() > MAX_BYTES {
            break;
        }
        output.push_str(&line);
    }
    if !slice.gaps.is_empty() && output.len() < MAX_BYTES {
        let line = format!(
            "- Evidence gaps: {}\n",
            slice
                .gaps
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
        output.push_str(
            &line
                .chars()
                .take(MAX_BYTES - output.len())
                .collect::<String>(),
        );
    }
    output
}

#[tauri::command]
pub async fn get_history_causal_trace(
    repo_path: String,
    selector: HistoryCausalSelector,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryCausalTrace, String> {
    let root = canonical_repo_path(&repo_path)?;
    let selector = resolve_selector(&root, selector)?;
    let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
    let limit = limit
        .unwrap_or(DEFAULT_TRACE_LIMIT)
        .clamp(1, MAX_TRACE_LIMIT);
    let cursor = cursor.as_deref().map(decode_cursor).transpose()?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        query_causal_trace(&connection, &root, &current_head, selector, limit, cursor)
    })
    .await
    .map_err(|error| format!("History causal query worker failed: {error}"))?
}

pub(crate) fn query_causal_trace(
    connection: &Connection,
    repo_root: &Path,
    current_head: &str,
    selector: HistoryCausalSelector,
    limit: usize,
    cursor: Option<(String, String)>,
) -> Result<HistoryCausalTrace, String> {
    let repo_path = repo_root.to_string_lossy().to_string();
    let total_events = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_events WHERE repo_path = ?1",
            params![repo_path],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Count history events: {error}"))? as usize;
    let (events, scan_truncated) =
        load_event_pool(connection, &repo_path, repo_root, cursor.as_ref())?;
    let scanned_events = events.len();
    let (mut episodes, mut gaps) = assemble_episodes(&events, &selector, limit);
    let response_truncated = episodes.iter().any(|episode| episode.truncated) || scan_truncated;
    if scan_truncated {
        gaps.push(format!(
            "Causal assembly scanned the newest {scanned_events} of {total_events} ledger events"
        ));
    }
    let next_cursor = scan_truncated
        .then(|| events.last())
        .flatten()
        .map(|event| encode_cursor(&event.event.recorded_at, &event.event.id))
        .transpose()?;
    let (indexed_head, coverage) = connection
        .query_row(
            "SELECT indexed_head, coverage_json FROM history_graph_repositories
             WHERE repo_path = ?1",
            params![repo_path],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load causal-query freshness: {error}"))?
        .map(|(head, coverage)| {
            (
                head.unwrap_or_default(),
                serde_json::from_str(&coverage).unwrap_or_else(|_| serde_json::json!({})),
            )
        })
        .unwrap_or_else(|| (String::new(), serde_json::json!({})));
    episodes.sort_by(|left, right| {
        right
            .ended_at
            .cmp(&left.ended_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(HistoryCausalTrace {
        schema_version: 1,
        repo_path,
        selector,
        episodes,
        stale: indexed_head.is_empty() || indexed_head != current_head,
        indexed_head,
        coverage,
        gaps,
        scanned_events,
        total_events,
        truncated: response_truncated,
        next_cursor,
    })
}

fn load_event_pool(
    connection: &Connection,
    repo_path: &str,
    repo_root: &Path,
    cursor: Option<&(String, String)>,
) -> Result<(Vec<StoredHistoryEvent>, bool), String> {
    let (cursor_time, cursor_id) = cursor
        .cloned()
        .map(|(time, id)| (Some(time), Some(id)))
        .unwrap_or_default();
    let mut statement = connection
        .prepare(
            "SELECT id, revision_sha, event_kind, entity_id, related_entity_id,
                    relation_kind, trust, origin, source_id, source_cursor, payload_json,
                    evidence_json, recorded_at
             FROM history_graph_events
             WHERE repo_path = ?1
               AND (?2 IS NULL OR recorded_at < ?2 OR (recorded_at = ?2 AND id < ?3))
             ORDER BY recorded_at DESC, id DESC LIMIT ?4",
        )
        .map_err(|error| format!("Prepare causal event scan: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repo_path,
                cursor_time,
                cursor_id,
                (MAX_EVENT_SCAN + 1) as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .map_err(|error| format!("Scan causal events: {error}"))?;
    let rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read causal event: {error}"))?;
    let scan_truncated = rows.len() > MAX_EVENT_SCAN;
    let mut events = Vec::with_capacity(rows.len().min(MAX_EVENT_SCAN));
    for row in rows.into_iter().take(MAX_EVENT_SCAN) {
        let (
            id,
            revision_sha,
            event_kind,
            entity_id,
            related_entity_id,
            relation_kind,
            trust,
            origin,
            source_id,
            source_cursor,
            payload_json,
            evidence_json,
            recorded_at,
        ) = row;
        let payload: Value =
            serde_json::from_str(&payload_json).unwrap_or_else(|_| serde_json::json!({}));
        let sources: Vec<GraphSourceAnchor> =
            serde_json::from_str(&evidence_json).unwrap_or_default();
        let episode_keys = string_array(&payload, "episode_keys");
        let explicit_refs = payload
            .get("related_event_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .into_iter()
            .collect();
        let effective_at = payload
            .get("effective_at")
            .and_then(Value::as_str)
            .map(str::to_string);
        let summary = event_summary(&payload, &event_kind);
        let source_available = sources
            .iter()
            .all(|source| resolve_source_path(repo_root, &source.path).exists());
        events.push(StoredHistoryEvent {
            event: HistoryCausalEvent {
                id,
                revision_sha,
                event_kind: event_kind.clone(),
                stage: classify_stage(&event_kind),
                summary,
                trust: GraphTrust::from_storage(&trust),
                origin,
                source_id,
                source_cursor,
                recorded_at,
                effective_at,
                entity_id,
                related_entity_id,
                relation_kind,
                episode_keys,
                sources,
                source_available,
            },
            payload,
            explicit_refs,
        });
    }
    Ok((events, scan_truncated))
}

fn review_entity_ids(
    connection: &Connection,
    repo_path: &str,
    revision: &str,
    files: &[String],
    limit: usize,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT n.id
             FROM history_graph_checkpoints c
             JOIN structural_graph_nodes n ON n.snapshot_id = c.snapshot_id
             WHERE c.repo_path = ?1 AND c.revision_sha = ?2 AND c.status = 'ready'
               AND n.path = ?3
             ORDER BY n.kind, n.label, n.id LIMIT ?4",
        )
        .map_err(|error| format!("Prepare review entity lookup: {error}"))?;
    let mut entity_ids = BTreeSet::new();
    for file in files {
        let remaining = limit.saturating_sub(entity_ids.len());
        if remaining == 0 {
            break;
        }
        let rows = statement
            .query_map(
                params![repo_path, revision, file, remaining as i64],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("Query review entities: {error}"))?;
        for entity_id in rows {
            entity_ids.insert(entity_id.map_err(|error| format!("Read review entity: {error}"))?);
        }
    }
    Ok(entity_ids.into_iter().collect())
}

fn review_revision_ids(
    connection: &Connection,
    repo_path: &str,
    files: &[String],
    limit: usize,
) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT p.revision_sha
             FROM history_graph_revision_paths p
             JOIN history_graph_revisions r
               ON r.repo_path = p.repo_path AND r.sha = p.revision_sha
             WHERE p.repo_path = ?1 AND (p.path = ?2 OR p.old_path = ?2)
             ORDER BY r.ordinal DESC LIMIT ?3",
        )
        .map_err(|error| format!("Prepare review revision lookup: {error}"))?;
    let mut revisions = HashSet::new();
    for file in files {
        let remaining = limit.saturating_sub(revisions.len());
        if remaining == 0 {
            break;
        }
        let rows = statement
            .query_map(params![repo_path, file, remaining as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("Query review revisions: {error}"))?;
        for revision in rows {
            revisions.insert(revision.map_err(|error| format!("Read review revision: {error}"))?);
        }
    }
    Ok(revisions)
}

fn review_event_matches(
    event: &StoredHistoryEvent,
    entity_ids: &HashSet<String>,
    revision_ids: &HashSet<String>,
    files: &HashSet<String>,
) -> bool {
    if event
        .event
        .revision_sha
        .as_ref()
        .is_some_and(|revision| revision_ids.contains(revision))
    {
        return true;
    }
    if event_entities(event)
        .iter()
        .any(|entity_id| entity_ids.contains(entity_id))
        || entity_ids
            .iter()
            .any(|entity_id| payload_mentions_entity(&event.payload, entity_id))
    {
        return true;
    }
    event.event.sources.iter().any(|source| {
        files
            .iter()
            .any(|file| history_path_matches(&source.path, file))
    }) || files
        .iter()
        .any(|file| payload_mentions_path(&event.payload, file))
}

fn payload_mentions_path(payload: &Value, file: &str) -> bool {
    if ["path", "old_path"]
        .iter()
        .any(|key| payload.get(*key).and_then(Value::as_str) == Some(file))
        || ["changed_paths", "source_paths"]
            .iter()
            .any(|key| string_array(payload, key).iter().any(|path| path == file))
    {
        return true;
    }
    payload
        .get("path_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|change| {
            ["path", "old_path"]
                .iter()
                .any(|key| change.get(*key).and_then(Value::as_str) == Some(file))
        })
}

fn history_path_matches(source_path: &str, file: &str) -> bool {
    let source_path = source_path.replace('\\', "/");
    let file = file.trim_start_matches("./");
    source_path.trim_start_matches("./") == file || source_path.ends_with(&format!("/{file}"))
}

fn take_review_events(
    events: &[HistoryCausalEvent],
    limit: usize,
    predicate: impl Fn(&HistoryCausalEvent) -> bool,
) -> Vec<HistoryCausalEvent> {
    events
        .iter()
        .filter(|event| predicate(event))
        .take(limit)
        .cloned()
        .collect()
}

fn stage_label(stage: &HistoryCausalStage) -> &'static str {
    match stage {
        HistoryCausalStage::Intent => "intent",
        HistoryCausalStage::Implementation => "implementation",
        HistoryCausalStage::Verification => "verification",
        HistoryCausalStage::Release => "release",
        HistoryCausalStage::Outcome => "outcome",
        HistoryCausalStage::Regression => "regression",
        HistoryCausalStage::FollowUp => "follow-up",
        HistoryCausalStage::Context => "context",
    }
}

mod causal;

#[cfg(test)]
mod tests;

use causal::*;
