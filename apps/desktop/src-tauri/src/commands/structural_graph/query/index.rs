use super::*;

pub(super) fn normalize(value: &str) -> String {
    value.trim().replace('\\', "/").to_lowercase()
}

pub(super) fn node_matches_filter(node: &StructuralGraphNode, filter: &GraphQueryFilter) -> bool {
    (filter.node_kinds.is_empty() || filter.node_kinds.iter().any(|kind| kind == &node.kind))
        && (filter.trust.is_empty() || filter.trust.contains(&node.trust))
}

pub(super) fn edge_matches_filter(edge: &StructuralGraphEdge, filter: &GraphQueryFilter) -> bool {
    (filter.edge_kinds.is_empty() || filter.edge_kinds.iter().any(|kind| kind == &edge.kind))
        && (filter.trust.is_empty() || filter.trust.contains(&edge.trust))
}

pub(super) fn rank_node(node: &StructuralGraphNode, needle: &str) -> Option<(u32, String)> {
    let id = normalize(&node.id);
    let qualified = node.qualified_name.as_deref().map(normalize);
    let path = node.path.as_deref().map(normalize);
    let label = normalize(&node.label);
    if id == needle {
        Some((0, "id".to_string()))
    } else if qualified.as_deref() == Some(needle) {
        Some((1, "qualified_name".to_string()))
    } else if path.as_deref() == Some(needle) {
        Some((2, "path".to_string()))
    } else if label == needle {
        Some((3, "label".to_string()))
    } else if qualified
        .as_deref()
        .is_some_and(|value| value.contains(needle))
    {
        Some((10, "qualified_name_contains".to_string()))
    } else if path.as_deref().is_some_and(|value| value.contains(needle)) {
        Some((11, "path_contains".to_string()))
    } else if label.contains(needle) {
        Some((12, "label_contains".to_string()))
    } else {
        None
    }
}

pub(super) fn lexical_tokens(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "does", "for", "from", "how", "in", "is", "of", "on", "or", "the",
        "to", "what", "when", "where", "which", "why", "with",
    ];
    let mut tokens = query
        .split(|character: char| {
            !(character.is_alphanumeric()
                || matches!(character, '_' | '-' | '.' | '/' | ':' | '\\'))
        })
        .map(str::trim)
        .filter(|token| token.len() >= 2 && !STOP_WORDS.contains(token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

pub(super) fn rank_question_tokens(
    node: &StructuralGraphNode,
    tokens: &[String],
) -> Option<(u32, String)> {
    if tokens.is_empty() {
        return None;
    }
    let haystack = normalize(&format!(
        "{} {} {} {} {}",
        node.label,
        node.qualified_name.as_deref().unwrap_or_default(),
        node.path.as_deref().unwrap_or_default(),
        node.kind,
        node.detail.as_deref().unwrap_or_default()
    ));
    let matched = tokens
        .iter()
        .filter(|token| haystack.contains(token.as_str()))
        .count();
    if matched == 0 {
        return None;
    }
    let missing = tokens.len() - matched;
    Some((20 + missing as u32 * 5, "lexical_question".to_string()))
}

pub(super) fn node_map(snapshot: &StructuralGraphSnapshot) -> HashMap<&str, &StructuralGraphNode> {
    snapshot
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect()
}

pub(super) fn query_index(snapshot: &StructuralGraphSnapshot) -> Arc<StructuralGraphQueryIndex> {
    let indexes = QUERY_INDEXES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = indexes.lock() {
        if let Some(index) = cache.get(&snapshot.id) {
            return Arc::clone(index);
        }
    }
    let mut index = StructuralGraphQueryIndex::default();
    for (ordinal, node) in snapshot.nodes.iter().enumerate() {
        for value in [
            Some(node.id.as_str()),
            node.path.as_deref(),
            node.qualified_name.as_deref(),
            Some(node.label.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            index
                .exact
                .entry(normalize(value))
                .or_default()
                .push(ordinal);
        }
        let searchable = normalize(&format!(
            "{} {} {} {} {}",
            node.label,
            node.qualified_name.as_deref().unwrap_or_default(),
            node.path.as_deref().unwrap_or_default(),
            node.kind,
            node.detail.as_deref().unwrap_or_default()
        ));
        for token in lexical_tokens(&searchable) {
            index.tokens.entry(token).or_default().push(ordinal);
        }
    }
    for postings in index.tokens.values_mut() {
        postings.sort_unstable();
        postings.dedup();
    }
    let index = Arc::new(index);
    if let Ok(mut cache) = indexes.lock() {
        if cache.len() >= MAX_QUERY_INDEXES {
            cache.clear();
        }
        cache.insert(snapshot.id.clone(), Arc::clone(&index));
    }
    index
}

pub(super) fn trust_cost(trust: GraphTrust) -> f64 {
    match trust {
        GraphTrust::Extracted => 1.0,
        GraphTrust::Inferred => 1.6,
        GraphTrust::Ambiguous => 3.5,
        GraphTrust::Legacy => 4.0,
    }
}
