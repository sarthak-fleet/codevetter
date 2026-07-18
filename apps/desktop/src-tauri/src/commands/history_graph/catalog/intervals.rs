use super::*;
use rusqlite::Transaction;

pub(super) const HISTORY_RELEASE_INTERVAL_SCHEMA_VERSION: i64 = 1;

pub(in crate::commands::history_graph) fn publish_release_intervals(
    transaction: &Transaction<'_>,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
) -> Result<String, String> {
    let repo_path = build.timeline.repo_path.as_str();
    let releases = release_positions(build, tags)?;
    let interval_identity = stable_graph_id(
        "history-release-intervals-v1",
        &format!(
            "{}\0{}\0{}\0{}",
            HISTORY_RELEASE_INTERVAL_SCHEMA_VERSION,
            build.timeline.head,
            build.facts_fingerprint,
            releases
                .iter()
                .map(|release| format!("{}:{}", release.tag.name, release.tag.commit_sha))
                .collect::<Vec<_>>()
                .join("\0")
        ),
    );
    transaction
        .execute(
            "DELETE FROM history_graph_release_intervals WHERE repo_path = ?1",
            [repo_path],
        )
        .map_err(|error| format!("Replace release interval rows: {error}"))?;
    let mut statement = transaction
        .prepare(
            "INSERT INTO history_graph_release_intervals (
                repo_path, tag, revision_sha, from_exclusive_sha, commit_count,
                observed_commit_count, coverage_kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|error| format!("Prepare release intervals: {error}"))?;
    for release in &releases {
        statement
            .execute(params![
                repo_path,
                release.tag.name,
                release.tag.commit_sha,
                release.from_exclusive_sha,
                release.commit_count,
                release.observed_commit_count,
                release.coverage_kind,
            ])
            .map_err(|error| format!("Persist release interval: {error}"))?;
    }
    drop(statement);

    let divergent_count = releases
        .iter()
        .filter(|release| release.coverage_kind == "divergent")
        .count();
    let partial = build.timeline.is_shallow || divergent_count > 0;
    let current_coverage: String = transaction
        .query_row(
            "SELECT coverage_json FROM history_graph_release_catalogs WHERE repo_path = ?1",
            [repo_path],
            |row| row.get(0),
        )
        .map_err(|error| format!("Load release interval coverage: {error}"))?;
    let mut coverage: serde_json::Value =
        serde_json::from_str(&current_coverage).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = coverage.as_object_mut() {
        object.insert(
            "interval_schema_version".to_string(),
            HISTORY_RELEASE_INTERVAL_SCHEMA_VERSION.into(),
        );
        object.insert("release_interval_count".to_string(), releases.len().into());
        object.insert(
            "divergent_release_count".to_string(),
            divergent_count.into(),
        );
        object.insert(
            "intervals_complete".to_string(),
            serde_json::Value::Bool(!partial),
        );
    }
    transaction
        .execute(
            "UPDATE history_graph_release_catalogs
             SET interval_schema_version = ?2, interval_identity = ?3,
                 status = CASE WHEN status = 'partial' OR ?4 = 1 THEN 'partial' ELSE status END,
                 coverage_json = ?5
             WHERE repo_path = ?1",
            params![
                repo_path,
                HISTORY_RELEASE_INTERVAL_SCHEMA_VERSION,
                interval_identity,
                i64::from(partial),
                coverage.to_string(),
            ],
        )
        .map_err(|error| format!("Publish release interval identity: {error}"))?;
    Ok(interval_identity)
}

struct ReleasePosition<'a> {
    tag: &'a GitTagRecord,
    from_exclusive_sha: Option<String>,
    commit_count: Option<i64>,
    observed_commit_count: i64,
    coverage_kind: &'static str,
}

fn release_positions<'a>(
    build: &HistoryTimelineBuild,
    tags: &'a [GitTagRecord],
) -> Result<Vec<ReleasePosition<'a>>, String> {
    let ordinals = build
        .timeline
        .reachable_revisions
        .iter()
        .enumerate()
        .map(|(ordinal, sha)| (sha.as_str(), ordinal))
        .collect::<HashMap<_, _>>();
    let mut release_revisions = tags
        .iter()
        .filter(|tag| is_release_tag(&tag.name))
        .filter_map(|tag| {
            ordinals
                .get(tag.commit_sha.as_str())
                .map(|ordinal| (tag.commit_sha.as_str(), *ordinal))
        })
        .collect::<Vec<_>>();
    release_revisions.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(right.0)));
    release_revisions.dedup_by(|left, right| left.0 == right.0);
    let ancestor_cache = release_revisions
        .iter()
        .map(|(sha, _)| Ok(((*sha).to_string(), observed_ancestors(build, sha)?)))
        .collect::<Result<HashMap<_, _>, String>>()?;

    let mut positions = Vec::new();
    for tag in tags.iter().filter(|tag| is_release_tag(&tag.name)) {
        let Some(ordinal) = ordinals.get(tag.commit_sha.as_str()).copied() else {
            positions.push(ReleasePosition {
                tag,
                from_exclusive_sha: None,
                commit_count: None,
                observed_commit_count: 0,
                coverage_kind: "divergent",
            });
            continue;
        };
        let ancestors = ancestor_cache
            .get(&tag.commit_sha)
            .ok_or_else(|| format!("Missing ancestry facts for release {}", tag.name))?;
        let previous = release_revisions
            .iter()
            .rev()
            .find(|(candidate, candidate_ordinal)| {
                *candidate_ordinal < ordinal && ancestors.contains(*candidate)
            })
            .map(|(candidate, _)| (*candidate).to_string());
        let previous_ancestors = previous
            .as_ref()
            .and_then(|sha| ancestor_cache.get(sha))
            .cloned()
            .unwrap_or_default();
        let observed = ancestors.difference(&previous_ancestors).count() as i64;
        let complete = !build.timeline.is_shallow;
        positions.push(ReleasePosition {
            tag,
            from_exclusive_sha: previous,
            commit_count: complete.then_some(observed),
            observed_commit_count: observed,
            coverage_kind: if complete { "complete" } else { "shallow" },
        });
    }
    positions.sort_by(|left, right| left.tag.name.cmp(&right.tag.name));
    Ok(positions)
}

fn observed_ancestors(
    build: &HistoryTimelineBuild,
    revision: &str,
) -> Result<HashSet<String>, String> {
    let mut ancestors = HashSet::new();
    let mut pending = vec![revision.to_string()];
    while let Some(sha) = pending.pop() {
        if !ancestors.insert(sha.clone()) {
            continue;
        }
        let fact = build
            .facts_by_revision
            .get(&sha)
            .ok_or_else(|| format!("Missing ancestry fact for revision {sha}"))?;
        for parent in &fact.parents {
            if build.facts_by_revision.contains_key(parent) {
                pending.push(parent.clone());
            } else if !build.timeline.is_shallow {
                return Err(format!("Complete history is missing parent {parent}"));
            }
        }
    }
    Ok(ancestors)
}

#[cfg(test)]
#[path = "intervals_tests.rs"]
mod tests;
