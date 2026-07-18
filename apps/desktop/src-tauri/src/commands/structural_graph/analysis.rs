use super::types::{
    stable_graph_id, GraphTrust, StructuralGraphCommunity, StructuralGraphCoverage,
    StructuralGraphEdge, StructuralGraphNode,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

pub const STRUCTURAL_GRAPH_ANALYSIS_VERSION: &str = "2";
const MAX_RANKED_METRICS: usize = 500;
const MAX_COMPONENTS: usize = 500;
const MAX_EXECUTION_FLOWS: usize = 100;
const MAX_EXECUTION_FLOW_DEPTH: usize = 8;
const PAGERANK_ITERATIONS: usize = 40;
const PAGERANK_DAMPING: f64 = 0.85;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphAnalysisPolicy {
    pub algorithm_version: String,
    pub included_edge_kinds: Vec<String>,
    pub execution_edge_kinds: Vec<String>,
    pub included_trust: Vec<GraphTrust>,
    pub direction: String,
    pub max_ranked_metrics: usize,
    pub max_components: usize,
    pub max_execution_flows: usize,
    pub max_execution_flow_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StructuralGraphAnalysisCoverage {
    pub complete: bool,
    pub reachability_complete: bool,
    pub trusted_edge_count: usize,
    pub excluded_edge_count: usize,
    pub unresolved_endpoint_count: usize,
    pub gaps: Vec<String>,
    pub output_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphNodeMetric {
    pub node_id: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub total_degree: usize,
    pub degree_centrality: f64,
    pub pagerank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphComponent {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub cyclic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphExecutionFlow {
    pub entrypoint_node_id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub terminal_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StructuralGraphAlgorithmResults {
    pub node_metrics: Vec<StructuralGraphNodeMetric>,
    pub strongly_connected_components: Vec<StructuralGraphComponent>,
    pub cycles: Vec<StructuralGraphComponent>,
    pub articulation_node_ids: Vec<String>,
    pub entrypoint_node_ids: Vec<String>,
    pub reachable_node_ids: Vec<String>,
    pub unreachable_node_ids: Vec<String>,
    pub execution_flows: Vec<StructuralGraphExecutionFlow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphNodeRank {
    pub node_id: String,
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub degree: usize,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphConnectionInsight {
    pub edge_id: String,
    pub from_community_id: String,
    pub to_community_id: String,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphSuggestedQuestion {
    pub question: String,
    pub node_ids: Vec<String>,
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StructuralGraphAnalysisSummary {
    #[serde(default = "default_analysis_policy")]
    pub policy: StructuralGraphAnalysisPolicy,
    #[serde(default)]
    pub coverage: StructuralGraphAnalysisCoverage,
    #[serde(default)]
    pub algorithms: StructuralGraphAlgorithmResults,
    pub communities: Vec<StructuralGraphCommunity>,
    pub hubs: Vec<StructuralGraphNodeRank>,
    pub super_hubs: Vec<StructuralGraphNodeRank>,
    pub bridges: Vec<StructuralGraphNodeRank>,
    pub cross_community_edges: Vec<StructuralGraphConnectionInsight>,
    pub surprising_connections: Vec<StructuralGraphConnectionInsight>,
    pub suggested_questions: Vec<StructuralGraphSuggestedQuestion>,
}

fn default_analysis_policy() -> StructuralGraphAnalysisPolicy {
    StructuralGraphAnalysisPolicy {
        algorithm_version: STRUCTURAL_GRAPH_ANALYSIS_VERSION.to_string(),
        included_edge_kinds: Vec::new(),
        execution_edge_kinds: Vec::new(),
        included_trust: vec![GraphTrust::Extracted, GraphTrust::Inferred],
        direction: "from_to".to_string(),
        max_ranked_metrics: MAX_RANKED_METRICS,
        max_components: MAX_COMPONENTS,
        max_execution_flows: MAX_EXECUTION_FLOWS,
        max_execution_flow_depth: MAX_EXECUTION_FLOW_DEPTH,
    }
}

impl Default for StructuralGraphAnalysisPolicy {
    fn default() -> Self {
        default_analysis_policy()
    }
}

pub fn analyze_graph(
    nodes: &mut [StructuralGraphNode],
    edges: &[StructuralGraphEdge],
) -> Vec<StructuralGraphCommunity> {
    let community_key_by_node = assign_community_keys(
        nodes,
        edges.iter().filter(|edge| is_algorithm_trusted(edge.trust)),
    );
    let mut degree: HashMap<&str, usize> = HashMap::new();
    let mut bridge_nodes: HashMap<String, HashSet<String>> = HashMap::new();
    for edge in edges.iter().filter(|edge| is_algorithm_trusted(edge.trust)) {
        *degree.entry(edge.from.as_str()).or_default() += 1;
        *degree.entry(edge.to.as_str()).or_default() += 1;
        let Some(from_community) = community_key_by_node.get(&edge.from) else {
            continue;
        };
        let Some(to_community) = community_key_by_node.get(&edge.to) else {
            continue;
        };
        if from_community != to_community {
            bridge_nodes
                .entry(from_community.clone())
                .or_default()
                .insert(edge.from.clone());
            bridge_nodes
                .entry(to_community.clone())
                .or_default()
                .insert(edge.to.clone());
        }
    }

    let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in nodes.iter_mut() {
        let key = community_key_by_node
            .get(&node.id)
            .cloned()
            .unwrap_or_else(|| "root".to_string());
        let community_id = stable_graph_id("community", &key);
        node.community_id = Some(community_id);
        members.entry(key).or_default().push(node.id.clone());
    }

    members
        .into_iter()
        .map(|(key, mut member_ids)| {
            member_ids.sort();
            let mut ranked = member_ids.clone();
            ranked.sort_by(|left, right| {
                degree
                    .get(right.as_str())
                    .copied()
                    .unwrap_or(0)
                    .cmp(&degree.get(left.as_str()).copied().unwrap_or(0))
                    .then_with(|| left.cmp(right))
            });
            let hub_node_ids = ranked
                .into_iter()
                .filter(|node_id| degree.get(node_id.as_str()).copied().unwrap_or(0) > 0)
                .take(5)
                .collect::<Vec<_>>();
            let mut bridges = bridge_nodes
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            bridges.sort();
            let score = member_ids
                .iter()
                .map(|node_id| degree.get(node_id.as_str()).copied().unwrap_or(0))
                .sum::<usize>() as f64;
            StructuralGraphCommunity {
                id: stable_graph_id("community", &key),
                label: key,
                member_count: member_ids.len(),
                hub_node_ids,
                bridge_node_ids: bridges,
                score,
            }
        })
        .collect()
}

fn assign_community_keys<'a>(
    nodes: &[StructuralGraphNode],
    edges: impl IntoIterator<Item = &'a StructuralGraphEdge>,
) -> HashMap<String, String> {
    let node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let path_seed = nodes
        .iter()
        .map(|node| (node.id.clone(), community_key(node)))
        .collect::<HashMap<_, _>>();
    let mut labels = path_seed.clone();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        if !node_ids.contains(edge.from.as_str()) || !node_ids.contains(edge.to.as_str()) {
            continue;
        }
        adjacency.entry(&edge.from).or_default().push(&edge.to);
        adjacency.entry(&edge.to).or_default().push(&edge.from);
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    let mut ordered_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    ordered_ids.sort_unstable();
    for _ in 0..8 {
        let selections = ordered_ids
            .par_iter()
            .filter_map(|node_id| {
                let current = labels.get(*node_id)?;
                let mut scores = HashMap::<&str, usize>::new();
                *scores.entry(current.as_str()).or_default() += 2;
                if let Some(seed) = path_seed.get(*node_id) {
                    *scores.entry(seed.as_str()).or_default() += 1;
                }
                for neighbor in adjacency.get(*node_id).into_iter().flatten() {
                    if let Some(label) = labels.get(*neighbor) {
                        *scores.entry(label.as_str()).or_default() += 1;
                    }
                }
                let selected = scores
                    .into_iter()
                    .max_by(|(left_label, left_score), (right_label, right_score)| {
                        left_score
                            .cmp(right_score)
                            .then_with(|| right_label.cmp(left_label))
                    })
                    .map(|(label, _)| label)
                    .unwrap_or(current.as_str());
                (selected != current).then(|| ((*node_id).to_string(), selected.to_string()))
            })
            .collect::<Vec<_>>();
        if selections.is_empty() {
            break;
        }
        for (node_id, selected) in selections {
            labels.insert(node_id, selected);
        }
    }
    labels
}

pub fn summarize_graph_analysis(
    nodes: &[StructuralGraphNode],
    edges: &[StructuralGraphEdge],
    communities: &[StructuralGraphCommunity],
) -> StructuralGraphAnalysisSummary {
    summarize_graph_analysis_with_context(
        nodes,
        edges,
        communities,
        &StructuralGraphCoverage::default(),
        false,
    )
}

pub fn summarize_graph_analysis_with_context(
    nodes: &[StructuralGraphNode],
    edges: &[StructuralGraphEdge],
    communities: &[StructuralGraphCommunity],
    snapshot_coverage: &StructuralGraphCoverage,
    snapshot_truncated: bool,
) -> StructuralGraphAnalysisSummary {
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let community_by_node = nodes
        .iter()
        .filter_map(|node| {
            node.community_id
                .as_deref()
                .map(|community| (node.id.as_str(), community))
        })
        .collect::<HashMap<_, _>>();
    let trusted_summary_edges = edges
        .iter()
        .filter(|edge| {
            is_algorithm_trusted(edge.trust)
                && node_by_id.contains_key(edge.from.as_str())
                && node_by_id.contains_key(edge.to.as_str())
        })
        .collect::<Vec<_>>();
    let mut degree = HashMap::<&str, usize>::new();
    for edge in &trusted_summary_edges {
        *degree.entry(edge.from.as_str()).or_default() += 1;
        *degree.entry(edge.to.as_str()).or_default() += 1;
    }
    let super_hub_threshold = ((trusted_summary_edges.len() as f64).sqrt().ceil() as usize).max(12);
    let mut ranked = nodes
        .iter()
        .filter_map(|node| {
            let node_degree = degree.get(node.id.as_str()).copied().unwrap_or_default();
            (node_degree > 0).then(|| StructuralGraphNodeRank {
                node_id: node.id.clone(),
                label: node.label.clone(),
                kind: node.kind.clone(),
                path: node.path.clone(),
                degree: node_degree,
                score: node_degree as f64,
                reason: "deterministic total degree".to_string(),
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .degree
            .cmp(&left.degree)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    let super_hubs = ranked
        .iter()
        .filter(|rank| rank.degree >= super_hub_threshold)
        .take(20)
        .cloned()
        .collect::<Vec<_>>();
    let super_hub_ids = super_hubs
        .iter()
        .map(|rank| rank.node_id.as_str())
        .collect::<HashSet<_>>();
    let hubs = ranked
        .iter()
        .filter(|rank| !super_hub_ids.contains(rank.node_id.as_str()))
        .take(20)
        .cloned()
        .collect::<Vec<_>>();

    let bridge_ids = communities
        .iter()
        .flat_map(|community| community.bridge_node_ids.iter())
        .collect::<HashSet<_>>();
    let mut bridges = ranked
        .iter()
        .filter(|rank| bridge_ids.contains(&rank.node_id))
        .cloned()
        .collect::<Vec<_>>();
    for bridge in &mut bridges {
        bridge.reason = "connects nodes assigned to different navigation communities".to_string();
    }
    bridges.truncate(30);

    let mut cross_community_edges = trusted_summary_edges
        .iter()
        .filter_map(|edge| {
            let from_community = community_by_node.get(edge.from.as_str())?;
            let to_community = community_by_node.get(edge.to.as_str())?;
            if from_community == to_community {
                return None;
            }
            let endpoint_degree = degree.get(edge.from.as_str()).copied().unwrap_or_default()
                + degree.get(edge.to.as_str()).copied().unwrap_or_default();
            Some(StructuralGraphConnectionInsight {
                edge_id: edge.id.clone(),
                from_community_id: (*from_community).to_string(),
                to_community_id: (*to_community).to_string(),
                score: 1.0 / (endpoint_degree.max(1) as f64),
                reason: format!("{} crosses navigation communities", edge.kind),
            })
        })
        .collect::<Vec<_>>();
    cross_community_edges.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    let mut surprising_connections = cross_community_edges.clone();
    surprising_connections.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.edge_id.cmp(&right.edge_id))
    });
    surprising_connections.truncate(20);
    cross_community_edges.truncate(100);

    let mut suggested_questions = Vec::new();
    for bridge in bridges.iter().take(5) {
        if let Some(node) = node_by_id.get(bridge.node_id.as_str()) {
            suggested_questions.push(StructuralGraphSuggestedQuestion {
                question: format!(
                    "Why does {} connect multiple repository communities?",
                    node.label
                ),
                node_ids: vec![node.id.clone()],
                source_paths: node
                    .sources
                    .iter()
                    .map(|source| source.path.clone())
                    .collect(),
            });
        }
    }
    for hub in hubs
        .iter()
        .take(5_usize.saturating_sub(suggested_questions.len()))
    {
        if let Some(node) = node_by_id.get(hub.node_id.as_str()) {
            suggested_questions.push(StructuralGraphSuggestedQuestion {
                question: format!("What depends on {}, and how is it verified?", node.label),
                node_ids: vec![node.id.clone()],
                source_paths: node
                    .sources
                    .iter()
                    .map(|source| source.path.clone())
                    .collect(),
            });
        }
    }

    let (policy, coverage, algorithms) =
        analyze_trusted_algorithms(nodes, edges, snapshot_coverage, snapshot_truncated);

    StructuralGraphAnalysisSummary {
        policy,
        coverage,
        algorithms,
        communities: communities.to_vec(),
        hubs,
        super_hubs,
        bridges,
        cross_community_edges,
        surprising_connections,
        suggested_questions,
    }
}

fn analyze_trusted_algorithms(
    nodes: &[StructuralGraphNode],
    edges: &[StructuralGraphEdge],
    snapshot_coverage: &StructuralGraphCoverage,
    snapshot_truncated: bool,
) -> (
    StructuralGraphAnalysisPolicy,
    StructuralGraphAnalysisCoverage,
    StructuralGraphAlgorithmResults,
) {
    let mut ordered_nodes = nodes.iter().collect::<Vec<_>>();
    ordered_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let node_index = ordered_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut included_kinds = BTreeSet::new();
    let mut execution_kinds = BTreeSet::new();
    let mut trusted_edges = Vec::new();
    let mut excluded_edge_count = 0;
    let mut unresolved_endpoint_count = 0;
    for edge in edges {
        if !is_algorithm_trusted(edge.trust) {
            excluded_edge_count += 1;
            continue;
        }
        let (Some(&from), Some(&to)) = (
            node_index.get(edge.from.as_str()),
            node_index.get(edge.to.as_str()),
        ) else {
            unresolved_endpoint_count += 1;
            continue;
        };
        included_kinds.insert(edge.kind.clone());
        if is_execution_edge_kind(&edge.kind) {
            execution_kinds.insert(edge.kind.clone());
        }
        trusted_edges.push((from, to, edge));
    }
    trusted_edges.sort_by(|left, right| left.2.id.cmp(&right.2.id));

    let node_count = ordered_nodes.len();
    let mut outgoing = vec![Vec::<usize>::new(); node_count];
    let mut incoming = vec![Vec::<usize>::new(); node_count];
    let mut undirected = vec![Vec::<usize>::new(); node_count];
    let mut execution = vec![Vec::<(usize, &str)>::new(); node_count];
    for (from, to, edge) in &trusted_edges {
        outgoing[*from].push(*to);
        incoming[*to].push(*from);
        undirected[*from].push(*to);
        undirected[*to].push(*from);
        if is_execution_edge_kind(&edge.kind) {
            execution[*from].push((*to, edge.id.as_str()));
        }
    }
    for neighbors in outgoing
        .iter_mut()
        .chain(incoming.iter_mut())
        .chain(undirected.iter_mut())
    {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    for neighbors in &mut execution {
        neighbors.sort_by(|left, right| {
            ordered_nodes[left.0]
                .id
                .cmp(&ordered_nodes[right.0].id)
                .then_with(|| left.1.cmp(right.1))
        });
    }

    let pagerank = calculate_pagerank(&outgoing, &incoming);
    let degree_denominator = node_count.saturating_sub(1).saturating_mul(2).max(1) as f64;
    let mut node_metrics = ordered_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let in_degree = incoming[index].len();
            let out_degree = outgoing[index].len();
            StructuralGraphNodeMetric {
                node_id: node.id.clone(),
                in_degree,
                out_degree,
                total_degree: in_degree + out_degree,
                degree_centrality: (in_degree + out_degree) as f64 / degree_denominator,
                pagerank: pagerank.get(index).copied().unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    node_metrics.sort_by(|left, right| {
        right
            .pagerank
            .total_cmp(&left.pagerank)
            .then_with(|| right.total_degree.cmp(&left.total_degree))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });

    let component_indices = strongly_connected_components(&outgoing, &incoming);
    let mut component_by_node = vec![0_usize; node_count];
    for (component_index, component) in component_indices.iter().enumerate() {
        for &node_index in component {
            component_by_node[node_index] = component_index;
        }
    }
    let mut component_edge_ids = vec![Vec::<String>::new(); component_indices.len()];
    for (from, to, edge) in &trusted_edges {
        let component_index = component_by_node[*from];
        if component_index == component_by_node[*to] {
            component_edge_ids[component_index].push(edge.id.clone());
        }
    }
    for edge_ids in &mut component_edge_ids {
        edge_ids.sort();
    }
    let mut components = component_indices
        .iter()
        .zip(component_edge_ids)
        .map(|(component, edge_ids)| build_component(component, &ordered_nodes, edge_ids))
        .collect::<Vec<_>>();
    components.sort_by(|left, right| left.node_ids[0].cmp(&right.node_ids[0]));
    let mut cycles = components
        .iter()
        .filter(|component| component.cyclic)
        .cloned()
        .collect::<Vec<_>>();

    let articulation_node_ids = articulation_points(&undirected)
        .into_iter()
        .map(|index| ordered_nodes[index].id.clone())
        .collect::<Vec<_>>();
    let entrypoint_indices = ordered_nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| is_entrypoint(node).then_some(index))
        .collect::<Vec<_>>();
    let mut reachable = vec![false; node_count];
    let mut pending = entrypoint_indices.clone();
    while let Some(index) = pending.pop() {
        if reachable[index] {
            continue;
        }
        reachable[index] = true;
        pending.extend(execution[index].iter().map(|(target, _)| *target));
    }
    let mut reachable_node_ids = Vec::new();
    let mut unreachable_node_ids = Vec::new();
    for (index, node) in ordered_nodes.iter().enumerate() {
        if reachable[index] {
            reachable_node_ids.push(node.id.clone());
        } else {
            unreachable_node_ids.push(node.id.clone());
        }
    }
    let (execution_flows, flows_truncated) =
        bounded_execution_flows(&ordered_nodes, &execution, &entrypoint_indices);

    let mut gaps = Vec::new();
    if snapshot_truncated {
        gaps.push("snapshot_truncated".to_string());
    }
    if snapshot_coverage.discovered_files > snapshot_coverage.indexed_files {
        gaps.push(format!(
            "files_not_indexed:{}",
            snapshot_coverage.discovered_files - snapshot_coverage.indexed_files
        ));
    }
    if snapshot_coverage.skipped_files > 0 {
        gaps.push(format!("skipped_files:{}", snapshot_coverage.skipped_files));
    }
    if snapshot_coverage.error_files > 0 {
        gaps.push(format!("error_files:{}", snapshot_coverage.error_files));
    }
    let unsupported_files = snapshot_coverage
        .languages
        .iter()
        .filter(|language| !language.supported)
        .map(|language| language.discovered_files)
        .sum::<usize>();
    if unsupported_files > 0 {
        gaps.push(format!("unsupported_language_files:{unsupported_files}"));
    }
    if unresolved_endpoint_count > 0 {
        gaps.push(format!("unresolved_endpoints:{unresolved_endpoint_count}"));
    }
    if excluded_edge_count > 0 {
        gaps.push(format!("untrusted_edges_excluded:{excluded_edge_count}"));
    }
    let dynamic_reference_count = nodes
        .iter()
        .filter(|node| node.kind == "dynamic_reference")
        .count();
    if dynamic_reference_count > 0 {
        gaps.push(format!("dynamic_references:{dynamic_reference_count}"));
    }
    if entrypoint_indices.is_empty() {
        gaps.push("no_qualified_entrypoints".to_string());
    }
    let output_truncated = node_metrics.len() > MAX_RANKED_METRICS
        || components.len() > MAX_COMPONENTS
        || cycles.len() > MAX_COMPONENTS
        || reachable_node_ids.len() > MAX_RANKED_METRICS
        || unreachable_node_ids.len() > MAX_RANKED_METRICS
        || flows_truncated;
    if output_truncated {
        gaps.push("analysis_output_limited".to_string());
    }
    node_metrics.truncate(MAX_RANKED_METRICS);
    components.truncate(MAX_COMPONENTS);
    cycles.truncate(MAX_COMPONENTS);
    reachable_node_ids.truncate(MAX_RANKED_METRICS);
    unreachable_node_ids.truncate(MAX_RANKED_METRICS);
    gaps.sort();
    gaps.dedup();
    let reachability_complete = gaps.is_empty();
    let coverage = StructuralGraphAnalysisCoverage {
        complete: gaps.is_empty(),
        reachability_complete,
        trusted_edge_count: trusted_edges.len(),
        excluded_edge_count,
        unresolved_endpoint_count,
        gaps,
        output_truncated,
    };
    let policy = StructuralGraphAnalysisPolicy {
        included_edge_kinds: included_kinds.into_iter().collect(),
        execution_edge_kinds: execution_kinds.into_iter().collect(),
        ..default_analysis_policy()
    };
    let algorithms = StructuralGraphAlgorithmResults {
        node_metrics,
        strongly_connected_components: components,
        cycles,
        articulation_node_ids,
        entrypoint_node_ids: entrypoint_indices
            .iter()
            .map(|index| ordered_nodes[*index].id.clone())
            .collect(),
        reachable_node_ids,
        unreachable_node_ids,
        execution_flows,
    };
    (policy, coverage, algorithms)
}

fn is_algorithm_trusted(trust: GraphTrust) -> bool {
    matches!(trust, GraphTrust::Extracted | GraphTrust::Inferred)
}

pub(crate) fn is_execution_edge_kind(kind: &str) -> bool {
    matches!(
        kind,
        "calls"
            | "invokes"
            | "invokes_command"
            | "routes_to"
            | "handles"
            | "dispatches"
            | "emits"
            | "subscribes"
            | "schedules"
            | "executes"
            | "queries"
            | "reads"
            | "reads_from"
            | "writes"
            | "writes_to"
            | "implemented_by"
            | "binds_to"
            | "depends_on"
            | "test_covers"
    )
}

pub(crate) fn is_entrypoint(node: &StructuralGraphNode) -> bool {
    if matches!(
        node.kind.as_str(),
        "entrypoint"
            | "route"
            | "command"
            | "tauri_command"
            | "job"
            | "event"
            | "event_subscription"
            | "test"
            | "resolver"
            | "openapi_operation"
            | "graphql_operation"
            | "protobuf_rpc"
    ) {
        return true;
    }
    if matches!(node.label.as_str(), "main" | "__main__") {
        return true;
    }
    let Some(path) = node.path.as_deref() else {
        return false;
    };
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    node.kind == "file"
        && (matches!(
            file_name,
            "main.rs" | "main.go" | "main.py" | "__main__.py" | "program.cs"
        ) || normalized.contains("/bin/"))
}

fn calculate_pagerank(outgoing: &[Vec<usize>], incoming: &[Vec<usize>]) -> Vec<f64> {
    let count = outgoing.len();
    if count == 0 {
        return Vec::new();
    }
    let base = (1.0 - PAGERANK_DAMPING) / count as f64;
    let mut ranks = vec![1.0 / count as f64; count];
    for _ in 0..PAGERANK_ITERATIONS {
        let dangling = outgoing
            .iter()
            .enumerate()
            .filter(|(_, targets)| targets.is_empty())
            .map(|(index, _)| ranks[index])
            .sum::<f64>()
            / count as f64;
        let mut next = vec![base + PAGERANK_DAMPING * dangling; count];
        for (target, sources) in incoming.iter().enumerate() {
            next[target] += PAGERANK_DAMPING
                * sources
                    .iter()
                    .map(|source| ranks[*source] / outgoing[*source].len() as f64)
                    .sum::<f64>();
        }
        let delta = ranks
            .iter()
            .zip(&next)
            .map(|(before, after)| (before - after).abs())
            .sum::<f64>();
        ranks = next;
        if delta < 1e-10 {
            break;
        }
    }
    ranks
}

fn strongly_connected_components(
    outgoing: &[Vec<usize>],
    incoming: &[Vec<usize>],
) -> Vec<Vec<usize>> {
    fn finish_order(start: usize, graph: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
        seen[start] = true;
        let mut stack = vec![(start, 0_usize)];
        while let Some((node, next_neighbor)) = stack.last_mut() {
            if *next_neighbor < graph[*node].len() {
                let target = graph[*node][*next_neighbor];
                *next_neighbor += 1;
                if !seen[target] {
                    seen[target] = true;
                    stack.push((target, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }

    let mut seen = vec![false; outgoing.len()];
    let mut order = Vec::with_capacity(outgoing.len());
    for start in 0..outgoing.len() {
        if !seen[start] {
            finish_order(start, outgoing, &mut seen, &mut order);
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for &start in order.iter().rev() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut component = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            component.push(node);
            for &target in incoming[node].iter().rev() {
                if !seen[target] {
                    seen[target] = true;
                    stack.push(target);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

fn build_component(
    component: &[usize],
    nodes: &[&StructuralGraphNode],
    edge_ids: Vec<String>,
) -> StructuralGraphComponent {
    let node_ids = component
        .iter()
        .map(|index| nodes[*index].id.clone())
        .collect::<Vec<_>>();
    let cyclic = node_ids.len() > 1 || !edge_ids.is_empty();
    StructuralGraphComponent {
        id: stable_graph_id("scc", &node_ids.join("\u{1f}")),
        node_ids,
        edge_ids,
        cyclic,
    }
}

fn articulation_points(graph: &[Vec<usize>]) -> Vec<usize> {
    let count = graph.len();
    let mut discovered = vec![0_usize; count];
    let mut low = vec![0_usize; count];
    let mut parent = vec![None; count];
    let mut child_count = vec![0_usize; count];
    let mut articulation = vec![false; count];
    let mut time = 0_usize;
    for root in 0..count {
        if discovered[root] != 0 {
            continue;
        }
        time += 1;
        discovered[root] = time;
        low[root] = time;
        let mut stack = vec![(root, 0_usize)];
        while let Some((node, next_neighbor)) = stack.last_mut() {
            if *next_neighbor < graph[*node].len() {
                let target = graph[*node][*next_neighbor];
                *next_neighbor += 1;
                if discovered[target] == 0 {
                    parent[target] = Some(*node);
                    child_count[*node] += 1;
                    time += 1;
                    discovered[target] = time;
                    low[target] = time;
                    stack.push((target, 0));
                } else if parent[*node] != Some(target) {
                    low[*node] = low[*node].min(discovered[target]);
                }
            } else {
                let (finished, _) = stack.pop().expect("DFS frame exists");
                if let Some(parent_index) = parent[finished] {
                    low[parent_index] = low[parent_index].min(low[finished]);
                    if parent[parent_index].is_some() && low[finished] >= discovered[parent_index] {
                        articulation[parent_index] = true;
                    }
                } else if child_count[finished] > 1 {
                    articulation[finished] = true;
                }
            }
        }
    }
    articulation
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| value.then_some(index))
        .collect()
}

fn bounded_execution_flows(
    nodes: &[&StructuralGraphNode],
    execution: &[Vec<(usize, &str)>],
    entrypoints: &[usize],
) -> (Vec<StructuralGraphExecutionFlow>, bool) {
    let mut flows = Vec::new();
    let mut truncated = false;
    for &entrypoint in entrypoints {
        let mut pending = vec![(vec![entrypoint], Vec::<String>::new())];
        while let Some((node_path, edge_path)) = pending.pop() {
            if flows.len() >= MAX_EXECUTION_FLOWS {
                truncated = true;
                break;
            }
            let current = *node_path.last().expect("execution path has a node");
            let at_depth_limit = edge_path.len() >= MAX_EXECUTION_FLOW_DEPTH;
            let mut next = execution[current]
                .iter()
                .filter(|(target, _)| !node_path.contains(target))
                .collect::<Vec<_>>();
            next.reverse();
            if at_depth_limit || next.is_empty() {
                let terminal_reason = if at_depth_limit {
                    "depth_limit"
                } else if execution[current].is_empty() {
                    "terminal"
                } else {
                    "cycle_avoided"
                };
                flows.push(StructuralGraphExecutionFlow {
                    entrypoint_node_id: nodes[entrypoint].id.clone(),
                    node_ids: node_path
                        .iter()
                        .map(|index| nodes[*index].id.clone())
                        .collect(),
                    edge_ids: edge_path,
                    terminal_reason: terminal_reason.to_string(),
                });
                continue;
            }
            for (target, edge_id) in next {
                let mut next_nodes = node_path.clone();
                next_nodes.push(*target);
                let mut next_edges = edge_path.clone();
                next_edges.push((*edge_id).to_string());
                pending.push((next_nodes, next_edges));
            }
        }
        if truncated {
            break;
        }
    }
    (flows, truncated)
}

fn community_key(node: &StructuralGraphNode) -> String {
    let Some(path) = node.path.as_deref() else {
        return node.kind.clone();
    };
    let components = Path::new(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter(|component| !component.is_empty() && *component != ".")
        .take(2)
        .collect::<Vec<_>>();
    match components.as_slice() {
        [] => "root".to_string(),
        [first] => (*first).to_string(),
        [first, second] if matches!(*first, "src" | "lib" | "app" | "pages" | "tests") => {
            (*first).to_string()
        }
        [first, second] => format!("{first}/{second}"),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::structural_graph::types::{GraphOrigin, GraphTrust};

    #[test]
    fn communities_hubs_and_bridges_are_deterministic() {
        let mut nodes = [
            node("a", "apps/api/a.rs"),
            node("b", "apps/api/b.rs"),
            node("c", "apps/web/c.ts"),
        ];
        let edges = [edge("a", "b"), edge("b", "c")];
        let first = analyze_graph(&mut nodes, &edges);
        let second = analyze_graph(&mut nodes, &edges);
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert!(first
            .iter()
            .any(|community| !community.bridge_node_ids.is_empty()));
        let first_summary = summarize_graph_analysis(&nodes, &edges, &first);
        let second_summary = summarize_graph_analysis(&nodes, &edges, &second);
        assert_eq!(first_summary, second_summary);
        assert!(!first_summary.cross_community_edges.is_empty());
        assert!(!first_summary.surprising_connections.is_empty());
        assert!(!first_summary.suggested_questions.is_empty());
    }

    #[test]
    fn trusted_algorithms_compute_cycles_centrality_articulation_and_reachability() {
        let mut nodes = [
            node_with_kind("entry", "apps/api/route.ts", "route"),
            node("a", "apps/api/a.ts"),
            node("b", "apps/api/b.ts"),
            node("c", "apps/api/c.ts"),
            node("isolated", "apps/api/unused.ts"),
        ];
        let edges = [
            edge("entry", "a"),
            edge("a", "b"),
            edge("b", "c"),
            edge("c", "a"),
        ];
        let communities = analyze_graph(&mut nodes, &edges);
        let summary = summarize_graph_analysis_with_context(
            &nodes,
            &edges,
            &communities,
            &complete_coverage(5),
            false,
        );

        assert_eq!(summary.policy.algorithm_version, "2");
        assert_eq!(
            summary.policy.included_trust,
            [GraphTrust::Extracted, GraphTrust::Inferred]
        );
        assert_eq!(summary.policy.included_edge_kinds, ["calls"]);
        assert_eq!(summary.algorithms.entrypoint_node_ids, ["entry"]);
        assert_eq!(
            summary.algorithms.reachable_node_ids,
            ["a", "b", "c", "entry"]
        );
        assert_eq!(summary.algorithms.unreachable_node_ids, ["isolated"]);
        assert!(summary.coverage.reachability_complete);
        assert!(summary
            .algorithms
            .cycles
            .iter()
            .any(|cycle| cycle.node_ids == ["a", "b", "c"]));
        assert!(summary
            .algorithms
            .articulation_node_ids
            .contains(&"a".to_string()));
        let ranked_ids = summary
            .algorithms
            .node_metrics
            .iter()
            .map(|metric| metric.node_id.as_str())
            .collect::<Vec<_>>();
        assert!(
            ranked_ids.iter().position(|id| *id == "a")
                < ranked_ids.iter().position(|id| *id == "isolated")
        );
        assert!(summary
            .algorithms
            .execution_flows
            .iter()
            .all(|flow| flow.node_ids.len() <= MAX_EXECUTION_FLOW_DEPTH + 1));
    }

    #[test]
    fn ambiguous_edges_and_partial_snapshots_prevent_global_claims() {
        let mut nodes = [
            node_with_kind("entry", "src/main.rs", "entrypoint"),
            node("trusted", "src/trusted.rs"),
            node("candidate", "src/candidate.rs"),
        ];
        let mut ambiguous = edge("trusted", "candidate");
        ambiguous.trust = GraphTrust::Ambiguous;
        let edges = [edge("entry", "trusted"), ambiguous];
        let communities = analyze_graph(&mut nodes, &edges);
        let coverage = StructuralGraphCoverage {
            discovered_files: 4,
            indexed_files: 3,
            skipped_files: 1,
            error_files: 0,
            generated_files: 0,
            sensitive_files: 0,
            binary_files: 0,
            languages: Vec::new(),
        };
        let summary =
            summarize_graph_analysis_with_context(&nodes, &edges, &communities, &coverage, true);

        assert!(!summary.coverage.complete);
        assert!(!summary.coverage.reachability_complete);
        assert_eq!(summary.coverage.trusted_edge_count, 1);
        assert_eq!(summary.coverage.excluded_edge_count, 1);
        assert!(summary
            .coverage
            .gaps
            .contains(&"snapshot_truncated".to_string()));
        assert!(summary
            .coverage
            .gaps
            .contains(&"untrusted_edges_excluded:1".to_string()));
        assert!(summary
            .algorithms
            .unreachable_node_ids
            .contains(&"candidate".to_string()));
        assert!(!summary
            .algorithms
            .strongly_connected_components
            .iter()
            .flat_map(|component| &component.edge_ids)
            .any(|edge_id| edge_id == "trusted:candidate"));
    }

    #[test]
    fn bounded_execution_flows_are_deterministic_and_report_limits() {
        let mut nodes = vec![node_with_kind("entry", "src/main.rs", "entrypoint")];
        let mut edges = Vec::new();
        for index in 0..(MAX_EXECUTION_FLOWS + 10) {
            let id = format!("leaf-{index:03}");
            nodes.push(node(&id, &format!("src/{id}.rs")));
            edges.push(edge("entry", &id));
        }
        let communities = analyze_graph(&mut nodes, &edges);
        let coverage = complete_coverage(nodes.len());
        let first =
            summarize_graph_analysis_with_context(&nodes, &edges, &communities, &coverage, false);
        let second =
            summarize_graph_analysis_with_context(&nodes, &edges, &communities, &coverage, false);

        assert_eq!(first, second);
        assert_eq!(first.algorithms.execution_flows.len(), MAX_EXECUTION_FLOWS);
        assert!(first.coverage.output_truncated);
        assert!(first
            .coverage
            .gaps
            .contains(&"analysis_output_limited".to_string()));
    }

    fn node(id: &str, path: &str) -> StructuralGraphNode {
        node_with_kind(id, path, "function")
    }

    fn node_with_kind(id: &str, path: &str, kind: &str) -> StructuralGraphNode {
        StructuralGraphNode {
            id: id.to_string(),
            kind: kind.to_string(),
            label: id.to_string(),
            qualified_name: None,
            path: Some(path.to_string()),
            detail: None,
            language: None,
            community_id: None,
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Syntax,
            sources: Vec::new(),
        }
    }

    fn complete_coverage(files: usize) -> StructuralGraphCoverage {
        StructuralGraphCoverage {
            discovered_files: files,
            indexed_files: files,
            ..StructuralGraphCoverage::default()
        }
    }

    fn edge(from: &str, to: &str) -> StructuralGraphEdge {
        StructuralGraphEdge {
            id: format!("{from}:{to}"),
            from: from.to_string(),
            to: to.to_string(),
            kind: "calls".to_string(),
            evidence: "fixture".to_string(),
            trust: GraphTrust::Inferred,
            origin: GraphOrigin::Resolution,
            sources: Vec::new(),
            candidates: Vec::new(),
        }
    }
}
