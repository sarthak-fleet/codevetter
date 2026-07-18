//! Durable, owner-identified archaeology job transitions over existing SQLite rows.

use super::adapter::{ArchaeologyAdapterLineage, ArchaeologyAdapterRegion, ArchaeologyLineageKind};
use super::contracts::{
    validate_revision_sha, ArchaeologyAttribute, ArchaeologyCoverage, ArchaeologyCoverageState,
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyJobStage, ArchaeologyJobState,
    ArchaeologyJobStatus, ArchaeologyRuleClause, ArchaeologyRuleLifecycle, ArchaeologyRulePacket,
    ArchaeologySourceClassification, ArchaeologySourceSpan, ArchaeologySourceUnitIdentity,
    ArchaeologyTrust, ARCHAEOLOGY_SCHEMA_VERSION, ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
    ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION,
};
use super::deterministic_rules::{
    cluster_evidence_compatible_rules, derive_evidence_packets, expected_rule_id,
    render_template_rules, ArchaeologyDeterministicLimits, ArchaeologyFactOrigin,
};
use super::evidence_store::{
    insert_clause_evidence_json, insert_link_patch_evidence_json, insert_relation_evidence_json,
    prune_orphan_evidence_identities,
};
use super::identity_store::{refresh_rule_identities, validate_rule_identities};
use super::invalidation::{
    ArchaeologyGenerationInput, ArchaeologyInputInvalidationMode, ArchaeologyInvalidationLimits,
};
use super::invalidation_store::{
    changed_source_paths, clone_unaffected_ready_facts, execute_refresh_parse_work_batch,
    load_generation_inputs, persist_generation_invalidation_metadata, persist_refresh_work_plan,
    plan_generation_invalidation, ArchaeologyRefreshExecution, ArchaeologyRefreshWorkItem,
};
use super::inventory::{
    git_head, inventory_repository_delta, inventory_repository_streaming,
    ArchaeologyInventoryLimits, ArchaeologyInventoryUnit, INVENTORY_POLICY_VERSION,
};
use super::lifecycle_store::{reconcile_generation_lifecycle, validate_generation_alias_relations};
use super::synthesis::{
    canonical_synthesis_clause_text, canonicalize_synthesis_response,
    quantifier_kinds_from_evidence, validate_synthesis_request, validate_synthesis_response,
    ArchaeologySynthesisLimits, ArchaeologySynthesisRequest, ArchaeologySynthesisResponse,
    ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
};
use super::temporal_store::{
    persist_temporal_projection, ArchaeologyTemporalCoverageInput,
    ArchaeologyTemporalCoverageState, ArchaeologyTemporalLimits, ArchaeologyTemporalProjection,
};
use super::{
    link_archaeology_facts, ArchaeologyLinkFact, ArchaeologyLinkLimits, ArchaeologyLinkPatch,
    ArchaeologyLinkUnit,
};
use crate::commands::history_read::temporal::{
    resolve_archaeology_temporal_context, PersistedTemporalCoverageState,
};
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::{stable_graph_id, StructuralGraphCancellation};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

const MAX_ID_BYTES: usize = 256;
const MAX_CHECKPOINT_BYTES: usize = 64 * 1024;
const MAX_ERRORS: usize = 16;
const MAX_ERRORS_JSON_BYTES: usize = 32 * 1024;
const MAX_CHECKPOINT_COUNTERS: usize = 32;
const MAX_CLEANUP_GENERATIONS: usize = 256;
const VALIDATION_RECEIPT_VERSION: u32 = 1;
const INVENTORY_COMPLETE_COUNTER: &str = "inventory_complete";
const MAX_COVERAGE_REASONS: usize = 32;
const MAX_COVERAGE_REASON_BYTES: usize = 512;
const MAX_RULE_TITLE_BYTES: usize = 4 * 1024;
const MAX_RULE_CLAUSES: usize = 1_024;
const MAX_RULE_CAVEATS: usize = 256;
const MAX_RULE_CLAUSE_TEXT_BYTES: usize = 64 * 1024;
const MAX_RULE_DOMAINS: usize = 256;
const MAX_RULE_DOMAIN_TEXT_BYTES: usize = 16 * 1024;
const MAX_VALIDATION_ROW_BYTES: usize = 256 * 1024;
const MAX_FINAL_RULES: usize = 100_000;
const MAX_FINAL_CLAUSES: usize = 1_000_000;
const MAX_FINAL_DOMAINS: usize = 1_000_000;
const MAX_FINAL_CATALOG_BYTES: usize = 256 * 1024 * 1024;
const PRODUCTION_PARSER_MANIFEST: &str = "parser-manifest:v1:codevetter-assembly-fallback@2,codevetter-cobol-fallback@2,codevetter-tree-sitter@1.archaeology2,unavailable@unavailable";
const PRODUCTION_ALGORITHM_IDENTITY: &str = "algorithm:v2";
const PRODUCTION_SYNTHESIS_IDENTITY: &str = "synthesis:v1";
const COMPACT_EVIDENCE_SEAL_TABLE: &str = concat!(
    "archaeology_evidence_links_compact link ",
    "JOIN archaeology_generation_keys generation USING(generation_key) ",
    "JOIN archaeology_evidence_identities owner ON owner.identity_key=link.owner_identity_key ",
    "JOIN archaeology_evidence_identities referenced ON referenced.identity_key=link.evidence_identity_key",
);
const COMPACT_EVIDENCE_SEAL_COLUMNS: &str =
    "link.owner_kind_code,owner.identity,link.evidence_kind_code,referenced.identity,link.role_code";
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyJobCheckpoint {
    pub(crate) cursor_identity: Option<String>,
    pub(crate) source_unit_id: Option<String>,
    pub(crate) ordinal: Option<u64>,
    pub(crate) counters: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyJobErrorCode {
    InventoryFailed,
    ParserFailed,
    LinkFailed,
    DerivationFailed,
    SynthesisFailed,
    ValidationFailed,
    PublicationFailed,
    CleanupFailed,
    OwnershipLost,
    Internal,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyGenerationIdentity<'a> {
    pub(crate) revision_sha: &'a str,
    pub(crate) source: &'a str,
    pub(crate) parser: &'a str,
    pub(crate) algorithm: &'a str,
    pub(crate) config: &'a str,
}

impl ArchaeologyGenerationIdentity<'_> {
    fn validate(&self) -> Result<(), String> {
        validate_revision_sha(self.revision_sha)?;
        for (label, value) in [
            ("source", self.source),
            ("parser", self.parser),
            ("algorithm", self.algorithm),
            ("config", self.config),
        ] {
            validate_id(label, value)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub(crate) struct NewArchaeologyJob<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) total_units: Option<u64>,
    pub(crate) now: &'a str,
}

pub(crate) struct ArchaeologyInventoryRefreshStage<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) units: &'a [ArchaeologyInventoryUnit],
    pub(crate) generation_inputs: &'a [ArchaeologyGenerationInput],
    pub(crate) cancellation: &'a StructuralGraphCancellation,
    pub(crate) limits: ArchaeologyInvalidationLimits,
    pub(crate) now: &'a str,
}

pub(crate) struct ArchaeologyInventoryRefreshRun<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) repository_root: &'a Path,
    pub(crate) inventory_limits: ArchaeologyInventoryLimits,
    pub(crate) invalidation_limits: ArchaeologyInvalidationLimits,
    pub(crate) cancellation: &'a StructuralGraphCancellation,
    pub(crate) now: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyInventoryRefreshOutcome {
    pub(crate) plan_identity: String,
    pub(crate) effective_generation_id: String,
    pub(crate) reused_ready_generation: bool,
    pub(crate) mode: ArchaeologyInputInvalidationMode,
    pub(crate) changed_paths: Vec<String>,
    pub(crate) next_stage: ArchaeologyJobStage,
}
#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyPublication<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) now: &'a str,
}
pub(crate) struct ArchaeologyLinkStage<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) cancellation: &'a StructuralGraphCancellation,
    pub(crate) limits: ArchaeologyLinkLimits,
    pub(crate) now: &'a str,
}
pub(crate) struct ArchaeologyDeriveStage<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) cancellation: &'a StructuralGraphCancellation,
    pub(crate) limits: ArchaeologyDeterministicLimits,
    pub(crate) now: &'a str,
}
pub(crate) struct ArchaeologySynthesisCatalogStage<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) repository_id: &'a str,
    pub(crate) generation_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) identity: ArchaeologyGenerationIdentity<'a>,
    pub(crate) cancellation: &'a StructuralGraphCancellation,
    pub(crate) now: &'a str,
}
pub(crate) struct ArchaeologyModelSynthesisCatalog<'a> {
    pub(crate) cache_key: &'a str,
    pub(crate) request: &'a ArchaeologySynthesisRequest,
    pub(crate) response: &'a ArchaeologySynthesisResponse,
    pub(crate) limits: ArchaeologySynthesisLimits,
}
#[derive(Deserialize)]
struct PersistedLinkUnit {
    source_unit_id: String,
    language: String,
    dialect: Option<String>,
    relative_path: Option<String>,
    parser_id: String,
    parser_version: String,
    lineage: Vec<ArchaeologyAdapterLineage>,
}
#[derive(Deserialize)]
struct PersistedLinkFact {
    source_unit_id: String,
    fact: ArchaeologyFact,
    evidence_spans: Vec<ArchaeologySourceSpan>,
}
#[derive(Deserialize)]
struct PersistedFactOrigin {
    fact_id: String,
    source_unit_id: String,
    path_identity: String,
    relative_path: Option<String>,
    start_byte: u64,
    end_byte: u64,
    classification: ArchaeologySourceClassification,
}
#[derive(Serialize)]
struct PersistedRuleClause<'a> {
    rule_id: &'a str,
    ordinal: usize,
    clause: &'a ArchaeologyRuleClause,
}
#[derive(Serialize)]
struct PersistedRuleRelation<'a> {
    relation_id: String,
    from_rule_id: &'a str,
    to_rule_id: &'a str,
    kind: &'static str,
}
struct ArchaeologySqliteProgress<'a>(&'a Connection);
impl Drop for ArchaeologySqliteProgress<'_> {
    fn drop(&mut self) {
        self.0.progress_handler(0, None::<fn() -> bool>);
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchaeologyCleanupMode {
    DryRun,
    Apply,
}
#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyCleanup<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) mode: ArchaeologyCleanupMode,
    pub(crate) retain_superseded: usize,
    pub(crate) now: &'a str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyCleanupGeneration {
    pub(crate) generation_id: String,
    pub(crate) status: String,
    pub(crate) search_index_rows: u64,
    pub(crate) synthesis_cache_rows: u64,
    pub(crate) synthesis_attempt_rows: u64,
    pub(crate) synthesis_response_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyCleanupReport {
    pub(crate) dry_run: bool,
    pub(crate) repository_id: String,
    pub(crate) candidates: Vec<ArchaeologyCleanupGeneration>,
    pub(crate) truncated: bool,
    pub(crate) deleted_generations: u64,
    pub(crate) deleted_search_index_rows: u64,
    pub(crate) deleted_synthesis_cache_rows: u64,
    pub(crate) deleted_synthesis_attempt_rows: u64,
    pub(crate) deleted_synthesis_response_bytes: u64,
    /// These resource types have no implementation or persisted ownership
    /// record yet, so cleanup must not claim or attempt deletion.
    pub(crate) unavailable_resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArchaeologyValidationReceipt {
    version: u32,
    repository_id: String,
    generation_id: String,
    revision_sha: String,
    source_identity: String,
    parser_identity: String,
    algorithm_identity: String,
    config_identity: String,
    schema_version: u32,
    snapshot: ArchaeologyValidationSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArchaeologyValidationSnapshot {
    empty_inventory_proven: bool,
    coverage_sha256: String,
    tables: BTreeMap<String, ArchaeologyTableSeal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArchaeologyTableSeal {
    count: u64,
    sha256: String,
}
#[derive(Default)]
struct CoverageTotals {
    discovered_units: u64,
    indexed_units: u64,
    discovered_bytes: u64,
    indexed_bytes: u64,
}
pub(crate) fn start_job(
    connection: &Connection,
    input: NewArchaeologyJob<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    let total_units = input.total_units.map(to_i64).transpose()?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology job transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO archaeology_generations (
                generation_id, repository_id, schema_version, revision_sha,
                source_identity, parser_identity, algorithm_identity,
                config_identity, status, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'staging', ?9)",
            params![
                input.generation_id,
                input.repository_id,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                input.now,
            ],
        )
        .map_err(|error| format!("Create archaeology staging generation: {error}"))?;
    transaction
        .execute(
            "INSERT INTO archaeology_jobs (
                job_id, repository_id, generation_id, owner_id, stage, state,
                checkpoint_json, completed_units, total_units,
                cancellation_requested, errors_json, started_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'inventory', 'running', '{}', 0,
                ?5, 0, '[]', ?6, ?6)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                total_units,
                input.now,
            ],
        )
        .map_err(|error| format!("Create archaeology job: {error}"))?;
    let status = load_job(&transaction, input.job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology job: {error}"))?;
    Ok(status)
}

/// Scan the exact Git revision and enter the durable incremental job flow.
/// Exact no-ops return the ready generation without creating staging state.
pub(crate) fn run_inventory_refresh(
    connection: &Connection,
    input: ArchaeologyInventoryRefreshRun<'_>,
) -> Result<ArchaeologyInventoryRefreshOutcome, String> {
    if let Some(ready) =
        ready_head_is_exact_noop(connection, input.repository_root, input.cancellation)?
    {
        return Ok(ArchaeologyInventoryRefreshOutcome {
            plan_identity: "no-op:ready-generation".into(),
            effective_generation_id: ready,
            reused_ready_generation: true,
            mode: ArchaeologyInputInvalidationMode::NoOp,
            changed_paths: Vec::new(),
            next_stage: ArchaeologyJobStage::Idle,
        });
    }
    let delta = ready_delta_inventory(
        connection,
        input.repository_root,
        input.cancellation,
        input.inventory_limits,
    )?;
    let (summary, units) = if let Some(inventory) = delta {
        if inventory.source_units.len() > input.invalidation_limits.max_invalidated_paths {
            return Err("Archaeology inventory refresh source-unit bound exceeded".into());
        }
        (inventory.summary(), inventory.source_units)
    } else {
        let mut units = Vec::new();
        let summary = inventory_repository_streaming(
            input.repository_root,
            input.cancellation,
            input.inventory_limits,
            &mut |unit| {
                if units.len() >= input.invalidation_limits.max_invalidated_paths {
                    return Err("Archaeology inventory refresh source-unit bound exceeded".into());
                }
                units.push(unit);
                Ok(())
            },
        )?;
        (summary, units)
    };
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
              created_at,updated_at)
             VALUES (?1,?2,?3,?4,NULL,?5,?5)
             ON CONFLICT(repository_id) DO UPDATE SET
               repo_path=excluded.repo_path,source_identity=excluded.source_identity,
               current_revision=excluded.current_revision,updated_at=excluded.updated_at",
            params![
                summary.repository.repository_id,
                input
                    .repository_root
                    .canonicalize()
                    .map_err(|error| format!("Resolve archaeology repository: {error}"))?
                    .to_string_lossy(),
                summary.repository.source_identity,
                summary.repository.revision_sha,
                input.now,
            ],
        )
        .map_err(|error| format!("Register archaeology repository inventory: {error}"))?;
    let generation_inputs = production_generation_inputs(
        &summary.repository.revision_sha,
        &summary.policy_version,
        &summary.config_identity,
    );
    if let Some(ready) = ready_inventory_is_exact_noop(
        connection,
        &summary.repository.repository_id,
        &units,
        &generation_inputs,
        input.invalidation_limits,
    )? {
        return Ok(ArchaeologyInventoryRefreshOutcome {
            plan_identity: "no-op:ready-generation".into(),
            effective_generation_id: ready,
            reused_ready_generation: true,
            mode: ArchaeologyInputInvalidationMode::NoOp,
            changed_paths: Vec::new(),
            next_stage: ArchaeologyJobStage::Idle,
        });
    }
    let identity = ArchaeologyGenerationIdentity {
        revision_sha: &summary.repository.revision_sha,
        source: &summary.repository.source_identity,
        parser: PRODUCTION_PARSER_MANIFEST,
        algorithm: PRODUCTION_ALGORITHM_IDENTITY,
        config: &summary.config_identity,
    };
    let job = NewArchaeologyJob {
        job_id: input.job_id,
        repository_id: &summary.repository.repository_id,
        generation_id: input.generation_id,
        owner_id: input.owner_id,
        identity,
        total_units: Some(summary.coverage.discovered_source_units),
        now: input.now,
    };
    start_job(connection, job.clone())?;
    let coverage_json = serde_json::to_string(&summary.coverage)
        .map_err(|error| format!("Serialize archaeology inventory coverage: {error}"))?;
    connection
        .execute(
            "UPDATE archaeology_generations SET coverage_json=?2
             WHERE generation_id=?1 AND repository_id=?3 AND status='staging'",
            params![
                input.generation_id,
                coverage_json,
                summary.repository.repository_id
            ],
        )
        .map_err(|error| format!("Persist archaeology inventory coverage: {error}"))?;
    prepare_incremental_refresh(
        connection,
        ArchaeologyInventoryRefreshStage {
            job_id: job.job_id,
            repository_id: job.repository_id,
            generation_id: job.generation_id,
            owner_id: job.owner_id,
            identity: job.identity,
            units: &units,
            generation_inputs: &generation_inputs,
            cancellation: input.cancellation,
            limits: input.invalidation_limits,
            now: job.now,
        },
    )
}

/// Load only the prior manifest metadata needed to prove a delta inventory is
/// safe. Any missing v2 proof returns `None` and preserves the full scan.
pub(crate) fn ready_delta_inventory(
    connection: &Connection,
    repository_root: &Path,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
) -> Result<Option<super::inventory::ArchaeologyRepositoryInventory>, String> {
    if cancellation.is_cancelled() {
        return Err("Archaeology inventory cancelled".into());
    }
    let canonical = repository_root
        .canonicalize()
        .map_err(|error| format!("Resolve archaeology repository: {error}"))?;
    let repo_path = canonical.to_string_lossy();
    let ready = connection
        .query_row(
            "SELECT repository.repository_id,generation.generation_id,generation.revision_sha,
                    generation.config_identity
             FROM archaeology_repositories repository
             JOIN archaeology_generations generation
               ON generation.generation_id=repository.ready_generation_id
              AND generation.repository_id=repository.repository_id
             WHERE repository.repo_path=?1 AND generation.status='ready'",
            [repo_path.as_ref()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology delta inventory candidate: {error}"))?;
    let Some((repository_id, generation_id, revision_sha, config_identity)) = ready else {
        return Ok(None);
    };
    let inputs = load_generation_inputs(connection, &repository_id, &generation_id)?;
    if !inputs.iter().any(|input| {
        matches!(
            input.kind,
            super::invalidation::ArchaeologyGenerationInputKind::Ignore
        ) && input.scope.is_none()
            && input.identity == INVENTORY_POLICY_VERSION
    }) {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(
            "SELECT source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
                    change_identity,language,dialect,classification,byte_count,line_count,
                    coverage_json
             FROM archaeology_source_units WHERE generation_id=?1 ORDER BY path_identity",
        )
        .map_err(|error| format!("Prepare archaeology delta manifest: {error}"))?;
    let rows = statement
        .query_map([&generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|error| format!("Query archaeology delta manifest: {error}"))?;
    let mut units = Vec::new();
    for row in rows {
        let (
            source_unit_id,
            path_identity,
            relative_path,
            content_hash,
            hash_algorithm,
            change_identity,
            language,
            dialect,
            classification,
            byte_count,
            line_count,
            coverage_json,
        ) = row.map_err(|error| format!("Read archaeology delta manifest: {error}"))?;
        let coverage: Value = serde_json::from_str(&coverage_json)
            .map_err(|_| "Stored archaeology inventory coverage is invalid")?;
        let Some(reasons) = coverage.get("inventory_reasons").and_then(Value::as_array) else {
            return Ok(None);
        };
        let mut coverage_reasons = Vec::with_capacity(reasons.len());
        for reason in reasons {
            let Some(reason) = reason.as_str() else {
                return Ok(None);
            };
            if reason.len() > MAX_COVERAGE_REASON_BYTES {
                return Ok(None);
            }
            coverage_reasons.push(reason.to_string());
        }
        if byte_count < 0 || line_count < 0 {
            return Ok(None);
        }
        units.push(ArchaeologyInventoryUnit {
            identity: ArchaeologySourceUnitIdentity {
                source_unit_id,
                repository_id: repository_id.clone(),
                revision_sha: revision_sha.clone(),
                path_identity,
                relative_path,
                content_hash,
                hash_algorithm,
                change_identity,
            },
            classification: parse_enum(&classification, "source classification")?,
            language,
            dialect,
            byte_count: byte_count as u64,
            line_count: line_count as u64,
            include_candidates: Vec::new(),
            coverage_reasons,
        });
        if units.len() > limits.max_files {
            return Ok(None);
        }
    }
    inventory_repository_delta(
        &canonical,
        &revision_sha,
        &config_identity,
        &units,
        cancellation,
        limits,
    )
}

fn ready_head_is_exact_noop(
    connection: &Connection,
    repository_root: &Path,
    cancellation: &StructuralGraphCancellation,
) -> Result<Option<String>, String> {
    if cancellation.is_cancelled() {
        return Err("Archaeology inventory cancelled".into());
    }
    let canonical = repository_root
        .canonicalize()
        .map_err(|error| format!("Resolve archaeology repository: {error}"))?;
    let revision_sha = git_head(&canonical)?;
    let repo_path = canonical.to_string_lossy();
    let ready = connection
        .query_row(
            "SELECT repository.repository_id,generation.generation_id,generation.config_identity
             FROM archaeology_repositories repository
             JOIN archaeology_generations generation
               ON generation.generation_id=repository.ready_generation_id
              AND generation.repository_id=repository.repository_id
             WHERE repository.repo_path=?1 AND repository.current_revision=?2
               AND generation.revision_sha=?2 AND generation.status='ready'",
            params![repo_path.as_ref(), revision_sha],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology ready HEAD generation: {error}"))?;
    let Some((repository_id, generation_id, config_identity)) = ready else {
        return Ok(None);
    };
    let current_inputs =
        production_generation_inputs(&revision_sha, INVENTORY_POLICY_VERSION, &config_identity);
    if !generation_inputs_match(connection, &repository_id, &generation_id, &current_inputs)? {
        return Ok(None);
    }
    if cancellation.is_cancelled() {
        return Err("Archaeology inventory cancelled".into());
    }
    Ok(Some(generation_id))
}

pub(crate) fn production_generation_inputs(
    revision_sha: &str,
    inventory_policy: &str,
    config_identity: &str,
) -> Vec<ArchaeologyGenerationInput> {
    use super::invalidation::ArchaeologyGenerationInputKind as Kind;
    vec![
        ArchaeologyGenerationInput {
            kind: Kind::Head,
            scope: None,
            identity: revision_sha.into(),
        },
        ArchaeologyGenerationInput {
            kind: Kind::Ignore,
            scope: None,
            identity: inventory_policy.into(),
        },
        ArchaeologyGenerationInput {
            kind: Kind::Config,
            scope: None,
            identity: config_identity.into(),
        },
        ArchaeologyGenerationInput {
            kind: Kind::Parser,
            scope: Some("global".into()),
            identity: PRODUCTION_PARSER_MANIFEST.into(),
        },
        ArchaeologyGenerationInput {
            kind: Kind::Schema,
            scope: None,
            identity: format!("schema:v{ARCHAEOLOGY_STORAGE_SCHEMA_VERSION}"),
        },
        ArchaeologyGenerationInput {
            kind: Kind::Algorithm,
            scope: None,
            identity: PRODUCTION_ALGORITHM_IDENTITY.into(),
        },
        ArchaeologyGenerationInput {
            kind: Kind::SynthesisPolicy,
            scope: Some("global".into()),
            identity: PRODUCTION_SYNTHESIS_IDENTITY.into(),
        },
    ]
}

fn generation_inputs_match(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    generation_inputs: &[ArchaeologyGenerationInput],
) -> Result<bool, String> {
    let mut prior_inputs = load_generation_inputs(connection, repository_id, generation_id)?;
    let mut current_inputs = generation_inputs.to_vec();
    let sort_inputs = |items: &mut Vec<ArchaeologyGenerationInput>| {
        items.sort_by(|left, right| {
            (left.kind, left.scope.as_deref(), left.identity.as_str()).cmp(&(
                right.kind,
                right.scope.as_deref(),
                right.identity.as_str(),
            ))
        });
    };
    sort_inputs(&mut prior_inputs);
    sort_inputs(&mut current_inputs);
    Ok(prior_inputs == current_inputs)
}

fn ready_inventory_is_exact_noop(
    connection: &Connection,
    repository_id: &str,
    units: &[ArchaeologyInventoryUnit],
    generation_inputs: &[ArchaeologyGenerationInput],
    limits: ArchaeologyInvalidationLimits,
) -> Result<Option<String>, String> {
    let ready = connection
        .query_row(
            "SELECT ready_generation_id FROM archaeology_repositories WHERE repository_id=?1",
            [repository_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| format!("Load archaeology ready generation: {error}"))?;
    let Some(ready) = ready else { return Ok(None) };
    if !generation_inputs_match(connection, repository_id, &ready, generation_inputs)? {
        return Ok(None);
    }
    if units.len() > limits.max_invalidated_paths {
        return Err("Archaeology inventory refresh source-unit bound exceeded".into());
    }
    // Persisted dialect is the adapter's resolved value, while inventory may
    // hold a broader candidate label. Content plus parser/config identities
    // already determine that resolution, so dialect is not an inventory
    // no-op input.
    type ManifestValue = (
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
    );
    let current = units
        .iter()
        .map(|unit| {
            Ok((
                unit.identity.path_identity.clone(),
                (
                    unit.identity.content_hash.clone(),
                    unit.identity.hash_algorithm.clone(),
                    unit.identity.change_identity.clone(),
                    unit.language.clone(),
                    source_classification_name(&unit.classification)?.to_string(),
                ),
            ))
        })
        .collect::<Result<BTreeMap<String, ManifestValue>, String>>()?;
    if current.len() != units.len() {
        return Err("Archaeology inventory contains duplicate path identities".into());
    }
    let limit = i64::try_from(limits.max_invalidated_paths.saturating_add(1))
        .map_err(|_| "Archaeology inventory comparison bound overflowed")?;
    let mut statement = connection.prepare(
        "SELECT path_identity,content_hash,hash_algorithm,change_identity,language,classification
         FROM archaeology_source_units WHERE generation_id=?1 ORDER BY path_identity LIMIT ?2",
    ).map_err(|error| format!("Prepare archaeology ready inventory: {error}"))?;
    let rows = statement
        .query_map(params![ready, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ),
            ))
        })
        .map_err(|error| format!("Query archaeology ready inventory: {error}"))?;
    let mut prior = BTreeMap::new();
    for row in rows {
        let (path, value) =
            row.map_err(|error| format!("Read archaeology ready inventory: {error}"))?;
        if prior.insert(path, value).is_some() {
            return Err("Archaeology ready inventory contains duplicate path identities".into());
        }
    }
    Ok((prior == current).then_some(ready))
}

/// Materialize the current inventory, classify it against the ready catalog,
/// clone exact unaffected facts, and checkpoint the Parse/Link transition.
/// Parsing must consume the persisted refresh work selection after this call.
pub(crate) fn prepare_incremental_refresh(
    connection: &Connection,
    input: ArchaeologyInventoryRefreshStage<'_>,
) -> Result<ArchaeologyInventoryRefreshOutcome, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    if input.cancellation.is_cancelled() {
        return Err("Archaeology inventory refresh cancelled".into());
    }
    if input.units.len() > input.limits.max_invalidated_paths {
        return Err("Archaeology inventory refresh source-unit bound exceeded".into());
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("Start archaeology inventory refresh transaction: {error}"))?;
    let job = load_job(&transaction, input.job_id)?;
    if job.repository_id.as_deref() != Some(input.repository_id)
        || job.generation_id.as_deref() != Some(input.generation_id)
        || job.owner_id.as_deref() != Some(input.owner_id)
        || job.stage != ArchaeologyJobStage::Inventory
        || job.state != ArchaeologyJobState::Running
        || job.cancellation_requested
    {
        return Err("Archaeology inventory refresh lost its job lease".into());
    }
    transaction
        .execute(
            "DELETE FROM archaeology_source_units WHERE generation_id=?1",
            [input.generation_id],
        )
        .map_err(|error| format!("Reset archaeology inventory manifest: {error}"))?;
    for unit in input.units {
        if input.cancellation.is_cancelled() {
            return Err("Archaeology inventory refresh cancelled".into());
        }
        if unit.identity.repository_id != input.repository_id
            || unit.identity.revision_sha != input.identity.revision_sha
        {
            return Err("Archaeology inventory unit is outside generation scope".into());
        }
        let coverage = serde_json::to_string(&serde_json::json!({
            "state": "inventory_only",
            "reasons": unit.coverage_reasons,
            "inventory_reasons": unit.coverage_reasons,
        }))
        .map_err(|error| format!("Serialize archaeology inventory coverage: {error}"))?;
        transaction
            .execute(
                "INSERT INTO archaeology_source_units
                 (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                  hash_algorithm,change_identity,language,dialect,parser_id,parser_version,
                  classification,byte_count,line_count,coverage_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'inventory:pending','0',?10,?11,?12,?13)",
                params![
                    input.generation_id,
                    unit.identity.source_unit_id,
                    unit.identity.path_identity,
                    unit.identity.relative_path,
                    unit.identity.content_hash,
                    unit.identity.hash_algorithm,
                    unit.identity.change_identity,
                    unit.language,
                    unit.dialect,
                    source_classification_name(&unit.classification)?,
                    i64::try_from(unit.byte_count)
                        .map_err(|_| "Archaeology inventory byte count overflowed")?,
                    i64::try_from(unit.line_count)
                        .map_err(|_| "Archaeology inventory line count overflowed")?,
                    coverage,
                ],
            )
            .map_err(|error| format!("Persist archaeology inventory unit: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology inventory manifest: {error}"))?;
    profile_archaeology_stage(profiling, "inventory.persist_manifest", started);

    persist_generation_invalidation_metadata(
        connection,
        input.repository_id,
        input.generation_id,
        input.generation_inputs,
        input.cancellation,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "inventory.inputs", started);
    let changed_paths = changed_source_paths(
        connection,
        input.repository_id,
        input.generation_id,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "inventory.changed_paths", started);
    let plan = plan_generation_invalidation(
        connection,
        input.repository_id,
        input.generation_id,
        &changed_paths,
        input.cancellation,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "inventory.invalidation", started);
    if plan.decision.mode == ArchaeologyInputInvalidationMode::NoOp {
        let ready = plan
            .prior_ready_generation_id
            .clone()
            .ok_or("Archaeology no-op refresh has no ready generation")?;
        let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("Start archaeology no-op cleanup: {error}"))?;
        let deleted_job = transaction
            .execute(
                "DELETE FROM archaeology_jobs
                 WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
                   AND stage='inventory' AND state='running' AND cancellation_requested=0",
                params![
                    input.job_id,
                    input.repository_id,
                    input.generation_id,
                    input.owner_id
                ],
            )
            .map_err(|error| format!("Remove archaeology no-op job: {error}"))?;
        if deleted_job != 1 {
            return Err("Archaeology no-op refresh lost its job lease".into());
        }
        let deleted_generation = transaction
            .execute(
                "DELETE FROM archaeology_generations
                 WHERE generation_id=?1 AND repository_id=?2 AND status='staging'",
                params![input.generation_id, input.repository_id],
            )
            .map_err(|error| format!("Remove archaeology no-op generation: {error}"))?;
        if deleted_generation != 1 {
            return Err("Archaeology no-op staging generation did not reconcile".into());
        }
        transaction
            .commit()
            .map_err(|error| format!("Commit archaeology no-op cleanup: {error}"))?;
        return Ok(ArchaeologyInventoryRefreshOutcome {
            plan_identity: "no-op:ready-generation".into(),
            effective_generation_id: ready,
            reused_ready_generation: true,
            mode: plan.decision.mode,
            changed_paths,
            next_stage: ArchaeologyJobStage::Idle,
        });
    }
    clone_unaffected_ready_facts(connection, input.repository_id, input.generation_id, &plan)?;
    profile_archaeology_stage(profiling, "inventory.clone_facts", started);
    let plan_identity = persist_refresh_work_plan(
        connection,
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &plan,
    )?;
    profile_archaeology_stage(profiling, "inventory.persist_work", started);
    let next_stage = if matches!(
        plan.decision.mode,
        ArchaeologyInputInvalidationMode::NoOp | ArchaeologyInputInvalidationMode::SynthesisOnly
    ) {
        ArchaeologyJobStage::Link
    } else {
        ArchaeologyJobStage::Parse
    };
    let checkpoint = ArchaeologyJobCheckpoint {
        cursor_identity: Some(plan_identity.clone()),
        counters: BTreeMap::from([
            ("inventory_complete".into(), 1),
            ("refresh_changed_paths".into(), changed_paths.len() as u64),
        ]),
        ..Default::default()
    };
    let checkpoint_json = serde_json::to_string(&checkpoint)
        .map_err(|error| format!("Encode archaeology refresh checkpoint: {error}"))?;
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs SET stage=?6,checkpoint_identity=?5,
                 checkpoint_json=?7,updated_at=?8
             WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
               AND stage='inventory' AND state='running' AND cancellation_requested=0",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                plan_identity,
                stage_name(&next_stage),
                checkpoint_json,
                input.now,
            ],
        )
        .map_err(|error| format!("Checkpoint archaeology refresh plan: {error}"))?;
    if changed != 1 {
        return Err("Archaeology refresh plan lost its job lease".into());
    }
    Ok(ArchaeologyInventoryRefreshOutcome {
        plan_identity,
        effective_generation_id: input.generation_id.into(),
        reused_ready_generation: false,
        mode: plan.decision.mode,
        changed_paths,
        next_stage,
    })
}

pub(crate) fn execute_incremental_parse_batch(
    connection: &Connection,
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
    plan_identity: &str,
    max_items: usize,
    now: &str,
    cancellation: &StructuralGraphCancellation,
    execute: impl FnMut(&Transaction<'_>, &ArchaeologyRefreshWorkItem) -> Result<(), String>,
) -> Result<ArchaeologyRefreshExecution, String> {
    let execution = execute_refresh_parse_work_batch(
        connection,
        job_id,
        repository_id,
        generation_id,
        owner_id,
        plan_identity,
        max_items,
        now,
        cancellation,
        execute,
    )?;
    if execution.remaining == 0 {
        let changed = connection
            .execute(
                "UPDATE archaeology_jobs SET stage='link',updated_at=?5
                 WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
                  AND stage='parse' AND state='running' AND cancellation_requested=0",
                params![job_id, repository_id, generation_id, owner_id, now],
            )
            .map_err(|error| format!("Complete archaeology incremental parse: {error}"))?;
        if changed != 1 {
            return Err("Archaeology incremental parse lost its job lease".into());
        }
    }
    Ok(execution)
}

pub(crate) fn execute_incremental_parse_and_link_batch(
    connection: &Connection,
    input: ArchaeologyLinkStage<'_>,
    plan_identity: &str,
    max_items: usize,
    execute: impl FnMut(&Transaction<'_>, &ArchaeologyRefreshWorkItem) -> Result<(), String>,
) -> Result<(ArchaeologyRefreshExecution, Option<ArchaeologyJobStatus>), String> {
    let execution = execute_incremental_parse_batch(
        connection,
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        plan_identity,
        max_items,
        input.now,
        input.cancellation,
        execute,
    )?;
    let linked = if execution.remaining == 0 {
        Some(link_generation(connection, input)?)
    } else {
        None
    };
    Ok((execution, linked))
}

/// Resolve one persisted parser generation and publish its linker patch in the
/// same owner-checked transaction as the Link -> Derive checkpoint.
pub(crate) fn link_generation(
    connection: &Connection,
    input: ArchaeologyLinkStage<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".to_string());
    }
    let sqlite_cancellation = input.cancellation.clone();
    connection.progress_handler(2_048, Some(move || sqlite_cancellation.is_cancelled()));
    let _sqlite_progress = ArchaeologySqliteProgress(connection);
    let manifest = parse_parser_manifest(input.identity.parser)?;
    let receipt = digest_identity(
        format!(
            "{}\0{}\0{}\0{}",
            input.repository_id,
            input.generation_id,
            input.identity.revision_sha,
            input.identity.parser
        )
        .as_bytes(),
        "archaeology-link:v1:",
    );
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology link transaction: {error}"))?;
    let (stage, checkpoint, completed, total): (String, Option<String>, i64, Option<i64>) =
        transaction
            .query_row(
                "SELECT job.stage,job.checkpoint_identity,job.completed_units,job.total_units
                 FROM archaeology_jobs job JOIN archaeology_generations generation
                   ON generation.generation_id=job.generation_id
                 WHERE job.job_id=?1 AND job.repository_id=?2 AND job.generation_id=?3
                   AND job.owner_id=?4 AND job.state='running'
                   AND job.stage IN ('link','derive') AND job.cancellation_requested=0
                   AND generation.repository_id=?2 AND generation.status='staging'
                   AND generation.revision_sha=?5 AND generation.source_identity=?6
                   AND generation.parser_identity=?7 AND generation.algorithm_identity=?8
                   AND generation.config_identity=?9 AND generation.schema_version=?10
                   AND julianday(?11)>=julianday(job.updated_at)",
                params![
                    input.job_id,
                    input.repository_id,
                    input.generation_id,
                    input.owner_id,
                    input.identity.revision_sha,
                    input.identity.source,
                    input.identity.parser,
                    input.identity.algorithm,
                    input.identity.config,
                    ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                    input.now,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| format!("Load owned archaeology link stage: {error}"))?
            .ok_or_else(|| cas_error("link", input.job_id))?;
    if stage == "derive" && checkpoint.as_deref() == Some(receipt.as_str()) {
        return load_job(&transaction, input.job_id);
    }
    if stage != "link" {
        return Err(cas_error("link", input.job_id));
    }

    let (unit_count, fact_count, edge_count, span_count, evidence_count, input_bytes): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = transaction
        .query_row(
            "SELECT (SELECT COUNT(*) FROM archaeology_source_units WHERE generation_id=?1),
                    (SELECT COUNT(*) FROM archaeology_facts WHERE generation_id=?1),
                    (SELECT COUNT(*) FROM archaeology_fact_edges WHERE generation_id=?1),
                    (SELECT COUNT(*) FROM archaeology_source_spans WHERE generation_id=?1),
                    (SELECT COUNT(*) FROM archaeology_evidence_links WHERE generation_id=?1),
                    (SELECT COALESCE(SUM(LENGTH(CAST(source_unit_id AS BLOB))+LENGTH(CAST(language AS BLOB))+
                        LENGTH(CAST(COALESCE(dialect,'') AS BLOB))+LENGTH(CAST(COALESCE(relative_path,'') AS BLOB))+
                        LENGTH(CAST(parser_id AS BLOB))+LENGTH(CAST(parser_version AS BLOB))+
                        LENGTH(CAST(include_lineage_json AS BLOB))+64),0)
                       FROM archaeology_source_units WHERE generation_id=?1)
                    +(SELECT COALESCE(SUM(LENGTH(CAST(fact_id AS BLOB))+LENGTH(CAST(kind AS BLOB))+
                        LENGTH(CAST(label AS BLOB))+LENGTH(CAST(parser_id AS BLOB))+LENGTH(CAST(trust AS BLOB))+
                        LENGTH(CAST(confidence AS BLOB))+LENGTH(CAST(attributes_json AS BLOB))+64),0)
                       FROM archaeology_facts WHERE generation_id=?1)
                    +(SELECT COALESCE(SUM(LENGTH(CAST(edge_id AS BLOB))+LENGTH(CAST(from_fact_id AS BLOB))+
                        LENGTH(CAST(to_fact_id AS BLOB))+LENGTH(CAST(kind AS BLOB))+LENGTH(CAST(trust AS BLOB))+
                        LENGTH(CAST(COALESCE(unresolved_reason,'') AS BLOB))+64),0)
                       FROM archaeology_fact_edges WHERE generation_id=?1)
                    +(SELECT COALESCE(SUM(LENGTH(CAST(link.owner_kind AS BLOB))+LENGTH(CAST(link.owner_id AS BLOB))+
                        LENGTH(CAST(link.evidence_id AS BLOB))+LENGTH(CAST(span.span_id AS BLOB))+
                        LENGTH(CAST(span.source_unit_id AS BLOB))+LENGTH(CAST(span.revision_sha AS BLOB))+160),0)
                       FROM archaeology_evidence_links link JOIN archaeology_source_spans span
                         ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
                      WHERE link.generation_id=?1 AND link.evidence_kind='span')",
            [input.generation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|error| format!("Count archaeology link input: {error}"))?;
    if count_exceeds(unit_count, input.limits.max_units)
        || count_exceeds(fact_count, input.limits.max_facts)
        || count_exceeds(edge_count, input.limits.max_edges)
        || count_exceeds(
            span_count.saturating_add(evidence_count),
            input.limits.max_output_items,
        )
        || count_exceeds(input_bytes, input.limits.max_input_bytes)
    {
        return Err("Archaeology linker persisted input bound exceeded".to_string());
    }

    let units: Vec<PersistedLinkUnit> = query_generation_json(
        &transaction,
        input.generation_id,
        "SELECT json_object('source_unit_id',source_unit_id,'language',language,
            'dialect',dialect,'relative_path',relative_path,'parser_id',parser_id,
            'parser_version',parser_version,'lineage',json(include_lineage_json))
         FROM archaeology_source_units WHERE generation_id=?1
           AND classification NOT IN ('protected','opaque') ORDER BY source_unit_id",
        "link units",
        "Archaeology linker cancelled",
        input.cancellation,
    )?;
    for unit in &units {
        if manifest.get(&unit.parser_id).map(String::as_str) != Some(unit.parser_version.as_str()) {
            return Err("Archaeology link unit parser is outside the generation manifest".into());
        }
        if let Some(path) = unit.relative_path.as_deref() {
            validate_persisted_path("link relative path", path)?;
        }
        validate_metadata_values(&unit.source_unit_id, &unit.lineage)?;
    }
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".into());
    }

    let facts: Vec<PersistedLinkFact> = query_generation_json(&transaction,input.generation_id,
        "SELECT json_object('source_unit_id',MIN(span.source_unit_id),'fact',json_object(
            'fact_id',fact.fact_id,'kind',fact.kind,'label',fact.label,'span_ids',json_group_array(span.span_id),
            'parser_id',fact.parser_id,'trust',fact.trust,'confidence',fact.confidence,
            'attributes',json(fact.attributes_json)),'evidence_spans',json_group_array(json_object(
            'span_id',span.span_id,'source_unit_id',span.source_unit_id,'revision_sha',span.revision_sha,
            'start',json_object('byte',span.start_byte,'line',span.start_line,'column',span.start_column),
            'end',json_object('byte',span.end_byte,'line',span.end_line,'column',span.end_column))))
         FROM archaeology_facts fact JOIN archaeology_evidence_links evidence
           ON evidence.generation_id=fact.generation_id AND evidence.owner_kind='fact'
          AND evidence.owner_id=fact.fact_id AND evidence.evidence_kind='span' AND evidence.role='supporting'
         JOIN archaeology_source_spans span ON span.generation_id=evidence.generation_id AND span.span_id=evidence.evidence_id
         WHERE fact.generation_id=?1 GROUP BY fact.fact_id HAVING COUNT(DISTINCT span.source_unit_id)=1 ORDER BY fact.fact_id",
        "link facts", "Archaeology linker cancelled", input.cancellation)?;
    if facts.len() != fact_count as usize {
        return Err("Archaeology link facts lack single-unit exact evidence".into());
    }
    for item in &facts {
        if !manifest.contains_key(&item.fact.parser_id) || fact_contains_secret(&item.fact) {
            return Err("Archaeology link fact violates parser or privacy scope".into());
        }
        for span in &item.evidence_spans {
            span.validate()?;
            if span.revision_sha != input.identity.revision_sha
                || span.source_unit_id != item.source_unit_id
            {
                return Err("Archaeology link evidence scope changed".into());
            }
        }
    }
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".into());
    }

    let edges: Vec<ArchaeologyFactEdge> = query_generation_json(&transaction,input.generation_id,
        "SELECT json_object('edge_id',edge.edge_id,'from_fact_id',edge.from_fact_id,
            'to_fact_id',edge.to_fact_id,'kind',edge.kind,'trust',edge.trust,
            'evidence_span_ids',json_group_array(span.span_id),'unresolved_reason',edge.unresolved_reason)
         FROM archaeology_fact_edges edge JOIN archaeology_evidence_links evidence
           ON evidence.generation_id=edge.generation_id AND evidence.owner_kind='fact_edge'
          AND evidence.owner_id=edge.edge_id AND evidence.evidence_kind='span' AND evidence.role='supporting'
         JOIN archaeology_source_spans span ON span.generation_id=evidence.generation_id AND span.span_id=evidence.evidence_id
          AND span.revision_sha=(SELECT revision_sha FROM archaeology_generations WHERE generation_id=?1)
         WHERE edge.generation_id=?1 GROUP BY edge.edge_id ORDER BY edge.edge_id","link edges", "Archaeology linker cancelled", input.cancellation)?;
    if edges.len() != edge_count as usize {
        return Err("Archaeology link edges lack exact evidence".into());
    }
    if edges
        .iter()
        .filter_map(|edge| edge.unresolved_reason.as_deref())
        .any(|value| looks_like_secret(value) || contains_sensitive_path(value))
    {
        return Err("Archaeology link edge violates privacy scope".into());
    }
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".into());
    }

    let unit_views = units
        .iter()
        .map(|unit| ArchaeologyLinkUnit {
            source_unit_id: &unit.source_unit_id,
            language: &unit.language,
            dialect: unit.dialect.as_deref(),
            relative_path: unit.relative_path.as_deref(),
            lineage: &unit.lineage,
        })
        .collect::<Vec<_>>();
    let fact_views = facts
        .iter()
        .map(|item| ArchaeologyLinkFact {
            source_unit_id: &item.source_unit_id,
            fact: &item.fact,
            evidence_spans: &item.evidence_spans,
        })
        .collect::<Vec<_>>();
    let patch = link_archaeology_facts(
        input.repository_id,
        input.identity.revision_sha,
        &unit_views,
        &fact_views,
        &edges,
        input.cancellation,
        input.limits,
    )?;
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".into());
    }
    persist_link_patch(&transaction, input.generation_id, &units, &patch)?;
    if input.cancellation.is_cancelled() {
        return Err("Archaeology linker cancelled".into());
    }
    let checkpoint = ArchaeologyJobCheckpoint {
        cursor_identity: Some(receipt.clone()),
        counters: BTreeMap::from([
            ("link_complete".into(), 1),
            ("linked_facts".into(), patch.upsert_facts.len() as u64),
            ("linked_edges".into(), patch.upsert_edges.len() as u64),
        ]),
        ..Default::default()
    };
    let checkpoint_json = serde_json::to_string(&checkpoint).map_err(|error| error.to_string())?;
    let changed=transaction.execute(
        "UPDATE archaeology_jobs SET stage='derive',checkpoint_identity=?5,checkpoint_json=?6,updated_at=?7
         WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
          AND state='running' AND stage='link' AND cancellation_requested=0
          AND completed_units=?8 AND (total_units IS ?9 OR total_units=?9)",
        params![input.job_id,input.repository_id,input.generation_id,input.owner_id,receipt,checkpoint_json,input.now,completed,total]
    ).map_err(|error| format!("Checkpoint archaeology link: {error}"))?;
    require_cas(changed, "link checkpoint", input.job_id)?;
    let status = load_job(&transaction, input.job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology link: {error}"))?;
    Ok(status)
}

/// Derive bounded evidence packets in memory, render deterministic candidate
/// rules, and publish the complete rule/evidence replacement with the
/// Derive -> Synthesize checkpoint. Evidence packets are deliberately not
/// cached: persisted facts and edges remain the source of truth on retry.
pub(crate) fn derive_template_candidates(
    connection: &Connection,
    input: ArchaeologyDeriveStage<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let stage_started = Instant::now();
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    if input.cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let manifest = parse_parser_manifest(input.identity.parser)?;
    let receipt = digest_identity(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            input.repository_id,
            input.generation_id,
            input.identity.revision_sha,
            input.identity.parser,
            input.identity.algorithm,
            input.identity.config
        )
        .as_bytes(),
        "archaeology-derive-cluster:v1:",
    );
    let sqlite_cancellation = input.cancellation.clone();
    connection.progress_handler(2_048, Some(move || sqlite_cancellation.is_cancelled()));
    let _sqlite_progress = ArchaeologySqliteProgress(connection);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology derivation transaction: {error}"))?;
    let (stage, checkpoint, completed, total, coverage_json): (
        String,
        Option<String>,
        i64,
        Option<i64>,
        String,
    ) = transaction
        .query_row(
            "SELECT job.stage,job.checkpoint_identity,job.completed_units,job.total_units,
                    generation.coverage_json
             FROM archaeology_jobs job JOIN archaeology_generations generation
               ON generation.generation_id=job.generation_id
             WHERE job.job_id=?1 AND job.repository_id=?2 AND job.generation_id=?3
               AND job.owner_id=?4 AND job.state='running'
               AND job.stage IN ('derive','synthesize') AND job.cancellation_requested=0
               AND generation.repository_id=?2 AND generation.status='staging'
               AND generation.revision_sha=?5 AND generation.source_identity=?6
               AND generation.parser_identity=?7 AND generation.algorithm_identity=?8
               AND generation.config_identity=?9 AND generation.schema_version=?10
               AND julianday(?11)>=julianday(job.updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                input.now,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load owned archaeology derive stage: {error}"))?
        .ok_or_else(|| cas_error("derive", input.job_id))?;
    if stage == "synthesize" && checkpoint.as_deref() == Some(receipt.as_str()) {
        return load_job(&transaction, input.job_id);
    }
    if stage != "derive" {
        return Err(cas_error("derive", input.job_id));
    }
    let coverage = parse_coverage(&coverage_json, "derivation generation")?;

    let (fact_count, edge_count, input_bytes): (i64, i64, i64) = transaction
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_facts WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_fact_edges WHERE generation_id=?1),
                LENGTH(CAST(?2 AS BLOB))
                +(SELECT COALESCE(SUM(
                    LENGTH(CAST(fact_id AS BLOB))+LENGTH(CAST(kind AS BLOB))+
                    LENGTH(CAST(label AS BLOB))+LENGTH(CAST(parser_id AS BLOB))+
                    LENGTH(CAST(trust AS BLOB))+LENGTH(CAST(confidence AS BLOB))+
                    LENGTH(CAST(attributes_json AS BLOB))+64),0)
                  FROM archaeology_facts WHERE generation_id=?1)
                +(SELECT COALESCE(SUM(
                    LENGTH(CAST(edge_id AS BLOB))+LENGTH(CAST(from_fact_id AS BLOB))+
                    LENGTH(CAST(to_fact_id AS BLOB))+LENGTH(CAST(kind AS BLOB))+
                    LENGTH(CAST(trust AS BLOB))+
                    LENGTH(CAST(COALESCE(unresolved_reason,'') AS BLOB))+64),0)
                  FROM archaeology_fact_edges WHERE generation_id=?1)
                +(SELECT COALESCE(SUM(
                    LENGTH(CAST(owner_id AS BLOB))+LENGTH(CAST(evidence_id AS BLOB))+32),0)
                  FROM archaeology_evidence_links
                  WHERE generation_id=?1 AND owner_kind IN ('fact','fact_edge')
                    AND evidence_kind='span' AND role='supporting')
                +(SELECT COALESCE(SUM(
                    LENGTH(CAST(fact_id AS BLOB))+LENGTH(CAST(source_unit_id AS BLOB))+
                    LENGTH(CAST(path_identity AS BLOB))+LENGTH(CAST(classification AS BLOB))+32),0)
                  FROM (
                    SELECT fact.fact_id,MIN(unit.source_unit_id) source_unit_id,
                           MIN(unit.path_identity) path_identity,
                           MIN(unit.classification) classification
                    FROM archaeology_facts fact
                    JOIN archaeology_evidence_links link
                      ON link.generation_id=fact.generation_id AND link.owner_kind='fact'
                     AND link.owner_id=fact.fact_id AND link.evidence_kind='span'
                     AND link.role='supporting'
                    JOIN archaeology_source_spans span
                      ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
                    JOIN archaeology_source_units unit
                      ON unit.generation_id=span.generation_id
                     AND unit.source_unit_id=span.source_unit_id
                    WHERE fact.generation_id=?1 GROUP BY fact.fact_id
                    HAVING COUNT(DISTINCT unit.source_unit_id)=1))",
            params![input.generation_id, coverage_json],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("Count archaeology derivation input: {error}"))?;
    if count_exceeds(fact_count, input.limits.max_facts)
        || count_exceeds(edge_count, input.limits.max_edges)
        || count_exceeds(input_bytes, input.limits.max_input_bytes)
    {
        return Err("Archaeology derivation persisted input bound exceeded".into());
    }
    let facts: Vec<ArchaeologyFact> = query_generation_json(
        &transaction,
        input.generation_id,
        "WITH evidence AS (
            SELECT fact.fact_id,fact.kind,fact.label,fact.parser_id,fact.trust,fact.confidence,
                   fact.attributes_json,span.span_id
            FROM archaeology_facts fact
            JOIN archaeology_evidence_links link
              ON link.generation_id=fact.generation_id AND link.owner_kind='fact'
             AND link.owner_id=fact.fact_id AND link.evidence_kind='span'
             AND link.role='supporting'
            JOIN archaeology_source_spans span
              ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
             AND span.revision_sha=(SELECT revision_sha FROM archaeology_generations
                                    WHERE generation_id=?1)
            WHERE fact.generation_id=?1 ORDER BY fact.fact_id,span.span_id
         ), grouped AS (
            SELECT fact_id,MIN(kind) kind,MIN(label) label,MIN(parser_id) parser_id,
                   MIN(trust) trust,MIN(confidence) confidence,
                   MIN(attributes_json) attributes_json,json_group_array(span_id) span_ids
            FROM evidence GROUP BY fact_id
         )
         SELECT json_object('fact_id',fact_id,'kind',kind,'label',label,
            'span_ids',json(span_ids),'parser_id',parser_id,'trust',trust,
            'confidence',confidence,'attributes',json(attributes_json))
         FROM grouped ORDER BY fact_id",
        "derivation facts",
        "Archaeology derivation cancelled",
        input.cancellation,
    )?;
    profile_archaeology_stage(profiling, "derive.load_facts", stage_started);
    if facts.len() != fact_count as usize {
        return Err("Archaeology derivation facts lack exact supporting spans".into());
    }
    for fact in &facts {
        if !manifest.contains_key(&fact.parser_id) || fact_contains_secret(fact) {
            return Err("Archaeology derivation fact violates parser or privacy scope".into());
        }
    }
    let persisted_origins: Vec<PersistedFactOrigin> = query_generation_json(
        &transaction,
        input.generation_id,
        "WITH origins AS (
            SELECT fact.fact_id,MIN(unit.source_unit_id) source_unit_id,
                   MIN(unit.path_identity) path_identity,MIN(unit.relative_path) relative_path,
                   MIN(span.start_byte) start_byte,MAX(span.end_byte) end_byte,
                   MIN(unit.classification) classification
            FROM archaeology_facts fact
            JOIN archaeology_evidence_links link
              ON link.generation_id=fact.generation_id AND link.owner_kind='fact'
             AND link.owner_id=fact.fact_id AND link.evidence_kind='span'
             AND link.role='supporting'
            JOIN archaeology_source_spans span
              ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
             AND span.revision_sha=(SELECT revision_sha FROM archaeology_generations
                                    WHERE generation_id=?1)
            JOIN archaeology_source_units unit
              ON unit.generation_id=span.generation_id
             AND unit.source_unit_id=span.source_unit_id
            WHERE fact.generation_id=?1 GROUP BY fact.fact_id
            HAVING COUNT(DISTINCT unit.source_unit_id)=1
         )
         SELECT json_object('fact_id',fact_id,'source_unit_id',source_unit_id,
            'path_identity',path_identity,'relative_path',relative_path,
            'start_byte',start_byte,'end_byte',end_byte,'classification',classification)
         FROM origins ORDER BY fact_id",
        "derivation fact origins",
        "Archaeology derivation cancelled",
        input.cancellation,
    )?;
    if persisted_origins.len() != facts.len() {
        return Err("Archaeology derivation facts require one exact source origin".into());
    }
    let origins = persisted_origins
        .into_iter()
        .map(|origin| {
            let relative_path = origin.relative_path.ok_or_else(|| {
                "Archaeology derivation fact origin lacks a repository-relative path".to_string()
            })?;
            validate_persisted_path("derivation fact origin", &relative_path)?;
            if origin.end_byte <= origin.start_byte {
                return Err("Archaeology derivation fact origin has an invalid byte range".into());
            }
            Ok(ArchaeologyFactOrigin {
                fact_id: origin.fact_id,
                source_unit_id: origin.source_unit_id,
                path_identity: origin.path_identity,
                ranking_path_identity: stable_graph_id(
                    "archaeology-ranking-path",
                    &format!(
                        "{relative_path}\0{}\0{}",
                        origin.start_byte, origin.end_byte
                    ),
                ),
                classification: origin.classification,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    profile_archaeology_stage(profiling, "derive.load_origins", stage_started);
    let edges: Vec<ArchaeologyFactEdge> = query_generation_json(
        &transaction,
        input.generation_id,
        "WITH evidence AS (
            SELECT edge.edge_id,edge.from_fact_id,edge.to_fact_id,edge.kind,edge.trust,
                   edge.unresolved_reason,span.span_id
            FROM archaeology_fact_edges edge
            JOIN archaeology_evidence_links link
              ON link.generation_id=edge.generation_id AND link.owner_kind='fact_edge'
             AND link.owner_id=edge.edge_id AND link.evidence_kind='span'
             AND link.role='supporting'
            JOIN archaeology_source_spans span
              ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
             AND span.revision_sha=(SELECT revision_sha FROM archaeology_generations
                                    WHERE generation_id=?1)
            WHERE edge.generation_id=?1 ORDER BY edge.edge_id,span.span_id
         ), grouped AS (
            SELECT edge_id,MIN(from_fact_id) from_fact_id,MIN(to_fact_id) to_fact_id,
                   MIN(kind) kind,MIN(trust) trust,MIN(unresolved_reason) unresolved_reason,
                   json_group_array(span_id) evidence_span_ids
            FROM evidence GROUP BY edge_id
         )
         SELECT json_object('edge_id',edge_id,'from_fact_id',from_fact_id,
            'to_fact_id',to_fact_id,'kind',kind,'trust',trust,
            'evidence_span_ids',json(evidence_span_ids),
            'unresolved_reason',unresolved_reason)
         FROM grouped ORDER BY edge_id",
        "derivation edges",
        "Archaeology derivation cancelled",
        input.cancellation,
    )?;
    profile_archaeology_stage(profiling, "derive.load_edges", stage_started);
    if edges.len() != edge_count as usize {
        return Err("Archaeology derivation edges lack exact supporting spans".into());
    }
    if edges
        .iter()
        .filter_map(|edge| edge.unresolved_reason.as_deref())
        .any(|value| looks_like_secret(value) || contains_sensitive_path(value))
    {
        return Err("Archaeology derivation edge violates privacy scope".into());
    }
    if input.cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }

    let packets = derive_evidence_packets(
        input.repository_id,
        input.identity.revision_sha,
        &facts,
        &edges,
        input.cancellation,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "derive.packets", stage_started);
    let rules = render_template_rules(
        input.repository_id,
        input.generation_id,
        input.identity.revision_sha,
        &packets,
        &facts,
        &edges,
        &coverage,
        input.identity.parser,
        input.identity.algorithm,
        input.cancellation,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "derive.render", stage_started);
    let rules = cluster_evidence_compatible_rules(
        input.repository_id,
        input.identity.revision_sha,
        &rules,
        &facts,
        &edges,
        &origins,
        input.cancellation,
        input.limits,
    )?;
    profile_archaeology_stage(profiling, "derive.cluster", stage_started);
    if input.cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    persist_deterministic_rules(
        &transaction,
        input.generation_id,
        input.identity.parser,
        input.identity.algorithm,
        input.now,
        &rules,
        input.limits,
        input.cancellation,
    )?;
    profile_archaeology_stage(profiling, "derive.persist", stage_started);
    if input.cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let primary_rules = rules
        .iter()
        .filter(|rule| rule.domain_ids.as_slice() == ["domain:other"])
        .count();
    let alias_rules = rules
        .iter()
        .filter(|rule| !rule.alias_rule_ids.is_empty())
        .count();
    let conflict_references = rules
        .iter()
        .map(|rule| rule.conflict_rule_ids.len())
        .sum::<usize>();
    if primary_rules.saturating_add(alias_rules) != rules.len() || conflict_references % 2 != 0 {
        return Err("Archaeology clustered rule accounting is inconsistent".into());
    }
    let checkpoint = ArchaeologyJobCheckpoint {
        cursor_identity: Some(receipt.clone()),
        counters: BTreeMap::from([
            ("derive_complete".into(), 1),
            ("evidence_packets".into(), packets.len() as u64),
            ("deterministic_rules".into(), rules.len() as u64),
            (
                "deterministic_clauses".into(),
                rules.iter().map(|rule| rule.clauses.len() as u64).sum(),
            ),
            ("cluster_primary_rules".into(), primary_rules as u64),
            ("cluster_alias_rules".into(), alias_rules as u64),
            (
                "cluster_conflict_pairs".into(),
                (conflict_references / 2) as u64,
            ),
            ("domain_other_rules".into(), primary_rules as u64),
        ]),
        ..Default::default()
    };
    let checkpoint_json = serde_json::to_string(&checkpoint).map_err(|error| error.to_string())?;
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET stage='synthesize',checkpoint_identity=?5,checkpoint_json=?6,updated_at=?7
             WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
               AND state='running' AND stage='derive' AND cancellation_requested=0
               AND completed_units=?8 AND (total_units IS ?9 OR total_units=?9)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                receipt,
                checkpoint_json,
                input.now,
                completed,
                total,
            ],
        )
        .map_err(|error| format!("Checkpoint archaeology derivation: {error}"))?;
    require_cas(changed, "derive checkpoint", input.job_id)?;
    let status = load_job(&transaction, input.job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology derivation: {error}"))?;
    Ok(status)
}

/// Validate the finalized zero-model/model-assisted rule catalog and build its
/// exact search projection in the same owner-checked transaction as the
/// Synthesize -> Validate checkpoint. This is the only supported way across
/// that stage boundary.
pub(crate) fn finalize_synthesis_catalog(
    connection: &Connection,
    input: ArchaeologySynthesisCatalogStage<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    finalize_synthesis_catalog_impl(connection, input, None)
}

pub(crate) fn finalize_model_synthesis_catalog(
    connection: &Connection,
    input: ArchaeologySynthesisCatalogStage<'_>,
    model: ArchaeologyModelSynthesisCatalog<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    validate_synthesis_request(model.request, model.limits)?;
    validate_synthesis_response(model.request, model.response, model.limits)?;
    let canonical = canonicalize_synthesis_response(model.request, model.response, model.limits)?;
    if &canonical != model.response {
        return Err("Archaeology model synthesis response is not canonical".into());
    }
    finalize_synthesis_catalog_impl(connection, input, Some(model))
}

fn finalize_synthesis_catalog_impl(
    connection: &Connection,
    input: ArchaeologySynthesisCatalogStage<'_>,
    model: Option<ArchaeologyModelSynthesisCatalog<'_>>,
) -> Result<ArchaeologyJobStatus, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let stage_started = Instant::now();
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    synthesis_catalog_cancelled(input.cancellation)?;
    let sqlite_cancellation = input.cancellation.clone();
    connection.progress_handler(2_048, Some(move || sqlite_cancellation.is_cancelled()));
    let _sqlite_progress = ArchaeologySqliteProgress(connection);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology synthesis catalog transaction: {error}"))?;
    let (stage, checkpoint_identity, checkpoint_json, completed_units, total_units): (
        String,
        Option<String>,
        String,
        i64,
        Option<i64>,
    ) = transaction
        .query_row(
            "SELECT job.stage,job.checkpoint_identity,job.checkpoint_json,
                    job.completed_units,job.total_units
             FROM archaeology_jobs job JOIN archaeology_generations generation
               ON generation.generation_id=job.generation_id
             WHERE job.job_id=?1 AND job.repository_id=?2 AND job.generation_id=?3
               AND job.owner_id=?4 AND job.state='running'
               AND job.stage IN ('synthesize','validate')
               AND job.cancellation_requested=0
               AND generation.repository_id=?2 AND generation.status='staging'
               AND generation.revision_sha=?5 AND generation.source_identity=?6
               AND generation.parser_identity=?7 AND generation.algorithm_identity=?8
               AND generation.config_identity=?9 AND generation.schema_version=?10
               AND julianday(?11)>=julianday(job.updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                input.now,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "load job", error))?
        .ok_or_else(|| cas_error("synthesis catalog", input.job_id))?;

    if let Some(model) = model.as_ref() {
        materialize_model_synthesis(&transaction, &input, model, stage == "synthesize")?;
    }
    let rule_ids = transaction
        .prepare(
            "SELECT rule_id FROM archaeology_rules
             WHERE generation_id=?1 ORDER BY rule_id",
        )
        .and_then(|mut statement| {
            statement
                .query_map([input.generation_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "load identity rules", error)
        })?;
    let identity_count = if stage == "synthesize" {
        refresh_rule_identities(
            &transaction,
            input.generation_id,
            &rule_ids,
            input.cancellation,
        )?
    } else {
        validate_rule_identities(
            &transaction,
            input.generation_id,
            &rule_ids,
            input.cancellation,
        )?
    };
    if identity_count != rule_ids.len() {
        return Err("Archaeology synthesis identities did not reconcile".into());
    }
    profile_archaeology_stage(profiling, "synthesize.identities", stage_started);
    validate_final_rule_catalog(&transaction, &input)?;
    profile_archaeology_stage(profiling, "synthesize.validate_catalog", stage_started);
    let (manifest_rows, fts_rows): (i64, i64) = transaction
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_rule_search_manifest
                 WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?1)",
            [input.generation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "count search rows", error)
        })?;
    if manifest_rows != 0 || fts_rows != 0 {
        validate_search_integrity(&transaction, input.generation_id)?;
        validate_search_fts_parity(&transaction, input.generation_id)?;
    }

    if stage == "synthesize" {
        replace_search_manifest(&transaction, input.generation_id, input.cancellation)?;
    } else if stage != "validate" {
        return Err(cas_error("synthesis catalog", input.job_id));
    }
    validate_search_integrity(&transaction, input.generation_id)?;
    validate_search_fts_parity(&transaction, input.generation_id)?;
    profile_archaeology_stage(profiling, "synthesize.search", stage_started);
    synthesis_catalog_cancelled(input.cancellation)?;

    let receipt = synthesis_catalog_receipt(&transaction, &input)?;
    profile_archaeology_stage(profiling, "synthesize.receipt", stage_started);
    if stage == "validate" {
        if checkpoint_identity.as_deref() == Some(receipt.as_str()) {
            return load_job(&transaction, input.job_id);
        }
        return Err("Archaeology synthesis catalog changed after validation".into());
    }

    let mut checkpoint: ArchaeologyJobCheckpoint = serde_json::from_str(&checkpoint_json)
        .map_err(|_| "Stored archaeology synthesis checkpoint is invalid".to_string())?;
    let (rule_count, clause_count, domain_count): (i64, i64, i64) = transaction
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_rule_search_manifest
                 WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_rule_clauses WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_rule_domains WHERE generation_id=?1)",
            [input.generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "count finalized catalog", error)
        })?;
    checkpoint.cursor_identity = Some(receipt.clone());
    checkpoint.counters.insert("synthesis_complete".into(), 1);
    checkpoint
        .counters
        .insert("final_rules".into(), to_u64(rule_count, "rule count")?);
    checkpoint.counters.insert(
        "final_clauses".into(),
        to_u64(clause_count, "clause count")?,
    );
    checkpoint.counters.insert(
        "final_domains".into(),
        to_u64(domain_count, "domain count")?,
    );
    validate_checkpoint(&checkpoint)?;
    let checkpoint_json = serde_json::to_string(&checkpoint)
        .map_err(|error| format!("Encode archaeology synthesis catalog checkpoint: {error}"))?;
    if checkpoint_json.len() > MAX_CHECKPOINT_BYTES {
        return Err(format!(
            "Archaeology checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes"
        ));
    }
    synthesis_catalog_cancelled(input.cancellation)?;
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET stage='validate',checkpoint_identity=?5,checkpoint_json=?6,updated_at=?7
             WHERE job_id=?1 AND repository_id=?2 AND generation_id=?3 AND owner_id=?4
               AND state='running' AND stage='synthesize' AND cancellation_requested=0
               AND completed_units=?8 AND (total_units IS ?9 OR total_units=?9)
               AND julianday(?7)>=julianday(updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                receipt,
                checkpoint_json,
                input.now,
                completed_units,
                total_units,
            ],
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "checkpoint catalog", error)
        })?;
    require_cas(changed, "synthesis catalog checkpoint", input.job_id)?;
    let status = load_job(&transaction, input.job_id)?;
    synthesis_catalog_cancelled(input.cancellation)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology synthesis catalog: {error}"))?;
    Ok(status)
}

fn materialize_model_synthesis(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
    model: &ArchaeologyModelSynthesisCatalog<'_>,
    apply: bool,
) -> Result<(), String> {
    let request = model.request;
    let response = model.response;
    if request.repository_id != input.repository_id
        || request.generation_id != input.generation_id
        || request.revision_sha != input.identity.revision_sha
        || request.parser_identity != input.identity.parser
        || request.algorithm_identity != input.identity.algorithm
    {
        return Err("Archaeology model synthesis request is outside the owned generation".into());
    }
    validate_ready_synthesis_cache(transaction, input, model)?;
    let fact_spans = validate_synthesis_request_projection(transaction, input, request)?;
    let rule_id = expected_rule_id(&request.packet);
    let expected_kind = json_scalar(&request.packet.kind, "rule kind")?;
    let (trust, synthesis_identity): (String, Option<String>) = transaction
        .query_row(
            "SELECT trust,synthesis_identity FROM archaeology_rules
             WHERE generation_id=?1 AND rule_id=?2 AND repository_id=?3
               AND revision_sha=?4 AND kind=?5 AND lifecycle='candidate'
               AND parser_identity=?6 AND algorithm_identity=?7",
            params![
                input.generation_id,
                rule_id,
                input.repository_id,
                input.identity.revision_sha,
                expected_kind,
                input.identity.parser,
                input.identity.algorithm,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "load model rule", error))?
        .ok_or_else(|| "Archaeology model synthesis has no matching canonical rule".to_string())?;

    if !apply {
        if trust == "model_synthesized" && synthesis_identity.as_deref() == Some(model.cache_key) {
            return Ok(());
        }
        return Err("Archaeology validated model rule does not match the synthesis cache".into());
    }
    if trust != "deterministic" || synthesis_identity.is_some() {
        return Err(
            "Archaeology model synthesis cannot replace non-deterministic rule state".into(),
        );
    }

    let existing_clause_ids = transaction
        .prepare(
            "SELECT clause_id FROM archaeology_rule_clauses
             WHERE generation_id=?1 AND rule_id=?2 ORDER BY ordinal,clause_id",
        )
        .and_then(|mut statement| {
            statement
                .query_map(params![input.generation_id, rule_id], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "load prior clauses", error)
        })?;
    for clause_id in &existing_clause_ids {
        synthesis_catalog_cancelled(input.cancellation)?;
        transaction
            .execute(
                "DELETE FROM archaeology_evidence_links
                 WHERE generation_id=?1 AND owner_kind='rule_clause' AND owner_id=?2",
                params![input.generation_id, clause_id],
            )
            .map_err(|error| {
                synthesis_catalog_sql_error(input.cancellation, "clear prior evidence", error)
            })?;
    }
    transaction
        .execute(
            "DELETE FROM archaeology_rule_clauses WHERE generation_id=?1 AND rule_id=?2",
            params![input.generation_id, rule_id],
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "clear prior clauses", error)
        })?;

    let confidence = json_scalar(&request.packet.confidence, "rule confidence")?;
    let caveats = serde_json::to_string(&request.packet.caveats)
        .map_err(|_| "Archaeology synthesis caveats are not serializable".to_string())?;
    let mut clause_ids = std::collections::BTreeSet::new();
    for (ordinal, clause) in response.clauses.iter().enumerate() {
        synthesis_catalog_cancelled(input.cancellation)?;
        let positive = clause.supporting_fact_ids();
        let shape = serde_json::to_string(&(
            &positive,
            &clause.contradicting_fact_ids,
            &clause.relationship_ids,
            &clause.quantifier,
        ))
        .map_err(|_| "Archaeology synthesis clause identity is not serializable".to_string())?;
        let clause_id = stable_graph_id(
            "archaeology-clause",
            &format!("{rule_id}\0model-v1\0{shape}"),
        );
        if !clause_ids.insert(clause_id.clone()) {
            return Err("Archaeology model synthesis produced duplicate canonical clauses".into());
        }
        let text = canonical_synthesis_clause_text(request, clause)?;
        transaction
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES (?1,?2,?3,?4,?5,'model_synthesized',?6,?7)",
                params![
                    input.generation_id,
                    rule_id,
                    clause_id,
                    ordinal,
                    text,
                    confidence,
                    caveats,
                ],
            )
            .map_err(|error| {
                synthesis_catalog_sql_error(input.cancellation, "insert model clause", error)
            })?;
        for &fact_id in &positive {
            insert_clause_evidence(
                transaction,
                input,
                &clause_id,
                "fact",
                fact_id,
                "supporting",
            )?;
            let spans = fact_spans
                .get(fact_id)
                .ok_or("Archaeology model clause supporting fact has no exact spans")?;
            for span_id in spans {
                insert_clause_evidence(
                    transaction,
                    input,
                    &clause_id,
                    "span",
                    span_id,
                    "supporting",
                )?;
            }
        }
        for fact_id in &clause.contradicting_fact_ids {
            insert_clause_evidence(
                transaction,
                input,
                &clause_id,
                "fact",
                fact_id,
                "contradicting",
            )?;
            let spans = fact_spans
                .get(fact_id)
                .ok_or("Archaeology model clause contradicting fact has no exact spans")?;
            for span_id in spans {
                insert_clause_evidence(
                    transaction,
                    input,
                    &clause_id,
                    "span",
                    span_id,
                    "contradicting",
                )?;
            }
        }
    }
    let changed = transaction
        .execute(
            "UPDATE archaeology_rules
             SET trust='model_synthesized',confidence=?4,synthesis_identity=?5
             WHERE generation_id=?1 AND rule_id=?2 AND repository_id=?3
               AND trust='deterministic' AND synthesis_identity IS NULL",
            params![
                input.generation_id,
                rule_id,
                input.repository_id,
                confidence,
                model.cache_key,
            ],
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "update model rule", error)
        })?;
    if changed != 1 {
        return Err("Archaeology model synthesis lost its canonical rule lease".into());
    }
    if refresh_rule_identities(
        transaction,
        input.generation_id,
        std::slice::from_ref(&rule_id),
        input.cancellation,
    )? != 1
    {
        return Err("Archaeology model synthesis identity did not reconcile".into());
    }
    Ok(())
}

fn validate_ready_synthesis_cache(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
    model: &ArchaeologyModelSynthesisCatalog<'_>,
) -> Result<(), String> {
    let (json, hash): (String, String) = transaction
        .query_row(
            "SELECT response_json,response_sha256 FROM archaeology_synthesis_cache
             WHERE generation_id=?1 AND cache_key=?2 AND request_id=?3 AND packet_id=?4
               AND status='ready' AND owner_id IS NULL",
            params![
                input.generation_id,
                model.cache_key,
                model.request.request_id,
                model.request.packet.packet_id,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "load ready synthesis", error)
        })?
        .ok_or_else(|| "Archaeology model synthesis cache is not exactly ready".to_string())?;
    if sha256_identity(json.as_bytes()) != hash {
        return Err("Archaeology model synthesis cache hash is invalid".into());
    }
    let cached: ArchaeologySynthesisResponse = serde_json::from_str(&json)
        .map_err(|_| "Archaeology model synthesis cache response is invalid".to_string())?;
    if &cached != model.response {
        return Err("Archaeology model synthesis cache response changed before publication".into());
    }
    Ok(())
}

fn validate_synthesis_request_projection(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
    request: &ArchaeologySynthesisRequest,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut fact_spans = BTreeMap::new();
    let mut all_spans = std::collections::BTreeSet::new();
    for fact in &request.facts {
        synthesis_catalog_cancelled(input.cancellation)?;
        let persisted: (String, String, String, String, String) = transaction
            .query_row(
                "SELECT kind,label,trust,confidence,attributes_json FROM archaeology_facts
                 WHERE generation_id=?1 AND fact_id=?2",
                params![input.generation_id, fact.fact_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                synthesis_catalog_sql_error(input.cancellation, "load synthesis fact", error)
            })?
            .ok_or_else(|| "Archaeology synthesis fact left its generation".to_string())?;
        let attributes: Vec<ArchaeologyAttribute> = serde_json::from_str(&persisted.4)
            .map_err(|_| "Stored archaeology synthesis fact attributes are invalid")?;
        if (persisted.0, persisted.1.clone(), persisted.2, persisted.3)
            != (
                json_scalar(&fact.kind, "fact kind")?,
                fact.label.clone(),
                json_scalar(&fact.trust, "fact trust")?,
                json_scalar(&fact.confidence, "fact confidence")?,
            )
            || fact.quantifier_kinds != quantifier_kinds_from_evidence(&persisted.1, &attributes)
        {
            return Err("Archaeology synthesis fact changed before publication".into());
        }
        let spans = transaction
            .prepare(
                "SELECT span.span_id FROM archaeology_evidence_links link
                 JOIN archaeology_source_spans span
                   ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
                 JOIN archaeology_source_units unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 WHERE link.generation_id=?1 AND link.owner_kind='fact'
                   AND link.owner_id=?2 AND link.evidence_kind='span'
                   AND link.role='supporting' AND span.revision_sha=?3
                   AND unit.classification NOT IN ('protected','opaque')
                 ORDER BY span.span_id",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(
                        params![
                            input.generation_id,
                            fact.fact_id,
                            input.identity.revision_sha
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|error| {
                synthesis_catalog_sql_error(input.cancellation, "load synthesis fact spans", error)
            })?;
        if spans.is_empty() {
            return Err("Archaeology synthesis fact lost its publishable evidence".into());
        }
        all_spans.extend(spans.iter().cloned());
        fact_spans.insert(fact.fact_id.clone(), spans);
    }
    for relationship in &request.relationships {
        synthesis_catalog_cancelled(input.cancellation)?;
        let persisted: (String, String, String, String, Option<String>) = transaction
            .query_row(
                "SELECT from_fact_id,to_fact_id,kind,trust,unresolved_reason
                 FROM archaeology_fact_edges WHERE generation_id=?1 AND edge_id=?2",
                params![input.generation_id, relationship.relationship_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                synthesis_catalog_sql_error(
                    input.cancellation,
                    "load synthesis relationship",
                    error,
                )
            })?
            .ok_or_else(|| "Archaeology synthesis relationship left its generation".to_string())?;
        if persisted.0 != relationship.from_fact_id
            || persisted.1 != relationship.to_fact_id
            || persisted.2 != json_scalar(&relationship.kind, "relationship kind")?
            || persisted.3 != json_scalar(&relationship.trust, "relationship trust")?
            || relationship.unresolved != (persisted.2 == "unresolved" || persisted.4.is_some())
        {
            return Err("Archaeology synthesis relationship changed before publication".into());
        }
        let spans = transaction
            .prepare(
                "SELECT span.span_id FROM archaeology_evidence_links link
                 JOIN archaeology_source_spans span
                   ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
                 JOIN archaeology_source_units unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 WHERE link.generation_id=?1 AND link.owner_kind='fact_edge'
                   AND link.owner_id=?2 AND link.evidence_kind='span'
                   AND link.role='supporting' AND span.revision_sha=?3
                   AND unit.classification NOT IN ('protected','opaque')
                 ORDER BY span.span_id",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(
                        params![
                            input.generation_id,
                            relationship.relationship_id,
                            input.identity.revision_sha,
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|error| {
                synthesis_catalog_sql_error(
                    input.cancellation,
                    "load synthesis relationship spans",
                    error,
                )
            })?;
        if spans.is_empty() {
            return Err("Archaeology synthesis relationship lost its publishable evidence".into());
        }
        all_spans.extend(spans);
    }
    if all_spans
        != request
            .packet
            .evidence_span_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    {
        return Err("Archaeology synthesis evidence changed before publication".into());
    }
    Ok(fact_spans)
}

fn insert_clause_evidence(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
    clause_id: &str,
    evidence_kind: &str,
    evidence_id: &str,
    role: &str,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'rule_clause',?2,?3,?4,?5)",
            params![
                input.generation_id,
                clause_id,
                evidence_kind,
                evidence_id,
                role,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "insert model evidence", error)
        })
}

fn json_scalar(value: &impl Serialize, label: &str) -> Result<String, String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| format!("Archaeology {label} is not a scalar contract value"))
}

fn persist_link_patch(
    transaction: &Transaction<'_>,
    generation_id: &str,
    units: &[PersistedLinkUnit],
    patch: &ArchaeologyLinkPatch,
) -> Result<(), String> {
    let removed_edges = serde_json::to_string(&patch.remove_edge_ids).map_err(|e| e.to_string())?;
    let removed_facts = serde_json::to_string(&patch.remove_fact_ids).map_err(|e| e.to_string())?;
    let facts = serde_json::to_string(&patch.upsert_facts).map_err(|e| e.to_string())?;
    let edges = serde_json::to_string(&patch.upsert_edges).map_err(|e| e.to_string())?;
    let evidence = serde_json::to_string(&patch.evidence).map_err(|e| e.to_string())?;
    transaction
        .execute(
            "DELETE FROM archaeology_evidence_links_compact
             WHERE generation_key=(SELECT generation_key FROM archaeology_generation_keys
                                    WHERE generation_id=?1)
               AND ((owner_kind_code=2 AND owner_identity_key IN (
                       SELECT identity_key FROM archaeology_evidence_identities
                        WHERE generation_key=archaeology_evidence_links_compact.generation_key
                          AND identity IN (SELECT value FROM json_each(?2))))
                 OR (owner_kind_code=1 AND owner_identity_key IN (
                       SELECT identity_key FROM archaeology_evidence_identities
                        WHERE generation_key=archaeology_evidence_links_compact.generation_key
                          AND identity IN (SELECT value FROM json_each(?3)))))",
            params![generation_id, removed_edges, removed_facts],
        )
        .map_err(|error| format!("Delete archaeology link evidence: {error}"))?;
    prune_orphan_evidence_identities(transaction, generation_id)
        .map_err(|error| format!("Prune archaeology link evidence identities: {error}"))?;
    transaction.execute(
        "DELETE FROM archaeology_fact_edges WHERE generation_id=?1 AND edge_id IN (SELECT value FROM json_each(?2))",
        params![generation_id,removed_edges]
    ).map_err(|error| format!("Delete archaeology linked edges: {error}"))?;
    transaction.execute(
        "DELETE FROM archaeology_facts WHERE generation_id=?1 AND fact_id IN (SELECT value FROM json_each(?2))",
        params![generation_id,removed_facts]
    ).map_err(|error| format!("Delete archaeology unresolved facts: {error}"))?;
    transaction.execute(
        "INSERT INTO archaeology_facts (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
         SELECT ?1,json_extract(value,'$.fact_id'),json_extract(value,'$.kind'),json_extract(value,'$.label'),
          json_extract(value,'$.parser_id'),json_extract(value,'$.trust'),json_extract(value,'$.confidence'),
          json(json_extract(value,'$.attributes')) FROM json_each(?2) WHERE 1
         ON CONFLICT(generation_id,fact_id) DO UPDATE SET kind=excluded.kind,label=excluded.label,
          parser_id=excluded.parser_id,trust=excluded.trust,confidence=excluded.confidence,attributes_json=excluded.attributes_json",
        params![generation_id,facts]
    ).map_err(|error| format!("Upsert archaeology linked facts: {error}"))?;
    transaction.execute(
        "INSERT INTO archaeology_fact_edges (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust,unresolved_reason)
         SELECT ?1,json_extract(value,'$.edge_id'),json_extract(value,'$.from_fact_id'),json_extract(value,'$.to_fact_id'),
          json_extract(value,'$.kind'),json_extract(value,'$.trust'),json_extract(value,'$.unresolved_reason')
         FROM json_each(?2) WHERE 1 ON CONFLICT(generation_id,edge_id) DO UPDATE SET
          from_fact_id=excluded.from_fact_id,to_fact_id=excluded.to_fact_id,kind=excluded.kind,
          trust=excluded.trust,unresolved_reason=excluded.unresolved_reason",
        params![generation_id,edges]
    ).map_err(|error| format!("Upsert archaeology linked edges: {error}"))?;
    let missing: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM json_each(?2) item WHERE NOT EXISTS (
          SELECT 1 FROM archaeology_source_spans span WHERE span.generation_id=?1
           AND span.span_id=json_extract(item.value,'$[2]'))",
            params![generation_id, evidence],
            |row| row.get(0),
        )
        .map_err(|error| format!("Validate archaeology link evidence: {error}"))?;
    if missing != 0 {
        return Err("Archaeology link patch has missing evidence spans".into());
    }
    insert_link_patch_evidence_json(transaction, generation_id, &evidence)
        .map_err(|error| format!("Upsert archaeology link evidence: {error}"))?;
    let missing:i64=transaction.query_row(
        "SELECT COUNT(*) FROM json_each(?2) item WHERE NOT EXISTS (
          SELECT 1 FROM archaeology_evidence_links persisted WHERE persisted.generation_id=?1
           AND persisted.owner_kind=json_extract(item.value,'$[0]') AND persisted.owner_id=json_extract(item.value,'$[1]')
           AND persisted.evidence_kind='span' AND persisted.evidence_id=json_extract(item.value,'$[2]') AND persisted.role='supporting')",
        params![generation_id,evidence],|row|row.get(0)
    ).map_err(|error| format!("Reconcile archaeology link evidence: {error}"))?;
    if missing != 0 {
        return Err("Archaeology link evidence did not reconcile".into());
    }
    let mut linked = BTreeMap::<&str, Vec<ArchaeologyAdapterLineage>>::new();
    for item in &patch.lineage {
        linked
            .entry(&item.source_unit_id)
            .or_default()
            .push(item.clone());
    }
    let lineage = units
        .iter()
        .map(|unit| {
            let mut lineage = unit
                .lineage
                .iter()
                .filter(|item| item.kind == ArchaeologyLineageKind::Preprocessed)
                .cloned()
                .collect::<Vec<_>>();
            lineage.extend(
                linked
                    .remove(unit.source_unit_id.as_str())
                    .unwrap_or_default(),
            );
            (&unit.source_unit_id, lineage)
        })
        .collect::<Vec<_>>();
    if !linked.is_empty() {
        return Err("Archaeology link patch references an unknown source unit".into());
    }
    let lineage = serde_json::to_string(&lineage).map_err(|e| e.to_string())?;
    let changed=transaction.execute(
        "UPDATE archaeology_source_units AS unit SET include_lineage_json=(SELECT json(json_extract(item.value,'$[1]'))
         FROM json_each(?2) item WHERE json_extract(item.value,'$[0]')=unit.source_unit_id)
         WHERE unit.generation_id=?1 AND unit.source_unit_id IN (
          SELECT json_extract(value,'$[0]') FROM json_each(?2))",params![generation_id,lineage]
    ).map_err(|error|format!("Update archaeology link lineage: {error}"))?;
    if changed != units.len() {
        return Err("Archaeology link lineage did not reconcile".into());
    }
    Ok(())
}

fn persist_deterministic_rules(
    transaction: &Transaction<'_>,
    generation_id: &str,
    parser_identity: &str,
    algorithm_identity: &str,
    now: &str,
    rules: &[ArchaeologyRulePacket],
    limits: ArchaeologyDeterministicLimits,
    cancellation: &StructuralGraphCancellation,
) -> Result<(), String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    let mut fact_spans = BTreeMap::<String, Vec<String>>::new();
    let mut statement = transaction
        .prepare(
            "SELECT owner_id,evidence_id FROM archaeology_evidence_links
             WHERE generation_id=?1 AND owner_kind='fact' AND evidence_kind='span'
               AND role='supporting' ORDER BY owner_id,evidence_id",
        )
        .map_err(|error| format!("Prepare archaeology deterministic fact spans: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Query archaeology deterministic fact spans: {error}"))?;
    for row in rows {
        let (fact_id, span_id) =
            row.map_err(|error| format!("Read archaeology deterministic fact spans: {error}"))?;
        fact_spans.entry(fact_id).or_default().push(span_id);
    }
    drop(statement);
    let mut rule_ids = std::collections::BTreeSet::new();
    let mut clause_ids = std::collections::BTreeSet::new();
    let mut clauses = Vec::new();
    let mut evidence = std::collections::BTreeSet::new();
    for (rule_index, rule) in rules.iter().enumerate() {
        if rule_index % 128 == 0 && cancellation.is_cancelled() {
            return Err("Archaeology derivation cancelled".into());
        }
        rule.validate()?;
        if rule.generation_id != generation_id
            || rule.parser_identity != parser_identity
            || rule.algorithm_identity != algorithm_identity
            || rule.lifecycle != ArchaeologyRuleLifecycle::Candidate
            || rule.trust != ArchaeologyTrust::Deterministic
            || rule.synthesis_identity.is_some()
            || !rule.dependency_rule_ids.is_empty()
            || !rule_ids.insert(rule.rule_id.as_str())
            || unsafe_rule_text(&rule.title)
        {
            return Err("Archaeology deterministic rule output is outside its scope".into());
        }
        for (ordinal, clause) in rule.clauses.iter().enumerate() {
            if ordinal % 256 == 0 && cancellation.is_cancelled() {
                return Err("Archaeology derivation cancelled".into());
            }
            if !clause_ids.insert(clause.clause_id.as_str())
                || unsafe_rule_text(&clause.text)
                || clause.caveats.iter().any(|value| unsafe_rule_text(value))
            {
                return Err(
                    "Archaeology deterministic clause output is unsafe or duplicated".into(),
                );
            }
            clauses.push(PersistedRuleClause {
                rule_id: &rule.rule_id,
                ordinal,
                clause,
            });
            for fact_id in &clause.supporting_fact_ids {
                evidence.insert((
                    clause.clause_id.as_str(),
                    "fact",
                    fact_id.as_str(),
                    "supporting",
                ));
            }
            let mut exact_spans = std::collections::BTreeSet::new();
            for fact_id in &clause.supporting_fact_ids {
                let spans = fact_spans
                    .get(fact_id)
                    .ok_or("Archaeology deterministic supporting fact has no exact spans")?;
                for span_id in spans {
                    exact_spans.insert(span_id.as_str());
                    evidence.insert((
                        clause.clause_id.as_str(),
                        "span",
                        span_id.as_str(),
                        "supporting",
                    ));
                }
            }
            for fact_id in &clause.contradicting_fact_ids {
                evidence.insert((
                    clause.clause_id.as_str(),
                    "fact",
                    fact_id.as_str(),
                    "contradicting",
                ));
                let spans = fact_spans
                    .get(fact_id)
                    .ok_or("Archaeology deterministic contradicting fact has no exact spans")?;
                for span_id in spans {
                    exact_spans.insert(span_id.as_str());
                    evidence.insert((
                        clause.clause_id.as_str(),
                        "span",
                        span_id.as_str(),
                        "contradicting",
                    ));
                }
            }
            if exact_spans
                != clause
                    .evidence_span_ids
                    .iter()
                    .map(String::as_str)
                    .collect()
            {
                return Err("Archaeology deterministic clause spans are not exact".into());
            }
        }
    }
    let rules_by_id = rules
        .iter()
        .map(|rule| (rule.rule_id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();
    let mut relations = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        if index % 128 == 0 && cancellation.is_cancelled() {
            return Err("Archaeology derivation cancelled".into());
        }
        validate_clustered_rule_shape(rule, &rules_by_id)?;
        if let Some(primary_id) = rule.alias_rule_ids.first() {
            relations.push(PersistedRuleRelation {
                relation_id: digest_identity(
                    format!("aliases\0{}\0{primary_id}", rule.rule_id).as_bytes(),
                    "archaeology-rule-relation:v1:",
                ),
                from_rule_id: &rule.rule_id,
                to_rule_id: primary_id,
                kind: "aliases",
            });
        }
        for conflict_id in &rule.conflict_rule_ids {
            if rule.rule_id < *conflict_id {
                relations.push(PersistedRuleRelation {
                    relation_id: digest_identity(
                        format!("conflicts_with\0{}\0{conflict_id}", rule.rule_id).as_bytes(),
                        "archaeology-rule-relation:v1:",
                    ),
                    from_rule_id: &rule.rule_id,
                    to_rule_id: conflict_id,
                    kind: "conflicts_with",
                });
            }
        }
    }
    if relations.len() > limits.max_cluster_relations {
        return Err("Archaeology deterministic relation bound exceeded".into());
    }
    relations.sort_by(|left, right| left.relation_id.cmp(&right.relation_id));
    if cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let rules_json = serde_json::to_string(rules).map_err(|error| error.to_string())?;
    if cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let clauses_json = serde_json::to_string(&clauses).map_err(|error| error.to_string())?;
    if cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let evidence_json = serde_json::to_string(&evidence).map_err(|error| error.to_string())?;
    if cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    let relations_json = serde_json::to_string(&relations).map_err(|error| error.to_string())?;
    if cancellation.is_cancelled() {
        return Err("Archaeology derivation cancelled".into());
    }
    if rules_json
        .len()
        .saturating_add(clauses_json.len())
        .saturating_add(evidence_json.len())
        .saturating_add(relations_json.len())
        > limits.max_cluster_output_bytes
    {
        return Err("Archaeology deterministic persistence payload bound exceeded".into());
    }
    profile_archaeology_stage(profiling, "derive.persist.serialize", started);

    let collisions: (i64, i64) = transaction
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM archaeology_rules existing JOIN json_each(?2) output
                  ON json_extract(output.value,'$.rule_id')=existing.rule_id
                WHERE existing.generation_id=?1 AND NOT (
                    existing.repository_id=json_extract(output.value,'$.repository_id')
                    AND existing.revision_sha=json_extract(output.value,'$.revision_sha')
                    AND existing.lifecycle='candidate' AND existing.trust='deterministic'
                    AND existing.parser_identity=?4 AND existing.algorithm_identity=?5
                    AND existing.synthesis_identity IS NULL)),
               (SELECT COUNT(*) FROM archaeology_rule_clauses existing
                  JOIN json_each(?3) output
                    ON json_extract(output.value,'$.clause.clause_id')=existing.clause_id
                 WHERE existing.generation_id=?1
                   AND existing.rule_id!=json_extract(output.value,'$.rule_id'))",
            params![
                generation_id,
                rules_json,
                clauses_json,
                parser_identity,
                algorithm_identity,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Validate archaeology deterministic collisions: {error}"))?;
    if collisions != (0, 0) {
        return Err("Archaeology deterministic output collides with durable reviewed data".into());
    }
    profile_archaeology_stage(profiling, "derive.persist.collisions", started);

    let has_existing_deterministic_rules: bool = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM archaeology_rules
                WHERE generation_id=?1 AND lifecycle='candidate' AND trust='deterministic'
                  AND parser_identity=?2 AND algorithm_identity=?3 AND synthesis_identity IS NULL
            )",
            params![generation_id, parser_identity, algorithm_identity],
            |row| row.get(0),
        )
        .map_err(|error| format!("Check archaeology deterministic replacement state: {error}"))?;
    if has_existing_deterministic_rules {
        transaction
            .execute(
                "DELETE FROM archaeology_evidence_links_compact
             WHERE generation_key=(SELECT generation_key FROM archaeology_generation_keys
                                    WHERE generation_id=?1)
               AND owner_kind_code=3 AND owner_identity_key IN (
               SELECT identity.identity_key FROM archaeology_evidence_identities identity
               WHERE identity.generation_key=archaeology_evidence_links_compact.generation_key
                 AND identity.identity IN (
               SELECT clause.clause_id FROM archaeology_rule_clauses clause
               JOIN archaeology_rules rule
                 ON rule.generation_id=clause.generation_id AND rule.rule_id=clause.rule_id
               WHERE clause.generation_id=?1 AND rule.lifecycle='candidate'
                 AND rule.trust='deterministic' AND rule.parser_identity=?2
                 AND rule.algorithm_identity=?3 AND rule.synthesis_identity IS NULL))",
                params![generation_id, parser_identity, algorithm_identity],
            )
            .map_err(|error| format!("Delete archaeology deterministic evidence: {error}"))?;
        transaction
            .execute(
                "DELETE FROM archaeology_evidence_links_compact
             WHERE generation_key=(SELECT generation_key FROM archaeology_generation_keys
                                    WHERE generation_id=?1)
               AND owner_kind_code=4 AND owner_identity_key IN (
               SELECT identity.identity_key FROM archaeology_evidence_identities identity
               WHERE identity.generation_key=archaeology_evidence_links_compact.generation_key
                 AND identity.identity IN (
               SELECT relation_id FROM archaeology_rule_relations WHERE generation_id=?1 AND (
                 from_rule_id IN (SELECT rule_id FROM archaeology_rules WHERE generation_id=?1
                   AND lifecycle='candidate' AND trust='deterministic' AND parser_identity=?2
                   AND algorithm_identity=?3 AND synthesis_identity IS NULL)
                 OR to_rule_id IN (SELECT rule_id FROM archaeology_rules WHERE generation_id=?1
                   AND lifecycle='candidate' AND trust='deterministic' AND parser_identity=?2
                   AND algorithm_identity=?3 AND synthesis_identity IS NULL))))",
                params![generation_id, parser_identity, algorithm_identity],
            )
            .map_err(|error| {
                format!("Delete archaeology deterministic relation evidence: {error}")
            })?;
        prune_orphan_evidence_identities(transaction, generation_id).map_err(|error| {
            format!("Prune archaeology deterministic evidence identities: {error}")
        })?;
        transaction
            .execute(
                "DELETE FROM archaeology_rules WHERE generation_id=?1
               AND lifecycle='candidate' AND trust='deterministic'
               AND parser_identity=?2 AND algorithm_identity=?3 AND synthesis_identity IS NULL",
                params![generation_id, parser_identity, algorithm_identity],
            )
            .map_err(|error| format!("Replace archaeology deterministic rules: {error}"))?;
    }
    profile_archaeology_stage(profiling, "derive.persist.delete", started);
    transaction.execute(
        "INSERT INTO archaeology_rules
          (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
           confidence,parser_identity,algorithm_identity,synthesis_identity,coverage_json,created_at)
         SELECT ?1,json_extract(value,'$.rule_id'),json_extract(value,'$.repository_id'),
           json_extract(value,'$.revision_sha'),json_extract(value,'$.kind'),
           json_extract(value,'$.title'),json_extract(value,'$.lifecycle'),
           json_extract(value,'$.trust'),json_extract(value,'$.confidence'),
           json_extract(value,'$.parser_identity'),json_extract(value,'$.algorithm_identity'),
           NULL,json(json_extract(value,'$.coverage')),?3
         FROM json_each(?2)",
        params![generation_id,rules_json,now]
    ).map_err(|error|format!("Insert archaeology deterministic rules: {error}"))?;
    transaction
        .execute(
            "INSERT INTO archaeology_rule_clauses
          (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
         SELECT ?1,json_extract(value,'$.rule_id'),json_extract(value,'$.clause.clause_id'),
           json_extract(value,'$.ordinal'),json_extract(value,'$.clause.text'),
           json_extract(value,'$.clause.trust'),json_extract(value,'$.clause.confidence'),
           json(json_extract(value,'$.clause.caveats')) FROM json_each(?2)",
            params![generation_id, clauses_json],
        )
        .map_err(|error| format!("Insert archaeology deterministic clauses: {error}"))?;
    profile_archaeology_stage(profiling, "derive.persist.rules_clauses", started);
    let missing_evidence: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM json_each(?2) item WHERE
          CASE json_extract(item.value,'$[1]')
           WHEN 'fact' THEN NOT EXISTS (SELECT 1 FROM archaeology_facts fact
             WHERE fact.generation_id=?1 AND fact.fact_id=json_extract(item.value,'$[2]'))
           WHEN 'span' THEN NOT EXISTS (SELECT 1 FROM archaeology_source_spans span
             JOIN archaeology_generations generation ON generation.generation_id=span.generation_id
             WHERE span.generation_id=?1 AND span.span_id=json_extract(item.value,'$[2]')
               AND span.revision_sha=generation.revision_sha)
           ELSE 1 END",
            params![generation_id, evidence_json],
            |row| row.get(0),
        )
        .map_err(|error| format!("Validate archaeology deterministic evidence: {error}"))?;
    if missing_evidence != 0 {
        return Err("Archaeology deterministic output cites missing evidence".into());
    }
    insert_clause_evidence_json(transaction, generation_id, &evidence_json)
        .map_err(|error| format!("Insert archaeology deterministic evidence: {error}"))?;
    profile_archaeology_stage(profiling, "derive.persist.clause_evidence", started);
    transaction
        .execute(
            "INSERT INTO archaeology_rule_domains
              (generation_id,rule_id,domain_id,domain_label,parent_domain_id)
             SELECT ?1,json_extract(rule.value,'$.rule_id'),domain.value,'Other',NULL
             FROM json_each(?2) rule JOIN json_each(json_extract(rule.value,'$.domain_ids')) domain
             WHERE domain.value='domain:other'",
            params![generation_id, rules_json],
        )
        .map_err(|error| format!("Insert archaeology deterministic domains: {error}"))?;
    transaction
        .execute(
            "INSERT INTO archaeology_rule_relations
              (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust,summary)
             SELECT ?1,json_extract(value,'$.relation_id'),
               json_extract(value,'$.from_rule_id'),json_extract(value,'$.to_rule_id'),
               json_extract(value,'$.kind'),'deterministic',NULL FROM json_each(?2)",
            params![generation_id, relations_json],
        )
        .map_err(|error| format!("Insert archaeology deterministic relations: {error}"))?;
    insert_relation_evidence_json(transaction, generation_id, &relations_json)
        .map_err(|error| format!("Insert archaeology deterministic relation evidence: {error}"))?;
    profile_archaeology_stage(profiling, "derive.persist.relations", started);
    let reconciliation: i64 = transaction
        .query_row(
            "WITH
              expected_rules(rule_id) AS MATERIALIZED (
                SELECT json_extract(value,'$.rule_id') FROM json_each(?2)),
              actual_rules(rule_id) AS MATERIALIZED (
                SELECT rule_id FROM archaeology_rules WHERE generation_id=?1
                  AND rule_id IN (SELECT rule_id FROM expected_rules)),
              expected_clauses(clause_id,rule_id) AS MATERIALIZED (
                SELECT json_extract(value,'$.clause.clause_id'),json_extract(value,'$.rule_id')
                FROM json_each(?3)),
              actual_clauses(clause_id,rule_id) AS MATERIALIZED (
                SELECT clause_id,rule_id FROM archaeology_rule_clauses WHERE generation_id=?1
                  AND clause_id IN (SELECT clause_id FROM expected_clauses)),
              expected_evidence(owner_id,evidence_kind,evidence_id,role) AS MATERIALIZED (
                SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]'),
                  json_extract(value,'$[2]'),json_extract(value,'$[3]') FROM json_each(?4)),
              actual_evidence(owner_id,evidence_kind,evidence_id,role) AS MATERIALIZED (
                SELECT owner_id,evidence_kind,evidence_id,role FROM archaeology_evidence_links
                WHERE generation_id=?1 AND owner_kind='rule_clause'
                  AND owner_id IN (SELECT clause_id FROM expected_clauses))
             SELECT
               (SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_rules EXCEPT SELECT * FROM actual_rules)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_rules EXCEPT SELECT * FROM expected_rules)))
              +(SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_clauses EXCEPT SELECT * FROM actual_clauses)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_clauses EXCEPT SELECT * FROM expected_clauses)))
              +(SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_evidence EXCEPT SELECT * FROM actual_evidence)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_evidence EXCEPT SELECT * FROM expected_evidence)))",
            params![generation_id, rules_json, clauses_json, evidence_json],
            |row| row.get(0),
        )
        .map_err(|error| format!("Reconcile archaeology deterministic output: {error}"))?;
    if reconciliation != 0 {
        return Err("Archaeology deterministic output did not reconcile".into());
    }
    profile_archaeology_stage(profiling, "derive.persist.reconcile", started);
    let cluster_reconciliation: i64 = transaction
        .query_row(
            "WITH
              expected_rules(rule_id) AS MATERIALIZED (
                SELECT json_extract(value,'$.rule_id') FROM json_each(?2)),
              expected_domains(rule_id,domain_id,domain_label,parent_domain_id) AS MATERIALIZED (
                SELECT json_extract(rule.value,'$.rule_id'),domain.value,'Other',NULL
                FROM json_each(?2) rule
                JOIN json_each(json_extract(rule.value,'$.domain_ids')) domain),
              actual_domains(rule_id,domain_id,domain_label,parent_domain_id) AS MATERIALIZED (
                SELECT rule_id,domain_id,domain_label,parent_domain_id
                FROM archaeology_rule_domains WHERE generation_id=?1
                  AND rule_id IN (SELECT rule_id FROM expected_rules)),
              expected_relations(relation_id,from_rule_id,to_rule_id,kind,trust,summary) AS MATERIALIZED (
                SELECT json_extract(value,'$.relation_id'),json_extract(value,'$.from_rule_id'),
                  json_extract(value,'$.to_rule_id'),json_extract(value,'$.kind'),'deterministic',NULL
                FROM json_each(?3)),
              actual_relations(relation_id,from_rule_id,to_rule_id,kind,trust,summary) AS MATERIALIZED (
                SELECT relation_id,from_rule_id,to_rule_id,kind,trust,summary
                FROM archaeology_rule_relations WHERE generation_id=?1
                  AND (from_rule_id IN (SELECT rule_id FROM expected_rules)
                    OR to_rule_id IN (SELECT rule_id FROM expected_rules))),
              expected_relation_evidence(owner_id,evidence_kind,evidence_id,role) AS MATERIALIZED (
                SELECT relation_id,'rule',from_rule_id,'supporting' FROM expected_relations
                UNION ALL SELECT relation_id,'rule',to_rule_id,'supporting' FROM expected_relations),
              actual_relation_evidence(owner_id,evidence_kind,evidence_id,role) AS MATERIALIZED (
                SELECT owner_id,evidence_kind,evidence_id,role FROM archaeology_evidence_links
                WHERE generation_id=?1 AND owner_kind='rule_relation'
                  AND owner_id IN (SELECT relation_id FROM expected_relations))
             SELECT
               (SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_domains EXCEPT SELECT * FROM actual_domains)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_domains EXCEPT SELECT * FROM expected_domains)))
              +(SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_relations EXCEPT SELECT * FROM actual_relations)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_relations EXCEPT SELECT * FROM expected_relations)))
              +(SELECT COUNT(*) FROM (
                  SELECT * FROM (SELECT * FROM expected_relation_evidence EXCEPT SELECT * FROM actual_relation_evidence)
                  UNION ALL SELECT * FROM (SELECT * FROM actual_relation_evidence EXCEPT SELECT * FROM expected_relation_evidence)))",
            params![generation_id, rules_json, relations_json],
            |row| row.get(0),
        )
        .map_err(|error| format!("Reconcile archaeology deterministic clusters: {error}"))?;
    if cluster_reconciliation != 0 {
        return Err("Archaeology deterministic clusters did not reconcile".into());
    }
    Ok(())
}

fn validate_clustered_rule_shape(
    rule: &ArchaeologyRulePacket,
    rules: &BTreeMap<&str, &ArchaeologyRulePacket>,
) -> Result<(), String> {
    let is_alias = !rule.alias_rule_ids.is_empty();
    if rule.alias_rule_ids.len() > 1
        || (is_alias && (!rule.domain_ids.is_empty() || !rule.conflict_rule_ids.is_empty()))
        || (!is_alias && (rule.domain_ids.len() != 1 || rule.domain_ids[0] != "domain:other"))
        || rule
            .conflict_rule_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err("Archaeology clustered rule shape is inconsistent".into());
    }
    if let Some(primary_id) = rule.alias_rule_ids.first() {
        let Some(primary) = rules.get(primary_id.as_str()) else {
            return Err("Archaeology clustered alias target is unknown".into());
        };
        if primary.rule_id == rule.rule_id
            || !primary.alias_rule_ids.is_empty()
            || primary.domain_ids.as_slice() != ["domain:other"]
            || primary.kind != rule.kind
        {
            return Err("Archaeology clustered alias is not a primary star".into());
        }
    }
    for conflict_id in &rule.conflict_rule_ids {
        let Some(conflict) = rules.get(conflict_id.as_str()) else {
            return Err("Archaeology clustered conflict target is unknown".into());
        };
        if conflict.rule_id == rule.rule_id
            || !conflict.alias_rule_ids.is_empty()
            || !conflict.conflict_rule_ids.contains(&rule.rule_id)
        {
            return Err("Archaeology clustered conflicts must be symmetric primaries".into());
        }
    }
    Ok(())
}

fn unsafe_rule_text(value: &str) -> bool {
    value.contains('\0') || looks_like_secret(value) || contains_sensitive_path(value)
}

fn synthesis_catalog_cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology synthesis catalog cancelled".into())
    } else {
        Ok(())
    }
}

fn synthesis_catalog_sql_error(
    cancellation: &StructuralGraphCancellation,
    action: &str,
    error: rusqlite::Error,
) -> String {
    if cancellation.is_cancelled() {
        "Archaeology synthesis catalog cancelled".into()
    } else {
        format!("Archaeology synthesis catalog {action}: {error}")
    }
}

fn to_u64(value: i64, label: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("Archaeology synthesis catalog {label} is invalid"))
}

fn validate_final_rule_catalog(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
) -> Result<(), String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    let (rules, clauses, domains, bytes): (i64, i64, i64, i64) = transaction
        .query_row(
            "SELECT
              (SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1),
              (SELECT COUNT(*) FROM archaeology_rule_clauses WHERE generation_id=?1),
              (SELECT COUNT(*) FROM archaeology_rule_domains WHERE generation_id=?1),
              (SELECT COALESCE(SUM(
                 LENGTH(CAST(rule_id AS BLOB))+LENGTH(CAST(title AS BLOB))+
                 LENGTH(CAST(parser_identity AS BLOB))+LENGTH(CAST(algorithm_identity AS BLOB))+
                 LENGTH(CAST(COALESCE(synthesis_identity,'') AS BLOB))+
                 LENGTH(CAST(coverage_json AS BLOB))+96),0)
               FROM archaeology_rules WHERE generation_id=?1)
              +(SELECT COALESCE(SUM(
                 LENGTH(CAST(clause_id AS BLOB))+LENGTH(CAST(clause_text AS BLOB))+
                 LENGTH(CAST(caveats_json AS BLOB))+64),0)
               FROM archaeology_rule_clauses WHERE generation_id=?1)
              +(SELECT COALESCE(SUM(
                 LENGTH(CAST(domain_id AS BLOB))+LENGTH(CAST(domain_label AS BLOB))+
                 LENGTH(CAST(COALESCE(parent_domain_id,'') AS BLOB))+48),0)
               FROM archaeology_rule_domains WHERE generation_id=?1)",
            [input.generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "count", error))?;
    profile_archaeology_stage(profiling, "validate_catalog.bounds", started);
    if count_exceeds(rules, MAX_FINAL_RULES)
        || count_exceeds(clauses, MAX_FINAL_CLAUSES)
        || count_exceeds(domains, MAX_FINAL_DOMAINS)
        || count_exceeds(bytes, MAX_FINAL_CATALOG_BYTES)
    {
        return Err("Archaeology final rule catalog exceeds its bounded limits".into());
    }

    let violations: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = transaction
        .query_row(
            "WITH aliases AS MATERIALIZED (
                SELECT from_rule_id AS rule_id
                FROM archaeology_rule_relations
                WHERE generation_id=?1 AND kind='aliases'
             ), canonical AS MATERIALIZED (
                SELECT rule_id FROM archaeology_rules
                WHERE generation_id=?1 AND rule_id NOT IN (SELECT rule_id FROM aliases)
             ), evidence AS NOT MATERIALIZED (
                SELECT generation.generation_id,link.owner_kind_code,
                       owner.identity AS owner_id,link.evidence_kind_code,
                       referenced.identity AS evidence_id,link.role_code
                FROM archaeology_evidence_links_compact link
                JOIN archaeology_generation_keys generation
                  ON generation.generation_key=link.generation_key
                 AND generation.generation_id=?1
                JOIN archaeology_evidence_identities owner
                  ON owner.generation_key=link.generation_key
                 AND owner.identity_key=link.owner_identity_key
                JOIN archaeology_evidence_identities referenced
                  ON referenced.generation_key=link.generation_key
                 AND referenced.identity_key=link.evidence_identity_key
             )
             SELECT
               (SELECT COUNT(*) FROM archaeology_rules rule
                WHERE rule.generation_id=?1 AND (
                  rule.repository_id!=?2 OR rule.revision_sha!=?3
                  OR rule.parser_identity!=?4 OR rule.algorithm_identity!=?5
                  OR rule.lifecycle!='candidate'
                  OR rule.trust NOT IN ('deterministic','model_synthesized')
                  OR trim(rule.rule_id)='' OR trim(rule.title)=''
                  OR trim(rule.parser_identity)='' OR trim(rule.algorithm_identity)=''
                  OR NOT json_valid(rule.coverage_json)
                  OR (rule.trust='model_synthesized' AND (
                    rule.synthesis_identity IS NULL OR NOT EXISTS (
                      SELECT 1 FROM archaeology_synthesis_cache cache
                      WHERE cache.generation_id=rule.generation_id
                        AND cache.cache_key=rule.synthesis_identity
                        AND cache.status='ready')))
                  OR (rule.trust='deterministic' AND rule.synthesis_identity IS NOT NULL)))
             , (SELECT COUNT(*) FROM archaeology_rules rule
                WHERE rule.generation_id=?1 AND NOT EXISTS (
                  SELECT 1 FROM archaeology_rule_clauses clause
                  WHERE clause.generation_id=rule.generation_id
                    AND clause.rule_id=rule.rule_id))
             , (SELECT COUNT(*) FROM archaeology_rule_clauses clause
                JOIN archaeology_rules rule USING (generation_id,rule_id)
                WHERE clause.generation_id=?1 AND (
                  trim(clause.clause_id)='' OR trim(clause.clause_text)=''
                  OR clause.trust!=rule.trust OR NOT json_valid(clause.caveats_json)
                  OR json_type(clause.caveats_json)!='array'
                  OR NOT EXISTS (SELECT 1 FROM evidence
                    JOIN archaeology_facts fact
                      ON fact.generation_id=evidence.generation_id
                     AND fact.fact_id=evidence.evidence_id
                    WHERE evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind_code=3
                      AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind_code=2 AND evidence.role_code=1)
                  OR NOT EXISTS (SELECT 1 FROM evidence
                    JOIN archaeology_source_spans span
                      ON span.generation_id=evidence.generation_id
                     AND span.span_id=evidence.evidence_id AND span.revision_sha=?3
                    WHERE evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind_code=3
                      AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind_code=1 AND evidence.role_code=1)))
             , (SELECT COUNT(*) FROM (
                  SELECT generation_id,rule_id,clause_text FROM archaeology_rule_clauses
                  WHERE generation_id=?1 GROUP BY generation_id,rule_id,clause_text
                  HAVING COUNT(*)>1))
             , (SELECT COUNT(*) FROM (
                  SELECT generation_id,rule_id,COUNT(*) count,MIN(ordinal) first,MAX(ordinal) last
                  FROM archaeology_rule_clauses WHERE generation_id=?1
                  GROUP BY generation_id,rule_id
                  HAVING first!=0 OR last!=count-1))
             , (SELECT COUNT(*) FROM canonical rule WHERE NOT EXISTS (
                  SELECT 1 FROM archaeology_rule_domains domain
                  WHERE domain.generation_id=?1 AND domain.rule_id=rule.rule_id))
             , (SELECT COUNT(*) FROM archaeology_rule_domains domain
                WHERE domain.generation_id=?1 AND (
                  trim(domain.domain_id)='' OR trim(domain.domain_label)=''
                  OR domain.rule_id IN (SELECT rule_id FROM aliases)))
             , (SELECT COUNT(*) FROM (
                  SELECT generation_id,rule_id,domain_label FROM archaeology_rule_domains
                  WHERE generation_id=?1 GROUP BY generation_id,rule_id,domain_label
                  HAVING COUNT(*)>1))
             , (SELECT COUNT(*) FROM evidence clause_span
                WHERE clause_span.generation_id=?1
                  AND clause_span.owner_kind_code=3
                  AND clause_span.evidence_kind_code=1
                  AND clause_span.role_code=1
                  AND NOT EXISTS (
                    SELECT 1 FROM evidence clause_fact
                    JOIN evidence fact_span
                      ON fact_span.generation_id=clause_fact.generation_id
                     AND fact_span.owner_kind_code=1
                     AND fact_span.owner_id=clause_fact.evidence_id
                     AND fact_span.evidence_kind_code=1
                     AND fact_span.evidence_id=clause_span.evidence_id
                     AND fact_span.role_code=1
                    WHERE clause_fact.generation_id=clause_span.generation_id
                      AND clause_fact.owner_kind_code=3
                      AND clause_fact.owner_id=clause_span.owner_id
                      AND clause_fact.evidence_kind_code=2
                      AND clause_fact.role_code=1))
             , (SELECT COUNT(*) FROM evidence contradiction
                WHERE contradiction.generation_id=?1
                  AND contradiction.owner_kind_code=3
                  AND contradiction.evidence_kind_code=2
                  AND contradiction.role_code=2
                  AND NOT EXISTS (
                    SELECT 1 FROM evidence supporting
                    JOIN archaeology_fact_edges edge
                      ON edge.generation_id=supporting.generation_id
                     AND edge.kind='contradicts'
                     AND ((edge.from_fact_id=supporting.evidence_id
                           AND edge.to_fact_id=contradiction.evidence_id)
                       OR (edge.to_fact_id=supporting.evidence_id
                           AND edge.from_fact_id=contradiction.evidence_id))
                    WHERE supporting.generation_id=contradiction.generation_id
                      AND supporting.owner_kind_code=3
                      AND supporting.owner_id=contradiction.owner_id
                      AND supporting.evidence_kind_code=2
                      AND supporting.role_code=1))
             , (SELECT COUNT(*) FROM (
                  SELECT from_rule_id FROM archaeology_rule_relations
                  WHERE generation_id=?1 AND kind='aliases'
                  GROUP BY from_rule_id HAVING COUNT(*)!=1))",
            params![
                input.generation_id,
                input.repository_id,
                input.identity.revision_sha,
                input.identity.parser,
                input.identity.algorithm,
            ],
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
                    row.get(10)?,
                ))
            },
        )
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "validate", error))?;
    profile_archaeology_stage(profiling, "validate_catalog.relations", started);
    let violation_total = [
        violations.0,
        violations.1,
        violations.2,
        violations.3,
        violations.4,
        violations.5,
        violations.6,
        violations.7,
        violations.8,
        violations.9,
        violations.10,
    ]
    .into_iter()
    .sum::<i64>();
    if violation_total != 0 {
        return Err(format!(
            "Archaeology final rule catalog validation failed: rule_scope={},missing_clauses={},invalid_clauses={},duplicate_clause_text={},ordinal_gap={},missing_domain={},invalid_domain={},duplicate_domain_label={},orphan_clause_span={},invalid_contradiction={},alias_multiplicity={}",
            violations.0,
            violations.1,
            violations.2,
            violations.3,
            violations.4,
            violations.5,
            violations.6,
            violations.7,
            violations.8,
            violations.9,
            violations.10,
        ));
    }
    validate_catalog_text(transaction, input)?;
    profile_archaeology_stage(profiling, "validate_catalog.text", started);
    validate_model_synthesis_cache(transaction, input)?;
    profile_archaeology_stage(profiling, "validate_catalog.model_cache", started);
    validate_model_rule_evidence(transaction, input)?;
    profile_archaeology_stage(profiling, "validate_catalog.model_evidence", started);
    validate_generation_alias_relations(transaction, input.repository_id, input.generation_id)?;
    profile_archaeology_stage(profiling, "validate_catalog.aliases", started);
    Ok(())
}

fn validate_catalog_text(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
) -> Result<(), String> {
    let sql = "SELECT rule_id,title,parser_identity,algorithm_identity,
                      COALESCE(synthesis_identity,''),coverage_json
               FROM archaeology_rules WHERE generation_id=?1
               UNION ALL
               SELECT clause_id,clause_text,'','','',caveats_json
               FROM archaeology_rule_clauses WHERE generation_id=?1
               UNION ALL
               SELECT domain_id,domain_label,COALESCE(parent_domain_id,''),'','','[]'
               FROM archaeology_rule_domains WHERE generation_id=?1
               UNION ALL
               SELECT relation_id,COALESCE(summary,''),from_rule_id,to_rule_id,'','[]'
               FROM archaeology_rule_relations WHERE generation_id=?1";
    let mut statement = transaction
        .prepare(sql)
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "prepare text", error))?;
    let rows = statement
        .query_map([input.generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "query text", error))?;
    for row in rows {
        synthesis_catalog_cancelled(input.cancellation)?;
        let values = row
            .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "read text", error))?;
        validate_persisted_token("catalog row", &values.0, MAX_ID_BYTES)?;
        for identity in [&values.2, &values.3] {
            if !identity.is_empty() {
                validate_id("catalog reference", identity)?;
            }
        }
        if !values.4.is_empty() {
            validate_persisted_token("catalog synthesis", &values.4, MAX_ID_BYTES)?;
        }
        for value in [&values.0, &values.1, &values.2, &values.3, &values.4] {
            if !value.is_empty()
                && (value.len() > MAX_VALIDATION_ROW_BYTES || unsafe_rule_text(value))
            {
                return Err("Archaeology final rule catalog contains unsafe text".into());
            }
        }
        if values.5.len() > MAX_VALIDATION_ROW_BYTES || unsafe_rule_text(&values.5) {
            return Err("Archaeology final rule catalog contains unsafe metadata".into());
        }
        let metadata: Value = serde_json::from_str(&values.5)
            .map_err(|_| "Archaeology final rule catalog metadata is invalid".to_string())?;
        if let Value::Array(values) = metadata {
            if values.len() > MAX_RULE_CAVEATS
                || values.iter().any(|value| {
                    value.as_str().is_none_or(|value| {
                        value.len() > MAX_RULE_CLAUSE_TEXT_BYTES || unsafe_rule_text(value)
                    })
                })
            {
                return Err("Archaeology final rule catalog caveats are unsafe".into());
            }
        } else {
            parse_coverage(&values.5, "final rule")?;
        }
    }
    Ok(())
}

fn validate_model_synthesis_cache(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "SELECT DISTINCT cache.cache_key,cache.request_id,cache.packet_id,
                    cache.response_json,cache.response_sha256
             FROM archaeology_rules rule JOIN archaeology_synthesis_cache cache
               ON cache.generation_id=rule.generation_id
              AND cache.cache_key=rule.synthesis_identity
             WHERE rule.generation_id=?1 AND rule.trust='model_synthesized'
               AND cache.status='ready' ORDER BY cache.cache_key",
        )
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "prepare model", error))?;
    let rows = statement
        .query_map([input.generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| synthesis_catalog_sql_error(input.cancellation, "query model", error))?;
    for row in rows {
        synthesis_catalog_cancelled(input.cancellation)?;
        let (cache_key, request_id, packet_id, json, hash) = row.map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "read model", error)
        })?;
        let response: ArchaeologySynthesisResponse = serde_json::from_str(&json)
            .map_err(|_| "Stored archaeology model synthesis response is invalid".to_string())?;
        validate_persisted_token("synthesis cache", &cache_key, MAX_ID_BYTES)?;
        if sha256_identity(json.as_bytes()) != hash {
            return Err("Stored archaeology model synthesis response hash is invalid".into());
        }
        if response.schema_version != ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION
            || response.contract_id != ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID
            || response.request_id != request_id
            || response.packet_id != packet_id
            || response.clauses.is_empty()
            || unsafe_rule_text(&json)
        {
            return Err("Stored archaeology model synthesis response is outside its scope".into());
        }
    }
    Ok(())
}

fn validate_model_rule_evidence(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
) -> Result<(), String> {
    let violations: i64 = transaction
        .query_row(
            "WITH model_rules AS MATERIALIZED (
                SELECT rule.generation_id,rule.rule_id,cache.response_json
                FROM archaeology_rules rule JOIN archaeology_synthesis_cache cache
                  ON cache.generation_id=rule.generation_id
                 AND cache.cache_key=rule.synthesis_identity
                WHERE rule.generation_id=?1 AND rule.trust='model_synthesized'
                  AND cache.status='ready'
             ), model_clauses AS MATERIALIZED (
                SELECT rule.generation_id,rule.rule_id,clause.clause_id,clause.ordinal,
                       json_extract(rule.response_json,
                         '$.clauses['||clause.ordinal||']') response_clause
                FROM model_rules rule JOIN archaeology_rule_clauses clause
                  ON clause.generation_id=rule.generation_id
                 AND clause.rule_id=rule.rule_id
             ), expected_support AS MATERIALIZED (
                SELECT generation_id,rule_id,clause_id,value fact_id
                FROM model_clauses,json_each(response_clause,'$.subject.fact_ids')
                UNION SELECT generation_id,rule_id,clause_id,value
                FROM model_clauses,json_each(response_clause,'$.action.fact_ids')
                UNION SELECT generation_id,rule_id,clause_id,value
                FROM model_clauses,json_each(response_clause,'$.condition.fact_ids')
                UNION SELECT generation_id,rule_id,clause_id,value
                FROM model_clauses,json_each(response_clause,'$.exception.fact_ids')
                UNION SELECT generation_id,rule_id,clause_id,value
                FROM model_clauses,json_each(response_clause,'$.quantifier.fact_ids')
             ), actual_support AS MATERIALIZED (
                SELECT link.generation_id,clause.rule_id,clause.clause_id,link.evidence_id fact_id
                FROM model_clauses clause JOIN archaeology_evidence_links link
                  ON link.generation_id=clause.generation_id
                 AND link.owner_kind='rule_clause' AND link.owner_id=clause.clause_id
                 AND link.evidence_kind='fact' AND link.role='supporting'
             ), expected_contradiction AS MATERIALIZED (
                SELECT generation_id,rule_id,clause_id,value fact_id
                FROM model_clauses,json_each(response_clause,'$.contradicting_fact_ids')
             ), actual_contradiction AS MATERIALIZED (
                SELECT link.generation_id,clause.rule_id,clause.clause_id,link.evidence_id fact_id
                FROM model_clauses clause JOIN archaeology_evidence_links link
                  ON link.generation_id=clause.generation_id
                 AND link.owner_kind='rule_clause' AND link.owner_id=clause.clause_id
                 AND link.evidence_kind='fact' AND link.role='contradicting'
             ), expected_spans AS MATERIALIZED (
                SELECT support.generation_id,support.rule_id,support.clause_id,
                       'supporting:'||fact_span.evidence_id span_id
                FROM expected_support support JOIN archaeology_evidence_links fact_span
                  ON fact_span.generation_id=support.generation_id
                 AND fact_span.owner_kind='fact' AND fact_span.owner_id=support.fact_id
                 AND fact_span.evidence_kind='span' AND fact_span.role='supporting'
                GROUP BY support.generation_id,support.rule_id,support.clause_id,
                         fact_span.evidence_id
                UNION ALL
                SELECT contradiction.generation_id,contradiction.rule_id,
                       contradiction.clause_id,
                       'contradicting:'||fact_span.evidence_id
                FROM expected_contradiction contradiction
                JOIN archaeology_evidence_links fact_span
                  ON fact_span.generation_id=contradiction.generation_id
                 AND fact_span.owner_kind='fact'
                 AND fact_span.owner_id=contradiction.fact_id
                 AND fact_span.evidence_kind='span' AND fact_span.role='supporting'
                GROUP BY contradiction.generation_id,contradiction.rule_id,
                         contradiction.clause_id,fact_span.evidence_id
             ), actual_spans AS MATERIALIZED (
                SELECT link.generation_id,clause.rule_id,clause.clause_id,
                       link.role||':'||link.evidence_id span_id
                FROM model_clauses clause JOIN archaeology_evidence_links link
                  ON link.generation_id=clause.generation_id
                 AND link.owner_kind='rule_clause' AND link.owner_id=clause.clause_id
                 AND link.evidence_kind='span'
                 AND link.role IN ('supporting','contradicting')
             ), expected_relationships AS MATERIALIZED (
                SELECT generation_id,rule_id,clause_id,value relationship_id
                FROM model_clauses,json_each(response_clause,'$.relationship_ids')
             ), differences AS (
                SELECT * FROM (SELECT * FROM expected_support EXCEPT SELECT * FROM actual_support)
                UNION ALL SELECT * FROM (SELECT * FROM actual_support EXCEPT SELECT * FROM expected_support)
                UNION ALL SELECT * FROM (SELECT * FROM expected_contradiction EXCEPT SELECT * FROM actual_contradiction)
                UNION ALL SELECT * FROM (SELECT * FROM actual_contradiction EXCEPT SELECT * FROM expected_contradiction)
                UNION ALL SELECT * FROM (SELECT * FROM expected_spans EXCEPT SELECT * FROM actual_spans)
                UNION ALL SELECT * FROM (SELECT * FROM actual_spans EXCEPT SELECT * FROM expected_spans)
             )
             SELECT
               (SELECT COUNT(*) FROM model_rules rule WHERE
                  (SELECT COUNT(*) FROM archaeology_rule_clauses clause
                   WHERE clause.generation_id=rule.generation_id
                     AND clause.rule_id=rule.rule_id)
                  !=json_array_length(rule.response_json,'$.clauses'))
             + (SELECT COUNT(*) FROM model_clauses WHERE response_clause IS NULL)
             + (SELECT COUNT(*) FROM differences)
             + (SELECT COUNT(*) FROM expected_relationships expected
                LEFT JOIN archaeology_fact_edges edge
                  ON edge.generation_id=expected.generation_id
                 AND edge.edge_id=expected.relationship_id
                WHERE edge.edge_id IS NULL OR edge.unresolved_reason IS NOT NULL
                  OR edge.trust NOT IN ('extracted','deterministic')
                  OR (edge.kind='contradicts' AND NOT (
                    (EXISTS (SELECT 1 FROM expected_support support
                      WHERE support.generation_id=expected.generation_id
                        AND support.clause_id=expected.clause_id
                        AND support.fact_id=edge.from_fact_id)
                     AND EXISTS (SELECT 1 FROM expected_contradiction contradiction
                      WHERE contradiction.generation_id=expected.generation_id
                        AND contradiction.clause_id=expected.clause_id
                        AND contradiction.fact_id=edge.to_fact_id))
                    OR
                    (EXISTS (SELECT 1 FROM expected_support support
                      WHERE support.generation_id=expected.generation_id
                        AND support.clause_id=expected.clause_id
                        AND support.fact_id=edge.to_fact_id)
                     AND EXISTS (SELECT 1 FROM expected_contradiction contradiction
                      WHERE contradiction.generation_id=expected.generation_id
                        AND contradiction.clause_id=expected.clause_id
                        AND contradiction.fact_id=edge.from_fact_id))))
                  OR (edge.kind!='contradicts' AND NOT (
                    EXISTS (SELECT 1 FROM expected_support support
                      WHERE support.generation_id=expected.generation_id
                        AND support.clause_id=expected.clause_id
                        AND support.fact_id=edge.from_fact_id)
                    AND EXISTS (SELECT 1 FROM expected_support support
                      WHERE support.generation_id=expected.generation_id
                        AND support.clause_id=expected.clause_id
                        AND support.fact_id=edge.to_fact_id))))",
            [input.generation_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(input.cancellation, "validate model evidence", error)
        })?;
    if violations == 0 {
        Ok(())
    } else {
        Err("Archaeology model rule evidence does not match its validated synthesis".into())
    }
}

fn replace_search_manifest(
    transaction: &Transaction<'_>,
    generation_id: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<(), String> {
    synthesis_catalog_cancelled(cancellation)?;
    transaction
        .execute(
            "DELETE FROM archaeology_rule_search_manifest WHERE generation_id=?1",
            [generation_id],
        )
        .map_err(|error| synthesis_catalog_sql_error(cancellation, "clear manifest", error))?;
    let remaining_fts: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?1",
            [generation_id],
            |row| row.get(0),
        )
        .map_err(|error| synthesis_catalog_sql_error(cancellation, "clear FTS", error))?;
    if remaining_fts != 0 {
        return Err("Archaeology FTS linkage did not clear with its manifest".into());
    }
    synthesis_catalog_cancelled(cancellation)?;
    let sql = search_expected_sql(
        "INSERT INTO archaeology_rule_search_manifest
         (generation_id,rule_id,title,clause_text,domain_text)
         SELECT generation_id,rule_id,title,clause_text,domain_text
         FROM expected ORDER BY rule_id",
    );
    let inserted = transaction
        .execute(&sql, [generation_id])
        .map_err(|error| {
            synthesis_catalog_sql_error(cancellation, "materialize manifest", error)
        })?;
    let expected: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rules rule
             WHERE rule.generation_id=?1 AND NOT EXISTS (
               SELECT 1 FROM archaeology_rule_relations relation
               WHERE relation.generation_id=rule.generation_id
                 AND relation.from_rule_id=rule.rule_id AND relation.kind='aliases')",
            [generation_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            synthesis_catalog_sql_error(cancellation, "count canonical rules", error)
        })?;
    if usize::try_from(expected).ok() != Some(inserted) {
        return Err("Archaeology search manifest row count did not reconcile".into());
    }
    Ok(())
}

fn synthesis_catalog_receipt(
    transaction: &Transaction<'_>,
    input: &ArchaeologySynthesisCatalogStage<'_>,
) -> Result<String, String> {
    let mut seals = BTreeMap::new();
    for (name, table, columns, order) in [
        ("rules", "archaeology_rules", "rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,confidence,parser_identity,algorithm_identity,synthesis_identity,coverage_json,identity_schema_version,stable_rule_identity,evidence_identity,contradiction_identity,description_identity,continuity_identity,parser_compatibility_identity,identity_provenance_json", "rule_id"),
        ("clauses", "archaeology_rule_clauses", "rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json", "rule_id,ordinal,clause_id"),
        // The compact store is the authoritative physical representation.
        // Seal stable code and opaque identity values directly, avoiding the
        // compatibility view's per-row text CASE expansion.
        ("evidence", COMPACT_EVIDENCE_SEAL_TABLE, COMPACT_EVIDENCE_SEAL_COLUMNS, COMPACT_EVIDENCE_SEAL_COLUMNS),
        ("domains", "archaeology_rule_domains", "rule_id,domain_id,domain_label,parent_domain_id", "rule_id,domain_id"),
        ("relations", "archaeology_rule_relations", "relation_id,from_rule_id,to_rule_id,kind,trust,summary", "relation_id"),
    ] {
        synthesis_catalog_cancelled(input.cancellation)?;
        seals.insert(
            name,
            table_seal(transaction, input.generation_id, name, table, columns, order)?,
        );
    }
    // Search linkage is sealed and compared immediately before every receipt
    // calculation (including a validate-stage retry). Keeping those two wide
    // text projections out of this second generic seal avoids re-hashing the
    // exact same rows while retaining both the integrity check and retry
    // corruption detection.
    let payload = serde_json::to_vec(&(
        input.repository_id,
        input.generation_id,
        input.identity.revision_sha,
        input.identity.parser,
        input.identity.algorithm,
        input.identity.config,
        seals,
    ))
    .map_err(|error| format!("Encode archaeology synthesis catalog receipt: {error}"))?;
    Ok(digest_identity(
        &payload,
        "archaeology-synthesis-catalog:v1:",
    ))
}

fn query_generation_json<T: DeserializeOwned>(
    transaction: &Transaction<'_>,
    generation_id: &str,
    sql: &str,
    label: &str,
    cancellation_error: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<Vec<T>, String> {
    let mut statement = transaction
        .prepare(sql)
        .map_err(|error| format!("Prepare archaeology {label}: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Query archaeology {label}: {error}"))?;
    rows.map(|row| {
        if cancellation.is_cancelled() {
            return Err(cancellation_error.to_string());
        }
        serde_json::from_str(
            &row.map_err(|_| format!("Stored archaeology {label} is not valid UTF-8"))?,
        )
        .map_err(|_| format!("Stored archaeology {label} is invalid"))
    })
    .collect()
}

fn count_exceeds(value: i64, limit: usize) -> bool {
    usize::try_from(value).map_or(true, |value| value > limit)
}

fn fact_contains_secret(fact: &ArchaeologyFact) -> bool {
    looks_like_secret(&fact.label)
        || fact.attributes.iter().any(|attribute| {
            looks_like_secret(&attribute.key)
                || looks_like_secret(&attribute.value)
                || looks_like_secret(&format!("{}={}", attribute.key, attribute.value))
        })
}

/// Seal the validated generation shape into the existing bounded checkpoint.
/// Generic checkpoints cannot cross this boundary because publication must be
/// able to recompute the exact receipt in its own transaction.
pub(crate) fn validate_generation_for_publication(
    connection: &Connection,
    input: ArchaeologyPublication<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology validation transaction: {error}"))?;
    let (checkpoint_json, completed_units, total_units): (String, i64, Option<i64>) = transaction
        .query_row(
            "SELECT checkpoint_json, completed_units, total_units
             FROM archaeology_jobs
             WHERE job_id = ?1 AND repository_id = ?2 AND generation_id = ?3
               AND owner_id = ?4 AND state = 'running' AND stage = 'validate'
               AND cancellation_requested = 0
               AND julianday(?5) >= julianday(updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                input.now,
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| format!("Load archaeology validation checkpoint: {error}"))?
        .ok_or_else(|| cas_error("validate", input.job_id))?;
    let prior_checkpoint: ArchaeologyJobCheckpoint = serde_json::from_str(&checkpoint_json)
        .map_err(|_| "Stored archaeology pre-validation checkpoint is invalid".to_string())?;
    let inventory_complete = prior_checkpoint.counters.get(INVENTORY_COMPLETE_COUNTER) == Some(&1);
    let receipt = build_validation_receipt(
        &transaction,
        &input,
        inventory_complete && completed_units == 0 && total_units == Some(0),
    )?;
    let receipt_json = encode_validation_receipt(&receipt)?;
    let receipt_identity = validation_receipt_identity(&receipt_json);
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET stage = 'publish', checkpoint_identity = ?5,
                 checkpoint_json = ?6, updated_at = ?7
             WHERE job_id = ?1 AND repository_id = ?2 AND generation_id = ?3
               AND owner_id = ?4 AND state = 'running' AND stage = 'validate'
               AND cancellation_requested = 0
               AND julianday(?7) >= julianday(updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                receipt_identity,
                receipt_json,
                input.now,
            ],
        )
        .map_err(|error| format!("Persist archaeology validation receipt: {error}"))?;
    require_cas(changed, "validation receipt", input.job_id)?;
    let generation_changed = transaction
        .execute(
            "UPDATE archaeology_generations
             SET source_unit_count = ?2, fact_count = ?3, rule_count = ?4
             WHERE generation_id = ?1 AND status = 'staging'",
            params![
                input.generation_id,
                to_i64(snapshot_count(&receipt.snapshot, "source_units"))?,
                to_i64(snapshot_count(&receipt.snapshot, "facts"))?,
                to_i64(snapshot_count(&receipt.snapshot, "rules"))?,
            ],
        )
        .map_err(|error| format!("Persist archaeology validated counts: {error}"))?;
    require_cas(generation_changed, "validated counts", input.job_id)?;
    let status = load_job(&transaction, input.job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology validation receipt: {error}"))?;
    Ok(status)
}

/// Atomically make one fully validated staging generation visible. Retrying
/// the exact publication after a successful commit is a read-only success.
pub(crate) fn publish_generation(
    connection: &Connection,
    input: ArchaeologyPublication<'_>,
) -> Result<ArchaeologyJobStatus, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let stage_started = Instant::now();
    validate_owned_generation(
        input.job_id,
        input.repository_id,
        input.generation_id,
        input.owner_id,
        &input.identity,
        input.now,
    )?;

    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("Start archaeology publication transaction: {error}"))?;

    verify_validation_receipt(&transaction, &input)?;
    profile_archaeology_stage(profiling, "publish.verify_receipt", stage_started);

    if publication_is_already_committed(&transaction, &input)? {
        let status = load_job(&transaction, input.job_id)?;
        transaction
            .commit()
            .map_err(|error| format!("Commit archaeology publication retry: {error}"))?;
        return Ok(status);
    }

    let (prior_ready, repo_path) = transaction
        .query_row(
            "SELECT ready_generation_id,repo_path FROM archaeology_repositories
             WHERE repository_id = ?1",
            [input.repository_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load archaeology ready pointer: {error}"))?
        .ok_or_else(|| "Archaeology repository does not exist".to_string())?;
    validate_ready_pointer(&transaction, input.repository_id, prior_ready.as_deref())?;
    reconcile_generation_lifecycle(
        &transaction,
        input.repository_id,
        input.generation_id,
        prior_ready.as_deref(),
        input.now,
    )?;
    let temporal_prior =
        compatible_temporal_prior(&transaction, input.repository_id, prior_ready.as_deref())?;
    let temporal_prior_revision = temporal_prior
        .map(|generation_id| {
            transaction
                .query_row(
                    "SELECT revision_sha FROM archaeology_generations
                     WHERE repository_id=?1 AND generation_id=?2 AND status='ready'",
                    params![input.repository_id, generation_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| format!("Load temporal prior revision: {error}"))
        })
        .transpose()?;
    let history_context = resolve_archaeology_temporal_context(
        &transaction,
        &repo_path,
        input.identity.revision_sha,
        temporal_prior_revision.as_deref(),
    )?;
    persist_temporal_projection(
        &transaction,
        ArchaeologyTemporalProjection {
            repository_id: input.repository_id,
            generation_id: input.generation_id,
            prior_generation_id: temporal_prior,
            history_coverage: ArchaeologyTemporalCoverageInput {
                state: match history_context.coverage_state {
                    PersistedTemporalCoverageState::Complete => {
                        ArchaeologyTemporalCoverageState::Complete
                    }
                    PersistedTemporalCoverageState::Partial => {
                        ArchaeologyTemporalCoverageState::Partial
                    }
                },
                reasons: history_context.coverage_reasons,
            },
            created_at: input.now,
            limits: ArchaeologyTemporalLimits {
                max_clauses_per_rule: MAX_RULE_CLAUSES,
                ..Default::default()
            },
        },
    )?;
    profile_archaeology_stage(profiling, "publish.temporal", stage_started);

    let job_changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET stage = 'cleanup', updated_at = ?5
             WHERE job_id = ?1 AND repository_id = ?2 AND generation_id = ?3
               AND owner_id = ?4 AND state = 'running' AND stage = 'publish'
               AND cancellation_requested = 0
               AND julianday(?5) >= julianday(updated_at)",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                input.now,
            ],
        )
        .map_err(|error| format!("Claim archaeology publication stage: {error}"))?;
    require_cas(job_changed, "publish", input.job_id)?;

    if let Some(prior_generation_id) = prior_ready.as_deref() {
        let superseded = transaction
            .execute(
                "UPDATE archaeology_generations SET status = 'superseded'
                 WHERE generation_id = ?1 AND repository_id = ?2 AND status = 'ready'",
                params![prior_generation_id, input.repository_id],
            )
            .map_err(|error| format!("Supersede prior archaeology generation: {error}"))?;
        require_cas(superseded, "supersede", input.job_id)?;
    }

    let published = transaction
        .execute(
            "UPDATE archaeology_generations
             SET status = 'ready', published_at = ?10
             WHERE generation_id = ?1 AND repository_id = ?2 AND status = 'staging'
               AND revision_sha = ?3 AND source_identity = ?4
               AND parser_identity = ?5 AND algorithm_identity = ?6
               AND config_identity = ?7 AND schema_version = ?8
               AND EXISTS (
                    SELECT 1 FROM archaeology_jobs
                    WHERE job_id = ?9 AND generation_id = ?1
                      AND repository_id = ?2 AND owner_id = ?11
                      AND state = 'running' AND stage = 'cleanup'
               )",
            params![
                input.generation_id,
                input.repository_id,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                input.job_id,
                input.now,
                input.owner_id,
            ],
        )
        .map_err(|error| format!("Publish archaeology generation: {error}"))?;
    require_cas(published, "generation publish", input.job_id)?;

    let pointer_changed = transaction
        .execute(
            "UPDATE archaeology_repositories
             SET ready_generation_id = ?3, updated_at = ?4
             WHERE repository_id = ?1 AND ready_generation_id IS ?2
               AND current_revision = ?5 AND source_identity = ?6
               AND julianday(?4) >= julianday(updated_at)",
            params![
                input.repository_id,
                prior_ready,
                input.generation_id,
                input.now,
                input.identity.revision_sha,
                input.identity.source,
            ],
        )
        .map_err(|error| format!("Publish archaeology ready pointer: {error}"))?;
    require_cas(pointer_changed, "ready pointer publish", input.job_id)?;

    let status = load_job(&transaction, input.job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology publication: {error}"))?;
    Ok(status)
}

fn profile_archaeology_stage(enabled: bool, label: &str, started: Instant) {
    if enabled {
        eprintln!(
            "ARCHAEOLOGY_PROFILE\t{label}\t{:.3}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

/// Plan or remove only SQLite resources whose generation ownership can be
/// proven from this job and repository. The bounded batch is intentionally
/// repeatable; callers continue while `truncated` is true.
pub(crate) fn cleanup_generations(
    connection: &Connection,
    input: ArchaeologyCleanup<'_>,
) -> Result<ArchaeologyCleanupReport, String> {
    validate_actor(input.job_id, input.owner_id, input.now)?;
    if input.retain_superseded > MAX_CLEANUP_GENERATIONS {
        return Err(format!(
            "Archaeology cleanup retention exceeds {MAX_CLEANUP_GENERATIONS} generations"
        ));
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology cleanup transaction: {error}"))?;
    let (repository_id, owns_current_lease) = authorize_cleanup(&transaction, &input)?;
    let (candidates, truncated) = cleanup_candidates(
        &transaction,
        &repository_id,
        input.owner_id,
        input.retain_superseded,
        owns_current_lease,
    )?;
    let mut deleted_generations = 0_u64;
    let mut deleted_search_index_rows = 0_u64;
    let mut deleted_synthesis_cache_rows = 0_u64;
    let mut deleted_synthesis_attempt_rows = 0_u64;
    let mut deleted_synthesis_response_bytes = 0_u64;

    if input.mode == ArchaeologyCleanupMode::Apply {
        deleted_search_index_rows = candidates
            .iter()
            .map(|candidate| candidate.search_index_rows)
            .sum();
        deleted_synthesis_cache_rows = candidates
            .iter()
            .map(|candidate| candidate.synthesis_cache_rows)
            .sum();
        deleted_synthesis_attempt_rows = candidates
            .iter()
            .map(|candidate| candidate.synthesis_attempt_rows)
            .sum();
        deleted_synthesis_response_bytes = candidates
            .iter()
            .map(|candidate| candidate.synthesis_response_bytes)
            .sum();
        for candidate in &candidates {
            let generation_rows = transaction
                .execute(
                    "DELETE FROM archaeology_generations
                     WHERE generation_id = ?1 AND repository_id = ?2 AND status = ?3
                       AND generation_id IS NOT (
                            SELECT ready_generation_id FROM archaeology_repositories
                            WHERE repository_id = ?2
                       )
                       AND NOT EXISTS (
                            SELECT 1 FROM archaeology_jobs
                            WHERE generation_id = ?1
                              AND state IN ('pending','running','paused','cancelling')
                       )
                       AND (
                            (status = 'superseded' AND EXISTS (
                                SELECT 1 FROM archaeology_jobs AS lease
                                JOIN archaeology_repositories AS repository
                                  ON repository.repository_id = lease.repository_id
                                WHERE lease.job_id = ?5 AND lease.owner_id = ?4
                                  AND repository.ready_generation_id = lease.generation_id
                            )) OR EXISTS (
                                SELECT 1 FROM archaeology_jobs
                                WHERE generation_id = ?1 AND owner_id = ?4
                                  AND state IN ('failed','cancelled','completed')
                            )
                       )",
                    params![
                        candidate.generation_id,
                        repository_id,
                        candidate.status,
                        input.owner_id,
                        input.job_id,
                    ],
                )
                .map_err(|error| format!("Delete owned archaeology generation: {error}"))?;
            require_cas(generation_rows, "cleanup generation", input.job_id)?;
            deleted_generations += 1;
        }
    }

    let report = ArchaeologyCleanupReport {
        dry_run: input.mode == ArchaeologyCleanupMode::DryRun,
        repository_id,
        candidates,
        truncated,
        deleted_generations,
        deleted_search_index_rows,
        deleted_synthesis_cache_rows,
        deleted_synthesis_attempt_rows,
        deleted_synthesis_response_bytes,
        unavailable_resources: vec!["parser_cache".to_string()],
    };
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology cleanup: {error}"))?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn checkpoint_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    expected_stage: ArchaeologyJobStage,
    next_stage: ArchaeologyJobStage,
    checkpoint_identity: &str,
    checkpoint: &ArchaeologyJobCheckpoint,
    completed_units: u64,
    total_units: Option<u64>,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    validate_stage_progression(&expected_stage, &next_stage)?;
    validate_persisted_token("checkpoint", checkpoint_identity, MAX_ID_BYTES)?;
    validate_checkpoint(checkpoint)?;
    let checkpoint_json = serde_json::to_string(checkpoint)
        .map_err(|error| format!("Encode archaeology checkpoint: {error}"))?;
    if checkpoint_json.len() > MAX_CHECKPOINT_BYTES {
        return Err(format!(
            "Archaeology checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes"
        ));
    }
    let completed = to_i64(completed_units)?;
    let total = total_units.map(to_i64).transpose()?;
    if total.is_some_and(|value| completed > value) {
        return Err("Completed archaeology units exceed total units".to_string());
    }
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs
             SET stage = ?4, checkpoint_identity = ?5, checkpoint_json = ?6,
                 completed_units = ?7, total_units = COALESCE(?8, total_units),
                 updated_at = ?9
             WHERE job_id = ?1 AND owner_id = ?2 AND state = 'running'
               AND stage = ?3 AND completed_units <= ?7
               AND (total_units IS NULL OR ?8 IS NULL OR total_units = ?8)
               AND (COALESCE(?8, total_units) IS NULL
                    OR ?7 <= COALESCE(?8, total_units))
               AND (?4 != ?3 OR completed_units < ?7 OR checkpoint_identity IS NULL
                    OR checkpoint_identity = ?5)
               AND julianday(?9) >= julianday(updated_at)",
            params![
                job_id,
                owner_id,
                stage_name(&expected_stage),
                stage_name(&next_stage),
                checkpoint_identity,
                checkpoint_json,
                completed,
                total,
                now,
            ],
        )
        .map_err(|error| format!("Checkpoint archaeology job: {error}"))?;
    require_cas(changed, "checkpoint", job_id)?;
    load_job(connection, job_id)
}

pub(crate) fn heartbeat_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs SET updated_at = ?3
             WHERE job_id = ?1 AND owner_id = ?2
               AND state IN ('running','cancelling')
               AND julianday(?3) >= julianday(updated_at)",
            params![job_id, owner_id, now],
        )
        .map_err(|error| format!("Heartbeat archaeology job: {error}"))?;
    require_cas(changed, "heartbeat", job_id)?;
    load_job(connection, job_id)
}

pub(crate) fn pause_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    transition_state(
        connection, job_id, owner_id, "running", "paused", false, now,
    )
}

pub(crate) fn resume_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    transition_state(
        connection, job_id, owner_id, "paused", "running", false, now,
    )
}

pub(crate) fn request_cancel(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs
             SET state = 'cancelling', cancellation_requested = 1, updated_at = ?3
             WHERE job_id = ?1 AND owner_id = ?2
               AND state IN ('running','paused')
               AND julianday(?3) >= julianday(updated_at)
               AND EXISTS (
                    SELECT 1 FROM archaeology_generations AS generation
                    WHERE generation.generation_id = archaeology_jobs.generation_id
                      AND (
                           generation.status = 'staging'
                           OR (archaeology_jobs.stage = 'cleanup'
                               AND generation.status = 'ready')
                      )
               )",
            params![job_id, owner_id, now],
        )
        .map_err(|error| format!("Request archaeology cancellation: {error}"))?;
    require_cas(changed, "cancel", job_id)?;
    load_job(connection, job_id)
}

pub(crate) fn acknowledge_cancel(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    finish_job(
        connection,
        job_id,
        owner_id,
        "cancelling",
        "cancelled",
        Some("cancelled"),
        now,
    )
}

pub(crate) fn complete_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology completion transaction: {error}"))?;
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET state = 'completed', stage = 'idle', finished_at = ?3, updated_at = ?3
             WHERE job_id = ?1 AND owner_id = ?2 AND state = 'running'
               AND stage = 'cleanup' AND cancellation_requested = 0
               AND julianday(?3) >= julianday(updated_at)
               AND EXISTS (
                    SELECT 1 FROM archaeology_generations AS generation
                    WHERE generation.generation_id = archaeology_jobs.generation_id
                      AND generation.repository_id = archaeology_jobs.repository_id
                      AND generation.status = 'ready'
               )",
            params![job_id, owner_id, now],
        )
        .map_err(|error| format!("Complete archaeology job: {error}"))?;
    require_cas(changed, "complete", job_id)?;
    let status = load_job(&transaction, job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology completion: {error}"))?;
    Ok(status)
}

pub(crate) fn fail_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    error_code: ArchaeologyJobErrorCode,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|database_error| {
            format!("Start archaeology failure transaction: {database_error}")
        })?;
    let current: String = transaction
        .query_row(
            "SELECT errors_json FROM archaeology_jobs
             WHERE job_id = ?1 AND owner_id = ?2
               AND state IN ('running','paused','cancelling')",
            params![job_id, owner_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|database_error| format!("Load archaeology errors: {database_error}"))?
        .ok_or_else(|| cas_error("fail", job_id))?;
    let mut errors: Vec<String> =
        serde_json::from_str(&current).map_err(|_| "Stored archaeology errors are invalid")?;
    if errors.len() >= MAX_ERRORS {
        return Err(format!(
            "Archaeology job retains at most {MAX_ERRORS} errors"
        ));
    }
    errors.push(error_code_name(error_code).to_string());
    let errors_json = serde_json::to_string(&errors).map_err(|value| value.to_string())?;
    if errors_json.len() > MAX_ERRORS_JSON_BYTES {
        return Err(format!(
            "Archaeology errors exceed {MAX_ERRORS_JSON_BYTES} bytes"
        ));
    }
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET state = 'failed', stage = 'idle', errors_json = ?3,
                 finished_at = ?4, updated_at = ?4
             WHERE job_id = ?1 AND owner_id = ?2
               AND state IN ('running','paused','cancelling')
               AND julianday(?4) >= julianday(updated_at)",
            params![job_id, owner_id, errors_json, now],
        )
        .map_err(|database_error| format!("Fail archaeology job: {database_error}"))?;
    require_cas(changed, "fail", job_id)?;
    update_staging_generation(&transaction, job_id, "failed")?;
    let status = load_job(&transaction, job_id)?;
    transaction
        .commit()
        .map_err(|database_error| format!("Commit archaeology failure: {database_error}"))?;
    Ok(status)
}

/// Transfer an expired active job. Running work becomes paused for explicit
/// resume; paused and cancelling intent are preserved. Retrying with the same
/// new owner is idempotent.
pub(crate) fn recover_stale_job(
    connection: &Connection,
    repository_id: &str,
    new_owner_id: &str,
    stale_before: &str,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_id("repository", repository_id)?;
    validate_id("owner", new_owner_id)?;
    let stale_before_time = validate_timestamp(stale_before)?;
    let now_time = validate_timestamp(now)?;
    if stale_before_time > now_time {
        return Err("Archaeology stale cutoff cannot be later than now".to_string());
    }
    let row = connection
        .query_row(
            "SELECT job_id, owner_id, state, updated_at
             FROM archaeology_jobs
             WHERE repository_id = ?1
               AND state IN ('pending','running','paused','cancelling')",
            [repository_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Find active archaeology job: {error}"))?
        .ok_or_else(|| "No active archaeology job exists".to_string())?;
    if row.1 == new_owner_id {
        return load_job(connection, &row.0);
    }
    if validate_timestamp(&row.3)? >= stale_before_time {
        return Err("Active archaeology job owner heartbeat is still live".to_string());
    }
    let recovered_state = if row.2 == "running" {
        "paused"
    } else {
        row.2.as_str()
    };
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs
             SET owner_id = ?5, state = ?6, updated_at = ?7
             WHERE job_id = ?1 AND repository_id = ?2 AND owner_id = ?3
               AND state = ?4 AND updated_at = ?8",
            params![
                row.0,
                repository_id,
                row.1,
                row.2,
                new_owner_id,
                recovered_state,
                now,
                row.3,
            ],
        )
        .map_err(|error| format!("Recover stale archaeology job: {error}"))?;
    require_cas(changed, "recover", &row.0)?;
    load_job(connection, &row.0)
}

pub(crate) fn load_job(
    connection: &Connection,
    job_id: &str,
) -> Result<ArchaeologyJobStatus, String> {
    type Row = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<String>,
        i64,
        String,
        String,
    );
    let row: Row = connection
        .query_row(
            "SELECT job_id, repository_id, generation_id, owner_id, stage, state,
                    completed_units, total_units, checkpoint_identity,
                    cancellation_requested, errors_json, updated_at
             FROM archaeology_jobs WHERE job_id = ?1",
            [job_id],
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
                    row.get(10)?,
                    row.get(11)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology job: {error}"))?
        .ok_or_else(|| "Archaeology job does not exist".to_string())?;
    Ok(ArchaeologyJobStatus {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        job_id: Some(row.0),
        repository_id: Some(row.1),
        generation_id: row.2,
        owner_id: Some(row.3),
        stage: parse_stage(&row.4)?,
        state: parse_state(&row.5)?,
        completed_units: u64::try_from(row.6).map_err(|_| "Negative completed units")?,
        total_units: row
            .7
            .map(u64::try_from)
            .transpose()
            .map_err(|_| "Negative total units")?,
        checkpoint_identity: row.8,
        cancellation_requested: row.9 != 0,
        coverage: ArchaeologyCoverage::default(),
        updated_at: Some(row.11),
        errors: serde_json::from_str(&row.10)
            .map_err(|_| "Stored archaeology errors are invalid")?,
    })
}

fn publication_is_already_committed(
    transaction: &Transaction<'_>,
    input: &ArchaeologyPublication<'_>,
) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM archaeology_jobs AS job
                JOIN archaeology_generations AS generation
                  ON generation.generation_id = job.generation_id
                JOIN archaeology_repositories AS repository
                  ON repository.repository_id = job.repository_id
                WHERE job.job_id = ?1 AND job.repository_id = ?2
                  AND job.generation_id = ?3 AND job.owner_id = ?4
                  AND job.state IN ('running','completed','failed','cancelled')
                  AND job.stage IN ('cleanup','idle')
                  AND generation.status = 'ready'
                  AND generation.revision_sha = ?5
                  AND generation.source_identity = ?6
                  AND generation.parser_identity = ?7
                  AND generation.algorithm_identity = ?8
                  AND generation.config_identity = ?9
                  AND generation.schema_version = ?10
                  AND repository.ready_generation_id = ?3
            )",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Check archaeology publication retry: {error}"))
}

fn verify_validation_receipt(
    transaction: &Transaction<'_>,
    input: &ArchaeologyPublication<'_>,
) -> Result<(), String> {
    let (identity, json): (String, String) = transaction
        .query_row(
            "SELECT checkpoint_identity, checkpoint_json FROM archaeology_jobs
             WHERE job_id = ?1 AND repository_id = ?2 AND generation_id = ?3
               AND owner_id = ?4
               AND state IN ('running','completed','failed','cancelled')
               AND stage IN ('publish','cleanup','idle')",
            params![
                input.job_id,
                input.repository_id,
                input.generation_id,
                input.owner_id,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load archaeology validation receipt: {error}"))?
        .ok_or_else(|| "Archaeology publication requires a validation receipt".to_string())?;
    if json.len() > MAX_CHECKPOINT_BYTES || validation_receipt_identity(&json) != identity {
        return Err("Archaeology validation receipt identity is invalid".to_string());
    }
    let stored: ArchaeologyValidationReceipt = serde_json::from_str(&json)
        .map_err(|_| "Stored archaeology validation receipt is invalid".to_string())?;
    if encode_validation_receipt(&stored)? != json {
        return Err("Stored archaeology validation receipt is not canonical".to_string());
    }
    let current =
        build_validation_receipt(transaction, input, stored.snapshot.empty_inventory_proven)?;
    verify_persisted_counts(transaction, input.generation_id, &current.snapshot)?;
    if stored == current {
        Ok(())
    } else {
        Err("Archaeology generation changed after validation".to_string())
    }
}

fn build_validation_receipt(
    transaction: &Transaction<'_>,
    input: &ArchaeologyPublication<'_>,
    empty_inventory_proven: bool,
) -> Result<ArchaeologyValidationReceipt, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    let coverage_json: String = transaction
        .query_row(
            "SELECT coverage_json FROM archaeology_generations
             WHERE generation_id = ?1 AND repository_id = ?2
               AND revision_sha = ?3 AND source_identity = ?4
               AND parser_identity = ?5 AND algorithm_identity = ?6
               AND config_identity = ?7 AND schema_version = ?8
               AND status IN ('staging','ready')",
            params![
                input.generation_id,
                input.repository_id,
                input.identity.revision_sha,
                input.identity.source,
                input.identity.parser,
                input.identity.algorithm,
                input.identity.config,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Load archaeology generation for validation: {error}"))?
        .ok_or_else(|| "Archaeology staging generation identity changed".to_string())?;
    let totals = validate_integrity(transaction, input)?;
    profile_archaeology_stage(profiling, "validation.integrity", started);
    let tables = validation_table_seals(transaction, input.generation_id)?;
    profile_archaeology_stage(profiling, "validation.seals", started);
    if tables.get("search_manifest") != tables.get("fts") {
        return Err("Archaeology FTS linkage does not match its manifest".to_string());
    }
    let count = |table: &str| tables.get(table).map_or(0, |seal| seal.count);
    let empty_inventory = count("source_units") == 0;
    if empty_inventory
        && (tables.values().map(|seal| seal.count).sum::<u64>() != 0 || !empty_inventory_proven)
    {
        return Err(
            "Empty archaeology publication requires completed inventory with total zero"
                .to_string(),
        );
    }
    let coverage = parse_coverage(&coverage_json, "generation")?;
    if (
        coverage.discovered_source_units,
        coverage.indexed_source_units,
        coverage.discovered_bytes,
        coverage.indexed_bytes,
    ) != (
        totals.discovered_units,
        totals.indexed_units,
        totals.discovered_bytes,
        totals.indexed_bytes,
    ) {
        return Err("Archaeology coverage does not match persisted source rows".to_string());
    }
    if !empty_inventory
        && count("facts") == 0
        && matches!(coverage.state, ArchaeologyCoverageState::Complete)
    {
        return Err(
            "Zero-fact archaeology catalogs require explicit incomplete coverage".to_string(),
        );
    }
    Ok(ArchaeologyValidationReceipt {
        version: VALIDATION_RECEIPT_VERSION,
        repository_id: input.repository_id.to_string(),
        generation_id: input.generation_id.to_string(),
        revision_sha: input.identity.revision_sha.to_string(),
        source_identity: input.identity.source.to_string(),
        parser_identity: input.identity.parser.to_string(),
        algorithm_identity: input.identity.algorithm.to_string(),
        config_identity: input.identity.config.to_string(),
        schema_version: ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
        snapshot: ArchaeologyValidationSnapshot {
            empty_inventory_proven: empty_inventory,
            coverage_sha256: sha256_identity(
                &serde_json::to_vec(&coverage).map_err(|error| error.to_string())?,
            ),
            tables,
        },
    })
}

fn parse_coverage(json: &str, label: &str) -> Result<ArchaeologyCoverage, String> {
    if json.len() > MAX_CHECKPOINT_BYTES {
        return Err(format!(
            "Archaeology {label} coverage exceeds its byte bound"
        ));
    }
    let coverage: ArchaeologyCoverage = serde_json::from_str(json)
        .map_err(|_| format!("Archaeology {label} coverage is invalid"))?;
    if matches!(
        coverage.state,
        ArchaeologyCoverageState::Partial | ArchaeologyCoverageState::Unavailable
    ) && coverage.reasons.is_empty()
    {
        return Err(format!(
            "Archaeology {label} partial or unavailable coverage requires a reason"
        ));
    }
    if coverage.reasons.len() > MAX_COVERAGE_REASONS
        || coverage.reasons.iter().any(|reason| {
            reason.is_empty()
                || reason.len() > MAX_COVERAGE_REASON_BYTES
                || looks_like_secret(reason)
                || contains_sensitive_path(reason)
        })
        || coverage.indexed_source_units > coverage.discovered_source_units
        || coverage.indexed_bytes > coverage.discovered_bytes
        || (matches!(coverage.state, ArchaeologyCoverageState::Complete)
            && (coverage.indexed_source_units != coverage.discovered_source_units
                || coverage.indexed_bytes != coverage.discovered_bytes))
    {
        return Err(format!("Archaeology {label} coverage is inconsistent"));
    }
    Ok(coverage)
}

fn parse_parser_manifest(identity: &str) -> Result<BTreeMap<String, String>, String> {
    let entries = identity
        .strip_prefix("parser-manifest:v1:")
        .ok_or_else(|| "Generation parser identity is not a v1 manifest".to_string())?;
    let mut manifest = BTreeMap::new();
    for entry in entries.split(',').filter(|entry| !entry.is_empty()) {
        let (parser_id, version) = entry
            .rsplit_once('@')
            .ok_or_else(|| "Parser manifest entry is malformed".to_string())?;
        validate_persisted_token("parser id", parser_id, 128)?;
        validate_persisted_token("parser version", version, 64)?;
        if manifest
            .insert(parser_id.to_string(), version.to_string())
            .is_some()
        {
            return Err("Parser manifest contains a duplicate parser".to_string());
        }
    }
    let canonical = manifest
        .iter()
        .map(|(parser, version)| format!("{parser}@{version}"))
        .collect::<Vec<_>>()
        .join(",");
    if canonical.is_empty() || format!("parser-manifest:v1:{canonical}") != identity {
        Err("Generation parser manifest is empty or noncanonical".to_string())
    } else {
        Ok(manifest)
    }
}

fn validate_opaque_id(value: &str, kind: &str) -> Result<(), String> {
    let digest = value
        .strip_prefix(kind)
        .and_then(|suffix| suffix.strip_prefix(':'));
    if digest.is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }) {
        Ok(())
    } else {
        Err(format!("Archaeology {kind} identity is not opaque"))
    }
}

fn validate_persisted_path(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || unsafe_persisted_string(value) {
        Err(format!(
            "Archaeology {label} violates the secret/path policy"
        ))
    } else {
        Ok(())
    }
}

fn unsafe_persisted_string(value: &str) -> bool {
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    value.contains('\0')
        || looks_like_secret(value)
        || contains_sensitive_path(value)
        || value.starts_with('/')
        || value.starts_with('\\')
        || value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
        || windows_drive
        || value.split(['/', '\\']).any(|part| part == "..")
}

fn parse_metadata_json<T>(json: &str, label: &str) -> Result<Vec<T>, String>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    if json.len() > MAX_CHECKPOINT_BYTES {
        return Err(format!("Archaeology {label} exceeds its byte bound"));
    }
    let values: Vec<T> =
        serde_json::from_str(json).map_err(|_| format!("Archaeology {label} is invalid"))?;
    if values.len() > 1_024 {
        return Err(format!("Archaeology {label} exceeds its item bound"));
    }
    validate_metadata_strings(
        &serde_json::to_value(&values).map_err(|_| format!("Archaeology {label} is invalid"))?,
        label,
    )?;
    Ok(values)
}

fn validate_metadata_strings(value: &Value, label: &str) -> Result<(), String> {
    match value {
        Value::String(value) if unsafe_persisted_string(value) => Err(format!(
            "Archaeology {label} violates the secret/path policy"
        )),
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_metadata_strings(value, label)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| validate_metadata_strings(value, label)),
        _ => Ok(()),
    }
}

fn canonical_content_hash(hash: Option<&str>, algorithm: Option<&str>) -> bool {
    algorithm == Some("sha256")
        && hash.is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

fn validate_metadata_values(
    owner_id: &str,
    lineage: &[ArchaeologyAdapterLineage],
) -> Result<(), String> {
    for entry in lineage {
        if entry.source_unit_id != owner_id {
            return Err("Archaeology lineage source does not match its owning unit".to_string());
        }
        if !entry.has_honest_target() {
            return Err("Archaeology lineage target metadata is not honestly resolved".to_string());
        }
    }
    Ok(())
}

const METADATA_LINK_INTEGRITY_SQL: &str = "
    WITH metadata(kind,owner_id,source_id,target_id,span_id) AS (
        SELECT 'lineage',unit.source_unit_id,
               json_extract(item.value,'$.source_unit_id'),
               json_extract(item.value,'$.target_source_unit_id'),
               json_extract(item.value,'$.evidence_span_id')
        FROM archaeology_source_units unit, json_each(unit.include_lineage_json) item
        WHERE unit.generation_id=?1
        UNION ALL
        SELECT 'recovery',unit.source_unit_id,unit.source_unit_id,NULL,
               json_extract(item.value,'$.span_id')
        FROM archaeology_source_units unit, json_each(unit.recovery_json) item
        WHERE unit.generation_id=?1
    )
    SELECT COALESCE(MAX(kind='lineage' AND source_id IS NOT owner_id),0),
           COALESCE(MAX(kind='lineage' AND target_id IS NOT NULL AND target.source_unit_id IS NULL),0),
           COALESCE(MAX(kind='lineage' AND span.span_id IS NULL),0),
           COALESCE(MAX(kind='recovery' AND span.span_id IS NULL),0)
    FROM metadata
    LEFT JOIN archaeology_source_units target
          ON target.generation_id=?1 AND target.source_unit_id=metadata.target_id
    LEFT JOIN archaeology_source_spans span
          ON span.generation_id=?1 AND span.span_id=metadata.span_id
         AND span.source_unit_id=metadata.owner_id";

fn validate_metadata_links(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<(), String> {
    let violations: (bool, bool, bool, bool) = transaction
        .query_row(METADATA_LINK_INTEGRITY_SQL, [generation_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|error| format!("Validate archaeology metadata links: {error}"))?;
    let reason = if violations.0 {
        Some("Archaeology lineage source does not match its owning unit")
    } else if violations.1 {
        Some("Archaeology lineage target is outside its generation")
    } else if violations.2 {
        Some("Archaeology lineage evidence span does not belong to its unit")
    } else if violations.3 {
        Some("Archaeology recovery span does not belong to its unit")
    } else {
        None
    };
    reason.map_or(Ok(()), |reason| Err(reason.to_string()))
}

fn validate_integrity(
    transaction: &Transaction<'_>,
    input: &ArchaeologyPublication<'_>,
) -> Result<CoverageTotals, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    let parser_manifest = parse_parser_manifest(input.identity.parser)?;
    let legacy_rules: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rules rule
             JOIN archaeology_generations generation
               ON generation.generation_id=rule.generation_id
             WHERE rule.generation_id=?1 AND generation.schema_version=2
               AND COALESCE(rule.identity_schema_version,0)<>2",
            [input.generation_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Validate archaeology rule identity versions: {error}"))?;
    if legacy_rules != 0 {
        return Err("Storage-v2 archaeology publication contains legacy rule identities".into());
    }
    let mut units = transaction
        .prepare(
            "SELECT unit.source_unit_id, unit.classification, unit.coverage_json,
                unit.path_identity, unit.relative_path, unit.content_hash, unit.hash_algorithm,
                unit.parser_id, unit.parser_version, unit.byte_count, unit.line_count,
                unit.include_lineage_json, unit.recovery_json,
                COUNT(span.span_id), MAX(span.start_column), MAX(span.end_column)
         FROM archaeology_source_units AS unit
         LEFT JOIN archaeology_source_spans AS span
           ON span.generation_id = unit.generation_id
          AND span.source_unit_id = unit.source_unit_id
         WHERE unit.generation_id = ?1
         GROUP BY unit.source_unit_id, unit.classification, unit.coverage_json,
                  unit.path_identity, unit.relative_path, unit.content_hash, unit.hash_algorithm,
                  unit.parser_id, unit.parser_version, unit.byte_count, unit.line_count,
                  unit.include_lineage_json, unit.recovery_json",
        )
        .map_err(|error| format!("Prepare source validation: {error}"))?;
    let rows = units
        .query_map([input.generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, Option<i64>>(14)?,
                row.get::<_, Option<i64>>(15)?,
            ))
        })
        .map_err(|error| format!("Query source validation: {error}"))?;
    let mut totals = CoverageTotals::default();
    for row in rows {
        let (
            id,
            classification,
            coverage_json,
            path_identity,
            path,
            hash,
            hash_algorithm,
            parser_id,
            parser_version,
            byte_count,
            line_count,
            lineage,
            recovery,
            spans,
            max_start_column,
            max_end_column,
        ) = row.map_err(|error| error.to_string())?;
        let coverage = parse_coverage(&coverage_json, "source unit")?;
        validate_opaque_id(&id, "archaeology-source-unit")?;
        validate_opaque_id(&path_identity, "archaeology-path")?;
        if let Some(path) = path.as_deref() {
            validate_persisted_path("relative path", path)?;
        }
        let lineage: Vec<ArchaeologyAdapterLineage> =
            parse_metadata_json(&lineage, "include lineage")?;
        let recovery: Vec<ArchaeologyAdapterRegion> =
            parse_metadata_json(&recovery, "recovery regions")?;
        let excluded = matches!(classification.as_str(), "protected" | "opaque");
        if parser_manifest.get(&parser_id).map(String::as_str) != Some(parser_version.as_str()) {
            return Err(format!(
                "Source unit {id} parser is outside the generation manifest"
            ));
        }
        let max_column = byte_count
            .checked_add(1)
            .ok_or("Source byte count cannot bound span columns")?;
        if max_start_column.is_some_and(|column| column > max_column)
            || max_end_column.is_some_and(|column| column > max_column)
        {
            return Err(format!("Source unit {id} has an out-of-bounds span column"));
        }
        let canonical_hash = canonical_content_hash(hash.as_deref(), hash_algorithm.as_deref());
        if (hash.is_some() || hash_algorithm.is_some()) && !canonical_hash {
            return Err(format!(
                "Source unit {id} has a noncanonical content identity"
            ));
        }
        if excluded && (spans != 0 || canonical_hash || line_count != 0) {
            return Err(format!(
                "Excluded source unit {id} cannot have indexed evidence"
            ));
        }
        if (excluded && (!lineage.is_empty() || !recovery.is_empty()))
            || (classification == "protected" && path.is_some())
        {
            return Err(format!(
                "Excluded source unit {id} retained path or parser metadata"
            ));
        }
        validate_metadata_values(&id, &lineage)?;
        if spans != 0 && !canonical_hash {
            return Err(format!(
                "Evidence-bearing source unit {id} requires a content hash"
            ));
        }
        if spans == 0
            && !matches!(
                coverage.state,
                ArchaeologyCoverageState::Partial | ArchaeologyCoverageState::Unavailable
            )
        {
            return Err(format!(
                "Unspanned source unit {id} requires incomplete coverage"
            ));
        }
        let bytes = u64::try_from(byte_count).map_err(|_| "Negative source byte count")?;
        totals.discovered_units += 1;
        totals.discovered_bytes = totals
            .discovered_bytes
            .checked_add(bytes)
            .ok_or("Discovered source bytes exceed the supported range")?;
        if !excluded && canonical_hash {
            totals.indexed_units += 1;
            totals.indexed_bytes = totals
                .indexed_bytes
                .checked_add(bytes)
                .ok_or("Indexed source bytes exceed the supported range")?;
        }
    }
    profile_archaeology_stage(profiling, "validation.units", started);
    validate_metadata_links(transaction, input.generation_id)?;
    profile_archaeology_stage(profiling, "validation.metadata", started);
    let parser_ids = serde_json::to_string(&parser_manifest.keys().collect::<Vec<_>>())
        .map_err(|error| format!("Encode parser manifest ids: {error}"))?;
    let (
        parser_violations,
        span_revision,
        span_bounds,
        rule_scope,
        uncited_fact,
        uncited_edge,
        rule_no_clause,
        clause_uncited,
        relation_uncited,
        dangling_owner,
        dangling_evidence,
    ): (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = transaction.query_row(
        "WITH evidence AS NOT MATERIALIZED (
           SELECT generation.generation_id,link.owner_kind_code,
                  owner.identity AS owner_id,link.evidence_kind_code,
                  referenced.identity AS evidence_id,link.role_code
           FROM archaeology_evidence_links_compact link
           JOIN archaeology_generation_keys generation
             ON generation.generation_key=link.generation_key
            AND generation.generation_id=?1
           JOIN archaeology_evidence_identities owner
             ON owner.generation_key=link.generation_key
            AND owner.identity_key=link.owner_identity_key
           JOIN archaeology_evidence_identities referenced
             ON referenced.generation_key=link.generation_key
            AND referenced.identity_key=link.evidence_identity_key
         )
         SELECT (SELECT COUNT(*) FROM archaeology_facts WHERE generation_id=?1
                   AND parser_id NOT IN (SELECT value FROM json_each(?6))),
           (SELECT COUNT(*) FROM archaeology_source_spans WHERE generation_id=?1 AND revision_sha!=?2),
           (SELECT COUNT(*) FROM archaeology_source_spans span
              JOIN archaeology_source_units unit
                ON unit.generation_id=span.generation_id
               AND unit.source_unit_id=span.source_unit_id
              WHERE span.generation_id=?1 AND (
                span.end_byte<=span.start_byte OR span.end_byte>unit.byte_count
                OR span.start_line<1 OR span.end_line<span.start_line
                OR span.start_column<1 OR span.end_column<1
                OR (span.end_line=span.start_line AND span.end_column<span.start_column)
                OR unit.line_count<=0 OR span.start_line>unit.line_count
                OR span.end_line>unit.line_count + CASE
                     WHEN span.end_byte=unit.byte_count AND span.end_column=1 THEN 1 ELSE 0 END)),
           (SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1 AND
              (repository_id!=?3 OR revision_sha!=?2 OR parser_identity!=?4 OR algorithm_identity!=?5
               OR (trust='model_synthesized' AND synthesis_identity IS NULL)
               OR (trust IN ('extracted','deterministic') AND synthesis_identity IS NOT NULL))),
           (SELECT COUNT(*) FROM archaeology_facts f WHERE f.generation_id=?1 AND NOT EXISTS (
              SELECT 1 FROM evidence e JOIN archaeology_source_spans s
                ON s.generation_id=e.generation_id AND s.span_id=e.evidence_id
              WHERE e.generation_id=f.generation_id AND e.owner_kind_code=1
                AND e.owner_id=f.fact_id AND e.evidence_kind_code=1 AND e.role_code=1)),
           (SELECT COUNT(*) FROM archaeology_fact_edges x WHERE x.generation_id=?1 AND NOT EXISTS (
              SELECT 1 FROM evidence e JOIN archaeology_source_spans s
                ON s.generation_id=e.generation_id AND s.span_id=e.evidence_id
              WHERE e.generation_id=x.generation_id AND e.owner_kind_code=2
                AND e.owner_id=x.edge_id AND e.evidence_kind_code=1 AND e.role_code=1)),
           (SELECT COUNT(*) FROM archaeology_rules r WHERE r.generation_id=?1 AND NOT EXISTS (
              SELECT 1 FROM archaeology_rule_clauses c WHERE c.generation_id=r.generation_id AND c.rule_id=r.rule_id)),
           (SELECT COUNT(*) FROM archaeology_rule_clauses c WHERE c.generation_id=?1 AND (
              NOT EXISTS (SELECT 1 FROM evidence e WHERE e.generation_id=c.generation_id AND e.owner_kind_code=3 AND e.owner_id=c.clause_id AND e.evidence_kind_code=2 AND e.role_code=1)
              OR NOT EXISTS (SELECT 1 FROM evidence e WHERE e.generation_id=c.generation_id AND e.owner_kind_code=3 AND e.owner_id=c.clause_id AND e.evidence_kind_code=1 AND e.role_code=1))),
           (SELECT COUNT(*) FROM archaeology_rule_relations r WHERE r.generation_id=?1 AND NOT EXISTS (
              SELECT 1 FROM evidence e WHERE e.generation_id=r.generation_id
                AND e.owner_kind_code=4 AND e.owner_id=r.relation_id
                AND e.evidence_kind_code IN (1,2,3) AND e.role_code=1)),
           (SELECT COUNT(*) FROM evidence e WHERE e.generation_id=?1 AND NOT (
              (e.owner_kind_code=1 AND EXISTS (SELECT 1 FROM archaeology_facts x WHERE x.generation_id=e.generation_id AND x.fact_id=e.owner_id)) OR
              (e.owner_kind_code=2 AND EXISTS (SELECT 1 FROM archaeology_fact_edges x WHERE x.generation_id=e.generation_id AND x.edge_id=e.owner_id)) OR
              (e.owner_kind_code=3 AND EXISTS (SELECT 1 FROM archaeology_rule_clauses x WHERE x.generation_id=e.generation_id AND x.clause_id=e.owner_id)) OR
              (e.owner_kind_code=4 AND EXISTS (SELECT 1 FROM archaeology_rule_relations x WHERE x.generation_id=e.generation_id AND x.relation_id=e.owner_id)))),
           (SELECT COUNT(*) FROM evidence e WHERE e.generation_id=?1 AND NOT (
              (e.evidence_kind_code=1 AND EXISTS (SELECT 1 FROM archaeology_source_spans x WHERE x.generation_id=e.generation_id AND x.span_id=e.evidence_id)) OR
              (e.evidence_kind_code=2 AND EXISTS (SELECT 1 FROM archaeology_facts x WHERE x.generation_id=e.generation_id AND x.fact_id=e.evidence_id)) OR
              (e.evidence_kind_code=3 AND EXISTS (SELECT 1 FROM archaeology_rules x WHERE x.generation_id=e.generation_id AND x.rule_id=e.evidence_id))))",
        params![input.generation_id, input.identity.revision_sha, input.repository_id,
                input.identity.parser, input.identity.algorithm, parser_ids],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?))
    ).map_err(|error| format!("Validate evidence integrity: {error}"))?;
    if parser_violations != 0 {
        return Err("Fact parser is outside the generation manifest".to_string());
    }
    if span_revision
        + span_bounds
        + rule_scope
        + uncited_fact
        + uncited_edge
        + rule_no_clause
        + clause_uncited
        + relation_uncited
        + dangling_owner
        + dangling_evidence
        != 0
    {
        return Err(format!(
            "Archaeology evidence validation failed: span_revision={span_revision},span_bounds={span_bounds},rule_scope={rule_scope},uncited_fact={uncited_fact},uncited_edge={uncited_edge},rule_no_clause={rule_no_clause},clause_uncited={clause_uncited},relation_uncited={relation_uncited},dangling_owner={dangling_owner},dangling_evidence={dangling_evidence}"
        ));
    }
    profile_archaeology_stage(profiling, "validation.evidence", started);
    validate_search_integrity(transaction, input.generation_id)?;
    profile_archaeology_stage(profiling, "validation.search", started);
    Ok(totals)
}

const SEARCH_BOUNDS_SQL: &str = "
    WITH clause_bounds AS (
        SELECT rule_id, COUNT(*) AS item_count,
               COALESCE(SUM(length(CAST(clause_text AS BLOB))),0) + COUNT(*) - 1 AS text_bytes
        FROM archaeology_rule_clauses WHERE generation_id=?1 GROUP BY rule_id
    ), domain_bounds AS (
        SELECT rule_id, COUNT(*) AS item_count,
               COALESCE(SUM(length(CAST(domain_label AS BLOB))),0) + COUNT(*) - 1 AS text_bytes
        FROM archaeology_rule_domains WHERE generation_id=?1 GROUP BY rule_id
    )
    SELECT
      (SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1
         AND length(CAST(title AS BLOB))>?2)
    + (SELECT COUNT(*) FROM clause_bounds WHERE item_count>?3 OR text_bytes>?4)
    + (SELECT COUNT(*) FROM domain_bounds WHERE item_count>?5 OR text_bytes>?6)
    + (SELECT COUNT(*) FROM archaeology_rule_search_manifest WHERE generation_id=?1 AND (
         length(CAST(title AS BLOB))>?2 OR length(CAST(clause_text AS BLOB))>?4
         OR length(CAST(domain_text AS BLOB))>?6))
";

fn search_expected_sql(tail: &str) -> String {
    format!(
        "
    WITH aliases AS MATERIALIZED (
        SELECT generation_id, from_rule_id AS rule_id
        FROM archaeology_rule_relations
        WHERE generation_id = ?1 AND kind = 'aliases'
        GROUP BY generation_id, from_rule_id
    ), clause_rows AS (
        SELECT generation_id, rule_id, group_concat(clause_text, char(10)) AS clause_text
        FROM (
            SELECT generation_id, rule_id, clause_text
            FROM archaeology_rule_clauses
            WHERE generation_id = ?1
            ORDER BY rule_id, ordinal, clause_id
        ) GROUP BY generation_id, rule_id
    ), domain_rows AS (
        SELECT generation_id, rule_id, group_concat(domain_label, char(10)) AS domain_text
        FROM (
            SELECT generation_id, rule_id, domain_label
            FROM archaeology_rule_domains
            WHERE generation_id = ?1
            ORDER BY rule_id, domain_id
        ) GROUP BY generation_id, rule_id
    ), expected AS (
        SELECT rule.generation_id, rule.rule_id, rule.title,
               COALESCE(clause_rows.clause_text, '') AS clause_text,
               COALESCE(domain_rows.domain_text, '') AS domain_text
        FROM archaeology_rules AS rule
        LEFT JOIN clause_rows USING (generation_id, rule_id)
        LEFT JOIN domain_rows USING (generation_id, rule_id)
        LEFT JOIN aliases USING (generation_id, rule_id)
        WHERE rule.generation_id = ?1
          AND aliases.rule_id IS NULL
    )
    {tail}"
    )
}

fn search_integrity_sql() -> String {
    search_expected_sql(
        ", actual AS (
        SELECT generation_id, rule_id, title, clause_text, domain_text
        FROM archaeology_rule_search_manifest WHERE generation_id = ?1
    ), mismatches AS (
        SELECT expected.rule_id
        FROM expected LEFT JOIN actual USING (generation_id, rule_id)
        WHERE actual.rule_id IS NULL OR actual.title IS NOT expected.title
           OR actual.clause_text IS NOT expected.clause_text
           OR actual.domain_text IS NOT expected.domain_text
        UNION ALL
        SELECT actual.rule_id
        FROM actual LEFT JOIN expected USING (generation_id, rule_id)
        WHERE expected.rule_id IS NULL
    )
    SELECT COUNT(*) FROM mismatches
",
    )
}

fn validate_search_integrity(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<(), String> {
    let bounds: i64 = transaction
        .query_row(
            SEARCH_BOUNDS_SQL,
            params![
                generation_id,
                MAX_RULE_TITLE_BYTES,
                MAX_RULE_CLAUSES,
                MAX_RULE_CLAUSE_TEXT_BYTES,
                MAX_RULE_DOMAINS,
                MAX_RULE_DOMAIN_TEXT_BYTES
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("Validate search bounds: {error}"))?;
    if bounds != 0 {
        return Err("Archaeology rule or search text exceeds its validation bound".to_string());
    }
    let violations: i64 = transaction
        .query_row(&search_integrity_sql(), [generation_id], |row| row.get(0))
        .map_err(|error| format!("Validate search integrity: {error}"))?;
    if violations != 0 {
        return Err("Archaeology search manifest does not match rules".to_string());
    }
    Ok(())
}

fn validate_search_fts_parity(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<(), String> {
    let manifest = table_seal(
        transaction,
        generation_id,
        "search_manifest",
        "archaeology_rule_search_manifest",
        "rule_id,title,clause_text,domain_text",
        "rule_id,title,clause_text,domain_text",
    )?;
    let fts = table_seal(
        transaction,
        generation_id,
        "fts",
        "archaeology_rule_fts",
        "rule_id,title,clause_text,domain_text",
        "rule_id,title,clause_text,domain_text",
    )?;
    if manifest == fts {
        Ok(())
    } else {
        Err("Archaeology FTS linkage does not match its manifest".into())
    }
}

fn validation_table_seals(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<BTreeMap<String, ArchaeologyTableSeal>, String> {
    const TABLES: &[(&str, &str, &str, &str)] = &[
        ("source_units", "archaeology_source_units", "source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,language,dialect,parser_id,parser_version,classification,byte_count,line_count,include_lineage_json,recovery_json,coverage_json", "source_unit_id"),
        ("source_spans", "archaeology_source_spans", "span_id,source_unit_id,revision_sha,start_byte,end_byte,start_line,start_column,end_line,end_column", "span_id"),
        ("facts", "archaeology_facts", "fact_id,kind,label,parser_id,trust,confidence,attributes_json", "fact_id"),
        ("fact_edges", "archaeology_fact_edges", "edge_id,from_fact_id,to_fact_id,kind,trust,unresolved_reason", "edge_id"),
        ("rules", "archaeology_rules", "rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,confidence,parser_identity,algorithm_identity,synthesis_identity,coverage_json,created_at,identity_schema_version,stable_rule_identity,evidence_identity,contradiction_identity,description_identity,continuity_identity,parser_compatibility_identity,identity_provenance_json", "rule_id"),
        ("rule_clauses", "archaeology_rule_clauses", "rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json", "clause_id"),
        ("evidence", COMPACT_EVIDENCE_SEAL_TABLE, COMPACT_EVIDENCE_SEAL_COLUMNS, COMPACT_EVIDENCE_SEAL_COLUMNS),
        ("domains", "archaeology_rule_domains", "rule_id,domain_id,domain_label,parent_domain_id", "rule_id,domain_id"),
        ("relations", "archaeology_rule_relations", "relation_id,from_rule_id,to_rule_id,kind,trust,summary", "relation_id"),
        ("search_manifest", "archaeology_rule_search_manifest", "rule_id,title,clause_text,domain_text", "rule_id,title,clause_text,domain_text"),
        ("fts", "archaeology_rule_fts", "rule_id,title,clause_text,domain_text", "rule_id,title,clause_text,domain_text"),
    ];
    TABLES
        .iter()
        .map(|(name, table, columns, order)| {
            table_seal(transaction, generation_id, name, table, columns, order)
                .map(|seal| ((*name).to_string(), seal))
        })
        .collect()
}

fn table_seal(
    transaction: &Transaction<'_>,
    generation_id: &str,
    name: &str,
    table: &str,
    columns: &str,
    order: &str,
) -> Result<ArchaeologyTableSeal, String> {
    let mut digest = Sha256::new();
    digest.update(
        if matches!(name, "search_manifest" | "fts") {
            "search_linkage"
        } else {
            name
        }
        .as_bytes(),
    );
    let mut count = 0_u64;
    let sql = format!(
        "SELECT json_array({columns}) FROM {table}
         WHERE generation_id = ?1 ORDER BY {order}"
    );
    let mut statement = transaction
        .prepare(&sql)
        .map_err(|error| format!("Prepare {name} seal: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Query {name} seal: {error}"))?;
    for row in rows {
        let row = row.map_err(|error| format!("Read {name} seal: {error}"))?;
        if row.len() > MAX_VALIDATION_ROW_BYTES {
            return Err(format!(
                "Archaeology {name} row exceeds its validation bound"
            ));
        }
        digest.update((row.len() as u64).to_le_bytes());
        digest.update(row.as_bytes());
        count += 1;
    }
    Ok(ArchaeologyTableSeal {
        count,
        sha256: format!("sha256:{}", super::inventory::hex(&digest.finalize())),
    })
}

fn snapshot_count(snapshot: &ArchaeologyValidationSnapshot, table: &str) -> u64 {
    snapshot.tables.get(table).map_or(0, |seal| seal.count)
}

fn verify_persisted_counts(
    transaction: &Transaction<'_>,
    generation_id: &str,
    snapshot: &ArchaeologyValidationSnapshot,
) -> Result<(), String> {
    let persisted: (i64, i64, i64) = transaction
        .query_row(
            "SELECT source_unit_count, fact_count, rule_count
             FROM archaeology_generations WHERE generation_id = ?1",
            [generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("Read persisted generation counts: {error}"))?;
    if persisted
        == (
            to_i64(snapshot_count(snapshot, "source_units"))?,
            to_i64(snapshot_count(snapshot, "facts"))?,
            to_i64(snapshot_count(snapshot, "rules"))?,
        )
    {
        Ok(())
    } else {
        Err("Persisted archaeology generation counts changed after validation".to_string())
    }
}

fn encode_validation_receipt(receipt: &ArchaeologyValidationReceipt) -> Result<String, String> {
    let json = serde_json::to_string(receipt)
        .map_err(|error| format!("Encode archaeology validation receipt: {error}"))?;
    if json.len() > MAX_CHECKPOINT_BYTES {
        Err(format!(
            "Archaeology validation receipt exceeds {MAX_CHECKPOINT_BYTES} bytes"
        ))
    } else {
        Ok(json)
    }
}

fn validation_receipt_identity(json: &str) -> String {
    digest_identity(
        json.as_bytes(),
        &format!("validation:v{VALIDATION_RECEIPT_VERSION}:"),
    )
}

fn sha256_identity(bytes: &[u8]) -> String {
    digest_identity(bytes, "sha256:")
}

fn digest_identity(bytes: &[u8], prefix: &str) -> String {
    format!("{prefix}{}", super::inventory::hex(&Sha256::digest(bytes)))
}

fn validate_ready_pointer(
    transaction: &Transaction<'_>,
    repository_id: &str,
    ready_generation_id: Option<&str>,
) -> Result<(), String> {
    let (ready_count, pointer_matches): (i64, i64) = transaction
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN generation_id IS ?2 THEN 1 ELSE 0 END), 0)
             FROM archaeology_generations
             WHERE repository_id = ?1 AND status = 'ready'",
            params![repository_id, ready_generation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Validate archaeology ready generation: {error}"))?;
    let expected = i64::from(ready_generation_id.is_some());
    if ready_count == expected && pointer_matches == expected {
        Ok(())
    } else {
        Err("Archaeology ready pointer and ready generation disagree".to_string())
    }
}

fn compatible_temporal_prior<'a>(
    transaction: &Transaction<'_>,
    repository_id: &str,
    prior_ready: Option<&'a str>,
) -> Result<Option<&'a str>, String> {
    let Some(generation_id) = prior_ready else {
        return Ok(None);
    };
    let compatible = transaction
        .query_row(
            "SELECT schema_version = ?3 FROM archaeology_generations
             WHERE repository_id = ?1 AND generation_id = ?2 AND status = 'ready'",
            params![
                repository_id,
                generation_id,
                ARCHAEOLOGY_STORAGE_SCHEMA_VERSION
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Load temporal prior archaeology generation: {error}"))?;
    Ok(compatible.then_some(generation_id))
}

fn authorize_cleanup(
    transaction: &Transaction<'_>,
    input: &ArchaeologyCleanup<'_>,
) -> Result<(String, bool), String> {
    let (repository_id, owns_current_lease) = transaction
        .query_row(
            "SELECT job.repository_id,
                    COALESCE(repository.ready_generation_id = job.generation_id, 0)
             FROM archaeology_jobs AS job
             JOIN archaeology_repositories AS repository
               ON repository.repository_id = job.repository_id
             WHERE job.job_id = ?1 AND job.owner_id = ?2
               AND (
                    (job.state = 'running' AND job.stage = 'cleanup'
                     AND job.cancellation_requested = 0)
                    OR (job.state IN ('failed','cancelled','completed')
                        AND job.stage = 'idle')
               )
               AND julianday(?3) >= julianday(job.updated_at)",
            params![input.job_id, input.owner_id, input.now],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Authorize archaeology cleanup: {error}"))?
        .ok_or_else(|| cas_error("cleanup", input.job_id))?;

    if input.mode == ArchaeologyCleanupMode::Apply {
        let changed = transaction
            .execute(
                "UPDATE archaeology_jobs SET updated_at = ?3
                 WHERE job_id = ?1 AND owner_id = ?2 AND repository_id = ?4
                   AND (
                        (state = 'running' AND stage = 'cleanup'
                         AND cancellation_requested = 0)
                        OR (state IN ('failed','cancelled','completed')
                            AND stage = 'idle')
                   )
                   AND julianday(?3) >= julianday(updated_at)",
                params![input.job_id, input.owner_id, input.now, repository_id],
            )
            .map_err(|error| format!("Claim archaeology cleanup: {error}"))?;
        require_cas(changed, "cleanup", input.job_id)?;
    }
    Ok((repository_id, owns_current_lease))
}

fn cleanup_candidates(
    transaction: &Transaction<'_>,
    repository_id: &str,
    owner_id: &str,
    retain_superseded: usize,
    owns_current_lease: bool,
) -> Result<(Vec<ArchaeologyCleanupGeneration>, bool), String> {
    let limit = i64::try_from(MAX_CLEANUP_GENERATIONS + 1)
        .map_err(|_| "Archaeology cleanup batch exceeds SQLite range")?;
    let retain = i64::try_from(retain_superseded)
        .map_err(|_| "Archaeology cleanup retention exceeds SQLite range")?;
    let mut statement = transaction
        .prepare(
            "WITH candidates AS (
                SELECT generation.generation_id, generation.status,
                       generation.created_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY generation.status
                           ORDER BY generation.created_at DESC,
                                    generation.generation_id DESC
                       ) AS status_rank
                FROM archaeology_generations AS generation
                WHERE generation.repository_id = ?1
                  AND generation.status IN ('staging','failed','cancelled','superseded')
                  AND generation.generation_id IS NOT (
                       SELECT ready_generation_id FROM archaeology_repositories
                       WHERE repository_id = ?1
                  )
                  AND NOT EXISTS (
                       SELECT 1 FROM archaeology_jobs AS active_job
                       WHERE active_job.generation_id = generation.generation_id
                         AND active_job.state IN ('pending','running','paused','cancelling')
                  )
            )
            SELECT candidate.generation_id, candidate.status
            FROM candidates AS candidate
            WHERE (?5 = 1 AND candidate.status = 'superseded' AND candidate.status_rank > ?3)
               OR (candidate.status IN ('staging','failed','cancelled') AND EXISTS (
                    SELECT 1 FROM archaeology_jobs AS owned_job
                    WHERE owned_job.generation_id = candidate.generation_id
                      AND owned_job.owner_id = ?2
                      AND owned_job.state IN ('failed','cancelled','completed')
               ))
            ORDER BY candidate.created_at, candidate.generation_id
            LIMIT ?4",
        )
        .map_err(|error| format!("Prepare archaeology cleanup plan: {error}"))?;
    let rows = statement
        .query_map(
            params![repository_id, owner_id, retain, limit, owns_current_lease],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| format!("Query archaeology cleanup plan: {error}"))?;
    let mut candidates = Vec::new();
    for row in rows {
        let (generation_id, status) =
            row.map_err(|error| format!("Read archaeology cleanup plan: {error}"))?;
        candidates.push(ArchaeologyCleanupGeneration {
            generation_id,
            status,
            search_index_rows: 0,
            synthesis_cache_rows: 0,
            synthesis_attempt_rows: 0,
            synthesis_response_bytes: 0,
        });
    }
    let truncated = candidates.len() > MAX_CLEANUP_GENERATIONS;
    candidates.truncate(MAX_CLEANUP_GENERATIONS);
    enrich_search_index_counts(transaction, &mut candidates)?;
    enrich_synthesis_cache_counts(transaction, &mut candidates)?;
    Ok((candidates, truncated))
}

fn enrich_synthesis_cache_counts(
    transaction: &Transaction<'_>,
    candidates: &mut [ArchaeologyCleanupGeneration],
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }
    let candidate_ids = cleanup_candidate_ids_json(candidates)?;
    let mut statement = transaction
        .prepare(
            "SELECT candidate.value,
                    (SELECT COUNT(*) FROM archaeology_synthesis_cache cache
                     WHERE cache.generation_id=candidate.value),
                    (SELECT COUNT(*) FROM archaeology_synthesis_attempts attempt
                     WHERE attempt.generation_id=candidate.value),
                    (SELECT COALESCE(SUM(LENGTH(CAST(COALESCE(cache.response_json,'') AS BLOB))),0)
                     FROM archaeology_synthesis_cache cache
                     WHERE cache.generation_id=candidate.value)
             FROM json_each(?1) candidate",
        )
        .map_err(|error| format!("Prepare archaeology synthesis cleanup plan: {error}"))?;
    let rows = statement
        .query_map([candidate_ids], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| format!("Query archaeology synthesis cleanup plan: {error}"))?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let (generation_id, cache_rows, attempt_rows, response_bytes) =
            row.map_err(|error| format!("Read archaeology synthesis cleanup plan: {error}"))?;
        counts.insert(
            generation_id,
            (
                u64::try_from(cache_rows)
                    .map_err(|_| "Negative archaeology synthesis cache row count")?,
                u64::try_from(attempt_rows)
                    .map_err(|_| "Negative archaeology synthesis attempt row count")?,
                u64::try_from(response_bytes)
                    .map_err(|_| "Negative archaeology synthesis response byte count")?,
            ),
        );
    }
    for candidate in candidates {
        let (cache_rows, attempt_rows, response_bytes) =
            counts.remove(&candidate.generation_id).unwrap_or_default();
        candidate.synthesis_cache_rows = cache_rows;
        candidate.synthesis_attempt_rows = attempt_rows;
        candidate.synthesis_response_bytes = response_bytes;
    }
    Ok(())
}

fn enrich_search_index_counts(
    transaction: &Transaction<'_>,
    candidates: &mut [ArchaeologyCleanupGeneration],
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }
    let candidate_ids = cleanup_candidate_ids_json(candidates)?;
    let mut statement = transaction
        .prepare(
            "SELECT generation_id, COUNT(*) FROM archaeology_rule_fts
             WHERE generation_id IN (SELECT value FROM json_each(?1))
             GROUP BY generation_id",
        )
        .map_err(|error| format!("Prepare archaeology search cleanup plan: {error}"))?;
    let rows = statement
        .query_map([candidate_ids], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| format!("Query archaeology search cleanup plan: {error}"))?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let (generation_id, count) =
            row.map_err(|error| format!("Read archaeology search cleanup plan: {error}"))?;
        counts.insert(
            generation_id,
            u64::try_from(count).map_err(|_| "Negative archaeology search row count")?,
        );
    }
    for candidate in candidates {
        candidate.search_index_rows = counts.remove(&candidate.generation_id).unwrap_or(0);
    }
    Ok(())
}

fn cleanup_candidate_ids_json(
    candidates: &[ArchaeologyCleanupGeneration],
) -> Result<String, String> {
    serde_json::to_string(
        &candidates
            .iter()
            .map(|candidate| candidate.generation_id.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| format!("Encode archaeology cleanup ownership: {error}"))
}

fn transition_state(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    from: &str,
    to: &str,
    cancellation_requested: bool,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let changed = connection
        .execute(
            "UPDATE archaeology_jobs SET state = ?4, cancellation_requested = ?5, updated_at = ?6
             WHERE job_id = ?1 AND owner_id = ?2 AND state = ?3
               AND julianday(?6) >= julianday(updated_at)",
            params![
                job_id,
                owner_id,
                from,
                to,
                i64::from(cancellation_requested),
                now
            ],
        )
        .map_err(|error| format!("Transition archaeology job: {error}"))?;
    require_cas(changed, to, job_id)?;
    load_job(connection, job_id)
}

fn finish_job(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    from: &str,
    to: &str,
    generation_status: Option<&str>,
    now: &str,
) -> Result<ArchaeologyJobStatus, String> {
    validate_actor(job_id, owner_id, now)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start archaeology completion transaction: {error}"))?;
    let changed = transaction
        .execute(
            "UPDATE archaeology_jobs
             SET state = ?4, stage = 'idle', finished_at = ?5, updated_at = ?5
             WHERE job_id = ?1 AND owner_id = ?2 AND state = ?3
               AND julianday(?5) >= julianday(updated_at)",
            params![job_id, owner_id, from, to, now],
        )
        .map_err(|error| format!("Finish archaeology job: {error}"))?;
    require_cas(changed, to, job_id)?;
    if let Some(status) = generation_status {
        update_staging_generation(&transaction, job_id, status)?;
    }
    let result = load_job(&transaction, job_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology completion: {error}"))?;
    Ok(result)
}

fn update_staging_generation(
    transaction: &Transaction<'_>,
    job_id: &str,
    status: &str,
) -> Result<(), String> {
    let changed = transaction
        .execute(
            "UPDATE archaeology_generations SET status = ?2
             WHERE generation_id = (
                SELECT generation_id FROM archaeology_jobs WHERE job_id = ?1
             ) AND status = 'staging'",
            params![job_id, status],
        )
        .map_err(|error| format!("Update owned archaeology generation: {error}"))?;
    if changed == 1 {
        return Ok(());
    }
    // Publication has already committed by cleanup. A later cleanup failure or
    // cancellation terminates the job but must never demote the ready data.
    let published_ready: bool = transaction
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM archaeology_jobs AS job
                JOIN archaeology_generations AS generation
                  ON generation.generation_id = job.generation_id
                JOIN archaeology_repositories AS repository
                  ON repository.repository_id = job.repository_id
                WHERE job.job_id = ?1 AND generation.status = 'ready'
                  AND repository.ready_generation_id = generation.generation_id
            )",
            [job_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Check published archaeology generation: {error}"))?;
    if published_ready && matches!(status, "failed" | "cancelled") {
        Ok(())
    } else {
        require_cas(changed, "generation update", job_id)
    }
}

fn validate_stage_progression(
    current: &ArchaeologyJobStage,
    next: &ArchaeologyJobStage,
) -> Result<(), String> {
    if matches!(current, ArchaeologyJobStage::Synthesize)
        && matches!(next, ArchaeologyJobStage::Validate)
    {
        return Err(
            "Archaeology synthesis must atomically validate and materialize its rule catalog"
                .to_string(),
        );
    }
    if matches!(current, ArchaeologyJobStage::Validate)
        && matches!(next, ArchaeologyJobStage::Publish)
    {
        return Err(
            "Archaeology validate must persist a deterministic publication receipt".to_string(),
        );
    }
    if matches!(current, ArchaeologyJobStage::Publish)
        && matches!(next, ArchaeologyJobStage::Cleanup)
    {
        return Err(
            "Archaeology publish must advance through atomic generation publication".to_string(),
        );
    }
    let current_index = stage_index(current).ok_or("Idle is not an active archaeology stage")?;
    let next_index = stage_index(next).ok_or("Idle is not an active archaeology stage")?;
    if next_index == current_index || next_index == current_index + 1 {
        Ok(())
    } else {
        Err("Archaeology stages must stay current or advance exactly once".to_string())
    }
}

fn stage_index(stage: &ArchaeologyJobStage) -> Option<usize> {
    match stage {
        ArchaeologyJobStage::Inventory => Some(0),
        ArchaeologyJobStage::Parse => Some(1),
        ArchaeologyJobStage::Link => Some(2),
        ArchaeologyJobStage::Derive => Some(3),
        ArchaeologyJobStage::Synthesize => Some(4),
        ArchaeologyJobStage::Validate => Some(5),
        ArchaeologyJobStage::Publish => Some(6),
        ArchaeologyJobStage::Cleanup => Some(7),
        ArchaeologyJobStage::Idle => None,
    }
}

fn stage_name(stage: &ArchaeologyJobStage) -> &'static str {
    match stage {
        ArchaeologyJobStage::Inventory => "inventory",
        ArchaeologyJobStage::Parse => "parse",
        ArchaeologyJobStage::Link => "link",
        ArchaeologyJobStage::Derive => "derive",
        ArchaeologyJobStage::Synthesize => "synthesize",
        ArchaeologyJobStage::Validate => "validate",
        ArchaeologyJobStage::Publish => "publish",
        ArchaeologyJobStage::Cleanup => "cleanup",
        ArchaeologyJobStage::Idle => "idle",
    }
}

fn source_classification_name(
    classification: &ArchaeologySourceClassification,
) -> Result<&'static str, String> {
    match classification {
        ArchaeologySourceClassification::Source => Ok("source"),
        ArchaeologySourceClassification::Generated => Ok("generated"),
        ArchaeologySourceClassification::Vendor => Ok("vendor"),
        ArchaeologySourceClassification::Protected => Ok("protected"),
        ArchaeologySourceClassification::Opaque => Ok("opaque"),
        ArchaeologySourceClassification::Unavailable => {
            Err("Archaeology inventory classification is unavailable".into())
        }
    }
}

fn parse_stage(value: &str) -> Result<ArchaeologyJobStage, String> {
    parse_enum(value, "stage")
}

fn parse_state(value: &str) -> Result<ArchaeologyJobState, String> {
    parse_enum(value, "state")
}

fn parse_enum<T: for<'de> serde::Deserialize<'de>>(value: &str, label: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|_| format!("Stored archaeology {label} is unsupported"))
}

fn validate_actor(job_id: &str, owner_id: &str, now: &str) -> Result<(), String> {
    validate_id("job", job_id)?;
    validate_id("owner", owner_id)?;
    validate_timestamp(now).map(|_| ())
}

fn validate_owned_generation(
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
    identity: &ArchaeologyGenerationIdentity<'_>,
    now: &str,
) -> Result<(), String> {
    validate_actor(job_id, owner_id, now)?;
    validate_id("repository", repository_id)?;
    validate_id("generation", generation_id)?;
    identity.validate()
}

fn validate_timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| "Archaeology timestamps must be RFC 3339".to_string())
}

fn validate_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_ID_BYTES || value.contains('\0') {
        Err(format!(
            "Archaeology {label} identity must contain 1..={MAX_ID_BYTES} safe bytes"
        ))
    } else {
        Ok(())
    }
}

fn validate_checkpoint(checkpoint: &ArchaeologyJobCheckpoint) -> Result<(), String> {
    for (label, value) in [
        ("checkpoint cursor", checkpoint.cursor_identity.as_deref()),
        (
            "checkpoint source unit",
            checkpoint.source_unit_id.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_persisted_token(label, value, MAX_ID_BYTES)?;
        }
    }
    if checkpoint.counters.len() > MAX_CHECKPOINT_COUNTERS {
        return Err(format!(
            "Archaeology checkpoint has more than {MAX_CHECKPOINT_COUNTERS} counters"
        ));
    }
    for key in checkpoint.counters.keys() {
        validate_persisted_token("checkpoint counter", key, 64)?;
    }
    Ok(())
}

fn error_code_name(code: ArchaeologyJobErrorCode) -> &'static str {
    match code {
        ArchaeologyJobErrorCode::InventoryFailed => "inventory_failed",
        ArchaeologyJobErrorCode::ParserFailed => "parser_failed",
        ArchaeologyJobErrorCode::LinkFailed => "link_failed",
        ArchaeologyJobErrorCode::DerivationFailed => "derivation_failed",
        ArchaeologyJobErrorCode::SynthesisFailed => "synthesis_failed",
        ArchaeologyJobErrorCode::ValidationFailed => "validation_failed",
        ArchaeologyJobErrorCode::PublicationFailed => "publication_failed",
        ArchaeologyJobErrorCode::CleanupFailed => "cleanup_failed",
        ArchaeologyJobErrorCode::OwnershipLost => "ownership_lost",
        ArchaeologyJobErrorCode::Internal => "internal",
    }
}

fn validate_persisted_token(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_bytes
        || looks_like_secret(value)
        || contains_sensitive_path(value)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'@')
        })
    {
        Err(format!(
            "Archaeology {label} must be an opaque safe token of 1..={max_bytes} bytes"
        ))
    } else {
        Ok(())
    }
}

fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "Archaeology progress exceeds SQLite range".to_string())
}

fn require_cas(changed: usize, action: &str, job_id: &str) -> Result<(), String> {
    if changed == 1 {
        Ok(())
    } else {
        Err(cas_error(action, job_id))
    }
}

fn cas_error(action: &str, job_id: &str) -> String {
    format!(
        "Archaeology job {action} rejected: owner, state, stage, or progress changed for {job_id}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::business_rule_archaeology::adapter::ArchaeologyAdapterRegionKind;
    use crate::commands::business_rule_archaeology::contracts::{
        ArchaeologyConfidence, ArchaeologyRuleKind, ArchaeologySourceUnitIdentity,
    };
    use crate::commands::business_rule_archaeology::invalidation::ArchaeologyGenerationInputKind;
    use crate::commands::business_rule_archaeology::lifecycle::{
        ArchaeologyLifecycleAction, ArchaeologyReviewerKind, ArchaeologyReviewerProvenance,
    };
    use crate::commands::business_rule_archaeology::lifecycle_store::{
        append_lifecycle_event, ensure_candidate_lifecycle, ArchaeologyLifecycleAppend,
    };
    use crate::commands::business_rule_archaeology::synthesis::{
        ArchaeologySynthesisClause, ArchaeologySynthesisSegment,
    };
    use crate::db::archaeology_schema::run_migration;

    const REPO: &str = "repo:jobs";
    const READY: &str = "generation:ready";
    const OWNER: &str = "owner:one";
    const PARSER_MANIFEST: &str = "parser-manifest:v1:parser:v1@1,unavailable@unavailable";
    const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const T0: &str = "2026-01-01T00:00:00.000Z";
    const T1: &str = "2026-01-01T00:01:00.000Z";

    #[test]
    fn valid_progress_pause_resume_and_completion_are_explicit() {
        let connection = fixture();
        start(&connection, "job:one", "generation:staging", OWNER);
        let status = checkpoint(
            &connection,
            "job:one",
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            2,
        );
        assert_eq!(status.stage, ArchaeologyJobStage::Parse);
        assert_eq!(status.completed_units, 2);
        assert_eq!(
            pause_job(&connection, "job:one", OWNER, T1).unwrap().state,
            ArchaeologyJobState::Paused
        );
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Link,
            "checkpoint:blocked",
            &ArchaeologyJobCheckpoint::default(),
            3,
            Some(10),
            T1,
        )
        .is_err());
        assert_eq!(
            resume_job(&connection, "job:one", OWNER, T1).unwrap().state,
            ArchaeologyJobState::Running
        );
        for (current, next, completed) in [
            (ArchaeologyJobStage::Parse, ArchaeologyJobStage::Link, 3),
            (ArchaeologyJobStage::Link, ArchaeologyJobStage::Derive, 4),
            (
                ArchaeologyJobStage::Derive,
                ArchaeologyJobStage::Synthesize,
                5,
            ),
            (
                ArchaeologyJobStage::Synthesize,
                ArchaeologyJobStage::Validate,
                6,
            ),
        ] {
            checkpoint(&connection, "job:one", current, next, completed);
        }
        seed_publishable_generation(&connection, "generation:staging");
        assert_eq!(
            validate_generation_for_publication(
                &connection,
                publication("job:one", "generation:staging"),
            )
            .unwrap()
            .stage,
            ArchaeologyJobStage::Publish
        );
        assert!(complete_job(&connection, "job:one", OWNER, T1).is_err());
        assert_eq!(
            publish(&connection, "job:one", "generation:staging").stage,
            ArchaeologyJobStage::Cleanup
        );
        assert!(request_cancel(&connection, "job:one", "owner:other", T1).is_err());
        assert_eq!(
            complete_job(&connection, "job:one", OWNER, T1)
                .unwrap()
                .state,
            ArchaeologyJobState::Completed
        );
        assert_eq!(
            generation_status(&connection, "generation:staging"),
            "ready"
        );
    }

    #[test]
    fn invalid_transitions_and_two_owner_contention_fail_closed() {
        let connection = fixture();
        start(&connection, "job:one", "generation:staging", OWNER);
        assert!(start_job(
            &connection,
            new_job("job:two", "generation:other", "owner:two")
        )
        .is_err());
        assert!(checkpoint_job(
            &connection,
            "job:one",
            "owner:two",
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            "checkpoint:wrong-owner",
            &ArchaeologyJobCheckpoint::default(),
            1,
            Some(10),
            T1,
        )
        .is_err());
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Derive,
            "checkpoint:skip",
            &ArchaeologyJobCheckpoint::default(),
            1,
            Some(10),
            T1,
        )
        .is_err());
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Inventory,
            "checkpoint:over-total",
            &ArchaeologyJobCheckpoint::default(),
            11,
            None,
            T1,
        )
        .is_err());
        checkpoint(
            &connection,
            "job:one",
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            2,
        );
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Parse,
            "checkpoint:equal-progress-conflict",
            &ArchaeologyJobCheckpoint::default(),
            2,
            None,
            T1,
        )
        .is_err());
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Parse,
            "checkpoint:regress",
            &ArchaeologyJobCheckpoint::default(),
            1,
            Some(10),
            T1,
        )
        .is_err());
    }

    #[test]
    fn synthesis_catalog_materializes_zero_model_rules_with_exact_fts_parity() {
        let connection = synthesis_catalog_fixture("zero-model");
        assert!(checkpoint_job(
            &connection,
            "job:zero-model",
            OWNER,
            ArchaeologyJobStage::Synthesize,
            ArchaeologyJobStage::Validate,
            "checkpoint:bypass",
            &ArchaeologyJobCheckpoint::default(),
            5,
            Some(10),
            T1,
        )
        .unwrap_err()
        .contains("atomically validate and materialize"));
        let cancellation = StructuralGraphCancellation::default();
        let status = finalize_synthesis_catalog(
            &connection,
            synthesis_catalog_input(
                "job:zero-model",
                "generation:zero-model",
                OWNER,
                &cancellation,
            ),
        )
        .unwrap();
        assert_eq!(status.stage, ArchaeologyJobStage::Validate);
        let rows: (i64, i64, i64) = connection
            .query_row(
                "SELECT
                  (SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1),
                  (SELECT COUNT(*) FROM archaeology_rule_search_manifest WHERE generation_id=?1),
                  (SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?1)",
                ["generation:zero-model"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(rows, (1, 1, 1));
        let search: (String, String, String) = connection
            .query_row(
                "SELECT title,clause_text,domain_text
                 FROM archaeology_rule_search_manifest WHERE generation_id=?1",
                ["generation:zero-model"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            search,
            (
                "Positive amount".into(),
                "Amount must be positive.".into(),
                "Other".into()
            )
        );
    }

    #[test]
    fn synthesis_catalog_accepts_mixed_deterministic_and_valid_model_rules() {
        let connection = synthesis_catalog_fixture("mixed");
        seed_model_rule(&connection, "generation:mixed");
        let cancellation = StructuralGraphCancellation::default();
        finalize_synthesis_catalog(
            &connection,
            synthesis_catalog_input("job:mixed", "generation:mixed", OWNER, &cancellation),
        )
        .unwrap();
        let rows = connection
            .prepare(
                "SELECT rule_id,title,clause_text,domain_text
                 FROM archaeology_rule_search_manifest WHERE generation_id=?1 ORDER BY rule_id",
            )
            .unwrap()
            .query_map(["generation:mixed"], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.0 == "rule:model"));
        let fts_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?1",
                ["generation:mixed"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_rows, 2);
    }

    #[test]
    fn synthesis_catalog_rejects_self_referential_alias_relations() {
        let connection = synthesis_catalog_fixture("alias-incompatible");
        let generation = "generation:alias-incompatible";
        seed_model_rule(&connection, generation);
        connection
            .execute(
                "DELETE FROM archaeology_rule_domains
                 WHERE generation_id=?1 AND rule_id='rule:model'",
                [generation],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_relations
                 (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
                 VALUES (?1,'relation:self-alias','rule:model','rule:model',
                         'aliases','deterministic')",
                [generation],
            )
            .unwrap();

        let cancellation = StructuralGraphCancellation::default();
        let error = finalize_synthesis_catalog(
            &connection,
            synthesis_catalog_input("job:alias-incompatible", generation, OWNER, &cancellation),
        )
        .unwrap_err();
        assert!(error.contains("self-referential alias"), "{error}");
        assert_eq!(
            load_job(&connection, "job:alias-incompatible")
                .unwrap()
                .stage,
            ArchaeologyJobStage::Synthesize
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_search_manifest",
                "generation_id='generation:alias-incompatible'"
            ),
            0
        );
    }

    #[test]
    fn synthesis_catalog_rejects_model_evidence_drift() {
        let connection = synthesis_catalog_fixture("model-drift");
        seed_model_rule(&connection, "generation:model-drift");
        connection
            .execute_batch(
                "INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES ('generation:model-drift','fact:unrelated','mutation','Unrelated change',
                         'parser:v1','extracted','high',
                         '[{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"}]');
                 INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation:model-drift','fact','fact:unrelated','span',
                         'span:generation:model-drift','supporting'),
                        ('generation:model-drift','rule_clause','clause:model','fact',
                         'fact:unrelated','supporting');",
            )
            .unwrap();
        let cancellation = StructuralGraphCancellation::default();
        let error = finalize_synthesis_catalog(
            &connection,
            synthesis_catalog_input(
                "job:model-drift",
                "generation:model-drift",
                OWNER,
                &cancellation,
            ),
        )
        .unwrap_err();
        assert!(error.contains("evidence does not match"), "{error}");
        assert_eq!(
            load_job(&connection, "job:model-drift").unwrap().stage,
            ArchaeologyJobStage::Synthesize
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_search_manifest",
                "generation_id='generation:model-drift'"
            ),
            0
        );
    }

    #[test]
    fn synthesis_catalog_rolls_back_invalid_model_and_catalog_rows() {
        let mutations = [
            (
                "model-without-identity",
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at)
                 SELECT generation_id,'rule:model-invalid',repository_id,revision_sha,kind,
                   'Model rule','candidate','model_synthesized','high',parser_identity,
                   algorithm_identity,coverage_json,created_at
                 FROM archaeology_rules WHERE generation_id=?1 LIMIT 1",
            ),
            (
                "missing-domain",
                "DELETE FROM archaeology_rule_domains WHERE generation_id=?1",
            ),
            (
                "duplicate-clause",
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 SELECT generation_id,rule_id,'clause:duplicate',1,clause_text,trust,confidence,
                        caveats_json FROM archaeology_rule_clauses WHERE generation_id=?1 LIMIT 1",
            ),
            (
                "unsafe-title",
                "UPDATE archaeology_rules SET title='.env' WHERE generation_id=?1",
            ),
            (
                "cross-revision",
                "UPDATE archaeology_rules
                 SET revision_sha='cccccccccccccccccccccccccccccccccccccccc'
                 WHERE generation_id=?1",
            ),
            (
                "uncited-clause",
                "DELETE FROM archaeology_evidence_links
                 WHERE generation_id=?1 AND owner_kind='rule_clause' AND evidence_kind='fact'",
            ),
        ];
        for (name, mutation) in mutations {
            let connection = synthesis_catalog_fixture(name);
            let generation = format!("generation:{name}");
            connection.execute(mutation, [&generation]).unwrap();
            let cancellation = StructuralGraphCancellation::default();
            assert!(finalize_synthesis_catalog(
                &connection,
                synthesis_catalog_input(&format!("job:{name}"), &generation, OWNER, &cancellation,),
            )
            .is_err());
            let state: (String, i64, i64) = connection
                .query_row(
                    "SELECT stage,
                      (SELECT COUNT(*) FROM archaeology_rule_search_manifest
                       WHERE generation_id=?2),
                      (SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?2)
                     FROM archaeology_jobs WHERE job_id=?1",
                    params![format!("job:{name}"), generation],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(state, ("synthesize".into(), 0, 0), "{name}");
        }
    }

    #[test]
    fn synthesis_catalog_rejects_stale_search_and_retries_idempotently() {
        let connection = synthesis_catalog_fixture("retry");
        let cancellation = StructuralGraphCancellation::default();
        let input =
            || synthesis_catalog_input("job:retry", "generation:retry", OWNER, &cancellation);
        let first = finalize_synthesis_catalog(&connection, input()).unwrap();
        let second = finalize_synthesis_catalog(&connection, input()).unwrap();
        assert_eq!(first.checkpoint_identity, second.checkpoint_identity);
        assert_eq!(second.stage, ArchaeologyJobStage::Validate);
        connection
            .execute(
                "UPDATE archaeology_rule_fts SET title='stale'
                 WHERE generation_id='generation:retry'",
                [],
            )
            .unwrap();
        assert!(finalize_synthesis_catalog(&connection, input())
            .unwrap_err()
            .contains("FTS linkage"));
    }

    #[test]
    fn synthesis_catalog_requires_owner_and_observes_cancellation_without_writes() {
        for (name, owner, cancelled) in [
            ("wrong-owner", "owner:other", false),
            ("cancelled", OWNER, true),
        ] {
            let connection = synthesis_catalog_fixture(name);
            let generation = format!("generation:{name}");
            let cancellation = StructuralGraphCancellation::default();
            if cancelled {
                cancellation.cancel();
            }
            assert!(finalize_synthesis_catalog(
                &connection,
                synthesis_catalog_input(&format!("job:{name}"), &generation, owner, &cancellation,),
            )
            .is_err());
            let rows: (String, i64, i64) = connection
                .query_row(
                    "SELECT stage,
                      (SELECT COUNT(*) FROM archaeology_rule_search_manifest
                       WHERE generation_id=?2),
                      (SELECT COUNT(*) FROM archaeology_rule_fts WHERE generation_id=?2)
                     FROM archaeology_jobs WHERE job_id=?1",
                    params![format!("job:{name}"), generation],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(rows, ("synthesize".into(), 0, 0));
        }
    }

    #[test]
    fn link_stage_persists_unique_relationships_and_retry_is_idempotent() {
        let connection = link_fixture("job:link", "generation:link", false);
        let cancellation = StructuralGraphCancellation::default();
        let linked = link_generation(
            &connection,
            link_input(
                "job:link",
                "generation:link",
                OWNER,
                &cancellation,
                ArchaeologyLinkLimits::default(),
            ),
        )
        .unwrap();
        assert_eq!(linked.stage, ArchaeologyJobStage::Derive);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_fact_edges",
                "generation_id='generation:link' AND kind='calls'"
            ),
            1
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_evidence_links",
                "generation_id='generation:link' AND owner_kind='fact_edge'"
            ),
            2
        );
        let lineage:String=connection.query_row("SELECT include_lineage_json FROM archaeology_source_units WHERE generation_id='generation:link' AND source_unit_id='unit:main'",[],|row|row.get(0)).unwrap();
        assert!(lineage.contains("unit:copy"));
        let before = count_where(
            &connection,
            "archaeology_fact_edges",
            "generation_id='generation:link'",
        );
        assert_eq!(
            link_generation(
                &connection,
                link_input(
                    "job:link",
                    "generation:link",
                    OWNER,
                    &cancellation,
                    ArchaeologyLinkLimits::default()
                )
            )
            .unwrap()
            .stage,
            ArchaeologyJobStage::Derive
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_fact_edges",
                "generation_id='generation:link'"
            ),
            before
        );
    }

    #[test]
    fn link_stage_retains_ambiguous_reference_as_cited_unresolved_fact() {
        let connection = link_fixture("job:ambiguous", "generation:ambiguous", true);
        let cancellation = StructuralGraphCancellation::default();
        link_generation(
            &connection,
            link_input(
                "job:ambiguous",
                "generation:ambiguous",
                OWNER,
                &cancellation,
                ArchaeologyLinkLimits::default(),
            ),
        )
        .unwrap();
        assert_eq!(
            count_where(
                &connection,
                "archaeology_facts",
                "generation_id='generation:ambiguous' AND kind='unresolved'"
            ),
            1
        );
        assert_eq!(count_where(&connection,"archaeology_fact_edges","generation_id='generation:ambiguous' AND kind='unresolved' AND unresolved_reason='reference target is ambiguous'"),1);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_evidence_links",
                "generation_id='generation:ambiguous' AND owner_kind='fact_edge'"
            ),
            1
        );
    }

    #[test]
    fn link_stage_scope_bounds_and_cancellation_roll_back() {
        let connection = link_fixture("job:rollback", "generation:rollback", false);
        let cancellation = StructuralGraphCancellation::default();
        assert!(link_generation(
            &connection,
            link_input(
                "job:rollback",
                "generation:rollback",
                "owner:other",
                &cancellation,
                ArchaeologyLinkLimits::default()
            )
        )
        .is_err());
        let limits = ArchaeologyLinkLimits {
            max_facts: 1,
            ..Default::default()
        };
        assert!(link_generation(
            &connection,
            link_input(
                "job:rollback",
                "generation:rollback",
                OWNER,
                &cancellation,
                limits
            )
        )
        .is_err());
        assert_eq!(
            load_job(&connection, "job:rollback").unwrap().stage,
            ArchaeologyJobStage::Link
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_fact_edges",
                "generation_id='generation:rollback'"
            ),
            0
        );
        let byte_limits = ArchaeologyLinkLimits {
            max_input_bytes: 1,
            ..Default::default()
        };
        assert!(link_generation(
            &connection,
            link_input(
                "job:rollback",
                "generation:rollback",
                OWNER,
                &cancellation,
                byte_limits
            )
        )
        .is_err());
        let delayed = StructuralGraphCancellation::default();
        delayed.cancel_after_checks(3);
        assert!(link_generation(
            &connection,
            link_input(
                "job:rollback",
                "generation:rollback",
                OWNER,
                &delayed,
                ArchaeologyLinkLimits::default()
            )
        )
        .is_err());
        assert_eq!(
            load_job(&connection, "job:rollback").unwrap().stage,
            ArchaeologyJobStage::Link
        );
        let wrong_stage = fixture();
        start(
            &wrong_stage,
            "job:parse-only",
            "generation:parse-only",
            OWNER,
        );
        checkpoint(
            &wrong_stage,
            "job:parse-only",
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            1,
        );
        assert!(link_generation(
            &wrong_stage,
            link_input(
                "job:parse-only",
                "generation:parse-only",
                OWNER,
                &cancellation,
                ArchaeologyLinkLimits::default()
            )
        )
        .is_err());

        let progress = link_fixture("job:progress", "generation:progress", false);
        progress.execute_batch("WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<5000)
            INSERT INTO archaeology_source_spans
            (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,start_line,start_column,end_line,end_column)
            SELECT 'generation:progress','span:bulk:'||value,'unit:main','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',value,value+1,value,1,value,2 FROM n;").unwrap();
        let sqlite_cancel = StructuralGraphCancellation::default();
        sqlite_cancel.cancel_after_checks(2);
        assert!(link_generation(
            &progress,
            link_input(
                "job:progress",
                "generation:progress",
                OWNER,
                &sqlite_cancel,
                ArchaeologyLinkLimits::default()
            )
        )
        .is_err());
        assert_eq!(
            load_job(&progress, "job:progress").unwrap().stage,
            ArchaeologyJobStage::Link
        );
    }

    #[test]
    fn derive_stage_persists_exact_candidate_evidence_and_retry_is_idempotent() {
        let connection = derive_fixture("job:derive", "generation:derive");
        let cancellation = StructuralGraphCancellation::default();
        let first = derive_template_candidates(
            &connection,
            derive_input(
                "job:derive",
                "generation:derive",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default(),
            ),
        )
        .unwrap();
        assert_eq!(first.stage, ArchaeologyJobStage::Synthesize);
        assert_eq!(count_where(&connection,"archaeology_rules","generation_id='generation:derive' AND lifecycle='candidate' AND trust='deterministic'"),2);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_domains",
                "generation_id='generation:derive' AND domain_id='domain:other'"
            ),
            1
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_relations",
                "generation_id='generation:derive' AND kind='aliases'"
            ),
            1
        );
        assert_eq!(count_where(&connection,"archaeology_evidence_links","generation_id='generation:derive' AND owner_kind='rule_relation' AND evidence_kind='rule'"),2);
        let cluster_counts: (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT json_extract(checkpoint_json,'$.counters.cluster_primary_rules'),
                    json_extract(checkpoint_json,'$.counters.cluster_alias_rules'),
                    json_extract(checkpoint_json,'$.counters.cluster_conflict_pairs'),
                    json_extract(checkpoint_json,'$.counters.domain_other_rules')
             FROM archaeology_jobs WHERE job_id='job:derive'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(cluster_counts, (1, 1, 0, 1));
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rules",
                "generation_id='generation:derive' AND rule_id='rule:stale'"
            ),
            0
        );
        assert_eq!(connection.query_row("SELECT title FROM archaeology_rules WHERE generation_id='generation:derive' AND rule_id='rule:accepted'",[],|row|row.get::<_,String>(0)).unwrap(),"Human-approved sentinel");
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_relations",
                "generation_id='generation:derive' AND relation_id='relation:stale'"
            ),
            0
        );
        assert_eq!(count_where(&connection,"archaeology_evidence_links","generation_id='generation:derive' AND owner_kind='rule_relation' AND owner_id='relation:stale'"),0);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:derive' AND event_id='review:accepted'"
            ),
            1
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_clauses",
                "generation_id='generation:derive' AND rule_id='rule:accepted'"
            ),
            1
        );
        let uncited:i64=connection.query_row(
            "SELECT COUNT(*) FROM archaeology_rule_clauses clause
             JOIN archaeology_rules rule ON rule.generation_id=clause.generation_id AND rule.rule_id=clause.rule_id
             WHERE clause.generation_id='generation:derive' AND rule.lifecycle='candidate'
               AND (NOT EXISTS (SELECT 1 FROM archaeology_evidence_links evidence
                    WHERE evidence.generation_id=clause.generation_id AND evidence.owner_kind='rule_clause'
                      AND evidence.owner_id=clause.clause_id AND evidence.evidence_kind='fact')
                 OR NOT EXISTS (SELECT 1 FROM archaeology_evidence_links evidence
                    WHERE evidence.generation_id=clause.generation_id AND evidence.owner_kind='rule_clause'
                      AND evidence.owner_id=clause.clause_id AND evidence.evidence_kind='span'))",
            [],|row|row.get(0)).unwrap();
        assert_eq!(uncited, 0);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE '%packet%'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        let before = (
            count_where(
                &connection,
                "archaeology_rules",
                "generation_id='generation:derive'",
            ),
            count_where(
                &connection,
                "archaeology_rule_clauses",
                "generation_id='generation:derive'",
            ),
            count_where(
                &connection,
                "archaeology_evidence_links",
                "generation_id='generation:derive' AND owner_kind='rule_clause'",
            ),
        );
        let retry = derive_template_candidates(
            &connection,
            derive_input(
                "job:derive",
                "generation:derive",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default(),
            ),
        )
        .unwrap();
        assert_eq!(retry, first);
        assert_eq!(
            before,
            (
                count_where(
                    &connection,
                    "archaeology_rules",
                    "generation_id='generation:derive'"
                ),
                count_where(
                    &connection,
                    "archaeology_rule_clauses",
                    "generation_id='generation:derive'"
                ),
                count_where(
                    &connection,
                    "archaeology_evidence_links",
                    "generation_id='generation:derive' AND owner_kind='rule_clause'"
                )
            )
        );
    }

    #[test]
    fn independent_clean_derivations_publish_byte_identical_bounded_rows() {
        let first = derive_fixture("job:derive", "generation:derive");
        let second = derive_fixture("job:derive", "generation:derive");
        let cancellation = StructuralGraphCancellation::default();
        let limits = ArchaeologyDeterministicLimits::default();
        for connection in [&first, &second] {
            derive_template_candidates(
                connection,
                derive_input(
                    "job:derive",
                    "generation:derive",
                    OWNER,
                    &cancellation,
                    limits,
                ),
            )
            .expect("clean derivation");
        }

        assert_eq!(
            derived_catalog_snapshot(&first, "generation:derive"),
            derived_catalog_snapshot(&second, "generation:derive")
        );
        let counts = derived_catalog_counts(&first);
        assert!(counts.0 <= limits.max_packets as i64);
        assert!(counts.1 <= (limits.max_packets * limits.max_clauses_per_rule) as i64);
        assert!(counts.2 <= limits.max_cluster_relations as i64);
        assert!(counts.3 <= limits.max_cluster_domains as i64);
        assert_eq!(
            first
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name LIKE '%packet%'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "evidence packets must remain bounded transient values"
        );
    }

    #[test]
    fn no_op_incremental_refresh_returns_the_ready_generation_identity() {
        let cancellation = StructuralGraphCancellation::default();
        let incremental = derive_fixture("job:prior", "generation:prior");
        remove_derive_retry_sentinel(&incremental, "generation:prior");
        make_derive_sources_publishable(&incremental, "generation:prior");
        derive_template_candidates(
            &incremental,
            derive_input(
                "job:prior",
                "generation:prior",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default(),
            ),
        )
        .expect("prior derivation");
        persist_generation_invalidation_metadata(
            &incremental,
            REPO,
            "generation:prior",
            &invalidation_inputs(REVISION),
            &cancellation,
            ArchaeologyInvalidationLimits::default(),
        )
        .expect("prior invalidation metadata");
        incremental
            .execute_batch(
                "DELETE FROM archaeology_jobs WHERE job_id='job:prior';
                 UPDATE archaeology_generations SET status='superseded'
                   WHERE generation_id='generation:ready';
                 UPDATE archaeology_generations SET status='ready',published_at='2026-01-01T00:00:30.000Z'
                   WHERE generation_id='generation:prior';
                 UPDATE archaeology_repositories SET ready_generation_id='generation:prior'
                   WHERE repository_id='repo:jobs';",
            )
            .expect("install prior ready generation");

        start_at(
            &incremental,
            "job:current",
            "generation:current",
            OWNER,
            REVISION,
        );
        let units = [incremental_inventory_unit(
            &opaque_test_id("archaeology-source-unit", "source-current"),
            &opaque_test_id("archaeology-path", "source"),
            "src/rules.cbl",
            'd',
            ArchaeologySourceClassification::Source,
            opaque_test_id("archaeology-change", "source"),
            REVISION,
        )];
        let inputs = invalidation_inputs(REVISION);
        let outcome = prepare_incremental_refresh(
            &incremental,
            ArchaeologyInventoryRefreshStage {
                job_id: "job:current",
                repository_id: REPO,
                generation_id: "generation:current",
                owner_id: OWNER,
                identity: generation_identity_at("generation:current", REVISION),
                units: &units,
                generation_inputs: &inputs,
                cancellation: &cancellation,
                limits: ArchaeologyInvalidationLimits::default(),
                now: T1,
            },
        )
        .expect("prepare no-op incremental generation");
        assert_eq!(outcome.mode, ArchaeologyInputInvalidationMode::NoOp);
        assert_eq!(outcome.next_stage, ArchaeologyJobStage::Idle);
        assert_eq!(outcome.effective_generation_id, "generation:prior");
        assert!(outcome.reused_ready_generation);
        assert_eq!(
            count_where(
                &incremental,
                "archaeology_generations",
                "generation_id='generation:current'"
            ),
            0
        );
    }

    #[test]
    fn derive_stage_is_owner_scoped_and_every_write_rolls_back() {
        let connection = derive_fixture("job:derive", "generation:derive");
        let cancellation = StructuralGraphCancellation::default();
        assert!(derive_template_candidates(
            &connection,
            derive_input(
                "job:derive",
                "generation:derive",
                "owner:other",
                &cancellation,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .is_err());
        let mut wrong_identity = derive_input(
            "job:derive",
            "generation:derive",
            OWNER,
            &cancellation,
            ArchaeologyDeterministicLimits::default(),
        );
        wrong_identity.identity.parser = "parser-manifest:v1:parser:other@1";
        assert!(derive_template_candidates(&connection, wrong_identity).is_err());
        connection
            .execute_batch(
                "CREATE TEMP TRIGGER reject_deterministic_rule
             BEFORE INSERT ON archaeology_rules
             WHEN NEW.generation_id='generation:derive'
             BEGIN SELECT RAISE(ABORT,'fixture derive rollback'); END;",
            )
            .unwrap();
        assert!(derive_template_candidates(
            &connection,
            derive_input(
                "job:derive",
                "generation:derive",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .unwrap_err()
        .contains("fixture derive rollback"));
        assert_eq!(
            load_job(&connection, "job:derive").unwrap().stage,
            ArchaeologyJobStage::Derive
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rules",
                "generation_id='generation:derive' AND rule_id='rule:stale'"
            ),
            1
        );
        assert_eq!(count_where(&connection,"archaeology_evidence_links","generation_id='generation:derive' AND owner_kind='rule_clause' AND owner_id='clause:stale'"),2);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_relations",
                "generation_id='generation:derive' AND relation_id='relation:stale'"
            ),
            1
        );
        assert_eq!(count_where(&connection,"archaeology_evidence_links","generation_id='generation:derive' AND owner_kind='rule_relation' AND owner_id='relation:stale'"),1);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rules",
                "generation_id='generation:derive' AND rule_id='rule:accepted'"
            ),
            1
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:derive'"
            ),
            1
        );
    }

    #[test]
    fn derive_stage_bounds_utf8_privacy_and_sql_cancellation_fail_closed() {
        let bounded = derive_fixture("job:bounded", "generation:bounded");
        let cancellation = StructuralGraphCancellation::default();
        let limits = ArchaeologyDeterministicLimits {
            max_facts: 1,
            ..Default::default()
        };
        assert!(derive_template_candidates(
            &bounded,
            derive_input(
                "job:bounded",
                "generation:bounded",
                OWNER,
                &cancellation,
                limits
            )
        )
        .is_err());
        assert_eq!(
            load_job(&bounded, "job:bounded").unwrap().stage,
            ArchaeologyJobStage::Derive
        );

        let secret = derive_fixture("job:secret", "generation:secret");
        secret.execute("UPDATE archaeology_facts SET label='password=correct-horse-battery-staple' WHERE generation_id=?1 AND fact_id='fact:predicate'",["generation:secret"]).unwrap();
        assert!(derive_template_candidates(
            &secret,
            derive_input(
                "job:secret",
                "generation:secret",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .unwrap_err()
        .contains("privacy"));

        let tampered = derive_fixture("job:semantic-tamper", "generation:semantic-tamper");
        tampered
            .execute(
                "UPDATE archaeology_facts SET attributes_json='[]'
             WHERE generation_id=?1 AND fact_id='fact:predicate'",
                ["generation:semantic-tamper"],
            )
            .unwrap();
        assert!(derive_template_candidates(
            &tampered,
            derive_input(
                "job:semantic-tamper",
                "generation:semantic-tamper",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .is_err());
        assert_eq!(
            load_job(&tampered, "job:semantic-tamper").unwrap().stage,
            ArchaeologyJobStage::Derive
        );

        let invalid = derive_fixture("job:utf8", "generation:utf8");
        invalid.execute("UPDATE archaeology_facts SET label=CAST(X'80' AS TEXT) WHERE generation_id=?1 AND fact_id='fact:predicate'",["generation:utf8"]).unwrap();
        assert!(derive_template_candidates(
            &invalid,
            derive_input(
                "job:utf8",
                "generation:utf8",
                OWNER,
                &cancellation,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .unwrap_err()
        .contains("UTF-8"));
        assert_eq!(
            load_job(&invalid, "job:utf8").unwrap().stage,
            ArchaeologyJobStage::Derive
        );

        let progress = derive_fixture("job:progress-derive", "generation:progress-derive");
        progress.execute_batch(
            "WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<5000)
             INSERT INTO archaeology_source_spans
              (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,start_line,start_column,end_line,end_column)
             SELECT 'generation:progress-derive','span:bulk:'||value,'unit:derive','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',value,value+1,value,1,value,2 FROM n;
             WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<5000)
             INSERT INTO archaeology_facts
              (generation_id,fact_id,kind,label,parser_id,trust,confidence)
             SELECT 'generation:progress-derive','fact:bulk:'||value,'data_field','FIELD-'||value,'parser:v1','extracted','high' FROM n;
             WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<5000)
             INSERT INTO archaeology_evidence_links
              (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             SELECT 'generation:progress-derive','fact','fact:bulk:'||value,'span','span:bulk:'||value,'supporting' FROM n;"
        ).unwrap();
        let delayed = StructuralGraphCancellation::default();
        delayed.cancel_after_checks(2);
        assert!(derive_template_candidates(
            &progress,
            derive_input(
                "job:progress-derive",
                "generation:progress-derive",
                OWNER,
                &delayed,
                ArchaeologyDeterministicLimits::default()
            )
        )
        .is_err());
        assert!(delayed.check_count() > 1);
        assert_eq!(
            load_job(&progress, "job:progress-derive").unwrap().stage,
            ArchaeologyJobStage::Derive
        );
        assert_eq!(
            count_where(
                &progress,
                "archaeology_rules",
                "generation_id='generation:progress-derive' AND rule_id='rule:stale'"
            ),
            1
        );
    }

    #[test]
    fn clustered_conflicts_domains_and_endpoint_evidence_persist_setwise() {
        let connection = derive_fixture("job:cluster-persist", "generation:cluster-persist");
        let rules = vec![
            persisted_cluster_rule(
                "rule:cluster:a",
                "fact:predicate",
                "span:predicate",
                "fact:generated:predicate",
                "span:generated:predicate",
                "rule:cluster:b",
            ),
            persisted_cluster_rule(
                "rule:cluster:b",
                "fact:generated:predicate",
                "span:generated:predicate",
                "fact:predicate",
                "span:predicate",
                "rule:cluster:a",
            ),
        ];
        let cancellation = StructuralGraphCancellation::default();
        let transaction = connection.unchecked_transaction().unwrap();
        persist_deterministic_rules(
            &transaction,
            "generation:cluster-persist",
            PARSER_MANIFEST,
            "algorithm:v1",
            T1,
            &rules,
            ArchaeologyDeterministicLimits::default(),
            &cancellation,
        )
        .unwrap();
        transaction.commit().unwrap();
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_domains",
                "generation_id='generation:cluster-persist' AND domain_id='domain:other'"
            ),
            2
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_relations",
                "generation_id='generation:cluster-persist' AND kind='conflicts_with'"
            ),
            1
        );
        assert_eq!(count_where(&connection,"archaeology_evidence_links","generation_id='generation:cluster-persist' AND owner_kind='rule_relation' AND evidence_kind='rule'"),2);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:cluster-persist' AND event_id='review:accepted'"
            ),
            1
        );
    }

    #[test]
    fn every_active_stage_can_cancel_without_replacing_ready() {
        let stages = [
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Link,
            ArchaeologyJobStage::Derive,
            ArchaeologyJobStage::Synthesize,
            ArchaeologyJobStage::Validate,
            ArchaeologyJobStage::Publish,
            ArchaeologyJobStage::Cleanup,
        ];
        for (target_index, target) in stages.iter().enumerate() {
            let connection = fixture();
            let job = format!("job:cancel:{target_index}");
            let generation = format!("generation:cancel:{target_index}");
            start(&connection, &job, &generation, OWNER);
            if target_index >= stage_index(&ArchaeologyJobStage::Publish).unwrap() {
                advance_to_publish(&connection, &job, &generation);
            } else {
                for index in 0..target_index {
                    checkpoint(
                        &connection,
                        &job,
                        stages[index].clone(),
                        stages[index + 1].clone(),
                        index as u64 + 1,
                    );
                }
            }
            if matches!(target, ArchaeologyJobStage::Cleanup) {
                publish(&connection, &job, &generation);
            }
            if target_index == 3 {
                pause_job(&connection, &job, OWNER, T1).unwrap();
            }
            let cancelling = request_cancel(&connection, &job, OWNER, T1).unwrap();
            assert_eq!(cancelling.stage, *target);
            assert!(cancelling.cancellation_requested);
            assert_eq!(
                acknowledge_cancel(&connection, &job, OWNER, T1)
                    .unwrap()
                    .state,
                ArchaeologyJobState::Cancelled
            );
            if matches!(target, ArchaeologyJobStage::Cleanup) {
                assert_ready_untouched_after_publish(&connection, &generation);
            } else {
                assert_eq!(generation_status(&connection, &generation), "cancelled");
                assert_ready_untouched(&connection);
            }
            assert_unrelated_codevetter_data_untouched(&connection);
        }
    }

    #[test]
    fn stale_recovery_is_cas_idempotent_and_live_owner_cannot_be_stolen() {
        let connection = fixture();
        start(&connection, "job:one", "generation:staging", OWNER);
        heartbeat_job(&connection, "job:one", OWNER, T1).unwrap();
        assert!(heartbeat_job(&connection, "job:one", OWNER, T0).is_err());
        assert!(recover_stale_job(
            &connection,
            REPO,
            "owner:two",
            "2026-01-01T00:03:00.000Z",
            "2026-01-01T00:02:00.000Z",
        )
        .unwrap_err()
        .contains("later than now"));
        assert!(recover_stale_job(
            &connection,
            REPO,
            "owner:two",
            "2026-01-01T00:00:30.000Z",
            "2026-01-01T00:02:00.000Z",
        )
        .unwrap_err()
        .contains("still live"));
        let recovered = recover_stale_job(
            &connection,
            REPO,
            "owner:two",
            "2026-01-01T00:01:30.000Z",
            "2026-01-01T00:02:00.000Z",
        )
        .unwrap();
        assert_eq!(recovered.owner_id.as_deref(), Some("owner:two"));
        assert_eq!(recovered.state, ArchaeologyJobState::Paused);
        assert_eq!(
            recover_stale_job(
                &connection,
                REPO,
                "owner:two",
                "2026-01-01T00:01:30.000Z",
                "2026-01-01T00:02:01.000Z",
            )
            .unwrap(),
            recovered
        );
        assert!(resume_job(&connection, "job:one", OWNER, T1).is_err());
        assert_eq!(
            resume_job(
                &connection,
                "job:one",
                "owner:two",
                "2026-01-01T00:02:01.000Z",
            )
            .unwrap()
            .state,
            ArchaeologyJobState::Running
        );
    }

    #[test]
    fn cancelling_and_paused_intent_survive_crash_recovery() {
        for cancelling in [false, true] {
            let connection = fixture();
            start(&connection, "job:one", "generation:staging", OWNER);
            if cancelling {
                request_cancel(&connection, "job:one", OWNER, T1).unwrap();
            } else {
                pause_job(&connection, "job:one", OWNER, T1).unwrap();
            }
            connection
                .execute(
                    "UPDATE archaeology_jobs SET updated_at = ?2 WHERE job_id = ?1",
                    params!["job:one", T0],
                )
                .unwrap();
            let recovered = recover_stale_job(
                &connection,
                REPO,
                "owner:recovery",
                "2026-01-01T00:00:30.000Z",
                T1,
            )
            .unwrap();
            assert_eq!(
                recovered.state,
                if cancelling {
                    ArchaeologyJobState::Cancelling
                } else {
                    ArchaeologyJobState::Paused
                }
            );
        }
    }

    #[test]
    fn checkpoint_and_error_storage_are_bounded_and_failure_preserves_ready() {
        let connection = fixture();
        start(&connection, "job:one", "generation:staging", OWNER);
        let oversized = "x".repeat(MAX_ID_BYTES + 1);
        assert!(checkpoint_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Inventory,
            "checkpoint:large",
            &ArchaeologyJobCheckpoint {
                cursor_identity: Some(oversized),
                ..ArchaeologyJobCheckpoint::default()
            },
            0,
            Some(10),
            T1,
        )
        .unwrap_err()
        .contains("opaque safe token"));
        for unsafe_cursor in [".env", "/Users/person/private.cbl", "password=secret-value"] {
            assert!(checkpoint_job(
                &connection,
                "job:one",
                OWNER,
                ArchaeologyJobStage::Inventory,
                ArchaeologyJobStage::Inventory,
                "checkpoint:unsafe",
                &ArchaeologyJobCheckpoint {
                    cursor_identity: Some(unsafe_cursor.to_string()),
                    ..ArchaeologyJobCheckpoint::default()
                },
                0,
                Some(10),
                T1,
            )
            .is_err());
        }
        let failed = fail_job(
            &connection,
            "job:one",
            OWNER,
            ArchaeologyJobErrorCode::ParserFailed,
            T1,
        )
        .unwrap();
        assert_eq!(failed.state, ArchaeologyJobState::Failed);
        assert_eq!(failed.errors, ["parser_failed"]);
        assert_eq!(
            generation_status(&connection, "generation:staging"),
            "failed"
        );
        assert_ready_untouched(&connection);
    }

    #[test]
    fn publication_is_atomic_owner_checked_and_idempotent() {
        let connection = fixture();
        start(&connection, "job:publish", "generation:publish", OWNER);
        advance_to_publish(&connection, "job:publish", "generation:publish");

        let mut wrong_identity = publication("job:publish", "generation:publish");
        wrong_identity.identity.config = "config:changed";
        assert!(publish_generation(&connection, wrong_identity).is_err());

        let mut wrong_owner = publication("job:publish", "generation:publish");
        wrong_owner.owner_id = "owner:other";
        assert!(publish_generation(&connection, wrong_owner).is_err());

        let first = publish_generation(
            &connection,
            publication("job:publish", "generation:publish"),
        )
        .unwrap();
        let retry = publish_generation(
            &connection,
            publication("job:publish", "generation:publish"),
        )
        .unwrap();
        assert_eq!(first, retry);
        assert_eq!(retry.stage, ArchaeologyJobStage::Cleanup);
        assert_eq!(generation_status(&connection, READY), "superseded");
        assert_eq!(
            generation_status(&connection, "generation:publish"),
            "ready"
        );
        assert_eq!(ready_generation(&connection), "generation:publish");
        assert_eq!(
            count_rows(&connection, "archaeology_temporal_generations"),
            1
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_snapshots"),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_events"),
            0
        );
        let temporal: (String, String, bool, bool) = connection
            .query_row(
                "SELECT generation.coverage_state,generation.coverage_reasons_json,
                        generation.prior_temporal_generation_identity IS NULL,
                        length(generation.catalog_identity) > 0
                 FROM archaeology_temporal_generations generation
                 WHERE generation.generation_id='generation:publish'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(temporal.0, "partial");
        assert!(temporal.1.contains("history_index_unavailable"));
        assert!(temporal.1.contains("missing_prior_generation"));
        assert!(temporal.2);
        assert!(temporal.3);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:publish' AND decision='candidate'"
            ),
            0
        );
    }

    #[test]
    fn compatible_publications_persist_a_partial_before_after_delta() {
        let connection = fixture();
        start(&connection, "job:first", "generation:first", OWNER);
        advance_to_publish(&connection, "job:first", "generation:first");
        publish(&connection, "job:first", "generation:first");
        complete_job(&connection, "job:first", OWNER, T1).unwrap();

        start(&connection, "job:second", "generation:second", OWNER);
        advance_to_publish(&connection, "job:second", "generation:second");
        publish(&connection, "job:second", "generation:second");

        assert_eq!(
            count_rows(&connection, "archaeology_temporal_generations"),
            2
        );
        let delta: (i64, String, bool, bool, bool, String) = connection
            .query_row(
                "SELECT COUNT(event.event_identity),MIN(event.event_kind),
                        MIN(event.before_snapshot_identity IS NOT NULL),
                        MIN(event.after_snapshot_identity IS NOT NULL),
                        MIN(generation.prior_temporal_generation_identity IS NOT NULL),
                        MIN(event.coverage_reasons_json)
                 FROM archaeology_temporal_generations generation
                 JOIN archaeology_rule_temporal_events event
                   USING (temporal_generation_identity)
                 WHERE generation.generation_id='generation:second'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(delta.0, 1);
        assert_eq!(delta.1, "observed");
        assert!(delta.2 && delta.3 && delta.4);
        assert!(delta.5.contains("history_index_unavailable"));
    }

    #[test]
    fn exact_persisted_history_justifies_changed_introduced_and_removed_events() {
        for (name, baseline_extra, current_extra, expected) in [
            ("changed", false, false, "changed"),
            ("introduced", false, true, "introduced"),
            ("removed", true, false, "removed"),
        ] {
            let connection = fixture();
            let (a, b, c) = (revision('a'), revision('b'), revision('c'));
            seed_exact_job_history(&connection, &b, &a, 1, "v1.0.0");
            let first_job = format!("job:{name}:first");
            let first_generation = format!("generation:{name}:first");
            start(&connection, &first_job, &first_generation, OWNER);
            advance_to_validate(&connection, &first_job);
            seed_publishable_generation(&connection, &first_generation);
            if baseline_extra {
                seed_additional_publishable_rule(&connection, &first_generation, &b);
            }
            validate_generation_for_publication(
                &connection,
                publication(&first_job, &first_generation),
            )
            .unwrap();
            publish(&connection, &first_job, &first_generation);
            complete_job(&connection, &first_job, OWNER, T1).unwrap();

            seed_exact_job_history(&connection, &c, &b, 2, "v2.0.0");
            let second_job = format!("job:{name}:second");
            let second_generation = format!("generation:{name}:second");
            start_at(&connection, &second_job, &second_generation, OWNER, &c);
            advance_to_validate_at(&connection, &second_job, &c);
            seed_publishable_generation_at(&connection, &second_generation, &c);
            if current_extra {
                seed_additional_publishable_rule(&connection, &second_generation, &c);
            }
            validate_generation_for_publication(
                &connection,
                publication_at(&second_job, &second_generation, &c),
            )
            .unwrap();
            publish_generation(
                &connection,
                publication_at(&second_job, &second_generation, &c),
            )
            .unwrap();

            let temporal: (String, String, String) = connection
                .query_row(
                    "SELECT generation.coverage_state,generation.coverage_reasons_json,
                            group_concat(event.event_kind,',')
                     FROM archaeology_temporal_generations generation
                     JOIN archaeology_rule_temporal_events event
                       USING (temporal_generation_identity)
                     WHERE generation.generation_id=?1",
                    [&second_generation],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(temporal.0, "complete", "{name}: {}", temporal.1);
            assert_eq!(temporal.1, "[]");
            assert!(
                temporal.2.split(',').any(|kind| kind == expected),
                "{name}: {}",
                temporal.2
            );
        }
    }

    #[test]
    fn exact_history_does_not_rebase_stale_accepted_evidence() {
        let connection = fixture();
        let (a, b, c) = (revision('a'), revision('b'), revision('c'));
        seed_exact_job_history(&connection, &b, &a, 1, "v1.0.0");
        start(
            &connection,
            "job:accepted:first",
            "generation:accepted:first",
            OWNER,
        );
        advance_to_publish(
            &connection,
            "job:accepted:first",
            "generation:accepted:first",
        );
        publish(
            &connection,
            "job:accepted:first",
            "generation:accepted:first",
        );

        let (rule_id, stable_rule): (String, String) = connection
            .query_row(
                "SELECT rule_id,stable_rule_identity FROM archaeology_rules
                 WHERE generation_id='generation:accepted:first' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let acceptance = sha256_identity(b"accepted-evidence-before-history-change");
        let transaction = connection.unchecked_transaction().unwrap();
        let candidate = ensure_candidate_lifecycle(
            &transaction,
            REPO,
            "generation:accepted:first",
            &rule_id,
            &stable_rule,
            T1,
        )
        .unwrap();
        assert_eq!(candidate.projected.last_sequence, 1);
        let prior_event = transaction
            .query_row(
                "SELECT event_id FROM archaeology_rule_review_events
                 WHERE repository_id=?1 AND stable_rule_identity=?2
                 ORDER BY logical_sequence DESC LIMIT 1",
                params![REPO, stable_rule],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        append_lifecycle_event(
            &transaction,
            ArchaeologyLifecycleAppend {
                event_id: &acceptance,
                repository_id: REPO,
                generation_id: "generation:accepted:first",
                rule_id: &rule_id,
                stable_rule_identity: &stable_rule,
                expected_previous_sequence: 1,
                expected_prior_event_id: Some(&prior_event),
                related_generation_id: None,
                related_rule_id: None,
                provenance: ArchaeologyReviewerProvenance {
                    kind: ArchaeologyReviewerKind::Human,
                    actor_id: "reviewer:human".into(),
                    authority_id: None,
                },
                action: ArchaeologyLifecycleAction::Accept,
                created_at: T1,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
        complete_job(&connection, "job:accepted:first", OWNER, T1).unwrap();

        seed_exact_job_history(&connection, &c, &b, 2, "v2.0.0");
        start_at(
            &connection,
            "job:accepted:second",
            "generation:accepted:second",
            OWNER,
            &c,
        );
        advance_to_validate_at(&connection, "job:accepted:second", &c);
        seed_publishable_generation_at(&connection, "generation:accepted:second", &c);
        validate_generation_for_publication(
            &connection,
            publication_at("job:accepted:second", "generation:accepted:second", &c),
        )
        .unwrap();
        publish_generation(
            &connection,
            publication_at("job:accepted:second", "generation:accepted:second", &c),
        )
        .unwrap();

        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:accepted:second' AND decision='accepted'"
            ),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT event.event_kind FROM archaeology_temporal_generations generation
                     JOIN archaeology_rule_temporal_events event
                       USING (temporal_generation_identity)
                     WHERE generation.generation_id='generation:accepted:second'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "changed"
        );
    }

    #[test]
    fn publication_rolls_back_every_write_when_pointer_swap_fails() {
        let connection = fixture();
        seed_exact_job_history(&connection, REVISION, &revision('a'), 1, "v1.0.0");
        start(&connection, "job:publish", "generation:publish", OWNER);
        advance_to_publish(&connection, "job:publish", "generation:publish");
        connection
            .execute_batch(
                "CREATE TEMP TRIGGER reject_archaeology_pointer
                 BEFORE UPDATE OF ready_generation_id ON archaeology_repositories
                 BEGIN SELECT RAISE(ABORT, 'fixture pointer race'); END;",
            )
            .unwrap();

        assert!(publish_generation(
            &connection,
            publication("job:publish", "generation:publish"),
        )
        .unwrap_err()
        .contains("fixture pointer race"));
        assert_eq!(generation_status(&connection, READY), "ready");
        assert_eq!(
            generation_status(&connection, "generation:publish"),
            "staging"
        );
        assert_eq!(ready_generation(&connection), READY);
        assert_eq!(
            load_job(&connection, "job:publish").unwrap().stage,
            ArchaeologyJobStage::Publish
        );
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "generation_id='generation:publish'"
            ),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_temporal_generations"),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_snapshots"),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_events"),
            0
        );
    }

    #[test]
    fn publication_requires_a_dedicated_receipt_and_proven_empty_inventory() {
        let connection = fixture();
        start(
            &connection,
            "job:empty-rejected",
            "generation:empty-rejected",
            OWNER,
        );
        advance_to_validate(&connection, "job:empty-rejected");
        assert!(checkpoint_job(
            &connection,
            "job:empty-rejected",
            OWNER,
            ArchaeologyJobStage::Validate,
            ArchaeologyJobStage::Publish,
            "checkpoint:bypass",
            &ArchaeologyJobCheckpoint::default(),
            7,
            Some(10),
            T1,
        )
        .unwrap_err()
        .contains("deterministic publication receipt"));
        assert!(validate_generation_for_publication(
            &connection,
            publication("job:empty-rejected", "generation:empty-rejected"),
        )
        .unwrap_err()
        .contains("completed inventory with total zero"));

        let connection = fixture();
        set_repository_current(&connection, "generation:empty-proven");
        let mut empty_job = new_job("job:empty-proven", "generation:empty-proven", OWNER);
        empty_job.total_units = Some(0);
        start_job(&connection, empty_job).unwrap();
        advance_empty_to_validate(&connection, "job:empty-proven");
        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json = ?2
                 WHERE generation_id = ?1",
                params![
                    "generation:empty-proven",
                    unavailable_coverage("Inventory completed with no source units"),
                ],
            )
            .unwrap();
        let validated = validate_generation_for_publication(
            &connection,
            publication("job:empty-proven", "generation:empty-proven"),
        )
        .unwrap();
        assert_eq!(validated.stage, ArchaeologyJobStage::Publish);
        assert!(validated
            .checkpoint_identity
            .as_deref()
            .is_some_and(|identity| identity.starts_with("validation:v1:")));
        assert_eq!(
            publish(&connection, "job:empty-proven", "generation:empty-proven").stage,
            ArchaeologyJobStage::Cleanup
        );
    }

    #[test]
    fn validation_rejects_tampered_receipts_uncited_clauses_and_search_drift() {
        let connection = fixture();
        start(&connection, "job:tampered", "generation:tampered", OWNER);
        advance_to_publish(&connection, "job:tampered", "generation:tampered");
        connection
            .execute(
                "UPDATE archaeology_jobs SET checkpoint_json = checkpoint_json || ' '
                 WHERE job_id = 'job:tampered'",
                [],
            )
            .unwrap();
        assert!(publish_generation(
            &connection,
            publication("job:tampered", "generation:tampered"),
        )
        .unwrap_err()
        .contains("receipt identity"));

        let connection = fixture();
        start(
            &connection,
            "job:post-validate",
            "generation:post-validate",
            OWNER,
        );
        advance_to_publish(&connection, "job:post-validate", "generation:post-validate");
        connection
            .execute(
                "UPDATE archaeology_facts SET label = 'same-count mutation'
                 WHERE generation_id = 'generation:post-validate'",
                [],
            )
            .unwrap();
        assert!(publish_generation(
            &connection,
            publication("job:post-validate", "generation:post-validate"),
        )
        .unwrap_err()
        .contains("changed after validation"));
    }

    #[test]
    fn validation_rejects_same_count_scope_polymorphic_relation_and_fts_mutations() {
        let cases = [
            (
                "coverage-units",
                "UPDATE archaeology_generations SET coverage_json=json_set(coverage_json,'$.discovered_source_units',2) WHERE generation_id=?1",
                "coverage",
            ),
            (
                "coverage-bytes",
                "UPDATE archaeology_source_units SET byte_count=81 WHERE generation_id=?1",
                "coverage does not match persisted source rows",
            ),
            (
                "sensitive-path",
                "UPDATE archaeology_source_units SET relative_path='.env' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "unix-absolute-path",
                "UPDATE archaeology_source_units SET relative_path='/workspace/program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "windows-drive-path",
                "UPDATE archaeology_source_units SET relative_path='C:\\workspace\\program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "windows-drive-relative-path",
                "UPDATE archaeology_source_units SET relative_path='C:workspace\\program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "windows-single-root-path",
                "UPDATE archaeology_source_units SET relative_path='\\workspace\\program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "unc-path",
                "UPDATE archaeology_source_units SET relative_path='\\\\server\\share\\program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "file-uri-path",
                "UPDATE archaeology_source_units SET relative_path='file:///workspace/program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "parent-traversal-path",
                "UPDATE archaeology_source_units SET relative_path='src/../shared/program.cbl' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "lineage-secret",
                "UPDATE archaeology_source_units SET include_lineage_json='[{\"kind\":\"include\",\"source_unit_id\":\"safe\",\"target_source_unit_id\":null,\"evidence_span_id\":\"span\",\"detail\":\".env\"}]' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "recovery-path",
                "UPDATE archaeology_source_units SET recovery_json='[{\"kind\":\"recovered\",\"span_id\":\"span\",\"reason\":\"file:///workspace/program.cbl\"}]' WHERE generation_id=?1",
                "secret/path policy",
            ),
            (
                "malformed-lineage",
                "UPDATE archaeology_source_units SET include_lineage_json='[1]' WHERE generation_id=?1",
                "include lineage is invalid",
            ),
            (
                "lineage-owner-mismatch",
                "UPDATE archaeology_source_units SET include_lineage_json=json_array(json_object('kind','include','source_unit_id','other','target_source_unit_id',NULL,'evidence_span_id',(SELECT span_id FROM archaeology_source_spans WHERE generation_id=?1 LIMIT 1),'detail','safe')) WHERE generation_id=?1",
                "lineage source does not match",
            ),
            (
                "lineage-dangling-target",
                "UPDATE archaeology_source_units SET include_lineage_json=json_array(json_object('kind','include','source_unit_id',source_unit_id,'target_source_unit_id','missing','evidence_span_id',(SELECT span_id FROM archaeology_source_spans WHERE generation_id=?1 LIMIT 1),'detail','safe')) WHERE generation_id=?1",
                "lineage target is outside",
            ),
            (
                "unresolved-lineage-without-marker",
                "UPDATE archaeology_source_units SET include_lineage_json=json_array(json_object('kind','copybook','source_unit_id',source_unit_id,'target_source_unit_id',NULL,'evidence_span_id',(SELECT span_id FROM archaeology_source_spans WHERE generation_id=?1 LIMIT 1),'detail','safe')) WHERE generation_id=?1",
                "target metadata is not honestly resolved",
            ),
            (
                "lineage-dangling-span",
                "UPDATE archaeology_source_units SET include_lineage_json=json_array(json_object('kind','copybook','source_unit_id',source_unit_id,'target_source_unit_id',NULL,'evidence_span_id','span:missing','detail','unresolved target')) WHERE generation_id=?1",
                "lineage evidence span does not belong",
            ),
            (
                "recovery-dangling-span",
                "UPDATE archaeology_source_units SET recovery_json=json_array(json_object('kind','recovered','span_id','span:missing','reason','safe')) WHERE generation_id=?1",
                "recovery span does not belong",
            ),
            (
                "raw-path-identity",
                "UPDATE archaeology_source_units SET path_identity='src/raw.cbl' WHERE generation_id=?1",
                "identity is not opaque",
            ),
            (
                "noncanonical-content-hash",
                "UPDATE archaeology_source_units SET content_hash='ABC',hash_algorithm='sha256' WHERE generation_id=?1",
                "noncanonical content identity",
            ),
            (
                "spanned-null-content-hash",
                "UPDATE archaeology_source_units SET content_hash=NULL,hash_algorithm=NULL WHERE generation_id=?1",
                "requires a content hash",
            ),
            (
                "empty-span",
                "UPDATE archaeology_source_spans SET end_byte=start_byte WHERE generation_id=?1",
                "span_bounds=1",
            ),
            (
                "span-past-unit-bytes",
                "UPDATE archaeology_source_spans SET end_byte=81 WHERE generation_id=?1",
                "span_bounds=1",
            ),
            (
                "span-past-unit-lines",
                "UPDATE archaeology_source_spans SET end_line=5 WHERE generation_id=?1",
                "span_bounds=1",
            ),
            (
                "span-start-column-past-unit",
                "UPDATE archaeology_source_spans SET start_column=82,end_line=2,end_column=1 WHERE generation_id=?1",
                "out-of-bounds span column",
            ),
            (
                "span-end-column-past-unit",
                "UPDATE archaeology_source_spans SET end_column=82 WHERE generation_id=?1",
                "out-of-bounds span column",
            ),
            (
                "opaque-with-evidence",
                "UPDATE archaeology_source_units SET classification='opaque' WHERE generation_id=?1",
                "cannot have indexed evidence",
            ),
            (
                "source-parser",
                "UPDATE archaeology_source_units SET parser_version='2' WHERE generation_id=?1",
                "outside the generation manifest",
            ),
            (
                "fact-parser",
                "UPDATE archaeology_facts SET parser_id='parser:other' WHERE generation_id=?1",
                "outside the generation manifest",
            ),
            (
                "uncited-clause",
                "DELETE FROM archaeology_evidence_links WHERE generation_id=?1 AND owner_kind='rule_clause' AND evidence_kind='fact'",
                "clause_uncited=1",
            ),
            (
                "cross-revision",
                "UPDATE archaeology_source_spans SET revision_sha='cccccccccccccccccccccccccccccccccccccccc' WHERE generation_id=?1",
                "span_revision=1",
            ),
            (
                "stale-fts",
                "UPDATE archaeology_rule_fts SET title='stale title' WHERE generation_id=?1",
                "FTS linkage",
            ),
            (
                "rule-identity",
                "UPDATE archaeology_rules SET parser_identity='parser:other' WHERE generation_id=?1",
                "rule_scope=1",
            ),
            (
                "dangling-owner",
                "INSERT INTO archaeology_evidence_links (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role) SELECT ?1,'fact','fact:missing','span',span_id,'supporting' FROM archaeology_source_spans WHERE generation_id=?1 LIMIT 1",
                "dangling_owner=1",
            ),
            (
                "uncited-relation",
                "INSERT INTO archaeology_rule_relations (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust) SELECT ?1,'relation:uncited',rule_id,rule_id,'depends_on','deterministic' FROM archaeology_rules WHERE generation_id=?1 LIMIT 1",
                "relation_uncited=1",
            ),
            (
                "null-fts",
                "UPDATE archaeology_rule_fts SET title=NULL WHERE generation_id=?1",
                "FTS linkage",
            ),
            (
                "duplicate-fts",
                "INSERT INTO archaeology_rule_fts (generation_id,rule_id,title,clause_text,domain_text) SELECT generation_id,rule_id,title,clause_text,domain_text FROM archaeology_rule_fts WHERE generation_id=?1",
                "FTS linkage",
            ),
        ];
        for (name, mutation, expected) in cases {
            let connection = fixture();
            let job = format!("job:{name}");
            let generation = format!("generation:{name}");
            start(&connection, &job, &generation, OWNER);
            advance_to_validate(&connection, &job);
            seed_publishable_generation(&connection, &generation);
            connection.execute(mutation, [&generation]).unwrap();
            let error =
                validate_generation_for_publication(&connection, publication(&job, &generation))
                    .unwrap_err();
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn hashed_zero_span_partial_unit_reconciles_as_indexed_inventory() {
        let connection = fixture();
        start(
            &connection,
            "job:hashed-zero-span",
            "generation:hashed-zero-span",
            OWNER,
        );
        advance_to_validate(&connection, "job:hashed-zero-span");
        seed_publishable_generation(&connection, "generation:hashed-zero-span");
        connection
            .execute_batch(
                "DELETE FROM archaeology_evidence_links
                   WHERE generation_id='generation:hashed-zero-span';
                 DELETE FROM archaeology_rules
                   WHERE generation_id='generation:hashed-zero-span';
                 DELETE FROM archaeology_facts
                   WHERE generation_id='generation:hashed-zero-span';
                 DELETE FROM archaeology_source_spans
                   WHERE generation_id='generation:hashed-zero-span';
                 UPDATE archaeology_generations
                    SET coverage_json=json_set(
                        coverage_json,
                        '$.state','partial',
                        '$.parser_coverage','partial',
                        '$.repository_coverage','partial',
                        '$.temporal_coverage','partial',
                        '$.reasons',json_array('No evidence spans emitted'))
                  WHERE generation_id='generation:hashed-zero-span';
                 UPDATE archaeology_source_units
                    SET coverage_json=json_set(
                        coverage_json,
                        '$.state','partial',
                        '$.parser_coverage','partial',
                        '$.repository_coverage','partial',
                        '$.temporal_coverage','partial',
                        '$.reasons',json_array('No evidence spans emitted'))
                  WHERE generation_id='generation:hashed-zero-span';",
            )
            .unwrap();

        assert_eq!(
            validate_generation_for_publication(
                &connection,
                publication("job:hashed-zero-span", "generation:hashed-zero-span"),
            )
            .unwrap()
            .stage,
            ArchaeologyJobStage::Publish
        );
        assert_eq!(
            publish(
                &connection,
                "job:hashed-zero-span",
                "generation:hashed-zero-span",
            )
            .stage,
            ArchaeologyJobStage::Cleanup
        );
    }

    #[test]
    fn validation_rejects_cross_unit_lineage_and_recovery_spans() {
        for (name, mutation, expected) in [
            (
                "lineage-cross-unit",
                "UPDATE archaeology_source_units
                    SET include_lineage_json=json_array(json_object(
                        'kind','copybook','source_unit_id',source_unit_id,
                        'target_source_unit_id',NULL,'evidence_span_id','span:cross-unit',
                        'detail','unresolved target'))
                  WHERE generation_id=?1 AND source_unit_id!=?2",
                "lineage evidence span does not belong",
            ),
            (
                "recovery-cross-unit",
                "UPDATE archaeology_source_units
                    SET recovery_json=json_array(json_object(
                        'kind','recovered','span_id','span:cross-unit','reason','safe'))
                  WHERE generation_id=?1 AND source_unit_id!=?2",
                "recovery span does not belong",
            ),
        ] {
            let connection = fixture();
            let job = format!("job:{name}");
            let generation = format!("generation:{name}");
            let cross_unit = opaque_test_id("archaeology-source-unit", &format!("{name}:cross"));
            start(&connection, &job, &generation, OWNER);
            advance_to_validate(&connection, &job);
            seed_publishable_generation(&connection, &generation);
            connection
                .execute(
                    "INSERT INTO archaeology_source_units
                       (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                        hash_algorithm,language,parser_id,parser_version,classification,
                        byte_count,line_count,coverage_json)
                     VALUES (?1,?2,?3,'src/cross.cbl',
                        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                        'sha256','cobol','parser:v1','1','source',8,1,?4)",
                    params![
                        generation,
                        cross_unit,
                        opaque_test_id("archaeology-path", &format!("{name}:cross")),
                        complete_coverage(),
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO archaeology_source_spans
                       (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                        start_line,start_column,end_line,end_column)
                     VALUES (?1,'span:cross-unit',?2,
                        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,1,1,1,1,2)",
                    params![generation, cross_unit],
                )
                .unwrap();
            connection
                .execute(mutation, params![generation, cross_unit])
                .unwrap();

            let error =
                validate_generation_for_publication(&connection, publication(&job, &generation))
                    .unwrap_err();
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn metadata_link_validation_is_set_based_at_the_4096_entry_bound() {
        let connection = fixture();
        let (job, generation) = ("job:metadata-scale", "generation:metadata-scale");
        start(&connection, job, generation, OWNER);
        advance_to_validate(&connection, job);
        seed_publishable_generation(&connection, generation);
        for index in 0..4 {
            let unit = opaque_test_id("archaeology-source-unit", &format!("metadata:{index}"));
            let path = opaque_test_id("archaeology-path", &format!("metadata:{index}"));
            let span = format!("s{index}");
            let recovery = serde_json::to_string(&vec![
                ArchaeologyAdapterRegion {
                    kind: ArchaeologyAdapterRegionKind::Recovered,
                    span_id: span.clone(),
                    reason: "x".into(),
                };
                1_024
            ])
            .unwrap();
            assert!(recovery.len() < MAX_CHECKPOINT_BYTES);
            connection
                .execute(
                    "INSERT INTO archaeology_source_units
                   (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                    hash_algorithm,language,parser_id,parser_version,classification,
                    byte_count,line_count,recovery_json,coverage_json)
                 VALUES (?1,?2,?3,?4,
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    'sha256','cobol','parser:v1','1','source',80,4,?5,?6)",
                    params![
                        generation,
                        unit,
                        path,
                        format!("src/metadata-{index}.cbl"),
                        recovery,
                        complete_coverage()
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO archaeology_source_spans
                   (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                    start_line,start_column,end_line,end_column)
                 VALUES (?1,?2,?3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,20,1,1,1,21)",
                    params![generation, span, unit],
                )
                .unwrap();
        }
        let coverage = serde_json::to_string(&ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Complete,
            parser_coverage: ArchaeologyCoverageState::Complete,
            repository_coverage: ArchaeologyCoverageState::Complete,
            temporal_coverage: ArchaeologyCoverageState::Complete,
            discovered_source_units: 5,
            indexed_source_units: 5,
            discovered_bytes: 400,
            indexed_bytes: 400,
            reasons: vec![],
        })
        .unwrap();
        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json=?2 WHERE generation_id=?1",
                params![generation, coverage],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT SUM(json_array_length(recovery_json)) FROM archaeology_source_units
             WHERE generation_id=?1",
                    [generation],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            4_096
        );
        assert_eq!(
            validate_generation_for_publication(&connection, publication(job, generation))
                .unwrap()
                .stage,
            ArchaeologyJobStage::Publish
        );

        let mut plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {METADATA_LINK_INTEGRITY_SQL}"))
            .unwrap();
        let details = plan
            .query_map([generation], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details.iter().all(|detail| !detail.contains("CORRELATED")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("target") && detail.contains("INDEX")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("span") && detail.contains("INDEX")),
            "{details:?}"
        );
    }

    #[test]
    fn stale_repository_blocks_publish_and_terminal_retry_is_read_only() {
        let connection = fixture();
        start(&connection, "job:stale", "generation:stale", OWNER);
        advance_to_publish(&connection, "job:stale", "generation:stale");
        connection
            .execute(
                "UPDATE archaeology_repositories
                 SET current_revision = 'cccccccccccccccccccccccccccccccccccccccc'
                 WHERE repository_id = ?1",
                [REPO],
            )
            .unwrap();
        assert!(
            publish_generation(&connection, publication("job:stale", "generation:stale"),).is_err()
        );
        assert_eq!(generation_status(&connection, READY), "ready");
        assert_eq!(
            generation_status(&connection, "generation:stale"),
            "staging"
        );
        assert_eq!(
            load_job(&connection, "job:stale").unwrap().stage,
            ArchaeologyJobStage::Publish
        );

        set_repository_current(&connection, "generation:stale");
        publish(&connection, "job:stale", "generation:stale");
        let completed = complete_job(&connection, "job:stale", OWNER, T1).unwrap();
        assert_eq!(completed.state, ArchaeologyJobState::Completed);
        assert_eq!(
            publish_generation(&connection, publication("job:stale", "generation:stale"),)
                .unwrap()
                .state,
            ArchaeologyJobState::Completed
        );
    }

    #[test]
    fn protected_only_catalog_publishes_with_explicit_bounded_gap_coverage() {
        let connection = fixture();
        start(&connection, "job:protected", "generation:protected", OWNER);
        advance_to_validate(&connection, "job:protected");
        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json = ?2
                 WHERE generation_id = ?1",
                params![
                    "generation:protected",
                    partial_coverage("Protected source was intentionally not read", 1, 0),
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_source_units
                 (generation_id, source_unit_id, path_identity, language,
                  parser_id, parser_version, classification, byte_count,
                  line_count)
                 VALUES ('generation:protected',?1,?2,
                         'unknown','unavailable','unavailable','protected',0,0)",
                params![
                    opaque_test_id("archaeology-source-unit", "protected"),
                    opaque_test_id("archaeology-path", "protected"),
                ],
            )
            .unwrap();
        assert!(validate_generation_for_publication(
            &connection,
            publication("job:protected", "generation:protected"),
        )
        .unwrap_err()
        .contains("source unit partial or unavailable coverage requires a reason"));
        connection
            .execute(
                "UPDATE archaeology_source_units SET coverage_json = ?2
                 WHERE generation_id = ?1",
                params![
                    "generation:protected",
                    unavailable_coverage("Protected source was intentionally not read"),
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE archaeology_source_units SET relative_path='.env'
                 WHERE generation_id='generation:protected'",
                [],
            )
            .unwrap();
        assert!(
            validation_error(&connection, "job:protected", "generation:protected")
                .contains("secret/path policy")
        );
        connection
            .execute(
                "UPDATE archaeology_source_units SET relative_path=NULL,path_identity='raw/path',
                    content_hash='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    hash_algorithm='sha256',
                    include_lineage_json='[{\"kind\":\"include\",\"source_unit_id\":\"safe\",\"target_source_unit_id\":null,\"evidence_span_id\":\"span\",\"detail\":\"safe\"}]',
                    recovery_json='[{\"kind\":\"recovered\",\"span_id\":\"span\",\"reason\":\"safe\"}]'
                 WHERE generation_id='generation:protected'",
                [],
            )
            .unwrap();
        assert!(
            validation_error(&connection, "job:protected", "generation:protected")
                .contains("identity is not opaque")
        );
        connection
            .execute(
                "UPDATE archaeology_source_units SET path_identity=?2
             WHERE generation_id=?1",
                params![
                    "generation:protected",
                    opaque_test_id("archaeology-path", "protected")
                ],
            )
            .unwrap();
        assert!(
            validation_error(&connection, "job:protected", "generation:protected")
                .contains("cannot have indexed evidence")
        );
        connection
            .execute(
                "UPDATE archaeology_source_units SET content_hash=NULL,hash_algorithm=NULL
             WHERE generation_id='generation:protected'",
                [],
            )
            .unwrap();
        assert!(
            validation_error(&connection, "job:protected", "generation:protected")
                .contains("retained path or parser metadata")
        );
        connection
            .execute_batch(
                "UPDATE archaeology_source_units SET include_lineage_json='[]',recovery_json='[]'
                 WHERE generation_id='generation:protected';
                 INSERT INTO archaeology_source_spans
                 (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                  start_line,start_column,end_line,end_column)
                 SELECT 'generation:protected','span:protected',source_unit_id,
                        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,0,1,1,1,1
                 FROM archaeology_source_units WHERE generation_id='generation:protected'",
            )
            .unwrap();
        assert!(
            validation_error(&connection, "job:protected", "generation:protected")
                .contains("cannot have indexed evidence")
        );
        connection
            .execute(
                "DELETE FROM archaeology_source_spans
                 WHERE generation_id='generation:protected'",
                [],
            )
            .unwrap();
        assert_eq!(
            validate_generation_for_publication(
                &connection,
                publication("job:protected", "generation:protected"),
            )
            .unwrap()
            .stage,
            ArchaeologyJobStage::Publish
        );
        assert_eq!(
            publish(&connection, "job:protected", "generation:protected").stage,
            ArchaeologyJobStage::Cleanup
        );
    }

    #[test]
    fn validation_bounds_coverage_and_search_payloads_before_hashing_or_aggregation() {
        let connection = fixture();
        start(&connection, "job:bounds", "generation:bounds", OWNER);
        advance_to_validate(&connection, "job:bounds");
        seed_publishable_generation(&connection, "generation:bounds");
        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json = ?2
                 WHERE generation_id = ?1",
                params![
                    "generation:bounds",
                    format!("{{\"oversized\":\"{}\"}}", "x".repeat(MAX_CHECKPOINT_BYTES)),
                ],
            )
            .unwrap();
        assert!(validate_generation_for_publication(
            &connection,
            publication("job:bounds", "generation:bounds"),
        )
        .unwrap_err()
        .contains("coverage exceeds its byte bound"));

        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json = ?2
                 WHERE generation_id = ?1",
                params!["generation:bounds", complete_coverage()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE archaeology_rule_search_manifest SET clause_text = ?2
                 WHERE generation_id = ?1",
                params![
                    "generation:bounds",
                    "x".repeat(MAX_RULE_CLAUSE_TEXT_BYTES + 1),
                ],
            )
            .unwrap();
        assert!(validate_generation_for_publication(
            &connection,
            publication("job:bounds", "generation:bounds"),
        )
        .unwrap_err()
        .contains("exceeds its validation bound"));

        let mut plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {}", search_integrity_sql()))
            .unwrap();
        let details = plan
            .query_map(["generation:bounds"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details.iter().all(|detail| !detail.contains("CORRELATED")),
            "100k-rule validation must stay set-based: {details:?}"
        );
        assert!(
            details
                .iter()
                .all(|detail| !detail.contains("VIRTUAL TABLE"))
                && details.iter().any(|detail| {
                    detail.contains("archaeology_rule_search_manifest") && detail.contains("INDEX")
                }),
            "validation must use the indexed manifest, not scan FTS: {details:?}"
        );
    }

    #[test]
    fn search_source_bounds_win_before_parity_aggregation() {
        let cases = [
            (
                "clause-bytes",
                format!("UPDATE archaeology_rule_clauses SET clause_text='{}' WHERE generation_id=?1",
                    "x".repeat(MAX_RULE_CLAUSE_TEXT_BYTES + 1)),
            ),
            (
                "domain-bytes",
                format!("INSERT INTO archaeology_rule_domains VALUES (?1,'rule:'||?1,'domain:large','{}',NULL)",
                    "x".repeat(MAX_RULE_DOMAIN_TEXT_BYTES + 1)),
            ),
            (
                "clause-count",
                format!("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{})
                    INSERT INTO archaeology_rule_clauses
                    SELECT ?1,'rule:'||?1,'clause:extra:'||x,x,'x','deterministic','high','[]' FROM n",
                    MAX_RULE_CLAUSES),
            ),
            (
                "domain-count",
                format!("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{})
                    INSERT INTO archaeology_rule_domains
                    SELECT ?1,'rule:'||?1,'domain:'||x,'x',NULL FROM n",
                    MAX_RULE_DOMAINS + 1),
            ),
            (
                "separator-byte",
                format!("UPDATE archaeology_rule_clauses SET clause_text='{}' WHERE generation_id=?1;
                    INSERT INTO archaeology_rule_clauses VALUES
                    (?1,'rule:'||?1,'clause:separator',1,'','deterministic','high','[]')",
                    "x".repeat(MAX_RULE_CLAUSE_TEXT_BYTES)),
            ),
        ];
        for (name, mutation) in cases {
            let mut connection = fixture();
            let generation = format!("generation:{name}");
            start(&connection, &format!("job:{name}"), &generation, OWNER);
            seed_publishable_generation(&connection, &generation);
            for statement in mutation
                .split(';')
                .filter(|statement| !statement.trim().is_empty())
            {
                connection.execute(statement, [&generation]).unwrap();
            }
            connection.execute(
                "UPDATE archaeology_rule_search_manifest SET title='parity drift' WHERE generation_id=?1",
                [&generation],
            ).unwrap();
            let transaction = connection.transaction().unwrap();
            let error = validate_search_integrity(&transaction, &generation).unwrap_err();
            assert!(
                error.contains("exceeds its validation bound"),
                "{name}: {error}"
            );
        }
    }

    #[test]
    fn cleanup_dry_run_and_apply_are_scoped_retryable_and_preserve_reviews() {
        let connection = fixture();
        start(&connection, "job:publish", "generation:publish", OWNER);
        advance_to_publish(&connection, "job:publish", "generation:publish");
        publish(&connection, "job:publish", "generation:publish");
        seed_superseded(&connection, "generation:old", "2020-01-01T00:00:00Z");
        connection
            .execute_batch(
                "INSERT INTO archaeology_rule_fts
                    (generation_id, rule_id, title, clause_text, domain_text)
                 VALUES ('generation:old','rule:one','one','clause','domain'),
                        ('generation:old','rule:two','two','clause','domain');
                 INSERT INTO archaeology_rule_review_events
                    (event_id, repository_id, rule_id, generation_id, decision,
                     reviewer_id, evidence_identity, created_at)
                 VALUES ('review:old','repo:jobs','rule:one','generation:old',
                         'accepted','reviewer:local','evidence:one','2021');
                 CREATE TABLE unrelated_cleanup_fixture (value TEXT NOT NULL);
                 INSERT INTO unrelated_cleanup_fixture VALUES ('keep');",
            )
            .unwrap();
        let synthesis_hash = format!("sha256:{}", "a".repeat(64));
        connection
            .execute(
                "INSERT INTO archaeology_synthesis_cache
                 (generation_id,cache_key,request_id,evidence_identity,packet_id,
                  provider_identity,provider_route_identity,model_identity,prompt_identity,policy_identity,status,
                  response_json,response_sha256,created_at,updated_at)
                 VALUES ('generation:old',?1,?1,?1,'packet:old','local',?1,'model',?1,?1,
                         'ready','{\"schema_version\":1}',?1,'2021','2021')",
                [&synthesis_hash],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_synthesis_attempts
                 (attempt_id,generation_id,cache_key,ordinal,status,network_scope,cost_class,
                  remote_disclosure_acknowledged,paid_disclosure_acknowledged,usage_source,
                  duration_ms,created_at)
                 VALUES ('attempt:old','generation:old',?1,1,'success','loopback','free',
                         0,0,'unavailable',1,'2021')",
                [&synthesis_hash],
            )
            .unwrap();

        let dry_run = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::DryRun, 1),
        )
        .unwrap();
        assert!(dry_run.dry_run);
        assert_eq!(dry_run.candidates.len(), 1);
        assert_eq!(dry_run.candidates[0].generation_id, "generation:old");
        assert_eq!(dry_run.candidates[0].search_index_rows, 2);
        assert_eq!(dry_run.candidates[0].synthesis_cache_rows, 1);
        assert_eq!(dry_run.candidates[0].synthesis_attempt_rows, 1);
        assert_eq!(dry_run.candidates[0].synthesis_response_bytes, 20);
        assert_eq!(dry_run.deleted_generations, 0);
        assert_eq!(
            generation_status(&connection, "generation:old"),
            "superseded"
        );

        let applied = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::Apply, 1),
        )
        .unwrap();
        assert_eq!(applied.deleted_generations, 1);
        assert_eq!(applied.deleted_search_index_rows, 2);
        assert_eq!(applied.deleted_synthesis_cache_rows, 1);
        assert_eq!(applied.deleted_synthesis_attempt_rows, 1);
        assert_eq!(applied.deleted_synthesis_response_bytes, 20);
        assert_eq!(applied.unavailable_resources, ["parser_cache"]);
        assert_eq!(ready_generation(&connection), "generation:publish");
        assert_eq!(generation_status(&connection, READY), "superseded");
        assert_eq!(count_rows(&connection, "archaeology_rule_review_events"), 1);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_rule_review_events",
                "event_id='review:old'"
            ),
            1
        );
        assert_eq!(count_rows(&connection, "unrelated_cleanup_fixture"), 1);

        let retry = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::Apply, 1),
        )
        .unwrap();
        assert!(retry.candidates.is_empty());
        assert_eq!(retry.deleted_generations, 0);

        let source_cleanup = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::Apply, 0),
        )
        .unwrap();
        assert_eq!(source_cleanup.deleted_generations, 1);
        assert_eq!(
            count_where(
                &connection,
                "archaeology_generations",
                "generation_id='generation:ready'"
            ),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_temporal_generations"),
            1
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_snapshots"),
            0
        );
        assert_eq!(
            count_rows(&connection, "archaeology_rule_temporal_events"),
            0
        );
        assert!(cleanup_generations(
            &connection,
            cleanup_input(
                "job:publish",
                "owner:other",
                ArchaeologyCleanupMode::DryRun,
                0,
            ),
        )
        .is_err());
    }

    #[test]
    fn cleanup_is_bounded_and_never_removes_the_ready_generation() {
        let connection = fixture();
        start(&connection, "job:publish", "generation:publish", OWNER);
        advance_to_publish(&connection, "job:publish", "generation:publish");
        publish(&connection, "job:publish", "generation:publish");
        for index in 0..=MAX_CLEANUP_GENERATIONS {
            seed_superseded(
                &connection,
                &format!("generation:obsolete:{index:03}"),
                &format!("2020-01-01T00:{:02}:00Z", index % 60),
            );
        }
        let first = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::Apply, 0),
        )
        .unwrap();
        assert_eq!(first.candidates.len(), MAX_CLEANUP_GENERATIONS);
        assert!(first.truncated);
        let second = cleanup_generations(
            &connection,
            cleanup_input("job:publish", OWNER, ArchaeologyCleanupMode::Apply, 0),
        )
        .unwrap();
        assert_eq!(second.deleted_generations, 2);
        assert!(!second.truncated);
        assert_ready_untouched_after_publish(&connection, "generation:publish");
    }

    #[test]
    fn failed_generation_cleanup_requires_its_terminal_job_owner() {
        let connection = fixture();
        start(&connection, "job:failed", "generation:failed", OWNER);
        fail_job(
            &connection,
            "job:failed",
            OWNER,
            ArchaeologyJobErrorCode::ParserFailed,
            T1,
        )
        .unwrap();
        seed_superseded(&connection, "generation:not-leased", "2020-01-01T00:00:00Z");
        let wrong_owner = cleanup_input(
            "job:failed",
            "owner:other",
            ArchaeologyCleanupMode::DryRun,
            1,
        );
        assert!(cleanup_generations(&connection, wrong_owner).is_err());
        let cleaned = cleanup_generations(
            &connection,
            cleanup_input("job:failed", OWNER, ArchaeologyCleanupMode::Apply, 0),
        )
        .unwrap();
        assert_eq!(cleaned.deleted_generations, 1);
        assert_eq!(
            generation_status(&connection, "generation:not-leased"),
            "superseded"
        );
        assert_ready_untouched(&connection);
    }

    fn link_fixture(job: &str, generation: &str, ambiguous: bool) -> Connection {
        let connection = fixture();
        start(&connection, job, generation, OWNER);
        checkpoint(
            &connection,
            job,
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            1,
        );
        checkpoint(
            &connection,
            job,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Link,
            2,
        );
        let lineage = serde_json::to_string(&vec![ArchaeologyAdapterLineage {
            kind: ArchaeologyLineageKind::Copybook,
            source_unit_id: "unit:main".into(),
            target_source_unit_id: None,
            evidence_span_id: "span:include".into(),
            detail: "unresolved include target".into(),
        }])
        .unwrap();
        for (id, path, lineage) in [
            ("unit:main", "src/main.cbl", lineage.as_str()),
            ("unit:copy", "copybooks/ACCOUNT.cpy", "[]"),
            ("unit:target:a", "src/a.cbl", "[]"),
        ] {
            connection.execute("INSERT INTO archaeology_source_units
            (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,language,
             parser_id,parser_version,classification,byte_count,line_count,include_lineage_json)
            VALUES (?1,?2,?3,?4,?5,'sha256','cobol','parser:v1','1','source',100,10,?6)",
            params![generation,id,format!("path:{id}"),path,"a".repeat(64),lineage]).unwrap();
        }
        if ambiguous {
            connection.execute("INSERT INTO archaeology_source_units
            (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,language,
             parser_id,parser_version,classification,byte_count,line_count)
            VALUES (?1,'unit:target:b','path:target:b','src/b.cbl',?2,'sha256','cobol','parser:v1','1','source',100,10)",
            params![generation,"b".repeat(64)]).unwrap();
        }
        let mut facts = vec![
            (
                "fact:include",
                "include",
                "ACCOUNT",
                "unit:main",
                "span:include",
                serde_json::json!([{"key":"target","value":"ACCOUNT"}]).to_string(),
            ),
            (
                "fact:call",
                "call",
                "credentials",
                "unit:main",
                "span:call",
                serde_json::json!([{"key":"target","value":"PROCESS"}]).to_string(),
            ),
            (
                "fact:target:a",
                "entry_point",
                "PROCESS",
                "unit:target:a",
                "span:target:a",
                "[]".into(),
            ),
        ];
        if ambiguous {
            facts.push((
                "fact:target:b",
                "entry_point",
                "PROCESS",
                "unit:target:b",
                "span:target:b",
                "[]".into(),
            ));
        }
        for (ordinal, (fact, kind, label, unit, span, attributes)) in facts.into_iter().enumerate()
        {
            connection.execute("INSERT INTO archaeology_source_spans
                (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,start_line,start_column,end_line,end_column)
                VALUES (?1,?2,?3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',?4,?5,1,?6,1,?7)",
                params![generation,span,unit,(ordinal*10) as i64,(ordinal*10+5) as i64,(ordinal*10+1) as i64,(ordinal*10+6) as i64]).unwrap();
            connection
                .execute(
                    "INSERT INTO archaeology_facts
                (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                VALUES (?1,?2,?3,?4,'parser:v1','extracted','high',?5)",
                    params![generation, fact, kind, label, attributes],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                VALUES (?1,'fact',?2,'span',?3,'supporting')",
                    params![generation, fact, span],
                )
                .unwrap();
        }
        connection
    }

    fn link_input<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        cancellation: &'a StructuralGraphCancellation,
        limits: ArchaeologyLinkLimits,
    ) -> ArchaeologyLinkStage<'a> {
        ArchaeologyLinkStage {
            job_id: job,
            repository_id: REPO,
            generation_id: generation,
            owner_id: owner,
            identity: generation_identity(generation),
            cancellation,
            limits,
            now: T1,
        }
    }

    fn derive_fixture(job: &str, generation: &str) -> Connection {
        let connection = fixture();
        start(&connection, job, generation, OWNER);
        for (current, next, completed) in [
            (
                ArchaeologyJobStage::Inventory,
                ArchaeologyJobStage::Parse,
                1,
            ),
            (ArchaeologyJobStage::Parse, ArchaeologyJobStage::Link, 2),
            (ArchaeologyJobStage::Link, ArchaeologyJobStage::Derive, 3),
        ] {
            checkpoint(&connection, job, current, next, completed);
        }
        connection
            .execute(
                "UPDATE archaeology_generations SET coverage_json=?2 WHERE generation_id=?1",
                params![generation, complete_coverage()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_source_units
              (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
               language,parser_id,parser_version,classification,byte_count,line_count,coverage_json)
             VALUES (?1,'unit:derive','path:derive','src/rules.cbl',?2,'sha256','cobol',
               'parser:v1','1','source',80,4,?3)",
                params![generation, "d".repeat(64), complete_coverage()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_source_units
              (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
               language,parser_id,parser_version,classification,byte_count,line_count,coverage_json)
             VALUES (?1,'unit:derive-generated','path:derive-generated','build/rules.generated.cbl',
               ?2,'sha256','cobol','parser:v1','1','generated',80,4,?3)",
                params![generation, "e".repeat(64), complete_coverage()],
            )
            .unwrap();
        for (span, unit, start) in [
            ("span:predicate", "unit:derive", 0_i64),
            ("span:field", "unit:derive", 20),
            ("span:generated:predicate", "unit:derive-generated", 0),
            ("span:generated:field", "unit:derive-generated", 20),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_source_spans
                  (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                   start_line,start_column,end_line,end_column)
                 VALUES (?1,?2,?3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                   ?4,?5,1,?6,1,?7)",
                    params![
                        generation,
                        span,
                        unit,
                        start,
                        start + 10,
                        start + 1,
                        start + 11
                    ],
                )
                .unwrap();
        }
        for (fact, kind, label, span, attributes) in [
            (
                "fact:predicate",
                "predicate",
                "ACCOUNT-ACTIVE",
                "span:predicate",
                serde_json::json!([
                    {"key":"credentials","value":"present"},
                    {"key":"semantic_expr","value":format!("v1:sha256:{}", "a".repeat(64))}
                ])
                .to_string(),
            ),
            (
                "fact:field",
                "data_field",
                "ACCOUNT-STATUS",
                "span:field",
                serde_json::json!([
                    {"key":"semantic_expr","value":format!("v1:sha256:{}", "b".repeat(64))}
                ])
                .to_string(),
            ),
            (
                "fact:generated:predicate",
                "predicate",
                "ACCOUNT-ACTIVE",
                "span:generated:predicate",
                serde_json::json!([
                    {"key":"credentials","value":"present"},
                    {"key":"semantic_expr","value":format!("v1:sha256:{}", "a".repeat(64))}
                ])
                .to_string(),
            ),
            (
                "fact:generated:field",
                "data_field",
                "ACCOUNT-STATUS",
                "span:generated:field",
                serde_json::json!([
                    {"key":"semantic_expr","value":format!("v1:sha256:{}", "b".repeat(64))}
                ])
                .to_string(),
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_facts
                  (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES (?1,?2,?3,?4,'parser:v1','extracted','high',?5)",
                    params![generation, fact, kind, label, attributes],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                  (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'fact',?2,'span',?3,'supporting')",
                    params![generation, fact, span],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO archaeology_fact_edges
              (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
             VALUES (?1,'edge:reads','fact:predicate','fact:field','reads','deterministic')",
                [generation],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_fact_edges
              (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
             VALUES (?1,'edge:generated:reads','fact:generated:predicate',
               'fact:generated:field','reads','deterministic')",
                [generation],
            )
            .unwrap();
        for (edge, span) in [
            ("edge:reads", "span:predicate"),
            ("edge:reads", "span:field"),
            ("edge:generated:reads", "span:generated:predicate"),
            ("edge:generated:reads", "span:generated:field"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                  (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'fact_edge',?2,'span',?3,'supporting')",
                    params![generation, edge, span],
                )
                .unwrap();
        }
        for (rule, title, lifecycle) in [
            ("rule:stale", "Stale generated candidate", "candidate"),
            ("rule:accepted", "Human-approved sentinel", "accepted"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_rules
                  (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                   confidence,parser_identity,algorithm_identity,created_at)
                 VALUES (?1,?2,?3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','validation',
                   ?4,?5,'deterministic','high',?6,'algorithm:v1',?7)",
                    params![
                        generation,
                        rule,
                        REPO,
                        title,
                        lifecycle,
                        PARSER_MANIFEST,
                        T0
                    ],
                )
                .unwrap();
            let clause = rule.replacen("rule:", "clause:", 1);
            connection
                .execute(
                    "INSERT INTO archaeology_rule_clauses
                  (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence)
                 VALUES (?1,?2,?3,0,?4,'deterministic','high')",
                    params![generation, rule, clause, format!("Clause for {title}")],
                )
                .unwrap();
            for (kind, evidence) in [("fact", "fact:predicate"), ("span", "span:predicate")] {
                connection
                    .execute(
                        "INSERT INTO archaeology_evidence_links
                      (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                     VALUES (?1,'rule_clause',?2,?3,?4,'supporting')",
                        params![generation, clause, kind, evidence],
                    )
                    .unwrap();
            }
        }
        connection
            .execute(
                "INSERT INTO archaeology_rule_search_manifest
              (generation_id,rule_id,title,clause_text,domain_text)
             VALUES (?1,'rule:stale','Stale generated candidate','stale','')",
                [generation],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_relations
              (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust,summary)
             VALUES (?1,'relation:stale','rule:stale','rule:accepted','depends_on',
               'deterministic','stale relation')",
                [generation],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
              (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'rule_relation','relation:stale','span','span:predicate','supporting')",
                [generation],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_review_events
              (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,
               body,evidence_identity,created_at)
             VALUES ('review:accepted',?1,'rule:accepted',?2,'accepted','reviewer:one',
               'approved','evidence:accepted',?3)",
                params![REPO, generation, T0],
            )
            .unwrap();
        connection
    }

    fn derive_input<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        cancellation: &'a StructuralGraphCancellation,
        limits: ArchaeologyDeterministicLimits,
    ) -> ArchaeologyDeriveStage<'a> {
        derive_input_at(job, generation, owner, cancellation, limits, REVISION)
    }

    fn remove_derive_retry_sentinel(connection: &Connection, generation: &str) {
        connection
            .execute(
                "DELETE FROM archaeology_evidence_links
                 WHERE generation_id=?1 AND (
                   (owner_kind='rule_clause' AND owner_id='clause:accepted')
                   OR (owner_kind='rule_relation' AND owner_id='relation:stale')
                   OR (owner_kind='fact' AND owner_id LIKE 'fact:generated:%')
                   OR (owner_kind='fact_edge' AND owner_id='edge:generated:reads')
                 )",
                [generation],
            )
            .expect("remove accepted retry evidence");
        connection
            .execute(
                "DELETE FROM archaeology_rules
                 WHERE generation_id=?1 AND rule_id='rule:accepted'",
                [generation],
            )
            .expect("remove accepted retry rule");
        connection
            .execute(
                "DELETE FROM archaeology_fact_edges
                 WHERE generation_id=?1 AND edge_id='edge:generated:reads'",
                [generation],
            )
            .expect("remove generated retry edge");
        connection
            .execute(
                "DELETE FROM archaeology_facts
                 WHERE generation_id=?1 AND fact_id LIKE 'fact:generated:%'",
                [generation],
            )
            .expect("remove generated retry facts");
        connection
            .execute(
                "DELETE FROM archaeology_source_spans
                 WHERE generation_id=?1 AND source_unit_id='unit:derive-generated'",
                [generation],
            )
            .expect("remove generated retry spans");
        connection
            .execute(
                "DELETE FROM archaeology_source_units
                 WHERE generation_id=?1 AND source_unit_id='unit:derive-generated'",
                [generation],
            )
            .expect("remove generated retry source");
        connection
            .execute_batch("DROP TRIGGER archaeology_review_events_no_delete")
            .expect("open append-only retry fixture");
        connection
            .execute(
                "DELETE FROM archaeology_rule_review_events
                 WHERE generation_id=?1 AND rule_id='rule:accepted'",
                [generation],
            )
            .expect("remove append-only retry fixture event");
        run_migration(connection).expect("restore append-only review trigger");
    }

    fn make_derive_sources_publishable(connection: &Connection, generation: &str) {
        let transaction = connection
            .unchecked_transaction()
            .expect("source transaction");
        transaction
            .execute_batch("PRAGMA defer_foreign_keys=ON")
            .expect("defer source identities");
        transaction
            .execute(
                "UPDATE archaeology_generations SET coverage_json=?2
                 WHERE generation_id=?1",
                params![generation, complete_generation_coverage(1, 80)],
            )
            .expect("publishable generation coverage");
        for (old_unit, seed) in [
            ("unit:derive", "source"),
            ("unit:derive-generated", "generated"),
        ] {
            let source_unit = opaque_test_id("archaeology-source-unit", seed);
            let path = opaque_test_id("archaeology-path", seed);
            let change = opaque_test_id("archaeology-change", seed);
            transaction
                .execute(
                    "UPDATE archaeology_source_units
                     SET source_unit_id=?3,path_identity=?4,change_identity=?5
                     WHERE generation_id=?1 AND source_unit_id=?2",
                    params![generation, old_unit, source_unit, path, change],
                )
                .expect("publishable source identity");
            transaction
                .execute(
                    "UPDATE archaeology_source_spans SET source_unit_id=?3
                     WHERE generation_id=?1 AND source_unit_id=?2",
                    params![generation, old_unit, source_unit],
                )
                .expect("publishable span source identity");
        }
        transaction
            .commit()
            .expect("publishable source transaction");
    }

    fn derive_input_at<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        cancellation: &'a StructuralGraphCancellation,
        limits: ArchaeologyDeterministicLimits,
        revision: &'a str,
    ) -> ArchaeologyDeriveStage<'a> {
        ArchaeologyDeriveStage {
            job_id: job,
            repository_id: REPO,
            generation_id: generation,
            owner_id: owner,
            identity: generation_identity_at(generation, revision),
            cancellation,
            limits,
            now: T1,
        }
    }

    fn persisted_cluster_rule(
        rule_id: &str,
        supporting_fact: &str,
        supporting_span: &str,
        contradicting_fact: &str,
        contradicting_span: &str,
        conflict_rule: &str,
    ) -> ArchaeologyRulePacket {
        let mut evidence_span_ids = vec![supporting_span.into(), contradicting_span.into()];
        evidence_span_ids.sort();
        ArchaeologyRulePacket {
            rule_id: rule_id.into(),
            repository_id: REPO.into(),
            generation_id: "generation:cluster-persist".into(),
            revision_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            kind: ArchaeologyRuleKind::Validation,
            title: format!("Cluster candidate {rule_id}"),
            domain_ids: vec!["domain:other".into()],
            lifecycle: ArchaeologyRuleLifecycle::Candidate,
            trust: ArchaeologyTrust::Deterministic,
            confidence: ArchaeologyConfidence::Low,
            clauses: vec![ArchaeologyRuleClause {
                clause_id: format!("clause:{rule_id}"),
                text: "Evidence-backed conflicting candidate".into(),
                trust: ArchaeologyTrust::Deterministic,
                confidence: ArchaeologyConfidence::Low,
                supporting_fact_ids: vec![supporting_fact.into()],
                contradicting_fact_ids: vec![contradicting_fact.into()],
                evidence_span_ids,
                caveats: vec!["packet has contradicting evidence".into()],
            }],
            dependency_rule_ids: vec![],
            conflict_rule_ids: vec![conflict_rule.into()],
            alias_rule_ids: vec![],
            coverage: Default::default(),
            parser_identity: PARSER_MANIFEST.into(),
            algorithm_identity: "algorithm:v1".into(),
            synthesis_identity: None,
        }
    }

    fn derived_catalog_snapshot(connection: &Connection, generation: &str) -> Vec<String> {
        [
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('rule_id',rule_id,'kind',kind,'title',title,
                 'lifecycle',lifecycle,'trust',trust,'confidence',confidence,
                 'parser',parser_identity,'algorithm',algorithm_identity,
                 'synthesis',synthesis_identity,'coverage',json(coverage_json)) value
               FROM archaeology_rules WHERE generation_id=?1
               ORDER BY rule_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('rule_id',rule_id,'clause_id',clause_id,'ordinal',ordinal,
                 'text',clause_text,'trust',trust,'confidence',confidence,
                 'caveats',json(caveats_json)) value
               FROM archaeology_rule_clauses WHERE generation_id=?1
               ORDER BY rule_id,ordinal,clause_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('owner_kind',owner_kind,'owner_id',owner_id,
                 'evidence_kind',evidence_kind,'evidence_id',evidence_id,'role',role) value
               FROM archaeology_evidence_links WHERE generation_id=?1
               ORDER BY owner_kind,owner_id,evidence_kind,evidence_id,role)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('rule_id',rule_id,'domain_id',domain_id,
                 'label',domain_label,'parent',parent_domain_id) value
               FROM archaeology_rule_domains WHERE generation_id=?1
               ORDER BY rule_id,domain_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('relation_id',relation_id,'from',from_rule_id,'to',to_rule_id,
                 'kind',kind,'trust',trust,'summary',summary) value
               FROM archaeology_rule_relations WHERE generation_id=?1
               ORDER BY relation_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
               SELECT json_object('rule_id',rule_id,'title',title,'clause',clause_text,
                 'domain',domain_text) value
               FROM archaeology_rule_search_manifest WHERE generation_id=?1
               ORDER BY rule_id)",
        ]
        .into_iter()
        .map(|query| {
            connection
                .query_row(query, [generation], |row| row.get::<_, String>(0))
                .unwrap()
        })
        .collect()
    }

    fn invalidation_inputs(revision: &str) -> Vec<ArchaeologyGenerationInput> {
        vec![
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Head,
                scope: None,
                identity: revision.into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Ignore,
                scope: None,
                identity: "ignore:v1".into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Config,
                scope: None,
                identity: "config:v1".into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Parser,
                scope: Some("global".into()),
                identity: PARSER_MANIFEST.into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Schema,
                scope: None,
                identity: "schema:v2".into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::Algorithm,
                scope: None,
                identity: "algorithm:v1".into(),
            },
            ArchaeologyGenerationInput {
                kind: ArchaeologyGenerationInputKind::SynthesisPolicy,
                scope: Some("global".into()),
                identity: "synthesis:v1".into(),
            },
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn incremental_inventory_unit(
        source_unit_id: &str,
        path_identity: &str,
        relative_path: &str,
        hash: char,
        classification: ArchaeologySourceClassification,
        change_identity: String,
        revision: &str,
    ) -> ArchaeologyInventoryUnit {
        ArchaeologyInventoryUnit {
            identity: ArchaeologySourceUnitIdentity {
                source_unit_id: source_unit_id.into(),
                repository_id: REPO.into(),
                revision_sha: revision.into(),
                path_identity: path_identity.into(),
                relative_path: Some(relative_path.into()),
                content_hash: Some(hash.to_string().repeat(64)),
                hash_algorithm: Some("sha256".into()),
                change_identity: Some(change_identity),
            },
            classification,
            language: "cobol".into(),
            dialect: None,
            byte_count: 80,
            line_count: 4,
            include_candidates: Vec::new(),
            coverage_reasons: Vec::new(),
        }
    }

    fn derived_catalog_counts(connection: &Connection) -> (i64, i64, i64, i64) {
        connection
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM archaeology_rules
                    WHERE generation_id='generation:derive' AND lifecycle='candidate'),
                   (SELECT COUNT(*) FROM archaeology_rule_clauses
                    WHERE generation_id='generation:derive'),
                   (SELECT COUNT(*) FROM archaeology_rule_relations
                    WHERE generation_id='generation:derive'),
                   (SELECT COUNT(*) FROM archaeology_rule_domains
                    WHERE generation_id='generation:derive')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }

    fn count_where(connection: &Connection, table: &str, predicate: &str) -> i64 {
        connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        run_migration(&connection).unwrap();
        crate::db::history_graph_schema::run_migration(&connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO archaeology_repositories (
                    repository_id, repo_path, source_identity, current_revision,
                    ready_generation_id, created_at, updated_at
                 ) VALUES (
                    'repo:jobs', '/fixture', 'source:ready',
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    'generation:ready', '2026-01-01T00:00:00.000Z',
                    '2026-01-01T00:00:00.000Z'
                 );
                 INSERT INTO archaeology_generations (
                    generation_id, repository_id, schema_version, revision_sha,
                    source_identity, parser_identity, algorithm_identity,
                    config_identity, status, created_at, published_at
                 ) VALUES (
                    'generation:ready', 'repo:jobs', 1,
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'source:ready',
                    'parser:ready', 'algorithm:ready', 'config:ready',
                    'ready', '2026-01-01T00:00:00.000Z',
                    '2026-01-01T00:00:00.000Z'
                 );
                 CREATE TABLE unrelated_codevetter_settings (
                    setting_id TEXT PRIMARY KEY,
                    setting_value TEXT NOT NULL
                 );
                 INSERT INTO unrelated_codevetter_settings
                    (setting_id, setting_value)
                 VALUES ('provider-account', 'credential-sentinel-unchanged');",
            )
            .unwrap();
        connection
    }

    fn new_job<'a>(job: &'a str, generation: &'a str, owner: &'a str) -> NewArchaeologyJob<'a> {
        new_job_at(job, generation, owner, REVISION)
    }

    fn new_job_at<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        revision_sha: &'a str,
    ) -> NewArchaeologyJob<'a> {
        NewArchaeologyJob {
            job_id: job,
            repository_id: REPO,
            generation_id: generation,
            owner_id: owner,
            identity: generation_identity_at(generation, revision_sha),
            total_units: Some(10),
            now: T0,
        }
    }

    fn start(connection: &Connection, job: &str, generation: &str, owner: &str) {
        start_at(connection, job, generation, owner, REVISION);
    }

    fn start_at(
        connection: &Connection,
        job: &str,
        generation: &str,
        owner: &str,
        revision_sha: &str,
    ) {
        set_repository_current_at(connection, generation, revision_sha);
        start_job(connection, new_job_at(job, generation, owner, revision_sha)).unwrap();
    }

    fn set_repository_current(connection: &Connection, source_identity: &str) {
        set_repository_current_at(connection, source_identity, REVISION);
    }

    fn set_repository_current_at(
        connection: &Connection,
        source_identity: &str,
        revision_sha: &str,
    ) {
        connection
            .execute(
                "UPDATE archaeology_repositories
                 SET current_revision = ?2, source_identity = ?3, updated_at = ?4
                 WHERE repository_id = ?1",
                params![REPO, revision_sha, source_identity, T0,],
            )
            .unwrap();
    }

    fn checkpoint(
        connection: &Connection,
        job: &str,
        current: ArchaeologyJobStage,
        next: ArchaeologyJobStage,
        completed: u64,
    ) -> ArchaeologyJobStatus {
        checkpoint_at(connection, job, current, next, completed, REVISION)
    }

    fn checkpoint_at(
        connection: &Connection,
        job: &str,
        current: ArchaeologyJobStage,
        next: ArchaeologyJobStage,
        completed: u64,
        revision_sha: &str,
    ) -> ArchaeologyJobStatus {
        if current == ArchaeologyJobStage::Synthesize && next == ArchaeologyJobStage::Validate {
            let generation = job_generation(connection, job);
            let cancellation = StructuralGraphCancellation::default();
            return finalize_synthesis_catalog(
                connection,
                synthesis_catalog_input_at(job, &generation, OWNER, revision_sha, &cancellation),
            )
            .unwrap();
        }
        checkpoint_job(
            connection,
            job,
            OWNER,
            current,
            next,
            &format!("checkpoint:{completed}"),
            &ArchaeologyJobCheckpoint {
                ordinal: Some(completed),
                counters: BTreeMap::from([(INVENTORY_COMPLETE_COUNTER.to_string(), 1)]),
                ..ArchaeologyJobCheckpoint::default()
            },
            completed,
            Some(10),
            T1,
        )
        .unwrap()
    }

    fn advance_to_publish(connection: &Connection, job: &str, generation: &str) {
        advance_to_validate(connection, job);
        seed_publishable_generation(connection, generation);
        validate_generation_for_publication(connection, publication(job, generation)).unwrap();
    }

    fn advance_to_validate(connection: &Connection, job: &str) {
        advance_to_validate_at(connection, job, REVISION);
    }

    fn advance_to_validate_at(connection: &Connection, job: &str, revision_sha: &str) {
        let stages = [
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Link,
            ArchaeologyJobStage::Derive,
            ArchaeologyJobStage::Synthesize,
            ArchaeologyJobStage::Validate,
        ];
        for index in 0..stages.len() - 1 {
            checkpoint_at(
                connection,
                job,
                stages[index].clone(),
                stages[index + 1].clone(),
                index as u64 + 1,
                revision_sha,
            );
        }
    }

    fn advance_empty_to_validate(connection: &Connection, job: &str) {
        let stages = [
            ArchaeologyJobStage::Inventory,
            ArchaeologyJobStage::Parse,
            ArchaeologyJobStage::Link,
            ArchaeologyJobStage::Derive,
            ArchaeologyJobStage::Synthesize,
        ];
        for index in 0..stages.len() - 1 {
            checkpoint_job(
                connection,
                job,
                OWNER,
                stages[index].clone(),
                stages[index + 1].clone(),
                &format!("checkpoint:empty:{index}"),
                &ArchaeologyJobCheckpoint {
                    counters: BTreeMap::from([(INVENTORY_COMPLETE_COUNTER.to_string(), 1)]),
                    ..ArchaeologyJobCheckpoint::default()
                },
                0,
                Some(0),
                T1,
            )
            .unwrap();
        }
        let generation = job_generation(connection, job);
        let cancellation = StructuralGraphCancellation::default();
        finalize_synthesis_catalog(
            connection,
            synthesis_catalog_input(job, &generation, OWNER, &cancellation),
        )
        .unwrap();
    }

    fn unavailable_coverage(reason: &str) -> String {
        coverage_json(ArchaeologyCoverageState::Unavailable, reason, 0, 0)
    }

    fn partial_coverage(reason: &str, discovered: u64, indexed: u64) -> String {
        coverage_json(
            ArchaeologyCoverageState::Partial,
            reason,
            discovered,
            indexed,
        )
    }

    fn coverage_json(
        state: ArchaeologyCoverageState,
        reason: &str,
        discovered: u64,
        indexed: u64,
    ) -> String {
        serde_json::to_string(&ArchaeologyCoverage {
            state: state.clone(),
            parser_coverage: state.clone(),
            repository_coverage: state.clone(),
            temporal_coverage: state,
            discovered_source_units: discovered,
            indexed_source_units: indexed,
            discovered_bytes: 0,
            indexed_bytes: 0,
            reasons: vec![reason.to_string()],
        })
        .unwrap()
    }

    fn complete_coverage() -> String {
        complete_generation_coverage(1, 80)
    }

    fn complete_generation_coverage(source_units: u64, bytes: u64) -> String {
        serde_json::to_string(&ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Complete,
            parser_coverage: ArchaeologyCoverageState::Complete,
            repository_coverage: ArchaeologyCoverageState::Complete,
            temporal_coverage: ArchaeologyCoverageState::Complete,
            discovered_source_units: source_units,
            indexed_source_units: source_units,
            discovered_bytes: bytes,
            indexed_bytes: bytes,
            reasons: Vec::new(),
        })
        .unwrap()
    }

    fn opaque_test_id(kind: &str, seed: &str) -> String {
        format!(
            "{kind}:{}",
            super::super::inventory::hex(&Sha256::digest(seed))
        )
    }

    fn seed_publishable_generation(connection: &Connection, generation: &str) {
        seed_publishable_generation_at(connection, generation, REVISION);
    }

    fn seed_publishable_generation_at(
        connection: &Connection,
        generation: &str,
        revision_sha: &str,
    ) {
        assert!(generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":-_".contains(&byte)));
        let coverage = complete_coverage().replace('\'', "''");
        let unit_id = opaque_test_id("archaeology-source-unit", generation);
        let path_id = opaque_test_id("archaeology-path", generation);
        connection.execute_batch(&format!("
            UPDATE archaeology_generations SET coverage_json='{coverage}'
            WHERE generation_id='{generation}' AND status='staging';
            INSERT INTO archaeology_source_units
              (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
               language,parser_id,parser_version,classification,byte_count,line_count,coverage_json)
            VALUES ('{generation}','{unit_id}','{path_id}','src/program.cbl',
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    'sha256','cobol','parser:v1','1','source',80,4,'{coverage}');
            INSERT INTO archaeology_source_spans
              (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
               start_line,start_column,end_line,end_column)
            VALUES ('{generation}','span:{generation}','{unit_id}',
                    '{revision_sha}',0,20,1,1,1,21);
            INSERT INTO archaeology_facts
              (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
            VALUES ('{generation}','fact:{generation}','predicate','AMOUNT > 0',
                    'parser:v1','extracted','high',
                    '[{{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}]');
            INSERT INTO archaeology_evidence_links
              (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
            VALUES ('{generation}','fact','fact:{generation}','span','span:{generation}','supporting');
            INSERT INTO archaeology_rules
              (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
               confidence,parser_identity,algorithm_identity,coverage_json,created_at)
            VALUES ('{generation}','rule:{generation}','{REPO}',
                    '{revision_sha}','validation','Positive amount',
                    'candidate','deterministic','high','{PARSER_MANIFEST}','algorithm:v1',
                    '{coverage}','{T0}');
            INSERT INTO archaeology_rule_clauses
              (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence)
            VALUES ('{generation}','rule:{generation}','clause:{generation}',0,
                    'Amount must be positive.','deterministic','high');
            INSERT INTO archaeology_evidence_links
              (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
            VALUES ('{generation}','rule_clause','clause:{generation}','fact',
                    'fact:{generation}','supporting'),
                   ('{generation}','rule_clause','clause:{generation}','span',
                    'span:{generation}','supporting');
            INSERT INTO archaeology_rule_domains
              (generation_id,rule_id,domain_id,domain_label,parent_domain_id)
            VALUES ('{generation}','rule:{generation}','domain:other','Other',NULL);
            INSERT INTO archaeology_rule_search_manifest
              (generation_id,rule_id,title,clause_text,domain_text)
            VALUES ('{generation}','rule:{generation}','Positive amount','Amount must be positive.','Other');
        ")).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        refresh_rule_identities(
            &transaction,
            generation,
            &[format!("rule:{generation}")],
            &StructuralGraphCancellation::default(),
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    fn seed_additional_publishable_rule(
        connection: &Connection,
        generation: &str,
        revision_sha: &str,
    ) {
        let coverage = complete_coverage().replace('\'', "''");
        let generation_coverage = complete_generation_coverage(2, 160).replace('\'', "''");
        let unit_id = opaque_test_id("archaeology-source-unit", "extra-rule");
        let path_id = opaque_test_id("archaeology-path", "extra-rule");
        connection
            .execute_batch(&format!(
                "UPDATE archaeology_generations SET coverage_json='{generation_coverage}'
                 WHERE generation_id='{generation}';
                 INSERT INTO archaeology_source_units
                   (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                    hash_algorithm,language,parser_id,parser_version,classification,
                    byte_count,line_count,coverage_json)
                 VALUES ('{generation}','{unit_id}','{path_id}','src/limit.cbl',
                    'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                    'sha256','cobol','parser:v1','1','source',80,4,'{coverage}');
                 INSERT INTO archaeology_source_spans
                   (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                    start_line,start_column,end_line,end_column)
                 VALUES ('{generation}','span:extra','{unit_id}','{revision_sha}',
                         0,20,1,1,1,21);
                 INSERT INTO archaeology_facts
                   (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES ('{generation}','fact:extra','predicate','AMOUNT < 1000',
                         'parser:v1','extracted','high',
                         '[{{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"}}]');
                 INSERT INTO archaeology_evidence_links
                   (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('{generation}','fact','fact:extra','span','span:extra','supporting');
                 INSERT INTO archaeology_rules
                   (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                    confidence,parser_identity,algorithm_identity,coverage_json,created_at)
                 VALUES ('{generation}','rule:extra','{REPO}','{revision_sha}','validation',
                         'Bounded amount','candidate','deterministic','high','{PARSER_MANIFEST}',
                         'algorithm:v1','{coverage}','{T0}');
                 INSERT INTO archaeology_rule_clauses
                   (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence)
                 VALUES ('{generation}','rule:extra','clause:extra',0,
                         'Amount must stay below 1000.','deterministic','high');
                 INSERT INTO archaeology_evidence_links
                   (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('{generation}','rule_clause','clause:extra','fact','fact:extra','supporting'),
                        ('{generation}','rule_clause','clause:extra','span','span:extra','supporting');
                 INSERT INTO archaeology_rule_domains
                   (generation_id,rule_id,domain_id,domain_label,parent_domain_id)
                 VALUES ('{generation}','rule:extra','domain:extra','Limits',NULL);
                 INSERT INTO archaeology_rule_search_manifest
                   (generation_id,rule_id,title,clause_text,domain_text)
                 VALUES ('{generation}','rule:extra','Bounded amount',
                         'Amount must stay below 1000.','Limits');"
            ))
            .unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        refresh_rule_identities(
            &transaction,
            generation,
            &["rule:extra".into()],
            &StructuralGraphCancellation::default(),
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    fn seed_exact_job_history(
        connection: &Connection,
        head: &str,
        parent: &str,
        ordinal: i64,
        tag: &str,
    ) {
        let coverage = r#"{"coverage_complete":true,"is_shallow":false,"truncated":false}"#;
        let release_coverage =
            r#"{"ancestry_complete":true,"is_shallow":false,"intervals_complete":true}"#;
        connection
            .execute(
                "INSERT INTO history_graph_repositories
                 (repo_path,repository_fingerprint,indexed_head,status,coverage_json,
                  created_at,updated_at)
                 VALUES ('/fixture','repo',?1,'ready',?2,'now','now')
                 ON CONFLICT(repo_path) DO UPDATE SET indexed_head=excluded.indexed_head,
                   status='ready',coverage_json=excluded.coverage_json,updated_at='now'",
                params![head, coverage],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO history_graph_revisions
                 (repo_path,sha,ordinal,committed_at,author_name,subject,parents_json)
                 VALUES ('/fixture',?1,?2,'now','Fixture','parent','[]')",
                params![parent, ordinal - 1],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO history_graph_revisions
                 (repo_path,sha,ordinal,committed_at,author_name,subject,parents_json)
                 VALUES ('/fixture',?1,?2,'now','Fixture','release',json_array(?3))
                 ON CONFLICT(repo_path,sha) DO UPDATE SET ordinal=excluded.ordinal,
                   parents_json=excluded.parents_json",
                params![head, ordinal, parent],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO history_graph_release_catalogs
                 (repo_path,index_identity,indexed_head,tags_fingerprint,status,coverage_json,
                  interval_schema_version,interval_identity,updated_at)
                 VALUES ('/fixture','catalog',?1,'tags','ready',?2,1,'intervals','now')
                 ON CONFLICT(repo_path) DO UPDATE SET indexed_head=excluded.indexed_head,
                   status='ready',coverage_json=excluded.coverage_json,
                   interval_schema_version=1,interval_identity='intervals',updated_at='now'",
                params![head, release_coverage],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO history_graph_fact_tags
                 (repo_path,tag,revision_sha,tag_object_sha,tag_kind,tagged_at)
                 VALUES ('/fixture',?1,?2,?2,'lightweight',1)",
                params![tag, head],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO history_graph_release_tags
                 (repo_path,tag,revision_sha,tag_object_sha,tag_kind,tagged_at)
                 VALUES ('/fixture',?1,?2,?2,'lightweight',1)",
                params![tag, head],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO history_graph_release_intervals
                 (repo_path,tag,revision_sha,from_exclusive_sha,commit_count,
                  observed_commit_count,coverage_kind)
                 VALUES ('/fixture',?1,?2,?3,1,1,'complete')",
                params![tag, head, parent],
            )
            .unwrap();
    }

    fn job_generation(connection: &Connection, job: &str) -> String {
        connection
            .query_row(
                "SELECT generation_id FROM archaeology_jobs WHERE job_id=?1",
                [job],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn synthesis_catalog_input<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        cancellation: &'a StructuralGraphCancellation,
    ) -> ArchaeologySynthesisCatalogStage<'a> {
        synthesis_catalog_input_at(job, generation, owner, REVISION, cancellation)
    }

    fn synthesis_catalog_input_at<'a>(
        job: &'a str,
        generation: &'a str,
        owner: &'a str,
        revision_sha: &'a str,
        cancellation: &'a StructuralGraphCancellation,
    ) -> ArchaeologySynthesisCatalogStage<'a> {
        ArchaeologySynthesisCatalogStage {
            job_id: job,
            repository_id: REPO,
            generation_id: generation,
            owner_id: owner,
            identity: generation_identity_at(generation, revision_sha),
            cancellation,
            now: T1,
        }
    }

    fn synthesis_catalog_fixture(name: &str) -> Connection {
        let connection = fixture();
        let job = format!("job:{name}");
        let generation = format!("generation:{name}");
        start(&connection, &job, &generation, OWNER);
        for (index, (current, next)) in [
            (ArchaeologyJobStage::Inventory, ArchaeologyJobStage::Parse),
            (ArchaeologyJobStage::Parse, ArchaeologyJobStage::Link),
            (ArchaeologyJobStage::Link, ArchaeologyJobStage::Derive),
            (ArchaeologyJobStage::Derive, ArchaeologyJobStage::Synthesize),
        ]
        .into_iter()
        .enumerate()
        {
            checkpoint_job(
                &connection,
                &job,
                OWNER,
                current,
                next,
                &format!("checkpoint:catalog:{index}"),
                &ArchaeologyJobCheckpoint {
                    counters: BTreeMap::from([(INVENTORY_COMPLETE_COUNTER.to_string(), 1)]),
                    ..Default::default()
                },
                index as u64 + 1,
                Some(10),
                T1,
            )
            .unwrap();
        }
        seed_publishable_generation(&connection, &generation);
        connection
            .execute(
                "DELETE FROM archaeology_rule_search_manifest WHERE generation_id=?1",
                [&generation],
            )
            .unwrap();
        connection
    }

    fn seed_model_rule(connection: &Connection, generation: &str) {
        let request_id = format!("sha256:{}", "1".repeat(64));
        let cache_key = format!("sha256:{}", "2".repeat(64));
        let evidence_identity = format!("sha256:{}", "3".repeat(64));
        let route_identity = format!("sha256:{}", "4".repeat(64));
        let prompt_identity = format!("sha256:{}", "5".repeat(64));
        let policy_identity = format!("sha256:{}", "6".repeat(64));
        let response = ArchaeologySynthesisResponse {
            schema_version: ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION,
            contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID.into(),
            request_id: request_id.clone(),
            packet_id: "packet:model".into(),
            clauses: vec![ArchaeologySynthesisClause {
                subject: ArchaeologySynthesisSegment {
                    text: "Amount".into(),
                    fact_ids: vec![format!("fact:{generation}")],
                },
                condition: None,
                action: ArchaeologySynthesisSegment {
                    text: "must remain positive".into(),
                    fact_ids: vec![format!("fact:{generation}")],
                },
                exception: None,
                quantifier: None,
                relationship_ids: Vec::new(),
                contradicting_fact_ids: Vec::new(),
            }],
        };
        let response_json = serde_json::to_string(&response).unwrap();
        let response_hash = sha256_identity(response_json.as_bytes());
        connection
            .execute(
                "INSERT INTO archaeology_synthesis_cache
                 (generation_id,cache_key,request_id,evidence_identity,packet_id,
                  provider_identity,provider_route_identity,model_identity,prompt_identity,
                  policy_identity,status,response_json,response_sha256,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,'packet:model','local-test',?5,'model:test',?6,?7,
                         'ready',?8,?9,?10,?10)",
                params![
                    generation,
                    cache_key,
                    request_id,
                    evidence_identity,
                    route_identity,
                    prompt_identity,
                    policy_identity,
                    response_json,
                    response_hash,
                    T0,
                ],
            )
            .unwrap();
        let coverage = complete_coverage();
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,synthesis_identity,coverage_json,
                  created_at)
                 VALUES (?1,'rule:model',?2,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                   'validation','Model-assisted positive amount','candidate','model_synthesized',
                   'high',?3,'algorithm:v1',?4,?5,?6)",
                params![generation, REPO, PARSER_MANIFEST, cache_key, coverage, T0],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES (?1,'rule:model','clause:model',0,'Amount must remain positive.',
                         'model_synthesized','high','[]')",
                [generation],
            )
            .unwrap();
        for (kind, evidence) in [
            ("fact", format!("fact:{generation}")),
            ("span", format!("span:{generation}")),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                     (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                     VALUES (?1,'rule_clause','clause:model',?2,?3,'supporting')",
                    params![generation, kind, evidence],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO archaeology_rule_domains
                 (generation_id,rule_id,domain_id,domain_label,parent_domain_id)
                 VALUES (?1,'rule:model','domain:model','Payments',NULL)",
                [generation],
            )
            .unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        let cancellation = StructuralGraphCancellation::default();
        assert_eq!(
            refresh_rule_identities(
                &transaction,
                generation,
                &["rule:model".to_string()],
                &cancellation,
            )
            .unwrap(),
            1
        );
        transaction.commit().unwrap();
    }

    fn generation_status(connection: &Connection, generation: &str) -> String {
        connection
            .query_row(
                "SELECT status FROM archaeology_generations WHERE generation_id = ?1",
                [generation],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn publication<'a>(job: &'a str, generation: &'a str) -> ArchaeologyPublication<'a> {
        publication_at(job, generation, REVISION)
    }

    fn publication_at<'a>(
        job: &'a str,
        generation: &'a str,
        revision_sha: &'a str,
    ) -> ArchaeologyPublication<'a> {
        ArchaeologyPublication {
            job_id: job,
            repository_id: REPO,
            generation_id: generation,
            owner_id: OWNER,
            identity: generation_identity_at(generation, revision_sha),
            now: T1,
        }
    }

    fn validation_error(connection: &Connection, job: &str, generation: &str) -> String {
        validate_generation_for_publication(connection, publication(job, generation)).unwrap_err()
    }

    fn publish(connection: &Connection, job: &str, generation: &str) -> ArchaeologyJobStatus {
        publish_generation(connection, publication(job, generation)).unwrap()
    }

    fn generation_identity(generation: &str) -> ArchaeologyGenerationIdentity<'_> {
        generation_identity_at(generation, REVISION)
    }

    fn generation_identity_at<'a>(
        generation: &'a str,
        revision_sha: &'a str,
    ) -> ArchaeologyGenerationIdentity<'a> {
        ArchaeologyGenerationIdentity {
            revision_sha,
            source: generation,
            parser: PARSER_MANIFEST,
            algorithm: "algorithm:v1",
            config: "config:v1",
        }
    }

    fn revision(value: char) -> String {
        value.to_string().repeat(40)
    }

    fn cleanup_input<'a>(
        job_id: &'a str,
        owner_id: &'a str,
        mode: ArchaeologyCleanupMode,
        retain_superseded: usize,
    ) -> ArchaeologyCleanup<'a> {
        ArchaeologyCleanup {
            job_id,
            owner_id,
            mode,
            retain_superseded,
            now: T1,
        }
    }

    fn seed_superseded(connection: &Connection, generation: &str, created_at: &str) {
        connection
            .execute(
                "INSERT INTO archaeology_generations (
                    generation_id, repository_id, schema_version, revision_sha,
                    source_identity, parser_identity, algorithm_identity,
                    config_identity, status, created_at, published_at
                 ) VALUES (?1, ?2, ?3,
                    'cccccccccccccccccccccccccccccccccccccccc', ?1,
                    'parser:old', 'algorithm:old', 'config:old',
                    'superseded', ?4, ?4)",
                params![
                    generation,
                    REPO,
                    ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
                    created_at
                ],
            )
            .unwrap();
    }

    fn ready_generation(connection: &Connection) -> String {
        connection
            .query_row(
                "SELECT ready_generation_id FROM archaeology_repositories
                 WHERE repository_id = ?1",
                [REPO],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn assert_unrelated_codevetter_data_untouched(connection: &Connection) {
        assert_eq!(
            connection
                .query_row(
                    "SELECT setting_value FROM unrelated_codevetter_settings
                     WHERE setting_id = 'provider-account'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("unrelated CodeVetter setting"),
            "credential-sentinel-unchanged"
        );
    }

    fn count_rows(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn assert_ready_untouched_after_publish(connection: &Connection, generation: &str) {
        assert_eq!(ready_generation(connection), generation);
        assert_eq!(generation_status(connection, generation), "ready");
    }

    fn assert_ready_untouched(connection: &Connection) {
        assert_eq!(generation_status(connection, READY), "ready");
        assert_eq!(
            connection
                .query_row(
                    "SELECT ready_generation_id FROM archaeology_repositories WHERE repository_id = ?1",
                    [REPO],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            READY
        );
    }
}
