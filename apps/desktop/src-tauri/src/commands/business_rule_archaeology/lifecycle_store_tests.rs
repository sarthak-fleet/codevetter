use super::*;
use crate::db::archaeology_schema::run_migration;
use rusqlite::{Connection, TransactionBehavior};

const CREATED: &str = "2026-07-17T00:00:00Z";

fn hash(label: &str) -> String {
    digest_fields("lifecycle-store-test:v1", &[label])
}

fn human() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::Human,
        actor_id: "reviewer:local".into(),
        authority_id: None,
    }
}

fn policy() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::DeterministicPolicy,
        actor_id: "codevetter:local".into(),
        authority_id: Some("policy:review:v1".into()),
    }
}

fn model() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::Model,
        actor_id: "provider:fixture".into(),
        authority_id: Some("model:fixture:v1".into()),
    }
}

struct Fixture {
    connection: Connection,
    repository: String,
    old_generation: String,
    generation: String,
    rule: String,
    alias_one: String,
    alias_two: String,
    canonical: String,
    other: String,
    predecessor: String,
    successor: String,
    successor_evidence: String,
    shared_continuity: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_repository(hash("repository"))
    }

    fn with_repository(repository: String) -> Self {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("real migrated schema");
        let old_generation = "generation:old".to_string();
        let generation = "generation:current".to_string();
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
                  created_at,updated_at)
                 VALUES (?1,'/fixture','source:fixture','revision:current',?2,?3,?3)",
                params![repository, generation, CREATED],
            )
            .expect("repository");
        for (id, revision, status, parser) in [
            (&old_generation, "revision:old", "superseded", "parser:old"),
            (&generation, "revision:current", "ready", "parser:current"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_generations
                     (generation_id,repository_id,schema_version,revision_sha,source_identity,
                      parser_identity,algorithm_identity,config_identity,status,created_at)
                     VALUES (?1,?2,2,?3,?3,?4,'algorithm:v1','config:v1',?5,?6)",
                    params![id, repository, revision, parser, status, CREATED],
                )
                .expect("generation");
        }

        let rule = hash("rule");
        let alias_one = hash("alias-one");
        let alias_two = hash("alias-two");
        let canonical = hash("canonical");
        let other = hash("other");
        let predecessor = hash("predecessor");
        let successor = hash("successor");
        let successor_evidence = hash("evidence-successor");
        let shared_continuity = hash("shared-continuity");
        for (stable, generated, continuity, evidence) in [
            (
                &rule,
                "rule:current",
                hash("continuity-rule"),
                hash("evidence-rule"),
            ),
            (
                &alias_one,
                "rule:alias-one",
                hash("continuity-alias-one"),
                hash("evidence-alias-one"),
            ),
            (
                &alias_two,
                "rule:alias-two",
                hash("continuity-alias-two"),
                hash("evidence-alias-two"),
            ),
            (
                &canonical,
                "rule:canonical",
                hash("continuity-canonical"),
                hash("evidence-canonical"),
            ),
            (
                &other,
                "rule:other",
                hash("continuity-other"),
                hash("evidence-other"),
            ),
            (
                &successor,
                "rule:successor",
                hash("successor-initial-continuity"),
                successor_evidence.clone(),
            ),
        ] {
            insert_rule(
                &connection,
                &repository,
                &generation,
                generated,
                stable,
                &continuity,
                &evidence,
                "parser:fixture:v1",
            );
        }
        insert_rule(
            &connection,
            &repository,
            &old_generation,
            "rule:predecessor",
            &predecessor,
            &shared_continuity,
            &hash("evidence-predecessor"),
            "parser:fixture:v1",
        );
        Self {
            connection,
            repository,
            old_generation,
            generation,
            rule,
            alias_one,
            alias_two,
            canonical,
            other,
            predecessor,
            successor,
            successor_evidence,
            shared_continuity,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_rule(
    connection: &Connection,
    repository: &str,
    generation: &str,
    generated_rule_id: &str,
    stable_rule_identity: &str,
    continuity_identity: &str,
    evidence_identity: &str,
    parser_identity: &str,
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
                     'high',?4,'algorithm:v1','{}',?5,2,?6,?7,?8,?9,?10,?11,'{}')",
            params![
                generation,
                generated_rule_id,
                repository,
                parser_identity,
                CREATED,
                stable_rule_identity,
                evidence_identity,
                hash("contradiction-none"),
                hash("description-original"),
                continuity_identity,
                hash(parser_identity),
            ],
        )
        .expect("v2 rule");
}

fn make_generation_alias_compatible(
    connection: &Connection,
    generation_id: &str,
    alias_rule_id: &str,
    canonical_rule_id: &str,
) {
    connection
        .execute(
            "UPDATE archaeology_rules AS alias
             SET stable_rule_identity=canonical.stable_rule_identity,
                 continuity_identity=canonical.continuity_identity,
                 parser_compatibility_identity=canonical.parser_compatibility_identity,
                 contradiction_identity=canonical.contradiction_identity
             FROM archaeology_rules AS canonical
             WHERE alias.generation_id=?1 AND alias.rule_id=?2
               AND canonical.generation_id=?1 AND canonical.rule_id=?3",
            params![generation_id, alias_rule_id, canonical_rule_id],
        )
        .expect("compatible generation alias");
}

fn append_candidate(
    transaction: &Transaction<'_>,
    fixture: &Fixture,
    event_id: &str,
) -> ArchaeologyStoredLifecycleProjection {
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: CREATED,
        },
    )
    .expect("candidate")
}

fn prepare_reconciliation_with_prior(fixture: &Fixture) {
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&fixture.generation],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='ready' WHERE generation_id=?1",
            [&fixture.old_generation],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE archaeology_repositories SET ready_generation_id=?1
             WHERE repository_id=?2",
            params![fixture.old_generation, fixture.repository],
        )
        .unwrap();
    insert_rule(
        &fixture.connection,
        &fixture.repository,
        &fixture.old_generation,
        "rule:old-current",
        &fixture.rule,
        &hash("continuity-rule"),
        &hash("evidence-rule"),
        "parser:fixture:v1",
    );
}

fn accept_prior_logical_rule(transaction: &Transaction<'_>, fixture: &Fixture) {
    let projected = ensure_candidate_lifecycle(
        transaction,
        &fixture.repository,
        &fixture.old_generation,
        "rule:old-current",
        &fixture.rule,
        CREATED,
    )
    .unwrap();
    assert_eq!(projected.projected.last_sequence, 1);
    let candidate = transaction
        .query_row(
            "SELECT event_id FROM archaeology_rule_review_events
             WHERE repository_id=?1 AND stable_rule_identity=?2
             ORDER BY logical_sequence DESC LIMIT 1",
            params![fixture.repository, fixture.rule],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("reconciliation-prior-accepted"),
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:old-current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .unwrap();
}

fn accept_predecessor(transaction: &Transaction<'_>, fixture: &Fixture) -> String {
    let candidate = hash("explicit-predecessor-candidate");
    let accepted = hash("explicit-predecessor-accepted");
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &candidate,
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: CREATED,
        },
    )
    .unwrap();
    append_lifecycle_event(
        transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .unwrap();
    accepted
}

fn explicit_supersession<'a>(
    fixture: &'a Fixture,
    expected_event_id: &'a str,
) -> ArchaeologyExplicitSupersession<'a> {
    ArchaeologyExplicitSupersession {
        repository_id: &fixture.repository,
        predecessor_generation_id: &fixture.old_generation,
        predecessor_rule_id: "rule:predecessor",
        predecessor_rule_identity: &fixture.predecessor,
        expected_predecessor_sequence: 2,
        expected_predecessor_event_id: Some(expected_event_id),
        successor_generation_id: &fixture.generation,
        successor_rule_id: "rule:successor",
        successor_rule_identity: &fixture.successor,
        continuity_identity: &fixture.shared_continuity,
        successor_evidence_identity: &fixture.successor_evidence,
        provenance: human(),
        created_at: CREATED,
    }
}

#[test]
fn lifecycle_append_enforces_cas_sequence_and_model_decision_boundary() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let candidate = hash("event-candidate");
    let accepted = hash("event-accepted");
    let annotation = hash("event-annotation");
    append_candidate(&transaction, &fixture, &candidate);

    let stale = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("event-stale"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(stale.contains("compare-and-swap"), "{stale}");

    let projected = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .expect("accept");
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );

    let projected = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &annotation,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 2,
            expected_prior_event_id: Some(&accepted),
            related_generation_id: None,
            related_rule_id: None,
            provenance: model(),
            action: ArchaeologyLifecycleAction::Annotate {
                annotation: "Wording needs human review.".into(),
            },
            created_at: CREATED,
        },
    )
    .expect("model annotation");
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );
    assert_eq!(projected.projected.last_sequence, 3);
    assert_eq!(projected.projected.annotations.len(), 1);

    let denial = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("event-model-reject"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 3,
            expected_prior_event_id: Some(&annotation),
            related_generation_id: None,
            related_rule_id: None,
            provenance: model(),
            action: ArchaeologyLifecycleAction::Reject {
                reason: "model decision".into(),
            },
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(denial.contains("model"), "{denial}");
    assert_eq!(
        transaction
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_review_events",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        3
    );
}

#[test]
fn lifecycle_selection_requires_exact_occurrence_when_stable_identity_is_shared() {
    let fixture = Fixture::new();
    insert_rule(
        &fixture.connection,
        &fixture.repository,
        &fixture.generation,
        "rule:generated-alias-occurrence",
        &fixture.rule,
        &hash("generated-alias-continuity"),
        &hash("generated-alias-evidence"),
        "parser:generated:v1",
    );
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let mismatched = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("ambiguous-mismatch"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:canonical",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(mismatched.contains("unavailable"), "{mismatched}");

    let projected = append_candidate(&transaction, &fixture, &hash("exact-primary-occurrence"));
    assert_eq!(projected.current_snapshot.rule_id, fixture.rule);
}

#[test]
fn annotation_does_not_rebase_an_accepted_decision_after_evidence_drift() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let candidate = hash("annotation-drift-candidate");
    let accepted = hash("annotation-drift-accepted");
    append_candidate(&transaction, &fixture, &candidate);
    append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .expect("accept");
    transaction
        .execute(
            "UPDATE archaeology_rules SET evidence_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:current'",
            params![hash("annotation-drift-evidence"), fixture.generation],
        )
        .unwrap();

    let projected = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("annotation-after-drift"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 2,
            expected_prior_event_id: Some(&accepted),
            related_generation_id: None,
            related_rule_id: None,
            provenance: model(),
            action: ArchaeologyLifecycleAction::Annotate {
                annotation: "The wording may need attention.".into(),
            },
            created_at: CREATED,
        },
    )
    .expect("annotation remains non-authoritative");
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );
    assert_eq!(
        projected.compatibility_mismatches,
        [ArchaeologyCompatibilityMismatch::Evidence]
    );
}

#[test]
fn failed_lifecycle_preflight_does_not_append_an_event() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let candidate = hash("preflight-candidate");
    let accepted = hash("preflight-accepted");
    append_candidate(&transaction, &fixture, &candidate);
    append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .unwrap();
    transaction
        .execute(
            "UPDATE archaeology_rules SET continuity_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:current'",
            params![hash("ambiguous-continuity"), fixture.generation],
        )
        .unwrap();

    let error = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("preflight-must-not-persist"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 2,
            expected_prior_event_id: Some(&accepted),
            related_generation_id: None,
            related_rule_id: None,
            provenance: model(),
            action: ArchaeologyLifecycleAction::Annotate {
                annotation: "must not persist".into(),
            },
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(error.contains("continuity is ambiguous"), "{error}");
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 2);
}

#[test]
fn generation_reconciliation_first_publish_is_deterministic_and_retry_safe() {
    let fixture = Fixture::with_repository("repo:jobs".into());
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&fixture.generation],
        )
        .unwrap();
    make_generation_alias_compatible(
        &fixture.connection,
        &fixture.generation,
        "rule:alias-one",
        "rule:canonical",
    );
    fixture
        .connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES (?1,'relation:generated-alias','rule:alias-one','rule:canonical',
                     'aliases','deterministic')",
            [&fixture.generation],
        )
        .unwrap();
    fixture
        .connection
        .execute(
            "UPDATE archaeology_repositories SET ready_generation_id=NULL
             WHERE repository_id=?1",
            [&fixture.repository],
        )
        .unwrap();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");

    let appended = reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        None,
        CREATED,
    )
    .expect("first lifecycle publication");
    assert_eq!(appended, 0);
    assert_eq!(
        transaction
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_review_events
                 WHERE rule_id='rule:alias-one'",
                [],
                |row| row.get::<_, usize>(0),
            )
            .unwrap(),
        0,
        "generation-local aliases are not lifecycle candidates"
    );
    let event_ids = transaction
        .prepare(
            "SELECT event_id FROM archaeology_rule_review_events
             ORDER BY stable_rule_identity",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(event_ids.is_empty(), "unreviewed candidates stay implicit");

    assert_eq!(
        reconcile_generation_lifecycle(
            &transaction,
            &fixture.repository,
            &fixture.generation,
            None,
            "2026-07-17T01:00:00Z",
        )
        .expect("retry"),
        0
    );
    let retried_ids = transaction
        .prepare(
            "SELECT event_id FROM archaeology_rule_review_events
             ORDER BY stable_rule_identity",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(retried_ids, event_ids);
}

#[test]
fn generation_alias_relations_require_exact_compatible_direct_stars() {
    let incompatible = Fixture::new();
    incompatible
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&incompatible.generation],
        )
        .unwrap();
    incompatible
        .connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES (?1,'relation:incompatible','rule:alias-one','rule:canonical',
                     'aliases','deterministic')",
            [&incompatible.generation],
        )
        .unwrap();
    let transaction = rusqlite::Transaction::new_unchecked(
        &incompatible.connection,
        TransactionBehavior::Immediate,
    )
    .unwrap();
    let error = reconcile_generation_lifecycle(
        &transaction,
        &incompatible.repository,
        &incompatible.generation,
        None,
        CREATED,
    )
    .unwrap_err();
    assert!(error.contains("semantically compatible"), "{error}");
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 0);

    let chain = Fixture::new();
    chain
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&chain.generation],
        )
        .unwrap();
    for alias in ["rule:alias-one", "rule:alias-two"] {
        make_generation_alias_compatible(
            &chain.connection,
            &chain.generation,
            alias,
            "rule:canonical",
        );
    }
    chain
        .connection
        .execute_batch(&format!(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES ('{}','relation:chain-one','rule:alias-one','rule:alias-two',
                     'aliases','deterministic'),
                    ('{}','relation:chain-two','rule:alias-two','rule:canonical',
                     'aliases','deterministic');",
            chain.generation, chain.generation
        ))
        .unwrap();
    let transaction =
        rusqlite::Transaction::new_unchecked(&chain.connection, TransactionBehavior::Immediate)
            .unwrap();
    let error = reconcile_generation_lifecycle(
        &transaction,
        &chain.repository,
        &chain.generation,
        None,
        CREATED,
    )
    .unwrap_err();
    assert!(error.contains("direct stars"), "{error}");
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 0);

    let exact = Fixture::new();
    exact
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&exact.generation],
        )
        .unwrap();
    make_generation_alias_compatible(
        &exact.connection,
        &exact.generation,
        "rule:alias-one",
        "rule:canonical",
    );
    exact
        .connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES (?1,'relation:exact','rule:alias-one','rule:canonical',
                     'aliases','deterministic')",
            [&exact.generation],
        )
        .unwrap();
    let transaction =
        rusqlite::Transaction::new_unchecked(&exact.connection, TransactionBehavior::Immediate)
            .unwrap();
    validate_generation_alias_relations(&transaction, &exact.repository, &exact.generation)
        .expect("compatible direct alias star with distinct evidence");
    assert_ne!(
        transaction
            .query_row(
                "SELECT evidence_identity FROM archaeology_rules
                 WHERE generation_id=?1 AND rule_id='rule:alias-one'",
                [&exact.generation],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        transaction
            .query_row(
                "SELECT evidence_identity FROM archaeology_rules
                 WHERE generation_id=?1 AND rule_id='rule:canonical'",
                [&exact.generation],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    );

    let untrusted = Fixture::new();
    make_generation_alias_compatible(
        &untrusted.connection,
        &untrusted.generation,
        "rule:alias-one",
        "rule:canonical",
    );
    untrusted
        .connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES (?1,'relation:untrusted','rule:alias-one','rule:canonical',
                     'aliases','model_synthesized')",
            [&untrusted.generation],
        )
        .unwrap();
    let transaction =
        rusqlite::Transaction::new_unchecked(&untrusted.connection, TransactionBehavior::Immediate)
            .unwrap();
    let error = validate_generation_alias_relations(
        &transaction,
        &untrusted.repository,
        &untrusted.generation,
    )
    .unwrap_err();
    assert!(error.contains("non-deterministic alias"), "{error}");

    let cross_scope = Fixture::new();
    let foreign_repository = hash("generation-alias-foreign-repository");
    cross_scope
        .connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES (?1,'/foreign-alias','source','revision',?2,?2)",
            params![foreign_repository, CREATED],
        )
        .unwrap();
    cross_scope
        .connection
        .execute(
            "UPDATE archaeology_rules SET repository_id=?1
             WHERE generation_id=?2 AND rule_id='rule:alias-one'",
            params![foreign_repository, cross_scope.generation],
        )
        .unwrap();
    cross_scope
        .connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES (?1,'relation:cross-scope','rule:alias-one','rule:canonical',
                     'aliases','deterministic')",
            [&cross_scope.generation],
        )
        .unwrap();
    let transaction = rusqlite::Transaction::new_unchecked(
        &cross_scope.connection,
        TransactionBehavior::Immediate,
    )
    .unwrap();
    let error = validate_generation_alias_relations(
        &transaction,
        &cross_scope.repository,
        &cross_scope.generation,
    )
    .unwrap_err();
    assert!(error.contains("outside exact scope"), "{error}");
}

#[test]
fn generation_reconciliation_does_not_carry_an_ancient_stream_across_missing_prior() {
    let fixture = Fixture::new();
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generations SET status='staging' WHERE generation_id=?1",
            [&fixture.generation],
        )
        .unwrap();
    insert_rule(
        &fixture.connection,
        &fixture.repository,
        &fixture.old_generation,
        "rule:old-current",
        &fixture.rule,
        &hash("continuity-rule"),
        &hash("evidence-rule"),
        "parser:fixture:v1",
    );
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    accept_prior_logical_rule(&transaction, &fixture);
    transaction
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,created_at)
             VALUES ('generation:immediate-prior',?1,2,'revision:prior','source:prior',
                     'parser:prior','algorithm:v1','config:v1','ready',?2)",
            params![fixture.repository, CREATED],
        )
        .unwrap();
    transaction
        .execute(
            "UPDATE archaeology_repositories SET ready_generation_id='generation:immediate-prior'
             WHERE repository_id=?1",
            [&fixture.repository],
        )
        .unwrap();

    reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        Some("generation:immediate-prior"),
        CREATED,
    )
    .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );
    assert_eq!(projected.projected.last_sequence, 3);
}

#[test]
fn generation_reconciliation_rejects_duplicate_prior_canonical_identity() {
    let fixture = Fixture::new();
    prepare_reconciliation_with_prior(&fixture);
    insert_rule(
        &fixture.connection,
        &fixture.repository,
        &fixture.old_generation,
        "rule:duplicate-old-current",
        &fixture.rule,
        &hash("duplicate-prior-continuity"),
        &hash("duplicate-prior-evidence"),
        "parser:fixture:v1",
    );
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let error = reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        Some(&fixture.old_generation),
        CREATED,
    )
    .unwrap_err();
    assert!(
        error.contains("Prior ready generation contains duplicate canonical"),
        "{error}"
    );
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 0);
}

#[test]
fn generation_reconciliation_preserves_acceptance_across_prose_only_change() {
    let fixture = Fixture::new();
    prepare_reconciliation_with_prior(&fixture);
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    accept_prior_logical_rule(&transaction, &fixture);
    transaction
        .execute(
            "UPDATE archaeology_rules SET description_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:current'",
            params![hash("description-reworded"), fixture.generation],
        )
        .unwrap();

    reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        Some(&fixture.old_generation),
        CREATED,
    )
    .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );
    assert!(projected.description_changed);
    assert_eq!(projected.projected.last_sequence, 2);
}

#[test]
fn generation_reconciliation_marks_changed_evidence_review_needed() {
    let fixture = Fixture::new();
    prepare_reconciliation_with_prior(&fixture);
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    accept_prior_logical_rule(&transaction, &fixture);
    transaction
        .execute(
            "UPDATE archaeology_rules SET evidence_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:current'",
            params![hash("reconciled-evidence-change"), fixture.generation],
        )
        .unwrap();

    reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        Some(&fixture.old_generation),
        CREATED,
    )
    .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );
    assert_eq!(projected.projected.last_sequence, 3);
    assert_eq!(
        projected.projected.decision_provenance, None,
        "automatic review-needed is not human acceptance"
    );
}

#[test]
fn generation_reconciliation_conflicts_accepted_contradiction_change() {
    let fixture = Fixture::new();
    prepare_reconciliation_with_prior(&fixture);
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    accept_prior_logical_rule(&transaction, &fixture);
    transaction
        .execute(
            "UPDATE archaeology_rules SET contradiction_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:current'",
            params![hash("reconciled-contradiction-change"), fixture.generation],
        )
        .unwrap();

    reconcile_generation_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        Some(&fixture.old_generation),
        CREATED,
    )
    .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Conflicted
    );
    assert_eq!(projected.projected.last_sequence, 3);
}

#[test]
fn projection_preserves_prose_only_decisions_and_invalidates_exact_drift() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let candidate = hash("compat-candidate");
    let accepted = hash("compat-accepted");
    append_candidate(&transaction, &fixture, &candidate);
    append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:current",
            stable_rule_identity: &fixture.rule,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .expect("accept");

    transaction
        .execute(
            "UPDATE archaeology_rules SET description_identity=?1
             WHERE generation_id=?2 AND stable_rule_identity=?3",
            params![
                hash("description-improved"),
                fixture.generation,
                fixture.rule
            ],
        )
        .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );
    assert!(projected.description_changed);

    transaction
        .execute(
            "UPDATE archaeology_rules SET evidence_identity=?1
             WHERE generation_id=?2 AND stable_rule_identity=?3",
            params![hash("evidence-changed"), fixture.generation, fixture.rule],
        )
        .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );
    assert_eq!(
        projected.compatibility_mismatches,
        [ArchaeologyCompatibilityMismatch::Evidence]
    );

    transaction
        .execute(
            "UPDATE archaeology_rules SET evidence_identity=?1,parser_compatibility_identity=?2
             WHERE generation_id=?3 AND stable_rule_identity=?4",
            params![
                hash("evidence-rule"),
                hash("parser:v2"),
                fixture.generation,
                fixture.rule
            ],
        )
        .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.compatibility_mismatches,
        [ArchaeologyCompatibilityMismatch::Parser]
    );

    transaction
        .execute(
            "UPDATE archaeology_rules SET parser_compatibility_identity=?1,
             contradiction_identity=?2
             WHERE generation_id=?3 AND stable_rule_identity=?4",
            params![
                hash("parser:fixture:v1"),
                hash("contradiction-changed"),
                fixture.generation,
                fixture.rule
            ],
        )
        .unwrap();
    let projected = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        projected.effective_lifecycle,
        ArchaeologyRuleLifecycle::Conflicted
    );
    assert_eq!(
        projected.compatibility_mismatches,
        [ArchaeologyCompatibilityMismatch::Contradiction]
    );
}

#[test]
fn alias_projection_supports_unlink_and_rejects_stale_self_chains_and_cycles() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let first = hash("alias-link-one");
    let second = hash("alias-link-two");
    let links = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &first,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .expect("first alias");
    assert_eq!(links.len(), 1);
    let links = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &second,
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-two",
            alias_rule_identity: &fixture.alias_two,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: policy(),
            created_at: CREATED,
        },
    )
    .expect("second alias");
    assert_eq!(links.len(), 2);

    let stale = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("alias-stale"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Unlinked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(stale.contains("compare-and-swap"), "{stale}");

    let self_alias = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("alias-self"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:other",
            alias_rule_identity: &fixture.other,
            canonical_rule_id: "rule:other",
            canonical_rule_identity: &fixture.other,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(self_alias.contains("itself"), "{self_alias}");

    let alias_to_alias = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("alias-chain"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:canonical",
            alias_rule_identity: &fixture.canonical,
            canonical_rule_id: "rule:other",
            canonical_rule_identity: &fixture.other,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(
        alias_to_alias.contains("cannot itself be an alias"),
        "{alias_to_alias}"
    );

    let cycle = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("alias-cycle"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:canonical",
            alias_rule_identity: &fixture.canonical,
            canonical_rule_id: "rule:alias-one",
            canonical_rule_identity: &fixture.alias_one,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(
        cycle.contains("cycle") || cycle.contains("alias"),
        "{cycle}"
    );

    let links = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("alias-unlink-one"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 1,
            action: ArchaeologyAliasAction::Unlinked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .expect("unlink");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].alias_rule_id, fixture.alias_two);
}

#[test]
fn failed_alias_preflight_does_not_append_an_unmatched_unlink() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("preflight-alias-link"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap();
    transaction
        .execute(
            "UPDATE archaeology_rules SET continuity_identity=?1
             WHERE generation_id=?2 AND rule_id='rule:alias-one'",
            params![hash("alias-continuity-moved"), fixture.generation],
        )
        .unwrap();

    let error = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("preflight-alias-unlink"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Unlinked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(error.contains("unmatched unlink"), "{error}");
    assert_eq!(count(&transaction, "archaeology_rule_alias_events"), 1);
}

#[test]
fn exact_scope_and_continuity_edges_fail_closed() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let foreign_repository = hash("foreign-repository");
    transaction
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES (?1,'/foreign','source','revision',?2,?2)",
            params![foreign_repository, CREATED],
        )
        .unwrap();
    let cross_scope = append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("cross-scope"),
            repository_id: &foreign_repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(cross_scope.contains("unavailable"), "{cross_scope}");

    let edge = append_continuity_edge(
        &transaction,
        ArchaeologyContinuityAppend {
            repository_id: &fixture.repository,
            continuity_identity: &fixture.shared_continuity,
            predecessor_rule_id: "rule:predecessor",
            predecessor_rule_identity: &fixture.predecessor,
            successor_rule_id: "rule:successor",
            successor_rule_identity: &fixture.successor,
            predecessor_generation_id: &fixture.old_generation,
            successor_generation_id: &fixture.generation,
            kind: ArchaeologyContinuityKind::Supersedes,
            evidence_identity: &hash("evidence-successor"),
            provenance: human(),
            created_at: CREATED,
        },
    )
    .expect("explicit continuity edge");
    validate_digest("edge", &edge).unwrap();
    assert_eq!(
        transaction
            .query_row(
                "SELECT kind FROM archaeology_rule_continuity_edges WHERE edge_identity=?1",
                [&edge],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "supersedes"
    );

    let wrong_evidence = append_continuity_edge(
        &transaction,
        ArchaeologyContinuityAppend {
            repository_id: &fixture.repository,
            continuity_identity: &fixture.shared_continuity,
            predecessor_rule_id: "rule:predecessor",
            predecessor_rule_identity: &fixture.predecessor,
            successor_rule_id: "rule:successor",
            successor_rule_identity: &fixture.successor,
            predecessor_generation_id: &fixture.old_generation,
            successor_generation_id: &fixture.generation,
            kind: ArchaeologyContinuityKind::Supersedes,
            evidence_identity: &hash("wrong-evidence"),
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(
        wrong_evidence.contains("successor snapshot"),
        "{wrong_evidence}"
    );
}

#[test]
fn explicit_supersession_atomically_links_and_resets_successor_review() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let accepted = accept_predecessor(&transaction, &fixture);
    let edge =
        append_explicit_supersession(&transaction, explicit_supersession(&fixture, &accepted))
            .expect("atomic explicit supersession");
    validate_digest("edge", &edge).unwrap();

    let predecessor = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.old_generation,
        "rule:predecessor",
        &fixture.predecessor,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        predecessor.effective_lifecycle,
        ArchaeologyRuleLifecycle::Superseded
    );
    assert_eq!(predecessor.projected.last_sequence, 3);
    assert_eq!(
        predecessor.projected.successor_rule_id,
        Some(fixture.successor.clone())
    );
    let successor = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:successor",
        &fixture.successor,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        successor.effective_lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );
    assert_eq!(successor.projected.last_sequence, 2);
    assert_eq!(successor.projected.decision_provenance, None);
    assert_eq!(count(&transaction, "archaeology_rule_continuity_edges"), 1);
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 5);
    assert_eq!(
        transaction
            .query_row(
                "SELECT continuity_identity FROM archaeology_rules
                 WHERE generation_id=?1 AND rule_id='rule:successor'",
                [&fixture.generation],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        hash("successor-initial-continuity")
    );
}

#[test]
fn explicit_supersession_duplicate_retry_rejects_without_partial_rows() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let accepted = accept_predecessor(&transaction, &fixture);
    append_explicit_supersession(&transaction, explicit_supersession(&fixture, &accepted)).unwrap();
    let before = (
        count(&transaction, "archaeology_rule_continuity_edges"),
        count(&transaction, "archaeology_rule_review_events"),
    );

    let error =
        append_explicit_supersession(&transaction, explicit_supersession(&fixture, &accepted))
            .unwrap_err();
    assert!(
        error.contains("ambiguity") || error.contains("compare-and-swap"),
        "{error}"
    );
    assert_eq!(
        (
            count(&transaction, "archaeology_rule_continuity_edges"),
            count(&transaction, "archaeology_rule_review_events"),
        ),
        before
    );
}

#[test]
fn standalone_supersede_and_model_authority_are_rejected_without_writes() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let accepted = accept_predecessor(&transaction, &fixture);
    let standalone = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("standalone-supersede"),
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 2,
            expected_prior_event_id: Some(&accepted),
            related_generation_id: Some(&fixture.generation),
            related_rule_id: Some("rule:successor"),
            provenance: human(),
            action: ArchaeologyLifecycleAction::Supersede {
                successor_rule_id: fixture.successor.clone(),
            },
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(standalone.contains("exact continuity edge"), "{standalone}");
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 2);

    let mut model_input = explicit_supersession(&fixture, &accepted);
    model_input.provenance = model();
    let model_error = append_explicit_supersession(&transaction, model_input).unwrap_err();
    assert!(model_error.contains("model"), "{model_error}");
    assert_eq!(count(&transaction, "archaeology_rule_continuity_edges"), 0);
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 2);
}

#[test]
fn explicit_supersession_rejects_wrong_scope_kind_and_reverse_time() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let accepted = accept_predecessor(&transaction, &fixture);

    let mut wrong_scope = explicit_supersession(&fixture, &accepted);
    wrong_scope.repository_id = "repo:foreign";
    let error = append_explicit_supersession(&transaction, wrong_scope).unwrap_err();
    assert!(error.contains("unavailable"), "{error}");

    transaction
        .execute(
            "UPDATE archaeology_rules SET kind='routing'
             WHERE generation_id=?1 AND rule_id='rule:successor'",
            [&fixture.generation],
        )
        .unwrap();
    let error =
        append_explicit_supersession(&transaction, explicit_supersession(&fixture, &accepted))
            .unwrap_err();
    assert!(error.contains("rule kinds"), "{error}");
    transaction
        .execute(
            "UPDATE archaeology_rules SET kind='eligibility'
             WHERE generation_id=?1 AND rule_id='rule:successor'",
            [&fixture.generation],
        )
        .unwrap();

    let reverse_continuity = hash("successor-initial-continuity");
    let predecessor_evidence = hash("evidence-predecessor");
    let error = append_explicit_supersession(
        &transaction,
        ArchaeologyExplicitSupersession {
            repository_id: &fixture.repository,
            predecessor_generation_id: &fixture.generation,
            predecessor_rule_id: "rule:successor",
            predecessor_rule_identity: &fixture.successor,
            expected_predecessor_sequence: 0,
            expected_predecessor_event_id: None,
            successor_generation_id: &fixture.old_generation,
            successor_rule_id: "rule:predecessor",
            successor_rule_identity: &fixture.predecessor,
            continuity_identity: &reverse_continuity,
            successor_evidence_identity: &predecessor_evidence,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap_err();
    assert!(error.contains("reverse generation order"), "{error}");
    assert_eq!(count(&transaction, "archaeology_rule_continuity_edges"), 0);
    assert_eq!(count(&transaction, "archaeology_rule_review_events"), 2);
}

#[test]
fn explicit_supersession_rejects_split_merge_and_cycle_ambiguity() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let accepted = accept_predecessor(&transaction, &fixture);
    transaction
        .execute(
            "INSERT INTO archaeology_rule_continuity_edges
             (edge_identity,repository_id,continuity_identity,predecessor_rule_identity,
              successor_rule_identity,predecessor_generation_id,successor_generation_id,
              kind,evidence_identity,provenance_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'split',?8,'{}',?9)",
            params![
                hash("existing-split"),
                fixture.repository,
                fixture.shared_continuity,
                fixture.predecessor,
                fixture.other,
                fixture.old_generation,
                fixture.generation,
                hash("split-evidence"),
                CREATED,
            ],
        )
        .unwrap();
    let before = (
        count(&transaction, "archaeology_rule_continuity_edges"),
        count(&transaction, "archaeology_rule_review_events"),
    );
    let error =
        append_explicit_supersession(&transaction, explicit_supersession(&fixture, &accepted))
            .unwrap_err();
    assert!(error.contains("split, merge"), "{error}");
    assert_eq!(
        (
            count(&transaction, "archaeology_rule_continuity_edges"),
            count(&transaction, "archaeology_rule_review_events"),
        ),
        before
    );

    let cycle_fixture = Fixture::new();
    let cycle_transaction = rusqlite::Transaction::new_unchecked(
        &cycle_fixture.connection,
        TransactionBehavior::Immediate,
    )
    .expect("cycle transaction");
    let cycle_accepted = accept_predecessor(&cycle_transaction, &cycle_fixture);
    cycle_transaction
        .execute(
            "INSERT INTO archaeology_rule_continuity_edges
             (edge_identity,repository_id,continuity_identity,predecessor_rule_identity,
              successor_rule_identity,predecessor_generation_id,successor_generation_id,
              kind,evidence_identity,provenance_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'supersedes',?8,'{}',?9)",
            params![
                hash("existing-reverse-edge"),
                cycle_fixture.repository,
                hash("successor-initial-continuity"),
                cycle_fixture.successor,
                cycle_fixture.predecessor,
                cycle_fixture.generation,
                cycle_fixture.old_generation,
                hash("evidence-predecessor"),
                CREATED,
            ],
        )
        .unwrap();
    let error = append_explicit_supersession(
        &cycle_transaction,
        explicit_supersession(&cycle_fixture, &cycle_accepted),
    )
    .unwrap_err();
    assert!(error.contains("cycle"), "{error}");
    assert_eq!(
        count(&cycle_transaction, "archaeology_rule_continuity_edges"),
        1
    );
    assert_eq!(
        count(&cycle_transaction, "archaeology_rule_review_events"),
        2
    );
}

#[test]
fn accepted_condition_change_is_explicitly_superseded_without_carrying_acceptance() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let candidate = hash("predecessor-candidate");
    let accepted = hash("predecessor-accepted");
    append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &candidate,
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: CREATED,
        },
    )
    .unwrap();
    append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &accepted,
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 1,
            expected_prior_event_id: Some(&candidate),
            related_generation_id: None,
            related_rule_id: None,
            provenance: human(),
            action: ArchaeologyLifecycleAction::Accept,
            created_at: CREATED,
        },
    )
    .unwrap();

    append_continuity_edge(
        &transaction,
        ArchaeologyContinuityAppend {
            repository_id: &fixture.repository,
            continuity_identity: &fixture.shared_continuity,
            predecessor_rule_id: "rule:predecessor",
            predecessor_rule_identity: &fixture.predecessor,
            successor_rule_id: "rule:successor",
            successor_rule_identity: &fixture.successor,
            predecessor_generation_id: &fixture.old_generation,
            successor_generation_id: &fixture.generation,
            kind: ArchaeologyContinuityKind::Supersedes,
            evidence_identity: &hash("evidence-successor"),
            provenance: human(),
            created_at: CREATED,
        },
    )
    .expect("reviewed explicit successor");
    assert_eq!(
        transaction
            .query_row(
                "SELECT continuity_identity FROM archaeology_rules
                 WHERE generation_id=?1 AND rule_id='rule:successor'",
                [&fixture.generation],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        hash("successor-initial-continuity")
    );

    let superseded = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("predecessor-superseded"),
            repository_id: &fixture.repository,
            generation_id: &fixture.old_generation,
            rule_id: "rule:predecessor",
            stable_rule_identity: &fixture.predecessor,
            expected_previous_sequence: 2,
            expected_prior_event_id: Some(&accepted),
            related_generation_id: Some(&fixture.generation),
            related_rule_id: Some("rule:successor"),
            provenance: human(),
            action: ArchaeologyLifecycleAction::Supersede {
                successor_rule_id: fixture.successor.clone(),
            },
            created_at: CREATED,
        },
    )
    .expect("supersede predecessor");
    assert_eq!(
        superseded.effective_lifecycle,
        ArchaeologyRuleLifecycle::Superseded
    );

    let successor = append_lifecycle_event(
        &transaction,
        ArchaeologyLifecycleAppend {
            event_id: &hash("successor-candidate"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            rule_id: "rule:successor",
            stable_rule_identity: &fixture.successor,
            expected_previous_sequence: 0,
            expected_prior_event_id: None,
            related_generation_id: None,
            related_rule_id: None,
            provenance: policy(),
            action: ArchaeologyLifecycleAction::Candidate,
            created_at: CREATED,
        },
    )
    .expect("successor candidate");
    assert_eq!(
        successor.effective_lifecycle,
        ArchaeologyRuleLifecycle::Candidate
    );
    assert_ne!(
        successor.effective_lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );
}

#[test]
fn stored_review_projection_rejects_non_digest_snapshot_identities() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    let provenance = encode_json(
        "reviewer provenance",
        &StoredReviewProvenance {
            reviewer: policy(),
            rule_kind_identity: "not-a-digest".into(),
        },
    )
    .unwrap();
    transaction
        .execute(
            "INSERT INTO archaeology_rule_review_events
             (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,body,
              evidence_identity,created_at,event_schema_version,event_stream_identity,
              logical_sequence,stable_rule_identity,contradiction_identity,
              description_identity,continuity_identity,parser_identity,prior_event_id,
              actor_kind,reviewer_provenance_json,legacy_stale)
             VALUES (?1,?2,'rule:current',?3,'candidate','codevetter:local',NULL,
                     ?4,?5,2,?6,1,?7,?8,?9,?10,?11,NULL,
                     'deterministic_policy',?12,0)",
            params![
                hash("tampered-review-event"),
                fixture.repository,
                fixture.generation,
                hash("evidence-rule"),
                CREATED,
                lifecycle_stream_identity(&fixture.repository, &fixture.rule),
                fixture.rule,
                hash("contradiction-none"),
                hash("description-original"),
                hash("continuity-rule"),
                hash("parser:fixture:v1"),
                provenance,
            ],
        )
        .expect("schema intentionally permits legacy-shaped opaque fields");

    let error = project_current_lifecycle(
        &transaction,
        &fixture.repository,
        &fixture.generation,
        "rule:current",
        &fixture.rule,
    )
    .unwrap_err();
    assert!(
        error.contains("rule kind must be an opaque SHA-256 identity"),
        "{error}"
    );
}

#[test]
fn lifecycle_tables_are_append_only_survive_generation_cleanup_and_cascade_with_repository() {
    let fixture = Fixture::new();
    let transaction =
        rusqlite::Transaction::new_unchecked(&fixture.connection, TransactionBehavior::Immediate)
            .expect("immediate transaction");
    append_candidate(&transaction, &fixture, &hash("cleanup-candidate"));
    append_alias_event(
        &transaction,
        ArchaeologyAliasAppend {
            event_id: &hash("cleanup-alias"),
            repository_id: &fixture.repository,
            generation_id: &fixture.generation,
            alias_rule_id: "rule:alias-one",
            alias_rule_identity: &fixture.alias_one,
            canonical_rule_id: "rule:canonical",
            canonical_rule_identity: &fixture.canonical,
            expected_previous_sequence: 0,
            action: ArchaeologyAliasAction::Linked,
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap();
    append_continuity_edge(
        &transaction,
        ArchaeologyContinuityAppend {
            repository_id: &fixture.repository,
            continuity_identity: &fixture.shared_continuity,
            predecessor_rule_id: "rule:predecessor",
            predecessor_rule_identity: &fixture.predecessor,
            successor_rule_id: "rule:successor",
            successor_rule_identity: &fixture.successor,
            predecessor_generation_id: &fixture.old_generation,
            successor_generation_id: &fixture.generation,
            kind: ArchaeologyContinuityKind::Supersedes,
            evidence_identity: &hash("evidence-successor"),
            provenance: human(),
            created_at: CREATED,
        },
    )
    .unwrap();

    for table in [
        "archaeology_rule_review_events",
        "archaeology_rule_alias_events",
        "archaeology_rule_continuity_edges",
    ] {
        assert!(transaction
            .execute(&format!("UPDATE {table} SET created_at='changed'"), [])
            .is_err());
        assert!(transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .is_err());
    }
    transaction
        .execute("DELETE FROM archaeology_generations", [])
        .expect("generation cleanup");
    for table in [
        "archaeology_rule_review_events",
        "archaeology_rule_alias_events",
        "archaeology_rule_continuity_edges",
    ] {
        assert_eq!(count(&transaction, table), 1, "generation cleanup {table}");
    }
    transaction
        .execute(
            "DELETE FROM archaeology_repositories WHERE repository_id=?1",
            [&fixture.repository],
        )
        .expect("repository cascade");
    for table in [
        "archaeology_rule_review_events",
        "archaeology_rule_alias_events",
        "archaeology_rule_continuity_edges",
    ] {
        assert_eq!(count(&transaction, table), 0, "repository cascade {table}");
    }
}

fn count(transaction: &Transaction<'_>, table: &str) -> i64 {
    transaction
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
