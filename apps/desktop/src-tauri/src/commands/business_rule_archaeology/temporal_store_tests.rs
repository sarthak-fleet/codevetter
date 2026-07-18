use super::{contracts::*, temporal_store::*};
use rusqlite::{params, Connection};

#[derive(Clone)]
struct RuleSeed {
    key: &'static str,
    stable: String,
    continuity: String,
    evidence: String,
    parser: String,
    contradiction: String,
    description: String,
    title: &'static str,
    clause: &'static str,
    content_hash: String,
    parser_version: &'static str,
}

impl RuleSeed {
    fn base(key: &'static str) -> Self {
        Self {
            key,
            stable: hash('a'),
            continuity: hash('b'),
            evidence: hash('c'),
            parser: hash('d'),
            contradiction: hash('e'),
            description: hash('f'),
            title: "Payment threshold",
            clause: "Reject payments above the configured threshold.",
            content_hash: "1".repeat(64),
            parser_version: "1",
        }
    }
}

#[test]
fn temporal_migration_is_additive_idempotent_and_fully_heals_marked_v2() {
    let connection = database();
    crate::db::archaeology_schema::run_migration(&connection).expect("repeat migration");
    let versions: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(versions, 5);
    for table in [
        "archaeology_temporal_generations",
        "archaeology_rule_temporal_snapshots",
        "archaeology_rule_temporal_events",
        "archaeology_rule_alias_events",
        "archaeology_rule_continuity_edges",
    ] {
        assert!(
            object_exists(&connection, "table", table),
            "missing {table}"
        );
    }
}

#[test]
fn baseline_and_noop_are_compact_deterministic_and_retry_safe() {
    let connection = database();
    let repo = seed_repository(&connection, "compact");
    let rule = RuleSeed::base("rule");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        std::slice::from_ref(&rule),
        true,
    );
    let first = project(&connection, &repo, "generation-1", None, complete()).unwrap();
    let retry = project(&connection, &repo, "generation-1", None, complete()).unwrap();
    assert_eq!(first, retry);
    assert_eq!(first.event_count, 0);
    assert_eq!(first.snapshot_count, 0);
    assert_eq!(
        event_kinds(&connection, &first.temporal_generation_identity),
        Vec::<String>::new()
    );
    let first_anchor: (String, Vec<String>) = connection
        .query_row(
            "SELECT coverage_state,coverage_reasons_json
             FROM archaeology_temporal_generations WHERE temporal_generation_identity=?1",
            [&first.temporal_generation_identity],
            |row| {
                let reasons: String = row.get(1)?;
                Ok((row.get(0)?, serde_json::from_str(&reasons).unwrap()))
            },
        )
        .unwrap();
    assert_eq!(first_anchor.0, "partial");
    assert!(first_anchor.1.contains(&"missing_prior_generation".into()));

    seed_generation(
        &connection,
        &repo,
        "generation-2",
        &revision('2'),
        "staging",
        "manifest:same",
        std::slice::from_ref(&rule),
        true,
    );
    let second = project(
        &connection,
        &repo,
        "generation-2",
        Some("generation-1"),
        complete(),
    )
    .unwrap();
    assert_eq!(
        second.coverage_state,
        ArchaeologyTemporalCoverageState::Complete
    );
    assert_eq!(second.event_count, 0);
    assert_eq!(second.snapshot_count, 1);
}

#[test]
fn temporal_projection_excludes_alias_occurrences_but_retains_alias_relation() {
    let connection = database();
    let repo = seed_repository(&connection, "alias-projection");
    let canonical = RuleSeed::base("canonical");
    let mut alias = RuleSeed::base("alias");
    alias.title = "Alias wording";
    alias.clause = "Equivalent alias clause.";
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        &[canonical, alias],
        true,
    );
    connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
             VALUES ('generation-1','relation:alias','rule:alias:1','rule:canonical:0',
                     'aliases','deterministic')",
            [],
        )
        .unwrap();

    let report = project(&connection, &repo, "generation-1", None, complete()).unwrap();
    assert_eq!(report.snapshot_count, 0);
    assert_eq!(report.event_count, 0);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_relations
                 WHERE generation_id='generation-1' AND kind='aliases'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn prose_only_change_is_observed_without_evidence_change() {
    let (connection, repo, mut current) = two_generation_fixture("prose");
    current.description = hash('4');
    current.title = "Clearer payment threshold";
    current.clause = "Payments above the configured threshold are rejected.";
    seed_current(&connection, &repo, &[current]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(report.event_count, 1);
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["observed"]
    );
    assert_eq!(
        event_coverage(&connection, &report.temporal_generation_identity),
        ("complete".into(), Vec::<String>::new())
    );
}

#[test]
fn exact_evidence_and_contradiction_drift_are_classified_separately() {
    let (connection, repo, mut changed) = two_generation_fixture("evidence");
    changed.evidence = hash('1');
    changed.content_hash = "2".repeat(64);
    seed_current(&connection, &repo, &[changed]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["changed"]
    );

    let (connection, repo, mut conflicted) = two_generation_fixture("contradiction");
    conflicted.contradiction = hash('3');
    seed_current(&connection, &repo, &[conflicted]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["conflicted"]
    );
}

#[test]
fn parser_drift_fails_closed_as_a_partial_observation() {
    let (connection, repo, mut current) = two_generation_fixture("parser");
    current.parser = hash('2');
    current.parser_version = "2";
    seed_current(&connection, &repo, &[current]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["observed"]
    );
    let (state, reasons) = event_coverage(&connection, &report.temporal_generation_identity);
    assert_eq!(state, "partial");
    assert!(reasons.contains(&"parser_incompatible".to_string()));
}

#[test]
fn exact_absence_is_removed_but_partial_absence_is_only_observed() {
    let (connection, repo, _) = two_generation_fixture("remove");
    seed_current(&connection, &repo, &[]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["removed"]
    );

    let (connection, repo, _) = two_generation_fixture("partial-remove");
    seed_current(&connection, &repo, &[]);
    let report = project_current(
        &connection,
        &repo,
        ArchaeologyTemporalCoverageInput {
            state: ArchaeologyTemporalCoverageState::Partial,
            reasons: vec!["shallow_history".into(), "absence_not_proven".into()],
        },
    )
    .unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["observed"]
    );
    let (_, reasons) = event_coverage(&connection, &report.temporal_generation_identity);
    assert!(reasons.contains(&"shallow_history".to_string()));
    assert!(reasons.contains(&"absence_not_proven".to_string()));
    assert_eq!(
        reasons
            .iter()
            .filter(|reason| reason.as_str() == "absence_not_proven")
            .count(),
        1
    );
}

#[test]
fn indexed_clause_lookup_preserves_large_rule_snapshots() {
    let connection = database();
    let repo = seed_repository(&connection, "clause-index");
    let rule = RuleSeed::base("scale");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        std::slice::from_ref(&rule),
        true,
    );
    for ordinal in 1..128 {
        let clause = format!("clause:scale:{ordinal}");
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES ('generation-1','rule:scale:0',?1,?2,?3,
                         'deterministic','high','[]')",
                params![clause, ordinal, format!("Scale clause {ordinal}")],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation-1','rule_clause',?1,'fact','fact:scale:0','supporting')",
                [clause],
            )
            .unwrap();
    }

    project(&connection, &repo, "generation-1", None, complete()).unwrap();
    seed_generation(
        &connection,
        &repo,
        "generation-2",
        &revision('2'),
        "staging",
        "manifest:same",
        &[rule],
        true,
    );
    let report = project_current(&connection, &repo, complete()).unwrap();
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM archaeology_rule_temporal_snapshots
             WHERE repository_id=?1 ORDER BY LENGTH(payload_json) DESC LIMIT 1",
            [&repo],
            |row| row.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let clauses = payload["clauses"].as_array().unwrap();
    assert_eq!(clauses.len(), 128);
    assert!(clauses.iter().all(|clause| clause["evidence"]
        .as_array()
        .is_some_and(|evidence| evidence.len() == 1)));
    assert_eq!(report.snapshot_count, 2);
}

#[test]
fn exact_empty_baseline_allows_introduction_while_missing_baseline_does_not() {
    let connection = database();
    let repo = seed_repository(&connection, "introduce");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        &[],
        true,
    );
    project(&connection, &repo, "generation-1", None, complete()).unwrap();
    seed_current(&connection, &repo, &[RuleSeed::base("rule")]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["introduced"]
    );

    let connection = database();
    let repo = seed_repository(&connection, "no-baseline");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        &[RuleSeed::base("rule")],
        true,
    );
    let report = project(&connection, &repo, "generation-1", None, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        Vec::<String>::new()
    );
    assert_eq!(report.snapshot_count, 0);
    assert!(report
        .coverage_reasons
        .contains(&"missing_prior_generation".into()));
}

#[test]
fn cleaned_prior_catalog_stays_partial_without_inferred_events() {
    let connection = database();
    let repo = seed_repository(&connection, "cleaned-prior");
    let prior = RuleSeed::base("rule");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        std::slice::from_ref(&prior),
        true,
    );
    project(&connection, &repo, "generation-1", None, complete()).unwrap();
    let mut current = prior;
    current.evidence = hash('1');
    current.content_hash = "2".repeat(64);
    seed_current(&connection, &repo, &[current]);
    connection
        .execute(
            "DELETE FROM archaeology_generations WHERE generation_id='generation-1'",
            [],
        )
        .unwrap();

    let first = project_current(&connection, &repo, complete()).unwrap();
    let retry = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(first, retry);
    assert_eq!(
        first.coverage_state,
        ArchaeologyTemporalCoverageState::Partial
    );
    assert!(first
        .coverage_reasons
        .contains(&"missing_prior_catalog".into()));
    assert_eq!(first.snapshot_count, 0);
    assert_eq!(first.event_count, 0);
}

#[test]
fn explicit_continuity_is_the_only_semantic_supersession_link() {
    let (connection, repo, _) = two_generation_fixture("supersede");
    let mut successor = RuleSeed::base("successor");
    successor.stable = hash('5');
    successor.continuity = hash('6');
    successor.evidence = hash('7');
    successor.description = hash('8');
    successor.content_hash = "3".repeat(64);
    seed_current(&connection, &repo, &[successor.clone()]);
    connection
        .execute(
            "INSERT INTO archaeology_rule_continuity_edges
             (edge_identity,repository_id,continuity_identity,predecessor_rule_identity,
              successor_rule_identity,predecessor_generation_id,successor_generation_id,
              kind,evidence_identity,provenance_json,created_at)
             VALUES (?1,?2,?3,?4,?5,'generation-1','generation-2','supersedes',?6,'{}','now')",
            params![
                hash('9'),
                repo,
                hash('b'),
                hash('a'),
                successor.stable,
                successor.evidence
            ],
        )
        .unwrap();
    let report = project_current(&connection, &repo, complete()).unwrap();
    assert_eq!(
        event_kinds(&connection, &report.temporal_generation_identity),
        ["superseded"]
    );
}

#[test]
fn compact_snapshots_survive_generation_cleanup_without_paths_or_source_bodies() {
    let (connection, repo, mut current) = two_generation_fixture("cleanup");
    current.evidence = hash('1');
    current.content_hash = "4".repeat(64);
    seed_current(&connection, &repo, &[current]);
    let report = project_current(&connection, &repo, complete()).unwrap();
    connection
        .execute(
            "DELETE FROM archaeology_generations WHERE repository_id=?1",
            [&repo],
        )
        .unwrap();
    assert_eq!(count(&connection, "archaeology_generations"), 0);
    assert_eq!(count(&connection, "archaeology_temporal_generations"), 2);
    assert_eq!(count(&connection, "archaeology_rule_temporal_events"), 1);
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM archaeology_rule_temporal_snapshots
             WHERE snapshot_identity=(SELECT after_snapshot_identity
               FROM archaeology_rule_temporal_events WHERE temporal_generation_identity=?1)",
            [&report.temporal_generation_identity],
            |row| row.get(0),
        )
        .unwrap();
    assert!(payload.contains("path:rule"));
    assert!(payload.contains(&"4".repeat(64)));
    assert!(!payload.contains("relative_path"));
    assert!(!payload.contains("source_body"));
    assert!(!payload.contains("/Users/"));
}

#[test]
fn hard_bounds_roll_back_without_partial_temporal_rows() {
    let connection = database();
    let repo = seed_repository(&connection, "bounds");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        &[RuleSeed::base("rule")],
        true,
    );
    let transaction = connection.unchecked_transaction().unwrap();
    let error = persist_temporal_projection(
        &transaction,
        ArchaeologyTemporalProjection {
            repository_id: &repo,
            generation_id: "generation-1",
            prior_generation_id: None,
            history_coverage: complete(),
            created_at: "2026-07-17T00:00:00Z",
            limits: ArchaeologyTemporalLimits {
                max_rules: 0,
                ..Default::default()
            },
        },
    )
    .unwrap_err();
    assert_eq!(error, "Archaeology temporal rule bound exceeded");
    drop(transaction);
    assert_eq!(count(&connection, "archaeology_temporal_generations"), 0);

    let transaction = connection.unchecked_transaction().unwrap();
    assert_eq!(
        persist_temporal_projection(
            &transaction,
            ArchaeologyTemporalProjection {
                repository_id: &repo,
                generation_id: "generation-1",
                prior_generation_id: None,
                history_coverage: complete(),
                created_at: "2026-07-17T00:00:00Z",
                limits: ArchaeologyTemporalLimits {
                    max_snapshot_bytes: 8,
                    ..Default::default()
                },
            },
        )
        .unwrap_err(),
        "Archaeology temporal snapshot byte bound exceeded"
    );
}

fn two_generation_fixture(name: &str) -> (Connection, String, RuleSeed) {
    let connection = database();
    let repo = seed_repository(&connection, name);
    let rule = RuleSeed::base("rule");
    seed_generation(
        &connection,
        &repo,
        "generation-1",
        &revision('1'),
        "ready",
        "manifest:same",
        std::slice::from_ref(&rule),
        true,
    );
    project(&connection, &repo, "generation-1", None, complete()).unwrap();
    (connection, repo, rule)
}

fn seed_current(connection: &Connection, repo: &str, rules: &[RuleSeed]) {
    seed_generation(
        connection,
        repo,
        "generation-2",
        &revision('2'),
        "staging",
        "manifest:same",
        rules,
        true,
    );
}

fn project_current(
    connection: &Connection,
    repo: &str,
    history: ArchaeologyTemporalCoverageInput,
) -> Result<ArchaeologyTemporalProjectionReport, String> {
    project(
        connection,
        repo,
        "generation-2",
        Some("generation-1"),
        history,
    )
}

fn project(
    connection: &Connection,
    repo: &str,
    generation: &str,
    prior: Option<&str>,
    history: ArchaeologyTemporalCoverageInput,
) -> Result<ArchaeologyTemporalProjectionReport, String> {
    let transaction = connection.unchecked_transaction().unwrap();
    let report = persist_temporal_projection(
        &transaction,
        ArchaeologyTemporalProjection {
            repository_id: repo,
            generation_id: generation,
            prior_generation_id: prior,
            history_coverage: history,
            created_at: "2026-07-17T00:00:00Z",
            limits: ArchaeologyTemporalLimits::default(),
        },
    )?;
    transaction.commit().unwrap();
    Ok(report)
}

fn seed_repository(connection: &Connection, name: &str) -> String {
    let repo = format!("repo:{name}");
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES (?1,?2,'source',?3,'now','now')",
            params![repo, format!("/fixture/{name}"), revision('2')],
        )
        .unwrap();
    repo
}

#[allow(clippy::too_many_arguments)]
fn seed_generation(
    connection: &Connection,
    repo: &str,
    generation: &str,
    revision_sha: &str,
    status: &str,
    parser_manifest: &str,
    rules: &[RuleSeed],
    complete_coverage: bool,
) {
    let coverage = coverage(complete_coverage);
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
             VALUES (?1,?2,2,?3,'source',?4,'algorithm','config',?5,?6,'now')",
            params![
                generation,
                repo,
                revision_sha,
                parser_manifest,
                status,
                coverage
            ],
        )
        .unwrap();
    for (ordinal, rule) in rules.iter().enumerate() {
        let unit = format!("unit:{}:{ordinal}", rule.key);
        let path = format!("path:{}", rule.key);
        let span = format!("span:{}:{ordinal}", rule.key);
        let fact = format!("fact:{}:{ordinal}", rule.key);
        let rule_id = format!("rule:{}:{ordinal}", rule.key);
        let clause = format!("clause:{}:{ordinal}", rule.key);
        connection
            .execute(
                "INSERT INTO archaeology_source_units
                 (generation_id,source_unit_id,path_identity,content_hash,hash_algorithm,
                  language,parser_id,parser_version,classification,byte_count,line_count)
                 VALUES (?1,?2,?3,?4,'sha256','cobol','parser',?5,'source',80,4)",
                params![
                    generation,
                    unit,
                    path,
                    rule.content_hash,
                    rule.parser_version
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_source_spans
                 (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                  start_line,start_column,end_line,end_column)
                 VALUES (?1,?2,?3,?4,0,40,1,1,2,1)",
                params![generation, span, unit, revision_sha],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES (?1,?2,'predicate','threshold','parser','extracted','high','[]')",
                params![generation, fact],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                  identity_schema_version,stable_rule_identity,evidence_identity,
                  contradiction_identity,description_identity,continuity_identity,
                  parser_compatibility_identity,identity_provenance_json)
                 VALUES (?1,?2,?3,?4,'validation',?5,'candidate','deterministic','high',
                         'parser','algorithm','{}','now',2,?6,?7,?8,?9,?10,?11,'{}')",
                params![
                    generation,
                    rule_id,
                    repo,
                    revision_sha,
                    rule.title,
                    rule.stable,
                    rule.evidence,
                    rule.contradiction,
                    rule.description,
                    rule.continuity,
                    rule.parser,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES (?1,?2,?3,0,?4,'deterministic','high','[]')",
                params![generation, rule_id, clause, rule.clause],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'fact',?2,'span',?3,'supporting'),
                        (?1,'rule_clause',?4,'fact',?2,'supporting')",
                params![generation, fact, span, clause],
            )
            .unwrap();
    }
}

fn coverage(complete: bool) -> String {
    let state = if complete {
        ArchaeologyCoverageState::Complete
    } else {
        ArchaeologyCoverageState::Partial
    };
    serde_json::to_string(&ArchaeologyCoverage {
        state: state.clone(),
        parser_coverage: state.clone(),
        repository_coverage: state,
        temporal_coverage: ArchaeologyCoverageState::Unavailable,
        discovered_source_units: 0,
        indexed_source_units: 0,
        discovered_bytes: 0,
        indexed_bytes: 0,
        reasons: if complete {
            Vec::new()
        } else {
            vec!["fixture_partial".into()]
        },
    })
    .unwrap()
}

fn complete() -> ArchaeologyTemporalCoverageInput {
    ArchaeologyTemporalCoverageInput::complete()
}

fn event_kinds(connection: &Connection, temporal_generation: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT event_kind FROM archaeology_rule_temporal_events
             WHERE temporal_generation_identity=?1 ORDER BY stable_rule_identity,event_kind",
        )
        .unwrap();
    statement
        .query_map([temporal_generation], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn event_coverage(connection: &Connection, temporal_generation: &str) -> (String, Vec<String>) {
    connection
        .query_row(
            "SELECT coverage_state,coverage_reasons_json
             FROM archaeology_rule_temporal_events WHERE temporal_generation_identity=?1",
            [temporal_generation],
            |row| {
                let json: String = row.get(1)?;
                Ok((row.get(0)?, serde_json::from_str(&json).unwrap()))
            },
        )
        .unwrap()
}

fn object_exists(connection: &Connection, kind: &str, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
            params![kind, name],
            |row| row.get(0),
        )
        .unwrap()
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    crate::db::schema::run_migrations(&connection).unwrap();
    connection
}

fn revision(value: char) -> String {
    value.to_string().repeat(40)
}

fn hash(value: char) -> String {
    format!("sha256:{}", value.to_string().repeat(64))
}
