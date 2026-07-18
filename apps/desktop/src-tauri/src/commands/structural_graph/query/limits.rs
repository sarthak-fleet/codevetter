use super::*;

pub(super) fn bounded_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn serialized_bytes<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn strip_node_excerpts(node: &mut StructuralGraphNode) {
    for source in &mut node.sources {
        source.excerpt = None;
    }
}

fn strip_edge_excerpts(edge: &mut StructuralGraphEdge) {
    for source in &mut edge.sources {
        source.excerpt = None;
    }
}

pub(super) fn enforce_projection_bytes(
    projection: &mut GraphProjection,
    protected_node_ids: &HashSet<String>,
) {
    if serialized_bytes(projection) <= MAX_RESPONSE_BYTES {
        return;
    }
    projection.truncated = true;
    for node in &mut projection.nodes {
        strip_node_excerpts(node);
    }
    for edge in &mut projection.edges {
        strip_edge_excerpts(edge);
    }
    while serialized_bytes(projection) > MAX_RESPONSE_BYTES && !projection.edges.is_empty() {
        projection.edges.pop();
    }
    while serialized_bytes(projection) > MAX_RESPONSE_BYTES {
        let Some(index) = projection
            .nodes
            .iter()
            .rposition(|node| !protected_node_ids.contains(&node.id))
        else {
            break;
        };
        projection.nodes.remove(index);
    }
    let retained = projection
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    projection.edges.retain(|edge| {
        retained.contains(edge.from.as_str()) && retained.contains(edge.to.as_str())
    });
}

pub(super) fn enforce_search_bytes(result: &mut GraphSearchResult, offset: usize, total: usize) {
    if serialized_bytes(result) <= MAX_RESPONSE_BYTES {
        return;
    }
    result.truncated = true;
    for hit in &mut result.hits {
        strip_node_excerpts(&mut hit.node);
    }
    while serialized_bytes(result) > MAX_RESPONSE_BYTES && !result.hits.is_empty() {
        result.hits.pop();
    }
    let next_offset = offset + result.hits.len();
    result.next_cursor = (next_offset < total).then(|| next_offset.to_string());
}

pub(super) fn enforce_path_bytes(result: &mut GraphPathResult) -> Result<(), String> {
    if serialized_bytes(result) <= MAX_RESPONSE_BYTES {
        return Ok(());
    }
    for node in &mut result.nodes {
        strip_node_excerpts(node);
    }
    for edge in &mut result.edges {
        strip_edge_excerpts(edge);
    }
    if serialized_bytes(result) > MAX_RESPONSE_BYTES {
        return Err(format!(
            "Graph path exceeds the {MAX_RESPONSE_BYTES}-byte response limit"
        ));
    }
    result.truncated = true;
    Ok(())
}

pub(super) fn enforce_impact_bytes(result: &mut GraphImpactResult) {
    if serialized_bytes(result) <= MAX_RESPONSE_BYTES {
        return;
    }
    result.truncated = true;
    strip_node_excerpts(&mut result.root);
    for node in &mut result.affected {
        strip_node_excerpts(node);
    }
    for edge in &mut result.edges {
        strip_edge_excerpts(edge);
    }
    while serialized_bytes(result) > MAX_RESPONSE_BYTES && !result.edges.is_empty() {
        result.edges.pop();
    }
    while serialized_bytes(result) > MAX_RESPONSE_BYTES && !result.affected.is_empty() {
        result.affected.pop();
    }
    let retained = result
        .affected
        .iter()
        .map(|node| node.id.as_str())
        .chain(std::iter::once(result.root.id.as_str()))
        .collect::<HashSet<_>>();
    result.edges.retain(|edge| {
        retained.contains(edge.from.as_str()) && retained.contains(edge.to.as_str())
    });
}

pub(super) fn parse_cursor(cursor: Option<&str>) -> Result<usize, String> {
    cursor
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "Graph cursor is invalid or expired".to_string())
}
