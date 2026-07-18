use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const STRUCTURAL_GRAPH_SCHEMA_VERSION: i64 = 3;
pub const BUNDLED_ENGINE_ID: &str = "codevetter-tree-sitter";
pub const BUNDLED_ENGINE_VERSION: &str = "1";
pub const STRUCTURAL_METRIC_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphTrust {
    Extracted,
    Inferred,
    Ambiguous,
    #[default]
    Legacy,
}

impl GraphTrust {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "extracted",
            Self::Inferred => "inferred",
            Self::Ambiguous => "ambiguous",
            Self::Legacy => "legacy",
        }
    }

    pub fn from_storage(value: &str) -> Self {
        match value {
            "extracted" => Self::Extracted,
            "inferred" => Self::Inferred,
            "ambiguous" => Self::Ambiguous,
            _ => Self::Legacy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphOrigin {
    Syntax,
    Resolution,
    Analysis,
    Metadata,
    Extracted,
    Deterministic,
    ModelSynthesized,
    HumanConfirmed,
    ImportedNodeLink,
    UserAnnotation,
    #[default]
    LegacyMetadata,
}

impl GraphOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Resolution => "resolution",
            Self::Analysis => "analysis",
            Self::Metadata => "metadata",
            Self::Extracted => "extracted",
            Self::Deterministic => "deterministic",
            Self::ModelSynthesized => "model_synthesized",
            Self::HumanConfirmed => "human_confirmed",
            Self::ImportedNodeLink => "imported_node_link",
            Self::UserAnnotation => "user_annotation",
            Self::LegacyMetadata => "legacy_metadata",
        }
    }

    pub fn from_storage(value: &str) -> Self {
        match value {
            "syntax" => Self::Syntax,
            "resolution" => Self::Resolution,
            "analysis" => Self::Analysis,
            "metadata" => Self::Metadata,
            "extracted" => Self::Extracted,
            "deterministic" => Self::Deterministic,
            "model_synthesized" => Self::ModelSynthesized,
            "human_confirmed" => Self::HumanConfirmed,
            "imported_node_link" => Self::ImportedNodeLink,
            "user_annotation" => Self::UserAnnotation,
            _ => Self::LegacyMetadata,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphSourceAnchor {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

impl GraphSourceAnchor {
    pub fn path(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            start_line: None,
            start_column: None,
            end_line: None,
            end_column: None,
            excerpt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_id: Option<String>,
    #[serde(default)]
    pub trust: GraphTrust,
    #[serde(default)]
    pub origin: GraphOrigin,
    #[serde(default)]
    pub sources: Vec<GraphSourceAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub evidence: String,
    #[serde(default)]
    pub trust: GraphTrust,
    #[serde(default)]
    pub origin: GraphOrigin,
    #[serde(default)]
    pub sources: Vec<GraphSourceAnchor>,
    #[serde(default)]
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphCommunity {
    pub id: String,
    pub label: String,
    pub member_count: usize,
    #[serde(default)]
    pub hub_node_ids: Vec<String>,
    #[serde(default)]
    pub bridge_node_ids: Vec<String>,
    #[serde(default)]
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanguageCoverage {
    pub language: String,
    pub supported: bool,
    pub discovered_files: usize,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub error_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StructuralGraphCoverage {
    pub discovered_files: usize,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub error_files: usize,
    pub generated_files: usize,
    pub sensitive_files: usize,
    pub binary_files: usize,
    #[serde(default)]
    pub languages: Vec<LanguageCoverage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphDiagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphEngineInfo {
    pub id: String,
    pub version: String,
    pub bundled: bool,
    pub syntax_aware: bool,
    #[serde(default)]
    pub supported_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphFileRecord {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub disposition: String,
    pub byte_size: u64,
    pub node_count: usize,
    pub edge_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralControlFlowFact {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub nesting: usize,
    pub source: GraphSourceAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralBoundaryFact {
    pub kind: String,
    pub target: String,
    pub source: GraphSourceAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StructuralCodeMetrics {
    pub line_count: usize,
    pub statement_count: usize,
    pub parameter_count: usize,
    pub cyclomatic_complexity: usize,
    pub cognitive_complexity: usize,
    pub max_nesting: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cohesion: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphMetricFact {
    pub schema_version: i64,
    pub id: String,
    pub node_id: String,
    pub path: String,
    pub scope_kind: String,
    pub language: String,
    pub public_surface: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_surface_reason: Option<String>,
    pub syntax_fingerprint: String,
    pub normalized_token_count: usize,
    pub normalization_method: String,
    pub metrics: StructuralCodeMetrics,
    #[serde(default)]
    pub control_flow: Vec<StructuralControlFlowFact>,
    #[serde(default)]
    pub definitions: Vec<String>,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<StructuralBoundaryFact>,
    #[serde(default)]
    pub sources: Vec<GraphSourceAnchor>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralCloneRegion {
    pub metric_id: String,
    pub node_id: String,
    pub path: String,
    pub source: GraphSourceAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralCloneGroup {
    pub id: String,
    pub syntax_fingerprint: String,
    pub normalization_method: String,
    pub normalized_token_count: usize,
    pub similarity: f64,
    pub regions: Vec<StructuralCloneRegion>,
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralGraphSnapshot {
    pub schema_version: i64,
    pub id: String,
    pub repo_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_head: Option<String>,
    pub created_at: String,
    pub engine: StructuralGraphEngineInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_fingerprint: Option<String>,
    pub coverage: StructuralGraphCoverage,
    #[serde(default)]
    pub diagnostics: Vec<StructuralGraphDiagnostic>,
    #[serde(default)]
    pub communities: Vec<StructuralGraphCommunity>,
    #[serde(default)]
    pub files: Vec<StructuralGraphFileRecord>,
    #[serde(default)]
    pub nodes: Vec<StructuralGraphNode>,
    #[serde(default)]
    pub edges: Vec<StructuralGraphEdge>,
    #[serde(default)]
    pub metrics: Vec<StructuralGraphMetricFact>,
    #[serde(default)]
    pub clone_groups: Vec<StructuralCloneGroup>,
    pub truncated: bool,
}

pub fn namespaced_graph_id(repository_id: &str, local_id: &str) -> String {
    stable_graph_id("workspace-node", &format!("{repository_id}\0{local_id}"))
}

#[derive(Debug, Clone)]
pub struct StructuralGraphBuildInput {
    pub repo_root: PathBuf,
    pub repo_head: Option<String>,
    pub changed_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub previous_cursor: Option<String>,
    pub previous_snapshot: Option<Box<StructuralGraphSnapshot>>,
    pub max_files: usize,
    pub max_bytes_per_file: u64,
}

impl StructuralGraphBuildInput {
    pub fn full(repo_root: PathBuf, repo_head: Option<String>) -> Self {
        Self {
            repo_root,
            repo_head,
            changed_files: Vec::new(),
            deleted_files: Vec::new(),
            previous_cursor: None,
            previous_snapshot: None,
            max_files: 25_000,
            max_bytes_per_file: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralGraphProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct StructuralGraphCancellation {
    cancelled: Arc<AtomicBool>,
    #[cfg(test)]
    cancel_after_checks: Arc<AtomicUsize>,
    #[cfg(test)]
    checks: Arc<AtomicUsize>,
}

impl StructuralGraphCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        #[cfg(test)]
        {
            let checks = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
            let threshold = self.cancel_after_checks.load(Ordering::SeqCst);
            if threshold > 0 && checks >= threshold {
                self.cancel();
            }
        }
        self.cancelled.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn cancel_after_checks(&self, checks: usize) {
        self.cancel_after_checks
            .store(checks.max(1), Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn check_count(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralGraphError {
    Cancelled,
    InvalidRepository(String),
    Io(String),
    Parse(String),
    Storage(String),
    UnsupportedSchema(i64),
}

impl std::fmt::Display for StructuralGraphError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "Structural graph build cancelled"),
            Self::InvalidRepository(message)
            | Self::Io(message)
            | Self::Parse(message)
            | Self::Storage(message) => formatter.write_str(message),
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "Unsupported structural graph schema version {version}"
                )
            }
        }
    }
}

impl std::error::Error for StructuralGraphError {}

pub trait StructuralGraphProgressSink: Send + Sync {
    fn report(&self, progress: StructuralGraphProgress);
}

impl<F> StructuralGraphProgressSink for F
where
    F: Fn(StructuralGraphProgress) + Send + Sync,
{
    fn report(&self, progress: StructuralGraphProgress) {
        self(progress);
    }
}

pub trait StructuralGraphEngine: Send + Sync {
    fn info(&self) -> StructuralGraphEngineInfo;

    fn build(
        &self,
        input: &StructuralGraphBuildInput,
        cancellation: &StructuralGraphCancellation,
        progress: &dyn StructuralGraphProgressSink,
    ) -> Result<StructuralGraphSnapshot, StructuralGraphError>;
}

pub fn stable_graph_id(kind: &str, identity: &str) -> String {
    // FNV-1a is deliberately implemented here instead of DefaultHasher, whose
    // output is not a stable persistence contract across Rust releases.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in kind.bytes().chain([0]).chain(identity.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{kind}:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_deterministic_and_kind_scoped() {
        assert_eq!(
            stable_graph_id("function", "src/main.rs::run"),
            stable_graph_id("function", "src/main.rs::run")
        );
        assert_ne!(
            stable_graph_id("function", "src/main.rs::run"),
            stable_graph_id("method", "src/main.rs::run")
        );
    }

    #[test]
    fn cancellation_is_shared_between_clones() {
        let first = StructuralGraphCancellation::default();
        let second = first.clone();
        second.cancel();
        assert!(first.is_cancelled());
    }

    #[test]
    fn workspace_ids_namespace_matching_local_symbols_by_repository() {
        let local = "function:shared";
        let first = namespaced_graph_id("repo:first", local);
        let second = namespaced_graph_id("repo:second", local);
        assert_ne!(first, second);
        assert_eq!(first, namespaced_graph_id("repo:first", local));
        assert!(first.starts_with("workspace-node:"));
    }
}
