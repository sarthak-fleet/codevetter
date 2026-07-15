use crate::commands::git_metadata::{is_release_tag, read_git_tags};
use crate::commands::structural_graph::analysis::StructuralGraphAnalysisSummary;
use crate::commands::structural_graph::extract::{
    build_snapshot_from_blob_delta, build_snapshot_from_blobs, HistoricalFileBlob,
};
use crate::commands::structural_graph::query::{self, GraphProjection};
use crate::commands::structural_graph::storage::load_snapshot_by_id;
use crate::commands::structural_graph::types::stable_graph_id;
use crate::commands::structural_graph::types::{
    GraphSourceAnchor, GraphTrust, StructuralCloneGroup, StructuralGraphCancellation,
    StructuralGraphCommunity, StructuralGraphCoverage, StructuralGraphDiagnostic,
    StructuralGraphEdge, StructuralGraphFileRecord, StructuralGraphMetricFact, StructuralGraphNode,
    StructuralGraphProgress, StructuralGraphSnapshot, BUNDLED_ENGINE_ID, BUNDLED_ENGINE_VERSION,
    STRUCTURAL_GRAPH_SCHEMA_VERSION,
};
use crate::DbState;
use chrono::Utc;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, State};

const DEFAULT_HISTORY_LIMIT: usize = 250;
const MAX_HISTORY_LIMIT: usize = 2_000;
const DEFAULT_GRAPH_LIMIT: usize = 360;
const MAX_GRAPH_LIMIT: usize = 1_500;
const MAX_HISTORICAL_FILES: usize = 25_000;
const MAX_HISTORICAL_BLOB_BYTES: usize = 2 * 1024 * 1024;

static ACTIVE_HISTORY_BACKFILLS: OnceLock<Mutex<HashMap<String, StructuralGraphCancellation>>> =
    OnceLock::new();

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
}

fn active_history_backfills() -> &'static Mutex<HashMap<String, StructuralGraphCancellation>> {
    ACTIVE_HISTORY_BACKFILLS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn release_history_allocator_pressure() {
    #[cfg(target_os = "macos")]
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::malloc_trim(0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryRevision {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub committed_at: String,
    pub author: String,
    pub subject: String,
    pub tags: Vec<String>,
    pub is_release: bool,
    pub is_head: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTimeline {
    pub schema_version: i64,
    pub repo_path: String,
    pub head: String,
    pub generated_at: String,
    pub revisions: Vec<HistoryRevision>,
    pub total_commits: usize,
    pub truncated: bool,
    pub is_shallow: bool,
    pub coverage_complete: bool,
    pub release_ranges: Vec<HistoryReleaseRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryReleaseRange {
    pub id: String,
    pub label: String,
    pub tag: Option<String>,
    pub from_exclusive: Option<String>,
    pub to_inclusive: String,
    pub commit_shas: Vec<String>,
    pub is_unreleased: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistorySearchResult {
    pub revisions: Vec<HistoryRevision>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTopologyNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTopologyEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTopology {
    pub schema_version: i64,
    pub repo_path: String,
    pub revision: String,
    pub nodes: Vec<HistoryTopologyNode>,
    pub edges: Vec<HistoryTopologyEdge>,
    pub changed_paths: Vec<String>,
    pub path_changes: Vec<HistoryPathChange>,
    pub total_files: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryPathChange {
    pub path: String,
    pub change_kind: String,
    pub old_path: Option<String>,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStructuralState {
    pub schema_version: i64,
    pub repo_path: String,
    pub revision: String,
    pub snapshot_id: String,
    pub cached: bool,
    pub projection: GraphProjection,
    pub analysis: StructuralGraphAnalysisSummary,
    pub changed_paths: Vec<String>,
    pub path_changes: Vec<HistoryPathChange>,
    pub indexed_files: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryStructuralDelta {
    pub schema_version: i64,
    #[serde(default)]
    pub materialization_version: i64,
    pub repo_path: String,
    pub before_revision: String,
    pub after_revision: String,
    pub before_snapshot_id: String,
    pub after_snapshot_id: String,
    pub added_node_ids: Vec<String>,
    pub removed_node_ids: Vec<String>,
    pub changed_node_ids: Vec<String>,
    pub added_edge_ids: Vec<String>,
    pub removed_edge_ids: Vec<String>,
    pub changed_edge_ids: Vec<String>,
    pub added_community_ids: Vec<String>,
    pub removed_community_ids: Vec<String>,
    pub added_hub_ids: Vec<String>,
    pub removed_hub_ids: Vec<String>,
    pub added_bridge_ids: Vec<String>,
    pub removed_bridge_ids: Vec<String>,
    pub path_changes: Vec<HistoryPathChange>,
    pub lineage: Vec<HistoryLineageEdge>,
    pub coverage_gap: Option<String>,
    pub generated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_nodes: Vec<StructuralGraphNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_edges: Vec<StructuralGraphEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_communities: Vec<StructuralGraphCommunity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_files: Vec<StructuralGraphFileRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_file_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_metrics: Vec<StructuralGraphMetricFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_metric_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_metric_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upsert_clone_groups: Vec<StructuralCloneGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_clone_group_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_clone_group_order: Vec<String>,
    #[serde(default)]
    pub after_coverage: StructuralGraphCoverage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_diagnostics: Vec<StructuralGraphDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ignore_fingerprint: Option<String>,
    #[serde(default)]
    pub after_truncated: bool,
    #[serde(default)]
    pub after_created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryLineageEdge {
    pub id: String,
    pub from_entity_id: String,
    pub to_entity_id: String,
    pub relation: String,
    pub trust: GraphTrust,
    pub evidence: String,
    pub sources: Vec<GraphSourceAnchor>,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntityMoment {
    pub revision_sha: String,
    pub committed_at: String,
    pub ordinal: i64,
    pub entity_id: String,
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntityEvolution {
    pub schema_version: i64,
    pub repo_path: String,
    pub resolved_revision: String,
    pub entity_id: String,
    pub entity_label: String,
    pub entity_kind: String,
    pub lineage: Vec<HistoryLineageEdge>,
    pub occurrences: Vec<HistoryEntityMoment>,
    pub first_seen: Option<HistoryEntityMoment>,
    pub last_changed: Option<HistoryEntityMoment>,
    pub last_present: Option<HistoryEntityMoment>,
    pub indexed_head: String,
    pub stale: bool,
    pub coverage_gap: Option<String>,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryTemporalReference {
    Revision { revision: String },
    Release { tag: String },
    Date { at: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryAsOfState {
    pub requested: HistoryTemporalReference,
    pub resolved_revision: String,
    pub committed_at: String,
    pub exact: bool,
    pub state: HistoryStructuralState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryBackfillProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub revision: Option<String>,
    pub detail: String,
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryBackfillResult {
    pub repo_path: String,
    pub total: usize,
    pub completed: usize,
    pub built: usize,
    pub cache_hits: usize,
    pub cancelled: bool,
    pub release_checkpoints: usize,
    pub coverage_complete: bool,
    pub refresh_kind: String,
    pub invalidated: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryGraphStatus {
    pub repo_path: String,
    pub indexed: bool,
    pub backfilling: bool,
    pub stale: bool,
    pub current_head: String,
    pub indexed_head: Option<String>,
    pub checkpoint_count: usize,
    pub event_count: usize,
    pub coverage: Value,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFacetStatus {
    Evidenced,
    QualifiedLead,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryFacet {
    pub name: String,
    pub status: HistoryFacetStatus,
    pub summary: String,
    pub trust: GraphTrust,
    pub sources: Vec<GraphSourceAnchor>,
    pub event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryFacetPacket {
    pub schema_version: i64,
    pub repo_path: String,
    pub as_of_revision: String,
    pub entity_id: String,
    pub entity_label: String,
    pub entity_kind: String,
    pub facets: Vec<HistoryFacet>,
    pub gaps: Vec<String>,
    pub contradictions: Vec<String>,
    pub trust_summary: BTreeMap<String, usize>,
    pub indexed_head: String,
    pub stale: bool,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAnnotationDecision {
    Note,
    Confirm,
    Reject,
    Correction,
}

impl HistoryAnnotationDecision {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Confirm => "confirm",
            Self::Reject => "reject",
            Self::Correction => "correction",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Self {
        match value {
            "confirm" => Self::Confirm,
            "reject" => Self::Reject,
            "correction" => Self::Correction,
            _ => Self::Note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryAnnotation {
    pub id: String,
    pub repo_path: String,
    pub revision_sha: Option<String>,
    pub entity_id: Option<String>,
    pub author: String,
    pub body: String,
    pub decision: HistoryAnnotationDecision,
    pub related_event_id: Option<String>,
    pub source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryAnnotationPage {
    pub annotations: Vec<HistoryAnnotation>,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

mod catalog;
mod delta;
mod git_objects;
mod query_helpers;
mod state;
mod storage;

pub(crate) use catalog::canonical_repo_path;
use catalog::git::*;
pub(crate) use catalog::git::{git_text, resolve_revision};
use catalog::persistence::*;
pub(crate) use catalog::repository_tag_fingerprint;
use catalog::*;
pub use catalog::{history_list_releases, history_search, load_history_revisions};
use delta::*;
use git_objects::*;
use query_helpers::*;
pub(crate) use query_helpers::{
    history_index_freshness, load_entity_annotation_contradictions, load_entity_occurrences,
    load_lineage_family, load_outcome_events,
};
use state::*;
pub use state::{
    get_history_as_of, get_history_entity_evolution, get_history_structural_delta,
    get_history_structural_state,
};
pub(crate) use state::{reconstruct_history_as_of, resolve_temporal_reference};
pub(crate) use storage::history_storage_key;
use storage::*;

#[cfg(test)]
mod tests;
