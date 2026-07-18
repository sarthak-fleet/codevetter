use crate::commands::structural_graph::types::{GraphSourceAnchor, GraphTrust};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryCausalSelector {
    Event { event_id: String },
    Entity { entity_id: String },
    Revision { revision: String },
    Release { tag: String },
    EpisodeKey { key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCausalStage {
    Intent,
    Implementation,
    Verification,
    Release,
    Outcome,
    Regression,
    FollowUp,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCausalLinkStatus {
    Evidenced,
    QualifiedLead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCausalEvent {
    pub id: String,
    pub revision_sha: Option<String>,
    pub event_kind: String,
    pub stage: HistoryCausalStage,
    pub summary: String,
    pub trust: GraphTrust,
    pub origin: String,
    pub source_id: String,
    pub source_cursor: Option<String>,
    pub recorded_at: String,
    pub effective_at: Option<String>,
    pub entity_id: Option<String>,
    pub related_entity_id: Option<String>,
    pub relation_kind: Option<String>,
    pub episode_keys: Vec<String>,
    pub sources: Vec<GraphSourceAnchor>,
    pub source_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCausalLink {
    pub id: String,
    pub from_event_id: String,
    pub to_event_id: String,
    pub relation: String,
    pub status: HistoryCausalLinkStatus,
    pub trust: GraphTrust,
    pub evidence: String,
    pub sources: Vec<GraphSourceAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryChangeEpisode {
    pub id: String,
    pub anchor_event_id: String,
    pub episode_keys: Vec<String>,
    pub events: Vec<HistoryCausalEvent>,
    pub links: Vec<HistoryCausalLink>,
    pub qualified_leads: Vec<HistoryCausalLink>,
    pub qualified_lead_events: Vec<HistoryCausalEvent>,
    pub stages_present: Vec<HistoryCausalStage>,
    pub gaps: Vec<String>,
    pub contradictions: Vec<String>,
    pub trust_summary: BTreeMap<String, usize>,
    pub started_at: String,
    pub ended_at: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCausalTrace {
    pub schema_version: i64,
    pub repo_path: String,
    pub selector: HistoryCausalSelector,
    pub episodes: Vec<HistoryChangeEpisode>,
    pub indexed_head: String,
    pub stale: bool,
    pub coverage: Value,
    pub gaps: Vec<String>,
    pub scanned_events: usize,
    pub total_events: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryReviewSlice {
    pub schema_version: i64,
    pub repo_path: String,
    pub files: Vec<String>,
    pub entity_ids: Vec<String>,
    pub episodes: Vec<HistoryChangeEpisode>,
    pub constraints: Vec<HistoryCausalEvent>,
    pub verification: Vec<HistoryCausalEvent>,
    pub failures: Vec<HistoryCausalEvent>,
    pub regressions: Vec<HistoryCausalEvent>,
    pub qualified_leads: Vec<HistoryCausalEvent>,
    pub gaps: Vec<String>,
    pub indexed_head: String,
    pub stale: bool,
    pub coverage: Value,
    pub truncated: bool,
}
