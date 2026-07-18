//! SQLite persistence for bounded incremental archaeology planning.
//!
//! The job engine owns stage transitions; this module owns exact invalidation
//! metadata, durable bounded work selection, and atomic refresh checkpoints.

use super::adapter::{ArchaeologyAdapterLineage, ArchaeologyLineageKind};
use super::evidence_store::{clear_compact_evidence_generation, clone_compact_span_evidence};
use super::invalidation::{
    classify_generation_input_changes, reverse_dependency_closure, ArchaeologyGenerationInput,
    ArchaeologyGenerationInputKind, ArchaeologyInputDecision, ArchaeologyInputInvalidationMode,
    ArchaeologyInvalidatedPath, ArchaeologyInvalidationLimits, ArchaeologySourceDependency,
    ArchaeologySourceDependencyKind,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyPersistedInvalidationMetadata {
    pub(crate) input_count: usize,
    pub(crate) dependency_count: usize,
    pub(crate) unresolved_lineage: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyInvalidationPlan {
    pub(crate) repository_id: String,
    pub(crate) generation_id: String,
    pub(crate) prior_ready_generation_id: Option<String>,
    pub(crate) decision: ArchaeologyInputDecision,
    pub(crate) invalidated_paths: Vec<ArchaeologyInvalidatedPath>,
    pub(crate) removed_path_identities: Vec<String>,
    pub(crate) unresolved_lineage: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyRefreshWorkItem {
    pub(crate) ordinal: u64,
    pub(crate) target_kind: String,
    pub(crate) target_identity: String,
    pub(crate) action: String,
    pub(crate) depth: usize,
    pub(crate) reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyRefreshExecution {
    pub(crate) plan_identity: String,
    pub(crate) completed: usize,
    pub(crate) remaining: usize,
}

#[derive(Clone, Copy)]
enum RefreshWorkPhase {
    All,
    Parse,
}

struct GenerationIdentity {
    revision_sha: String,
    schema_version: i64,
    parser_identity: String,
    algorithm_identity: String,
    config_identity: String,
}

/// Replace one generation's invalidation metadata atomically.
///
/// Source dependencies are materialized only from exact, resolved adapter
/// lineage. Symbol/call/data/rule ownership is not inferred here because the
/// persisted fact tables do not retain unambiguous cross-unit ownership.
pub(crate) fn persist_generation_invalidation_metadata(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    inputs: &[ArchaeologyGenerationInput],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<ArchaeologyPersistedInvalidationMetadata, String> {
    cancelled(cancellation)?;
    // Reusing the pure classifier against the same set gives storage the exact
    // same scope, uniqueness, and identity validation as planning.
    classify_generation_input_changes(inputs, inputs)?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("Begin archaeology invalidation metadata transaction: {error}"))?;
    let generation = require_staging_generation_scope(&transaction, repository_id, generation_id)?;
    validate_canonical_inputs(&transaction, generation_id, inputs, &generation, limits)?;

    transaction
        .execute(
            "DELETE FROM archaeology_generation_inputs WHERE generation_id=?1",
            [generation_id],
        )
        .map_err(|error| format!("Clear archaeology generation inputs: {error}"))?;
    transaction
        .execute(
            "DELETE FROM archaeology_source_dependencies WHERE generation_id=?1",
            [generation_id],
        )
        .map_err(|error| format!("Clear archaeology source dependencies: {error}"))?;

    let mut sorted_inputs = inputs.to_vec();
    sorted_inputs.sort_by(|left, right| {
        (left.kind, left.scope.as_deref(), left.identity.as_str()).cmp(&(
            right.kind,
            right.scope.as_deref(),
            right.identity.as_str(),
        ))
    });
    for input in &sorted_inputs {
        cancelled(cancellation)?;
        transaction
            .execute(
                "INSERT INTO archaeology_generation_inputs
                 (generation_id,input_kind,scope_identity,input_identity)
                 VALUES (?1,?2,?3,?4)",
                params![
                    generation_id,
                    input_kind_name(input.kind),
                    input.scope.as_deref().unwrap_or(""),
                    input.identity
                ],
            )
            .map_err(|error| format!("Persist archaeology generation input: {error}"))?;
    }

    let (dependencies, unresolved_lineage) =
        derive_provable_dependencies(&transaction, generation_id, cancellation, limits)?;
    for dependency in &dependencies {
        cancelled(cancellation)?;
        transaction
            .execute(
                "INSERT INTO archaeology_source_dependencies
                 (generation_id,dependent_path_identity,prerequisite_path_identity,kind,
                  evidence_identity)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    generation_id,
                    dependency.dependent_path_identity,
                    dependency.prerequisite_path_identity,
                    dependency_kind_name(dependency.kind),
                    dependency_evidence_identity(repository_id, dependency),
                ],
            )
            .map_err(|error| format!("Persist archaeology source dependency: {error}"))?;
    }
    cancelled(cancellation)?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology invalidation metadata: {error}"))?;
    Ok(ArchaeologyPersistedInvalidationMetadata {
        input_count: sorted_inputs.len(),
        dependency_count: dependencies.len(),
        unresolved_lineage,
    })
}

/// Plan against the repository's prior ready generation without publishing or
/// mutating its ready pointer.
pub(crate) fn plan_generation_invalidation(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    changed_path_identities: &[String],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<ArchaeologyInvalidationPlan, String> {
    cancelled(cancellation)?;
    if changed_path_identities.len() > limits.max_seed_paths {
        return Err("Archaeology invalidation seed bound exceeded".into());
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Deferred)
        .map_err(|error| format!("Begin archaeology invalidation planning transaction: {error}"))?;
    require_generation_scope(&transaction, repository_id, generation_id)?;
    let ready_generation_id = transaction
        .query_row(
            "SELECT ready_generation_id FROM archaeology_repositories
             WHERE repository_id=?1",
            [repository_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|error| format!("Load archaeology ready generation: {error}"))?
        .ok_or("Archaeology repository scope does not exist")?;

    preflight_generation_input_bounds(&transaction, generation_id, limits)?;
    let current_inputs = load_generation_inputs(&transaction, repository_id, generation_id)?;
    let current_identity = load_generation_identity(&transaction, repository_id, generation_id)?;
    validate_canonical_inputs(
        &transaction,
        generation_id,
        &current_inputs,
        &current_identity,
        limits,
    )?;
    let mut unresolved_lineage = generation_has_unresolved_lineage(
        &transaction,
        repository_id,
        generation_id,
        cancellation,
        limits,
    )?;
    let (previous_inputs, dependencies) = match ready_generation_id.as_deref() {
        Some(ready) if ready != generation_id => {
            require_ready_generation_scope(&transaction, repository_id, ready)?;
            unresolved_lineage |= generation_has_unresolved_lineage(
                &transaction,
                repository_id,
                ready,
                cancellation,
                limits,
            )?;
            preflight_generation_input_bounds(&transaction, ready, limits)?;
            preflight_dependency_bounds(&transaction, ready, limits)?;
            let ready_inputs = load_generation_inputs(&transaction, repository_id, ready)?;
            let ready_identity = load_generation_identity(&transaction, repository_id, ready)?;
            validate_canonical_inputs(&transaction, ready, &ready_inputs, &ready_identity, limits)?;
            (
                ready_inputs,
                load_source_dependencies(&transaction, repository_id, ready)?,
            )
        }
        Some(ready) => {
            require_ready_generation_scope(&transaction, repository_id, ready)?;
            (
                current_inputs.clone(),
                load_source_dependencies(&transaction, repository_id, ready)?,
            )
        }
        None => (Vec::new(), Vec::new()),
    };

    validate_seed_scope(
        &transaction,
        repository_id,
        generation_id,
        ready_generation_id.as_deref(),
        changed_path_identities,
        cancellation,
        limits,
    )?;
    let mut decision = classify_generation_input_changes(&previous_inputs, &current_inputs)?;
    if ready_generation_id.is_none() || unresolved_lineage {
        decision.mode = ArchaeologyInputInvalidationMode::GlobalRebuild;
    } else if !changed_path_identities.is_empty()
        && matches!(
            decision.mode,
            ArchaeologyInputInvalidationMode::NoOp
                | ArchaeologyInputInvalidationMode::SynthesisOnly
        )
    {
        decision.mode = ArchaeologyInputInvalidationMode::Scoped;
        if !decision
            .changed_kinds
            .contains(&ArchaeologyGenerationInputKind::Head)
        {
            decision
                .changed_kinds
                .push(ArchaeologyGenerationInputKind::Head);
            decision.changed_kinds.sort();
        }
    } else if decision.mode == ArchaeologyInputInvalidationMode::Scoped
        && changed_path_identities.is_empty()
    {
        // A commit-only HEAD move with identical inventory is a true no-op.
        // Other scoped changes still lack a provable unit mapping and rebuild.
        decision.mode = if decision.changed_kinds == [ArchaeologyGenerationInputKind::Head] {
            ArchaeologyInputInvalidationMode::NoOp
        } else {
            ArchaeologyInputInvalidationMode::GlobalRebuild
        };
    }
    let invalidated_paths = match decision.mode {
        ArchaeologyInputInvalidationMode::Scoped => reverse_dependency_closure(
            changed_path_identities,
            &dependencies,
            cancellation,
            limits,
        )?,
        ArchaeologyInputInvalidationMode::GlobalRebuild => load_global_paths(
            &transaction,
            generation_id,
            ready_generation_id.as_deref(),
            limits,
        )?,
        ArchaeologyInputInvalidationMode::NoOp
        | ArchaeologyInputInvalidationMode::SynthesisOnly => Vec::new(),
    };
    let mut removed_path_identities = Vec::new();
    for path in &invalidated_paths {
        if !path_exists(&transaction, generation_id, &path.path_identity)? {
            removed_path_identities.push(path.path_identity.clone());
        }
    }
    cancelled(cancellation)?;
    let plan = ArchaeologyInvalidationPlan {
        repository_id: repository_id.to_string(),
        generation_id: generation_id.to_string(),
        prior_ready_generation_id: ready_generation_id,
        decision,
        invalidated_paths,
        removed_path_identities,
        unresolved_lineage,
    };
    transaction.commit().map_err(|error| {
        format!("Finish archaeology invalidation planning transaction: {error}")
    })?;
    Ok(plan)
}

pub(crate) fn persist_refresh_work_plan(
    connection: &Connection,
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
    plan: &ArchaeologyInvalidationPlan,
) -> Result<String, String> {
    if plan.repository_id != repository_id || plan.generation_id != generation_id {
        return Err("Archaeology refresh plan is outside job scope".into());
    }
    let plan_identity = refresh_plan_identity(plan);
    let work = work_items(plan)?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("Begin archaeology refresh work transaction: {error}"))?;
    require_active_job_scope(&transaction, job_id, repository_id, generation_id, owner_id)?;
    let (existing_plan_count, existing_plan) = transaction
        .query_row(
            "SELECT COUNT(DISTINCT plan_identity),MIN(plan_identity)
             FROM archaeology_refresh_work_items WHERE job_id=?1",
            [job_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(|error| format!("Load archaeology refresh work plan: {error}"))?;
    if existing_plan_count > 1 {
        return Err("Archaeology refresh job has conflicting work plans".into());
    }
    if let Some(existing_plan) = existing_plan {
        if existing_plan != plan_identity {
            return Err("Archaeology refresh job already has a different work plan".into());
        }
        let existing = load_refresh_work_items(
            &transaction,
            job_id,
            &plan_identity,
            false,
            i64::MAX,
            RefreshWorkPhase::All,
        )?;
        if existing != work {
            return Err("Archaeology refresh work plan does not reconcile".into());
        }
        transaction
            .commit()
            .map_err(|error| format!("Finish archaeology refresh work transaction: {error}"))?;
        return Ok(plan_identity);
    }
    for item in &work {
        let reasons = serde_json::to_string(&item.reasons)
            .map_err(|error| format!("Serialize archaeology refresh reasons: {error}"))?;
        transaction
            .execute(
                "INSERT INTO archaeology_refresh_work_items
                 (job_id,plan_identity,ordinal,target_kind,target_identity,action,depth,reasons_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    job_id,
                    plan_identity,
                    i64::try_from(item.ordinal)
                        .map_err(|_| "Archaeology refresh ordinal overflowed")?,
                    item.target_kind,
                    item.target_identity,
                    item.action,
                    i64::try_from(item.depth)
                        .map_err(|_| "Archaeology refresh depth overflowed")?,
                    reasons,
                ],
            )
            .map_err(|error| format!("Persist archaeology refresh work item: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology refresh work plan: {error}"))?;
    Ok(plan_identity)
}

/// Apply a bounded batch of prepared refresh results. Keep expensive parsing
/// outside this callback: it runs in the same short transaction as the durable
/// completion checkpoint, so persisted output and retry state stay atomic.
pub(crate) fn execute_refresh_work_batch(
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
    execute_refresh_work_batch_for_phase(
        connection,
        job_id,
        repository_id,
        generation_id,
        owner_id,
        plan_identity,
        max_items,
        now,
        cancellation,
        RefreshWorkPhase::All,
        execute,
    )
}

pub(crate) fn execute_refresh_parse_work_batch(
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
    execute_refresh_work_batch_for_phase(
        connection,
        job_id,
        repository_id,
        generation_id,
        owner_id,
        plan_identity,
        max_items,
        now,
        cancellation,
        RefreshWorkPhase::Parse,
        execute,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_refresh_work_batch_for_phase(
    connection: &Connection,
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
    plan_identity: &str,
    max_items: usize,
    now: &str,
    cancellation: &StructuralGraphCancellation,
    phase: RefreshWorkPhase,
    mut execute: impl FnMut(&Transaction<'_>, &ArchaeologyRefreshWorkItem) -> Result<(), String>,
) -> Result<ArchaeologyRefreshExecution, String> {
    if max_items == 0 || max_items > 10_000 {
        return Err("Archaeology refresh batch bound is invalid".into());
    }
    validate_digest_identity(plan_identity, "refresh plan")?;
    cancelled(cancellation)?;
    require_active_job_scope(connection, job_id, repository_id, generation_id, owner_id)?;
    let pending = load_refresh_work_items(
        connection,
        job_id,
        plan_identity,
        true,
        i64::try_from(max_items).map_err(|_| "Archaeology refresh batch bound overflowed")?,
        phase,
    )?;
    let mut completed = 0;
    for item in pending {
        cancelled(cancellation)?;
        require_active_job_scope(connection, job_id, repository_id, generation_id, owner_id)?;
        let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("Begin archaeology refresh checkpoint: {error}"))?;
        require_active_job_scope(&transaction, job_id, repository_id, generation_id, owner_id)?;
        execute(&transaction, &item)?;
        cancelled(cancellation)?;
        let changed = transaction
            .execute(
                "UPDATE archaeology_refresh_work_items
                 SET completed=1,completed_at=?4
                 WHERE job_id=?1 AND plan_identity=?2 AND ordinal=?3 AND completed=0",
                params![job_id, plan_identity, item.ordinal, now],
            )
            .map_err(|error| format!("Checkpoint archaeology refresh work item: {error}"))?;
        if changed != 1 {
            return Err("Archaeology refresh work checkpoint did not reconcile".into());
        }
        transaction
            .commit()
            .map_err(|error| format!("Commit archaeology refresh checkpoint: {error}"))?;
        completed += 1;
    }
    let remaining = count_pending_refresh_work(connection, job_id, plan_identity, phase)?;
    Ok(ArchaeologyRefreshExecution {
        plan_identity: plan_identity.to_string(),
        completed,
        remaining: usize::try_from(remaining)
            .map_err(|_| "Archaeology refresh remaining count overflowed")?,
    })
}

pub(crate) fn clone_unaffected_ready_facts(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    plan: &ArchaeologyInvalidationPlan,
) -> Result<(), String> {
    if plan.repository_id != repository_id || plan.generation_id != generation_id {
        return Err("Archaeology clone plan is outside generation scope".into());
    }
    require_staging_generation_scope(connection, repository_id, generation_id)?;
    let Some(ready) = plan.prior_ready_generation_id.as_deref() else {
        return Ok(());
    };
    require_ready_generation_scope(connection, repository_id, ready)?;
    let invalidated = serde_json::to_string(
        &plan
            .invalidated_paths
            .iter()
            .map(|path| path.path_identity.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| format!("Serialize archaeology invalidated paths: {error}"))?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| format!("Begin archaeology unchanged fact clone: {error}"))?;
    transaction
        .execute_batch("PRAGMA defer_foreign_keys=ON")
        .map_err(|error| format!("Defer archaeology clone constraints: {error}"))?;
    clear_compact_evidence_generation(&transaction, generation_id)
        .and_then(|_| {
            transaction.execute(
                "DELETE FROM archaeology_fact_edges WHERE generation_id=?1",
                [generation_id],
            )
        })
        .and_then(|_| {
            transaction.execute(
                "DELETE FROM archaeology_facts WHERE generation_id=?1",
                [generation_id],
            )
        })
        .and_then(|_| {
            transaction.execute(
                "DELETE FROM archaeology_source_spans WHERE generation_id=?1",
                [generation_id],
            )
        })
        .map_err(|error| format!("Reset archaeology unchanged fact clone: {error}"))?;
    transaction
        .execute(
            "UPDATE archaeology_source_units AS current SET
                 parser_id=(SELECT prior.parser_id FROM archaeology_source_units prior
                     WHERE prior.generation_id=?2 AND prior.path_identity=current.path_identity),
                 parser_version=(SELECT prior.parser_version FROM archaeology_source_units prior
                     WHERE prior.generation_id=?2 AND prior.path_identity=current.path_identity),
                 include_lineage_json='[]',
                 recovery_json=(SELECT prior.recovery_json FROM archaeology_source_units prior
                     WHERE prior.generation_id=?2 AND prior.path_identity=current.path_identity),
                 coverage_json=(SELECT prior.coverage_json FROM archaeology_source_units prior
                     WHERE prior.generation_id=?2 AND prior.path_identity=current.path_identity)
             WHERE current.generation_id=?1
               AND current.path_identity NOT IN (SELECT value FROM json_each(?3))
               AND EXISTS(SELECT 1 FROM archaeology_source_units prior
                   WHERE prior.generation_id=?2 AND prior.path_identity=current.path_identity)",
            params![generation_id, ready, invalidated],
        )
        .map_err(|error| format!("Clone unchanged archaeology unit metadata: {error}"))?;
    if plan.decision.mode == ArchaeologyInputInvalidationMode::NoOp {
        transaction
            .execute(
                "UPDATE archaeology_generations SET coverage_json=(
                     SELECT coverage_json FROM archaeology_generations WHERE generation_id=?2
                 ) WHERE generation_id=?1",
                params![generation_id, ready],
            )
            .map_err(|error| format!("Clone no-op archaeology generation coverage: {error}"))?;
    }
    transaction
        .execute(
            "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             SELECT ?1,span.span_id,current.source_unit_id,generation.revision_sha,
                    span.start_byte,span.end_byte,span.start_line,span.start_column,
                    span.end_line,span.end_column
             FROM archaeology_source_spans span
             JOIN archaeology_source_units prior ON prior.generation_id=span.generation_id
              AND prior.source_unit_id=span.source_unit_id
             JOIN archaeology_source_units current ON current.generation_id=?1
              AND current.path_identity=prior.path_identity
             JOIN archaeology_generations generation ON generation.generation_id=?1
             WHERE span.generation_id=?2
               AND prior.path_identity NOT IN (SELECT value FROM json_each(?3))",
            params![generation_id, ready, invalidated],
        )
        .map_err(|error| format!("Clone unchanged archaeology spans: {error}"))?;
    // Linker identities include the revision and the Link stage recomputes the
    // complete projection. Carrying them forward would retain the old edge
    // beside its current-revision replacement and duplicate derived clauses.
    transaction
        .execute(
            "WITH ready_generation AS (
                 SELECT generation_key FROM archaeology_generation_keys WHERE generation_id=?2
             ), eligible_facts AS MATERIALIZED (
                 SELECT owner.identity AS fact_id
                 FROM archaeology_evidence_links_compact AS link
                 JOIN archaeology_evidence_identities AS owner
                   ON owner.generation_key=link.generation_key
                  AND owner.identity_key=link.owner_identity_key
                 JOIN archaeology_evidence_identities AS evidence
                   ON evidence.generation_key=link.generation_key
                  AND evidence.identity_key=link.evidence_identity_key
                 JOIN archaeology_source_spans AS span
                   ON span.generation_id=?2 AND span.span_id=evidence.identity
                 JOIN archaeology_source_units AS unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 WHERE link.generation_key=(SELECT generation_key FROM ready_generation)
                   AND link.owner_kind_code=1 AND link.evidence_kind_code=1
                 GROUP BY owner.identity
                 HAVING SUM(CASE WHEN unit.path_identity IN (
                     SELECT value FROM json_each(?3)
                 ) THEN 1 ELSE 0 END)=0
             )
             INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
             SELECT ?1,fact.fact_id,fact.kind,fact.label,fact.parser_id,fact.trust,
                    fact.confidence,fact.attributes_json
             FROM archaeology_facts fact
             JOIN eligible_facts ON eligible_facts.fact_id=fact.fact_id
             WHERE fact.generation_id=?2
               AND fact.fact_id NOT LIKE 'archaeology-link-fact:%'
            ",
            params![generation_id, ready, invalidated],
        )
        .map_err(|error| format!("Clone unchanged archaeology facts: {error}"))?;
    clone_compact_span_evidence(&transaction, generation_id, ready, "fact")
        .map_err(|error| format!("Clone unchanged archaeology fact evidence: {error}"))?;
    transaction
        .execute(
            "INSERT INTO archaeology_fact_edges
             (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust,unresolved_reason)
             SELECT ?1,edge.edge_id,edge.from_fact_id,edge.to_fact_id,edge.kind,
                    edge.trust,edge.unresolved_reason
             FROM archaeology_fact_edges edge
             JOIN archaeology_facts source ON source.generation_id=?1
              AND source.fact_id=edge.from_fact_id
             JOIN archaeology_facts target ON target.generation_id=?1
              AND target.fact_id=edge.to_fact_id
             WHERE edge.generation_id=?2
               AND edge.edge_id NOT LIKE 'archaeology-link-edge:%'
               AND NOT EXISTS(SELECT 1 FROM archaeology_evidence_links evidence
                   WHERE evidence.generation_id=edge.generation_id
                    AND evidence.owner_kind='fact_edge' AND evidence.owner_id=edge.edge_id
                    AND evidence.evidence_kind='span'
                    AND NOT EXISTS(SELECT 1 FROM archaeology_source_spans current_span
                        WHERE current_span.generation_id=?1
                         AND current_span.span_id=evidence.evidence_id))",
            params![generation_id, ready],
        )
        .map_err(|error| format!("Clone unchanged archaeology fact edges: {error}"))?;
    clone_compact_span_evidence(&transaction, generation_id, ready, "fact_edge")
        .map_err(|error| format!("Clone unchanged archaeology edge evidence: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology unchanged fact clone: {error}"))
}

pub(crate) fn load_generation_inputs(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<Vec<ArchaeologyGenerationInput>, String> {
    require_generation_scope(connection, repository_id, generation_id)?;
    let mut statement = connection
        .prepare(
            "SELECT input_kind,scope_identity,input_identity
             FROM archaeology_generation_inputs WHERE generation_id=?1
             ORDER BY input_kind,scope_identity,input_identity",
        )
        .map_err(|error| format!("Prepare archaeology generation inputs: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            let kind = row.get::<_, String>(0)?;
            let scope = row.get::<_, String>(1)?;
            Ok((kind, scope, row.get::<_, String>(2)?))
        })
        .map_err(|error| format!("Query archaeology generation inputs: {error}"))?;
    let mut inputs = Vec::new();
    for row in rows {
        let (kind, scope, identity) =
            row.map_err(|error| format!("Read archaeology generation input: {error}"))?;
        inputs.push(ArchaeologyGenerationInput {
            kind: parse_input_kind(&kind)?,
            scope: (!scope.is_empty()).then_some(scope),
            identity,
        });
    }
    classify_generation_input_changes(&inputs, &inputs)?;
    Ok(inputs)
}

pub(crate) fn load_source_dependencies(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<Vec<ArchaeologySourceDependency>, String> {
    require_generation_scope(connection, repository_id, generation_id)?;
    let mut statement = connection
        .prepare(
            "SELECT dependent_path_identity,prerequisite_path_identity,kind
             FROM archaeology_source_dependencies WHERE generation_id=?1
             ORDER BY dependent_path_identity,prerequisite_path_identity,kind",
        )
        .map_err(|error| format!("Prepare archaeology source dependencies: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Query archaeology source dependencies: {error}"))?;
    let mut dependencies = Vec::new();
    for row in rows {
        let (dependent_path_identity, prerequisite_path_identity, kind) =
            row.map_err(|error| format!("Read archaeology source dependency: {error}"))?;
        dependencies.push(ArchaeologySourceDependency {
            dependent_path_identity,
            prerequisite_path_identity,
            kind: parse_dependency_kind(&kind)?,
        });
    }
    Ok(dependencies)
}

pub(crate) fn changed_source_paths(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    limits: ArchaeologyInvalidationLimits,
) -> Result<Vec<String>, String> {
    require_staging_generation_scope(connection, repository_id, generation_id)?;
    let ready = connection
        .query_row(
            "SELECT ready_generation_id FROM archaeology_repositories WHERE repository_id=?1",
            [repository_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| format!("Load ready generation for source comparison: {error}"))?;
    let Some(ready) = ready else {
        return load_generation_paths(connection, generation_id, limits);
    };
    require_ready_generation_scope(connection, repository_id, &ready)?;
    let limit = limits
        .max_seed_paths
        .checked_add(1)
        .ok_or("Archaeology changed source bound overflowed")?;
    let mut statement = connection
        .prepare(
            "SELECT path_identity FROM (
                 SELECT current.path_identity
                 FROM archaeology_source_units current
                 LEFT JOIN archaeology_source_units prior
                   ON prior.generation_id=?2 AND prior.path_identity=current.path_identity
                 WHERE current.generation_id=?1 AND (
                     prior.path_identity IS NULL
                     OR prior.content_hash IS NOT current.content_hash
                     OR prior.hash_algorithm IS NOT current.hash_algorithm
                     OR prior.change_identity IS NOT current.change_identity
                     OR prior.language<>current.language OR prior.dialect IS NOT current.dialect
                     OR prior.classification<>current.classification
                 )
                 UNION
                 SELECT prior.path_identity
                 FROM archaeology_source_units prior
                 LEFT JOIN archaeology_source_units current
                   ON current.generation_id=?1 AND current.path_identity=prior.path_identity
                 WHERE prior.generation_id=?2 AND current.path_identity IS NULL
             ) ORDER BY path_identity LIMIT ?3",
        )
        .map_err(|error| format!("Prepare changed archaeology sources: {error}"))?;
    let rows = statement
        .query_map(
            params![
                generation_id,
                ready,
                i64::try_from(limit).map_err(|_| "Archaeology source bound overflowed")?
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Query changed archaeology sources: {error}"))?;
    let paths = rows
        .map(|row| row.map_err(|error| format!("Read changed archaeology source: {error}")))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > limits.max_seed_paths {
        Err("Archaeology invalidation seed bound exceeded".into())
    } else {
        Ok(paths)
    }
}

fn load_generation_paths(
    connection: &Connection,
    generation_id: &str,
    limits: ArchaeologyInvalidationLimits,
) -> Result<Vec<String>, String> {
    let limit = limits
        .max_seed_paths
        .checked_add(1)
        .ok_or("Archaeology source bound overflowed")?;
    let mut statement = connection
        .prepare(
            "SELECT path_identity FROM archaeology_source_units WHERE generation_id=?1
             ORDER BY path_identity LIMIT ?2",
        )
        .map_err(|error| format!("Prepare archaeology generation paths: {error}"))?;
    let paths = statement
        .query_map(
            params![
                generation_id,
                i64::try_from(limit).map_err(|_| "Archaeology source bound overflowed")?
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Query archaeology generation paths: {error}"))?
        .map(|row| row.map_err(|error| format!("Read archaeology generation path: {error}")))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > limits.max_seed_paths {
        Err("Archaeology invalidation seed bound exceeded".into())
    } else {
        Ok(paths)
    }
}

fn derive_provable_dependencies(
    connection: &Connection,
    generation_id: &str,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(Vec<ArchaeologySourceDependency>, bool), String> {
    preflight_lineage_bounds(connection, generation_id, limits)?;
    let mut statement = connection
        .prepare(
            "SELECT source_unit_id,path_identity,include_lineage_json
             FROM archaeology_source_units WHERE generation_id=?1 ORDER BY source_unit_id",
        )
        .map_err(|error| format!("Prepare archaeology source lineage: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Query archaeology source lineage: {error}"))?;
    let mut units = BTreeMap::<String, (String, Vec<ArchaeologyAdapterLineage>)>::new();
    for row in rows {
        cancelled(cancellation)?;
        let (source_unit_id, path_identity, lineage_json) =
            row.map_err(|error| format!("Read archaeology source lineage: {error}"))?;
        let lineage = serde_json::from_str(&lineage_json)
            .map_err(|error| format!("Parse archaeology source lineage: {error}"))?;
        units.insert(source_unit_id, (path_identity, lineage));
    }

    let mut dependencies = BTreeSet::new();
    let mut unresolved = false;
    for (owner_unit_id, (owner_path, lineage)) in &units {
        for item in lineage {
            cancelled(cancellation)?;
            let kind = match item.kind {
                ArchaeologyLineageKind::Preprocessed => continue,
                ArchaeologyLineageKind::Include => ArchaeologySourceDependencyKind::Include,
                ArchaeologyLineageKind::Copybook => ArchaeologySourceDependencyKind::Copybook,
                ArchaeologyLineageKind::Macro => ArchaeologySourceDependencyKind::Macro,
            };
            if item.source_unit_id != *owner_unit_id || !item.has_honest_target() {
                unresolved = true;
                continue;
            }
            let Some(target_unit_id) = item.target_source_unit_id.as_deref() else {
                unresolved = true;
                continue;
            };
            let span_exists = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM archaeology_source_spans
                     WHERE generation_id=?1 AND source_unit_id=?2 AND span_id=?3)",
                    params![generation_id, owner_unit_id, item.evidence_span_id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| format!("Validate archaeology lineage evidence: {error}"))?;
            let Some((target_path, _)) = units.get(target_unit_id) else {
                unresolved = true;
                continue;
            };
            if !span_exists || owner_path == target_path {
                unresolved = true;
                continue;
            }
            dependencies.insert((owner_path.clone(), target_path.clone(), kind));
            if dependencies.len() > limits.max_dependencies {
                return Err("Archaeology invalidation dependency bound exceeded".into());
            }
        }
    }
    let mut statement = connection
        .prepare(
            "WITH fact_owners AS (
                 SELECT fact.fact_id,COUNT(DISTINCT unit.source_unit_id) AS source_count,
                        MIN(unit.path_identity) AS path_identity
                 FROM archaeology_facts fact
                 LEFT JOIN archaeology_evidence_links evidence
                   ON evidence.generation_id=fact.generation_id
                  AND evidence.owner_kind='fact' AND evidence.owner_id=fact.fact_id
                  AND evidence.evidence_kind='span'
                 LEFT JOIN archaeology_source_spans span
                   ON span.generation_id=evidence.generation_id
                  AND span.span_id=evidence.evidence_id
                 LEFT JOIN archaeology_source_units unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 WHERE fact.generation_id=?1 GROUP BY fact.fact_id
             )
             SELECT edge.kind,source.source_count,source.path_identity,
                    target.source_count,target.path_identity
             FROM archaeology_fact_edges edge
             JOIN fact_owners source ON source.fact_id=edge.from_fact_id
             JOIN fact_owners target ON target.fact_id=edge.to_fact_id
             WHERE edge.generation_id=?1 AND edge.unresolved_reason IS NULL
               AND edge.kind IN (
                   'includes','calls','reads','writes','defines','calculates',
                   'controls','branches_to'
               )
             ORDER BY edge.edge_id",
        )
        .map_err(|error| format!("Prepare archaeology typed fact dependencies: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|error| format!("Query archaeology typed fact dependencies: {error}"))?;
    for row in rows {
        cancelled(cancellation)?;
        let (edge_kind, source_count, source_path, target_count, target_path) =
            row.map_err(|error| format!("Read archaeology typed fact dependency: {error}"))?;
        let (Some(source_path), Some(target_path)) = (source_path, target_path) else {
            unresolved = true;
            continue;
        };
        if source_count != 1 || target_count != 1 {
            unresolved = true;
            continue;
        }
        if source_path == target_path {
            continue;
        }
        let kind = match edge_kind.as_str() {
            "includes" => ArchaeologySourceDependencyKind::Include,
            "calls" => ArchaeologySourceDependencyKind::Call,
            "reads" | "writes" | "defines" | "calculates" => ArchaeologySourceDependencyKind::Data,
            "controls" | "branches_to" => ArchaeologySourceDependencyKind::Symbol,
            _ => return Err("Archaeology normalized dependency kind is invalid".into()),
        };
        dependencies.insert((source_path, target_path, kind));
        if dependencies.len() > limits.max_dependencies {
            return Err("Archaeology invalidation dependency bound exceeded".into());
        }
    }
    let mut statement = connection
        .prepare(
            "WITH rule_owners AS (
                 SELECT rule.rule_id,COUNT(DISTINCT unit.source_unit_id) AS source_count,
                        MIN(unit.path_identity) AS path_identity
                 FROM archaeology_rules rule
                 LEFT JOIN archaeology_rule_clauses clause
                   ON clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id
                 LEFT JOIN archaeology_evidence_links evidence
                   ON evidence.generation_id=clause.generation_id
                  AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                  AND evidence.evidence_kind='span'
                 LEFT JOIN archaeology_source_spans span
                   ON span.generation_id=evidence.generation_id AND span.span_id=evidence.evidence_id
                 LEFT JOIN archaeology_source_units unit
                   ON unit.generation_id=span.generation_id
                  AND unit.source_unit_id=span.source_unit_id
                 WHERE rule.generation_id=?1 GROUP BY rule.rule_id
             )
             SELECT source.source_count,source.path_identity,
                    target.source_count,target.path_identity
             FROM archaeology_rule_relations relation
             JOIN rule_owners source ON source.rule_id=relation.from_rule_id
             JOIN rule_owners target ON target.rule_id=relation.to_rule_id
             WHERE relation.generation_id=?1 AND relation.kind='depends_on'
             ORDER BY relation.relation_id",
        )
        .map_err(|error| format!("Prepare archaeology typed rule dependencies: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| format!("Query archaeology typed rule dependencies: {error}"))?;
    for row in rows {
        cancelled(cancellation)?;
        let (source_count, source_path, target_count, target_path) =
            row.map_err(|error| format!("Read archaeology typed rule dependency: {error}"))?;
        let (Some(source_path), Some(target_path)) = (source_path, target_path) else {
            unresolved = true;
            continue;
        };
        if source_count != 1 || target_count != 1 {
            unresolved = true;
            continue;
        }
        if source_path == target_path {
            continue;
        }
        dependencies.insert((
            source_path,
            target_path,
            ArchaeologySourceDependencyKind::Rule,
        ));
        if dependencies.len() > limits.max_dependencies {
            return Err("Archaeology invalidation dependency bound exceeded".into());
        }
    }
    Ok((
        dependencies
            .into_iter()
            .map(
                |(dependent_path_identity, prerequisite_path_identity, kind)| {
                    ArchaeologySourceDependency {
                        dependent_path_identity,
                        prerequisite_path_identity,
                        kind,
                    }
                },
            )
            .collect(),
        unresolved,
    ))
}

fn generation_has_unresolved_lineage(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<bool, String> {
    require_generation_scope(connection, repository_id, generation_id)?;
    Ok(derive_provable_dependencies(connection, generation_id, cancellation, limits)?.1)
}

fn validate_seed_scope(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    ready_generation_id: Option<&str>,
    seeds: &[String],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(), String> {
    if seeds.len() > limits.max_seed_paths {
        return Err("Archaeology invalidation seed bound exceeded".into());
    }
    let mut unique = BTreeSet::new();
    for seed in seeds {
        cancelled(cancellation)?;
        if !unique.insert(seed) {
            return Err("Archaeology invalidation seed identity is duplicated".into());
        }
        let exists = connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM archaeology_source_units unit
                     JOIN archaeology_generations generation
                       ON generation.generation_id=unit.generation_id
                     WHERE generation.repository_id=?1 AND unit.path_identity=?2
                       AND (unit.generation_id=?3 OR unit.generation_id=?4)
                 )",
                params![repository_id, seed, generation_id, ready_generation_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("Validate archaeology invalidation seed: {error}"))?;
        if !exists {
            return Err("Archaeology invalidation seed is outside generation scope".into());
        }
    }
    Ok(())
}

fn load_global_paths(
    connection: &Connection,
    generation_id: &str,
    ready_generation_id: Option<&str>,
    limits: ArchaeologyInvalidationLimits,
) -> Result<Vec<ArchaeologyInvalidatedPath>, String> {
    let mut statement = connection
        .prepare(
            "SELECT path_identity FROM archaeology_source_units
             WHERE generation_id=?1 OR generation_id=?2
             GROUP BY path_identity ORDER BY path_identity LIMIT ?3",
        )
        .map_err(|error| format!("Prepare global archaeology source paths: {error}"))?;
    let limit = limits
        .max_invalidated_paths
        .checked_add(1)
        .ok_or("Archaeology invalidation path bound overflowed")?;
    let rows = statement
        .query_map(
            params![
                generation_id,
                ready_generation_id,
                i64::try_from(limit).map_err(|_| "Archaeology path bound overflowed")?
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Query global archaeology source paths: {error}"))?;
    let paths = rows
        .map(|row| {
            row.map(|path_identity| ArchaeologyInvalidatedPath {
                path_identity,
                depth: 0,
                via: Vec::new(),
            })
            .map_err(|error| format!("Read global archaeology source path: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > limits.max_invalidated_paths {
        Err("Archaeology invalidation path bound exceeded".into())
    } else {
        Ok(paths)
    }
}

fn path_exists(
    connection: &Connection,
    generation_id: &str,
    path_identity: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM archaeology_source_units
             WHERE generation_id=?1 AND path_identity=?2)",
            params![generation_id, path_identity],
            |row| row.get(0),
        )
        .map_err(|error| format!("Check archaeology source path: {error}"))
}

fn require_generation_scope(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2)",
            params![repository_id, generation_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Validate archaeology generation scope: {error}"))?;
    if exists {
        Ok(())
    } else {
        Err("Archaeology generation is outside repository scope".into())
    }
}

fn require_ready_generation_scope(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2 AND status='ready')",
            params![repository_id, generation_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Validate archaeology ready generation: {error}"))?;
    if exists {
        Ok(())
    } else {
        Err("Archaeology ready generation is outside repository scope".into())
    }
}

fn require_staging_generation_scope(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<GenerationIdentity, String> {
    require_generation_scope(connection, repository_id, generation_id)?;
    let status = connection
        .query_row(
            "SELECT status
             FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, generation_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Validate staging archaeology generation scope: {error}"))?;
    if status != "staging" {
        return Err(
            "Archaeology metadata replacement requires its exact staging generation".into(),
        );
    }
    load_generation_identity(connection, repository_id, generation_id)
}

fn load_generation_identity(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
) -> Result<GenerationIdentity, String> {
    connection
        .query_row(
            "SELECT revision_sha,schema_version,parser_identity,algorithm_identity,config_identity
             FROM archaeology_generations WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, generation_id],
            |row| {
                Ok(GenerationIdentity {
                    revision_sha: row.get(0)?,
                    schema_version: row.get(1)?,
                    parser_identity: row.get(2)?,
                    algorithm_identity: row.get(3)?,
                    config_identity: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology generation identity: {error}"))?
        .ok_or_else(|| "Archaeology generation is outside repository scope".into())
}

fn validate_canonical_inputs(
    connection: &Connection,
    generation_id: &str,
    inputs: &[ArchaeologyGenerationInput],
    generation: &GenerationIdentity,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(), String> {
    if inputs.len() > limits.max_seed_paths {
        return Err("Archaeology generation input bound exceeded".into());
    }
    let input_bytes = inputs.iter().try_fold(0_usize, |total, input| {
        total
            .checked_add(input.identity.len())
            .and_then(|value| value.checked_add(input.scope.as_deref().map_or(0, str::len)))
            .and_then(|value| value.checked_add(16))
            .ok_or("Archaeology generation input byte bound exceeded")
    })?;
    if input_bytes > limits.max_input_bytes {
        return Err("Archaeology generation input byte bound exceeded".into());
    }
    let required_keys = [
        (ArchaeologyGenerationInputKind::Head, None),
        (ArchaeologyGenerationInputKind::Ignore, None),
        (ArchaeologyGenerationInputKind::Config, None),
        (ArchaeologyGenerationInputKind::Schema, None),
        (ArchaeologyGenerationInputKind::Algorithm, None),
        (ArchaeologyGenerationInputKind::Parser, Some("global")),
        (
            ArchaeologyGenerationInputKind::SynthesisPolicy,
            Some("global"),
        ),
    ];
    if required_keys.iter().any(|(kind, scope)| {
        !inputs
            .iter()
            .any(|input| input.kind == *kind && input.scope.as_deref() == *scope)
    }) {
        return Err("Archaeology generation input set is incomplete".into());
    }
    let schema_identity = format!("schema:v{}", generation.schema_version);
    let required = [
        (
            ArchaeologyGenerationInputKind::Head,
            None,
            generation.revision_sha.as_str(),
        ),
        (
            ArchaeologyGenerationInputKind::Config,
            None,
            generation.config_identity.as_str(),
        ),
        (
            ArchaeologyGenerationInputKind::Schema,
            None,
            schema_identity.as_str(),
        ),
        (
            ArchaeologyGenerationInputKind::Algorithm,
            None,
            generation.algorithm_identity.as_str(),
        ),
        (
            ArchaeologyGenerationInputKind::Parser,
            Some("global"),
            generation.parser_identity.as_str(),
        ),
    ];
    for (kind, scope, identity) in required {
        if !inputs.iter().any(|input| {
            input.kind == kind && input.scope.as_deref() == scope && input.identity == identity
        }) {
            return Err(
                "Archaeology generation inputs do not reconcile with generation identity".into(),
            );
        }
    }
    let synthesis_identity = inputs
        .iter()
        .find(|input| {
            input.kind == ArchaeologyGenerationInputKind::SynthesisPolicy
                && input.scope.as_deref() == Some("global")
        })
        .map(|input| input.identity.as_str())
        .ok_or("Archaeology generation input set is incomplete")?;
    let mismatched_synthesis = connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rules
             WHERE generation_id=?1 AND synthesis_identity IS NOT NULL
               AND synthesis_identity<>?2",
            params![generation_id, synthesis_identity],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Reconcile archaeology synthesis identity: {error}"))?;
    if mismatched_synthesis != 0 {
        return Err("Archaeology synthesis input does not reconcile with generation rules".into());
    }
    Ok(())
}

fn preflight_generation_input_bounds(
    connection: &Connection,
    generation_id: &str,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(), String> {
    let (rows, bytes) = connection
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(
                 LENGTH(CAST(input_kind AS BLOB))+LENGTH(CAST(scope_identity AS BLOB))+
                 LENGTH(CAST(input_identity AS BLOB))+16),0)
             FROM archaeology_generation_inputs WHERE generation_id=?1",
            [generation_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| format!("Preflight archaeology generation inputs: {error}"))?;
    if rows < 0 || rows as usize > limits.max_seed_paths {
        return Err("Archaeology generation input bound exceeded".into());
    }
    if bytes < 0 || bytes as usize > limits.max_input_bytes {
        return Err("Archaeology generation input byte bound exceeded".into());
    }
    Ok(())
}

fn preflight_dependency_bounds(
    connection: &Connection,
    generation_id: &str,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(), String> {
    let rows = connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_source_dependencies WHERE generation_id=?1",
            [generation_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Preflight archaeology source dependencies: {error}"))?;
    if rows < 0 || rows as usize > limits.max_dependencies {
        Err("Archaeology invalidation dependency bound exceeded".into())
    } else {
        Ok(())
    }
}

fn preflight_lineage_bounds(
    connection: &Connection,
    generation_id: &str,
    limits: ArchaeologyInvalidationLimits,
) -> Result<(), String> {
    let (rows, bytes) = connection
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(LENGTH(CAST(source_unit_id AS BLOB))+
                 LENGTH(CAST(path_identity AS BLOB))+
                 LENGTH(CAST(include_lineage_json AS BLOB))+32),0)
             FROM archaeology_source_units WHERE generation_id=?1",
            [generation_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| format!("Preflight archaeology source lineage: {error}"))?;
    if rows < 0 || rows as usize > limits.max_invalidated_paths {
        return Err("Archaeology invalidation source-unit bound exceeded".into());
    }
    if bytes < 0 || bytes as usize > limits.max_input_bytes {
        return Err("Archaeology invalidation source-lineage byte bound exceeded".into());
    }
    Ok(())
}

fn work_items(
    plan: &ArchaeologyInvalidationPlan,
) -> Result<Vec<ArchaeologyRefreshWorkItem>, String> {
    let mut work = Vec::new();
    match plan.decision.mode {
        ArchaeologyInputInvalidationMode::NoOp => {}
        ArchaeologyInputInvalidationMode::GlobalRebuild => {
            if plan.invalidated_paths.is_empty() {
                work.push(ArchaeologyRefreshWorkItem {
                    ordinal: 1,
                    target_kind: "global".into(),
                    target_identity: "global".into(),
                    action: "global_rebuild".into(),
                    depth: 0,
                    reasons: plan_reasons(plan),
                });
            } else {
                for path in &plan.invalidated_paths {
                    work.push(ArchaeologyRefreshWorkItem {
                        ordinal: u64::try_from(work.len() + 1)
                            .map_err(|_| "Archaeology refresh ordinal overflowed")?,
                        target_kind: "source_path".into(),
                        target_identity: path.path_identity.clone(),
                        action: if plan.removed_path_identities.contains(&path.path_identity) {
                            "remove".into()
                        } else {
                            "reprocess".into()
                        },
                        depth: path.depth,
                        reasons: plan_reasons(plan),
                    });
                }
            }
        }
        ArchaeologyInputInvalidationMode::SynthesisOnly => {
            for scope in &plan.decision.synthesis_policy_scopes {
                work.push(ArchaeologyRefreshWorkItem {
                    ordinal: u64::try_from(work.len() + 1)
                        .map_err(|_| "Archaeology refresh ordinal overflowed")?,
                    target_kind: "synthesis_scope".into(),
                    target_identity: scope.clone(),
                    action: "synthesize".into(),
                    depth: 0,
                    reasons: vec!["synthesis_policy_changed".into()],
                });
            }
        }
        ArchaeologyInputInvalidationMode::Scoped => {
            for path in &plan.invalidated_paths {
                let mut reasons = path
                    .via
                    .iter()
                    .map(|kind| format!("dependency:{}", dependency_kind_name(*kind)))
                    .collect::<Vec<_>>();
                if path.depth == 0 {
                    reasons.push("changed_source".into());
                }
                reasons.sort();
                reasons.dedup();
                work.push(ArchaeologyRefreshWorkItem {
                    ordinal: u64::try_from(work.len() + 1)
                        .map_err(|_| "Archaeology refresh ordinal overflowed")?,
                    target_kind: "source_path".into(),
                    target_identity: path.path_identity.clone(),
                    action: if plan.removed_path_identities.contains(&path.path_identity) {
                        "remove".into()
                    } else {
                        "reprocess".into()
                    },
                    depth: path.depth,
                    reasons,
                });
            }
            if plan
                .decision
                .changed_kinds
                .contains(&ArchaeologyGenerationInputKind::SynthesisPolicy)
            {
                for scope in &plan.decision.synthesis_policy_scopes {
                    work.push(ArchaeologyRefreshWorkItem {
                        ordinal: u64::try_from(work.len() + 1)
                            .map_err(|_| "Archaeology refresh ordinal overflowed")?,
                        target_kind: "synthesis_scope".into(),
                        target_identity: scope.clone(),
                        action: "synthesize".into(),
                        depth: 0,
                        reasons: vec!["synthesis_policy_changed".into()],
                    });
                }
            }
        }
    }
    Ok(work)
}

fn plan_reasons(plan: &ArchaeologyInvalidationPlan) -> Vec<String> {
    let mut reasons = plan
        .decision
        .changed_kinds
        .iter()
        .map(|kind| format!("input:{}", input_kind_name(*kind)))
        .collect::<Vec<_>>();
    if plan.unresolved_lineage {
        reasons.push("unresolved_lineage".into());
    }
    if plan.prior_ready_generation_id.is_none() {
        reasons.push("missing_ready_generation".into());
    }
    if reasons.is_empty() {
        reasons.push("unsafe_scoped_invalidation".into());
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

fn refresh_plan_identity(plan: &ArchaeologyInvalidationPlan) -> String {
    let mut digest = Sha256::new();
    for value in [
        "archaeology-refresh-plan:v1",
        plan.repository_id.as_str(),
        plan.generation_id.as_str(),
        plan.prior_ready_generation_id.as_deref().unwrap_or(""),
        match plan.decision.mode {
            ArchaeologyInputInvalidationMode::NoOp => "no_op",
            ArchaeologyInputInvalidationMode::SynthesisOnly => "synthesis_only",
            ArchaeologyInputInvalidationMode::Scoped => "scoped",
            ArchaeologyInputInvalidationMode::GlobalRebuild => "global_rebuild",
        },
    ] {
        update_digest_field(&mut digest, value);
    }
    for kind in &plan.decision.changed_kinds {
        update_digest_field(&mut digest, input_kind_name(*kind));
    }
    for scope in &plan.decision.parser_scopes {
        update_digest_field(&mut digest, scope);
    }
    for scope in &plan.decision.synthesis_policy_scopes {
        update_digest_field(&mut digest, scope);
    }
    for path in &plan.invalidated_paths {
        update_digest_field(&mut digest, &path.path_identity);
        update_digest_field(&mut digest, &path.depth.to_string());
        for kind in &path.via {
            update_digest_field(&mut digest, dependency_kind_name(*kind));
        }
    }
    for path in &plan.removed_path_identities {
        update_digest_field(&mut digest, path);
    }
    format!("sha256:{:x}", digest.finalize())
}

fn update_digest_field(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

fn load_refresh_work_items(
    connection: &Connection,
    job_id: &str,
    plan_identity: &str,
    pending_only: bool,
    limit: i64,
    phase: RefreshWorkPhase,
) -> Result<Vec<ArchaeologyRefreshWorkItem>, String> {
    let sql = match (pending_only, phase) {
        (true, RefreshWorkPhase::All) => {
            "SELECT ordinal,target_kind,target_identity,action,depth,reasons_json
             FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2 AND completed=0 ORDER BY ordinal LIMIT ?3"
        }
        (false, RefreshWorkPhase::All) => {
            "SELECT ordinal,target_kind,target_identity,action,depth,reasons_json
             FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2 ORDER BY ordinal LIMIT ?3"
        }
        (true, RefreshWorkPhase::Parse) => {
            "SELECT ordinal,target_kind,target_identity,action,depth,reasons_json
             FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2 AND completed=0
               AND target_kind IN ('source_path','global') ORDER BY ordinal LIMIT ?3"
        }
        (false, RefreshWorkPhase::Parse) => {
            "SELECT ordinal,target_kind,target_identity,action,depth,reasons_json
             FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2
               AND target_kind IN ('source_path','global') ORDER BY ordinal LIMIT ?3"
        }
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| format!("Prepare archaeology refresh work: {error}"))?;
    let rows = statement
        .query_map(params![job_id, plan_identity, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| format!("Query archaeology refresh work: {error}"))?;
    let mut work = Vec::new();
    for row in rows {
        let (ordinal, target_kind, target_identity, action, depth, reasons) =
            row.map_err(|error| format!("Read archaeology refresh work: {error}"))?;
        work.push(ArchaeologyRefreshWorkItem {
            ordinal: u64::try_from(ordinal)
                .map_err(|_| "Archaeology refresh ordinal is invalid")?,
            target_kind,
            target_identity,
            action,
            depth: usize::try_from(depth).map_err(|_| "Archaeology refresh depth is invalid")?,
            reasons: serde_json::from_str(&reasons)
                .map_err(|error| format!("Parse archaeology refresh reasons: {error}"))?,
        });
    }
    Ok(work)
}

fn count_pending_refresh_work(
    connection: &Connection,
    job_id: &str,
    plan_identity: &str,
    phase: RefreshWorkPhase,
) -> Result<i64, String> {
    let sql = match phase {
        RefreshWorkPhase::All => {
            "SELECT COUNT(*) FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2 AND completed=0"
        }
        RefreshWorkPhase::Parse => {
            "SELECT COUNT(*) FROM archaeology_refresh_work_items
             WHERE job_id=?1 AND plan_identity=?2 AND completed=0
               AND target_kind IN ('source_path','global')"
        }
    };
    connection
        .query_row(sql, params![job_id, plan_identity], |row| row.get(0))
        .map_err(|error| format!("Count archaeology refresh work: {error}"))
}

fn require_active_job_scope(
    connection: &Connection,
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
) -> Result<(), String> {
    let valid = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM archaeology_jobs job
                 JOIN archaeology_generations generation
                   ON generation.generation_id=job.generation_id
                 WHERE job.job_id=?1 AND job.repository_id=?2 AND job.generation_id=?3
                   AND job.owner_id=?4 AND job.state='running'
                   AND job.cancellation_requested=0 AND generation.status='staging'
                   AND generation.repository_id=job.repository_id
             )",
            params![job_id, repository_id, generation_id, owner_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Validate archaeology refresh job lease: {error}"))?;
    if valid {
        Ok(())
    } else {
        Err("Archaeology refresh job lease is unavailable".into())
    }
}

fn validate_digest_identity(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(format!("Archaeology {label} identity is invalid"))
    }
}

fn dependency_evidence_identity(
    repository_id: &str,
    dependency: &ArchaeologySourceDependency,
) -> String {
    let mut digest = Sha256::new();
    for value in [
        "archaeology-source-dependency:v2",
        repository_id,
        dependency.dependent_path_identity.as_str(),
        dependency.prerequisite_path_identity.as_str(),
        dependency_kind_name(dependency.kind),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn input_kind_name(kind: ArchaeologyGenerationInputKind) -> &'static str {
    match kind {
        ArchaeologyGenerationInputKind::Head => "head",
        ArchaeologyGenerationInputKind::Ignore => "ignore",
        ArchaeologyGenerationInputKind::Config => "config",
        ArchaeologyGenerationInputKind::Parser => "parser",
        ArchaeologyGenerationInputKind::Schema => "schema",
        ArchaeologyGenerationInputKind::Algorithm => "algorithm",
        ArchaeologyGenerationInputKind::SynthesisPolicy => "synthesis_policy",
    }
}

fn parse_input_kind(value: &str) -> Result<ArchaeologyGenerationInputKind, String> {
    match value {
        "head" => Ok(ArchaeologyGenerationInputKind::Head),
        "ignore" => Ok(ArchaeologyGenerationInputKind::Ignore),
        "config" => Ok(ArchaeologyGenerationInputKind::Config),
        "parser" => Ok(ArchaeologyGenerationInputKind::Parser),
        "schema" => Ok(ArchaeologyGenerationInputKind::Schema),
        "algorithm" => Ok(ArchaeologyGenerationInputKind::Algorithm),
        "synthesis_policy" => Ok(ArchaeologyGenerationInputKind::SynthesisPolicy),
        _ => Err("Archaeology generation input kind is invalid".into()),
    }
}

fn dependency_kind_name(kind: ArchaeologySourceDependencyKind) -> &'static str {
    match kind {
        ArchaeologySourceDependencyKind::Include => "include",
        ArchaeologySourceDependencyKind::Copybook => "copybook",
        ArchaeologySourceDependencyKind::Macro => "macro",
        ArchaeologySourceDependencyKind::Symbol => "symbol",
        ArchaeologySourceDependencyKind::Call => "call",
        ArchaeologySourceDependencyKind::Data => "data",
        ArchaeologySourceDependencyKind::Rule => "rule",
    }
}

fn parse_dependency_kind(value: &str) -> Result<ArchaeologySourceDependencyKind, String> {
    match value {
        "include" => Ok(ArchaeologySourceDependencyKind::Include),
        "copybook" => Ok(ArchaeologySourceDependencyKind::Copybook),
        "macro" => Ok(ArchaeologySourceDependencyKind::Macro),
        "symbol" => Ok(ArchaeologySourceDependencyKind::Symbol),
        "call" => Ok(ArchaeologySourceDependencyKind::Call),
        "data" => Ok(ArchaeologySourceDependencyKind::Data),
        "rule" => Ok(ArchaeologySourceDependencyKind::Rule),
        _ => Err("Archaeology source dependency kind is invalid".into()),
    }
}

fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology invalidation persistence cancelled".into())
    } else {
        Ok(())
    }
}
