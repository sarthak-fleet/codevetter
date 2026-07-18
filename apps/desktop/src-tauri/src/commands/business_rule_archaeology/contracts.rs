use serde::{Deserialize, Serialize};

pub const ARCHAEOLOGY_SCHEMA_VERSION: u32 = 1;
pub const ARCHAEOLOGY_CONTRACT_ID: &str = "codevetter.business-rule-archaeology.v1";
/// Persistence evolves independently from the desktop/read envelope so an
/// additive local migration does not silently change every public consumer.
pub(crate) const ARCHAEOLOGY_STORAGE_SCHEMA_VERSION: u32 = 2;
/// Optional synthesis remains on its already-qualified wire contract.
pub(crate) const ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyCoverageState {
    Complete,
    Partial,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyTrust {
    Extracted,
    Deterministic,
    ModelSynthesized,
    HumanConfirmed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyConfidence {
    High,
    Medium,
    Low,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyRuleLifecycle {
    Candidate,
    ReviewNeeded,
    Accepted,
    Rejected,
    Superseded,
    Conflicted,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyJobStage {
    Inventory,
    Parse,
    Link,
    Derive,
    Synthesize,
    Validate,
    Publish,
    Cleanup,
    #[default]
    Idle,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyJobState {
    Pending,
    Running,
    Paused,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyFactKind {
    Declaration,
    DataField,
    Constant,
    Predicate,
    Decision,
    Calculation,
    Mutation,
    Call,
    InputOutput,
    Transaction,
    ControlFlow,
    EntryPoint,
    Include,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyFactEdgeKind {
    Defines,
    Reads,
    Writes,
    Calls,
    Includes,
    Controls,
    BranchesTo,
    Calculates,
    BeginsTransaction,
    CommitsTransaction,
    RollsBackTransaction,
    Supports,
    Contradicts,
    Aliases,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyRuleKind {
    Validation,
    Calculation,
    Eligibility,
    Entitlement,
    Routing,
    Mutation,
    Exception,
    Lifecycle,
    Transaction,
    Other,
}

/// Canonical owned payload persisted for one immutable temporal rule snapshot.
/// Publication and historical reads share this shape so snapshot JSON has one
/// interpretation across both paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyTemporalSnapshotPayload {
    pub title: String,
    pub clauses: Vec<ArchaeologyTemporalClausePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyTemporalClausePayload {
    pub ordinal: u64,
    pub text: String,
    pub trust: String,
    pub confidence: String,
    pub caveats: Vec<String>,
    pub evidence: Vec<ArchaeologyTemporalEvidencePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyTemporalEvidencePayload {
    pub role: String,
    pub fact_identity: String,
    pub fact_kind: String,
    pub parser_identity: String,
    pub spans: Vec<ArchaeologyTemporalSpanPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyTemporalSpanPayload {
    pub path_identity: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u64,
    pub start_column: u64,
    pub end_line: u64,
    pub end_column: u64,
}

/// One deterministic, cited rule candidate passed to optional synthesis.
///
/// Deterministic rendering, optional synthesis, and the TypeScript boundary
/// intentionally share this packet shape rather than maintaining provider DTOs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyEvidencePacket {
    pub packet_id: String,
    pub kind: ArchaeologyRuleKind,
    pub anchor_fact_id: String,
    pub supporting_fact_ids: Vec<String>,
    pub contradicting_fact_ids: Vec<String>,
    pub relationship_ids: Vec<String>,
    pub evidence_span_ids: Vec<String>,
    pub unresolved_fact_ids: Vec<String>,
    pub unresolved_reasons: Vec<String>,
    pub confidence: ArchaeologyConfidence,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyRepositoryIdentity {
    pub repository_id: String,
    pub revision_sha: String,
    pub source_identity: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologySourceUnitIdentity {
    pub source_unit_id: String,
    pub repository_id: String,
    pub revision_sha: String,
    pub path_identity: String,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub hash_algorithm: Option<String>,
    /// Revision-neutral, one-way identity used to detect changes when content
    /// hashing is intentionally unavailable (for example protected sources).
    pub change_identity: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologySourceClassification {
    Source,
    Generated,
    Vendor,
    Protected,
    Opaque,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyPosition {
    pub byte: u64,
    /// One-based line for editor interoperability.
    pub line: u64,
    /// One-based Unicode-scalar column. Byte identity remains authoritative.
    pub column: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologySourceSpan {
    pub span_id: String,
    pub source_unit_id: String,
    pub revision_sha: String,
    pub start: ArchaeologyPosition,
    pub end: ArchaeologyPosition,
}

impl ArchaeologySourceSpan {
    pub fn validate(&self) -> Result<(), String> {
        if self.span_id.is_empty() || self.source_unit_id.is_empty() {
            return Err("Source span identity is required".to_string());
        }
        validate_revision_sha(&self.revision_sha)?;
        if self.start.line == 0
            || self.start.column == 0
            || self.end.line == 0
            || self.end.column == 0
        {
            return Err("Source span lines and columns are one-based".to_string());
        }
        if self.end.byte < self.start.byte
            || (self.end.line, self.end.column) < (self.start.line, self.start.column)
        {
            return Err("Source span end precedes its start".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyParserCapability {
    pub parser_id: String,
    pub parser_version: String,
    pub language: String,
    pub dialects: Vec<String>,
    pub constructs: Vec<ArchaeologyFactKind>,
    pub exact_spans: bool,
    pub preprocessing: bool,
    pub recovery: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyCoverage {
    pub state: ArchaeologyCoverageState,
    pub parser_coverage: ArchaeologyCoverageState,
    pub repository_coverage: ArchaeologyCoverageState,
    pub temporal_coverage: ArchaeologyCoverageState,
    pub discovered_source_units: u64,
    pub indexed_source_units: u64,
    pub discovered_bytes: u64,
    pub indexed_bytes: u64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyFreshness {
    pub indexed_revision: Option<String>,
    pub current_revision: Option<String>,
    pub parser_identity: Option<String>,
    pub current_parser_identity: Option<String>,
    pub config_identity: Option<String>,
    pub current_config_identity: Option<String>,
    pub stale: bool,
    pub reasons: Vec<String>,
    /// A human decision remains auditable after an index becomes stale, but it
    /// must not be presented as review of the current code.
    pub human_review_decisions_present: bool,
    pub human_review_decisions_stale: bool,
    pub human_review_stale_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyFact {
    pub fact_id: String,
    pub kind: ArchaeologyFactKind,
    pub label: String,
    pub span_ids: Vec<String>,
    pub parser_id: String,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    #[serde(default)]
    pub attributes: Vec<ArchaeologyAttribute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyAttribute {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyFactEdge {
    pub edge_id: String,
    pub from_fact_id: String,
    pub to_fact_id: String,
    pub kind: ArchaeologyFactEdgeKind,
    pub trust: ArchaeologyTrust,
    pub evidence_span_ids: Vec<String>,
    pub unresolved_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyRuleClause {
    pub clause_id: String,
    pub text: String,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    pub supporting_fact_ids: Vec<String>,
    pub contradicting_fact_ids: Vec<String>,
    pub evidence_span_ids: Vec<String>,
    pub caveats: Vec<String>,
}

impl ArchaeologyRuleClause {
    pub fn validate(&self) -> Result<(), String> {
        if self.clause_id.is_empty() || self.text.trim().is_empty() {
            return Err("Rule clause identity and text are required".to_string());
        }
        if self.supporting_fact_ids.is_empty() || self.evidence_span_ids.is_empty() {
            return Err("Every rule clause requires supporting facts and source spans".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyRulePacket {
    pub rule_id: String,
    pub repository_id: String,
    pub generation_id: String,
    pub revision_sha: String,
    pub kind: ArchaeologyRuleKind,
    pub title: String,
    pub domain_ids: Vec<String>,
    pub lifecycle: ArchaeologyRuleLifecycle,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    pub clauses: Vec<ArchaeologyRuleClause>,
    pub dependency_rule_ids: Vec<String>,
    pub conflict_rule_ids: Vec<String>,
    pub alias_rule_ids: Vec<String>,
    pub coverage: ArchaeologyCoverage,
    pub parser_identity: String,
    pub algorithm_identity: String,
    pub synthesis_identity: Option<String>,
}

impl ArchaeologyRulePacket {
    pub fn validate(&self) -> Result<(), String> {
        if self.rule_id.is_empty() || self.repository_id.is_empty() || self.generation_id.is_empty()
        {
            return Err("Rule, repository, and generation identities are required".to_string());
        }
        validate_revision_sha(&self.revision_sha)?;
        if self.clauses.is_empty() {
            return Err("A published rule requires at least one clause".to_string());
        }
        for clause in &self.clauses {
            clause.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyRuleConflict {
    pub conflict_id: String,
    pub rule_ids: Vec<String>,
    pub supporting_fact_ids: Vec<String>,
    pub summary: String,
    pub trust: ArchaeologyTrust,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyJobStatus {
    pub schema_version: u32,
    pub job_id: Option<String>,
    pub repository_id: Option<String>,
    pub generation_id: Option<String>,
    pub owner_id: Option<String>,
    pub stage: ArchaeologyJobStage,
    pub state: ArchaeologyJobState,
    pub completed_units: u64,
    pub total_units: Option<u64>,
    pub checkpoint_identity: Option<String>,
    pub cancellation_requested: bool,
    pub coverage: ArchaeologyCoverage,
    pub updated_at: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyPageInfo {
    pub applied_limit: usize,
    pub total_rows: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ArchaeologyCatalogPage {
    pub schema_version: u32,
    pub contract_id: String,
    pub repository_id: Option<String>,
    pub generation_id: Option<String>,
    pub rules: Vec<ArchaeologyRulePacket>,
    pub coverage: ArchaeologyCoverage,
    pub freshness: ArchaeologyFreshness,
    pub page: ArchaeologyPageInfo,
}

impl Default for ArchaeologyCatalogPage {
    fn default() -> Self {
        Self {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            contract_id: ARCHAEOLOGY_CONTRACT_ID.to_string(),
            repository_id: None,
            generation_id: None,
            rules: Vec::new(),
            coverage: ArchaeologyCoverage::default(),
            freshness: ArchaeologyFreshness::default(),
            page: ArchaeologyPageInfo::default(),
        }
    }
}

pub(crate) fn validate_revision_sha(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("An exact lowercase full revision SHA is required".to_string())
    }
}

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod tests;
