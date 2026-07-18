use super::super::history_facts::{
    HistoryAutomationKind, HistoryPathStatus, HISTORY_FACTS_SCHEMA_VERSION,
    HISTORY_FACT_CLASSIFICATION_VERSION,
};
use super::*;
use rusqlite::Transaction;

pub(in crate::commands::history_graph) fn publish_history_facts(
    transaction: &Transaction<'_>,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
    updated_at: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<String, String> {
    publish_history_facts_inner(
        transaction,
        build,
        tags,
        updated_at,
        cancellation,
        None,
        false,
    )
}

/// Extends a current normalized catalog without replacing prior per-revision
/// path and contributor facts. Tags remain repository-scoped metadata and are
/// refreshed atomically for the complete reachable history.
pub(in crate::commands::history_graph) fn publish_incremental_history_facts(
    transaction: &Transaction<'_>,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
    updated_at: &str,
    cancellation: &StructuralGraphCancellation,
    introduced_revisions: &HashSet<String>,
) -> Result<String, String> {
    publish_history_facts_inner(
        transaction,
        build,
        tags,
        updated_at,
        cancellation,
        Some(introduced_revisions),
        false,
    )
}

fn publish_history_facts_inner(
    transaction: &Transaction<'_>,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
    updated_at: &str,
    cancellation: &StructuralGraphCancellation,
    introduced_revisions: Option<&HashSet<String>>,
    fail_after_replace: bool,
) -> Result<String, String> {
    ensure_publication_active(cancellation)?;
    let timeline = &build.timeline;
    let repo_path = timeline.repo_path.as_str();
    let tags_fingerprint = all_tags_fingerprint(tags);
    let index_identity = stable_graph_id(
        "history-fact-index-v1",
        &format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            HISTORY_FACTS_SCHEMA_VERSION,
            HISTORY_FACT_CLASSIFICATION_VERSION,
            timeline.head,
            tags_fingerprint,
            build.mailmap_fingerprint,
            build.facts_fingerprint,
        ),
    );
    let tags_by_revision = tags_by_commit_from_records(tags);

    if introduced_revisions.is_none() {
        let existing_revisions = stage_revision_ordinals(transaction, repo_path)?;
        let reachable = timeline
            .reachable_revisions
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for revision in existing_revisions
            .iter()
            .filter(|revision| !reachable.contains(revision.as_str()))
        {
            transaction
                .execute(
                    "DELETE FROM history_graph_revisions WHERE repo_path = ?1 AND sha = ?2",
                    params![repo_path, revision],
                )
                .map_err(|error| format!("Remove stale normalized history revision: {error}"))?;
        }
    }
    let mut revision_statement = transaction
        .prepare(
            "INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, author_email_hash,
                subject, parents_json, tags_json, is_release, is_head, coverage_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(repo_path, sha) DO UPDATE SET
                ordinal = excluded.ordinal,
                committed_at = excluded.committed_at,
                author_name = excluded.author_name,
                author_email_hash = NULL,
                subject = excluded.subject,
                parents_json = excluded.parents_json,
                tags_json = excluded.tags_json,
                is_release = excluded.is_release,
                is_head = excluded.is_head,
                coverage_json = excluded.coverage_json",
        )
        .map_err(|error| format!("Prepare normalized history revisions: {error}"))?;
    for (ordinal, sha) in timeline.reachable_revisions.iter().enumerate() {
        if introduced_revisions.is_some_and(|introduced| !introduced.contains(sha)) {
            continue;
        }
        ensure_publication_active(cancellation)?;
        let fact = build
            .facts_by_revision
            .get(sha)
            .ok_or_else(|| format!("Missing normalized facts for revision {sha}"))?;
        let revision_tags = tags_by_revision.get(sha).cloned().unwrap_or_default();
        revision_statement
            .execute(params![
                repo_path,
                sha,
                ordinal as i64,
                fact.committed_at,
                fact.primary.display_name,
                fact.subject,
                serde_json::to_string(&fact.parents).map_err(|error| error.to_string())?,
                serde_json::to_string(&revision_tags).map_err(|error| error.to_string())?,
                i64::from(revision_tags.iter().any(|tag| is_release_tag(tag))),
                i64::from(fact.is_head),
                serde_json::json!({
                    "facts_schema_version": HISTORY_FACTS_SCHEMA_VERSION,
                    "classification_version": HISTORY_FACT_CLASSIFICATION_VERSION,
                    "binary_paths": fact.paths.iter().filter(|path| path.binary).count(),
                    "generated_paths": fact.paths.iter().filter(|path| path.generated).count(),
                    "vendored_paths": fact.paths.iter().filter(|path| path.vendored).count(),
                    "merge": fact.is_merge,
                    "malformed_coauthor_count": fact.malformed_coauthor_count,
                })
                .to_string(),
            ])
            .map_err(|error| format!("Persist normalized history revision: {error}"))?;
    }
    drop(revision_statement);

    if introduced_revisions.is_none() {
        for table in [
            "history_graph_revision_contributors",
            "history_graph_contributors",
            "history_graph_revision_paths",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE repo_path = ?1"),
                    [repo_path],
                )
                .map_err(|error| format!("Replace normalized history table {table}: {error}"))?;
        }
    }
    transaction
        .execute(
            "DELETE FROM history_graph_fact_tags WHERE repo_path = ?1",
            [repo_path],
        )
        .map_err(|error| format!("Replace normalized Git tag table: {error}"))?;
    persist_all_tags(transaction, repo_path, tags)?;
    ensure_publication_active(cancellation)?;
    persist_paths(
        transaction,
        repo_path,
        build,
        cancellation,
        introduced_revisions,
    )?;
    persist_contributors(
        transaction,
        repo_path,
        build,
        cancellation,
        introduced_revisions,
    )?;
    ensure_publication_active(cancellation)?;
    if fail_after_replace {
        return Err("Forced normalized history publication failure".to_string());
    }
    transaction
        .execute(
            "INSERT INTO history_graph_fact_catalogs (
                repo_path, schema_version, classification_version, index_identity,
                indexed_head, tags_fingerprint, mailmap_fingerprint, facts_fingerprint,
                status, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'ready', ?9)
             ON CONFLICT(repo_path) DO UPDATE SET
                schema_version = excluded.schema_version,
                classification_version = excluded.classification_version,
                index_identity = excluded.index_identity,
                indexed_head = excluded.indexed_head,
                tags_fingerprint = excluded.tags_fingerprint,
                mailmap_fingerprint = excluded.mailmap_fingerprint,
                facts_fingerprint = excluded.facts_fingerprint,
                status = 'ready',
                updated_at = excluded.updated_at",
            params![
                repo_path,
                HISTORY_FACTS_SCHEMA_VERSION,
                HISTORY_FACT_CLASSIFICATION_VERSION,
                index_identity,
                timeline.head,
                tags_fingerprint,
                build.mailmap_fingerprint,
                build.facts_fingerprint,
                updated_at,
            ],
        )
        .map_err(|error| format!("Publish normalized history fact identity: {error}"))?;
    Ok(index_identity)
}

fn stage_revision_ordinals(
    transaction: &Transaction<'_>,
    repo_path: &str,
) -> Result<Vec<String>, String> {
    let mut statement = transaction
        .prepare("SELECT sha FROM history_graph_revisions WHERE repo_path = ?1 ORDER BY sha")
        .map_err(|error| format!("Prepare history ordinal staging: {error}"))?;
    let revisions = statement
        .query_map([repo_path], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Query history ordinal staging: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read history ordinal staging: {error}"))?;
    drop(statement);
    for (index, sha) in revisions.iter().enumerate() {
        transaction
            .execute(
                "UPDATE history_graph_revisions SET ordinal = ?3
                 WHERE repo_path = ?1 AND sha = ?2",
                params![repo_path, sha, -1_i64 - index as i64],
            )
            .map_err(|error| format!("Stage history ordinal: {error}"))?;
    }
    Ok(revisions)
}

fn persist_all_tags(
    transaction: &Transaction<'_>,
    repo_path: &str,
    tags: &[GitTagRecord],
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "INSERT INTO history_graph_fact_tags (
                repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|error| format!("Prepare normalized Git tags: {error}"))?;
    for tag in tags {
        statement
            .execute(params![
                repo_path,
                tag.name,
                tag.commit_sha,
                tag.object_sha,
                if tag.object_sha == tag.commit_sha {
                    "lightweight"
                } else {
                    "annotated"
                },
                tag.created_ts,
            ])
            .map_err(|error| format!("Persist normalized Git tag: {error}"))?;
    }
    Ok(())
}

fn persist_paths(
    transaction: &Transaction<'_>,
    repo_path: &str,
    build: &HistoryTimelineBuild,
    cancellation: &StructuralGraphCancellation,
    introduced_revisions: Option<&HashSet<String>>,
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "INSERT INTO history_graph_revision_paths (
                repo_path, revision_sha, path, change_kind, old_path,
                additions, deletions, binary, generated, vendored
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(|error| format!("Prepare normalized revision paths: {error}"))?;
    for sha in &build.timeline.reachable_revisions {
        if introduced_revisions.is_some_and(|introduced| !introduced.contains(sha)) {
            continue;
        }
        ensure_publication_active(cancellation)?;
        let fact = build
            .facts_by_revision
            .get(sha)
            .ok_or_else(|| format!("Missing normalized paths for revision {sha}"))?;
        for path in &fact.paths {
            let additions = path
                .additions
                .map(i64::try_from)
                .transpose()
                .map_err(|_| format!("Additions exceed SQLite range for {}", path.path))?;
            let deletions = path
                .deletions
                .map(i64::try_from)
                .transpose()
                .map_err(|_| format!("Deletions exceed SQLite range for {}", path.path))?;
            statement
                .execute(params![
                    repo_path,
                    sha,
                    path.path,
                    path_status(path.status),
                    path.old_path,
                    additions,
                    deletions,
                    i64::from(path.binary),
                    i64::from(path.generated),
                    i64::from(path.vendored),
                ])
                .map_err(|error| format!("Persist normalized revision path: {error}"))?;
        }
    }
    Ok(())
}

fn persist_contributors(
    transaction: &Transaction<'_>,
    repo_path: &str,
    build: &HistoryTimelineBuild,
    cancellation: &StructuralGraphCancellation,
    introduced_revisions: Option<&HashSet<String>>,
) -> Result<(), String> {
    let mut contributors = BTreeMap::new();
    for sha in &build.timeline.reachable_revisions {
        if introduced_revisions.is_some_and(|introduced| !introduced.contains(sha)) {
            continue;
        }
        let fact = build
            .facts_by_revision
            .get(sha)
            .ok_or_else(|| format!("Missing normalized contributors for revision {sha}"))?;
        for identity in std::iter::once(&fact.primary).chain(&fact.coauthors) {
            let stored = contributors
                .entry(identity.contributor_id.clone())
                .or_insert(identity);
            if identity.alias_count > stored.alias_count {
                *stored = identity;
            }
        }
    }
    let mut contributor_statement = transaction
        .prepare(
            "INSERT INTO history_graph_contributors (
                repo_path, contributor_id, display_name, identity_kind, alias_count
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_path, contributor_id) DO UPDATE SET
                display_name = excluded.display_name,
                identity_kind = excluded.identity_kind,
                alias_count = MAX(history_graph_contributors.alias_count, excluded.alias_count)",
        )
        .map_err(|error| format!("Prepare normalized contributors: {error}"))?;
    for identity in contributors.values() {
        contributor_statement
            .execute(params![
                repo_path,
                identity.contributor_id,
                identity.display_name,
                automation_kind(identity.automation),
                i64::try_from(identity.alias_count)
                    .map_err(|_| "Contributor alias count exceeds SQLite range".to_string())?,
            ])
            .map_err(|error| format!("Persist normalized contributor: {error}"))?;
    }
    drop(contributor_statement);

    let mut role_statement = transaction
        .prepare(
            "INSERT INTO history_graph_revision_contributors (
                repo_path, revision_sha, contributor_id, role
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repo_path, revision_sha, contributor_id, role) DO NOTHING",
        )
        .map_err(|error| format!("Prepare normalized contributor roles: {error}"))?;
    for sha in &build.timeline.reachable_revisions {
        if introduced_revisions.is_some_and(|introduced| !introduced.contains(sha)) {
            continue;
        }
        ensure_publication_active(cancellation)?;
        let fact = build
            .facts_by_revision
            .get(sha)
            .ok_or_else(|| format!("Missing normalized contributors for revision {sha}"))?;
        role_statement
            .execute(params![
                repo_path,
                sha,
                fact.primary.contributor_id,
                "primary"
            ])
            .map_err(|error| format!("Persist primary contributor role: {error}"))?;
        let mut seen_coauthors = HashSet::new();
        for coauthor in fact
            .coauthors
            .iter()
            .filter(|coauthor| coauthor.contributor_id != fact.primary.contributor_id)
            .filter(|coauthor| seen_coauthors.insert(coauthor.contributor_id.as_str()))
        {
            role_statement
                .execute(params![repo_path, sha, coauthor.contributor_id, "coauthor"])
                .map_err(|error| format!("Persist coauthor contributor role: {error}"))?;
        }
    }
    Ok(())
}

fn all_tags_fingerprint(tags: &[GitTagRecord]) -> String {
    let mut identities = tags
        .iter()
        .map(|tag| {
            format!(
                "{}\0{}\0{}\0{}",
                tag.name, tag.object_sha, tag.commit_sha, tag.created_ts
            )
        })
        .collect::<Vec<_>>();
    identities.sort();
    stable_graph_id("history-all-tags-v1", &identities.join("\0"))
}

fn path_status(status: HistoryPathStatus) -> &'static str {
    match status {
        HistoryPathStatus::Added => "added",
        HistoryPathStatus::Copied => "copied",
        HistoryPathStatus::Deleted => "deleted",
        HistoryPathStatus::Modified => "modified",
        HistoryPathStatus::Renamed => "renamed",
        HistoryPathStatus::TypeChanged => "type_changed",
        HistoryPathStatus::Unmerged => "unmerged",
        HistoryPathStatus::Unknown => "unknown",
    }
}

fn automation_kind(kind: HistoryAutomationKind) -> &'static str {
    match kind {
        HistoryAutomationKind::Human => "human",
        HistoryAutomationKind::Automation => "automation",
        HistoryAutomationKind::Unknown => "unknown",
    }
}

fn ensure_publication_active(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Normalized history publication cancelled".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn publish_history_facts_forced_failure(
    transaction: &Transaction<'_>,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
) -> Result<String, String> {
    publish_history_facts_inner(
        transaction,
        build,
        tags,
        "forced",
        &StructuralGraphCancellation::default(),
        None,
        true,
    )
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
