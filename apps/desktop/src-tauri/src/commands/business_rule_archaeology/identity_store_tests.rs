use super::identity_store::{refresh_rule_identities, validate_rule_identities};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use crate::db::archaeology_schema::run_migration;
use rusqlite::{params, Connection, Transaction, TransactionBehavior};

const CREATED_AT: &str = "2026-07-17T00:00:00Z";
const CONTENT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEMANTIC_EXPRESSION: &str =
    "v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn fixture() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .expect("foreign keys");
    run_migration(&connection).expect("real migrated schema");
    connection
}

fn seed_rule(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    rule_id: &str,
    unit_parser_id: &str,
    fact_parser_id: &str,
    hash_algorithm: &str,
) {
    let repo_path = format!("/fixture/{repository_id}");
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES (?1,?2,'source:fixture','revision:fixture',?3,?3)",
            params![repository_id, repo_path, CREATED_AT],
        )
        .expect("repository");
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,created_at)
             VALUES (?1,?2,2,'revision:fixture','source:fixture','parser-set:fixture',
                     'algorithm:fixture','config:fixture','staging',?3)",
            params![generation_id, repository_id, CREATED_AT],
        )
        .expect("generation");
    connection
        .execute(
            "INSERT INTO archaeology_source_units
             (generation_id,source_unit_id,path_identity,relative_path,content_hash,
              hash_algorithm,language,parser_id,parser_version,classification,byte_count,line_count)
             VALUES (?1,'unit:fixture','path:fixture','fixture.cbl',?2,?3,'cobol',?4,
                     '1.0.0','source',32,1)",
            params![generation_id, CONTENT_HASH, hash_algorithm, unit_parser_id],
        )
        .expect("source unit");
    connection
        .execute(
            "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             VALUES (?1,'span:fixture','unit:fixture','revision:fixture',0,16,1,1,1,17)",
            [generation_id],
        )
        .expect("source span");
    let attributes = serde_json::json!([{
        "key": "semantic_expr",
        "value": SEMANTIC_EXPRESSION,
    }])
    .to_string();
    connection
        .execute(
            "INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
             VALUES (?1,'fact:fixture','predicate','fixture predicate',?2,
                     'deterministic','high',?3)",
            params![generation_id, fact_parser_id, attributes],
        )
        .expect("fact");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'fact','fact:fixture','span','span:fixture','supporting')",
            [generation_id],
        )
        .expect("fact evidence");
    connection
        .execute(
            "INSERT INTO archaeology_rules
             (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
              confidence,parser_identity,algorithm_identity,coverage_json,created_at)
             VALUES (?1,?2,?3,'revision:fixture','eligibility','Fixture rule','candidate',
                     'deterministic','high','parser-set:fixture','algorithm:fixture','{}',?4)",
            params![generation_id, rule_id, repository_id, CREATED_AT],
        )
        .expect("rule");
    connection
        .execute(
            "INSERT INTO archaeology_rule_clauses
             (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
             VALUES (?1,?2,'clause:fixture',0,'The fixture predicate must hold.',
                     'deterministic','high','[]')",
            params![generation_id, rule_id],
        )
        .expect("clause");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'rule_clause','clause:fixture','fact','fact:fixture','supporting')",
            [generation_id],
        )
        .expect("rule evidence");
}

fn seed_additional_rule(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    rule_id: &str,
) {
    connection
        .execute(
            "INSERT INTO archaeology_rules
             (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
              confidence,parser_identity,algorithm_identity,coverage_json,created_at)
             VALUES (?1,?2,?3,'revision:fixture','eligibility','Additional fixture rule',
                     'candidate','deterministic','high','parser-set:fixture',
                     'algorithm:fixture','{}',?4)",
            params![generation_id, rule_id, repository_id, CREATED_AT],
        )
        .expect("additional rule");
    connection
        .execute(
            "INSERT INTO archaeology_rule_clauses
             (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
             VALUES (?1,?2,'clause:additional',0,'The additional predicate must hold.',
                     'deterministic','high','[]')",
            params![generation_id, rule_id],
        )
        .expect("additional clause");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'rule_clause','clause:additional','fact','fact:fixture','supporting')",
            [generation_id],
        )
        .expect("additional rule evidence");
}

fn refresh(
    connection: &Connection,
    generation_id: &str,
    rule_ids: &[String],
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .expect("identity transaction");
    let result = refresh_rule_identities(&transaction, generation_id, rule_ids, cancellation);
    if result.is_ok() {
        transaction.commit().expect("commit identity projection");
    }
    result
}

fn validate(
    connection: &Connection,
    generation_id: &str,
    rule_ids: &[String],
) -> Result<usize, String> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Deferred)
        .expect("validation transaction");
    validate_rule_identities(
        &transaction,
        generation_id,
        rule_ids,
        &StructuralGraphCancellation::default(),
    )
}

fn parser_compatibility(connection: &Connection, generation_id: &str, rule_id: &str) -> String {
    connection
        .query_row(
            "SELECT parser_compatibility_identity FROM archaeology_rules
             WHERE generation_id=?1 AND rule_id=?2",
            params![generation_id, rule_id],
            |row| row.get(0),
        )
        .expect("parser compatibility identity")
}

#[test]
fn parser_compatibility_is_repository_scoped() {
    let connection = fixture();
    seed_rule(
        &connection,
        "repository:one",
        "generation:one",
        "rule:one",
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );
    seed_rule(
        &connection,
        "repository:two",
        "generation:two",
        "rule:two",
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );

    refresh(
        &connection,
        "generation:one",
        &["rule:one".into()],
        &StructuralGraphCancellation::default(),
    )
    .expect("first identity");
    refresh(
        &connection,
        "generation:two",
        &["rule:two".into()],
        &StructuralGraphCancellation::default(),
    )
    .expect("second identity");

    assert_ne!(
        parser_compatibility(&connection, "generation:one", "rule:one"),
        parser_compatibility(&connection, "generation:two", "rule:two")
    );
}

#[test]
fn one_batch_refreshes_and_validates_multiple_rules() {
    let connection = fixture();
    let repository = "repository:batch";
    let generation = "generation:batch";
    seed_rule(
        &connection,
        repository,
        generation,
        "rule:one",
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );
    seed_additional_rule(&connection, repository, generation, "rule:two");
    let selected = ["rule:one".to_string(), "rule:two".to_string()];

    assert_eq!(
        refresh(
            &connection,
            generation,
            &selected,
            &StructuralGraphCancellation::default(),
        )
        .expect("batch identity projection"),
        2
    );
    assert_eq!(
        validate(&connection, generation, &selected).expect("batch identity validation"),
        2
    );
    let projected: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rules
             WHERE generation_id=?1 AND identity_schema_version=2",
            [generation],
            |row| row.get(0),
        )
        .expect("projected rules");
    assert_eq!(projected, 2);
}

#[test]
fn fact_and_unit_parser_mismatch_fails_closed() {
    let connection = fixture();
    seed_rule(
        &connection,
        "repository:mismatch",
        "generation:mismatch",
        "rule:mismatch",
        "parser:unit",
        "parser:fact",
        "sha256",
    );

    let error = refresh(
        &connection,
        "generation:mismatch",
        &["rule:mismatch".into()],
        &StructuralGraphCancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("fact is unavailable"), "{error}");
}

#[test]
fn non_sha256_source_hash_fails_closed() {
    let connection = fixture();
    seed_rule(
        &connection,
        "repository:hash",
        "generation:hash",
        "rule:hash",
        "parser:fixture",
        "parser:fixture",
        "sha1",
    );

    let error = refresh(
        &connection,
        "generation:hash",
        &["rule:hash".into()],
        &StructuralGraphCancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("fact is unavailable"), "{error}");
}

#[test]
fn duplicate_requested_rule_ids_are_rejected_before_projection() {
    let connection = fixture();
    seed_rule(
        &connection,
        "repository:duplicate",
        "generation:duplicate",
        "rule:duplicate",
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );

    let error = refresh(
        &connection,
        "generation:duplicate",
        &["rule:duplicate".into(), "rule:duplicate".into()],
        &StructuralGraphCancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("selection is invalid"), "{error}");
}

#[test]
fn cancellation_stops_before_identity_projection() {
    let connection = fixture();
    seed_rule(
        &connection,
        "repository:cancel",
        "generation:cancel",
        "rule:cancel",
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );
    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel_after_checks(2);

    let error = refresh(
        &connection,
        "generation:cancel",
        &["rule:cancel".into()],
        &cancellation,
    )
    .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(cancellation.check_count() >= 2);
    let identity_version: Option<i64> = connection
        .query_row(
            "SELECT identity_schema_version FROM archaeology_rules
             WHERE generation_id='generation:cancel' AND rule_id='rule:cancel'",
            [],
            |row| row.get(0),
        )
        .expect("identity version");
    assert_eq!(identity_version, None);
}

#[test]
fn validation_rejects_stored_identity_and_provenance_tampering() {
    let connection = fixture();
    let generation = "generation:tamper";
    let rule = "rule:tamper";
    let selected = [rule.to_string()];
    seed_rule(
        &connection,
        "repository:tamper",
        generation,
        rule,
        "parser:fixture",
        "parser:fixture",
        "sha256",
    );
    refresh(
        &connection,
        generation,
        &selected,
        &StructuralGraphCancellation::default(),
    )
    .expect("identity projection");
    validate(&connection, generation, &selected).expect("valid projection");

    connection
        .execute(
            "UPDATE archaeology_rules SET description_identity=?3
             WHERE generation_id=?1 AND rule_id=?2",
            params![
                generation,
                rule,
                "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            ],
        )
        .expect("tamper identity");
    let error = validate(&connection, generation, &selected).unwrap_err();
    assert!(error.contains("does not reconcile"), "{error}");

    refresh(
        &connection,
        generation,
        &selected,
        &StructuralGraphCancellation::default(),
    )
    .expect("repair projection");
    connection
        .execute(
            "UPDATE archaeology_rules SET identity_provenance_json='{}'
             WHERE generation_id=?1 AND rule_id=?2",
            params![generation, rule],
        )
        .expect("tamper provenance");
    let error = validate(&connection, generation, &selected).unwrap_err();
    assert!(error.contains("does not reconcile"), "{error}");
}
