use super::history_facts::{
    history_facts_fingerprint, read_all_history_facts, read_history_facts_since,
    HistoryAutomationKind, HistoryFactsBatch, HistoryIdentityFact, HistoryPathFact,
    HistoryPathStatus, HistoryRevisionFact, HISTORY_FACTS_SCHEMA_VERSION,
    HISTORY_FACT_CLASSIFICATION_VERSION,
};
use super::*;

#[derive(Clone)]
pub(super) struct HistoryTimelineBuild {
    pub(super) timeline: HistoryTimeline,
    /// The normalized fact reader uses one bounded `git log` process for a
    /// full or fast-forward history walk. Retaining this in the build result
    /// lets qualification report that invariant without launching another
    /// history scan just to count processes.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed only by history qualification tests")
    )]
    pub(super) fact_git_process_count: usize,
    facts_by_revision: HashMap<String, HistoryRevisionFact>,
    mailmap_fingerprint: String,
    facts_fingerprint: String,
}

impl HistoryTimelineBuild {
    pub(super) fn path_changes_between(
        &self,
        before_revision: &str,
        after_revision: &str,
    ) -> Option<Vec<HistoryPathChange>> {
        let revision = self.facts_by_revision.get(after_revision)?;
        if revision.parents.first().map(String::as_str) != Some(before_revision) {
            return None;
        }
        revision
            .paths
            .iter()
            .map(|path| {
                Some(HistoryPathChange {
                    path: path.path.clone(),
                    change_kind: match path.status {
                        HistoryPathStatus::Added => "added",
                        HistoryPathStatus::Copied => "copied",
                        HistoryPathStatus::Deleted => "deleted",
                        HistoryPathStatus::Modified => "modified",
                        HistoryPathStatus::Renamed => "renamed",
                        HistoryPathStatus::TypeChanged => "type_changed",
                        HistoryPathStatus::Unmerged => "unmerged",
                        HistoryPathStatus::Unknown => return None,
                    }
                    .to_string(),
                    old_path: path.old_path.clone(),
                    additions: path.additions.and_then(|value| value.try_into().ok()),
                    deletions: path.deletions.and_then(|value| value.try_into().ok()),
                })
            })
            .collect()
    }
}

pub fn load_history_revisions(
    connection: &Connection,
    repo_path: &str,
    query: Option<&str>,
    releases_only: bool,
    limit: usize,
) -> Result<HistorySearchResult, String> {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let mut statement = connection
        .prepare(
            "SELECT sha, substr(sha, 1, 8), parents_json, committed_at, author_name,
                    subject, tags_json, is_release, is_head, ordinal
             FROM history_graph_revisions
             WHERE repo_path = ?1
               AND (?2 = 0 OR is_release = 1)
               AND (?3 = '' OR lower(subject) LIKE '%' || ?3 || '%'
                    OR lower(author_name) LIKE '%' || ?3 || '%'
                    OR lower(tags_json) LIKE '%' || ?3 || '%'
                    OR lower(sha) LIKE ?3 || '%')
             ORDER BY ordinal DESC
             LIMIT ?4",
        )
        .map_err(|error| format!("Prepare history query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repo_path,
                i64::from(releases_only),
                query,
                (limit + 1) as i64
            ],
            |row| {
                let parents_json: String = row.get(2)?;
                let tags_json: String = row.get(6)?;
                Ok(HistoryRevision {
                    sha: row.get(0)?,
                    short_sha: row.get(1)?,
                    parents: serde_json::from_str(&parents_json).unwrap_or_default(),
                    committed_at: row.get(3)?,
                    author: row.get(4)?,
                    subject: row.get(5)?,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    is_release: row.get::<_, i64>(7)? != 0,
                    is_head: row.get::<_, i64>(8)? != 0,
                    ordinal: row.get(9)?,
                })
            },
        )
        .map_err(|error| format!("Query history revisions: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read history revisions: {error}"))?;
    let truncated = rows.len() > limit;
    let mut revisions = rows;
    revisions.truncate(limit);
    Ok(HistorySearchResult {
        revisions,
        truncated,
    })
}

/// Loads the bounded playback timeline from normalized local facts. This is the
/// ordinary path after indexing and deliberately performs no Git work.
pub(super) fn load_indexed_timeline(
    connection: &Connection,
    repo_path: &str,
    limit: Option<usize>,
) -> Result<Option<HistoryTimeline>, String> {
    let limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    let metadata = connection
        .query_row(
            "SELECT repository.indexed_head, repository.coverage_json, repository.updated_at
             FROM history_graph_repositories repository
             JOIN history_graph_fact_catalogs facts ON facts.repo_path = repository.repo_path
             WHERE repository.repo_path = ?1
               AND repository.status = 'ready'
               AND facts.status = 'ready'",
            [repo_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load indexed history timeline metadata: {error}"))?;
    let Some((head, coverage_json, generated_at)) = metadata else {
        return Ok(None);
    };
    let total_commits = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_revisions WHERE repo_path = ?1",
            [repo_path],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Count indexed history revisions: {error}"))?;
    let total_commits = usize::try_from(total_commits)
        .map_err(|_| "Indexed history revision count is invalid".to_string())?;
    let mut revisions = indexed_timeline_revisions(connection, repo_path, limit)?;
    let mut present = revisions
        .iter()
        .map(|revision| revision.sha.clone())
        .collect::<HashSet<_>>();
    for revision in indexed_release_revisions(connection, repo_path)? {
        if present.insert(revision.sha.clone()) {
            revisions.push(revision);
        }
    }
    revisions.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.sha.cmp(&right.sha))
    });
    let coverage = serde_json::from_str::<serde_json::Value>(&coverage_json).unwrap_or_default();
    let is_shallow = coverage
        .get("is_shallow")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let truncated = total_commits > revisions.len();
    let release_ranges = release_ranges(&revisions, &head);
    Ok(Some(HistoryTimeline {
        schema_version: 1,
        repo_path: repo_path.to_string(),
        head,
        generated_at,
        revisions: revisions.clone(),
        total_commits,
        truncated,
        is_shallow,
        coverage_complete: !is_shallow && !truncated,
        release_ranges,
        reachable_revisions: revisions.into_iter().map(|revision| revision.sha).collect(),
    }))
}

fn indexed_timeline_revisions(
    connection: &Connection,
    repo_path: &str,
    limit: usize,
) -> Result<Vec<HistoryRevision>, String> {
    let mut statement = connection
        .prepare(
            "SELECT sha, substr(sha, 1, 8), parents_json, committed_at, author_name,
                    subject, tags_json, is_release, is_head, ordinal
             FROM history_graph_revisions
             WHERE repo_path = ?1 ORDER BY ordinal DESC LIMIT ?2",
        )
        .map_err(|error| format!("Prepare indexed history timeline: {error}"))?;
    let mut revisions = statement
        .query_map(params![repo_path, limit as i64], indexed_timeline_revision)
        .map_err(|error| format!("Query indexed history timeline: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read indexed history timeline: {error}"))?;
    revisions.reverse();
    Ok(revisions)
}

fn indexed_release_revisions(
    connection: &Connection,
    repo_path: &str,
) -> Result<Vec<HistoryRevision>, String> {
    let mut statement = connection
        .prepare(
            "SELECT sha, substr(sha, 1, 8), parents_json, committed_at, author_name,
                    subject, tags_json, is_release, is_head, ordinal
             FROM history_graph_revisions
             WHERE repo_path = ?1 AND is_release = 1
             ORDER BY ordinal DESC LIMIT ?2",
        )
        .map_err(|error| format!("Prepare indexed release timeline: {error}"))?;
    let revisions = statement
        .query_map(
            params![repo_path, MAX_HISTORY_LIMIT as i64],
            indexed_timeline_revision,
        )
        .map_err(|error| format!("Query indexed release timeline: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read indexed release timeline: {error}"))?;
    Ok(revisions)
}

fn indexed_timeline_revision(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRevision> {
    let parents_json: String = row.get(2)?;
    let tags_json: String = row.get(6)?;
    Ok(HistoryRevision {
        sha: row.get(0)?,
        short_sha: row.get(1)?,
        parents: serde_json::from_str(&parents_json).unwrap_or_default(),
        committed_at: row.get(3)?,
        author: row.get(4)?,
        subject: row.get(5)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        is_release: row.get::<_, i64>(7)? != 0,
        is_head: row.get::<_, i64>(8)? != 0,
        ordinal: row.get(9)?,
    })
}

/// Rehydrates the privacy-preserving normalized facts required to extend a
/// fast-forward history index. The catalog deliberately stores no raw email,
/// so a changed mailmap is handled by the full-rebuild path instead.
fn load_indexed_history_facts(
    connection: &Connection,
    repo_path: &str,
    expected_head: &str,
) -> Result<HistoryFactsBatch, String> {
    let metadata = connection
        .query_row(
            "SELECT schema_version, classification_version, indexed_head,
                    mailmap_fingerprint, facts_fingerprint
             FROM history_graph_fact_catalogs
             WHERE repo_path = ?1 AND status = 'ready'",
            [repo_path],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load indexed history facts metadata: {error}"))?
        .ok_or_else(|| "Fast-forward history facts are unavailable".to_string())?;
    if metadata.0 != HISTORY_FACTS_SCHEMA_VERSION
        || metadata.1 != HISTORY_FACT_CLASSIFICATION_VERSION
        || metadata.2 != expected_head
    {
        return Err(
            "Fast-forward history facts are incompatible with the stored cursor".to_string(),
        );
    }

    let mut revisions = connection
        .prepare(
            "SELECT revision.sha, revision.parents_json, revision.committed_at, revision.subject,
                    primary_contributor.contributor_id, primary_contributor.display_name,
                    primary_contributor.identity_kind, primary_contributor.alias_count,
                    revision.tags_json, revision.is_head
             FROM history_graph_revisions revision
             JOIN history_graph_revision_contributors primary_role
               ON primary_role.repo_path = revision.repo_path
              AND primary_role.revision_sha = revision.sha
              AND primary_role.role = 'primary'
             JOIN history_graph_contributors primary_contributor
               ON primary_contributor.repo_path = primary_role.repo_path
              AND primary_contributor.contributor_id = primary_role.contributor_id
             WHERE revision.repo_path = ?1
             ORDER BY revision.ordinal ASC",
        )
        .map_err(|error| format!("Prepare indexed history fact revisions: {error}"))?
        .query_map([repo_path], |row| {
            let parents_json: String = row.get(1)?;
            let tags_json: String = row.get(8)?;
            let parents: Vec<String> = serde_json::from_str(&parents_json).unwrap_or_default();
            Ok(HistoryRevisionFact {
                sha: row.get(0)?,
                is_merge: parents.len() > 1,
                parents,
                committed_at: row.get(2)?,
                subject: row.get(3)?,
                primary: HistoryIdentityFact {
                    contributor_id: row.get(4)?,
                    display_name: row.get(5)?,
                    automation: automation_kind_from_db(&row.get::<_, String>(6)?),
                    alias_count: usize::try_from(row.get::<_, i64>(7)?).unwrap_or_default(),
                },
                coauthors: Vec::new(),
                malformed_coauthor_count: 0,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                paths: Vec::new(),
                is_head: row.get::<_, i64>(9)? != 0,
            })
        })
        .map_err(|error| format!("Query indexed history fact revisions: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read indexed history fact revisions: {error}"))?;
    if revisions.is_empty() {
        return Err("Fast-forward history facts have no revisions".to_string());
    }
    let positions = revisions
        .iter()
        .enumerate()
        .map(|(index, revision)| (revision.sha.clone(), index))
        .collect::<HashMap<_, _>>();

    let mut contributor_statement = connection
        .prepare(
            "SELECT role.revision_sha, contributor.contributor_id, contributor.display_name,
                    contributor.identity_kind, contributor.alias_count
             FROM history_graph_revision_contributors role
             JOIN history_graph_contributors contributor
               ON contributor.repo_path = role.repo_path
              AND contributor.contributor_id = role.contributor_id
             WHERE role.repo_path = ?1 AND role.role = 'coauthor'
             ORDER BY role.revision_sha, contributor.contributor_id",
        )
        .map_err(|error| format!("Prepare indexed history coauthors: {error}"))?;
    let coauthors = contributor_statement
        .query_map([repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                HistoryIdentityFact {
                    contributor_id: row.get(1)?,
                    display_name: row.get(2)?,
                    automation: automation_kind_from_db(&row.get::<_, String>(3)?),
                    alias_count: usize::try_from(row.get::<_, i64>(4)?).unwrap_or_default(),
                },
            ))
        })
        .map_err(|error| format!("Query indexed history coauthors: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read indexed history coauthors: {error}"))?;
    drop(contributor_statement);
    for (sha, contributor) in coauthors {
        let index = positions
            .get(&sha)
            .ok_or_else(|| format!("Indexed coauthor points at unknown revision {sha}"))?;
        revisions[*index].coauthors.push(contributor);
    }

    let mut path_statement = connection
        .prepare(
            "SELECT revision_sha, path, old_path, change_kind, additions, deletions,
                    binary, generated, vendored
             FROM history_graph_revision_paths
             WHERE repo_path = ?1
             ORDER BY revision_sha, path, old_path",
        )
        .map_err(|error| format!("Prepare indexed history paths: {error}"))?;
    let paths = path_statement
        .query_map([repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                HistoryPathFact {
                    path: row.get(1)?,
                    old_path: row.get(2)?,
                    status: path_status_from_db(&row.get::<_, String>(3)?),
                    additions: row
                        .get::<_, Option<i64>>(4)?
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, 0))?,
                    deletions: row
                        .get::<_, Option<i64>>(5)?
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, 0))?,
                    binary: row.get::<_, i64>(6)? != 0,
                    generated: row.get::<_, i64>(7)? != 0,
                    vendored: row.get::<_, i64>(8)? != 0,
                },
            ))
        })
        .map_err(|error| format!("Query indexed history paths: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read indexed history paths: {error}"))?;
    drop(path_statement);
    for (sha, path) in paths {
        let index = positions
            .get(&sha)
            .ok_or_else(|| format!("Indexed path points at unknown revision {sha}"))?;
        revisions[*index].paths.push(path);
    }
    let batch = HistoryFactsBatch {
        schema_version: HISTORY_FACTS_SCHEMA_VERSION,
        classification_version: HISTORY_FACT_CLASSIFICATION_VERSION,
        git_process_count: 1,
        mailmap_fingerprint: metadata.3,
        facts_fingerprint: metadata.4,
        revisions,
    };
    batch.validate()?;
    Ok(batch)
}

fn automation_kind_from_db(value: &str) -> HistoryAutomationKind {
    match value {
        "human" => HistoryAutomationKind::Human,
        "automation" => HistoryAutomationKind::Automation,
        _ => HistoryAutomationKind::Unknown,
    }
}

fn path_status_from_db(value: &str) -> HistoryPathStatus {
    match value {
        "added" => HistoryPathStatus::Added,
        "copied" => HistoryPathStatus::Copied,
        "deleted" => HistoryPathStatus::Deleted,
        "modified" => HistoryPathStatus::Modified,
        "renamed" => HistoryPathStatus::Renamed,
        "type_changed" => HistoryPathStatus::TypeChanged,
        "unmerged" => HistoryPathStatus::Unmerged,
        _ => HistoryPathStatus::Unknown,
    }
}

pub(super) fn build_timeline(root: &Path, limit: Option<usize>) -> Result<HistoryTimeline, String> {
    let tag_records = read_git_tags(root)?;
    build_timeline_with_tags(root, limit, &tag_records)
}

pub(super) fn build_timeline_with_tags(
    root: &Path,
    limit: Option<usize>,
    tag_records: &[GitTagRecord],
) -> Result<HistoryTimeline, String> {
    Ok(build_timeline_bundle_with_tags_cancellable(
        root,
        limit,
        tag_records,
        &StructuralGraphCancellation::default(),
    )?
    .timeline)
}

pub(super) fn build_timeline_bundle_with_tags_cancellable(
    root: &Path,
    limit: Option<usize>,
    tag_records: &[GitTagRecord],
    cancellation: &StructuralGraphCancellation,
) -> Result<HistoryTimelineBuild, String> {
    let facts = read_all_history_facts(root, cancellation)?;
    build_timeline_bundle_from_facts(root, limit, tag_records, facts)
}

/// Builds a complete timeline from the local normalized catalog plus only the
/// commits introduced after an already-indexed fast-forward cursor. No
/// all-history Git walk occurs on this path.
pub(super) fn build_incremental_timeline_bundle_with_tags_cancellable(
    connection: &Connection,
    root: &Path,
    limit: Option<usize>,
    tag_records: &[GitTagRecord],
    previous_head: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<(HistoryTimelineBuild, HashSet<String>), String> {
    let repo_path = root.to_string_lossy();
    let mut indexed = load_indexed_history_facts(connection, &repo_path, previous_head)?;
    let introduced = read_history_facts_since(root, previous_head, cancellation)?;
    introduced.validate()?;
    let introduced_shas = introduced
        .revisions
        .iter()
        .map(|revision| revision.sha.clone())
        .collect::<HashSet<_>>();
    if introduced_shas.is_empty() {
        return Err("Fast-forward history refresh did not contain a new HEAD revision".to_string());
    }
    for revision in &mut indexed.revisions {
        revision.is_head = false;
    }
    indexed.revisions.extend(introduced.revisions);
    indexed.facts_fingerprint = history_facts_fingerprint(&indexed.revisions);
    build_timeline_bundle_from_facts(root, limit, tag_records, indexed)
        .map(|build| (build, introduced_shas))
}

/// Rebuilds derived timeline metadata from already-indexed facts when only tag
/// metadata changed. It intentionally performs no history Git traversal.
pub(super) fn build_indexed_timeline_bundle_with_tags(
    connection: &Connection,
    root: &Path,
    limit: Option<usize>,
    tag_records: &[GitTagRecord],
    expected_head: &str,
) -> Result<HistoryTimelineBuild, String> {
    let repo_path = root.to_string_lossy();
    let facts = load_indexed_history_facts(connection, &repo_path, expected_head)?;
    build_timeline_bundle_from_facts(root, limit, tag_records, facts)
}

fn build_timeline_bundle_from_facts(
    root: &Path,
    limit: Option<usize>,
    tag_records: &[GitTagRecord],
    facts: HistoryFactsBatch,
) -> Result<HistoryTimelineBuild, String> {
    let limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    facts.validate()?;
    let head = facts
        .revisions
        .iter()
        .find(|revision| revision.is_head)
        .map(|revision| revision.sha.clone())
        .ok_or_else(|| "Batched history facts did not identify HEAD".to_string())?;
    let tags = tags_by_commit_from_records(tag_records);
    let total_commits = facts.revisions.len();
    let is_shallow = git_text(root, &["rev-parse", "--is-shallow-repository"])? == "true";
    let ordinals = facts
        .revisions
        .iter()
        .enumerate()
        .map(|(ordinal, revision)| (revision.sha.clone(), ordinal as i64))
        .collect::<HashMap<_, _>>();
    let all_revisions = facts
        .revisions
        .iter()
        .enumerate()
        .map(|(ordinal, revision)| {
            (
                revision.sha.clone(),
                timeline_revision(revision, ordinal as i64, &tags, &head),
            )
        })
        .collect::<HashMap<_, _>>();
    let recent_start = total_commits.saturating_sub(limit);
    let mut revisions = facts.revisions[recent_start..]
        .iter()
        .filter_map(|revision| all_revisions.get(&revision.sha).cloned())
        .collect::<Vec<_>>();
    let mut present = revisions
        .iter()
        .map(|revision| revision.sha.clone())
        .collect::<HashSet<_>>();
    let missing_releases = tags
        .iter()
        .filter(|(_, values)| values.iter().any(|tag| is_release_tag(tag)))
        .map(|(sha, _)| sha)
        .filter(|sha| !present.contains(*sha) && all_revisions.contains_key(*sha))
        .cloned()
        .collect::<Vec<_>>();
    for sha in missing_releases {
        if let Some(revision) = all_revisions.get(&sha).cloned() {
            present.insert(revision.sha.clone());
            revisions.push(revision);
        }
    }
    revisions.sort_by(|left, right| {
        ordinals
            .get(&left.sha)
            .cmp(&ordinals.get(&right.sha))
            .then_with(|| left.sha.cmp(&right.sha))
    });
    let release_ranges = release_ranges(&revisions, &head);
    let truncated = total_commits > revisions.len();
    let reachable_revisions = facts
        .revisions
        .iter()
        .map(|revision| revision.sha.clone())
        .collect();
    let mailmap_fingerprint = facts.mailmap_fingerprint.clone();
    let facts_fingerprint = facts.facts_fingerprint.clone();
    let fact_git_process_count = facts.git_process_count;
    let facts_by_revision = facts
        .revisions
        .into_iter()
        .map(|revision| (revision.sha.clone(), revision))
        .collect();
    Ok(HistoryTimelineBuild {
        timeline: HistoryTimeline {
            schema_version: 1,
            repo_path: root.to_string_lossy().to_string(),
            head,
            generated_at: Utc::now().to_rfc3339(),
            truncated,
            is_shallow,
            coverage_complete: !is_shallow && !truncated,
            release_ranges,
            total_commits,
            revisions,
            reachable_revisions,
        },
        fact_git_process_count,
        facts_by_revision,
        mailmap_fingerprint,
        facts_fingerprint,
    })
}

fn timeline_revision(
    fact: &HistoryRevisionFact,
    ordinal: i64,
    tags: &HashMap<String, Vec<String>>,
    head: &str,
) -> HistoryRevision {
    let revision_tags = tags.get(&fact.sha).cloned().unwrap_or_default();
    HistoryRevision {
        sha: fact.sha.clone(),
        short_sha: fact.sha[..8].to_string(),
        parents: fact.parents.clone(),
        committed_at: fact.committed_at.clone(),
        author: fact.primary.display_name.clone(),
        subject: fact.subject.clone(),
        is_release: revision_tags.iter().any(|tag| is_release_tag(tag)),
        is_head: fact.sha == head,
        tags: revision_tags,
        ordinal,
    }
}

#[cfg(test)]
pub(super) fn revision_ordinals(root: &Path) -> Result<HashMap<String, i64>, String> {
    let output = git_text(root, &["rev-list", "--topo-order", "--reverse", "HEAD"])?;
    Ok(output
        .lines()
        .filter(|sha| !sha.is_empty())
        .enumerate()
        .map(|(ordinal, sha)| (sha.to_string(), ordinal as i64))
        .collect())
}

pub(super) fn release_ranges(
    revisions: &[HistoryRevision],
    head: &str,
) -> Vec<HistoryReleaseRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut previous_release = None::<String>;
    for (index, revision) in revisions.iter().enumerate() {
        if !revision.is_release {
            continue;
        }
        let tag = revision
            .tags
            .iter()
            .find(|tag| is_release_tag(tag))
            .cloned();
        let label = tag
            .clone()
            .unwrap_or_else(|| format!("Release {}", revision.short_sha));
        ranges.push(HistoryReleaseRange {
            id: stable_graph_id(
                "release-range",
                &format!("{}\0{}", revision.sha, tag.as_deref().unwrap_or_default()),
            ),
            label,
            tag,
            from_exclusive: previous_release.clone(),
            to_inclusive: revision.sha.clone(),
            commit_shas: revisions[start..=index]
                .iter()
                .map(|commit| commit.sha.clone())
                .collect(),
            is_unreleased: false,
        });
        start = index + 1;
        previous_release = Some(revision.sha.clone());
    }
    ranges.push(HistoryReleaseRange {
        id: stable_graph_id(
            "release-range",
            &format!(
                "unreleased\0{}",
                previous_release.as_deref().unwrap_or("root")
            ),
        ),
        label: "Unreleased".to_string(),
        tag: None,
        from_exclusive: previous_release,
        to_inclusive: head.to_string(),
        commit_shas: revisions[start..]
            .iter()
            .map(|commit| commit.sha.clone())
            .collect(),
        is_unreleased: true,
    });
    ranges
}

pub(super) fn timeline_tag_fingerprint(timeline: &HistoryTimeline) -> String {
    let tag_identity = timeline
        .revisions
        .iter()
        .flat_map(|revision| {
            revision
                .tags
                .iter()
                .map(move |tag| format!("{}\0{tag}", revision.sha))
        })
        .collect::<Vec<_>>()
        .join("\0");
    stable_graph_id("tags", &tag_identity)
}

pub(crate) fn repository_tag_fingerprint(root: &Path) -> Result<String, String> {
    Ok(release_tag_fingerprint(&read_git_tags(root)?))
}

pub(super) fn release_tag_fingerprint(tags: &[GitTagRecord]) -> String {
    let mut tag_identity = tags
        .iter()
        .filter(|tag| is_release_tag(&tag.name))
        .map(|tag| {
            format!(
                "{}\0{}\0{}\0{}",
                tag.name, tag.object_sha, tag.commit_sha, tag.created_ts
            )
        })
        .collect::<Vec<_>>();
    tag_identity.sort();
    stable_graph_id("tags", &tag_identity.join("\0"))
}

pub(super) fn classify_history_refresh(
    previous_head: Option<&str>,
    rewritten: bool,
    engine_incompatible: bool,
    fast_forward: bool,
    tags_changed: bool,
) -> &'static str {
    if previous_head.is_none() {
        "initial"
    } else if rewritten {
        "rewritten_history"
    } else if engine_incompatible {
        "engine_repair"
    } else if fast_forward {
        "fast_forward"
    } else if tags_changed {
        "tag_metadata"
    } else {
        "no_op"
    }
}

pub(super) fn has_incompatible_history_checkpoints(
    connection: &Connection,
    repo_path: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM history_graph_checkpoints
                WHERE repo_path = ?1
                  AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4
                       OR EXISTS(
                           SELECT 1 FROM structural_graph_snapshots snapshot
                           WHERE snapshot.id = history_graph_checkpoints.snapshot_id
                             AND snapshot.ignore_fingerprint IS NOT NULL
                             AND snapshot.ignore_fingerprint != ?5
                       ))
             )",
            params![
                repo_path,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
                crate::commands::structural_graph::extract::current_ignore_fingerprint(),
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Inspect history checkpoint compatibility: {error}"))
}

pub(super) fn compatible_history_checkpoint_exists(
    connection: &Connection,
    repo_path: &str,
    revision: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM history_graph_checkpoints
                WHERE repo_path = ?1 AND revision_sha = ?2
                  AND engine_id = ?3 AND engine_version = ?4 AND schema_version = ?5
                  AND status = 'ready'
                  AND NOT EXISTS(
                      SELECT 1 FROM structural_graph_snapshots snapshot
                      WHERE snapshot.id = history_graph_checkpoints.snapshot_id
                        AND snapshot.ignore_fingerprint IS NOT NULL
                        AND snapshot.ignore_fingerprint != ?6
                  )
             )",
            params![
                repo_path,
                revision,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
                crate::commands::structural_graph::extract::current_ignore_fingerprint(),
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Inspect history checkpoint cache: {error}"))
}

pub(super) mod facts;
pub(super) mod git;
pub(super) mod intervals;
pub(super) mod landmarks;
pub(super) mod persistence;

pub(super) use facts::*;
pub(crate) use git::canonical_repo_path;
use git::*;
pub(super) use intervals::*;
pub(super) use landmarks::*;
