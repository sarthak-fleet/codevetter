use super::*;

#[derive(Debug, Clone)]
pub struct HistoricalFileBlob {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn build_snapshot_from_blobs(
    storage_repo_path: &str,
    revision: &str,
    mut blobs: Vec<HistoricalFileBlob>,
    cancellation: &StructuralGraphCancellation,
    progress: &dyn StructuralGraphProgressSink,
) -> Result<StructuralGraphSnapshot, StructuralGraphError> {
    blobs.sort_by(|left, right| left.path.cmp(&right.path));
    blobs.dedup_by(|left, right| left.path == right.path);
    let truncated = blobs.len() > 25_000;
    blobs.truncate(25_000);
    let total = blobs.len();
    let completed = AtomicUsize::new(0);
    let contributions = blobs
        .par_iter()
        .map(|blob| {
            if cancellation.is_cancelled() {
                return Err(StructuralGraphError::Cancelled);
            }
            let contribution = extract_blob(&blob.path, &blob.bytes, 2 * 1024 * 1024);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if done == total || done.is_multiple_of(100) {
                progress.report(StructuralGraphProgress {
                    phase: "historical_extract".to_string(),
                    completed: done,
                    total,
                    detail: blob.path.clone(),
                });
            }
            Ok(contribution)
        })
        .collect::<Result<Vec<_>, StructuralGraphError>>()?;
    if cancellation.is_cancelled() {
        return Err(StructuralGraphError::Cancelled);
    }
    let files = contributions
        .iter()
        .map(file_record_from_contribution)
        .collect::<Vec<_>>();
    let nodes = contributions
        .iter()
        .flat_map(|contribution| contribution.nodes.iter().cloned())
        .collect::<Vec<_>>();
    let edges = contributions
        .iter()
        .flat_map(|contribution| contribution.edges.iter().cloned())
        .collect::<Vec<_>>();
    let metrics = contributions
        .iter()
        .flat_map(|contribution| contribution.metrics.iter().cloned())
        .collect::<Vec<_>>();
    let diagnostics = contributions
        .iter()
        .flat_map(|contribution| contribution.diagnostics.iter().cloned())
        .collect::<Vec<_>>();
    finalize_historical_snapshot(
        storage_repo_path,
        revision,
        files,
        nodes,
        edges,
        metrics,
        diagnostics,
        truncated,
    )
}

pub fn build_snapshot_from_blob_delta(
    storage_repo_path: &str,
    revision: &str,
    previous: &StructuralGraphSnapshot,
    mut changed_blobs: Vec<HistoricalFileBlob>,
    deleted_paths: &[String],
    cancellation: &StructuralGraphCancellation,
    progress: &dyn StructuralGraphProgressSink,
) -> Result<StructuralGraphSnapshot, StructuralGraphError> {
    changed_blobs.sort_by(|left, right| left.path.cmp(&right.path));
    changed_blobs.dedup_by(|left, right| left.path == right.path);
    let total = changed_blobs.len();
    let completed = AtomicUsize::new(0);
    let contributions = changed_blobs
        .par_iter()
        .map(|blob| {
            if cancellation.is_cancelled() {
                return Err(StructuralGraphError::Cancelled);
            }
            let contribution = extract_blob(&blob.path, &blob.bytes, 2 * 1024 * 1024);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if done == total || done.is_multiple_of(100) {
                progress.report(StructuralGraphProgress {
                    phase: "historical_delta_extract".to_string(),
                    completed: done,
                    total,
                    detail: blob.path.clone(),
                });
            }
            Ok(contribution)
        })
        .collect::<Result<Vec<_>, StructuralGraphError>>()?;
    if cancellation.is_cancelled() {
        return Err(StructuralGraphError::Cancelled);
    }
    let affected_paths = changed_blobs
        .iter()
        .map(|blob| blob.path.replace('\\', "/"))
        .chain(deleted_paths.iter().map(|path| path.replace('\\', "/")))
        .collect::<HashSet<_>>();
    let mut nodes = previous
        .nodes
        .iter()
        .filter(|node| !node_belongs_to_paths(node, &affected_paths))
        .cloned()
        .collect::<Vec<_>>();
    let retained_node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut edges = previous
        .edges
        .iter()
        .filter(|edge| {
            !matches!(edge.origin, GraphOrigin::Resolution | GraphOrigin::Analysis)
                && retained_node_ids.contains(edge.from.as_str())
                && retained_node_ids.contains(edge.to.as_str())
                && !sources_touch_paths(&edge.sources, &affected_paths)
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut diagnostics = previous
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .path
                .as_ref()
                .is_none_or(|path| !affected_paths.contains(path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut metrics = previous
        .metrics
        .iter()
        .filter(|fact| !affected_paths.contains(&fact.path))
        .cloned()
        .collect::<Vec<_>>();
    let mut files = previous
        .files
        .iter()
        .filter(|file| !affected_paths.contains(&file.path))
        .cloned()
        .collect::<Vec<_>>();
    nodes.extend(
        contributions
            .iter()
            .flat_map(|contribution| contribution.nodes.iter().cloned()),
    );
    edges.extend(
        contributions
            .iter()
            .flat_map(|contribution| contribution.edges.iter().cloned()),
    );
    diagnostics.extend(
        contributions
            .iter()
            .flat_map(|contribution| contribution.diagnostics.iter().cloned()),
    );
    metrics.extend(
        contributions
            .iter()
            .flat_map(|contribution| contribution.metrics.iter().cloned()),
    );
    files.extend(contributions.iter().map(file_record_from_contribution));
    finalize_historical_snapshot(
        storage_repo_path,
        revision,
        files,
        nodes,
        edges,
        metrics,
        diagnostics,
        previous.truncated,
    )
}

fn finalize_historical_snapshot(
    storage_repo_path: &str,
    revision: &str,
    mut files: Vec<StructuralGraphFileRecord>,
    mut nodes: Vec<StructuralGraphNode>,
    mut edges: Vec<StructuralGraphEdge>,
    mut metrics: Vec<StructuralGraphMetricFact>,
    mut diagnostics: Vec<StructuralGraphDiagnostic>,
    truncated: bool,
) -> Result<StructuralGraphSnapshot, StructuralGraphError> {
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);
    let coverage = coverage_from_file_records(&files);
    deduplicate_nodes(&mut nodes);
    deduplicate_edges(&mut edges);
    resolve_cross_file(&nodes, &mut edges);
    deduplicate_edges(&mut edges);
    deduplicate_metrics(&mut metrics);
    finalize_metric_degrees(&mut metrics, &edges);
    let clone_groups = detect_clone_groups(&metrics);
    let communities = analyze_graph(&mut nodes, &edges);
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    let cursor_identity = files
        .iter()
        .map(|file| {
            file.content_hash
                .as_ref()
                .map(|hash| format!("{}\0{hash}", file.path))
                .unwrap_or_else(|| format!("{}\0{}", file.path, file.disposition))
        })
        .collect::<Vec<_>>()
        .join("\0");
    let cursor = stable_graph_id("cursor", &cursor_identity);
    let snapshot_id = stable_graph_id(
        "historical-snapshot",
        &format!(
            "{storage_repo_path}\0{revision}\0{}\0{cursor}",
            BUNDLED_ENGINE_VERSION
        ),
    );
    Ok(StructuralGraphSnapshot {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
        id: snapshot_id,
        repo_path: storage_repo_path.to_string(),
        repo_head: Some(revision.to_string()),
        created_at: Utc::now().to_rfc3339(),
        engine: BundledTreeSitterEngine.info(),
        cursor: Some(cursor),
        ignore_fingerprint: Some(current_ignore_fingerprint()),
        coverage,
        diagnostics,
        communities,
        files,
        nodes,
        edges,
        metrics,
        clone_groups,
        truncated,
    })
}
