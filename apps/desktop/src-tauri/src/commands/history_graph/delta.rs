use super::*;

pub(super) fn set_delta<'a>(
    before: impl Iterator<Item = &'a str>,
    after: impl Iterator<Item = &'a str>,
) -> (Vec<String>, Vec<String>) {
    let before = before.collect::<HashSet<_>>();
    let after = after.collect::<HashSet<_>>();
    let mut added = after
        .difference(&before)
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut removed = before
        .difference(&after)
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    added.sort();
    removed.sort();
    (added, removed)
}

pub(super) fn compute_and_persist_structural_delta(
    connection: &Connection,
    root: &Path,
    repo_path: &str,
    before_revision: &str,
    after_revision: &str,
    before: &StructuralGraphSnapshot,
    after: &StructuralGraphSnapshot,
) -> Result<HistoryStructuralDelta, String> {
    let path_changes = changed_path_records_between(root, before_revision, after_revision)?;
    compute_and_persist_structural_delta_with_paths(
        connection,
        repo_path,
        before_revision,
        after_revision,
        before,
        after,
        path_changes,
    )
}

pub(super) fn compute_and_persist_structural_delta_with_paths(
    connection: &Connection,
    repo_path: &str,
    before_revision: &str,
    after_revision: &str,
    before: &StructuralGraphSnapshot,
    after: &StructuralGraphSnapshot,
    path_changes: Vec<HistoryPathChange>,
) -> Result<HistoryStructuralDelta, String> {
    let structural = query::diff_snapshots(before, after);
    let (added_community_ids, removed_community_ids) = set_delta(
        before
            .communities
            .iter()
            .map(|community| community.id.as_str()),
        after
            .communities
            .iter()
            .map(|community| community.id.as_str()),
    );
    let (added_hub_ids, removed_hub_ids) = set_delta(
        before
            .communities
            .iter()
            .flat_map(|community| community.hub_node_ids.iter().map(String::as_str)),
        after
            .communities
            .iter()
            .flat_map(|community| community.hub_node_ids.iter().map(String::as_str)),
    );
    let (added_bridge_ids, removed_bridge_ids) = set_delta(
        before
            .communities
            .iter()
            .flat_map(|community| community.bridge_node_ids.iter().map(String::as_str)),
        after
            .communities
            .iter()
            .flat_map(|community| community.bridge_node_ids.iter().map(String::as_str)),
    );
    let coverage_gap = (before.truncated || after.truncated)
        .then(|| "One or both structural checkpoints were bounded".to_string());
    let mut lineage = derive_lineage(before, after, &path_changes, after_revision);
    lineage.extend(derive_reintroductions(
        connection,
        repo_path,
        after,
        &structural.added_node_ids,
        after_revision,
    )?);
    lineage.sort_by(|left, right| left.id.cmp(&right.id));
    lineage.dedup_by(|left, right| left.id == right.id);
    let upsert_node_ids = structural
        .added_node_ids
        .iter()
        .chain(structural.changed_node_ids.iter())
        .collect::<HashSet<_>>();
    let upsert_edge_ids = structural
        .added_edge_ids
        .iter()
        .chain(structural.changed_edge_ids.iter())
        .collect::<HashSet<_>>();
    let upsert_nodes = after
        .nodes
        .iter()
        .filter(|node| upsert_node_ids.contains(&node.id))
        .cloned()
        .collect();
    let upsert_edges = after
        .edges
        .iter()
        .filter(|edge| upsert_edge_ids.contains(&edge.id))
        .cloned()
        .collect();
    let before_files = before
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<HashMap<_, _>>();
    let after_file_paths = after
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    let upsert_files = after
        .files
        .iter()
        .filter(|file| before_files.get(file.path.as_str()).copied() != Some(*file))
        .cloned()
        .collect();
    let mut removed_file_paths = before
        .files
        .iter()
        .filter(|file| !after_file_paths.contains(file.path.as_str()))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    removed_file_paths.sort();
    let before_metrics = before
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric))
        .collect::<HashMap<_, _>>();
    let after_metric_ids = after
        .metrics
        .iter()
        .map(|metric| metric.id.as_str())
        .collect::<HashSet<_>>();
    let upsert_metrics = after
        .metrics
        .iter()
        .filter(|metric| before_metrics.get(metric.id.as_str()).copied() != Some(*metric))
        .cloned()
        .collect();
    let mut removed_metric_ids = before
        .metrics
        .iter()
        .filter(|metric| !after_metric_ids.contains(metric.id.as_str()))
        .map(|metric| metric.id.clone())
        .collect::<Vec<_>>();
    removed_metric_ids.sort();
    let before_clone_groups = before
        .clone_groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect::<HashMap<_, _>>();
    let after_clone_ids = after
        .clone_groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<HashSet<_>>();
    let upsert_clone_groups = after
        .clone_groups
        .iter()
        .filter(|group| before_clone_groups.get(group.id.as_str()).copied() != Some(*group))
        .cloned()
        .collect();
    let mut removed_clone_group_ids = before
        .clone_groups
        .iter()
        .filter(|group| !after_clone_ids.contains(group.id.as_str()))
        .map(|group| group.id.clone())
        .collect::<Vec<_>>();
    removed_clone_group_ids.sort();
    let delta = HistoryStructuralDelta {
        schema_version: 1,
        materialization_version: 1,
        repo_path: repo_path.to_string(),
        before_revision: before_revision.to_string(),
        after_revision: after_revision.to_string(),
        before_snapshot_id: before.id.clone(),
        after_snapshot_id: after.id.clone(),
        added_node_ids: structural.added_node_ids,
        removed_node_ids: structural.removed_node_ids,
        changed_node_ids: structural.changed_node_ids,
        added_edge_ids: structural.added_edge_ids,
        removed_edge_ids: structural.removed_edge_ids,
        changed_edge_ids: structural.changed_edge_ids,
        added_community_ids,
        removed_community_ids,
        added_hub_ids,
        removed_hub_ids,
        added_bridge_ids,
        removed_bridge_ids,
        path_changes,
        lineage,
        coverage_gap,
        generated_at: Utc::now().to_rfc3339(),
        upsert_nodes,
        upsert_edges,
        upsert_communities: after.communities.clone(),
        upsert_files,
        removed_file_paths,
        upsert_metrics,
        removed_metric_ids,
        after_metric_order: after
            .metrics
            .iter()
            .map(|metric| metric.id.clone())
            .collect(),
        upsert_clone_groups,
        removed_clone_group_ids,
        after_clone_group_order: after
            .clone_groups
            .iter()
            .map(|group| group.id.clone())
            .collect(),
        after_coverage: after.coverage.clone(),
        after_diagnostics: after.diagnostics.clone(),
        after_cursor: after.cursor.clone(),
        after_ignore_fingerprint: after.ignore_fingerprint.clone(),
        after_truncated: after.truncated,
        after_created_at: after.created_at.clone(),
    };
    persist_structural_delta(connection, &delta)?;
    Ok(delta)
}

pub(super) fn persist_structural_delta(
    connection: &Connection,
    delta: &HistoryStructuralDelta,
) -> Result<(), String> {
    let event_id = structural_delta_event_id(
        &delta.repo_path,
        &delta.before_revision,
        &delta.after_revision,
    );
    let summary = serde_json::json!({
        "schema_version": delta.schema_version,
        "materialization_version": delta.materialization_version,
        "repo_path": delta.repo_path,
        "before_revision": delta.before_revision,
        "after_revision": delta.after_revision,
        "before_snapshot_id": delta.before_snapshot_id,
        "after_snapshot_id": delta.after_snapshot_id,
        "added_node_ids": delta.added_node_ids,
        "removed_node_ids": delta.removed_node_ids,
        "changed_node_ids": delta.changed_node_ids,
        "added_edge_ids": delta.added_edge_ids,
        "removed_edge_ids": delta.removed_edge_ids,
        "changed_edge_ids": delta.changed_edge_ids,
        "added_community_ids": delta.added_community_ids,
        "removed_community_ids": delta.removed_community_ids,
        "added_hub_ids": delta.added_hub_ids,
        "removed_hub_ids": delta.removed_hub_ids,
        "added_bridge_ids": delta.added_bridge_ids,
        "removed_bridge_ids": delta.removed_bridge_ids,
        "path_changes": delta.path_changes,
        "lineage": delta.lineage,
        "coverage_gap": delta.coverage_gap,
        "generated_at": delta.generated_at,
        "payload_encoding": "zlib-json-v1",
    })
    .to_string();
    connection
        .execute(
            "INSERT OR REPLACE INTO history_graph_events (
                id, repo_path, revision_sha, event_kind, trust, origin,
                source_id, source_cursor, payload_json, evidence_json, recorded_at
             ) VALUES (?1, ?2, ?3, 'structural_delta', 'extracted', 'analysis',
                'codevetter-structural-history', ?4, ?5, '[]', ?6)",
            params![
                event_id,
                delta.repo_path,
                delta.after_revision,
                delta.after_snapshot_id,
                summary,
                delta.generated_at,
            ],
        )
        .map_err(|error| format!("Persist structural history delta: {error}"))?;
    persist_history_delta_blob(connection, &event_id, delta)?;
    for lineage in &delta.lineage {
        connection
            .execute(
                "INSERT OR REPLACE INTO history_graph_events (
                    id, repo_path, revision_sha, event_kind, entity_id, related_entity_id,
                    relation_kind, trust, origin, source_id, source_cursor,
                    payload_json, evidence_json, recorded_at
                 ) VALUES (?1, ?2, ?3, 'entity_lineage', ?4, ?5, ?6, ?7,
                    'analysis', 'codevetter-lineage', ?8, ?9, ?10, ?11)",
                params![
                    lineage.id,
                    delta.repo_path,
                    delta.after_revision,
                    lineage.from_entity_id,
                    lineage.to_entity_id,
                    lineage.relation,
                    lineage.trust.as_str(),
                    delta.after_snapshot_id,
                    serde_json::to_string(lineage).map_err(|error| error.to_string())?,
                    serde_json::to_string(&lineage.sources).map_err(|error| error.to_string())?,
                    delta.generated_at,
                ],
            )
            .map_err(|error| format!("Persist structural lineage: {error}"))?;
    }
    Ok(())
}

pub(super) fn derive_reintroductions(
    connection: &Connection,
    repo_path: &str,
    after: &StructuralGraphSnapshot,
    added_node_ids: &[String],
    after_revision: &str,
) -> Result<Vec<HistoryLineageEdge>, String> {
    const REINTRODUCTION_QUERY_CHUNK: usize = 500;
    if added_node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let added = added_node_ids.iter().collect::<HashSet<_>>();
    let mut removals = HashMap::new();
    for node_ids in added_node_ids.chunks(REINTRODUCTION_QUERY_CHUNK) {
        let placeholders = std::iter::repeat_n("?", node_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = connection
            .prepare(&format!(
                "SELECT entity_id, payload_json FROM history_graph_events
                 WHERE repo_path = ? AND event_kind = 'entity_lineage'
                   AND relation_kind = 'removed_in' AND entity_id IN ({placeholders})
                 ORDER BY entity_id, recorded_at DESC, id DESC"
            ))
            .map_err(|error| format!("Prepare reintroduction query: {error}"))?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(
                    std::iter::once(repo_path).chain(node_ids.iter().map(String::as_str)),
                ),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| format!("Query prior removals: {error}"))?;
        for row in rows {
            let (node_id, payload) = row.map_err(|error| format!("Read prior removal: {error}"))?;
            if removals.contains_key(&node_id) {
                continue;
            }
            removals.insert(
                node_id,
                serde_json::from_str::<HistoryLineageEdge>(&payload)
                    .map_err(|error| format!("Decode prior removal: {error}"))?,
            );
        }
    }
    let mut reintroductions = Vec::new();
    for node in after.nodes.iter().filter(|node| added.contains(&node.id)) {
        let Some(removal) = removals.get(&node.id) else {
            continue;
        };
        reintroductions.push(HistoryLineageEdge {
            id: stable_graph_id(
                "lineage",
                &format!("reintroduced_in\0{}\0{after_revision}", node.id),
            ),
            from_entity_id: node.id.clone(),
            to_entity_id: node.id.clone(),
            relation: "reintroduced_in".to_string(),
            trust: GraphTrust::Extracted,
            evidence: format!(
                "Entity returns after the prior removal event {}",
                removal.id
            ),
            sources: node.sources.clone(),
            candidates: Vec::new(),
        });
    }
    Ok(reintroductions)
}

pub(super) fn derive_lineage(
    before: &StructuralGraphSnapshot,
    after: &StructuralGraphSnapshot,
    path_changes: &[HistoryPathChange],
    after_revision: &str,
) -> Vec<HistoryLineageEdge> {
    let mut lineage = Vec::new();
    let after_by_id = after
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    for source in &before.nodes {
        let Some(target) = after_by_id.get(source.id.as_str()) else {
            continue;
        };
        if lineage_relevant_change(source, target) {
            lineage.push(lineage_edge(
                source,
                target,
                "same_as",
                GraphTrust::Extracted,
                "Stable structural identity persists while entity attributes change".to_string(),
                Vec::new(),
            ));
        }
    }
    let after_by_path = after
        .nodes
        .iter()
        .filter_map(|node| node.path.as_deref().map(|path| (path, node)))
        .fold(HashMap::<&str, Vec<_>>::new(), |mut map, (path, node)| {
            map.entry(path).or_default().push(node);
            map
        });
    let mut matched_before = HashSet::new();
    let mut matched_after = HashSet::new();
    for change in path_changes.iter().filter(|change| {
        matches!(change.change_kind.as_str(), "renamed" | "copied") && change.old_path.is_some()
    }) {
        let old_path = change.old_path.as_deref().unwrap_or_default();
        let Some(candidates_at_target) = after_by_path.get(change.path.as_str()) else {
            continue;
        };
        for source in before
            .nodes
            .iter()
            .filter(|node| node.path.as_deref() == Some(old_path))
        {
            let mut candidates = candidates_at_target
                .iter()
                .copied()
                .filter(|target| {
                    target.kind == source.kind
                        && (target.label == source.label
                            || (source.kind == "file" && target.kind == "file"))
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| left.id.cmp(&right.id));
            if candidates.is_empty() {
                continue;
            }
            let target = candidates[0];
            let trust = if candidates.len() == 1 {
                GraphTrust::Extracted
            } else {
                GraphTrust::Ambiguous
            };
            let relation = if change.change_kind == "renamed" {
                "moved_to"
            } else {
                "evolved_from"
            };
            lineage.push(lineage_edge(
                source,
                target,
                relation,
                trust,
                format!(
                    "Git {} maps {} to {} and structural kind/label remains compatible",
                    change.change_kind, old_path, change.path
                ),
                candidates
                    .iter()
                    .skip(1)
                    .map(|node| node.id.clone())
                    .collect(),
            ));
            matched_before.insert(source.id.as_str());
            matched_after.insert(target.id.as_str());
        }
    }
    let rename_sources = before
        .nodes
        .iter()
        .filter(|node| {
            !after.nodes.iter().any(|target| target.id == node.id)
                && !matched_before.contains(node.id.as_str())
        })
        .collect::<Vec<_>>();
    let mut merge_targets = HashMap::<&str, Vec<_>>::new();
    for source in &rename_sources {
        let source_line = source.sources.first().and_then(|anchor| anchor.start_line);
        for target in after
            .nodes
            .iter()
            .filter(|target| !matched_after.contains(target.id.as_str()))
            .filter(|target| target.kind == source.kind && target.path == source.path)
            .filter(|target| {
                source_line.is_some()
                    && target.sources.first().and_then(|anchor| anchor.start_line) == source_line
            })
        {
            merge_targets
                .entry(target.id.as_str())
                .or_default()
                .push(*source);
        }
    }
    for (target_id, mut sources) in merge_targets {
        if sources.len() < 2 {
            continue;
        }
        sources.sort_by(|left, right| left.id.cmp(&right.id));
        let Some(target) = after_by_id.get(target_id) else {
            continue;
        };
        for source in &sources {
            lineage.push(lineage_edge(
                source,
                target,
                "merged_from",
                GraphTrust::Ambiguous,
                "Multiple removed entities share the successor's path, kind, and source line"
                    .to_string(),
                sources
                    .iter()
                    .filter(|candidate| candidate.id != source.id)
                    .map(|candidate| candidate.id.clone())
                    .collect(),
            ));
            matched_before.insert(source.id.as_str());
        }
        matched_after.insert(target.id.as_str());
    }
    for source in rename_sources {
        if matched_before.contains(source.id.as_str()) {
            continue;
        }
        let source_line = source.sources.first().and_then(|anchor| anchor.start_line);
        let mut candidates = after
            .nodes
            .iter()
            .filter(|target| !matched_after.contains(target.id.as_str()))
            .filter(|target| target.kind == source.kind && target.path == source.path)
            .filter(|target| {
                source_line.is_some()
                    && target.sources.first().and_then(|anchor| anchor.start_line) == source_line
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        if candidates.len() == 1 {
            let target = candidates[0];
            lineage.push(lineage_edge(
                source,
                target,
                if source.label == target.label {
                    "evolved_from"
                } else {
                    "renamed_to"
                },
                GraphTrust::Inferred,
                "Same path, structural kind, and source line across adjacent revisions".to_string(),
                Vec::new(),
            ));
            matched_before.insert(source.id.as_str());
            matched_after.insert(target.id.as_str());
        } else if candidates.len() > 1 {
            lineage.push(lineage_edge(
                source,
                candidates[0],
                "split_into",
                GraphTrust::Ambiguous,
                "Multiple same-path structural candidates follow the removed entity".to_string(),
                candidates
                    .iter()
                    .skip(1)
                    .map(|node| node.id.clone())
                    .collect(),
            ));
            matched_before.insert(source.id.as_str());
        }
    }
    let revision_entity = stable_graph_id("revision", after_revision);
    for source in before.nodes.iter().filter(|node| {
        !after.nodes.iter().any(|target| target.id == node.id)
            && !matched_before.contains(node.id.as_str())
    }) {
        lineage.push(HistoryLineageEdge {
            id: stable_graph_id(
                "lineage",
                &format!("removed_in\0{}\0{revision_entity}", source.id),
            ),
            from_entity_id: source.id.clone(),
            to_entity_id: revision_entity.clone(),
            relation: "removed_in".to_string(),
            trust: GraphTrust::Extracted,
            evidence: "Entity is absent from the exact next structural checkpoint".to_string(),
            sources: source.sources.clone(),
            candidates: Vec::new(),
        });
    }
    lineage.sort_by(|left, right| left.id.cmp(&right.id));
    lineage
}

pub(super) fn lineage_relevant_change(
    source: &crate::commands::structural_graph::types::StructuralGraphNode,
    target: &crate::commands::structural_graph::types::StructuralGraphNode,
) -> bool {
    source.label != target.label
        || source.qualified_name != target.qualified_name
        || source.path != target.path
        || source.kind != target.kind
        || source.detail != target.detail
        || source.language != target.language
        || source
            .sources
            .first()
            .and_then(|anchor| anchor.excerpt.as_deref())
            != target
                .sources
                .first()
                .and_then(|anchor| anchor.excerpt.as_deref())
}

pub(super) fn lineage_edge(
    source: &crate::commands::structural_graph::types::StructuralGraphNode,
    target: &crate::commands::structural_graph::types::StructuralGraphNode,
    relation: &str,
    trust: GraphTrust,
    evidence: String,
    candidates: Vec<String>,
) -> HistoryLineageEdge {
    HistoryLineageEdge {
        id: stable_graph_id(
            "lineage",
            &format!("{relation}\0{}\0{}", source.id, target.id),
        ),
        from_entity_id: source.id.clone(),
        to_entity_id: target.id.clone(),
        relation: relation.to_string(),
        trust,
        evidence,
        sources: source
            .sources
            .iter()
            .chain(target.sources.iter())
            .cloned()
            .collect(),
        candidates,
    }
}
