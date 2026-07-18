use super::*;

pub fn search(
    snapshot: &StructuralGraphSnapshot,
    query: &str,
    filter: &GraphQueryFilter,
    limit: Option<usize>,
) -> GraphSearchResult {
    search_page(snapshot, query, filter, limit, None).expect("default graph cursor is valid")
}

pub fn search_page(
    snapshot: &StructuralGraphSnapshot,
    query: &str,
    filter: &GraphQueryFilter,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<GraphSearchResult, String> {
    let needle = normalize(query);
    if needle.is_empty() {
        return Ok(GraphSearchResult {
            hits: Vec::new(),
            truncated: false,
            next_cursor: None,
            context: query_context(snapshot),
        });
    }

    let tokens = lexical_tokens(&needle);
    let index = query_index(snapshot);
    let mut candidate_indices = HashSet::new();
    if let Some(exact) = index.exact.get(&needle) {
        candidate_indices.extend(exact.iter().copied());
    }
    for token in &tokens {
        if let Some(postings) = index.tokens.get(token) {
            candidate_indices.extend(postings.iter().copied());
        }
    }
    let candidates = if candidate_indices.is_empty() {
        (0..snapshot.nodes.len()).collect::<Vec<_>>()
    } else {
        let mut candidates = candidate_indices.into_iter().collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates
    };
    let mut hits = candidates
        .into_iter()
        .filter_map(|index| snapshot.nodes.get(index))
        .filter(|node| node_matches_filter(node, filter))
        .filter_map(|node| {
            rank_node(node, &needle)
                .or_else(|| rank_question_tokens(node, &tokens))
                .map(|(score, matched_by)| (node, score, matched_by))
        })
        .collect::<Vec<_>>();
    hits.sort_by(|(left_node, left_score, _), (right_node, right_score, _)| {
        left_score
            .cmp(right_score)
            .then_with(|| left_node.label.cmp(&right_node.label))
            .then_with(|| left_node.id.cmp(&right_node.id))
    });

    let offset = parse_cursor(cursor)?;
    if offset > hits.len() {
        return Err("Graph cursor is invalid or expired".to_string());
    }
    let limit = bounded_limit(limit);
    let total = hits.len();
    let page = hits
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let next_offset = offset + page.len();
    let mut result = GraphSearchResult {
        hits: page
            .into_iter()
            .map(|(node, score, matched_by)| GraphSearchHit {
                node: node.clone(),
                score,
                matched_by,
            })
            .collect(),
        truncated: next_offset < total,
        next_cursor: (next_offset < total).then(|| next_offset.to_string()),
        context: query_context(snapshot),
    };
    enforce_search_bytes(&mut result, offset, total);
    Ok(result)
}

pub fn resolve_node<'a>(
    snapshot: &'a StructuralGraphSnapshot,
    reference: &str,
) -> Result<&'a StructuralGraphNode, String> {
    let needle = normalize(reference);
    if needle.is_empty() {
        return Err("A node id, qualified name, path, or label is required".to_string());
    }

    let index = query_index(snapshot);
    let candidates = index
        .exact
        .get(&needle)
        .cloned()
        .unwrap_or_else(|| (0..snapshot.nodes.len()).collect());
    for exact_score in 0..=3 {
        let matches = candidates
            .iter()
            .filter_map(|index| snapshot.nodes.get(*index))
            .filter(|node| rank_node(node, &needle).is_some_and(|(score, _)| score == exact_score))
            .collect::<Vec<_>>();
        match matches.len() {
            0 => continue,
            1 => return Ok(matches[0]),
            count => {
                return Err(format!(
                    "Node reference is ambiguous ({count} matches); use the stable node id"
                ))
            }
        }
    }
    Err(format!("No graph node matches '{reference}'"))
}

pub fn explain(
    snapshot: &StructuralGraphSnapshot,
    reference: &str,
) -> Result<GraphExplanation, String> {
    let node = resolve_node(snapshot, reference)?;
    let mut incoming_kinds = HashSet::new();
    let mut outgoing_kinds = HashSet::new();
    let mut incoming_count = 0;
    let mut outgoing_count = 0;
    for edge in &snapshot.edges {
        if edge.to == node.id {
            incoming_count += 1;
            incoming_kinds.insert(edge.kind.clone());
        }
        if edge.from == node.id {
            outgoing_count += 1;
            outgoing_kinds.insert(edge.kind.clone());
        }
    }
    let mut incoming_kinds = incoming_kinds.into_iter().collect::<Vec<_>>();
    let mut outgoing_kinds = outgoing_kinds.into_iter().collect::<Vec<_>>();
    incoming_kinds.sort();
    outgoing_kinds.sort();
    Ok(GraphExplanation {
        node: node.clone(),
        incoming_count,
        outgoing_count,
        incoming_kinds,
        outgoing_kinds,
        truncated: false,
        context: query_context(snapshot),
    })
}

pub fn neighbors(
    snapshot: &StructuralGraphSnapshot,
    reference: &str,
    direction: GraphDirection,
    filter: &GraphQueryFilter,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<GraphProjection, String> {
    let root = resolve_node(snapshot, reference)?;
    let node_by_id = node_map(snapshot);
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| edge_matches_filter(edge, filter))
        .filter(|edge| match direction {
            GraphDirection::Incoming => edge.to == root.id,
            GraphDirection::Outgoing => edge.from == root.id,
            GraphDirection::Both => edge.from == root.id || edge.to == root.id,
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.id.cmp(&right.id))
    });

    let offset = parse_cursor(cursor)?;
    let limit = bounded_limit(limit);
    let page = edges
        .iter()
        .skip(offset)
        .take(limit)
        .copied()
        .collect::<Vec<_>>();
    let truncated = offset + page.len() < edges.len();
    let mut node_ids = HashSet::from([root.id.as_str()]);
    for edge in &page {
        node_ids.insert(edge.from.as_str());
        node_ids.insert(edge.to.as_str());
    }
    let mut nodes = node_ids
        .into_iter()
        .filter_map(|id| node_by_id.get(id).copied())
        .filter(|node| node_matches_filter(node, filter) || node.id == root.id)
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let protected = HashSet::from([root.id.clone()]);
    let mut projection = GraphProjection {
        nodes,
        edges: page.into_iter().cloned().collect(),
        truncated,
        next_cursor: truncated.then(|| (offset + limit).to_string()),
        context: query_context(snapshot),
    };
    enforce_projection_bytes(&mut projection, &protected);
    Ok(projection)
}
