use super::analysis::{summarize_graph_analysis_with_context, StructuralGraphAnalysisSummary};
use super::types::{
    GraphTrust, StructuralGraphCoverage, StructuralGraphEdge, StructuralGraphNode,
    StructuralGraphSnapshot,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const MAX_EDGE_LIMIT: usize = 2_000;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_PATH_HOPS: usize = 32;
const MAX_PATH_VISITS: usize = 25_000;
const MAX_DIFF_IDS: usize = 500;
const MAX_QUERY_INDEXES: usize = 16;

#[derive(Debug, Default)]
struct StructuralGraphQueryIndex {
    exact: HashMap<String, Vec<usize>>,
    tokens: HashMap<String, Vec<usize>>,
}

static QUERY_INDEXES: OnceLock<Mutex<HashMap<String, Arc<StructuralGraphQueryIndex>>>> =
    OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphDirection {
    Incoming,
    Outgoing,
    #[default]
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphQueryFilter {
    #[serde(default)]
    pub node_kinds: Vec<String>,
    #[serde(default)]
    pub edge_kinds: Vec<String>,
    #[serde(default)]
    pub trust: Vec<GraphTrust>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSearchHit {
    pub node: StructuralGraphNode,
    pub score: u32,
    pub matched_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSearchResult {
    pub hits: Vec<GraphSearchHit>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphExplanation {
    pub node: StructuralGraphNode,
    pub incoming_count: usize,
    pub outgoing_count: usize,
    pub incoming_kinds: Vec<String>,
    pub outgoing_kinds: Vec<String>,
    pub truncated: bool,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphProjection {
    pub nodes: Vec<StructuralGraphNode>,
    pub edges: Vec<StructuralGraphEdge>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPathResult {
    pub nodes: Vec<StructuralGraphNode>,
    pub edges: Vec<StructuralGraphEdge>,
    pub total_cost: f64,
    pub visited: usize,
    pub truncated: bool,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphImpactResult {
    pub root: StructuralGraphNode,
    pub affected: Vec<StructuralGraphNode>,
    pub edges: Vec<StructuralGraphEdge>,
    pub depth_reached: usize,
    pub truncated: bool,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshotDiff {
    pub before_snapshot_id: String,
    pub after_snapshot_id: String,
    pub added_node_ids: Vec<String>,
    pub removed_node_ids: Vec<String>,
    pub changed_node_ids: Vec<String>,
    pub added_edge_ids: Vec<String>,
    pub removed_edge_ids: Vec<String>,
    pub changed_edge_ids: Vec<String>,
    pub truncated: bool,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphAnalysisResult {
    #[serde(flatten)]
    pub analysis: StructuralGraphAnalysisSummary,
    pub truncated: bool,
    pub context: GraphQueryContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GraphTrustSummary {
    pub extracted: usize,
    pub inferred: usize,
    pub ambiguous: usize,
    pub legacy: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphFreshness {
    pub indexed_head: Option<String>,
    pub current_head: Option<String>,
    /// `None` means the caller did not provide a live repository HEAD.
    pub stale: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphQueryContext {
    pub snapshot_id: String,
    pub schema_version: i64,
    pub engine_id: String,
    pub engine_version: String,
    pub created_at: String,
    pub freshness: GraphFreshness,
    pub coverage: StructuralGraphCoverage,
    pub trust: GraphTrustSummary,
    pub max_results: usize,
    pub max_edges: usize,
    pub max_hops: usize,
    pub max_bytes: usize,
}

impl GraphQueryContext {
    pub fn observe_current_head(&mut self, current_head: Option<String>) {
        self.freshness.stale = current_head
            .as_ref()
            .map(|head| self.freshness.indexed_head.as_ref() != Some(head));
        self.freshness.current_head = current_head;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralGraphMetadata {
    pub snapshot_id: String,
    pub schema_version: i64,
    pub repo_path: String,
    pub repo_head: Option<String>,
    pub created_at: String,
    pub engine_id: String,
    pub engine_version: String,
    pub indexed_files: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub diagnostic_count: usize,
    pub coverage: StructuralGraphCoverage,
    pub trust: Option<GraphTrustSummary>,
    pub freshness: GraphFreshness,
    pub truncated: bool,
}

mod index;
mod limits;
mod path_visit;
mod projection;
mod search;
mod traversal;

use index::{
    edge_matches_filter, lexical_tokens, node_map, node_matches_filter, normalize, query_index,
    rank_node, rank_question_tokens, trust_cost,
};
use limits::{
    bounded_limit, enforce_impact_bytes, enforce_path_bytes, enforce_projection_bytes,
    enforce_search_bytes, parse_cursor,
};
use path_visit::PathVisit;
use projection::query_context;

pub use projection::{
    analysis, analysis_summary, community, community_page, diff_snapshots, metadata, overview,
    overview_page, subgraph,
};
pub use search::{explain, neighbors, resolve_node, search, search_page};
pub use traversal::{impact, shortest_path};

#[cfg(test)]
mod tests;
