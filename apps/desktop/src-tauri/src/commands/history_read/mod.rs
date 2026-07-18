//! Read-only release-history query service shared by Tauri and MCP.
//!
//! This is the only layer in the MCP path allowed to understand graph/history
//! persistence. The protocol adapter maps typed inputs and outputs only.

use crate::commands::{
    history_graph::{
        canonical_repo_path, git_text, history_index_freshness, history_storage_key,
        load_entity_annotation_contradictions, load_entity_occurrences, load_history_revisions,
        load_lineage_family, load_outcome_events, reconstruct_history_as_of,
        repository_tag_fingerprint, resolve_temporal_reference, HistoryAnnotation,
        HistoryAnnotationDecision, HistoryAnnotationPage, HistoryAsOfState, HistoryEntityEvolution,
        HistoryFacet, HistoryFacetPacket, HistoryFacetStatus, HistoryGraphStatus,
        HistoryOpaqueCursor, HistoryReleaseCatalog, HistorySearchResult, HistoryStructuralState,
        HistoryTemporalReference, HistoryTimelineCenter, HistoryTimelineWindow,
    },
    history_query::{query_causal_trace, HistoryCausalSelector, HistoryCausalTrace},
    structural_graph::{
        query::{self, GraphSnapshotDiff},
        types::{GraphSourceAnchor, GraphTrust},
    },
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistorySearchKind {
    Release,
    Commit,
    Entity,
    Event,
    Annotation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySearchItem {
    pub kind: HistorySearchKind,
    pub id: String,
    pub label: String,
    pub summary: String,
    pub revision: Option<String>,
    pub recorded_at: Option<String>,
    pub trust: GraphTrust,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryUnifiedSearch {
    pub schema_version: i64,
    pub items: Vec<HistorySearchItem>,
    pub truncated: bool,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryComparison {
    pub schema_version: i64,
    pub before: HistoryTemporalReference,
    pub after: HistoryTemporalReference,
    pub before_revision: String,
    pub after_revision: String,
    pub structural: GraphSnapshotDiff,
    pub changed_paths: Vec<String>,
    pub event_kind_counts: BTreeMap<String, usize>,
    pub gaps: Vec<String>,
    pub stale: bool,
    pub indexed_head: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvidenceDetail {
    pub schema_version: i64,
    pub id: String,
    pub event_kind: String,
    pub revision_sha: Option<String>,
    pub entity_id: Option<String>,
    pub related_entity_id: Option<String>,
    pub relation_kind: Option<String>,
    pub trust: GraphTrust,
    pub origin: String,
    pub source_id: String,
    pub source_cursor: Option<String>,
    pub summary: Option<String>,
    pub sources: Vec<GraphSourceAnchor>,
    pub recorded_at: String,
    pub available: bool,
}

pub struct HistoryReadService<'a> {
    connection: &'a Connection,
    root: PathBuf,
    repo_path: String,
    storage_key: String,
    current_head: String,
}

impl<'a> HistoryReadService<'a> {
    pub fn new(connection: &'a Connection, repo_path: &str) -> Result<Self, String> {
        let root = canonical_repo_path(repo_path)?;
        let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
        Self::new_with_current_head(connection, root, current_head)
    }

    pub fn new_with_current_head(
        connection: &'a Connection,
        root: PathBuf,
        current_head: String,
    ) -> Result<Self, String> {
        let repo_path = root.to_string_lossy().to_string();
        let storage_key = history_storage_key(&repo_path);
        Ok(Self {
            connection,
            root,
            repo_path,
            storage_key,
            current_head,
        })
    }
}

mod annotations;
pub mod api;
pub(crate) mod contributors;
mod evidence;
mod explain;
mod landmarks;
mod releases;
mod search;
mod state;
mod status;
pub(crate) mod temporal;

pub(super) fn unknown_facet(name: &str, summary: &str) -> HistoryFacet {
    HistoryFacet {
        name: name.to_string(),
        status: HistoryFacetStatus::Unknown,
        summary: summary.to_string(),
        trust: GraphTrust::Inferred,
        sources: Vec::new(),
        event_ids: Vec::new(),
    }
}

pub(super) fn weakest_trust(values: impl Iterator<Item = GraphTrust>) -> GraphTrust {
    values
        .max_by_key(|trust| match trust {
            GraphTrust::Extracted => 0,
            GraphTrust::Inferred => 1,
            GraphTrust::Ambiguous => 2,
            GraphTrust::Legacy => 3,
        })
        .unwrap_or(GraphTrust::Inferred)
}

pub(super) fn source_is_available(root: &std::path::Path, source: &GraphSourceAnchor) -> bool {
    if source.path.is_empty() {
        true
    } else {
        let path = PathBuf::from(&source.path);
        if path.is_absolute() {
            path.exists()
        } else {
            root.join(path).exists()
        }
    }
}

/// Keeps cursor transport identical across read services while each service
/// retains ownership of its schema and scope validation.
pub(super) fn encode_opaque_cursor(
    payload: &impl Serialize,
    context: &str,
) -> Result<HistoryOpaqueCursor, String> {
    serde_json::to_vec(payload)
        .map(|bytes| HistoryOpaqueCursor(URL_SAFE_NO_PAD.encode(bytes)))
        .map_err(|error| format!("Encode {context}: {error}"))
}

pub(super) fn decode_opaque_cursor<T: DeserializeOwned>(
    cursor: &HistoryOpaqueCursor,
) -> Result<T, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(&cursor.0)
        .map_err(|_| "Invalid history cursor".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid history cursor".to_string())
}

#[cfg(test)]
mod tests;
