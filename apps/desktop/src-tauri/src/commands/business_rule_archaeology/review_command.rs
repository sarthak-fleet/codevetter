//! Strict desktop mutations over the append-only archaeology lifecycle store.

use super::{
    contracts::{ArchaeologyRuleLifecycle, ARCHAEOLOGY_STORAGE_SCHEMA_VERSION},
    lifecycle::{
        ArchaeologyLifecycleAction, ArchaeologyReviewerKind, ArchaeologyReviewerProvenance,
    },
    lifecycle_store::{
        append_alias_event, append_explicit_supersession, append_lifecycle_event,
        ensure_candidate_lifecycle, project_current_lifecycle, ArchaeologyAliasAction,
        ArchaeologyAliasAppend, ArchaeologyExplicitSupersession, ArchaeologyLifecycleAppend,
    },
};
use crate::DbState;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::State;

const MAX_REQUEST_ID_BYTES: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyReviewDecision {
    Accept,
    Reject,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyAliasMutation {
    Link,
    Unlink,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchaeologyReviewMutation {
    Review {
        decision: ArchaeologyReviewDecision,
        reason: Option<String>,
    },
    Annotate {
        annotation: String,
    },
    Alias {
        alias_rule_id: String,
        mutation: ArchaeologyAliasMutation,
    },
    Supersede {
        predecessor_generation_id: String,
        predecessor_rule_id: String,
        expected_predecessor_lifecycle: ArchaeologyRuleLifecycle,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyReviewMutationInput {
    pub request_id: String,
    pub repository_id: String,
    pub generation_id: String,
    pub rule_id: String,
    pub expected_lifecycle: ArchaeologyRuleLifecycle,
    pub mutation: ArchaeologyReviewMutation,
}

#[derive(Debug, Serialize)]
pub struct ArchaeologyReviewMutationResult {
    pub repository_id: String,
    pub generation_id: String,
    pub rule_id: String,
    pub lifecycle: ArchaeologyRuleLifecycle,
    pub last_sequence: u64,
    pub last_event_id: String,
    pub annotation_count: usize,
    pub alias_rule_ids: Vec<String>,
    pub continuity_edge_id: Option<String>,
}

#[derive(Debug)]
struct RuleOccurrence {
    occurrence_id: String,
    stable_rule_identity: String,
    continuity_identity: String,
    evidence_identity: String,
}

#[tauri::command]
pub async fn mutate_business_rule_archaeology_review(
    db: State<'_, DbState>,
    input: serde_json::Value,
) -> Result<ArchaeologyReviewMutationResult, String> {
    let input = serde_json::from_value::<ArchaeologyReviewMutationInput>(input)
        .map_err(|_| "Invalid archaeology review request".to_string())?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let mut connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        mutate_review_core(&mut connection, input)
    })
    .await
    .map_err(|error| format!("Archaeology review worker failed: {error}"))?
}

fn mutate_review_core(
    connection: &mut Connection,
    input: ArchaeologyReviewMutationInput,
) -> Result<ArchaeologyReviewMutationResult, String> {
    validate_request_id(&input.request_id)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("Begin archaeology review transaction: {error}"))?;
    if let ArchaeologyReviewMutation::Supersede {
        predecessor_generation_id,
        predecessor_rule_id,
        expected_predecessor_lifecycle,
    } = &input.mutation
    {
        let result = supersede_predecessor(
            &transaction,
            &input,
            predecessor_generation_id,
            predecessor_rule_id,
            expected_predecessor_lifecycle.clone(),
        )?;
        transaction
            .commit()
            .map_err(|error| format!("Commit archaeology review transaction: {error}"))?;
        return Ok(result);
    }
    require_ready_generation(&transaction, &input.repository_id, &input.generation_id)?;
    let current = load_occurrence(
        &transaction,
        &input.repository_id,
        &input.generation_id,
        &input.rule_id,
    )?;
    let created_at = Utc::now().to_rfc3339();
    let before = ensure_candidate_lifecycle(
        &transaction,
        &input.repository_id,
        &input.generation_id,
        &current.occurrence_id,
        &current.stable_rule_identity,
        &created_at,
    )?;
    if before.effective_lifecycle != input.expected_lifecycle {
        return Err("Archaeology review state changed; refresh before retrying".into());
    }
    let previous_event_id = last_event_id(
        &transaction,
        &input.repository_id,
        &current.stable_rule_identity,
    )?;

    let provenance = ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::Human,
        actor_id: "human:local".into(),
        authority_id: None,
    };
    let mut aliases = Vec::new();
    let continuity_edge_id = None;
    match input.mutation {
        ArchaeologyReviewMutation::Review { decision, reason } => {
            let action = match decision {
                ArchaeologyReviewDecision::Accept => {
                    if reason
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        return Err("Accept review does not take a rejection reason".into());
                    }
                    ArchaeologyLifecycleAction::Accept
                }
                ArchaeologyReviewDecision::Reject => ArchaeologyLifecycleAction::Reject {
                    reason: reason.unwrap_or_default(),
                },
            };
            append_lifecycle_event(
                &transaction,
                ArchaeologyLifecycleAppend {
                    event_id: &event_id("review", &input.request_id, &input.rule_id),
                    repository_id: &input.repository_id,
                    generation_id: &input.generation_id,
                    rule_id: &current.occurrence_id,
                    stable_rule_identity: &current.stable_rule_identity,
                    expected_previous_sequence: before.projected.last_sequence,
                    expected_prior_event_id: previous_event_id.as_deref(),
                    related_generation_id: None,
                    related_rule_id: None,
                    provenance: provenance.clone(),
                    action,
                    created_at: &created_at,
                },
            )?;
        }
        ArchaeologyReviewMutation::Annotate { annotation } => {
            append_lifecycle_event(
                &transaction,
                ArchaeologyLifecycleAppend {
                    event_id: &event_id("annotation", &input.request_id, &input.rule_id),
                    repository_id: &input.repository_id,
                    generation_id: &input.generation_id,
                    rule_id: &current.occurrence_id,
                    stable_rule_identity: &current.stable_rule_identity,
                    expected_previous_sequence: before.projected.last_sequence,
                    expected_prior_event_id: previous_event_id.as_deref(),
                    related_generation_id: None,
                    related_rule_id: None,
                    provenance: provenance.clone(),
                    action: ArchaeologyLifecycleAction::Annotate { annotation },
                    created_at: &created_at,
                },
            )?;
        }
        ArchaeologyReviewMutation::Alias {
            alias_rule_id,
            mutation,
        } => {
            let alias = load_occurrence(
                &transaction,
                &input.repository_id,
                &input.generation_id,
                &alias_rule_id,
            )?;
            let sequence = alias_sequence(&transaction, &input.repository_id, &alias_rule_id)?;
            aliases = append_alias_event(
                &transaction,
                ArchaeologyAliasAppend {
                    event_id: &event_id("alias", &input.request_id, &alias_rule_id),
                    repository_id: &input.repository_id,
                    generation_id: &input.generation_id,
                    alias_rule_id: &alias.occurrence_id,
                    alias_rule_identity: &alias.stable_rule_identity,
                    canonical_rule_id: &current.occurrence_id,
                    canonical_rule_identity: &current.stable_rule_identity,
                    expected_previous_sequence: sequence,
                    action: match mutation {
                        ArchaeologyAliasMutation::Link => ArchaeologyAliasAction::Linked,
                        ArchaeologyAliasMutation::Unlink => ArchaeologyAliasAction::Unlinked,
                    },
                    provenance: provenance.clone(),
                    created_at: &created_at,
                },
            )?
            .into_iter()
            .map(|alias| alias.alias_rule_id)
            .collect();
        }
        ArchaeologyReviewMutation::Supersede { .. } => unreachable!("handled before projection"),
    }

    let after = project_current_lifecycle(
        &transaction,
        &input.repository_id,
        &input.generation_id,
        &current.occurrence_id,
        &current.stable_rule_identity,
    )?
    .ok_or_else(|| "Updated archaeology review stream is unavailable".to_string())?;
    let updated_event_id = last_event_id(
        &transaction,
        &input.repository_id,
        &current.stable_rule_identity,
    )?
    .ok_or_else(|| "Updated archaeology review event is unavailable".to_string())?;
    let result = ArchaeologyReviewMutationResult {
        repository_id: input.repository_id,
        generation_id: input.generation_id,
        rule_id: input.rule_id,
        lifecycle: after.effective_lifecycle,
        last_sequence: after.projected.last_sequence,
        last_event_id: updated_event_id,
        annotation_count: after.projected.annotations.len(),
        alias_rule_ids: aliases,
        continuity_edge_id,
    };
    transaction
        .commit()
        .map_err(|error| format!("Commit archaeology review transaction: {error}"))?;
    Ok(result)
}

#[cfg(test)]
pub(crate) fn mutate_review_for_qualification(
    connection: &mut Connection,
    input: ArchaeologyReviewMutationInput,
) -> Result<ArchaeologyReviewMutationResult, String> {
    mutate_review_core(connection, input)
}

fn supersede_predecessor(
    transaction: &Transaction<'_>,
    input: &ArchaeologyReviewMutationInput,
    predecessor_generation_id: &str,
    predecessor_rule_id: &str,
    expected_predecessor_lifecycle: ArchaeologyRuleLifecycle,
) -> Result<ArchaeologyReviewMutationResult, String> {
    require_ready_generation(transaction, &input.repository_id, &input.generation_id)?;
    require_reviewable_generation(transaction, &input.repository_id, predecessor_generation_id)?;
    let successor = load_occurrence(
        transaction,
        &input.repository_id,
        &input.generation_id,
        &input.rule_id,
    )?;
    let successor_lifecycle = declared_lifecycle(
        transaction,
        &input.repository_id,
        &input.generation_id,
        &successor.occurrence_id,
    )?;
    if successor_lifecycle != input.expected_lifecycle {
        return Err("Archaeology successor state changed; refresh before retrying".into());
    }
    let predecessor = load_occurrence(
        transaction,
        &input.repository_id,
        predecessor_generation_id,
        predecessor_rule_id,
    )?;
    let before = project_current_lifecycle(
        transaction,
        &input.repository_id,
        predecessor_generation_id,
        &predecessor.occurrence_id,
        &predecessor.stable_rule_identity,
    )?
    .ok_or_else(|| "Archaeology predecessor has no review stream".to_string())?;
    if before.effective_lifecycle != expected_predecessor_lifecycle {
        return Err("Archaeology predecessor state changed; refresh before retrying".into());
    }
    let previous_event_id = last_event_id(
        transaction,
        &input.repository_id,
        &predecessor.stable_rule_identity,
    )?;
    let created_at = Utc::now().to_rfc3339();
    let continuity_edge_id = append_explicit_supersession(
        transaction,
        ArchaeologyExplicitSupersession {
            repository_id: &input.repository_id,
            predecessor_generation_id,
            predecessor_rule_id: &predecessor.occurrence_id,
            predecessor_rule_identity: &predecessor.stable_rule_identity,
            expected_predecessor_sequence: before.projected.last_sequence,
            expected_predecessor_event_id: previous_event_id.as_deref(),
            successor_generation_id: &input.generation_id,
            successor_rule_id: &successor.occurrence_id,
            successor_rule_identity: &successor.stable_rule_identity,
            continuity_identity: &predecessor.continuity_identity,
            successor_evidence_identity: &successor.evidence_identity,
            provenance: ArchaeologyReviewerProvenance {
                kind: ArchaeologyReviewerKind::Human,
                actor_id: "human:local".into(),
                authority_id: None,
            },
            created_at: &created_at,
        },
    )?;
    let after = project_current_lifecycle(
        transaction,
        &input.repository_id,
        &input.generation_id,
        &successor.occurrence_id,
        &successor.stable_rule_identity,
    )?
    .ok_or_else(|| "Updated archaeology successor stream is unavailable".to_string())?;
    Ok(ArchaeologyReviewMutationResult {
        repository_id: input.repository_id.clone(),
        generation_id: input.generation_id.clone(),
        rule_id: input.rule_id.clone(),
        lifecycle: after.effective_lifecycle,
        last_sequence: after.projected.last_sequence,
        last_event_id: after.projected.last_state_event_id,
        annotation_count: after.projected.annotations.len(),
        alias_rule_ids: Vec::new(),
        continuity_edge_id: Some(continuity_edge_id),
    })
}

fn require_ready_generation(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<(), String> {
    let ready = transaction
        .query_row(
            "SELECT 1 FROM archaeology_repositories repository
             JOIN archaeology_generations generation
               ON generation.generation_id=repository.ready_generation_id
              AND generation.repository_id=repository.repository_id
             WHERE repository.repository_id=?1 AND generation.generation_id=?2
               AND generation.status='ready' AND generation.schema_version=?3",
            params![
                repository_id,
                generation_id,
                i64::from(ARCHAEOLOGY_STORAGE_SCHEMA_VERSION)
            ],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| "Archaeology review scope lookup failed".to_string())?;
    ready.ok_or_else(|| "Archaeology review scope is unavailable".to_string())
}

fn require_reviewable_generation(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<(), String> {
    let available = transaction
        .query_row(
            "SELECT 1 FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2 AND schema_version=?3
               AND status IN ('ready','superseded')",
            params![
                repository_id,
                generation_id,
                i64::from(ARCHAEOLOGY_STORAGE_SCHEMA_VERSION)
            ],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| "Archaeology review scope lookup failed".to_string())?;
    available.ok_or_else(|| "Archaeology review scope is unavailable".to_string())
}

fn load_occurrence(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    stable_rule_identity: &str,
) -> Result<RuleOccurrence, String> {
    transaction
        .query_row(
            "SELECT rule_id,stable_rule_identity,continuity_identity,evidence_identity
             FROM archaeology_rules WHERE repository_id=?1 AND generation_id=?2
               AND stable_rule_identity=?3 AND identity_schema_version=2",
            (repository_id, generation_id, stable_rule_identity),
            |row| {
                Ok(RuleOccurrence {
                    occurrence_id: row.get(0)?,
                    stable_rule_identity: row.get(1)?,
                    continuity_identity: row.get(2)?,
                    evidence_identity: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|_| "Archaeology rule lookup failed".to_string())?
        .ok_or_else(|| "Archaeology rule is unavailable".to_string())
}

fn declared_lifecycle(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    occurrence_id: &str,
) -> Result<ArchaeologyRuleLifecycle, String> {
    let value = transaction
        .query_row(
            "SELECT lifecycle FROM archaeology_rules
             WHERE repository_id=?1 AND generation_id=?2 AND rule_id=?3",
            (repository_id, generation_id, occurrence_id),
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "Archaeology rule lifecycle lookup failed".to_string())?
        .ok_or_else(|| "Archaeology rule is unavailable".to_string())?;
    serde_json::from_value(serde_json::Value::String(value))
        .map_err(|_| "Archaeology rule lifecycle is invalid".to_string())
}

fn alias_sequence(
    transaction: &Transaction<'_>,
    repository_id: &str,
    alias_rule_identity: &str,
) -> Result<u64, String> {
    transaction
        .query_row(
            "SELECT COALESCE(MAX(logical_sequence),0)
             FROM archaeology_rule_alias_events
             WHERE repository_id=?1 AND alias_rule_identity=?2",
            (repository_id, alias_rule_identity),
            |row| row.get(0),
        )
        .map_err(|_| "Archaeology alias state lookup failed".to_string())
}

fn last_event_id(
    transaction: &Transaction<'_>,
    repository_id: &str,
    stable_rule_identity: &str,
) -> Result<Option<String>, String> {
    transaction
        .query_row(
            "SELECT event_id FROM archaeology_rule_review_events
             WHERE repository_id=?1 AND stable_rule_identity=?2
             ORDER BY logical_sequence DESC,event_id DESC LIMIT 1",
            (repository_id, stable_rule_identity),
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "Archaeology review state lookup failed".to_string())
}

fn validate_request_id(request_id: &str) -> Result<(), String> {
    if request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID_BYTES
        || request_id.chars().any(char::is_control)
    {
        return Err("Archaeology review request identity is invalid".into());
    }
    Ok(())
}

fn event_id(kind: &str, request_id: &str, rule_id: &str) -> String {
    let mut digest = Sha256::new();
    for value in ["archaeology-desktop-review:v1", kind, request_id, rule_id] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("sha256:{}", super::inventory::hex(&digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;

    const CREATED: &str = "2026-07-17T00:00:00Z";

    fn hash(label: &str) -> String {
        event_id("test", label, "fixture")
    }

    fn insert_rule(
        connection: &Connection,
        repository: &str,
        generation: &str,
        occurrence: &str,
        stable: &str,
        continuity: &str,
        evidence: &str,
    ) {
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                  identity_schema_version,stable_rule_identity,evidence_identity,
                  contradiction_identity,description_identity,continuity_identity,
                  parser_compatibility_identity,identity_provenance_json)
                 VALUES (?1,?2,?3,'revision','eligibility','fixture','candidate','deterministic',
                         'high',?4,?5,'{}',?6,2,?7,?8,?9,?10,?11,?12,'{}')",
                params![
                    generation,
                    occurrence,
                    repository,
                    hash("parser"),
                    hash("algorithm"),
                    CREATED,
                    stable,
                    evidence,
                    hash("contradiction"),
                    hash("description"),
                    continuity,
                    hash("parser-compatibility"),
                ],
            )
            .expect("rule");
    }

    fn fixture() -> (Connection, String, String, String, String, String) {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        let repository = hash("repository");
        let old_generation = "generation:old".to_string();
        let ready_generation = "generation:ready".to_string();
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
                  created_at,updated_at)
                 VALUES (?1,'/fixture',?2,'revision:ready',?3,?4,?4)",
                params![repository, hash("source"), ready_generation, CREATED],
            )
            .expect("repository");
        for (generation, revision, status) in [
            (&old_generation, "revision:old", "superseded"),
            (&ready_generation, "revision:ready", "ready"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_generations
                     (generation_id,repository_id,schema_version,revision_sha,source_identity,
                      parser_identity,algorithm_identity,config_identity,status,created_at)
                     VALUES (?1,?2,2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        generation,
                        repository,
                        revision,
                        hash("source"),
                        hash("parser"),
                        hash("algorithm"),
                        hash("config"),
                        status,
                        CREATED,
                    ],
                )
                .expect("generation");
        }
        let predecessor = hash("predecessor");
        let successor = hash("successor");
        let continuity = hash("continuity");
        insert_rule(
            &connection,
            &repository,
            &old_generation,
            "rule:old",
            &predecessor,
            &continuity,
            &hash("old-evidence"),
        );
        insert_rule(
            &connection,
            &repository,
            &ready_generation,
            "rule:ready",
            &successor,
            &hash("new-continuity"),
            &hash("new-evidence"),
        );
        let transaction = connection
            .unchecked_transaction()
            .expect("review transaction");
        let candidate = hash("candidate");
        let policy = ArchaeologyReviewerProvenance {
            kind: ArchaeologyReviewerKind::DeterministicPolicy,
            actor_id: "codevetter:local".into(),
            authority_id: Some("policy:test:v1".into()),
        };
        append_lifecycle_event(
            &transaction,
            ArchaeologyLifecycleAppend {
                event_id: &candidate,
                repository_id: &repository,
                generation_id: &old_generation,
                rule_id: "rule:old",
                stable_rule_identity: &predecessor,
                expected_previous_sequence: 0,
                expected_prior_event_id: None,
                related_generation_id: None,
                related_rule_id: None,
                provenance: policy,
                action: ArchaeologyLifecycleAction::Candidate,
                created_at: CREATED,
            },
        )
        .expect("candidate");
        append_lifecycle_event(
            &transaction,
            ArchaeologyLifecycleAppend {
                event_id: &hash("accepted"),
                repository_id: &repository,
                generation_id: &old_generation,
                rule_id: "rule:old",
                stable_rule_identity: &predecessor,
                expected_previous_sequence: 1,
                expected_prior_event_id: Some(&candidate),
                related_generation_id: None,
                related_rule_id: None,
                provenance: ArchaeologyReviewerProvenance {
                    kind: ArchaeologyReviewerKind::Human,
                    actor_id: "human:test".into(),
                    authority_id: None,
                },
                action: ArchaeologyLifecycleAction::Accept,
                created_at: CREATED,
            },
        )
        .expect("accepted");
        transaction.commit().expect("seed review stream");
        (
            connection,
            repository,
            old_generation,
            ready_generation,
            predecessor,
            successor,
        )
    }

    #[test]
    fn supersession_links_an_exact_prior_rule_to_the_current_ready_successor() {
        let (mut connection, repository, old_generation, ready_generation, predecessor, successor) =
            fixture();
        let result = mutate_review_core(
            &mut connection,
            ArchaeologyReviewMutationInput {
                request_id: "request:supersede".into(),
                repository_id: repository.clone(),
                generation_id: ready_generation.clone(),
                rule_id: successor.clone(),
                expected_lifecycle: ArchaeologyRuleLifecycle::Candidate,
                mutation: ArchaeologyReviewMutation::Supersede {
                    predecessor_generation_id: old_generation.clone(),
                    predecessor_rule_id: predecessor.clone(),
                    expected_predecessor_lifecycle: ArchaeologyRuleLifecycle::Accepted,
                },
            },
        )
        .expect("forward supersession");
        assert_eq!(result.lifecycle, ArchaeologyRuleLifecycle::ReviewNeeded);
        assert!(result.continuity_edge_id.is_some());
        assert_eq!(
            connection
                .query_row(
                    "SELECT decision FROM archaeology_rule_review_events
                     WHERE repository_id=?1 AND generation_id=?2 AND stable_rule_identity=?3
                     ORDER BY logical_sequence DESC LIMIT 1",
                    params![repository, old_generation, predecessor],
                    |row| row.get::<_, String>(0),
                )
                .expect("predecessor state"),
            "superseded"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT decision FROM archaeology_rule_review_events
                     WHERE repository_id=?1 AND generation_id=?2 AND stable_rule_identity=?3
                     ORDER BY logical_sequence DESC LIMIT 1",
                    params![repository, ready_generation, successor],
                    |row| row.get::<_, String>(0),
                )
                .expect("successor state"),
            "review_needed"
        );
    }

    #[test]
    fn mutation_contract_rejects_unknown_fields() {
        let error = serde_json::from_value::<ArchaeologyReviewMutationInput>(serde_json::json!({
            "request_id": "request:one",
            "repository_id": "repository",
            "generation_id": "generation",
            "rule_id": "rule",
            "expected_lifecycle": "candidate",
            "mutation": { "kind": "annotate", "annotation": "note" },
            "unexpected": true
        }))
        .expect_err("unknown input must fail");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn first_review_materializes_candidate_then_human_event_and_retry_is_idempotent() {
        let (mut connection, repository, _, ready_generation, _, successor) = fixture();
        let review = || ArchaeologyReviewMutationInput {
            request_id: "request:accept".into(),
            repository_id: repository.clone(),
            generation_id: ready_generation.clone(),
            rule_id: successor.clone(),
            expected_lifecycle: ArchaeologyRuleLifecycle::Candidate,
            mutation: ArchaeologyReviewMutation::Review {
                decision: ArchaeologyReviewDecision::Accept,
                reason: None,
            },
        };
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_rule_review_events
                     WHERE repository_id=?1 AND stable_rule_identity=?2",
                    params![repository, successor],
                    |row| row.get::<_, u64>(0),
                )
                .expect("initial event count"),
            0
        );
        let accepted = mutate_review_core(&mut connection, review()).expect("accept");
        assert_eq!(accepted.lifecycle, ArchaeologyRuleLifecycle::Accepted);
        assert_eq!(accepted.last_sequence, 2);
        let events = connection
            .prepare(
                "SELECT event_id,decision,logical_sequence,prior_event_id,actor_kind
                 FROM archaeology_rule_review_events
                 WHERE repository_id=?1 AND stable_rule_identity=?2
                 ORDER BY logical_sequence",
            )
            .expect("event query")
            .query_map(params![repository, successor], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .expect("event rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, "candidate");
        assert_eq!(events[0].2, 1);
        assert_eq!(events[0].3, None);
        assert_eq!(events[0].4, "deterministic_policy");
        assert_eq!(events[1].1, "accepted");
        assert_eq!(events[1].2, 2);
        assert_eq!(events[1].3.as_deref(), Some(events[0].0.as_str()));
        assert_eq!(events[1].4, "human");
        assert_eq!(accepted.last_event_id, events[1].0);

        let error = mutate_review_core(&mut connection, review()).expect_err("stale review");
        assert!(error.contains("state changed"));
        let rows_after = connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_review_events
                 WHERE repository_id=?1 AND stable_rule_identity=?2",
                params![repository, successor],
                |row| row.get::<_, u64>(0),
            )
            .expect("event count");
        assert_eq!(rows_after, 2);
    }

    #[test]
    fn invalid_first_review_rolls_back_the_lazy_candidate_baseline() {
        let (mut connection, repository, _, ready_generation, _, successor) = fixture();
        let error = mutate_review_core(
            &mut connection,
            ArchaeologyReviewMutationInput {
                request_id: "request:invalid-accept".into(),
                repository_id: repository.clone(),
                generation_id: ready_generation,
                rule_id: successor.clone(),
                expected_lifecycle: ArchaeologyRuleLifecycle::Candidate,
                mutation: ArchaeologyReviewMutation::Review {
                    decision: ArchaeologyReviewDecision::Accept,
                    reason: Some("not allowed".into()),
                },
            },
        )
        .expect_err("invalid accept");
        assert!(error.contains("does not take a rejection reason"));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_rule_review_events
                     WHERE repository_id=?1 AND stable_rule_identity=?2",
                    params![repository, successor],
                    |row| row.get::<_, u64>(0),
                )
                .expect("rolled-back event count"),
            0
        );
    }
}
