use super::super::inflections::{
    derive_history_inflections, enrich_candidate_with_structural_delta, CoverageStatus,
    HistoryInflectionFact, InflectionCandidate, StructuralChangeMeasurements, ALGORITHM,
    ALGORITHM_VERSION, MAX_REVISIONS,
};
use super::*;
use rusqlite::{OptionalExtension, Transaction};

pub(super) const HISTORY_LANDMARK_SCHEMA_VERSION: i64 = 1;
pub(super) const MAX_PUBLISHED_INFLECTIONS: usize = 512;
const MAX_STRUCTURAL_SUMMARY_BYTES: i64 = 2 * 1024 * 1024;

pub(in crate::commands::history_graph) fn publish_candidate_inflections(
    transaction: &Transaction<'_>,
    repo_path: &str,
    index_identity: &str,
    history_coverage_complete: bool,
    updated_at: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<String, String> {
    publish_candidate_inflections_inner(
        transaction,
        repo_path,
        index_identity,
        history_coverage_complete,
        updated_at,
        cancellation,
        false,
    )
}

fn publish_candidate_inflections_inner(
    transaction: &Transaction<'_>,
    repo_path: &str,
    index_identity: &str,
    history_coverage_complete: bool,
    updated_at: &str,
    cancellation: &StructuralGraphCancellation,
    fail_after_replace: bool,
) -> Result<String, String> {
    ensure_active(cancellation)?;
    let facts = load_inflection_facts(transaction, repo_path)?;
    let mut derivation = derive_history_inflections(&facts);
    let candidate_total = derivation.candidates.len();
    derivation.candidates.truncate(MAX_PUBLISHED_INFLECTIONS);
    let storage_truncated = candidate_total > derivation.candidates.len();
    let generation_id = stable_graph_id(
        "history-landmark-generation-v1",
        &format!("{repo_path}\0{index_identity}\0{ALGORITHM}\0{ALGORITHM_VERSION}"),
    );

    let mut structural_available = 0_usize;
    let mut structural_partial = 0_usize;
    let mut structural_unavailable = 0_usize;
    for candidate in &mut derivation.candidates {
        ensure_active(cancellation)?;
        // The derivation kernel is repository-agnostic. Persistence owns the
        // repository-scoped opaque identity so identical SHAs in forks differ.
        candidate.id = stable_graph_id(
            "history-landmark-v1",
            &format!(
                "{repo_path}\0{ALGORITHM}\0{ALGORITHM_VERSION}\0{}",
                candidate.revision_sha
            ),
        );
        match load_structural_measurements(transaction, repo_path, &candidate.revision_sha)? {
            StructuralLoad::Available(measurements) => {
                structural_available += 1;
                structural_partial += usize::from(measurements.coverage_gap.is_some());
                enrich_candidate_with_structural_delta(candidate, measurements);
            }
            StructuralLoad::Missing => {
                structural_unavailable += 1;
                candidate.caveats.push(
                    "No persisted structural delta is available; this candidate uses churn and file facts only."
                        .to_string(),
                );
            }
            StructuralLoad::Bounded => {
                structural_partial += 1;
                candidate.caveats.push(format!(
                    "The persisted structural summary exceeds the {MAX_STRUCTURAL_SUMMARY_BYTES}-byte enrichment bound."
                ));
            }
            StructuralLoad::Invalid => {
                structural_partial += 1;
                candidate.caveats.push(
                    "The persisted structural summary is unreadable; no structural measurements were inferred."
                        .to_string(),
                );
            }
        }
        if !history_coverage_complete {
            candidate.caveats.push(
                "Indexed repository history is partial; the baseline does not represent omitted revisions."
                    .to_string(),
            );
        }
    }

    ensure_active(cancellation)?;
    transaction
        .execute(
            "DELETE FROM history_graph_landmarks WHERE repo_path = ?1",
            [repo_path],
        )
        .map_err(|error| format!("Replace history landmarks: {error}"))?;
    if fail_after_replace {
        return Err("Forced landmark publication failure".to_string());
    }

    let mut statement = transaction
        .prepare(
            "INSERT INTO history_graph_landmarks (
                repo_path, generation_id, id, revision_sha, ordinal, kind, label,
                trust, score_milli, components_json, reasons_json, caveats_json,
                coverage_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'candidate_inflection', ?6, ?7,
                ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(|error| format!("Prepare candidate inflections: {error}"))?;
    for candidate in &derivation.candidates {
        ensure_active(cancellation)?;
        let structural_status = structural_status(candidate);
        let partial = derivation.coverage.status != CoverageStatus::Complete
            || structural_status != "complete"
            || !history_coverage_complete;
        statement
            .execute(params![
                repo_path,
                generation_id,
                candidate.id,
                candidate.revision_sha,
                candidate.ordinal,
                format!(
                    "Candidate inflection · {}",
                    candidate.revision_sha.chars().take(8).collect::<String>()
                ),
                if partial {
                    "qualified_partial"
                } else {
                    "qualified"
                },
                candidate.aggregate_score_milli,
                serde_json::json!({
                    "algorithm": ALGORITHM,
                    "algorithm_version": ALGORITHM_VERSION,
                    "churn": candidate.churn,
                    "changed_files": candidate.changed_files,
                    "aggregate_score_milli": candidate.aggregate_score_milli,
                    "noise_weight_milli": candidate.noise_weight_milli,
                    "binary_files": candidate.binary_files,
                    "generated_files": candidate.generated_files,
                    "vendored_files": candidate.vendored_files,
                    "release_only": candidate.release_only,
                    "merge": candidate.merge,
                    "structural": candidate.structural,
                })
                .to_string(),
                serde_json::to_string(&candidate.reasons).map_err(|error| error.to_string())?,
                serde_json::to_string(&candidate.caveats).map_err(|error| error.to_string())?,
                serde_json::json!({
                    "fact_coverage": derivation.coverage.status,
                    "structural_coverage": structural_status,
                    "non_causal": true,
                })
                .to_string(),
            ])
            .map_err(|error| format!("Persist candidate inflection: {error}"))?;
    }
    drop(statement);
    ensure_active(cancellation)?;

    let status = if derivation.coverage.status == CoverageStatus::Unavailable {
        "unavailable"
    } else if derivation.coverage.status == CoverageStatus::Partial
        || storage_truncated
        || structural_partial > 0
        || structural_unavailable > 0
        || !history_coverage_complete
    {
        "partial"
    } else {
        "ready"
    };
    transaction
        .execute(
            "INSERT INTO history_graph_landmark_generations (
                repo_path, schema_version, algorithm, algorithm_version,
                generation_id, index_identity, status, landmark_count,
                coverage_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(repo_path) DO UPDATE SET
                schema_version = excluded.schema_version,
                algorithm = excluded.algorithm,
                algorithm_version = excluded.algorithm_version,
                generation_id = excluded.generation_id,
                index_identity = excluded.index_identity,
                status = excluded.status,
                landmark_count = excluded.landmark_count,
                coverage_json = excluded.coverage_json,
                updated_at = excluded.updated_at",
            params![
                repo_path,
                HISTORY_LANDMARK_SCHEMA_VERSION,
                ALGORITHM,
                ALGORITHM_VERSION,
                generation_id,
                index_identity,
                status,
                derivation.candidates.len(),
                serde_json::json!({
                    "detector": derivation.coverage,
                    "baseline": derivation.baseline,
                    "thresholds": derivation.thresholds,
                    "candidate_total": candidate_total,
                    "published_limit": MAX_PUBLISHED_INFLECTIONS,
                    "storage_truncated": storage_truncated,
                    "history_coverage_complete": history_coverage_complete,
                    "structural_available": structural_available,
                    "structural_partial": structural_partial,
                    "structural_unavailable": structural_unavailable,
                    "non_causal": true,
                })
                .to_string(),
                updated_at,
            ],
        )
        .map_err(|error| format!("Publish candidate-inflection generation: {error}"))?;
    Ok(generation_id)
}

fn load_inflection_facts(
    transaction: &Transaction<'_>,
    repo_path: &str,
) -> Result<Vec<HistoryInflectionFact>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT r.sha, r.ordinal, r.coverage_json,
                    COUNT(p.path),
                    COALESCE(SUM(p.binary), 0),
                    COALESCE(SUM(p.generated), 0),
                    COALESCE(SUM(p.vendored), 0),
                    COALESCE(SUM(CASE
                        WHEN p.additions IS NOT NULL OR p.deletions IS NOT NULL
                        THEN COALESCE(p.additions, 0) + COALESCE(p.deletions, 0)
                        ELSE 0 END), 0),
                    COALESCE(SUM(CASE
                        WHEN p.additions IS NOT NULL OR p.deletions IS NOT NULL
                        THEN 1 ELSE 0 END), 0),
                    CASE WHEN COUNT(p.path) > 0 AND COUNT(p.path) = COALESCE(SUM(CASE
                        WHEN lower(p.path) IN (
                            'changelog', 'changelog.md', 'changelog.txt',
                            'package.json', 'package-lock.json', 'pnpm-lock.yaml', 'yarn.lock',
                            'cargo.toml', 'cargo.lock', 'version', 'version.txt'
                        ) OR lower(p.path) LIKE '.changeset/%'
                          OR lower(p.path) LIKE '%/.changeset/%'
                          OR lower(p.path) LIKE 'release-notes/%'
                          OR lower(p.path) LIKE '%/release-notes/%'
                          OR lower(p.path) LIKE '%/changelog.md'
                        THEN 1 ELSE 0 END), 0)
                    THEN 1 ELSE 0 END
             FROM history_graph_revisions r
             LEFT JOIN history_graph_revision_paths p
               ON p.repo_path = r.repo_path AND p.revision_sha = r.sha
             WHERE r.repo_path = ?1
             GROUP BY r.sha, r.ordinal, r.coverage_json
             ORDER BY r.ordinal, r.sha
             LIMIT ?2",
        )
        .map_err(|error| format!("Prepare persisted inflection facts: {error}"))?;
    let rows = statement
        .query_map(params![repo_path, (MAX_REVISIONS + 1) as i64], |row| {
            let coverage_json: String = row.get(2)?;
            let coverage: serde_json::Value =
                serde_json::from_str(&coverage_json).unwrap_or_default();
            let numeric_path_count = row.get::<_, u64>(8)?;
            Ok(HistoryInflectionFact {
                revision_sha: row.get(0)?,
                ordinal: row.get(1)?,
                churn: (numeric_path_count > 0)
                    .then(|| row.get::<_, u64>(7))
                    .transpose()?,
                changed_files: row.get(3)?,
                binary_files: row.get(4)?,
                generated_files: row.get(5)?,
                vendored_files: row.get(6)?,
                release_only: row.get::<_, i64>(9)? != 0,
                merge: coverage
                    .get("merge")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                coverage_complete: coverage.get("facts_schema_version").is_some(),
            })
        })
        .map_err(|error| format!("Query persisted inflection facts: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read persisted inflection facts: {error}"))?;
    Ok(rows)
}

enum StructuralLoad {
    Available(StructuralChangeMeasurements),
    Missing,
    Bounded,
    Invalid,
}

fn load_structural_measurements(
    transaction: &Transaction<'_>,
    repo_path: &str,
    revision_sha: &str,
) -> Result<StructuralLoad, String> {
    let row = transaction
        .query_row(
            "SELECT length(payload_json), payload_json
             FROM history_graph_events
             WHERE repo_path = ?1 AND revision_sha = ?2 AND event_kind = 'structural_delta'
             ORDER BY id LIMIT 1",
            params![repo_path, revision_sha],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load persisted structural measurements: {error}"))?;
    let Some((bytes, payload)) = row else {
        return Ok(StructuralLoad::Missing);
    };
    if bytes > MAX_STRUCTURAL_SUMMARY_BYTES {
        return Ok(StructuralLoad::Bounded);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
        return Ok(StructuralLoad::Invalid);
    };
    let count = |names: &[&str]| -> Option<u64> {
        names.iter().try_fold(0_u64, |total, name| {
            total.checked_add(value.get(name)?.as_array()?.len() as u64)
        })
    };
    let Some(measurements) = (|| {
        Some(StructuralChangeMeasurements {
            node_changes: count(&["added_node_ids", "removed_node_ids", "changed_node_ids"])?,
            edge_changes: count(&["added_edge_ids", "removed_edge_ids", "changed_edge_ids"])?,
            community_changes: count(&["added_community_ids", "removed_community_ids"])?,
            hub_changes: count(&["added_hub_ids", "removed_hub_ids"])?,
            bridge_changes: count(&["added_bridge_ids", "removed_bridge_ids"])?,
            coverage_gap: value
                .get("coverage_gap")
                .and_then(|gap| gap.as_str())
                .map(str::to_string),
        })
    })() else {
        return Ok(StructuralLoad::Invalid);
    };
    Ok(StructuralLoad::Available(measurements))
}

fn structural_status(candidate: &InflectionCandidate) -> &'static str {
    match candidate.structural.as_ref() {
        Some(structural) if structural.coverage_gap.is_none() => "complete",
        Some(_) => "partial",
        None => "unavailable",
    }
}

fn ensure_active(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Candidate-inflection publication cancelled".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn publish_candidate_inflections_forced_failure(
    transaction: &Transaction<'_>,
    repo_path: &str,
    index_identity: &str,
) -> Result<String, String> {
    publish_candidate_inflections_inner(
        transaction,
        repo_path,
        index_identity,
        true,
        "forced",
        &StructuralGraphCancellation::default(),
        true,
    )
}

#[cfg(test)]
#[path = "landmarks_tests.rs"]
mod tests;
