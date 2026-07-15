use super::*;

#[tauri::command]
pub async fn get_history_structural_state(
    repo_path: String,
    revision: String,
    max_nodes: Option<usize>,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryStructuralState, String> {
    let root = canonical_repo_path(&repo_path)?;
    let revision = resolve_revision(&root, &revision)?;
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let reconstructed = {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            reconstruct_history_as_of(&connection, &canonical, &storage_key, &revision)?
        };
        let (snapshot, cached) = match reconstructed {
            Some(snapshot) => (snapshot, true),
            None => load_or_build_history_snapshot(
                &root,
                &canonical,
                &storage_key,
                &revision,
                &app,
                &database,
            )?,
        };
        let path_changes = changed_path_records(&root, &revision)?;
        let mut revision_changes = path_changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        revision_changes.sort();
        Ok(HistoryStructuralState {
            schema_version: 1,
            repo_path: canonical,
            revision,
            snapshot_id: snapshot.id.clone(),
            cached,
            projection: query::overview(&snapshot, max_nodes),
            analysis: query::analysis_summary(&snapshot),
            changed_paths: revision_changes,
            path_changes,
            indexed_files: snapshot.coverage.indexed_files,
            node_count: snapshot.nodes.len(),
            edge_count: snapshot.edges.len(),
            generated_at: snapshot.created_at,
        })
    })
    .await
    .map_err(|error| format!("Historical structural state worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_structural_delta(
    repo_path: String,
    before_revision: String,
    after_revision: String,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryStructuralDelta, String> {
    let root = canonical_repo_path(&repo_path)?;
    let before_revision = resolve_revision(&root, &before_revision)?;
    let after_revision = resolve_revision(&root, &after_revision)?;
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let cached_delta = {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            load_history_structural_delta(
                &connection,
                &canonical,
                &before_revision,
                &after_revision,
            )?
        };
        if let Some(delta) = cached_delta {
            return Ok(delta);
        }
        let (before, _) = load_or_build_history_snapshot(
            &root,
            &canonical,
            &storage_key,
            &before_revision,
            &app,
            &database,
        )?;
        let (after, _) = load_or_build_history_snapshot(
            &root,
            &canonical,
            &storage_key,
            &after_revision,
            &app,
            &database,
        )?;
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        let delta = compute_and_persist_structural_delta(
            &connection,
            &root,
            &canonical,
            &before_revision,
            &after_revision,
            &before,
            &after,
        )?;
        Ok(delta)
    })
    .await
    .map_err(|error| format!("Historical structural delta worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_as_of(
    repo_path: String,
    reference: HistoryTemporalReference,
    max_nodes: Option<usize>,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryAsOfState, String> {
    let root = canonical_repo_path(&repo_path)?;
    let revision = resolve_temporal_reference(&root, &reference)?;
    let committed_at = git_text(&root, &["show", "-s", "--format=%cI", &revision])?;
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let reconstructed = {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            reconstruct_history_as_of(&connection, &canonical, &storage_key, &revision)?
        };
        let (snapshot, cached) = match reconstructed {
            Some(snapshot) => (snapshot, true),
            None => load_or_build_history_snapshot(
                &root,
                &canonical,
                &storage_key,
                &revision,
                &app,
                &database,
            )?,
        };
        let path_changes = changed_path_records(&root, &revision)?;
        let mut changed_paths = path_changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        changed_paths.sort();
        Ok(HistoryAsOfState {
            requested: reference,
            resolved_revision: revision.clone(),
            committed_at,
            exact: true,
            state: HistoryStructuralState {
                schema_version: 1,
                repo_path: canonical,
                revision,
                snapshot_id: snapshot.id.clone(),
                cached,
                projection: query::overview(&snapshot, max_nodes),
                analysis: query::analysis_summary(&snapshot),
                changed_paths,
                path_changes,
                indexed_files: snapshot.coverage.indexed_files,
                node_count: snapshot.nodes.len(),
                edge_count: snapshot.edges.len(),
                generated_at: snapshot.created_at,
            },
        })
    })
    .await
    .map_err(|error| format!("Historical as-of worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_history_entity_evolution(
    repo_path: String,
    entity: String,
    revision: Option<String>,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryEntityEvolution, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let revision = resolve_revision(&root, revision.as_deref().unwrap_or("HEAD"))?;
    let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
    let storage_key = history_storage_key(&canonical);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let (snapshot, _) = load_or_build_history_snapshot(
            &root,
            &canonical,
            &storage_key,
            &revision,
            &app,
            &database,
        )?;
        let node = query::resolve_node(&snapshot, &entity)?.clone();
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        let (lineage, family_ids, lineage_truncated) =
            load_lineage_family(&connection, &canonical, &node.id, 200)?;
        let (occurrences, occurrence_truncated) =
            load_entity_occurrences(&connection, &canonical, &family_ids, 500)?;
        let first_seen = occurrences.first().cloned();
        let last_present = occurrences.last().cloned();
        let mut last_changed = None;
        let mut previous_signature: Option<(&str, &str, Option<&str>, Option<&str>)> = None;
        for occurrence in &occurrences {
            let signature = (
                occurrence.entity_id.as_str(),
                occurrence.label.as_str(),
                occurrence.path.as_deref(),
                occurrence.detail.as_deref(),
            );
            if previous_signature != Some(signature) {
                last_changed = Some(occurrence.clone());
            }
            previous_signature = Some(signature);
        }
        let (indexed_head, stale, coverage) =
            history_index_freshness(&connection, &canonical, &current_head)?;
        let coverage_complete = coverage
            .get("coverage_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated = lineage_truncated || occurrence_truncated;
        let coverage_gap = if truncated {
            Some("Entity evolution exceeded local query bounds".to_string())
        } else if !coverage_complete {
            Some("First/last moments are bounded by the indexed history coverage".to_string())
        } else {
            None
        };
        Ok(HistoryEntityEvolution {
            schema_version: 1,
            repo_path: canonical,
            resolved_revision: revision,
            entity_id: node.id,
            entity_label: node.label,
            entity_kind: node.kind,
            lineage,
            occurrences,
            first_seen,
            last_changed,
            last_present,
            indexed_head,
            stale,
            coverage_gap,
            truncated,
            next_cursor: None,
        })
    })
    .await
    .map_err(|error| format!("History entity evolution worker failed: {error}"))?
}

pub(crate) fn resolve_temporal_reference(
    root: &Path,
    reference: &HistoryTemporalReference,
) -> Result<String, String> {
    match reference {
        HistoryTemporalReference::Revision { revision } => resolve_revision(root, revision),
        HistoryTemporalReference::Release { tag } => resolve_revision(root, tag),
        HistoryTemporalReference::Date { at } => {
            chrono::DateTime::parse_from_rfc3339(at)
                .map_err(|error| format!("History date must be RFC3339: {error}"))?;
            let revision = git_text(root, &["rev-list", "-1", &format!("--before={at}"), "HEAD"])?;
            if revision.is_empty() {
                Err(format!("No reachable commit exists at or before {at}"))
            } else {
                Ok(revision)
            }
        }
    }
}

pub(crate) fn reconstruct_history_as_of(
    connection: &Connection,
    repo_path: &str,
    storage_key: &str,
    target_revision: &str,
) -> Result<Option<StructuralGraphSnapshot>, String> {
    let target_exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM history_graph_revisions
             WHERE repo_path = ?1 AND sha = ?2)",
            params![repo_path, target_revision],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Resolve indexed as-of revision: {error}"))?;
    if !target_exists {
        return Ok(None);
    }
    let mut checkpoint_statement = connection
        .prepare(
            "SELECT revision_sha, snapshot_id FROM history_graph_checkpoints
             WHERE repo_path = ?1 AND status = 'ready'
               AND engine_id = ?2 AND engine_version = ?3 AND schema_version = ?4",
        )
        .map_err(|error| format!("Prepare compatible history checkpoints: {error}"))?;
    let checkpoints = checkpoint_statement
        .query_map(
            params![
                repo_path,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| format!("Query compatible history checkpoints: {error}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| format!("Read compatible history checkpoints: {error}"))?;

    let mut materialization_chain = vec![target_revision.to_string()];
    while !checkpoints.contains_key(
        materialization_chain
            .last()
            .expect("materialization chain has a target"),
    ) {
        if materialization_chain.len() > MAX_HISTORY_LIMIT + checkpoints.len() + 1 {
            return Ok(None);
        }
        let current = materialization_chain
            .last()
            .expect("materialization chain has a current revision");
        let parents_json = connection
            .query_row(
                "SELECT parents_json FROM history_graph_revisions
                 WHERE repo_path = ?1 AND sha = ?2",
                params![repo_path, current],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Load materialization parent: {error}"))?;
        let Some(parents_json) = parents_json else {
            return Ok(None);
        };
        let parents: Vec<String> = serde_json::from_str(&parents_json).unwrap_or_default();
        let Some(parent) = parents.first() else {
            return Ok(None);
        };
        let parent_indexed = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history_graph_revisions
                 WHERE repo_path = ?1 AND sha = ?2)",
                params![repo_path, parent],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("Check materialization parent coverage: {error}"))?;
        if !parent_indexed || materialization_chain.contains(parent) {
            return Ok(None);
        }
        materialization_chain.push(parent.clone());
    }
    let checkpoint_revision = materialization_chain
        .last()
        .expect("checkpoint terminates materialization chain")
        .clone();
    let Some(snapshot_id) = checkpoints.get(&checkpoint_revision).cloned() else {
        return Ok(None);
    };
    let snapshot_blob = load_history_snapshot_blob(connection, repo_path, &snapshot_id)?;
    let normalized_snapshot = if snapshot_blob.is_none() {
        load_snapshot_by_id(connection, storage_key, &snapshot_id)
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let Some(mut snapshot) = snapshot_blob.or(normalized_snapshot) else {
        return Ok(None);
    };
    materialization_chain.reverse();
    for pair in materialization_chain.windows(2) {
        let Some(delta) = load_history_structural_delta(connection, repo_path, &pair[0], &pair[1])?
        else {
            return Ok(None);
        };
        if delta.before_revision != pair[0]
            || delta.after_revision != pair[1]
            || delta.before_snapshot_id != snapshot.id
        {
            return Ok(None);
        }
        let next_blob =
            load_history_snapshot_blob(connection, repo_path, &delta.after_snapshot_id)?;
        let next_normalized = if next_blob.is_none() {
            load_snapshot_by_id(connection, storage_key, &delta.after_snapshot_id)
                .map_err(|error| error.to_string())?
        } else {
            None
        };
        if let Some(next_snapshot) = next_blob.or(next_normalized) {
            snapshot = next_snapshot;
        } else if delta.materialization_version == 1 {
            snapshot = apply_structural_delta(snapshot, &delta)?;
        } else {
            return Ok(None);
        }
    }
    if snapshot.repo_head.as_deref() == Some(target_revision) {
        Ok(Some(snapshot))
    } else {
        Ok(None)
    }
}

pub(super) fn apply_structural_delta(
    mut snapshot: StructuralGraphSnapshot,
    delta: &HistoryStructuralDelta,
) -> Result<StructuralGraphSnapshot, String> {
    if snapshot.id != delta.before_snapshot_id || delta.materialization_version != 1 {
        return Err("Structural delta is incompatible with its base checkpoint".to_string());
    }
    let removed_nodes = delta.removed_node_ids.iter().collect::<HashSet<_>>();
    let upsert_nodes = delta
        .upsert_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    snapshot.nodes.retain(|node| {
        !removed_nodes.contains(&node.id) && !upsert_nodes.contains(node.id.as_str())
    });
    snapshot.nodes.extend(delta.upsert_nodes.iter().cloned());
    snapshot.nodes.sort_by(|left, right| left.id.cmp(&right.id));

    let removed_edges = delta.removed_edge_ids.iter().collect::<HashSet<_>>();
    let upsert_edges = delta
        .upsert_edges
        .iter()
        .map(|edge| edge.id.as_str())
        .collect::<HashSet<_>>();
    snapshot.edges.retain(|edge| {
        !removed_edges.contains(&edge.id) && !upsert_edges.contains(edge.id.as_str())
    });
    snapshot.edges.extend(delta.upsert_edges.iter().cloned());
    snapshot.edges.sort_by(|left, right| left.id.cmp(&right.id));

    let removed_files = delta
        .removed_file_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let upsert_files = delta
        .upsert_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    snapshot.files.retain(|file| {
        !removed_files.contains(file.path.as_str()) && !upsert_files.contains(file.path.as_str())
    });
    snapshot.files.extend(delta.upsert_files.iter().cloned());
    snapshot
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));

    let removed_metrics = delta.removed_metric_ids.iter().collect::<HashSet<_>>();
    let upsert_metrics = delta
        .upsert_metrics
        .iter()
        .map(|metric| metric.id.as_str())
        .collect::<HashSet<_>>();
    snapshot.metrics.retain(|metric| {
        !removed_metrics.contains(&metric.id) && !upsert_metrics.contains(metric.id.as_str())
    });
    snapshot
        .metrics
        .extend(delta.upsert_metrics.iter().cloned());
    let metric_order = delta
        .after_metric_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<HashMap<_, _>>();
    snapshot.metrics.sort_by_key(|metric| {
        metric_order
            .get(metric.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    let removed_clones = delta.removed_clone_group_ids.iter().collect::<HashSet<_>>();
    let upsert_clones = delta
        .upsert_clone_groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<HashSet<_>>();
    snapshot.clone_groups.retain(|group| {
        !removed_clones.contains(&group.id) && !upsert_clones.contains(group.id.as_str())
    });
    snapshot
        .clone_groups
        .extend(delta.upsert_clone_groups.iter().cloned());
    let clone_order = delta
        .after_clone_group_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<HashMap<_, _>>();
    snapshot.clone_groups.sort_by_key(|group| {
        clone_order
            .get(group.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    snapshot.id = delta.after_snapshot_id.clone();
    snapshot.repo_head = Some(delta.after_revision.clone());
    snapshot.created_at = delta.after_created_at.clone();
    snapshot.cursor = delta.after_cursor.clone();
    snapshot.ignore_fingerprint = delta.after_ignore_fingerprint.clone();
    snapshot.coverage = delta.after_coverage.clone();
    snapshot.diagnostics = delta.after_diagnostics.clone();
    snapshot.communities = delta.upsert_communities.clone();
    snapshot.truncated = delta.after_truncated;
    Ok(snapshot)
}
