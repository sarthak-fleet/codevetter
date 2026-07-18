//! Append-only SQLite persistence for rule review, alias, and continuity history.
//!
//! Callers should wrap writes in an IMMEDIATE transaction. Database triggers
//! provide a second fail-closed sequence and append-only boundary.

use super::contracts::ArchaeologyRuleLifecycle;
use super::lifecycle::{
    evaluate_snapshot_compatibility, project_lifecycle, validate_rule_aliases,
    ArchaeologyCompatibilityMismatch, ArchaeologyCompatibilityOutcome, ArchaeologyLifecycleAction,
    ArchaeologyLifecycleEvent, ArchaeologyLifecycleProjection, ArchaeologyReviewerKind,
    ArchaeologyReviewerProvenance, ArchaeologyRuleAlias, ArchaeologyRuleSnapshotIdentity,
    MAX_ALIASES_PER_REPOSITORY, MAX_LIFECYCLE_EVENTS_PER_RULE,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_EVENT_JSON_BYTES: usize = 16 * 1024;
const MAX_TIMESTAMP_BYTES: usize = 128;
const MAX_LIFECYCLE_RULES_PER_GENERATION: usize = 100_000;
const MAX_RECONCILIATION_EVENTS: usize = 1_000_000;
const DIGEST_PREFIX: &str = "sha256:";

#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyLifecycleAppend<'a> {
    pub event_id: &'a str,
    pub repository_id: &'a str,
    pub generation_id: &'a str,
    pub rule_id: &'a str,
    pub stable_rule_identity: &'a str,
    pub expected_previous_sequence: u64,
    pub expected_prior_event_id: Option<&'a str>,
    pub related_generation_id: Option<&'a str>,
    pub related_rule_id: Option<&'a str>,
    pub provenance: ArchaeologyReviewerProvenance,
    pub action: ArchaeologyLifecycleAction,
    pub created_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyStoredLifecycleProjection {
    pub projected: ArchaeologyLifecycleProjection,
    pub effective_lifecycle: ArchaeologyRuleLifecycle,
    pub description_changed: bool,
    pub compatibility_mismatches: Vec<ArchaeologyCompatibilityMismatch>,
    pub current_snapshot: ArchaeologyRuleSnapshotIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchaeologyAliasAction {
    Linked,
    Unlinked,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyAliasAppend<'a> {
    pub event_id: &'a str,
    pub repository_id: &'a str,
    pub generation_id: &'a str,
    pub alias_rule_id: &'a str,
    pub alias_rule_identity: &'a str,
    pub canonical_rule_id: &'a str,
    pub canonical_rule_identity: &'a str,
    pub expected_previous_sequence: u64,
    pub action: ArchaeologyAliasAction,
    pub provenance: ArchaeologyReviewerProvenance,
    pub created_at: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchaeologyContinuityKind {
    SameEvidence,
    Supersedes,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyContinuityAppend<'a> {
    pub repository_id: &'a str,
    pub continuity_identity: &'a str,
    pub predecessor_rule_id: &'a str,
    pub predecessor_rule_identity: &'a str,
    pub successor_rule_id: &'a str,
    pub successor_rule_identity: &'a str,
    pub predecessor_generation_id: &'a str,
    pub successor_generation_id: &'a str,
    pub kind: ArchaeologyContinuityKind,
    pub evidence_identity: &'a str,
    pub provenance: ArchaeologyReviewerProvenance,
    pub created_at: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchaeologyExplicitSupersession<'a> {
    pub repository_id: &'a str,
    pub predecessor_generation_id: &'a str,
    pub predecessor_rule_id: &'a str,
    pub predecessor_rule_identity: &'a str,
    pub expected_predecessor_sequence: u64,
    pub expected_predecessor_event_id: Option<&'a str>,
    pub successor_generation_id: &'a str,
    pub successor_rule_id: &'a str,
    pub successor_rule_identity: &'a str,
    pub continuity_identity: &'a str,
    pub successor_evidence_identity: &'a str,
    pub provenance: ArchaeologyReviewerProvenance,
    pub created_at: &'a str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReviewProvenance {
    reviewer: ArchaeologyReviewerProvenance,
    rule_kind_identity: String,
}

#[derive(Debug, Clone)]
struct StoredReviewRow {
    generation_id: String,
    event: ArchaeologyLifecycleEvent,
    snapshot: ArchaeologyRuleSnapshotIdentity,
    prior_event_id: Option<String>,
}

#[derive(Debug)]
struct StoredRuleSnapshot {
    generated_rule_id: String,
    identity: ArchaeologyRuleSnapshotIdentity,
}

#[derive(Debug, Clone)]
struct StoredAliasRow {
    event_id: String,
    repository_id: String,
    event_stream_identity: String,
    logical_sequence: u64,
    action: ArchaeologyAliasAction,
    alias_rule_identity: String,
    alias_continuity_identity: String,
    canonical_rule_identity: String,
    canonical_continuity_identity: String,
    provenance: ArchaeologyReviewerProvenance,
}

#[derive(Debug)]
struct RawReviewRow {
    event_id: String,
    generation_id: String,
    stable_rule_identity: String,
    logical_sequence: u64,
    decision: String,
    body: Option<String>,
    evidence_identity: String,
    contradiction_identity: String,
    description_identity: String,
    continuity_identity: String,
    parser_identity: String,
    prior_event_id: Option<String>,
    related_rule_identity: Option<String>,
    related_continuity_identity: Option<String>,
    reviewer_id: String,
    actor_kind: String,
    reviewer_provenance_json: String,
}

pub(crate) fn append_lifecycle_event(
    transaction: &Transaction<'_>,
    input: ArchaeologyLifecycleAppend<'_>,
) -> Result<ArchaeologyStoredLifecycleProjection, String> {
    validate_digest("lifecycle event", input.event_id)?;
    validate_timestamp(input.created_at)?;
    input.provenance.validate()?;
    if matches!(input.provenance.kind, ArchaeologyReviewerKind::Model)
        && !matches!(input.action, ArchaeologyLifecycleAction::Annotate { .. })
    {
        return Err("A model may only annotate lifecycle history".into());
    }

    let current = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.generation_id,
        input.rule_id,
        input.stable_rule_identity,
    )?;
    let stream_identity =
        lifecycle_stream_identity(input.repository_id, input.stable_rule_identity);
    let existing = load_review_rows(transaction, input.repository_id, &stream_identity)?;
    let prior = existing.last().map(|row| row.event.event_id.as_str());
    let previous_sequence = existing.last().map_or(0, |row| row.event.sequence);
    if input.expected_previous_sequence != previous_sequence
        || input.expected_prior_event_id != prior
    {
        return Err("Lifecycle append compare-and-swap failed".into());
    }

    let related = match &input.action {
        ArchaeologyLifecycleAction::Supersede { successor_rule_id } => {
            let generation_id = input
                .related_generation_id
                .ok_or("Lifecycle supersession requires an exact successor generation")?;
            let rule_id = input
                .related_rule_id
                .ok_or("Lifecycle supersession requires an exact successor rule occurrence")?;
            let successor = load_rule_snapshot(
                transaction,
                input.repository_id,
                generation_id,
                rule_id,
                successor_rule_id,
            )?;
            require_unique_supersession_edge(
                transaction,
                input.repository_id,
                input.generation_id,
                &current,
                generation_id,
                &successor,
            )?;
            Some(successor)
        }
        _ if input.related_generation_id.is_some() || input.related_rule_id.is_some() => {
            return Err("Only supersession may name a related rule occurrence".into())
        }
        _ => None,
    };
    let sequence = previous_sequence
        .checked_add(1)
        .ok_or("Lifecycle sequence overflowed")?;
    let event = ArchaeologyLifecycleEvent {
        event_id: input.event_id.into(),
        repository_id: input.repository_id.into(),
        rule_id: input.stable_rule_identity.into(),
        sequence,
        expected_previous_sequence: input.expected_previous_sequence,
        provenance: input.provenance.clone(),
        action: input.action.clone(),
    };
    persist_lifecycle_event(
        transaction,
        &current,
        &existing,
        input.generation_id,
        input.created_at,
        event,
        input.expected_prior_event_id,
        related.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_lifecycle_event(
    transaction: &Transaction<'_>,
    current: &StoredRuleSnapshot,
    existing: &[StoredReviewRow],
    generation_id: &str,
    created_at: &str,
    event: ArchaeologyLifecycleEvent,
    prior_event_id: Option<&str>,
    related: Option<&StoredRuleSnapshot>,
) -> Result<ArchaeologyStoredLifecycleProjection, String> {
    let mut prospective_rows = existing.to_vec();
    prospective_rows.push(StoredReviewRow {
        generation_id: generation_id.into(),
        event: event.clone(),
        snapshot: current.identity.clone(),
        prior_event_id: prior_event_id.map(str::to_owned),
    });
    let projected = project_stored_lifecycle(current, &prospective_rows)?
        .ok_or_else(|| "Appended lifecycle stream is unavailable".to_string())?;
    let stored_provenance = encode_json(
        "reviewer provenance",
        &StoredReviewProvenance {
            reviewer: event.provenance.clone(),
            rule_kind_identity: current.identity.rule_kind_identity.clone(),
        },
    )?;
    let (decision, body) = action_columns(&event.action);
    let (related_rule_identity, related_continuity_identity) = related
        .map(|snapshot| {
            (
                Some(snapshot.identity.rule_id.as_str()),
                Some(snapshot.identity.continuity_identity.as_str()),
            )
        })
        .unwrap_or((None, None));
    transaction
        .execute(
            "INSERT INTO archaeology_rule_review_events
             (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,body,
              evidence_identity,created_at,event_schema_version,event_stream_identity,
              logical_sequence,stable_rule_identity,contradiction_identity,
              description_identity,continuity_identity,parser_identity,prior_event_id,
              related_rule_identity,related_continuity_identity,actor_kind,
              reviewer_provenance_json,legacy_stale)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,2,?10,?11,?12,?13,?14,?15,
                     ?16,?17,?18,?19,?20,?21,0)",
            params![
                event.event_id,
                event.repository_id,
                current.generated_rule_id,
                generation_id,
                decision,
                event.provenance.actor_id,
                body,
                current.identity.evidence_identity,
                created_at,
                lifecycle_stream_identity(&event.repository_id, &event.rule_id),
                event.sequence,
                current.identity.rule_id,
                current.identity.contradiction_identity,
                current.identity.description_identity,
                current.identity.continuity_identity,
                current.identity.parser_compatibility_identity,
                prior_event_id,
                related_rule_identity,
                related_continuity_identity,
                actor_kind(&event.provenance, true)?,
                stored_provenance,
            ],
        )
        .map_err(|error| format!("Append archaeology lifecycle event: {error}"))?;
    Ok(projected)
}

pub(crate) fn project_current_lifecycle(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    rule_id: &str,
    stable_rule_identity: &str,
) -> Result<Option<ArchaeologyStoredLifecycleProjection>, String> {
    let current = load_rule_snapshot(
        transaction,
        repository_id,
        generation_id,
        rule_id,
        stable_rule_identity,
    )?;
    let stream_identity = lifecycle_stream_identity(repository_id, stable_rule_identity);
    let rows = load_review_rows(transaction, repository_id, &stream_identity)?;
    project_stored_lifecycle(&current, &rows)
}

/// Materializes the deterministic candidate baseline only when a real review
/// stream is about to be mutated. The caller owns the surrounding transaction,
/// so a failed human action rolls this baseline back with it.
pub(crate) fn ensure_candidate_lifecycle(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    rule_id: &str,
    stable_rule_identity: &str,
    created_at: &str,
) -> Result<ArchaeologyStoredLifecycleProjection, String> {
    let current = load_rule_snapshot(
        transaction,
        repository_id,
        generation_id,
        rule_id,
        stable_rule_identity,
    )?;
    let stream_identity = lifecycle_stream_identity(repository_id, stable_rule_identity);
    let existing = load_review_rows(transaction, repository_id, &stream_identity)?;
    if !existing.is_empty() {
        return project_stored_lifecycle(&current, &existing)?
            .ok_or_else(|| "Lifecycle stream is unavailable".to_string());
    }
    let provenance = ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::DeterministicPolicy,
        actor_id: "codevetter:local".into(),
        authority_id: Some("policy:archaeology-lifecycle-reconciliation:v1".into()),
    };
    let event_id = digest_fields(
        "archaeology-lifecycle-reconciliation-event:v1",
        &[
            repository_id,
            generation_id,
            stable_rule_identity,
            "candidate",
            &current.identity.evidence_identity,
            &current.identity.parser_compatibility_identity,
            &current.identity.contradiction_identity,
        ],
    );
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &event_id,
            repository_id,
            generation_id,
            rule_id,
            stable_rule_identity,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance,
            action: ArchaeologyLifecycleAction::Candidate,
            created_at,
        },
    )
}

fn project_stored_lifecycle(
    current: &StoredRuleSnapshot,
    rows: &[StoredReviewRow],
) -> Result<Option<ArchaeologyStoredLifecycleProjection>, String> {
    if rows.is_empty() {
        return Ok(None);
    }
    let events = rows.iter().map(|row| row.event.clone()).collect::<Vec<_>>();
    let projected = project_lifecycle(&events)?;
    let previous = &rows
        .iter()
        .rev()
        .find(|row| {
            !matches!(
                row.event.action,
                ArchaeologyLifecycleAction::Annotate { .. }
            )
        })
        .ok_or("Lifecycle stream has no state event")?
        .snapshot;
    let compatibility = evaluate_snapshot_compatibility(
        previous,
        &current.identity,
        projected.lifecycle.clone(),
        None,
    )?;
    let (effective_lifecycle, description_changed, compatibility_mismatches) = match compatibility {
        ArchaeologyCompatibilityOutcome::Compatible {
            lifecycle,
            description_changed,
        } => (lifecycle, description_changed, Vec::new()),
        ArchaeologyCompatibilityOutcome::ReviewNeeded { reasons } => {
            (ArchaeologyRuleLifecycle::ReviewNeeded, false, reasons)
        }
        ArchaeologyCompatibilityOutcome::Conflicted { reasons } => {
            (ArchaeologyRuleLifecycle::Conflicted, false, reasons)
        }
        ArchaeologyCompatibilityOutcome::Superseded { .. } => {
            return Err("Current lifecycle projection cannot infer a successor".into())
        }
    };
    Ok(Some(ArchaeologyStoredLifecycleProjection {
        projected,
        effective_lifecycle,
        description_changed,
        compatibility_mismatches,
        current_snapshot: current.identity.clone(),
    }))
}

pub(crate) fn reconcile_generation_lifecycle(
    transaction: &Transaction<'_>,
    repository_id: &str,
    staging_generation_id: &str,
    prior_ready_generation_id: Option<&str>,
    created_at: &str,
) -> Result<usize, String> {
    validate_scope("repository", repository_id)?;
    validate_timestamp(created_at)?;
    validate_generation_scope(
        transaction,
        repository_id,
        staging_generation_id,
        "staging",
        true,
    )?;
    if let Some(prior_generation_id) = prior_ready_generation_id {
        if prior_generation_id == staging_generation_id {
            return Err("Prior ready and staging generations must be distinct".into());
        }
        validate_generation_scope(
            transaction,
            repository_id,
            prior_generation_id,
            "ready",
            false,
        )?;
    }

    let snapshots = load_generation_snapshots(transaction, repository_id, staging_generation_id)?;
    let canonical = canonical_snapshots(transaction, staging_generation_id, snapshots, "Staging")?;
    if canonical.len() > MAX_LIFECYCLE_RULES_PER_GENERATION {
        return Err("Lifecycle reconciliation rule bound exceeded".into());
    }
    let prior_canonical_identities = if let Some(prior_generation_id) = prior_ready_generation_id {
        if generation_uses_storage_v2(transaction, repository_id, prior_generation_id)? {
            let prior_snapshots =
                load_generation_snapshots(transaction, repository_id, prior_generation_id)?;
            canonical_snapshots(
                transaction,
                prior_generation_id,
                prior_snapshots,
                "Prior ready",
            )?
            .into_keys()
            .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        }
    } else {
        BTreeSet::new()
    };

    let mut streams =
        load_generation_review_rows(transaction, repository_id, staging_generation_id)?;
    let provenance = ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::DeterministicPolicy,
        actor_id: "codevetter:local".into(),
        authority_id: Some("policy:archaeology-lifecycle-reconciliation:v1".into()),
    };
    let mut appended = 0usize;
    for (stable_rule_identity, current) in canonical {
        let existing = streams.remove(&stable_rule_identity).unwrap_or_default();
        let action = if existing.is_empty() {
            None
        } else if existing
            .iter()
            .rev()
            .find(|row| {
                !matches!(
                    row.event.action,
                    ArchaeologyLifecycleAction::Annotate { .. }
                )
            })
            .is_some_and(|row| row.generation_id != staging_generation_id)
            && !prior_canonical_identities.contains(&stable_rule_identity)
        {
            let projected = project_lifecycle(
                &existing
                    .iter()
                    .map(|row| row.event.clone())
                    .collect::<Vec<_>>(),
            )?;
            if projected.lifecycle == ArchaeologyRuleLifecycle::Superseded {
                return Err(
                    "A superseded stable rule cannot reappear without an explicit successor".into(),
                );
            }
            (projected.lifecycle != ArchaeologyRuleLifecycle::ReviewNeeded).then(|| {
                ArchaeologyLifecycleAction::ReviewNeeded {
                    reason: "Immediate prior generation has no unique canonical rule match.".into(),
                }
            })
        } else {
            let projection = project_stored_lifecycle(&current, &existing)?
                .ok_or("Lifecycle reconciliation stream disappeared")?;
            if projection.compatibility_mismatches.is_empty() {
                None
            } else {
                match projection.effective_lifecycle {
                    ArchaeologyRuleLifecycle::ReviewNeeded
                        if projection.projected.lifecycle
                            != ArchaeologyRuleLifecycle::ReviewNeeded =>
                    {
                        Some(ArchaeologyLifecycleAction::ReviewNeeded {
                            reason: reconciliation_reason(&projection.compatibility_mismatches),
                        })
                    }
                    ArchaeologyRuleLifecycle::Conflicted
                        if projection.projected.lifecycle
                            != ArchaeologyRuleLifecycle::Conflicted =>
                    {
                        Some(ArchaeologyLifecycleAction::Conflict {
                            reason: reconciliation_reason(&projection.compatibility_mismatches),
                        })
                    }
                    ArchaeologyRuleLifecycle::Superseded => return Err(
                        "A superseded stable rule cannot reappear without an explicit successor"
                            .into(),
                    ),
                    _ => None,
                }
            }
        };
        let Some(action) = action else {
            continue;
        };
        let previous_sequence = existing.last().map_or(0, |row| row.event.sequence);
        let prior_event_id = existing.last().map(|row| row.event.event_id.as_str());
        let sequence = previous_sequence
            .checked_add(1)
            .ok_or("Lifecycle sequence overflowed")?;
        let (decision, _) = action_columns(&action);
        let event_id = digest_fields(
            "archaeology-lifecycle-reconciliation-event:v1",
            &[
                repository_id,
                staging_generation_id,
                &stable_rule_identity,
                decision,
                &current.identity.evidence_identity,
                &current.identity.parser_compatibility_identity,
                &current.identity.contradiction_identity,
            ],
        );
        let event = ArchaeologyLifecycleEvent {
            event_id,
            repository_id: repository_id.into(),
            rule_id: stable_rule_identity,
            sequence,
            expected_previous_sequence: previous_sequence,
            provenance: provenance.clone(),
            action,
        };
        persist_lifecycle_event(
            transaction,
            &current,
            &existing,
            staging_generation_id,
            created_at,
            event,
            prior_event_id,
            None,
        )?;
        appended = appended
            .checked_add(1)
            .ok_or("Lifecycle reconciliation count overflowed")?;
    }
    Ok(appended)
}

fn reconciliation_reason(mismatches: &[ArchaeologyCompatibilityMismatch]) -> String {
    let mut labels = Vec::new();
    for mismatch in mismatches {
        labels.push(match mismatch {
            ArchaeologyCompatibilityMismatch::Evidence => "supporting evidence",
            ArchaeologyCompatibilityMismatch::Parser => "parser compatibility",
            ArchaeologyCompatibilityMismatch::Contradiction => "contradiction state",
        });
    }
    format!("Lifecycle compatibility changed: {}.", labels.join(", "))
}

pub(crate) fn append_explicit_supersession(
    transaction: &Transaction<'_>,
    input: ArchaeologyExplicitSupersession<'_>,
) -> Result<String, String> {
    transaction
        .execute_batch("SAVEPOINT archaeology_explicit_supersession")
        .map_err(|error| format!("Begin archaeology supersession savepoint: {error}"))?;
    let result = append_explicit_supersession_inner(transaction, &input);
    match result {
        Ok(edge_identity) => {
            transaction
                .execute_batch("RELEASE SAVEPOINT archaeology_explicit_supersession")
                .map_err(|error| format!("Commit archaeology supersession savepoint: {error}"))?;
            Ok(edge_identity)
        }
        Err(error) => {
            let rollback = transaction.execute_batch(
                "ROLLBACK TO SAVEPOINT archaeology_explicit_supersession;
                 RELEASE SAVEPOINT archaeology_explicit_supersession;",
            );
            match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!(
                    "{error}; rollback archaeology supersession savepoint: {rollback_error}"
                )),
            }
        }
    }
}

fn append_explicit_supersession_inner(
    transaction: &Transaction<'_>,
    input: &ArchaeologyExplicitSupersession<'_>,
) -> Result<String, String> {
    validate_scope("repository", input.repository_id)?;
    validate_digest("continuity", input.continuity_identity)?;
    validate_digest("successor evidence", input.successor_evidence_identity)?;
    validate_timestamp(input.created_at)?;
    input.provenance.validate()?;
    if matches!(input.provenance.kind, ArchaeologyReviewerKind::Model) {
        return Err("A model cannot supersede a rule".into());
    }
    if input.predecessor_generation_id == input.successor_generation_id {
        return Err("Rule supersession requires distinct generations".into());
    }
    let predecessor = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.predecessor_generation_id,
        input.predecessor_rule_id,
        input.predecessor_rule_identity,
    )?;
    let successor = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.successor_generation_id,
        input.successor_rule_id,
        input.successor_rule_identity,
    )?;
    if predecessor.identity.rule_id == successor.identity.rule_id {
        return Err("Rule supersession requires distinct stable rule identities".into());
    }
    if predecessor.identity.rule_kind_identity != successor.identity.rule_kind_identity {
        return Err("Rule supersession cannot cross rule kinds".into());
    }
    if predecessor.identity.continuity_identity != input.continuity_identity {
        return Err("Rule supersession continuity does not match the predecessor".into());
    }
    if successor.identity.evidence_identity != input.successor_evidence_identity {
        return Err("Rule supersession evidence does not match the successor".into());
    }
    validate_supersession_generation_order(
        transaction,
        input.repository_id,
        input.predecessor_generation_id,
        input.successor_generation_id,
    )?;
    validate_supersession_graph(
        transaction,
        input.repository_id,
        input.predecessor_rule_identity,
        input.successor_rule_identity,
    )?;

    let predecessor_stream =
        lifecycle_stream_identity(input.repository_id, input.predecessor_rule_identity);
    let predecessor_rows = load_review_rows(transaction, input.repository_id, &predecessor_stream)?;
    let previous_sequence = predecessor_rows.last().map_or(0, |row| row.event.sequence);
    let previous_event_id = predecessor_rows
        .last()
        .map(|row| row.event.event_id.as_str());
    if previous_sequence != input.expected_predecessor_sequence
        || previous_event_id != input.expected_predecessor_event_id
    {
        return Err("Lifecycle supersession compare-and-swap failed".into());
    }
    if predecessor_rows.is_empty() {
        return Err("Lifecycle supersession requires an existing predecessor stream".into());
    }
    let successor_stream =
        lifecycle_stream_identity(input.repository_id, input.successor_rule_identity);
    if !load_review_rows(transaction, input.repository_id, &successor_stream)?.is_empty() {
        return Err("Lifecycle successor stream already exists".into());
    }

    let predecessor_event_id =
        supersession_event_identity("predecessor-superseded", input, &predecessor, &successor);
    let policy = ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::DeterministicPolicy,
        actor_id: "codevetter:local".into(),
        authority_id: Some("policy:archaeology-explicit-supersession:v1".into()),
    };
    let successor_candidate_id =
        supersession_event_identity("successor-candidate", input, &predecessor, &successor);
    let successor_review_id =
        supersession_event_identity("successor-review-needed", input, &predecessor, &successor);

    let edge_identity = append_continuity_edge(
        transaction,
        ArchaeologyContinuityAppend {
            repository_id: input.repository_id,
            continuity_identity: input.continuity_identity,
            predecessor_rule_id: input.predecessor_rule_id,
            predecessor_rule_identity: input.predecessor_rule_identity,
            successor_rule_id: input.successor_rule_id,
            successor_rule_identity: input.successor_rule_identity,
            predecessor_generation_id: input.predecessor_generation_id,
            successor_generation_id: input.successor_generation_id,
            kind: ArchaeologyContinuityKind::Supersedes,
            evidence_identity: input.successor_evidence_identity,
            provenance: input.provenance.clone(),
            created_at: input.created_at,
        },
    )?;
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &predecessor_event_id,
            repository_id: input.repository_id,
            generation_id: input.predecessor_generation_id,
            rule_id: input.predecessor_rule_id,
            stable_rule_identity: input.predecessor_rule_identity,
            expected_previous_sequence: previous_sequence,
            expected_prior_event_id: previous_event_id,
            related_generation_id: Some(input.successor_generation_id),
            related_rule_id: Some(input.successor_rule_id),
            provenance: input.provenance.clone(),
            action: ArchaeologyLifecycleAction::Supersede {
                successor_rule_id: input.successor_rule_identity.into(),
            },
            created_at: input.created_at,
        },
    )?;
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &successor_candidate_id,
            repository_id: input.repository_id,
            generation_id: input.successor_generation_id,
            rule_id: input.successor_rule_id,
            stable_rule_identity: input.successor_rule_identity,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy.clone(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: input.created_at,
        },
    )?;
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &successor_review_id,
            repository_id: input.repository_id,
            generation_id: input.successor_generation_id,
            rule_id: input.successor_rule_id,
            stable_rule_identity: input.successor_rule_identity,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&successor_candidate_id),
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy,
            action: ArchaeologyLifecycleAction::ReviewNeeded {
                reason: "Explicit successor requires review against changed supporting evidence."
                    .into(),
            },
            created_at: input.created_at,
        },
    )?;
    Ok(edge_identity)
}

fn supersession_event_identity(
    phase: &str,
    input: &ArchaeologyExplicitSupersession<'_>,
    predecessor: &StoredRuleSnapshot,
    successor: &StoredRuleSnapshot,
) -> String {
    digest_fields(
        "archaeology-explicit-supersession-event:v1",
        &[
            phase,
            input.repository_id,
            input.predecessor_generation_id,
            &predecessor.identity.rule_id,
            input.successor_generation_id,
            &successor.identity.rule_id,
            input.continuity_identity,
            input.successor_evidence_identity,
        ],
    )
}

pub(crate) fn append_alias_event(
    transaction: &Transaction<'_>,
    input: ArchaeologyAliasAppend<'_>,
) -> Result<Vec<ArchaeologyRuleAlias>, String> {
    validate_digest("alias event", input.event_id)?;
    validate_timestamp(input.created_at)?;
    input.provenance.validate()?;
    if matches!(input.provenance.kind, ArchaeologyReviewerKind::Model) {
        return Err("A model cannot create or remove a rule alias".into());
    }
    let alias = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.generation_id,
        input.alias_rule_id,
        input.alias_rule_identity,
    )?;
    let canonical = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.generation_id,
        input.canonical_rule_id,
        input.canonical_rule_identity,
    )?;
    if alias.identity.rule_id == canonical.identity.rule_id
        || alias.identity.continuity_identity == canonical.identity.continuity_identity
    {
        return Err("A rule cannot alias itself".into());
    }
    let stream_identity =
        alias_stream_identity(input.repository_id, &alias.identity.continuity_identity);
    let rows = load_alias_rows(transaction, input.repository_id)?;
    if rows.len() >= MAX_ALIASES_PER_REPOSITORY {
        return Err("Rule alias event bound exceeded".into());
    }
    let stream_rows = rows
        .iter()
        .filter(|row| row.event_stream_identity == stream_identity)
        .collect::<Vec<_>>();
    let previous_sequence = stream_rows.last().map_or(0, |row| row.logical_sequence);
    if input.expected_previous_sequence != previous_sequence {
        return Err("Alias append compare-and-swap failed".into());
    }
    let active = project_alias_rows(&rows)?;
    let existing_alias = active
        .iter()
        .find(|item| item.alias_rule_id == input.alias_rule_identity);
    match (input.action, existing_alias) {
        (ArchaeologyAliasAction::Linked, Some(_)) => {
            return Err("Rule alias is already linked; unlink it first".into())
        }
        (ArchaeologyAliasAction::Unlinked, None) => {
            return Err("Rule alias is not currently linked".into())
        }
        (ArchaeologyAliasAction::Unlinked, Some(item))
            if item.canonical_rule_id != input.canonical_rule_identity =>
        {
            return Err("Alias unlink does not match its canonical rule".into())
        }
        _ => {}
    }
    let logical_sequence = previous_sequence
        .checked_add(1)
        .ok_or("Alias sequence overflowed")?;
    let mut prospective_rows = rows.clone();
    prospective_rows.push(StoredAliasRow {
        event_id: input.event_id.into(),
        repository_id: input.repository_id.into(),
        event_stream_identity: stream_identity.clone(),
        logical_sequence,
        action: input.action,
        alias_rule_identity: alias.identity.rule_id.clone(),
        alias_continuity_identity: alias.identity.continuity_identity.clone(),
        canonical_rule_identity: canonical.identity.rule_id.clone(),
        canonical_continuity_identity: canonical.identity.continuity_identity.clone(),
        provenance: input.provenance.clone(),
    });
    let projected = project_alias_rows(&prospective_rows)?;
    let provenance = encode_json("alias provenance", &input.provenance)?;
    transaction
        .execute(
            "INSERT INTO archaeology_rule_alias_events
             (event_id,repository_id,generation_id,event_stream_identity,logical_sequence,
              action,alias_rule_identity,alias_continuity_identity,canonical_rule_identity,
              canonical_continuity_identity,evidence_identity,reviewer_id,actor_kind,
              provenance_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                input.event_id,
                input.repository_id,
                input.generation_id,
                stream_identity,
                logical_sequence,
                alias_action_name(input.action),
                alias.identity.rule_id,
                alias.identity.continuity_identity,
                canonical.identity.rule_id,
                canonical.identity.continuity_identity,
                alias.identity.evidence_identity,
                input.provenance.actor_id,
                actor_kind(&input.provenance, false)?,
                provenance,
                input.created_at,
            ],
        )
        .map_err(|error| format!("Append archaeology alias event: {error}"))?;
    Ok(projected)
}

pub(crate) fn project_rule_aliases(
    transaction: &Transaction<'_>,
    repository_id: &str,
) -> Result<Vec<ArchaeologyRuleAlias>, String> {
    let rows = load_alias_rows(transaction, repository_id)?;
    project_alias_rows(&rows)
}

fn validate_supersession_generation_order(
    transaction: &Transaction<'_>,
    repository_id: &str,
    predecessor_generation_id: &str,
    successor_generation_id: &str,
) -> Result<(), String> {
    let predecessor_order = transaction
        .query_row(
            "SELECT rowid FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, predecessor_generation_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("Load predecessor generation order: {error}"))?
        .ok_or("Exact predecessor generation is unavailable")?;
    let successor_order = transaction
        .query_row(
            "SELECT rowid FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, successor_generation_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("Load successor generation order: {error}"))?
        .ok_or("Exact successor generation is unavailable")?;
    if predecessor_order >= successor_order {
        return Err("Rule supersession cannot reverse generation order".into());
    }
    Ok(())
}

fn validate_supersession_graph(
    transaction: &Transaction<'_>,
    repository_id: &str,
    predecessor_rule_identity: &str,
    successor_rule_identity: &str,
) -> Result<(), String> {
    let ambiguous = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM archaeology_rule_continuity_edges
               WHERE repository_id=?1 AND (
                 kind IN ('split','merge') AND (
                   predecessor_rule_identity IN (?2,?3)
                   OR successor_rule_identity IN (?2,?3)
                 )
                 OR kind='supersedes' AND (
                   predecessor_rule_identity=?2 OR successor_rule_identity=?3
                 )
               )
             )",
            params![
                repository_id,
                predecessor_rule_identity,
                successor_rule_identity
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Check archaeology supersession ambiguity: {error}"))?;
    if ambiguous {
        return Err("Rule supersession would create split, merge, or duplicate ambiguity".into());
    }
    let reverse_path = transaction
        .query_row(
            "WITH RECURSIVE reachable(rule_identity,depth) AS (
               SELECT successor_rule_identity,1
               FROM archaeology_rule_continuity_edges
               WHERE repository_id=?1 AND predecessor_rule_identity=?3
                 AND kind='supersedes'
               UNION
               SELECT edge.successor_rule_identity,reachable.depth+1
               FROM reachable
               JOIN archaeology_rule_continuity_edges edge
                 ON edge.repository_id=?1
                AND edge.predecessor_rule_identity=reachable.rule_identity
                AND edge.kind='supersedes'
               WHERE reachable.depth < 1000
             )
             SELECT EXISTS(SELECT 1 FROM reachable WHERE rule_identity=?2)",
            params![
                repository_id,
                predecessor_rule_identity,
                successor_rule_identity
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Check archaeology supersession cycle: {error}"))?;
    if reverse_path {
        return Err("Rule supersession would create a continuity cycle".into());
    }
    Ok(())
}

fn require_unique_supersession_edge(
    transaction: &Transaction<'_>,
    repository_id: &str,
    predecessor_generation_id: &str,
    predecessor: &StoredRuleSnapshot,
    successor_generation_id: &str,
    successor: &StoredRuleSnapshot,
) -> Result<(), String> {
    let count = transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rule_continuity_edges
             WHERE repository_id=?1 AND predecessor_rule_identity=?2
               AND successor_rule_identity=?3 AND predecessor_generation_id=?4
               AND successor_generation_id=?5 AND kind='supersedes'
               AND continuity_identity=?6 AND evidence_identity=?7",
            params![
                repository_id,
                predecessor.identity.rule_id,
                successor.identity.rule_id,
                predecessor_generation_id,
                successor_generation_id,
                predecessor.identity.continuity_identity,
                successor.identity.evidence_identity,
            ],
            |row| row.get::<_, usize>(0),
        )
        .map_err(|error| format!("Validate archaeology supersession edge: {error}"))?;
    if count != 1 {
        return Err("Lifecycle supersession requires one exact continuity edge".into());
    }
    Ok(())
}

fn append_continuity_edge(
    transaction: &Transaction<'_>,
    input: ArchaeologyContinuityAppend<'_>,
) -> Result<String, String> {
    validate_digest("continuity", input.continuity_identity)?;
    validate_digest("continuity evidence", input.evidence_identity)?;
    validate_timestamp(input.created_at)?;
    input.provenance.validate()?;
    if matches!(input.provenance.kind, ArchaeologyReviewerKind::Model) {
        return Err("A model cannot create a rule continuity edge".into());
    }
    if input.predecessor_generation_id == input.successor_generation_id {
        return Err("Rule continuity requires distinct generations".into());
    }
    let predecessor = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.predecessor_generation_id,
        input.predecessor_rule_id,
        input.predecessor_rule_identity,
    )?;
    let successor = load_rule_snapshot(
        transaction,
        input.repository_id,
        input.successor_generation_id,
        input.successor_rule_id,
        input.successor_rule_identity,
    )?;
    if predecessor.identity.rule_id == successor.identity.rule_id {
        return Err("Rule continuity requires distinct rule identities".into());
    }
    if predecessor.identity.continuity_identity != input.continuity_identity {
        return Err("Rule continuity identity does not match the predecessor snapshot".into());
    }
    if successor.identity.evidence_identity != input.evidence_identity {
        return Err("Rule continuity evidence does not match the successor snapshot".into());
    }
    if input.kind == ArchaeologyContinuityKind::SameEvidence
        && (predecessor.identity.evidence_identity != successor.identity.evidence_identity
            || successor.identity.continuity_identity != input.continuity_identity)
    {
        return Err("Same-evidence continuity requires identical evidence and continuity".into());
    }
    let kind = continuity_kind_name(input.kind);
    let edge_identity = digest_fields(
        "archaeology-rule-continuity-edge:v1",
        &[
            input.repository_id,
            input.continuity_identity,
            input.predecessor_rule_identity,
            input.successor_rule_identity,
            input.predecessor_generation_id,
            input.successor_generation_id,
            kind,
            input.evidence_identity,
        ],
    );
    let collision = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM archaeology_rule_continuity_edges
               WHERE edge_identity=?1 OR (
                 repository_id=?2 AND predecessor_rule_identity=?3
                 AND successor_rule_identity=?4 AND kind=?5
               )
             )",
            params![
                edge_identity,
                input.repository_id,
                input.predecessor_rule_identity,
                input.successor_rule_identity,
                kind,
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Check archaeology continuity edge: {error}"))?;
    if collision {
        return Err("Archaeology continuity edge is already recorded".into());
    }
    let provenance = encode_json("continuity provenance", &input.provenance)?;
    transaction
        .execute(
            "INSERT INTO archaeology_rule_continuity_edges
             (edge_identity,repository_id,continuity_identity,predecessor_rule_identity,
              successor_rule_identity,predecessor_generation_id,successor_generation_id,
              kind,evidence_identity,provenance_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                edge_identity,
                input.repository_id,
                input.continuity_identity,
                input.predecessor_rule_identity,
                input.successor_rule_identity,
                input.predecessor_generation_id,
                input.successor_generation_id,
                kind,
                input.evidence_identity,
                provenance,
                input.created_at,
            ],
        )
        .map_err(|error| format!("Append archaeology continuity edge: {error}"))?;
    Ok(edge_identity)
}

fn load_rule_snapshot(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    rule_id: &str,
    stable_rule_identity: &str,
) -> Result<StoredRuleSnapshot, String> {
    validate_scope("repository", repository_id)?;
    validate_digest("stable rule", stable_rule_identity)?;
    let row = transaction
        .query_row(
            "SELECT rule_id,kind,evidence_identity,parser_compatibility_identity,contradiction_identity,
                    description_identity,continuity_identity
             FROM archaeology_rules
             WHERE repository_id=?1 AND generation_id=?2 AND rule_id=?3
               AND stable_rule_identity=?4
               AND identity_schema_version=2",
            params![repository_id, generation_id, rule_id, stable_rule_identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology rule snapshot: {error}"))?
        .ok_or_else(|| "Exact archaeology rule snapshot is unavailable".to_string())?;
    for (label, value) in [
        ("evidence", row.2.as_str()),
        ("parser compatibility", row.3.as_str()),
        ("contradiction", row.4.as_str()),
        ("description", row.5.as_str()),
        ("continuity", row.6.as_str()),
    ] {
        validate_digest(label, value)?;
    }
    Ok(StoredRuleSnapshot {
        generated_rule_id: row.0,
        identity: ArchaeologyRuleSnapshotIdentity {
            repository_id: repository_id.into(),
            rule_id: stable_rule_identity.into(),
            rule_kind_identity: digest_fields("archaeology-rule-kind:v1", &[repository_id, &row.1]),
            continuity_identity: row.6,
            evidence_identity: row.2,
            parser_compatibility_identity: row.3,
            contradiction_identity: row.4,
            description_identity: row.5,
        },
    })
}

fn validate_generation_scope(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    expected_status: &str,
    require_storage_v2: bool,
) -> Result<(), String> {
    let exists = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM archaeology_generations
               WHERE repository_id=?1 AND generation_id=?2 AND status=?3
                 AND (?4=0 OR schema_version=2)
             )",
            params![
                repository_id,
                generation_id,
                expected_status,
                require_storage_v2
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Validate archaeology generation lifecycle scope: {error}"))?;
    if !exists {
        return Err(format!(
            "Exact {expected_status} archaeology generation is unavailable"
        ));
    }
    Ok(())
}

fn generation_uses_storage_v2(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT schema_version=2 FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, generation_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Load archaeology generation storage version: {error}"))
}

fn load_generation_snapshots(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<Vec<StoredRuleSnapshot>, String> {
    let total = transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rules
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, generation_id],
            |row| row.get::<_, usize>(0),
        )
        .map_err(|error| format!("Count archaeology generation rules: {error}"))?;
    if total > MAX_LIFECYCLE_RULES_PER_GENERATION {
        return Err("Lifecycle reconciliation rule bound exceeded".into());
    }
    let mut statement = transaction
        .prepare(
            "SELECT rule_id,stable_rule_identity,kind,evidence_identity,
                    parser_compatibility_identity,contradiction_identity,
                    description_identity,continuity_identity
             FROM archaeology_rules
             WHERE repository_id=?1 AND generation_id=?2 AND identity_schema_version=2
             ORDER BY stable_rule_identity,rule_id LIMIT ?3",
        )
        .map_err(|error| format!("Prepare archaeology generation snapshots: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repository_id,
                generation_id,
                MAX_LIFECYCLE_RULES_PER_GENERATION + 1
            ],
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
                ))
            },
        )
        .map_err(|error| format!("Query archaeology generation snapshots: {error}"))?;
    let mut result = Vec::with_capacity(total);
    for row in rows {
        let row = row.map_err(|error| format!("Read archaeology generation snapshot: {error}"))?;
        for (label, value) in [
            ("stable rule", row.1.as_str()),
            ("evidence", row.3.as_str()),
            ("parser compatibility", row.4.as_str()),
            ("contradiction", row.5.as_str()),
            ("description", row.6.as_str()),
            ("continuity", row.7.as_str()),
        ] {
            validate_digest(label, value)?;
        }
        result.push(StoredRuleSnapshot {
            generated_rule_id: row.0,
            identity: ArchaeologyRuleSnapshotIdentity {
                repository_id: repository_id.into(),
                rule_id: row.1,
                rule_kind_identity: digest_fields(
                    "archaeology-rule-kind:v1",
                    &[repository_id, &row.2],
                ),
                evidence_identity: row.3,
                parser_compatibility_identity: row.4,
                contradiction_identity: row.5,
                description_identity: row.6,
                continuity_identity: row.7,
            },
        });
    }
    if result.len() != total {
        return Err("Staging generation contains a rule without a complete v2 identity".into());
    }
    Ok(result)
}

fn canonical_snapshots(
    transaction: &Transaction<'_>,
    generation_id: &str,
    snapshots: Vec<StoredRuleSnapshot>,
    generation_label: &str,
) -> Result<BTreeMap<String, StoredRuleSnapshot>, String> {
    let alias_occurrences = validate_generation_alias_relations_for_snapshots(
        transaction,
        generation_id,
        &snapshots,
        generation_label,
    )?;

    let mut canonical = BTreeMap::<String, StoredRuleSnapshot>::new();
    for snapshot in snapshots {
        if alias_occurrences.contains(&snapshot.generated_rule_id) {
            continue;
        }
        if let Some(existing) = canonical.get(&snapshot.identity.rule_id) {
            let existing_metadata =
                duplicate_rule_metadata(transaction, generation_id, &existing.generated_rule_id)?;
            let duplicate_metadata =
                duplicate_rule_metadata(transaction, generation_id, &snapshot.generated_rule_id)?;
            return Err(format!(
                "{generation_label} generation contains duplicate canonical stable rule identities: stable={},first=[{}],second=[{}]",
                snapshot.identity.rule_id, existing_metadata, duplicate_metadata
            ));
        }
        canonical.insert(snapshot.identity.rule_id.clone(), snapshot);
    }
    Ok(canonical)
}

fn duplicate_rule_metadata(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_id: &str,
) -> Result<String, String> {
    transaction
        .query_row(
            "SELECT rule.rule_id,rule.kind,rule.evidence_identity,rule.contradiction_identity,
                    rule.description_identity,
                    (SELECT COUNT(*) FROM archaeology_rule_clauses clause
                     WHERE clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id),
                    (SELECT COUNT(DISTINCT evidence.evidence_id)
                     FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links evidence
                       ON evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind='fact' AND evidence.role='supporting'
                     WHERE clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id),
                    (SELECT COUNT(DISTINCT evidence.evidence_id)
                     FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links evidence
                       ON evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind='span' AND evidence.role='supporting'
                     WHERE clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id),
                    (SELECT COUNT(DISTINCT span.source_unit_id)
                     FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links evidence
                       ON evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind='span' AND evidence.role='supporting'
                     JOIN archaeology_source_spans span
                       ON span.generation_id=evidence.generation_id AND span.span_id=evidence.evidence_id
                     WHERE clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id),
                    (SELECT COUNT(DISTINCT evidence.evidence_id)
                     FROM archaeology_rule_clauses clause
                     JOIN archaeology_evidence_links evidence
                       ON evidence.generation_id=clause.generation_id
                      AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                      AND evidence.evidence_kind='fact' AND evidence.role='contradicting'
                     WHERE clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id),
                    (SELECT COUNT(*) FROM archaeology_rule_relations relation
                     WHERE relation.generation_id=rule.generation_id AND relation.kind='conflicts_with'
                       AND (relation.from_rule_id=rule.rule_id OR relation.to_rule_id=rule.rule_id))
             FROM archaeology_rules rule
             WHERE rule.generation_id=?1 AND rule.rule_id=?2",
            params![generation_id, rule_id],
            |row| {
                Ok(format!(
                    "rule={},kind={},evidence={},contradiction={},description={},clauses={},supporting_facts={},supporting_spans={},source_units={},contradicting_facts={},conflicts={}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(|error| format!("Load duplicate archaeology rule metadata: {error}"))
}

pub(crate) fn validate_generation_alias_relations(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<(), String> {
    let snapshots = load_generation_snapshots(transaction, repository_id, generation_id)?;
    validate_generation_alias_relations_for_snapshots(
        transaction,
        generation_id,
        &snapshots,
        "Archaeology",
    )?;
    Ok(())
}

fn validate_generation_alias_relations_for_snapshots(
    transaction: &Transaction<'_>,
    generation_id: &str,
    snapshots: &[StoredRuleSnapshot],
    generation_label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT from_rule_id,to_rule_id,trust FROM archaeology_rule_relations
             WHERE generation_id=?1 AND kind='aliases'
             ORDER BY from_rule_id,to_rule_id,relation_id LIMIT ?2",
        )
        .map_err(|error| format!("Prepare archaeology generation aliases: {error}"))?;
    let rows = statement
        .query_map(
            params![generation_id, MAX_LIFECYCLE_RULES_PER_GENERATION + 1],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| format!("Query archaeology generation aliases: {error}"))?;
    let snapshots_by_occurrence = snapshots
        .iter()
        .map(|snapshot| (snapshot.generated_rule_id.as_str(), snapshot))
        .collect::<BTreeMap<_, _>>();
    let mut aliases = BTreeMap::new();
    for row in rows {
        let (alias_rule_id, canonical_rule_id, trust) =
            row.map_err(|error| format!("Read archaeology generation alias: {error}"))?;
        validate_scope("generated alias rule", &alias_rule_id)?;
        validate_scope("generated canonical rule", &canonical_rule_id)?;
        if alias_rule_id == canonical_rule_id {
            return Err(format!(
                "{generation_label} generation contains a self-referential alias"
            ));
        }
        if trust != "deterministic" {
            return Err(format!(
                "{generation_label} generation contains a non-deterministic alias"
            ));
        }
        if aliases.insert(alias_rule_id, canonical_rule_id).is_some() {
            return Err(format!(
                "{generation_label} generation contains duplicate alias relations"
            ));
        }
        if aliases.len() > MAX_LIFECYCLE_RULES_PER_GENERATION {
            return Err("Lifecycle reconciliation alias bound exceeded".into());
        }
    }

    let alias_occurrences = aliases.keys().cloned().collect::<BTreeSet<_>>();
    for (alias_rule_id, canonical_rule_id) in &aliases {
        if alias_occurrences.contains(canonical_rule_id) {
            return Err(format!(
                "{generation_label} generation aliases must form direct stars"
            ));
        }
        let alias = snapshots_by_occurrence
            .get(alias_rule_id.as_str())
            .ok_or_else(|| {
                format!("{generation_label} generation alias occurrence is outside exact scope")
            })?;
        let canonical = snapshots_by_occurrence
            .get(canonical_rule_id.as_str())
            .ok_or_else(|| {
                format!(
                    "{generation_label} generation canonical alias occurrence is outside exact scope"
                )
            })?;
        if alias.identity.rule_id != canonical.identity.rule_id
            || alias.identity.rule_kind_identity != canonical.identity.rule_kind_identity
            || alias.identity.continuity_identity != canonical.identity.continuity_identity
            || alias.identity.parser_compatibility_identity
                != canonical.identity.parser_compatibility_identity
            || alias.identity.contradiction_identity != canonical.identity.contradiction_identity
        {
            return Err(format!(
                "{generation_label} generation alias is not semantically compatible with its canonical rule"
            ));
        }
    }
    Ok(alias_occurrences)
}

fn load_review_rows(
    transaction: &Transaction<'_>,
    repository_id: &str,
    event_stream_identity: &str,
) -> Result<Vec<StoredReviewRow>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT event_id,generation_id,stable_rule_identity,logical_sequence,decision,body,
                    evidence_identity,contradiction_identity,description_identity,
                    continuity_identity,parser_identity,prior_event_id,related_rule_identity,
                    related_continuity_identity,reviewer_id,actor_kind,reviewer_provenance_json
             FROM archaeology_rule_review_events
             WHERE repository_id=?1 AND event_stream_identity=?2
               AND event_schema_version=2 AND legacy_stale=0
             ORDER BY logical_sequence,event_id LIMIT ?3",
        )
        .map_err(|error| format!("Prepare archaeology lifecycle stream: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repository_id,
                event_stream_identity,
                MAX_LIFECYCLE_EVENTS_PER_RULE + 1
            ],
            |row| read_raw_review_row(row, 0),
        )
        .map_err(|error| format!("Query archaeology lifecycle stream: {error}"))?;
    let mut result = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("Read archaeology lifecycle stream: {error}"))?;
        result.push(decode_raw_review_row(
            repository_id,
            event_stream_identity,
            row,
        )?);
    }
    if result.len() > MAX_LIFECYCLE_EVENTS_PER_RULE {
        return Err("Lifecycle event bound exceeded".into());
    }
    validate_review_chain(&result)?;
    Ok(result)
}

fn load_generation_review_rows(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<BTreeMap<String, Vec<StoredReviewRow>>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT events.event_stream_identity,
                    events.event_id,events.generation_id,events.stable_rule_identity,
                    events.logical_sequence,events.decision,events.body,events.evidence_identity,
                    events.contradiction_identity,events.description_identity,
                    events.continuity_identity,events.parser_identity,events.prior_event_id,
                    events.related_rule_identity,events.related_continuity_identity,
                    events.reviewer_id,events.actor_kind,events.reviewer_provenance_json
             FROM archaeology_rule_review_events AS events
             INNER JOIN (
               SELECT DISTINCT stable_rule_identity
               FROM archaeology_rules
               WHERE repository_id=?1 AND generation_id=?2 AND identity_schema_version=2
             ) AS current
               ON current.stable_rule_identity=events.stable_rule_identity
             WHERE events.repository_id=?1 AND events.event_schema_version=2
               AND events.legacy_stale=0
             ORDER BY events.event_stream_identity,events.logical_sequence,events.event_id
             LIMIT ?3",
        )
        .map_err(|error| format!("Prepare archaeology reconciliation streams: {error}"))?;
    let rows = statement
        .query_map(
            params![repository_id, generation_id, MAX_RECONCILIATION_EVENTS + 1],
            |row| Ok((row.get::<_, String>(0)?, read_raw_review_row(row, 1)?)),
        )
        .map_err(|error| format!("Query archaeology reconciliation streams: {error}"))?;
    let mut result = BTreeMap::<String, Vec<StoredReviewRow>>::new();
    let mut count = 0usize;
    for row in rows {
        let (stream_identity, raw) =
            row.map_err(|error| format!("Read archaeology reconciliation stream: {error}"))?;
        count = count
            .checked_add(1)
            .ok_or("Lifecycle reconciliation event count overflowed")?;
        if count > MAX_RECONCILIATION_EVENTS {
            return Err("Lifecycle reconciliation event bound exceeded".into());
        }
        let decoded = decode_raw_review_row(repository_id, &stream_identity, raw)?;
        let stable_rule_identity = decoded.event.rule_id.clone();
        let stream = result.entry(stable_rule_identity).or_default();
        stream.push(decoded);
        if stream.len() > MAX_LIFECYCLE_EVENTS_PER_RULE {
            return Err("Lifecycle event bound exceeded".into());
        }
    }
    for stream in result.values() {
        validate_review_chain(stream)?;
    }
    Ok(result)
}

fn read_raw_review_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<RawReviewRow> {
    Ok(RawReviewRow {
        event_id: row.get(offset)?,
        generation_id: row.get(offset + 1)?,
        stable_rule_identity: row.get(offset + 2)?,
        logical_sequence: row.get(offset + 3)?,
        decision: row.get(offset + 4)?,
        body: row.get(offset + 5)?,
        evidence_identity: row.get(offset + 6)?,
        contradiction_identity: row.get(offset + 7)?,
        description_identity: row.get(offset + 8)?,
        continuity_identity: row.get(offset + 9)?,
        parser_identity: row.get(offset + 10)?,
        prior_event_id: row.get(offset + 11)?,
        related_rule_identity: row.get(offset + 12)?,
        related_continuity_identity: row.get(offset + 13)?,
        reviewer_id: row.get(offset + 14)?,
        actor_kind: row.get(offset + 15)?,
        reviewer_provenance_json: row.get(offset + 16)?,
    })
}

fn decode_raw_review_row(
    repository_id: &str,
    event_stream_identity: &str,
    row: RawReviewRow,
) -> Result<StoredReviewRow, String> {
    for (label, value) in [
        ("lifecycle event", row.event_id.as_str()),
        ("stable rule", row.stable_rule_identity.as_str()),
        ("evidence", row.evidence_identity.as_str()),
        ("contradiction", row.contradiction_identity.as_str()),
        ("description", row.description_identity.as_str()),
        ("continuity", row.continuity_identity.as_str()),
        ("parser compatibility", row.parser_identity.as_str()),
    ] {
        validate_digest(label, value)?;
    }
    if let Some(value) = row.related_rule_identity.as_deref() {
        validate_digest("related rule", value)?;
    }
    if let Some(value) = row.related_continuity_identity.as_deref() {
        validate_digest("related continuity", value)?;
    }
    let stored: StoredReviewProvenance =
        decode_json("reviewer provenance", &row.reviewer_provenance_json)?;
    validate_digest("rule kind", &stored.rule_kind_identity)?;
    validate_stored_actor(&stored.reviewer, &row.reviewer_id, &row.actor_kind, true)?;
    let action = columns_action(&row.decision, row.body, row.related_rule_identity.clone())?;
    let has_complete_relation =
        row.related_rule_identity.is_some() && row.related_continuity_identity.is_some();
    let has_partial_relation =
        row.related_rule_identity.is_some() != row.related_continuity_identity.is_some();
    if has_partial_relation
        || matches!(action, ArchaeologyLifecycleAction::Supersede { .. }) != has_complete_relation
    {
        return Err("Stored lifecycle related continuity is invalid".into());
    }
    if matches!(stored.reviewer.kind, ArchaeologyReviewerKind::Model)
        && !matches!(action, ArchaeologyLifecycleAction::Annotate { .. })
    {
        return Err("Stored model lifecycle event is not an annotation".into());
    }
    let event = ArchaeologyLifecycleEvent {
        event_id: row.event_id,
        repository_id: repository_id.into(),
        rule_id: row.stable_rule_identity.clone(),
        sequence: row.logical_sequence,
        expected_previous_sequence: row.logical_sequence.saturating_sub(1),
        provenance: stored.reviewer,
        action,
    };
    let snapshot = ArchaeologyRuleSnapshotIdentity {
        repository_id: repository_id.into(),
        rule_id: row.stable_rule_identity,
        rule_kind_identity: stored.rule_kind_identity,
        continuity_identity: row.continuity_identity,
        evidence_identity: row.evidence_identity,
        parser_compatibility_identity: row.parser_identity,
        contradiction_identity: row.contradiction_identity,
        description_identity: row.description_identity,
    };
    snapshot.validate()?;
    if event_stream_identity != lifecycle_stream_identity(repository_id, &snapshot.rule_id) {
        return Err("Stored lifecycle event stream identity is invalid".into());
    }
    Ok(StoredReviewRow {
        generation_id: row.generation_id,
        event,
        snapshot,
        prior_event_id: row.prior_event_id,
    })
}

fn validate_review_chain(rows: &[StoredReviewRow]) -> Result<(), String> {
    for (offset, row) in rows.iter().enumerate() {
        let expected_sequence = u64::try_from(offset)
            .map_err(|_| "Lifecycle event sequence exceeds supported range")?
            + 1;
        if row.event.sequence != expected_sequence {
            return Err("Lifecycle event sequence is duplicated or has a gap".into());
        }
        let expected_prior = offset
            .checked_sub(1)
            .map(|prior| rows[prior].event.event_id.as_str());
        if row.prior_event_id.as_deref() != expected_prior {
            return Err("Stored lifecycle prior-event chain is invalid".into());
        }
    }
    Ok(())
}

fn load_alias_rows(
    transaction: &Transaction<'_>,
    repository_id: &str,
) -> Result<Vec<StoredAliasRow>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT event_id,event_stream_identity,logical_sequence,action,
                    alias_rule_identity,alias_continuity_identity,canonical_rule_identity,
                    canonical_continuity_identity,evidence_identity,reviewer_id,actor_kind,
                    provenance_json
             FROM archaeology_rule_alias_events WHERE repository_id=?1
             ORDER BY event_stream_identity,logical_sequence,event_id LIMIT ?2",
        )
        .map_err(|error| format!("Prepare archaeology alias stream: {error}"))?;
    let rows = statement
        .query_map(
            params![repository_id, MAX_ALIASES_PER_REPOSITORY + 1],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .map_err(|error| format!("Query archaeology alias stream: {error}"))?;
    let mut result = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("Read archaeology alias stream: {error}"))?;
        for (label, value) in [
            ("alias event", row.0.as_str()),
            ("alias stream", row.1.as_str()),
            ("alias rule", row.4.as_str()),
            ("alias continuity", row.5.as_str()),
            ("canonical rule", row.6.as_str()),
            ("canonical continuity", row.7.as_str()),
            ("alias evidence", row.8.as_str()),
        ] {
            validate_digest(label, value)?;
        }
        let provenance: ArchaeologyReviewerProvenance = decode_json("alias provenance", &row.11)?;
        validate_stored_actor(&provenance, &row.9, &row.10, false)?;
        result.push(StoredAliasRow {
            event_id: row.0,
            repository_id: repository_id.into(),
            event_stream_identity: row.1,
            logical_sequence: row.2,
            action: parse_alias_action(&row.3)?,
            alias_rule_identity: row.4,
            alias_continuity_identity: row.5,
            canonical_rule_identity: row.6,
            canonical_continuity_identity: row.7,
            provenance,
        });
    }
    if result.len() > MAX_ALIASES_PER_REPOSITORY {
        return Err("Rule alias event bound exceeded".into());
    }
    Ok(result)
}

fn project_alias_rows(rows: &[StoredAliasRow]) -> Result<Vec<ArchaeologyRuleAlias>, String> {
    let mut streams = BTreeMap::<&str, Vec<&StoredAliasRow>>::new();
    for row in rows {
        streams
            .entry(row.event_stream_identity.as_str())
            .or_default()
            .push(row);
    }
    let mut active = Vec::new();
    for stream in streams.values() {
        let mut linked: Option<&StoredAliasRow> = None;
        for (offset, row) in stream.iter().enumerate() {
            let sequence = u64::try_from(offset).map_err(|_| "Alias sequence overflowed")? + 1;
            if row.logical_sequence != sequence {
                return Err("Alias event sequence is duplicated or has a gap".into());
            }
            if row.event_stream_identity
                != alias_stream_identity(&row.repository_id, &row.alias_continuity_identity)
            {
                return Err("Alias event stream identity is invalid".into());
            }
            match (row.action, linked) {
                (ArchaeologyAliasAction::Linked, None) => linked = Some(row),
                (ArchaeologyAliasAction::Linked, Some(_)) => {
                    return Err("Alias stream contains a duplicate link".into())
                }
                (ArchaeologyAliasAction::Unlinked, Some(link))
                    if link.alias_rule_identity == row.alias_rule_identity
                        && link.canonical_rule_identity == row.canonical_rule_identity
                        && link.canonical_continuity_identity
                            == row.canonical_continuity_identity =>
                {
                    linked = None;
                }
                (ArchaeologyAliasAction::Unlinked, _) => {
                    return Err("Alias stream contains an unmatched unlink".into())
                }
            }
        }
        if let Some(row) = linked {
            active.push(ArchaeologyRuleAlias {
                event_id: row.event_id.clone(),
                alias_repository_id: row.repository_id.clone(),
                alias_rule_id: row.alias_rule_identity.clone(),
                canonical_repository_id: row.repository_id.clone(),
                canonical_rule_id: row.canonical_rule_identity.clone(),
                provenance: row.provenance.clone(),
            });
        }
    }
    active.sort_by(|left, right| {
        left.alias_rule_id
            .cmp(&right.alias_rule_id)
            .then_with(|| left.canonical_rule_id.cmp(&right.canonical_rule_id))
    });
    validate_rule_aliases(&active)?;
    Ok(active)
}

fn action_columns(action: &ArchaeologyLifecycleAction) -> (&'static str, Option<&str>) {
    match action {
        ArchaeologyLifecycleAction::Candidate => ("candidate", None),
        ArchaeologyLifecycleAction::ReviewNeeded { reason } => ("review_needed", Some(reason)),
        ArchaeologyLifecycleAction::Accept => ("accepted", None),
        ArchaeologyLifecycleAction::Reject { reason } => ("rejected", Some(reason)),
        ArchaeologyLifecycleAction::Conflict { reason } => ("conflicted", Some(reason)),
        ArchaeologyLifecycleAction::Supersede { .. } => ("superseded", None),
        ArchaeologyLifecycleAction::Annotate { annotation } => ("annotation", Some(annotation)),
    }
}

fn columns_action(
    decision: &str,
    body: Option<String>,
    related_rule_identity: Option<String>,
) -> Result<ArchaeologyLifecycleAction, String> {
    let require_body = |name: &str| {
        body.clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Stored {name} lifecycle event has no body"))
    };
    let require_empty = || {
        if body.is_some() {
            Err("Stored lifecycle state event has an unexpected body".to_string())
        } else {
            Ok(())
        }
    };
    match decision {
        "candidate" => {
            require_empty()?;
            Ok(ArchaeologyLifecycleAction::Candidate)
        }
        "review_needed" => Ok(ArchaeologyLifecycleAction::ReviewNeeded {
            reason: require_body("review-needed")?,
        }),
        "accepted" => {
            require_empty()?;
            Ok(ArchaeologyLifecycleAction::Accept)
        }
        "rejected" => Ok(ArchaeologyLifecycleAction::Reject {
            reason: require_body("rejected")?,
        }),
        "conflicted" => Ok(ArchaeologyLifecycleAction::Conflict {
            reason: require_body("conflicted")?,
        }),
        "superseded" => {
            require_empty()?;
            Ok(ArchaeologyLifecycleAction::Supersede {
                successor_rule_id: related_rule_identity
                    .ok_or("Stored supersession has no related rule identity")?,
            })
        }
        "annotation" => Ok(ArchaeologyLifecycleAction::Annotate {
            annotation: require_body("annotation")?,
        }),
        _ => Err("Stored lifecycle decision is invalid".into()),
    }
}

fn actor_kind(
    provenance: &ArchaeologyReviewerProvenance,
    allow_model_annotation: bool,
) -> Result<&'static str, String> {
    match provenance.kind {
        ArchaeologyReviewerKind::Human => Ok("human"),
        ArchaeologyReviewerKind::DeterministicPolicy => Ok("deterministic_policy"),
        ArchaeologyReviewerKind::Model if allow_model_annotation => Ok("imported"),
        ArchaeologyReviewerKind::Model => Err("A model cannot author this event".into()),
    }
}

fn validate_stored_actor(
    provenance: &ArchaeologyReviewerProvenance,
    reviewer_id: &str,
    actor_kind_value: &str,
    allow_model_annotation: bool,
) -> Result<(), String> {
    provenance.validate()?;
    if provenance.actor_id != reviewer_id
        || actor_kind(provenance, allow_model_annotation)? != actor_kind_value
    {
        return Err("Stored reviewer provenance does not match its actor columns".into());
    }
    Ok(())
}

fn alias_action_name(action: ArchaeologyAliasAction) -> &'static str {
    match action {
        ArchaeologyAliasAction::Linked => "linked",
        ArchaeologyAliasAction::Unlinked => "unlinked",
    }
}

fn parse_alias_action(value: &str) -> Result<ArchaeologyAliasAction, String> {
    match value {
        "linked" => Ok(ArchaeologyAliasAction::Linked),
        "unlinked" => Ok(ArchaeologyAliasAction::Unlinked),
        _ => Err("Stored alias action is invalid".into()),
    }
}

fn continuity_kind_name(kind: ArchaeologyContinuityKind) -> &'static str {
    match kind {
        ArchaeologyContinuityKind::SameEvidence => "same_evidence",
        ArchaeologyContinuityKind::Supersedes => "supersedes",
    }
}

fn lifecycle_stream_identity(repository_id: &str, stable_rule_identity: &str) -> String {
    digest_fields(
        "archaeology-lifecycle-stream:v1",
        &[repository_id, stable_rule_identity],
    )
}

fn alias_stream_identity(repository_id: &str, alias_continuity_identity: &str) -> String {
    digest_fields(
        "archaeology-alias-stream:v1",
        &[repository_id, alias_continuity_identity],
    )
}

fn digest_fields(tag: &str, fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in std::iter::once(tag).chain(fields.iter().copied()) {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!(
        "{DIGEST_PREFIX}{}",
        super::inventory::hex(&digest.finalize())
    )
}

fn validate_digest(label: &str, value: &str) -> Result<(), String> {
    let suffix = value.strip_prefix(DIGEST_PREFIX).unwrap_or_default();
    if suffix.len() != 64
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} must be an opaque SHA-256 identity"));
    }
    Ok(())
}

fn validate_scope(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(|character| character.is_control())
    {
        return Err(format!("{label} scope is invalid"));
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_TIMESTAMP_BYTES || value.chars().any(char::is_control)
    {
        return Err("Lifecycle timestamp is invalid".into());
    }
    Ok(())
}

fn encode_json<T: Serialize>(label: &str, value: &T) -> Result<String, String> {
    let encoded =
        serde_json::to_string(value).map_err(|error| format!("Encode {label}: {error}"))?;
    if encoded.len() > MAX_EVENT_JSON_BYTES {
        return Err(format!("{label} exceeds its byte bound"));
    }
    Ok(encoded)
}

fn decode_json<T: for<'de> Deserialize<'de>>(label: &str, value: &str) -> Result<T, String> {
    if value.len() > MAX_EVENT_JSON_BYTES {
        return Err(format!("Stored {label} exceeds its byte bound"));
    }
    serde_json::from_str(value).map_err(|error| format!("Decode stored {label}: {error}"))
}

#[cfg(test)]
#[path = "lifecycle_store_tests.rs"]
mod tests;
