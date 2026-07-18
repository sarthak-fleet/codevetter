use super::*;

pub fn metadata(snapshot: &StructuralGraphSnapshot) -> StructuralGraphMetadata {
    StructuralGraphMetadata {
        snapshot_id: snapshot.id.clone(),
        schema_version: snapshot.schema_version,
        repo_path: snapshot.repo_path.clone(),
        repo_head: snapshot.repo_head.clone(),
        created_at: snapshot.created_at.clone(),
        engine_id: snapshot.engine.id.clone(),
        engine_version: snapshot.engine.version.clone(),
        indexed_files: snapshot.coverage.indexed_files,
        node_count: snapshot.nodes.len(),
        edge_count: snapshot.edges.len(),
        diagnostic_count: snapshot.diagnostics.len(),
        coverage: snapshot.coverage.clone(),
        trust: Some(trust_summary(snapshot)),
        freshness: GraphFreshness {
            indexed_head: snapshot.repo_head.clone(),
            current_head: None,
            stale: None,
        },
        truncated: snapshot.truncated,
    }
}

pub(super) fn query_context(snapshot: &StructuralGraphSnapshot) -> GraphQueryContext {
    GraphQueryContext {
        snapshot_id: snapshot.id.clone(),
        schema_version: snapshot.schema_version,
        engine_id: snapshot.engine.id.clone(),
        engine_version: snapshot.engine.version.clone(),
        created_at: snapshot.created_at.clone(),
        freshness: GraphFreshness {
            indexed_head: snapshot.repo_head.clone(),
            current_head: None,
            stale: None,
        },
        coverage: snapshot.coverage.clone(),
        trust: trust_summary(snapshot),
        max_results: MAX_LIMIT,
        max_edges: MAX_EDGE_LIMIT,
        max_hops: MAX_PATH_HOPS,
        max_bytes: MAX_RESPONSE_BYTES,
    }
}

fn trust_summary(snapshot: &StructuralGraphSnapshot) -> GraphTrustSummary {
    let mut summary = GraphTrustSummary::default();
    for trust in snapshot
        .nodes
        .iter()
        .map(|node| node.trust)
        .chain(snapshot.edges.iter().map(|edge| edge.trust))
    {
        match trust {
            GraphTrust::Extracted => summary.extracted += 1,
            GraphTrust::Inferred => summary.inferred += 1,
            GraphTrust::Ambiguous => summary.ambiguous += 1,
            GraphTrust::Legacy => summary.legacy += 1,
        }
    }
    summary
}

pub fn analysis(snapshot: &StructuralGraphSnapshot) -> GraphAnalysisResult {
    GraphAnalysisResult {
        analysis: analysis_summary(snapshot),
        truncated: snapshot.truncated,
        context: query_context(snapshot),
    }
}

pub fn analysis_summary(snapshot: &StructuralGraphSnapshot) -> StructuralGraphAnalysisSummary {
    summarize_graph_analysis_with_context(
        &snapshot.nodes,
        &snapshot.edges,
        &snapshot.communities,
        &snapshot.coverage,
        snapshot.truncated,
    )
}

pub fn overview(snapshot: &StructuralGraphSnapshot, limit: Option<usize>) -> GraphProjection {
    overview_page(snapshot, limit, None).expect("default graph cursor is valid")
}

pub fn overview_page(
    snapshot: &StructuralGraphSnapshot,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<GraphProjection, String> {
    let limit = bounded_limit(limit);
    let offset = parse_cursor(cursor)?;
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for edge in &snapshot.edges {
        *degree.entry(&edge.from).or_default() += 1;
        *degree.entry(&edge.to).or_default() += 1;
    }
    let mut ranked = snapshot.nodes.iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        degree
            .get(right.id.as_str())
            .copied()
            .unwrap_or_default()
            .cmp(&degree.get(left.id.as_str()).copied().unwrap_or_default())
            .then_with(|| left.id.cmp(&right.id))
    });
    if offset > ranked.len() {
        return Err("Graph cursor is invalid or expired".to_string());
    }
    let page = ranked
        .iter()
        .skip(offset)
        .take(limit)
        .copied()
        .collect::<Vec<_>>();
    let next_offset = offset + page.len();
    let selected = page
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut nodes = page.into_iter().cloned().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| selected.contains(edge.from.as_str()) && selected.contains(edge.to.as_str()))
        .take(MAX_EDGE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let edge_truncated = snapshot
        .edges
        .iter()
        .filter(|edge| selected.contains(edge.from.as_str()) && selected.contains(edge.to.as_str()))
        .count()
        > edges.len();
    let mut projection = GraphProjection {
        nodes,
        edges,
        truncated: next_offset < ranked.len() || edge_truncated,
        next_cursor: (next_offset < ranked.len()).then(|| next_offset.to_string()),
        context: query_context(snapshot),
    };
    enforce_projection_bytes(&mut projection, &HashSet::new());
    Ok(projection)
}

pub fn community(
    snapshot: &StructuralGraphSnapshot,
    community_id: &str,
    limit: Option<usize>,
) -> Result<GraphProjection, String> {
    community_page(snapshot, community_id, limit, None)
}

pub fn community_page(
    snapshot: &StructuralGraphSnapshot,
    community_id: &str,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<GraphProjection, String> {
    if !snapshot
        .communities
        .iter()
        .any(|community| community.id == community_id)
    {
        return Err(format!("No graph community matches '{community_id}'"));
    }
    let limit = bounded_limit(limit);
    let offset = parse_cursor(cursor)?;
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for edge in &snapshot.edges {
        *degree.entry(&edge.from).or_default() += 1;
        *degree.entry(&edge.to).or_default() += 1;
    }
    let mut members = snapshot
        .nodes
        .iter()
        .filter(|node| node.community_id.as_deref() == Some(community_id))
        .collect::<Vec<_>>();
    members.sort_by(|left, right| {
        degree
            .get(right.id.as_str())
            .copied()
            .unwrap_or_default()
            .cmp(&degree.get(left.id.as_str()).copied().unwrap_or_default())
            .then_with(|| left.id.cmp(&right.id))
    });
    if offset > members.len() {
        return Err("Graph cursor is invalid or expired".to_string());
    }
    let total_members = members.len();
    let members = members
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let next_offset = offset + members.len();
    let selected = members
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut nodes = members.into_iter().cloned().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| selected.contains(edge.from.as_str()) && selected.contains(edge.to.as_str()))
        .take(MAX_EDGE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let edge_truncated = snapshot
        .edges
        .iter()
        .filter(|edge| selected.contains(edge.from.as_str()) && selected.contains(edge.to.as_str()))
        .count()
        > edges.len();
    let mut projection = GraphProjection {
        nodes,
        edges,
        truncated: next_offset < total_members || edge_truncated,
        next_cursor: (next_offset < total_members).then(|| next_offset.to_string()),
        context: query_context(snapshot),
    };
    enforce_projection_bytes(&mut projection, &HashSet::new());
    Ok(projection)
}

pub fn subgraph(
    snapshot: &StructuralGraphSnapshot,
    seeds: &[String],
    depth: Option<usize>,
    filter: &GraphQueryFilter,
    limit: Option<usize>,
) -> Result<GraphProjection, String> {
    if seeds.is_empty() {
        return Err("At least one graph seed is required".to_string());
    }
    let max_depth = depth.unwrap_or(2).clamp(0, 8);
    let limit = bounded_limit(limit);
    let roots = seeds
        .iter()
        .map(|seed| resolve_node(snapshot, seed))
        .collect::<Result<Vec<_>, _>>()?;
    let mut adjacency = HashMap::<&str, Vec<&StructuralGraphEdge>>::new();
    for edge in snapshot
        .edges
        .iter()
        .filter(|edge| edge_matches_filter(edge, filter))
    {
        adjacency.entry(edge.from.as_str()).or_default().push(edge);
        adjacency.entry(edge.to.as_str()).or_default().push(edge);
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| left.id.cmp(&right.id));
    }
    let mut queue = roots
        .iter()
        .map(|node| (node.id.as_str(), 0_usize))
        .collect::<VecDeque<_>>();
    let mut selected = roots
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut selected_edges = HashSet::new();
    let mut truncated = false;
    while let Some((node_id, current_depth)) = queue.pop_front() {
        if current_depth >= max_depth {
            continue;
        }
        for edge in adjacency.get(node_id).into_iter().flatten() {
            let neighbor = if edge.from == node_id {
                edge.to.as_str()
            } else {
                edge.from.as_str()
            };
            if selected.len() >= limit && !selected.contains(neighbor) {
                truncated = true;
                continue;
            }
            selected_edges.insert(edge.id.as_str());
            if selected.insert(neighbor) {
                queue.push_back((neighbor, current_depth + 1));
            }
        }
    }
    let mut nodes = snapshot
        .nodes
        .iter()
        .filter(|node| selected.contains(node.id.as_str()))
        .filter(|node| {
            node_matches_filter(node, filter) || roots.iter().any(|root| root.id == node.id)
        })
        .cloned()
        .collect::<Vec<_>>();
    let retained = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| selected_edges.contains(edge.id.as_str()))
        .filter(|edge| retained.contains(edge.from.as_str()) && retained.contains(edge.to.as_str()))
        .take(MAX_EDGE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    if selected_edges.len() > edges.len() {
        truncated = true;
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let protected = roots
        .iter()
        .map(|root| root.id.clone())
        .collect::<HashSet<_>>();
    let mut projection = GraphProjection {
        nodes,
        edges,
        truncated,
        next_cursor: None,
        context: query_context(snapshot),
    };
    enforce_projection_bytes(&mut projection, &protected);
    Ok(projection)
}

pub fn diff_snapshots(
    before: &StructuralGraphSnapshot,
    after: &StructuralGraphSnapshot,
) -> GraphSnapshotDiff {
    let before_nodes = before
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let after_nodes = after
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let before_edges = before
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect::<HashMap<_, _>>();
    let after_edges = after
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect::<HashMap<_, _>>();
    let (mut added_node_ids, mut removed_node_ids, mut changed_node_ids) =
        diff_identity_maps(&before_nodes, &after_nodes);
    let (mut added_edge_ids, mut removed_edge_ids, mut changed_edge_ids) =
        diff_identity_maps(&before_edges, &after_edges);
    let truncated = [
        added_node_ids.len(),
        removed_node_ids.len(),
        changed_node_ids.len(),
        added_edge_ids.len(),
        removed_edge_ids.len(),
        changed_edge_ids.len(),
    ]
    .into_iter()
    .any(|count| count > MAX_DIFF_IDS);
    for ids in [
        &mut added_node_ids,
        &mut removed_node_ids,
        &mut changed_node_ids,
        &mut added_edge_ids,
        &mut removed_edge_ids,
        &mut changed_edge_ids,
    ] {
        ids.truncate(MAX_DIFF_IDS);
    }
    GraphSnapshotDiff {
        before_snapshot_id: before.id.clone(),
        after_snapshot_id: after.id.clone(),
        added_node_ids,
        removed_node_ids,
        changed_node_ids,
        added_edge_ids,
        removed_edge_ids,
        changed_edge_ids,
        truncated,
        context: query_context(after),
    }
}

fn diff_identity_maps<T: PartialEq>(
    before: &HashMap<&str, &T>,
    after: &HashMap<&str, &T>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut added = after
        .keys()
        .filter(|id| !before.contains_key(**id))
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut removed = before
        .keys()
        .filter(|id| !after.contains_key(**id))
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut changed = after
        .iter()
        .filter_map(|(id, value)| {
            before
                .get(id)
                .filter(|before| *before != value)
                .map(|_| (*id).to_string())
        })
        .collect::<Vec<_>>();
    added.sort();
    removed.sort();
    changed.sort();
    (added, removed, changed)
}
