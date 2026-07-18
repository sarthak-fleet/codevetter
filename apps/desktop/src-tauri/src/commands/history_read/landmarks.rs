use super::{
    decode_opaque_cursor, encode_opaque_cursor,
    releases::{coverage_from_metadata, freshness_from_metadata},
    HistoryReadService,
};
use crate::commands::history_graph::{
    HistoryCoverageState, HistoryLandmark, HistoryLandmarkCatalog, HistoryLandmarkKind,
    HistoryLandmarkTrust, HistoryOpaqueCursor, HistoryReadCoverage,
    HISTORY_LANDMARK_CATALOG_SCHEMA_VERSION,
};
use crate::commands::structural_graph::types::stable_graph_id;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_LANDMARK_PAGE_LIMIT: usize = 100;
const MAX_LANDMARK_PAGE_LIMIT: usize = 500;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LandmarkCursorPayload {
    version: u8,
    scope: String,
    index_identity: String,
    query_identity: String,
    ordinal: i64,
    kind_rank: i64,
    sort_key: String,
}

#[derive(Debug)]
struct LandmarkGeneration {
    generation_id: String,
    index_identity: String,
    status: String,
}

#[derive(Debug)]
struct LandmarkRow {
    ordinal: i64,
    kind_rank: i64,
    sort_key: String,
    landmark: HistoryLandmark,
}

impl<'a> HistoryReadService<'a> {
    /// Lists release and candidate-inflection landmarks from indexed SQLite facts.
    /// This never invokes Git or reconstructs a historical graph.
    pub fn landmark_catalog(
        &self,
        kind: Option<HistoryLandmarkKind>,
        limit: Option<usize>,
        cursor: Option<&HistoryOpaqueCursor>,
    ) -> Result<HistoryLandmarkCatalog, String> {
        let applied_limit = limit
            .unwrap_or(DEFAULT_LANDMARK_PAGE_LIMIT)
            .clamp(1, MAX_LANDMARK_PAGE_LIMIT);
        let Some(metadata) = self.release_catalog_metadata()? else {
            return Ok(HistoryLandmarkCatalog {
                applied_limit,
                ..HistoryLandmarkCatalog::default()
            });
        };
        let generation = self.landmark_generation()?;
        let generation_current = generation
            .as_ref()
            .is_some_and(|value| value.index_identity == metadata.index_identity);
        let generation_identity = generation
            .as_ref()
            .filter(|_| generation_current)
            .map(|value| value.generation_id.as_str())
            .unwrap_or("none");
        let kind_name = kind_name(kind.as_ref());
        let query_identity = format!(
            "landmark_catalog:v1:kind={kind_name}:limit={applied_limit}:generation={generation_identity}"
        );
        let after = cursor
            .map(|cursor| {
                self.decode_landmark_cursor(cursor, &metadata.index_identity, &query_identity)
            })
            .transpose()?;
        let mut rows = self.query_landmarks(
            kind.as_ref(),
            generation_current.then_some(generation_identity),
            after.as_ref(),
            applied_limit + 1,
        )?;
        let truncated = rows.len() > applied_limit;
        rows.truncate(applied_limit);
        let next_cursor = if truncated {
            rows.last()
                .map(|row| {
                    self.encode_landmark_cursor(
                        &metadata.index_identity,
                        &query_identity,
                        row.ordinal,
                        row.kind_rank,
                        &row.sort_key,
                    )
                })
                .transpose()?
        } else {
            None
        };

        let mut coverage = coverage_from_metadata(&metadata);
        if let Some(generation) = generation {
            if !generation_current {
                add_coverage_reason(&mut coverage, "landmark_generation_stale");
            } else if generation.status != "ready" {
                add_coverage_reason(&mut coverage, "candidate_inflection_partial");
            }
        }
        Ok(HistoryLandmarkCatalog {
            schema_version: HISTORY_LANDMARK_CATALOG_SCHEMA_VERSION,
            landmarks: rows.into_iter().map(|row| row.landmark).collect(),
            coverage,
            freshness: freshness_from_metadata(&metadata, &self.current_head),
            applied_limit,
            truncated,
            next_cursor,
        })
    }

    fn landmark_generation(&self) -> Result<Option<LandmarkGeneration>, String> {
        self.connection
            .query_row(
                "SELECT generation_id, index_identity, status
                 FROM history_graph_landmark_generations WHERE repo_path = ?1",
                params![self.repo_path],
                |row| {
                    Ok(LandmarkGeneration {
                        generation_id: row.get(0)?,
                        index_identity: row.get(1)?,
                        status: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("Load landmark generation: {error}"))
    }

    fn query_landmarks(
        &self,
        kind: Option<&HistoryLandmarkKind>,
        generation_id: Option<&str>,
        after: Option<&LandmarkCursorPayload>,
        limit: usize,
    ) -> Result<Vec<LandmarkRow>, String> {
        let include_release = !matches!(kind, Some(HistoryLandmarkKind::CandidateInflection));
        let include_candidate =
            !matches!(kind, Some(HistoryLandmarkKind::Release)) && generation_id.is_some();
        let mut rows = Vec::new();
        if include_release {
            let mut statement = self.connection.prepare(
                "SELECT r.ordinal, t.tag, t.revision_sha,
                        (SELECT json_group_array(grouped.tag) FROM (
                            SELECT sibling.tag FROM history_graph_release_tags sibling
                            WHERE sibling.repo_path = t.repo_path
                              AND sibling.revision_sha = t.revision_sha ORDER BY sibling.tag
                        ) grouped)
                 FROM history_graph_release_tags t
                 JOIN history_graph_revisions r ON r.repo_path = t.repo_path AND r.sha = t.revision_sha
                 WHERE t.repo_path = ?1",
            ).map_err(|error| format!("Prepare release landmarks: {error}"))?;
            let release_rows = statement
                .query_map(params![self.repo_path], |row| {
                    let tag: String = row.get(1)?;
                    let revision_sha: String = row.get(2)?;
                    let tags_json: String = row.get(3)?;
                    let tags = serde_json::from_str(&tags_json).unwrap_or_default();
                    Ok(LandmarkRow {
                        ordinal: row.get(0)?,
                        kind_rank: 0,
                        sort_key: tag.clone(),
                        landmark: HistoryLandmark {
                            id: stable_graph_id(
                                "release-tag",
                                &format!("{}\0{}\0{}", self.repo_path, tag, revision_sha),
                            ),
                            kind: HistoryLandmarkKind::Release,
                            revision_sha,
                            ordinal: row.get(0)?,
                            label: tag,
                            tags,
                            trust: HistoryLandmarkTrust::Extracted,
                            score_milli: None,
                            components: Value::Null,
                            reasons: Vec::new(),
                            caveats: Vec::new(),
                            coverage: Value::Null,
                            evidence_ids: Vec::new(),
                        },
                    })
                })
                .map_err(|error| format!("Query release landmarks: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Read release landmarks: {error}"))?;
            rows.extend(release_rows);
        }
        if include_candidate {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT id, revision_sha, ordinal, label, trust, score_milli,
                        components_json, reasons_json, caveats_json, coverage_json
                 FROM history_graph_landmarks
                 WHERE repo_path = ?1 AND generation_id = ?2 AND kind = 'candidate_inflection'",
                )
                .map_err(|error| format!("Prepare candidate landmarks: {error}"))?;
            let candidate_rows = statement
                .query_map(params![self.repo_path, generation_id], |row| {
                    let trust: String = row.get(4)?;
                    let components: String = row.get(6)?;
                    let reasons: String = row.get(7)?;
                    let caveats: String = row.get(8)?;
                    let coverage: String = row.get(9)?;
                    let id: String = row.get(0)?;
                    let trust = match trust.as_str() {
                        "qualified" => HistoryLandmarkTrust::Qualified,
                        "qualified_partial" => HistoryLandmarkTrust::QualifiedPartial,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(LandmarkRow {
                        ordinal: row.get(2)?,
                        kind_rank: 1,
                        sort_key: id.clone(),
                        landmark: HistoryLandmark {
                            id,
                            kind: HistoryLandmarkKind::CandidateInflection,
                            revision_sha: row.get(1)?,
                            ordinal: row.get(2)?,
                            label: row.get(3)?,
                            tags: Vec::new(),
                            trust,
                            score_milli: Some(row.get(5)?),
                            components: serde_json::from_str(&components).unwrap_or_default(),
                            reasons: serde_json::from_str(&reasons).unwrap_or_default(),
                            caveats: serde_json::from_str(&caveats).unwrap_or_default(),
                            coverage: serde_json::from_str(&coverage).unwrap_or_default(),
                            evidence_ids: Vec::new(),
                        },
                    })
                })
                .map_err(|error| format!("Query candidate landmarks: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Read candidate landmarks: {error}"))?;
            rows.extend(candidate_rows);
        }
        rows.sort_by(|left, right| {
            right
                .ordinal
                .cmp(&left.ordinal)
                .then_with(|| left.kind_rank.cmp(&right.kind_rank))
                .then_with(|| left.sort_key.cmp(&right.sort_key))
        });
        if let Some(after) = after {
            rows.retain(|row| {
                row.ordinal < after.ordinal
                    || (row.ordinal == after.ordinal
                        && (row.kind_rank > after.kind_rank
                            || (row.kind_rank == after.kind_rank && row.sort_key > after.sort_key)))
            });
        }
        rows.truncate(limit);
        Ok(rows)
    }

    fn encode_landmark_cursor(
        &self,
        index_identity: &str,
        query_identity: &str,
        ordinal: i64,
        kind_rank: i64,
        sort_key: &str,
    ) -> Result<HistoryOpaqueCursor, String> {
        let payload = LandmarkCursorPayload {
            version: 1,
            scope: stable_graph_id("landmark-cursor-scope", &self.repo_path),
            index_identity: index_identity.to_string(),
            query_identity: query_identity.to_string(),
            ordinal,
            kind_rank,
            sort_key: sort_key.to_string(),
        };
        encode_opaque_cursor(&payload, "history landmark cursor")
    }

    fn decode_landmark_cursor(
        &self,
        cursor: &HistoryOpaqueCursor,
        index_identity: &str,
        query_identity: &str,
    ) -> Result<LandmarkCursorPayload, String> {
        let payload: LandmarkCursorPayload = decode_opaque_cursor(cursor)?;
        if payload.version != 1 {
            return Err("Invalid history cursor".to_string());
        }
        if payload.scope != stable_graph_id("landmark-cursor-scope", &self.repo_path)
            || payload.query_identity != query_identity
        {
            return Err("History cursor does not match this repository or query".to_string());
        }
        if payload.index_identity != index_identity {
            return Err("History cursor is stale".to_string());
        }
        Ok(payload)
    }
}

fn kind_name(kind: Option<&HistoryLandmarkKind>) -> &'static str {
    match kind {
        Some(HistoryLandmarkKind::Release) => "release",
        Some(HistoryLandmarkKind::CandidateInflection) => "candidate_inflection",
        None => "all",
    }
}

fn add_coverage_reason(coverage: &mut HistoryReadCoverage, reason: &str) {
    coverage.state = HistoryCoverageState::Partial;
    if !coverage.reasons.iter().any(|value| value == reason) {
        coverage.reasons.push(reason.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::history_graph::HistoryTimelineCenter;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn sha(value: usize) -> String {
        format!("{value:040x}")
    }

    fn service<'a>(connection: &'a Connection, repo: &str) -> HistoryReadService<'a> {
        HistoryReadService::new_with_current_head(connection, PathBuf::from(repo), sha(4)).unwrap()
    }

    fn fixture() -> (Connection, String) {
        let connection = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&connection).unwrap();
        let repo = "/fixture/landmarks".to_string();
        connection.execute_batch(&format!(
            "INSERT INTO history_graph_repositories (repo_path, repository_fingerprint, indexed_head, indexed_tags_fingerprint, status, coverage_json, created_at, updated_at)
             VALUES ('{repo}', 'fixture', '{}', 'tags-v1', 'ready', '{{}}', 'now', 'now');
             INSERT INTO history_graph_revisions (repo_path, sha, ordinal, committed_at, author_name, subject, parents_json, tags_json, is_release, is_head) VALUES
             ('{repo}', '{}', 1, 'now', 'Fixture', 'one', '[]', '[]', 0, 0),
             ('{repo}', '{}', 2, 'now', 'Fixture', 'two', '[]', '[]', 0, 0),
             ('{repo}', '{}', 3, 'now', 'Fixture', 'three', '[]', '[]', 0, 0),
             ('{repo}', '{}', 4, 'now', 'Fixture', 'four', '[]', '[]', 0, 1);
             INSERT INTO history_graph_release_catalogs (repo_path, index_identity, indexed_head, tags_fingerprint, status, coverage_json, updated_at)
             VALUES ('{repo}', 'index-v1', '{}', 'tags-v1', 'ready', '{{\"ancestry_complete\":true}}', 'now');
             INSERT INTO history_graph_release_tags (repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at) VALUES
             ('{repo}', 'v1.0.0', '{}', '{}', 'lightweight', 1),
             ('{repo}', 'v1.0.0-lts', '{}', '{}', 'annotated', 1);
             INSERT INTO history_graph_landmark_generations (repo_path, schema_version, algorithm, algorithm_version, generation_id, index_identity, status, landmark_count, coverage_json, updated_at)
             VALUES ('{repo}', 1, 'robust_mad', 1, 'generation-v1', 'index-v1', 'ready', 1, '{{}}', 'now');
             INSERT INTO history_graph_landmarks (repo_path, generation_id, id, revision_sha, ordinal, kind, label, trust, score_milli, components_json, reasons_json, caveats_json, coverage_json)
             VALUES ('{repo}', 'generation-v1', 'landmark-3', '{}', 3, 'candidate_inflection', 'Candidate inflection', 'qualified', 9000, '{{\"churn\":42}}', '[\"42 changed lines\"]', '[]', '{{\"non_causal\":true}}');",
            sha(4), sha(1), sha(2), sha(3), sha(4), sha(4), sha(2), sha(2), sha(2), sha(2), sha(3)
        )).unwrap();
        (connection, repo)
    }

    #[test]
    fn landmark_catalog_is_deterministic_paginated_and_revision_exact() {
        let (connection, repo) = fixture();
        let first = service(&connection, &repo)
            .landmark_catalog(None, Some(2), None)
            .unwrap();
        assert_eq!(first.landmarks.len(), 2);
        assert_eq!(
            first.landmarks[0].kind,
            HistoryLandmarkKind::CandidateInflection
        );
        assert_eq!(first.landmarks[0].revision_sha, sha(3));
        assert_eq!(first.landmarks[1].kind, HistoryLandmarkKind::Release);
        assert_eq!(first.landmarks[1].tags, ["v1.0.0", "v1.0.0-lts"]);
        let second = service(&connection, &repo)
            .landmark_catalog(None, Some(2), first.next_cursor.as_ref())
            .unwrap();
        assert_eq!(second.landmarks.len(), 1);
        assert_eq!(second.landmarks[0].label, "v1.0.0-lts");
        assert!(!second.truncated);
        let window = service(&connection, &repo)
            .timeline_window(
                HistoryTimelineCenter::Landmark {
                    landmark_id: "landmark-3".to_string(),
                },
                Some(3),
            )
            .unwrap();
        assert_eq!(window.center_revision.as_deref(), Some(sha(3).as_str()));
    }

    #[test]
    fn landmark_catalog_hides_stale_candidate_generation_but_keeps_release_facts() {
        let (connection, repo) = fixture();
        connection
            .execute(
                "UPDATE history_graph_landmark_generations SET index_identity = 'old'",
                [],
            )
            .unwrap();
        let catalog = service(&connection, &repo)
            .landmark_catalog(None, None, None)
            .unwrap();
        assert!(catalog
            .landmarks
            .iter()
            .all(|landmark| landmark.kind == HistoryLandmarkKind::Release));
        assert_eq!(catalog.coverage.state, HistoryCoverageState::Partial);
        assert!(catalog
            .coverage
            .reasons
            .contains(&"landmark_generation_stale".to_string()));
    }

    #[test]
    fn legacy_database_returns_versioned_empty_catalog() {
        let connection = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&connection).unwrap();
        let catalog = service(&connection, "/fixture/legacy")
            .landmark_catalog(None, None, None)
            .unwrap();
        assert_eq!(
            catalog.schema_version,
            HISTORY_LANDMARK_CATALOG_SCHEMA_VERSION
        );
        assert!(catalog.landmarks.is_empty());
        assert_eq!(catalog.coverage.state, HistoryCoverageState::Unavailable);
        assert_eq!(catalog.applied_limit, 100);
    }
}
