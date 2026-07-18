use super::*;

#[derive(Debug, Default)]
pub struct BundledTreeSitterEngine;

impl StructuralGraphEngine for BundledTreeSitterEngine {
    fn info(&self) -> StructuralGraphEngineInfo {
        StructuralGraphEngineInfo {
            id: BUNDLED_ENGINE_ID.to_string(),
            version: BUNDLED_ENGINE_VERSION.to_string(),
            bundled: true,
            syntax_aware: true,
            supported_languages: supported_language_names(),
        }
    }

    fn build(
        &self,
        input: &StructuralGraphBuildInput,
        cancellation: &StructuralGraphCancellation,
        progress: &dyn StructuralGraphProgressSink,
    ) -> Result<StructuralGraphSnapshot, StructuralGraphError> {
        let root = input.repo_root.canonicalize().map_err(|error| {
            StructuralGraphError::InvalidRepository(format!(
                "Cannot resolve repository {}: {error}",
                input.repo_root.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(StructuralGraphError::InvalidRepository(format!(
                "Repository path is not a directory: {}",
                root.display()
            )));
        }
        if let Some(previous) = input.previous_snapshot.as_deref() {
            if input.previous_cursor != previous.cursor {
                return Err(StructuralGraphError::Parse(
                    "Incremental graph cursor does not match the previous snapshot; rebuild the index"
                        .to_string(),
                ));
            }
        }

        progress.report(StructuralGraphProgress {
            phase: "discover".to_string(),
            completed: 0,
            total: 0,
            detail: "Discovering repository files from Git".to_string(),
        });
        let incremental = input.previous_snapshot.is_some();
        let mut paths = if incremental {
            input
                .changed_files
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        } else {
            discover_paths(&root)?
        };
        paths.sort();
        paths.dedup();
        let truncated = paths.len() > input.max_files;
        paths.truncate(input.max_files);

        if cancellation.is_cancelled() {
            return Err(StructuralGraphError::Cancelled);
        }

        let completed = AtomicUsize::new(0);
        let total = paths.len();
        let contributions = paths
            .par_iter()
            .map(|path| {
                if cancellation.is_cancelled() {
                    return Err(StructuralGraphError::Cancelled);
                }
                let contribution = extract_path(&root, path, input.max_bytes_per_file);
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done == total || done.is_multiple_of(100) {
                    progress.report(StructuralGraphProgress {
                        phase: "extract".to_string(),
                        completed: done,
                        total,
                        detail: path.to_string_lossy().replace('\\', "/"),
                    });
                }
                Ok(contribution)
            })
            .collect::<Result<Vec<_>, StructuralGraphError>>()?;

        if cancellation.is_cancelled() {
            return Err(StructuralGraphError::Cancelled);
        }

        progress.report(StructuralGraphProgress {
            phase: "assemble".to_string(),
            completed: total,
            total,
            detail: "Assembling deterministic structural graph".to_string(),
        });

        let affected_paths = input
            .changed_files
            .iter()
            .chain(input.deleted_files.iter())
            .map(|path| path.replace('\\', "/"))
            .collect::<HashSet<_>>();
        let (mut files, mut nodes, mut edges, mut metrics, mut diagnostics, inherited_truncation) =
            if let Some(previous) = input.previous_snapshot.as_deref() {
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
                (
                    files,
                    nodes,
                    edges,
                    metrics,
                    diagnostics,
                    previous.truncated,
                )
            } else {
                (
                    contributions
                        .iter()
                        .map(file_record_from_contribution)
                        .collect(),
                    contributions
                        .iter()
                        .flat_map(|contribution| contribution.nodes.iter().cloned())
                        .collect(),
                    contributions
                        .iter()
                        .flat_map(|contribution| contribution.edges.iter().cloned())
                        .collect(),
                    contributions
                        .iter()
                        .flat_map(|contribution| contribution.metrics.iter().cloned())
                        .collect(),
                    contributions
                        .iter()
                        .flat_map(|contribution| contribution.diagnostics.iter().cloned())
                        .collect(),
                    false,
                )
            };
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
        let repo_path = root.to_string_lossy().to_string();
        let snapshot_id = stable_graph_id(
            "snapshot",
            &format!(
                "{}\0{}\0{}\0{}",
                repo_path,
                input.repo_head.as_deref().unwrap_or("working-tree"),
                BUNDLED_ENGINE_VERSION,
                cursor
            ),
        );

        Ok(StructuralGraphSnapshot {
            schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
            id: snapshot_id,
            repo_path,
            repo_head: input.repo_head.clone(),
            created_at: Utc::now().to_rfc3339(),
            engine: self.info(),
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
            truncated: truncated || inherited_truncation,
        })
    }
}
