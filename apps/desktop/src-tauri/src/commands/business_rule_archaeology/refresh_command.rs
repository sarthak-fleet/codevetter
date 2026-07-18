//! Strict desktop entrypoint for bounded local archaeology inventory refreshes.

use super::adapter::{
    run_archaeology_adapter, ArchaeologyAdapterEvents, ArchaeologyAdapterLimits,
    ArchaeologyAdapterOutcome, ArchaeologyAdapterOutput, ArchaeologyLanguageAdapter,
};
use super::assembly_adapter::AssemblyAdapter;
use super::cobol_adapter::CobolAdapter;
use super::contracts::{
    ArchaeologyCoverage, ArchaeologyCoverageState, ArchaeologyFact, ArchaeologyFactEdge,
    ArchaeologyJobStage, ArchaeologyJobState, ArchaeologyJobStatus,
    ArchaeologySourceClassification, ArchaeologySourceSpan, ArchaeologySourceUnitIdentity,
};
use super::evidence_store::insert_compact_evidence_json;
use super::invalidation::{ArchaeologyInputInvalidationMode, ArchaeologyInvalidationLimits};
use super::invalidation_store::{load_generation_inputs, persist_generation_invalidation_metadata};
use super::inventory::{ArchaeologyInventoryLimits, ArchaeologyInventoryUnit};
use super::jobs::{
    acknowledge_cancel, complete_job, derive_template_candidates, execute_incremental_parse_batch,
    finalize_synthesis_catalog, link_generation, load_job, publish_generation, request_cancel,
    resume_job, run_inventory_refresh, validate_generation_for_publication, ArchaeologyDeriveStage,
    ArchaeologyGenerationIdentity, ArchaeologyInventoryRefreshRun, ArchaeologyLinkStage,
    ArchaeologyPublication, ArchaeologySynthesisCatalogStage,
};
use super::modern_adapter::ModernLanguageAdapter;
use crate::commands::structural_graph::language::SupportedLanguage;
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use crate::DbState;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyRefreshCommandInput {
    repo_path: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchaeologyRefreshCommandResult {
    repository_generation_id: String,
    job_id: Option<String>,
    reused_ready_generation: bool,
    mode: &'static str,
    changed_path_count: usize,
    next_stage: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyRefreshContinueInput {
    job_id: String,
    #[serde(default = "default_max_steps")]
    max_steps: usize,
}

#[derive(Debug, Serialize)]
pub struct ArchaeologyRefreshLifecycleResult {
    job: ArchaeologyJobStatus,
    ready: bool,
}

fn default_max_steps() -> usize {
    8
}

fn open_archaeology_worker_connection(
    database: &Arc<Mutex<Connection>>,
) -> Result<Connection, String> {
    let database_path: String = {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        connection
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("Resolve archaeology database path: {error}"))?
    };
    if database_path.is_empty() {
        return Err("Archaeology worker requires a file-backed database".to_string());
    }
    let connection = Connection::open(&database_path)
        .map_err(|error| format!("Open archaeology worker database: {error}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("Configure archaeology worker timeout: {error}"))?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|error| format!("Configure archaeology worker database: {error}"))?;
    Ok(connection)
}

fn run_refresh(
    connection: &rusqlite::Connection,
    input: ArchaeologyRefreshCommandInput,
) -> Result<ArchaeologyRefreshCommandResult, String> {
    let repo_path = PathBuf::from(input.repo_path.trim());
    if input.repo_path.trim().is_empty() {
        return Err("Archaeology repository path is required".into());
    }
    let job_id = format!("archaeology-job:{}", uuid::Uuid::new_v4());
    let generation_id = format!("archaeology-generation:{}", uuid::Uuid::new_v4());
    let owner_id = format!("archaeology-owner:{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    let cancellation = StructuralGraphCancellation::default();
    let outcome = run_inventory_refresh(
        connection,
        ArchaeologyInventoryRefreshRun {
            job_id: &job_id,
            generation_id: &generation_id,
            owner_id: &owner_id,
            repository_root: &repo_path,
            inventory_limits: ArchaeologyInventoryLimits::default(),
            invalidation_limits: ArchaeologyInvalidationLimits::default(),
            cancellation: &cancellation,
            now: &now,
        },
    )?;
    Ok(ArchaeologyRefreshCommandResult {
        repository_generation_id: outcome.effective_generation_id,
        job_id: (!outcome.reused_ready_generation).then_some(job_id),
        reused_ready_generation: outcome.reused_ready_generation,
        mode: mode_name(outcome.mode),
        changed_path_count: outcome.changed_paths.len(),
        next_stage: stage_name(outcome.next_stage),
    })
}

#[tauri::command]
pub async fn refresh_business_rule_archaeology(
    db: State<'_, DbState>,
    input: ArchaeologyRefreshCommandInput,
) -> Result<ArchaeologyRefreshCommandResult, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = open_archaeology_worker_connection(&database)?;
        run_refresh(&connection, input)
    })
    .await
    .map_err(|error| format!("Archaeology refresh worker failed: {error}"))?
}

#[derive(Debug)]
struct RefreshContext {
    job_id: String,
    repository_id: String,
    generation_id: String,
    owner_id: String,
    revision_sha: String,
    source_identity: String,
    parser_identity: String,
    algorithm_identity: String,
    config_identity: String,
    repo_path: PathBuf,
}

impl RefreshContext {
    fn identity(&self) -> ArchaeologyGenerationIdentity<'_> {
        ArchaeologyGenerationIdentity {
            revision_sha: &self.revision_sha,
            source: &self.source_identity,
            parser: &self.parser_identity,
            algorithm: &self.algorithm_identity,
            config: &self.config_identity,
        }
    }
}

fn load_refresh_context(
    connection: &rusqlite::Connection,
    job_id: &str,
) -> Result<RefreshContext, String> {
    connection
        .query_row(
            "SELECT job.job_id,job.repository_id,job.generation_id,job.owner_id,
                    generation.revision_sha,generation.source_identity,generation.parser_identity,
                    generation.algorithm_identity,generation.config_identity,repository.repo_path
             FROM archaeology_jobs job
             JOIN archaeology_generations generation ON generation.generation_id=job.generation_id
             JOIN archaeology_repositories repository ON repository.repository_id=job.repository_id
             WHERE job.job_id=?1 AND generation.repository_id=job.repository_id",
            [job_id],
            |row| {
                Ok(RefreshContext {
                    job_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    generation_id: row.get(2)?,
                    owner_id: row.get(3)?,
                    revision_sha: row.get(4)?,
                    source_identity: row.get(5)?,
                    parser_identity: row.get(6)?,
                    algorithm_identity: row.get(7)?,
                    config_identity: row.get(8)?,
                    repo_path: PathBuf::from(row.get::<_, String>(9)?),
                })
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology refresh context: {error}"))?
        .ok_or_else(|| "Archaeology refresh job does not exist".to_string())
}

fn public_job(mut status: ArchaeologyJobStatus) -> ArchaeologyJobStatus {
    status.owner_id = None;
    status
}

fn lifecycle_result(
    connection: &rusqlite::Connection,
    job_id: &str,
) -> Result<ArchaeologyRefreshLifecycleResult, String> {
    let status = load_job(connection, job_id)?;
    let ready = status
        .generation_id
        .as_deref()
        .is_some_and(|generation_id| {
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM archaeology_generations
                 WHERE generation_id=?1 AND status='ready')",
                    [generation_id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false)
        });
    Ok(ArchaeologyRefreshLifecycleResult {
        job: public_job(status),
        ready,
    })
}

fn continue_refresh(
    connection: &rusqlite::Connection,
    input: ArchaeologyRefreshContinueInput,
) -> Result<ArchaeologyRefreshLifecycleResult, String> {
    if input.max_steps == 0 || input.max_steps > 64 {
        return Err("Archaeology refresh step bound must be between 1 and 64".into());
    }
    let context = load_refresh_context(connection, input.job_id.trim())?;
    let cancellation = StructuralGraphCancellation::default();
    let mut status = load_job(connection, &context.job_id)?;
    if status.state == ArchaeologyJobState::Paused {
        status = resume_job(
            connection,
            &context.job_id,
            &context.owner_id,
            &chrono::Utc::now().to_rfc3339(),
        )?;
    }
    for _ in 0..input.max_steps {
        if status.state == ArchaeologyJobState::Cancelling || status.cancellation_requested {
            acknowledge_cancel(
                connection,
                &context.job_id,
                &context.owner_id,
                &chrono::Utc::now().to_rfc3339(),
            )?;
            break;
        }
        if status.state != ArchaeologyJobState::Running {
            break;
        }
        let now = chrono::Utc::now().to_rfc3339();
        match status.stage {
            ArchaeologyJobStage::Parse => {
                let plan = status
                    .checkpoint_identity
                    .as_deref()
                    .ok_or("Archaeology parse job has no refresh plan")?;
                execute_incremental_parse_batch(
                    connection,
                    &context.job_id,
                    &context.repository_id,
                    &context.generation_id,
                    &context.owner_id,
                    plan,
                    32,
                    &now,
                    &cancellation,
                    |transaction, item| {
                        parse_refresh_item(transaction, item, &context, &cancellation)
                    },
                )?;
            }
            ArchaeologyJobStage::Link => {
                link_generation(
                    connection,
                    ArchaeologyLinkStage {
                        job_id: &context.job_id,
                        repository_id: &context.repository_id,
                        generation_id: &context.generation_id,
                        owner_id: &context.owner_id,
                        identity: context.identity(),
                        cancellation: &cancellation,
                        limits: Default::default(),
                        now: &now,
                    },
                )?;
            }
            ArchaeologyJobStage::Derive => {
                derive_template_candidates(
                    connection,
                    ArchaeologyDeriveStage {
                        job_id: &context.job_id,
                        repository_id: &context.repository_id,
                        generation_id: &context.generation_id,
                        owner_id: &context.owner_id,
                        identity: context.identity(),
                        cancellation: &cancellation,
                        limits: Default::default(),
                        now: &now,
                    },
                )?;
                let inputs = load_generation_inputs(
                    connection,
                    &context.repository_id,
                    &context.generation_id,
                )?;
                persist_generation_invalidation_metadata(
                    connection,
                    &context.repository_id,
                    &context.generation_id,
                    &inputs,
                    &cancellation,
                    ArchaeologyInvalidationLimits::default(),
                )?;
            }
            ArchaeologyJobStage::Synthesize => {
                finalize_synthesis_catalog(
                    connection,
                    ArchaeologySynthesisCatalogStage {
                        job_id: &context.job_id,
                        repository_id: &context.repository_id,
                        generation_id: &context.generation_id,
                        owner_id: &context.owner_id,
                        identity: context.identity(),
                        cancellation: &cancellation,
                        now: &now,
                    },
                )?;
            }
            ArchaeologyJobStage::Validate => {
                validate_generation_for_publication(
                    connection,
                    ArchaeologyPublication {
                        job_id: &context.job_id,
                        repository_id: &context.repository_id,
                        generation_id: &context.generation_id,
                        owner_id: &context.owner_id,
                        identity: context.identity(),
                        now: &now,
                    },
                )?;
            }
            ArchaeologyJobStage::Publish => {
                publish_generation(
                    connection,
                    ArchaeologyPublication {
                        job_id: &context.job_id,
                        repository_id: &context.repository_id,
                        generation_id: &context.generation_id,
                        owner_id: &context.owner_id,
                        identity: context.identity(),
                        now: &now,
                    },
                )?;
            }
            ArchaeologyJobStage::Cleanup => {
                complete_job(connection, &context.job_id, &context.owner_id, &now)?;
            }
            ArchaeologyJobStage::Inventory | ArchaeologyJobStage::Idle => break,
        }
        status = load_job(connection, &context.job_id)?;
    }
    lifecycle_result(connection, &context.job_id)
}

#[tauri::command]
pub async fn get_business_rule_archaeology_refresh_status(
    db: State<'_, DbState>,
    job_id: String,
) -> Result<ArchaeologyRefreshLifecycleResult, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        lifecycle_result(&connection, job_id.trim())
    })
    .await
    .map_err(|error| format!("Archaeology refresh status worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_current_business_rule_archaeology_refresh_status(
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<Option<ArchaeologyRefreshLifecycleResult>, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let canonical = Path::new(repo_path.trim())
            .canonicalize()
            .map_err(|_| "Archaeology repository is unavailable".to_string())?;
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        let job_id = connection
            .query_row(
                "SELECT job.job_id FROM archaeology_repositories repository
                 JOIN archaeology_jobs job ON job.repository_id=repository.repository_id
                 WHERE repository.repo_path=?1
                 ORDER BY job.state IN ('pending','running','paused','cancelling') DESC,
                          julianday(job.updated_at) DESC,job.job_id DESC LIMIT 1",
                [canonical.to_string_lossy().as_ref()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Load current archaeology refresh: {error}"))?;
        job_id
            .map(|job_id| lifecycle_result(&connection, &job_id))
            .transpose()
    })
    .await
    .map_err(|error| format!("Current archaeology refresh worker failed: {error}"))?
}

#[tauri::command]
pub async fn continue_business_rule_archaeology_refresh(
    db: State<'_, DbState>,
    input: ArchaeologyRefreshContinueInput,
) -> Result<ArchaeologyRefreshLifecycleResult, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = open_archaeology_worker_connection(&database)?;
        continue_refresh(&connection, input)
    })
    .await
    .map_err(|error| format!("Archaeology refresh continuation worker failed: {error}"))?
}

#[tauri::command]
pub async fn cancel_business_rule_archaeology_refresh(
    db: State<'_, DbState>,
    job_id: String,
) -> Result<ArchaeologyRefreshLifecycleResult, String> {
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        let context = load_refresh_context(&connection, job_id.trim())?;
        let now = chrono::Utc::now().to_rfc3339();
        let status = load_job(&connection, &context.job_id)?;
        if matches!(
            status.state,
            ArchaeologyJobState::Running | ArchaeologyJobState::Paused
        ) {
            request_cancel(&connection, &context.job_id, &context.owner_id, &now)?;
            acknowledge_cancel(&connection, &context.job_id, &context.owner_id, &now)?;
        }
        lifecycle_result(&connection, &context.job_id)
    })
    .await
    .map_err(|error| format!("Archaeology refresh cancellation worker failed: {error}"))?
}

#[derive(Default)]
struct PersistedAdapterOutput {
    active: bool,
    spans: Vec<ArchaeologySourceSpan>,
    facts: Vec<ArchaeologyFact>,
    edges: Vec<ArchaeologyFactEdge>,
    outcome: Option<ArchaeologyAdapterOutcome>,
}

impl ArchaeologyAdapterEvents for PersistedAdapterOutput {
    fn emit_span(&mut self, span: ArchaeologySourceSpan) -> Result<(), String> {
        self.spans.push(span);
        Ok(())
    }

    fn emit_fact(&mut self, fact: ArchaeologyFact) -> Result<(), String> {
        self.facts.push(fact);
        Ok(())
    }

    fn emit_edge(&mut self, edge: ArchaeologyFactEdge) -> Result<(), String> {
        self.edges.push(edge);
        Ok(())
    }
}

impl ArchaeologyAdapterOutput for PersistedAdapterOutput {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> {
        if self.active {
            return Err("Archaeology adapter output unit is already active".into());
        }
        self.active = true;
        Ok(())
    }

    fn commit_unit(&mut self, outcome: &ArchaeologyAdapterOutcome) -> Result<(), String> {
        if !self.active {
            return Err("Archaeology adapter output has no active unit".into());
        }
        self.outcome = Some(outcome.clone());
        self.active = false;
        Ok(())
    }

    fn abort_unit(&mut self) -> Result<(), String> {
        self.active = false;
        self.spans.clear();
        self.facts.clear();
        self.edges.clear();
        self.outcome = None;
        Ok(())
    }
}

fn parse_refresh_item(
    transaction: &Transaction<'_>,
    item: &super::invalidation_store::ArchaeologyRefreshWorkItem,
    context: &RefreshContext,
    cancellation: &StructuralGraphCancellation,
) -> Result<(), String> {
    if item.target_kind == "global" || item.action == "remove" {
        return Ok(());
    }
    if item.target_kind != "source_path" || item.action != "reprocess" {
        return Err("Archaeology parse work item is unsupported".into());
    }
    let row = transaction
        .query_row(
            "SELECT source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
                    change_identity,language,dialect,classification,byte_count,line_count
             FROM archaeology_source_units WHERE generation_id=?1 AND path_identity=?2",
            params![context.generation_id, item.target_identity],
            |row| {
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
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology parse unit: {error}"))?
        .ok_or("Archaeology parse work unit is unavailable")?;
    let classification = parse_classification(&row.8)?;
    let unit = ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: row.0,
            repository_id: context.repository_id.clone(),
            revision_sha: context.revision_sha.clone(),
            path_identity: row.1,
            relative_path: row.2,
            content_hash: row.3,
            hash_algorithm: row.4,
            change_identity: row.5,
        },
        classification: classification.clone(),
        language: row.6,
        dialect: row.7,
        byte_count: u64::try_from(row.9).map_err(|_| "Negative archaeology source bytes")?,
        line_count: u64::try_from(row.10).map_err(|_| "Negative archaeology source lines")?,
        include_candidates: Vec::new(),
        coverage_reasons: Vec::new(),
    };
    if matches!(
        classification,
        ArchaeologySourceClassification::Protected | ArchaeologySourceClassification::Opaque
    ) {
        return persist_unavailable_unit(transaction, context, &unit, "source_content_excluded");
    }
    let path = unit
        .identity
        .relative_path
        .as_deref()
        .ok_or("Archaeology parse unit has no repository-relative path")?;
    let source = git_blob(&context.repo_path, &context.revision_sha, path)?;
    let adapter = match adapter_for(&unit) {
        Ok(adapter) => adapter,
        Err(_) => {
            return persist_unavailable_unit(transaction, context, &unit, "parser_unavailable");
        }
    };
    let mut output = PersistedAdapterOutput::default();
    run_archaeology_adapter(
        adapter.as_ref(),
        super::adapter::ArchaeologyAdapterInput {
            unit: &unit,
            source: &source,
        },
        &mut output,
        cancellation,
        ArchaeologyAdapterLimits::default(),
    )?;
    persist_adapter_output(transaction, context, &unit, output)
}

fn adapter_for(
    unit: &ArchaeologyInventoryUnit,
) -> Result<Box<dyn ArchaeologyLanguageAdapter>, String> {
    match unit.language.as_str() {
        "cobol" => Ok(Box::new(CobolAdapter::default())),
        "assembly" => Ok(Box::new(AssemblyAdapter::default())),
        language => SupportedLanguage::ALL
            .into_iter()
            .find(|candidate| candidate.name() == language)
            .map(|language| {
                Box::new(ModernLanguageAdapter::new(language))
                    as Box<dyn ArchaeologyLanguageAdapter>
            })
            .ok_or_else(|| format!("Archaeology has no parser for language {language}")),
    }
}

fn git_blob(root: &Path, revision: &str, relative_path: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(["show", &format!("{revision}:{relative_path}")])
        .current_dir(root)
        .output()
        .map_err(|error| format!("Read archaeology Git blob: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err("Archaeology Git blob is unavailable at the inventoried revision".into())
    }
}

fn persist_unavailable_unit(
    transaction: &Transaction<'_>,
    context: &RefreshContext,
    unit: &ArchaeologyInventoryUnit,
    reason: &str,
) -> Result<(), String> {
    let coverage = ArchaeologyCoverage {
        state: ArchaeologyCoverageState::Unavailable,
        parser_coverage: ArchaeologyCoverageState::Unavailable,
        repository_coverage: ArchaeologyCoverageState::Complete,
        temporal_coverage: ArchaeologyCoverageState::Unavailable,
        discovered_source_units: 1,
        discovered_bytes: unit.byte_count,
        reasons: vec![reason.into()],
        ..Default::default()
    };
    let changed = transaction
        .execute(
            "UPDATE archaeology_source_units SET parser_id='unavailable',parser_version='unavailable',
             coverage_json=?3,include_lineage_json='[]',recovery_json='[]'
             WHERE generation_id=?1 AND source_unit_id=?2",
            params![
                context.generation_id,
                unit.identity.source_unit_id,
                serialize_unit_coverage(&coverage, unit)?
            ],
        )
        .map_err(|error| format!("Persist unavailable archaeology unit: {error}"))?;
    if changed != 1 {
        return Err("Archaeology parse unit lost its generation scope".into());
    }
    Ok(())
}

/// Preserve inventory-time exclusions after parser metadata is written. A
/// later delta refresh may reuse unchanged rows only when this proof exists.
fn serialize_unit_coverage(
    coverage: &ArchaeologyCoverage,
    unit: &ArchaeologyInventoryUnit,
) -> Result<String, String> {
    let mut value = serde_json::to_value(coverage).map_err(|error| error.to_string())?;
    value["inventory_reasons"] = serde_json::json!(unit.coverage_reasons);
    serde_json::to_string(&value).map_err(|error| error.to_string())
}

fn persist_adapter_output(
    transaction: &Transaction<'_>,
    context: &RefreshContext,
    unit: &ArchaeologyInventoryUnit,
    output: PersistedAdapterOutput,
) -> Result<(), String> {
    let outcome = output
        .outcome
        .ok_or("Archaeology adapter did not commit its output")?;
    let (parser_id, parser_version) = outcome
        .parser_identity
        .rsplit_once('@')
        .ok_or("Archaeology adapter parser identity is malformed")?;
    let coverage = ArchaeologyCoverage {
        state: if outcome.metadata.coverage_reasons.is_empty() {
            ArchaeologyCoverageState::Complete
        } else {
            ArchaeologyCoverageState::Partial
        },
        parser_coverage: if outcome.metadata.coverage_reasons.is_empty() {
            ArchaeologyCoverageState::Complete
        } else {
            ArchaeologyCoverageState::Partial
        },
        repository_coverage: ArchaeologyCoverageState::Complete,
        temporal_coverage: ArchaeologyCoverageState::Unavailable,
        discovered_source_units: 1,
        indexed_source_units: 1,
        discovered_bytes: unit.byte_count,
        indexed_bytes: unit.byte_count,
        reasons: outcome.metadata.coverage_reasons.clone(),
    };
    transaction
        .execute(
            "UPDATE archaeology_source_units SET dialect=?3,parser_id=?4,parser_version=?5,
         include_lineage_json=?6,recovery_json=?7,coverage_json=?8
         WHERE generation_id=?1 AND source_unit_id=?2",
            params![
                context.generation_id,
                unit.identity.source_unit_id,
                outcome.metadata.dialect,
                parser_id,
                parser_version,
                serde_json::to_string(&outcome.metadata.lineage)
                    .map_err(|error| error.to_string())?,
                serde_json::to_string(&outcome.metadata.regions)
                    .map_err(|error| error.to_string())?,
                serialize_unit_coverage(&coverage, unit)?
            ],
        )
        .map_err(|error| format!("Persist archaeology adapter metadata: {error}"))?;
    for span in &output.spans {
        transaction
            .execute(
                "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    context.generation_id,
                    span.span_id,
                    span.source_unit_id,
                    span.revision_sha,
                    i64::try_from(span.start.byte)
                        .map_err(|_| "Archaeology span offset overflowed")?,
                    i64::try_from(span.end.byte)
                        .map_err(|_| "Archaeology span offset overflowed")?,
                    i64::try_from(span.start.line)
                        .map_err(|_| "Archaeology span line overflowed")?,
                    i64::try_from(span.start.column)
                        .map_err(|_| "Archaeology span column overflowed")?,
                    i64::try_from(span.end.line).map_err(|_| "Archaeology span line overflowed")?,
                    i64::try_from(span.end.column)
                        .map_err(|_| "Archaeology span column overflowed")?
                ],
            )
            .map_err(|error| format!("Persist archaeology source span: {error}"))?;
    }
    let mut evidence = std::collections::BTreeSet::new();
    for fact in &output.facts {
        transaction
            .execute(
                "INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    context.generation_id,
                    fact.fact_id,
                    enum_name(&fact.kind)?,
                    fact.label,
                    fact.parser_id,
                    enum_name(&fact.trust)?,
                    enum_name(&fact.confidence)?,
                    serde_json::to_string(&fact.attributes).map_err(|error| error.to_string())?
                ],
            )
            .map_err(|error| format!("Persist archaeology fact: {error}"))?;
        for span_id in fact
            .span_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
        {
            evidence.insert((
                "fact",
                fact.fact_id.as_str(),
                "span",
                span_id.as_str(),
                "supporting",
            ));
        }
    }
    for edge in &output.edges {
        transaction
            .execute(
                "INSERT INTO archaeology_fact_edges
             (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust,unresolved_reason)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    context.generation_id,
                    edge.edge_id,
                    edge.from_fact_id,
                    edge.to_fact_id,
                    enum_name(&edge.kind)?,
                    enum_name(&edge.trust)?,
                    edge.unresolved_reason
                ],
            )
            .map_err(|error| format!("Persist archaeology fact edge: {error}"))?;
        for span_id in edge
            .evidence_span_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
        {
            evidence.insert((
                "fact_edge",
                edge.edge_id.as_str(),
                "span",
                span_id.as_str(),
                "supporting",
            ));
        }
    }
    let evidence_json = serde_json::to_string(&evidence).map_err(|error| error.to_string())?;
    insert_compact_evidence_json(transaction, &context.generation_id, &evidence_json, false)
        .map_err(|error| format!("Persist archaeology evidence: {error}"))?;
    Ok(())
}

fn enum_name<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| error.to_string())?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Archaeology enum serialization is invalid".to_string())
}

fn parse_classification(value: &str) -> Result<ArchaeologySourceClassification, String> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| "Stored archaeology source classification is invalid".into())
}

fn mode_name(mode: ArchaeologyInputInvalidationMode) -> &'static str {
    match mode {
        ArchaeologyInputInvalidationMode::NoOp => "no_op",
        ArchaeologyInputInvalidationMode::SynthesisOnly => "synthesis_only",
        ArchaeologyInputInvalidationMode::Scoped => "scoped",
        ArchaeologyInputInvalidationMode::GlobalRebuild => "global_rebuild",
    }
}

fn stage_name(stage: ArchaeologyJobStage) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn worker_connection_does_not_retain_the_shared_database_mutex() {
        let directory = tempdir().expect("database directory");
        let path = directory.path().join("codevetter.sqlite");
        let shared = Arc::new(Mutex::new(
            Connection::open(&path).expect("shared database"),
        ));
        shared
            .lock()
            .expect("shared connection")
            .execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES (1);")
            .expect("probe schema");

        let worker = open_archaeology_worker_connection(&shared).expect("worker connection");
        let _shared_guard = shared.lock().expect("shared mutex remains available");
        assert_eq!(
            worker
                .query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                .expect("worker read"),
            1
        );
    }

    #[test]
    fn production_entrypoint_reuses_noop_and_selects_changed_and_global_work() {
        let connection = rusqlite::Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        crate::db::history_graph_schema::run_migration(&connection).expect("history schema");
        let repository = tempdir().expect("repository");
        git(repository.path(), &["init", "-q"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(repository.path(), &["config", "user.name", "Test"]);
        std::fs::write(
            repository.path().join("rules.cbl"),
            "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. RULES.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       01 AMOUNT PIC 9(5).\n       PROCEDURE DIVISION.\n       MAIN.\n       IF AMOUNT > 100\n           MOVE 100 TO AMOUNT\n       END-IF.\n",
        )
        .expect("source");
        git(repository.path(), &["add", "rules.cbl"]);
        git(repository.path(), &["commit", "-qm", "initial"]);
        let command = || ArchaeologyRefreshCommandInput {
            repo_path: repository.path().to_string_lossy().into_owned(),
        };

        let initial = run_refresh(&connection, command()).expect("initial refresh");
        assert_eq!(initial.mode, "global_rebuild");
        let initial_job = initial.job_id.clone().expect("initial job");
        let initial_lifecycle = continue_refresh(
            &connection,
            ArchaeologyRefreshContinueInput {
                job_id: initial_job,
                max_steps: 64,
            },
        )
        .expect("publish initial refresh");
        assert!(initial_lifecycle.ready);
        assert_eq!(initial_lifecycle.job.state, ArchaeologyJobState::Completed);
        let ready = initial.repository_generation_id.clone();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_source_units
                     WHERE generation_id=?1 AND json_type(coverage_json,'$.inventory_reasons')='array'",
                    [&ready],
                    |row| row.get::<_, i64>(0),
                )
                .expect("inventory coverage proof"),
            1
        );
        connection
            .execute(
                "UPDATE archaeology_source_units SET dialect='adapter-normalized'
                 WHERE generation_id=?1",
                [&ready],
            )
            .expect("simulate adapter-resolved dialect metadata");
        let noop = run_refresh(&connection, command()).expect("no-op refresh");
        assert_eq!(noop.repository_generation_id, ready);
        assert!(noop.reused_ready_generation);
        assert_eq!(noop.job_id, None);
        assert_eq!(noop.next_stage, "idle");

        std::fs::write(
            repository.path().join("rules.cbl"),
            "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. RULES.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       01 AMOUNT PIC 9(5).\n       PROCEDURE DIVISION.\n       MAIN.\n       IF AMOUNT > 200\n           MOVE 200 TO AMOUNT\n       END-IF.\n",
        )
        .expect("changed source");
        git(repository.path(), &["add", "rules.cbl"]);
        git(repository.path(), &["commit", "-qm", "change"]);
        assert!(
            crate::commands::business_rule_archaeology::jobs::ready_delta_inventory(
                &connection,
                repository.path(),
                &StructuralGraphCancellation::default(),
                ArchaeologyInventoryLimits::default(),
            )
            .expect("delta eligibility")
            .is_some(),
            "a v2 ready generation with one source edit must take the Git-delta inventory path"
        );
        let changed = run_refresh(&connection, command()).expect("changed refresh");
        assert_eq!(changed.mode, "scoped");
        assert_eq!(changed.next_stage, "parse");
        assert_eq!(changed.changed_path_count, 1);
        let changed_lifecycle = continue_refresh(
            &connection,
            ArchaeologyRefreshContinueInput {
                job_id: changed.job_id.clone().expect("changed job"),
                max_steps: 64,
            },
        )
        .expect("publish changed refresh");
        assert!(changed_lifecycle.ready);
        assert_eq!(changed_lifecycle.job.state, ArchaeologyJobState::Completed);
        let changed_ready = changed.repository_generation_id;
        let clean = rusqlite::Connection::open_in_memory().expect("clean database");
        run_migration(&clean).expect("clean schema");
        crate::db::history_graph_schema::run_migration(&clean).expect("clean history schema");
        let clean_refresh = run_refresh(&clean, command()).expect("clean changed-head refresh");
        continue_refresh(
            &clean,
            ArchaeologyRefreshContinueInput {
                job_id: clean_refresh.job_id.clone().expect("clean job"),
                max_steps: 64,
            },
        )
        .expect("publish clean changed-head refresh");
        assert_eq!(
            catalog_snapshot(&connection, &changed_ready),
            catalog_snapshot(&clean, &clean_refresh.repository_generation_id),
            "incremental changed publication must match a clean build of the same revision"
        );

        connection
            .execute(
                "UPDATE archaeology_generations SET algorithm_identity='algorithm:v1' WHERE generation_id=?1",
                [&changed_ready],
            )
            .expect("install prior algorithm generation");
        connection
            .execute(
                "UPDATE archaeology_generation_inputs SET input_identity='algorithm:v1'
                 WHERE generation_id=?1 AND input_kind='algorithm'",
                [&changed_ready],
            )
            .expect("install prior algorithm input");
        let global = run_refresh(&connection, command()).expect("v1 upgrade refresh");
        assert_eq!(global.mode, "global_rebuild");
        assert_eq!(global.next_stage, "parse");
        assert!(!global.reused_ready_generation);
        assert_ne!(global.repository_generation_id, changed_ready);
        let global_lifecycle = continue_refresh(
            &connection,
            ArchaeologyRefreshContinueInput {
                job_id: global.job_id.expect("global job"),
                max_steps: 64,
            },
        )
        .expect("publish global refresh");
        assert!(global_lifecycle.ready);
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn catalog_snapshot(connection: &rusqlite::Connection, generation_id: &str) -> Vec<String> {
        [
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('id',fact_id,'kind',kind,'label',label,'parser',parser_id,
               'trust',trust,'confidence',confidence,'attributes',json(attributes_json)) value
             FROM archaeology_facts WHERE generation_id=?1 ORDER BY fact_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('id',edge_id,'from',from_fact_id,'to',to_fact_id,'kind',kind,
               'trust',trust,'unresolved',unresolved_reason) value
             FROM archaeology_fact_edges WHERE generation_id=?1 ORDER BY edge_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('id',rule_id,'kind',kind,'title',title,'lifecycle',lifecycle,
               'trust',trust,'confidence',confidence,'parser',parser_identity,
               'algorithm',algorithm_identity,'synthesis',synthesis_identity) value
             FROM archaeology_rules WHERE generation_id=?1 ORDER BY rule_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('rule',rule_id,'id',clause_id,'ordinal',ordinal,'text',clause_text,
               'trust',trust,'confidence',confidence,'caveats',json(caveats_json)) value
             FROM archaeology_rule_clauses WHERE generation_id=?1 ORDER BY rule_id,ordinal,clause_id)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('owner_kind',owner_kind,'owner',owner_id,'evidence_kind',evidence_kind,
               'evidence',evidence_id,'role',role) value FROM archaeology_evidence_links
             WHERE generation_id=?1 ORDER BY owner_kind,owner_id,evidence_kind,evidence_id,role)",
            "SELECT COALESCE(json_group_array(value),'[]') FROM (
             SELECT json_object('id',relation_id,'from',from_rule_id,'to',to_rule_id,'kind',kind,
               'trust',trust,'summary',summary) value FROM archaeology_rule_relations
             WHERE generation_id=?1 ORDER BY relation_id)",
        ]
        .into_iter()
        .map(|query| {
            connection
                .query_row(query, [generation_id], |row| row.get::<_, String>(0))
                .expect("catalog snapshot")
        })
        .collect()
    }
}

#[cfg(test)]
#[path = "qualification_benchmark.rs"]
mod qualification_benchmark;

#[cfg(test)]
#[path = "correctness_qualification.rs"]
mod correctness_qualification;
