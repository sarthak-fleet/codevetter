use super::*;
use crate::commands::business_rule_archaeology::adapter::{
    assert_no_duplicated_source_body, compose_captured_events, run_archaeology_adapter,
    ArchaeologyAdapterLimits, ArchaeologyAdapterOutcome, ArchaeologyAdapterOutput, CapturedEvents,
};
use crate::commands::business_rule_archaeology::contracts::{
    ArchaeologyCoverage, ArchaeologyRuleKind, ArchaeologySourceClassification,
    ArchaeologySourceSpan, ArchaeologySourceUnitIdentity,
};
use crate::commands::business_rule_archaeology::deterministic_rules::{
    cluster_evidence_compatible_rules, derive_evidence_packets, render_template_rules,
    ArchaeologyDeterministicLimits, ArchaeologyFactOrigin,
};
use crate::commands::business_rule_archaeology::inventory::{
    ArchaeologyIncludeCandidate, ArchaeologyInventoryUnit,
};
use crate::commands::business_rule_archaeology::{
    link_archaeology_facts, ArchaeologyLinkFact, ArchaeologyLinkLimits, ArchaeologyLinkUnit,
};
use crate::commands::structural_graph::types::stable_graph_id;
use sha2::{Digest, Sha256};

const FIXED: &[u8] = include_bytes!("fixtures/sources/cobol/fixed_claim.cbl");
const FREE: &[u8] = include_bytes!("fixtures/sources/cobol/free_route.cbl");
const COPYBOOK: &[u8] = include_bytes!("fixtures/sources/cobol/CLAIMREC.cpy");
const RECOVERY: &[u8] = include_bytes!("fixtures/sources/recovery/broken_claim.cbl");
const GENERATED: &[u8] = include_bytes!("fixtures/sources/generated/claim_listing.lst");
const CONFLICT: &[u8] = include_bytes!("fixtures/sources/conflict/override.cbl");
const REVISION: &str = "dddddddddddddddddddddddddddddddddddddddd";

#[test]
fn labeled_fixed_free_and_copybook_fixtures_have_exact_facts_and_lineage() {
    let fixed = run(FIXED, "cobol/fixed_claim.cbl", "fixed", false).unwrap();
    assert_no_duplicated_source_body(&fixed.events, FIXED);
    assert_eq!(fixed.outcome().metadata.dialect.as_deref(), Some("fixed"));
    assert_kinds(
        &fixed,
        &[
            ArchaeologyFactKind::Declaration,
            ArchaeologyFactKind::EntryPoint,
            ArchaeologyFactKind::Include,
            ArchaeologyFactKind::Unresolved,
            ArchaeologyFactKind::Predicate,
            ArchaeologyFactKind::Mutation,
        ],
    );
    let predicate = fact(&fixed, ArchaeologyFactKind::Predicate);
    assert_eq!(slice(&fixed, FIXED, predicate), b"CLAIM-AMOUNT > ZERO");
    let include = fact(&fixed, ArchaeologyFactKind::Include);
    assert_eq!(slice(&fixed, FIXED, include), b"COPY CLAIMREC");
    let include_span = fact_span(&fixed, include);
    assert_eq!(include_span.source_unit_id, "unit:cobol/fixed_claim.cbl");
    assert_eq!(coordinates(include_span), (99, 4, 12, 112, 4, 25));
    assert_eq!(fixed.outcome().metadata.lineage.len(), 1);
    let lineage = &fixed.outcome().metadata.lineage[0];
    assert_eq!(lineage.source_unit_id, include_span.source_unit_id);
    assert_eq!(lineage.evidence_span_id, include_span.span_id);
    assert!(lineage
        .detail
        .contains("unresolved cross-unit include target="));
    assert!(lineage.target_source_unit_id.is_none());
    assert!(fixed.outcome().metadata.regions.iter().any(|region| {
        region.kind == ArchaeologyAdapterRegionKind::Unsupported
            && region.reason.contains("not expanded")
    }));
    assert!(fixed
        .edges
        .iter()
        .any(|edge| edge.kind == ArchaeologyFactEdgeKind::Unresolved));

    let free = run(FREE, "cobol/free_route.cbl", "free", false).unwrap();
    assert_no_duplicated_source_body(&free.events, FREE);
    assert_eq!(free.outcome().metadata.dialect.as_deref(), Some("free"));
    assert_kinds(
        &free,
        &[
            ArchaeologyFactKind::Decision,
            ArchaeologyFactKind::Predicate,
            ArchaeologyFactKind::Mutation,
        ],
    );
    assert_eq!(
        free.facts
            .iter()
            .filter(|fact| fact.kind == ArchaeologyFactKind::Mutation)
            .count(),
        2
    );
    assert!(
        free.edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count()
            >= 4
    );

    let copybook = run(COPYBOOK, "cobol/CLAIMREC.cpy", "copybook", false).unwrap();
    assert_no_duplicated_source_body(&copybook.events, COPYBOOK);
    assert_eq!(
        copybook.outcome().metadata.dialect.as_deref(),
        Some("copybook")
    );
    assert_eq!(
        copybook
            .facts
            .iter()
            .filter(|fact| fact.kind == ArchaeologyFactKind::DataField)
            .count(),
        3
    );
    let condition = fact(&copybook, ArchaeologyFactKind::Constant);
    assert_eq!(condition.label, "CLAIM-IS-ELIGIBLE");
    assert_eq!(
        slice(&copybook, COPYBOOK, condition),
        b"88 CLAIM-IS-ELIGIBLE VALUE 'Y'"
    );
    let condition_span = fact_span(&copybook, condition);
    assert_eq!(condition_span.source_unit_id, "unit:cobol/CLAIMREC.cpy");
    assert_eq!(coordinates(condition_span), (111, 4, 14, 141, 4, 44));
    assert_ne!(include_span.source_unit_id, condition_span.source_unit_id);
    assert_ne!(include_span.span_id, condition_span.span_id);
}

#[test]
#[rustfmt::skip]
fn real_adapter_output_links_copybook_data_without_expansion_or_guessing() {
    let fixed = run(FIXED, "cobol/fixed_claim.cbl", "fixed", false).unwrap();
    let copy = run(COPYBOOK, "cobol/CLAIMREC.cpy", "copybook", false).unwrap();
    let units = [
        ArchaeologyLinkUnit { source_unit_id: "unit:cobol/fixed_claim.cbl", language: "cobol", dialect: Some("fixed"), relative_path: Some("cobol/fixed_claim.cbl"), lineage: &fixed.outcome().metadata.lineage },
        ArchaeologyLinkUnit { source_unit_id: "unit:cobol/CLAIMREC.cpy", language: "cobol", dialect: Some("copybook"), relative_path: Some("cobol/CLAIMREC.cpy"), lineage: &copy.outcome().metadata.lineage },
    ];
    let facts = fixed.facts.iter().map(|fact| ArchaeologyLinkFact { source_unit_id: units[0].source_unit_id, fact, evidence_spans: &fixed.spans })
        .chain(copy.facts.iter().map(|fact| ArchaeologyLinkFact { source_unit_id: units[1].source_unit_id, fact, evidence_spans: &copy.spans })).collect::<Vec<_>>();
    let edges = fixed.edges.iter().chain(&copy.edges).cloned().collect::<Vec<_>>();
    let patch = link_archaeology_facts("repository:fixture", REVISION, &units, &facts, &edges,
        &StructuralGraphCancellation::default(), ArchaeologyLinkLimits::default()).unwrap();
    let include = fact(&fixed, ArchaeologyFactKind::Include);
    let placeholder = edges.iter().find(|edge| edge.from_fact_id == include.fact_id && edge.kind == ArchaeologyFactEdgeKind::Unresolved).unwrap();
    assert_eq!(patch.lineage[0].target_source_unit_id.as_deref(), Some(units[1].source_unit_id));
    assert!(patch.remove_edge_ids.contains(&placeholder.edge_id) && patch.remove_fact_ids.contains(&placeholder.to_fact_id));
    let amount = copy.facts.iter().find(|fact| fact.kind == ArchaeologyFactKind::DataField && fact.label == "CLAIM-AMOUNT").unwrap();
    let eligible = copy.facts.iter().find(|fact| fact.kind == ArchaeologyFactKind::DataField && fact.label == "CLAIM-ELIGIBLE").unwrap();
    let predicate = fixed.facts.iter().find(|fact| fact.kind == ArchaeologyFactKind::Predicate).unwrap();
    let read = patch.upsert_edges.iter().find(|edge| edge.kind == ArchaeologyFactEdgeKind::Reads).unwrap();
    assert_eq!(read.to_fact_id, amount.fact_id);
    assert!(read.evidence_span_ids.iter().any(|id| fixed.spans.iter().any(|span| span.span_id == *id))
        && read.evidence_span_ids.iter().any(|id| copy.spans.iter().any(|span| span.span_id == *id)));
    assert_eq!(patch.upsert_edges.iter().filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Writes && edge.to_fact_id == eligible.fact_id).count(), 2);
    let packet_facts = fixed.facts.iter().chain(&copy.facts).cloned()
        .chain(patch.upsert_facts.iter().cloned()).collect::<Vec<_>>();
    let packet_edges = edges.iter().cloned().chain(patch.upsert_edges.iter().cloned()).collect::<Vec<_>>();
    let packets = derive_evidence_packets("repository:fixture", REVISION, &packet_facts, &packet_edges,
        &StructuralGraphCancellation::default(), ArchaeologyDeterministicLimits::default()).unwrap();
    let eligibility = packets.iter().find(|packet| packet.anchor_fact_id == predicate.fact_id).unwrap();
    assert_eq!(eligibility.kind, ArchaeologyRuleKind::Eligibility);
    assert!(eligibility.supporting_fact_ids.contains(&amount.fact_id)
        && eligibility.supporting_fact_ids.contains(&eligible.fact_id));
    assert!(eligibility.evidence_span_ids.iter().any(|id| fixed.spans.iter().any(|span| span.span_id == *id))
        && eligibility.evidence_span_ids.iter().any(|id| copy.spans.iter().any(|span| span.span_id == *id)));
}

#[test]
fn real_cobol_predicates_keep_operator_and_literal_semantics_through_clustering() {
    let parsed = run_statement(
        "IF AMOUNT > 0\n       END-IF.\n       IF AMOUNT < 0\n       END-IF.\n       IF AMOUNT > 100\n       END-IF.\n       IF LIMIT > 0\n       END-IF.\n       IF amount > 0\n       END-IF.",
    )
    .unwrap();
    let mut predicates = parsed
        .facts
        .iter()
        .filter(|fact| fact.kind == ArchaeologyFactKind::Predicate)
        .collect::<Vec<_>>();
    predicates.sort_by_key(|fact| fact_span(&parsed, fact).start.byte);
    assert_eq!(predicates.len(), 5);
    let expressions = predicates
        .iter()
        .map(|fact| {
            fact.attributes
                .iter()
                .find(|attribute| attribute.key == "semantic_expr")
                .map(|attribute| attribute.value.as_str())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(expressions[0], expressions[4]);
    assert_ne!(expressions[0], expressions[1]);
    assert_ne!(expressions[0], expressions[2]);
    assert_ne!(expressions[0], expressions[3]);
    assert!(expressions
        .iter()
        .all(|value| value.starts_with("v1:sha256:")
            && !value.contains("AMOUNT")
            && !value.contains('>')));
    let rhs = predicates
        .iter()
        .map(|fact| attribute_values(fact, "comparison_rhs_expr"))
        .collect::<Vec<_>>();
    assert_eq!(rhs[0], rhs[1]);
    assert_eq!(rhs[0], rhs[3]);
    assert_eq!(rhs[0], rhs[4]);
    assert_ne!(rhs[0], rhs[2]);
    assert!(rhs.iter().flatten().all(|value| {
        value.starts_with("v1:sha256:") && !value.contains("AMOUNT") && !value.contains("100")
    }));

    let cancellation = StructuralGraphCancellation::default();
    let packets = derive_evidence_packets(
        "repository:cobol-cluster",
        REVISION,
        &parsed.facts,
        &parsed.edges,
        &cancellation,
        Default::default(),
    )
    .unwrap();
    let rules = render_template_rules(
        "repository:cobol-cluster",
        "generation:cobol-cluster",
        REVISION,
        &packets,
        &parsed.facts,
        &parsed.edges,
        &ArchaeologyCoverage::default(),
        "parser:manifest",
        "algorithm:v1",
        &cancellation,
        Default::default(),
    )
    .unwrap();
    let duplicate_id = predicates[4].fact_id.as_str();
    let origins = parsed
        .facts
        .iter()
        .map(|fact| ArchaeologyFactOrigin {
            fact_id: fact.fact_id.clone(),
            source_unit_id: format!("unit:{}", fact.fact_id),
            path_identity: format!("path:{}", fact.fact_id),
            ranking_path_identity: stable_graph_id(
                "archaeology-ranking-path",
                &format!("src/{}.cbl", fact.fact_id),
            ),
            classification: if fact.fact_id == duplicate_id {
                ArchaeologySourceClassification::Generated
            } else {
                ArchaeologySourceClassification::Source
            },
        })
        .collect::<Vec<_>>();
    let clustered = cluster_evidence_compatible_rules(
        "repository:cobol-cluster",
        REVISION,
        &rules,
        &parsed.facts,
        &parsed.edges,
        &origins,
        &cancellation,
        Default::default(),
    )
    .unwrap();
    assert_eq!(clustered.len(), 5);
    assert_eq!(
        clustered
            .iter()
            .filter(|rule| rule.domain_ids == ["domain:other"])
            .count(),
        4
    );
    assert_eq!(
        clustered
            .iter()
            .filter(|rule| !rule.alias_rule_ids.is_empty())
            .count(),
        1
    );
}

#[test]
fn real_cobol_when_semantics_include_the_active_evaluate_subject() {
    let parsed = run_statement(
        "EVALUATE ROUTE-CODE\n       WHEN 1\n       END-EVALUATE.\n       EVALUATE STATUS-CODE\n       WHEN 1\n       END-EVALUATE.\n       EVALUATE ROUTE-CODE\n       WHEN 2\n       END-EVALUATE.",
    )
    .unwrap();
    let predicates = parsed
        .facts
        .iter()
        .filter(|fact| fact.label == "WHEN condition")
        .collect::<Vec<_>>();
    assert_eq!(predicates.len(), 3);
    let signatures = predicates
        .iter()
        .map(|fact| attribute_values(fact, "semantic_expr")[0])
        .collect::<Vec<_>>();
    assert_ne!(signatures[0], signatures[1]);
    assert_ne!(signatures[0], signatures[2]);
    assert_ne!(signatures[1], signatures[2]);
    assert!(predicates
        .iter()
        .all(|fact| attribute_values(fact, "semantic_context").is_empty()));
}

#[test]
fn labeled_recovery_generated_and_conflict_fixtures_fail_closed_by_region() {
    let recovery = run(RECOVERY, "recovery/broken_claim.cbl", "fixed", false).unwrap();
    let error = recovery
        .outcome()
        .metadata
        .regions
        .iter()
        .find(|region| region.kind == ArchaeologyAdapterRegionKind::Error)
        .expect("error region");
    let error_span = recovery
        .spans
        .iter()
        .find(|span| span.span_id == error.span_id)
        .unwrap();
    assert_eq!(
        &RECOVERY[error_span.start.byte as usize..error_span.end.byte as usize],
        b"       IF CLAIM-AMOUNT >"
    );
    assert!(recovery
        .facts
        .iter()
        .all(|fact| fact.span_ids.iter().all(|id| id != &error.span_id)));

    let generated = run(
        GENERATED,
        "generated/claim_listing.lst",
        "generated-listing",
        false,
    )
    .unwrap();
    assert!(generated.facts.is_empty());
    assert_eq!(generated.outcome().metadata.dialect, None);
    assert!(generated.outcome().metadata.coverage_reasons[0].contains("generated"));

    let ambiguous = run(FIXED, "ambiguous.cbl", "ambiguous", false).unwrap();
    assert!(ambiguous.facts.is_empty());
    assert_eq!(ambiguous.outcome().metadata.dialect, None);
    assert!(ambiguous.outcome().metadata.coverage_reasons[0].contains("positive"));

    let conflict = run(CONFLICT, "conflict/override.cbl", "fixed", false).unwrap();
    let predicate = fact(&conflict, ArchaeologyFactKind::Predicate);
    assert_eq!(
        slice(&conflict, CONFLICT, predicate),
        b"CLAIM-AMOUNT <= ZERO"
    );
}

#[test]
fn utf8_positions_cancellation_and_token_bound_are_exact() {
    let source = "       IF AMOUNT > 0\n           MOVE 'é' TO STATUS\n".as_bytes();
    let result = run(source, "unicode.cbl", "fixed", false).unwrap();
    let mutation = fact(&result, ArchaeologyFactKind::Mutation);
    let span = result
        .spans
        .iter()
        .find(|span| span.span_id == mutation.span_ids[0])
        .unwrap();
    assert_eq!((span.start.line, span.start.column), (2, 12));
    assert_eq!(
        slice(&result, source, mutation),
        "MOVE 'é' TO STATUS".as_bytes()
    );
    let crossing = run("00000é IF X = 1\n".as_bytes(), "column.cbl", "fixed", false).unwrap();
    assert!(crossing.outcome().metadata.regions.iter().any(|region| {
        region.kind == ArchaeologyAdapterRegionKind::Unsupported
            && region.reason.contains("indicator")
    }));

    let error = run(FIXED, "cobol/fixed_claim.cbl", "fixed", true).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");

    let at_bound = vec!["A"; super::super::legacy::MAX_LEGACY_TOKENS].join(" ");
    let line = super::super::legacy::lines(&at_bound, LegacyFormat::Free)
        .next()
        .unwrap();
    assert_eq!(
        super::super::legacy::tokens(&at_bound, line).unwrap().len(),
        super::super::legacy::MAX_LEGACY_TOKENS
    );
    let over_bound = format!("{at_bound} A");
    let line = super::super::legacy::lines(&over_bound, LegacyFormat::Free)
        .next()
        .unwrap();
    assert!(super::super::legacy::tokens(&over_bound, line).is_err());

    let fixed = format!("       MOVE 1 TO X{}IDENTIFICATION-AREA", " ".repeat(54));
    let line = super::super::legacy::lines(&fixed, LegacyFormat::Fixed)
        .next()
        .unwrap();
    let words = super::super::legacy::tokens(&fixed, line).unwrap();
    assert_eq!(words.last().unwrap().text(&fixed), "X");
}

#[test]
fn policy_constructs_have_three_labeled_positives_per_cobol_dialect() {
    let fixed_source = qualification_program(false);
    let free_source = qualification_program(true);
    for (dialect, source) in [
        ("fixed", fixed_source.as_bytes()),
        ("free", free_source.as_bytes()),
    ] {
        let result = run(
            source,
            &format!("qualification-{dialect}.cbl"),
            dialect,
            false,
        )
        .unwrap();
        for (label, kind, minimum) in [
            ("division", ArchaeologyFactKind::Declaration, 3),
            ("data-layout", ArchaeologyFactKind::DataField, 3),
            ("condition-name", ArchaeologyFactKind::Constant, 3),
            ("evaluate", ArchaeologyFactKind::Decision, 3),
            ("perform", ArchaeologyFactKind::ControlFlow, 3),
            ("calculation", ArchaeologyFactKind::Calculation, 3),
            ("mutation", ArchaeologyFactKind::Mutation, 3),
            ("call", ArchaeologyFactKind::Call, 3),
            ("io", ArchaeologyFactKind::InputOutput, 3),
        ] {
            assert!(count_kind(&result, kind) >= minimum, "{dialect}/{label}");
        }
        for (label, count) in [
            (
                "paragraph",
                count_attribute(&result, "declaration", "paragraph"),
            ),
            ("if", count_label(&result, "IF predicate")),
            ("embedded-sql", count_label(&result, "embedded SQL")),
            (
                "file-io",
                ["OPEN", "READ", "CLOSE"]
                    .iter()
                    .map(|verb| count_attribute(&result, "operation", verb))
                    .sum(),
            ),
        ] {
            assert!(count >= 3, "{dialect}/{label}={count}");
        }
        assert!(result.facts.iter().all(|fact| fact.span_ids.len() == 1
            && result
                .spans
                .iter()
                .any(|span| span.span_id == fact.span_ids[0])));
        assert!(
            result.outcome().metadata.regions.len() >= 3,
            "{dialect}/unsupported"
        );
    }

    let copybook = b"       01 REC-A PIC X.\n       COPY A.\n       01 REC-B PIC X.\n       COPY B.\n       01 REC-C PIC X.\n       COPY C.\n";
    let result = run(copybook, "qualification.cpy", "copybook", false).unwrap();
    assert!(count_kind(&result, ArchaeologyFactKind::DataField) >= 3);
    assert!(count_kind(&result, ArchaeologyFactKind::Include) >= 3);
    assert_eq!(result.outcome().metadata.lineage.len(), 3);
    assert!(result
        .outcome()
        .metadata
        .lineage
        .iter()
        .all(|lineage| lineage.target_source_unit_id.is_none()
            && lineage.detail.contains("unresolved")));
}

#[test]
fn exact_sql_transactions_are_normalized_without_commit_false_positives() {
    let source = b"       IDENTIFICATION DIVISION.\n       PROCEDURE DIVISION.\n       MAIN.\n       EXEC SQL COMMIT END-EXEC.\n       EXEC SQL ROLLBACK END-EXEC.\n       EXEC SQL\n       COMMIT\n       END-EXEC.\n       EXEC SQL SELECT COMMIT FROM AUDIT END-EXEC.\n       EXEC SQL COMMIT WORK END-EXEC.\n       CALL 'COMMIT'.\n       DISPLAY 'EXEC SQL COMMIT END-EXEC'.\n       COMMIT.\n";
    let result = run(source, "transactions.cbl", "fixed", false).unwrap();
    let again = run(source, "transactions.cbl", "fixed", false).unwrap();
    assert_eq!(
        result.facts, again.facts,
        "transaction identities must be stable"
    );
    let transactions = result
        .facts
        .iter()
        .filter(|fact| fact.kind == ArchaeologyFactKind::Transaction)
        .collect::<Vec<_>>();
    assert_eq!(
        transactions
            .iter()
            .map(|fact| fact.label.as_str())
            .collect::<Vec<_>>(),
        ["commit", "rollback", "commit"]
    );
    assert_eq!(
        slice(&result, source, transactions[0]),
        b"EXEC SQL COMMIT END-EXEC"
    );
    assert_eq!(
        slice(&result, source, transactions[1]),
        b"EXEC SQL ROLLBACK END-EXEC"
    );
    assert_eq!(
        slice(&result, source, transactions[2]),
        b"EXEC SQL\n       COMMIT\n       END-EXEC"
    );
    assert_eq!(
        count_label(&result, "embedded SQL"),
        2,
        "SQL containing COMMIT and COMMIT WORK remain I/O, not transactions"
    );
    assert_eq!(count_kind(&result, ArchaeologyFactKind::Call), 1);
    let units = [ArchaeologyLinkUnit {
        source_unit_id: "unit:transactions.cbl",
        language: "cobol",
        dialect: Some("fixed"),
        relative_path: Some("transactions.cbl"),
        lineage: &result.outcome().metadata.lineage,
    }];
    let facts = result
        .facts
        .iter()
        .map(|fact| ArchaeologyLinkFact {
            source_unit_id: units[0].source_unit_id,
            fact,
            evidence_spans: &result.spans,
        })
        .collect::<Vec<_>>();
    let patch = link_archaeology_facts(
        "repository:fixture",
        REVISION,
        &units,
        &facts,
        &result.edges,
        &StructuralGraphCancellation::default(),
        ArchaeologyLinkLimits::default(),
    )
    .unwrap();
    assert_eq!(
        patch
            .upsert_edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::CommitsTransaction)
            .count(),
        2
    );
    assert_eq!(
        patch
            .upsert_edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::RollsBackTransaction)
            .count(),
        1
    );
    assert!(patch
        .upsert_edges
        .iter()
        .filter(|edge| matches!(
            edge.kind,
            ArchaeologyFactEdgeKind::CommitsTransaction
                | ArchaeologyFactEdgeKind::RollsBackTransaction
        ))
        .all(|edge| edge.unresolved_reason.as_deref() == Some("reference target is unavailable")));
}

#[test]
#[rustfmt::skip]
fn exact_relationship_hints_are_repeated_and_bounded() {
    let program = run(b"       IDENTIFICATION DIVISION.\n       PROGRAM-ID. PAYLINK.\n", "program.cbl", "fixed", false).unwrap();
    assert_eq!(attribute_values(fact(&program, ArchaeologyFactKind::EntryPoint), "exported"), ["true"]);
    let result = run_statement("IF SOURCE-A > LIMIT-B\n       MOVE SOURCE-A TO DEST-A\n       END-IF.\n       EVALUATE ROUTE-CODE\n       WHEN LIMIT-B MOVE SOURCE-B TO DEST-B\n       END-EVALUATE.\n       COMPUTE TOTAL-C = SOURCE-A + LIMIT-B\n       ADD SOURCE-B TO TOTAL-C\n       DIVIDE LIMIT-B INTO TOTAL-C\n       CALL 'PAYASM'\n       PERFORM WORK-PARA\n       READ CLAIM-FILE\n       OPEN INPUT CLAIM-FILE\n       ACCEPT STATUS-X").unwrap();
    let predicate = result.facts.iter().find(|item| item.label == "IF predicate").unwrap();
    assert_eq!(attribute_values(predicate, "reads"), ["SOURCE-A", "LIMIT-B"]);
    let decision = result.facts.iter().find(|item| item.kind == ArchaeologyFactKind::Decision).unwrap();
    assert_eq!(attribute_values(decision, "reads"), ["ROUTE-CODE"]);
    let moves = result.facts.iter().filter(|item| item.label == "MOVE").collect::<Vec<_>>();
    assert_eq!(attribute_values(moves[0], "reads"), ["SOURCE-A"]);
    assert_eq!(attribute_values(moves[0], "writes"), ["DEST-A"]);
    let compute = result.facts.iter().find(|item| item.label == "COMPUTE").unwrap();
    assert_eq!(attribute_values(compute, "reads"), ["SOURCE-A", "LIMIT-B"]);
    assert_eq!(attribute_values(compute, "writes"), ["TOTAL-C"]);
    let add = result.facts.iter().find(|item| item.label == "ADD").unwrap();
    assert_eq!(attribute_values(add, "reads"), ["SOURCE-B", "TOTAL-C"]);
    assert_eq!(attribute_values(add, "writes"), ["TOTAL-C"]);
    let divide = result.facts.iter().find(|item| item.label == "DIVIDE").unwrap();
    assert_eq!(attribute_values(divide, "reads"), ["LIMIT-B", "TOTAL-C"]);
    assert_eq!(attribute_values(divide, "writes"), ["TOTAL-C"]);
    let call = result.facts.iter().find(|item| item.kind == ArchaeologyFactKind::Call).unwrap();
    assert_eq!(attribute_values(call, "target"), ["PAYASM"]);
    for target in ["READ", "OPEN", "ACCEPT"] {
        let io = result.facts.iter().find(|item| item.label == target).unwrap();
        assert_eq!(attribute_values(io, "target").len(), 1);
    }
}

#[test]
fn malformed_divisions_levels_identifiers_and_reserved_sentences_fail_closed() {
    for division in ["FOO", "IDENTIFICATION-EXTRA", "PROCEDURES"] {
        let result = run_statement(&format!("{division} DIVISION.")).unwrap();
        assert!(!result
            .facts
            .iter()
            .any(|fact| fact.label == format!("{division} DIVISION")));
        assert_error(&result);
    }
    for level in [0, 50, 65, 67, 76, 79, 87, 89, 99] {
        let result = run_statement(&format!("{level:02} FIELD-X PIC X.")).unwrap();
        assert_eq!(count_kind(&result, ArchaeologyFactKind::DataField), 0);
        assert_error(&result);
    }
    for statement in ["01 -BAD PIC X.", "01 BAD- PIC X.", "01 TO PIC X."] {
        let result = run_statement(statement).unwrap();
        assert_eq!(count_kind(&result, ArchaeologyFactKind::DataField), 0);
        assert_error(&result);
    }
    let valid = run_statement(
        "01 A PIC X.\n       49 B PIC X.\n       66 C RENAMES A.\n       77 D PIC X.\n       78 E VALUE 1.\n       88 F VALUE 1.",
    )
    .unwrap();
    assert_eq!(count_kind(&valid, ArchaeologyFactKind::DataField), 4);
    assert_eq!(count_kind(&valid, ArchaeologyFactKind::Constant), 2);

    let reserved =
        run_statement("GOBACK.\n       EXIT.\n       CONTINUE.\n       END-PERFORM.").unwrap();
    assert!(!reserved
        .facts
        .iter()
        .any(|fact| ["GOBACK", "EXIT", "CONTINUE", "END-PERFORM"].contains(&fact.label.as_str())));
}

#[test]
fn malformed_action_shapes_emit_regions_not_facts() {
    let cases = [
        ("MOVE X", ArchaeologyFactKind::Mutation),
        ("MOVE TO X", ArchaeologyFactKind::Mutation),
        ("SET X", ArchaeologyFactKind::Mutation),
        ("INITIALIZE", ArchaeologyFactKind::Mutation),
        ("COMPUTE X =", ArchaeologyFactKind::Calculation),
        ("ADD TO X", ArchaeologyFactKind::Calculation),
        ("SUBTRACT X", ArchaeologyFactKind::Calculation),
        ("MULTIPLY BY X", ArchaeologyFactKind::Calculation),
        ("DIVIDE X BY", ArchaeologyFactKind::Calculation),
        ("CALL", ArchaeologyFactKind::Call),
        ("CALL TO", ArchaeologyFactKind::Call),
        ("CALL 'X' GARBAGE", ArchaeologyFactKind::Call),
        ("PERFORM", ArchaeologyFactKind::ControlFlow),
        ("PERFORM UNTIL X", ArchaeologyFactKind::ControlFlow),
        (
            "PERFORM VARYING X FROM 1 BY 1",
            ArchaeologyFactKind::ControlFlow,
        ),
        ("PERFORM TARGET GARBAGE", ArchaeologyFactKind::ControlFlow),
        ("OPEN CLAIM-FILE", ArchaeologyFactKind::InputOutput),
        ("CLOSE", ArchaeologyFactKind::InputOutput),
        ("READ", ArchaeologyFactKind::InputOutput),
        ("READ CLAIM-FILE GARBAGE", ArchaeologyFactKind::InputOutput),
        ("WRITE", ArchaeologyFactKind::InputOutput),
        ("WRITE CLAIM-REC GARBAGE", ArchaeologyFactKind::InputOutput),
        ("DISPLAY", ArchaeologyFactKind::InputOutput),
        ("ACCEPT", ArchaeologyFactKind::InputOutput),
        ("SELECT F ASSIGN X", ArchaeologyFactKind::InputOutput),
        ("FD", ArchaeologyFactKind::InputOutput),
    ];
    for (statement, kind) in cases {
        let result = run_statement(statement).unwrap();
        assert_eq!(count_kind(&result, kind), 0, "{statement}");
        assert_error(&result);
    }
    #[rustfmt::skip]
    let grammar_cases = [
        ("COMPUTE X = +", ArchaeologyFactKind::Calculation), ("COMPUTE X = X +", ArchaeologyFactKind::Calculation),
        ("ADD , TO X", ArchaeologyFactKind::Calculation), ("MOVE , TO X", ArchaeologyFactKind::Mutation),
        ("OPEN INPUT ,", ArchaeologyFactKind::InputOutput), ("CLOSE ,", ArchaeologyFactKind::InputOutput),
        ("DISPLAY ,", ArchaeologyFactKind::InputOutput),
        ("PERFORM VARYING X BY 1 FROM 1 UNTIL X = 3", ArchaeologyFactKind::ControlFlow),
        ("PERFORM VARYING X FROM 1 BY 1 UNTIL X = 3 JUNK", ArchaeologyFactKind::ControlFlow),
    ];
    for (statement, kind) in grammar_cases {
        let result = run_statement(statement).unwrap();
        assert_eq!(count_kind(&result, kind), 0, "{statement}");
        assert_error(&result);
    }
    for (statement, kind) in [
        ("IF X = 1 GARBAGE", ArchaeologyFactKind::Predicate),
        ("EVALUATE X Y", ArchaeologyFactKind::Decision),
        ("WHEN X Y", ArchaeologyFactKind::Predicate),
        ("COPY TO", ArchaeologyFactKind::Include),
        ("88 FLAG VALUE .", ArchaeologyFactKind::Constant),
    ] {
        let result = run_statement(statement).unwrap();
        assert_eq!(count_kind(&result, kind), 0, "{statement}");
        assert_error(&result);
    }
}

#[test]
fn perform_else_period_continuation_and_action_shapes_are_bounded() {
    let result = run_statement(
        "PERFORM TARGET\n       PERFORM TARGET UNTIL X = 1\n       PERFORM UNTIL X = 1\n       PERFORM VARYING X FROM 1 BY 1 UNTIL X = 3\n       SET FLAG TO TRUE\n       INITIALIZE RECORD-X\n       COMPUTE X = X + 1\n       COMPUTE Y = 1\n       ADD 1 TO X\n       SUBTRACT 1 FROM X\n       MULTIPLY 2 BY X\n       DIVIDE 2 INTO X\n       CALL 'AUDIT'\n       CALL 'AUDIT' USING X\n       OPEN INPUT CLAIM-FILE\n       CLOSE CLAIM-FILE\n       READ CLAIM-FILE\n       READ CLAIM-FILE INTO CLAIM-REC\n       WRITE CLAIM-REC\n       WRITE CLAIM-REC FROM RECORD-X\n       REWRITE CLAIM-REC\n       DELETE CLAIM-FILE\n       START CLAIM-FILE\n       DISPLAY 'OK'\n       ACCEPT STATUS-X",
    )
    .unwrap();
    assert_eq!(count_kind(&result, ArchaeologyFactKind::ControlFlow), 4);
    assert_eq!(count_kind(&result, ArchaeologyFactKind::Calculation), 6);
    assert_eq!(count_kind(&result, ArchaeologyFactKind::Mutation), 2);
    assert_eq!(count_kind(&result, ArchaeologyFactKind::Call), 2);
    assert!(count_kind(&result, ArchaeologyFactKind::InputOutput) >= 9);
    let targets = result
        .facts
        .iter()
        .filter(|fact| fact.kind == ArchaeologyFactKind::ControlFlow)
        .flat_map(|fact| &fact.attributes)
        .filter(|attribute| attribute.key == "target")
        .map(|attribute| attribute.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(targets, ["TARGET", "TARGET", "inline", "inline"]);

    let branches =
        run_statement("IF X = 1\n       MOVE 1 TO Y\n       ELSE MOVE 2 TO Y.\n       MOVE 3 TO Y")
            .unwrap();
    assert_eq!(count_kind(&branches, ArchaeologyFactKind::Mutation), 3);
    assert_eq!(
        branches
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        2
    );

    let evaluate =
        run_statement("EVALUATE X\n       WHEN 1 MOVE 1 TO Y.\n       MOVE 2 TO Y").unwrap();
    assert_eq!(count_kind(&evaluate, ArchaeologyFactKind::Mutation), 2);
    assert_eq!(
        evaluate
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        2
    );

    let continuation = run(
        b"       IDENTIFICATION DIVISION.\n      -MOVE 1 TO X\n",
        "continuation.cbl",
        "fixed",
        false,
    )
    .unwrap();
    assert!(continuation
        .outcome()
        .metadata
        .regions
        .iter()
        .any(|region| {
            region.kind == ArchaeologyAdapterRegionKind::Unsupported
                && region.reason.contains("continuation")
        }));
}

#[test]
fn standalone_period_terminates_control_context_without_creating_an_empty_range() {
    let source = b">>SOURCE FORMAT FREE\nIDENTIFICATION DIVISION.\nPROCEDURE DIVISION.\nMAIN.\nIF X = 1\nMOVE 1 TO Y\n.\nMOVE 2 TO Y\n.\n";
    let result = run(source, "standalone-period.cbl", "free", false).unwrap();

    assert_eq!(count_kind(&result, ArchaeologyFactKind::Mutation), 2);
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        1
    );
}

fn run(source: &[u8], path: &str, dialect: &str, cancel: bool) -> Result<Collected, String> {
    let classification = if dialect == "generated-listing" {
        ArchaeologySourceClassification::Generated
    } else {
        ArchaeologySourceClassification::Source
    };
    let includes = String::from_utf8_lossy(source)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let target = line.trim().strip_prefix("COPY ")?.trim_end_matches('.');
            Some(ArchaeologyIncludeCandidate {
                kind: "copybook".into(),
                target: target.into(),
                line: index as u64 + 1,
            })
        })
        .collect();
    let unit = ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: format!("unit:{path}"),
            repository_id: "repository:fixture".into(),
            revision_sha: REVISION.into(),
            path_identity: format!("path:{path}"),
            relative_path: Some(path.into()),
            content_hash: Some(format!("{:x}", Sha256::digest(source))),
            hash_algorithm: Some("sha256".into()),
            change_identity: None,
        },
        classification,
        language: "cobol".into(),
        dialect: Some(dialect.into()),
        byte_count: source.len() as u64,
        line_count: source.iter().filter(|byte| **byte == b'\n').count() as u64,
        include_candidates: includes,
        coverage_reasons: vec![],
    };
    let cancellation = StructuralGraphCancellation::default();
    if cancel {
        cancellation.cancel_after_checks(4);
    }
    let mut output = Collected::default();
    match run_archaeology_adapter(
        &CobolAdapter::default(),
        ArchaeologyAdapterInput {
            unit: &unit,
            source,
        },
        &mut output,
        &cancellation,
        ArchaeologyAdapterLimits::default(),
    ) {
        Ok(outcome) => {
            output.outcome = Some(outcome);
            Ok(output)
        }
        Err(error) => Err(error),
    }
}

fn assert_kinds(result: &Collected, kinds: &[ArchaeologyFactKind]) {
    for kind in kinds {
        assert!(
            result.facts.iter().any(|fact| &fact.kind == kind),
            "missing {kind:?}"
        );
    }
}

fn count_kind(result: &Collected, kind: ArchaeologyFactKind) -> usize {
    result.facts.iter().filter(|fact| fact.kind == kind).count()
}

#[rustfmt::skip]
fn count_label(result: &Collected, label: &str) -> usize {
    result.facts.iter().filter(|fact| fact.label == label).count()
}

#[rustfmt::skip]
fn count_attribute(result: &Collected, key: &str, value: &str) -> usize {
    result.facts.iter().filter(|fact| fact.attributes.iter().any(|item| item.key == key && item.value == value)).count()
}

fn attribute_values<'a>(fact: &'a ArchaeologyFact, key: &str) -> Vec<&'a str> {
    fact.attributes
        .iter()
        .filter(|item| item.key == key)
        .map(|item| item.value.as_str())
        .collect()
}

fn assert_error(result: &Collected) {
    assert!(result
        .outcome()
        .metadata
        .regions
        .iter()
        .any(|region| region.kind == ArchaeologyAdapterRegionKind::Error));
}

fn run_statement(statement: &str) -> Result<Collected, String> {
    let source = format!(
        "       IDENTIFICATION DIVISION.\n       PROCEDURE DIVISION.\n       MAIN.\n       {}\n",
        statement.replace('\n', "\n       ")
    );
    run(source.as_bytes(), "statement.cbl", "fixed", false)
}

fn qualification_program(free: bool) -> String {
    format!(
        "{}       IDENTIFICATION DIVISION.\n       DATA DIVISION.\n       01 ITEM-A PIC 9.\n       88 ITEM-A-READY VALUE 1.\n       01 ITEM-B PIC 9.\n       88 ITEM-B-READY VALUE 1.\n       01 ITEM-C PIC 9.\n       88 ITEM-C-READY VALUE 1.\n       PROCEDURE DIVISION.\n       P-A.\n       IF ITEM-A = 1\n       MOVE 1 TO ITEM-A\n       COMPUTE ITEM-A = ITEM-A + 1\n       CALL 'A'\n       END-IF.\n       P-B.\n       IF ITEM-B = 1\n       MOVE 1 TO ITEM-B\n       COMPUTE ITEM-B = ITEM-B + 1\n       CALL 'B'\n       END-IF.\n       P-C.\n       IF ITEM-C = 1\n       MOVE 1 TO ITEM-C\n       COMPUTE ITEM-C = ITEM-C + 1\n       CALL 'C'\n       END-IF.\n       EVALUATE ITEM-A\n       WHEN 1 DISPLAY 'A'\n       END-EVALUATE.\n       EVALUATE ITEM-B\n       WHEN 1 DISPLAY 'B'\n       END-EVALUATE.\n       EVALUATE ITEM-C\n       WHEN 1 DISPLAY 'C'\n       END-EVALUATE.\n       PERFORM P-A\n       PERFORM P-B\n       PERFORM P-C\n       OPEN INPUT CLAIM-FILE\n       READ CLAIM-FILE\n       CLOSE CLAIM-FILE\n       DISPLAY 'A'\n       DISPLAY 'B'\n       DISPLAY 'C'\n       EXEC SQL SELECT A FROM T END-EXEC.\n       EXEC SQL SELECT B FROM T END-EXEC.\n       EXEC SQL SELECT C FROM T END-EXEC.\n       >>UNSUPPORTED A\n       >>UNSUPPORTED B\n       >>UNSUPPORTED C\n",
        if free { ">>SOURCE FORMAT FREE\n" } else { "" }
    )
}

fn fact(result: &Collected, kind: ArchaeologyFactKind) -> &ArchaeologyFact {
    result.facts.iter().find(|fact| fact.kind == kind).unwrap()
}

fn fact_span<'a>(result: &'a Collected, fact: &ArchaeologyFact) -> &'a ArchaeologySourceSpan {
    result
        .spans
        .iter()
        .find(|span| span.span_id == fact.span_ids[0])
        .unwrap()
}

#[rustfmt::skip]
fn coordinates(span: &ArchaeologySourceSpan) -> (u64, u64, u64, u64, u64, u64) {
    (span.start.byte, span.start.line, span.start.column, span.end.byte, span.end.line, span.end.column)
}

fn slice<'a>(result: &Collected, source: &'a [u8], fact: &ArchaeologyFact) -> &'a [u8] {
    let span = result
        .spans
        .iter()
        .find(|span| span.span_id == fact.span_ids[0])
        .unwrap();
    &source[span.start.byte as usize..span.end.byte as usize]
}

#[derive(Default, Debug)]
struct Collected {
    events: CapturedEvents,
    outcome: Option<ArchaeologyAdapterOutcome>,
}

impl Collected {
    fn outcome(&self) -> &ArchaeologyAdapterOutcome {
        self.outcome.as_ref().unwrap()
    }
}

compose_captured_events!(Collected, events);

#[rustfmt::skip]
impl ArchaeologyAdapterEvents for Collected {
    fn emit_span(&mut self, value: ArchaeologySourceSpan) -> Result<(), String> { self.events.emit_span(value) }
    fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> { self.events.emit_fact(value) }
    fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> { self.events.emit_edge(value) }
}

#[rustfmt::skip]
impl ArchaeologyAdapterOutput for Collected {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> { Ok(()) }
    fn commit_unit(&mut self, _: &ArchaeologyAdapterOutcome) -> Result<(), String> { Ok(()) }
    fn abort_unit(&mut self) -> Result<(), String> { self.events.clear(); Ok(()) }
}
