use super::adapter::{ArchaeologyAdapterLineage, ArchaeologyLineageKind};
use super::contracts::{
    ArchaeologyJobStage, ArchaeologySourceClassification, ArchaeologySourceUnitIdentity,
};
use super::invalidation::{
    ArchaeologyGenerationInput, ArchaeologyGenerationInputKind as InputKind,
    ArchaeologyInputDecision, ArchaeologyInputInvalidationMode as Mode,
    ArchaeologyInvalidationLimits, ArchaeologySourceDependencyKind as DependencyKind,
};
use super::invalidation_store::{
    changed_source_paths, execute_refresh_work_batch, load_generation_inputs,
    load_source_dependencies, persist_generation_invalidation_metadata, persist_refresh_work_plan,
    plan_generation_invalidation, ArchaeologyInvalidationPlan, ArchaeologyRefreshWorkItem,
};
use super::inventory::{
    inventory_repository_streaming, ArchaeologyInventoryLimits, ArchaeologyInventoryUnit,
};
use super::jobs::{
    execute_incremental_parse_batch, prepare_incremental_refresh, ArchaeologyGenerationIdentity,
    ArchaeologyInventoryRefreshStage,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use crate::db::archaeology_schema;
use rusqlite::{params, Connection, Transaction};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

const CREATED_AT: &str = "2026-07-17T00:00:00Z";

#[test]
fn metadata_round_trips_exact_inputs_and_only_provable_lineage() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    seed_unit(
        &connection,
        "generation:ready",
        "unit:copy",
        "path:copy",
        &[],
    );
    let mut exact_lineage = resolved_lineage(
        ArchaeologyLineageKind::Copybook,
        "unit:program",
        "unit:copy",
    );
    exact_lineage.detail = "x".repeat(2_048);
    seed_unit(
        &connection,
        "generation:ready",
        "unit:program",
        "path:program",
        &[exact_lineage],
    );
    let baseline_inputs = inputs('a');

    let persisted = persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &baseline_inputs,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("persist metadata");
    assert_eq!(persisted.input_count, baseline_inputs.len());
    assert_eq!(persisted.dependency_count, 1);
    assert!(!persisted.unresolved_lineage);
    let loaded_inputs =
        load_generation_inputs(&connection, "repo:a", "generation:ready").expect("load inputs");
    assert_eq!(loaded_inputs.len(), baseline_inputs.len());
    for expected in baseline_inputs {
        assert!(loaded_inputs.contains(&expected));
    }
    assert_eq!(
        load_source_dependencies(&connection, "repo:a", "generation:ready")
            .expect("load dependencies"),
        [super::invalidation::ArchaeologySourceDependency {
            dependent_path_identity: "path:program".into(),
            prerequisite_path_identity: "path:copy".into(),
            kind: DependencyKind::Copybook,
        }]
    );
    let evidence: String = connection
        .query_row(
            "SELECT evidence_identity FROM archaeology_source_dependencies",
            [],
            |row| row.get(0),
        )
        .expect("evidence identity");
    assert_eq!(evidence.len(), 71);
    assert!(evidence.starts_with("sha256:"));

    let prior_dependencies = load_source_dependencies(&connection, "repo:a", "generation:ready")
        .expect("prior dependencies");
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &[],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .unwrap_err()
    .contains("incomplete"));
    let mut mismatched = inputs('a');
    mismatched
        .iter_mut()
        .find(|input| input.kind == InputKind::Config)
        .expect("config input")
        .identity = "config:wrong".into();
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &mismatched,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .unwrap_err()
    .contains("reconcile"));
    let tight = ArchaeologyInvalidationLimits {
        max_invalidated_paths: 1,
        ..Default::default()
    };
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        tight,
    )
    .unwrap_err()
    .contains("source-unit bound"));
    let byte_tight = ArchaeologyInvalidationLimits {
        max_input_bytes: 512,
        ..Default::default()
    };
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        byte_tight,
    )
    .unwrap_err()
    .contains("source-lineage byte bound"));
    assert_eq!(
        load_source_dependencies(&connection, "repo:a", "generation:ready")
            .expect("rolled-back dependencies"),
        prior_dependencies
    );
    promote_ready(&connection, "repo:a", "generation:ready");
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .unwrap_err()
    .contains("exact staging generation"));
}

#[test]
fn dependency_evidence_identity_is_stable_across_equivalent_rebuilds() {
    let lineage = resolved_lineage(
        ArchaeologyLineageKind::Copybook,
        "unit:program",
        "unit:copy",
    );
    let build = |generation_id: &str| {
        let connection = database();
        seed_repository(&connection, "repo:a");
        seed_generation(&connection, "repo:a", generation_id, "staging", 'a');
        seed_unit(&connection, generation_id, "unit:copy", "path:copy", &[]);
        seed_unit(
            &connection,
            generation_id,
            "unit:program",
            "path:program",
            std::slice::from_ref(&lineage),
        );
        persist_generation_invalidation_metadata(
            &connection,
            "repo:a",
            generation_id,
            &inputs('a'),
            &StructuralGraphCancellation::default(),
            ArchaeologyInvalidationLimits::default(),
        )
        .expect("persist equivalent metadata");
        connection
            .query_row(
                "SELECT evidence_identity FROM archaeology_source_dependencies
                 WHERE generation_id=?1",
                [generation_id],
                |row| row.get::<_, String>(0),
            )
            .expect("dependency evidence identity")
    };

    assert_eq!(build("generation:first"), build("generation:second"));
}

#[test]
fn cross_file_rule_dependency_drives_reverse_invalidation() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    seed_unit(&connection, "generation:ready", "unit:a", "path:a", &[]);
    seed_unit(&connection, "generation:ready", "unit:b", "path:b", &[]);
    for (rule_id, unit_id) in [("rule:a", "unit:a"), ("rule:b", "unit:b")] {
        connection
            .execute(
                "INSERT INTO archaeology_rules
             (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
              confidence,parser_identity,algorithm_identity,coverage_json,created_at)
             VALUES ('generation:ready',?1,'repo:a',?2,'validation',?1,'candidate',
                     'deterministic','high','parser:fixture','algorithm:fixture','{}',?3)",
                params![rule_id, "a".repeat(40), CREATED_AT],
            )
            .expect("rule");
        let clause_id = format!("clause:{rule_id}");
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
             (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence)
             VALUES ('generation:ready',?1,?2,0,?1,'deterministic','high')",
                params![rule_id, clause_id],
            )
            .expect("rule clause");
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES ('generation:ready','rule_clause',?1,'span',?2,'supporting')",
                params![clause_id, format!("span:{unit_id}")],
            )
            .expect("clause evidence");
    }
    connection
        .execute(
            "INSERT INTO archaeology_rule_relations
         (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust)
         VALUES ('generation:ready','relation:a-b','rule:a','rule:b','depends_on','deterministic')",
            [],
        )
        .expect("rule relation");
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready rule dependencies");
    assert!(
        load_source_dependencies(&connection, "repo:a", "generation:ready")
            .expect("typed dependencies")
            .contains(&super::invalidation::ArchaeologySourceDependency {
                dependent_path_identity: "path:a".into(),
                prerequisite_path_identity: "path:b".into(),
                kind: DependencyKind::Rule,
            })
    );
    promote_ready(&connection, "repo:a", "generation:ready");
    seed_generation(&connection, "repo:a", "generation:staging", "staging", 'b');
    seed_unit(
        &connection,
        "generation:staging",
        "unit:a:new",
        "path:a",
        &[],
    );
    seed_unit(
        &connection,
        "generation:staging",
        "unit:b:new",
        "path:b",
        &[],
    );
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:staging",
        &inputs('b'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("staging metadata");
    let plan = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:staging",
        &["path:b".into()],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("reverse rule invalidation");
    assert_eq!(
        plan.invalidated_paths
            .iter()
            .map(|path| (path.path_identity.as_str(), path.depth))
            .collect::<Vec<_>>(),
        [("path:a", 1), ("path:b", 0)]
    );
    assert!(plan.invalidated_paths[0]
        .via
        .contains(&DependencyKind::Rule));
}

#[test]
fn identical_ready_and_staging_inputs_produce_a_true_noop() {
    let connection = seeded_ready_and_staging();
    seed_unit(
        &connection,
        "generation:ready",
        "unit:seed",
        "path:seed",
        &[],
    );
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    let plan = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:ready",
        &[],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("no-op plan");
    assert_eq!(plan.repository_id, "repo:a");
    assert_eq!(plan.generation_id, "generation:ready");
    assert_eq!(
        plan.prior_ready_generation_id.as_deref(),
        Some("generation:ready")
    );
    assert_eq!(plan.decision.mode, Mode::NoOp);
    assert!(plan.invalidated_paths.is_empty());
    assert!(!plan.unresolved_lineage);

    let scoped = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:ready",
        &["path:seed".into()],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("explicit changed seed");
    assert_eq!(scoped.decision.mode, Mode::Scoped);
    assert_eq!(scoped.decision.changed_kinds, [InputKind::Head]);
    assert_eq!(scoped.invalidated_paths[0].path_identity, "path:seed");
    let limits = ArchaeologyInvalidationLimits {
        max_seed_paths: 1,
        ..Default::default()
    };
    assert!(plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:ready",
        &["missing:a".into(), "missing:b".into()],
        &StructuralGraphCancellation::default(),
        limits,
    )
    .unwrap_err()
    .contains("seed bound"));
}

#[test]
fn durable_noop_plan_executes_zero_callbacks() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:staging", "staging", 'a');
    seed_job(
        &connection,
        "job:noop",
        "repo:a",
        "generation:staging",
        "owner:a",
    );
    let plan = ArchaeologyInvalidationPlan {
        repository_id: "repo:a".into(),
        generation_id: "generation:staging".into(),
        prior_ready_generation_id: Some("generation:prior".into()),
        decision: ArchaeologyInputDecision {
            mode: Mode::NoOp,
            changed_kinds: Vec::new(),
            parser_scopes: Vec::new(),
            synthesis_policy_scopes: Vec::new(),
        },
        invalidated_paths: Vec::new(),
        removed_path_identities: Vec::new(),
        unresolved_lineage: false,
    };
    let identity = persist_refresh_work_plan(
        &connection,
        "job:noop",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan,
    )
    .expect("persist no-op");
    let mut callbacks = 0;
    let execution = execute_refresh_work_batch(
        &connection,
        "job:noop",
        "repo:a",
        "generation:staging",
        "owner:a",
        &identity,
        1,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |_, _| {
            callbacks += 1;
            Ok(())
        },
    )
    .expect("execute no-op");
    assert_eq!(
        (callbacks, execution.completed, execution.remaining),
        (0, 0, 0)
    );
}

#[test]
fn job_inventory_transition_skips_parse_for_an_exact_noop() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    seed_unit(
        &connection,
        "generation:ready",
        "unit:stable",
        "path:stable",
        &[],
    );
    seed_fact(
        &connection,
        "generation:ready",
        "fact:stable",
        "unit:stable",
    );
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    seed_generation(&connection, "repo:a", "generation:staging", "staging", 'b');
    seed_job(
        &connection,
        "job:noop-transition",
        "repo:a",
        "generation:staging",
        "owner:a",
    );
    let units = [inventory_unit('b', "unit:stable", "path:stable", 'd')];
    let revision = "b".repeat(40);
    let identity = ArchaeologyGenerationIdentity {
        revision_sha: &revision,
        source: "source:fixture",
        parser: "parser:fixture",
        algorithm: "algorithm:fixture",
        config: "config:fixture",
    };
    let current_inputs = inputs('b');
    let outcome = prepare_incremental_refresh(
        &connection,
        ArchaeologyInventoryRefreshStage {
            job_id: "job:noop-transition",
            repository_id: "repo:a",
            generation_id: "generation:staging",
            owner_id: "owner:a",
            identity,
            units: &units,
            generation_inputs: &current_inputs,
            cancellation: &StructuralGraphCancellation::default(),
            limits: ArchaeologyInvalidationLimits::default(),
            now: CREATED_AT,
        },
    )
    .expect("prepare no-op refresh");
    assert_eq!(outcome.mode, Mode::NoOp);
    assert_eq!(outcome.next_stage, ArchaeologyJobStage::Idle);
    assert_eq!(outcome.effective_generation_id, "generation:ready");
    assert!(outcome.reused_ready_generation);
    assert!(outcome.changed_paths.is_empty());
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_refresh_work_items
                 WHERE job_id='job:noop-transition'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no-op work count"),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_generations
                 WHERE generation_id='generation:staging'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("discarded no-op staging generation"),
        0
    );
}

#[test]
fn real_inventory_revisions_select_only_content_and_protected_changes() {
    let repository = TempDir::new().expect("temporary repository");
    git(repository.path(), &["init", "-q"]);
    git(
        repository.path(),
        &["config", "user.name", "CodeVetter Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "codevetter@example.invalid"],
    );
    fs::create_dir_all(repository.path().join("src")).expect("source directory");
    fs::write(
        repository.path().join("src/stable.cbl"),
        "DISPLAY 'STABLE'.\n",
    )
    .expect("stable source");
    fs::write(
        repository.path().join("src/changed.cbl"),
        "DISPLAY 'VALUE-A'.\n",
    )
    .expect("changed source v1");
    fs::write(repository.path().join(".env"), "SECRET=A\n").expect("protected source v1");
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-qm", "first"]);
    let first = inventory(repository.path());

    fs::write(
        repository.path().join("src/changed.cbl"),
        "DISPLAY 'VALUE-B'.\n",
    )
    .expect("changed source v2");
    fs::write(repository.path().join(".env"), "SECRET=B\n").expect("protected source v2");
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-qm", "second"]);
    let second = inventory(repository.path());

    let stable_first = inventory_unit_by_path(&first, "src/stable.cbl");
    let stable_second = inventory_unit_by_path(&second, "src/stable.cbl");
    assert_ne!(
        stable_first.identity.source_unit_id, stable_second.identity.source_unit_id,
        "inventory source IDs intentionally include the revision"
    );
    assert_eq!(
        stable_first.identity.change_identity, stable_second.identity.change_identity,
        "the revision-neutral change signal must remain stable"
    );
    let protected_first = first
        .iter()
        .find(|unit| unit.classification == ArchaeologySourceClassification::Protected)
        .expect("protected first unit");
    let protected_second = second
        .iter()
        .find(|unit| unit.classification == ArchaeologySourceClassification::Protected)
        .expect("protected second unit");
    assert_eq!(protected_first.byte_count, protected_second.byte_count);
    assert!(protected_first.identity.content_hash.is_none());
    assert!(protected_second.identity.content_hash.is_none());
    assert_ne!(
        protected_first.identity.change_identity, protected_second.identity.change_identity,
        "same-size protected changes need an opaque change signal"
    );

    let repository_id = first[0].identity.repository_id.clone();
    let connection = database();
    seed_repository(&connection, &repository_id);
    seed_inventory_generation(
        &connection,
        &repository_id,
        "generation:ready",
        &first,
        "staging",
    );
    promote_ready(&connection, &repository_id, "generation:ready");
    seed_inventory_generation(
        &connection,
        &repository_id,
        "generation:staging",
        &second,
        "staging",
    );
    let changed = changed_source_paths(
        &connection,
        &repository_id,
        "generation:staging",
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("changed paths");
    assert_eq!(
        changed,
        [
            protected_second.identity.path_identity.clone(),
            inventory_unit_by_path(&second, "src/changed.cbl")
                .identity
                .path_identity
                .clone(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    assert!(!changed.contains(&stable_second.identity.path_identity));
}

#[test]
fn job_changed_unit_refresh_retries_and_reconciles_clean_fact_ownership() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    for (unit, path, lineage) in [
        ("unit:shared", "path:shared", Vec::new()),
        (
            "unit:program",
            "path:program",
            vec![resolved_lineage(
                ArchaeologyLineageKind::Copybook,
                "unit:program",
                "unit:shared",
            )],
        ),
        ("unit:unrelated", "path:unrelated", Vec::new()),
    ] {
        seed_unit(&connection, "generation:ready", unit, path, &lineage);
        seed_fact(
            &connection,
            "generation:ready",
            &format!("fact:{unit}"),
            unit,
        );
    }
    connection
        .execute_batch(
            "INSERT INTO archaeology_facts
               (generation_id,fact_id,kind,label,parser_id,trust,confidence)
             VALUES ('generation:ready','archaeology-link-fact:old','unresolved',
                     'unresolved reference','parser:fixture','deterministic','low');
             INSERT INTO archaeology_evidence_links
               (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES ('generation:ready','fact','archaeology-link-fact:old','span',
                     'span:unit:unrelated','supporting');
             INSERT INTO archaeology_fact_edges
               (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
             VALUES
               ('generation:ready','archaeology-link-edge:old','fact:unit:unrelated',
                'archaeology-link-fact:old','unresolved','deterministic'),
               ('generation:ready','edge:parser-owned','fact:unit:unrelated',
                'fact:unit:unrelated','controls','extracted');
             INSERT INTO archaeology_evidence_links
               (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES
               ('generation:ready','fact_edge','archaeology-link-edge:old','span',
                'span:unit:unrelated','supporting'),
               ('generation:ready','fact_edge','edge:parser-owned','span',
                'span:unit:unrelated','supporting');",
        )
        .expect("ready linker and parser artifacts");
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    seed_generation(&connection, "repo:a", "generation:staging", "staging", 'b');
    seed_job(
        &connection,
        "job:changed-transition",
        "repo:a",
        "generation:staging",
        "owner:a",
    );
    let units = [
        inventory_unit('b', "unit:shared", "path:shared", 'e'),
        inventory_unit('b', "unit:program", "path:program", 'd'),
        inventory_unit('b', "unit:unrelated", "path:unrelated", 'd'),
    ];
    let revision = "b".repeat(40);
    let identity = ArchaeologyGenerationIdentity {
        revision_sha: &revision,
        source: "source:fixture",
        parser: "parser:fixture",
        algorithm: "algorithm:fixture",
        config: "config:fixture",
    };
    let mut current_inputs = inputs('b');
    current_inputs
        .iter_mut()
        .find(|input| input.kind == InputKind::SynthesisPolicy)
        .expect("synthesis policy input")
        .identity = "synthesis:v2".into();
    let outcome = prepare_incremental_refresh(
        &connection,
        ArchaeologyInventoryRefreshStage {
            job_id: "job:changed-transition",
            repository_id: "repo:a",
            generation_id: "generation:staging",
            owner_id: "owner:a",
            identity,
            units: &units,
            generation_inputs: &current_inputs,
            cancellation: &StructuralGraphCancellation::default(),
            limits: ArchaeologyInvalidationLimits::default(),
            now: CREATED_AT,
        },
    )
    .expect("prepare changed refresh");
    assert_eq!(outcome.mode, Mode::Scoped);
    assert_eq!(outcome.next_stage, ArchaeologyJobStage::Parse);
    assert_eq!(outcome.changed_paths, ["path:shared"]);
    assert_eq!(
        fact_owner_paths(&connection, "generation:staging"),
        ["path:unrelated"]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_source_units
                 WHERE generation_id='generation:staging' AND include_lineage_json!='[]'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("cloned lineage count"),
        0,
        "revision-scoped lineage must be rebuilt against current source-unit identities"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_facts
                 WHERE generation_id='generation:staging'
                   AND fact_id LIKE 'archaeology-link-fact:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("cloned linker fact count"),
        0,
        "link-derived facts must be recomputed for the current revision"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_fact_edges
                 WHERE generation_id='generation:staging'
                   AND edge_id LIKE 'archaeology-link-edge:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("cloned linker edge count"),
        0,
        "link-derived edges must be recomputed for the current revision"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_fact_edges
                 WHERE generation_id='generation:staging'
                   AND edge_id='edge:parser-owned'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("cloned parser edge count"),
        1,
        "unchanged parser-owned edges must remain reusable"
    );

    assert!(execute_incremental_parse_batch(
        &connection,
        "job:changed-transition",
        "repo:a",
        "generation:staging",
        "owner:a",
        &outcome.plan_identity,
        1,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |_, _| Err("parser interrupted".into()),
    )
    .unwrap_err()
    .contains("interrupted"));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_refresh_work_items
                 WHERE job_id='job:changed-transition' AND completed=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("unchanged work checkpoint"),
        0
    );
    let execution = execute_incremental_parse_batch(
        &connection,
        "job:changed-transition",
        "repo:a",
        "generation:staging",
        "owner:a",
        &outcome.plan_identity,
        10,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        persist_parsed_fixture_fact,
    )
    .expect("resume changed refresh");
    assert_eq!((execution.completed, execution.remaining), (2, 0));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_refresh_work_items
                 WHERE job_id='job:changed-transition' AND completed=0
                   AND target_kind='synthesis_scope'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("deferred synthesis work"),
        1,
        "parse execution must neither consume nor wait on synthesis work"
    );
    assert_eq!(
        fact_owner_paths(&connection, "generation:staging"),
        ["path:program", "path:shared", "path:unrelated"]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT stage FROM archaeology_jobs WHERE job_id='job:changed-transition'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("post-parse stage"),
        "link"
    );
}

#[test]
fn ready_graph_closes_over_shared_and_transitive_dependencies() {
    let connection = seeded_ready_and_staging();
    seed_unit(
        &connection,
        "generation:ready",
        "unit:shared",
        "path:shared",
        &[],
    );
    seed_unit(
        &connection,
        "generation:ready",
        "unit:a",
        "path:a",
        &[resolved_lineage(
            ArchaeologyLineageKind::Copybook,
            "unit:a",
            "unit:shared",
        )],
    );
    seed_cross_unit_edge(
        &connection,
        "generation:ready",
        "unit:service",
        "unit:a",
        "calls",
    );
    seed_unit(
        &connection,
        "generation:ready",
        "unit:b",
        "path:b",
        &[resolved_lineage(
            ArchaeologyLineageKind::Include,
            "unit:b",
            "unit:shared",
        )],
    );
    seed_unit(
        &connection,
        "generation:ready",
        "unit:service",
        "path:service",
        &[resolved_lineage(
            ArchaeologyLineageKind::Macro,
            "unit:service",
            "unit:a",
        )],
    );
    seed_unit(
        &connection,
        "generation:ready",
        "unit:unrelated",
        "path:unrelated",
        &[],
    );
    seed_unit(
        &connection,
        "generation:staging",
        "unit:shared",
        "path:shared",
        &[],
    );
    for (unit, path) in [
        ("unit:a", "path:a"),
        ("unit:b", "path:b"),
        ("unit:service", "path:service"),
        ("unit:unrelated", "path:unrelated"),
    ] {
        seed_unit(&connection, "generation:staging", unit, path, &[]);
    }
    connection
        .execute(
            "UPDATE archaeology_source_units SET content_hash=?1
             WHERE generation_id='generation:staging' AND path_identity<>'path:unrelated'",
            ["e".repeat(64)],
        )
        .expect("changed staging hashes");
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:staging",
        &inputs('b'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("staging metadata");

    let plan = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:staging",
        &["path:shared".into()],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("scoped plan");
    assert_eq!(plan.decision.mode, Mode::Scoped);
    assert!(
        load_source_dependencies(&connection, "repo:a", "generation:ready")
            .expect("typed dependencies")
            .iter()
            .any(|dependency| dependency.kind == DependencyKind::Call)
    );
    assert!(!plan
        .invalidated_paths
        .iter()
        .any(|path| path.path_identity == "path:unrelated"));
    assert_eq!(
        plan.invalidated_paths
            .iter()
            .map(|item| (item.path_identity.as_str(), item.depth))
            .collect::<Vec<_>>(),
        [
            ("path:a", 1),
            ("path:b", 1),
            ("path:service", 2),
            ("path:shared", 0),
        ]
    );

    seed_job(
        &connection,
        "job:refresh",
        "repo:a",
        "generation:staging",
        "owner:a",
    );
    let plan_identity = persist_refresh_work_plan(
        &connection,
        "job:refresh",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan,
    )
    .expect("persist refresh work");
    assert_eq!(
        persist_refresh_work_plan(
            &connection,
            "job:refresh",
            "repo:a",
            "generation:staging",
            "owner:a",
            &plan,
        )
        .expect("idempotent refresh work"),
        plan_identity
    );
    let mut executed = Vec::new();
    let clean = load_source_hashes(&connection, "generation:staging");
    let mut incremental = load_source_hashes(&connection, "generation:ready");
    let first = execute_refresh_work_batch(
        &connection,
        "job:refresh",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan_identity,
        2,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |transaction, item| {
            executed.push(item.target_identity.clone());
            incremental.insert(
                item.target_identity.clone(),
                source_hash(transaction, "generation:staging", &item.target_identity),
            );
            Ok(())
        },
    )
    .expect("first refresh batch");
    assert_eq!((first.completed, first.remaining), (2, 2));
    let cancelled = StructuralGraphCancellation::default();
    cancelled.cancel();
    assert!(execute_refresh_work_batch(
        &connection,
        "job:refresh",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan_identity,
        10,
        CREATED_AT,
        &cancelled,
        |_, _| Ok(()),
    )
    .unwrap_err()
    .contains("cancelled"));
    let resumed = execute_refresh_work_batch(
        &connection,
        "job:refresh",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan_identity,
        10,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |transaction, item| {
            executed.push(item.target_identity.clone());
            incremental.insert(
                item.target_identity.clone(),
                source_hash(transaction, "generation:staging", &item.target_identity),
            );
            Ok(())
        },
    )
    .expect("resumed refresh batch");
    assert_eq!((resumed.completed, resumed.remaining), (2, 0));
    assert_eq!(
        executed,
        plan.invalidated_paths
            .iter()
            .map(|path| path.path_identity.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(incremental, clean);
}

#[test]
fn changed_seed_overrides_synthesis_only_planning() {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    seed_unit(
        &connection,
        "generation:ready",
        "unit:seed",
        "path:seed",
        &[],
    );
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    seed_generation_with_source(
        &connection,
        "repo:a",
        "generation:staging",
        "staging",
        'a',
        "source:changed",
    );
    seed_unit(
        &connection,
        "generation:staging",
        "unit:seed",
        "path:seed",
        &[],
    );
    let mut current = inputs('a');
    current
        .iter_mut()
        .find(|input| input.kind == InputKind::SynthesisPolicy)
        .expect("synthesis input")
        .identity = "synthesis:v2".into();
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:staging",
        &current,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("staging metadata");

    let plan = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:staging",
        &["path:seed".into()],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("mixed source and synthesis plan");
    assert_eq!(plan.decision.mode, Mode::Scoped);
    assert_eq!(
        plan.decision.changed_kinds,
        [InputKind::Head, InputKind::SynthesisPolicy]
    );
    assert_eq!(plan.decision.synthesis_policy_scopes, ["global"]);
    assert_eq!(plan.invalidated_paths[0].path_identity, "path:seed");
}

#[test]
fn cancellation_rolls_back_and_cross_repository_scope_is_rejected() {
    let connection = seeded_ready_and_staging();
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("baseline metadata");
    let cancellation = StructuralGraphCancellation::default();
    // Cancel after the replacement transaction has begun and at least one
    // input insert has run, proving that the clear/replace operation rolls
    // back instead of exposing partial metadata.
    cancellation.cancel_after_checks(3);
    assert!(persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &cancellation,
        ArchaeologyInvalidationLimits::default(),
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(cancellation.check_count() >= 3);
    assert!(
        load_generation_inputs(&connection, "repo:a", "generation:ready")
            .expect("unchanged inputs")
            .iter()
            .any(|input| input.kind == InputKind::Head && input.identity == "a".repeat(40))
    );

    seed_repository(&connection, "repo:b");
    seed_generation(&connection, "repo:b", "generation:b-ready", "staging", 'c');
    let error = persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:b-ready",
        &inputs('c'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect_err("cross-scope generation");
    assert!(error.contains("outside repository scope"), "{error}");
}

#[test]
fn unresolved_include_lineage_forces_a_global_rebuild() {
    let connection = seeded_ready_and_staging();
    seed_unit(
        &connection,
        "generation:ready",
        "unit:copy",
        "path:copy",
        &[],
    );
    seed_unit(
        &connection,
        "generation:staging",
        "unit:copy",
        "path:copy",
        &[],
    );
    seed_unit(
        &connection,
        "generation:staging",
        "unit:program",
        "path:program",
        &[unresolved_lineage("unit:program")],
    );
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    let persisted = persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:staging",
        &inputs('b'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("unresolved metadata");
    assert!(persisted.unresolved_lineage);
    assert_eq!(persisted.dependency_count, 0);

    let plan = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:staging",
        &[],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("global plan");
    assert!(plan.unresolved_lineage);
    assert_eq!(plan.decision.mode, Mode::GlobalRebuild);
    assert_eq!(plan.invalidated_paths.len(), 2);
    seed_job(
        &connection,
        "job:global",
        "repo:a",
        "generation:staging",
        "owner:a",
    );
    let identity = persist_refresh_work_plan(
        &connection,
        "job:global",
        "repo:a",
        "generation:staging",
        "owner:a",
        &plan,
    )
    .expect("persist global rebuild");
    assert!(execute_refresh_work_batch(
        &connection,
        "job:global",
        "repo:a",
        "generation:staging",
        "owner:a",
        &identity,
        1,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |_, item| {
            assert_eq!(item.action, "reprocess");
            Err("interrupted global rebuild".into())
        },
    )
    .unwrap_err()
    .contains("interrupted"));
    let resumed = execute_refresh_work_batch(
        &connection,
        "job:global",
        "repo:a",
        "generation:staging",
        "owner:a",
        &identity,
        10,
        CREATED_AT,
        &StructuralGraphCancellation::default(),
        |_, item| {
            assert_eq!(item.target_kind, "source_path");
            Ok(())
        },
    )
    .expect("resume global rebuild");
    assert_eq!((resumed.completed, resumed.remaining), (2, 0));
}

#[test]
fn planning_never_changes_the_ready_pointer() {
    let connection = seeded_ready_and_staging();
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:ready",
        &inputs('a'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("ready metadata");
    promote_ready(&connection, "repo:a", "generation:ready");
    persist_generation_invalidation_metadata(
        &connection,
        "repo:a",
        "generation:staging",
        &inputs('b'),
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("staging metadata");
    let _ = plan_generation_invalidation(
        &connection,
        "repo:a",
        "generation:staging",
        &[],
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("plan");
    let ready: Option<String> = connection
        .query_row(
            "SELECT ready_generation_id FROM archaeology_repositories WHERE repository_id='repo:a'",
            [],
            |row| row.get(0),
        )
        .expect("ready pointer");
    assert_eq!(ready.as_deref(), Some("generation:ready"));
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM archaeology_generations WHERE generation_id='generation:ready'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("ready status"),
        "ready"
    );
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys=ON")
        .expect("foreign keys");
    archaeology_schema::run_migration(&connection).expect("real migration");
    connection
}

fn seeded_ready_and_staging() -> Connection {
    let connection = database();
    seed_repository(&connection, "repo:a");
    seed_generation(&connection, "repo:a", "generation:ready", "staging", 'a');
    seed_generation(&connection, "repo:a", "generation:staging", "staging", 'b');
    connection
}

fn seed_repository(connection: &Connection, repository_id: &str) {
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
              created_at,updated_at)
             VALUES (?1,?2,'source:fixture',?3,NULL,?4,?4)",
            params![
                repository_id,
                format!("/fixture/{repository_id}"),
                "a".repeat(40),
                CREATED_AT
            ],
        )
        .expect("repository");
}

fn seed_generation(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    status: &str,
    revision: char,
) {
    seed_generation_with_source(
        connection,
        repository_id,
        generation_id,
        status,
        revision,
        "source:fixture",
    );
}

fn seed_generation_with_source(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    status: &str,
    revision: char,
    source_identity: &str,
) {
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,created_at)
             VALUES (?1,?2,2,?3,?4,'parser:fixture','algorithm:fixture',
                     'config:fixture',?5,?6)",
            params![
                generation_id,
                repository_id,
                revision.to_string().repeat(40),
                source_identity,
                status,
                CREATED_AT
            ],
        )
        .expect("generation");
    if status == "ready" {
        connection
            .execute(
                "UPDATE archaeology_repositories SET ready_generation_id=?2 WHERE repository_id=?1",
                params![repository_id, generation_id],
            )
            .expect("ready pointer");
    }
}

fn promote_ready(connection: &Connection, repository_id: &str, generation_id: &str) {
    connection
        .execute(
            "UPDATE archaeology_generations SET status='ready',published_at=?3
             WHERE repository_id=?1 AND generation_id=?2 AND status='staging'",
            params![repository_id, generation_id, CREATED_AT],
        )
        .expect("promote ready generation");
    connection
        .execute(
            "UPDATE archaeology_repositories SET ready_generation_id=?2 WHERE repository_id=?1",
            params![repository_id, generation_id],
        )
        .expect("ready pointer");
}

fn seed_unit(
    connection: &Connection,
    generation_id: &str,
    source_unit_id: &str,
    path_identity: &str,
    lineage: &[ArchaeologyAdapterLineage],
) {
    let lineage_json = serde_json::to_string(lineage).expect("lineage JSON");
    connection
        .execute(
            "INSERT INTO archaeology_source_units
             (generation_id,source_unit_id,path_identity,relative_path,content_hash,
              hash_algorithm,language,parser_id,parser_version,classification,byte_count,
              line_count,include_lineage_json)
             VALUES (?1,?2,?3,?4,?5,'sha256','cobol','parser:fixture','1','source',16,1,?6)",
            params![
                generation_id,
                source_unit_id,
                path_identity,
                format!("{path_identity}.cbl"),
                "d".repeat(64),
                lineage_json
            ],
        )
        .expect("source unit");
    connection
        .execute(
            "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             SELECT ?1,?2,?3,revision_sha,0,1,1,1,1,2
             FROM archaeology_generations WHERE generation_id=?1",
            params![
                generation_id,
                format!("span:{source_unit_id}"),
                source_unit_id
            ],
        )
        .expect("source span");
}

fn seed_cross_unit_edge(
    connection: &Connection,
    generation_id: &str,
    from_unit: &str,
    to_unit: &str,
    kind: &str,
) {
    for (fact_id, unit_id) in [("fact:from", from_unit), ("fact:to", to_unit)] {
        connection
            .execute(
                "INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence)
                 VALUES (?1,?2,'declaration',?2,'parser:fixture','extracted','high')",
                params![generation_id, fact_id],
            )
            .expect("fact");
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'fact',?2,'span',?3,'supporting')",
                params![generation_id, fact_id, format!("span:{unit_id}")],
            )
            .expect("fact evidence");
    }
    connection
        .execute(
            "INSERT INTO archaeology_fact_edges
             (generation_id,edge_id,from_fact_id,to_fact_id,kind,trust)
             VALUES (?1,'edge:typed','fact:from','fact:to',?2,'deterministic')",
            params![generation_id, kind],
        )
        .expect("fact edge");
}

fn seed_fact(connection: &Connection, generation_id: &str, fact_id: &str, source_unit_id: &str) {
    connection
        .execute(
            "INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence)
             VALUES (?1,?2,'declaration',?2,'parser:fixture','extracted','high')",
            params![generation_id, fact_id],
        )
        .expect("fact");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'fact',?2,'span',?3,'supporting')",
            params![generation_id, fact_id, format!("span:{source_unit_id}")],
        )
        .expect("fact evidence");
}

fn inventory_unit(
    revision: char,
    source_unit_id: &str,
    path_identity: &str,
    hash: char,
) -> ArchaeologyInventoryUnit {
    ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: source_unit_id.into(),
            repository_id: "repo:a".into(),
            revision_sha: revision.to_string().repeat(40),
            path_identity: path_identity.into(),
            relative_path: Some(format!("{path_identity}.cbl")),
            content_hash: Some(hash.to_string().repeat(64)),
            hash_algorithm: Some("sha256".into()),
            change_identity: None,
        },
        classification: ArchaeologySourceClassification::Source,
        language: "cobol".into(),
        dialect: None,
        byte_count: 16,
        line_count: 1,
        include_candidates: Vec::new(),
        coverage_reasons: Vec::new(),
    }
}

fn inventory(root: &std::path::Path) -> Vec<ArchaeologyInventoryUnit> {
    let mut units = Vec::new();
    inventory_repository_streaming(
        root,
        &StructuralGraphCancellation::default(),
        ArchaeologyInventoryLimits::default(),
        &mut |unit| {
            units.push(unit);
            Ok(())
        },
    )
    .expect("repository inventory");
    units
}

fn inventory_unit_by_path<'a>(
    units: &'a [ArchaeologyInventoryUnit],
    path: &str,
) -> &'a ArchaeologyInventoryUnit {
    units
        .iter()
        .find(|unit| unit.identity.relative_path.as_deref() == Some(path))
        .unwrap_or_else(|| panic!("missing inventory path {path}"))
}

fn seed_inventory_generation(
    connection: &Connection,
    repository_id: &str,
    generation_id: &str,
    units: &[ArchaeologyInventoryUnit],
    status: &str,
) {
    let revision = &units.first().expect("inventory unit").identity.revision_sha;
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,created_at)
             VALUES (?1,?2,2,?3,'source:fixture','parser:fixture','algorithm:fixture',
                     'config:fixture',?4,?5)",
            params![generation_id, repository_id, revision, status, CREATED_AT],
        )
        .expect("inventory generation");
    for unit in units {
        let classification = match unit.classification {
            ArchaeologySourceClassification::Source => "source",
            ArchaeologySourceClassification::Generated => "generated",
            ArchaeologySourceClassification::Vendor => "vendor",
            ArchaeologySourceClassification::Protected => "protected",
            ArchaeologySourceClassification::Opaque => "opaque",
            ArchaeologySourceClassification::Unavailable => {
                panic!("inventory cannot emit unavailable classification")
            }
        };
        connection
            .execute(
                "INSERT INTO archaeology_source_units
                 (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                  hash_algorithm,change_identity,language,dialect,parser_id,parser_version,
                  classification,byte_count,line_count)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'parser:fixture','1',?10,?11,?12)",
                params![
                    generation_id,
                    unit.identity.source_unit_id,
                    unit.identity.path_identity,
                    unit.identity.relative_path,
                    unit.identity.content_hash,
                    unit.identity.hash_algorithm,
                    unit.identity.change_identity,
                    unit.language,
                    unit.dialect,
                    classification,
                    unit.byte_count,
                    unit.line_count,
                ],
            )
            .expect("inventory source unit");
    }
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_job(
    connection: &Connection,
    job_id: &str,
    repository_id: &str,
    generation_id: &str,
    owner_id: &str,
) {
    connection
        .execute(
            "INSERT INTO archaeology_jobs
             (job_id,repository_id,generation_id,owner_id,stage,state,updated_at)
             VALUES (?1,?2,?3,?4,'inventory','running',?5)",
            params![job_id, repository_id, generation_id, owner_id, CREATED_AT],
        )
        .expect("refresh job");
}

fn load_source_hashes(connection: &Connection, generation_id: &str) -> BTreeMap<String, String> {
    let mut statement = connection
        .prepare(
            "SELECT path_identity,content_hash FROM archaeology_source_units
             WHERE generation_id=?1 ORDER BY path_identity",
        )
        .expect("source hashes");
    statement
        .query_map([generation_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query source hashes")
        .map(|row| row.expect("read source hash"))
        .collect()
}

fn source_hash(connection: &Connection, generation_id: &str, path_identity: &str) -> String {
    connection
        .query_row(
            "SELECT content_hash FROM archaeology_source_units
             WHERE generation_id=?1 AND path_identity=?2",
            params![generation_id, path_identity],
            |row| row.get(0),
        )
        .expect("source hash")
}

fn persist_parsed_fixture_fact(
    transaction: &Transaction<'_>,
    item: &ArchaeologyRefreshWorkItem,
) -> Result<(), String> {
    if item.target_kind != "source_path" || item.action != "reprocess" {
        return Err("unexpected fixture refresh work".into());
    }
    let (source_unit_id, revision): (String, String) = transaction
        .query_row(
            "SELECT unit.source_unit_id,generation.revision_sha
             FROM archaeology_source_units unit
             JOIN archaeology_generations generation
               ON generation.generation_id=unit.generation_id
             WHERE unit.generation_id='generation:staging' AND unit.path_identity=?1",
            [&item.target_identity],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("load fixture parse unit: {error}"))?;
    let span_id = format!("span:parsed:{source_unit_id}");
    let fact_id = format!("fact:parsed:{source_unit_id}");
    transaction
        .execute(
            "UPDATE archaeology_source_units SET parser_id='parser:fixture',parser_version='1',
                 coverage_json='{}' WHERE generation_id='generation:staging' AND source_unit_id=?1",
            [&source_unit_id],
        )
        .and_then(|_| {
            transaction.execute(
                "INSERT INTO archaeology_source_spans
                 (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                  start_line,start_column,end_line,end_column)
                 VALUES ('generation:staging',?1,?2,?3,0,1,1,1,1,2)",
                params![span_id, source_unit_id, revision],
            )
        })
        .and_then(|_| {
            transaction.execute(
                "INSERT INTO archaeology_facts
                 (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
                 VALUES ('generation:staging',?1,'declaration',?1,'parser:fixture',
                         'extracted','high','[]')",
                [&fact_id],
            )
        })
        .and_then(|_| {
            transaction.execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation:staging','fact',?1,'span',?2,'supporting')",
                params![fact_id, span_id],
            )
        })
        .map_err(|error| format!("persist fixture parse result: {error}"))?;
    Ok(())
}

fn fact_owner_paths(connection: &Connection, generation_id: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT unit.path_identity FROM archaeology_facts fact
             JOIN archaeology_evidence_links evidence ON evidence.generation_id=fact.generation_id
              AND evidence.owner_kind='fact' AND evidence.owner_id=fact.fact_id
              AND evidence.evidence_kind='span'
             JOIN archaeology_source_spans span ON span.generation_id=evidence.generation_id
              AND span.span_id=evidence.evidence_id
             JOIN archaeology_source_units unit ON unit.generation_id=span.generation_id
              AND unit.source_unit_id=span.source_unit_id
             WHERE fact.generation_id=?1 ORDER BY unit.path_identity",
        )
        .expect("fact owner paths");
    statement
        .query_map([generation_id], |row| row.get::<_, String>(0))
        .expect("query fact owner paths")
        .map(|row| row.expect("read fact owner path"))
        .collect()
}

fn resolved_lineage(
    kind: ArchaeologyLineageKind,
    source_unit_id: &str,
    target_source_unit_id: &str,
) -> ArchaeologyAdapterLineage {
    ArchaeologyAdapterLineage {
        kind,
        source_unit_id: source_unit_id.into(),
        target_source_unit_id: Some(target_source_unit_id.into()),
        evidence_span_id: format!("span:{source_unit_id}"),
        detail: "exact linked target".into(),
    }
}

fn unresolved_lineage(source_unit_id: &str) -> ArchaeologyAdapterLineage {
    ArchaeologyAdapterLineage {
        kind: ArchaeologyLineageKind::Copybook,
        source_unit_id: source_unit_id.into(),
        target_source_unit_id: None,
        evidence_span_id: format!("span:{source_unit_id}"),
        detail: "unresolved copybook target".into(),
    }
}

fn inputs(head: char) -> Vec<ArchaeologyGenerationInput> {
    vec![
        ArchaeologyGenerationInput {
            kind: InputKind::Head,
            scope: None,
            identity: head.to_string().repeat(40),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::Ignore,
            scope: None,
            identity: "ignore:v1".into(),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::Config,
            scope: None,
            identity: "config:fixture".into(),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::Parser,
            scope: Some("global".into()),
            identity: "parser:fixture".into(),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::Schema,
            scope: None,
            identity: "schema:v2".into(),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::Algorithm,
            scope: None,
            identity: "algorithm:fixture".into(),
        },
        ArchaeologyGenerationInput {
            kind: InputKind::SynthesisPolicy,
            scope: Some("global".into()),
            identity: "synthesis:v1".into(),
        },
    ]
}
