use crate::commands::unpack_types::{
    RepoGraph, RepoGraphEdge, RepoGraphNode, RepoGraphSourceLocation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs;

pub const MAX_GRAPH_IMPORT_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_GRAPH_IMPORT_NODES: usize = 5_000;
pub const MAX_GRAPH_IMPORT_EDGES: usize = 10_000;
pub const DEFAULT_PATH_HOPS: usize = 8;
pub const DEFAULT_PATH_VISITED: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEndpointCandidate {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphEndpointStatus {
    Resolved,
    Ambiguous,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEndpointResolution {
    pub query: String,
    pub status: GraphEndpointStatus,
    pub selected: Option<GraphEndpointCandidate>,
    pub candidates: Vec<GraphEndpointCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphPathHop {
    pub from: RepoGraphNode,
    pub to: RepoGraphNode,
    pub kind: String,
    pub trust: String,
    pub origin: String,
    pub confidence_label: Option<String>,
    pub evidence: String,
    pub sources: Vec<String>,
    pub follows_stored_direction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphPathBounds {
    pub max_hops: usize,
    pub max_visited_nodes: usize,
    pub visited_nodes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphPathResult {
    pub source: GraphEndpointResolution,
    pub target: GraphEndpointResolution,
    pub hops: Vec<GraphPathHop>,
    pub found: bool,
    pub trust_summary: String,
    pub requires_verification: bool,
    pub message: String,
    pub bounds: GraphPathBounds,
}

fn bounded_text(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn value_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value_string(value.get(*key)))
        .filter(|value| !value.trim().is_empty())
}

fn endpoint_id(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Object(object)) => first_string(&Value::Object(object.clone()), &["id", "key"]),
        other => value_string(other),
    }
}

fn parse_location(value: &Value) -> Option<RepoGraphSourceLocation> {
    if let Some(location) = value
        .get("source_location")
        .or_else(|| value.get("location"))
    {
        match location {
            Value::String(path) if !path.trim().is_empty() => {
                return Some(RepoGraphSourceLocation {
                    path: bounded_text(path, 1_024),
                    line: None,
                    column: None,
                });
            }
            Value::Object(_) => {
                let path = first_string(location, &["path", "file", "source_file"])?;
                return Some(RepoGraphSourceLocation {
                    path: bounded_text(&path, 1_024),
                    line: location.get("line").and_then(Value::as_u64),
                    column: location.get("column").and_then(Value::as_u64),
                });
            }
            _ => {}
        }
    }
    let path = first_string(value, &["source_file", "file"])?;
    Some(RepoGraphSourceLocation {
        path: bounded_text(&path, 1_024),
        line: value.get("line").and_then(Value::as_u64),
        column: value.get("column").and_then(Value::as_u64),
    })
}

fn imported_trust(confidence: Option<&str>) -> String {
    match confidence
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "extracted" | "high" | "certain" | "verified" => "extracted",
        "inferred" | "medium" | "moderate" | "likely" => "inferred",
        _ => "ambiguous",
    }
    .to_string()
}

pub fn normalize_external_graph_json(bytes: &[u8]) -> Result<RepoGraph, String> {
    if bytes.len() as u64 > MAX_GRAPH_IMPORT_BYTES {
        return Err(format!(
            "Graph JSON is too large ({} bytes; limit is {} bytes).",
            bytes.len(),
            MAX_GRAPH_IMPORT_BYTES
        ));
    }
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Graph JSON is malformed: {error}"))?;
    let nodes_raw = root
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Unsupported graph JSON: expected a `nodes` array.".to_string())?;
    let edges_raw = root
        .get("links")
        .or_else(|| root.get("edges"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Unsupported graph JSON: expected a `links` or `edges` array.".to_string()
        })?;
    if nodes_raw.len() > MAX_GRAPH_IMPORT_NODES {
        return Err(format!(
            "Graph has too many nodes ({}; limit is {}).",
            nodes_raw.len(),
            MAX_GRAPH_IMPORT_NODES
        ));
    }
    if edges_raw.len() > MAX_GRAPH_IMPORT_EDGES {
        return Err(format!(
            "Graph has too many relationships ({}; limit is {}).",
            edges_raw.len(),
            MAX_GRAPH_IMPORT_EDGES
        ));
    }

    let mut nodes = Vec::with_capacity(nodes_raw.len());
    let mut ids = HashSet::new();
    for (index, raw) in nodes_raw.iter().enumerate() {
        let id = bounded_text(
            &first_string(raw, &["id", "key"])
                .ok_or_else(|| format!("Node {} has no usable `id`.", index + 1))?,
            512,
        );
        if !ids.insert(id.clone()) {
            return Err(format!("Graph contains duplicate node id `{id}`."));
        }
        let label = first_string(raw, &["label", "name", "title"]).unwrap_or_else(|| id.clone());
        let path = first_string(raw, &["path", "source_file", "file"]);
        let source_location = parse_location(raw);
        let mut sources = Vec::new();
        if let Some(location) = &source_location {
            sources.push(location.path.clone());
        } else if let Some(path) = &path {
            sources.push(path.clone());
        }
        nodes.push(RepoGraphNode {
            id,
            kind: bounded_text(
                &first_string(raw, &["kind", "type"]).unwrap_or_else(|| "concept".to_string()),
                128,
            ),
            label: bounded_text(&label, 1_024),
            path: path.map(|value| bounded_text(&value, 1_024)),
            detail: first_string(raw, &["detail", "description"])
                .map(|value| bounded_text(&value, 2_048)),
            sources,
            source_location,
            community: first_string(raw, &["community", "community_id", "cluster"])
                .map(|value| bounded_text(&value, 256)),
        });
    }

    let mut edges = Vec::with_capacity(edges_raw.len());
    for (index, raw) in edges_raw.iter().enumerate() {
        let from = bounded_text(
            &endpoint_id(raw.get("source").or_else(|| raw.get("from"))).ok_or_else(|| {
                format!("Relationship {} has no source/from endpoint.", index + 1)
            })?,
            512,
        );
        let to = bounded_text(
            &endpoint_id(raw.get("target").or_else(|| raw.get("to")))
                .ok_or_else(|| format!("Relationship {} has no target/to endpoint.", index + 1))?,
            512,
        );
        if !ids.contains(&from) || !ids.contains(&to) {
            return Err(format!(
                "Relationship {} references a missing endpoint (`{from}` -> `{to}`).",
                index + 1
            ));
        }
        let kind = first_string(raw, &["relation", "kind", "type"])
            .unwrap_or_else(|| "related_to".to_string());
        let confidence_label = first_string(raw, &["confidence", "trust"]);
        let location = parse_location(raw);
        let mut sources = Vec::new();
        if let Some(location) = &location {
            sources.push(match (location.line, location.column) {
                (Some(line), Some(column)) => format!("{}#L{line}:C{column}", location.path),
                (Some(line), None) => format!("{}#L{line}", location.path),
                _ => location.path.clone(),
            });
        }
        if let Some(Value::Array(extra)) = raw.get("sources") {
            sources.extend(
                extra
                    .iter()
                    .filter_map(|value| value.as_str().map(ToOwned::to_owned)),
            );
        }
        sources.sort();
        sources.dedup();
        let evidence = first_string(raw, &["evidence", "description"])
            .unwrap_or_else(|| format!("Imported relationship `{kind}`"));
        edges.push(RepoGraphEdge {
            from,
            to,
            kind: bounded_text(&kind, 256),
            evidence: bounded_text(&evidence, 2_048),
            sources: sources
                .into_iter()
                .map(|value| bounded_text(&value, 1_024))
                .collect(),
            trust: imported_trust(confidence_label.as_deref()),
            origin: "imported".to_string(),
            confidence_label: confidence_label.map(|value| bounded_text(&value, 128)),
        });
    }

    Ok(RepoGraph {
        schema_version: 2,
        nodes,
        edges,
        truncated: false,
    })
}

#[tauri::command]
pub async fn import_external_graph_preview(file_path: String) -> Result<RepoGraph, String> {
    let metadata = fs::metadata(&file_path)
        .map_err(|error| format!("Cannot inspect selected graph file: {error}"))?;
    if !metadata.is_file() {
        return Err("Selected graph path is not a file.".to_string());
    }
    if metadata.len() > MAX_GRAPH_IMPORT_BYTES {
        return Err(format!(
            "Graph JSON is too large ({} bytes; limit is {} bytes).",
            metadata.len(),
            MAX_GRAPH_IMPORT_BYTES
        ));
    }
    let bytes = fs::read(&file_path)
        .map_err(|error| format!("Cannot read selected graph file: {error}"))?;
    normalize_external_graph_json(&bytes)
}

fn tokens(value: &str) -> HashSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| part.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect()
}

pub fn resolve_graph_endpoint(
    graph: &RepoGraph,
    query: &str,
    selected_id: Option<&str>,
) -> GraphEndpointResolution {
    let query = query.trim();
    if let Some(selected_id) = selected_id {
        if let Some(node) = graph.nodes.iter().find(|node| node.id == selected_id) {
            let candidate = endpoint_candidate(node, 1_000);
            return GraphEndpointResolution {
                query: query.to_string(),
                status: GraphEndpointStatus::Resolved,
                selected: Some(candidate.clone()),
                candidates: vec![candidate],
            };
        }
    }
    if query.is_empty() {
        return GraphEndpointResolution {
            query: String::new(),
            status: GraphEndpointStatus::NotFound,
            selected: None,
            candidates: Vec::new(),
        };
    }
    let lower = query.to_ascii_lowercase();
    let query_tokens = tokens(query);
    let mut ranked = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let id = node.id.to_ascii_lowercase();
            let path = node.path.as_deref().unwrap_or("").to_ascii_lowercase();
            let label = node.label.to_ascii_lowercase();
            let score = if id == lower {
                400
            } else if !path.is_empty() && path == lower {
                300
            } else if label == lower {
                200
            } else {
                let node_tokens = tokens(&format!("{} {} {}", node.id, node.label, path));
                let common = query_tokens.intersection(&node_tokens).count() as u32;
                if common == 0 {
                    return None;
                }
                50 + common * 10
            };
            Some(endpoint_candidate(node, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    ranked.truncate(6);
    let Some(first) = ranked.first().cloned() else {
        return GraphEndpointResolution {
            query: query.to_string(),
            status: GraphEndpointStatus::NotFound,
            selected: None,
            candidates: Vec::new(),
        };
    };
    let ambiguous = ranked.get(1).is_some_and(|second| {
        second.score == first.score || (first.score < 200 && first.score - second.score <= 10)
    });
    GraphEndpointResolution {
        query: query.to_string(),
        status: if ambiguous {
            GraphEndpointStatus::Ambiguous
        } else {
            GraphEndpointStatus::Resolved
        },
        selected: if ambiguous { None } else { Some(first) },
        candidates: ranked,
    }
}

fn endpoint_candidate(node: &RepoGraphNode, score: u32) -> GraphEndpointCandidate {
    GraphEndpointCandidate {
        id: node.id.clone(),
        label: node.label.clone(),
        kind: node.kind.clone(),
        path: node.path.clone(),
        score,
    }
}

fn edge_weight(edge: &RepoGraphEdge) -> u32 {
    match edge.trust.as_str() {
        "extracted" => 1,
        "inferred" => 4,
        "legacy" => 6,
        _ => 8,
    }
}

pub fn trace_graph_path(
    graph: &RepoGraph,
    source_query: &str,
    target_query: &str,
    source_id: Option<&str>,
    target_id: Option<&str>,
    max_hops: usize,
    max_visited_nodes: usize,
) -> GraphPathResult {
    let max_hops = max_hops.clamp(1, 16);
    let max_visited_nodes = max_visited_nodes.clamp(1, 20_000);
    let source = resolve_graph_endpoint(graph, source_query, source_id);
    let target = resolve_graph_endpoint(graph, target_query, target_id);
    let empty = |message: String| GraphPathResult {
        source: source.clone(),
        target: target.clone(),
        hops: Vec::new(),
        found: false,
        trust_summary: "none".to_string(),
        requires_verification: false,
        message,
        bounds: GraphPathBounds {
            max_hops,
            max_visited_nodes,
            visited_nodes: 0,
            truncated: false,
        },
    };
    if source.status != GraphEndpointStatus::Resolved
        || target.status != GraphEndpointStatus::Resolved
    {
        return empty(
            "Select decisive source and target endpoints before tracing a path.".to_string(),
        );
    }
    let source_id = &source.selected.as_ref().expect("resolved source").id;
    let target_id = &target.selected.as_ref().expect("resolved target").id;
    let node_by_id = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    if source_id == target_id {
        let mut result = empty("Source and target resolve to the same graph node.".to_string());
        result.found = true;
        result.trust_summary = "same_node".to_string();
        return result;
    }

    let mut adjacency: HashMap<&str, Vec<(usize, &str, bool)>> = HashMap::new();
    for (index, edge) in graph.edges.iter().enumerate() {
        if node_by_id.contains_key(edge.from.as_str()) && node_by_id.contains_key(edge.to.as_str())
        {
            adjacency
                .entry(&edge.from)
                .or_default()
                .push((index, &edge.to, true));
            adjacency
                .entry(&edge.to)
                .or_default()
                .push((index, &edge.from, false));
        }
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(&b.0)));
    }

    let mut heap = BinaryHeap::new();
    heap.push(Reverse((0_u32, 0_usize, source_id.clone())));
    let mut best = HashMap::from([(source_id.clone(), (0_u32, 0_usize))]);
    let mut previous: HashMap<String, (String, usize, bool)> = HashMap::new();
    let mut visited = 0_usize;
    let mut truncated = false;
    let mut reached_target = false;
    while let Some(Reverse((cost, hops, current))) = heap.pop() {
        if best
            .get(&current)
            .is_some_and(|value| *value < (cost, hops))
        {
            continue;
        }
        visited += 1;
        if visited > max_visited_nodes {
            truncated = true;
            break;
        }
        if &current == target_id {
            reached_target = true;
            break;
        }
        if hops >= max_hops {
            truncated = true;
            continue;
        }
        for (edge_index, next, forward) in adjacency.get(current.as_str()).into_iter().flatten() {
            let next_cost = cost + edge_weight(&graph.edges[*edge_index]);
            let next_hops = hops + 1;
            let next_id = (*next).to_string();
            if best
                .get(&next_id)
                .is_none_or(|value| (next_cost, next_hops) < *value)
            {
                best.insert(next_id.clone(), (next_cost, next_hops));
                previous.insert(next_id.clone(), (current.clone(), *edge_index, *forward));
                heap.push(Reverse((next_cost, next_hops, next_id)));
            }
        }
    }

    if !reached_target {
        let mut result = empty(format!(
            "No bounded path was found within {max_hops} hops; this is not proof that the concepts are unrelated."
        ));
        result.bounds.visited_nodes = visited.min(max_visited_nodes);
        result.bounds.truncated = truncated;
        return result;
    }
    let mut chain = Vec::new();
    let mut cursor = target_id.clone();
    while &cursor != source_id {
        let Some((prior, edge_index, forward)) = previous.get(&cursor).cloned() else {
            break;
        };
        chain.push((prior.clone(), cursor.clone(), edge_index, forward));
        cursor = prior;
    }
    chain.reverse();
    let hops = chain
        .into_iter()
        .filter_map(|(from_id, to_id, edge_index, forward)| {
            let edge = graph.edges.get(edge_index)?;
            Some(GraphPathHop {
                from: (*node_by_id.get(from_id.as_str())?).clone(),
                to: (*node_by_id.get(to_id.as_str())?).clone(),
                kind: edge.kind.clone(),
                trust: edge.trust.clone(),
                origin: edge.origin.clone(),
                confidence_label: edge.confidence_label.clone(),
                evidence: edge.evidence.clone(),
                sources: edge.sources.clone(),
                follows_stored_direction: forward,
            })
        })
        .collect::<Vec<_>>();
    let requires_verification = hops
        .iter()
        .any(|hop| hop.trust != "extracted" || hop.origin != "codevetter");
    let trust_summary = if requires_verification {
        "navigation_lead".to_string()
    } else {
        "source_backed".to_string()
    };
    GraphPathResult {
        source,
        target,
        found: true,
        message: if requires_verification {
            "Path found with uncertain or imported hops; verify every lead against source before relying on it.".to_string()
        } else {
            "Source-backed connectivity path found; stored edge arrows show relationship direction, not execution order.".to_string()
        },
        hops,
        trust_summary,
        requires_verification,
        bounds: GraphPathBounds {
            max_hops,
            max_visited_nodes,
            visited_nodes: visited.min(max_visited_nodes),
            truncated,
        },
    }
}

#[tauri::command]
pub async fn trace_repo_graph_path(
    graph: RepoGraph,
    source_query: String,
    target_query: String,
    source_id: Option<String>,
    target_id: Option<String>,
    max_hops: Option<usize>,
    max_visited_nodes: Option<usize>,
) -> Result<GraphPathResult, String> {
    if graph.nodes.len() > MAX_GRAPH_IMPORT_NODES || graph.edges.len() > MAX_GRAPH_IMPORT_EDGES {
        return Err("Graph exceeds the supported path-query bounds.".to_string());
    }
    Ok(trace_graph_path(
        &graph,
        &source_query,
        &target_query,
        source_id.as_deref(),
        target_id.as_deref(),
        max_hops.unwrap_or(DEFAULT_PATH_HOPS),
        max_visited_nodes.unwrap_or(DEFAULT_PATH_VISITED),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(nodes: &[(&str, &str, Option<&str>)], edges: &[(&str, &str, &str)]) -> RepoGraph {
        RepoGraph {
            schema_version: 2,
            nodes: nodes
                .iter()
                .map(|(id, label, path)| RepoGraphNode {
                    id: (*id).to_string(),
                    kind: "file".to_string(),
                    label: (*label).to_string(),
                    path: path.map(|value| value.to_string()),
                    detail: None,
                    sources: Vec::new(),
                    source_location: None,
                    community: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to, trust)| RepoGraphEdge {
                    from: (*from).to_string(),
                    to: (*to).to_string(),
                    kind: "connects".to_string(),
                    evidence: "fixture".to_string(),
                    sources: vec!["src/lib.rs:1".to_string()],
                    trust: (*trust).to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn imports_node_link_json_and_preserves_metadata() {
        let input = br#"{"nodes":[{"id":"a","label":"A","type":"module","source_file":"src/a.ts","line":4,"community":2},{"id":"b","name":"B"}],"links":[{"source":"a","target":"b","relation":"calls","confidence":"high","source_file":"src/a.ts","line":8}]}"#;
        let parsed = normalize_external_graph_json(input).expect("valid external graph");
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.nodes[0].community.as_deref(), Some("2"));
        assert_eq!(
            parsed.nodes[0]
                .source_location
                .as_ref()
                .and_then(|value| value.line),
            Some(4)
        );
        assert_eq!(parsed.edges[0].kind, "calls");
        assert_eq!(parsed.edges[0].trust, "extracted");
        assert_eq!(parsed.edges[0].confidence_label.as_deref(), Some("high"));
        assert_eq!(parsed.edges[0].sources, vec!["src/a.ts#L8"]);
    }

    #[test]
    fn schema_v1_snapshot_loads_as_legacy_without_rewriting_input() {
        let input = r#"{"schema_version":1,"nodes":[{"id":"a","kind":"file","label":"A","path":"src/a.ts","detail":null,"sources":["src/a.ts"]},{"id":"b","kind":"route","label":"/","path":null,"detail":null,"sources":[]}],"edges":[{"from":"a","to":"b","kind":"routes_to","evidence":"old snapshot","sources":["src/a.ts"]}],"truncated":false}"#;
        let graph: RepoGraph = serde_json::from_str(input).expect("schema v1 remains readable");
        assert_eq!(graph.schema_version, 1);
        assert_eq!(graph.edges[0].trust, "legacy");
        assert_eq!(graph.edges[0].origin, "codevetter");
        assert!(!input.contains("legacy"));
    }

    #[test]
    fn imports_loose_edges_and_maps_unknown_confidence_to_ambiguous() {
        let input = br#"{"nodes":[{"id":"a"},{"id":"b"}],"edges":[{"from":"a","to":"b","kind":"owns","confidence":"mystery"}]}"#;
        let parsed = normalize_external_graph_json(input).expect("loose edges accepted");
        assert_eq!(parsed.edges[0].kind, "owns");
        assert_eq!(parsed.edges[0].trust, "ambiguous");
        assert_eq!(parsed.edges[0].origin, "imported");
    }

    #[test]
    fn rejects_malformed_dangling_and_caps() {
        assert!(normalize_external_graph_json(b"{")
            .unwrap_err()
            .contains("malformed"));
        let dangling = br#"{"nodes":[{"id":"a"}],"links":[{"source":"a","target":"missing"}]}"#;
        assert!(normalize_external_graph_json(dangling)
            .unwrap_err()
            .contains("missing endpoint"));
        assert!(
            normalize_external_graph_json(&vec![b' '; MAX_GRAPH_IMPORT_BYTES as usize + 1])
                .unwrap_err()
                .contains("too large")
        );
        let nodes = (0..=MAX_GRAPH_IMPORT_NODES)
            .map(|i| serde_json::json!({"id":i.to_string()}))
            .collect::<Vec<_>>();
        let too_many = serde_json::to_vec(&serde_json::json!({"nodes":nodes,"links":[]})).unwrap();
        assert!(normalize_external_graph_json(&too_many)
            .unwrap_err()
            .contains("too many nodes"));
        let edges = (0..=MAX_GRAPH_IMPORT_EDGES)
            .map(|_| serde_json::json!({"source":"a","target":"a"}))
            .collect::<Vec<_>>();
        let too_many =
            serde_json::to_vec(&serde_json::json!({"nodes":[{"id":"a"}],"links":edges})).unwrap();
        assert!(normalize_external_graph_json(&too_many)
            .unwrap_err()
            .contains("too many relationships"));
    }

    #[test]
    fn endpoint_precedence_and_ambiguity_are_deterministic() {
        let graph = graph(
            &[
                ("exact", "Shared", Some("src/a.ts")),
                ("other", "Shared", Some("src/b.ts")),
            ],
            &[],
        );
        assert_eq!(
            resolve_graph_endpoint(&graph, "exact", None)
                .selected
                .unwrap()
                .id,
            "exact"
        );
        assert_eq!(
            resolve_graph_endpoint(&graph, "src/b.ts", None)
                .selected
                .unwrap()
                .id,
            "other"
        );
        assert_eq!(
            resolve_graph_endpoint(&graph, "Shared", None).status,
            GraphEndpointStatus::Ambiguous
        );
    }

    #[test]
    fn path_prefers_extracted_route_and_preserves_reverse_direction() {
        let graph = graph(
            &[
                ("a", "A", None),
                ("b", "B", None),
                ("c", "C", None),
                ("d", "D", None),
            ],
            &[
                ("a", "d", "ambiguous"),
                ("a", "b", "extracted"),
                ("c", "b", "extracted"),
                ("c", "d", "extracted"),
            ],
        );
        let result = trace_graph_path(&graph, "a", "d", None, None, 5, 20);
        assert!(result.found);
        assert_eq!(result.hops.len(), 3);
        assert!(result.hops.iter().all(|hop| hop.trust == "extracted"));
        assert!(!result.hops[1].follows_stored_direction);
    }

    #[test]
    fn reports_no_path_and_traversal_cap_without_claiming_unrelatedness() {
        let graph = graph(
            &[("a", "A", None), ("b", "B", None), ("c", "C", None)],
            &[("a", "b", "extracted")],
        );
        let result = trace_graph_path(&graph, "a", "c", None, None, 3, 20);
        assert!(!result.found);
        assert!(result.message.contains("not proof"));
        let capped = trace_graph_path(&graph, "a", "b", None, None, 3, 1);
        assert!(!capped.found);
        assert!(capped.bounds.truncated);
    }

    #[test]
    fn tauri_command_boundary_imports_fixture_then_traces_native_path() {
        let fixture_path = std::env::temp_dir().join(format!(
            "codevetter-external-graph-runtime-{}.json",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(
            &fixture_path,
            br#"{"nodes":[{"id":"file","label":"src/page.tsx","type":"file","source_file":"src/page.tsx"},{"id":"route","label":"/billing","type":"route"}],"links":[{"source":"file","target":"route","relation":"routes_to","confidence":"high","source_file":"src/page.tsx","line":1}]}"#,
        )
        .expect("write local graph fixture");
        let imported = tauri::async_runtime::block_on(import_external_graph_preview(
            fixture_path.to_string_lossy().to_string(),
        ))
        .expect("Tauri import command accepts fixture");
        let traced = tauri::async_runtime::block_on(trace_repo_graph_path(
            imported,
            "src/page.tsx".to_string(),
            "/billing".to_string(),
            None,
            None,
            Some(6),
            Some(100),
        ))
        .expect("Tauri trace command returns result");
        assert!(traced.found);
        assert_eq!(traced.hops.len(), 1);
        assert_eq!(traced.hops[0].sources, vec!["src/page.tsx#L1"]);

        let native = graph(
            &[
                ("file", "src/page.tsx", Some("src/page.tsx")),
                ("route", "/billing", None),
            ],
            &[("file", "route", "extracted")],
        );
        let native_traced = tauri::async_runtime::block_on(trace_repo_graph_path(
            native,
            "src/page.tsx".to_string(),
            "/billing".to_string(),
            None,
            None,
            Some(6),
            Some(100),
        ))
        .expect("Tauri trace command handles native graph");
        assert!(native_traced.found);
        assert_eq!(native_traced.trust_summary, "source_backed");
    }
}
