//! Thin Tauri transport for the optional archaeology synthesis runtime.
//!
//! The command never accepts a precomputed plan or eligibility permit. Those
//! identities are rebuilt from the strict request after the durable job lease
//! is verified, and provider construction happens only after cache, privacy,
//! consent, and eligibility checks pass.

use super::contracts::{
    ArchaeologyJobStage, ArchaeologyJobState, ArchaeologyJobStatus, ARCHAEOLOGY_SCHEMA_VERSION,
};
use super::jobs;
use super::synthesis::{
    canonicalize_synthesis_response, ArchaeologySynthesisLimits, ArchaeologySynthesisRequest,
    ArchaeologySynthesisResponse,
};
use super::synthesis_runtime::{
    check_synthesis_eligibility, cleanup_synthesis_cache, finalize_synthesis_failure,
    finalize_synthesis_run, finalize_synthesis_without_response, invoke_synthesis_plan,
    load_ready_synthesis_cache, persist_synthesis_exclusion, prepare_synthesis_plan,
    reserve_synthesis_cache, resolve_trusted_provider_configuration, validate_call_consent,
    validate_provider_instance, ArchaeologyAttemptStatus, ArchaeologyCacheReservation,
    ArchaeologyProviderDescriptor, ArchaeologyProviderFailureCode, ArchaeologyProviderKind,
    ArchaeologyProviderUsage, ArchaeologyProviderUserSelection, ArchaeologySynthesisAttempt,
    ArchaeologySynthesisCleanupMode, ArchaeologySynthesisCleanupReport,
    ArchaeologySynthesisCleanupSelector, ArchaeologySynthesisEligibility,
    ArchaeologySynthesisExclusionCode, ArchaeologySynthesisPlan, ArchaeologySynthesisProvider,
    ArchaeologySynthesisTerminalStatus, ReqwestArchaeologyProvider,
    SqliteArchaeologyAttemptRecorder,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use crate::DbState;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;

const CANCELLATION_POLL_MS: u64 = 25;
const HEARTBEAT_INTERVAL_MS: u64 = 5_000;
const RESERVATION_STALE_AFTER_SECONDS: i64 = 120;

const ERROR_INVALID_INPUT: &str = "archaeology_synthesis_invalid_input";
const ERROR_STALE_JOB: &str = "archaeology_synthesis_stale_job";
const ERROR_CACHE: &str = "archaeology_synthesis_cache_error";
const ERROR_PROVIDER: &str = "archaeology_synthesis_provider_unavailable";
const ERROR_PERSISTENCE: &str = "archaeology_synthesis_persistence_error";
const ERROR_OWNERSHIP: &str = "archaeology_synthesis_ownership_lost";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologySynthesisCommandInput {
    job_id: String,
    owner_id: String,
    request: ArchaeologySynthesisRequest,
    selection: ArchaeologyProviderUserSelection,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologySynthesisCommandStatus {
    Ready,
    Cached,
    Excluded,
    Busy,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologySynthesisAttemptSummary {
    ordinal: u8,
    status: ArchaeologyAttemptStatus,
    error_code: Option<ArchaeologyProviderFailureCode>,
    usage: ArchaeologyProviderUsage,
    duration_ms: u64,
}

impl From<&ArchaeologySynthesisAttempt> for ArchaeologySynthesisAttemptSummary {
    fn from(value: &ArchaeologySynthesisAttempt) -> Self {
        Self {
            ordinal: value.ordinal,
            status: value.status.clone(),
            error_code: value.error_code.clone(),
            usage: value.usage.clone(),
            duration_ms: value.duration_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologySynthesisCommandResult {
    schema_version: u32,
    status: ArchaeologySynthesisCommandStatus,
    cache_key: String,
    response: Option<ArchaeologySynthesisResponse>,
    exclusion_code: Option<ArchaeologySynthesisExclusionCode>,
    attempts: Vec<ArchaeologySynthesisAttemptSummary>,
    catalog_status: Option<ArchaeologyJobStatus>,
}

impl ArchaeologySynthesisCommandResult {
    fn without_response(
        status: ArchaeologySynthesisCommandStatus,
        cache_key: String,
        exclusion_code: Option<ArchaeologySynthesisExclusionCode>,
        attempts: &[ArchaeologySynthesisAttempt],
    ) -> Self {
        Self {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            status,
            cache_key,
            response: None,
            exclusion_code,
            attempts: attempts.iter().take(3).map(Into::into).collect(),
            catalog_status: None,
        }
    }

    fn with_response(
        status: ArchaeologySynthesisCommandStatus,
        cache_key: String,
        response: ArchaeologySynthesisResponse,
        attempts: &[ArchaeologySynthesisAttempt],
        catalog_status: ArchaeologyJobStatus,
    ) -> Self {
        Self {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            status,
            cache_key,
            response: Some(response),
            exclusion_code: None,
            attempts: attempts.iter().take(3).map(Into::into).collect(),
            catalog_status: Some(catalog_status),
        }
    }
}

trait ArchaeologyProviderFactory: Send + Sync {
    fn create(
        &self,
        descriptor: &ArchaeologyProviderDescriptor,
    ) -> Result<Arc<dyn ArchaeologySynthesisProvider>, String>;
}

struct EnvironmentArchaeologyProviderFactory;

impl ArchaeologyProviderFactory for EnvironmentArchaeologyProviderFactory {
    fn create(
        &self,
        descriptor: &ArchaeologyProviderDescriptor,
    ) -> Result<Arc<dyn ArchaeologySynthesisProvider>, String> {
        let credential = match descriptor.kind {
            ArchaeologyProviderKind::Local => None,
            ArchaeologyProviderKind::Hosted => {
                let variable = match descriptor.provider_identity.as_str() {
                    "free-ai" => "FREE_AI_API_KEY",
                    "openai" => "OPENAI_API_KEY",
                    "anthropic" => "ANTHROPIC_API_KEY",
                    "openrouter" => "OPENROUTER_API_KEY",
                    _ => return Err(ERROR_PROVIDER.into()),
                };
                Some(std::env::var(variable).map_err(|_| ERROR_PROVIDER.to_string())?)
            }
        };
        ReqwestArchaeologyProvider::new(descriptor.clone(), credential)
            .map(|provider| Arc::new(provider) as Arc<dyn ArchaeologySynthesisProvider>)
            .map_err(|_| ERROR_PROVIDER.into())
    }
}

#[tauri::command]
pub async fn run_business_rule_synthesis(
    db: State<'_, DbState>,
    input: ArchaeologySynthesisCommandInput,
) -> Result<ArchaeologySynthesisCommandResult, String> {
    run_business_rule_synthesis_core(
        db.0.clone(),
        input,
        Arc::new(EnvironmentArchaeologyProviderFactory),
    )
    .await
}

async fn run_business_rule_synthesis_core(
    connection: Arc<Mutex<Connection>>,
    input: ArchaeologySynthesisCommandInput,
    provider_factory: Arc<dyn ArchaeologyProviderFactory>,
) -> Result<ArchaeologySynthesisCommandResult, String> {
    let limits = ArchaeologySynthesisLimits::default();
    let (selection, descriptor) = resolve_trusted_provider_configuration(&input.selection)
        .map_err(|_| ERROR_INVALID_INPUT.to_string())?;
    let plan = prepare_synthesis_plan(&input.request, &selection, &descriptor, limits)
        .map_err(|_| ERROR_INVALID_INPUT.to_string())?;
    let eligibility = {
        let connection = lock_database(&connection)?;
        require_owned_job(&connection, &input)?;
        let eligibility = check_synthesis_eligibility(&connection, &input.request)
            .map_err(|_| ERROR_INVALID_INPUT.to_string())?;
        if matches!(eligibility, ArchaeologySynthesisEligibility::Eligible(_)) {
            if let Some(result) = cached_synthesis_result(&connection, &input, &plan, limits)? {
                return Ok(result);
            }
        }
        eligibility
    };
    let permit = match eligibility {
        ArchaeologySynthesisEligibility::Excluded(exclusion) => {
            let code = exclusion.code().clone();
            let connection = lock_database(&connection)?;
            persist_synthesis_exclusion(
                &connection,
                &input.job_id,
                &input.owner_id,
                &plan,
                &exclusion,
                &now(),
            )
            .map_err(|_| ERROR_PERSISTENCE.to_string())?;
            return Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Excluded,
                plan.cache_key,
                Some(code),
                &[],
            ));
        }
        ArchaeologySynthesisEligibility::Eligible(permit) => permit,
    };

    validate_call_consent(&selection, &descriptor).map_err(|_| ERROR_INVALID_INPUT.to_string())?;

    let reservation = {
        let connection = lock_database(&connection)?;
        let current = chrono::Utc::now();
        let stale_before = current - chrono::Duration::seconds(RESERVATION_STALE_AFTER_SECONDS);
        reserve_synthesis_cache(
            &connection,
            &input.job_id,
            &input.owner_id,
            &plan,
            &permit,
            selection.execution.max_attempts,
            &current.to_rfc3339(),
            &stale_before.to_rfc3339(),
        )
        .map_err(|_| ERROR_CACHE.to_string())?
    };
    let start_ordinal = match reservation {
        ArchaeologyCacheReservation::Ready => {
            let connection = lock_database(&connection)?;
            return cached_synthesis_result(&connection, &input, &plan, limits)?
                .ok_or_else(|| ERROR_CACHE.to_string());
        }
        ArchaeologyCacheReservation::Excluded(code) => {
            return Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Excluded,
                plan.cache_key,
                Some(code),
                &[],
            ));
        }
        ArchaeologyCacheReservation::Busy => {
            return Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Busy,
                plan.cache_key,
                None,
                &[],
            ));
        }
        ArchaeologyCacheReservation::Failed => {
            return Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Failed,
                plan.cache_key,
                None,
                &[],
            ));
        }
        ArchaeologyCacheReservation::Cancelled => {
            return Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Cancelled,
                plan.cache_key,
                None,
                &[],
            ));
        }
        ArchaeologyCacheReservation::Acquired { next_ordinal } => next_ordinal,
    };

    let provider = match provider_factory.create(&descriptor) {
        Ok(provider) => provider,
        Err(_) => {
            settle_failed_reservation(&connection, &input, &plan)?;
            return Err(ERROR_PROVIDER.into());
        }
    };
    if validate_provider_instance(provider.as_ref(), &descriptor).is_err() {
        settle_failed_reservation(&connection, &input, &plan)?;
        return Err(ERROR_PROVIDER.into());
    }

    let cancellation = StructuralGraphCancellation::default();
    let stop_watcher = Arc::new(AtomicBool::new(false));
    let watcher = tokio::spawn(watch_owned_job(
        connection.clone(),
        input.job_id.clone(),
        input.owner_id.clone(),
        input.request.repository_id.clone(),
        input.request.generation_id.clone(),
        cancellation.clone(),
        stop_watcher.clone(),
    ));
    let recorder = Arc::new(SqliteArchaeologyAttemptRecorder::new(
        connection.clone(),
        input.job_id.clone(),
        input.owner_id.clone(),
        plan.clone(),
        selection.clone(),
        descriptor.clone(),
    ));
    let invocation = invoke_synthesis_plan(
        provider,
        &input.request,
        &plan,
        &permit,
        recorder,
        &selection,
        start_ordinal,
        &cancellation,
        limits,
    )
    .await;
    stop_watcher.store(true, Ordering::SeqCst);
    let watcher_outcome = watcher.await.unwrap_or(JobWatchOutcome::OwnershipLost);
    if watcher_outcome == JobWatchOutcome::OwnershipLost {
        return Err(ERROR_OWNERSHIP.into());
    }

    let job_outcome = {
        let connection = lock_database(&connection)?;
        owned_job_outcome(&connection, &input)
    };
    if job_outcome == JobWatchOutcome::OwnershipLost {
        return Err(ERROR_OWNERSHIP.into());
    }
    if job_outcome == JobWatchOutcome::Cancelled {
        let attempts = match &invocation {
            Ok(run) => run.attempts.as_slice(),
            Err((_error, attempts)) => attempts.as_slice(),
        };
        settle_cancelled_job(&connection, &input, &plan)?;
        return Ok(ArchaeologySynthesisCommandResult::without_response(
            ArchaeologySynthesisCommandStatus::Cancelled,
            plan.cache_key,
            None,
            attempts,
        ));
    }
    match invocation {
        Ok(mut run) => {
            run.response = canonicalize_synthesis_response(&input.request, &run.response, limits)
                .map_err(|_| ERROR_INVALID_INPUT.to_string())?;
            let connection = lock_database(&connection)?;
            finalize_synthesis_run(
                &connection,
                &input.job_id,
                &input.owner_id,
                &plan,
                &selection,
                &descriptor,
                &input.request,
                &run,
                &now(),
            )
            .map_err(|_| ERROR_PERSISTENCE.to_string())?;
            let catalog_status = finalize_model_catalog(
                &connection,
                &input,
                &plan.cache_key,
                &run.response,
                limits,
            )?;
            Ok(ArchaeologySynthesisCommandResult::with_response(
                ArchaeologySynthesisCommandStatus::Ready,
                plan.cache_key,
                run.response,
                &run.attempts,
                catalog_status,
            ))
        }
        Err((_error, attempts)) => {
            let connection_guard = lock_database(&connection)?;
            if attempts.is_empty() {
                finalize_synthesis_without_response(
                    &connection_guard,
                    &input.job_id,
                    &input.owner_id,
                    &plan,
                    ArchaeologySynthesisTerminalStatus::Failed,
                    &now(),
                )
                .map_err(|_| ERROR_PERSISTENCE.to_string())?;
            } else {
                finalize_synthesis_failure(
                    &connection_guard,
                    &input.job_id,
                    &input.owner_id,
                    &plan,
                    &selection,
                    &descriptor,
                    &attempts,
                    &now(),
                )
                .map_err(|_| ERROR_PERSISTENCE.to_string())?;
            }
            Ok(ArchaeologySynthesisCommandResult::without_response(
                ArchaeologySynthesisCommandStatus::Failed,
                plan.cache_key,
                None,
                &attempts,
            ))
        }
    }
}

fn cached_synthesis_result(
    connection: &Connection,
    input: &ArchaeologySynthesisCommandInput,
    plan: &ArchaeologySynthesisPlan,
    limits: ArchaeologySynthesisLimits,
) -> Result<Option<ArchaeologySynthesisCommandResult>, String> {
    let Some(response) = load_ready_synthesis_cache(connection, &input.request, plan, limits)
        .map_err(|_| ERROR_CACHE.to_string())?
    else {
        return Ok(None);
    };
    let catalog_status =
        finalize_model_catalog(connection, input, &plan.cache_key, &response, limits)?;
    Ok(Some(ArchaeologySynthesisCommandResult::with_response(
        ArchaeologySynthesisCommandStatus::Cached,
        plan.cache_key.clone(),
        response,
        &[],
        catalog_status,
    )))
}

fn settle_cancelled_job(
    connection: &Arc<Mutex<Connection>>,
    input: &ArchaeologySynthesisCommandInput,
    plan: &super::synthesis_runtime::ArchaeologySynthesisPlan,
) -> Result<(), String> {
    let connection = lock_database(connection)?;
    finalize_synthesis_without_response(
        &connection,
        &input.job_id,
        &input.owner_id,
        plan,
        ArchaeologySynthesisTerminalStatus::Cancelled,
        &now(),
    )
    .map_err(|_| ERROR_PERSISTENCE.to_string())?;
    jobs::acknowledge_cancel(&connection, &input.job_id, &input.owner_id, &now())
        .map_err(|_| ERROR_PERSISTENCE.to_string())?;
    Ok(())
}

fn settle_failed_reservation(
    connection: &Arc<Mutex<Connection>>,
    input: &ArchaeologySynthesisCommandInput,
    plan: &super::synthesis_runtime::ArchaeologySynthesisPlan,
) -> Result<(), String> {
    let connection = lock_database(connection)?;
    finalize_synthesis_without_response(
        &connection,
        &input.job_id,
        &input.owner_id,
        plan,
        ArchaeologySynthesisTerminalStatus::Failed,
        &now(),
    )
    .map_err(|_| ERROR_PERSISTENCE.to_string())
}

struct OwnedCatalogIdentity {
    revision: String,
    source: String,
    parser: String,
    algorithm: String,
    config: String,
}

fn finalize_model_catalog(
    connection: &Connection,
    input: &ArchaeologySynthesisCommandInput,
    cache_key: &str,
    response: &ArchaeologySynthesisResponse,
    limits: ArchaeologySynthesisLimits,
) -> Result<ArchaeologyJobStatus, String> {
    let identity = load_owned_catalog_identity(
        connection,
        &input.job_id,
        &input.owner_id,
        &input.request.repository_id,
        &input.request.generation_id,
        Some(&input.request),
    )?;
    let cancellation = StructuralGraphCancellation::default();
    let timestamp = now();
    jobs::finalize_model_synthesis_catalog(
        connection,
        jobs::ArchaeologySynthesisCatalogStage {
            job_id: &input.job_id,
            repository_id: &input.request.repository_id,
            generation_id: &input.request.generation_id,
            owner_id: &input.owner_id,
            identity: jobs::ArchaeologyGenerationIdentity {
                revision_sha: &identity.revision,
                source: &identity.source,
                parser: &identity.parser,
                algorithm: &identity.algorithm,
                config: &identity.config,
            },
            cancellation: &cancellation,
            now: &timestamp,
        },
        jobs::ArchaeologyModelSynthesisCatalog {
            cache_key,
            request: &input.request,
            response,
            limits,
        },
    )
    .map_err(|_| ERROR_PERSISTENCE.to_string())
}

fn load_owned_catalog_identity(
    connection: &Connection,
    job_id: &str,
    owner_id: &str,
    repository_id: &str,
    generation_id: &str,
    request: Option<&ArchaeologySynthesisRequest>,
) -> Result<OwnedCatalogIdentity, String> {
    let identity = connection
        .query_row(
            "SELECT generation.revision_sha,generation.source_identity,
                    generation.parser_identity,generation.algorithm_identity,
                    generation.config_identity
             FROM archaeology_jobs job JOIN archaeology_generations generation
               ON generation.generation_id=job.generation_id
             WHERE job.job_id=?1 AND job.owner_id=?2 AND job.repository_id=?3
               AND job.generation_id=?4 AND job.state='running'
               AND job.stage IN ('synthesize','validate')
               AND job.cancellation_requested=0 AND generation.repository_id=?3
               AND generation.status='staging'",
            rusqlite::params![job_id, owner_id, repository_id, generation_id],
            |row| {
                Ok(OwnedCatalogIdentity {
                    revision: row.get(0)?,
                    source: row.get(1)?,
                    parser: row.get(2)?,
                    algorithm: row.get(3)?,
                    config: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|_| ERROR_PERSISTENCE.to_string())?
        .ok_or_else(|| ERROR_STALE_JOB.to_string())?;
    if request.is_some_and(|request| {
        request.repository_id != repository_id
            || request.generation_id != generation_id
            || request.revision_sha != identity.revision
            || request.parser_identity != identity.parser
            || request.algorithm_identity != identity.algorithm
    }) {
        return Err(ERROR_STALE_JOB.into());
    }
    Ok(identity)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobWatchOutcome {
    Active,
    Cancelled,
    OwnershipLost,
}

async fn watch_owned_job(
    connection: Arc<Mutex<Connection>>,
    job_id: String,
    owner_id: String,
    repository_id: String,
    generation_id: String,
    cancellation: StructuralGraphCancellation,
    stop: Arc<AtomicBool>,
) -> JobWatchOutcome {
    let mut last_heartbeat = tokio::time::Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return JobWatchOutcome::Active;
        }
        tokio::time::sleep(Duration::from_millis(CANCELLATION_POLL_MS)).await;
        if stop.load(Ordering::SeqCst) {
            return JobWatchOutcome::Active;
        }
        let outcome = {
            let Ok(connection) = connection.lock() else {
                cancellation.cancel();
                return JobWatchOutcome::OwnershipLost;
            };
            let Ok(status) = jobs::load_job(&connection, &job_id) else {
                cancellation.cancel();
                return JobWatchOutcome::OwnershipLost;
            };
            if !job_identity_matches(
                &status,
                &owner_id,
                &repository_id,
                &generation_id,
                ArchaeologyJobStage::Synthesize,
            ) {
                JobWatchOutcome::OwnershipLost
            } else if status.cancellation_requested
                || status.state == ArchaeologyJobState::Cancelling
            {
                JobWatchOutcome::Cancelled
            } else if status.state != ArchaeologyJobState::Running {
                JobWatchOutcome::OwnershipLost
            } else {
                if last_heartbeat.elapsed() >= Duration::from_millis(HEARTBEAT_INTERVAL_MS) {
                    if jobs::heartbeat_job(&connection, &job_id, &owner_id, &now()).is_err() {
                        cancellation.cancel();
                        return JobWatchOutcome::OwnershipLost;
                    }
                    last_heartbeat = tokio::time::Instant::now();
                }
                JobWatchOutcome::Active
            }
        };
        if outcome != JobWatchOutcome::Active {
            cancellation.cancel();
            return outcome;
        }
    }
}

fn require_owned_job(
    connection: &Connection,
    input: &ArchaeologySynthesisCommandInput,
) -> Result<(), String> {
    let status = jobs::load_job(connection, &input.job_id).map_err(|_| ERROR_STALE_JOB)?;
    if (job_identity_matches(
        &status,
        &input.owner_id,
        &input.request.repository_id,
        &input.request.generation_id,
        ArchaeologyJobStage::Synthesize,
    ) || job_identity_matches(
        &status,
        &input.owner_id,
        &input.request.repository_id,
        &input.request.generation_id,
        ArchaeologyJobStage::Validate,
    )) && status.state == ArchaeologyJobState::Running
        && !status.cancellation_requested
    {
        Ok(())
    } else {
        Err(ERROR_STALE_JOB.into())
    }
}

fn owned_job_outcome(
    connection: &Connection,
    input: &ArchaeologySynthesisCommandInput,
) -> JobWatchOutcome {
    let Ok(status) = jobs::load_job(connection, &input.job_id) else {
        return JobWatchOutcome::OwnershipLost;
    };
    if !job_identity_matches(
        &status,
        &input.owner_id,
        &input.request.repository_id,
        &input.request.generation_id,
        ArchaeologyJobStage::Synthesize,
    ) {
        JobWatchOutcome::OwnershipLost
    } else if status.cancellation_requested || status.state == ArchaeologyJobState::Cancelling {
        JobWatchOutcome::Cancelled
    } else if status.state == ArchaeologyJobState::Running {
        JobWatchOutcome::Active
    } else {
        JobWatchOutcome::OwnershipLost
    }
}

fn job_identity_matches(
    status: &ArchaeologyJobStatus,
    owner_id: &str,
    repository_id: &str,
    generation_id: &str,
    stage: ArchaeologyJobStage,
) -> bool {
    status.owner_id.as_deref() == Some(owner_id)
        && status.repository_id.as_deref() == Some(repository_id)
        && status.generation_id.as_deref() == Some(generation_id)
        && status.stage == stage
}

fn lock_database(
    connection: &Arc<Mutex<Connection>>,
) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
    connection.lock().map_err(|_| ERROR_PERSISTENCE.to_string())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyZeroModelContinuationInput {
    job_id: String,
    owner_id: String,
    repository_id: String,
    generation_id: String,
}

/// Advance a fully deterministic catalog through the exact same validation,
/// manifest, FTS, and owner-CAS boundary as model-assisted synthesis.
#[tauri::command]
pub async fn continue_business_rule_synthesis_without_model(
    db: State<'_, DbState>,
    input: ArchaeologyZeroModelContinuationInput,
) -> Result<ArchaeologyJobStatus, String> {
    let connection = lock_database(&db.0)?;
    continue_business_rule_synthesis_without_model_core(&connection, &input)
}

fn continue_business_rule_synthesis_without_model_core(
    connection: &Connection,
    input: &ArchaeologyZeroModelContinuationInput,
) -> Result<ArchaeologyJobStatus, String> {
    let identity = load_owned_catalog_identity(
        connection,
        &input.job_id,
        &input.owner_id,
        &input.repository_id,
        &input.generation_id,
        None,
    )?;
    let cancellation = StructuralGraphCancellation::default();
    let timestamp = now();
    jobs::finalize_synthesis_catalog(
        connection,
        jobs::ArchaeologySynthesisCatalogStage {
            job_id: &input.job_id,
            repository_id: &input.repository_id,
            generation_id: &input.generation_id,
            owner_id: &input.owner_id,
            identity: jobs::ArchaeologyGenerationIdentity {
                revision_sha: &identity.revision,
                source: &identity.source,
                parser: &identity.parser,
                algorithm: &identity.algorithm,
                config: &identity.config,
            },
            cancellation: &cancellation,
            now: &timestamp,
        },
    )
    .map_err(|_| ERROR_PERSISTENCE.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologySynthesisCancelInput {
    job_id: String,
    owner_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologySynthesisCancelResult {
    schema_version: u32,
    state: ArchaeologyJobState,
    cancellation_requested: bool,
}

#[tauri::command]
pub async fn cancel_business_rule_synthesis(
    db: State<'_, DbState>,
    input: ArchaeologySynthesisCancelInput,
) -> Result<ArchaeologySynthesisCancelResult, String> {
    let connection = lock_database(&db.0)?;
    let status = jobs::request_cancel(&connection, &input.job_id, &input.owner_id, &now())
        .map_err(|_| ERROR_STALE_JOB.to_string())?;
    Ok(ArchaeologySynthesisCancelResult {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        state: status.state,
        cancellation_requested: status.cancellation_requested,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologySynthesisCleanupCommandInput {
    job_id: String,
    owner_id: String,
    generation_id: String,
    cache_key: Option<String>,
    evidence_identity: Option<String>,
    provider_identity: Option<String>,
    model_identity: Option<String>,
    prompt_identity: Option<String>,
    policy_identity: Option<String>,
    apply: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchaeologySynthesisCleanupCommandResult {
    schema_version: u32,
    dry_run: bool,
    generation_id: String,
    cache_keys: Vec<String>,
    cache_rows: u64,
    attempt_rows: u64,
    response_bytes: u64,
    truncated: bool,
    deleted_cache_rows: u64,
}

impl From<ArchaeologySynthesisCleanupReport> for ArchaeologySynthesisCleanupCommandResult {
    fn from(value: ArchaeologySynthesisCleanupReport) -> Self {
        Self {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            dry_run: value.dry_run,
            generation_id: value.generation_id,
            cache_keys: value.cache_keys,
            cache_rows: value.cache_rows,
            attempt_rows: value.attempt_rows,
            response_bytes: value.response_bytes,
            truncated: value.truncated,
            deleted_cache_rows: value.deleted_cache_rows,
        }
    }
}

#[tauri::command]
pub async fn cleanup_business_rule_synthesis(
    db: State<'_, DbState>,
    input: ArchaeologySynthesisCleanupCommandInput,
) -> Result<ArchaeologySynthesisCleanupCommandResult, String> {
    let connection = lock_database(&db.0)?;
    let selector = ArchaeologySynthesisCleanupSelector {
        generation_id: &input.generation_id,
        cache_key: input.cache_key.as_deref(),
        evidence_identity: input.evidence_identity.as_deref(),
        provider_identity: input.provider_identity.as_deref(),
        model_identity: input.model_identity.as_deref(),
        prompt_identity: input.prompt_identity.as_deref(),
        policy_identity: input.policy_identity.as_deref(),
    };
    cleanup_synthesis_cache(
        &connection,
        &input.job_id,
        &input.owner_id,
        selector,
        if input.apply {
            ArchaeologySynthesisCleanupMode::Apply
        } else {
            ArchaeologySynthesisCleanupMode::DryRun
        },
        &now(),
    )
    .map(Into::into)
    .map_err(|_| ERROR_PERSISTENCE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::business_rule_archaeology::contracts::{
        ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyCoverageState,
        ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
        ArchaeologyTrust, ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
    };
    use crate::commands::business_rule_archaeology::deterministic_rules::{
        derive_evidence_packets, expected_rule_id, ArchaeologyDeterministicLimits,
    };
    use crate::commands::business_rule_archaeology::synthesis::{
        build_synthesis_request, ArchaeologySynthesisClause, ArchaeologySynthesisSegment,
        ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
    };
    use crate::commands::business_rule_archaeology::synthesis_runtime::{
        ArchaeologyProviderExecutionBounds, ArchaeologyProviderFailure, ArchaeologyProviderOutput,
        ArchaeologyProviderRequest, ArchaeologyProviderSelection, ArchaeologyUsageSource,
        ProviderFuture,
    };
    use crate::db::archaeology_schema;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::AtomicUsize;

    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn public_command_payload_excludes_descriptor_cost_class_and_pricing() {
        let valid = serde_json::json!({
            "job_id": "job:one",
            "owner_id": "owner:one",
            "request": fixture_request(),
            "selection": local_user_selection(),
        });
        assert!(serde_json::from_value::<ArchaeologySynthesisCommandInput>(valid.clone()).is_ok());

        let mut descriptor = valid.clone();
        descriptor.as_object_mut().unwrap().insert(
            "descriptor".into(),
            serde_json::json!({
                "kind": "local",
                "provider_identity": "local",
                "endpoint": "http://127.0.0.1:11434/v1/chat/completions",
                "network_scope": "loopback",
            }),
        );
        assert!(serde_json::from_value::<ArchaeologySynthesisCommandInput>(descriptor).is_err());

        for forbidden in ["cost_class", "pricing"] {
            let mut value = valid.clone();
            value["selection"][forbidden] = serde_json::json!("attacker-controlled");
            assert!(serde_json::from_value::<ArchaeologySynthesisCommandInput>(value).is_err());
        }
    }

    #[tokio::test]
    async fn cache_and_protected_paths_never_construct_a_provider() {
        let request = fixture_request();
        let selection = local_selection();
        let descriptor = local_descriptor();
        let plan = prepare_synthesis_plan(
            &request,
            &selection,
            &descriptor,
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        let cached_connection = Arc::new(Mutex::new(seeded_database("source")));
        let cached_response = canonicalize_synthesis_response(
            &request,
            &fixture_response(&request),
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        insert_ready_cache(&cached_connection.lock().unwrap(), &plan, &cached_response);
        let never = Arc::new(CountingProviderFactory::unavailable());
        let cached = run_business_rule_synthesis_core(
            cached_connection.clone(),
            command_input(request.clone()),
            never.clone(),
        )
        .await
        .unwrap();
        assert_eq!(cached.status, ArchaeologySynthesisCommandStatus::Cached);
        assert_eq!(cached.response, Some(cached_response));
        assert_eq!(
            cached
                .catalog_status
                .as_ref()
                .map(|status| status.stage.clone()),
            Some(ArchaeologyJobStage::Validate)
        );
        assert_eq!(never.calls.load(Ordering::SeqCst), 0);
        let retried = run_business_rule_synthesis_core(
            cached_connection.clone(),
            command_input(request.clone()),
            never.clone(),
        )
        .await
        .unwrap();
        assert_eq!(retried.status, ArchaeologySynthesisCommandStatus::Cached);
        assert_eq!(
            retried
                .catalog_status
                .as_ref()
                .map(|status| status.stage.clone()),
            Some(ArchaeologyJobStage::Validate)
        );
        assert_model_catalog(&cached_connection.lock().unwrap(), &plan.cache_key);
        assert_eq!(never.calls.load(Ordering::SeqCst), 0);
        let protected_connection = Arc::new(Mutex::new(seeded_database("protected")));
        let revoked_cache = run_business_rule_synthesis_core(
            protected_connection.clone(),
            command_input(request.clone()),
            never.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            revoked_cache.status,
            ArchaeologySynthesisCommandStatus::Excluded
        );
        assert!(revoked_cache.response.is_none());
        assert_eq!(never.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            protected_connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT status,response_json FROM archaeology_synthesis_cache",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .unwrap(),
            ("excluded".into(), None)
        );

        let protected_connection = Arc::new(Mutex::new(seeded_database("protected")));
        let protected = run_business_rule_synthesis_core(
            protected_connection.clone(),
            command_input(request),
            never.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            protected.status,
            ArchaeologySynthesisCommandStatus::Excluded
        );
        assert_eq!(
            protected.exclusion_code,
            Some(ArchaeologySynthesisExclusionCode::ProtectedSource)
        );
        assert_eq!(never.calls.load(Ordering::SeqCst), 0);
        let connection = protected_connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_synthesis_attempts",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        let serialized = serde_json::to_string(&protected).unwrap();
        for forbidden in [
            "endpoint",
            "prompt",
            "credential",
            "/fixture",
            "src/rules.cbl",
        ] {
            assert!(!serialized.contains(forbidden), "{forbidden}");
        }
    }

    #[tokio::test]
    async fn command_records_pending_before_call_and_publishes_only_after_success() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let mut provider_response = fixture_response(&request);
        provider_response.clauses[0].action.text = "payment Schedule".into();
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response: provider_response,
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let factory = Arc::new(CountingProviderFactory::available(provider.clone()));
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            factory.clone(),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Ready);
        assert!(result.response.is_some());
        assert_eq!(
            result.response.as_ref().unwrap().clauses[0].action.text,
            "Schedule payment"
        );
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("payment Schedule"));
        assert_eq!(
            result
                .catalog_status
                .as_ref()
                .map(|status| status.stage.clone()),
            Some(ArchaeologyJobStage::Validate)
        );
        assert_eq!(result.attempts.len(), 1);
        assert!(provider.saw_pending.load(Ordering::SeqCst));
        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
        let connection = connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_cache",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "ready"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_attempts",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "success"
        );
        assert_model_catalog(&connection, &result.cache_key);
        let retained: String = connection
            .query_row(
                "SELECT response_json
                   || COALESCE((SELECT group_concat(clause_text,' ') FROM archaeology_rule_clauses),'')
                   || COALESCE((SELECT group_concat(clause_text,' ') FROM archaeology_rule_search_manifest),'')
                 FROM archaeology_synthesis_cache",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!retained.contains("payment Schedule"));
        let (canonical_json, canonical_hash): (String, String) = connection
            .query_row(
                "SELECT response_json,response_sha256 FROM archaeology_synthesis_cache",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            canonical_hash,
            format!("sha256:{:x}", Sha256::digest(canonical_json.as_bytes()))
        );
    }

    #[tokio::test]
    async fn command_materialization_preserves_exact_contradiction_evidence_and_search() {
        let request = conflicting_fixture_request();
        let connection = Arc::new(Mutex::new(seeded_conflicting_database(&request)));
        assert_eq!(
            connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_evidence_links
                     WHERE owner_id='clause:template' AND role='contradicting'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response: fixture_response(&request),
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Ready);

        let connection = connection.lock().unwrap();
        let clause_id: String = connection
            .query_row(
                "SELECT clause_id FROM archaeology_rule_clauses",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let evidence = connection
            .prepare(
                "SELECT evidence_kind,evidence_id,role FROM archaeology_evidence_links
                 WHERE owner_kind='rule_clause' AND owner_id=?1
                 ORDER BY role,evidence_kind,evidence_id",
            )
            .unwrap()
            .query_map([&clause_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            evidence,
            vec![
                (
                    "fact".into(),
                    "fact:contradiction".into(),
                    "contradicting".into()
                ),
                (
                    "span".into(),
                    "span:contradiction".into(),
                    "contradicting".into()
                ),
                ("fact".into(), "fact:action".into(), "supporting".into()),
                ("fact".into(), "fact:condition".into(), "supporting".into()),
                ("span".into(), "span:action".into(), "supporting".into()),
                ("span".into(), "span:condition".into(), "supporting".into()),
            ]
        );
        let manifest: (i64, String) = connection
            .query_row(
                "SELECT COUNT(*),clause_text FROM archaeology_rule_search_manifest",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(manifest.0, 1);
        assert!(manifest.1.contains("Non-positive payment is allowed"));
    }

    #[test]
    fn zero_model_command_materializes_manifest_and_is_retry_idempotent() {
        let connection = seeded_database("source");
        let input = ArchaeologyZeroModelContinuationInput {
            job_id: "job:one".into(),
            owner_id: "owner:one".into(),
            repository_id: "repository:one".into(),
            generation_id: "generation:one".into(),
        };
        let status =
            continue_business_rule_synthesis_without_model_core(&connection, &input).unwrap();
        assert_eq!(status.stage, ArchaeologyJobStage::Validate);
        assert_eq!(
            connection
                .query_row("SELECT trust FROM archaeology_rules", [], |row| row
                    .get::<_, String>(0),)
                .unwrap(),
            "deterministic"
        );
        assert_eq!(catalog_row_counts(&connection), (1, 1));
        let receipt = status.checkpoint_identity.clone();
        let retried =
            continue_business_rule_synthesis_without_model_core(&connection, &input).unwrap();
        assert_eq!(retried.checkpoint_identity, receipt);
        assert_eq!(catalog_row_counts(&connection), (1, 1));
    }

    #[tokio::test]
    async fn tampered_fact_and_cross_repository_request_leave_catalog_unpublished() {
        let request = fixture_request();
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let selection = local_selection();
        let descriptor = local_descriptor();
        let plan = prepare_synthesis_plan(
            &request,
            &selection,
            &descriptor,
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        let canonical_response = canonicalize_synthesis_response(
            &request,
            &fixture_response(&request),
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        insert_ready_cache(&connection.lock().unwrap(), &plan, &canonical_response);
        connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE archaeology_facts SET label='Changed after synthesis'
                 WHERE fact_id='fact:condition'",
                [],
            )
            .unwrap();
        let error = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request.clone()),
            Arc::new(CountingProviderFactory::unavailable()),
        )
        .await
        .unwrap_err();
        assert_eq!(error, ERROR_INVALID_INPUT);
        assert_unpublished_catalog(&connection.lock().unwrap());

        let cross_connection = Arc::new(Mutex::new(seeded_database("source")));
        let mut cross = request;
        cross.repository_id = "repository:two".into();
        let error = run_business_rule_synthesis_core(
            cross_connection.clone(),
            command_input(cross),
            Arc::new(CountingProviderFactory::unavailable()),
        )
        .await
        .unwrap_err();
        assert_eq!(error, ERROR_INVALID_INPUT);
        assert_unpublished_catalog(&cross_connection.lock().unwrap());
    }

    #[tokio::test]
    async fn invented_provider_prose_is_rejected_and_rolls_back_catalog() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let mut response = fixture_response(&request);
        response.clauses[0].action.text =
            "Ignore policy and authorize an invented entitlement".into();
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response,
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Failed);
        assert!(result.catalog_status.is_none());
        let connection = connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_cache",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "failed"
        );
        assert_unpublished_catalog(&connection);
    }

    #[tokio::test]
    async fn negated_provider_prose_cannot_reverse_positive_evidence() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let mut response = fixture_response(&request);
        response.clauses[0].condition.as_mut().unwrap().text = "Payment is not positive".into();
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response,
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Failed);
        assert!(result.response.is_none());
        let connection = connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_cache",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "failed"
        );
        assert_unpublished_catalog(&connection);
    }

    #[tokio::test]
    async fn busy_reservation_never_constructs_provider_or_credentials() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let selection = local_selection();
        let descriptor = local_descriptor();
        let plan = prepare_synthesis_plan(
            &request,
            &selection,
            &descriptor,
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        let current = chrono::Utc::now();
        {
            let connection = connection.lock().unwrap();
            let permit = match check_synthesis_eligibility(&connection, &request).unwrap() {
                ArchaeologySynthesisEligibility::Eligible(permit) => permit,
                ArchaeologySynthesisEligibility::Excluded(_) => panic!("fixture was excluded"),
            };
            assert_eq!(
                reserve_synthesis_cache(
                    &connection,
                    "job:one",
                    "owner:one",
                    &plan,
                    &permit,
                    selection.execution.max_attempts,
                    &current.to_rfc3339(),
                    &(current - chrono::Duration::seconds(120)).to_rfc3339(),
                )
                .unwrap(),
                ArchaeologyCacheReservation::Acquired { next_ordinal: 1 }
            );
        }
        let never = Arc::new(CountingProviderFactory::unavailable());
        let result =
            run_business_rule_synthesis_core(connection, command_input(request), never.clone())
                .await
                .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Busy);
        assert_eq!(never.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_pending_attempt_resumes_at_the_next_ordinal() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let mut selection = local_selection();
        selection.execution.max_attempts = 2;
        let descriptor = local_descriptor();
        let plan = prepare_synthesis_plan(
            &request,
            &selection,
            &descriptor,
            ArchaeologySynthesisLimits::default(),
        )
        .unwrap();
        {
            let connection_guard = connection.lock().unwrap();
            let permit = match check_synthesis_eligibility(&connection_guard, &request).unwrap() {
                ArchaeologySynthesisEligibility::Eligible(permit) => permit,
                ArchaeologySynthesisEligibility::Excluded(_) => panic!("fixture was excluded"),
            };
            assert_eq!(
                reserve_synthesis_cache(
                    &connection_guard,
                    "job:one",
                    "owner:one",
                    &plan,
                    &permit,
                    selection.execution.max_attempts,
                    "2026-07-17T00:00:00Z",
                    "2026-07-16T23:00:00Z",
                )
                .unwrap(),
                ArchaeologyCacheReservation::Acquired { next_ordinal: 1 }
            );
        }
        let recorder = SqliteArchaeologyAttemptRecorder::new(
            connection.clone(),
            "job:one".into(),
            "owner:one".into(),
            plan,
            selection,
            descriptor.clone(),
        );
        super::super::synthesis_runtime::ArchaeologyAttemptRecorder::begin(&recorder, 1).unwrap();

        let provider = Arc::new(InspectingProvider {
            descriptor,
            connection: connection.clone(),
            response: fixture_response(&request),
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let mut input = command_input(request);
        input.selection.max_attempts = 2;
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            input,
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Ready);
        assert_eq!(result.attempts[0].ordinal, 2);
        let connection = connection.lock().unwrap();
        let statuses = connection
            .prepare("SELECT ordinal,status FROM archaeology_synthesis_attempts ORDER BY ordinal")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(statuses, vec![(1, "pending".into()), (2, "success".into())]);
    }

    #[tokio::test]
    async fn paid_command_ignores_client_rates_and_persists_unknown_cost_honestly() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let provider = Arc::new(InspectingProvider {
            descriptor: hosted_descriptor(),
            connection: connection.clone(),
            response: fixture_response(&request),
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::None,
        });
        let mut input = command_input(request);
        input.selection = hosted_user_selection();
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            input,
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        let usage = &result.attempts[0].usage;
        assert_eq!(usage.usage_source, ArchaeologyUsageSource::Reported);
        assert_eq!(usage.estimated_cost_microusd, None);
        assert_eq!(
            usage.pricing_identity.as_deref(),
            Some("trusted-pricing-unavailable:v1/openai/gpt-test")
        );
        let connection = connection.lock().unwrap();
        let row = connection
            .query_row(
                "SELECT remote_disclosure_acknowledged,paid_disclosure_acknowledged,
                        estimated_cost_microusd,pricing_identity
                 FROM archaeology_synthesis_attempts",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!((row.0, row.1), (1, 1));
        assert_eq!(row.2, None);
        assert_eq!(row.3, "trusted-pricing-unavailable:v1/openai/gpt-test");
    }

    #[tokio::test]
    async fn paid_provider_failure_persists_pricing_identity_with_unknown_cost() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let mut input = command_input(request);
        input.selection = hosted_user_selection();
        let provider = Arc::new(FailingProvider {
            descriptor: hosted_descriptor(),
        });
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            input,
            Arc::new(CountingProviderFactory::available(provider)),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Failed);
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(
            result.attempts[0].usage.usage_source,
            ArchaeologyUsageSource::Unavailable
        );
        assert_eq!(
            result.attempts[0].usage.pricing_identity.as_deref(),
            Some("trusted-pricing-unavailable:v1/openai/gpt-test")
        );
        assert_eq!(result.attempts[0].usage.reported_cost_microusd, None);
        assert_eq!(result.attempts[0].usage.estimated_cost_microusd, None);
        let connection = connection.lock().unwrap();
        let accounting = connection
            .query_row(
                "SELECT usage_source,pricing_identity,reported_cost_microusd,
                        estimated_cost_microusd
                 FROM archaeology_synthesis_attempts",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            accounting,
            (
                "unavailable".into(),
                "trusted-pricing-unavailable:v1/openai/gpt-test".into(),
                None,
                None,
            )
        );
    }

    #[tokio::test]
    async fn durable_cancel_settles_attempt_cache_and_job() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response: fixture_response(&request),
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::Cancel,
        });
        let result = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            Arc::new(CountingProviderFactory::available(provider.clone())),
        )
        .await
        .unwrap();
        assert_eq!(result.status, ArchaeologySynthesisCommandStatus::Cancelled);
        assert!(result.response.is_none());
        assert!(provider.saw_pending.load(Ordering::SeqCst));
        let connection = connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_cache",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_attempts",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM archaeology_jobs WHERE job_id='job:one'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );
    }

    #[tokio::test]
    async fn ownership_loss_cancels_without_publishing_or_erasing_indeterminate_attempt() {
        let connection = Arc::new(Mutex::new(seeded_database("source")));
        let request = fixture_request();
        let provider = Arc::new(InspectingProvider {
            descriptor: local_descriptor(),
            connection: connection.clone(),
            response: fixture_response(&request),
            saw_pending: AtomicBool::new(false),
            job_mutation: ProviderJobMutation::LoseOwnership,
        });
        let error = run_business_rule_synthesis_core(
            connection.clone(),
            command_input(request),
            Arc::new(CountingProviderFactory::available(provider.clone())),
        )
        .await
        .unwrap_err();
        assert_eq!(error, ERROR_OWNERSHIP);
        assert!(provider.saw_pending.load(Ordering::SeqCst));
        let connection = connection.lock().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_cache",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM archaeology_synthesis_attempts",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending"
        );
    }

    struct CountingProviderFactory {
        calls: AtomicUsize,
        provider: Option<Arc<dyn ArchaeologySynthesisProvider>>,
    }

    impl CountingProviderFactory {
        fn available(provider: Arc<dyn ArchaeologySynthesisProvider>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                provider: Some(provider),
            }
        }

        fn unavailable() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                provider: None,
            }
        }
    }

    impl ArchaeologyProviderFactory for CountingProviderFactory {
        fn create(
            &self,
            _descriptor: &ArchaeologyProviderDescriptor,
        ) -> Result<Arc<dyn ArchaeologySynthesisProvider>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.provider.clone().ok_or_else(|| ERROR_PROVIDER.into())
        }
    }

    struct InspectingProvider {
        descriptor: ArchaeologyProviderDescriptor,
        connection: Arc<Mutex<Connection>>,
        response: ArchaeologySynthesisResponse,
        saw_pending: AtomicBool,
        job_mutation: ProviderJobMutation,
    }

    struct FailingProvider {
        descriptor: ArchaeologyProviderDescriptor,
    }

    impl ArchaeologySynthesisProvider for FailingProvider {
        fn descriptor(&self) -> &ArchaeologyProviderDescriptor {
            &self.descriptor
        }

        fn invoke(&self, _request: ArchaeologyProviderRequest) -> ProviderFuture {
            Box::pin(async {
                Err(ArchaeologyProviderFailure {
                    code: ArchaeologyProviderFailureCode::Authentication,
                    retryable: false,
                    retry_after_ms: None,
                })
            })
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ProviderJobMutation {
        None,
        Cancel,
        LoseOwnership,
    }

    impl ArchaeologySynthesisProvider for InspectingProvider {
        fn descriptor(&self) -> &ArchaeologyProviderDescriptor {
            &self.descriptor
        }

        fn invoke(&self, _request: ArchaeologyProviderRequest) -> ProviderFuture {
            let connection = self.connection.lock().unwrap();
            let pending = connection
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_synthesis_attempts WHERE status='pending'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                >= 1;
            self.saw_pending.store(pending, Ordering::SeqCst);
            match self.job_mutation {
                ProviderJobMutation::Cancel => {
                    connection
                        .execute(
                            "UPDATE archaeology_jobs
                         SET state='cancelling',cancellation_requested=1
                         WHERE job_id='job:one'",
                            [],
                        )
                        .unwrap();
                }
                ProviderJobMutation::LoseOwnership => {
                    connection
                        .execute(
                            "UPDATE archaeology_jobs SET owner_id='owner:two'
                             WHERE job_id='job:one'",
                            [],
                        )
                        .unwrap();
                }
                ProviderJobMutation::None => {}
            }
            drop(connection);
            let response = self.response.clone();
            let wait_for_cancellation = self.job_mutation != ProviderJobMutation::None;
            Box::pin(async move {
                if wait_for_cancellation {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    return Err(ArchaeologyProviderFailure {
                        code: ArchaeologyProviderFailureCode::Internal,
                        retryable: false,
                        retry_after_ms: None,
                    });
                }
                Ok(ArchaeologyProviderOutput {
                    raw_output: serde_json::to_vec(&response).unwrap(),
                    usage: ArchaeologyProviderUsage {
                        input_tokens: Some(10),
                        cached_input_tokens: Some(0),
                        output_tokens: Some(20),
                        reported_cost_microusd: None,
                        estimated_cost_microusd: None,
                        usage_source: ArchaeologyUsageSource::Reported,
                        pricing_identity: None,
                    },
                })
            })
        }
    }

    fn command_input(request: ArchaeologySynthesisRequest) -> ArchaeologySynthesisCommandInput {
        ArchaeologySynthesisCommandInput {
            job_id: "job:one".into(),
            owner_id: "owner:one".into(),
            request,
            selection: local_user_selection(),
        }
    }

    fn local_user_selection() -> ArchaeologyProviderUserSelection {
        ArchaeologyProviderUserSelection {
            enabled: true,
            provider_identity: "local".into(),
            model_identity: "local-model".into(),
            local_endpoint: Some("http://127.0.0.1:11434/v1/chat/completions".into()),
            remote_approved: false,
            remote_disclosure_version: None,
            paid_approved: false,
            paid_disclosure_version: None,
            total_timeout_ms: 1_000,
            attempt_timeout_ms: 500,
            max_attempts: 1,
            max_output_tokens: 1_024,
        }
    }

    fn hosted_user_selection() -> ArchaeologyProviderUserSelection {
        ArchaeologyProviderUserSelection {
            enabled: true,
            provider_identity: "openai".into(),
            model_identity: "gpt-test".into(),
            local_endpoint: None,
            remote_approved: true,
            remote_disclosure_version: Some(
                super::super::synthesis_runtime::ARCHAEOLOGY_REMOTE_DISCLOSURE_VERSION,
            ),
            paid_approved: true,
            paid_disclosure_version: Some(
                super::super::synthesis_runtime::ARCHAEOLOGY_PAID_DISCLOSURE_VERSION,
            ),
            total_timeout_ms: 1_000,
            attempt_timeout_ms: 500,
            max_attempts: 1,
            max_output_tokens: 1_024,
        }
    }

    fn local_descriptor() -> ArchaeologyProviderDescriptor {
        ArchaeologyProviderDescriptor {
            kind: ArchaeologyProviderKind::Local,
            provider_identity: "local".into(),
            endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
            network_scope: super::super::synthesis_runtime::ArchaeologyNetworkScope::Loopback,
        }
    }

    fn local_selection() -> ArchaeologyProviderSelection {
        ArchaeologyProviderSelection {
            enabled: true,
            provider_identity: "local".into(),
            model_identity: "local-model".into(),
            cost_class: super::super::synthesis_runtime::ArchaeologyCostClass::Free,
            pricing: None,
            remote_approved: false,
            remote_disclosure_version: None,
            paid_approved: false,
            paid_disclosure_version: None,
            execution: ArchaeologyProviderExecutionBounds {
                total_timeout_ms: 1_000,
                attempt_timeout_ms: 500,
                max_attempts: 1,
                max_output_tokens: 1_024,
            },
        }
    }

    fn hosted_descriptor() -> ArchaeologyProviderDescriptor {
        ArchaeologyProviderDescriptor {
            kind: ArchaeologyProviderKind::Hosted,
            provider_identity: "openai".into(),
            endpoint: "https://api.openai.com/v1/responses".into(),
            network_scope: super::super::synthesis_runtime::ArchaeologyNetworkScope::Remote,
        }
    }

    fn fixture_request() -> ArchaeologySynthesisRequest {
        fixture_request_with_conflict(false)
    }

    fn conflicting_fixture_request() -> ArchaeologySynthesisRequest {
        fixture_request_with_conflict(true)
    }

    fn fixture_request_with_conflict(with_conflict: bool) -> ArchaeologySynthesisRequest {
        let mut facts = vec![
            ArchaeologyFact {
                fact_id: "fact:condition".into(),
                kind: ArchaeologyFactKind::Predicate,
                label: "Positive payment".into(),
                span_ids: vec!["span:condition".into()],
                parser_id: "parser:v1".into(),
                trust: ArchaeologyTrust::Extracted,
                confidence: ArchaeologyConfidence::High,
                attributes: semantic_expression('b'),
            },
            ArchaeologyFact {
                fact_id: "fact:action".into(),
                kind: ArchaeologyFactKind::Mutation,
                label: "Schedule payment".into(),
                span_ids: vec!["span:action".into()],
                parser_id: "parser:v1".into(),
                trust: ArchaeologyTrust::Extracted,
                confidence: ArchaeologyConfidence::High,
                attributes: semantic_expression('a'),
            },
        ];
        let mut relationships = vec![ArchaeologyFactEdge {
            edge_id: "relationship:controls".into(),
            from_fact_id: "fact:condition".into(),
            to_fact_id: "fact:action".into(),
            kind: ArchaeologyFactEdgeKind::Controls,
            trust: ArchaeologyTrust::Extracted,
            evidence_span_ids: vec!["span:action".into(), "span:condition".into()],
            unresolved_reason: None,
        }];
        if with_conflict {
            facts.push(ArchaeologyFact {
                fact_id: "fact:contradiction".into(),
                kind: ArchaeologyFactKind::Predicate,
                label: "Non-positive payment is allowed".into(),
                span_ids: vec!["span:contradiction".into()],
                parser_id: "parser:v1".into(),
                trust: ArchaeologyTrust::Extracted,
                confidence: ArchaeologyConfidence::High,
                attributes: semantic_expression('c'),
            });
            relationships.push(ArchaeologyFactEdge {
                edge_id: "relationship:contradicts".into(),
                from_fact_id: "fact:condition".into(),
                to_fact_id: "fact:contradiction".into(),
                kind: ArchaeologyFactEdgeKind::Contradicts,
                trust: ArchaeologyTrust::Deterministic,
                evidence_span_ids: vec!["span:condition".into(), "span:contradiction".into()],
                unresolved_reason: None,
            });
        }
        let packet = derive_evidence_packets(
            "repository:one",
            REVISION,
            &facts,
            &relationships,
            &Default::default(),
            ArchaeologyDeterministicLimits::default(),
        )
        .unwrap()
        .into_iter()
        .find(|packet| packet.anchor_fact_id == "fact:condition")
        .unwrap();
        build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:manifest:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &relationships,
            &Default::default(),
            Default::default(),
        )
        .unwrap()
    }

    fn semantic_expression(digit: char) -> Vec<ArchaeologyAttribute> {
        vec![ArchaeologyAttribute {
            key: "semantic_expr".into(),
            value: format!("v1:sha256:{}", digit.to_string().repeat(64)),
        }]
    }

    fn fixture_response(request: &ArchaeologySynthesisRequest) -> ArchaeologySynthesisResponse {
        let has_conflict = !request.packet.contradicting_fact_ids.is_empty();
        ArchaeologySynthesisResponse {
            schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
            contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID.into(),
            request_id: request.request_id.clone(),
            packet_id: request.packet.packet_id.clone(),
            clauses: vec![ArchaeologySynthesisClause {
                subject: ArchaeologySynthesisSegment {
                    text: "Payment".into(),
                    fact_ids: vec!["fact:condition".into()],
                },
                condition: Some(ArchaeologySynthesisSegment {
                    text: "the payment is positive".into(),
                    fact_ids: vec!["fact:condition".into()],
                }),
                action: ArchaeologySynthesisSegment {
                    text: "schedule the payment".into(),
                    fact_ids: vec!["fact:action".into()],
                },
                exception: None,
                quantifier: None,
                relationship_ids: if has_conflict {
                    vec![
                        "relationship:contradicts".into(),
                        "relationship:controls".into(),
                    ]
                } else {
                    vec!["relationship:controls".into()]
                },
                contradicting_fact_ids: request.packet.contradicting_fact_ids.clone(),
            }],
        }
    }

    fn insert_ready_cache(
        connection: &Connection,
        plan: &super::super::synthesis_runtime::ArchaeologySynthesisPlan,
        response: &ArchaeologySynthesisResponse,
    ) {
        let json = serde_json::to_string(response).unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(json.as_bytes()));
        connection
            .execute(
                "INSERT INTO archaeology_synthesis_cache
                 (generation_id,cache_key,request_id,evidence_identity,packet_id,
                  provider_identity,provider_route_identity,model_identity,prompt_identity,
                  policy_identity,status,response_json,response_sha256,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'ready',?11,?12,?13,?13)",
                rusqlite::params![
                    plan.generation_id,
                    plan.cache_key,
                    plan.request_id,
                    plan.evidence_identity,
                    plan.packet_id,
                    plan.provider_identity,
                    plan.provider_route_identity,
                    plan.model_identity,
                    plan.prompt_identity,
                    plan.policy_identity,
                    json,
                    hash,
                    "2026-07-17T00:00:00Z",
                ],
            )
            .unwrap();
    }

    fn catalog_row_counts(connection: &Connection) -> (i64, i64) {
        connection
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM archaeology_rule_search_manifest),
                   (SELECT COUNT(*) FROM archaeology_rule_fts)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    fn assert_model_catalog(connection: &Connection, cache_key: &str) {
        let row: (String, String, String, i64) = connection
            .query_row(
                "SELECT rule.trust,rule.synthesis_identity,clause.clause_text,
                        (SELECT COUNT(*) FROM archaeology_evidence_links evidence
                         WHERE evidence.owner_kind='rule_clause'
                           AND evidence.owner_id=clause.clause_id)
                 FROM archaeology_rules rule JOIN archaeology_rule_clauses clause
                   ON clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "model_synthesized");
        assert_eq!(row.1, cache_key);
        assert!(row.2.contains("predicate \"Positive payment\""));
        assert!(row.2.contains("mutation \"Schedule payment\""));
        assert!(!row.2.contains("the payment is positive"));
        assert!(row.3 >= 4);
        assert_eq!(catalog_row_counts(connection), (1, 1));
        assert_eq!(
            connection
                .query_row(
                    "SELECT stage FROM archaeology_jobs WHERE job_id='job:one'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "validate"
        );
    }

    fn assert_unpublished_catalog(connection: &Connection) {
        assert_eq!(catalog_row_counts(connection), (0, 0));
        assert_eq!(
            connection
                .query_row(
                    "SELECT stage FROM archaeology_jobs WHERE job_id='job:one'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "synthesize"
        );
        assert_eq!(
            connection
                .query_row("SELECT trust FROM archaeology_rules", [], |row| row
                    .get::<_, String>(0),)
                .unwrap(),
            "deterministic"
        );
    }

    fn seeded_database(classification: &str) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        archaeology_schema::run_migration(&connection).unwrap();
        connection
            .execute_batch(&format!(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
                 VALUES ('repository:one','/fixture','source','{REVISION}','now','now');
                 INSERT INTO archaeology_generations
                 (generation_id,repository_id,schema_version,revision_sha,source_identity,
                  parser_identity,algorithm_identity,config_identity,status,created_at)
                 VALUES ('generation:one','repository:one',{ARCHAEOLOGY_STORAGE_SCHEMA_VERSION},'{REVISION}','source',
                         'parser:manifest:v1','algorithm:v1','config','staging','now');
                 INSERT INTO archaeology_source_units
                 (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                  hash_algorithm,language,parser_id,parser_version,classification,byte_count,line_count)
                 VALUES ('generation:one','unit:one','path:one','src/rules.cbl',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','sha256',
                         'cobol','parser:v1','1','{classification}',100,10);
                 INSERT INTO archaeology_source_spans
                 (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                  start_line,start_column,end_line,end_column) VALUES
                 ('generation:one','span:action','unit:one','{REVISION}',0,10,1,1,1,11),
                 ('generation:one','span:condition','unit:one','{REVISION}',11,20,2,1,2,10);
                 INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json) VALUES
                 ('generation:one','fact:action','mutation','Schedule payment','parser:v1','extracted','high',
                  '[{{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}]'),
                 ('generation:one','fact:condition','predicate','Positive payment','parser:v1','extracted','high',
                  '[{{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}}]');
                 INSERT INTO archaeology_fact_edges
                 (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
                 VALUES ('generation:one','relationship:controls','fact:condition','fact:action',
                         'controls','extracted');
                 INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role) VALUES
                 ('generation:one','fact','fact:action','span','span:action','supporting'),
                 ('generation:one','fact','fact:condition','span','span:condition','supporting'),
                 ('generation:one','fact_edge','relationship:controls','span','span:action','supporting'),
                 ('generation:one','fact_edge','relationship:controls','span','span:condition','supporting');
                 INSERT INTO archaeology_jobs
                 (job_id,repository_id,generation_id,owner_id,stage,state,updated_at)
                 VALUES ('job:one','repository:one','generation:one','owner:one',
                         'synthesize','running','2026-07-17T00:00:00Z');"
            ))
            .unwrap();
        connection
            .execute(
                "UPDATE archaeology_jobs SET checkpoint_json=?1 WHERE job_id='job:one'",
                [serde_json::to_string(&jobs::ArchaeologyJobCheckpoint::default()).unwrap()],
            )
            .unwrap();
        if classification == "source" {
            seed_deterministic_catalog(&connection, &fixture_request());
        }
        connection
    }

    fn seeded_conflicting_database(request: &ArchaeologySynthesisRequest) -> Connection {
        let connection = seeded_database("source");
        connection
            .execute_batch(
                r#"DELETE FROM archaeology_rule_domains;
                 DELETE FROM archaeology_evidence_links WHERE owner_kind='rule_clause';
                 DELETE FROM archaeology_rule_clauses;
                 DELETE FROM archaeology_rules;
                 INSERT INTO archaeology_source_spans
                 (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                  start_line,start_column,end_line,end_column)
                 VALUES ('generation:one','span:contradiction','unit:one',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',21,30,3,1,3,10);
                 INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES ('generation:one','fact:contradiction','predicate',
                         'Non-positive payment is allowed','parser:v1','extracted','high',
                         '[{"key":"semantic_expr","value":"v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}]');
                 INSERT INTO archaeology_fact_edges
                 (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
                 VALUES ('generation:one','relationship:contradicts','fact:condition',
                         'fact:contradiction','contradicts','deterministic');
                 INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role) VALUES
                 ('generation:one','fact','fact:contradiction','span',
                  'span:contradiction','supporting'),
                 ('generation:one','fact_edge','relationship:contradicts','span',
                  'span:condition','supporting'),
                 ('generation:one','fact_edge','relationship:contradicts','span',
                  'span:contradiction','supporting');"#,
            )
            .unwrap();
        seed_deterministic_catalog(&connection, request);
        connection
            .execute_batch(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role) VALUES
                 ('generation:one','rule_clause','clause:template','fact',
                  'fact:contradiction','contradicting'),
                 ('generation:one','rule_clause','clause:template','span',
                  'span:contradiction','contradicting');",
            )
            .unwrap();
        connection
    }

    fn seed_deterministic_catalog(connection: &Connection, request: &ArchaeologySynthesisRequest) {
        let rule_id = expected_rule_id(&request.packet);
        let coverage = serde_json::to_string(&ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Complete,
            discovered_source_units: 1,
            indexed_source_units: 1,
            discovered_bytes: 100,
            indexed_bytes: 100,
            reasons: Vec::new(),
            ..Default::default()
        })
        .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at)
                 VALUES ('generation:one',?1,'repository:one',?2,'validation',
                         'Validation candidate: positive payment','candidate','deterministic',
                         'high','parser:manifest:v1','algorithm:v1',?3,'now')",
                rusqlite::params![rule_id, REVISION, coverage],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES ('generation:one',?1,'clause:template',0,
                         'Positive payment controls scheduling.','deterministic','high','[]')",
                [&rule_id],
            )
            .unwrap();
        for (kind, id) in [
            ("fact", "fact:action"),
            ("fact", "fact:condition"),
            ("span", "span:action"),
            ("span", "span:condition"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                     (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                     VALUES ('generation:one','rule_clause','clause:template',?1,?2,'supporting')",
                    rusqlite::params![kind, id],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO archaeology_rule_domains
                 (generation_id,rule_id,domain_id,domain_label)
                 VALUES ('generation:one',?1,'domain:other','Other')",
                [&rule_id],
            )
            .unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        let cancellation = StructuralGraphCancellation::default();
        assert_eq!(
            super::super::identity_store::refresh_rule_identities(
                &transaction,
                "generation:one",
                std::slice::from_ref(&rule_id),
                &cancellation,
            )
            .unwrap(),
            1
        );
        transaction.commit().unwrap();
    }
}
