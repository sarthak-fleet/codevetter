//! Canonical, SQLite-only reads for a published archaeology catalog.
//!
//! Desktop IPC and MCP are transport adapters over this service. Keeping the
//! service dependent only on `rusqlite::Connection` makes normal reads
//! mechanically incapable of invoking Git, reading source files, using the
//! network, or calling a model.

use super::contracts::{
    ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyFreshness, ArchaeologyRuleKind,
    ArchaeologyRuleLifecycle, ArchaeologyTemporalSnapshotPayload, ArchaeologyTrust,
    ARCHAEOLOGY_SCHEMA_VERSION, ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
};
use super::inventory::git_head;
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::DbState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tauri::State;

pub(crate) const ARCHAEOLOGY_READ_CONTRACT_ID: &str =
    "codevetter.business-rule-archaeology.read.v1";
const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 500;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_QUERY_BYTES: usize = 512;
const MAX_QUERY_TOKENS: usize = 16;
const MAX_FILTER_VALUES: usize = 32;
const MAX_EVIDENCE_IDS: usize = 128;
const MAX_LANGUAGE_ROWS: usize = 64;
const MAX_ID_BYTES: usize = 256;
const MAX_CURSOR_BYTES: usize = 4096;
const UNAVAILABLE: &str = "Archaeology identity is unavailable in this repository";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ArchaeologyRuleFilter {
    pub query: Option<String>,
    pub kinds: Vec<ArchaeologyRuleKind>,
    pub trust: Vec<ArchaeologyTrust>,
    pub lifecycle: Vec<ArchaeologyRuleLifecycle>,
    pub domain_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ArchaeologySourceSelector {
    Path { path_identity: String },
    Unit { source_unit_id: String },
    Span { span_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyRelationKind {
    DependsOn,
    Precedes,
    Overrides,
    Aliases,
    ConflictsWith,
    Supersedes,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyRelationDirection {
    Incoming,
    Outgoing,
    #[default]
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyEvidenceKind {
    Fact,
    Span,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyEvidenceSelector {
    pub kind: ArchaeologyEvidenceKind,
    pub evidence_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ArchaeologyTemporalSelector {
    Generation { generation_id: String },
    Revision { revision_sha: String },
    Release { tag: String },
}

/// One strict transport-neutral request. `deny_unknown_fields` prevents an
/// MCP or IPC adapter from silently accepting a field it does not enforce.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ArchaeologyReadRequest {
    ListRules {
        repository_id: String,
        #[serde(default)]
        filter: ArchaeologyRuleFilter,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    ListDomains {
        repository_id: String,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    GetRule {
        repository_id: String,
        rule_id: String,
    },
    ReverseSource {
        repository_id: String,
        source: ArchaeologySourceSelector,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    ListRelations {
        repository_id: String,
        rule_id: String,
        #[serde(default)]
        kinds: Vec<ArchaeologyRelationKind>,
        #[serde(default)]
        direction: ArchaeologyRelationDirection,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    HydrateEvidence {
        repository_id: String,
        rule_id: String,
        evidence: Vec<ArchaeologyEvidenceSelector>,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    CompareTemporal {
        repository_id: String,
        before: ArchaeologyTemporalSelector,
        after: ArchaeologyTemporalSelector,
        limit: Option<usize>,
        cursor: Option<String>,
    },
}

impl ArchaeologyReadRequest {
    fn repository_id(&self) -> &str {
        match self {
            Self::ListRules { repository_id, .. }
            | Self::ListDomains { repository_id, .. }
            | Self::GetRule { repository_id, .. }
            | Self::ReverseSource { repository_id, .. }
            | Self::ListRelations { repository_id, .. }
            | Self::HydrateEvidence { repository_id, .. }
            | Self::CompareTemporal { repository_id, .. } => repository_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub(crate) enum ArchaeologyReadResponse {
    ListRules(Box<ArchaeologyPage<ArchaeologyRuleSummary>>),
    ListDomains(Box<ArchaeologyPage<ArchaeologyDomainSummary>>),
    GetRule(Box<ArchaeologyResult<ArchaeologyRuleDetail>>),
    ReverseSource(Box<ArchaeologyPage<ArchaeologyRuleSummary>>),
    ListRelations(Box<ArchaeologyPage<ArchaeologyRuleRelation>>),
    HydrateEvidence(Box<ArchaeologyPage<ArchaeologyEvidence>>),
    CompareTemporal(Box<ArchaeologyResult<ArchaeologyTemporalComparison>>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyReadBounds {
    pub max_page_rows: usize,
    pub max_response_bytes: usize,
    pub max_evidence_ids: usize,
    pub max_query_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyLanguageCoverage {
    pub language: String,
    pub dialect: Option<String>,
    pub classification: String,
    pub source_units: u64,
    pub indexed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyReadContext {
    pub schema_version: u32,
    pub contract_id: String,
    pub repository_id: String,
    pub generation_id: String,
    pub revision_sha: String,
    pub published_at: Option<String>,
    pub parser_identity: String,
    pub algorithm_identity: String,
    pub config_identity: String,
    pub coverage: ArchaeologyCoverage,
    pub freshness: ArchaeologyFreshness,
    pub language_coverage: Vec<ArchaeologyLanguageCoverage>,
    pub omitted_language_rows: u64,
    pub bounds: ArchaeologyReadBounds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyPageInfo {
    pub applied_limit: usize,
    pub returned_rows: usize,
    pub total_rows: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyPage<T> {
    pub context: ArchaeologyReadContext,
    pub items: Vec<T>,
    pub page: ArchaeologyPageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyResult<T> {
    pub context: ArchaeologyReadContext,
    pub value: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleSummary {
    /// Stable logical identity, never the generation-local occurrence ID.
    pub rule_id: String,
    pub title: String,
    pub kind: ArchaeologyRuleKind,
    pub lifecycle: ArchaeologyRuleLifecycle,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    pub domain_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleClauseDetail {
    pub clause_id: String,
    pub ordinal: u64,
    pub text: String,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    pub caveats: Vec<String>,
    pub supporting_fact_ids: Vec<String>,
    pub contradicting_fact_ids: Vec<String>,
    pub evidence_span_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleDetail {
    #[serde(flatten)]
    pub summary: ArchaeologyRuleSummary,
    pub revision_sha: String,
    pub evidence_identity: String,
    pub contradiction_identity: String,
    pub description_identity: String,
    pub continuity_identity: String,
    pub parser_compatibility_identity: String,
    pub parser_identity: String,
    pub algorithm_identity: String,
    pub synthesis_identity: Option<String>,
    pub clauses: Vec<ArchaeologyRuleClauseDetail>,
    pub alias_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyDomainSummary {
    pub domain_id: String,
    pub label: String,
    pub parent_domain_id: Option<String>,
    pub rule_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleRelation {
    pub relation_id: String,
    pub direction: ArchaeologyRelationDirection,
    pub kind: ArchaeologyRelationKind,
    pub rule_id: String,
    pub trust: ArchaeologyTrust,
    pub summary: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyEvidenceSource {
    pub source_id: String,
    pub source_unit_id: String,
    pub relative_path: Option<String>,
    pub language: String,
    pub dialect: Option<String>,
    pub classification: String,
    pub revision_sha: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u64,
    pub start_column: u64,
    pub end_line: u64,
    pub end_column: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ArchaeologyEvidence {
    Fact {
        evidence_id: String,
        fact_kind: String,
        label: String,
        trust: ArchaeologyTrust,
        confidence: ArchaeologyConfidence,
        span_ids: Vec<String>,
    },
    Span {
        evidence_id: String,
        source: ArchaeologyEvidenceSource,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalComparison {
    pub before: ArchaeologyTemporalPoint,
    pub after: ArchaeologyTemporalPoint,
    pub coverage: String,
    pub reasons: Vec<String>,
    pub changes: Vec<ArchaeologyTemporalChange>,
    pub page: ArchaeologyPageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalPoint {
    pub selector: ArchaeologyTemporalSelector,
    pub temporal_generation_id: String,
    pub generation_id: String,
    pub revision_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalSnapshot {
    pub snapshot_id: String,
    pub stable_rule_id: String,
    pub continuity_id: String,
    pub kind: ArchaeologyRuleKind,
    pub evidence_identity: String,
    pub parser_compatibility_identity: String,
    pub contradiction_identity: String,
    pub description_identity: String,
    pub payload: ArchaeologyTemporalSnapshotPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalChange {
    pub event_id: String,
    pub classification: String,
    pub stable_rule_id: String,
    pub continuity_id: String,
    pub predecessor_rule_id: Option<String>,
    pub successor_rule_id: Option<String>,
    pub coverage: String,
    pub reasons: Vec<String>,
    pub before: Option<ArchaeologyTemporalSnapshot>,
    pub after: Option<ArchaeologyTemporalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorPayload {
    version: u8,
    repository_id: String,
    generation_id: String,
    operation: String,
    query_identity: String,
    primary: String,
    secondary: String,
}

#[derive(Debug, Clone)]
struct ReadyScope {
    repository_id: String,
    repo_path: String,
    generation_id: String,
    context: ArchaeologyReadContext,
}

#[derive(Debug)]
struct PageRow<T> {
    item: T,
    primary: String,
    secondary: String,
}

type RawRuleSummaryRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

pub(crate) struct ArchaeologyReadService<'a> {
    connection: &'a Connection,
    current_head: Option<String>,
    response_byte_limit: usize,
    #[cfg(test)]
    hydration_query_count: Cell<usize>,
}

/// The desktop transport is intentionally one tagged command over the same
/// SQLite-only service used by every other adapter.
#[tauri::command]
pub async fn read_business_rule_archaeology(
    db: State<'_, DbState>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let request: ArchaeologyReadRequest = serde_json::from_value(request)
        .map_err(|_| "Invalid archaeology read request".to_string())?;
    let repository_id = request.repository_id().to_string();
    let database = Arc::clone(&db.0);
    let response = tokio::task::spawn_blocking(move || {
        let repo_path: String = {
            let connection = database
                .lock()
                .map_err(|_| "Archaeology database is unavailable".to_string())?;
            connection
                .query_row(
                    "SELECT repo_path FROM archaeology_repositories WHERE repository_id=?1",
                    [&repository_id],
                    |row| row.get(0),
                )
                .map_err(|_| "Business-rule archaeology catalog is unavailable".to_string())?
        };
        let canonical = std::fs::canonicalize(&repo_path)
            .map_err(|_| "Business-rule archaeology repository is unavailable".to_string())?;
        let current_head = git_head(&canonical)?;
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        read_business_rule_archaeology_core_with_current_head(&connection, request, current_head)
    })
    .await
    .map_err(|error| format!("Archaeology read worker failed: {error}"))??;
    serde_json::to_value(response).map_err(|_| "Archaeology response is unavailable".to_string())
}

fn read_business_rule_archaeology_core(
    connection: &Connection,
    request: ArchaeologyReadRequest,
) -> Result<ArchaeologyReadResponse, String> {
    ArchaeologyReadService::new(connection).execute(request)
}

fn read_business_rule_archaeology_core_with_current_head(
    connection: &Connection,
    request: ArchaeologyReadRequest,
    current_head: String,
) -> Result<ArchaeologyReadResponse, String> {
    ArchaeologyReadService::new_with_current_head(connection, current_head).execute(request)
}

impl<'a> ArchaeologyReadService<'a> {
    pub(crate) fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            current_head: None,
            response_byte_limit: MAX_RESPONSE_BYTES,
            #[cfg(test)]
            hydration_query_count: Cell::new(0),
        }
    }

    pub(crate) fn new_with_current_head(connection: &'a Connection, current_head: String) -> Self {
        Self {
            connection,
            current_head: Some(current_head),
            response_byte_limit: MAX_RESPONSE_BYTES,
            #[cfg(test)]
            hydration_query_count: Cell::new(0),
        }
    }

    pub(crate) fn with_response_byte_limit(mut self, limit: usize) -> Self {
        self.response_byte_limit = limit.clamp(1, MAX_RESPONSE_BYTES);
        self
    }

    #[cfg(test)]
    fn hydration_query_count(&self) -> usize {
        self.hydration_query_count.get()
    }

    #[cfg(test)]
    fn record_hydration_query(&self) {
        self.hydration_query_count
            .set(self.hydration_query_count.get() + 1);
    }

    #[cfg(not(test))]
    fn record_hydration_query(&self) {}

    pub(crate) fn execute(
        &self,
        request: ArchaeologyReadRequest,
    ) -> Result<ArchaeologyReadResponse, String> {
        validate_id("repository", request.repository_id())?;
        let scope = self.ready_scope(request.repository_id())?;
        match request {
            ArchaeologyReadRequest::ListRules {
                filter,
                limit,
                cursor,
                ..
            } => self
                .list_rules(&scope, filter, limit, cursor.as_deref())
                .map(Box::new)
                .map(ArchaeologyReadResponse::ListRules),
            ArchaeologyReadRequest::ListDomains { limit, cursor, .. } => self
                .list_domains(&scope, limit, cursor.as_deref())
                .map(Box::new)
                .map(ArchaeologyReadResponse::ListDomains),
            ArchaeologyReadRequest::GetRule { rule_id, .. } => self
                .get_rule(&scope, &rule_id)
                .map(|value| ArchaeologyReadResponse::GetRule(Box::new(result(&scope, value)))),
            ArchaeologyReadRequest::ReverseSource {
                source,
                limit,
                cursor,
                ..
            } => self
                .reverse_source(&scope, source, limit, cursor.as_deref())
                .map(Box::new)
                .map(ArchaeologyReadResponse::ReverseSource),
            ArchaeologyReadRequest::ListRelations {
                rule_id,
                kinds,
                direction,
                limit,
                cursor,
                ..
            } => self
                .list_relations(&scope, &rule_id, kinds, direction, limit, cursor.as_deref())
                .map(Box::new)
                .map(ArchaeologyReadResponse::ListRelations),
            ArchaeologyReadRequest::HydrateEvidence {
                rule_id,
                evidence,
                limit,
                cursor,
                ..
            } => self
                .hydrate_evidence(&scope, &rule_id, evidence, limit, cursor.as_deref())
                .map(Box::new)
                .map(ArchaeologyReadResponse::HydrateEvidence),
            ArchaeologyReadRequest::CompareTemporal {
                before,
                after,
                limit,
                cursor,
                ..
            } => self
                .compare_temporal(&scope, before, after, limit, cursor.as_deref())
                .map(|value| {
                    ArchaeologyReadResponse::CompareTemporal(Box::new(result(&scope, value)))
                }),
        }
    }

    fn ready_scope(&self, repository_id: &str) -> Result<ReadyScope, String> {
        let row = self
            .connection
            .query_row(
                "SELECT repository.repo_path, repository.ready_generation_id, repository.current_revision,
                        repository.source_identity, generation.revision_sha,
                        generation.source_identity, generation.parser_identity,
                        generation.algorithm_identity, generation.config_identity,
                        generation.coverage_json, generation.published_at
                 FROM archaeology_repositories repository
                 JOIN archaeology_generations generation
                   ON generation.generation_id=repository.ready_generation_id
                  AND generation.repository_id=repository.repository_id
                 WHERE repository.repository_id=?1 AND generation.status='ready'
                   AND generation.schema_version=?2",
                (repository_id, i64::from(ARCHAEOLOGY_STORAGE_SCHEMA_VERSION)),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Load archaeology ready catalog: {error}"))?
            .ok_or_else(|| UNAVAILABLE.to_string())?;
        let (
            repo_path,
            generation_id,
            current_revision,
            current_source,
            revision_sha,
            indexed_source,
            parser_identity,
            algorithm_identity,
            config_identity,
            coverage_json,
            published_at,
        ) = row;
        for value in [
            generation_id.as_str(),
            revision_sha.as_str(),
            indexed_source.as_str(),
            parser_identity.as_str(),
            algorithm_identity.as_str(),
            config_identity.as_str(),
        ] {
            validate_id("ready catalog identity", value)?;
        }
        let coverage: ArchaeologyCoverage = parse_json(&coverage_json, "catalog coverage")?;
        validate_coverage(&coverage)?;
        let current_inputs = self.current_input_identities(
            repository_id,
            &generation_id,
            &current_revision,
            &current_source,
        )?;
        let current_parser_identity = current_inputs.as_ref().map(|value| value.0.clone());
        let current_config_identity = current_inputs.as_ref().map(|value| value.1.clone());
        let parser_changed = current_parser_identity
            .as_ref()
            .is_some_and(|current| current != &parser_identity);
        let config_changed = current_config_identity
            .as_ref()
            .is_some_and(|current| current != &config_identity);
        let observed_revision = self.current_head.as_deref().unwrap_or(&current_revision);
        let stale = observed_revision != revision_sha
            || current_source != indexed_source
            || parser_changed
            || config_changed;
        let mut reasons = Vec::new();
        if observed_revision != revision_sha {
            reasons.push("repository_revision_changed".into());
        }
        if current_source != indexed_source {
            reasons.push("repository_source_identity_changed".into());
        }
        if parser_changed {
            reasons.push("parser_identity_changed".into());
        }
        if config_changed {
            reasons.push("config_identity_changed".into());
        }
        let human_review_decisions_present = self
            .connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM archaeology_rule_review_events review
                   JOIN archaeology_rules rule
                     ON rule.generation_id=?2
                    AND (rule.stable_rule_identity=review.stable_rule_identity
                         OR rule.continuity_identity=review.continuity_identity)
                   WHERE review.repository_id=?1
                     AND review.event_schema_version=2 AND review.legacy_stale=0
                     AND review.actor_kind='human'
                     AND review.decision IN ('accepted','rejected','superseded','conflicted')
                 )",
                (repository_id, generation_id.as_str()),
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("Read archaeology human review freshness: {error}"))?;
        let human_review_decisions_stale = stale && human_review_decisions_present;
        let human_review_stale_reasons = if human_review_decisions_stale {
            reasons.clone()
        } else {
            Vec::new()
        };
        let (language_coverage, omitted_language_rows) = self.language_coverage(&generation_id)?;
        let context = ArchaeologyReadContext {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            contract_id: ARCHAEOLOGY_READ_CONTRACT_ID.into(),
            repository_id: repository_id.into(),
            generation_id: generation_id.clone(),
            revision_sha: revision_sha.clone(),
            published_at,
            parser_identity: parser_identity.clone(),
            algorithm_identity,
            config_identity: config_identity.clone(),
            coverage,
            freshness: ArchaeologyFreshness {
                indexed_revision: Some(revision_sha),
                current_revision: Some(current_revision),
                parser_identity: Some(parser_identity),
                current_parser_identity,
                config_identity: Some(config_identity),
                current_config_identity,
                stale,
                reasons,
                human_review_decisions_present,
                human_review_decisions_stale,
                human_review_stale_reasons,
            },
            language_coverage,
            omitted_language_rows,
            bounds: ArchaeologyReadBounds {
                max_page_rows: MAX_PAGE_LIMIT,
                max_response_bytes: self.response_byte_limit,
                max_evidence_ids: MAX_EVIDENCE_IDS,
                max_query_bytes: MAX_QUERY_BYTES,
            },
        };
        Ok(ReadyScope {
            repository_id: repository_id.into(),
            repo_path,
            generation_id,
            context,
        })
    }

    fn language_coverage(
        &self,
        generation_id: &str,
    ) -> Result<(Vec<ArchaeologyLanguageCoverage>, u64), String> {
        let total_groups = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM (
                   SELECT 1 FROM archaeology_source_units WHERE generation_id=?1
                   GROUP BY language,dialect,classification
                 )",
                [generation_id],
                |row| row.get::<_, u64>(0),
            )
            .map_err(|error| format!("Count archaeology language coverage: {error}"))?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT language,dialect,classification,COUNT(*),COALESCE(SUM(byte_count),0)
                 FROM archaeology_source_units WHERE generation_id=?1
                 GROUP BY language,dialect,classification
                 ORDER BY language,dialect,classification LIMIT ?2",
            )
            .map_err(|error| format!("Prepare archaeology language coverage: {error}"))?;
        let rows = statement
            .query_map((generation_id, MAX_LANGUAGE_ROWS as i64), |row| {
                Ok(ArchaeologyLanguageCoverage {
                    language: row.get(0)?,
                    dialect: row.get(1)?,
                    classification: row.get(2)?,
                    source_units: row.get::<_, u64>(3)?,
                    indexed_bytes: row.get::<_, u64>(4)?,
                })
            })
            .map_err(|error| format!("Query archaeology language coverage: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology language coverage: {error}"))?;
        let omitted = total_groups.saturating_sub(rows.len() as u64);
        for row in &rows {
            safe_public_text(&row.language, 128)?;
            if let Some(dialect) = row.dialect.as_deref() {
                safe_public_text(dialect, 128)?;
            }
        }
        Ok((rows, omitted))
    }

    fn list_rules(
        &self,
        scope: &ReadyScope,
        mut filter: ArchaeologyRuleFilter,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyPage<ArchaeologyRuleSummary>, String> {
        normalize_filter(&mut filter)?;
        let applied_limit = bounded_limit(limit);
        let query_identity = query_identity("list_rules", &(filter.clone(), applied_limit))?;
        let after = self.decode_cursor(scope, "list_rules", &query_identity, cursor)?;
        let (where_sql, mut values, fts) = rule_predicates(scope, &filter)?;
        let base_values = values.clone();
        values.push(scope.generation_id.clone().into());
        let mut after_sql = String::new();
        if let Some(cursor) = after {
            after_sql = " AND rule.rule_id>?".into();
            values.push(cursor.primary.into());
        }
        let from_sql = rule_list_from_sql(fts);
        let sql = rule_list_sql(from_sql, &where_sql, &after_sql);
        values.push(((applied_limit + 1) as i64).into());
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Prepare archaeology rule list: {error}"))?;
        let counted = statement
            .query_map(params_from_iter(values), |row| {
                Ok((decode_raw_rule_summary_row(row)?, row.get::<_, u64>(8)?))
            })
            .map_err(|error| format!("Query archaeology rule list: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology rule list: {error}"))?;
        let total_rows = match counted.first() {
            Some(row) => row.1,
            None => query_count(
                self.connection,
                &format!("SELECT COUNT(*) {from_sql} WHERE {where_sql}"),
                &base_values,
            )?,
        };
        let rows = counted
            .into_iter()
            .map(|(raw, _)| rule_summary_page_row(raw))
            .collect::<Result<Vec<_>, String>>()?;
        finish_page(
            scope,
            "list_rules",
            &query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }

    fn list_domains(
        &self,
        scope: &ReadyScope,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyPage<ArchaeologyDomainSummary>, String> {
        let applied_limit = bounded_limit(limit);
        let query_identity = query_identity("list_domains", &applied_limit)?;
        let after = self.decode_cursor(scope, "list_domains", &query_identity, cursor)?;
        let after_id = after.as_ref().map(|cursor| cursor.primary.as_str());
        let total_rows = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM (SELECT DISTINCT domain.domain_id
                 FROM archaeology_rule_domains domain JOIN archaeology_rules rule
                   ON rule.generation_id=domain.generation_id AND rule.rule_id=domain.rule_id
                 WHERE domain.generation_id=?1 AND rule.identity_schema_version=2
                   AND NOT EXISTS (SELECT 1 FROM archaeology_rule_relations alias
                     WHERE alias.generation_id=rule.generation_id
                       AND alias.from_rule_id=rule.rule_id AND alias.kind='aliases'))",
                [scope.generation_id.as_str()],
                |row| row.get::<_, u64>(0),
            )
            .map_err(|error| format!("Count archaeology domains: {error}"))?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT domain.domain_id,MIN(domain.domain_label),MIN(domain.parent_domain_id),
                        COUNT(DISTINCT rule.rule_id)
                 FROM archaeology_rule_domains domain JOIN archaeology_rules rule
                   ON rule.generation_id=domain.generation_id AND rule.rule_id=domain.rule_id
                 WHERE domain.generation_id=?1 AND rule.identity_schema_version=2
                   AND (?2 IS NULL OR domain.domain_id>?2)
                   AND NOT EXISTS (SELECT 1 FROM archaeology_rule_relations alias
                     WHERE alias.generation_id=rule.generation_id
                       AND alias.from_rule_id=rule.rule_id AND alias.kind='aliases')
                 GROUP BY domain.domain_id ORDER BY domain.domain_id LIMIT ?3",
            )
            .map_err(|error| format!("Prepare archaeology domains: {error}"))?;
        let raw = statement
            .query_map(
                (
                    scope.generation_id.as_str(),
                    after_id,
                    (applied_limit + 1) as i64,
                ),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("Query archaeology domains: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology domains: {error}"))?;
        let rows = raw
            .into_iter()
            .map(|(domain_id, label, parent_domain_id, rule_count)| {
                validate_id("domain", &domain_id)?;
                safe_public_text(&label, 1024)?;
                if let Some(parent) = parent_domain_id.as_deref() {
                    validate_id("parent domain", parent)?;
                }
                Ok(PageRow {
                    primary: domain_id.clone(),
                    secondary: String::new(),
                    item: ArchaeologyDomainSummary {
                        domain_id,
                        label,
                        parent_domain_id,
                        rule_count,
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        finish_page(
            scope,
            "list_domains",
            &query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }

    fn get_rule(
        &self,
        scope: &ReadyScope,
        stable_rule_identity: &str,
    ) -> Result<ArchaeologyRuleDetail, String> {
        validate_digest_id("rule", stable_rule_identity)?;
        let raw = self.canonical_rule(scope, stable_rule_identity)?;
        let summary = self.rule_summary(scope, &raw.0)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT clause.clause_id,clause.ordinal,clause.clause_text,clause.trust,
                        clause.confidence,clause.caveats_json,
                        COALESCE((SELECT json_group_array(item.evidence_id) FROM (
                          SELECT evidence.evidence_id FROM archaeology_evidence_links evidence
                          WHERE evidence.generation_id=clause.generation_id
                            AND evidence.owner_kind='rule_clause'
                            AND evidence.owner_id=clause.clause_id
                            AND evidence.evidence_kind='fact' AND evidence.role='supporting'
                          ORDER BY evidence.evidence_id) item),'[]'),
                        COALESCE((SELECT json_group_array(item.evidence_id) FROM (
                          SELECT evidence.evidence_id FROM archaeology_evidence_links evidence
                          WHERE evidence.generation_id=clause.generation_id
                            AND evidence.owner_kind='rule_clause'
                            AND evidence.owner_id=clause.clause_id
                            AND evidence.evidence_kind='fact' AND evidence.role='contradicting'
                          ORDER BY evidence.evidence_id) item),'[]'),
                        COALESCE((SELECT json_group_array(item.evidence_id) FROM (
                          SELECT evidence.evidence_id FROM archaeology_evidence_links evidence
                          WHERE evidence.generation_id=clause.generation_id
                            AND evidence.owner_kind='rule_clause'
                            AND evidence.owner_id=clause.clause_id
                            AND evidence.evidence_kind='span'
                          ORDER BY evidence.evidence_id) item),'[]')
                 FROM archaeology_rule_clauses clause
                 WHERE clause.generation_id=?1 AND clause.rule_id=?2
                 ORDER BY clause.ordinal,clause.clause_id LIMIT 257",
            )
            .map_err(|error| format!("Prepare archaeology rule clauses: {error}"))?;
        let clauses = statement
            .query_map((scope.generation_id.as_str(), raw.0.as_str()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|error| format!("Query archaeology rule clauses: {error}"))?
            .map(|row| {
                let (id, ordinal, text, trust, confidence, caveats, support, conflict, spans) =
                    row.map_err(|error| format!("Read archaeology rule clause: {error}"))?;
                safe_public_text(&text, 64 * 1024)?;
                Ok(ArchaeologyRuleClauseDetail {
                    clause_id: id,
                    ordinal,
                    text,
                    trust: parse_enum(&trust, "clause trust")?,
                    confidence: parse_enum(&confidence, "clause confidence")?,
                    caveats: parse_safe_strings(&caveats, "clause caveats", 1024)?,
                    supporting_fact_ids: parse_ids(&support, "supporting facts")?,
                    contradicting_fact_ids: parse_ids(&conflict, "contradicting facts")?,
                    evidence_span_ids: parse_ids(&spans, "evidence spans")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if clauses.is_empty() || clauses.len() > 256 {
            return Err("Archaeology rule detail exceeds its clause bound".into());
        }
        let aliases = self.aliases(scope, &raw.0)?;
        let detail = ArchaeologyRuleDetail {
            summary,
            revision_sha: raw.1,
            evidence_identity: raw.2,
            contradiction_identity: raw.3,
            description_identity: raw.4,
            continuity_identity: raw.5,
            parser_compatibility_identity: raw.6,
            parser_identity: raw.7,
            algorithm_identity: raw.8,
            synthesis_identity: raw.9,
            clauses,
            alias_rule_ids: aliases,
        };
        if serialized_bytes(&detail)? > scope.context.bounds.max_response_bytes {
            return Err("Archaeology rule detail exceeds the response byte bound".into());
        }
        Ok(detail)
    }

    /// Returns the canonical occurrence ID and immutable identity fields.
    #[allow(clippy::type_complexity)]
    fn canonical_rule(
        &self,
        scope: &ReadyScope,
        stable_rule_identity: &str,
    ) -> Result<
        (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
        ),
        String,
    > {
        let mut statement = self
            .connection
            .prepare(
                "SELECT rule.rule_id,rule.revision_sha,rule.evidence_identity,
                        rule.contradiction_identity,rule.description_identity,
                        rule.continuity_identity,rule.parser_compatibility_identity,
                        rule.parser_identity,rule.algorithm_identity,rule.synthesis_identity
                 FROM archaeology_rules rule
                 WHERE rule.generation_id=?1 AND rule.repository_id=?2
                   AND rule.identity_schema_version=2 AND rule.stable_rule_identity=?3
                   AND NOT EXISTS (SELECT 1 FROM archaeology_rule_relations alias
                     WHERE alias.generation_id=rule.generation_id
                       AND alias.from_rule_id=rule.rule_id AND alias.kind='aliases')
                 ORDER BY rule.rule_id LIMIT 2",
            )
            .map_err(|error| format!("Prepare archaeology rule lookup: {error}"))?;
        let rows = statement
            .query_map(
                (
                    scope.generation_id.as_str(),
                    scope.repository_id.as_str(),
                    stable_rule_identity,
                ),
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .map_err(|error| format!("Query archaeology rule lookup: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology rule lookup: {error}"))?;
        if rows.len() != 1 {
            return Err(UNAVAILABLE.into());
        }
        Ok(rows.into_iter().next().expect("one canonical rule"))
    }

    fn rule_summary(
        &self,
        scope: &ReadyScope,
        occurrence_id: &str,
    ) -> Result<ArchaeologyRuleSummary, String> {
        let sql = format!(
            "SELECT rule.stable_rule_identity,manifest.title,rule.kind,{lifecycle},
                    rule.trust,rule.confidence,
                    COALESCE((SELECT json_group_array(item.domain_id) FROM (
                      SELECT domain.domain_id FROM archaeology_rule_domains domain
                      WHERE domain.generation_id=rule.generation_id AND domain.rule_id=rule.rule_id
                      ORDER BY domain.domain_id) item),'[]')
             FROM archaeology_rules rule JOIN archaeology_rule_search_manifest manifest
               ON manifest.generation_id=rule.generation_id AND manifest.rule_id=rule.rule_id
             WHERE rule.generation_id=?1 AND rule.rule_id=?2",
            lifecycle = effective_lifecycle_sql()
        );
        self.connection
            .query_row(&sql, (scope.generation_id.as_str(), occurrence_id), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|_| UNAVAILABLE.to_string())
            .and_then(
                |(rule_id, title, kind, lifecycle, trust, confidence, domains)| {
                    safe_public_text(&title, 16 * 1024)?;
                    Ok(ArchaeologyRuleSummary {
                        rule_id,
                        title,
                        kind: parse_enum(&kind, "rule kind")?,
                        lifecycle: parse_enum(&lifecycle, "rule lifecycle")?,
                        trust: parse_enum(&trust, "rule trust")?,
                        confidence: parse_enum(&confidence, "rule confidence")?,
                        domain_ids: parse_ids(&domains, "rule domains")?,
                    })
                },
            )
    }

    fn aliases(
        &self,
        scope: &ReadyScope,
        canonical_occurrence: &str,
    ) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT DISTINCT alias_rule.stable_rule_identity
                 FROM archaeology_rule_relations relation JOIN archaeology_rules alias_rule
                   ON alias_rule.generation_id=relation.generation_id
                  AND alias_rule.rule_id=relation.from_rule_id
                 WHERE relation.generation_id=?1 AND relation.to_rule_id=?2
                   AND relation.kind='aliases' ORDER BY alias_rule.stable_rule_identity LIMIT 501",
            )
            .map_err(|error| format!("Prepare archaeology aliases: {error}"))?;
        let mut values = statement
            .query_map(
                (scope.generation_id.as_str(), canonical_occurrence),
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("Query archaeology aliases: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology aliases: {error}"))?;
        if values.len() > 500 {
            return Err("Archaeology rule alias detail exceeds its bound".into());
        }
        values.dedup();
        Ok(values)
    }

    fn reverse_source(
        &self,
        scope: &ReadyScope,
        source: ArchaeologySourceSelector,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyPage<ArchaeologyRuleSummary>, String> {
        let span_predicate = self.source_span_predicate(scope, &source)?;
        let applied_limit = bounded_limit(limit);
        let query_identity = query_identity("reverse_source", &(source, applied_limit))?;
        let after = self.decode_cursor(scope, "reverse_source", &query_identity, cursor)?;
        let mut values = span_predicate.1;
        let rule_cte = reverse_rule_cte(&span_predicate.0);
        let from_sql = "FROM matched occurrence
             CROSS JOIN archaeology_rules candidate
               ON candidate.generation_id=? AND candidate.rule_id=occurrence.rule_id
             LEFT JOIN archaeology_rule_relations alias
               ON alias.generation_id=candidate.generation_id
              AND alias.from_rule_id=candidate.rule_id AND alias.kind='aliases'
             CROSS JOIN archaeology_rules canonical
               ON canonical.generation_id=candidate.generation_id
              AND canonical.rule_id=COALESCE(alias.to_rule_id,candidate.rule_id)
             CROSS JOIN archaeology_rule_search_manifest manifest
              ON manifest.generation_id=canonical.generation_id
              AND manifest.rule_id=canonical.rule_id
             WHERE canonical.repository_id=? AND canonical.identity_schema_version=2";
        // CTE parameters first, then generation and repository.
        values.push(scope.generation_id.clone().into());
        values.push(scope.repository_id.clone().into());
        let base_values = values.clone();
        values.push(scope.generation_id.clone().into());
        let after_sql = if let Some(after) = after {
            values.push(after.primary.into());
            " WHERE canonical.rule_id>?"
        } else {
            ""
        };
        let lifecycle = effective_lifecycle_sql().replace("rule.", "canonical.");
        let sql = format!(
            "{rule_cte}, canonical_matches(rule_id,total_rows) AS MATERIALIZED (
               SELECT canonical.rule_id,COUNT(*) OVER()
               {from_sql}
               GROUP BY canonical.rule_id
             )
             SELECT canonical.rule_id,canonical.stable_rule_identity,manifest.title,
                    canonical.kind,{lifecycle},canonical.trust,canonical.confidence,
                    COALESCE((SELECT json_group_array(item.domain_id) FROM (
                      SELECT domain.domain_id FROM archaeology_rule_domains domain
                      WHERE domain.generation_id=canonical.generation_id
                        AND domain.rule_id=canonical.rule_id ORDER BY domain.domain_id) item),'[]'),
                    matched.total_rows
             FROM canonical_matches matched
             CROSS JOIN archaeology_rules canonical
               ON canonical.generation_id=? AND canonical.rule_id=matched.rule_id
             CROSS JOIN archaeology_rule_search_manifest manifest
               ON manifest.generation_id=canonical.generation_id
              AND manifest.rule_id=canonical.rule_id
             {after_sql}
             ORDER BY canonical.rule_id LIMIT ?"
        );
        values.push(((applied_limit + 1) as i64).into());
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Prepare archaeology source reverse lookup: {error}"))?;
        let counted = statement
            .query_map(params_from_iter(values), |row| {
                Ok((decode_raw_rule_summary_row(row)?, row.get::<_, u64>(8)?))
            })
            .map_err(|error| format!("Query archaeology source reverse lookup: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology source reverse lookup: {error}"))?;
        // A counted row avoids evaluating the reverse-evidence CTE twice on
        // ordinary pages. Only an empty cursor page needs the separate count
        // to preserve the response's catalog-wide total.
        let total_rows = match counted.first() {
            Some(row) => row.1,
            None => query_count(
                self.connection,
                &format!("{rule_cte} SELECT COUNT(DISTINCT canonical.rule_id) {from_sql}"),
                &base_values,
            )?,
        };
        let rows = counted
            .into_iter()
            .map(|(raw, _)| rule_summary_page_row(raw))
            .collect::<Result<Vec<_>, String>>()?;
        finish_page(
            scope,
            "reverse_source",
            &query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }

    fn source_span_predicate(
        &self,
        scope: &ReadyScope,
        selector: &ArchaeologySourceSelector,
    ) -> Result<(String, Vec<SqlValue>), String> {
        let (predicate, identity) = match selector {
            ArchaeologySourceSelector::Path { path_identity } => {
                validate_id("source path", path_identity)?;
                ("unit.path_identity=?", path_identity)
            }
            ArchaeologySourceSelector::Unit { source_unit_id } => {
                validate_id("source unit", source_unit_id)?;
                ("unit.source_unit_id=?", source_unit_id)
            }
            ArchaeologySourceSelector::Span { span_id } => {
                validate_id("source span", span_id)?;
                ("span.span_id=?", span_id)
            }
        };
        let row = self
            .connection
            .query_row(
                &format!(
                    "SELECT unit.classification,unit.relative_path
                     FROM archaeology_source_units unit JOIN archaeology_source_spans span
                       ON span.generation_id=unit.generation_id
                      AND span.source_unit_id=unit.source_unit_id
                     WHERE unit.generation_id=?1 AND {predicate} LIMIT 1"
                ),
                (scope.generation_id.as_str(), identity.as_str()),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| format!("Resolve archaeology source identity: {error}"))?
            .ok_or_else(|| UNAVAILABLE.to_string())?;
        if matches!(row.0.as_str(), "protected" | "opaque") {
            return Err(UNAVAILABLE.into());
        }
        let path = row.1.ok_or_else(|| UNAVAILABLE.to_string())?;
        safe_relative_path(&path)?;
        Ok((
            predicate.replace('?', "?2"),
            vec![scope.generation_id.clone().into(), identity.clone().into()],
        ))
    }

    fn list_relations(
        &self,
        scope: &ReadyScope,
        rule_id: &str,
        mut kinds: Vec<ArchaeologyRelationKind>,
        direction: ArchaeologyRelationDirection,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyPage<ArchaeologyRuleRelation>, String> {
        validate_digest_id("rule", rule_id)?;
        let canonical = self.canonical_rule(scope, rule_id)?.0;
        if kinds.len() > MAX_FILTER_VALUES {
            return Err("Archaeology relation kind bound exceeded".into());
        }
        kinds.sort_by_key(relation_kind_name);
        kinds.dedup();
        let applied_limit = bounded_limit(limit);
        let query_identity = query_identity(
            "list_relations",
            &(rule_id, &kinds, &direction, applied_limit),
        )?;
        let after = self.decode_cursor(scope, "list_relations", &query_identity, cursor)?;
        let mut predicates = vec!["relation.generation_id=?".to_string()];
        let mut values = vec![scope.generation_id.clone().into()];
        match direction {
            ArchaeologyRelationDirection::Incoming => {
                predicates.push("relation.to_rule_id=?".into());
                values.push(canonical.clone().into());
            }
            ArchaeologyRelationDirection::Outgoing => {
                predicates.push("relation.from_rule_id=?".into());
                values.push(canonical.clone().into());
            }
            ArchaeologyRelationDirection::Both => {
                predicates.push("(relation.from_rule_id=? OR relation.to_rule_id=?)".into());
                values.push(canonical.clone().into());
                values.push(canonical.clone().into());
            }
        }
        if !kinds.is_empty() {
            predicates.push(format!("relation.kind IN ({})", placeholders(kinds.len())));
            values.extend(
                kinds
                    .iter()
                    .map(|kind| relation_kind_name(kind).to_string().into()),
            );
        }
        let base_predicates = predicates.clone();
        let base_values = values.clone();
        if let Some(after) = after {
            predicates
                .push("(relation.kind>? OR (relation.kind=? AND relation.relation_id>?))".into());
            values.push(after.primary.clone().into());
            values.push(after.primary.into());
            values.push(after.secondary.into());
        }
        let where_sql = predicates.join(" AND ");
        let total_rows = query_count(
            self.connection,
            &format!(
                "SELECT COUNT(*) FROM archaeology_rule_relations relation WHERE {}",
                base_predicates.join(" AND ")
            ),
            &base_values,
        )?;
        let sql = format!(
            "SELECT relation.relation_id,relation.kind,relation.from_rule_id,
                    relation.to_rule_id,relation.trust,relation.summary,
                    COALESCE((SELECT json_group_array(item.evidence_id) FROM (
                      SELECT evidence.evidence_id FROM archaeology_evidence_links evidence
                      WHERE evidence.generation_id=relation.generation_id
                        AND evidence.owner_kind='rule_relation'
                        AND evidence.owner_id=relation.relation_id
                      ORDER BY evidence.evidence_id) item),'[]'),
                    COALESCE(alias.to_rule_id,target.rule_id),canonical_target.stable_rule_identity
             FROM archaeology_rule_relations relation
             JOIN archaeology_rules target ON target.generation_id=relation.generation_id
               AND target.rule_id=CASE WHEN relation.from_rule_id=? THEN relation.to_rule_id
                                       ELSE relation.from_rule_id END
             LEFT JOIN archaeology_rule_relations alias ON alias.generation_id=target.generation_id
               AND alias.from_rule_id=target.rule_id AND alias.kind='aliases'
             JOIN archaeology_rules canonical_target ON canonical_target.generation_id=target.generation_id
               AND canonical_target.rule_id=COALESCE(alias.to_rule_id,target.rule_id)
             WHERE {where_sql} ORDER BY relation.kind,relation.relation_id LIMIT ?"
        );
        values.insert(0, canonical.clone().into());
        values.push(((applied_limit + 1) as i64).into());
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| format!("Prepare archaeology relations: {error}"))?;
        let raw = statement
            .query_map(params_from_iter(values), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|error| format!("Query archaeology relations: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology relations: {error}"))?;
        let rows = raw
            .into_iter()
            .map(
                |(relation_id, kind, from, _to, trust, summary, evidence, target_rule)| {
                    if let Some(summary) = summary.as_deref() {
                        safe_public_text(summary, 4096)?;
                    }
                    Ok(PageRow {
                        primary: kind.clone(),
                        secondary: relation_id.clone(),
                        item: ArchaeologyRuleRelation {
                            relation_id,
                            direction: if from == canonical {
                                ArchaeologyRelationDirection::Outgoing
                            } else {
                                ArchaeologyRelationDirection::Incoming
                            },
                            kind: parse_enum(&kind, "relation kind")?,
                            rule_id: target_rule,
                            trust: parse_enum(&trust, "relation trust")?,
                            summary,
                            evidence_ids: parse_ids(&evidence, "relation evidence")?,
                        },
                    })
                },
            )
            .collect::<Result<Vec<_>, String>>()?;
        finish_page(
            scope,
            "list_relations",
            &query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }

    fn hydrate_evidence(
        &self,
        scope: &ReadyScope,
        rule_id: &str,
        evidence: Vec<ArchaeologyEvidenceSelector>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyPage<ArchaeologyEvidence>, String> {
        validate_digest_id("rule", rule_id)?;
        let canonical = self.canonical_rule(scope, rule_id)?.0;
        if evidence.is_empty() || evidence.len() > MAX_EVIDENCE_IDS {
            return Err(format!(
                "Archaeology evidence request must contain 1..={MAX_EVIDENCE_IDS} identities"
            ));
        }
        for item in &evidence {
            validate_id("evidence", &item.evidence_id)?;
        }
        let mut seen = BTreeSet::new();
        let evidence = evidence
            .into_iter()
            .filter(|selector| {
                seen.insert((
                    evidence_kind_name(&selector.kind),
                    selector.evidence_id.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let applied_limit = bounded_limit(limit).min(MAX_EVIDENCE_IDS);
        let query_identity =
            query_identity("hydrate_evidence", &(rule_id, &evidence, applied_limit))?;
        let after = self.decode_cursor(scope, "hydrate_evidence", &query_identity, cursor)?;
        let start = after
            .as_ref()
            .and_then(|after| {
                evidence.iter().position(|item| {
                    evidence_kind_name(&item.kind) == after.primary
                        && item.evidence_id == after.secondary
                })
            })
            .map_or(0, |index| index + 1);
        if after.is_some() && start == 0 {
            return Err("Archaeology cursor is invalid".into());
        }
        let total_rows = evidence.len() as u64;
        let selectors = evidence
            .into_iter()
            .skip(start)
            .take(applied_limit + 1)
            .collect::<Vec<_>>();
        let fact_ids = selectors
            .iter()
            .filter(|selector| matches!(selector.kind, ArchaeologyEvidenceKind::Fact))
            .map(|selector| selector.evidence_id.clone())
            .collect::<Vec<_>>();
        let span_ids = selectors
            .iter()
            .filter(|selector| matches!(selector.kind, ArchaeologyEvidenceKind::Span))
            .map(|selector| selector.evidence_id.clone())
            .collect::<Vec<_>>();
        let mut hydrated = self.hydrate_facts(scope, &canonical, &fact_ids)?;
        hydrated.extend(self.hydrate_spans(scope, &canonical, &span_ids)?);
        let mut rows = Vec::with_capacity(selectors.len());
        for selector in selectors {
            let key = (
                evidence_kind_name(&selector.kind).to_string(),
                selector.evidence_id.clone(),
            );
            let item = hydrated
                .remove(&key)
                .ok_or_else(|| UNAVAILABLE.to_string())?;
            rows.push(PageRow {
                primary: key.0,
                secondary: selector.evidence_id,
                item,
            });
        }
        finish_page(
            scope,
            "hydrate_evidence",
            &query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }

    fn hydrate_facts(
        &self,
        scope: &ReadyScope,
        occurrence_id: &str,
        fact_ids: &[String],
    ) -> Result<BTreeMap<(String, String), ArchaeologyEvidence>, String> {
        if fact_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let requested_json = serde_json::to_string(fact_ids)
            .map_err(|error| format!("Encode archaeology fact identities: {error}"))?;
        self.record_hydration_query();
        let mut statement = self
            .connection
            .prepare(
                "SELECT fact.fact_id,fact.kind,fact.label,fact.trust,fact.confidence,
                        COALESCE((SELECT json_group_array(item.evidence_id) FROM (
                          SELECT span.evidence_id FROM archaeology_evidence_links span
                          WHERE span.generation_id=fact.generation_id
                            AND span.owner_kind='fact' AND span.owner_id=fact.fact_id
                            AND span.evidence_kind='span' ORDER BY span.evidence_id) item),'[]')
                 FROM archaeology_facts fact
                 JOIN json_each(?3) requested
                   ON CAST(requested.value AS TEXT)=fact.fact_id
                 WHERE fact.generation_id=?1
                   AND EXISTS (SELECT 1 FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links link
                       ON link.generation_id=clause.generation_id
                      AND link.owner_kind='rule_clause' AND link.owner_id=clause.clause_id
                      AND link.evidence_kind='fact' AND link.evidence_id=fact.fact_id
                     WHERE clause.generation_id=fact.generation_id AND clause.rule_id=?2)
                 ORDER BY CAST(requested.key AS INTEGER)",
            )
            .map_err(|error| format!("Prepare archaeology fact hydration: {error}"))?;
        let hydrated = statement
            .query_map(
                (scope.generation_id.as_str(), occurrence_id, requested_json),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .map_err(|error| format!("Query archaeology fact hydration: {error}"))?
            .map(|row| {
                let (evidence_id, fact_kind, label, trust, confidence, spans) =
                    row.map_err(|error| format!("Read archaeology fact hydration: {error}"))?;
                safe_public_text(&label, 16 * 1024)?;
                Ok((
                    ("fact".into(), evidence_id.clone()),
                    ArchaeologyEvidence::Fact {
                        evidence_id,
                        fact_kind,
                        label,
                        trust: parse_enum(&trust, "fact trust")?,
                        confidence: parse_enum(&confidence, "fact confidence")?,
                        span_ids: parse_ids(&spans, "fact spans")?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(hydrated)
    }

    fn hydrate_spans(
        &self,
        scope: &ReadyScope,
        occurrence_id: &str,
        span_ids: &[String],
    ) -> Result<BTreeMap<(String, String), ArchaeologyEvidence>, String> {
        if span_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let requested_json = serde_json::to_string(span_ids)
            .map_err(|error| format!("Encode archaeology span identities: {error}"))?;
        self.record_hydration_query();
        let mut statement = self
            .connection
            .prepare(
                "SELECT span.span_id,unit.path_identity,unit.source_unit_id,unit.relative_path,
                        unit.language,unit.dialect,unit.classification,span.revision_sha,
                        span.start_byte,span.end_byte,span.start_line,span.start_column,
                        span.end_line,span.end_column
                 FROM archaeology_source_spans span JOIN archaeology_source_units unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 JOIN json_each(?3) requested
                   ON CAST(requested.value AS TEXT)=span.span_id
                 WHERE span.generation_id=?1
                   AND unit.classification NOT IN ('protected','opaque')
                   AND EXISTS (
                     SELECT 1 FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links direct
                       ON direct.generation_id=clause.generation_id
                      AND direct.owner_kind='rule_clause' AND direct.owner_id=clause.clause_id
                     WHERE clause.generation_id=span.generation_id AND clause.rule_id=?2
                       AND ((direct.evidence_kind='span' AND direct.evidence_id=span.span_id)
                         OR (direct.evidence_kind='fact' AND EXISTS (
                           SELECT 1 FROM archaeology_evidence_links fact_span
                           WHERE fact_span.generation_id=span.generation_id
                             AND fact_span.owner_kind='fact'
                             AND fact_span.owner_id=direct.evidence_id
                             AND fact_span.evidence_kind='span'
                             AND fact_span.evidence_id=span.span_id))))
                 ORDER BY CAST(requested.key AS INTEGER)",
            )
            .map_err(|error| format!("Prepare archaeology span hydration: {error}"))?;
        let hydrated = statement
            .query_map(
                (scope.generation_id.as_str(), occurrence_id, requested_json),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        ArchaeologyEvidenceSource {
                            source_id: row.get(1)?,
                            source_unit_id: row.get(2)?,
                            relative_path: row.get(3)?,
                            language: row.get(4)?,
                            dialect: row.get(5)?,
                            classification: row.get(6)?,
                            revision_sha: row.get(7)?,
                            start_byte: row.get(8)?,
                            end_byte: row.get(9)?,
                            start_line: row.get(10)?,
                            start_column: row.get(11)?,
                            end_line: row.get(12)?,
                            end_column: row.get(13)?,
                        },
                    ))
                },
            )
            .map_err(|error| format!("Query archaeology span hydration: {error}"))?
            .map(|row| {
                let (evidence_id, source) =
                    row.map_err(|error| format!("Read archaeology span hydration: {error}"))?;
                let path = source
                    .relative_path
                    .as_deref()
                    .ok_or_else(|| UNAVAILABLE.to_string())?;
                safe_relative_path(path)?;
                safe_public_text(&source.language, 128)?;
                if let Some(dialect) = source.dialect.as_deref() {
                    safe_public_text(dialect, 128)?;
                }
                Ok((
                    ("span".into(), evidence_id.clone()),
                    ArchaeologyEvidence::Span {
                        evidence_id,
                        source,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(hydrated)
    }

    fn decode_cursor(
        &self,
        scope: &ReadyScope,
        operation: &str,
        query_identity: &str,
        cursor: Option<&str>,
    ) -> Result<Option<CursorPayload>, String> {
        let Some(cursor) = cursor else {
            return Ok(None);
        };
        if cursor.len() > MAX_CURSOR_BYTES * 2 {
            return Err("Archaeology cursor is invalid".into());
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| "Archaeology cursor is invalid".to_string())?;
        if bytes.len() > MAX_CURSOR_BYTES {
            return Err("Archaeology cursor is invalid".into());
        }
        let payload: CursorPayload = serde_json::from_slice(&bytes)
            .map_err(|_| "Archaeology cursor is invalid".to_string())?;
        if payload.version != 1 || payload.operation != operation {
            return Err("Archaeology cursor is invalid".into());
        }
        if payload.repository_id != scope.repository_id || payload.query_identity != query_identity
        {
            return Err("Archaeology cursor is unavailable for this scope".into());
        }
        if payload.generation_id != scope.generation_id {
            return Err("Archaeology cursor is stale".into());
        }
        validate_id("cursor position", &payload.primary)?;
        if !payload.secondary.is_empty() {
            validate_id("cursor position", &payload.secondary)?;
        }
        Ok(Some(payload))
    }
}

fn decode_raw_rule_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRuleSummaryRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn rule_summary_page_row(
    raw: RawRuleSummaryRow,
) -> Result<PageRow<ArchaeologyRuleSummary>, String> {
    let (occurrence, stable, title, kind, lifecycle, trust, confidence, domains) = raw;
    safe_public_text(&title, 16 * 1024)?;
    Ok(PageRow {
        primary: occurrence,
        secondary: String::new(),
        item: ArchaeologyRuleSummary {
            rule_id: stable,
            title,
            kind: parse_enum(&kind, "rule kind")?,
            lifecycle: parse_enum(&lifecycle, "rule lifecycle")?,
            trust: parse_enum(&trust, "rule trust")?,
            confidence: parse_enum(&confidence, "rule confidence")?,
            domain_ids: parse_ids(&domains, "rule domains")?,
        },
    })
}

fn result<T>(scope: &ReadyScope, value: T) -> ArchaeologyResult<T> {
    ArchaeologyResult {
        context: scope.context.clone(),
        value,
    }
}

fn decode_temporal_snapshot(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<ArchaeologyTemporalSnapshot>> {
    let Some(snapshot_id) = row.get::<_, Option<String>>(offset)? else {
        return Ok(None);
    };
    let payload_json = row.get::<_, String>(offset + 8)?;
    let mut payload: ArchaeologyTemporalSnapshotPayload = serde_json::from_str(&payload_json)
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                offset + 8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    // Content hashes stay in the persisted compatibility payload but never
    // cross a desktop or MCP transport boundary.
    for span in payload
        .clauses
        .iter_mut()
        .flat_map(|clause| &mut clause.evidence)
        .flat_map(|evidence| &mut evidence.spans)
    {
        span.content_hash.clear();
    }
    let kind =
        parse_enum(&row.get::<_, String>(offset + 3)?, "temporal rule kind").map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                offset + 3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?;
    Ok(Some(ArchaeologyTemporalSnapshot {
        snapshot_id,
        stable_rule_id: row.get(offset + 1)?,
        continuity_id: row.get(offset + 2)?,
        kind,
        evidence_identity: row.get(offset + 4)?,
        parser_compatibility_identity: row.get(offset + 5)?,
        contradiction_identity: row.get(offset + 6)?,
        description_identity: row.get(offset + 7)?,
        payload,
    }))
}

fn validate_temporal_snapshot(
    snapshot: Option<ArchaeologyTemporalSnapshot>,
) -> Result<Option<ArchaeologyTemporalSnapshot>, String> {
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    for (label, value) in [
        ("temporal snapshot", snapshot.snapshot_id.as_str()),
        ("temporal stable rule", snapshot.stable_rule_id.as_str()),
        ("temporal continuity", snapshot.continuity_id.as_str()),
        ("temporal evidence", snapshot.evidence_identity.as_str()),
        (
            "temporal parser compatibility",
            snapshot.parser_compatibility_identity.as_str(),
        ),
        (
            "temporal contradiction",
            snapshot.contradiction_identity.as_str(),
        ),
        (
            "temporal description",
            snapshot.description_identity.as_str(),
        ),
    ] {
        validate_digest_id(label, value)?;
    }
    safe_public_text(&snapshot.payload.title, 16 * 1024)?;
    if snapshot.payload.clauses.len() > 256 {
        return Err("Stored archaeology temporal clause bound is invalid".into());
    }
    for clause in &snapshot.payload.clauses {
        safe_public_text(&clause.text, 64 * 1024)?;
        parse_enum::<ArchaeologyTrust>(&clause.trust, "temporal clause trust")?;
        parse_enum::<ArchaeologyConfidence>(&clause.confidence, "temporal clause confidence")?;
        if clause.caveats.len() > 256 || clause.evidence.len() > 512 {
            return Err("Stored archaeology temporal clause bound is invalid".into());
        }
        for caveat in &clause.caveats {
            safe_public_text(caveat, 1024)?;
        }
        for evidence in &clause.evidence {
            validate_id("temporal evidence role", &evidence.role)?;
            validate_id("temporal fact", &evidence.fact_identity)?;
            validate_id("temporal fact kind", &evidence.fact_kind)?;
            validate_id("temporal parser", &evidence.parser_identity)?;
            if evidence.spans.len() > 256 {
                return Err("Stored archaeology temporal span bound is invalid".into());
            }
            for span in &evidence.spans {
                validate_id("temporal path", &span.path_identity)?;
                if span.end_byte < span.start_byte
                    || span.end_line < span.start_line
                    || (span.end_line == span.start_line && span.end_column < span.start_column)
                {
                    return Err("Stored archaeology temporal span is invalid".into());
                }
            }
        }
    }
    Ok(Some(snapshot))
}

fn validate_coverage_name(value: &str) -> Result<(), String> {
    if matches!(value, "complete" | "partial" | "unavailable") {
        Ok(())
    } else {
        Err("Stored archaeology temporal coverage is invalid".into())
    }
}

fn weakest_coverage(left: &str, right: &str) -> String {
    let rank = |value| match value {
        "complete" => 0,
        "partial" => 1,
        _ => 2,
    };
    if rank(left) >= rank(right) {
        left.to_string()
    } else {
        right.to_string()
    }
}

fn weaken_coverage(current: &mut String, candidate: &str) {
    *current = weakest_coverage(current, candidate);
}

fn finish_page<T: Serialize>(
    scope: &ReadyScope,
    operation: &str,
    query_identity: &str,
    applied_limit: usize,
    total_rows: u64,
    mut rows: Vec<PageRow<T>>,
) -> Result<ArchaeologyPage<T>, String> {
    let has_more = rows.len() > applied_limit;
    rows.truncate(applied_limit);
    let returned_rows = rows.len();
    let mut positions = Vec::with_capacity(returned_rows);
    let mut items = Vec::with_capacity(returned_rows);
    for row in rows {
        positions.push((row.primary, row.secondary));
        items.push(row.item);
    }
    let mut page = ArchaeologyPage {
        context: scope.context.clone(),
        items,
        page: ArchaeologyPageInfo {
            applied_limit,
            returned_rows,
            total_rows,
            truncated: has_more,
            next_cursor: None,
        },
    };
    while serialized_bytes(&page)? > scope.context.bounds.max_response_bytes
        && !positions.is_empty()
    {
        positions.pop();
        page.items.pop();
        page.page.returned_rows = positions.len();
        page.page.truncated = true;
    }
    if page.page.truncated {
        loop {
            let last = positions
                .last()
                .ok_or("Archaeology response item exceeds the byte bound")?;
            let payload = CursorPayload {
                version: 1,
                repository_id: scope.repository_id.clone(),
                generation_id: scope.generation_id.clone(),
                operation: operation.into(),
                query_identity: query_identity.into(),
                primary: last.0.clone(),
                secondary: last.1.clone(),
            };
            let bytes = serde_json::to_vec(&payload)
                .map_err(|error| format!("Encode archaeology cursor: {error}"))?;
            if bytes.len() > MAX_CURSOR_BYTES {
                return Err("Archaeology cursor exceeds its byte bound".into());
            }
            page.page.next_cursor = Some(URL_SAFE_NO_PAD.encode(bytes));
            if serialized_bytes(&page)? <= scope.context.bounds.max_response_bytes {
                break;
            }
            positions.pop();
            page.items.pop();
            page.page.returned_rows = positions.len();
        }
    }
    Ok(page)
}

fn rule_predicates(
    scope: &ReadyScope,
    filter: &ArchaeologyRuleFilter,
) -> Result<(String, Vec<SqlValue>, bool), String> {
    let mut predicates = vec![
        "rule.generation_id=?".to_string(),
        "rule.repository_id=?".to_string(),
        "rule.identity_schema_version=2".to_string(),
        "NOT EXISTS (SELECT 1 FROM archaeology_rule_relations alias
          WHERE alias.generation_id=rule.generation_id
            AND alias.from_rule_id=rule.rule_id AND alias.kind='aliases')"
            .to_string(),
    ];
    let mut values = vec![
        scope.generation_id.clone().into(),
        scope.repository_id.clone().into(),
    ];
    let fts = filter.query.is_some();
    if let Some(query) = filter.query.as_deref() {
        predicates.push("archaeology_rule_fts MATCH ?".into());
        values.push(fts_query(query)?.into());
    }
    if !filter.kinds.is_empty() {
        predicates.push(format!(
            "rule.kind IN ({})",
            placeholders(filter.kinds.len())
        ));
        values.extend(
            filter
                .kinds
                .iter()
                .map(|kind| rule_kind_name(kind).to_string().into()),
        );
    }
    if !filter.trust.is_empty() {
        predicates.push(format!(
            "rule.trust IN ({})",
            placeholders(filter.trust.len())
        ));
        values.extend(
            filter
                .trust
                .iter()
                .map(|trust| trust_name(trust).to_string().into()),
        );
    }
    if !filter.lifecycle.is_empty() {
        predicates.push(format!(
            "{} IN ({})",
            effective_lifecycle_sql(),
            placeholders(filter.lifecycle.len())
        ));
        values.extend(
            filter
                .lifecycle
                .iter()
                .map(|lifecycle| lifecycle_name(lifecycle).to_string().into()),
        );
    }
    if !filter.domain_ids.is_empty() {
        predicates.push(format!(
            "EXISTS (SELECT 1 FROM archaeology_rule_domains wanted
              WHERE wanted.generation_id=rule.generation_id AND wanted.rule_id=rule.rule_id
                AND wanted.domain_id IN ({}))",
            placeholders(filter.domain_ids.len())
        ));
        values.extend(filter.domain_ids.iter().cloned().map(Into::into));
    }
    Ok((predicates.join(" AND "), values, fts))
}

fn reverse_rule_cte(span_predicate: &str) -> String {
    // Reverse lookup is the latency-sensitive consumer of the compact v4
    // evidence store. Querying the text compatibility view here makes SQLite
    // decode and scan every identity in a generation before applying the span
    // predicate. Resolve the generation/span keys once, then force exact
    // target-first probes through the compact reverse index.
    format!(
        "WITH evidence_generation(generation_key) AS MATERIALIZED (
           SELECT generation_key FROM archaeology_generation_keys WHERE generation_id=?1
         ), target_spans(span_id) AS MATERIALIZED (
           SELECT span.span_id FROM archaeology_source_spans span
           JOIN archaeology_source_units unit ON unit.generation_id=span.generation_id
             AND unit.source_unit_id=span.source_unit_id
           WHERE span.generation_id=?1 AND {span_predicate}
         ), target_evidence(evidence_identity_key) AS MATERIALIZED (
           SELECT identity.identity_key FROM evidence_generation generation
           JOIN archaeology_evidence_identities identity
             ON identity.generation_key=generation.generation_key
           JOIN target_spans target ON target.span_id=identity.identity
         ), matched(rule_id) AS (
           SELECT clause.rule_id FROM evidence_generation generation
           CROSS JOIN target_evidence target
           CROSS JOIN archaeology_evidence_links_compact AS direct
             INDEXED BY idx_archaeology_evidence_reverse
             ON direct.generation_key=generation.generation_key
            AND direct.evidence_kind_code=1
            AND direct.evidence_identity_key=target.evidence_identity_key
            AND direct.owner_kind_code=3
           JOIN archaeology_evidence_identities direct_owner
             ON direct_owner.generation_key=direct.generation_key
            AND direct_owner.identity_key=direct.owner_identity_key
           JOIN archaeology_rule_clauses clause
             ON clause.generation_id=?1 AND clause.clause_id=direct_owner.identity
           UNION ALL
           SELECT clause.rule_id FROM evidence_generation generation
           CROSS JOIN target_evidence target
           CROSS JOIN archaeology_evidence_links_compact AS fact_span
             INDEXED BY idx_archaeology_evidence_reverse
             ON fact_span.generation_key=generation.generation_key
            AND fact_span.evidence_kind_code=1
            AND fact_span.evidence_identity_key=target.evidence_identity_key
            AND fact_span.owner_kind_code=1
           CROSS JOIN archaeology_evidence_links_compact AS fact_link
             INDEXED BY idx_archaeology_evidence_reverse
             ON fact_link.generation_key=fact_span.generation_key
            AND fact_link.evidence_kind_code=2
            AND fact_link.evidence_identity_key=fact_span.owner_identity_key
            AND fact_link.owner_kind_code=3
           JOIN archaeology_evidence_identities fact_link_owner
             ON fact_link_owner.generation_key=fact_link.generation_key
            AND fact_link_owner.identity_key=fact_link.owner_identity_key
           JOIN archaeology_rule_clauses clause
             ON clause.generation_id=?1 AND clause.clause_id=fact_link_owner.identity
           UNION ALL
           SELECT relation.from_rule_id FROM evidence_generation generation
           CROSS JOIN target_evidence target
           CROSS JOIN archaeology_evidence_links_compact AS link
             INDEXED BY idx_archaeology_evidence_reverse
             ON link.generation_key=generation.generation_key
            AND link.evidence_kind_code=1
            AND link.evidence_identity_key=target.evidence_identity_key
            AND link.owner_kind_code=4
           JOIN archaeology_evidence_identities link_owner
             ON link_owner.generation_key=link.generation_key
            AND link_owner.identity_key=link.owner_identity_key
           JOIN archaeology_rule_relations relation
             ON relation.generation_id=?1 AND relation.relation_id=link_owner.identity
           UNION ALL
           SELECT relation.to_rule_id FROM evidence_generation generation
           CROSS JOIN target_evidence target
           CROSS JOIN archaeology_evidence_links_compact AS link
             INDEXED BY idx_archaeology_evidence_reverse
             ON link.generation_key=generation.generation_key
            AND link.evidence_kind_code=1
            AND link.evidence_identity_key=target.evidence_identity_key
            AND link.owner_kind_code=4
           JOIN archaeology_evidence_identities link_owner
             ON link_owner.generation_key=link.generation_key
            AND link_owner.identity_key=link.owner_identity_key
           JOIN archaeology_rule_relations relation
             ON relation.generation_id=?1 AND relation.relation_id=link_owner.identity
         )"
    )
}

fn rule_list_from_sql(fts: bool) -> &'static str {
    if fts {
        " FROM archaeology_rules rule
           JOIN archaeology_rule_search_manifest manifest
             ON manifest.generation_id=rule.generation_id AND manifest.rule_id=rule.rule_id
           JOIN archaeology_rule_fts
             ON archaeology_rule_fts.generation_id=manifest.generation_id
            AND archaeology_rule_fts.rule_id=manifest.rule_id "
    } else {
        " FROM archaeology_rules rule
           JOIN archaeology_rule_search_manifest manifest
             ON manifest.generation_id=rule.generation_id AND manifest.rule_id=rule.rule_id "
    }
}

fn rule_list_sql(from_sql: &str, where_sql: &str, after_sql: &str) -> String {
    format!(
        "WITH matched_rules(rule_id,total_rows) AS MATERIALIZED (
           SELECT rule.rule_id,COUNT(*) OVER()
           {from_sql} WHERE {where_sql}
         )
         SELECT rule.rule_id,rule.stable_rule_identity,manifest.title,rule.kind,
                {lifecycle},rule.trust,rule.confidence,
                COALESCE((SELECT json_group_array(item.domain_id) FROM (
                  SELECT domain.domain_id FROM archaeology_rule_domains domain
                  WHERE domain.generation_id=rule.generation_id AND domain.rule_id=rule.rule_id
                  ORDER BY domain.domain_id) item),'[]'),matched.total_rows
         FROM matched_rules matched
         CROSS JOIN archaeology_rules rule
           ON rule.generation_id=? AND rule.rule_id=matched.rule_id
         CROSS JOIN archaeology_rule_search_manifest manifest
           ON manifest.generation_id=rule.generation_id AND manifest.rule_id=rule.rule_id
         WHERE 1=1{after_sql}
         ORDER BY rule.rule_id LIMIT ?",
        lifecycle = effective_lifecycle_sql()
    )
}

fn effective_lifecycle_sql() -> &'static str {
    "COALESCE((SELECT review.decision FROM archaeology_rule_review_events review
       WHERE review.repository_id=rule.repository_id
         AND review.stable_rule_identity=rule.stable_rule_identity
         AND review.event_schema_version=2 AND review.legacy_stale=0
         AND review.decision<>'annotation'
       ORDER BY review.logical_sequence DESC,review.event_id DESC LIMIT 1),rule.lifecycle)"
}

fn normalize_filter(filter: &mut ArchaeologyRuleFilter) -> Result<(), String> {
    if let Some(query) = filter.query.as_mut() {
        *query = query.trim().to_string();
        if query.is_empty() {
            filter.query = None;
        } else {
            fts_query(query)?;
        }
    }
    if filter.kinds.len() > MAX_FILTER_VALUES
        || filter.trust.len() > MAX_FILTER_VALUES
        || filter.lifecycle.len() > MAX_FILTER_VALUES
        || filter.domain_ids.len() > MAX_FILTER_VALUES
    {
        return Err("Archaeology rule filter bound exceeded".into());
    }
    filter.kinds.sort_by_key(rule_kind_name);
    filter.kinds.dedup();
    filter.trust.sort_by_key(trust_name);
    filter.trust.dedup();
    filter.lifecycle.sort_by_key(lifecycle_name);
    filter.lifecycle.dedup();
    filter.domain_ids.sort();
    filter.domain_ids.dedup();
    for domain in &filter.domain_ids {
        validate_id("domain", domain)?;
    }
    Ok(())
}

fn fts_query(value: &str) -> Result<String, String> {
    if value.len() > MAX_QUERY_BYTES || value.contains('\0') {
        return Err("Archaeology search query exceeds its bound".into());
    }
    let tokens = value
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .take(MAX_QUERY_TOKENS + 1)
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > MAX_QUERY_TOKENS {
        return Err("Archaeology search query has no bounded searchable terms".into());
    }
    Ok(tokens
        .into_iter()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND "))
}

fn query_count(connection: &Connection, sql: &str, values: &[SqlValue]) -> Result<u64, String> {
    connection
        .query_row(sql, params_from_iter(values.iter()), |row| row.get(0))
        .map_err(|error| format!("Count archaeology read rows: {error}"))
}

fn query_identity<T: Serialize>(operation: &str, value: &T) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"archaeology-read-query:v1\0");
    digest.update(operation.as_bytes());
    digest.update([0]);
    digest.update(
        serde_json::to_vec(value)
            .map_err(|error| format!("Encode archaeology query identity: {error}"))?,
    );
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn bounded_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

fn parse_enum<T: DeserializeOwned>(value: &str, label: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| format!("Stored archaeology {label} is invalid"))
}

fn parse_json<T: DeserializeOwned>(value: &str, label: &str) -> Result<T, String> {
    serde_json::from_str(value).map_err(|_| format!("Stored archaeology {label} is invalid"))
}

fn parse_ids(value: &str, label: &str) -> Result<Vec<String>, String> {
    let values: Vec<String> = parse_json(value, label)?;
    for value in &values {
        validate_id(label, value)?;
    }
    Ok(values)
}

fn parse_safe_strings(value: &str, label: &str, max: usize) -> Result<Vec<String>, String> {
    let values: Vec<String> = parse_json(value, label)?;
    for value in &values {
        safe_public_text(value, max)?;
    }
    Ok(values)
}

fn validate_coverage(coverage: &ArchaeologyCoverage) -> Result<(), String> {
    if coverage.indexed_source_units > coverage.discovered_source_units
        || coverage.indexed_bytes > coverage.discovered_bytes
        || coverage.reasons.len() > 256
    {
        return Err("Stored archaeology coverage is inconsistent".into());
    }
    for reason in &coverage.reasons {
        safe_public_text(reason, 2048)?;
    }
    Ok(())
}

fn validate_temporal_selector(selector: &ArchaeologyTemporalSelector) -> Result<(), String> {
    match selector {
        ArchaeologyTemporalSelector::Generation { generation_id } => {
            validate_id("generation", generation_id)
        }
        ArchaeologyTemporalSelector::Revision { revision_sha } => {
            if matches!(revision_sha.len(), 40 | 64)
                && revision_sha
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                Ok(())
            } else {
                Err("Archaeology temporal revision is invalid".into())
            }
        }
        ArchaeologyTemporalSelector::Release { tag } => validate_id("release", tag),
    }
}

fn validate_id(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > MAX_ID_BYTES
        || value.contains('\0')
        || value.chars().any(char::is_control)
        || looks_like_secret(value)
        || contains_sensitive_path(value)
    {
        Err(format!("Archaeology {label} identity is invalid"))
    } else {
        Ok(())
    }
}

fn validate_digest_id(label: &str, value: &str) -> Result<(), String> {
    validate_id(label, value)?;
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(format!("Archaeology {label} identity is invalid"))
    }
}

fn safe_public_text(value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.contains('\0')
        || looks_like_secret(value)
        || contains_sensitive_path(value)
    {
        Err("Stored archaeology text is not safe to expose".into())
    } else {
        Ok(())
    }
}

fn safe_relative_path(value: &str) -> Result<(), String> {
    let path = value.replace('\\', "/");
    let windows_absolute = path.as_bytes().get(1) == Some(&b':');
    if path.starts_with('/')
        || windows_absolute
        || path.split('/').any(|part| part.is_empty() || part == "..")
        || contains_sensitive_path(&path)
        || looks_like_secret(&path)
    {
        Err(UNAVAILABLE.into())
    } else {
        Ok(())
    }
}

fn serialized_bytes<T: Serialize>(value: &T) -> Result<usize, String> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| format!("Serialize archaeology response: {error}"))
}

fn rule_kind_name(value: &ArchaeologyRuleKind) -> &'static str {
    match value {
        ArchaeologyRuleKind::Validation => "validation",
        ArchaeologyRuleKind::Calculation => "calculation",
        ArchaeologyRuleKind::Eligibility => "eligibility",
        ArchaeologyRuleKind::Entitlement => "entitlement",
        ArchaeologyRuleKind::Routing => "routing",
        ArchaeologyRuleKind::Mutation => "mutation",
        ArchaeologyRuleKind::Exception => "exception",
        ArchaeologyRuleKind::Lifecycle => "lifecycle",
        ArchaeologyRuleKind::Transaction => "transaction",
        ArchaeologyRuleKind::Other => "other",
    }
}

fn trust_name(value: &ArchaeologyTrust) -> &'static str {
    match value {
        ArchaeologyTrust::Extracted => "extracted",
        ArchaeologyTrust::Deterministic => "deterministic",
        ArchaeologyTrust::ModelSynthesized => "model_synthesized",
        ArchaeologyTrust::HumanConfirmed => "human_confirmed",
        ArchaeologyTrust::Unknown => "unknown",
    }
}

fn lifecycle_name(value: &ArchaeologyRuleLifecycle) -> &'static str {
    match value {
        ArchaeologyRuleLifecycle::Candidate => "candidate",
        ArchaeologyRuleLifecycle::ReviewNeeded => "review_needed",
        ArchaeologyRuleLifecycle::Accepted => "accepted",
        ArchaeologyRuleLifecycle::Rejected => "rejected",
        ArchaeologyRuleLifecycle::Superseded => "superseded",
        ArchaeologyRuleLifecycle::Conflicted => "conflicted",
        ArchaeologyRuleLifecycle::Unavailable => "unavailable",
    }
}

fn relation_kind_name(value: &ArchaeologyRelationKind) -> &'static str {
    match value {
        ArchaeologyRelationKind::DependsOn => "depends_on",
        ArchaeologyRelationKind::Precedes => "precedes",
        ArchaeologyRelationKind::Overrides => "overrides",
        ArchaeologyRelationKind::Aliases => "aliases",
        ArchaeologyRelationKind::ConflictsWith => "conflicts_with",
        ArchaeologyRelationKind::Supersedes => "supersedes",
    }
}

fn evidence_kind_name(value: &ArchaeologyEvidenceKind) -> &'static str {
    match value {
        ArchaeologyEvidenceKind::Fact => "fact",
        ArchaeologyEvidenceKind::Span => "span",
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;

#[path = "read_temporal.rs"]
mod temporal;
