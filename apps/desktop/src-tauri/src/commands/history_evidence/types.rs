use crate::commands::structural_graph::types::{GraphSourceAnchor, GraphTrust};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAdapterAvailability {
    Available,
    Empty,
    NeedsConfiguration,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAdapterConsent {
    LocalDefault,
    ExplicitImport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEvidenceAdapterDescriptor {
    pub id: String,
    pub label: String,
    pub source_kind: String,
    pub availability: HistoryAdapterAvailability,
    pub consent: HistoryAdapterConsent,
    pub configured: bool,
    pub local_only: bool,
    pub network_access: bool,
    pub reads: Vec<String>,
    pub redaction: String,
    pub source_cursor: Option<String>,
    pub last_observed_at: Option<String>,
    pub freshness: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub struct HistoryEvidenceRecord {
    pub id: String,
    pub source_id: String,
    pub source_record_id: String,
    pub source_cursor: Option<String>,
    pub event_kind: String,
    pub observed_at: String,
    pub effective_at: Option<String>,
    pub entity_candidates: Vec<String>,
    pub release_candidates: Vec<String>,
    pub episode_keys: Vec<String>,
    pub trust: GraphTrust,
    pub summary: String,
    pub sources: Vec<GraphSourceAnchor>,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub struct HistoryEvidenceBatch {
    pub adapter_id: String,
    pub records: Vec<HistoryEvidenceRecord>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEvidenceRefreshResult {
    pub repo_path: String,
    pub imported: usize,
    pub already_present: usize,
    pub adapters: Vec<(String, usize)>,
    pub network_requests: usize,
    pub refreshed_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryLocalEvidenceExport {
    pub schema_version: i64,
    pub source: String,
    pub cursor: Option<String>,
    pub records: Vec<HistoryLocalEvidenceExportRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryLocalEvidenceExportRecord {
    pub id: String,
    pub event_kind: String,
    pub observed_at: String,
    pub effective_at: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub release_ids: Vec<String>,
    #[serde(default)]
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub episode_keys: Vec<String>,
}

#[allow(dead_code)]
pub struct HistoryEvidenceContext<'a> {
    pub repo_path: &'a Path,
    pub cursor: Option<&'a str>,
    pub limit: usize,
}

/// Local-first ingestion boundary for immutable historical evidence.
///
/// Implementations must return deterministic source IDs, never retain credentials,
/// and never perform network I/O unless a future, separately configured adapter is
/// explicitly invoked through a consent-bearing surface.
#[allow(dead_code)]
pub trait HistoryEvidenceAdapter: Send + Sync {
    fn descriptor(
        &self,
        connection: &Connection,
        repo_path: &Path,
    ) -> Result<HistoryEvidenceAdapterDescriptor, String>;

    fn collect(
        &self,
        connection: &Connection,
        context: &HistoryEvidenceContext<'_>,
    ) -> Result<HistoryEvidenceBatch, String>;
}
