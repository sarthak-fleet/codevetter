//! Exact archaeology revision context from the persisted history index only.
//!
//! This helper deliberately performs no Git command, checkout, or graph
//! reconstruction. Missing or incompatible persisted facts weaken coverage.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

const MAX_ANCESTRY_REVISIONS: i64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistedTemporalCoverageState {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedReleaseTag {
    pub tag: String,
    pub kind: String,
    pub tagged_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedReleaseInterval {
    pub tag: String,
    pub from_exclusive_revision: Option<String>,
    pub commit_count: Option<i64>,
    pub observed_commit_count: i64,
    pub coverage_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedArchaeologyTemporalContext {
    pub revision_sha: String,
    pub ordinal: Option<i64>,
    pub prior_revision_sha: Option<String>,
    pub prior_is_ancestor: bool,
    pub release_tags: Vec<PersistedReleaseTag>,
    pub release_intervals: Vec<PersistedReleaseInterval>,
    pub coverage_state: PersistedTemporalCoverageState,
    pub coverage_reasons: Vec<String>,
}

pub(crate) fn resolve_archaeology_temporal_context(
    connection: &Connection,
    repo_path: &str,
    revision_sha: &str,
    prior_revision_sha: Option<&str>,
) -> Result<PersistedArchaeologyTemporalContext, String> {
    validate_scope(repo_path, revision_sha, prior_revision_sha)?;
    let mut context = PersistedArchaeologyTemporalContext {
        revision_sha: revision_sha.to_string(),
        ordinal: None,
        prior_revision_sha: prior_revision_sha.map(str::to_string),
        prior_is_ancestor: prior_revision_sha.is_none(),
        release_tags: Vec::new(),
        release_intervals: Vec::new(),
        coverage_state: PersistedTemporalCoverageState::Complete,
        coverage_reasons: Vec::new(),
    };

    let repository = connection
        .query_row(
            "SELECT indexed_head,status,coverage_json
             FROM history_graph_repositories WHERE repo_path=?1",
            [repo_path],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load persisted archaeology history index: {error}"))?;
    let Some((indexed_head, status, repository_coverage)) = repository else {
        partial(&mut context, "history_index_unavailable");
        finish(&mut context);
        return Ok(context);
    };
    if status != "ready" {
        partial(&mut context, "history_index_not_ready");
    }
    if indexed_head.as_deref() != Some(revision_sha) {
        partial(&mut context, "history_index_stale");
    }
    apply_repository_coverage(&mut context, &repository_coverage);

    context.ordinal = connection
        .query_row(
            "SELECT ordinal FROM history_graph_revisions WHERE repo_path=?1 AND sha=?2",
            params![repo_path, revision_sha],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Resolve persisted archaeology history revision: {error}"))?;
    if context.ordinal.is_none() {
        partial(&mut context, "history_revision_missing");
        finish(&mut context);
        return Ok(context);
    }

    apply_release_catalog(connection, repo_path, revision_sha, &mut context)?;
    load_release_context(connection, repo_path, revision_sha, &mut context)?;
    if let Some(prior_revision) = prior_revision_sha {
        let prior_exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history_graph_revisions
                 WHERE repo_path=?1 AND sha=?2)",
                params![repo_path, prior_revision],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("Resolve prior persisted history revision: {error}"))?;
        if !prior_exists {
            partial(&mut context, "prior_history_revision_missing");
        } else {
            apply_ancestry(
                connection,
                repo_path,
                revision_sha,
                prior_revision,
                &mut context,
            )?;
        }
    }
    finish(&mut context);
    Ok(context)
}

fn apply_repository_coverage(context: &mut PersistedArchaeologyTemporalContext, raw: &str) {
    let Ok(coverage) = serde_json::from_str::<Value>(raw) else {
        partial(context, "history_coverage_invalid");
        return;
    };
    if coverage.get("is_shallow").and_then(Value::as_bool) == Some(true) {
        partial(context, "history_shallow");
    }
    if ["history_truncated", "truncated"]
        .iter()
        .any(|key| coverage.get(*key).and_then(Value::as_bool) == Some(true))
    {
        partial(context, "history_truncated");
    }
    if coverage.get("coverage_complete").and_then(Value::as_bool) != Some(true) {
        partial(context, "history_coverage_incomplete");
    }
}

fn apply_release_catalog(
    connection: &Connection,
    repo_path: &str,
    indexed_head: &str,
    context: &mut PersistedArchaeologyTemporalContext,
) -> Result<(), String> {
    let catalog = connection
        .query_row(
            "SELECT indexed_head,status,coverage_json,interval_schema_version,
                    interval_identity
             FROM history_graph_release_catalogs WHERE repo_path=?1",
            [repo_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load persisted release catalog context: {error}"))?;
    let Some((catalog_head, status, raw_coverage, interval_version, interval_identity)) = catalog
    else {
        partial(context, "release_catalog_missing");
        return Ok(());
    };
    if catalog_head != indexed_head {
        partial(context, "release_catalog_stale");
    }
    if status != "ready" {
        partial(context, "release_catalog_partial");
    }
    let Ok(coverage) = serde_json::from_str::<Value>(&raw_coverage) else {
        partial(context, "release_catalog_coverage_invalid");
        return Ok(());
    };
    if coverage.get("ancestry_complete").and_then(Value::as_bool) != Some(true) {
        partial(context, "release_ancestry_incomplete");
    }
    if coverage.get("is_shallow").and_then(Value::as_bool) == Some(true) {
        partial(context, "release_history_shallow");
    }
    if interval_version <= 0 || interval_identity.is_none() {
        partial(context, "release_intervals_unavailable");
    }
    if coverage.get("intervals_complete").and_then(Value::as_bool) != Some(true) {
        partial(context, "release_intervals_incomplete");
    }
    Ok(())
}

fn load_release_context(
    connection: &Connection,
    repo_path: &str,
    revision_sha: &str,
    context: &mut PersistedArchaeologyTemporalContext,
) -> Result<(), String> {
    let mut tags = connection
        .prepare(
            "SELECT tag,tag_kind,tagged_at FROM history_graph_release_tags
             WHERE repo_path=?1 AND revision_sha=?2 ORDER BY tag",
        )
        .map_err(|error| format!("Prepare persisted release tags: {error}"))?;
    context.release_tags = tags
        .query_map(params![repo_path, revision_sha], |row| {
            Ok(PersistedReleaseTag {
                tag: row.get(0)?,
                kind: row.get(1)?,
                tagged_at: row.get(2)?,
            })
        })
        .map_err(|error| format!("Query persisted release tags: {error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("Read persisted release tags: {error}"))?;
    drop(tags);

    let mut intervals = connection
        .prepare(
            "SELECT tag,from_exclusive_sha,commit_count,observed_commit_count,coverage_kind
             FROM history_graph_release_intervals
             WHERE repo_path=?1 AND revision_sha=?2 ORDER BY tag",
        )
        .map_err(|error| format!("Prepare persisted release intervals: {error}"))?;
    context.release_intervals = intervals
        .query_map(params![repo_path, revision_sha], |row| {
            Ok(PersistedReleaseInterval {
                tag: row.get(0)?,
                from_exclusive_revision: row.get(1)?,
                commit_count: row.get(2)?,
                observed_commit_count: row.get(3)?,
                coverage_kind: row.get(4)?,
            })
        })
        .map_err(|error| format!("Query persisted release intervals: {error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("Read persisted release intervals: {error}"))?;
    drop(intervals);

    let interval_reasons = context
        .release_intervals
        .iter()
        .filter_map(|interval| match interval.coverage_kind.as_str() {
            "complete" if interval.commit_count != Some(interval.observed_commit_count) => {
                Some("release_interval_count_mismatch")
            }
            "complete" => None,
            "shallow" => Some("release_interval_shallow"),
            "divergent" => Some("release_interval_divergent"),
            _ => Some("release_interval_coverage_invalid"),
        })
        .collect::<Vec<_>>();
    for reason in interval_reasons {
        partial(context, reason);
    }
    if context.release_tags.len() != context.release_intervals.len()
        || context
            .release_tags
            .iter()
            .zip(&context.release_intervals)
            .any(|(tag, interval)| tag.tag != interval.tag)
    {
        partial(context, "release_interval_missing");
    }
    Ok(())
}

fn apply_ancestry(
    connection: &Connection,
    repo_path: &str,
    revision_sha: &str,
    prior_revision_sha: &str,
    context: &mut PersistedArchaeologyTemporalContext,
) -> Result<(), String> {
    let (ancestor, count, missing_parent, invalid_parents): (bool, i64, bool, bool) = connection
        .query_row(
            "WITH RECURSIVE ancestry(sha) AS (
                 SELECT ?2
                 UNION
                 SELECT CAST(parent.value AS TEXT)
                 FROM ancestry
                 JOIN history_graph_revisions revision
                   ON revision.repo_path=?1 AND revision.sha=ancestry.sha
                 JOIN json_each(CASE WHEN json_valid(revision.parents_json)
                                     THEN revision.parents_json ELSE '[]' END) parent
                   ON parent.type='text'
                 LIMIT ?4
             )
             SELECT EXISTS(SELECT 1 FROM ancestry WHERE sha=?3),
                    COUNT(*),
                    EXISTS(
                      SELECT 1 FROM ancestry
                      JOIN history_graph_revisions revision
                        ON revision.repo_path=?1 AND revision.sha=ancestry.sha
                      JOIN json_each(CASE WHEN json_valid(revision.parents_json)
                                          THEN revision.parents_json ELSE '[]' END) parent
                        ON parent.type='text'
                      LEFT JOIN history_graph_revisions persisted_parent
                        ON persisted_parent.repo_path=?1
                       AND persisted_parent.sha=CAST(parent.value AS TEXT)
                      WHERE persisted_parent.sha IS NULL
                    ),
                    EXISTS(
                      SELECT 1 FROM ancestry
                      JOIN history_graph_revisions revision
                        ON revision.repo_path=?1 AND revision.sha=ancestry.sha
                      WHERE NOT json_valid(revision.parents_json)
                    )
             FROM ancestry",
            params![
                repo_path,
                revision_sha,
                prior_revision_sha,
                MAX_ANCESTRY_REVISIONS + 1
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| format!("Resolve persisted archaeology ancestry: {error}"))?;
    context.prior_is_ancestor = ancestor;
    if count > MAX_ANCESTRY_REVISIONS {
        partial(context, "history_ancestry_bound_exceeded");
    }
    if missing_parent {
        partial(context, "history_parent_missing");
    }
    if invalid_parents {
        partial(context, "history_ancestry_invalid");
    }
    if !ancestor {
        partial(context, "non_ancestral_rebase");
    }
    Ok(())
}

fn validate_scope(
    repo_path: &str,
    revision_sha: &str,
    prior_revision_sha: Option<&str>,
) -> Result<(), String> {
    if repo_path.is_empty() || repo_path.len() > 4_096 || repo_path.contains('\0') {
        return Err("Persisted history repository scope is invalid".into());
    }
    validate_revision(revision_sha)?;
    if let Some(prior) = prior_revision_sha {
        validate_revision(prior)?;
    }
    Ok(())
}

fn validate_revision(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("Persisted history revision must be an exact lowercase Git SHA".into())
    }
}

fn partial(context: &mut PersistedArchaeologyTemporalContext, reason: &str) {
    if context.coverage_state == PersistedTemporalCoverageState::Complete {
        context.coverage_state = PersistedTemporalCoverageState::Partial;
    }
    context.coverage_reasons.push(reason.to_string());
}

fn finish(context: &mut PersistedArchaeologyTemporalContext) {
    context.coverage_reasons.sort();
    context.coverage_reasons.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history_graph_schema::run_migration;

    const REPO: &str = "/history-temporal-fixture";

    #[test]
    fn exact_context_resolves_coincident_tags_and_release_intervals() {
        let connection = database();
        seed_exact_catalog(&connection);
        let context =
            resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), Some(&sha('a')))
                .unwrap();
        assert_eq!(
            context.coverage_state,
            PersistedTemporalCoverageState::Complete
        );
        assert!(context.prior_is_ancestor);
        assert_eq!(
            context
                .release_tags
                .iter()
                .map(|tag| tag.tag.as_str())
                .collect::<Vec<_>>(),
            ["v2.0.0", "v2.0.0-lts"]
        );
        assert_eq!(context.release_intervals.len(), 2);
        assert!(context.release_intervals.iter().all(|interval| {
            interval.from_exclusive_revision.as_deref() == Some(sha('a').as_str())
                && interval.commit_count == Some(1)
                && interval.coverage_kind == "complete"
        }));
    }

    #[test]
    fn missing_indexes_and_revisions_fail_closed() {
        let connection = database();
        let missing =
            resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), None).unwrap();
        assert_eq!(
            missing.coverage_state,
            PersistedTemporalCoverageState::Partial
        );
        assert_eq!(missing.coverage_reasons, ["history_index_unavailable"]);

        seed_exact_catalog(&connection);
        let current =
            resolve_archaeology_temporal_context(&connection, REPO, &sha('c'), None).unwrap();
        assert_eq!(
            current.coverage_state,
            PersistedTemporalCoverageState::Partial
        );
        assert!(current
            .coverage_reasons
            .contains(&"history_revision_missing".into()));
        let prior =
            resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), Some(&sha('c')))
                .unwrap();
        assert_eq!(
            prior.coverage_state,
            PersistedTemporalCoverageState::Partial
        );
        assert!(prior
            .coverage_reasons
            .contains(&"prior_history_revision_missing".into()));
    }

    #[test]
    fn shallow_truncated_and_non_ancestral_history_remain_partial() {
        for (coverage, reason) in [
            (
                r#"{"coverage_complete":false,"is_shallow":true,"truncated":false}"#,
                "history_shallow",
            ),
            (
                r#"{"coverage_complete":false,"is_shallow":false,"history_truncated":true}"#,
                "history_truncated",
            ),
        ] {
            let connection = database();
            seed_exact_catalog(&connection);
            connection
                .execute(
                    "UPDATE history_graph_repositories SET coverage_json=?2 WHERE repo_path=?1",
                    params![REPO, coverage],
                )
                .unwrap();
            let context =
                resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), Some(&sha('a')))
                    .unwrap();
            assert_eq!(
                context.coverage_state,
                PersistedTemporalCoverageState::Partial
            );
            assert!(context.coverage_reasons.iter().any(|item| item == reason));
        }

        let connection = database();
        seed_exact_catalog(&connection);
        connection
            .execute(
                "UPDATE history_graph_revisions SET parents_json='[]'
                 WHERE repo_path=?1 AND sha=?2",
                params![REPO, sha('b')],
            )
            .unwrap();
        let rebased =
            resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), Some(&sha('a')))
                .unwrap();
        assert!(!rebased.prior_is_ancestor);
        assert!(rebased
            .coverage_reasons
            .contains(&"non_ancestral_rebase".into()));
    }

    #[test]
    fn incomplete_release_intervals_weaken_exact_history() {
        for (kind, reason) in [
            ("shallow", "release_interval_shallow"),
            ("divergent", "release_interval_divergent"),
        ] {
            let connection = database();
            seed_exact_catalog(&connection);
            connection
                .execute(
                    "UPDATE history_graph_release_intervals
                     SET coverage_kind=?2,commit_count=NULL WHERE repo_path=?1",
                    params![REPO, kind],
                )
                .unwrap();
            let context =
                resolve_archaeology_temporal_context(&connection, REPO, &sha('b'), Some(&sha('a')))
                    .unwrap();
            assert_eq!(
                context.coverage_state,
                PersistedTemporalCoverageState::Partial
            );
            assert!(context.coverage_reasons.iter().any(|item| item == reason));
        }
    }

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_migration(&connection).unwrap();
        connection
    }

    fn seed_exact_catalog(connection: &Connection) {
        let a = sha('a');
        let b = sha('b');
        connection
            .execute(
                "INSERT INTO history_graph_repositories
                 (repo_path,repository_fingerprint,indexed_head,status,coverage_json,
                  created_at,updated_at)
                 VALUES (?1,'repo',?2,'ready',?3,'now','now')",
                params![
                    REPO,
                    b,
                    r#"{"coverage_complete":true,"is_shallow":false,"truncated":false}"#
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO history_graph_revisions
                 (repo_path,sha,ordinal,committed_at,author_name,subject,parents_json)
                 VALUES (?1,?2,0,'now','Fixture','base','[]'),
                        (?1,?3,1,'now','Fixture','release',json_array(?2))",
                params![REPO, a, b],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO history_graph_release_catalogs
                 (repo_path,index_identity,indexed_head,tags_fingerprint,status,coverage_json,
                  interval_schema_version,interval_identity,updated_at)
                 VALUES (?1,'catalog',?2,'tags','ready',?3,1,'intervals','now')",
                params![
                    REPO,
                    b,
                    r#"{"ancestry_complete":true,"is_shallow":false,"intervals_complete":true}"#
                ],
            )
            .unwrap();
        for tag in ["v2.0.0", "v2.0.0-lts"] {
            connection
                .execute(
                    "INSERT INTO history_graph_fact_tags
                     (repo_path,tag,revision_sha,tag_object_sha,tag_kind,tagged_at)
                     VALUES (?1,?2,?3,?3,'lightweight',1)",
                    params![REPO, tag, b],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO history_graph_release_tags
                     (repo_path,tag,revision_sha,tag_object_sha,tag_kind,tagged_at)
                     VALUES (?1,?2,?3,?3,'lightweight',1)",
                    params![REPO, tag, b],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO history_graph_release_intervals
                     (repo_path,tag,revision_sha,from_exclusive_sha,commit_count,
                      observed_commit_count,coverage_kind)
                     VALUES (?1,?2,?3,?4,1,1,'complete')",
                    params![REPO, tag, b, a],
                )
                .unwrap();
        }
    }

    fn sha(value: char) -> String {
        value.to_string().repeat(40)
    }
}
