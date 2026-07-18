use super::*;
use crate::commands::history_graph::{
    HistoryCoverageState, HistoryOpaqueCursor, HistoryReadFreshness,
};
use crate::commands::structural_graph::types::stable_graph_id;

pub const HISTORY_CONTRIBUTOR_SUMMARY_SCHEMA_VERSION: i64 = 1;
const DEFAULT_CONTRIBUTOR_LIMIT: usize = 20;
const MAX_CONTRIBUTOR_LIMIT: usize = 100;
const MAX_AREAS_PER_CONTRIBUTOR: usize = 8;
const MAX_EVIDENCE_IDS_PER_CONTRIBUTOR: usize = 16;
const MAX_REVISION_REFS_PER_CONTRIBUTOR: usize = 16;
const MAX_INTERVAL_REVISIONS: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryContributorScope {
    ReleaseCycleThrough {
        tag: String,
        to_inclusive: Option<String>,
    },
    ExactInterval {
        from_exclusive: Option<String>,
        to_inclusive: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryContributorSummary {
    pub schema_version: i64,
    pub from_exclusive: Option<String>,
    pub to_inclusive: String,
    pub contributors: Vec<HistoryContributorRow>,
    pub other: HistoryContributorAggregate,
    pub totals: HistoryContributorAggregate,
    pub human_primary_commit_share: f64,
    pub top_human_primary_concentration: f64,
    pub automation_primary_commit_share: f64,
    pub coverage: HistoryCoverageState,
    pub caveats: Vec<String>,
    pub freshness: HistoryReadFreshness,
    pub applied_limit: usize,
    pub applied_offset: usize,
    pub truncated: bool,
    pub next_offset: Option<usize>,
    /// Opaque continuation for new callers. `next_offset` remains for legacy local payloads.
    pub next_cursor: Option<HistoryOpaqueCursor>,
}

impl Default for HistoryContributorSummary {
    fn default() -> Self {
        Self {
            schema_version: HISTORY_CONTRIBUTOR_SUMMARY_SCHEMA_VERSION,
            from_exclusive: None,
            to_inclusive: String::new(),
            contributors: Vec::new(),
            other: HistoryContributorAggregate::default(),
            totals: HistoryContributorAggregate::default(),
            human_primary_commit_share: 0.0,
            top_human_primary_concentration: 0.0,
            automation_primary_commit_share: 0.0,
            coverage: HistoryCoverageState::Unavailable,
            caveats: vec!["contributor_facts_unavailable".to_string()],
            freshness: HistoryReadFreshness::default(),
            applied_limit: 0,
            applied_offset: 0,
            truncated: false,
            next_offset: None,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContributorCursorPayload {
    version: u8,
    scope: String,
    index_identity: String,
    query_identity: String,
    offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HistoryContributorAggregate {
    pub contributor_count: usize,
    pub primary_commits: usize,
    pub coauthor_participations: usize,
    pub additions: u64,
    pub deletions: u64,
    pub active_days: usize,
    pub binary_changes: usize,
    pub generated_changes: usize,
    pub vendored_changes: usize,
    pub merge_commits: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryContributorRow {
    pub contributor_id: String,
    pub display_name: String,
    pub identity_kind: String,
    pub alias_count: usize,
    pub activity: HistoryContributorAggregate,
    pub areas: Vec<String>,
    /// Recent, bounded, exact revisions that back this participation summary.
    /// These are local Git object identifiers, not identity evidence.
    pub revisions: Vec<HistoryContributorRevision>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryContributorRevision {
    pub sha: String,
    pub role: String,
}

#[derive(Default, Clone)]
struct MutableContributor {
    id: String,
    display_name: String,
    identity_kind: String,
    alias_count: usize,
    primary_revisions: BTreeSet<String>,
    coauthor_revisions: BTreeSet<String>,
    revision_ordinals: BTreeMap<String, i64>,
    active_days: BTreeSet<String>,
    additions: u64,
    deletions: u64,
    binary_changes: usize,
    generated_changes: usize,
    vendored_changes: usize,
    merge_commits: usize,
    area_counts: BTreeMap<String, usize>,
}

impl MutableContributor {
    fn aggregate(&self) -> HistoryContributorAggregate {
        HistoryContributorAggregate {
            contributor_count: 1,
            primary_commits: self.primary_revisions.len(),
            coauthor_participations: self.coauthor_revisions.len(),
            additions: self.additions,
            deletions: self.deletions,
            active_days: self.active_days.len(),
            binary_changes: self.binary_changes,
            generated_changes: self.generated_changes,
            vendored_changes: self.vendored_changes,
            merge_commits: self.merge_commits,
        }
    }
}

impl<'a> HistoryReadService<'a> {
    /// Cursor-based contributor page used by Tauri and MCP adapters. It retains
    /// the offset field in the response only so databases written by older app
    /// versions remain readable.
    pub fn contributor_summary_page(
        &self,
        scope: HistoryContributorScope,
        limit: Option<usize>,
        cursor: Option<&HistoryOpaqueCursor>,
    ) -> Result<HistoryContributorSummary, String> {
        let applied_limit = limit
            .unwrap_or(DEFAULT_CONTRIBUTOR_LIMIT)
            .clamp(1, MAX_CONTRIBUTOR_LIMIT);
        let metadata = match self.contributor_metadata()? {
            Some(metadata) => metadata,
            None => {
                return Ok(HistoryContributorSummary {
                    applied_limit,
                    ..HistoryContributorSummary::default()
                })
            }
        };
        let scope_json = serde_json::to_string(&scope)
            .map_err(|error| format!("Encode contributor scope: {error}"))?;
        let query_identity = format!("contributors:v1:scope={scope_json}:limit={applied_limit}");
        let index_identity = metadata.identity();
        let offset = cursor
            .map(|cursor| self.decode_contributor_cursor(cursor, &index_identity, &query_identity))
            .transpose()?
            .unwrap_or_default();
        let mut summary = self.contributor_summary(scope, Some(applied_limit), Some(offset))?;
        summary.next_cursor = summary
            .next_offset
            .map(|next_offset| {
                self.encode_contributor_cursor(&index_identity, &query_identity, next_offset)
            })
            .transpose()?;
        Ok(summary)
    }

    pub fn contributor_summary(
        &self,
        scope: HistoryContributorScope,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<HistoryContributorSummary, String> {
        let applied_limit = limit
            .unwrap_or(DEFAULT_CONTRIBUTOR_LIMIT)
            .clamp(1, MAX_CONTRIBUTOR_LIMIT);
        let metadata = self
            .contributor_metadata()?
            .ok_or_else(|| "Contributor facts are not indexed for this repository".to_string())?;
        let (from_exclusive, to_inclusive, scope_coverage, mut caveats) =
            self.resolve_contributor_scope(scope)?;
        let revisions =
            self.interval_revisions(from_exclusive.as_deref(), &to_inclusive, metadata.partial)?;
        if revisions.is_empty() {
            return Err("Contributor interval contains no indexed revisions".to_string());
        }
        let mut contributors = self.aggregate_contributors(&revisions)?;
        contributors.sort_by(|left, right| {
            right
                .primary_revisions
                .len()
                .cmp(&left.primary_revisions.len())
                .then_with(|| {
                    right
                        .coauthor_revisions
                        .len()
                        .cmp(&left.coauthor_revisions.len())
                })
                .then_with(|| left.display_name.cmp(&right.display_name))
                .then_with(|| left.id.cmp(&right.id))
        });
        let totals = aggregate_many(&contributors);
        let automation_commits = contributors
            .iter()
            .filter(|row| row.identity_kind == "automation")
            .map(|row| row.primary_revisions.len())
            .sum::<usize>();
        let human_commits = contributors
            .iter()
            .filter(|row| row.identity_kind == "human")
            .map(|row| row.primary_revisions.len())
            .sum::<usize>();
        let top_human = contributors
            .iter()
            .filter(|row| row.identity_kind == "human")
            .map(|row| row.primary_revisions.len())
            .max()
            .unwrap_or_default();
        let applied_offset = offset.unwrap_or_default().min(contributors.len());
        let page_end = applied_offset
            .saturating_add(applied_limit)
            .min(contributors.len());
        let page = contributors[applied_offset..page_end].to_vec();
        let other = aggregate_many(
            &contributors
                .iter()
                .enumerate()
                .filter(|(index, _)| *index < applied_offset || *index >= page_end)
                .map(|(_, contributor)| contributor.clone())
                .collect::<Vec<_>>(),
        );
        let rows = page
            .into_iter()
            .map(|contributor| contributor_row(&self.repo_path, contributor))
            .collect::<Vec<_>>();
        let total_primary = totals.primary_commits.max(1) as f64;
        let human_total = human_commits.max(1) as f64;
        caveats.extend(self.interval_caveats(&revisions)?);
        if metadata.partial {
            caveats.push("ancestry_coverage_partial".to_string());
        }
        if metadata.mailmap_fingerprint.is_empty() {
            caveats.push("mailmap_identity_unavailable".to_string());
        } else if metadata.mailmap_fingerprint == stable_graph_id("history-mailmap-v1", "absent") {
            caveats.push("mailmap_not_present".to_string());
        }
        caveats.push("current_tag_freshness_unavailable".to_string());
        caveats.sort();
        caveats.dedup();
        let stale = metadata.indexed_head != self.current_head;
        Ok(HistoryContributorSummary {
            schema_version: HISTORY_CONTRIBUTOR_SUMMARY_SCHEMA_VERSION,
            from_exclusive,
            to_inclusive,
            contributors: rows,
            other,
            totals,
            human_primary_commit_share: human_commits as f64 / total_primary,
            top_human_primary_concentration: top_human as f64 / human_total,
            automation_primary_commit_share: automation_commits as f64 / total_primary,
            coverage: if stale
                || metadata.partial
                || scope_coverage == HistoryCoverageState::Partial
            {
                HistoryCoverageState::Partial
            } else {
                HistoryCoverageState::Complete
            },
            caveats,
            freshness: HistoryReadFreshness {
                indexed_revision: Some(metadata.indexed_head),
                current_revision: Some(self.current_head.clone()),
                indexed_tags_fingerprint: Some(metadata.tags_fingerprint),
                current_tags_fingerprint: None,
                stale,
            },
            applied_limit,
            applied_offset,
            truncated: page_end < contributors.len(),
            next_offset: (page_end < contributors.len()).then_some(page_end),
            next_cursor: None,
        })
    }

    fn resolve_contributor_scope(
        &self,
        scope: HistoryContributorScope,
    ) -> Result<(Option<String>, String, HistoryCoverageState, Vec<String>), String> {
        match scope {
            HistoryContributorScope::ExactInterval {
                from_exclusive,
                to_inclusive,
            } => {
                validate_sha(&to_inclusive)?;
                if let Some(from) = &from_exclusive {
                    validate_sha(from)?;
                }
                Ok((
                    from_exclusive,
                    to_inclusive,
                    HistoryCoverageState::Complete,
                    Vec::new(),
                ))
            }
            HistoryContributorScope::ReleaseCycleThrough { tag, to_inclusive } => {
                let (release_revision, from_exclusive, coverage_kind): (
                    String,
                    Option<String>,
                    String,
                ) = self
                    .connection
                    .query_row(
                        "SELECT revision_sha, from_exclusive_sha, coverage_kind
                         FROM history_graph_release_intervals
                         WHERE repo_path = ?1 AND tag = ?2",
                        params![self.repo_path, tag],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(|error| format!("Resolve contributor release interval: {error}"))?
                    .ok_or_else(|| {
                        "Release interval is not indexed for this repository".to_string()
                    })?;
                if coverage_kind == "divergent" {
                    return Err(
                        "Divergent release cannot resolve an exact contributor interval"
                            .to_string(),
                    );
                }
                let to_inclusive = to_inclusive.unwrap_or(release_revision);
                validate_sha(&to_inclusive)?;
                Ok((
                    from_exclusive,
                    to_inclusive,
                    if coverage_kind == "complete" {
                        HistoryCoverageState::Complete
                    } else {
                        HistoryCoverageState::Partial
                    },
                    if coverage_kind == "complete" {
                        Vec::new()
                    } else {
                        vec![coverage_kind]
                    },
                ))
            }
        }
    }

    fn interval_revisions(
        &self,
        from: Option<&str>,
        to: &str,
        allow_partial: bool,
    ) -> Result<Vec<String>, String> {
        if !self.revision_exists(to)?
            || from.map(|sha| self.revision_exists(sha)).transpose()? == Some(false)
        {
            return Err("Contributor interval revision is not indexed".to_string());
        }
        let (interval, boundary_found) = self.bounded_interval(from, to, allow_partial)?;
        if from.is_some() && !boundary_found {
            return Err("Contributor interval boundary is not an ancestor".to_string());
        }
        let revisions = interval
            .into_iter()
            .map(|(_, revision)| revision)
            .collect::<Vec<_>>();
        if revisions.len() > MAX_INTERVAL_REVISIONS {
            return Err("Contributor interval exceeds its revision bound".to_string());
        }
        Ok(revisions)
    }

    fn revision_exists(&self, revision: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history_graph_revisions
                 WHERE repo_path = ?1 AND sha = ?2)",
                params![self.repo_path, revision],
                |row| row.get(0),
            )
            .map_err(|error| format!("Resolve contributor interval revision: {error}"))
    }

    fn bounded_interval(
        &self,
        from: Option<&str>,
        to: &str,
        allow_partial: bool,
    ) -> Result<(Vec<(i64, String)>, bool), String> {
        let mut statement = self
            .connection
            .prepare(
                "WITH RECURSIVE ancestry(sha) AS (
                    SELECT sha FROM history_graph_revisions
                    WHERE repo_path = ?1 AND sha = ?2
                    UNION
                    SELECT parent.sha
                    FROM ancestry child
                    JOIN history_graph_revisions child_revision
                      ON child_revision.repo_path = ?1 AND child_revision.sha = child.sha
                    JOIN json_each(child_revision.parents_json) edge
                    JOIN history_graph_revisions parent
                      ON parent.repo_path = ?1 AND parent.sha = edge.value
                    WHERE ?3 IS NULL OR child.sha != ?3
                 )
                 SELECT revision.ordinal, ancestry.sha FROM ancestry
                 JOIN history_graph_revisions revision
                   ON revision.repo_path = ?1 AND revision.sha = ancestry.sha
                 ORDER BY revision.ordinal, ancestry.sha LIMIT ?4",
            )
            .map_err(|error| format!("Prepare contributor ancestry walk: {error}"))?;
        let rows = statement
            .query_map(
                params![self.repo_path, to, from, MAX_INTERVAL_REVISIONS + 2],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| format!("Query contributor ancestry walk: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read contributor ancestry walk: {error}"))?;
        let boundary_found = from.is_none()
            || from.is_some_and(|boundary| rows.iter().any(|(_, revision)| revision == boundary));
        let interval = rows
            .into_iter()
            .filter(|(_, revision)| from != Some(revision.as_str()))
            .collect::<Vec<_>>();
        if interval.len() > MAX_INTERVAL_REVISIONS {
            return Err("Contributor interval exceeds its revision bound".to_string());
        }
        if !allow_partial {
            let revision_json = Self::revision_set_json(
                &interval
                    .iter()
                    .map(|(_, revision)| revision.clone())
                    .collect::<Vec<_>>(),
            )?;
            let missing_parent: bool = self
                .connection
                .query_row(
                    "WITH selected(sha) AS (SELECT value FROM json_each(?2))
                     SELECT EXISTS(
                        SELECT 1 FROM selected s
                        JOIN history_graph_revisions child
                          ON child.repo_path = ?1 AND child.sha = s.sha
                        JOIN json_each(child.parents_json) edge
                        LEFT JOIN history_graph_revisions parent
                          ON parent.repo_path = ?1 AND parent.sha = edge.value
                        WHERE parent.sha IS NULL
                     )",
                    params![self.repo_path, revision_json],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Validate contributor ancestry coverage: {error}"))?;
            if missing_parent {
                return Err("Indexed contributor ancestry is incomplete".to_string());
            }
        }
        Ok((interval, boundary_found))
    }

    fn revision_set_json(revisions: &[String]) -> Result<String, String> {
        serde_json::to_string(revisions)
            .map_err(|error| format!("Encode contributor revision set: {error}"))
    }

    fn aggregate_contributors(
        &self,
        revisions: &[String],
    ) -> Result<Vec<MutableContributor>, String> {
        let revision_json = Self::revision_set_json(revisions)?;
        let sql = "WITH selected(sha) AS (SELECT value FROM json_each(?2))
             SELECT rc.revision_sha, rc.role, c.contributor_id, c.display_name, c.identity_kind,
                    c.alias_count, r.ordinal, r.committed_at, json_array_length(r.parents_json), p.path,
                    p.additions, p.deletions, p.binary, p.generated, p.vendored
             FROM selected s
             JOIN history_graph_revision_contributors rc
               ON rc.repo_path = ?1 AND rc.revision_sha = s.sha
             JOIN history_graph_contributors c
               ON c.repo_path = rc.repo_path AND c.contributor_id = rc.contributor_id
             JOIN history_graph_revisions r
               ON r.repo_path = rc.repo_path AND r.sha = rc.revision_sha
             LEFT JOIN history_graph_revision_paths p
               ON p.repo_path = rc.repo_path AND p.revision_sha = rc.revision_sha
             ORDER BY rc.revision_sha, rc.role, c.contributor_id, p.path";
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(|error| format!("Prepare contributor facts: {error}"))?;
        let mut rows = statement
            .query(params![self.repo_path, revision_json])
            .map_err(|error| format!("Query contributor facts: {error}"))?;
        let mut contributors = BTreeMap::<String, MutableContributor>::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Read contributor facts: {error}"))?
        {
            let id: String = row.get(2).map_err(|error| error.to_string())?;
            let role: String = row.get(1).map_err(|error| error.to_string())?;
            let revision: String = row.get(0).map_err(|error| error.to_string())?;
            let display_name: String = row.get(3).map_err(|error| error.to_string())?;
            let identity_kind: String = row.get(4).map_err(|error| error.to_string())?;
            let alias_count: i64 = row.get(5).map_err(|error| error.to_string())?;
            if !privacy_safe_id(&id)
                || display_name.contains('@')
                || display_name.len() > 256
                || !matches!(identity_kind.as_str(), "human" | "automation" | "unknown")
                || !matches!(role.as_str(), "primary" | "coauthor")
            {
                return Err("Indexed contributor identity is not privacy-safe".to_string());
            }
            let alias_count = usize::try_from(alias_count)
                .map_err(|_| "Indexed contributor alias count is invalid".to_string())?;
            let ordinal: i64 = row.get(6).map_err(|error| error.to_string())?;
            let committed_at: String = row.get(7).map_err(|error| error.to_string())?;
            let parent_count: i64 = row.get(8).map_err(|error| error.to_string())?;
            let path: Option<String> = row.get(9).map_err(|error| error.to_string())?;
            let additions: Option<i64> = row.get(10).map_err(|error| error.to_string())?;
            let deletions: Option<i64> = row.get(11).map_err(|error| error.to_string())?;
            let binary: Option<i64> = row.get(12).map_err(|error| error.to_string())?;
            let generated: Option<i64> = row.get(13).map_err(|error| error.to_string())?;
            let vendored: Option<i64> = row.get(14).map_err(|error| error.to_string())?;
            let contributor =
                contributors
                    .entry(id.clone())
                    .or_insert_with(|| MutableContributor {
                        id,
                        display_name,
                        identity_kind,
                        alias_count,
                        ..MutableContributor::default()
                    });
            let active_day = committed_at
                .get(..10)
                .ok_or_else(|| "Indexed contributor timestamp is invalid".to_string())?;
            contributor.active_days.insert(active_day.to_string());
            contributor
                .revision_ordinals
                .insert(revision.clone(), ordinal);
            if role == "primary" {
                let first_for_revision = contributor.primary_revisions.insert(revision.clone());
                if first_for_revision && parent_count > 1 {
                    contributor.merge_commits += 1;
                }
                if let Some(path) = path {
                    contributor.additions = contributor
                        .additions
                        .saturating_add(nonnegative(additions, "additions")?);
                    contributor.deletions = contributor
                        .deletions
                        .saturating_add(nonnegative(deletions, "deletions")?);
                    contributor.binary_changes += flag(binary, "binary")?;
                    contributor.generated_changes += flag(generated, "generated")?;
                    contributor.vendored_changes += flag(vendored, "vendored")?;
                    *contributor.area_counts.entry(area(&path)).or_default() += 1;
                }
            } else {
                contributor.coauthor_revisions.insert(revision);
                if let Some(path) = path {
                    *contributor.area_counts.entry(area(&path)).or_default() += 1;
                }
            }
        }
        Ok(contributors.into_values().collect())
    }

    fn contributor_metadata(&self) -> Result<Option<ContributorMetadata>, String> {
        self.connection
            .query_row(
                "SELECT f.indexed_head, f.tags_fingerprint, f.mailmap_fingerprint,
                        COALESCE(r.status = 'partial', 0)
                 FROM history_graph_fact_catalogs f
                 LEFT JOIN history_graph_release_catalogs r ON r.repo_path = f.repo_path
                 WHERE f.repo_path = ?1 AND f.status = 'ready'",
                [self.repo_path.as_str()],
                |row| {
                    Ok(ContributorMetadata {
                        indexed_head: row.get(0)?,
                        tags_fingerprint: row.get(1)?,
                        mailmap_fingerprint: row.get(2)?,
                        partial: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("Load contributor index metadata: {error}"))
    }

    fn encode_contributor_cursor(
        &self,
        index_identity: &str,
        query_identity: &str,
        offset: usize,
    ) -> Result<HistoryOpaqueCursor, String> {
        let payload = ContributorCursorPayload {
            version: 1,
            scope: stable_graph_id("contributor-cursor-scope", &self.repo_path),
            index_identity: index_identity.to_string(),
            query_identity: query_identity.to_string(),
            offset,
        };
        super::encode_opaque_cursor(&payload, "contributor cursor")
    }

    fn decode_contributor_cursor(
        &self,
        cursor: &HistoryOpaqueCursor,
        index_identity: &str,
        query_identity: &str,
    ) -> Result<usize, String> {
        let payload: ContributorCursorPayload = super::decode_opaque_cursor(cursor)?;
        if payload.version != 1 {
            return Err("Invalid history cursor".to_string());
        }
        if payload.scope != stable_graph_id("contributor-cursor-scope", &self.repo_path)
            || payload.query_identity != query_identity
        {
            return Err("History cursor does not match this repository or query".to_string());
        }
        if payload.index_identity != index_identity {
            return Err("History cursor is stale".to_string());
        }
        Ok(payload.offset)
    }

    fn interval_caveats(&self, revisions: &[String]) -> Result<Vec<String>, String> {
        let revision_json = Self::revision_set_json(revisions)?;
        let sql =
            "WITH selected(sha) AS (SELECT value FROM json_each(?2))
             SELECT
                EXISTS(SELECT 1 FROM selected s JOIN history_graph_revisions r
                    ON r.repo_path = ?1 AND r.sha = s.sha WHERE json_array_length(r.parents_json) > 1),
                EXISTS(SELECT 1 FROM selected s JOIN history_graph_revision_paths p
                    ON p.repo_path = ?1 AND p.revision_sha = s.sha WHERE p.binary = 1),
                EXISTS(SELECT 1 FROM selected s JOIN history_graph_revision_paths p
                    ON p.repo_path = ?1 AND p.revision_sha = s.sha WHERE p.generated = 1),
                EXISTS(SELECT 1 FROM selected s JOIN history_graph_revision_paths p
                    ON p.repo_path = ?1 AND p.revision_sha = s.sha WHERE p.vendored = 1)";
        let flags: (bool, bool, bool, bool) = self
            .connection
            .query_row(sql, params![self.repo_path, revision_json], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|error| format!("Load contributor coverage caveats: {error}"))?;
        Ok([
            (flags.0, "merge_commits_present"),
            (flags.1, "binary_churn_unavailable"),
            (flags.2, "generated_paths_present"),
            (flags.3, "vendored_paths_present"),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .map(|(_, caveat)| caveat.to_string())
        .collect())
    }
}

struct ContributorMetadata {
    indexed_head: String,
    tags_fingerprint: String,
    mailmap_fingerprint: String,
    partial: bool,
}

impl ContributorMetadata {
    fn identity(&self) -> String {
        stable_graph_id(
            "history-contributor-index-v1",
            &format!(
                "{}\0{}\0{}",
                self.indexed_head, self.tags_fingerprint, self.mailmap_fingerprint
            ),
        )
    }
}

fn contributor_row(repo_path: &str, contributor: MutableContributor) -> HistoryContributorRow {
    let activity = contributor.aggregate();
    let mut areas = contributor.area_counts.into_iter().collect::<Vec<_>>();
    areas.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    areas.truncate(MAX_AREAS_PER_CONTRIBUTOR);
    let mut evidence_ids = contributor
        .primary_revisions
        .iter()
        .chain(&contributor.coauthor_revisions)
        .map(|revision| {
            stable_graph_id(
                "history-contribution",
                &format!("{repo_path}\0{}\0{revision}", contributor.id),
            )
        })
        .collect::<Vec<_>>();
    evidence_ids.sort();
    evidence_ids.dedup();
    evidence_ids.truncate(MAX_EVIDENCE_IDS_PER_CONTRIBUTOR);
    let mut revisions = contributor
        .primary_revisions
        .iter()
        .chain(&contributor.coauthor_revisions)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|sha| HistoryContributorRevision {
            sha: sha.clone(),
            role: if contributor.primary_revisions.contains(sha) {
                "primary".to_string()
            } else {
                "coauthor".to_string()
            },
        })
        .collect::<Vec<_>>();
    revisions.sort_by(|left, right| {
        contributor
            .revision_ordinals
            .get(&right.sha)
            .cmp(&contributor.revision_ordinals.get(&left.sha))
            .then_with(|| left.sha.cmp(&right.sha))
            .then_with(|| left.role.cmp(&right.role))
    });
    revisions.truncate(MAX_REVISION_REFS_PER_CONTRIBUTOR);
    HistoryContributorRow {
        contributor_id: contributor.id,
        display_name: contributor.display_name,
        identity_kind: contributor.identity_kind,
        alias_count: contributor.alias_count,
        activity,
        areas: areas.into_iter().map(|(area, _)| area).collect(),
        revisions,
        evidence_ids,
    }
}

fn aggregate_many(values: &[MutableContributor]) -> HistoryContributorAggregate {
    values.iter().fold(
        HistoryContributorAggregate::default(),
        |mut total, value| {
            let item = value.aggregate();
            total.primary_commits += item.primary_commits;
            total.contributor_count += item.contributor_count;
            total.coauthor_participations += item.coauthor_participations;
            total.additions = total.additions.saturating_add(item.additions);
            total.deletions = total.deletions.saturating_add(item.deletions);
            total.active_days += item.active_days;
            total.binary_changes += item.binary_changes;
            total.generated_changes += item.generated_changes;
            total.vendored_changes += item.vendored_changes;
            total.merge_commits += item.merge_commits;
            total
        },
    )
}

fn validate_sha(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("An exact full revision SHA is required".to_string())
    }
}

fn nonnegative(value: Option<i64>, label: &str) -> Result<u64, String> {
    match value {
        Some(value) => {
            u64::try_from(value).map_err(|_| format!("Indexed contributor {label} is negative"))
        }
        None => Ok(0),
    }
}

fn flag(value: Option<i64>, label: &str) -> Result<usize, String> {
    match value {
        Some(0) => Ok(0),
        Some(1) => Ok(1),
        _ => Err(format!("Indexed contributor {label} flag is invalid")),
    }
}

fn privacy_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '@' | '/' | '\\'))
}

fn area(path: &str) -> String {
    path.split('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("root")
        .to_string()
}

#[cfg(test)]
#[path = "contributors_tests.rs"]
mod tests;
