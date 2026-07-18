use super::*;

pub(super) fn unknown_facet(name: &str, summary: &str) -> HistoryFacet {
    HistoryFacet {
        name: name.to_string(),
        status: HistoryFacetStatus::Unknown,
        summary: summary.to_string(),
        trust: GraphTrust::Inferred,
        sources: Vec::new(),
        event_ids: Vec::new(),
    }
}

pub(super) fn git_path_history(
    root: &Path,
    revision: &str,
    path: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let output = git_text(
        root,
        &[
            "log",
            "--follow",
            "--reverse",
            "--format=%H%x1f%cI%x1f%s%x1e",
            revision,
            "--",
            path,
        ],
    )?;
    Ok(output
        .split('\u{1e}')
        .filter_map(|record| {
            let fields = record.trim().splitn(3, '\u{1f}').collect::<Vec<_>>();
            (fields.len() == 3).then(|| {
                (
                    fields[0].to_string(),
                    fields[1].to_string(),
                    fields[2].to_string(),
                )
            })
        })
        .collect())
}

pub(crate) fn load_outcome_events(
    connection: &Connection,
    repo_path: &str,
    entity_id: &str,
) -> Result<Vec<(String, String, GraphTrust)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, event_kind, trust FROM history_graph_events
             WHERE repo_path = ?1 AND entity_id = ?2
               AND event_kind IN ('deploy', 'release', 'incident', 'observed_outcome',
                   'analytics_provider_ingestion', 'analytics_provider_delivery')
             ORDER BY recorded_at DESC, id LIMIT 100",
        )
        .map_err(|error| format!("Prepare outcome evidence query: {error}"))?;
    let outcomes = statement
        .query_map(params![repo_path, entity_id], |row| {
            let trust: String = row.get(2)?;
            Ok((row.get(0)?, row.get(1)?, GraphTrust::from_storage(&trust)))
        })
        .map_err(|error| format!("Query outcome evidence: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read outcome evidence: {error}"))?;
    Ok(outcomes)
}

pub(crate) fn load_entity_annotation_contradictions(
    connection: &Connection,
    repo_path: &str,
    entity_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT decision, body FROM history_graph_annotations
             WHERE repo_path = ?1 AND entity_id = ?2
               AND decision IN ('reject', 'correction')
             ORDER BY created_at DESC, id LIMIT 20",
        )
        .map_err(|error| format!("Prepare entity contradiction query: {error}"))?;
    let contradictions = statement
        .query_map(params![repo_path, entity_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Query entity contradictions: {error}"))?
        .map(|row| {
            row.map(|(decision, body)| {
                format!(
                    "Local {decision} annotation: {}",
                    body.chars().take(500).collect::<String>()
                )
            })
            .map_err(|error| format!("Read entity contradiction: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(contradictions)
}

pub(crate) fn history_index_freshness(
    connection: &Connection,
    repo_path: &str,
    current_head: &str,
) -> Result<(String, bool, Value), String> {
    let row = connection
        .query_row(
            "SELECT indexed_head, indexed_tags_fingerprint, coverage_json
             FROM history_graph_repositories
             WHERE repo_path = ?1",
            params![repo_path],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load history freshness: {error}"))?;
    let Some((indexed_head, indexed_tags_fingerprint, coverage_json)) = row else {
        return Ok((String::new(), true, serde_json::json!({})));
    };
    let indexed_head = indexed_head.unwrap_or_default();
    let tags_stale = repository_tag_fingerprint(Path::new(repo_path))
        .ok()
        .zip(indexed_tags_fingerprint)
        .is_some_and(|(current, indexed)| current != indexed);
    let stale = indexed_head.is_empty() || indexed_head != current_head || tags_stale;
    let coverage = serde_json::from_str(&coverage_json).unwrap_or_else(|_| serde_json::json!({}));
    Ok((indexed_head, stale, coverage))
}

pub(crate) fn load_lineage_family(
    connection: &Connection,
    repo_path: &str,
    seed_entity_id: &str,
    limit: usize,
) -> Result<(Vec<HistoryLineageEdge>, HashSet<String>, bool), String> {
    let mut statement = connection
        .prepare(
            "SELECT payload_json FROM history_graph_events
             WHERE repo_path = ?1 AND event_kind = 'entity_lineage'
               AND (entity_id = ?2 OR related_entity_id = ?2)
             ORDER BY recorded_at, id LIMIT ?3",
        )
        .map_err(|error| format!("Prepare lineage query: {error}"))?;
    let mut family = HashSet::from([seed_entity_id.to_string()]);
    let mut queue = vec![seed_entity_id.to_string()];
    let mut cursor = 0;
    let mut edges = BTreeMap::<String, HistoryLineageEdge>::new();
    let mut truncated = false;
    while cursor < queue.len() {
        if edges.len() >= limit || family.len() >= limit {
            truncated = true;
            break;
        }
        let entity_id = queue[cursor].clone();
        cursor += 1;
        let rows = statement
            .query_map(params![repo_path, entity_id, (limit + 1) as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("Query entity lineage: {error}"))?;
        for payload in rows {
            let payload = payload.map_err(|error| format!("Read entity lineage: {error}"))?;
            let edge: HistoryLineageEdge = serde_json::from_str(&payload)
                .map_err(|error| format!("Decode entity lineage: {error}"))?;
            if edges.contains_key(&edge.id) {
                continue;
            }
            if edges.len() >= limit {
                truncated = true;
                break;
            }
            let mut related_ids = vec![edge.from_entity_id.clone()];
            if edge.relation != "removed_in" {
                related_ids.push(edge.to_entity_id.clone());
            }
            related_ids.extend(edge.candidates.iter().cloned());
            for related_id in related_ids {
                if family.len() >= limit {
                    truncated = true;
                    break;
                }
                if family.insert(related_id.clone()) {
                    queue.push(related_id);
                }
            }
            edges.insert(edge.id.clone(), edge);
        }
    }
    Ok((edges.into_values().collect(), family, truncated))
}

pub(crate) fn load_entity_occurrences(
    connection: &Connection,
    repo_path: &str,
    entity_ids: &HashSet<String>,
    limit: usize,
) -> Result<(Vec<HistoryEntityMoment>, bool), String> {
    let mut statement = connection
        .prepare(
            "SELECT c.revision_sha, r.committed_at, r.ordinal, n.id, n.label,
                    n.kind, n.path, n.detail
             FROM history_graph_checkpoints c
             JOIN history_graph_revisions r
               ON r.repo_path = c.repo_path AND r.sha = c.revision_sha
             JOIN structural_graph_nodes n ON n.snapshot_id = c.snapshot_id
             WHERE c.repo_path = ?1 AND c.status = 'ready' AND c.engine_id = ?2
               AND c.engine_version = ?3 AND c.schema_version = ?4 AND n.id = ?5
             ORDER BY r.ordinal, n.id",
        )
        .map_err(|error| format!("Prepare entity occurrence query: {error}"))?;
    let mut occurrences = BTreeMap::<(i64, String, String), HistoryEntityMoment>::new();
    let mut ids = entity_ids.iter().collect::<Vec<_>>();
    ids.sort();
    let mut truncated = false;
    for entity_id in ids {
        let rows = statement
            .query_map(
                params![
                    repo_path,
                    BUNDLED_ENGINE_ID,
                    BUNDLED_ENGINE_VERSION,
                    STRUCTURAL_GRAPH_SCHEMA_VERSION,
                    entity_id
                ],
                |row| {
                    Ok(HistoryEntityMoment {
                        revision_sha: row.get(0)?,
                        committed_at: row.get(1)?,
                        ordinal: row.get(2)?,
                        entity_id: row.get(3)?,
                        label: row.get(4)?,
                        kind: row.get(5)?,
                        path: row.get(6)?,
                        detail: row.get(7)?,
                    })
                },
            )
            .map_err(|error| format!("Query entity occurrences: {error}"))?;
        for moment in rows {
            let moment = moment.map_err(|error| format!("Read entity occurrence: {error}"))?;
            if occurrences.len() >= limit {
                truncated = true;
                break;
            }
            occurrences.insert(
                (
                    moment.ordinal,
                    moment.revision_sha.clone(),
                    moment.entity_id.clone(),
                ),
                moment,
            );
        }
        if truncated {
            break;
        }
    }
    Ok((occurrences.into_values().collect(), truncated))
}

pub(super) fn estimate_eta_ms(
    started: std::time::Instant,
    completed: usize,
    total: usize,
) -> Option<u64> {
    if completed == 0 || completed >= total {
        return None;
    }
    let per_item = started.elapsed().as_millis() / completed as u128;
    Some((per_item * (total - completed) as u128).min(u64::MAX as u128) as u64)
}

#[cfg(test)]
pub(super) fn reachable_release_revisions(root: &Path) -> Result<Vec<String>, String> {
    reachable_release_revisions_from_tags(root, &read_git_tags(root)?)
}

#[cfg(test)]
pub(super) fn reachable_release_revisions_from_tags(
    root: &Path,
    tags: &[GitTagRecord],
) -> Result<Vec<String>, String> {
    let mut releases = tags
        .iter()
        .filter(|tag| is_release_tag(&tag.name))
        .map(|tag| tag.commit_sha.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|sha| git_is_ancestor(root, sha, "HEAD"))
        .map(|sha| {
            let committed_at = git_text(root, &["show", "-s", "--format=%cI", &sha])?;
            Ok((committed_at, sha))
        })
        .collect::<Result<Vec<_>, String>>()?;
    releases.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    Ok(releases.into_iter().map(|(_, sha)| sha).collect())
}
