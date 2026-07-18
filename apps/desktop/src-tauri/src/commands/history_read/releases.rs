use super::*;
use crate::commands::{
    history_graph::{
        HistoryCoverageState, HistoryOpaqueCursor, HistoryReadCoverage, HistoryReadFreshness,
        HistoryReleaseCatalogEntry, HistoryReleaseIntervalMetadata, HistoryReleaseTagKind,
        HISTORY_RELEASE_CATALOG_SCHEMA_VERSION, HISTORY_TIMELINE_WINDOW_SCHEMA_VERSION,
    },
    structural_graph::types::stable_graph_id,
};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;

const DEFAULT_RELEASE_PAGE_LIMIT: usize = 100;
const MAX_RELEASE_PAGE_LIMIT: usize = 500;
const DEFAULT_TIMELINE_WINDOW_LIMIT: usize = 51;
const MAX_TIMELINE_WINDOW_LIMIT: usize = 201;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseCursorPayload {
    version: u8,
    scope: String,
    index_identity: String,
    query_identity: String,
    position: ReleaseCursorPosition,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReleaseCursorPosition {
    Catalog { ordinal: i64, tag: String },
    Revision { revision_sha: String },
}

#[derive(Debug)]
pub(super) struct ReleaseCatalogMetadata {
    pub(super) index_identity: String,
    pub(super) indexed_head: String,
    pub(super) tags_fingerprint: String,
    pub(super) status: String,
    pub(super) coverage: JsonValue,
    pub(super) repository_coverage: JsonValue,
}

#[derive(Debug)]
struct ReleaseRow {
    tag: String,
    tag_kind: HistoryReleaseTagKind,
    revision_sha: String,
    ordinal: i64,
    tagged_at: Option<i64>,
    coincident_tags: Vec<String>,
    interval_from_exclusive_sha: Option<String>,
    interval_commit_count: Option<i64>,
    interval_observed_commit_count: Option<i64>,
    interval_coverage_kind: Option<String>,
}

impl<'a> HistoryReadService<'a> {
    /// Lists normalized release rows without invoking Git or reconstructing a graph.
    pub fn release_catalog(
        &self,
        limit: Option<usize>,
        cursor: Option<&HistoryOpaqueCursor>,
    ) -> Result<HistoryReleaseCatalog, String> {
        let applied_limit = limit
            .unwrap_or(DEFAULT_RELEASE_PAGE_LIMIT)
            .clamp(1, MAX_RELEASE_PAGE_LIMIT);
        let Some(metadata) = self.release_catalog_metadata()? else {
            return Ok(HistoryReleaseCatalog {
                applied_limit,
                ..HistoryReleaseCatalog::default()
            });
        };
        let query_identity = format!("release_catalog:v1:limit={applied_limit}");
        let after = cursor
            .map(|cursor| {
                match self.decode_release_cursor(
                    cursor,
                    &metadata.index_identity,
                    &query_identity,
                )? {
                    ReleaseCursorPosition::Catalog { ordinal, tag } => Ok((ordinal, tag)),
                    _ => Err("Invalid history cursor".to_string()),
                }
            })
            .transpose()?;
        let mut rows = self.query_release_rows(after.as_ref(), applied_limit + 1)?;
        let truncated = rows.len() > applied_limit;
        rows.truncate(applied_limit);
        let next_cursor = if truncated {
            rows.last()
                .map(|row| {
                    self.encode_release_cursor(
                        &metadata.index_identity,
                        &query_identity,
                        ReleaseCursorPosition::Catalog {
                            ordinal: row.ordinal,
                            tag: row.tag.clone(),
                        },
                    )
                })
                .transpose()?
        } else {
            None
        };
        Ok(HistoryReleaseCatalog {
            schema_version: HISTORY_RELEASE_CATALOG_SCHEMA_VERSION,
            releases: rows
                .into_iter()
                .map(|row| self.release_entry(row))
                .collect(),
            coverage: coverage_from_metadata(&metadata),
            freshness: freshness_from_metadata(&metadata, &self.current_head),
            applied_limit,
            truncated,
            next_cursor,
        })
    }

    /// Loads a bounded revision window around an exact indexed release or revision.
    pub fn timeline_window(
        &self,
        center: HistoryTimelineCenter,
        limit: Option<usize>,
    ) -> Result<HistoryTimelineWindow, String> {
        let applied_limit = limit
            .unwrap_or(DEFAULT_TIMELINE_WINDOW_LIMIT)
            .clamp(1, MAX_TIMELINE_WINDOW_LIMIT);
        let metadata = self
            .release_catalog_metadata()?
            .ok_or_else(|| "Release history is not indexed for this repository".to_string())?;
        let query_identity = format!("timeline_window:v1:limit={applied_limit}");
        let center_revision = match center {
            HistoryTimelineCenter::Release { tag } => self.resolve_release_tag(&tag)?,
            HistoryTimelineCenter::Revision { revision_sha } => {
                validate_exact_revision(&revision_sha)?;
                revision_sha
            }
            HistoryTimelineCenter::Landmark { landmark_id } => {
                self.resolve_landmark_id(&landmark_id, &metadata.index_identity)?
            }
            HistoryTimelineCenter::Cursor { cursor } => match self.decode_release_cursor(
                &cursor,
                &metadata.index_identity,
                &query_identity,
            )? {
                ReleaseCursorPosition::Revision { revision_sha } => revision_sha,
                _ => return Err("Invalid history cursor".to_string()),
            },
        };
        let center_ordinal = self.ordinal(&center_revision)?;
        let mut rows = self.query_window_revisions(center_ordinal, applied_limit)?;
        rows.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.sha.cmp(&right.1.sha))
        });
        let first_ordinal = rows.first().map(|row| row.0).unwrap_or(center_ordinal);
        let last_ordinal = rows.last().map(|row| row.0).unwrap_or(center_ordinal);
        let (older_revision, newer_revision) =
            self.adjacent_revisions(first_ordinal, last_ordinal)?;
        let has_older = older_revision.is_some();
        let has_newer = newer_revision.is_some();
        let older_cursor = older_revision
            .map(|revision| {
                self.encode_release_cursor(
                    &metadata.index_identity,
                    &query_identity,
                    ReleaseCursorPosition::Revision {
                        revision_sha: revision,
                    },
                )
            })
            .transpose()?;
        let newer_cursor = newer_revision
            .map(|revision| {
                self.encode_release_cursor(
                    &metadata.index_identity,
                    &query_identity,
                    ReleaseCursorPosition::Revision {
                        revision_sha: revision,
                    },
                )
            })
            .transpose()?;
        let releases = self.release_rows_between(first_ordinal, last_ordinal)?;
        Ok(HistoryTimelineWindow {
            schema_version: HISTORY_TIMELINE_WINDOW_SCHEMA_VERSION,
            center_revision: Some(center_revision),
            revisions: rows.into_iter().map(|row| row.1).collect(),
            releases: releases
                .into_iter()
                .map(|row| self.release_entry(row))
                .collect(),
            coverage: coverage_from_metadata(&metadata),
            freshness: freshness_from_metadata(&metadata, &self.current_head),
            applied_limit,
            truncated: has_older || has_newer,
            has_older,
            has_newer,
            older_cursor,
            newer_cursor,
        })
    }

    pub(super) fn release_catalog_metadata(
        &self,
    ) -> Result<Option<ReleaseCatalogMetadata>, String> {
        self.connection
            .query_row(
                "SELECT c.index_identity, c.indexed_head, c.tags_fingerprint, c.status,
                        c.coverage_json, r.coverage_json
                 FROM history_graph_release_catalogs c
                 JOIN history_graph_repositories r ON r.repo_path = c.repo_path
                 WHERE c.repo_path = ?1",
                params![self.repo_path],
                |row| {
                    let catalog_coverage: String = row.get(4)?;
                    let repository_coverage: String = row.get(5)?;
                    Ok(ReleaseCatalogMetadata {
                        index_identity: row.get(0)?,
                        indexed_head: row.get(1)?,
                        tags_fingerprint: row.get(2)?,
                        status: row.get(3)?,
                        coverage: serde_json::from_str(&catalog_coverage).unwrap_or_default(),
                        repository_coverage: serde_json::from_str(&repository_coverage)
                            .unwrap_or_default(),
                    })
                },
            )
            .optional()
            .map_err(|error| format!("Load release catalog metadata: {error}"))
    }

    fn query_release_rows(
        &self,
        after: Option<&(i64, String)>,
        limit: usize,
    ) -> Result<Vec<ReleaseRow>, String> {
        let (after_ordinal, after_tag) = after.cloned().unwrap_or((i64::MAX, String::new()));
        let mut statement = self
            .connection
            .prepare(&format!(
                "{} WHERE t.repo_path = ?1
                   AND (?2 = '' OR r.ordinal < ?3 OR (r.ordinal = ?3 AND t.tag > ?2))
                 ORDER BY r.ordinal DESC, t.tag ASC LIMIT ?4",
                release_row_select()
            ))
            .map_err(|error| format!("Prepare release catalog query: {error}"))?;
        let rows = statement
            .query_map(
                params![self.repo_path, after_tag, after_ordinal, limit as i64],
                map_release_row,
            )
            .map_err(|error| format!("Query release catalog: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read release catalog: {error}"))?;
        Ok(rows)
    }

    fn release_rows_between(&self, first: i64, last: i64) -> Result<Vec<ReleaseRow>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "{} WHERE t.repo_path = ?1 AND r.ordinal BETWEEN ?2 AND ?3
                 ORDER BY r.ordinal, t.tag",
                release_row_select()
            ))
            .map_err(|error| format!("Prepare window release query: {error}"))?;
        let rows = statement
            .query_map(params![self.repo_path, first, last], map_release_row)
            .map_err(|error| format!("Query window releases: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read window releases: {error}"))?;
        Ok(rows)
    }

    fn resolve_release_tag(&self, tag: &str) -> Result<String, String> {
        let tag = tag.trim();
        if tag.is_empty() || tag.starts_with('-') || tag.len() > 256 {
            return Err("A valid exact release tag is required".to_string());
        }
        self.connection
            .query_row(
                "SELECT revision_sha FROM history_graph_release_tags
                 WHERE repo_path = ?1 AND tag = ?2",
                params![self.repo_path, tag],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Resolve release tag: {error}"))?
            .ok_or_else(|| "Release tag is not indexed for this repository".to_string())
    }

    fn resolve_landmark_id(
        &self,
        landmark_id: &str,
        index_identity: &str,
    ) -> Result<String, String> {
        let landmark_id = landmark_id.trim();
        if landmark_id.is_empty() || landmark_id.len() > 512 || landmark_id.contains('\0') {
            return Err("A valid landmark identifier is required".to_string());
        }
        self.connection
            .query_row(
                "SELECT landmark.revision_sha
                 FROM history_graph_landmarks landmark
                 JOIN history_graph_landmark_generations generation
                   ON generation.repo_path = landmark.repo_path
                  AND generation.generation_id = landmark.generation_id
                 WHERE landmark.repo_path = ?1
                   AND landmark.id = ?2
                   AND generation.index_identity = ?3",
                params![self.repo_path, landmark_id, index_identity],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Resolve history landmark: {error}"))?
            .ok_or_else(|| "Landmark is not indexed for this repository".to_string())
    }

    fn query_window_revisions(
        &self,
        center_ordinal: i64,
        limit: usize,
    ) -> Result<Vec<(i64, crate::commands::history_graph::HistoryRevision)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT ordinal, sha, substr(sha, 1, 8), parents_json, committed_at,
                        author_name, subject, tags_json, is_release, is_head
                 FROM history_graph_revisions WHERE repo_path = ?1
                 ORDER BY abs(ordinal - ?2), ordinal, sha LIMIT ?3",
            )
            .map_err(|error| format!("Prepare timeline window: {error}"))?;
        let rows = statement
            .query_map(
                params![self.repo_path, center_ordinal, limit as i64],
                |row| {
                    let parents: String = row.get(3)?;
                    let tags: String = row.get(7)?;
                    Ok((
                        row.get(0)?,
                        crate::commands::history_graph::HistoryRevision {
                            sha: row.get(1)?,
                            short_sha: row.get(2)?,
                            parents: serde_json::from_str(&parents).unwrap_or_default(),
                            committed_at: row.get(4)?,
                            author: row.get(5)?,
                            subject: row.get(6)?,
                            tags: serde_json::from_str(&tags).unwrap_or_default(),
                            is_release: row.get::<_, i64>(8)? != 0,
                            is_head: row.get::<_, i64>(9)? != 0,
                            ordinal: row.get(0)?,
                        },
                    ))
                },
            )
            .map_err(|error| format!("Query timeline window: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read timeline window: {error}"))?;
        Ok(rows)
    }

    fn adjacent_revisions(
        &self,
        first_ordinal: i64,
        last_ordinal: i64,
    ) -> Result<(Option<String>, Option<String>), String> {
        self.connection
            .query_row(
                "SELECT
                    (SELECT sha FROM history_graph_revisions
                     WHERE repo_path = ?1 AND ordinal < ?2 ORDER BY ordinal DESC LIMIT 1),
                    (SELECT sha FROM history_graph_revisions
                     WHERE repo_path = ?1 AND ordinal > ?3 ORDER BY ordinal ASC LIMIT 1)",
                params![self.repo_path, first_ordinal, last_ordinal],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("Resolve adjacent history revision: {error}"))
    }

    fn release_entry(&self, row: ReleaseRow) -> HistoryReleaseCatalogEntry {
        let id = stable_graph_id(
            "release-tag",
            &format!("{}\0{}\0{}", self.repo_path, row.tag, row.revision_sha),
        );
        HistoryReleaseCatalogEntry {
            // Tag rows are the canonical extracted fact; no hydratable event exists yet.
            evidence_ids: Vec::new(),
            id,
            tag: row.tag,
            tag_kind: row.tag_kind,
            revision_sha: row.revision_sha,
            ordinal: row.ordinal,
            tagged_at: row.tagged_at.and_then(|seconds| {
                DateTime::<Utc>::from_timestamp(seconds, 0).map(|value| value.to_rfc3339())
            }),
            coincident_tags: row.coincident_tags,
            interval: row.interval_coverage_kind.map(|coverage_kind| {
                HistoryReleaseIntervalMetadata {
                    schema_version: 1,
                    from_exclusive_sha: row.interval_from_exclusive_sha,
                    commit_count: row
                        .interval_commit_count
                        .and_then(|count| usize::try_from(count).ok()),
                    observed_commit_count: row
                        .interval_observed_commit_count
                        .and_then(|count| usize::try_from(count).ok())
                        .unwrap_or_default(),
                    coverage: if coverage_kind == "complete" {
                        HistoryCoverageState::Complete
                    } else {
                        HistoryCoverageState::Partial
                    },
                    coverage_reason: (coverage_kind != "complete").then_some(coverage_kind),
                }
            }),
        }
    }

    fn encode_release_cursor(
        &self,
        index_identity: &str,
        query_identity: &str,
        position: ReleaseCursorPosition,
    ) -> Result<HistoryOpaqueCursor, String> {
        let scope = stable_graph_id("release-cursor-scope", &self.repo_path);
        let payload = ReleaseCursorPayload {
            version: 1,
            scope,
            index_identity: index_identity.to_string(),
            query_identity: query_identity.to_string(),
            position,
        };
        encode_opaque_cursor(&payload, "history cursor")
    }

    fn decode_release_cursor(
        &self,
        cursor: &HistoryOpaqueCursor,
        index_identity: &str,
        query_identity: &str,
    ) -> Result<ReleaseCursorPosition, String> {
        let payload: ReleaseCursorPayload = decode_opaque_cursor(cursor)?;
        let scope = stable_graph_id("release-cursor-scope", &self.repo_path);
        if payload.version != 1 {
            return Err("Invalid history cursor".to_string());
        }
        if payload.scope != scope || payload.query_identity != query_identity {
            return Err("History cursor does not match this repository or query".to_string());
        }
        if payload.index_identity != index_identity {
            return Err("History cursor is stale".to_string());
        }
        Ok(payload.position)
    }
}

fn release_row_select() -> &'static str {
    "SELECT t.tag, t.tag_kind, t.revision_sha, r.ordinal, t.tagged_at,
            (SELECT json_group_array(grouped.tag) FROM (
                SELECT sibling.tag FROM history_graph_release_tags sibling
                WHERE sibling.repo_path = t.repo_path
                  AND sibling.revision_sha = t.revision_sha ORDER BY sibling.tag
            ) grouped),
            i.from_exclusive_sha, i.commit_count, i.observed_commit_count, i.coverage_kind
     FROM history_graph_release_tags t
     JOIN history_graph_revisions r
       ON r.repo_path = t.repo_path AND r.sha = t.revision_sha
     LEFT JOIN history_graph_release_intervals i
       ON i.repo_path = t.repo_path AND i.tag = t.tag"
}

fn map_release_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReleaseRow> {
    let kind: String = row.get(1)?;
    let coincident_json: String = row.get(5)?;
    let tag_kind = match kind.as_str() {
        "annotated" => HistoryReleaseTagKind::Annotated,
        "lightweight" => HistoryReleaseTagKind::Lightweight,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid release tag kind")
                    .into(),
            ))
        }
    };
    Ok(ReleaseRow {
        tag: row.get(0)?,
        tag_kind,
        revision_sha: row.get(2)?,
        ordinal: row.get(3)?,
        tagged_at: row.get(4)?,
        coincident_tags: serde_json::from_str(&coincident_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        interval_from_exclusive_sha: row.get(6)?,
        interval_commit_count: row.get(7)?,
        interval_observed_commit_count: row.get(8)?,
        interval_coverage_kind: row.get(9)?,
    })
}

pub(super) fn coverage_from_metadata(metadata: &ReleaseCatalogMetadata) -> HistoryReadCoverage {
    let value = |source: &JsonValue, key| source.get(key).and_then(JsonValue::as_bool);
    let ancestry_complete = value(&metadata.coverage, "ancestry_complete").unwrap_or(false);
    let is_shallow = value(&metadata.coverage, "is_shallow")
        .or_else(|| value(&metadata.repository_coverage, "is_shallow"))
        .unwrap_or(false);
    let truncated = value(&metadata.repository_coverage, "truncated").unwrap_or(false);
    let mut reasons = Vec::new();
    for (applies, reason) in [
        (metadata.status != "ready", "release_catalog_partial"),
        (!ancestry_complete, "ancestry_incomplete"),
        (is_shallow, "shallow_repository"),
        (truncated, "revision_index_truncated"),
    ] {
        if applies {
            reasons.push(reason.to_string());
        }
    }
    HistoryReadCoverage {
        state: if reasons.is_empty() {
            HistoryCoverageState::Complete
        } else {
            HistoryCoverageState::Partial
        },
        ancestry_complete,
        is_shallow,
        truncated,
        reasons,
    }
}

pub(super) fn freshness_from_metadata(
    metadata: &ReleaseCatalogMetadata,
    current_head: &str,
) -> HistoryReadFreshness {
    let current_revision = (!current_head.is_empty()).then(|| current_head.to_string());
    let stale = current_revision.as_deref() != Some(metadata.indexed_head.as_str());
    HistoryReadFreshness {
        indexed_revision: Some(metadata.indexed_head.clone()),
        current_revision,
        indexed_tags_fingerprint: Some(metadata.tags_fingerprint.clone()),
        // Live tag identity must be supplied by a watcher/caller; indexed tags are not current.
        current_tags_fingerprint: None,
        stale,
    }
}

fn validate_exact_revision(revision: &str) -> Result<(), String> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("A full exact Git revision SHA is required".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        connection: Connection,
        repo: String,
        other_repo: String,
        revisions: Vec<String>,
    }

    impl Fixture {
        fn new(partial: bool) -> Self {
            let connection = Connection::open_in_memory().expect("database");
            crate::db::schema::run_migrations(&connection).expect("schema");
            let repo = "/fixture/release-read-a".to_string();
            let other_repo = "/fixture/release-read-b".to_string();
            let revisions = (1..=8).map(sha).collect::<Vec<_>>();
            connection.execute_batch(
                "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, indexed_head, indexed_tags_fingerprint,
                    status, coverage_json, created_at, updated_at) VALUES
                    ('/fixture/release-read-a', 'fixture', printf('%040x', 8), 'tags-v1',
                     'ready', '{}', 'now', 'now'),
                    ('/fixture/release-read-b', 'fixture', printf('%040x', 1), 'tags-v1',
                     'ready', '{}', 'now', 'now');
                 WITH RECURSIVE seq(ordinal) AS (SELECT 0 UNION ALL SELECT ordinal + 1 FROM seq WHERE ordinal < 7)
                 INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject, parents_json,
                    tags_json, is_release, is_head)
                 SELECT '/fixture/release-read-a', printf('%040x', ordinal + 1), ordinal,
                    '2026-01-01T00:00:00Z', 'Fixture', printf('commit %d', ordinal), '[]', '[]', 0, 0 FROM seq;
                 INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject, parents_json,
                    tags_json, is_release, is_head) VALUES
                    ('/fixture/release-read-b', printf('%040x', 1), 0, '2026-01-01T00:00:00Z',
                     'Fixture', 'commit 0', '[]', '[]', 0, 0);
                 INSERT INTO history_graph_release_catalogs (
                    repo_path, index_identity, indexed_head, tags_fingerprint, status,
                    coverage_json, updated_at) VALUES
                    ('/fixture/release-read-a', 'index:/fixture/release-read-a', printf('%040x', 8),
                     'tags-v1', 'ready', '{\"ancestry_complete\":true}', 'now'),
                    ('/fixture/release-read-b', 'index:/fixture/release-read-b', printf('%040x', 1),
                     'tags-v1', 'ready', '{\"ancestry_complete\":true}', 'now');
                 INSERT INTO history_graph_release_tags (
                    repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at) VALUES
                    ('/fixture/release-read-a', 'v1.0.0', printf('%040x', 2), printf('%040x', 102), 'annotated', 1),
                    ('/fixture/release-read-a', 'v2.0.0', printf('%040x', 5), printf('%040x', 105), 'lightweight', 4),
                    ('/fixture/release-read-a', 'v2.0.0-lts', printf('%040x', 5), printf('%040x', 105), 'annotated', 4),
                    ('/fixture/release-read-a', 'v3.0.0', printf('%040x', 7), printf('%040x', 107), 'lightweight', 6),
                    ('/fixture/release-read-b', 'v1.0.0', printf('%040x', 1), printf('%040x', 1), 'lightweight', 1);
                 INSERT INTO history_graph_fact_tags (
                    repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at) VALUES
                    ('/fixture/release-read-a', 'v2.0.0', printf('%040x', 5), printf('%040x', 105), 'lightweight', 4);
                 INSERT INTO history_graph_release_intervals (
                    repo_path, tag, revision_sha, from_exclusive_sha, commit_count,
                    observed_commit_count, coverage_kind) VALUES
                    ('/fixture/release-read-a', 'v2.0.0', printf('%040x', 5),
                     printf('%040x', 2), 3, 3, 'complete');"
            ).expect("release read fixture");
            if partial {
                connection
                    .execute_batch(
                        "UPDATE history_graph_repositories SET coverage_json =
                        '{\"truncated\":true,\"is_shallow\":true}'
                     WHERE repo_path = '/fixture/release-read-a';
                     UPDATE history_graph_release_catalogs SET status = 'partial', coverage_json =
                        '{\"ancestry_complete\":false,\"is_shallow\":true}'
                     WHERE repo_path = '/fixture/release-read-a';",
                    )
                    .expect("partial coverage");
            }
            Self {
                connection,
                repo,
                other_repo,
                revisions,
            }
        }

        fn service(&self) -> HistoryReadService<'_> {
            HistoryReadService::new_with_current_head(
                &self.connection,
                PathBuf::from(&self.repo),
                self.revisions[7].clone(),
            )
            .expect("service")
        }

        fn other_service(&self) -> HistoryReadService<'_> {
            HistoryReadService::new_with_current_head(
                &self.connection,
                PathBuf::from(&self.other_repo),
                self.revisions[0].clone(),
            )
            .expect("service")
        }

        fn window(&self, center: HistoryTimelineCenter) -> HistoryTimelineWindow {
            self.service()
                .timeline_window(center, Some(3))
                .expect("timeline window")
        }
    }

    #[test]
    fn catalog_paginates_deterministically_and_preserves_coincident_tags() {
        let fixture = Fixture::new(false);
        let service = fixture.service();
        let first = service.release_catalog(Some(2), None).expect("first page");
        assert_eq!(release_tags(&first.releases), ["v3.0.0", "v2.0.0"]);
        assert_eq!(first.releases[1].coincident_tags, ["v2.0.0", "v2.0.0-lts"]);
        let interval = first.releases[1]
            .interval
            .as_ref()
            .expect("release interval");
        assert_eq!(
            interval.from_exclusive_sha.as_ref(),
            Some(&fixture.revisions[1])
        );
        assert_eq!(interval.commit_count, Some(3));
        assert_eq!(interval.coverage, HistoryCoverageState::Complete);
        let cursor = first.next_cursor.as_ref().expect("cursor");
        let second = service
            .release_catalog(Some(2), Some(cursor))
            .expect("second page");
        assert_eq!(release_tags(&second.releases), ["v2.0.0-lts", "v1.0.0"]);
        assert_eq!(
            service.release_catalog(Some(2), None).expect("repeat"),
            first
        );
        assert!(!second.truncated);
    }

    #[test]
    fn old_release_and_revision_windows_are_exact_and_boundary_aware() {
        let fixture = Fixture::new(false);
        let old = fixture.window(HistoryTimelineCenter::Release {
            tag: "v1.0.0".to_string(),
        });
        assert_eq!(old.center_revision.as_ref(), Some(&fixture.revisions[1]));
        assert_eq!(
            revision_shas(&old),
            fixture.revisions[0..3]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert!(!old.has_older && old.has_newer);

        let middle = fixture.window(HistoryTimelineCenter::Revision {
            revision_sha: fixture.revisions[4].clone(),
        });
        assert_eq!(middle.releases.len(), 2);
        assert!(middle
            .releases
            .iter()
            .all(|release| release.revision_sha == fixture.revisions[4]));
        assert!(middle.has_older && middle.has_newer);

        let last = fixture.window(HistoryTimelineCenter::Revision {
            revision_sha: fixture.revisions[7].clone(),
        });
        assert!(last.has_older && !last.has_newer);
        assert_eq!(
            last.revisions.last().map(|revision| &revision.sha),
            Some(&fixture.revisions[7])
        );
        let next = fixture
            .service()
            .timeline_window(
                HistoryTimelineCenter::Cursor {
                    cursor: old.newer_cursor.expect("newer cursor"),
                },
                Some(3),
            )
            .expect("next window");
        assert_eq!(next.center_revision, Some(fixture.revisions[3].clone()));
    }

    #[test]
    fn cursors_reject_cross_repo_query_and_stale_catalog_reuse() {
        let fixture = Fixture::new(false);
        let service = fixture.service();
        let page = service.release_catalog(Some(2), None).expect("page");
        let cursor = page.next_cursor.as_ref().expect("cursor");
        assert_eq!(
            fixture
                .other_service()
                .release_catalog(Some(2), Some(cursor))
                .unwrap_err(),
            "History cursor does not match this repository or query"
        );
        assert_eq!(
            service.release_catalog(Some(3), Some(cursor)).unwrap_err(),
            "History cursor does not match this repository or query"
        );
        fixture
            .connection
            .execute(
                "UPDATE history_graph_release_catalogs SET index_identity = 'index:new'
                 WHERE repo_path = ?1",
                params![fixture.repo],
            )
            .expect("advance index");
        assert_eq!(
            service.release_catalog(Some(2), Some(cursor)).unwrap_err(),
            "History cursor is stale"
        );
    }

    #[test]
    fn missing_or_non_exact_temporal_references_fail_closed() {
        let fixture = Fixture::new(false);
        let service = fixture.service();
        assert_eq!(
            service
                .timeline_window(
                    HistoryTimelineCenter::Release {
                        tag: "v0.0.0".to_string(),
                    },
                    None,
                )
                .unwrap_err(),
            "Release tag is not indexed for this repository"
        );
        assert_eq!(
            service
                .timeline_window(
                    HistoryTimelineCenter::Revision {
                        revision_sha: "abc123".to_string(),
                    },
                    None,
                )
                .unwrap_err(),
            "A full exact Git revision SHA is required"
        );
        assert_eq!(
            service
                .timeline_window(
                    HistoryTimelineCenter::Revision {
                        revision_sha: sha(999),
                    },
                    None,
                )
                .unwrap_err(),
            "Selected revision is outside indexed history coverage"
        );
    }

    #[test]
    fn partial_coverage_head_drift_and_unknown_current_tags_are_explicit() {
        let fixture = Fixture::new(true);
        let catalog = fixture
            .service()
            .release_catalog(None, None)
            .expect("catalog");
        assert_eq!(catalog.coverage.state, HistoryCoverageState::Partial);
        assert!(!catalog.coverage.ancestry_complete);
        assert!(catalog.coverage.is_shallow);
        assert!(catalog.coverage.truncated);
        assert_eq!(
            catalog.coverage.reasons,
            [
                "release_catalog_partial",
                "ancestry_incomplete",
                "shallow_repository",
                "revision_index_truncated"
            ]
        );
        assert!(!catalog.freshness.stale);
        assert!(catalog.freshness.current_tags_fingerprint.is_none());
        let drifted = HistoryReadService::new_with_current_head(
            &fixture.connection,
            PathBuf::from(&fixture.repo),
            sha(999),
        )
        .expect("drifted service")
        .release_catalog(None, None)
        .expect("drifted catalog");
        assert!(drifted.freshness.stale);
        assert_eq!(drifted.freshness.current_revision, Some(sha(999)));
    }

    fn sha(value: usize) -> String {
        format!("{value:040x}")
    }

    fn release_tags(releases: &[HistoryReleaseCatalogEntry]) -> Vec<&str> {
        releases
            .iter()
            .map(|release| release.tag.as_str())
            .collect()
    }

    fn revision_shas(window: &HistoryTimelineWindow) -> Vec<&str> {
        window
            .revisions
            .iter()
            .map(|revision| revision.sha.as_str())
            .collect()
    }
}
