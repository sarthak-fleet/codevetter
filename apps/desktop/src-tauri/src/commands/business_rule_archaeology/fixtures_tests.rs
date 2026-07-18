use super::adapter::{semantic_expression, ArchaeologyAdapterLineage, ArchaeologyLineageKind};
use super::contracts::{
    ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyEvidencePacket,
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
    ArchaeologyPosition, ArchaeologyRuleClause, ArchaeologyRuleKind, ArchaeologyRuleLifecycle,
    ArchaeologyRulePacket, ArchaeologySourceClassification, ArchaeologySourceSpan,
    ArchaeologyTrust,
};
use super::deterministic_rules::{
    cluster_evidence_compatible_rules, derive_evidence_packets, render_template_rules,
    ArchaeologyDeterministicLimits, ArchaeologyFactOrigin,
};
use super::{
    link_archaeology_facts, ArchaeologyLinkFact, ArchaeologyLinkLimits, ArchaeologyLinkPatch,
    ArchaeologyLinkUnit,
};
use crate::commands::structural_graph::types::{stable_graph_id, StructuralGraphCancellation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("fixtures/expected.json.fixture");
const LINK_REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[rustfmt::skip]
mod linker_tests {
use super::*;

#[test]
fn linker_resolves_only_unique_compatible_include_lineage() {
    let mut fixture = LinkFixture::default();
    fixture.unit("main", "cobol", Some("fixed"), Some("src/main.cbl"));
    fixture.unit("copy", "cobol", Some("copybook"), Some("copy/CLAIMREC.cpy"));
    fixture.lineage("main", ArchaeologyLineageKind::Copybook, "include-span");
    fixture.fact("main", "include", ArchaeologyFactKind::Include, "CLAIMREC",
        &[("target", "CLAIMREC")]);
    fixture.fact("main", "include-gap", ArchaeologyFactKind::Unresolved, "gap", &[]);
    let old = unresolved_edge("include-gap-edge", "include", "include-gap");
    let unique = fixture.link(std::slice::from_ref(&old), ArchaeologyLinkLimits::default(), None).unwrap();
    assert_eq!(unique.lineage[0].target_source_unit_id.as_deref(), Some("copy"));
    assert!(unique.upsert_edges.is_empty(), "include resolution is lineage-only");
    assert_eq!(unique.remove_edge_ids, ["include-gap-edge"]);
    assert_eq!(unique.remove_fact_ids, ["include-gap"]);

    fixture.units[1].path = Some("copy/OTHER.cpy".into());
    let missing = fixture.link(std::slice::from_ref(&old), ArchaeologyLinkLimits::default(), None).unwrap();
    assert_eq!(missing.lineage[0].target_source_unit_id, None);
    assert!(missing.lineage[0].detail.contains("unavailable"));
    assert!(missing.remove_edge_ids.is_empty());

    fixture.units[1].path = Some("a/CLAIMREC.cpy".into());
    fixture.unit("copy-2", "cobol", Some("copybook"), Some("b/CLAIMREC.cpy"));
    let ambiguous = fixture.link(&[old], ArchaeologyLinkLimits::default(), None).unwrap();
    assert_eq!(ambiguous.lineage[0].target_source_unit_id, None);
    assert!(ambiguous.lineage[0].detail.contains("ambiguous"));
}

#[test]
fn linker_emits_typed_unique_edges_and_bounded_unresolved_references() {
    let mut fixture = LinkFixture::default();
    fixture.unit("a", "cobol", Some("fixed"), Some("src/a.cbl"));
    fixture.unit("b", "cobol", Some("fixed"), Some("src/b.cbl"));
    fixture.unit("js", "typescript", Some("typescript"), Some("src/x.ts"));
    fixture.fact("b", "service", ArchaeologyFactKind::EntryPoint, "SERVICE", &[]);
    fixture.fact("b", "amount", ArchaeologyFactKind::DataField, "AMOUNT", &[]);
    fixture.fact("b", "block", ArchaeologyFactKind::EntryPoint, "BLOCK", &[]);
    fixture.fact("b", "tx", ArchaeologyFactKind::Transaction, "TX", &[]);
    fixture.fact("js", "export", ArchaeologyFactKind::EntryPoint, "runJs", &[("exported", "true")]);
    fixture.fact("js", "private", ArchaeologyFactKind::EntryPoint, "privateJs", &[]);
    fixture.fact("b", "dup-1", ArchaeologyFactKind::EntryPoint, "DUP", &[]);
    fixture.fact("b", "dup-2", ArchaeologyFactKind::EntryPoint, "DUP", &[]);
    fixture.fact("a", "call", ArchaeologyFactKind::Call, "CALL", &[("target", "service")]);
    fixture.fact("a", "data", ArchaeologyFactKind::Mutation, "MOVE", &[("reads", "amount"), ("writes", "amount")]);
    fixture.fact("a", "branch", ArchaeologyFactKind::ControlFlow, "BRANCH", &[("target", "block")]);
    fixture.fact("a", "commit", ArchaeologyFactKind::Transaction, "COMMIT", &[("target", "tx"), ("operation", "commit")]);
    fixture.fact("a", "cross", ArchaeologyFactKind::Call, "CALL", &[("target", "runJs")]);
    fixture.fact("a", "wrong-case", ArchaeologyFactKind::Call, "CALL", &[("target", "RUNJS")]);
    fixture.fact("a", "private-call", ArchaeologyFactKind::Call, "CALL", &[("target", "privateJs")]);
    fixture.fact("a", "missing", ArchaeologyFactKind::Call, "CALL", &[("target", "ABSENT")]);
    fixture.fact("a", "ambiguous", ArchaeologyFactKind::Call, "CALL", &[("target", "DUP")]);
    fixture.fact("js", "case-sensitive", ArchaeologyFactKind::Mutation, "read",
        &[("reads", "Foo"), ("reads", "foo")]);
    fixture.fact("a", "old-placeholder", ArchaeologyFactKind::Unresolved, "old", &[]);
    let old = ArchaeologyFactEdge { edge_id: "old-edge".into(), from_fact_id: "call".into(),
        to_fact_id: "old-placeholder".into(), kind: ArchaeologyFactEdgeKind::Unresolved,
        trust: ArchaeologyTrust::Extracted, evidence_span_ids: vec!["call-span".into()],
        unresolved_reason: Some("old".into()) };
    let patch = fixture.link(std::slice::from_ref(&old), ArchaeologyLinkLimits::default(), None).unwrap();
    assert!(fixture.link(std::slice::from_ref(&old), ArchaeologyLinkLimits { max_candidates_per_reference: 1, ..Default::default() }, None).is_err());
    assert!(fixture.link(&[old], ArchaeologyLinkLimits { max_edges: 0, ..Default::default() }, None).is_err());
    for kind in [ArchaeologyFactEdgeKind::Calls, ArchaeologyFactEdgeKind::Reads,
        ArchaeologyFactEdgeKind::Writes, ArchaeologyFactEdgeKind::BranchesTo,
        ArchaeologyFactEdgeKind::CommitsTransaction] {
        assert!(patch.upsert_edges.iter().any(|edge| edge.kind == kind), "{kind:?}");
    }
    assert!(patch.upsert_edges.iter().any(|edge| edge.to_fact_id == "export"));
    assert!(!patch.upsert_edges.iter().any(|edge| edge.to_fact_id == "private"));
    assert_eq!(patch.upsert_edges.iter().filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Unresolved).count(), 6);
    assert_eq!(patch.upsert_edges.iter().filter(|edge| edge.from_fact_id == "case-sensitive").count(), 2);
    assert!(patch.upsert_facts.iter().any(|fact| fact.attributes.iter().any(|item| item.key == "candidate_count" && item.value == "2")));
    assert!(patch.upsert_facts.iter().all(|fact| fact.attributes.iter().all(|item| item.key != "candidate_fact_id")));
    assert!(patch.remove_edge_ids.contains(&"old-edge".into()));
    assert!(patch.remove_fact_ids.contains(&"old-placeholder".into()));
    assert!(patch.upsert_edges.iter().filter(|edge| edge.kind != ArchaeologyFactEdgeKind::Unresolved)
        .all(|edge| edge.trust == ArchaeologyTrust::Deterministic && edge.evidence_span_ids.len() == 2));
}

#[test]
fn linker_emits_only_exact_bounded_complementary_predicate_conflicts() {
    let mut fixture = LinkFixture::default();
    fixture.unit("positive-unit", "cobol", Some("fixed"), Some("src/positive.cbl"));
    fixture.unit("non-positive-unit", "cobol", Some("fixed"), Some("src/non-positive.cbl"));
    fixture.unit("different-bound-unit", "cobol", Some("fixed"), Some("src/different-bound.cbl"));
    let zero = semantic_expression("ZERO", true).unwrap();
    let hundred = semantic_expression("100", true).unwrap();
    fixture.fact("positive-unit", "positive", ArchaeologyFactKind::Predicate, "IF predicate",
        &[("operator", ">"), ("reads", "CLAIM-AMOUNT"), ("comparison_rhs_expr", &zero)]);
    fixture.fact("non-positive-unit", "non-positive", ArchaeologyFactKind::Predicate, "IF predicate",
        &[("operator", "<="), ("reads", "claim-amount"), ("comparison_rhs_expr", &zero)]);
    fixture.fact("different-bound-unit", "different-bound", ArchaeologyFactKind::Predicate, "IF predicate",
        &[("operator", "<="), ("reads", "CLAIM-AMOUNT"), ("comparison_rhs_expr", &hundred)]);

    let patch = fixture.link(&[], ArchaeologyLinkLimits::default(), None).unwrap();
    let contradictions = patch.upsert_edges.iter()
        .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Contradicts).collect::<Vec<_>>();
    assert_eq!(contradictions.len(), 1);
    assert_eq!(
        (&contradictions[0].from_fact_id, &contradictions[0].to_fact_id),
        (&"non-positive".to_string(), &"positive".to_string()),
    );
    assert_eq!(contradictions[0].evidence_span_ids, ["non-positive-span", "positive-span"]);
    assert!(fixture.link(&[], ArchaeologyLinkLimits { max_candidates_per_reference: 1,
        ..Default::default() }, None).unwrap_err().contains("candidate bound"));
}

#[test]
fn linker_is_order_independent_idempotent_and_cycle_safe() {
    let mut fixture = LinkFixture::default();
    fixture.unit("a-unit", "cobol", Some("fixed"), Some("src/A.cbl"));
    fixture.unit("b-unit", "cobol", Some("fixed"), Some("src/B.cbl"));
    fixture.lineage("a-unit", ArchaeologyLineageKind::Include, "a-span");
    fixture.lineage("b-unit", ArchaeologyLineageKind::Include, "b-span");
    fixture.fact("a-unit", "a", ArchaeologyFactKind::Include, "B", &[("target", "B")]);
    fixture.fact("b-unit", "b", ArchaeologyFactKind::Include, "A", &[("target", "A")]);
    let first = fixture.link(&[], ArchaeologyLinkLimits::default(), None).unwrap();
    assert_eq!(first, fixture.link(&[], ArchaeologyLinkLimits::default(), None).unwrap());
    fixture.facts.reverse(); fixture.units.reverse();
    let reversed = fixture.link(&[], ArchaeologyLinkLimits::default(), None).unwrap();
    assert_eq!(first, reversed);
    assert!(first.upsert_edges.is_empty());
    assert_eq!(first.lineage.len(), 2);
    assert!(first.lineage.iter().all(|lineage| lineage.detail.contains("direct cycle")));
}

#[test]
fn linker_fails_closed_on_bounds_cancellation_duplicates_and_private_input() {
    let mut fixture = LinkFixture::default();
    fixture.unit("unit", "cobol", Some("fixed"), Some("private/hidden.cbl"));
    fixture.fact("unit", "call", ArchaeologyFactKind::Call, "source body must stay private",
        &[("target", "SECRET-TARGET-123456")]);
    let patch = fixture.link(&[], ArchaeologyLinkLimits::default(), None).unwrap();
    let json = serde_json::to_string(&patch).unwrap();
    assert!(!json.contains("SECRET-TARGET") && !json.contains("hidden.cbl") && !json.contains("source body"));
    for limits in [ArchaeologyLinkLimits { max_units: 0, ..Default::default() },
        ArchaeologyLinkLimits { max_facts: 0, ..Default::default() },
        ArchaeologyLinkLimits { max_references: 0, ..Default::default() },
        ArchaeologyLinkLimits { max_output_edges: 0, ..Default::default() },
        ArchaeologyLinkLimits { max_input_bytes: 1, ..Default::default() },
        ArchaeologyLinkLimits { max_output_items: 0, ..Default::default() },
        ArchaeologyLinkLimits { max_output_bytes: 1, ..Default::default() }] {
        assert!(fixture.link(&[], limits, None).is_err());
    }
    fixture.facts.push(fixture.facts[0].clone());
    assert!(fixture.link(&[], Default::default(), None).unwrap_err().contains("duplicate fact"));
    fixture.facts.pop();
    let cancellation = StructuralGraphCancellation::default(); cancellation.cancel();
    assert!(fixture.link(&[], Default::default(), Some(&cancellation)).unwrap_err().contains("cancelled"));
    let cancellation = StructuralGraphCancellation::default(); cancellation.cancel_after_checks(3);
    assert!(fixture.link(&[], Default::default(), Some(&cancellation)).unwrap_err().contains("cancelled"));
}

#[rustfmt::skip]
mod packet_tests {
use super::*;

#[test]
fn deterministic_packets_cover_every_required_behavior_without_prose() {
    let facts = vec![
        packet_fact("validation", ArchaeologyFactKind::Predicate, "amount check", &[]),
        packet_fact("mutation", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "AMOUNT")]),
        packet_fact("calculation", ArchaeologyFactKind::Calculation, "COMPUTE", &[("writes", "TOTAL")]),
        packet_fact("eligibility", ArchaeologyFactKind::Predicate, "claim check", &[]),
        packet_fact("eligible-write", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "CLAIM-ELIGIBLE")]),
        packet_fact("entitlement", ArchaeologyFactKind::Predicate, "benefit check", &[]),
        packet_fact("entitled-write", ArchaeologyFactKind::Mutation, "SET", &[("writes", "MEMBER-ENTITLEMENT")]),
        packet_fact("routing", ArchaeologyFactKind::Decision, "EVALUATE", &[]),
        packet_fact("route-call", ArchaeologyFactKind::Call, "send", &[("target", "FAST-QUEUE")]),
        packet_fact("exception", ArchaeologyFactKind::ControlFlow, "branch", &[]),
        packet_fact("reject", ArchaeologyFactKind::EntryPoint, "reject_payment", &[]),
        packet_fact("lifecycle", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "PAYMENT-STATUS")]),
        packet_fact("transaction", ArchaeologyFactKind::Transaction, "commit", &[("operation", "commit")]),
        packet_fact("transaction-gap", ArchaeologyFactKind::Unresolved, "gap", &[]),
    ];
    let edges = vec![
        packet_edge("validation-control", "validation", "mutation", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("eligibility-control", "eligibility", "eligible-write", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("entitlement-control", "entitlement", "entitled-write", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("routing-control", "routing", "route-call", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("exception-branch", "exception", "reject", ArchaeologyFactEdgeKind::BranchesTo, None),
        packet_edge("transaction-link", "transaction", "transaction-gap", ArchaeologyFactEdgeKind::CommitsTransaction, Some("reference target is unavailable")),
    ];
    let packets = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &StructuralGraphCancellation::default(), ArchaeologyDeterministicLimits::default()).unwrap();
    let kinds = packets.iter().map(|packet| &packet.kind).collect::<Vec<_>>();
    for kind in [ArchaeologyRuleKind::Validation, ArchaeologyRuleKind::Calculation,
        ArchaeologyRuleKind::Eligibility, ArchaeologyRuleKind::Entitlement,
        ArchaeologyRuleKind::Routing, ArchaeologyRuleKind::Mutation,
        ArchaeologyRuleKind::Exception, ArchaeologyRuleKind::Lifecycle,
        ArchaeologyRuleKind::Transaction] {
        assert!(kinds.contains(&&kind), "missing {kind:?}");
    }
    let transaction = packets.iter().find(|packet| packet.kind == ArchaeologyRuleKind::Transaction).unwrap();
    assert_eq!(transaction.unresolved_fact_ids, ["transaction-gap"]);
    assert_eq!(transaction.unresolved_reasons, ["unavailable_reference"]);
    assert_eq!(transaction.confidence, ArchaeologyConfidence::Low);
    assert!(transaction.caveats.iter().any(|caveat| caveat.contains("unresolved")));
    assert!(packets.iter().filter(|packet| matches!(packet.kind,
        ArchaeologyRuleKind::Eligibility | ArchaeologyRuleKind::Entitlement | ArchaeologyRuleKind::Exception | ArchaeologyRuleKind::Lifecycle))
        .all(|packet| packet.confidence == ArchaeologyConfidence::Medium
            && packet.caveats == ["kind is identifier-derived and requires review"]));
}

#[test]
fn packets_are_order_independent_scoped_private_cancellable_and_bounded() {
    let mut facts = vec![
        packet_fact("predicate", ArchaeologyFactKind::Predicate, "password=not-retained", &[]),
        packet_fact("first", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "FIELD-A")]),
        packet_fact("second", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "FIELD-B")]),
    ];
    let mut edges = vec![
        packet_edge("a", "predicate", "first", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("b", "predicate", "second", ArchaeologyFactEdgeKind::Controls, None),
    ];
    let cancellation = StructuralGraphCancellation::default();
    let first = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, ArchaeologyDeterministicLimits::default()).unwrap();
    facts.reverse(); edges.reverse();
    let reversed = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, ArchaeologyDeterministicLimits::default()).unwrap();
    assert_eq!(first, reversed);
    assert!(!serde_json::to_string(&first).unwrap().contains("password"));
    let other_revision = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    assert_ne!(first[0].packet_id, derive_evidence_packets("repository:packets", other_revision,
        &facts, &edges, &cancellation, Default::default()).unwrap()[0].packet_id);
    let truncated = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, ArchaeologyDeterministicLimits { max_facts_per_packet: 1, max_edges_per_packet: 0, ..Default::default() }).unwrap();
    assert!(truncated.iter().any(|packet| packet.caveats.iter().any(|value| value.contains("truncated"))
        && packet.confidence == ArchaeologyConfidence::Low));
    for limits in [ArchaeologyDeterministicLimits { max_facts: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_edges: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_packets: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_facts_per_packet: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_examined_edges_per_packet: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_input_bytes: 1, ..Default::default() },
        ArchaeologyDeterministicLimits { max_spans_per_packet: 0, ..Default::default() },
        ArchaeologyDeterministicLimits { max_output_bytes: 1, ..Default::default() }] {
        assert!(derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
            &cancellation, limits).is_err());
    }
    let cancelled = StructuralGraphCancellation::default(); cancelled.cancel_after_checks(2);
    assert!(derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancelled, Default::default()).unwrap_err().contains("cancelled"));
    assert!(derive_evidence_packets("repository\0packets", LINK_REVISION, &facts, &edges,
        &cancellation, Default::default()).is_err());
    assert!(derive_evidence_packets("repository:packets", "not-a-revision", &facts, &edges,
        &cancellation, Default::default()).is_err());
    facts.push(facts[0].clone());
    assert!(derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, Default::default()).unwrap_err().contains("unique cited facts"));
}

#[test]
fn contradictions_are_terminal_and_dense_reverse_fanout_is_not_scanned() {
    let facts = vec![
        packet_fact("anchor", ArchaeologyFactKind::Predicate, "amount check", &[]),
        packet_fact("child", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "AMOUNT")]),
        packet_fact("contrary", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "CLAIM-ELIGIBLE")]),
        packet_fact("descendant", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "PAYMENT-STATUS")]),
    ];
    let edges = vec![
        packet_edge("a-control", "anchor", "child", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("b-contradiction", "contrary", "child", ArchaeologyFactEdgeKind::Contradicts, None),
        packet_edge("c-descendant", "contrary", "descendant", ArchaeologyFactEdgeKind::Controls, None),
    ];
    let packets = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &StructuralGraphCancellation::default(), Default::default()).unwrap();
    let packet = packets.iter().find(|packet| packet.anchor_fact_id == "anchor").unwrap();
    assert_eq!(packet.kind, ArchaeologyRuleKind::Validation,
        "contradicting identifiers must not classify the supported rule");
    assert_eq!(packet.contradicting_fact_ids, ["contrary"]);
    assert!(!packet.supporting_fact_ids.contains(&"contrary".into())
        && !packet.supporting_fact_ids.contains(&"descendant".into()));
    assert_eq!(packet.confidence, ArchaeologyConfidence::Low);
    assert!(packet.caveats.iter().any(|value| value.contains("contradicting")));

    let order_facts = vec![
        packet_fact("anchor", ArchaeologyFactKind::Predicate, "amount check", &[]),
        packet_fact("a-contrary", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "CLAIM-ELIGIBLE")]),
        packet_fact("z-child", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "AMOUNT")]),
        packet_fact("descendant", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "PAYMENT-STATUS")]),
    ];
    let order_edges = vec![
        packet_edge("a", "anchor", "a-contrary", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("b", "anchor", "z-child", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("c", "a-contrary", "descendant", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("d", "a-contrary", "z-child", ArchaeologyFactEdgeKind::Contradicts, None),
    ];
    let ordered = derive_evidence_packets("repository:packets", LINK_REVISION, &order_facts, &order_edges,
        &StructuralGraphCancellation::default(), Default::default()).unwrap();
    let ordered = ordered.iter().find(|packet| packet.anchor_fact_id == "anchor").unwrap();
    assert_eq!(ordered.kind, ArchaeologyRuleKind::Validation);
    assert_eq!(ordered.contradicting_fact_ids, ["z-child"]);
    assert!(ordered.supporting_fact_ids.contains(&"a-contrary".into())
        && !ordered.supporting_fact_ids.contains(&"descendant".into())
        && !ordered.relationship_ids.contains(&"c".into()));

    let chain_facts = vec![
        packet_fact("anchor", ArchaeologyFactKind::Predicate, "amount check", &[]),
        packet_fact("a", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "A")]),
        packet_fact("b", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "B")]),
        packet_fact("c", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "C")]),
    ];
    let chain_edges = vec![
        packet_edge("control-a", "anchor", "a", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("control-b", "anchor", "b", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("control-c", "anchor", "c", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("contradict-ab", "a", "b", ArchaeologyFactEdgeKind::Contradicts, None),
        packet_edge("contradict-bc", "b", "c", ArchaeologyFactEdgeKind::Contradicts, None),
    ];
    let chain = derive_evidence_packets("repository:packets", LINK_REVISION, &chain_facts, &chain_edges,
        &StructuralGraphCancellation::default(), Default::default()).unwrap();
    let chain = chain.iter().find(|packet| packet.anchor_fact_id == "anchor").unwrap();
    assert_eq!(chain.contradicting_fact_ids, ["b"]);
    assert_eq!(chain.supporting_fact_ids, ["a", "anchor", "c"]);

    let mut dense_facts = vec![packet_fact("shared", ArchaeologyFactKind::DataField, "SHARED", &[])];
    let mut dense_edges = Vec::new();
    for index in 0..128 {
        let id = format!("writer-{index:03}");
        dense_facts.push(packet_fact(&id, ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "SHARED")]));
        dense_edges.push(packet_edge(&format!("write-{index:03}"), &id, "shared", ArchaeologyFactEdgeKind::Writes, None));
    }
    let dense = derive_evidence_packets("repository:packets", LINK_REVISION, &dense_facts, &dense_edges,
        &StructuralGraphCancellation::default(), ArchaeologyDeterministicLimits {
            max_examined_edges_per_packet: 1, ..Default::default()
        }).unwrap();
    assert_eq!(dense.len(), 128);
    assert!(dense.iter().all(|packet| packet.relationship_ids.len() == 1
        && packet.caveats.iter().all(|value| !value.contains("truncated"))));

    let bounded = derive_evidence_packets("repository:packets", LINK_REVISION, &facts,
        &[packet_edge("a", "anchor", "child", ArchaeologyFactEdgeKind::Controls, None),
          packet_edge("b", "anchor", "contrary", ArchaeologyFactEdgeKind::Controls, None),
          packet_edge("c", "anchor", "descendant", ArchaeologyFactEdgeKind::Controls, None)],
        &StructuralGraphCancellation::default(), ArchaeologyDeterministicLimits {
            max_examined_edges_per_packet: 2, ..Default::default()
        }).unwrap();
    let bounded = bounded.iter().find(|packet| packet.anchor_fact_id == "anchor").unwrap();
    assert_eq!(bounded.relationship_ids.len(), 2);
    assert!(bounded.caveats.iter().any(|value| value.contains("truncated")));
}

#[test]
fn every_deterministic_rule_gate_accepts_lowercase_sha1_and_sha256_only() {
    let facts = vec![
        packet_fact("predicate", ArchaeologyFactKind::Predicate, "positive amount", &[]),
        packet_fact("mutation", ArchaeologyFactKind::Mutation, "schedule", &[("writes", "PAYMENT")]),
    ];
    let edges = vec![packet_edge("controls", "predicate", "mutation",
        ArchaeologyFactEdgeKind::Controls, None)];
    let origins = facts.iter().map(|fact| ArchaeologyFactOrigin {
        fact_id: fact.fact_id.clone(), source_unit_id: format!("unit:{}", fact.fact_id),
        path_identity: format!("path:{}", fact.fact_id),
        ranking_path_identity: stable_graph_id(
            "archaeology-ranking-path",
            &format!("src/{}.cbl", fact.fact_id),
        ),
        classification: ArchaeologySourceClassification::Source,
    }).collect::<Vec<_>>();
    let cancellation = StructuralGraphCancellation::default();
    for revision in ["a".repeat(40), "b".repeat(64)] {
        let packets = derive_evidence_packets("repository:packets", &revision, &facts, &edges,
            &cancellation, Default::default()).expect("derive revision");
        let rules = render_template_rules("repository:packets", "generation:packets", &revision,
            &packets, &facts, &edges, &Default::default(), "parser:manifest", "algorithm:v1",
            &cancellation, Default::default()).expect("render revision");
        assert!(rules.iter().all(|rule| rule.revision_sha == revision));
        assert!(cluster_evidence_compatible_rules("repository:packets", &revision, &rules,
            &facts, &edges, &origins, &cancellation, Default::default()).is_ok());
    }
    let packets = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, Default::default()).unwrap();
    let rules = render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &facts, &edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, Default::default()).unwrap();
    for revision in ["a".repeat(39), "b".repeat(63), "A".repeat(40), "B".repeat(64)] {
        assert!(derive_evidence_packets("repository:packets", &revision, &facts, &edges,
            &cancellation, Default::default()).is_err());
        assert!(render_template_rules("repository:packets", "generation:packets", &revision,
            &packets, &facts, &edges, &Default::default(), "parser:manifest", "algorithm:v1",
            &cancellation, Default::default()).is_err());
        assert!(cluster_evidence_compatible_rules("repository:packets", &revision, &rules,
            &facts, &edges, &origins, &cancellation, Default::default()).is_err());
    }
}

#[test]
fn template_rules_are_useful_atomic_exact_and_zero_model() {
    let facts = vec![
        packet_fact("predicate", ArchaeologyFactKind::Predicate, "amount above zero", &[]),
        packet_fact("mutation", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "CLAIM-ELIGIBLE")]),
        packet_fact("field", ArchaeologyFactKind::DataField, "CLAIM-ELIGIBLE", &[]),
        packet_fact("contrary", ArchaeologyFactKind::Predicate, "amount at or below zero", &[]),
        packet_fact("transaction", ArchaeologyFactKind::Transaction, "commit", &[("operation", "commit")]),
        packet_fact("gap", ArchaeologyFactKind::Unresolved, "gap", &[]),
    ];
    let edges = vec![
        packet_edge("control", "predicate", "mutation", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("write", "mutation", "field", ArchaeologyFactEdgeKind::Writes, None),
        packet_edge("conflict", "contrary", "predicate", ArchaeologyFactEdgeKind::Contradicts, None),
        packet_edge("commit", "transaction", "gap", ArchaeologyFactEdgeKind::CommitsTransaction,
            Some("reference target is unavailable")),
    ];
    let cancellation = StructuralGraphCancellation::default();
    let limits = ArchaeologyDeterministicLimits::default();
    let packets = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, limits).unwrap();
    let rules = render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &facts, &edges, &ArchaeologyCoverage::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).unwrap();
    assert_eq!(rules.len(), packets.len());
    assert!(rules.iter().all(|rule| rule.trust == ArchaeologyTrust::Deterministic
        && rule.lifecycle == ArchaeologyRuleLifecycle::Candidate && rule.synthesis_identity.is_none()
        && rule.clauses.iter().all(|clause| clause.validate().is_ok())));
    let validation = rules.iter().find(|rule| rule.kind == ArchaeologyRuleKind::Eligibility).unwrap();
    assert!(validation.title.contains("amount above zero"));
    assert!(validation.clauses.iter().any(|clause| clause.text.contains("controls"))
        && validation.clauses.iter().any(|clause| clause.text.contains("writes")));
    let contradiction = validation
        .clauses
        .iter()
        .find(|clause| clause.contradicting_fact_ids == ["contrary"])
        .unwrap();
    assert!(contradiction
        .caveats
        .iter()
        .any(|value| value.contains("contradicting")));
    let transaction = rules.iter().find(|rule| rule.kind == ArchaeologyRuleKind::Transaction).unwrap();
    assert!(transaction.clauses.iter().any(|clause| clause.text.contains("unresolved transaction relationship")
        && clause.confidence == ArchaeologyConfidence::Low
        && clause.caveats == ["relationship target is unresolved"]));
    let encoded = serde_json::to_string(&rules).unwrap().to_ascii_lowercase();
    for unsupported in ["should", "quality", "intent", "correct implementation", "business policy requires"] {
        assert!(!encoded.contains(unsupported), "unsupported template claim: {unsupported}");
    }
    let known_facts = facts.iter().map(|fact| fact.fact_id.as_str()).collect::<BTreeSet<_>>();
    let known_spans = facts.iter().flat_map(|fact| fact.span_ids.iter().map(String::as_str)).collect::<BTreeSet<_>>();
    assert!(rules.iter().flat_map(|rule| &rule.clauses).all(|clause|
        clause.supporting_fact_ids.iter().chain(&clause.contradicting_fact_ids).all(|id| known_facts.contains(id.as_str()))
        && clause.evidence_span_ids.iter().all(|id| known_spans.contains(id.as_str()))));
}

#[test]
fn template_rules_reject_drift_secrets_oversize_and_cancellation() {
    let facts = vec![
        packet_fact("predicate", ArchaeologyFactKind::Predicate, "password=do-not-render", &[]),
        packet_fact("mutation", ArchaeologyFactKind::Mutation, "/private/source.cbl", &[("writes", "FIELD")]),
    ];
    let edges = vec![packet_edge("control", "predicate", "mutation", ArchaeologyFactEdgeKind::Controls, None)];
    let cancellation = StructuralGraphCancellation::default();
    let limits = ArchaeologyDeterministicLimits::default();
    let packets = derive_evidence_packets("repository:packets", LINK_REVISION, &facts, &edges,
        &cancellation, limits).unwrap();
    let render = |packets: &[ArchaeologyEvidencePacket], coverage: &ArchaeologyCoverage,
        cancellation: &StructuralGraphCancellation, limits| render_template_rules(
            "repository:packets", "generation:packets", LINK_REVISION, packets, &facts, &edges,
            coverage, "parser:manifest", "algorithm:v1", cancellation, limits);
    let rules = render(&packets, &ArchaeologyCoverage::default(), &cancellation, limits).unwrap();
    let encoded = serde_json::to_string(&rules).unwrap();
    assert!(!encoded.contains("password") && !encoded.contains("private/source"));

    let mut drifted = packets.clone(); drifted[0].evidence_span_ids.push("unknown-span".into());
    assert!(render(&drifted, &Default::default(), &cancellation, limits).is_err());
    let mut rogue = packets.clone(); rogue[0].caveats.push("read /Users/person/.env".into());
    assert!(render(&rogue, &Default::default(), &cancellation, limits).is_err());
    let secret_coverage = ArchaeologyCoverage { reasons: vec!["password=secret-value-123456".into()], ..Default::default() };
    assert!(render(&packets, &secret_coverage, &cancellation, limits).is_err());
    for bounded in [ArchaeologyDeterministicLimits { max_clauses_per_rule: 1, ..limits },
        ArchaeologyDeterministicLimits { max_clause_text_bytes: 1, ..limits },
        ArchaeologyDeterministicLimits { max_rule_output_bytes: 1, ..limits }] {
        assert!(render(&packets, &Default::default(), &cancellation, bounded).is_err());
    }
    let cancelled = StructuralGraphCancellation::default(); cancelled.cancel();
    assert!(render(&packets, &Default::default(), &cancelled, limits).unwrap_err().contains("cancelled"));
    let first = render(&packets, &Default::default(), &cancellation, limits).unwrap();
    let mut reversed = packets.clone(); reversed.reverse();
    assert_eq!(first, render(&reversed, &Default::default(), &cancellation, limits).unwrap());
    let mut reordered = packets.clone();
    for packet in &mut reordered {
        packet.supporting_fact_ids.reverse();
        packet.contradicting_fact_ids.reverse();
        packet.relationship_ids.reverse();
        packet.evidence_span_ids.reverse();
        packet.caveats.reverse();
    }
    assert_eq!(first, render(&reordered, &Default::default(), &cancellation, limits).unwrap());

    let mut false_kind = packets.clone(); false_kind[0].kind = ArchaeologyRuleKind::Other;
    assert!(render(&false_kind, &Default::default(), &cancellation, limits).is_err());
    let mut false_confidence = packets.clone(); false_confidence[0].confidence = ArchaeologyConfidence::Unavailable;
    assert!(render(&false_confidence, &Default::default(), &cancellation, limits).is_err());
    let mut duplicate_edge = packets.clone();
    let edge = duplicate_edge[0].relationship_ids[0].clone(); duplicate_edge[0].relationship_ids.push(edge);
    assert!(render(&duplicate_edge, &Default::default(), &cancellation, limits).is_err());

    let mut changed_identity = packets.clone();
    changed_identity[0].packet_id = "different-safe-packet-id".into();
    assert!(render(&changed_identity, &Default::default(), &cancellation, limits).is_err());

    let mut untrusted_facts = facts.clone();
    untrusted_facts[0].trust = ArchaeologyTrust::ModelSynthesized;
    assert!(derive_evidence_packets("repository:packets", LINK_REVISION, &untrusted_facts, &edges,
        &cancellation, limits).is_err());
    assert!(render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &untrusted_facts, &edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).is_err());
    let mut untrusted_edges = edges.clone();
    untrusted_edges[0].trust = ArchaeologyTrust::Unknown;
    assert!(render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &facts, &untrusted_edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).is_err());

    let mut unrelated_facts = facts.clone();
    unrelated_facts.push(packet_fact("other", ArchaeologyFactKind::DataField, "OTHER", &[]));
    let mut unrelated_edges = edges.clone();
    unrelated_edges[0].evidence_span_ids = vec!["other-span".into()];
    assert!(render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &unrelated_facts, &unrelated_edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).is_err());

    let mut incomplete_edges = edges.clone();
    incomplete_edges[0].evidence_span_ids = vec!["predicate-span".into()];
    assert!(render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &facts, &incomplete_edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).is_err());

    let mut reversed_edges = edges.clone();
    for edge in &mut reversed_edges {
        edge.evidence_span_ids.reverse();
        edge.evidence_span_ids.push(edge.evidence_span_ids[0].clone());
    }
    assert_eq!(first, render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
        &packets, &facts, &reversed_edges, &Default::default(), "parser:manifest", "algorithm:v1",
        &cancellation, limits).unwrap());

    for bounded in [ArchaeologyDeterministicLimits { max_facts: 1, ..limits },
        ArchaeologyDeterministicLimits { max_edges: 0, ..limits },
        ArchaeologyDeterministicLimits { max_input_bytes: 1, ..limits }] {
        assert!(render(&packets, &Default::default(), &cancellation, bounded).is_err());
    }
    for identity in ["/private/repository", "password=do-not-copy", "repo\\private"] {
        assert!(render_template_rules(identity, "generation:packets", LINK_REVISION, &packets,
            &facts, &edges, &Default::default(), "parser:manifest", "algorithm:v1", &cancellation,
            limits).is_err());
        assert!(render_template_rules("repository:packets", "generation:packets", LINK_REVISION,
            &packets, &facts, &edges, &Default::default(), identity, "algorithm:v1", &cancellation,
            limits).is_err());
    }
}

#[test]
fn rule_clustering_prefers_source_preserves_members_and_is_reorder_stable() {
    let facts = vec![
        packet_fact("source-fact", ArchaeologyFactKind::Predicate, "Amount > zero", &[("symbol", "AMOUNT")]),
        packet_fact("generated-fact", ArchaeologyFactKind::Predicate, "amount > ZERO", &[("symbol", "amount")]),
        packet_fact("other-fact", ArchaeologyFactKind::Predicate, "Amount is unavailable", &[("symbol", "AMOUNT")]),
        packet_fact("inverse-fact", ArchaeologyFactKind::Predicate, "Amount < zero", &[("symbol", "AMOUNT")]),
    ];
    let rules = vec![
        cluster_rule("rule:z-source", "Source wording", ArchaeologyRuleKind::Validation, &["source-fact"], &[]),
        cluster_rule("rule:a-generated", "Different generated wording", ArchaeologyRuleKind::Validation, &["generated-fact"], &[]),
        cluster_rule("rule:b-other", "Source wording", ArchaeologyRuleKind::Validation, &["other-fact"], &[]),
        cluster_rule("rule:c-inverse", "Inverse wording", ArchaeologyRuleKind::Validation, &["inverse-fact"], &[]),
    ];
    let origins = vec![
        cluster_origin("source-fact", ArchaeologySourceClassification::Source, "path:z-source"),
        cluster_origin("generated-fact", ArchaeologySourceClassification::Generated, "path:a-generated"),
        cluster_origin("other-fact", ArchaeologySourceClassification::Source, "path:b-other"),
        cluster_origin("inverse-fact", ArchaeologySourceClassification::Source, "path:c-inverse"),
    ];
    let cluster = |rules: &[ArchaeologyRulePacket], facts: &[ArchaeologyFact], origins: &[ArchaeologyFactOrigin]| {
        cluster_evidence_compatible_rules("repository:packets", LINK_REVISION, rules, facts, &[], origins,
            &StructuralGraphCancellation::default(), Default::default()).unwrap()
    };
    let first = cluster(&rules, &facts, &origins);
    let source = first.iter().find(|rule| rule.rule_id == "rule:z-source").unwrap();
    let generated = first.iter().find(|rule| rule.rule_id == "rule:a-generated").unwrap();
    let other = first.iter().find(|rule| rule.rule_id == "rule:b-other").unwrap();
    let inverse = first.iter().find(|rule| rule.rule_id == "rule:c-inverse").unwrap();
    assert_eq!(source.domain_ids, ["domain:other"]);
    assert!(source.alias_rule_ids.is_empty());
    assert_eq!(generated.alias_rule_ids, ["rule:z-source"]);
    assert!(generated.domain_ids.is_empty());
    assert_eq!(other.domain_ids, ["domain:other"]);
    assert!(other.alias_rule_ids.is_empty(), "equal prose must not override distinct evidence");
    assert!(inverse.alias_rule_ids.is_empty(), "opposite predicates must not become aliases");
    assert_eq!(first.iter().filter(|rule| rule.domain_ids == ["domain:other"]).count(), 3);
    assert_eq!(source.clauses[0].supporting_fact_ids, ["source-fact"]);
    assert_eq!(generated.clauses[0].supporting_fact_ids, ["generated-fact"]);

    let mut reordered_rules = rules.clone(); reordered_rules.reverse();
    let mut reordered_facts = facts.clone(); reordered_facts.reverse();
    let mut reordered_origins = origins.clone(); reordered_origins.reverse();
    assert_eq!(first, cluster(&reordered_rules, &reordered_facts, &reordered_origins));
}

#[test]
fn rule_clustering_canonicalizes_clause_order() {
    let facts = vec![
        packet_fact("fact:a", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:b", ArchaeologyFactKind::Predicate, "B", &[]),
    ];
    let mut rule = cluster_rule(
        "rule:ordered",
        "Ordered rule",
        ArchaeologyRuleKind::Validation,
        &["fact:a"],
        &[],
    );
    rule.clauses[0].text = "Zeta clause".into();
    rule.clauses.push(ArchaeologyRuleClause {
        clause_id: "clause:alpha".into(),
        text: "Alpha clause".into(),
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::High,
        supporting_fact_ids: vec!["fact:b".into()],
        contradicting_fact_ids: vec![],
        evidence_span_ids: vec!["fact:b-span".into()],
        caveats: vec![],
    });
    let origins = vec![
        cluster_origin(
            "fact:a",
            ArchaeologySourceClassification::Source,
            "path:a",
        ),
        cluster_origin(
            "fact:b",
            ArchaeologySourceClassification::Source,
            "path:b",
        ),
    ];
    let cluster = |rule: ArchaeologyRulePacket| {
        cluster_evidence_compatible_rules(
            "repository:packets",
            LINK_REVISION,
            &[rule],
            &facts,
            &[],
            &origins,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap()
    };

    let first = cluster(rule.clone());
    rule.clauses.reverse();
    let second = cluster(rule);

    assert_eq!(first, second);
    assert_eq!(
        first[0]
            .clauses
            .iter()
            .map(|clause| clause.text.as_str())
            .collect::<Vec<_>>(),
        ["Alpha clause", "Zeta clause"]
    );
}

#[test]
fn rule_clustering_primary_is_invariant_to_repository_scoped_identities() {
    let facts = vec![
        packet_fact("fact:left", ArchaeologyFactKind::Predicate, "Amount > zero", &[]),
        packet_fact("fact:right", ArchaeologyFactKind::Predicate, "amount > ZERO", &[]),
    ];
    let rules = vec![
        cluster_rule("rule:z", "Left wording", ArchaeologyRuleKind::Validation, &["fact:left"], &[]),
        cluster_rule("rule:a", "Right wording", ArchaeologyRuleKind::Validation, &["fact:right"], &[]),
    ];
    let ranking_left = stable_graph_id(
        "archaeology-ranking-path",
        "src/route.s\0start=91\0end=94",
    );
    let ranking_right = stable_graph_id(
        "archaeology-ranking-path",
        "src/route.s\0start=129\0end=132",
    );
    let origins = vec![
        ArchaeologyFactOrigin {
            fact_id: "fact:left".into(), source_unit_id: "unit:opaque-left-one".into(),
            path_identity: "path:opaque-left-one".into(),
            ranking_path_identity: ranking_left.clone(),
            classification: ArchaeologySourceClassification::Source,
        },
        ArchaeologyFactOrigin {
            fact_id: "fact:right".into(), source_unit_id: "unit:opaque-right-one".into(),
            path_identity: "path:opaque-right-one".into(),
            ranking_path_identity: ranking_right.clone(),
            classification: ArchaeologySourceClassification::Source,
        },
    ];
    let project = |clustered: &[ArchaeologyRulePacket]| {
        let titles = clustered.iter().map(|rule| (rule.rule_id.as_str(), rule.title.as_str()))
            .collect::<BTreeMap<_, _>>();
        clustered.iter().map(|rule| (
            rule.title.clone(),
            rule.domain_ids == ["domain:other"],
            rule.alias_rule_ids.iter().map(|id| titles[id.as_str()].to_string()).collect::<Vec<_>>(),
        )).collect::<BTreeSet<_>>()
    };
    let first = cluster_evidence_compatible_rules(
        "repository:packets", LINK_REVISION, &rules, &facts, &[], &origins,
        &StructuralGraphCancellation::default(), Default::default(),
    ).unwrap();

    let mut second_rules = rules.clone();
    for rule in &mut second_rules {
        rule.repository_id = "repository:two".into();
        rule.generation_id = "generation:two".into();
        rule.revision_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    }
    second_rules[0].rule_id = "rule:a-other".into();
    second_rules[1].rule_id = "rule:z-other".into();
    let second_origins = vec![
        ArchaeologyFactOrigin {
            fact_id: "fact:left".into(), source_unit_id: "unit:opaque-left-two".into(),
            path_identity: "path:opaque-left-two".into(), ranking_path_identity: ranking_left,
            classification: ArchaeologySourceClassification::Source,
        },
        ArchaeologyFactOrigin {
            fact_id: "fact:right".into(), source_unit_id: "unit:opaque-right-two".into(),
            path_identity: "path:opaque-right-two".into(), ranking_path_identity: ranking_right,
            classification: ArchaeologySourceClassification::Source,
        },
    ];
    let second = cluster_evidence_compatible_rules(
        "repository:two", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", &second_rules, &facts, &[],
        &second_origins, &StructuralGraphCancellation::default(), Default::default(),
    ).unwrap();
    assert_eq!(project(&first), project(&second));
}

#[test]
fn rule_clustering_reconciles_prose_only_stable_identity_duplicates() {
    let facts = vec![
        packet_fact("fact:a", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:b", ArchaeologyFactKind::Predicate, "B", &[]),
        packet_fact("fact:c", ArchaeologyFactKind::Predicate, "C", &[]),
    ];
    let mut combined = cluster_rule(
        "rule:a-combined",
        "Combined A and B",
        ArchaeologyRuleKind::Validation,
        &["fact:a", "fact:b"],
        &["fact:c"],
    );
    combined.clauses[0].clause_id = "clause:combined".into();
    let mut split = cluster_rule(
        "rule:z-split",
        "Split A",
        ArchaeologyRuleKind::Validation,
        &["fact:a", "fact:b"],
        &["fact:c"],
    );
    split.clauses[0].supporting_fact_ids = vec!["fact:a".into()];
    split.clauses[0].evidence_span_ids = vec!["fact:a-span".into(), "fact:c-span".into()];
    split.clauses.push(ArchaeologyRuleClause {
        clause_id: "clause:split-b".into(),
        text: "Split B".into(),
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::High,
        supporting_fact_ids: vec!["fact:b".into()],
        contradicting_fact_ids: vec![],
        evidence_span_ids: vec!["fact:b-span".into()],
        caveats: vec![],
    });
    let conflict_rule = cluster_rule(
        "rule:c",
        "C",
        ArchaeologyRuleKind::Validation,
        &["fact:c"],
        &[],
    );
    let origins = vec![
        cluster_origin(
            "fact:a",
            ArchaeologySourceClassification::Source,
            "path:a",
        ),
        cluster_origin(
            "fact:b",
            ArchaeologySourceClassification::Source,
            "path:b",
        ),
        cluster_origin(
            "fact:c",
            ArchaeologySourceClassification::Source,
            "path:c",
        ),
    ];
    let clustered = cluster_evidence_compatible_rules(
        "repository:packets",
        LINK_REVISION,
        &[combined.clone(), split.clone(), conflict_rule.clone()],
        &facts,
        &[],
        &origins,
        &StructuralGraphCancellation::default(),
        Default::default(),
    )
    .unwrap();

    assert_eq!(clustered.len(), 2);
    let canonical = clustered
        .iter()
        .find(|rule| rule.rule_id == "rule:a-combined")
        .unwrap();
    let conflict = clustered
        .iter()
        .find(|rule| rule.rule_id == "rule:c")
        .unwrap();
    assert_eq!(canonical.clauses.len(), 3);
    assert_eq!(canonical.conflict_rule_ids, ["rule:c"]);
    assert_eq!(conflict.conflict_rule_ids, ["rule:a-combined"]);
    assert!(canonical.clauses.iter().any(|clause| {
        clause.supporting_fact_ids == ["fact:a", "fact:b"]
            && clause.contradicting_fact_ids == ["fact:c"]
            && clause.evidence_span_ids
                == ["fact:a-span", "fact:b-span", "fact:c-span"]
    }));

    let mut opaque_combined = combined;
    opaque_combined.rule_id = "rule:z-combined".into();
    let mut opaque_split = split;
    opaque_split.rule_id = "rule:a-split".into();
    let mut opaque_conflict = conflict_rule;
    opaque_conflict.rule_id = "rule:q-conflict".into();
    for rule in [&mut opaque_combined, &mut opaque_split, &mut opaque_conflict] {
        rule.repository_id = "repository:other".into();
        rule.generation_id = "generation:other".into();
        rule.revision_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    }
    let mut opaque_origins = origins;
    for (index, origin) in opaque_origins.iter_mut().enumerate() {
        origin.source_unit_id = format!("unit:opaque:{index}");
        origin.path_identity = format!("path:opaque:{index}");
    }
    let opaque = cluster_evidence_compatible_rules(
        "repository:other", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        &[opaque_combined, opaque_split, opaque_conflict], &facts, &[], &opaque_origins,
        &StructuralGraphCancellation::default(), Default::default(),
    ).unwrap();
    let canonical = opaque.iter().find(|rule| rule.title == "Combined A and B").unwrap();
    assert_eq!(canonical.rule_id, "rule:z-combined");
    assert_eq!(canonical.clauses.len(), 3);
    let canonical_text = |rule: &ArchaeologyRulePacket| {
        rule
            .clauses
            .iter()
            .map(|clause| clause.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(canonical_text(canonical), canonical_text(
        clustered.iter().find(|rule| rule.title == "Combined A and B").unwrap()
    ));
}

#[test]
fn rule_clustering_reconciles_distinct_occurrences_with_one_stable_semantics() {
    let facts = vec![
        packet_fact("fact:a-one", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:a-two", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:b-one", ArchaeologyFactKind::Mutation, "B", &[]),
        packet_fact("fact:b-two", ArchaeologyFactKind::Mutation, "B", &[]),
    ];
    let combined = cluster_rule(
        "rule:combined",
        "Combined behavior",
        ArchaeologyRuleKind::Routing,
        &["fact:a-one", "fact:b-one"],
        &[],
    );
    let mut split = cluster_rule(
        "rule:split",
        "A behavior",
        ArchaeologyRuleKind::Routing,
        &["fact:a-two"],
        &[],
    );
    split.clauses.push(ArchaeologyRuleClause {
        clause_id: "clause:split-b".into(),
        text: "B behavior".into(),
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::High,
        supporting_fact_ids: vec!["fact:b-two".into()],
        contradicting_fact_ids: vec![],
        evidence_span_ids: vec!["fact:b-two-span".into()],
        caveats: vec![],
    });
    let origins = facts
        .iter()
        .map(|fact| {
            cluster_origin(
                &fact.fact_id,
                ArchaeologySourceClassification::Source,
                &format!("path:{}", fact.fact_id),
            )
        })
        .collect::<Vec<_>>();

    let clustered = cluster_evidence_compatible_rules(
        "repository:packets",
        LINK_REVISION,
        &[combined, split],
        &facts,
        &[],
        &origins,
        &StructuralGraphCancellation::default(),
        Default::default(),
    )
    .unwrap();

    assert_eq!(clustered.len(), 1);
    let cited = clustered[0]
        .clauses
        .iter()
        .flat_map(|clause| clause.supporting_fact_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        cited,
        BTreeSet::from(["fact:a-one", "fact:a-two", "fact:b-one", "fact:b-two"])
    );
}

#[test]
fn rule_clustering_marks_generated_only_members_and_uses_opaque_path_rank() {
    let facts = vec![
        packet_fact("generated-a", ArchaeologyFactKind::Mutation, "MOVE", &[("writes", "STATUS")]),
        packet_fact("vendor-z", ArchaeologyFactKind::Mutation, "move", &[("writes", "status")]),
    ];
    let rules = vec![
        cluster_rule("rule:a", "generated", ArchaeologyRuleKind::Mutation, &["generated-a"], &[]),
        cluster_rule("rule:z", "vendor", ArchaeologyRuleKind::Mutation, &["vendor-z"], &[]),
    ];
    let origins = vec![
        cluster_origin("generated-a", ArchaeologySourceClassification::Generated, "path:z"),
        cluster_origin("vendor-z", ArchaeologySourceClassification::Vendor, "path:a"),
    ];
    let clustered = cluster_evidence_compatible_rules("repository:packets", LINK_REVISION, &rules,
        &facts, &[], &origins, &StructuralGraphCancellation::default(), Default::default()).unwrap();
    let primary = clustered.iter().find(|rule| rule.rule_id == "rule:z").unwrap();
    let alias = clustered.iter().find(|rule| rule.rule_id == "rule:a").unwrap();
    assert_eq!(primary.domain_ids, ["domain:other"]);
    assert_eq!(alias.alias_rule_ids, ["rule:z"]);
    assert!(clustered.iter().all(|rule| rule.confidence == ArchaeologyConfidence::Low
        && rule.clauses.iter().all(|clause| clause.confidence == ArchaeologyConfidence::Low)
        && rule.clauses[0].caveats[0] == "cluster contains only generated or vendor evidence"));
}

#[test]
fn rule_clustering_emits_only_explicit_symmetric_primary_conflicts() {
    let facts = vec![
        packet_fact("fact:a", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:generated-a", ArchaeologyFactKind::Predicate, "a", &[]),
        packet_fact("fact:b", ArchaeologyFactKind::Predicate, "B", &[]),
        packet_fact("fact:b-copy", ArchaeologyFactKind::Predicate, "b", &[]),
    ];
    let rules = vec![
        cluster_rule("rule:a", "A conflicts with B", ArchaeologyRuleKind::Validation, &["fact:a"], &["fact:b-copy"]),
        cluster_rule("rule:generated-a", "generated A conflict", ArchaeologyRuleKind::Validation,
            &["fact:generated-a"], &["fact:b"]),
        cluster_rule("rule:b", "B", ArchaeologyRuleKind::Validation, &["fact:b"], &[]),
    ];
    let origins = vec![
        cluster_origin("fact:a", ArchaeologySourceClassification::Source, "path:a"),
        cluster_origin("fact:generated-a", ArchaeologySourceClassification::Generated, "path:generated-a"),
        cluster_origin("fact:b", ArchaeologySourceClassification::Source, "path:b"),
        cluster_origin("fact:b-copy", ArchaeologySourceClassification::Source, "path:b-copy"),
    ];
    let clustered = cluster_evidence_compatible_rules("repository:packets", LINK_REVISION, &rules,
        &facts, &[], &origins, &StructuralGraphCancellation::default(), Default::default()).unwrap();
    let source = clustered.iter().find(|rule| rule.rule_id == "rule:a").unwrap();
    let alias = clustered.iter().find(|rule| rule.rule_id == "rule:generated-a").unwrap();
    let other = clustered.iter().find(|rule| rule.rule_id == "rule:b").unwrap();
    assert_eq!(source.conflict_rule_ids, ["rule:b"]);
    assert_eq!(other.conflict_rule_ids, ["rule:a"]);
    assert_eq!(alias.alias_rule_ids, ["rule:a"]);
    assert!(alias.conflict_rule_ids.is_empty() && alias.domain_ids.is_empty());
    assert!(source.domain_ids == ["domain:other"] && other.domain_ids == ["domain:other"]);
}

#[test]
fn rule_clustering_rejects_bounds_cancellation_and_private_or_empty_evidence() {
    let facts = vec![
        packet_fact("fact:a", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:b", ArchaeologyFactKind::Predicate, "a", &[]),
        packet_fact("fact:c", ArchaeologyFactKind::Predicate, "A", &[]),
        packet_fact("fact:d", ArchaeologyFactKind::Predicate, "D", &[]),
    ];
    let rules = vec![
        cluster_rule("rule:a", "A", ArchaeologyRuleKind::Validation, &["fact:a"], &[]),
        cluster_rule("rule:b", "A alias", ArchaeologyRuleKind::Validation, &["fact:b"], &[]),
        cluster_rule("rule:c", "A alias two", ArchaeologyRuleKind::Validation, &["fact:c"], &[]),
        cluster_rule("rule:d", "D", ArchaeologyRuleKind::Validation, &["fact:d"], &[]),
    ];
    let origins = vec![
        cluster_origin("fact:a", ArchaeologySourceClassification::Source, "path:a"),
        cluster_origin("fact:b", ArchaeologySourceClassification::Generated, "path:b"),
        cluster_origin("fact:c", ArchaeologySourceClassification::Source, "path:c"),
        cluster_origin("fact:d", ArchaeologySourceClassification::Source, "path:d"),
    ];
    let run = |rules: &[ArchaeologyRulePacket], facts: &[ArchaeologyFact], origins: &[ArchaeologyFactOrigin],
        cancellation: &StructuralGraphCancellation, limits| cluster_evidence_compatible_rules(
            "repository:packets", LINK_REVISION, rules, facts, &[], origins, cancellation, limits);
    let limits = ArchaeologyDeterministicLimits::default();
    for bounded in [
        ArchaeologyDeterministicLimits { max_facts: 3, ..limits },
        ArchaeologyDeterministicLimits { max_input_bytes: 1, ..limits },
        ArchaeologyDeterministicLimits { max_cluster_members: 1, ..limits },
        ArchaeologyDeterministicLimits { max_cluster_relations: 1, ..limits },
        ArchaeologyDeterministicLimits { max_cluster_domains: 1, ..limits },
        ArchaeologyDeterministicLimits { max_cluster_output_bytes: 1, ..limits },
    ] {
        assert!(run(&rules, &facts, &origins, &Default::default(), bounded).is_err());
    }
    let dense_edges = vec![
        packet_edge("dense:a-b", "fact:a", "fact:b", ArchaeologyFactEdgeKind::Controls, None),
        packet_edge("dense:a-c", "fact:a", "fact:c", ArchaeologyFactEdgeKind::Controls, None),
    ];
    assert!(cluster_evidence_compatible_rules("repository:packets", LINK_REVISION, &rules,
        &facts, &dense_edges, &origins, &Default::default(), ArchaeologyDeterministicLimits {
            max_examined_edges_per_packet: 1, ..limits
        }).is_err());
    let mut too_many_clauses = rules.clone();
    let extra_clause = too_many_clauses[0].clauses[0].clone();
    too_many_clauses[0].clauses.push(extra_clause);
    assert!(run(&too_many_clauses, &facts, &origins, &Default::default(),
        ArchaeologyDeterministicLimits { max_clauses_per_rule: 1, ..limits }).is_err());
    let mut too_many_facts = rules.clone();
    too_many_facts[0].clauses[0].supporting_fact_ids.push("fact:b".into());
    too_many_facts[0].clauses[0].evidence_span_ids.push("fact:b-span".into());
    assert!(run(&too_many_facts, &facts, &origins, &Default::default(),
        ArchaeologyDeterministicLimits { max_facts_per_packet: 1, ..limits }).is_err());
    assert!(run(&too_many_facts, &facts, &origins, &Default::default(),
        ArchaeologyDeterministicLimits { max_spans_per_packet: 1, ..limits }).is_err());
    let cancelled = StructuralGraphCancellation::default(); cancelled.cancel();
    assert!(run(&rules, &facts, &origins, &cancelled, limits).unwrap_err().contains("cancelled"));
    let mid_key = StructuralGraphCancellation::default(); mid_key.cancel_after_checks(10);
    assert!(run(&rules, &facts, &origins, &mid_key, limits).unwrap_err().contains("cancelled"));

    for classification in [ArchaeologySourceClassification::Protected,
        ArchaeologySourceClassification::Opaque, ArchaeologySourceClassification::Unavailable] {
        let mut private = origins.clone(); private[0].classification = classification;
        assert!(run(&rules, &facts, &private, &Default::default(), limits).is_err());
    }
    let mut private = origins.clone(); private[0].path_identity = "/private/source".into();
    assert!(run(&rules, &facts, &private, &Default::default(), limits).is_err());
    let mut secret = facts.clone(); secret[0].label = "password=do-not-cluster".into();
    assert!(run(&rules, &secret, &origins, &Default::default(), limits).is_err());
    let mut missing_semantics = facts.clone();
    missing_semantics[0].attributes.retain(|attribute| attribute.key != "semantic_expr");
    assert!(run(&rules, &missing_semantics, &origins, &Default::default(), limits).is_err());
    let mut malformed_semantics = facts.clone();
    malformed_semantics[0].attributes.iter_mut().find(|attribute| attribute.key == "semantic_expr").unwrap().value = format!("v1:sha256:{}", "A".repeat(64));
    assert!(run(&rules, &malformed_semantics, &origins, &Default::default(), limits).is_err());
    let mut duplicate_semantics = facts.clone();
    let semantic = duplicate_semantics[0].attributes.iter().find(|attribute| attribute.key == "semantic_expr").unwrap().clone();
    duplicate_semantics[0].attributes.push(semantic);
    assert!(run(&rules, &duplicate_semantics, &origins, &Default::default(), limits).is_err());
    let mut business_identifier = facts.clone(); business_identifier[0].label = "CREDENTIALS".into();
    assert!(run(&rules, &business_identifier, &origins, &Default::default(), limits).is_ok());
    let mut private_rules = rules.clone(); private_rules[0].title = "/private/source".into();
    assert!(run(&private_rules, &facts, &origins, &Default::default(), limits).is_err());
    let mut cross_generation = rules.clone(); cross_generation[0].generation_id = "generation:other".into();
    assert!(run(&cross_generation, &facts, &origins, &Default::default(), limits).is_err());
    let mut preclustered = rules.clone(); preclustered[0].domain_ids = vec!["domain:other".into()];
    assert!(run(&preclustered, &facts, &origins, &Default::default(), limits).is_err());
    let mut swapped_evidence = rules.clone();
    swapped_evidence[0].clauses[0].evidence_span_ids = vec!["fact:b-span".into()];
    assert!(run(&swapped_evidence, &facts, &origins, &Default::default(), limits).is_err());
    let mut empty = facts.clone(); empty[0].label = "---".into();
    assert!(run(&rules, &empty, &origins, &Default::default(), limits).is_err());
}

fn cluster_rule(id: &str, title: &str, kind: ArchaeologyRuleKind, supporting: &[&str],
    contradicting: &[&str]) -> ArchaeologyRulePacket {
    let mut spans = supporting.iter().chain(contradicting).map(|id| format!("{id}-span")).collect::<Vec<_>>();
    spans.sort(); spans.dedup();
    ArchaeologyRulePacket { rule_id: id.into(), repository_id: "repository:packets".into(),
        generation_id: "generation:packets".into(), revision_sha: LINK_REVISION.into(), kind,
        title: title.into(), domain_ids: vec![], lifecycle: ArchaeologyRuleLifecycle::Candidate,
        trust: ArchaeologyTrust::Deterministic, confidence: ArchaeologyConfidence::High,
        clauses: vec![ArchaeologyRuleClause { clause_id: format!("clause:{id}"), text: title.into(),
            trust: ArchaeologyTrust::Deterministic, confidence: ArchaeologyConfidence::High,
            supporting_fact_ids: supporting.iter().map(|id| (*id).into()).collect(),
            contradicting_fact_ids: contradicting.iter().map(|id| (*id).into()).collect(),
            evidence_span_ids: spans, caveats: vec![] }], dependency_rule_ids: vec![],
        conflict_rule_ids: vec![], alias_rule_ids: vec![], coverage: Default::default(),
        parser_identity: "parser:manifest".into(), algorithm_identity: "algorithm:v1".into(),
        synthesis_identity: None }
}
fn cluster_origin(fact_id: &str, classification: ArchaeologySourceClassification,
    path_identity: &str) -> ArchaeologyFactOrigin {
    ArchaeologyFactOrigin { fact_id: fact_id.into(), source_unit_id: format!("unit:{fact_id}"),
        path_identity: path_identity.into(),
        ranking_path_identity: stable_graph_id("archaeology-ranking-path", path_identity),
        classification }
}

fn packet_fact(id: &str, kind: ArchaeologyFactKind, label: &str, attributes: &[(&str, &str)]) -> ArchaeologyFact {
    let semantic_source = std::iter::once(label)
        .chain(attributes.iter().flat_map(|(key, value)| [*key, *value]))
        .collect::<Vec<_>>()
        .join(" ");
    let mut attributes = attributes.iter().map(|(key, value)| ArchaeologyAttribute {
        key: (*key).into(), value: (*value).into()
    }).collect::<Vec<_>>();
    if kind != ArchaeologyFactKind::Unresolved {
        attributes.push(ArchaeologyAttribute {
            key: "semantic_expr".into(), value: semantic_expression(&semantic_source, true).unwrap()
        });
    }
    ArchaeologyFact { fact_id: id.into(), kind, label: label.into(), span_ids: vec![format!("{id}-span")],
        parser_id: "parser:v1".into(), trust: ArchaeologyTrust::Extracted, confidence: ArchaeologyConfidence::High,
        attributes }
}
fn packet_edge(id: &str, from: &str, to: &str, kind: ArchaeologyFactEdgeKind, reason: Option<&str>) -> ArchaeologyFactEdge {
    ArchaeologyFactEdge { edge_id: id.into(), from_fact_id: from.into(), to_fact_id: to.into(), kind,
        trust: ArchaeologyTrust::Deterministic, evidence_span_ids: vec![format!("{from}-span"), format!("{to}-span")],
        unresolved_reason: reason.map(str::to_string) }
}
}

#[derive(Clone, Default)]
struct LinkFixture { units: Vec<LinkUnitFixture>, facts: Vec<LinkFactFixture> }
#[derive(Clone)]
struct LinkUnitFixture { id: String, language: String, dialect: Option<String>, path: Option<String>, lineage: Vec<ArchaeologyAdapterLineage> }
#[derive(Clone)]
struct LinkFactFixture { unit: String, fact: ArchaeologyFact, spans: Vec<ArchaeologySourceSpan> }
fn unresolved_edge(id: &str, from: &str, to: &str) -> ArchaeologyFactEdge {
    ArchaeologyFactEdge { edge_id: id.into(), from_fact_id: from.into(), to_fact_id: to.into(),
        kind: ArchaeologyFactEdgeKind::Unresolved, trust: ArchaeologyTrust::Extracted,
        evidence_span_ids: vec![format!("{from}-span")], unresolved_reason: Some("old".into()) }
}
impl LinkFixture {
    fn unit(&mut self, id: &str, language: &str, dialect: Option<&str>, path: Option<&str>) {
        self.units.push(LinkUnitFixture { id: id.into(), language: language.into(), dialect: dialect.map(str::to_string), path: path.map(str::to_string), lineage: vec![] });
    }
    fn lineage(&mut self, unit: &str, kind: ArchaeologyLineageKind, span: &str) {
        self.units.iter_mut().find(|item| item.id == unit).unwrap().lineage.push(ArchaeologyAdapterLineage {
            kind, source_unit_id: unit.into(), target_source_unit_id: None, evidence_span_id: span.into(), detail: "unresolved target".into() });
    }
    fn fact(&mut self, unit: &str, id: &str, kind: ArchaeologyFactKind, label: &str, attributes: &[(&str, &str)]) {
        let span_id = format!("{id}-span");
        self.facts.push(LinkFactFixture { unit: unit.into(), fact: ArchaeologyFact { fact_id: id.into(), kind,
            label: label.into(), span_ids: vec![span_id.clone()], parser_id: "fixture".into(),
            trust: ArchaeologyTrust::Extracted, confidence: ArchaeologyConfidence::High,
            attributes: attributes.iter().map(|(key, value)| ArchaeologyAttribute { key: (*key).into(), value: (*value).into() }).collect() },
            spans: vec![ArchaeologySourceSpan { span_id, source_unit_id: unit.into(), revision_sha: LINK_REVISION.into(),
                start: ArchaeologyPosition { byte: 0, line: 1, column: 1 }, end: ArchaeologyPosition { byte: 1, line: 1, column: 2 } }] });
    }
    fn link(&self, edges: &[ArchaeologyFactEdge], limits: ArchaeologyLinkLimits,
        cancellation: Option<&StructuralGraphCancellation>) -> Result<ArchaeologyLinkPatch, String> {
        let units = self.units.iter().map(|item| ArchaeologyLinkUnit { source_unit_id: &item.id,
            language: &item.language, dialect: item.dialect.as_deref(), relative_path: item.path.as_deref(), lineage: &item.lineage }).collect::<Vec<_>>();
        let facts = self.facts.iter().map(|item| ArchaeologyLinkFact { source_unit_id: &item.unit,
            fact: &item.fact, evidence_spans: &item.spans }).collect::<Vec<_>>();
        link_archaeology_facts("repository", LINK_REVISION, &units, &facts, edges,
            cancellation.unwrap_or(&StructuralGraphCancellation::default()), limits)
    }
}
}

#[derive(Clone, Serialize, Deserialize)]
struct Corpus {
    schema_version: u32,
    corpus_id: String,
    revisions: BTreeMap<String, String>,
    source_units: Vec<SourceUnit>,
    spans: Vec<Span>,
    facts: Vec<Fact>,
    edges: Vec<Edge>,
    rules: Vec<Rule>,
    duplicate_groups: Vec<DuplicateGroup>,
    conflicts: Vec<Conflict>,
    gaps: Vec<Gap>,
    history_changes: Vec<HistoryChange>,
    negative_cases: Vec<NegativeCase>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SourceUnit {
    id: String,
    path: String,
    revision: String,
    language: String,
    dialect: String,
    parser_id: String,
    classification: String,
    generated: bool,
    protected: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Span {
    id: String,
    source_unit_id: String,
    start: [u64; 3],
    end: [u64; 3],
    text: Option<String>,
    #[serde(default)]
    protected: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Fact {
    id: String,
    kind: String,
    label: String,
    span_ids: Vec<String>,
    trust: String,
    confidence: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Edge {
    id: String,
    from: String,
    to: String,
    kind: String,
    span_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Rule {
    id: String,
    revision: String,
    kind: String,
    lifecycle: String,
    primary: bool,
    alias_of: Option<String>,
    clauses: Vec<Clause>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Clause {
    id: String,
    kind: String,
    text: String,
    supporting_fact_ids: Vec<String>,
    contradicting_fact_ids: Vec<String>,
    span_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DuplicateGroup {
    id: String,
    primary_rule_id: String,
    rule_ids: Vec<String>,
    reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Conflict {
    id: String,
    rule_ids: Vec<String>,
    fact_ids: Vec<String>,
    reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Gap {
    id: String,
    source_unit_id: String,
    kind: String,
    span_id: Option<String>,
    reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct HistoryChange {
    id: String,
    from_revision: String,
    to_revision: String,
    before_rule_id: String,
    after_rule_id: String,
    classification: String,
    span_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct NegativeCase {
    id: String,
    assertion: String,
    target_id: String,
    expected_error: String,
}

#[test]
fn labeled_corpus_is_exact_connected_and_privacy_safe() {
    let corpus = parse_corpus();
    validate_corpus(&corpus, MANIFEST).expect("valid hand-labeled corpus");

    let dialects = corpus
        .source_units
        .iter()
        .map(|unit| (unit.language.as_str(), unit.dialect.as_str()))
        .collect::<BTreeSet<_>>();
    for expected in [
        ("typescript", "typescript"),
        ("cobol", "ibm-fixed"),
        ("cobol", "free"),
        ("cobol", "ibm-copybook"),
        ("assembly", "hlasm"),
        ("assembly", "x86-64-gas-att"),
        ("assembly", "ambiguous"),
    ] {
        assert!(dialects.contains(&expected), "missing {expected:?}");
    }
    assert_eq!(corpus.duplicate_groups.len(), 1);
    assert_eq!(corpus.conflicts.len(), 1);
    assert_eq!(corpus.history_changes.len(), 1);
}

#[test]
fn validator_catches_span_reference_clause_and_secret_regressions() {
    let corpus = parse_corpus();

    let mut off_by_one = corpus.clone();
    off_by_one.spans[0].start[0] += 1;
    assert!(validate_corpus(&off_by_one, &encoded(&off_by_one))
        .unwrap_err()
        .contains("coordinate"));

    let mut dangling = corpus.clone();
    dangling.edges[0].from = "fact:missing".to_string();
    assert!(validate_corpus(&dangling, &encoded(&dangling))
        .unwrap_err()
        .contains("unknown fact"));

    let mut unsupported = corpus.clone();
    unsupported.rules[0].clauses[0].kind = "intent".to_string();
    assert!(validate_corpus(&unsupported, &encoded(&unsupported))
        .unwrap_err()
        .contains("unsupported clause"));

    let protected = fs::read_to_string(source_root().join("protected/private_rules.env"))
        .expect("protected fixture");
    let mut leaked = corpus;
    leaked.rules[0].clauses[0].text = protected.trim().to_string();
    assert!(validate_corpus(&leaked, &encoded(&leaked))
        .unwrap_err()
        .contains("protected literal"));

    let mut leaked_value = parse_corpus();
    leaked_value.rules[0].clauses[0].text = protected
        .split_once('=')
        .expect("key/value protected fixture")
        .1
        .trim()
        .to_string();
    assert!(validate_corpus(&leaked_value, &encoded(&leaked_value))
        .unwrap_err()
        .contains("protected literal"));
}

#[test]
fn validator_rejects_cross_entity_integrity_mutations() {
    let corpus = parse_corpus();
    type CorpusMutation = (&'static str, fn(&mut Corpus));
    let cases: &[CorpusMutation] = &[
        ("lowercase SHA-256", |value| {
            value
                .revisions
                .get_mut("current")
                .unwrap()
                .make_ascii_uppercase()
        }),
        ("invalid normalized fact", |value| {
            value.facts[0].span_ids.clear()
        }),
        ("has no source span", |value| {
            value.edges[0].span_ids.clear()
        }),
        ("unsupported clause evidence", |value| {
            value.rules[0].clauses[0].span_ids.clear()
        }),
        ("does not overlap supporting fact", |value| {
            value.rules[0].clauses[0].span_ids = vec!["span:x86:entry".to_string()]
        }),
        ("fixture classification differs", |value| {
            value.source_units[8].generated = false
        }),
        ("fixture classification differs", |value| {
            value.source_units[11].classification = "source".to_string()
        }),
        ("invalid duplicate group", |value| {
            value.duplicate_groups[0].primary_rule_id = "rule:claim:generated".to_string()
        }),
        ("span belongs to another unit", |value| {
            value.gaps[0].source_unit_id = "unit:x86".to_string()
        }),
        ("revision differs from rule", |value| {
            value.history_changes[0].before_rule_id = "rule:payment:current".to_string()
        }),
        ("negative-case coverage changed", |value| {
            value.negative_cases[0].target_id = "unit:missing".to_string()
        }),
        ("ambiguous fact semantics", |value| {
            let fact = value
                .facts
                .iter_mut()
                .find(|fact| fact.id == "fact:ambiguous")
                .unwrap();
            fact.kind = "predicate".to_string();
            fact.confidence = "high".to_string();
        }),
        ("generated fact confidence", |value| {
            let fact = value
                .facts
                .iter_mut()
                .find(|fact| fact.id == "fact:generated:predicate")
                .unwrap();
            fact.confidence = "high".to_string();
        }),
        ("does not support both endpoint facts", |value| {
            value.edges[0].span_ids = vec!["span:x86:entry".to_string()]
        }),
        ("at least two unique rules", |value| {
            value.duplicate_groups[0].rule_ids = vec!["rule:claim:duplicate".to_string()];
            let rule = value
                .rules
                .iter_mut()
                .find(|rule| rule.id == "rule:claim:eligible")
                .unwrap();
            rule.primary = true;
            rule.alias_of = None;
        }),
        ("alias outside its group", |value| {
            value
                .rules
                .iter_mut()
                .find(|rule| rule.id == "rule:claim:eligible")
                .unwrap()
                .alias_of = Some("rule:payment:current".to_string())
        }),
        ("different revisions and rules", |value| {
            value.history_changes[0].from_revision = "current".to_string();
            value.history_changes[0].before_rule_id = "rule:payment:current".to_string();
        }),
        ("invalid history change", |value| {
            value.history_changes[0].classification = "probably_changed".to_string()
        }),
        ("lacks changed condition evidence", |value| {
            value.history_changes[0].span_ids = vec![
                "span:history:approved".to_string(),
                "span:modern:approved".to_string(),
            ]
        }),
        ("negative-case coverage changed", |value| {
            let rule = value
                .rules
                .iter_mut()
                .find(|rule| rule.id == "rule:claim:generated")
                .unwrap();
            rule.clauses[0].supporting_fact_ids = vec!["fact:fixed:predicate".to_string()];
            rule.clauses[0].span_ids = vec!["span:fixed:predicate".to_string()];
            rule.clauses[1].supporting_fact_ids = vec!["fact:fixed:eligible".to_string()];
            rule.clauses[1].span_ids = vec!["span:fixed:eligible".to_string()];
        }),
    ];

    for (expected, mutate) in cases {
        let mut mutated = corpus.clone();
        mutate(&mut mutated);
        let error = match validate_corpus(&mutated, &encoded(&mutated)) {
            Ok(()) => panic!("mutation for {expected:?} was accepted"),
            Err(error) => error,
        };
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn validator_rejects_every_source_classification_swap() {
    let corpus = parse_corpus();
    for left in 0..corpus.source_units.len() {
        for right in left + 1..corpus.source_units.len() {
            if corpus.source_units[left].classification == corpus.source_units[right].classification
            {
                continue;
            }
            let mut mutated = corpus.clone();
            let (left_classification, left_generated, left_protected) = {
                let unit = &corpus.source_units[left];
                (unit.classification.clone(), unit.generated, unit.protected)
            };
            mutated.source_units[left].classification =
                corpus.source_units[right].classification.clone();
            mutated.source_units[left].generated = corpus.source_units[right].generated;
            mutated.source_units[left].protected = corpus.source_units[right].protected;
            mutated.source_units[right].classification = left_classification;
            mutated.source_units[right].generated = left_generated;
            mutated.source_units[right].protected = left_protected;
            assert!(validate_corpus(&mutated, &encoded(&mutated))
                .unwrap_err()
                .contains("fixture classification differs"));
        }
    }
}

fn parse_corpus() -> Corpus {
    serde_json::from_str(MANIFEST).expect("fixture manifest")
}

fn encoded(corpus: &Corpus) -> String {
    serde_json::to_string(corpus).expect("encode mutated corpus")
}

fn validate_corpus(corpus: &Corpus, encoded_manifest: &str) -> Result<(), String> {
    if corpus.schema_version != 1 || corpus.corpus_id.is_empty() {
        return Err("unsupported corpus identity".to_string());
    }
    for revision in corpus.revisions.values() {
        if revision.len() != 64
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("revision must be a lowercase SHA-256".to_string());
        }
    }

    let unit_ids = unique(
        corpus.source_units.iter().map(|unit| unit.id.as_str()),
        "source unit",
    )?;
    let span_ids = unique(corpus.spans.iter().map(|span| span.id.as_str()), "span")?;
    let fact_ids = unique(corpus.facts.iter().map(|fact| fact.id.as_str()), "fact")?;
    let edge_ids = unique(corpus.edges.iter().map(|edge| edge.id.as_str()), "edge")?;
    let rule_ids = unique(corpus.rules.iter().map(|rule| rule.id.as_str()), "rule")?;
    let clause_ids = unique(
        corpus
            .rules
            .iter()
            .flat_map(|rule| rule.clauses.iter().map(|clause| clause.id.as_str())),
        "clause",
    )?;
    unique(corpus.gaps.iter().map(|gap| gap.id.as_str()), "gap")?;
    unique(
        corpus.negative_cases.iter().map(|case| case.id.as_str()),
        "negative case",
    )?;

    let units = corpus
        .source_units
        .iter()
        .map(|unit| (unit.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let spans = corpus
        .spans
        .iter()
        .map(|span| (span.id.as_str(), span))
        .collect::<BTreeMap<_, _>>();
    let facts = corpus
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let rules = corpus
        .rules
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect::<BTreeMap<_, _>>();
    for unit in &corpus.source_units {
        let path = Path::new(&unit.path);
        if path.is_absolute() || unit.path.split('/').any(|part| part == "..") {
            return Err(format!(
                "source path is not repository-relative: {}",
                unit.path
            ));
        }
        if !corpus.revisions.contains_key(&unit.revision)
            || unit.parser_id.is_empty()
            || unit.classification.is_empty()
        {
            return Err(format!("incomplete source identity: {}", unit.id));
        }
        if !source_root().join(path).is_file() {
            return Err(format!("missing source fixture: {}", unit.path));
        }
        let (classification, generated, protected) = expected_source_classification(&unit.id)
            .ok_or_else(|| format!("unexpected fixture source unit: {}", unit.id))?;
        if (unit.classification.as_str(), unit.generated, unit.protected)
            != (classification, generated, protected)
        {
            return Err(format!("fixture classification differs: {}", unit.id));
        }
    }
    for span in &corpus.spans {
        let unit = units
            .get(span.source_unit_id.as_str())
            .ok_or_else(|| format!("span {} has unknown source unit", span.id))?;
        validate_span(span, unit)?;
    }

    let error_spans = corpus
        .gaps
        .iter()
        .filter(|gap| gap.kind == "parser_error_region")
        .filter_map(|gap| gap.span_id.as_deref())
        .map(|id| {
            spans
                .get(id)
                .copied()
                .ok_or_else(|| format!("unknown error span {id}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for fact in &corpus.facts {
        parse_enum::<ArchaeologyFactKind>(&fact.kind, "fact kind")?;
        if fact.label.trim().is_empty()
            || fact.span_ids.is_empty()
            || !matches!(fact.trust.as_str(), "extracted" | "deterministic")
            || !matches!(fact.confidence.as_str(), "high" | "medium" | "low")
        {
            return Err(format!("invalid normalized fact: {}", fact.id));
        }
        let mut fact_units = BTreeSet::new();
        for span_id in &fact.span_ids {
            let span = spans
                .get(span_id.as_str())
                .ok_or_else(|| format!("fact {} has unknown span {span_id}", fact.id))?;
            let unit = units[span.source_unit_id.as_str()];
            if unit.protected || overlaps_any(span, &error_spans) {
                return Err(format!(
                    "fact {} uses protected or error-region evidence",
                    fact.id
                ));
            }
            fact_units.insert(unit.id.as_str());
        }
        if fact_units.len() != 1 {
            return Err(format!("fact {} crosses source units", fact.id));
        }
        let unit = units[*fact_units.first().expect("fact has a source unit")];
        if unit.classification == "ambiguous"
            && (fact.kind != "unresolved" || fact.confidence != "low")
        {
            return Err(format!(
                "ambiguous fact semantics are promoted: {}",
                fact.id
            ));
        }
        if unit.generated && fact.confidence != "low" {
            return Err(format!(
                "generated fact confidence is promoted: {}",
                fact.id
            ));
        }
    }
    for edge in &corpus.edges {
        parse_enum::<ArchaeologyFactEdgeKind>(&edge.kind, "edge kind")?;
        if edge.span_ids.is_empty() {
            return Err(format!("edge {} has no source span", edge.id));
        }
        if !fact_ids.contains(edge.from.as_str()) || !fact_ids.contains(edge.to.as_str()) {
            return Err(format!("edge {} references unknown fact", edge.id));
        }
        require_all(&span_ids, &edge.span_ids, "edge span", &edge.id)?;
        if !edge_supports_fact(edge, facts[edge.from.as_str()], &spans)
            || !edge_supports_fact(edge, facts[edge.to.as_str()], &spans)
        {
            return Err(format!(
                "edge {} does not support both endpoint facts",
                edge.id
            ));
        }
    }

    let allowed_clauses = ["subject", "condition", "action", "exception", "quantifier"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for rule in &corpus.rules {
        parse_enum::<ArchaeologyRuleKind>(&rule.kind, "rule kind")?;
        if !corpus.revisions.contains_key(&rule.revision)
            || !matches!(
                rule.lifecycle.as_str(),
                "candidate" | "review_needed" | "superseded" | "conflicted"
            )
            || rule.clauses.is_empty()
        {
            return Err(format!("invalid rule packet: {}", rule.id));
        }
        if let Some(alias) = rule.alias_of.as_deref() {
            if !rule_ids.contains(alias) || rule.primary {
                return Err(format!("invalid rule alias: {}", rule.id));
            }
        }
        for clause in &rule.clauses {
            if !allowed_clauses.contains(clause.kind.as_str()) {
                return Err(format!("unsupported clause kind: {}", clause.kind));
            }
            if clause.text.trim().is_empty()
                || clause.supporting_fact_ids.is_empty()
                || clause.span_ids.is_empty()
            {
                return Err(format!("unsupported clause evidence: {}", clause.id));
            }
            require_all(
                &fact_ids,
                &clause.supporting_fact_ids,
                "supporting fact",
                &clause.id,
            )?;
            require_all(
                &fact_ids,
                &clause.contradicting_fact_ids,
                "contradicting fact",
                &clause.id,
            )?;
            require_all(&span_ids, &clause.span_ids, "clause span", &clause.id)?;
            for fact_id in &clause.supporting_fact_ids {
                let fact = facts[fact_id.as_str()];
                if !fact.span_ids.iter().any(|fact_span_id| {
                    let fact_span = spans[fact_span_id.as_str()];
                    clause
                        .span_ids
                        .iter()
                        .any(|clause_span_id| overlaps(fact_span, spans[clause_span_id.as_str()]))
                }) {
                    return Err(format!(
                        "clause {} does not overlap supporting fact {}",
                        clause.id, fact.id
                    ));
                }
            }
            if clause
                .span_ids
                .iter()
                .any(|id| units[spans[id.as_str()].source_unit_id.as_str()].protected)
            {
                return Err(format!("clause {} uses protected evidence", clause.id));
            }
        }
        let supporting_units = rule_supporting_units(rule, &facts, &spans, &units);
        if supporting_units.iter().any(|unit| {
            unit.protected
                || matches!(unit.classification.as_str(), "ambiguous" | "error_recovery")
                || unit.revision != rule.revision
        }) {
            return Err(format!(
                "rule {} uses non-semantic source classification",
                rule.id
            ));
        }
        let has_generated = supporting_units.iter().any(|unit| unit.generated);
        if has_generated
            && (!supporting_units.iter().all(|unit| unit.generated)
                || rule.primary
                || rule.alias_of.is_none())
        {
            return Err(format!(
                "generated rule semantics are promoted: {}",
                rule.id
            ));
        }
    }

    for group in &corpus.duplicate_groups {
        require_all(&rule_ids, &group.rule_ids, "duplicate rule", &group.id)?;
        let members = group
            .rule_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let primaries = group
            .rule_ids
            .iter()
            .filter(|id| rules[id.as_str()].primary)
            .map(String::as_str)
            .collect::<Vec<_>>();
        let aliases_stay_in_group = group.rule_ids.iter().all(|id| {
            rules[id.as_str()]
                .alias_of
                .as_deref()
                .is_none_or(|alias| members.contains(alias))
        });
        if members.len() < 2
            || members.len() != group.rule_ids.len()
            || !members.contains(group.primary_rule_id.as_str())
            || primaries.as_slice() != [group.primary_rule_id.as_str()]
            || !aliases_stay_in_group
            || group.reason.trim().is_empty()
        {
            let detail = if !aliases_stay_in_group {
                "alias outside its group"
            } else {
                "at least two unique rules and exactly one primary"
            };
            return Err(format!("invalid duplicate group {}: {detail}", group.id));
        }
        if group.rule_ids.iter().any(|id| {
            let rule = rules[id.as_str()];
            rule.alias_of.is_some()
                && rule.alias_of.as_deref() != Some(group.primary_rule_id.as_str())
        }) {
            return Err(format!("invalid duplicate group: {}", group.id));
        }
    }
    for conflict in &corpus.conflicts {
        require_all(&rule_ids, &conflict.rule_ids, "conflict rule", &conflict.id)?;
        require_all(&fact_ids, &conflict.fact_ids, "conflict fact", &conflict.id)?;
        if conflict.rule_ids.len() < 2 || conflict.reason.is_empty() {
            return Err(format!("invalid conflict: {}", conflict.id));
        }
    }
    for gap in &corpus.gaps {
        if !unit_ids.contains(gap.source_unit_id.as_str()) || gap.reason.is_empty() {
            return Err(format!("invalid coverage gap: {}", gap.id));
        }
        if let Some(span_id) = gap.span_id.as_deref() {
            if !span_ids.contains(span_id) {
                return Err(format!("gap {} has unknown span", gap.id));
            }
            if spans[span_id].source_unit_id != gap.source_unit_id {
                return Err(format!("gap {} span belongs to another unit", gap.id));
            }
        }
    }
    for change in &corpus.history_changes {
        if !corpus.revisions.contains_key(&change.from_revision)
            || !corpus.revisions.contains_key(&change.to_revision)
            || !rule_ids.contains(change.before_rule_id.as_str())
            || !rule_ids.contains(change.after_rule_id.as_str())
            || !matches!(
                change.classification.as_str(),
                "condition_changed" | "action_changed" | "condition_and_action_changed"
            )
        {
            return Err(format!("invalid history change: {}", change.id));
        }
        if rules[change.before_rule_id.as_str()].revision != change.from_revision
            || rules[change.after_rule_id.as_str()].revision != change.to_revision
        {
            return Err(format!(
                "history change {} revision differs from rule",
                change.id
            ));
        }
        if change.from_revision == change.to_revision
            || change.before_rule_id == change.after_rule_id
        {
            return Err(format!(
                "history change {} requires different revisions and rules",
                change.id
            ));
        }
        require_all(&span_ids, &change.span_ids, "history span", &change.id)?;
        let revisions = change
            .span_ids
            .iter()
            .map(|id| {
                units[spans[id.as_str()].source_unit_id.as_str()]
                    .revision
                    .as_str()
            })
            .collect::<BTreeSet<_>>();
        if !revisions.contains(change.from_revision.as_str())
            || !revisions.contains(change.to_revision.as_str())
        {
            return Err(format!(
                "history change {} lacks revision evidence",
                change.id
            ));
        }
        let before = rules[change.before_rule_id.as_str()];
        let after = rules[change.after_rule_id.as_str()];
        for (label, fact_kinds) in history_evidence_kinds(&change.classification) {
            let before_spans = rule_fact_spans(before, &facts, fact_kinds);
            let after_spans = rule_fact_spans(after, &facts, fact_kinds);
            let cited_before = change
                .span_ids
                .iter()
                .filter(|id| before_spans.contains(id.as_str()))
                .collect::<Vec<_>>();
            let cited_after = change
                .span_ids
                .iter()
                .filter(|id| after_spans.contains(id.as_str()))
                .collect::<Vec<_>>();
            let before_text = cited_before
                .iter()
                .map(|id| spans[id.as_str()].text.as_deref())
                .collect::<BTreeSet<_>>();
            let after_text = cited_after
                .iter()
                .map(|id| spans[id.as_str()].text.as_deref())
                .collect::<BTreeSet<_>>();
            if cited_before.is_empty() || cited_after.is_empty() || before_text == after_text {
                return Err(format!(
                    "history change {} lacks changed {label} evidence",
                    change.id
                ));
            }
        }
    }

    let expected_negatives = [
        "ambiguous_semantics_suppressed",
        "protected_content_excluded",
        "error_region_not_evidence",
        "generated_duplicate_not_primary",
        "unsupported_clause_rejected",
        "dangling_reference_rejected",
        "secret_value_not_retained",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let actual_negatives = corpus
        .negative_cases
        .iter()
        .map(|case| case.assertion.as_str())
        .collect::<BTreeSet<_>>();
    if actual_negatives != expected_negatives
        || corpus.negative_cases.len() != expected_negatives.len()
        || corpus.negative_cases.iter().any(|case| {
            let typed_target = match case.assertion.as_str() {
                "ambiguous_semantics_suppressed" => units
                    .get(case.target_id.as_str())
                    .is_some_and(|unit| unit.classification == "ambiguous"),
                "protected_content_excluded" | "secret_value_not_retained" => units
                    .get(case.target_id.as_str())
                    .is_some_and(|unit| unit.protected),
                "error_region_not_evidence" => {
                    error_spans.iter().any(|span| span.id == case.target_id)
                }
                "generated_duplicate_not_primary" => {
                    rules.get(case.target_id.as_str()).is_some_and(|rule| {
                        !rule.primary
                            && rule.alias_of.is_some()
                            && rule_supporting_units(rule, &facts, &spans, &units)
                                .iter()
                                .all(|unit| unit.generated)
                    })
                }
                "unsupported_clause_rejected" => clause_ids.contains(case.target_id.as_str()),
                "dangling_reference_rejected" => edge_ids.contains(case.target_id.as_str()),
                _ => false,
            };
            !typed_target || case.expected_error.trim().is_empty()
        })
    {
        return Err("explicit negative-case coverage changed".to_string());
    }

    for protected in corpus.source_units.iter().filter(|unit| unit.protected) {
        let literal = fs::read_to_string(source_root().join(&protected.path))
            .map_err(|error| format!("read protected fixture: {error}"))?;
        if protected_fragments(&literal)
            .iter()
            .any(|fragment| encoded_manifest.contains(fragment))
        {
            return Err("protected literal leaked into expected output".to_string());
        }
    }
    Ok(())
}

fn validate_span(span: &Span, unit: &SourceUnit) -> Result<(), String> {
    let source = fs::read_to_string(source_root().join(&unit.path))
        .map_err(|error| format!("read {}: {error}", unit.path))?;
    let start = usize::try_from(span.start[0]).map_err(|_| "start byte exceeds usize")?;
    let end = usize::try_from(span.end[0]).map_err(|_| "end byte exceeds usize")?;
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return Err(format!("span {} has invalid byte range", span.id));
    }
    if coordinate(&source, start) != span.start || coordinate(&source, end) != span.end {
        return Err(format!("span {} coordinate is off by one", span.id));
    }
    if unit.protected != span.protected {
        return Err(format!("span {} protected classification differs", span.id));
    }
    match span.text.as_deref() {
        Some(_) if unit.protected => {
            return Err(format!("span {} exposes protected text", span.id))
        }
        Some(text) if text != &source[start..end] => {
            return Err(format!("span {} expected text differs", span.id));
        }
        None if !unit.protected => return Err(format!("span {} omits expected text", span.id)),
        _ => {}
    }
    Ok(())
}

fn coordinate(source: &str, byte: usize) -> [u64; 3] {
    let prefix = &source[..byte];
    let line = prefix.bytes().filter(|value| *value == b'\n').count() as u64 + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count() as u64
        + 1;
    [byte as u64, line, column]
}

fn overlaps_any(span: &Span, ranges: &[&Span]) -> bool {
    ranges.iter().any(|range| overlaps(span, range))
}

fn overlaps(left: &Span, right: &Span) -> bool {
    left.source_unit_id == right.source_unit_id
        && left.start[0] < right.end[0]
        && right.start[0] < left.end[0]
}

fn edge_supports_fact(edge: &Edge, fact: &Fact, spans: &BTreeMap<&str, &Span>) -> bool {
    edge.span_ids.iter().any(|edge_id| {
        fact.span_ids
            .iter()
            .any(|fact_id| overlaps(spans[edge_id.as_str()], spans[fact_id.as_str()]))
    })
}

fn expected_source_classification(id: &str) -> Option<(&'static str, bool, bool)> {
    Some(match id {
        "unit:modern" => ("reference", false, false),
        "unit:history" => ("historical", false, false),
        "unit:cobol-fixed" | "unit:cobol-free" | "unit:duplicate" | "unit:hlasm" | "unit:x86" => {
            ("source", false, false)
        }
        "unit:copybook" => ("copybook", false, false),
        "unit:ambiguous" => ("ambiguous", false, false),
        "unit:generated" => ("generated_listing", true, false),
        "unit:recovery" => ("error_recovery", false, false),
        "unit:conflict" => ("conflicting_source", false, false),
        "unit:protected" => ("protected", false, true),
        _ => return None,
    })
}

fn rule_supporting_units<'a>(
    rule: &Rule,
    facts: &BTreeMap<&str, &'a Fact>,
    spans: &BTreeMap<&str, &Span>,
    units: &BTreeMap<&str, &'a SourceUnit>,
) -> Vec<&'a SourceUnit> {
    let ids = rule
        .clauses
        .iter()
        .flat_map(|clause| &clause.supporting_fact_ids)
        .flat_map(|id| &facts[id.as_str()].span_ids)
        .map(|id| spans[id.as_str()].source_unit_id.as_str())
        .collect::<BTreeSet<_>>();
    ids.into_iter().map(|id| units[id]).collect()
}

fn history_evidence_kinds(classification: &str) -> Vec<(&'static str, &'static [&'static str])> {
    const CONDITION: &[&str] = &["predicate", "decision", "control_flow"];
    const ACTION: &[&str] = &[
        "mutation",
        "calculation",
        "call",
        "input_output",
        "transaction",
    ];
    match classification {
        "condition_changed" => vec![("condition", CONDITION)],
        "action_changed" => vec![("action", ACTION)],
        "condition_and_action_changed" => vec![("condition", CONDITION), ("action", ACTION)],
        _ => Vec::new(),
    }
}

fn rule_fact_spans<'a>(
    rule: &Rule,
    facts: &BTreeMap<&str, &'a Fact>,
    kinds: &[&str],
) -> BTreeSet<&'a str> {
    rule.clauses
        .iter()
        .flat_map(|clause| &clause.supporting_fact_ids)
        .map(|id| facts[id.as_str()])
        .filter(|fact| kinds.contains(&fact.kind.as_str()))
        .flat_map(|fact| fact.span_ids.iter().map(String::as_str))
        .collect()
}

fn protected_fragments(value: &str) -> BTreeSet<&str> {
    value
        .lines()
        .flat_map(|line| {
            let line = line.trim();
            let mut fragments = vec![line];
            if let Some((key, secret)) = line.split_once('=') {
                fragments.extend([key.trim(), secret.trim()]);
            }
            fragments.extend(
                line.split(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '_'
                }),
            );
            fragments
        })
        .filter(|fragment| fragment.len() >= 8)
        .collect()
}

fn parse_enum<T: for<'de> Deserialize<'de>>(value: &str, label: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| format!("unsupported {label}: {value}"))
}

fn unique<'a>(
    values: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<BTreeSet<&'a str>, String> {
    let mut result = BTreeSet::new();
    for value in values {
        if value.is_empty() || !result.insert(value) {
            return Err(format!("empty or duplicate {label}: {value}"));
        }
    }
    Ok(result)
}

fn require_all(
    known: &BTreeSet<&str>,
    values: &[String],
    label: &str,
    owner: &str,
) -> Result<(), String> {
    if values.iter().any(|value| !known.contains(value.as_str())) {
        Err(format!("{owner} references unknown {label}"))
    } else {
        Ok(())
    }
}

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/business_rule_archaeology/fixtures/sources")
}
