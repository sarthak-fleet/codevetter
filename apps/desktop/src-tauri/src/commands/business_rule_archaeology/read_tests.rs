use super::*;
use crate::db::archaeology_schema::run_migration;
use rusqlite::{params, Connection};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const REPO: &str = "archaeology-repository:one";
const OTHER_REPO: &str = "archaeology-repository:two";
const GENERATION: &str = "archaeology-generation:one";
const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn hashed(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn coverage() -> String {
    serde_json::json!({
        "state": "complete",
        "parser_coverage": "complete",
        "repository_coverage": "complete",
        "temporal_coverage": "unavailable",
        "discovered_source_units": 1,
        "indexed_source_units": 1,
        "discovered_bytes": 100,
        "indexed_bytes": 100,
        "reasons": []
    })
    .to_string()
}

fn fixture() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .expect("foreign keys");
    run_migration(&connection).expect("archaeology schema");
    seed_repository(&connection, REPO, GENERATION, REVISION, true);
    seed_repository(
        &connection,
        OTHER_REPO,
        "archaeology-generation:other",
        &"b".repeat(40),
        true,
    );

    connection
        .execute(
            "INSERT INTO archaeology_source_units
             (generation_id,source_unit_id,path_identity,relative_path,content_hash,
              hash_algorithm,language,dialect,parser_id,parser_version,classification,
              byte_count,line_count,coverage_json)
             VALUES (?1,'source-unit:one','source-path:one','src/rules.cbl',?2,
                     'sha256','cobol','fixed','parser:cobol','1','source',100,10,?3)",
            params![GENERATION, "c".repeat(64), coverage()],
        )
        .expect("source unit");
    connection
        .execute(
            "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             VALUES (?1,'span:one','source-unit:one',?2,10,30,2,1,3,4)",
            params![GENERATION, REVISION],
        )
        .expect("span");
    connection
        .execute(
            "INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
             VALUES (?1,'fact:one','predicate','Claim amount is positive','parser:cobol',
                     'extracted','high','[]')",
            [GENERATION],
        )
        .expect("fact");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'fact','fact:one','span','span:one','supporting')",
            [GENERATION],
        )
        .expect("fact span");

    seed_rule(
        &connection,
        REPO,
        GENERATION,
        "occurrence:one",
        "1",
        "Eligible claims are scheduled",
        "accepted",
    );
    seed_rule(
        &connection,
        REPO,
        GENERATION,
        "occurrence:two",
        "2",
        "Positive claims require review",
        "candidate",
    );
    seed_rule(
        &connection,
        REPO,
        GENERATION,
        "occurrence:alias",
        "3",
        "Generated eligibility alias",
        "candidate",
    );
    for (rule, clause, ordinal) in [
        ("occurrence:one", "clause:one", 0),
        ("occurrence:two", "clause:two", 0),
    ] {
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES (?1,?2,?3,?4,'A claim is handled when its amount is positive.',
                         'deterministic','high','[]')",
                params![GENERATION, rule, clause, ordinal],
            )
            .expect("clause");
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'rule_clause',?2,'fact','fact:one','supporting'),
                        (?1,'rule_clause',?2,'span','span:one','supporting')",
                params![GENERATION, clause],
            )
            .expect("clause evidence");
    }
    connection
        .execute_batch(
            "INSERT INTO archaeology_rule_domains
             (generation_id,rule_id,domain_id,domain_label)
             VALUES ('archaeology-generation:one','occurrence:one','domain:claims','Claims'),
                    ('archaeology-generation:one','occurrence:two','domain:claims','Claims');
             INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust,summary)
             VALUES ('archaeology-generation:one','relation:dependency','occurrence:one',
                     'occurrence:two','depends_on','deterministic','Uses the reviewed claim rule'),
                    ('archaeology-generation:one','relation:alias','occurrence:alias',
                     'occurrence:one','aliases','deterministic','Exact generated duplicate');",
        )
        .expect("domains and relations");
    connection
}

fn seed_repository(
    connection: &Connection,
    repository: &str,
    generation: &str,
    revision: &str,
    ready: bool,
) {
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES (?1,?2,?3,?4,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            params![
                repository,
                format!("/private/{repository}"),
                digest('a'),
                revision
            ],
        )
        .expect("repository");
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,coverage_json,
              created_at,published_at)
             VALUES (?1,?2,2,?3,?4,?5,?6,?7,?8,?9,
                     '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            params![
                generation,
                repository,
                revision,
                digest('a'),
                digest('b'),
                digest('c'),
                digest('d'),
                if ready { "ready" } else { "superseded" },
                coverage(),
            ],
        )
        .expect("generation");
    if ready {
        connection
            .execute(
                "UPDATE archaeology_repositories SET ready_generation_id=?2
                 WHERE repository_id=?1",
                params![repository, generation],
            )
            .expect("ready pointer");
    }
}

fn seed_rule(
    connection: &Connection,
    repository: &str,
    generation: &str,
    occurrence: &str,
    identity_seed: &str,
    title: &str,
    decision: &str,
) {
    let stable = if identity_seed.len() == 1 {
        digest(
            identity_seed
                .chars()
                .next()
                .expect("one identity character"),
        )
    } else {
        hashed(&format!("stable:{identity_seed}"))
    };
    let evidence = hashed(&format!("evidence:{identity_seed}"));
    let contradiction = hashed(&format!("contradiction:{identity_seed}"));
    let description = hashed(&format!("description:{identity_seed}"));
    let continuity = hashed(&format!("continuity:{identity_seed}"));
    let parser = digest('e');
    connection
        .execute(
            "INSERT INTO archaeology_rules
             (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
              confidence,parser_identity,algorithm_identity,coverage_json,created_at,
              identity_schema_version,stable_rule_identity,evidence_identity,
              contradiction_identity,description_identity,continuity_identity,
              parser_compatibility_identity,identity_provenance_json)
             VALUES (?1,?2,?3,?4,'validation',?5,'candidate','deterministic','high',
                     ?6,?7,?8,'2026-01-01T00:00:00Z',2,?9,?10,?11,?12,?13,?14,'{}')",
            params![
                generation,
                occurrence,
                repository,
                REVISION,
                title,
                digest('b'),
                digest('c'),
                coverage(),
                stable,
                evidence,
                contradiction,
                description,
                continuity,
                parser,
            ],
        )
        .expect("rule");
    connection
        .execute(
            "INSERT INTO archaeology_rule_search_manifest
             (generation_id,rule_id,title,clause_text,domain_text)
             VALUES (?1,?2,?3,?3,'Claims')",
            params![generation, occurrence, title],
        )
        .expect("search manifest");
    let candidate_event = hashed(&format!("event:candidate:{identity_seed}"));
    let stream = hashed(&format!("stream:{identity_seed}"));
    connection
        .execute(
            "INSERT INTO archaeology_rule_review_events
             (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,
              evidence_identity,created_at,event_schema_version,event_stream_identity,
              logical_sequence,stable_rule_identity,contradiction_identity,
              description_identity,continuity_identity,parser_identity,actor_kind,
              reviewer_provenance_json,legacy_stale)
             VALUES (?1,?2,?3,?4,'candidate','codevetter:local',?5,
                     '2026-01-01T00:00:00Z',2,?6,1,?7,?8,?9,?10,?11,
                     'deterministic_policy','{}',0)",
            params![
                candidate_event,
                repository,
                occurrence,
                generation,
                evidence,
                stream,
                stable,
                contradiction,
                description,
                continuity,
                parser,
            ],
        )
        .expect("review event");
    if decision != "candidate" {
        connection
            .execute(
                "INSERT INTO archaeology_rule_review_events
                 (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,
                  evidence_identity,created_at,event_schema_version,event_stream_identity,
                  logical_sequence,stable_rule_identity,contradiction_identity,
                  description_identity,continuity_identity,parser_identity,prior_event_id,
                  actor_kind,reviewer_provenance_json,legacy_stale)
                 VALUES (?1,?2,?3,?4,?5,'reviewer:fixture',?6,
                         '2026-01-01T00:00:01Z',2,?7,2,?8,?9,?10,?11,?12,?13,
                         'human','{}',0)",
                params![
                    hashed(&format!("event:{decision}:{identity_seed}")),
                    repository,
                    occurrence,
                    generation,
                    decision,
                    evidence,
                    stream,
                    stable,
                    contradiction,
                    description,
                    continuity,
                    parser,
                    candidate_event,
                ],
            )
            .expect("decision event");
    }
}

fn list_request(
    repository_id: &str,
    limit: usize,
    cursor: Option<String>,
) -> ArchaeologyReadRequest {
    ArchaeologyReadRequest::ListRules {
        repository_id: repository_id.into(),
        filter: ArchaeologyRuleFilter::default(),
        limit: Some(limit),
        cursor,
    }
}

fn list(response: ArchaeologyReadResponse) -> ArchaeologyPage<ArchaeologyRuleSummary> {
    match response {
        ArchaeologyReadResponse::ListRules(page) => *page,
        other => panic!("unexpected response: {other:?}"),
    }
}

fn temporal_payload(title: &str) -> String {
    serde_json::json!({
        "title": title,
        "clauses": [{
            "ordinal": 0,
            "text": "A claim is handled when its amount is positive.",
            "trust": "deterministic",
            "confidence": "high",
            "caveats": [],
            "evidence": [{
                "role": "supporting",
                "fact_identity": "fact:one",
                "fact_kind": "predicate",
                "parser_identity": "parser:cobol",
                "spans": [{
                    "path_identity": "source-path:one",
                    "content_hash": "raw-content-hash-marker",
                    "start_byte": 10,
                    "end_byte": 30,
                    "start_line": 2,
                    "start_column": 1,
                    "end_line": 3,
                    "end_column": 4
                }]
            }]
        }]
    })
    .to_string()
}

fn seed_temporal_history(connection: &Connection) -> (String, String) {
    const BEFORE_GENERATION: &str = "archaeology-generation:before";
    const BEFORE_REVISION: &str = "ffffffffffffffffffffffffffffffffffffffff";
    let before_temporal = digest('7');
    let after_temporal = digest('8');
    let before_snapshot = digest('5');
    let after_snapshot = digest('6');
    let stable = digest('1');
    let continuity = digest('4');
    connection
        .execute_batch(include_str!("../../db/schema/history_graph.sql"))
        .expect("history schema");
    connection
        .execute_batch(include_str!(
            "../../db/schema/history_graph_release_catalog.sql"
        ))
        .expect("release schema");
    let repo_path = format!("/private/{REPO}");
    connection
        .execute(
            "INSERT INTO history_graph_repositories
             (repo_path,repository_fingerprint,indexed_head,status,coverage_json,created_at,updated_at)
             VALUES (?1,'fixture',?2,'ready','{}','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            params![repo_path, REVISION],
        )
        .expect("history repository");
    for (revision, ordinal) in [(BEFORE_REVISION, 1_i64), (REVISION, 2_i64)] {
        connection
            .execute(
                "INSERT INTO history_graph_revisions
                 (repo_path,sha,ordinal,committed_at,author_name,subject,parents_json,coverage_json)
                 VALUES (?1,?2,?3,'2026-01-01T00:00:00Z','fixture','fixture','[]','{}')",
                params![repo_path, revision, ordinal],
            )
            .expect("history revision");
    }
    connection
        .execute(
            "INSERT INTO history_graph_release_catalogs
             (repo_path,index_identity,indexed_head,tags_fingerprint,status,coverage_json,updated_at)
             VALUES (?1,'catalog',?2,'tags','ready','{}','2026-01-01T00:00:00Z')",
            params![repo_path, REVISION],
        )
        .expect("release catalog");
    connection
        .execute(
            "INSERT INTO history_graph_release_tags
             (repo_path,tag,revision_sha,tag_object_sha,tag_kind,tagged_at)
             VALUES (?1,'v2',?2,?3,'lightweight',1)",
            params![repo_path, REVISION, "e".repeat(40)],
        )
        .expect("release tag");
    for (temporal, generation, revision, prior) in [
        (&before_temporal, BEFORE_GENERATION, BEFORE_REVISION, None),
        (
            &after_temporal,
            GENERATION,
            REVISION,
            Some(before_temporal.as_str()),
        ),
    ] {
        connection
            .execute(
                "INSERT INTO archaeology_temporal_generations
                 (temporal_generation_identity,repository_id,generation_id,revision_sha,
                  prior_temporal_generation_identity,source_schema_version,catalog_identity,
                  rule_count,coverage_state,coverage_reasons_json,created_at)
                 VALUES (?1,?2,?3,?4,?5,2,?6,1,'complete','[]','2026-01-01T00:00:00Z')",
                params![temporal, REPO, generation, revision, prior, digest('3')],
            )
            .expect("temporal generation");
    }
    for (snapshot, title) in [
        (&before_snapshot, "Claims require review"),
        (&after_snapshot, "Eligible claims are scheduled"),
    ] {
        connection
            .execute(
                "INSERT INTO archaeology_rule_temporal_snapshots
                 (snapshot_identity,repository_id,stable_rule_identity,continuity_identity,
                  rule_kind,evidence_identity,parser_compatibility_identity,
                  contradiction_identity,description_identity,payload_json,created_at)
                 VALUES (?1,?2,?3,?4,'validation',?5,?6,?7,?8,?9,'2026-01-01T00:00:00Z')",
                params![
                    snapshot,
                    REPO,
                    stable,
                    continuity,
                    digest('2'),
                    digest('a'),
                    digest('b'),
                    digest('c'),
                    temporal_payload(title)
                ],
            )
            .expect("temporal snapshot");
    }
    connection
        .execute(
            "INSERT INTO archaeology_rule_temporal_events
             (event_identity,repository_id,temporal_generation_identity,
              prior_temporal_generation_identity,event_kind,stable_rule_identity,
              continuity_identity,before_snapshot_identity,after_snapshot_identity,
              coverage_state,coverage_reasons_json,created_at)
             VALUES (?1,?2,?3,?4,'changed',?5,?6,?7,?8,'complete','[]',
                     '2026-01-01T00:00:00Z')",
            params![
                digest('9'),
                REPO,
                after_temporal,
                before_temporal,
                stable,
                continuity,
                before_snapshot,
                after_snapshot
            ],
        )
        .expect("temporal event");
    let introduced_snapshot = digest('d');
    let introduced_stable = digest('2');
    let introduced_continuity = digest('e');
    connection
        .execute(
            "INSERT INTO archaeology_rule_temporal_snapshots
             (snapshot_identity,repository_id,stable_rule_identity,continuity_identity,
              rule_kind,evidence_identity,parser_compatibility_identity,
              contradiction_identity,description_identity,payload_json,created_at)
             VALUES (?1,?2,?3,?4,'eligibility',?5,?6,?7,?8,?9,
                     '2026-01-01T00:00:00Z')",
            params![
                introduced_snapshot,
                REPO,
                introduced_stable,
                introduced_continuity,
                digest('2'),
                digest('a'),
                digest('b'),
                digest('c'),
                temporal_payload("A new eligibility rule")
            ],
        )
        .expect("introduced snapshot");
    connection
        .execute(
            "INSERT INTO archaeology_rule_temporal_events
             (event_identity,repository_id,temporal_generation_identity,
              prior_temporal_generation_identity,event_kind,stable_rule_identity,
              continuity_identity,after_snapshot_identity,coverage_state,
              coverage_reasons_json,created_at)
             VALUES (?1,?2,?3,?4,'introduced',?5,?6,?7,'complete','[]',
                     '2026-01-01T00:00:00Z')",
            params![
                digest('f'),
                REPO,
                after_temporal,
                before_temporal,
                introduced_stable,
                introduced_continuity,
                introduced_snapshot
            ],
        )
        .expect("introduced event");
    (BEFORE_GENERATION.into(), BEFORE_REVISION.into())
}

#[test]
fn strict_requests_reject_unknown_fields_and_missing_temporal_selectors() {
    let unknown = serde_json::json!({
        "operation": "list_rules",
        "repository_id": REPO,
        "filter": {},
        "limit": 10,
        "cursor": null,
        "ignored": true
    });
    assert!(serde_json::from_value::<ArchaeologyReadRequest>(unknown).is_err());

    let connection = fixture();
    let error = ArchaeologyReadService::new(&connection)
        .execute(ArchaeologyReadRequest::CompareTemporal {
            repository_id: REPO.into(),
            before: ArchaeologyTemporalSelector::Revision {
                revision_sha: REVISION.into(),
            },
            after: ArchaeologyTemporalSelector::Release { tag: "v2".into() },
            limit: Some(10),
            cursor: None,
        })
        .expect_err("missing selector must fail closed");
    assert_eq!(error, UNAVAILABLE);
}

#[test]
fn temporal_compare_resolves_exact_release_and_returns_typed_persisted_snapshots() {
    let connection = fixture();
    let (before_generation, before_revision) = seed_temporal_history(&connection);
    let response = ArchaeologyReadService::new(&connection)
        .execute(ArchaeologyReadRequest::CompareTemporal {
            repository_id: REPO.into(),
            before: ArchaeologyTemporalSelector::Generation {
                generation_id: before_generation.clone(),
            },
            after: ArchaeologyTemporalSelector::Release { tag: "v2".into() },
            limit: Some(1),
            cursor: None,
        })
        .expect("temporal comparison");
    let ArchaeologyReadResponse::CompareTemporal(result) = response else {
        panic!("wrong response")
    };
    assert_eq!(result.value.coverage, "complete");
    assert!(result.value.reasons.is_empty());
    assert_eq!(result.value.before.generation_id, before_generation);
    assert_eq!(result.value.before.revision_sha, before_revision);
    assert_eq!(result.value.after.generation_id, GENERATION);
    assert_eq!(result.value.after.revision_sha, REVISION);
    assert_eq!(result.value.page.total_rows, 2);
    assert_eq!(result.value.page.returned_rows, 1);
    assert!(result.value.page.truncated);
    let change = &result.value.changes[0];
    assert_eq!(change.classification, "changed");
    let before = change.before.as_ref().expect("before snapshot");
    let after = change.after.as_ref().expect("after snapshot");
    assert_eq!(before.payload.title, "Claims require review");
    assert_eq!(after.payload.title, "Eligible claims are scheduled");
    assert_eq!(after.payload.clauses[0].evidence[0].spans[0].start_byte, 10);
    let serialized = serde_json::to_string(&result).expect("serialize temporal result");
    for private in ["repo_path", "/private/", "absolute_path", "source_body"] {
        assert!(!serialized.contains(private), "leaked {private}");
    }
    assert!(!serialized.contains("content_hash"));
    assert!(!serialized.contains("raw-content-hash-marker"));

    let second = ArchaeologyReadService::new(&connection)
        .execute(ArchaeologyReadRequest::CompareTemporal {
            repository_id: REPO.into(),
            before: ArchaeologyTemporalSelector::Generation {
                generation_id: before_generation,
            },
            after: ArchaeologyTemporalSelector::Release { tag: "v2".into() },
            limit: Some(1),
            cursor: result.value.page.next_cursor.clone(),
        })
        .expect("second temporal page");
    let ArchaeologyReadResponse::CompareTemporal(second) = second else {
        panic!("wrong response")
    };
    assert_eq!(second.value.changes.len(), 1);
    assert_eq!(second.value.changes[0].classification, "introduced");
    assert!(!second.value.page.truncated);
}

#[test]
fn temporal_compare_degrades_when_persisted_lineage_is_not_adjacent() {
    let connection = fixture();
    let (before_generation, _) = seed_temporal_history(&connection);
    let response = ArchaeologyReadService::new(&connection)
        .execute(ArchaeologyReadRequest::CompareTemporal {
            repository_id: REPO.into(),
            before: ArchaeologyTemporalSelector::Generation {
                generation_id: GENERATION.into(),
            },
            after: ArchaeologyTemporalSelector::Generation {
                generation_id: before_generation,
            },
            limit: Some(10),
            cursor: None,
        })
        .expect("partial temporal comparison");
    let ArchaeologyReadResponse::CompareTemporal(result) = response else {
        panic!("wrong response")
    };
    assert_eq!(result.value.coverage, "partial");
    assert_eq!(result.value.reasons, ["temporal_lineage_not_adjacent"]);
    assert!(result.value.changes.is_empty());
}

#[test]
fn freshness_uses_only_active_persisted_current_input_identities() {
    let connection = fixture();
    let initial = list(
        ArchaeologyReadService::new(&connection)
            .execute(list_request(REPO, 10, None))
            .expect("initial read"),
    );
    assert_eq!(initial.context.freshness.current_parser_identity, None);
    assert_eq!(initial.context.freshness.current_config_identity, None);
    assert!(initial.context.freshness.human_review_decisions_present);
    assert!(!initial.context.freshness.human_review_decisions_stale);
    assert!(initial
        .context
        .freshness
        .human_review_stale_reasons
        .is_empty());

    let staging = "archaeology-generation:active-input";
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
             VALUES (?1,?2,2,?3,?4,?5,?6,?7,'staging',?8,'2026-01-02T00:00:00Z')",
            params![
                staging,
                REPO,
                REVISION,
                digest('a'),
                digest('8'),
                digest('c'),
                digest('9'),
                coverage()
            ],
        )
        .expect("staging generation");
    connection
        .execute(
            "INSERT INTO archaeology_jobs
             (job_id,repository_id,generation_id,owner_id,stage,state,updated_at)
             VALUES ('job:active-input',?1,?2,'owner:test','parse','running',
                     '2026-01-02T00:00:00Z')",
            params![REPO, staging],
        )
        .expect("active job");
    let current = list(
        ArchaeologyReadService::new(&connection)
            .execute(list_request(REPO, 10, None))
            .expect("current-input read"),
    );
    assert_eq!(
        current.context.freshness.current_parser_identity,
        Some(digest('8'))
    );
    assert_eq!(
        current.context.freshness.current_config_identity,
        Some(digest('9'))
    );
    assert!(current.context.freshness.stale);
    assert!(current.context.freshness.human_review_decisions_present);
    assert!(current.context.freshness.human_review_decisions_stale);
    assert!(current
        .context
        .freshness
        .reasons
        .contains(&"parser_identity_changed".into()));
    assert!(current
        .context
        .freshness
        .reasons
        .contains(&"config_identity_changed".into()));
    assert!(current
        .context
        .freshness
        .human_review_stale_reasons
        .contains(&"parser_identity_changed".into()));
}

#[test]
fn desktop_command_core_uses_the_bounded_service_without_private_storage_fields() {
    let connection = fixture();
    let response = read_business_rule_archaeology_core(
        &connection,
        ArchaeologyReadRequest::ListRules {
            repository_id: REPO.into(),
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(usize::MAX),
            cursor: None,
        },
    )
    .expect("desktop read");
    let ArchaeologyReadResponse::ListRules(page) = &response else {
        panic!("wrong response")
    };
    assert_eq!(page.page.applied_limit, MAX_PAGE_LIMIT);
    assert!(serialized_bytes(&response).expect("serialize") <= MAX_RESPONSE_BYTES);

    let serialized = serde_json::to_string(&response).expect("wire response");
    for private in [
        "repo_path",
        "source_body",
        "absolute_path",
        "/private/",
        "occurrence:",
    ] {
        assert!(!serialized.contains(private), "leaked {private}");
    }
}

#[test]
fn canonical_pages_reconcile_aliases_and_effective_lifecycle() {
    let connection = fixture();
    let service = ArchaeologyReadService::new(&connection);
    let first = list(
        service
            .execute(list_request(REPO, 1, None))
            .expect("first page"),
    );
    assert_eq!(first.page.total_rows, 2);
    assert_eq!(first.items.len(), 1);
    assert!(first.page.truncated);
    let second = list(
        service
            .execute(list_request(REPO, 1, first.page.next_cursor.clone()))
            .expect("second page"),
    );
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.page.total_rows, 2);
    assert_ne!(first.items[0].rule_id, second.items[0].rule_id);
    assert!(!second.page.truncated);
    let lifecycles = first
        .items
        .iter()
        .chain(&second.items)
        .map(|rule| rule.lifecycle.clone())
        .collect::<Vec<_>>();
    assert!(lifecycles.contains(&ArchaeologyRuleLifecycle::Accepted));
    assert!(lifecycles.contains(&ArchaeologyRuleLifecycle::Candidate));
    assert!(first.context.freshness.reasons.is_empty());
    assert_eq!(first.context.language_coverage[0].language, "cobol");
}

#[test]
fn injected_live_head_marks_a_persisted_catalog_stale() {
    let connection = fixture();
    let page = list(
        ArchaeologyReadService::new_with_current_head(&connection, "d".repeat(40))
            .execute(list_request(REPO, 10, None))
            .expect("stale page"),
    );
    assert!(page.context.freshness.stale);
    assert!(page
        .context
        .freshness
        .reasons
        .contains(&"repository_revision_changed".to_string()));
}

#[test]
fn rule_search_keeps_exact_total_with_the_single_pass_page_query() {
    let connection = fixture();
    let page = list(
        ArchaeologyReadService::new(&connection)
            .execute(ArchaeologyReadRequest::ListRules {
                repository_id: REPO.into(),
                filter: ArchaeologyRuleFilter {
                    query: Some("scheduled".into()),
                    ..Default::default()
                },
                limit: Some(10),
                cursor: None,
            })
            .expect("search rules"),
    );
    assert_eq!(page.page.total_rows, 1);
    assert_eq!(page.page.returned_rows, 1);
    assert_eq!(page.items[0].title, "Eligible claims are scheduled");
}

#[test]
fn rule_search_plan_pages_from_fts_matches_without_rescanning_the_rule_generation() {
    let connection = fixture();
    let service = ArchaeologyReadService::new(&connection);
    let scope = service.ready_scope(REPO).expect("ready scope");
    let mut filter = ArchaeologyRuleFilter {
        query: Some("scheduled".into()),
        ..Default::default()
    };
    normalize_filter(&mut filter).expect("normalize search");
    let (where_sql, mut values, fts) = rule_predicates(&scope, &filter).expect("predicates");
    assert!(fts);
    values.push(scope.generation_id.into());
    values.push(51_i64.into());
    let sql = rule_list_sql(rule_list_from_sql(fts), &where_sql, "");
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare search query plan");
    let details = statement
        .query_map(params_from_iter(values), |row| row.get::<_, String>(3))
        .expect("query search plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("read search plan");

    assert!(
        details
            .iter()
            .any(|detail| detail.contains("SCAN archaeology_rule_fts VIRTUAL TABLE")),
        "search must start from the bounded FTS matches: {details:?}"
    );
    assert!(
        details.iter().any(|detail| detail == "SCAN matched"),
        "the page must iterate materialized matches first: {details:?}"
    );
    let rule_searches = details
        .iter()
        .filter(|detail| detail.contains("SEARCH rule"))
        .collect::<Vec<_>>();
    assert!(
        !rule_searches.is_empty()
            && rule_searches
                .iter()
                .all(|detail| detail.contains("generation_id=? AND rule_id=?")),
        "rule hydration must use exact primary-key probes: {details:?}"
    );
    assert!(
        details
            .iter()
            .all(|detail| !detail.contains("AUTOMATIC COVERING INDEX (rule_id=?)")),
        "the page must not build a transient match index and rescan every rule: {details:?}"
    );
}

#[test]
fn selective_search_and_reverse_work_stay_bounded_with_irrelevant_catalog_noise() {
    const NOISE_ROWS: usize = 4_096;
    const PROGRESS_INTERVAL: i32 = 100;
    const MAX_SEARCH_CALLBACKS: usize = 10;
    const MAX_BROAD_SEARCH_STEPS_PER_MATCH: usize = 128;
    const MAX_REVERSE_CALLBACKS: usize = 30;
    const FANOUT_ROWS: usize = 512;
    const MAX_FANOUT_REVERSE_STEPS_PER_RULE: usize = 160;
    let connection = fixture();
    connection
        .execute_batch(&format!(
            "WITH RECURSIVE sequence(value) AS (
               VALUES(1) UNION ALL SELECT value+1 FROM sequence WHERE value<{NOISE_ROWS}
             )
             INSERT INTO archaeology_rules
               (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                confidence,parser_identity,algorithm_identity,synthesis_identity,coverage_json,
                created_at,identity_schema_version,stable_rule_identity,evidence_identity,
                contradiction_identity,description_identity,continuity_identity,
                parser_compatibility_identity,identity_provenance_json)
             SELECT template.generation_id,'noise-rule:'||printf('%06d',sequence.value),
                    template.repository_id,template.revision_sha,template.kind,
                    'Background invariant',template.lifecycle,template.trust,template.confidence,
                    template.parser_identity,template.algorithm_identity,template.synthesis_identity,
                    template.coverage_json,template.created_at,template.identity_schema_version,
                    'sha256:'||printf('%064x',sequence.value),template.evidence_identity,
                    template.contradiction_identity,template.description_identity,
                    template.continuity_identity,template.parser_compatibility_identity,
                    template.identity_provenance_json
             FROM sequence JOIN archaeology_rules template
               ON template.generation_id='{GENERATION}' AND template.rule_id='occurrence:one';
             WITH RECURSIVE sequence(value) AS (
               VALUES(1) UNION ALL SELECT value+1 FROM sequence WHERE value<{NOISE_ROWS}
             )
             INSERT INTO archaeology_rule_search_manifest
               (generation_id,rule_id,title,clause_text,domain_text)
             SELECT '{GENERATION}','noise-rule:'||printf('%06d',value),
                    'Background invariant','Unrelated behavior','Background'
             FROM sequence;
             WITH RECURSIVE sequence(value) AS (
               VALUES(1) UNION ALL SELECT value+1 FROM sequence WHERE value<{NOISE_ROWS}
             )
             INSERT INTO archaeology_evidence_links
               (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             SELECT '{GENERATION}','fact','noise-fact:'||printf('%06d',value),
                    'span','noise-span:'||printf('%06d',value),'supporting'
             FROM sequence;"
        ))
        .expect("seed irrelevant catalog noise");

    let service = ArchaeologyReadService::new(&connection);
    let (search, search_callbacks) =
        counted_sqlite_progress(&connection, PROGRESS_INTERVAL, || {
            service.execute(ArchaeologyReadRequest::ListRules {
                repository_id: REPO.into(),
                filter: ArchaeologyRuleFilter {
                    query: Some("scheduled".into()),
                    ..Default::default()
                },
                limit: Some(50),
                cursor: None,
            })
        });
    let page = list(search.expect("selective search"));
    assert_eq!(page.page.total_rows, 1);
    assert_eq!(page.items[0].title, "Eligible claims are scheduled");
    eprintln!(
        "ARCHAEOLOGY_READ_VM_STEPS search={} limit<{}",
        search_callbacks * PROGRESS_INTERVAL as usize,
        MAX_SEARCH_CALLBACKS * PROGRESS_INTERVAL as usize
    );
    assert!(
        search_callbacks < MAX_SEARCH_CALLBACKS,
        "selective search used too much SQLite work: {search_callbacks} callbacks at {PROGRESS_INTERVAL} VM steps"
    );

    let (broad_search, broad_search_callbacks) =
        counted_sqlite_progress(&connection, PROGRESS_INTERVAL, || {
            service.execute(ArchaeologyReadRequest::ListRules {
                repository_id: REPO.into(),
                filter: ArchaeologyRuleFilter {
                    query: Some("background".into()),
                    ..Default::default()
                },
                limit: Some(50),
                cursor: None,
            })
        });
    let broad_page = list(broad_search.expect("broad search"));
    assert_eq!(broad_page.page.total_rows, NOISE_ROWS as u64);
    assert_eq!(broad_page.items.len(), 50);
    assert!(broad_page.page.truncated);
    eprintln!(
        "ARCHAEOLOGY_READ_VM_STEPS broad_search={} limit<{}",
        broad_search_callbacks * PROGRESS_INTERVAL as usize,
        NOISE_ROWS * MAX_BROAD_SEARCH_STEPS_PER_MATCH
    );
    assert!(
        broad_search_callbacks * (PROGRESS_INTERVAL as usize)
            < NOISE_ROWS * MAX_BROAD_SEARCH_STEPS_PER_MATCH,
        "broad search exceeded its per-match SQLite work bound: {broad_search_callbacks} callbacks at {PROGRESS_INTERVAL} VM steps"
    );

    let (reverse, reverse_callbacks) =
        counted_sqlite_progress(&connection, PROGRESS_INTERVAL, || {
            service.execute(ArchaeologyReadRequest::ReverseSource {
                repository_id: REPO.into(),
                source: ArchaeologySourceSelector::Path {
                    path_identity: "source-path:one".into(),
                },
                limit: Some(50),
                cursor: None,
            })
        });
    let ArchaeologyReadResponse::ReverseSource(reverse) = reverse.expect("selective reverse")
    else {
        panic!("wrong response")
    };
    assert_eq!(reverse.page.total_rows, 2);
    eprintln!(
        "ARCHAEOLOGY_READ_VM_STEPS reverse={} limit<{}",
        reverse_callbacks * PROGRESS_INTERVAL as usize,
        MAX_REVERSE_CALLBACKS * PROGRESS_INTERVAL as usize
    );
    assert!(
        reverse_callbacks < MAX_REVERSE_CALLBACKS,
        "selective reverse lookup used too much SQLite work: {reverse_callbacks} callbacks at {PROGRESS_INTERVAL} VM steps"
    );

    connection
        .execute_batch(&format!(
            "WITH RECURSIVE sequence(value) AS (
               VALUES(1) UNION ALL SELECT value+1 FROM sequence WHERE value<{FANOUT_ROWS}
             )
             INSERT INTO archaeology_rule_clauses
               (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
             SELECT '{GENERATION}','noise-rule:'||printf('%06d',value),
                    'noise-clause:'||printf('%06d',value),0,
                    'Fanout rule uses the selected source.','deterministic','high','[]'
             FROM sequence;
             WITH RECURSIVE sequence(value) AS (
               VALUES(1) UNION ALL SELECT value+1 FROM sequence WHERE value<{FANOUT_ROWS}
             )
             INSERT INTO archaeology_evidence_links
               (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             SELECT '{GENERATION}','rule_clause','noise-clause:'||printf('%06d',value),
                    'span','span:one','supporting'
             FROM sequence;"
        ))
        .expect("seed reverse fanout");
    let (fanout, fanout_callbacks) =
        counted_sqlite_progress(&connection, PROGRESS_INTERVAL, || {
            service.execute(ArchaeologyReadRequest::ReverseSource {
                repository_id: REPO.into(),
                source: ArchaeologySourceSelector::Path {
                    path_identity: "source-path:one".into(),
                },
                limit: Some(50),
                cursor: None,
            })
        });
    let ArchaeologyReadResponse::ReverseSource(fanout) = fanout.expect("fanout reverse") else {
        panic!("wrong response")
    };
    assert_eq!(fanout.page.total_rows, (FANOUT_ROWS + 2) as u64);
    assert_eq!(fanout.items.len(), 50);
    assert!(fanout.page.truncated);
    eprintln!(
        "ARCHAEOLOGY_READ_VM_STEPS fanout_reverse={} limit<{}",
        fanout_callbacks * PROGRESS_INTERVAL as usize,
        (FANOUT_ROWS + 2) * MAX_FANOUT_REVERSE_STEPS_PER_RULE
    );
    assert!(
        fanout_callbacks * (PROGRESS_INTERVAL as usize)
            < (FANOUT_ROWS + 2) * MAX_FANOUT_REVERSE_STEPS_PER_RULE,
        "fanout reverse exceeded its per-rule SQLite work bound: {fanout_callbacks} callbacks at {PROGRESS_INTERVAL} VM steps"
    );
}

fn counted_sqlite_progress<T>(
    connection: &Connection,
    interval: i32,
    operation: impl FnOnce() -> T,
) -> (T, usize) {
    let callbacks = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&callbacks);
    connection.progress_handler(
        interval,
        Some(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            false
        }),
    );
    let result = operation();
    connection.progress_handler(0, None::<fn() -> bool>);
    (result, callbacks.load(Ordering::Relaxed))
}

#[test]
fn cursors_reject_cross_repository_query_changes_and_ready_pointer_changes() {
    let connection = fixture();
    let service = ArchaeologyReadService::new(&connection);
    let first = list(service.execute(list_request(REPO, 1, None)).expect("first"));
    let cursor = first.page.next_cursor.expect("cursor");
    assert_eq!(
        service
            .execute(list_request(OTHER_REPO, 1, Some(cursor.clone())))
            .unwrap_err(),
        "Archaeology cursor is unavailable for this scope"
    );
    assert_eq!(
        service
            .execute(list_request(REPO, 2, Some(cursor.clone())))
            .unwrap_err(),
        "Archaeology cursor is unavailable for this scope"
    );

    connection
        .execute(
            "UPDATE archaeology_generations SET status='superseded' WHERE generation_id=?1",
            [GENERATION],
        )
        .expect("supersede old");
    seed_repository(
        &connection,
        "archaeology-repository:replacement",
        "archaeology-generation:replacement",
        &"d".repeat(40),
        true,
    );
    connection
        .execute(
            "UPDATE archaeology_generations SET repository_id=?1 WHERE generation_id=?2",
            params![REPO, "archaeology-generation:replacement"],
        )
        .expect("move replacement generation");
    connection
        .execute(
            "UPDATE archaeology_repositories SET ready_generation_id=?2,
             current_revision=?3,source_identity=?4 WHERE repository_id=?1",
            params![
                REPO,
                "archaeology-generation:replacement",
                "d".repeat(40),
                digest('a')
            ],
        )
        .expect("advance pointer");
    assert_eq!(
        service
            .execute(list_request(REPO, 1, Some(cursor)))
            .unwrap_err(),
        "Archaeology cursor is stale"
    );
}

#[test]
fn detail_reverse_relations_and_evidence_share_canonical_scope() {
    let connection = fixture();
    let service = ArchaeologyReadService::new(&connection);
    let stable_one = digest('1');
    let stable_two = digest('2');
    let detail = service
        .execute(ArchaeologyReadRequest::GetRule {
            repository_id: REPO.into(),
            rule_id: stable_one.clone(),
        })
        .expect("detail");
    let ArchaeologyReadResponse::GetRule(detail) = detail else {
        panic!("wrong response")
    };
    assert_eq!(
        detail.value.summary.lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );
    assert_eq!(detail.value.clauses.len(), 1);
    assert_eq!(detail.value.alias_rule_ids, [digest('3')]);
    assert_eq!(
        service
            .execute(ArchaeologyReadRequest::GetRule {
                repository_id: REPO.into(),
                rule_id: digest('3'),
            })
            .unwrap_err(),
        UNAVAILABLE
    );

    let reverse = service
        .execute(ArchaeologyReadRequest::ReverseSource {
            repository_id: REPO.into(),
            source: ArchaeologySourceSelector::Path {
                path_identity: "source-path:one".into(),
            },
            limit: Some(10),
            cursor: None,
        })
        .expect("source reverse");
    let ArchaeologyReadResponse::ReverseSource(reverse) = reverse else {
        panic!("wrong response")
    };
    assert_eq!(reverse.page.total_rows, 2);
    assert_eq!(
        reverse
            .items
            .iter()
            .map(|item| &item.rule_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([&stable_one, &stable_two])
    );

    let first_reverse_page = service
        .execute(ArchaeologyReadRequest::ReverseSource {
            repository_id: REPO.into(),
            source: ArchaeologySourceSelector::Path {
                path_identity: "source-path:one".into(),
            },
            limit: Some(1),
            cursor: None,
        })
        .expect("first reverse page");
    let ArchaeologyReadResponse::ReverseSource(first_reverse_page) = first_reverse_page else {
        panic!("wrong response")
    };
    assert_eq!(first_reverse_page.page.total_rows, 2);
    assert!(first_reverse_page.page.truncated);
    let second_reverse_page = service
        .execute(ArchaeologyReadRequest::ReverseSource {
            repository_id: REPO.into(),
            source: ArchaeologySourceSelector::Path {
                path_identity: "source-path:one".into(),
            },
            limit: Some(1),
            cursor: first_reverse_page.page.next_cursor,
        })
        .expect("second reverse page");
    let ArchaeologyReadResponse::ReverseSource(second_reverse_page) = second_reverse_page else {
        panic!("wrong response")
    };
    assert_eq!(second_reverse_page.page.total_rows, 2);
    assert_eq!(second_reverse_page.page.returned_rows, 1);
    assert!(!second_reverse_page.page.truncated);

    let relations = service
        .execute(ArchaeologyReadRequest::ListRelations {
            repository_id: REPO.into(),
            rule_id: stable_one.clone(),
            kinds: vec![ArchaeologyRelationKind::DependsOn],
            direction: ArchaeologyRelationDirection::Outgoing,
            limit: Some(10),
            cursor: None,
        })
        .expect("relations");
    let ArchaeologyReadResponse::ListRelations(relations) = relations else {
        panic!("wrong response")
    };
    assert_eq!(relations.items[0].rule_id, stable_two);

    let hydrated = service
        .execute(ArchaeologyReadRequest::HydrateEvidence {
            repository_id: REPO.into(),
            rule_id: stable_one,
            evidence: vec![
                ArchaeologyEvidenceSelector {
                    kind: ArchaeologyEvidenceKind::Span,
                    evidence_id: "span:one".into(),
                },
                ArchaeologyEvidenceSelector {
                    kind: ArchaeologyEvidenceKind::Fact,
                    evidence_id: "fact:one".into(),
                },
                ArchaeologyEvidenceSelector {
                    kind: ArchaeologyEvidenceKind::Span,
                    evidence_id: "span:one".into(),
                },
            ],
            limit: Some(10),
            cursor: None,
        })
        .expect("evidence");
    let ArchaeologyReadResponse::HydrateEvidence(hydrated) = hydrated else {
        panic!("wrong response")
    };
    assert_eq!(hydrated.items.len(), 2);
    assert_eq!(hydrated.page.total_rows, 2);
    assert!(matches!(
        &hydrated.items[0],
        ArchaeologyEvidence::Span { evidence_id, .. } if evidence_id == "span:one"
    ));
    assert!(matches!(
        &hydrated.items[1],
        ArchaeologyEvidence::Fact { evidence_id, .. } if evidence_id == "fact:one"
    ));
    assert_eq!(service.hydration_query_count(), 2);
    assert!(hydrated.items.iter().any(|item| matches!(
        item,
        ArchaeologyEvidence::Span { source, .. }
            if source.relative_path.as_deref() == Some("src/rules.cbl")
                && source.language == "cobol"
                && source.dialect.as_deref() == Some("fixed")
    )));
}

#[test]
fn reverse_source_plan_probes_every_evidence_lookup_by_exact_target() {
    let connection = fixture();
    let sql = format!(
        "{} SELECT COUNT(*) FROM matched",
        reverse_rule_cte("unit.path_identity=?")
    );
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare reverse query plan");
    let details = statement
        .query_map(params![GENERATION, "source-path:one"], |row| {
            row.get::<_, String>(3)
        })
        .expect("query reverse plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("read reverse plan");
    let evidence_lookups = details
        .iter()
        .filter(|detail| {
            detail.contains("idx_archaeology_evidence_reverse")
                && (detail.contains("SEARCH direct")
                    || detail.contains("SEARCH fact_span")
                    || detail.contains("SEARCH fact_link")
                    || detail.contains("SEARCH link"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        evidence_lookups.len(),
        5,
        "expected all five reverse-evidence probes: {details:?}"
    );
    for detail in evidence_lookups {
        assert!(
            detail.contains("idx_archaeology_evidence_reverse")
                && detail.contains(
                    "generation_key=? AND evidence_kind_code=? AND evidence_identity_key=? AND owner_kind_code=?"
                ),
            "reverse evidence lookup must probe the exact target instead of scanning the generation: {detail}"
        );
    }
}

#[test]
fn source_hydration_excludes_absolute_and_protected_paths_without_leaking_scope() {
    let connection = fixture();
    let request = || ArchaeologyReadRequest::HydrateEvidence {
        repository_id: REPO.into(),
        rule_id: digest('1'),
        evidence: vec![ArchaeologyEvidenceSelector {
            kind: ArchaeologyEvidenceKind::Span,
            evidence_id: "span:one".into(),
        }],
        limit: Some(1),
        cursor: None,
    };
    connection
        .execute(
            "UPDATE archaeology_source_units SET relative_path='/private/source.cbl'
             WHERE generation_id=?1",
            [GENERATION],
        )
        .expect("absolute corruption");
    assert_eq!(
        ArchaeologyReadService::new(&connection)
            .execute(request())
            .unwrap_err(),
        UNAVAILABLE
    );
    connection
        .execute(
            "UPDATE archaeology_source_units SET relative_path=NULL,classification='protected'
             WHERE generation_id=?1",
            [GENERATION],
        )
        .expect("protected source");
    assert_eq!(
        ArchaeologyReadService::new(&connection)
            .execute(request())
            .unwrap_err(),
        UNAVAILABLE
    );
}

#[test]
fn response_bytes_trim_large_pages_with_a_reconcilable_cursor() {
    let connection = fixture();
    for index in 0..80 {
        let occurrence = format!("occurrence:large:{index:03}");
        seed_rule(
            &connection,
            REPO,
            GENERATION,
            &occurrence,
            &format!("large:{index}"),
            &format!("Rule {index:03} {}", "x".repeat(14_000)),
            "candidate",
        );
    }
    let page = list(
        ArchaeologyReadService::new(&connection)
            .execute(list_request(REPO, 500, None))
            .expect("large bounded page"),
    );
    assert!(page.page.truncated);
    assert!(page.page.next_cursor.is_some());
    assert!(serialized_bytes(&page).expect("serialize") <= MAX_RESPONSE_BYTES);
    assert_eq!(page.page.total_rows, 82);
}
