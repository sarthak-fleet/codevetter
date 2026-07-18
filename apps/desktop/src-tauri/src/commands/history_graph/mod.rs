use crate::commands::git_metadata::{is_release_tag, read_git_tags, GitTagRecord};
use crate::commands::history_evidence::refresh_builtin_adapters;
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
    #[serde(default)]
    pub ordinal: i64,
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
    #[serde(skip, default)]
    pub reachable_revisions: Vec<String>,
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

pub const HISTORY_RELEASE_CATALOG_SCHEMA_VERSION: i64 = 1;
pub const HISTORY_TIMELINE_WINDOW_SCHEMA_VERSION: i64 = 1;
pub const HISTORY_LANDMARK_CATALOG_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryTimelineCenter {
    Release { tag: String },
    Revision { revision_sha: String },
    Landmark { landmark_id: String },
    Cursor { cursor: HistoryOpaqueCursor },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HistoryOpaqueCursor(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryReleaseTagKind {
    Annotated,
    Lightweight,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCoverageState {
    Complete,
    Partial,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryReadCoverage {
    pub state: HistoryCoverageState,
    pub ancestry_complete: bool,
    pub is_shallow: bool,
    pub truncated: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryReadFreshness {
    pub indexed_revision: Option<String>,
    pub current_revision: Option<String>,
    pub indexed_tags_fingerprint: Option<String>,
    pub current_tags_fingerprint: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryReleaseCatalogEntry {
    pub id: String,
    pub tag: String,
    pub tag_kind: HistoryReleaseTagKind,
    pub revision_sha: String,
    pub ordinal: i64,
    pub tagged_at: Option<String>,
    /// All tags at this rail position, while this row still represents one tag.
    pub coincident_tags: Vec<String>,
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub interval: Option<HistoryReleaseIntervalMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryReleaseIntervalMetadata {
    pub schema_version: i64,
    pub from_exclusive_sha: Option<String>,
    pub commit_count: Option<usize>,
    pub observed_commit_count: usize,
    pub coverage: HistoryCoverageState,
    pub coverage_reason: Option<String>,
}

/// A select-able point on the revision timeline. Release tags are extracted
/// Git facts; candidate inflections are qualified, non-causal observations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryLandmarkKind {
    Release,
    CandidateInflection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryLandmarkTrust {
    Extracted,
    Qualified,
    QualifiedPartial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryLandmark {
    pub id: String,
    pub kind: HistoryLandmarkKind,
    pub revision_sha: String,
    pub ordinal: i64,
    pub label: String,
    /// Every release tag at this revision. Candidate inflections leave this empty.
    pub tags: Vec<String>,
    pub trust: HistoryLandmarkTrust,
    pub score_milli: Option<i64>,
    pub components: Value,
    pub reasons: Vec<String>,
    pub caveats: Vec<String>,
    pub coverage: Value,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryLandmarkCatalog {
    pub schema_version: i64,
    pub landmarks: Vec<HistoryLandmark>,
    pub coverage: HistoryReadCoverage,
    pub freshness: HistoryReadFreshness,
    pub applied_limit: usize,
    pub truncated: bool,
    pub next_cursor: Option<HistoryOpaqueCursor>,
}

impl Default for HistoryLandmarkCatalog {
    fn default() -> Self {
        Self {
            schema_version: HISTORY_LANDMARK_CATALOG_SCHEMA_VERSION,
            landmarks: Vec::new(),
            coverage: HistoryReadCoverage::default(),
            freshness: HistoryReadFreshness::default(),
            applied_limit: 0,
            truncated: false,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryReleaseCatalog {
    pub schema_version: i64,
    /// One canonical row per tag; coincident tags are not collapsed here.
    pub releases: Vec<HistoryReleaseCatalogEntry>,
    pub coverage: HistoryReadCoverage,
    pub freshness: HistoryReadFreshness,
    pub applied_limit: usize,
    pub truncated: bool,
    pub next_cursor: Option<HistoryOpaqueCursor>,
}

impl Default for HistoryReleaseCatalog {
    fn default() -> Self {
        Self {
            schema_version: HISTORY_RELEASE_CATALOG_SCHEMA_VERSION,
            releases: Vec::new(),
            coverage: HistoryReadCoverage::default(),
            freshness: HistoryReadFreshness::default(),
            applied_limit: 0,
            truncated: false,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryTimelineWindow {
    pub schema_version: i64,
    pub center_revision: Option<String>,
    pub revisions: Vec<HistoryRevision>,
    pub releases: Vec<HistoryReleaseCatalogEntry>,
    pub coverage: HistoryReadCoverage,
    pub freshness: HistoryReadFreshness,
    pub applied_limit: usize,
    pub truncated: bool,
    pub has_older: bool,
    pub has_newer: bool,
    pub older_cursor: Option<HistoryOpaqueCursor>,
    pub newer_cursor: Option<HistoryOpaqueCursor>,
}

impl Default for HistoryTimelineWindow {
    fn default() -> Self {
        Self {
            schema_version: HISTORY_TIMELINE_WINDOW_SCHEMA_VERSION,
            center_revision: None,
            revisions: Vec::new(),
            releases: Vec::new(),
            coverage: HistoryReadCoverage::default(),
            freshness: HistoryReadFreshness::default(),
            applied_limit: 0,
            truncated: false,
            has_older: false,
            has_newer: false,
            older_cursor: None,
            newer_cursor: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistorySearchResult {
    pub revisions: Vec<HistoryRevision>,
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

pub mod api;
pub mod catalog;
mod delta;
mod git_objects;
mod history_facts;
// The pure detector is integrated with atomic landmark publication in task 3.3.
#[allow(dead_code)]
pub(crate) mod inflections;
mod query_helpers;
pub mod state;
mod storage;

pub use api::{
    add_history_annotation, backfill_history_graph, cancel_history_backfill,
    explain_history_entity, get_history_graph_status, get_history_timeline,
    list_history_annotations,
};
pub use catalog::load_history_revisions;
pub use state::{
    get_history_entity_evolution, get_history_structural_delta, get_history_structural_state,
};

pub(crate) use catalog::git::{git_text, resolve_revision};
pub(crate) use catalog::{canonical_repo_path, repository_tag_fingerprint};
pub(crate) use query_helpers::{
    history_index_freshness, load_entity_annotation_contradictions, load_entity_occurrences,
    load_lineage_family, load_outcome_events,
};
pub(crate) use state::{reconstruct_history_as_of, resolve_temporal_reference};
pub(crate) use storage::history_storage_key;

use catalog::git::*;
use catalog::persistence::*;
use catalog::*;
use delta::*;
use git_objects::*;
use query_helpers::*;
use state::*;
use storage::*;

#[cfg(test)]
mod tests;
